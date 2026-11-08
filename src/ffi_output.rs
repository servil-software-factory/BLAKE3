//! Grouped first-chunk compression. On x86, descriptors carry each lane's
//! block, CV, length and flags independently. The ARM probe reuses the
//! existing equal-key/length NEON and SME2 message kernels.
#[cfg(all(test, not(blake3_neon)))]
use crate::hasher;
#[cfg(not(blake3_neon))]
use crate::platform::Platform;
use crate::{BLOCK_LEN, CHUNK_END, CHUNK_LEN, CHUNK_START, Hasher, OUT_LEN, ROOT};

#[cfg(not(blake3_neon))]
#[repr(C)]
pub(crate) struct Input<'a> {
    block: &'a [u8; BLOCK_LEN],
    cv: &'a [u32; 8],
    len: u8,
    flags: u8,
}
#[cfg(not(blake3_neon))]
const _: () = {
    assert!(core::mem::size_of::<Input<'static>>() == 24);
    assert!(core::mem::offset_of!(Input<'static>, block) == 0);
    assert!(core::mem::offset_of!(Input<'static>, cv) == 8);
    assert!(core::mem::offset_of!(Input<'static>, len) == 16);
    assert!(core::mem::offset_of!(Input<'static>, flags) == 17);
};
#[cfg(not(blake3_neon))]
unsafe extern "C" {
    fn blake3_servil_output16(inputs: *const Input<'_>, out: *mut u8);
}

/// A whole message still in its first chunk, with no staged suffix or
/// stacked subtree. Its final block is already padded by ChunkState.
/// Other states keep Hasher's existing finalization path.
#[cfg(not(blake3_neon))]
pub(crate) fn input(hasher: &Hasher) -> Option<Input<'_>> {
    let c = &hasher.core.chunk_state;
    (hasher.staged == 0 && hasher.core.initial_chunk_counter == 0 && c.chunk_counter == 0 && hasher.core.cv_stack.len() == 0).then(|| Input {
        block: &c.buf,
        cv: &c.cv,
        len: c.buf_len,
        flags: c.flags | c.start_flag() | CHUNK_END | ROOT,
    })
}

/// Requires AVX-512F+VL on Linux. Each descriptor holds a readable padded
/// block and CV; its block length is 0..64. Output holds sixteen CVs or root
/// digests, according to each descriptor's flags. All counters are zero.
#[cfg(not(blake3_neon))]
pub(crate) unsafe fn hash_group(inputs: &[Input<'_>; 16], out: &mut [[u8; OUT_LEN]]) {
    assert_eq!(out.len(), 16, "sixteen compression outputs");
    crate::ran_kernel(blake3_servil_output16 as *const ());
    // Sound: caller supplies the ISA; references and fixed descriptor count
    // provide the readable data, and the checked output holds all stores.
    unsafe { blake3_servil_output16(inputs.as_ptr(), out.as_mut_ptr().cast()) };
}

/// Gather a bulk caller's first chunk without serial prefix compression.
/// On x86 the chunk buffer holds the first block and the stage its suffix.
/// On ARM the existing stage holds the contiguous first chunk that its
/// message kernels take. Bulk finalization compresses those prefixes together.
pub(crate) fn gather_first(hasher: &mut Hasher, bytes: &[u8]) -> bool {
    let c = &mut hasher.core.chunk_state;
    if hasher.core.initial_chunk_counter != 0
        || c.chunk_counter != 0
        || c.blocks_compressed != 0
        || hasher.core.cv_stack.len() != 0
        || (hasher.staged != 0 && c.buf_len as usize != if cfg!(blake3_neon) { 0 } else { BLOCK_LEN })
        || c.buf_len as usize + hasher.staged + bytes.len() > CHUNK_LEN
    {
        return false;
    }
    #[cfg(blake3_neon)]
    {
        // Existing ARM message kernels take contiguous prefixes. A fresh
        // first chunk stays in the existing stage, with no extra buffer.
        if c.buf_len != 0 {
            return false;
        }
        hasher.stage(bytes);
    }
    #[cfg(not(blake3_neon))]
    {
        let take = bytes.len().min(BLOCK_LEN - c.buf_len as usize);
        let start = c.buf_len as usize;
        c.buf[start..start + take].copy_from_slice(&bytes[..take]);
        c.buf_len += take as u8;
        hasher.stage(&bytes[take..]);
    }
    true
}

/// Finalize sixteen first-chunk states whose uncompressed suffixes wait in
/// their stages. Hashers remain immutable. Finished lanes keep their digest
/// while longer lanes complete their prefix blocks.
#[cfg(not(blake3_neon))]
pub(crate) unsafe fn staged_group(hashers: &[Hasher], which: &[usize; 16], out: &mut [[u8; OUT_LEN]]) -> bool {
    let states = which.map(|i| &hashers[i]);
    if states.iter().any(|h| {
        let c = &h.core.chunk_state;
        h.core.initial_chunk_counter != 0
            || c.chunk_counter != 0
            || c.blocks_compressed != 0
            || h.core.cv_stack.len() != 0
            || (h.staged != 0 && c.buf_len as usize != BLOCK_LEN)
            || h.staged + c.buf_len as usize > CHUNK_LEN
    }) {
        return false;
    }
    let ends: [usize; 16] = core::array::from_fn(|i| (states[i].core.chunk_state.buf_len as usize + states[i].staged).saturating_sub(1) / BLOCK_LEN);
    let lens: [u8; 16] = core::array::from_fn(|i| (states[i].core.chunk_state.buf_len as usize + states[i].staged - ends[i] * BLOCK_LEN) as u8);
    let mut tails = [[0u8; BLOCK_LEN]; 16];
    for i in 0..16 {
        if states[i].staged == 0 {
            tails[i] = states[i].core.chunk_state.buf;
        } else {
            let start = (ends[i] - 1) * BLOCK_LEN;
            tails[i][..lens[i] as usize].copy_from_slice(&states[i].staged_bytes()[start..]);
        }
    }
    let mut cvs: [[u32; 8]; 16] = core::array::from_fn(|i| states[i].core.chunk_state.cv);
    let mut values = [[0u8; OUT_LEN]; 16];
    for block in 0..=*ends.iter().max().unwrap() {
        let inputs = core::array::from_fn(|i| {
            let last = block >= ends[i];
            let bytes = if last {
                &tails[i]
            } else if block == 0 {
                &states[i].core.chunk_state.buf
            } else {
                states[i].staged_bytes()[(block - 1) * BLOCK_LEN..block * BLOCK_LEN].try_into().unwrap()
            };
            Input {
                block: bytes,
                cv: &cvs[i],
                len: if last { lens[i] } else { BLOCK_LEN as u8 },
                flags: states[i].core.chunk_state.flags | if block == 0 { CHUNK_START } else { 0 } | if last { CHUNK_END | ROOT } else { 0 },
            }
        });
        // Sound: caller supplies AVX-512F+VL; every pointer borrows a readable
        // full block and CV, including the padded final blocks above.
        unsafe { hash_group(&inputs, &mut values) };
        for i in 0..16 {
            if block == ends[i] {
                out[i] = values[i];
            } else if block < ends[i] {
                cvs[i] = core::array::from_fn(|w| u32::from_le_bytes(values[i][4 * w..4 * w + 4].try_into().unwrap()));
            }
        }
    }
    true
}

#[cfg(not(blake3_neon))]
const SELF_TEST_EXPECTED: u64 = 0x153e0dfbeac2bb44;
#[cfg(not(blake3_neon))]
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
#[cfg(not(blake3_neon))]
fn check_self_test(answer: u64) {
    assert_eq!(answer, SELF_TEST_EXPECTED, "blake3-servil startup self-test failed: sixteen root outputs");
}
#[cfg(not(blake3_neon))]
pub(crate) fn self_test(platform: Platform) -> usize {
    if !matches!(platform, Platform::AVX512) {
        return 0;
    }
    let answer = self_test_shape(|inputs, out| {
        // Sound: initialize detected this platform before testing its kernels.
        unsafe { hash_group(inputs, out) };
    });
    check_self_test(answer);
    1
}
#[cfg(not(blake3_neon))]
#[cfg(test)]
pub(crate) fn entry() -> *const () {
    blake3_servil_output16 as *const ()
}

#[cfg(not(blake3_neon))]
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
        if matches!(Platform::detect(), Platform::AVX512) {
            assert_eq!(self_test_shape(|inputs, out| unsafe { hash_group(inputs, out) }), answer);
        }
    }
    #[test]
    fn independent_blocks_cvs_descriptors_and_output_against_guard_pages() {
        use crate::test::guard_pages::Guarded;
        crate::initialize();
        if !matches!(Platform::detect(), Platform::AVX512) {
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
                unsafe { hash_group(descriptors, guarded_out) };
                out.copy_from_slice(guarded_out);
            });
            assert_eq!(answer, SELF_TEST_EXPECTED, "guard placement at_end={at_end}");
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
    fn bulk_first_chunk_prefixes_all_modes_and_continuations() {
        crate::initialize();
        for count in [2, 15, 16, 17, 31, 32, 65] {
            let lens = [0usize, 1, 63, 64, 65, 127, 128, 255, 512, 1023, 1024];
            let keys: Vec<_> = (0..count).map(|i| [i as u8; 32]).collect();
            let data: Vec<Vec<u8>> = (0..count).map(|i| (0..lens[i % lens.len()]).map(|j| (i * 37 + j * 11) as u8).collect()).collect();
            let mode = |i: usize| match i % 3 {
                0 => crate::Mode::Plain,
                1 => crate::Mode::Keyed(&keys[i]),
                _ => crate::Mode::DeriveKey("BLAKE3-Servil bulk prefix test"),
            };
            let mut hashers: Vec<_> = (0..count).map(|i| Hasher::new(mode(i), None)).collect();
            // Ordinary and grouped updates can alternate on the same state.
            for (h, bytes) in hashers.iter_mut().zip(&data) {
                h.update(&bytes[..bytes.len().min(3)]);
            }
            for from in (3..1024).step_by(17) {
                let pieces: Vec<_> = data.iter().enumerate().map(|(i, bytes)| (i, &bytes[from.min(bytes.len())..(from + 17).min(bytes.len())])).collect();
                crate::each::update_each(&mut hashers, &pieces);
            }
            let which: Vec<_> = (0..count).rev().collect();
            let wanted: Vec<_> = which.iter().map(|&i| crate::test::official::hash(mode(i), &data[i])).collect();
            let mut out = vec![[0u8; OUT_LEN]; count];
            // Grouped and ordinary finishing agree, and the states hold
            // their bytes until finished (a clone finished alone).
            for (i, h) in hashers.iter().enumerate() {
                assert_eq!(h.count(), data[i].len() as u64);
                assert_eq!(h.clone().finalize().as_bytes(), &crate::test::official::hash(mode(i), &data[i]));
            }
            crate::each::finalize_each(&mut hashers, &which, &mut out);
            assert_eq!(out, wanted);
            // finalize_each reset them: each is the empty message again.
            crate::each::finalize_each(&mut hashers, &which, &mut out);
            for (slot, &i) in which.iter().enumerate() {
                assert_eq!(out[slot], crate::test::official::hash(mode(i), &[]), "reset after finishing, message {i}");
            }
            // The same messages again, continued across the chunk and
            // 16-KiB staging boundaries before finishing.
            for from in (0..1024).step_by(17) {
                let pieces: Vec<_> = data.iter().enumerate().map(|(i, bytes)| (i, &bytes[from.min(bytes.len())..(from + 17).min(bytes.len())])).collect();
                crate::each::update_each(&mut hashers, &pieces);
            }
            let suffix = vec![0xacu8; 17003];
            let pieces: Vec<_> = (0..count).map(|i| (i, suffix.as_slice())).collect();
            crate::each::update_each(&mut hashers, &pieces);
            crate::each::finalize_each(&mut hashers, &which, &mut out);
            for (slot, &i) in which.iter().enumerate() {
                let mut whole = data[i].clone();
                whole.extend_from_slice(&suffix);
                assert_eq!(out[slot], crate::test::official::hash(mode(i), &whole));
            }
        }
    }

    #[test]
    fn independently_ending_staged_prefixes_at_every_position() {
        crate::initialize();
        for rotation in 0..16 {
            for phase in [0usize, 1, 7, 31, 62, 63] {
                let mut hashers: Vec<_> = (0..16).map(|_| Hasher::new(crate::Mode::Plain, None)).collect();
                let data: Vec<Vec<u8>> = (0..16)
                    .map(|i| {
                        let len = if i == 0 && phase == 0 { 0 } else { 64 * ((i + rotation) % 16) + 1 + (i + phase) % 64 };
                        (0..len).map(|j| (i * 73 + j * 11) as u8).collect()
                    })
                    .collect();
                for start in (0..1024).step_by(17) {
                    let pieces: Vec<_> = data.iter().enumerate().map(|(i, bytes)| (i, &bytes[start.min(bytes.len())..(start + 17).min(bytes.len())])).collect();
                    crate::each::update_each(&mut hashers, &pieces);
                }
                let which: Vec<_> = (0..16).rev().collect();
                let mut out = [[0u8; OUT_LEN]; 16];
                crate::each::finalize_each(&mut hashers, &which, &mut out);
                for (slot, &i) in which.iter().enumerate() {
                    assert_eq!(out[slot], crate::test::official::hash(crate::Mode::Plain, &data[i]), "rotation={rotation} phase={phase} lane={i}");
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
        crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
        crate::each::finalize_each(&mut hashers, &which, &mut out);
        assert_eq!(out, expected);
        assert!(!crate::RAN_ENTRIES.with(|r| r.borrow().contains(&(entry() as usize))));
    }
}

/// Probe: reuse the proved ARM message kernels for equal-length plain
/// first chunks. All preparation and padding belong to the call.
#[cfg(blake3_neon)]
pub(crate) fn arm_group(hashers: &mut [Hasher], which: &[usize], out: &mut [[u8; OUT_LEN]]) -> bool {
    let first = &hashers[which[0]];
    let len = first.staged;
    let key = crate::IV;
    let flags = first.core.chunk_state.flags;
    // The first experiment needs plain messages alone. Mode flags select
    // it; secret key values stay out of the dispatcher's comparisons.
    if flags != 0
        || len == 0
        || len > CHUNK_LEN
        || which.iter().any(|&i| {
            let h = &hashers[i];
            h.staged != len || h.core.initial_chunk_counter != 0 || h.core.count() != 0 || h.core.chunk_state.flags != flags
        })
    {
        return false;
    }
    let blocks = len.div_ceil(BLOCK_LEN);
    let last_len = len - (blocks - 1) * BLOCK_LEN;
    let mut inputs = arrayvec::ArrayVec::<*const u8, { crate::many::TABLE }>::new();
    // One raw base keeps the stage pointers valid as later elements are
    // borrowed for padding. The eligibility reads above checked each index.
    let base = hashers.as_mut_ptr();
    for &i in which {
        let h = unsafe { &mut *base.add(i) };
        h.stage[len..blocks * BLOCK_LEN].fill(core::mem::MaybeUninit::new(0));
        inputs.push(h.stage.as_ptr().cast());
    }
    #[cfg(blake3_sme2)]
    {
        let sized = which.len() >= 16 && which.len() * len >= crate::SME2_SIZED_LEN;
        let turn = crate::platform::Sme2Turn::take(crate::platform::Platform::detect(), sized);
        if sized && matches!(turn.platform(), crate::platform::Platform::SME2) {
            while inputs.len() % 16 != 0 {
                inputs.push(*inputs.last().unwrap());
            }
            macro_rules! run {
                ($n:expr) => {{
                    // Sound: every slot has $n initialized bytes; repeated
                    // tail lanes borrow the last valid slot. Detection and
                    // the turn supply SME2 with sixteen streaming lanes.
                    let refs: arrayvec::ArrayVec<_, { crate::many::TABLE }> = inputs.iter().map(|&p| unsafe { &*p.cast::<[u8; $n]>() }).collect();
                    unsafe { crate::sme2::hash_messages(&refs, &key, flags, CHUNK_START, CHUNK_END | ROOT, last_len, out.as_flattened_mut()) };
                }};
            }
            match blocks {
                2 => run!(128),
                3 => run!(192),
                4 => run!(256),
                5 => run!(320),
                6 => run!(384),
                7 => run!(448),
                8 => run!(512),
                9 => run!(576),
                10 => run!(640),
                11 => run!(704),
                12 => run!(768),
                13 => run!(832),
                14 => run!(896),
                15 => run!(960),
                16 => run!(1024),
                _ => unreachable!("an SME-sized group has at least two blocks per message"),
            }
            return true;
        }
    }
    for (inputs, out) in inputs.chunks(16).zip(out.chunks_mut(16)) {
        // Sound: NEON exists on this target. Every prefix holds `blocks`
        // initialized blocks, padded above; key/flags/length agree per group.
        unsafe {
            crate::neon_hybrid::hash_messages_raw(
                inputs.as_ptr(),
                inputs.len(),
                blocks,
                &key,
                0,
                flags,
                CHUNK_START,
                CHUNK_END | ROOT,
                last_len,
                out.as_flattened_mut(),
            );
        }
    }
    true
}

#[cfg(all(test, blake3_neon))]
mod arm_tests {
    use super::*;

    #[test]
    fn grouped_first_chunks_match_the_reference_and_reset() {
        crate::initialize();
        for n in [2, 16, 17, 64, 128] {
            for len in [0, 1, 63, 64, 65, 255, 1023, 1024, 1025, 2048, 16385] {
                for width in [17, 256] {
                    let key = [37; 32];
                    for mode in [crate::Mode::Plain, crate::Mode::Keyed(&key), crate::Mode::DeriveKey("ARM grouped first chunk")] {
                        let data: Vec<Vec<u8>> = (0..n).map(|i| (0..len).map(|j| (i * 37 + j * 13) as u8).collect()).collect();
                        let mut hashers: Vec<_> = (0..n).map(|_| Hasher::new(mode, None)).collect();
                        for start in (0..len).step_by(width) {
                            let pieces: Vec<_> = data.iter().enumerate().map(|(i, d)| (i, &d[start..(start + width).min(len)])).collect();
                            crate::each::update_each(&mut hashers, &pieces);
                        }
                        assert!(hashers.iter().all(|h| h.count() == len as u64));
                        let which: Vec<_> = (0..n).rev().collect();
                        let mut out = vec![[0; 32]; n];
                        crate::each::finalize_each(&mut hashers, &which, &mut out);
                        for (&i, got) in which.iter().zip(&out) {
                            assert_eq!(*got, crate::test::official::hash(mode, &data[i]), "n={n}, len={len}, width={width}");
                        }
                        assert!(hashers.iter().all(|h| h.count() == 0));
                        crate::each::finalize_each(&mut hashers, &which, &mut out);
                        assert!(out.iter().all(|got| *got == crate::test::official::hash(mode, &[])));
                    }
                }
            }
        }
    }
}
