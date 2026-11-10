//! Eight or sixteen independent counter-zero compressions, one full-round compression
//! per lane. Descriptors borrow blocks and CVs; keys, lengths and flags may
//! differ between lanes. ROOT selects root digests; other blocks produce CVs.
#[cfg(test)]
use crate::hasher;
use crate::{BLOCK_LEN, CHUNK_END, CHUNK_LEN, CHUNK_START, Hasher, OUT_LEN, ROOT, platform::Platform};

#[repr(C)]
pub(crate) struct Input<'a> {
    block: &'a [u8; BLOCK_LEN],
    cv: &'a [u32; 8],
    len: u8,
    flags: u8,
}
const _: () = {
    assert!(core::mem::size_of::<Input<'static>>() == 24);
    assert!(core::mem::offset_of!(Input<'static>, block) == 0);
    assert!(core::mem::offset_of!(Input<'static>, cv) == 8);
    assert!(core::mem::offset_of!(Input<'static>, len) == 16);
    assert!(core::mem::offset_of!(Input<'static>, flags) == 17);
};
#[repr(C, align(64))]
struct Params8 {
    msg: [*const u8; 8],
    cvs: [[u32; 8]; 8],
    lens: [u32; 8],
    flags: [u32; 8],
}
const _: () = {
    assert!(core::mem::size_of::<Params8>() == 384);
    assert!(core::mem::offset_of!(Params8, cvs) == 64);
    assert!(core::mem::offset_of!(Params8, lens) == 320);
    assert!(core::mem::offset_of!(Params8, flags) == 352);
};
unsafe extern "C" {
    fn blake3_servil_output16(inputs: *const Input<'_>, out: *mut u8);
    fn blake3_servil_output8(params: *const Params8, out: *mut u8);
}

pub(crate) fn lanes(platform: Platform) -> usize {
    match platform {
        Platform::AVX512 => 16,
        Platform::AVX2 => 8,
        _ => 0,
    }
}

/// A whole message still in its first chunk, with no staged suffix or
/// stacked subtree. Its final block is already padded by ChunkState.
/// Other states keep Hasher's existing finalization path.
pub(crate) fn input(hasher: &Hasher) -> Option<Input<'_>> {
    let c = &hasher.core.chunk_state;
    (hasher.staged == 0 && hasher.core.initial_chunk_counter == 0 && c.chunk_counter == 0 && hasher.core.cv_stack.len() == 0).then(|| Input {
        block: &c.buf,
        cv: &c.cv,
        len: c.buf_len,
        flags: c.flags | c.start_flag() | CHUNK_END | ROOT,
    })
}

/// Requires AVX2 for W=8, AVX-512F+VL for W=16, on Linux. Every descriptor
/// borrows a readable padded block/CV, length 0..64. All counters are zero.
pub(crate) unsafe fn hash_group<const W: usize>(inputs: &[Input<'_>; W], out: &mut [[u8; OUT_LEN]]) {
    assert_eq!(out.len(), W, "one compression output per lane");
    if W == 16 {
        crate::ran_kernel(blake3_servil_output16 as *const ());
        // Sound: caller supplies the ISA; fixed descriptors and checked
        // output provide readable inputs and room for every store.
        unsafe { blake3_servil_output16(inputs.as_ptr(), out.as_mut_ptr().cast()) };
    } else {
        assert_eq!(W, 8, "eight or sixteen compression lanes");
        let p = Params8 {
            msg: core::array::from_fn(|i| inputs[i].block.as_ptr()),
            cvs: core::array::from_fn(|w| core::array::from_fn(|i| inputs[i].cv[w])),
            lens: core::array::from_fn(|i| inputs[i].len as u32),
            flags: core::array::from_fn(|i| inputs[i].flags as u32),
        };
        crate::ran_kernel(blake3_servil_output8 as *const ());
        // Sound: caller supplies AVX2; all borrowed blocks reach64bytes,
        // and the parameter layout and output size are checked above.
        unsafe { blake3_servil_output8(&p, out.as_mut_ptr().cast()) };
    }
}

/// Hash complete held stages without grouping by their keys. Leaf calls
/// already fill their chunk lanes; parents borrow each message's own key
/// and flags in the existing independent-compression descriptors. The
/// caller supplies 1..=TABLE stages and a detected AVX2/AVX512 platform.
/// Keep this workspace frame out of empty and unsupported callers.
#[inline(never)]
pub(crate) fn held_groups(hashers: &mut [Hasher], held: &[usize], platform: Platform) {
    let width = lanes(platform);
    debug_assert!(!held.is_empty() && width != 0);
    const PER: usize = crate::STAGE_LEN / CHUNK_LEN;
    const { assert!(PER == 16) };
    let n = held.len();
    let mut work = crate::each::HeldWorkspace::<{ crate::many::TABLE }>::new();
    let (mut level, mut next) = work.buffers(n);
    for (k, &i) in held.iter().enumerate() {
        let h = &hashers[i];
        let (counter, bytes) = h.held_group().expect("a held group");
        let refs: [&[u8; CHUNK_LEN]; PER] = core::array::from_fn(|j| bytes[j * CHUNK_LEN..(j + 1) * CHUNK_LEN].try_into().unwrap());
        platform.hash_many(
            &refs,
            &h.core.key,
            counter,
            crate::IncrementCounter::Yes,
            h.core.chunk_state.flags,
            CHUNK_START,
            CHUNK_END,
            level[k * PER..(k + 1) * PER].as_flattened_mut(),
        );
    }
    for depth in 0..4 {
        let per_message = PER >> (depth + 1);
        let count = n * per_message;
        let input = |j: usize| {
            let h = &hashers[held[j / per_message]];
            Input {
                block: level.as_flattened()[j * BLOCK_LEN..(j + 1) * BLOCK_LEN].try_into().unwrap(),
                cv: &h.core.key,
                len: BLOCK_LEN as u8,
                flags: h.core.chunk_state.flags | crate::PARENT,
            }
        };
        let mut full = count / width * width;
        for base in (0..full).step_by(width) {
            // Sound: width matches the detected ISA. All blocks and keys
            // are readable; next supplies disjoint complete outputs.
            if width == 16 {
                let inputs = core::array::from_fn(|j| input(base + j));
                unsafe { hash_group::<16>(&inputs, &mut next[base..base + 16]) };
            } else {
                let inputs = core::array::from_fn(|j| input(base + j));
                unsafe { hash_group::<8>(&inputs, &mut next[base..base + 8]) };
            }
        }
        if width == 16 && count - full >= 8 && crate::platform::avx2_detected() {
            let inputs = core::array::from_fn(|j| input(full + j));
            // Sound: AVX2 is detected; eight real parent blocks and keys
            // remain, and next has disjoint room for all eight outputs.
            unsafe { hash_group::<8>(&inputs, &mut next[full..full + 8]) };
            full += 8;
        }
        for (j, out) in next.iter_mut().enumerate().take(count).skip(full) {
            let input = input(j);
            let mut cv = *input.cv;
            platform.compress_in_place(&mut cv, input.block, input.len, 0, input.flags);
            *out = crate::hasher::le_bytes(&cv);
        }
        core::mem::swap(&mut level, &mut next);
    }
    for (k, &i) in held.iter().enumerate() {
        let (counter, _) = hashers[i].held_group().unwrap();
        let mut result = [0u8; BLOCK_LEN];
        if counter == 0 {
            // Four swaps leave each final parent's two children in next.
            result.copy_from_slice(next[2 * k..2 * k + 2].as_flattened());
        } else {
            result[..OUT_LEN].copy_from_slice(&level[k]);
        }
        hashers[i].take_held(&result);
    }
}

const SELF_TEST_EXPECTED: u64 = 0x153e0dfbeac2bb44;
fn self_test_shape(mut hash: impl FnMut(&[Input<'_>; 16], &mut [[u8; OUT_LEN]])) -> u64 {
    let mut lens: [u8; 16] = [0, 1, 31, 63, 64, 7, 32, 55, 3, 15, 48, 60, 2, 33, 62, 64];
    let flags: [u8; 16] = core::array::from_fn(|i| {
        let shape = [3u8, 2, 4][i % 3];
        if shape == 4 {
            lens[i] = 64;
        }
        shape | [0u8, 16, 32, 64][i % 4]
    });
    let cvs: [[u32; 8]; 16] = core::array::from_fn(|i| {
        core::array::from_fn(|w| (0x1234abcdu32.wrapping_add(i as u32 * 0x9e37).wrapping_add(w as u32 * 0x1020304)).wrapping_mul(0x85ebca6b))
    });
    let blocks: [[u8; 64]; 16] = core::array::from_fn(|i| core::array::from_fn(|j| if j < lens[i] as usize { ((i * 37 + j * 17) ^ 0x5a) as u8 } else { 0 }));
    let inputs = core::array::from_fn(|i| Input { block: &blocks[i], cv: &cvs[i], len: lens[i], flags: flags[i] | if (i / 4) % 2 == 0 { ROOT } else { 0 } });
    let mut out = [[0u8; OUT_LEN]; 16];
    hash(&inputs, &mut out);
    crate::self_test::fold(out.as_flattened())
}
fn check_self_test(answer: u64) {
    assert_eq!(answer, SELF_TEST_EXPECTED, "blake3-servil startup self-test failed: independent counter-zero compressions");
}
unsafe fn hash_eight_groups(inputs: &[Input<'_>; 16], out: &mut [[u8; OUT_LEN]]) {
    for (inputs, out) in inputs.chunks_exact(8).zip(out.chunks_mut(8)) {
        unsafe { hash_group::<8>(inputs.try_into().unwrap(), out) };
    }
}
unsafe fn hash_on_platform(platform: Platform, inputs: &[Input<'_>; 16], out: &mut [[u8; OUT_LEN]]) {
    if lanes(platform) == 16 {
        unsafe { hash_group::<16>(inputs, out) };
    } else {
        assert_eq!(lanes(platform), 8);
        unsafe { hash_eight_groups(inputs, out) };
    }
}
pub(crate) fn self_test(platform: Platform) -> usize {
    if lanes(platform) == 0 {
        return 0;
    }
    let mut checked = 1;
    let answer = self_test_shape(|inputs, out| {
        // Sound: initialize detected this platform before testing its kernels.
        unsafe { hash_on_platform(platform, inputs, out) };
        if lanes(platform) == 16 && crate::platform::avx2_detected() {
            // Check the wide output before the remainder kernel reuses it.
            check_self_test(crate::self_test::fold(out.as_flattened()));
            unsafe { hash_eight_groups(inputs, out) };
            checked = 2;
        }
    });
    check_self_test(answer);
    checked
}
#[cfg(test)]
pub(crate) fn entry() -> *const () {
    blake3_servil_output16 as *const ()
}
#[cfg(test)]
pub(crate) fn entry8() -> *const () {
    blake3_servil_output8 as *const ()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_answers_match_independent_compression() {
        crate::initialize();
        let answer = self_test_shape(|inputs, out| {
            for (input, digest) in inputs.iter().zip(out) {
                let mut cv = *input.cv;
                unsafe { crate::sse41::compress_in_place(&mut cv, input.block, input.len, 0, input.flags) };
                *digest = hasher::le_bytes(&cv);
            }
        });
        assert_eq!(answer, SELF_TEST_EXPECTED);
        if lanes(Platform::detect()) != 0 {
            assert_eq!(self_test_shape(|inputs, out| unsafe { hash_on_platform(Platform::detect(), inputs, out) }), answer);
        }
    }
    #[test]
    fn independent_blocks_cvs_descriptors_and_output_against_guard_pages() {
        use crate::test::guard_pages::Guarded;
        crate::initialize();
        if lanes(Platform::detect()) == 0 {
            return;
        }
        for at_end in [false, true] {
            let answer = self_test_shape(|inputs, out| {
                let mut blocks: Vec<_> = (0..16).map(|_| Guarded::new(64, at_end)).collect();
                let mut cvs: Vec<_> = (0..16).map(|_| Guarded::new(32, at_end)).collect();
                let mut table = Guarded::new(core::mem::size_of_val(inputs), at_end);
                let table_ptr = table.bytes().as_mut_ptr().cast::<Input<'_>>();
                for i in 0..16 {
                    blocks[i].bytes().copy_from_slice(inputs[i].block);
                    cvs[i].bytes().copy_from_slice(&hasher::le_bytes(inputs[i].cv));
                    // Page placement aligns CVs to u32 and descriptors to
                    // Input. The guarded mappings outlive every borrowed view.
                    let block = unsafe { &*blocks[i].bytes().as_ptr().cast::<[u8; 64]>() };
                    let cv = unsafe { &*cvs[i].bytes().as_ptr().cast::<[u32; 8]>() };
                    unsafe { table_ptr.add(i).write(Input { block, cv, len: inputs[i].len, flags: inputs[i].flags }) };
                }
                for page in blocks.iter_mut().chain(cvs.iter_mut()) {
                    page.read_only();
                }
                table.read_only();
                let descriptors = unsafe { &*table_ptr.cast::<[Input<'_>; 16]>() };
                let mut output = Guarded::new(16 * OUT_LEN, at_end);
                let guarded_out = output.bytes().as_chunks_mut::<OUT_LEN>().0;
                unsafe { hash_on_platform(Platform::detect(), descriptors, guarded_out) };
                let expected = crate::self_test::fold(guarded_out.as_flattened());
                if crate::platform::avx2_detected() {
                    unsafe { hash_eight_groups(descriptors, guarded_out) };
                    assert_eq!(crate::self_test::fold(guarded_out.as_flattened()), expected);
                }
                out.copy_from_slice(guarded_out);
            });
            assert_eq!(answer, SELF_TEST_EXPECTED, "guard placement at_end={at_end}");
        }
    }

    #[test]
    fn startup_covers_each_supported_platform() {
        crate::initialize();
        for platform in crate::test::every_platform() {
            crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
            let checked = self_test(platform);
            let width = lanes(platform);
            let eight = width == 8 || (width == 16 && crate::platform::avx2_detected());
            let sixteen = width == 16;
            assert_eq!(checked, usize::from(eight) + usize::from(sixteen), "startup families on {platform:?}");
            let entries = crate::RAN_ENTRIES.with(|r| r.borrow().clone());
            assert_eq!(entries.contains(&(entry8() as usize)), eight, "eight-lane startup on {platform:?}");
            assert_eq!(entries.contains(&(entry() as usize)), sixteen, "sixteen-lane startup on {platform:?}");
        }
    }

    #[test]
    fn startup_rejects_corrupted_answer() {
        assert!(std::panic::catch_unwind(|| check_self_test(SELF_TEST_EXPECTED ^ 1)).is_err());
    }

    #[test]
    fn incremental_roots_batch_all_modes_and_selected_indices() {
        crate::initialize();
        for count in [0usize, 1, 15, 16, 17, 31, 32, 65] {
            let mut hashers = Vec::new();
            let mut expected = Vec::new();
            let mut empty = Vec::new();
            let mut next = Vec::new();
            for i in 0..count {
                let len = [0usize, 1, 31, 63, 64, 65, 127, 255, 511, 1023, 1024][i % 11];
                let data: Vec<u8> = (0..len).map(|j| (i * 37 + j * 17) as u8).collect();
                let key = [i as u8; 32];
                let mode = match i % 3 {
                    0 => crate::Mode::Plain,
                    1 => crate::Mode::Keyed(&key),
                    _ => crate::Mode::DeriveKey("BLAKE3-Servil bulk-root test"),
                };
                let mut h = Hasher::new(mode, None);
                for part in data.chunks(17) {
                    h.update(part);
                }
                expected.push(crate::test::official::hash(mode, &data));
                empty.push(crate::test::official::hash(mode, b""));
                next.push(crate::test::official::hash(mode, b"next"));
                hashers.push(h);
            }
            let which: Vec<usize> = (0..count).rev().collect();
            let mut out = vec![[0u8; 32]; count];
            crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
            crate::each::finalize_each(&mut hashers, &which, &mut out);
            let wanted: Vec<_> = which.iter().map(|&i| expected[i]).collect();
            assert_eq!(out, wanted);
            let reached = crate::RAN_ENTRIES.with(|r| r.borrow().contains(&(entry() as usize)));
            assert_eq!(reached, count >= 16 && matches!(Platform::detect(), Platform::AVX512));
            assert!(hashers.iter().all(|h| h.count() == 0), "selected hashers reset");
            crate::each::finalize_each(&mut hashers, &which, &mut out);
            let wanted: Vec<_> = which.iter().map(|&i| empty[i]).collect();
            assert_eq!(out, wanted, "reset preserves every key and mode");
            for h in &mut hashers {
                h.update(b"next");
            }
            let selected: Vec<_> = (0..count).step_by(2).collect();
            let mut out = vec![[0u8; 32]; selected.len()];
            crate::each::finalize_each(&mut hashers, &selected, &mut out);
            let wanted: Vec<_> = selected.iter().map(|&i| next[i]).collect();
            assert_eq!(out, wanted);
            for (i, h) in hashers.iter().enumerate() {
                assert_eq!(h.count(), if i % 2 == 0 { 0 } else { 4 }, "only selected states reset");
            }
        }
    }

    #[test]
    fn staged_long_roots_keep_the_existing_path() {
        crate::initialize();
        let mut hashers = Vec::new();
        let mut expected = Vec::new();
        for i in 0..32 {
            let data = vec![i as u8; if i == 3 || i == 20 { 4097 } else { 129 }];
            let mut h = Hasher::new(crate::Mode::Plain, None);
            for piece in data.chunks(23) {
                h.update(piece);
            }
            expected.push(crate::test::official::hash(crate::Mode::Plain, &data));
            hashers.push(h);
        }
        let which: Vec<_> = (0..32).collect();
        let mut out = vec![[0u8; 32]; 32];
        crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
        crate::each::finalize_each(&mut hashers, &which, &mut out);
        assert_eq!(out, expected);
        assert!(!crate::RAN_ENTRIES.with(|r| r.borrow().contains(&(entry() as usize))));
    }
}
