//! Direct-load SIMD kernels for contiguous `Packed<16>`, `Packed<32>`, and
//! `Packed<48>` batches across AVX-512, AVX2, Apple/AArch64 SME2, and AArch64
//! NEON `8+3` (#501).
//!
//! Each backend hashes whole groups of contiguous packed messages directly
//! from `input` into `outputs` with zero 8 KiB stack expansion and zero
//! zero-word message additions, returning how many messages were hashed.

use crate::{CHUNK_END, CHUNK_START, CVWords, OUT_LEN, ROOT, platform::Platform};

/// Hash full SIMD groups of `len`-byte packed messages (`len` in
/// `{16, 32, 48}`) on `platform`, writing one 32-byte digest per message to
/// `outputs` and returning the number of messages hashed. On x86-64 AVX-512,
/// a `10..=15`-message remainder after full 16-lane groups is left for
/// `one_block_groups` so it finishes in one padded `output16` call.
///
/// Requires `input.len() == len * outputs.len()`.
pub(crate) fn hash_packed_groups(input: &[u8], len: usize, key: &CVWords, flags: u8, outputs: &mut [[u8; OUT_LEN]], platform: Platform) -> usize {
    assert!(matches!(len, 16 | 32 | 48), "packed messages are 16, 32, or 48 bytes, not {len}");
    assert_eq!(input.len(), len.checked_mul(outputs.len()).expect("packed input size overflow"), "packed input length must equal len * outputs.len()");
    let root_flags = u32::from(flags | CHUNK_START | CHUNK_END | ROOT);
    let mut done = 0usize;

    #[cfg(target_arch = "aarch64")]
    if matches!(platform, Platform::SME2 { .. }) {
        let groups = outputs.len() / 16;
        if groups > 0 {
            let kernel: unsafe extern "C" fn(*const u8, *const u32, u32, *mut u8, u64) -> u64 = match len {
                16 => ffi_sme2::blake3_servil_sme2_packed16,
                32 => ffi_sme2::blake3_servil_sme2_packed32,
                48 => ffi_sme2::blake3_servil_sme2_packed48,
                _ => unreachable!("asserted len in {{16, 32, 48}}"),
            };
            // Sound: `Platform::SME2` verified SME2 support; `input` holds at
            // least `16 * groups * len` bytes and `outputs` holds at least
            // `16 * groups` 32-byte slots. The kernel checks SVL == 16 via
            // `rdsvl` before touching ZA/streaming state and returns the live
            // SVL; only when it returns 16 were the outputs written.
            let svl = unsafe { kernel(input.as_ptr(), key.as_ptr(), root_flags, outputs.as_mut_ptr().cast(), groups as u64) };
            if svl == 16 {
                crate::ran_kernel(kernel as *const ());
                done += groups * 16;
            }
        }
        // Any remaining messages (either SVL != 16 on Linux, < 16 messages,
        // or the 11..15 tail after full SME2 groups) fall through to NEON 8+3.
    }

    #[cfg(target_arch = "aarch64")]
    if matches!(platform, Platform::NEON { .. } | Platform::SME2 { .. }) {
        let rem = outputs.len() - done;
        let groups = rem / 11;
        if groups > 0 {
            let kernel: unsafe extern "C" fn(*const u8, u64, *const u32, u32, *mut u8) = match len {
                16 => ffi_neon::blake3_servil_neon_p11_packed16,
                32 => ffi_neon::blake3_servil_neon_p11_packed32,
                48 => ffi_neon::blake3_servil_neon_p11_packed48,
                _ => unreachable!("asserted len in {{16, 32, 48}}"),
            };
            crate::ran_kernel(kernel as *const ());
            // Sound: AArch64 NEON is mandatory on `Platform::NEON`/`SME2`;
            // `input[done * len..]` has at least `11 * groups * len` readable
            // bytes and `outputs[done..]` has at least `11 * groups` writable
            // 32-byte slots.
            unsafe {
                kernel(input[done * len..].as_ptr(), groups as u64, key.as_ptr(), root_flags, outputs[done..].as_mut_ptr().cast());
            }
            done += groups * 11;
        }
        return done;
    }

    #[cfg(target_arch = "x86_64")]
    if matches!(platform, Platform::AVX512 { .. }) {
        let groups = outputs.len() / 16;
        if groups > 0 {
            let kernel: unsafe extern "C" fn(*const u8, u64, *const u32, u32, *mut u8) = match len {
                16 => ffi_x86::blake3_servil_avx512_packed16,
                32 => ffi_x86::blake3_servil_avx512_packed32,
                48 => ffi_x86::blake3_servil_avx512_packed48,
                _ => unreachable!("asserted len in {{16, 32, 48}}"),
            };
            crate::ran_kernel(kernel as *const ());
            // Sound: `Platform::AVX512` verified AVX-512F+VL; `input` holds at
            // least `16 * groups * len` readable bytes and `outputs` holds at
            // least `16 * groups` writable 32-byte slots.
            unsafe {
                kernel(input.as_ptr(), groups as u64, key.as_ptr(), root_flags, outputs.as_mut_ptr().cast());
            }
            done += groups * 16;
        }
    }

    #[cfg(target_arch = "x86_64")]
    if matches!(platform, Platform::AVX2 { .. }) || {
        // On AVX-512, a 10..=15 remainder is faster as a single padded 16-lane
        // `output16` call than `avx2_packed` (8) + a second kernel call for
        // the remaining 2..=7 messages; keep AVX2 for `8..=9`.
        let rem = outputs.len() - done;
        let prefer_output16 = (10..16).contains(&rem);
        matches!(platform, Platform::AVX512 { .. }) && !prefer_output16
    } {
        let rem = outputs.len() - done;
        let groups = rem / 8;
        if groups > 0 {
            let kernel: unsafe extern "C" fn(*const u8, u64, *const u32, u32, *mut u8) = match len {
                16 => ffi_x86::blake3_servil_avx2_packed16,
                32 => ffi_x86::blake3_servil_avx2_packed32,
                48 => ffi_x86::blake3_servil_avx2_packed48,
                _ => unreachable!("asserted len in {{16, 32, 48}}"),
            };
            crate::ran_kernel(kernel as *const ());
            // Sound: `Platform::AVX2` and `Platform::AVX512` guarantee AVX2;
            // `input[done * len..]` holds at least `8 * groups * len` readable
            // bytes and `outputs[done..]` holds at least `8 * groups` writable
            // 32-byte slots.
            unsafe {
                kernel(input[done * len..].as_ptr(), groups as u64, key.as_ptr(), root_flags, outputs[done..].as_mut_ptr().cast());
            }
            done += groups * 8;
        }
    }

    done
}

const SELF_TEST_COUNT: usize = 43;
const SELF_TEST_KEY: [u8; 32] = [
    0x13, 0x27, 0x3b, 0x4f, 0x63, 0x77, 0x8b, 0x9f, 0xb3, 0xc7, 0xdb, 0xef, 0x05, 0x19, 0x2d, 0x41, 0x55, 0x69, 0x7d, 0x91, 0xa5, 0xb9, 0xcd, 0xe1, 0xf5, 0x0a,
    0x1e, 0x32, 0x46, 0x5a, 0x6e, 0x82,
];

fn fill_self_test_input(buf: &mut [u8], len: usize) {
    for (i, slot) in buf.chunks_exact_mut(len).enumerate() {
        for (j, b) in slot.iter_mut().enumerate() {
            *b = crate::low_byte((i.wrapping_mul(53).wrapping_add(j.wrapping_mul(19))) ^ 0xa5);
        }
    }
}

/// Folds of `43` keyed packed digests (`16`, `32`, `48` B) from official BLAKE3 1.8.7.
/// `43 = 2*16 + 11 = 5*8 + 3 = 3*11 + 10`, exercising multi-group pipelining and
/// remainder dispatch across SME2, AVX-512, AVX2, and NEON `8+3` (on x86-64
/// AVX-512, `self_test` additionally checks an 8-message batch on `avx2_packed`).
const SELF_TEST_EXPECTED: [u64; 3] = [0x13f1c329fc53200f, 0x1e4294615a858573, 0x93901e850f4221b8];

#[inline(never)]
pub(crate) fn self_test(platform: Platform) -> usize {
    let key = crate::platform::words_from_le_bytes_32(&SELF_TEST_KEY);
    let mut input = [0u8; SELF_TEST_COUNT * 48];
    let mut out = [[0u8; OUT_LEN]; SELF_TEST_COUNT];
    for (idx, len) in [16usize, 32, 48].into_iter().enumerate() {
        let slice = &mut input[..SELF_TEST_COUNT * len];
        fill_self_test_input(slice, len);
        crate::many::hash_many_on(slice, len, len, &key, crate::KEYED_HASH, &mut out, platform);
        let got = crate::self_test::fold(out.as_flattened());
        if got != SELF_TEST_EXPECTED[idx] {
            crate::self_test::fail(format_args!(
                "blake3-servil startup self-test failed: packed<{len}> (platform {}): \
                 expected {:#018x}, got {:#018x}",
                platform.name(),
                SELF_TEST_EXPECTED[idx],
                got,
            ));
        }
        // On AVX-512, 43 = 2*16 + 11 dispatches the 11-message tail through
        // padded `output16` instead of `avx2_packed`; also exercise an
        // 8-message packed batch so `avx2_packed{16,32,48}` is verified at startup.
        #[cfg(target_arch = "x86_64")]
        if matches!(platform, Platform::AVX512) {
            let mut out8 = [[0u8; OUT_LEN]; 8];
            crate::many::hash_many_on(&slice[..8 * len], len, len, &key, crate::KEYED_HASH, &mut out8, platform);
            if out8 != out[..8] {
                crate::self_test::fail(format_args!("blake3-servil startup self-test failed: avx2_packed<{len}> mismatch against avx512_packed<{len}>"));
            }
        }
    }
    3
}

/// Exported entry points of the packed kernels compiled for this target, for
/// `self_test::test_self_test_runs_every_kernel`.
#[cfg(all(test, target_arch = "aarch64"))]
pub(crate) fn entries_neon() -> [(&'static str, *const ()); 3] {
    [
        ("neon_p11_packed16", ffi_neon::blake3_servil_neon_p11_packed16 as *const ()),
        ("neon_p11_packed32", ffi_neon::blake3_servil_neon_p11_packed32 as *const ()),
        ("neon_p11_packed48", ffi_neon::blake3_servil_neon_p11_packed48 as *const ()),
    ]
}

#[cfg(all(test, target_arch = "aarch64"))]
pub(crate) fn entries_sme2() -> [(&'static str, *const ()); 3] {
    [
        ("sme2_packed16", ffi_sme2::blake3_servil_sme2_packed16 as *const ()),
        ("sme2_packed32", ffi_sme2::blake3_servil_sme2_packed32 as *const ()),
        ("sme2_packed48", ffi_sme2::blake3_servil_sme2_packed48 as *const ()),
    ]
}

#[cfg(all(test, target_arch = "x86_64"))]
pub(crate) fn entries_avx2() -> [(&'static str, *const ()); 3] {
    [
        ("avx2_packed16", ffi_x86::blake3_servil_avx2_packed16 as *const ()),
        ("avx2_packed32", ffi_x86::blake3_servil_avx2_packed32 as *const ()),
        ("avx2_packed48", ffi_x86::blake3_servil_avx2_packed48 as *const ()),
    ]
}

#[cfg(all(test, target_arch = "x86_64"))]
pub(crate) fn entries_avx512() -> [(&'static str, *const ()); 3] {
    [
        ("avx512_packed16", ffi_x86::blake3_servil_avx512_packed16 as *const ()),
        ("avx512_packed32", ffi_x86::blake3_servil_avx512_packed32 as *const ()),
        ("avx512_packed48", ffi_x86::blake3_servil_avx512_packed48 as *const ()),
    ]
}

#[cfg(target_arch = "x86_64")]
mod ffi_x86 {
    unsafe extern "C" {
        pub fn blake3_servil_avx512_packed16(input: *const u8, groups: u64, key: *const u32, flags: u32, out: *mut u8);
        pub fn blake3_servil_avx512_packed32(input: *const u8, groups: u64, key: *const u32, flags: u32, out: *mut u8);
        pub fn blake3_servil_avx512_packed48(input: *const u8, groups: u64, key: *const u32, flags: u32, out: *mut u8);
        pub fn blake3_servil_avx2_packed16(input: *const u8, groups: u64, key: *const u32, flags: u32, out: *mut u8);
        pub fn blake3_servil_avx2_packed32(input: *const u8, groups: u64, key: *const u32, flags: u32, out: *mut u8);
        pub fn blake3_servil_avx2_packed48(input: *const u8, groups: u64, key: *const u32, flags: u32, out: *mut u8);
    }
}

#[cfg(all(test, target_arch = "aarch64"))]
pub(crate) unsafe fn call_sme2_packed(len: usize, input: *const u8, key: *const u32, flags: u32, out: *mut u8, groups: u64) -> u64 {
    // Sound: caller verified SME2 support and provided buffers covering `16 * groups` packed messages and outputs.
    unsafe {
        match len {
            16 => ffi_sme2::blake3_servil_sme2_packed16(input, key, flags, out, groups),
            32 => ffi_sme2::blake3_servil_sme2_packed32(input, key, flags, out, groups),
            48 => ffi_sme2::blake3_servil_sme2_packed48(input, key, flags, out, groups),
            _ => unreachable!("packed width {len}"),
        }
    }
}

#[cfg(target_arch = "aarch64")]
mod ffi_sme2 {
    unsafe extern "C" {
        pub fn blake3_servil_sme2_packed16(input: *const u8, key: *const u32, flags: u32, out: *mut u8, groups: u64) -> u64;
        pub fn blake3_servil_sme2_packed32(input: *const u8, key: *const u32, flags: u32, out: *mut u8, groups: u64) -> u64;
        pub fn blake3_servil_sme2_packed48(input: *const u8, key: *const u32, flags: u32, out: *mut u8, groups: u64) -> u64;
    }
}

#[cfg(target_arch = "aarch64")]
mod ffi_neon {
    unsafe extern "C" {
        pub fn blake3_servil_neon_p11_packed16(input: *const u8, groups: u64, key: *const u32, flags: u32, out: *mut u8);
        pub fn blake3_servil_neon_p11_packed32(input: *const u8, groups: u64, key: *const u32, flags: u32, out: *mut u8);
        pub fn blake3_servil_neon_p11_packed48(input: *const u8, groups: u64, key: *const u32, flags: u32, out: *mut u8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Mode;
    use crate::throughput::{Packed, hash_many};

    #[test]
    fn packed_self_test_expected_matches_official_reference() {
        let mut input = [0u8; SELF_TEST_COUNT * 48];
        let mut out = [[0u8; OUT_LEN]; SELF_TEST_COUNT];
        let mut computed = [0u64; 3];
        for (idx, len) in [16usize, 32, 48].into_iter().enumerate() {
            let slice = &mut input[..SELF_TEST_COUNT * len];
            fill_self_test_input(slice, len);
            for (i, msg) in slice.chunks_exact(len).enumerate() {
                out[i] = crate::test::official::hash(Mode::Keyed(&SELF_TEST_KEY), msg);
            }
            computed[idx] = crate::self_test::fold(out.as_flattened());
        }
        assert_eq!(SELF_TEST_EXPECTED, computed, "SELF_TEST_EXPECTED should be [{:#018x}, {:#018x}, {:#018x}]", computed[0], computed[1], computed[2]);
    }

    fn check_width<const N: usize>()
    where
        Packed<N>: crate::block::Prepared,
    {
        crate::initialize();
        let key = [0x42u8; 32];
        let ctx = "blake3-servil packed direct-load test";
        for (m_idx, mode) in [Mode::Plain, Mode::Keyed(&key), Mode::DeriveKey(ctx)].into_iter().enumerate() {
            for count in [0usize, 1, 7, 8, 9, 10, 11, 12, 15, 16, 17, 22, 24, 27, 32, 33, 43] {
                let mut packed = Packed::<N>::new(count);
                for (i, slot) in packed.iter_mut().enumerate() {
                    for (j, b) in slot.iter_mut().enumerate() {
                        *b = ((i.wrapping_mul(257).wrapping_add(j.wrapping_mul(31))) ^ 0x5c).to_le_bytes()[0];
                    }
                }
                let mut out = std::vec![[0u8; OUT_LEN]; count];
                hash_many(mode, &packed, &mut out);
                for (i, (slot, got)) in packed.iter().zip(&out).enumerate() {
                    let want = crate::test::official::hash(mode, slot);
                    assert_eq!(*got, want, "Packed<{N}> count={count} i={i} mode={m_idx}");
                }
            }
        }
    }

    #[test]
    fn packed_16_32_48_match_official_across_counts_and_modes() {
        check_width::<16>();
        check_width::<32>();
        check_width::<48>();
    }

    #[test]
    fn packed_whole_groups_take_direct_load_kernels() {
        crate::initialize();
        let platform = Platform::detect();
        let (group_len, entries): (usize, [usize; 3]) = match platform {
            #[cfg(target_arch = "aarch64")]
            Platform::SME2 { .. } => (
                16,
                [
                    ffi_sme2::blake3_servil_sme2_packed16 as *const () as usize,
                    ffi_sme2::blake3_servil_sme2_packed32 as *const () as usize,
                    ffi_sme2::blake3_servil_sme2_packed48 as *const () as usize,
                ],
            ),
            #[cfg(target_arch = "aarch64")]
            Platform::NEON { .. } => (
                11,
                [
                    ffi_neon::blake3_servil_neon_p11_packed16 as *const () as usize,
                    ffi_neon::blake3_servil_neon_p11_packed32 as *const () as usize,
                    ffi_neon::blake3_servil_neon_p11_packed48 as *const () as usize,
                ],
            ),
            #[cfg(target_arch = "x86_64")]
            Platform::AVX512 { .. } => (
                16,
                [
                    ffi_x86::blake3_servil_avx512_packed16 as *const () as usize,
                    ffi_x86::blake3_servil_avx512_packed32 as *const () as usize,
                    ffi_x86::blake3_servil_avx512_packed48 as *const () as usize,
                ],
            ),
            #[cfg(target_arch = "x86_64")]
            Platform::AVX2 { .. } => (
                8,
                [
                    ffi_x86::blake3_servil_avx2_packed16 as *const () as usize,
                    ffi_x86::blake3_servil_avx2_packed32 as *const () as usize,
                    ffi_x86::blake3_servil_avx2_packed48 as *const () as usize,
                ],
            ),
            #[cfg(target_arch = "x86_64")]
            Platform::SSE41 { .. } => return,
        };

        for (idx, len) in [16usize, 32, 48].into_iter().enumerate() {
            for (count, reached) in [(group_len, true), (2 * group_len, true), (group_len - 1, false)] {
                let mut input = std::vec![0u8; count * len];
                for (i, b) in input.iter_mut().enumerate() {
                    *b = (i.wrapping_mul(17) ^ 0x33).to_le_bytes()[0];
                }
                let mut out = std::vec![[0u8; OUT_LEN]; count];
                crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
                crate::many::hash_many_on(&input, len, len, &crate::IV, 0, &mut out, platform);
                let ran = crate::RAN_ENTRIES.with(|r| r.borrow().clone());
                assert_eq!(ran.contains(&entries[idx]), reached, "packed<{len}> count={count}");
                if reached {
                    assert_eq!(ran.len(), 1, "packed<{len}> exact groups run direct kernel alone: {ran:?}");
                }
            }
        }
    }

    #[test]
    fn packed_direct_load_kernels_respect_guard_pages() {
        use crate::test::guard::Guarded;
        crate::initialize();
        let platform = Platform::detect();
        for len in [16usize, 32, 48] {
            for count in [8usize, 11, 16, 22, 27, 32, 43] {
                for at_end in [false, true] {
                    let mut input = Guarded::new(count * len, at_end);
                    for (i, b) in input.bytes().iter_mut().enumerate() {
                        *b = (i.wrapping_mul(29) ^ 0x71).to_le_bytes()[0];
                    }
                    let mut out = Guarded::new(count * OUT_LEN, at_end);
                    let out_slice = out.bytes().as_chunks_mut::<OUT_LEN>().0;
                    crate::many::hash_many_on(input.bytes(), len, len, &crate::IV, 0, out_slice, platform);
                    for (i, (msg, got)) in input.bytes().chunks_exact(len).zip(out_slice.iter()).enumerate() {
                        assert_eq!(*got, crate::test::official::hash(Mode::Plain, msg), "len={len} count={count} at_end={at_end} i={i}");
                    }
                }
            }
        }
    }

    #[test]
    #[should_panic(expected = "packed messages are 16, 32, or 48 bytes")]
    fn hash_packed_groups_rejects_invalid_len() {
        crate::initialize();
        // Use 1 output (below every packed SIMD group width of 4/8/16) so even
        // if validation were stripped, no SIMD kernel would run on this fixture.
        let input = [0u8; 24];
        let mut out = [[0u8; OUT_LEN]; 1];
        let _ = hash_packed_groups(&input, 24, &crate::IV, 0, &mut out, Platform::detect());
    }

    #[test]
    #[should_panic(expected = "packed input length must equal len * outputs.len()")]
    fn hash_packed_groups_rejects_short_input() {
        crate::initialize();
        let input = [0u8; 15];
        let mut out = [[0u8; OUT_LEN]; 1];
        let _ = hash_packed_groups(&input, 16, &crate::IV, 0, &mut out, Platform::detect());
    }

    #[test]
    #[should_panic(expected = "packed input length must equal len * outputs.len()")]
    fn hash_packed_groups_rejects_excess_input() {
        crate::initialize();
        let input = [0u8; 17];
        let mut out = [[0u8; OUT_LEN]; 1];
        let _ = hash_packed_groups(&input, 16, &crate::IV, 0, &mut out, Platform::detect());
    }
}
