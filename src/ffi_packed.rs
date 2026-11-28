//! Direct-load SIMD kernels for contiguous `Packed<16>`, `Packed<32>`, and
//! `Packed<48>` batches across AVX-512, AVX2, Apple/AArch64 SME2, and AArch64
//! NEON `8+3` (#501, #583).
//!
//! Full SIMD groups read contiguous packed messages directly from `input` into
//! `outputs` with zero 8 KiB stack expansion and zero zero-word message
//! additions; on AArch64 NEON, `8..=10`-message remainders also finish in one
//! 11-lane call — in place over the final 11 messages when a prior group
//! already ran (`done > 0`, zero scratch and zero copies), or via an
//! out-of-line size-matched `[[u8; N]; 11]` input scratch (`176`/`352`/`528` B)
//! and `[[u8; 32]; 11]` output scratch for a lone `8..=10`-message batch (#583).

use crate::{CHUNK_END, CHUNK_START, CVWords, OUT_LEN, ROOT, platform::Platform};

/// Minimum AArch64 NEON remainder count hashed by padding into one 11-lane
/// `blake3_servil_neon_p11_packed{16,32,48}` call instead of expanding into a
/// 1 KiB block buffer and calling the `p8`/`p9`/`pn10` parent plans (#583).
///
/// Measured on Apple M6 (`mac-m6-timing`, `on_this_core()`, `Mode::Keyed`,
/// min of 9 x 20k warm calls, ns/call; jobs
/// `20261128T002745Z-john-example-blincs583586-{base3,cand3}`):
/// - `w = 7` / `18` are faster unpadded on `p7` (`Packed<32>`: `120.3` vs
///   `128.1 ns`, `242.2` vs `255.2 ns`), so padding starts at `8`.
/// - `w = 8..=10`: `Packed<16>` `139.0..146.1 -> 126.0..131.4 ns` (-6.9%..-13.3%),
///   `Packed<32>` `141.4..149.7 -> 128.3..130.0 ns` (-9.3%..-13.2%),
///   `Packed<48>` `143.8..155.9 -> 135.4..136.2 ns` (-5.7%..-12.6%).
/// - `w = 19..=21`: `Packed<16>` `251.2..265.2 -> 244.0..245.1 ns` (-2.9%..-7.6%),
///   `Packed<32>` `258.0..275.5 -> 253.4..257.0 ns` (-1.4%..-6.7%),
///   `Packed<48>` `265.4..281.0 -> 263.3..264.7 ns` (-0.8%..-5.8%).
///
/// Re-measuring the earlier inlined fixed-528-byte scratch (`67bf3eb6`, job
/// `20261128T034802Z-john-example-blincs586`) showed `+4.3..+8.5 ns` overhead
/// on tails and `+384 B` on `many::hash_packed`'s common frame. When a prior
/// group already ran (`done > 0`), at least 11 contiguous messages exist in
/// place, so the tail re-runs the final 11 slots directly with zero scratch or
/// copies; only a lone `8..=10` batch uses the out-of-line size-matched
/// `neon_packed_tail::<N>` (`[[0u8; N]; 11]`) scratch.
#[cfg(target_arch = "aarch64")]
const NEON_PACKED_PAD_MIN: usize = 8;

#[cfg(target_arch = "aarch64")]
#[inline]
fn neon_packed_kernel(len: usize) -> unsafe extern "C" fn(*const u8, u64, *const u32, u32, *mut u8) {
    match len {
        16 => ffi_neon::blake3_servil_neon_p11_packed16,
        32 => ffi_neon::blake3_servil_neon_p11_packed32,
        48 => ffi_neon::blake3_servil_neon_p11_packed48,
        _ => unreachable!("asserted len in {{16, 32, 48}}"),
    }
}

#[cfg(all(test, target_arch = "aarch64"))]
std::thread_local! {
    /// When `Some`, records `(inputs_ptr, outputs_ptr, groups)` for each NEON
    /// packed FFI invocation on this thread, letting tests distinguish in-place
    /// tails from stack-scratch tails without growing during unrelated tests.
    static NEON_PACKED_CALLS: core::cell::RefCell<Option<std::vec::Vec<(usize, usize, u64)>>> = const { core::cell::RefCell::new(None) };
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn call_neon_packed(
    kernel: unsafe extern "C" fn(*const u8, u64, *const u32, u32, *mut u8),
    inputs: *const u8,
    groups: u64,
    key: *const u32,
    root_flags: u32,
    outputs: *mut u8,
) {
    crate::ran_kernel(kernel as *const ());
    #[cfg(test)]
    let _ = NEON_PACKED_CALLS.try_with(|c| {
        if let Some(log) = c.borrow_mut().as_mut() {
            log.push((inputs as usize, outputs as usize, groups));
        }
    });
    // Sound: forwarded from caller's `unsafe fn` contract (`11 * groups * len`
    // readable input bytes and `11 * groups` writable 32-byte output slots).
    unsafe {
        kernel(inputs, groups, key, root_flags, outputs);
    }
}

#[cfg(target_arch = "aarch64")]
#[inline(never)]
fn neon_packed_tail<const N: usize>(input: &[u8], key: &CVWords, root_flags: u32, outputs: &mut [[u8; OUT_LEN]]) {
    const { assert!(N == 16 || N == 32 || N == 48, "packed messages are 16, 32, or 48 bytes") };
    assert!((NEON_PACKED_PAD_MIN..11).contains(&outputs.len()), "NEON packed tail count must be in {NEON_PACKED_PAD_MIN}..11");
    assert_eq!(input.len(), outputs.len() * N, "NEON packed tail input size mismatch");
    let kernel = neon_packed_kernel(N);
    let mut pad_in = [[0u8; N]; 11];
    pad_in.as_flattened_mut()[..input.len()].copy_from_slice(input);
    let mut pad_out = [[0u8; OUT_LEN]; 11];
    // Sound: AArch64 NEON is mandatory on `Platform::NEON`/`SME2`; `kernel`
    // matches `N`, so `pad_in` holds `11 * N` initialized readable bytes and
    // `pad_out` holds 11 writable 32-byte slots.
    unsafe {
        call_neon_packed(kernel, pad_in.as_ptr().cast(), 1, key.as_ptr(), root_flags, pad_out.as_mut_ptr().cast());
    }
    outputs.copy_from_slice(&pad_out[..outputs.len()]);
}

/// Hash full SIMD groups (and on AArch64 NEON, a padded `8..=10`-message tail
/// via `neon_p11_packed`) of `len`-byte packed messages (`len` in
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
            let kernel = neon_packed_kernel(len);
            // Sound: AArch64 NEON is mandatory on `Platform::NEON`/`SME2`;
            // `input[done * len..]` has at least `11 * groups * len` readable
            // bytes and `outputs[done..]` has at least `11 * groups` writable
            // 32-byte slots.
            unsafe {
                call_neon_packed(kernel, input[done * len..].as_ptr(), groups as u64, key.as_ptr(), root_flags, outputs[done..].as_mut_ptr().cast());
            }
            done += groups * 11;
        }
        let rem = outputs.len() - done;
        if rem >= NEON_PACKED_PAD_MIN {
            if done > 0 {
                // At least 11 contiguous messages already sit at the end of
                // `input` / `outputs` (`done >= 11` after NEON or `>= 16` after
                // SME2): finish the final 11 slots in place with zero scratch
                // buffer, zero zero-fill, and zero copies (1..=3 overlapping
                // digests are recomputed to identical bytes).
                let tail_start = outputs.len() - 11;
                let kernel = neon_packed_kernel(len);
                // Sound: AArch64 NEON is mandatory on `Platform::NEON`/`SME2`;
                // `done > 0` and `rem >= 8` guarantee `outputs.len() >= 11`, so
                // `input[tail_start * len..]` has `11 * len` readable bytes and
                // `outputs[tail_start..]` has 11 writable 32-byte slots.
                unsafe {
                    call_neon_packed(kernel, input[tail_start * len..].as_ptr(), 1, key.as_ptr(), root_flags, outputs[tail_start..].as_mut_ptr().cast());
                }
            } else {
                match len {
                    16 => neon_packed_tail::<16>(input, key, root_flags, outputs),
                    32 => neon_packed_tail::<32>(input, key, root_flags, outputs),
                    48 => neon_packed_tail::<48>(input, key, root_flags, outputs),
                    _ => unreachable!("asserted len in {{16, 32, 48}}"),
                }
            }
            done += rem;
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
            #[cfg(target_arch = "aarch64")]
            let below_group = if matches!(platform, Platform::NEON { .. }) { NEON_PACKED_PAD_MIN - 1 } else { group_len - 1 };
            #[cfg(not(target_arch = "aarch64"))]
            let below_group = group_len - 1;
            for (count, reached) in [(group_len, true), (2 * group_len, true), (below_group, false)] {
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

    #[cfg(target_arch = "aarch64")]
    fn fill_distinct_packed(buf: &mut [u8], len: usize) {
        for (msg_idx, msg) in buf.chunks_exact_mut(len).enumerate() {
            let tag = (msg_idx as u32).to_le_bytes();
            msg[..4].copy_from_slice(&tag);
            for (j, b) in msg[4..].iter_mut().enumerate() {
                *b = ((msg_idx.wrapping_mul(37) ^ j.wrapping_mul(17) ^ 0xa5) & 0xff) as u8;
            }
        }
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn packed_neon_padded_tail_threshold_and_digests() {
        crate::initialize();
        let neon = Platform::own_core();
        let neon_entries = [
            ffi_neon::blake3_servil_neon_p11_packed16 as *const () as usize,
            ffi_neon::blake3_servil_neon_p11_packed32 as *const () as usize,
            ffi_neon::blake3_servil_neon_p11_packed48 as *const () as usize,
        ];
        let key_bytes = [0x42u8; 32];
        let ctx = "blake3-servil neon packed tail test";
        for mode in [Mode::Plain, Mode::Keyed(&key_bytes), Mode::DeriveKey(ctx)] {
            let (key, flags) = mode.key_and_flags();
            for (idx, len) in [16usize, 32, 48].into_iter().enumerate() {
                for (count, want_done, want_ran) in [
                    (NEON_PACKED_PAD_MIN - 1, 0, false),
                    (NEON_PACKED_PAD_MIN, NEON_PACKED_PAD_MIN, true),
                    (9, 9, true),
                    (10, 10, true),
                    (18, 11, true),
                    (19, 19, true),
                    (20, 20, true),
                    (21, 21, true),
                ] {
                    let mut input = std::vec![0u8; count * len];
                    fill_distinct_packed(&mut input, len);
                    let mut out = std::vec![[0u8; OUT_LEN]; count];
                    crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
                    NEON_PACKED_CALLS.with(|c| *c.borrow_mut() = Some(std::vec::Vec::new()));
                    let done = hash_packed_groups(&input, len, &key, flags, &mut out, neon);
                    let calls = NEON_PACKED_CALLS.with(|c| c.borrow_mut().take()).unwrap();
                    let ran = crate::RAN_ENTRIES.with(|r| r.borrow().clone());
                    let in_base = input.as_ptr() as usize;
                    let out_base = out.as_ptr() as usize;
                    assert_eq!(done, want_done, "NEON packed<{len}> count={count}");
                    assert_eq!(ran.contains(&neon_entries[idx]), want_ran, "NEON packed<{len}> count={count} ran={ran:?}");
                    if !want_ran {
                        assert!(calls.is_empty(), "NEON packed<{len}> count={count} unexpected calls={calls:?}");
                    } else if count < 11 {
                        // Lone 8..=10-message batch runs through stack scratch.
                        assert_eq!(calls.len(), 1, "NEON lone tail packed<{len}> count={count}");
                        assert_eq!(calls[0].2, 1);
                        assert_ne!(calls[0].0, in_base, "NEON lone tail must use input scratch");
                        assert_ne!(calls[0].1, out_base, "NEON lone tail must use output scratch");
                    } else if count == 18 {
                        assert_eq!(calls, std::vec![(in_base, out_base, 1)], "NEON packed<{len}> count={count}");
                    } else {
                        // 19..=21: 1 full group at 0 + 1 in-place overlapping tail at count - 11.
                        let tail = count - 11;
                        assert_eq!(
                            calls,
                            std::vec![(in_base, out_base, 1), (in_base + tail * len, out_base + tail * OUT_LEN, 1)],
                            "NEON overlapping tail packed<{len}> count={count}"
                        );
                    }
                    for i in 0..done {
                        let want = crate::test::official::hash(mode, &input[i * len..][..len]);
                        assert_eq!(out[i], want, "NEON packed<{len}> count={count} i={i}");
                    }
                }
            }
        }
        // When running on a 512-bit SME2 thread, also verify post-SME2 NEON
        // in-place overlapping tails (16 + 7..=11 and 32 + 8..=10) check both
        // kernels, exact in-place pointer offsets, and distinct per-lane digests.
        if let Some(sme2) = Platform::sme2() {
            let sme2_entries = [
                ffi_sme2::blake3_servil_sme2_packed16 as *const () as usize,
                ffi_sme2::blake3_servil_sme2_packed32 as *const () as usize,
                ffi_sme2::blake3_servil_sme2_packed48 as *const () as usize,
            ];
            let (key, flags) = Mode::Keyed(&key_bytes).key_and_flags();
            for (idx, len) in [16usize, 32, 48].into_iter().enumerate() {
                for (count, want_done, want_neon) in
                    [(23, 16, false), (24, 24, true), (25, 25, true), (26, 26, true), (27, 27, true), (40, 40, true), (41, 41, true), (42, 42, true)]
                {
                    let mut input = std::vec![0u8; count * len];
                    fill_distinct_packed(&mut input, len);
                    let mut out = std::vec![[0u8; OUT_LEN]; count];
                    crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
                    NEON_PACKED_CALLS.with(|c| *c.borrow_mut() = Some(std::vec::Vec::new()));
                    let done = hash_packed_groups(&input, len, &key, flags, &mut out, sme2);
                    let calls = NEON_PACKED_CALLS.with(|c| c.borrow_mut().take()).unwrap();
                    let ran = crate::RAN_ENTRIES.with(|r| r.borrow().clone());
                    let in_base = input.as_ptr() as usize;
                    let out_base = out.as_ptr() as usize;
                    assert_eq!(done, want_done, "SME2+NEON packed<{len}> count={count}");
                    assert!(ran.contains(&sme2_entries[idx]), "SME2 packed<{len}> count={count} ran={ran:?}");
                    assert_eq!(ran.contains(&neon_entries[idx]), want_neon, "post-SME2 NEON packed<{len}> count={count} ran={ran:?}");
                    if !want_neon {
                        assert!(calls.is_empty(), "post-SME2 packed<{len}> count={count} unexpected NEON calls={calls:?}");
                    } else {
                        let tail = count - 11;
                        assert_eq!(
                            calls,
                            std::vec![(in_base + tail * len, out_base + tail * OUT_LEN, 1)],
                            "post-SME2 in-place NEON tail packed<{len}> count={count}"
                        );
                    }
                    for i in 0..done {
                        let want = crate::test::official::hash(Mode::Keyed(&key_bytes), &input[i * len..][..len]);
                        assert_eq!(out[i], want, "SME2+NEON packed<{len}> count={count} i={i}");
                    }
                }
            }
        }
    }

    #[test]
    fn packed_direct_load_kernels_respect_guard_pages() {
        use crate::test::guard::Guarded;
        crate::initialize();
        let detected = Platform::detect();
        let own = Platform::own_core();
        let platforms: &[Platform] = if own == detected { &[detected] } else { &[detected, own] };
        for &platform in platforms {
            for len in [16usize, 32, 48] {
                for count in [8usize, 10, 11, 16, 19, 21, 22, 24, 26, 27, 32, 43] {
                    for at_end in [false, true] {
                        let mut input = Guarded::new(count * len, at_end);
                        for (msg_idx, msg) in input.bytes().chunks_exact_mut(len).enumerate() {
                            msg[..4].copy_from_slice(&(msg_idx as u32).to_le_bytes());
                            for (j, b) in msg[4..].iter_mut().enumerate() {
                                *b = ((msg_idx.wrapping_mul(29) ^ j.wrapping_mul(13) ^ 0x71) & 0xff) as u8;
                            }
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
