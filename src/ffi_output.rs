//! Eight or sixteen independent counter-zero compressions, one full-round compression
//! per lane. Descriptors borrow blocks and CVs; keys, lengths and flags may
//! differ between lanes. ROOT selects root digests; other blocks produce CVs.
#[cfg(test)]
use crate::hasher;
use crate::output_common::self_test_shape;
pub(crate) use crate::output_common::{Input, check_self_test, input};
use crate::{BLOCK_LEN, CHUNK_END, CHUNK_LEN, CHUNK_START, Hasher, OUT_LEN, ROOT, platform::Platform};

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

/// A packed message's descriptor (`Packed<16>`, `Packed<32>`): its block
/// zero-padded past `len` bytes, the key as its chaining value, and the
/// flags of a whole one-chunk root (CHUNK_START, CHUNK_END and ROOT with
/// the mode's). The caller checks `len <= BLOCK_LEN` for the batch.
pub(crate) fn packed<'a>(block: &'a [u8; BLOCK_LEN], key: &'a [u32; 8], len: u8, flags: u8) -> Input<'a> {
    Input { block, cv: key, len, flags: flags | CHUNK_START | CHUNK_END | ROOT }
}

/// Final block descriptor for a multi-block message inside chunk 0 whose
/// preceding full blocks already compressed into `cv`.
pub(crate) fn final_block<'a>(block: &'a [u8; BLOCK_LEN], cv: &'a [u32; 8], len: u8, flags: u8) -> Input<'a> {
    Input { block, cv, len, flags: flags | CHUNK_END | ROOT }
}

/// Arbitrary counter-zero final block descriptor (for variable-length items in
/// `hash_each`, where `k == 0` includes `CHUNK_START` and `k > 0` does not).
pub(crate) fn item_last<'a>(block: &'a [u8; BLOCK_LEN], cv: &'a [u32; 8], len: u8, flags: u8) -> Input<'a> {
    Input { block, cv, len, flags }
}

/// Padded lane count for a `<= 16`-item prefix bucket in `each::by_blocks_at_once`:
/// rounds `9..=15 -> 16` on AVX-512, `5..=7 -> 8` on AVX2/AVX-512, and
/// `13..=15 -> 16` on AVX2 (avoiding a 5..7-lane scalar tail after 8 lanes).
pub(crate) fn by_blocks_prefix_padded(n: usize, width: usize) -> usize {
    if width == 16 && n >= 9 {
        16
    } else if width != 0 && (5..8).contains(&n) {
        8
    } else if width == 8 && (13..16).contains(&n) {
        16
    } else {
        n
    }
}

/// Pad `inputs` in place for a `<= 16`-item prefix bucket in
/// `each::by_blocks_at_once`, returning the padded lane count.
pub(crate) fn pad_by_blocks_inputs<const GROUP: usize>(inputs: &mut crate::stack_vec::StackVec<*const u8, GROUP>, platform: Platform) -> usize {
    let n = inputs.len();
    let target = by_blocks_prefix_padded(n, lanes(platform));
    let last = *inputs.last().unwrap();
    inputs.extend(core::iter::repeat_n(last, target - n));
    target
}

/// Finish counter-zero final blocks in `each::by_blocks_at_once` using
/// 16-lane `output16` and 8-lane `output8` for chunks of `>= 4` items,
/// returning how many leading entries of `order` were written into `out`.
pub(crate) fn finish_last_blocks_order(
    order: &[usize],
    items: &[&[u8]],
    cvs: &[crate::CVWords],
    counter: u64,
    flags: u8,
    flags_end: u8,
    out: &mut [[u8; OUT_LEN]],
    platform: Platform,
) -> usize {
    let width = lanes(platform);
    if counter != 0 || width == 0 {
        return 0;
    }
    let chunks = order.chunks_exact(width);
    let rem = chunks.remainder();
    for chunk in chunks {
        if width == 16 {
            finish_last_blocks_simd::<16>(chunk, items, cvs, flags, flags_end, out);
        } else {
            finish_last_blocks_simd::<8>(chunk, items, cvs, flags, flags_end, out);
        }
    }
    if rem.len() < 4 {
        return order.len() - rem.len();
    }
    if rem.len() <= 8 {
        finish_last_blocks_simd::<8>(rem, items, cvs, flags, flags_end, out);
    } else {
        finish_last_blocks_simd::<16>(rem, items, cvs, flags, flags_end, out);
    }
    order.len()
}

/// Compress `4..=W` (`W` in `{8, 16}`) counter-zero final blocks for
/// `each::by_blocks_at_once` in a single `output8`/`output16` call, padding
/// spare lanes by repeating the last item in `chunk`.
#[inline(never)]
pub(crate) fn finish_last_blocks_simd<const W: usize>(
    chunk: &[usize],
    items: &[&[u8]],
    cvs: &[crate::CVWords],
    flags: u8,
    flags_end: u8,
    out: &mut [[u8; OUT_LEN]],
) {
    let count = chunk.len();
    let mut blocks = [[0u8; BLOCK_LEN]; W];
    let mut lens = [0u8; W];
    let mut item_flags = [0u8; W];
    for (j, &i) in chunk.iter().enumerate() {
        let item = items[i];
        let k = item.len().saturating_sub(1) / BLOCK_LEN;
        let last = item.len() - k * BLOCK_LEN;
        blocks[j][..last].copy_from_slice(&item[k * BLOCK_LEN..]);
        lens[j] = last.to_le_bytes()[0];
        let start = if k == 0 { CHUNK_START } else { 0 };
        item_flags[j] = flags | start | flags_end;
    }
    let inputs: [Input<'_>; W] = core::array::from_fn(|j| {
        let idx = j.min(count - 1);
        item_last(&blocks[idx], &cvs[chunk[idx]], lens[idx], item_flags[idx])
    });
    let mut scratch = [[0u8; OUT_LEN]; W];
    // Sound: caller verified platform supports W-lane output16/output8;
    // every block/CV reference stays live across the call.
    unsafe { hash_group::<W>(&inputs, &mut scratch) };
    for (j, &i) in chunk.iter().enumerate() {
        out[i] = scratch[j];
    }
}

/// Finish packed counter-zero eligible `slots` from `ffi_partial::finalize_each`
/// using 16-lane `output16` (full 16s and padded 9..=15 tails on AVX-512) and
/// 8-lane `output8` (full 8s and padded 4..=7 tails on both AVX-512 and AVX2),
/// returning how many leading entries of `slots` were written into `o_chunk`.
pub(crate) fn finalize_output_slots(hashers: &[Hasher], w_chunk: &[usize], slots: &[u8], o_chunk: &mut [[u8; OUT_LEN]], width: usize) -> usize {
    if width == 0 {
        return 0;
    }
    let mut rest = slots;
    if width == 16 {
        let chunks = rest.chunks_exact(16);
        rest = chunks.remainder();
        for part in chunks {
            let inputs: [Input<'_>; 16] = core::array::from_fn(|j| input(&hashers[w_chunk[usize::from(part[j])]]).unwrap());
            let mut digests = [[0u8; OUT_LEN]; 16];
            // Sound: caller verified Platform::AVX512; every descriptor borrows
            // a live Hasher in `hashers` and `digests` holds 16 slots.
            unsafe { hash_group::<16>(&inputs, &mut digests) };
            for (&k, digest) in part.iter().zip(&digests) {
                o_chunk[usize::from(k)] = *digest;
            }
        }
        if rest.len() >= 9 {
            let inputs: [Input<'_>; 16] = core::array::from_fn(|j| input(&hashers[w_chunk[usize::from(rest[j.min(rest.len() - 1)])]]).unwrap());
            let mut digests = [[0u8; OUT_LEN]; 16];
            // Sound: caller verified Platform::AVX512; spare lanes repeat the
            // last eligible descriptor and their outputs are discarded.
            unsafe { hash_group::<16>(&inputs, &mut digests) };
            for (&k, digest) in rest.iter().zip(&digests) {
                o_chunk[usize::from(k)] = *digest;
            }
            return slots.len();
        }
    }
    let chunks = rest.chunks_exact(8);
    rest = chunks.remainder();
    for part in chunks {
        let inputs: [Input<'_>; 8] = core::array::from_fn(|j| input(&hashers[w_chunk[usize::from(part[j])]]).unwrap());
        let mut digests = [[0u8; OUT_LEN]; 8];
        // Sound: both Platform::AVX512 and Platform::AVX2 support AVX2
        // `output8`; every descriptor borrows a live Hasher in `hashers`.
        unsafe { hash_group::<8>(&inputs, &mut digests) };
        for (&k, digest) in part.iter().zip(&digests) {
            o_chunk[usize::from(k)] = *digest;
        }
    }
    if rest.len() >= 4 {
        let inputs: [Input<'_>; 8] = core::array::from_fn(|j| input(&hashers[w_chunk[usize::from(rest[j.min(rest.len() - 1)])]]).unwrap());
        let mut digests = [[0u8; OUT_LEN]; 8];
        // Sound: both Platform::AVX512 and Platform::AVX2 support AVX2
        // `output8`; spare lanes repeat the last eligible descriptor.
        unsafe { hash_group::<8>(&inputs, &mut digests) };
        for (&k, digest) in rest.iter().zip(&digests) {
            o_chunk[usize::from(k)] = *digest;
        }
        return slots.len();
    }
    slots.len() - rest.len()
}

pub(crate) fn lanes(platform: Platform) -> usize {
    match platform {
        Platform::AVX512 => 16,
        Platform::AVX2 => 8,
        _ => 0,
    }
}

#[cfg(test)]
std::thread_local! {
    /// Widths (`8` or `16`) passed to `hash_group` on this thread, for dispatch tests.
    pub(crate) static OUTPUT_CALLS: core::cell::RefCell<std::vec::Vec<usize>> = const { core::cell::RefCell::new(std::vec::Vec::new()) };
}

/// Requires AVX2 for W=8, AVX-512F+VL for W=16, on Linux. Every descriptor
/// borrows a readable padded block/CV, length 0..64. All counters are zero.
pub(crate) unsafe fn hash_group<const W: usize>(inputs: &[Input<'_>; W], out: &mut [[u8; OUT_LEN]]) {
    assert_eq!(out.len(), W, "one compression output per lane");
    #[cfg(test)]
    let _ = OUTPUT_CALLS.try_with(|c| c.borrow_mut().push(W));
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
/// Dispatches to a 16-stage workspace for small turns so they do not
/// reserve the full 128-stage stack frame.
pub(crate) fn held_groups(hashers: &mut [Hasher], held: &[usize], platform: Platform) {
    if held.len() <= 16 {
        held_groups_in::<16>(hashers, held, platform);
    } else {
        held_groups_in::<{ crate::many::TABLE }>(hashers, held, platform);
    }
}

#[inline(never)]
fn held_groups_in<const N: usize>(hashers: &mut [Hasher], held: &[usize], platform: Platform) {
    let width = lanes(platform);
    debug_assert!(!held.is_empty() && width != 0);
    const PER: usize = crate::STAGE_LEN / CHUNK_LEN;
    const { assert!(PER == 16) };
    let n = held.len();
    let mut work = crate::each::HeldWorkspace::<N>::new();
    let (mut level, mut next) = work.buffers(n);
    for (k, &i) in held.iter().enumerate() {
        let h = &hashers[i];
        let (counter, bytes) = h.held_group().expect("a held group");
        let refs: [&[u8; CHUNK_LEN]; PER] = core::array::from_fn(|j| bytes[j * CHUNK_LEN..(j + 1) * CHUNK_LEN].try_into().unwrap());
        let domain = h.domain();
        platform.hash_many(
            &refs,
            domain.key(),
            counter,
            crate::IncrementCounter::Yes,
            domain.flags(),
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
            let domain = h.domain();
            Input {
                block: level.as_flattened()[j * BLOCK_LEN..(j + 1) * BLOCK_LEN].try_into().unwrap(),
                cv: domain.key(),
                len: u8::try_from(BLOCK_LEN).unwrap(),
                flags: domain.flags() | crate::PARENT,
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

unsafe fn hash_eight_groups(inputs: &[Input<'_>; 16], out: &mut [[u8; OUT_LEN]]) {
    for (inputs, out) in inputs.chunks_exact(8).zip(out.chunks_mut(8)) {
        // Sound: caller verified AVX2/AVX-512 support; each chunk has exact length 8 matching `hash_group::<8>`.
        unsafe { hash_group::<8>(inputs.try_into().unwrap(), out) };
    }
}
unsafe fn hash_on_platform(platform: Platform, inputs: &[Input<'_>; 16], out: &mut [[u8; OUT_LEN]]) {
    if lanes(platform) == 16 {
        // Sound: `lanes(platform) == 16` guarantees AVX-512F+VL support; `inputs` and `out` hold 16 entries.
        unsafe { hash_group::<16>(inputs, out) };
    } else {
        assert_eq!(lanes(platform), 8);
        // Sound: `lanes(platform) == 8` guarantees AVX2 support; `inputs` and `out` hold 16 entries.
        unsafe { hash_eight_groups(inputs, out) };
    }
}
/// Planted corruption selector for subprocess fail-stop tests:
/// 1 = corrupt last output byte after 16-lane pass (before 8-lane overwrite),
/// 2 = corrupt last output byte of the final pass.
#[cfg(test)]
pub(crate) static PLANTED_WRONG_OUTPUT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

#[inline(never)]
pub(crate) fn self_test(platform: Platform) -> usize {
    if lanes(platform) == 0 {
        return 0;
    }
    let mut checked = 1;
    let answer = self_test_shape(|inputs, out| {
        // Sound: initialize detected this platform before testing its kernels.
        unsafe { hash_on_platform(platform, inputs, out) };
        if lanes(platform) == 16 && crate::platform::avx2_detected() {
            #[cfg(test)]
            if PLANTED_WRONG_OUTPUT.load(core::sync::atomic::Ordering::Relaxed) == 1 {
                out[15][OUT_LEN - 1] = !out[15][OUT_LEN - 1];
            }
            // Check the wide output before the remainder kernel reuses it.
            check_self_test(crate::self_test::fold(out.as_flattened()));
            // Sound: `avx2_detected()` is true above; `inputs` and `out` hold 16 entries.
            unsafe { hash_eight_groups(inputs, out) };
            checked = 2;
        }
        #[cfg(test)]
        if PLANTED_WRONG_OUTPUT.load(core::sync::atomic::Ordering::Relaxed) == 2 {
            out[15][OUT_LEN - 1] = !out[15][OUT_LEN - 1];
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
    use crate::output_common::{SELF_TEST_EXPECTED, check_self_test_value};
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
        use crate::test::guard::Guarded;
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

    /// Short one-block batches (1 to 63 bytes) and multi-block short-last
    /// batches (65 to 1023 bytes) on Linux x86-64 take `output16` / `output8`
    /// for whole groups and padded tails (`4..=8` on `output8`, `9..=16` on
    /// `output16`), while `1..=3` remainders stay serial. Packed `16/32/48`-byte
    /// remainders of `10..=15` on AVX-512 bypass `avx2_packed` to finish in one
    /// padded `output16` call.
    #[test]
    fn packed_groups_take_the_independent_kernel() {
        use crate::block::Prepared;
        use crate::throughput::hash_many;
        let platform = Platform::detect();
        let width = lanes(platform);
        if width == 0 {
            return;
        }
        let e16 = entry() as usize;
        let e8 = entry8() as usize;
        // Counts 1..=3 stay serial; 4..=8 take output8; on AVX-512, 9..=16 take
        // output16, and 17..=24 take output16 plus optional output8.
        let cases: &[(usize, Option<usize>, usize)] = if width == 16 {
            &[(3, None, 0), (4, Some(e8), 1), (8, Some(e8), 1), (9, Some(e16), 1), (15, Some(e16), 1), (16, Some(e16), 1), (32, Some(e16), 1)]
        } else {
            &[(3, None, 0), (4, Some(e8), 1), (7, Some(e8), 1), (8, Some(e8), 1), (16, Some(e8), 1)]
        };
        for &(count, expected_entry, expected_kernels) in cases {
            let mut out = std::vec![[0u8; OUT_LEN]; count];
            for len in [1, 33, 63] {
                let mut batch = crate::throughput::Batch::new(len, count);
                for i in 0..count {
                    batch.message_mut(i).fill(i as u8);
                }
                crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
                hash_many(crate::Mode::Plain, &batch, &mut out);
                let ran = crate::RAN_ENTRIES.with(|r| r.borrow().clone());
                match expected_entry {
                    Some(k) => {
                        assert!(ran.contains(&k), "{count} batch messages of {len} bytes should reach {k:#x}: {ran:?}");
                        assert_eq!(ran.len(), expected_kernels, "{count} batch messages of {len} bytes on the group kernel alone: {ran:?}");
                    }
                    None => {
                        assert!(!ran.contains(&e16) && !ran.contains(&e8), "{count} batch messages of {len} bytes should stay serial: {ran:?}");
                    }
                }
                for i in 0..count {
                    assert_eq!(out[i], *crate::convenience::hash(&std::vec![i as u8; len]).as_bytes(), "{len} bytes, message {i}");
                }
            }
        }
        // Verify Packed<16/32/48> remainder dispatch on AVX-512 vs AVX2:
        // on AVX-512 Linux std, 8..=9 remainders take `avx2_packed`, while
        // 10..=15 remainders bypass `avx2_packed` and finish in one `output16`.
        let avx2_packed = crate::ffi_packed::entries_avx2();
        let avx512_packed = crate::ffi_packed::entries_avx512();
        for (idx, len) in [16usize, 32, 48].into_iter().enumerate() {
            let p2 = avx2_packed[idx].1 as usize;
            let p512 = avx512_packed[idx].1 as usize;
            for count in [8usize, 9, 10, 11, 15, 24, 25, 26, 27, 31] {
                let mut bytes = std::vec![0u8; count * len];
                for i in 0..count {
                    for j in 0..len {
                        bytes[i * len + j] = ((i * 29 + j * 13 + len) & 0xff) as u8;
                    }
                }
                let mut out = std::vec![[0u8; OUT_LEN]; count];
                crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
                crate::many::hash_many_on(&bytes, len, len, crate::IV, 0, &mut out, platform);
                let ran = crate::RAN_ENTRIES.with(|r| r.borrow().clone());
                if width == 16 {
                    let rem = count % 16;
                    assert_eq!(ran.contains(&p512), count >= 16, "Packed<{len}> count={count} avx512_packed");
                    assert_eq!(ran.contains(&p2), (8..=9).contains(&rem), "Packed<{len}> count={count} avx2_packed");
                    assert_eq!(ran.contains(&e16), (10..=15).contains(&rem), "Packed<{len}> count={count} output16");
                } else {
                    assert!(ran.contains(&p2), "AVX2 Packed<{len}> count={count} avx2_packed");
                    let rem = count % 8;
                    assert_eq!(ran.contains(&e8), rem >= 4, "AVX2 Packed<{len}> count={count} output8");
                }
                for i in 0..count {
                    assert_eq!(out[i], *blake3::hash(&bytes[i * len..][..len]).as_bytes(), "Packed<{len}> count={count} msg {i}");
                }
            }
        }
        // Multi-block messages with a short last block (65..1023 B) tile by
        // platform width across every supported platform, exercising tile-2
        // `< 4` serial remainders (`17..19`, `129..132`) with distinct bytes
        // per slot and mixed-width `output16` + `output8` (`24` on AVX-512).
        let key = [0x42u8; crate::KEY_LEN];
        for plat in crate::test::every_platform() {
            let pw = lanes(plat);
            for len in [65, 100, 200, 500, 1000] {
                for count in [3, 4, 7, 8, 9, 15, 16, 17, 18, 19, 24, 33, 129, 130, 131, 132] {
                    let mut batch = crate::throughput::Batch::new(len, count);
                    for i in 0..count {
                        let msg = batch.message_mut(i);
                        for (j, b) in msg.iter_mut().enumerate() {
                            *b = ((i * 31 + j * 17 + len) & 0xff) as u8;
                        }
                    }
                    let mut out = std::vec![[0u8; OUT_LEN]; count];
                    let slot = batch.slot();
                    for mode in [crate::Mode::Plain, crate::Mode::Keyed(&key), crate::Mode::DeriveKey("x86-short-last")] {
                        let (kw, fl) = match mode {
                            crate::Mode::Plain => (*crate::IV, 0u8),
                            crate::Mode::Keyed(k) => (crate::platform::words_from_le_bytes_32(k), crate::KEYED_HASH),
                            crate::Mode::DeriveKey(ctx) => (
                                crate::platform::words_from_le_bytes_32(crate::hash_serial(ctx.as_bytes(), crate::IV, crate::DERIVE_KEY_CONTEXT).as_bytes()),
                                crate::DERIVE_KEY_MATERIAL,
                            ),
                            crate::Mode::DeriveKeyWith(_) => unreachable!(),
                        };
                        crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
                        crate::many::hash_many_on(batch.bytes(), len, slot, &kw, fl, &mut out, plat);
                        let ran = crate::RAN_ENTRIES.with(|r| r.borrow().clone());
                        if pw == 0 || count < 4 {
                            assert!(!ran.contains(&e16) && !ran.contains(&e8), "{plat:?} {count} x {len} B should stay serial");
                        } else if pw == 16 {
                            let need16 = count >= 9;
                            let need8 = (count < 9) || (4..=8).contains(&(count % 16));
                            assert_eq!(ran.contains(&e16), need16, "{plat:?} {count} x {len} B output16");
                            assert_eq!(ran.contains(&e8), need8, "{plat:?} {count} x {len} B output8");
                        } else {
                            assert!(ran.contains(&e8), "{plat:?} {count} x {len} B should reach output8");
                            assert!(!ran.contains(&e16), "{plat:?} {count} x {len} B should not reach output16");
                        }
                        for i in 0..count {
                            let msg = &batch.bytes()[i * slot..][..len];
                            let expected = match mode {
                                crate::Mode::Plain => *blake3::hash(msg).as_bytes(),
                                crate::Mode::Keyed(k) => *blake3::keyed_hash(k, msg).as_bytes(),
                                crate::Mode::DeriveKey(ctx) => blake3::derive_key(ctx, msg),
                                crate::Mode::DeriveKeyWith(_) => unreachable!(),
                            };
                            assert_eq!(out[i], expected, "{plat:?} {count} x {len} B, msg {i}");
                        }
                    }
                }
            }
        }
        // Pin prefix_padded_count boundaries for AVX2 (8) and AVX-512 (16).
        for (count, w, want) in [
            (4, 8, 4),
            (5, 8, 8),
            (7, 8, 8),
            (8, 8, 8),
            (12, 8, 12),
            (13, 8, 16),
            (4, 16, 4),
            (5, 16, 8),
            (8, 16, 8),
            (9, 16, 16),
            (15, 16, 16),
            (16, 16, 16),
            (20, 16, 20),
            (21, 16, 24),
            (24, 16, 24),
            (25, 16, 32),
        ] {
            assert_eq!(crate::many::prefix_padded_count(count, w), want, "prefix_padded_count({count}, {w})");
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
        assert!(!check_self_test_value(SELF_TEST_EXPECTED ^ 1));
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
                // Keyed/derived turns wait in ChunkState; plain turns and
                // <= 64 B updates wait in the stage. Eligible descriptors pack
                // into `output16` when 16 remain in `rest`.
                let mut h = Hasher::new(mode, None);
                if len <= BLOCK_LEN && i % 2 == 0 {
                    for part in data.chunks(17) {
                        h.update(part);
                    }
                    hashers.push(h);
                } else {
                    hashers.push(h);
                    for part in data.chunks(17) {
                        crate::each::update_each(&mut hashers, &[(i, part)]);
                    }
                }
                expected.push(crate::test::official::hash(mode, &data));
                empty.push(crate::test::official::hash(mode, b""));
                next.push(crate::test::official::hash(mode, b"next"));
            }
            let which: Vec<usize> = (0..count).rev().collect();
            let mut out = vec![[0u8; 32]; count];
            crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
            crate::each::finalize_each(&mut hashers, &which, &mut out);
            let wanted: Vec<_> = which.iter().map(|&i| expected[i]).collect();
            assert_eq!(out, wanted);
            // Plain multi-block items (> 64 B) are routed to `hash_many_rounded` /
            // `hash_by_blocks` on AVX2/AVX-512; `hash_by_blocks` calls `output16`
            // for chunks of >= 9 (`output8` for 4..=8), and `output_slots` packs
            // remaining eligible counter-zero descriptors the same way.
            let rest_count = (0..count)
                .filter(|&i| {
                    let len = [0usize, 1, 31, 63, 64, 65, 127, 255, 511, 1023, 1024][i % 11];
                    i % 3 != 0 || len <= BLOCK_LEN
                })
                .count();
            let short_plain_count = (0..count)
                .filter(|&i| {
                    let len = [0usize, 1, 31, 63, 64, 65, 127, 255, 511, 1023, 1024][i % 11];
                    i % 3 == 0 && len > BLOCK_LEN && len < CHUNK_LEN
                })
                .count();
            let reached = crate::RAN_ENTRIES.with(|r| r.borrow().contains(&(entry() as usize)));
            assert_eq!(reached, (rest_count >= 9 || short_plain_count >= 9) && matches!(Platform::detect(), Platform::AVX512));
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

    /// A keyed message partly in its chunk state (a turn's bytes) and partly in
    /// the stage (an update's after them) is not eligible for `output16`, and a
    /// plain message follows its staged path: every digest matches official BLAKE3.
    #[test]
    fn a_turn_then_an_update_finishes_outside_the_group() {
        crate::initialize();
        let key = [0x19; 32];
        for mode in [crate::Mode::Plain, crate::Mode::Keyed(&key)] {
            for (in_turn, updated) in [(100usize, 10usize), (10, 30), (64, 64), (0, 40)] {
                let data: Vec<Vec<u8>> = (0..32).map(|i| (0..in_turn + updated).map(|j| (i * 29 + j * 7) as u8).collect()).collect();
                let mut hashers: Vec<_> = (0..32).map(|_| Hasher::new(mode, None)).collect();
                for (i, d) in data.iter().enumerate() {
                    crate::each::update_each(&mut hashers, &[(i, &d[..in_turn])]);
                    hashers[i].update(&d[in_turn..]);
                }
                let which: Vec<usize> = (0..32).collect();
                let mut out = vec![[0u8; 32]; 32];
                crate::each::finalize_each(&mut hashers, &which, &mut out);
                for (d, got) in data.iter().zip(&out) {
                    assert_eq!(*got, crate::test::official::hash(mode, d), "in_turn={in_turn} updated={updated}");
                }
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
        crate::COUNTED.with(|c| c[crate::Counted::OutputGroupTurn as usize].set(0));
        crate::each::finalize_each(&mut hashers, &which, &mut out);
        assert_eq!(out, expected);
        // None of the 32 hashers enters `output_slots`: 129 B plain takes
        // `short_plain -> hash_by_blocks`, and 4097 B takes `rest`.
        assert_eq!(crate::COUNTED.with(|c| c[crate::Counted::OutputGroupTurn as usize].get()), 0);
    }

    /// Eligible `output_slots` descriptors pack independently around ineligible
    /// ones on both AVX-512 (`output16` + `output8`) and AVX2 (`output8`), and
    /// `OutputGroupTurn` counts once per `finalize_each` call even across
    /// multiple 128-slot windows.
    #[test]
    fn mixed_eligible_output_slots_pack_around_ineligible_messages() {
        crate::initialize();
        let key = [0x4Du8; 32];
        let width = lanes(Platform::detect());
        let mut hashers = Vec::new();
        let mut expected = Vec::new();
        for i in 0..17 {
            if i == 8 {
                let data = vec![0x77u8; 1025];
                let mut h = Hasher::new(crate::Mode::Plain, None);
                h.update(&data);
                hashers.push(h);
                expected.push(crate::test::official::hash(crate::Mode::Plain, &data));
            } else {
                let data = vec![i as u8; 32];
                hashers.push(Hasher::new(crate::Mode::Keyed(&key), None));
                crate::each::update_each(&mut hashers, &[(i, &data)]);
                expected.push(crate::test::official::hash(crate::Mode::Keyed(&key), &data));
            }
        }
        let which: Vec<usize> = (0..17).collect();
        let mut out = vec![[0u8; 32]; 17];
        OUTPUT_CALLS.with(|c| c.borrow_mut().clear());
        crate::COUNTED.with(|c| c[crate::Counted::OutputGroupTurn as usize].set(0));
        crate::each::finalize_each(&mut hashers, &which, &mut out);
        assert_eq!(out, expected);
        let calls = OUTPUT_CALLS.with(|c| c.borrow().clone());
        let want_calls: Vec<usize> = match width {
            16 => vec![16],
            8 => vec![8, 8],
            _ => vec![],
        };
        assert_eq!(calls, want_calls, "packed 16 eligible + 1 ineligible width={width}");
        assert_eq!(crate::COUNTED.with(|c| c[crate::Counted::OutputGroupTurn as usize].get()), u64::from(width != 0));

        // 256 eligible keyed hashers spanning two 128-slot windows still count one OutputGroupTurn.
        let mut many: Vec<_> = (0..256).map(|_| Hasher::new(crate::Mode::Keyed(&key), None)).collect();
        let pieces: Vec<_> = (0..256).map(|i| (i, b"short-keyed-msg".as_slice())).collect();
        crate::each::update_each(&mut many, &pieces);
        let which256: Vec<usize> = (0..256).collect();
        let mut out256 = vec![[0u8; 32]; 256];
        crate::COUNTED.with(|c| c[crate::Counted::OutputGroupTurn as usize].set(0));
        crate::each::finalize_each(&mut many, &which256, &mut out256);
        assert_eq!(crate::COUNTED.with(|c| c[crate::Counted::OutputGroupTurn as usize].get()), u64::from(width != 0));
    }
}
