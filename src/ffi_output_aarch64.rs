//! Eight-lane NEON independent counter-zero compressions, plus multi-group
//! keyed SME2 chunk hashing, on 64-bit Arm (#336). Descriptors borrow blocks
//! and CVs; keys, lengths and flags may differ between lanes. Key values never
//! select grouping, allocation, or dispatch.
#[cfg(test)]
use crate::hasher;
use crate::{BLOCK_LEN, CHUNK_END, CHUNK_LEN, CHUNK_START, Hasher, OUT_LEN, ROOT, platform::Platform};

#[derive(Clone, Copy)]
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

unsafe extern "C" {
    fn blake3_servil_neon_output8(inputs: *const Input<'_>, out: *mut u8, groups: u64);
}
#[cfg(blake3_sme2)]
unsafe extern "C" {
    fn blake3_servil_sme2_chunks_keyed(
        chunks: *const *const u8,
        counters: *const u64,
        keys: *const [u32; 8],
        flags: *const u8,
        out: *mut u8,
        groups: u64,
    ) -> u64;
}

#[cfg(test)]
pub(crate) fn lanes(platform: Platform) -> usize {
    match platform {
        #[cfg(blake3_sme2)]
        Platform::SME2 => 16,
        Platform::NEON => 8,
    }
}

/// A whole message whose root is one compression away: still in its first
/// chunk, with no staged suffix or stacked subtree, its final block already
/// padded by ChunkState; or a block or less waiting in the stage, padded
/// there (`Stage::first_block`). Other states keep Hasher's existing path.
pub(crate) fn input(hasher: &Hasher) -> Option<Input<'_>> {
    let c = hasher.chunk_state();
    if c.chunk_counter() != 0 {
        return None;
    }
    let staged = hasher.staged_len();
    if staged == 0 {
        let (block, cv, len, flags) = c.output_parts();
        return Some(Input { block, cv, len, flags: flags + ROOT });
    }
    const WHOLE_ROOT_FLAGS: u8 = CHUNK_START + CHUNK_END + ROOT;
    (staged <= BLOCK_LEN && c.count() == 0).then(|| Input {
        block: hasher.stage_first_block(),
        cv: c.cv(),
        len: staged.to_le_bytes()[0],
        flags: hasher.domain().flags() + WHOLE_ROOT_FLAGS,
    })
}

/// Hash all full 8-lane NEON groups of independent counter-zero compressions
/// on AArch64, returning the number of digests written to `out`.
///
/// On Apple M6, NEON `output8` runs at ~27 ns/compression with zero SME
/// transition overhead, beating a 16-lane SME2 single-block kernel even at
/// 128 lanes (~38 ns/compression) while leaving the shared SME unit free.
pub(crate) fn hash_groups_arm(inputs: &[Input<'_>], out: &mut [[u8; OUT_LEN]]) -> usize {
    assert_eq!(inputs.len(), out.len(), "one output per descriptor");
    let groups = inputs.len() / 8;
    if groups > 0 {
        crate::ran_kernel(blake3_servil_neon_output8 as *const ());
        // Sound: `blake3_neon` guarantees AArch64 NEON; `inputs` holds
        // `8 * groups` valid descriptors and `out` has exact room.
        unsafe { blake3_servil_neon_output8(inputs.as_ptr(), out.as_mut_ptr().cast(), groups as u64) };
    }
    groups * 8
}

/// Hash complete held stages without grouping by their keys.
///
/// Small turns (`<= 4` stages) use `HeldWorkspace<4>` (3 KiB) so worker
/// threads with small stacks (e.g. 24 KiB on AArch64) never overflow; larger
/// turns process in 16-stage L1D-hot blocks (`HeldWorkspace<16>`, 12 KiB).
/// Holds `Sme2Turn` only during leaf hashing so the shared SME unit is free
/// while NEON `output8` reduces the 4 parent levels.
pub(crate) fn held_groups(hashers: &mut [Hasher], held: &[usize], platform: Platform) {
    if held.len() <= 4 {
        held_groups_in::<4, 32>(hashers, held, platform);
    } else {
        for chunk in held.chunks(16) {
            held_groups_in::<16, 128>(hashers, chunk, platform);
        }
    }
}

#[inline(never)]
fn held_groups_in<const S: usize, const MAX_PARENTS: usize>(hashers: &mut [Hasher], held: &[usize], platform: Platform) {
    const PER: usize = crate::STAGE_LEN / CHUNK_LEN;
    const { assert!(PER == 16 && MAX_PARENTS == S * (PER / 2)) };
    let n = held.len();
    let mut work = crate::each::HeldWorkspace::<S>::new();
    let (mut level, mut next) = work.buffers(n);

    // 1. Leaf chunks: on SME2, one multi-group kernel entry hashes all `n`
    //    keyed/mixed stages together with zero key comparisons; on NEON (or if
    //    SME2 is busy / declined on nonzero TPIDR2_EL0 reserved bytes), each
    //    stage runs at full 16-chunk NEON width.
    #[cfg(blake3_sme2)]
    let leaves_done = {
        let turn = crate::platform::Sme2Turn::take(platform, true);
        if matches!(turn.platform(), Platform::SME2) {
            let mut chunks = crate::stack_vec::StackVec::<[*const u8; PER], S>::new();
            let mut counters = crate::stack_vec::StackVec::<u64, S>::new();
            let mut keys = crate::stack_vec::StackVec::<[u32; 8], S>::new();
            let mut flags = crate::stack_vec::StackVec::<u8, S>::new();
            for &i in held {
                let h = &hashers[i];
                let (counter, bytes) = h.held_group().expect("a held group");
                let domain = h.domain();
                chunks.push(core::array::from_fn(|j| bytes[j * CHUNK_LEN..].as_ptr()));
                counters.push(counter);
                keys.push(*domain.key());
                flags.push(domain.flags());
            }
            // Sound: `Platform::SME2` verified SME2 support; `chunks` holds
            // `16 * n` pointers to readable 1024-byte chunks, `counters`/`keys`/
            // `flags` have length `n`, and `level` has room for `16 * n` CVs.
            let svl = unsafe {
                blake3_servil_sme2_chunks_keyed(
                    chunks.as_slice().as_flattened().as_ptr(),
                    counters.as_slice().as_ptr(),
                    keys.as_slice().as_ptr(),
                    flags.as_slice().as_ptr(),
                    level.as_flattened_mut().as_mut_ptr(),
                    n as u64,
                )
            };
            if svl == 16 {
                crate::ran_kernel(blake3_servil_sme2_chunks_keyed as *const ());
                true
            } else {
                false
            }
        } else {
            false
        }
    };
    #[cfg(not(blake3_sme2))]
    let leaves_done = {
        let _ = platform;
        false
    };
    if !leaves_done {
        for (k, &i) in held.iter().enumerate() {
            let h = &hashers[i];
            let (counter, bytes) = h.held_group().expect("a held group");
            let refs: [&[u8; CHUNK_LEN]; PER] = core::array::from_fn(|j| bytes[j * CHUNK_LEN..(j + 1) * CHUNK_LEN].try_into().unwrap());
            let domain = h.domain();
            Platform::NEON.hash_many(
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
    }

    // 2. Four parent levels across all `n` stages (<= 128 parents per level) on NEON output8.
    for depth in 0..4 {
        let per_message = PER >> (depth + 1);
        let count = n * per_message;
        let mut inputs = crate::stack_vec::StackVec::<Input<'_>, MAX_PARENTS>::new();
        for j in 0..count {
            let h = &hashers[held[j / per_message]];
            let domain = h.domain();
            inputs.push(Input {
                block: level.as_flattened()[j * BLOCK_LEN..(j + 1) * BLOCK_LEN].try_into().unwrap(),
                cv: domain.key(),
                len: u8::try_from(BLOCK_LEN).unwrap(),
                flags: domain.flags() + crate::PARENT,
            });
        }
        let done = hash_groups_arm(inputs.as_slice(), &mut next[..count]);
        for j in done..count {
            let inp = inputs.as_slice()[j];
            let mut cv = *inp.cv;
            Platform::NEON.compress_in_place(&mut cv, inp.block, inp.len, 0, inp.flags);
            next[j] = crate::hasher::le_bytes(&cv);
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
#[cfg(blake3_sme2)]
const CHUNKS_KEYED_SELF_TEST_EXPECTED: u64 = 0x199d_615f_c41e_830b;

/// Planted corruption selector for subprocess fail-stop tests:
/// 1 = corrupt last output byte of NEON `output8`,
/// 2 = corrupt last output byte of SME2 `chunks_keyed`.
#[cfg(test)]
pub(crate) static PLANTED_WRONG_OUTPUT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

fn self_test_shape(mut hash: impl FnMut(&[Input<'_>; 16], &mut [[u8; OUT_LEN]])) -> u64 {
    let mut lens: [u8; 16] = [0, 1, 31, 63, 64, 7, 32, 55, 3, 15, 48, 60, 2, 33, 62, 64];
    let flags: [u8; 16] = core::array::from_fn(|i| {
        let shape = [3u8, 2, 4][i % 3];
        if shape == 4 {
            lens[i] = 64;
        }
        shape + [0u8, 16, 32, 64][i % 4]
    });
    let cvs: [[u32; 8]; 16] = core::array::from_fn(|i| {
        let i = u32::try_from(i).unwrap();
        core::array::from_fn(|w| {
            let w = u32::try_from(w).unwrap();
            (0x1234abcdu32.wrapping_add(i * 0x9e37).wrapping_add(w * 0x1020304)).wrapping_mul(0x85ebca6b)
        })
    });
    let blocks: [[u8; 64]; 16] =
        core::array::from_fn(|i| core::array::from_fn(|j| if j < lens[i] as usize { ((i * 37 + j * 17) ^ 0x5a).to_le_bytes()[0] } else { 0 }));
    let inputs = core::array::from_fn(|i| Input { block: &blocks[i], cv: &cvs[i], len: lens[i], flags: flags[i] + if (i / 4) % 2 == 0 { ROOT } else { 0 } });
    let mut out = [[0u8; OUT_LEN]; 16];
    hash(&inputs, &mut out);
    crate::self_test::fold(out.as_flattened())
}

#[cfg(blake3_sme2)]
#[inline(never)]
fn chunks_keyed_self_test_shape(mut hash: impl FnMut(&[[u8; CHUNK_LEN]; 32], &[u64; 2], &[[u32; 8]; 2], &[u8; 2], &mut [[u8; OUT_LEN]; 32])) -> u64 {
    // Initialize the 32 KiB input table in place (avoiding `core::array::from_fn`'s
    // duplicate 32 KiB temporary on the stack) so startup fits easily in a 128 KiB
    // thread stack (#509).
    let mut chunks = [[0u8; CHUNK_LEN]; 32];
    for (c, chunk) in chunks.iter_mut().enumerate() {
        for (b, byte) in chunk.iter_mut().enumerate() {
            *byte = ((c * 37 + b * 17 + (b >> 6)) ^ 0x5a).to_le_bytes()[0];
        }
    }
    let counters = [u64::from(u32::MAX) - 5, 0x0000_0003_0000_0007];
    let keys: [[u32; 8]; 2] = core::array::from_fn(|g| {
        let g = u32::try_from(g).unwrap();
        core::array::from_fn(|w| {
            let w = u32::try_from(w).unwrap();
            (0x2468_ace0u32.wrapping_add(g * 0x9e37_79b9).wrapping_add(w * 0x0102_0304)).wrapping_mul(0x85eb_ca6b)
        })
    });
    let flags = [crate::KEYED_HASH, crate::DERIVE_KEY_MATERIAL];
    let mut out = [[0u8; OUT_LEN]; 32];
    hash(&chunks, &counters, &keys, &flags, &mut out);
    crate::self_test::fold(out.as_flattened())
}

#[inline(never)]
pub(crate) fn self_test(platform: Platform) -> usize {
    let neon_answer = self_test_shape(|inputs, out| {
        crate::ran_kernel(blake3_servil_neon_output8 as *const ());
        // Sound: `blake3_neon` guarantees AArch64 NEON; `inputs` and `out` hold 16 entries (2 groups of 8).
        unsafe { blake3_servil_neon_output8(inputs.as_ptr(), out.as_mut_ptr().cast(), 2) };
        #[cfg(test)]
        if PLANTED_WRONG_OUTPUT.load(core::sync::atomic::Ordering::Relaxed) == 1 {
            out[15][OUT_LEN - 1] = !out[15][OUT_LEN - 1];
        }
    });
    if neon_answer != SELF_TEST_EXPECTED {
        crate::self_test::fail(format_args!("blake3-servil startup self-test failed: independent counter-zero compressions"));
    }

    #[cfg(blake3_sme2)]
    if matches!(platform, Platform::SME2) {
        let chunks_answer = chunks_keyed_self_test_shape(|chunks, counters, keys, flags, out| {
            let ptrs: [*const u8; 32] = core::array::from_fn(|i| chunks[i].as_ptr());
            // Sound: `Platform::SME2` verified SME2; 32 readable chunks, 2 groups, exact output room.
            let svl = unsafe { blake3_servil_sme2_chunks_keyed(ptrs.as_ptr(), counters.as_ptr(), keys.as_ptr(), flags.as_ptr(), out.as_mut_ptr().cast(), 2) };
            assert_eq!(svl, 16, "SME2 chunks_keyed self-test declined");
            crate::ran_kernel(blake3_servil_sme2_chunks_keyed as *const ());
            #[cfg(test)]
            if PLANTED_WRONG_OUTPUT.load(core::sync::atomic::Ordering::Relaxed) == 2 {
                out[31][OUT_LEN - 1] = !out[31][OUT_LEN - 1];
            }
        });
        if chunks_answer != CHUNKS_KEYED_SELF_TEST_EXPECTED {
            crate::self_test::fail(format_args!("blake3-servil startup self-test failed: SME2 keyed chunks"));
        }
        return 2;
    }
    #[cfg(not(blake3_sme2))]
    let _ = platform;
    1
}

#[cfg(test)]
pub(crate) fn entry() -> *const () {
    blake3_servil_neon_output8 as *const ()
}
#[cfg(test)]
pub(crate) fn entry8() -> *const () {
    blake3_servil_neon_output8 as *const ()
}
#[cfg(test)]
pub(crate) fn entries_neon() -> [(&'static str, *const ()); 1] {
    [("blake3_servil_neon_output8", blake3_servil_neon_output8 as *const ())]
}
#[cfg(all(test, blake3_sme2))]
pub(crate) fn entry_chunks_keyed() -> *const () {
    blake3_servil_sme2_chunks_keyed as *const ()
}
#[cfg(all(test, blake3_sme2))]
pub(crate) fn entries_sme2() -> [(&'static str, *const ()); 1] {
    [("blake3_servil_sme2_chunks_keyed", blake3_servil_sme2_chunks_keyed as *const ())]
}
#[cfg(all(test, blake3_sme2))]
pub(crate) unsafe fn call_sme2_chunks_keyed(
    chunks: *const *const u8,
    counters: *const u64,
    keys: *const [u32; 8],
    flags: *const u8,
    out: *mut u8,
    groups: u64,
) -> u64 {
    // Sound: test helper forwards caller's SME2 & buffer contract to `blake3_servil_sme2_chunks_keyed`.
    unsafe { blake3_servil_sme2_chunks_keyed(chunks, counters, keys, flags, out, groups) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_answers_match_independent_compression() {
        crate::initialize();
        let platform = Platform::detect();
        let answer = self_test_shape(|inputs, out| {
            for (input, digest) in inputs.iter().zip(out) {
                let mut cv = *input.cv;
                platform.compress_in_place(&mut cv, input.block, input.len, 0, input.flags);
                *digest = hasher::le_bytes(&cv);
            }
        });
        assert_eq!(answer, SELF_TEST_EXPECTED);
        assert_eq!(
            self_test_shape(|inputs, out| {
                crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
                assert_eq!(hash_groups_arm(&inputs[..7], &mut out[..7]), 0);
                assert!(!crate::RAN_ENTRIES.with(|r| r.borrow().contains(&(entry8() as usize))), "fewer than 8 inputs must not invoke NEON output8");
                assert_eq!(hash_groups_arm(inputs, out), 16);
                assert!(crate::RAN_ENTRIES.with(|r| r.borrow().contains(&(entry8() as usize))));
            }),
            answer
        );
        #[cfg(blake3_sme2)]
        {
            let ref_chunks = chunks_keyed_self_test_shape(|chunks, counters, keys, flags, out| {
                for g in 0..2 {
                    for lane in 0..16 {
                        let idx = g * 16 + lane;
                        let mut cv = keys[g];
                        let ctr = counters[g] + lane as u64;
                        for blk in 0..16 {
                            let f = flags[g]
                                | if blk == 0 {
                                    CHUNK_START
                                } else if blk == 15 {
                                    CHUNK_END
                                } else {
                                    0
                                };
                            let b: &[u8; 64] = chunks[idx][blk * 64..(blk + 1) * 64].try_into().unwrap();
                            platform.compress_in_place(&mut cv, b, 64, ctr, f);
                        }
                        out[idx] = hasher::le_bytes(&cv);
                    }
                }
            });
            assert_eq!(ref_chunks, CHUNKS_KEYED_SELF_TEST_EXPECTED);
        }
    }

    #[test]
    fn independent_blocks_cvs_descriptors_and_output_against_guard_pages() {
        use crate::test::guard::Guarded;
        crate::initialize();
        for at_end in [false, true] {
            let answer = self_test_shape(|inputs, out| {
                let mut blocks: Vec<_> = (0..16).map(|_| Guarded::new(64, at_end)).collect();
                let mut cvs: Vec<_> = (0..16).map(|_| Guarded::new(32, at_end)).collect();
                let mut table = Guarded::new(core::mem::size_of_val(inputs), at_end);
                let table_ptr = table.bytes().as_mut_ptr().cast::<Input<'_>>();
                for i in 0..16 {
                    blocks[i].bytes().copy_from_slice(inputs[i].block);
                    cvs[i].bytes().copy_from_slice(&hasher::le_bytes(inputs[i].cv));
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
                assert_eq!(hash_groups_arm(descriptors, guarded_out), 16);
                out.copy_from_slice(guarded_out);
            });
            assert_eq!(answer, SELF_TEST_EXPECTED, "guard placement at_end={at_end}");
        }
    }

    #[test]
    fn startup_covers_each_supported_platform() {
        crate::initialize();
        for platform in crate::test::every_platform() {
            let expected_width = match platform {
                #[cfg(blake3_sme2)]
                Platform::SME2 => 16,
                Platform::NEON => 8,
            };
            assert_eq!(lanes(platform), expected_width, "lanes({platform:?})");
            crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
            let checked = self_test(platform);
            let sme2 = expected_width == 16;
            assert_eq!(checked, if sme2 { 2 } else { 1 }, "startup families on {platform:?}");
            let entries = crate::RAN_ENTRIES.with(|r| r.borrow().clone());
            assert!(entries.contains(&(entry8() as usize)), "eight-lane NEON startup on {platform:?}");
            #[cfg(blake3_sme2)]
            assert_eq!(entries.contains(&(entry_chunks_keyed() as usize)), sme2, "SME2 chunks_keyed startup on {platform:?}");
        }
    }

    #[test]
    fn self_test_aborts_on_wrong_output() {
        const ENV: &str = "BLAKE3_SERVIL_TEST_ARM_OUTPUT_FAIL";
        if let Ok(case) = std::env::var(ENV) {
            let platform = Platform::detect_unchecked();
            let code = match case.as_str() {
                "neon" => 1,
                "sme2" => 2,
                other => panic!("unexpected case {other}"),
            };
            PLANTED_WRONG_OUTPUT.store(code, core::sync::atomic::Ordering::Relaxed);
            self_test(platform);
            std::process::exit(0);
        }
        let exe = std::env::current_exe().expect("test binary path");
        let mut cases: Vec<(&str, &str)> = vec![("neon", "independent counter-zero compressions")];
        #[cfg(blake3_sme2)]
        if matches!(Platform::detect_unchecked(), Platform::SME2) {
            cases.push(("sme2", "SME2 keyed chunks"));
        }
        for (case, want_msg) in cases {
            let out = std::process::Command::new(&exe)
                .arg("--exact")
                .arg("ffi_output::tests::self_test_aborts_on_wrong_output")
                .arg("--nocapture")
                .env(ENV, case)
                .output()
                .expect("spawn fail-stop child");
            assert!(!out.status.success(), "case {case} must fail-stop, got {:?}", out.status);
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(stderr.contains(want_msg), "case {case} stderr missing {want_msg:?}: {stderr}");
        }
    }

    #[test]
    fn incremental_roots_batch_all_modes_and_selected_indices() {
        crate::initialize();
        for count in [0usize, 1, 7, 8, 15, 16, 17, 31, 32, 65, 240] {
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
            let keyed_short = (0..count).filter(|&i| i % 3 != 0 && [0usize, 1, 31, 63, 64, 65, 127, 255, 511, 1023, 1024][i % 11] <= BLOCK_LEN).count();
            let ran = crate::RAN_ENTRIES.with(|r| r.borrow().clone());
            assert_eq!(ran.contains(&(entry8() as usize)), keyed_short >= 8, "NEON output8 for {keyed_short} short keyed hashers");
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
        // Exact boundary of the 8-lane NEON threshold: 7 stays scalar, 8 runs output8.
        for n in [7usize, 8] {
            let mut hashers: Vec<_> = (0..n).map(|i| Hasher::new(crate::Mode::Keyed(&[i as u8; 32]), None)).collect();
            let data: Vec<[u8; 32]> = (0..n).map(|i| [i as u8; 32]).collect();
            for (i, h) in hashers.iter_mut().enumerate() {
                h.update(&data[i]);
            }
            let which: Vec<usize> = (0..n).collect();
            let mut out = vec![[0u8; 32]; n];
            crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
            crate::each::finalize_each(&mut hashers, &which, &mut out);
            for (i, got) in out.iter().enumerate() {
                assert_eq!(*got, crate::test::official::hash(crate::Mode::Keyed(&[i as u8; 32]), &data[i]));
            }
            let ran = crate::RAN_ENTRIES.with(|r| r.borrow().contains(&(entry8() as usize)));
            assert_eq!(ran, n >= 8, "boundary n={n}");
        }
    }

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
        let which: Vec<usize> = (0..32).collect();
        let mut out = vec![[0u8; 32]; 32];
        crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
        crate::each::finalize_each(&mut hashers, &which, &mut out);
        assert_eq!(out, expected);
        assert!(!crate::RAN_ENTRIES.with(|r| r.borrow().contains(&(entry() as usize))));
    }

    /// Small keyed turns (`<= 4` stages) use `HeldWorkspace<4>` (3 KiB) so
    /// worker threads with compact stacks succeed without overflow (#336).
    #[test]
    fn small_stack_handles_three_and_four_keyed_stages() {
        crate::initialize();
        for n in [3usize, 4] {
            let handle = std::thread::Builder::new()
                .stack_size(24 * 1024)
                .spawn(move || {
                    let keys: Vec<[u8; 32]> = (0..n).map(|i| [i as u8 + 1; 32]).collect();
                    let mut hashers: Vec<_> = keys.iter().map(|k| Hasher::new(crate::Mode::Keyed(k), None)).collect();
                    let payload = vec![0x3Cu8; crate::STAGE_LEN];
                    let pieces: Vec<(usize, &[u8])> = (0..n).map(|i| (i, payload.as_slice())).collect();
                    crate::each::update_each(&mut hashers, &pieces);
                    let which: Vec<usize> = (0..n).collect();
                    let mut out = vec![[0u8; OUT_LEN]; n];
                    crate::each::finalize_each(&mut hashers, &which, &mut out);
                    for i in 0..n {
                        assert_eq!(out[i], crate::test::official::hash(crate::Mode::Keyed(&keys[i]), &payload));
                    }
                })
                .expect("spawn 24 KiB stack thread");
            handle.join().expect("24 KiB stack thread completed without overflow");
        }
    }
}
