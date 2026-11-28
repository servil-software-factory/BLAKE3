//! x86-64 AVX-512 (16-lane) and AVX2 (8-lane) fused multi-step 32-byte
//! hash-chain FFI wrappers (#584).

use super::{CHAIN_IN_LEN, CHAIN_OUT_LEN};
use crate::{CVWords, platform::Platform};

/// Minimum chain remainder padded into one 8-lane AVX2 call instead of serial
/// loops (on AVX-512/AVX2, 2 serial chains cost ~6.4 us/59 steps vs ~4.5 us
/// for an 8-lane padded AVX2 burst).
pub(super) const PAD_SIMD_MIN: usize = 2;

#[inline(never)]
fn avx512_chain_padded(init: &[[u8; CHAIN_IN_LEN]], steps: u32, key: &CVWords, rflags: u32, out: &mut [[u8; CHAIN_OUT_LEN]]) {
    assert!((1..16).contains(&init.len()), "AVX-512 padded chain count must be in 1..16");
    assert_eq!(init.len(), out.len());
    let mut pad_in = [[0u8; CHAIN_IN_LEN]; 16];
    pad_in[..init.len()].copy_from_slice(init);
    let mut pad_out = [[0u8; CHAIN_OUT_LEN]; 16];
    crate::ran_kernel(ffi_x86::blake3_servil_avx512_chain32_trunc16 as *const ());
    // Sound: caller verified `Platform::AVX512`; `pad_in` holds 16 initialized
    // 32-byte chain slots, `key` holds 8 `u32` words, and `pad_out` holds 16
    // writable 16-byte output slots.
    unsafe {
        ffi_x86::blake3_servil_avx512_chain32_trunc16(pad_in.as_ptr().cast(), 1, key.as_ptr(), rflags, pad_out.as_mut_ptr().cast(), steps);
    }
    out.copy_from_slice(&pad_out[..out.len()]);
}

#[inline(never)]
fn avx2_chain_padded(init: &[[u8; CHAIN_IN_LEN]], steps: u32, key: &CVWords, rflags: u32, out: &mut [[u8; CHAIN_OUT_LEN]]) {
    assert!((1..8).contains(&init.len()), "AVX2 padded chain count must be in 1..8");
    assert_eq!(init.len(), out.len());
    let mut pad_in = [[0u8; CHAIN_IN_LEN]; 8];
    pad_in[..init.len()].copy_from_slice(init);
    let mut pad_out = [[0u8; CHAIN_OUT_LEN]; 8];
    crate::ran_kernel(ffi_x86::blake3_servil_avx2_chain32_trunc16 as *const ());
    // Sound: caller verified `Platform::AVX2`/`AVX512`; `pad_in` holds 8
    // initialized 32-byte chain slots, `key` holds 8 `u32` words, and `pad_out`
    // holds 8 writable 16-byte output slots.
    unsafe {
        ffi_x86::blake3_servil_avx2_chain32_trunc16(pad_in.as_ptr().cast(), 1, key.as_ptr(), rflags, pad_out.as_mut_ptr().cast(), steps);
    }
    out.copy_from_slice(&pad_out[..out.len()]);
}

#[inline]
pub(super) fn varlen_lane_width(platform: Platform, active_count: usize, total_work: u64) -> usize {
    let _ = (active_count, total_work);
    match platform {
        Platform::AVX512 { .. } => 16,
        Platform::AVX2 { .. } => 8,
        Platform::SSE41 { .. } => 1,
    }
}

pub(super) fn hash_chain_groups(
    init: &[[u8; CHAIN_IN_LEN]],
    steps: u32,
    key: &CVWords,
    rflags: u32,
    out: &mut [[u8; CHAIN_OUT_LEN]],
    platform: Platform,
) -> usize {
    let mut done = 0usize;

    if matches!(platform, Platform::AVX512 { .. }) {
        let groups = init.len() / 16;
        if groups != 0 {
            let kernel = ffi_x86::blake3_servil_avx512_chain32_trunc16;
            crate::ran_kernel(kernel as *const ());
            // Sound: `Platform::AVX512` verified AVX-512F+VL; `init` holds at
            // least `16 * groups` readable 32-byte slots, `key` holds 8 `u32`
            // words, and `out` holds at least `16 * groups` writable 16-byte
            // slots.
            unsafe {
                kernel(init.as_ptr().cast(), groups as u64, key.as_ptr(), rflags, out.as_mut_ptr().cast(), steps);
            }
            done += groups * 16;
        }
        let rem = init.len() - done;
        if rem >= 9 {
            avx512_chain_padded(&init[done..], steps, key, rflags, &mut out[done..]);
            done += rem;
        }
    }

    if matches!(platform, Platform::AVX2 { .. } | Platform::AVX512 { .. }) {
        let rem = init.len() - done;
        let groups = rem / 8;
        if groups != 0 {
            let kernel = ffi_x86::blake3_servil_avx2_chain32_trunc16;
            crate::ran_kernel(kernel as *const ());
            // Sound: `Platform::AVX2`/`AVX512` guarantee AVX2; `init[done..]`
            // holds at least `8 * groups` readable 32-byte slots, `key` holds
            // 8 `u32` words, and `out[done..]` holds at least `8 * groups`
            // writable 16-byte slots.
            unsafe {
                kernel(init[done..].as_ptr().cast(), groups as u64, key.as_ptr(), rflags, out[done..].as_mut_ptr().cast(), steps);
            }
            done += groups * 8;
        }
        let rem = init.len() - done;
        if rem >= PAD_SIMD_MIN {
            avx2_chain_padded(&init[done..], steps, key, rflags, &mut out[done..]);
            done += rem;
        }
    }

    done
}

#[cfg(test)]
pub(crate) fn entries_avx2() -> [(&'static str, *const ()); 1] {
    [("avx2_chain32_trunc16", ffi_x86::blake3_servil_avx2_chain32_trunc16 as *const ())]
}

#[cfg(test)]
pub(crate) fn entries_avx512() -> [(&'static str, *const ()); 1] {
    [("avx512_chain32_trunc16", ffi_x86::blake3_servil_avx512_chain32_trunc16 as *const ())]
}

mod ffi_x86 {
    unsafe extern "C" {
        pub fn blake3_servil_avx512_chain32_trunc16(init: *const u8, groups: u64, key: *const u32, flags: u32, out: *mut u8, steps: u32);
        pub fn blake3_servil_avx2_chain32_trunc16(init: *const u8, groups: u64, key: *const u32, flags: u32, out: *mut u8, steps: u32);
    }
}
