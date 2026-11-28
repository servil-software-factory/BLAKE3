//! AArch64 SME2 + 2-integer hybrid (18-lane, `w4/w9/w13/w16`) and NEON 8+3
//! (11-lane, `w4/w9/w16`) zero-word-pruned counter-range and early-exit `find`
//! FFI wrappers (#511, #585).

use super::{FindHitCtx, TRUNC16_LEN, base_and_word, find_chunk_groups, mask_and_target_u64, record_find_hits, wrapping_add_u64, wrapping_add_usize};
use crate::{BLOCK_LEN, CVWords, OUT_LEN, platform::Platform};

type Sme2CounterFn = unsafe extern "C" fn(*const u32, *const u32, u64, u64, *mut u8, u64) -> u64;
type Sme2FindFn = unsafe extern "C" fn(*const u32, *const u32, u64, u64, u64, u64) -> u64;
type NeonCounterFn = unsafe extern "C" fn(*const u32, *const u32, u64, u64, *mut u8, u64);
type NeonFindFn = unsafe extern "C" fn(*const u32, *const u32, u64, u64, u64, u64) -> u64;

#[inline]
pub(super) fn sme2_counter_kernel(blen_u8: u8) -> Sme2CounterFn {
    match blen_u8 {
        ..=16 => ffi_sme2::blake3_servil_sme2_counter_w4,
        ..=36 => ffi_sme2::blake3_servil_sme2_counter_w9,
        ..=52 => ffi_sme2::blake3_servil_sme2_counter_w13,
        _ => ffi_sme2::blake3_servil_sme2_counter_w16,
    }
}

#[inline]
pub(super) fn sme2_find_kernel(blen_u8: u8) -> Sme2FindFn {
    match blen_u8 {
        ..=16 => ffi_sme2::blake3_servil_sme2_find_w4,
        ..=36 => ffi_sme2::blake3_servil_sme2_find_w9,
        ..=52 => ffi_sme2::blake3_servil_sme2_find_w13,
        _ => ffi_sme2::blake3_servil_sme2_find_w16,
    }
}

#[inline]
pub(super) fn neon_counter_kernel(blen_u8: u8) -> NeonCounterFn {
    match blen_u8 {
        ..=16 => ffi_neon::blake3_servil_neon_p11_counter_w4,
        ..=36 => ffi_neon::blake3_servil_neon_p11_counter_w9,
        _ => ffi_neon::blake3_servil_neon_p11_counter_w16,
    }
}

#[inline]
pub(super) fn neon_find_kernel(blen_u8: u8) -> NeonFindFn {
    match blen_u8 {
        ..=16 => ffi_neon::blake3_servil_neon_p11_find_w4,
        ..=36 => ffi_neon::blake3_servil_neon_p11_find_w9,
        _ => ffi_neon::blake3_servil_neon_p11_find_w16,
    }
}

pub(super) fn hash_counter_groups<const OUT: usize>(
    tmpl_words: &[u32; 16],
    blen_u8: u8,
    counter_word: usize,
    start: u32,
    key: &CVWords,
    root_flags: u8,
    out: &mut [[u8; OUT]],
    platform: Platform,
) -> usize {
    const {
        assert!(OUT == OUT_LEN || OUT == TRUNC16_LEN);
    }
    let flags_and_len = u64::from_le_bytes([root_flags, blen_u8, u8::from(OUT == TRUNC16_LEN), 0, 0, 0, 0, 0]);
    let mut done = 0usize;

    if matches!(platform, Platform::SME2 { .. }) {
        let groups = out.len() / 18;
        if groups != 0 {
            let turn = crate::platform::Sme2Turn::take(platform, true);
            if matches!(turn.platform(), Platform::SME2 { .. }) {
                let kernel = sme2_counter_kernel(blen_u8);
                // Sound: `Platform::SME2` verified SME2 support; `tmpl_words` holds
                // 16 readable `u32` words with zero words past the tier width,
                // `key` holds 8 readable `u32` words, `counter_word < tier_words`,
                // and `out` holds at least `18 * groups` writable `OUT`-byte slots.
                // The kernel checks SVL == 16 and `TPIDR2_EL0` reserved bytes
                // before touching ZA/streaming state and returns 16 only when all
                // `18 * groups` digests were written.
                let svl = unsafe {
                    kernel(tmpl_words.as_ptr(), key.as_ptr(), base_and_word(start, counter_word), flags_and_len, out.as_mut_ptr().cast(), groups as u64)
                };
                if svl == 16 {
                    crate::ran_kernel(kernel as *const ());
                    done += groups * 18;
                }
            }
            // Drop `turn` before NEON or scalar tail runs so concurrent callers
            // are never blocked on SME2 while this call drains a scalar tail.
        }
    }

    if matches!(platform, Platform::NEON { .. } | Platform::SME2 { .. }) {
        let rem = out.len() - done;
        let groups = rem / 11;
        if groups != 0 {
            let kernel = neon_counter_kernel(blen_u8);
            crate::ran_kernel(kernel as *const ());
            let cur = wrapping_add_usize(start, done);
            // Sound: AArch64 NEON is mandatory on `Platform::NEON`/`SME2`;
            // `tmpl_words` holds 16 readable `u32` words with zero words past
            // the tier width, `key` holds 8 readable `u32` words,
            // `counter_word < tier_words`, and `out[done..]` holds at least
            // `11 * groups` writable `OUT`-byte slots.
            unsafe {
                kernel(tmpl_words.as_ptr(), key.as_ptr(), base_and_word(cur, counter_word), flags_and_len, out[done..].as_mut_ptr().cast(), groups as u64);
            }
            done += groups * 11;
        }
    }

    done
}

#[allow(clippy::too_many_arguments)]
pub(super) fn find_counter_groups(
    tmpl_words: &[u32; 16],
    ctx: &FindHitCtx<'_>,
    start: u32,
    count: u64,
    mask: u32,
    target: u32,
    block: &mut [u8; BLOCK_LEN],
    matches: &mut [(u32, [u8; OUT_LEN])],
    found: &mut usize,
) -> u64 {
    let flags_and_len = u64::from_le_bytes([ctx.root_flags, ctx.blen_u8, 0, 0, 0, 0, 0, 0]);
    let mt = mask_and_target_u64(mask, target);
    let mut off = 0u64;

    if matches!(ctx.platform, Platform::SME2 { .. }) && count >= 18 && *found != matches.len() {
        let turn = crate::platform::Sme2Turn::take(ctx.platform, true);
        if matches!(turn.platform(), Platform::SME2 { .. }) {
            let kernel = sme2_find_kernel(ctx.blen_u8);
            while *found != matches.len() {
                let rem = count.checked_sub(off).expect("off <= count");
                if rem < 18 {
                    break;
                }
                let groups = find_chunk_groups(rem, 18);
                let cur = wrapping_add_u64(start, off);
                // Sound: `Platform::SME2` verified SME2 support; `tmpl_words` holds
                // 16 readable `u32` words with zero words past the tier width,
                // `key` holds 8 readable `u32` words, `counter_word < tier_words`,
                // and `groups <= MAX_FIND_GROUPS`. The kernel checks SVL == 16 and
                // `TPIDR2_EL0` reserved bytes and returns `u64::MAX` if declined.
                let res = unsafe { kernel(tmpl_words.as_ptr(), ctx.key.as_ptr(), base_and_word(cur, ctx.counter_word), flags_and_len, mt, groups) };
                if res == u64::MAX {
                    break;
                }
                crate::ran_kernel(kernel as *const ());
                let hit_mask = res & ((1u64 << 18) - 1);
                let g_idx = res >> 18;
                let next_off = if hit_mask != 0 {
                    let base_off = off + g_idx * 18;
                    record_find_hits(base_off, hit_mask, start, block, ctx, matches, found);
                    base_off + 18
                } else {
                    off + g_idx * 18
                };
                assert!(next_off > off, "SME2 find kernel failed to advance");
                off = next_off;
            }
        }
        // Drop `turn` before NEON or scalar tail runs so concurrent callers
        // are never blocked on SME2 while this call drains a scalar tail.
    }

    if matches!(ctx.platform, Platform::NEON { .. } | Platform::SME2 { .. }) {
        let kernel = neon_find_kernel(ctx.blen_u8);
        while *found != matches.len() {
            let rem = count.checked_sub(off).expect("off <= count");
            if rem < 11 {
                break;
            }
            let groups = find_chunk_groups(rem, 11);
            let cur = wrapping_add_u64(start, off);
            crate::ran_kernel(kernel as *const ());
            // Sound: AArch64 NEON is mandatory on `Platform::NEON`/`SME2`;
            // `tmpl_words` holds 16 readable `u32` words with zero words past
            // the tier width, `key` holds 8 readable `u32` words, and
            // `counter_word < tier_words`.
            let res = unsafe { kernel(tmpl_words.as_ptr(), ctx.key.as_ptr(), base_and_word(cur, ctx.counter_word), flags_and_len, mt, groups) };
            let hit_mask = res & ((1u64 << 11) - 1);
            let g_idx = res >> 11;
            let next_off = if hit_mask != 0 {
                let base_off = off + g_idx * 11;
                record_find_hits(base_off, hit_mask, start, block, ctx, matches, found);
                base_off + 11
            } else {
                off + g_idx * 11
            };
            assert!(next_off > off, "NEON find kernel failed to advance");
            off = next_off;
        }
    }

    off
}

#[cfg(test)]
pub(crate) fn entries_neon() -> [(&'static str, *const ()); 6] {
    [
        ("neon_p11_counter_w4", ffi_neon::blake3_servil_neon_p11_counter_w4 as *const ()),
        ("neon_p11_counter_w9", ffi_neon::blake3_servil_neon_p11_counter_w9 as *const ()),
        ("neon_p11_counter_w16", ffi_neon::blake3_servil_neon_p11_counter_w16 as *const ()),
        ("neon_p11_find_w4", ffi_neon::blake3_servil_neon_p11_find_w4 as *const ()),
        ("neon_p11_find_w9", ffi_neon::blake3_servil_neon_p11_find_w9 as *const ()),
        ("neon_p11_find_w16", ffi_neon::blake3_servil_neon_p11_find_w16 as *const ()),
    ]
}

#[cfg(test)]
pub(crate) fn entries_sme2() -> [(&'static str, *const ()); 8] {
    [
        ("sme2_counter_w4", ffi_sme2::blake3_servil_sme2_counter_w4 as *const ()),
        ("sme2_counter_w9", ffi_sme2::blake3_servil_sme2_counter_w9 as *const ()),
        ("sme2_counter_w13", ffi_sme2::blake3_servil_sme2_counter_w13 as *const ()),
        ("sme2_counter_w16", ffi_sme2::blake3_servil_sme2_counter_w16 as *const ()),
        ("sme2_find_w4", ffi_sme2::blake3_servil_sme2_find_w4 as *const ()),
        ("sme2_find_w9", ffi_sme2::blake3_servil_sme2_find_w9 as *const ()),
        ("sme2_find_w13", ffi_sme2::blake3_servil_sme2_find_w13 as *const ()),
        ("sme2_find_w16", ffi_sme2::blake3_servil_sme2_find_w16 as *const ()),
    ]
}

#[cfg(test)]
pub(crate) unsafe fn call_sme2_counter(template: *const u32, key: *const u32, base_and_word: u64, flags_and_len: u64, out: *mut u8, groups: u64) -> u64 {
    // Sound: caller verified SME2 support and provided valid buffers covering
    // 16 `u32` template words (words 13..16 zero), 8 `u32` key words, and
    // `18 * groups` outputs.
    unsafe { ffi_sme2::blake3_servil_sme2_counter_w13(template, key, base_and_word, flags_and_len, out, groups) }
}

#[cfg(test)]
pub(crate) unsafe fn call_sme2_find(template: *const u32, key: *const u32, base_and_word: u64, flags_and_len: u64, mask_and_target: u64, groups: u64) -> u64 {
    // Sound: caller verified SME2 support and provided valid buffers covering
    // 16 `u32` template words (words 13..16 zero) and 8 `u32` key words.
    unsafe { ffi_sme2::blake3_servil_sme2_find_w13(template, key, base_and_word, flags_and_len, mask_and_target, groups) }
}

mod ffi_sme2 {
    unsafe extern "C" {
        pub fn blake3_servil_sme2_counter_w4(template: *const u32, key: *const u32, base_and_word: u64, flags_and_len: u64, out: *mut u8, groups: u64) -> u64;
        pub fn blake3_servil_sme2_counter_w9(template: *const u32, key: *const u32, base_and_word: u64, flags_and_len: u64, out: *mut u8, groups: u64) -> u64;
        pub fn blake3_servil_sme2_counter_w13(template: *const u32, key: *const u32, base_and_word: u64, flags_and_len: u64, out: *mut u8, groups: u64) -> u64;
        pub fn blake3_servil_sme2_counter_w16(template: *const u32, key: *const u32, base_and_word: u64, flags_and_len: u64, out: *mut u8, groups: u64) -> u64;
        pub fn blake3_servil_sme2_find_w4(
            template: *const u32,
            key: *const u32,
            base_and_word: u64,
            flags_and_len: u64,
            mask_and_target: u64,
            groups: u64,
        ) -> u64;
        pub fn blake3_servil_sme2_find_w9(
            template: *const u32,
            key: *const u32,
            base_and_word: u64,
            flags_and_len: u64,
            mask_and_target: u64,
            groups: u64,
        ) -> u64;
        pub fn blake3_servil_sme2_find_w13(
            template: *const u32,
            key: *const u32,
            base_and_word: u64,
            flags_and_len: u64,
            mask_and_target: u64,
            groups: u64,
        ) -> u64;
        pub fn blake3_servil_sme2_find_w16(
            template: *const u32,
            key: *const u32,
            base_and_word: u64,
            flags_and_len: u64,
            mask_and_target: u64,
            groups: u64,
        ) -> u64;
    }
}

mod ffi_neon {
    unsafe extern "C" {
        pub fn blake3_servil_neon_p11_counter_w4(template: *const u32, key: *const u32, base_and_word: u64, flags_and_len: u64, out: *mut u8, groups: u64);
        pub fn blake3_servil_neon_p11_counter_w9(template: *const u32, key: *const u32, base_and_word: u64, flags_and_len: u64, out: *mut u8, groups: u64);
        pub fn blake3_servil_neon_p11_counter_w16(template: *const u32, key: *const u32, base_and_word: u64, flags_and_len: u64, out: *mut u8, groups: u64);
        pub fn blake3_servil_neon_p11_find_w4(
            template: *const u32,
            key: *const u32,
            base_and_word: u64,
            flags_and_len: u64,
            mask_and_target: u64,
            groups: u64,
        ) -> u64;
        pub fn blake3_servil_neon_p11_find_w9(
            template: *const u32,
            key: *const u32,
            base_and_word: u64,
            flags_and_len: u64,
            mask_and_target: u64,
            groups: u64,
        ) -> u64;
        pub fn blake3_servil_neon_p11_find_w16(
            template: *const u32,
            key: *const u32,
            base_and_word: u64,
            flags_and_len: u64,
            mask_and_target: u64,
            groups: u64,
        ) -> u64;
    }
}
