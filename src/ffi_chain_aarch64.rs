//! AArch64 SME2 + 2-integer hybrid (18-lane) and NEON 8+3 (11-lane) fused
//! multi-step 32-byte hash-chain FFI wrappers (#584).

use super::{CHAIN_IN_LEN, CHAIN_OUT_LEN};
use crate::{CVWords, platform::Platform};

/// Minimum chain remainder padded into one 11-lane NEON call instead of serial
/// loops (on Apple M6, 3 serial chains of 59 steps take ~8.5 us vs ~6.9 us for
/// an 11-lane fused NEON burst).
pub(super) const PAD_SIMD_MIN: usize = 3;

#[inline(never)]
fn neon_p11_chain_padded(init: &[[u8; CHAIN_IN_LEN]], steps: u32, key: &CVWords, rflags: u32, out: &mut [[u8; CHAIN_OUT_LEN]]) {
    assert!((1..11).contains(&init.len()), "NEON padded chain count must be in 1..11");
    assert_eq!(init.len(), out.len());
    let mut pad_in = [[0u8; CHAIN_IN_LEN]; 11];
    pad_in[..init.len()].copy_from_slice(init);
    let mut pad_out = [[0u8; CHAIN_OUT_LEN]; 11];
    crate::ran_kernel(ffi_neon::blake3_servil_neon_p11_chain32_trunc16 as *const ());
    // Sound: AArch64 NEON is mandatory on `Platform::NEON`/`SME2`; `pad_in`
    // holds 11 initialized 32-byte chain slots, `key` holds 8 `u32` words, and
    // `pad_out` holds 11 writable 16-byte output slots.
    unsafe {
        ffi_neon::blake3_servil_neon_p11_chain32_trunc16(pad_in.as_ptr().cast(), 1, key.as_ptr(), rflags, pad_out.as_mut_ptr().cast(), steps);
    }
    out.copy_from_slice(&pad_out[..out.len()]);
}

#[inline]
pub(super) fn varlen_lane_width(platform: Platform, active_count: usize, total_work: u64) -> usize {
    match platform {
        Platform::SME2 { .. } if active_count >= super::SME2_CHAIN_MIN && total_work >= super::SME2_VARLEN_MIN_WORK => 18,
        Platform::SME2 { .. } | Platform::NEON { .. } => 11,
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

    if matches!(platform, Platform::SME2 { .. }) {
        let groups = init.len() / 18;
        if groups != 0 {
            let kernel = ffi_sme2::blake3_servil_sme2_chain32_trunc16;
            // Sound: `Platform::SME2` verified SME2 support; `init` holds at
            // least `18 * groups` readable 32-byte slots, `key` holds 8 `u32`
            // words, and `out` holds at least `18 * groups` writable 16-byte
            // slots. The kernel checks SVL == 16 and `TPIDR2_EL0` reserved
            // bytes before touching ZA/streaming state and returns 16 only when
            // all `18 * groups` outputs were written.
            let svl = unsafe { kernel(init.as_ptr().cast(), key.as_ptr(), rflags, steps, out.as_mut_ptr().cast(), groups as u64) };
            if svl == 16 {
                crate::ran_kernel(kernel as *const ());
                done += groups * 18;
            }
        }
    }

    if matches!(platform, Platform::NEON { .. } | Platform::SME2 { .. }) {
        let rem = init.len() - done;
        let groups = rem / 11;
        if groups != 0 {
            let kernel = ffi_neon::blake3_servil_neon_p11_chain32_trunc16;
            crate::ran_kernel(kernel as *const ());
            // Sound: AArch64 NEON is mandatory on `Platform::NEON`/`SME2`;
            // `init[done..]` holds at least `11 * groups` readable 32-byte
            // slots, `key` holds 8 `u32` words, and `out[done..]` holds at
            // least `11 * groups` writable 16-byte slots.
            unsafe {
                kernel(init[done..].as_ptr().cast(), groups as u64, key.as_ptr(), rflags, out[done..].as_mut_ptr().cast(), steps);
            }
            done += groups * 11;
        }
        let rem = init.len() - done;
        if rem >= PAD_SIMD_MIN {
            neon_p11_chain_padded(&init[done..], steps, key, rflags, &mut out[done..]);
            done += rem;
        }
    }

    done
}

#[cfg(test)]
pub(crate) fn entries_neon() -> [(&'static str, *const ()); 1] {
    [("neon_p11_chain32_trunc16", ffi_neon::blake3_servil_neon_p11_chain32_trunc16 as *const ())]
}

#[cfg(test)]
pub(crate) fn entries_sme2() -> [(&'static str, *const ()); 1] {
    [("sme2_chain32_trunc16", ffi_sme2::blake3_servil_sme2_chain32_trunc16 as *const ())]
}

#[cfg(test)]
pub(crate) unsafe fn call_sme2_chain(init: *const u8, key: *const u32, flags: u32, steps: u32, out: *mut u8, groups: u64) -> u64 {
    // Sound: caller verified SME2 support and provided valid buffers covering
    // `18 * groups` 32-byte inputs, 8 `u32` key words, and `18 * groups`
    // 16-byte outputs.
    unsafe { ffi_sme2::blake3_servil_sme2_chain32_trunc16(init, key, flags, steps, out, groups) }
}

mod ffi_sme2 {
    unsafe extern "C" {
        pub fn blake3_servil_sme2_chain32_trunc16(init: *const u8, key: *const u32, flags: u32, steps: u32, out: *mut u8, groups: u64) -> u64;
    }
}

mod ffi_neon {
    unsafe extern "C" {
        pub fn blake3_servil_neon_p11_chain32_trunc16(init: *const u8, groups: u64, key: *const u32, flags: u32, out: *mut u8, steps: u32);
    }
}
