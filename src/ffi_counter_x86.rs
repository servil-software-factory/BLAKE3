//! x86-64 AVX-512 (16-lane, `w4/w9/w13/w16`) and AVX2 (8-lane, `w4/w9/w16`)
//! zero-word-pruned counter-range and early-exit `find` FFI wrappers (#511, #585).

use super::{FindHitCtx, TRUNC16_LEN, base_and_word, find_chunk_groups, mask_and_target_u64, record_find_hits, wrapping_add_u64, wrapping_add_usize};
use crate::{BLOCK_LEN, CVWords, OUT_LEN, platform::Platform};

type X86CounterFn = unsafe extern "C" fn(*const u32, u64, *const u32, u64, u64, *mut u8);
type X86FindFn = unsafe extern "C" fn(*const u32, u64, *const u32, u64, u64, u64) -> u64;

#[inline]
pub(super) fn avx512_counter_kernel(blen_u8: u8) -> X86CounterFn {
    match blen_u8 {
        ..=16 => ffi_x86::blake3_servil_avx512_counter_w4,
        ..=36 => ffi_x86::blake3_servil_avx512_counter_w9,
        ..=52 => ffi_x86::blake3_servil_avx512_counter_w13,
        _ => ffi_x86::blake3_servil_avx512_counter_w16,
    }
}

#[inline]
pub(super) fn avx512_find_kernel(blen_u8: u8) -> X86FindFn {
    match blen_u8 {
        ..=16 => ffi_x86::blake3_servil_avx512_find_w4,
        ..=36 => ffi_x86::blake3_servil_avx512_find_w9,
        ..=52 => ffi_x86::blake3_servil_avx512_find_w13,
        _ => ffi_x86::blake3_servil_avx512_find_w16,
    }
}

#[inline]
pub(super) fn avx2_counter_kernel(blen_u8: u8) -> X86CounterFn {
    match blen_u8 {
        ..=16 => ffi_x86::blake3_servil_avx2_counter_w4,
        ..=36 => ffi_x86::blake3_servil_avx2_counter_w9,
        _ => ffi_x86::blake3_servil_avx2_counter_w16,
    }
}

#[inline]
pub(super) fn avx2_find_kernel(blen_u8: u8) -> X86FindFn {
    match blen_u8 {
        ..=16 => ffi_x86::blake3_servil_avx2_find_w4,
        ..=36 => ffi_x86::blake3_servil_avx2_find_w9,
        _ => ffi_x86::blake3_servil_avx2_find_w16,
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

    if matches!(platform, Platform::AVX512 { .. }) {
        let groups = out.len() / 16;
        if groups != 0 {
            let kernel = avx512_counter_kernel(blen_u8);
            crate::ran_kernel(kernel as *const ());
            // Sound: `Platform::AVX512` verified AVX-512F+VL; `tmpl_words`
            // holds 16 readable `u32` words with zero words past the tier
            // width, `key` holds 8 readable `u32` words,
            // `counter_word < tier_words`, and `out` holds at least
            // `16 * groups` writable `OUT`-byte slots.
            unsafe {
                kernel(tmpl_words.as_ptr(), groups as u64, key.as_ptr(), base_and_word(start, counter_word), flags_and_len, out.as_mut_ptr().cast());
            }
            done += groups * 16;
        }
    }

    if matches!(platform, Platform::AVX2 { .. } | Platform::AVX512 { .. }) {
        let rem = out.len() - done;
        let groups = rem / 8;
        if groups != 0 {
            let kernel = avx2_counter_kernel(blen_u8);
            crate::ran_kernel(kernel as *const ());
            let cur = wrapping_add_usize(start, done);
            // Sound: `Platform::AVX2` and `Platform::AVX512` guarantee AVX2;
            // `tmpl_words` holds 16 readable `u32` words with zero words past
            // the tier width, `key` holds 8 readable `u32` words,
            // `counter_word < tier_words`, and `out[done..]` holds at least
            // `8 * groups` writable `OUT`-byte slots.
            unsafe {
                kernel(tmpl_words.as_ptr(), groups as u64, key.as_ptr(), base_and_word(cur, counter_word), flags_and_len, out[done..].as_mut_ptr().cast());
            }
            done += groups * 8;
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

    if matches!(ctx.platform, Platform::AVX512 { .. }) {
        let kernel = avx512_find_kernel(ctx.blen_u8);
        while *found != matches.len() {
            let rem = count.checked_sub(off).expect("off <= count");
            if rem < 16 {
                break;
            }
            let groups = find_chunk_groups(rem, 16);
            let cur = wrapping_add_u64(start, off);
            crate::ran_kernel(kernel as *const ());
            // Sound: `Platform::AVX512` verified AVX-512F+VL; `tmpl_words`
            // holds 16 readable `u32` words with zero words past the tier
            // width, `key` holds 8 readable `u32` words, and
            // `counter_word < tier_words`.
            let res = unsafe { kernel(tmpl_words.as_ptr(), groups, ctx.key.as_ptr(), base_and_word(cur, ctx.counter_word), flags_and_len, mt) };
            let hit_mask = res & ((1u64 << 16) - 1);
            let g_idx = res >> 16;
            let next_off = if hit_mask != 0 {
                let base_off = off + g_idx * 16;
                record_find_hits(base_off, hit_mask, start, block, ctx, matches, found);
                base_off + 16
            } else {
                off + g_idx * 16
            };
            assert!(next_off > off, "AVX-512 find kernel failed to advance");
            off = next_off;
        }
    }

    if matches!(ctx.platform, Platform::AVX2 { .. } | Platform::AVX512 { .. }) {
        let kernel = avx2_find_kernel(ctx.blen_u8);
        while *found != matches.len() {
            let rem = count.checked_sub(off).expect("off <= count");
            if rem < 8 {
                break;
            }
            let groups = find_chunk_groups(rem, 8);
            let cur = wrapping_add_u64(start, off);
            crate::ran_kernel(kernel as *const ());
            // Sound: `Platform::AVX2` and `Platform::AVX512` guarantee AVX2;
            // `tmpl_words` holds 16 readable `u32` words with zero words past
            // the tier width, `key` holds 8 readable `u32` words, and
            // `counter_word < tier_words`.
            let res = unsafe { kernel(tmpl_words.as_ptr(), groups, ctx.key.as_ptr(), base_and_word(cur, ctx.counter_word), flags_and_len, mt) };
            let hit_mask = res & ((1u64 << 8) - 1);
            let g_idx = res >> 8;
            let next_off = if hit_mask != 0 {
                let base_off = off + g_idx * 8;
                record_find_hits(base_off, hit_mask, start, block, ctx, matches, found);
                base_off + 8
            } else {
                off + g_idx * 8
            };
            assert!(next_off > off, "AVX2 find kernel failed to advance");
            off = next_off;
        }
    }

    off
}

#[cfg(test)]
pub(crate) fn entries_avx2() -> [(&'static str, *const ()); 6] {
    [
        ("avx2_counter_w4", ffi_x86::blake3_servil_avx2_counter_w4 as *const ()),
        ("avx2_counter_w9", ffi_x86::blake3_servil_avx2_counter_w9 as *const ()),
        ("avx2_counter_w16", ffi_x86::blake3_servil_avx2_counter_w16 as *const ()),
        ("avx2_find_w4", ffi_x86::blake3_servil_avx2_find_w4 as *const ()),
        ("avx2_find_w9", ffi_x86::blake3_servil_avx2_find_w9 as *const ()),
        ("avx2_find_w16", ffi_x86::blake3_servil_avx2_find_w16 as *const ()),
    ]
}

#[cfg(test)]
pub(crate) fn entries_avx512() -> [(&'static str, *const ()); 8] {
    [
        ("avx512_counter_w4", ffi_x86::blake3_servil_avx512_counter_w4 as *const ()),
        ("avx512_counter_w9", ffi_x86::blake3_servil_avx512_counter_w9 as *const ()),
        ("avx512_counter_w13", ffi_x86::blake3_servil_avx512_counter_w13 as *const ()),
        ("avx512_counter_w16", ffi_x86::blake3_servil_avx512_counter_w16 as *const ()),
        ("avx512_find_w4", ffi_x86::blake3_servil_avx512_find_w4 as *const ()),
        ("avx512_find_w9", ffi_x86::blake3_servil_avx512_find_w9 as *const ()),
        ("avx512_find_w13", ffi_x86::blake3_servil_avx512_find_w13 as *const ()),
        ("avx512_find_w16", ffi_x86::blake3_servil_avx512_find_w16 as *const ()),
    ]
}

mod ffi_x86 {
    unsafe extern "C" {
        pub fn blake3_servil_avx512_counter_w4(template: *const u32, groups: u64, key: *const u32, base_and_word: u64, flags_and_len: u64, out: *mut u8);
        pub fn blake3_servil_avx512_counter_w9(template: *const u32, groups: u64, key: *const u32, base_and_word: u64, flags_and_len: u64, out: *mut u8);
        pub fn blake3_servil_avx512_counter_w13(template: *const u32, groups: u64, key: *const u32, base_and_word: u64, flags_and_len: u64, out: *mut u8);
        pub fn blake3_servil_avx512_counter_w16(template: *const u32, groups: u64, key: *const u32, base_and_word: u64, flags_and_len: u64, out: *mut u8);
        pub fn blake3_servil_avx512_find_w4(
            template: *const u32,
            groups: u64,
            key: *const u32,
            base_and_word: u64,
            flags_and_len: u64,
            mask_and_target: u64,
        ) -> u64;
        pub fn blake3_servil_avx512_find_w9(
            template: *const u32,
            groups: u64,
            key: *const u32,
            base_and_word: u64,
            flags_and_len: u64,
            mask_and_target: u64,
        ) -> u64;
        pub fn blake3_servil_avx512_find_w13(
            template: *const u32,
            groups: u64,
            key: *const u32,
            base_and_word: u64,
            flags_and_len: u64,
            mask_and_target: u64,
        ) -> u64;
        pub fn blake3_servil_avx512_find_w16(
            template: *const u32,
            groups: u64,
            key: *const u32,
            base_and_word: u64,
            flags_and_len: u64,
            mask_and_target: u64,
        ) -> u64;
        pub fn blake3_servil_avx2_counter_w4(template: *const u32, groups: u64, key: *const u32, base_and_word: u64, flags_and_len: u64, out: *mut u8);
        pub fn blake3_servil_avx2_counter_w9(template: *const u32, groups: u64, key: *const u32, base_and_word: u64, flags_and_len: u64, out: *mut u8);
        pub fn blake3_servil_avx2_counter_w16(template: *const u32, groups: u64, key: *const u32, base_and_word: u64, flags_and_len: u64, out: *mut u8);
        pub fn blake3_servil_avx2_find_w4(
            template: *const u32,
            groups: u64,
            key: *const u32,
            base_and_word: u64,
            flags_and_len: u64,
            mask_and_target: u64,
        ) -> u64;
        pub fn blake3_servil_avx2_find_w9(
            template: *const u32,
            groups: u64,
            key: *const u32,
            base_and_word: u64,
            flags_and_len: u64,
            mask_and_target: u64,
        ) -> u64;
        pub fn blake3_servil_avx2_find_w16(
            template: *const u32,
            groups: u64,
            key: *const u32,
            base_and_word: u64,
            flags_and_len: u64,
            mask_and_target: u64,
        ) -> u64;
    }
}
