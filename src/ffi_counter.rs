//! Single-template zero-word-pruned SIMD kernels for
//! [`crate::throughput::hash_counter_range`],
//! [`crate::throughput::hash_counter_range_trunc16`],
//! [`crate::throughput::find_counter_range`], and their [`crate::throughput::TreeHasher`]
//! methods across AVX-512, AVX2, Apple/AArch64 SME2 + 2-integer hybrid (18 lanes),
//! and AArch64 NEON `8+3` (11 lanes) (#511, #585).
//!
//! Each backend broadcasts the single-block (`4..=64` byte) template once
//! before the group loop, selects a zero-word-pruned variant for the template's
//! active word count (`<= 16 / 36 / 52 / 64` bytes), and updates only
//! `counter_word` per group, eliminating caller-side per-message input buffers,
//! message transposes, and Apple P-core -> SME2 dirty cache-line coherency stalls.

use crate::{BLOCK_LEN, CHUNK_END, CHUNK_START, CVWords, OUT_LEN, ROOT, platform::Platform};

#[cfg(target_arch = "aarch64")]
#[path = "ffi_counter_aarch64.rs"]
mod arch;
#[cfg(target_arch = "x86_64")]
#[path = "ffi_counter_x86.rs"]
mod arch;

#[cfg(all(test, target_arch = "aarch64"))]
pub(crate) use arch::{call_sme2_counter, call_sme2_find, entries_neon, entries_sme2};
#[cfg(all(test, target_arch = "x86_64"))]
pub(crate) use arch::{entries_avx2, entries_avx512};

/// Output length in bytes of [`crate::throughput::hash_counter_range_trunc16`].
pub(crate) const TRUNC16_LEN: usize = 16;

/// Cap per FFI `find` call so `groups_done << 18` stays well below `u64::MAX`
/// (SME2's declined sentinel) while running up to `18 * 2^40` compressions per
/// call.
const MAX_FIND_GROUPS: u64 = 1 << 40;

/// A validated single-block counter-range template and counter-word index.
/// Checking once at construction (`template.len() <= BLOCK_LEN` and
/// `counter_word < template.len() / 4`) eliminates redundant re-validation
/// across public wrappers and FFI dispatch, and guarantees `counter_word` is
/// strictly below the active word count of every length-specialized kernel.
#[derive(Clone, Copy)]
pub(crate) struct ValidatedTemplate<'a> {
    bytes: &'a [u8],
    counter_word: usize,
}

impl<'a> ValidatedTemplate<'a> {
    #[inline]
    pub(crate) fn new(template: &'a [u8], counter_word: usize) -> Self {
        assert!(template.len() <= BLOCK_LEN, "hash_counter_range template must fit in one block ({BLOCK_LEN} bytes), got {}", template.len());
        assert!(
            counter_word < template.len() / 4,
            "hash_counter_range counter_word {counter_word} (bytes {}..{}) out of bounds for {}-byte template",
            counter_word.saturating_mul(4),
            counter_word.saturating_mul(4).saturating_add(4),
            template.len(),
        );
        Self { bytes: template, counter_word }
    }

    #[inline]
    fn bytes(self) -> &'a [u8] {
        self.bytes
    }

    #[inline]
    fn counter_word(self) -> usize {
        self.counter_word
    }

    #[inline]
    fn len_u8(self) -> u8 {
        u8::try_from(self.bytes.len()).expect("validated <= BLOCK_LEN")
    }
}

#[inline]
pub(crate) fn words_from_block(block: &[u8; BLOCK_LEN]) -> [u32; 16] {
    let mut words = [0u32; 16];
    for (word, chunk) in words.iter_mut().zip(block.as_chunks::<4>().0) {
        *word = u32::from_le_bytes(*chunk);
    }
    words
}

#[inline]
fn wrapping_add_usize(start: u32, delta: usize) -> u32 {
    let bytes = delta.to_le_bytes();
    start.wrapping_add(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

#[inline]
fn wrapping_add_u64(start: u32, delta: u64) -> u32 {
    let bytes = delta.to_le_bytes();
    start.wrapping_add(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

#[inline]
fn base_and_word(base: u32, counter_word: usize) -> u64 {
    u64::from(base) + ((counter_word as u64) << 32)
}

#[inline]
fn mask_and_target_u64(mask: u32, target: u32) -> u64 {
    u64::from(mask) + (u64::from(target) << 32)
}

#[inline]
fn find_chunk_groups(remaining: u64, lanes: u64) -> u64 {
    (remaining / lanes).min(MAX_FIND_GROUPS)
}

/// Hash `out.len()` single-block messages derived from a validated template on
/// `platform`, replacing the 4-byte little-endian word at `counter_word * 4`
/// with `start.wrapping_add(i)` (modulo 2^32) for `i in 0..out.len()`, writing
/// `OUT`-byte digests (`32` or `16`).
pub(crate) fn hash_counter_range_on<const OUT: usize>(
    template: ValidatedTemplate<'_>,
    start: u32,
    key: &CVWords,
    flags: u8,
    out: &mut [[u8; OUT]],
    platform: Platform,
) {
    let bytes = template.bytes();
    let counter_word = template.counter_word();
    let mut block = [0u8; BLOCK_LEN];
    block[..bytes.len()].copy_from_slice(bytes);
    let tmpl_words = words_from_block(&block);

    const ROOT_BLOCK_FLAGS: u8 = CHUNK_START + CHUNK_END + ROOT;
    let root_flags = flags + ROOT_BLOCK_FLAGS;
    let blen_u8 = template.len_u8();
    let done = arch::hash_counter_groups(&tmpl_words, blen_u8, counter_word, start, key, root_flags, out, platform);

    // Serial tail (< 18 / < 11 / < 16 / < 8, or SSE4.1).
    let cw_off = counter_word * 4;
    let mut cur = wrapping_add_usize(start, done);
    for slot in &mut out[done..] {
        block[cw_off..cw_off + 4].copy_from_slice(&cur.to_le_bytes());
        let mut cv = *key;
        platform.compress_in_place(&mut cv, &block, blen_u8, 0, root_flags);
        let full = crate::hasher::le_bytes(&cv);
        slot.copy_from_slice(&full[..OUT]);
        cur = cur.wrapping_add(1);
    }
}

/// Shared immutable context for recording matching counters inside `find`.
pub(super) struct FindHitCtx<'a> {
    pub(super) counter_word: usize,
    pub(super) cw_off: usize,
    pub(super) blen_u8: u8,
    pub(super) key: &'a CVWords,
    pub(super) root_flags: u8,
    pub(super) platform: Platform,
}

/// Record every matching lane in `hit_mask` (ascending lane order) into
/// `matches[*found..]`, computing each matching counter's full 32-byte digest.
pub(super) fn record_find_hits(
    base_off: u64,
    mut hit_mask: u64,
    start: u32,
    block: &mut [u8; BLOCK_LEN],
    ctx: &FindHitCtx<'_>,
    matches: &mut [(u32, [u8; OUT_LEN])],
    found: &mut usize,
) {
    while hit_mask != 0 && *found < matches.len() {
        let lane = u64::from(hit_mask.trailing_zeros());
        hit_mask &= hit_mask - 1;
        let ctr = wrapping_add_u64(start, base_off + lane);
        block[ctx.cw_off..ctx.cw_off + 4].copy_from_slice(&ctr.to_le_bytes());
        let mut cv = *ctx.key;
        ctx.platform.compress_in_place(&mut cv, block, ctx.blen_u8, 0, ctx.root_flags);
        matches[*found] = (ctr, crate::hasher::le_bytes(&cv));
        *found += 1;
    }
}

/// True when a `find_counter_range` call has zero search space, zero output
/// capacity, or a mathematically unsatisfiable `(word0 & mask) == target`.
#[inline]
pub(crate) fn find_is_noop(count: u64, matches_len: usize, mask: u32, target: u32) -> bool {
    if count == 0 {
        return true;
    }
    if matches_len == 0 {
        return true;
    }
    (target & !mask) != 0
}

/// Search `count` consecutive single-block messages derived from `template`
/// starting at `start` for digests whose first little-endian 32-bit word
/// satisfies `(word0 & mask) == target`, recording matches in ascending offset
/// order into `matches` and returning the number of matches found (stopping
/// early as soon as `matches` is full).
#[allow(clippy::too_many_arguments)]
pub(crate) fn find_counter_range_on(
    template: ValidatedTemplate<'_>,
    start: u32,
    count: u64,
    mask: u32,
    target: u32,
    key: &CVWords,
    flags: u8,
    matches: &mut [(u32, [u8; OUT_LEN])],
    platform: Platform,
) -> usize {
    if find_is_noop(count, matches.len(), mask, target) {
        return 0;
    }
    let bytes = template.bytes();
    let counter_word = template.counter_word();
    let mut block = [0u8; BLOCK_LEN];
    block[..bytes.len()].copy_from_slice(bytes);
    let tmpl_words = words_from_block(&block);

    const ROOT_BLOCK_FLAGS: u8 = CHUNK_START + CHUNK_END + ROOT;
    let ctx = FindHitCtx { counter_word, cw_off: counter_word * 4, blen_u8: template.len_u8(), key, root_flags: flags + ROOT_BLOCK_FLAGS, platform };

    let mut found = 0usize;
    let off = arch::find_counter_groups(&tmpl_words, &ctx, start, count, mask, target, &mut block, matches, &mut found);

    // Serial tail (< 18 / < 11 / < 16 / < 8, or SSE4.1).
    for tail_off in off..count {
        if found == matches.len() {
            break;
        }
        let ctr = wrapping_add_u64(start, tail_off);
        block[ctx.cw_off..ctx.cw_off + 4].copy_from_slice(&ctr.to_le_bytes());
        let mut cv = *key;
        platform.compress_in_place(&mut cv, &block, ctx.blen_u8, 0, ctx.root_flags);
        if (cv[0] & mask) == target {
            matches[found] = (ctr, crate::hasher::le_bytes(&cv));
            found += 1;
        }
    }

    found
}

/// `47 = 2*18 + 11 = 2*16 + 8 + 7 = 4*11 + 3 = 5*8 + 7`, exercising multi-group
/// wrapping and primary-to-secondary SIMD fallback on every ISA in one call.
const SELF_TEST_COUNT: usize = 47;
const SELF_TEST_TIERS: [(usize, usize); 4] = [(16, 3), (36, 8), (52, 12), (64, 15)];
const SELF_TEST_START: u32 = u32::MAX - 5;
const SELF_TEST_KEY: [u8; 32] = [
    0x17, 0x2b, 0x3f, 0x53, 0x67, 0x7b, 0x8f, 0xa3, 0xb7, 0xcb, 0xdf, 0xf3, 0x09, 0x1d, 0x31, 0x45, 0x59, 0x6d, 0x81, 0x95, 0xa9, 0xbd, 0xd1, 0xe5, 0xf9, 0x0e,
    0x22, 0x36, 0x4a, 0x5e, 0x72, 0x86,
];

fn self_test_template() -> [u8; BLOCK_LEN] {
    let mut tmpl = [0u8; BLOCK_LEN];
    for (j, b) in tmpl.iter_mut().enumerate() {
        *b = crate::low_byte(j.wrapping_mul(37) ^ 0x9a);
    }
    tmpl
}

/// Fold of `47` keyed counter-range digests across all four length tiers
/// (`16, 36, 52, 64` bytes, `counter_word = 3, 8, 12, 15`, `start = u32::MAX - 5`)
/// from official BLAKE3 1.8.7.
const SELF_TEST_EXPECTED: u64 = 0xa760_4b92_4272_b97b;

/// Test-only planted fault mode for `self_test::stops`:
/// `1` flips a 32-byte digest byte, `2` flips a 16-byte truncated digest byte,
/// `3` corrupts the `find_counter_range` hit digest.
#[cfg(test)]
pub(crate) static PLANTED_WRONG_COUNTER: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

#[inline(never)]
pub(crate) fn self_test(platform: Platform) -> usize {
    let key = crate::platform::words_from_le_bytes_32(&SELF_TEST_KEY);
    let tmpl = self_test_template();
    let mut out32 = [[0u8; OUT_LEN]; SELF_TEST_COUNT];
    let mut out16 = [[0u8; TRUNC16_LEN]; SELF_TEST_COUNT];
    let mut fold_buf = [0u8; 4 * 8];
    #[cfg(test)]
    let planted = PLANTED_WRONG_COUNTER.load(core::sync::atomic::Ordering::Relaxed);

    for (tier_idx, &(len, cw)) in SELF_TEST_TIERS.iter().enumerate() {
        let vt = ValidatedTemplate::new(&tmpl[..len], cw);
        hash_counter_range_on(vt, SELF_TEST_START, &key, crate::KEYED_HASH, &mut out32, platform);
        #[cfg(test)]
        if planted == 1 {
            out32[0][0] = !out32[0][0];
        }
        let tier_fold = crate::self_test::fold(out32.as_flattened());
        fold_buf[tier_idx * 8..(tier_idx + 1) * 8].copy_from_slice(&tier_fold.to_le_bytes());

        hash_counter_range_on(vt, SELF_TEST_START, &key, crate::KEYED_HASH, &mut out16, platform);
        #[cfg(test)]
        if planted == 2 {
            out16[0][0] = !out16[0][0];
        }
        for (d32, d16) in out32.iter().zip(out16.iter()) {
            if &d32[..TRUNC16_LEN] != d16 {
                crate::self_test::fail(format_args!(
                    "blake3-servil startup self-test failed: hash_counter_range_trunc16 len={len} (platform {})",
                    platform.name(),
                ));
            }
        }

        // Exercise both primary-kernel hit (idx 17: SME2 int17 / AVX-512 g1 / NEON g1 / AVX2 g2)
        // and secondary-kernel hit (idx 39: misses 2 SME2 or AVX-512 groups, hits NEON lane 3 / AVX2 lane 7).
        for idx in [17usize, 39usize] {
            let w0 = u32::from_le_bytes([out32[idx][0], out32[idx][1], out32[idx][2], out32[idx][3]]);
            let want_ctr = wrapping_add_usize(SELF_TEST_START, idx);
            let mut hit = [(0u32, [0u8; OUT_LEN]); 1];
            let n = find_counter_range_on(vt, SELF_TEST_START, SELF_TEST_COUNT as u64, u32::MAX, w0, &key, crate::KEYED_HASH, &mut hit, platform);
            #[cfg(test)]
            if planted == 3 {
                hit[0].1[0] = !hit[0].1[0];
            }
            if n != 1 {
                crate::self_test::fail(format_args!(
                    "blake3-servil startup self-test failed: find_counter_range len={len} idx={idx} count={n} (platform {})",
                    platform.name(),
                ));
            }
            if hit[0] != (want_ctr, out32[idx]) {
                crate::self_test::fail(format_args!(
                    "blake3-servil startup self-test failed: find_counter_range len={len} idx={idx} digest mismatch (platform {})",
                    platform.name(),
                ));
            }
        }
    }

    let got = crate::self_test::fold(&fold_buf);
    if got != SELF_TEST_EXPECTED {
        crate::self_test::fail(format_args!(
            "blake3-servil startup self-test failed: hash_counter_range (platform {}): \
             expected {:#018x}, got {:#018x}",
            platform.name(),
            SELF_TEST_EXPECTED,
            got,
        ));
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Mode;
    use crate::throughput::{TreeHasher, find_counter_range, hash_counter_range, hash_counter_range_trunc16};

    fn official_counter_range(mode: Mode, template: &[u8], counter_word: usize, start: u32, count: usize) -> std::vec::Vec<[u8; OUT_LEN]> {
        let mut msg = template.to_vec();
        let off = counter_word * 4;
        (0..count)
            .map(|i| {
                // Independent mod-2^32 counter progression (not calling production `wrapping_add_usize`).
                let c = start.wrapping_add(u32::try_from(i % (1usize << 32)).unwrap());
                msg[off..off + 4].copy_from_slice(&c.to_le_bytes());
                crate::test::official::hash(mode, &msg)
            })
            .collect()
    }

    #[test]
    fn wrapping_add_helpers_truncate_high_bits_without_overflow() {
        assert_eq!(wrapping_add_usize(u32::MAX, 1), 0);
        assert_eq!(wrapping_add_usize(7, (1usize << 32) + 5), 12);
        assert_eq!(wrapping_add_usize(u32::MAX - 2, (3usize << 32) + 5), 2);
        assert_eq!(wrapping_add_u64(u32::MAX, 1), 0);
        assert_eq!(wrapping_add_u64(7, (1u64 << 32) + 5), 12);
        assert_eq!(wrapping_add_u64(u32::MAX - 2, (3u64 << 32) + 5), 2);
        assert_eq!(find_chunk_groups(u64::MAX, 18), MAX_FIND_GROUPS);
        assert_eq!(find_chunk_groups(37, 18), 2);
    }

    #[test]
    fn counter_self_test_expected_matches_official_reference() {
        let tmpl = self_test_template();
        let mut fold_buf = [0u8; 4 * 8];
        for (tier_idx, &(len, cw)) in SELF_TEST_TIERS.iter().enumerate() {
            let out = official_counter_range(Mode::Keyed(&SELF_TEST_KEY), &tmpl[..len], cw, SELF_TEST_START, SELF_TEST_COUNT);
            let tier_fold = crate::self_test::fold(out.as_flattened());
            fold_buf[tier_idx * 8..(tier_idx + 1) * 8].copy_from_slice(&tier_fold.to_le_bytes());
        }
        let computed = crate::self_test::fold(&fold_buf);
        assert_eq!(SELF_TEST_EXPECTED, computed, "SELF_TEST_EXPECTED should be {computed:#018x}");
    }

    fn supported_platforms() -> std::vec::Vec<Platform> {
        let mut platforms = std::vec::Vec::new();
        #[cfg(target_arch = "x86_64")]
        {
            if let Some(p) = Platform::avx512() {
                platforms.push(p);
            }
            if let Some(p) = Platform::avx2() {
                platforms.push(p);
            }
            if let Some(p) = Platform::sse41() {
                platforms.push(p);
            }
        }
        #[cfg(target_arch = "aarch64")]
        {
            if let Some(p) = Platform::sme2() {
                platforms.push(p);
            }
            if let Some(p) = Platform::neon() {
                platforms.push(p);
            }
        }
        platforms
    }

    #[test]
    fn counter_range_and_trunc16_match_official_across_tiers_words_wraps_and_modes() {
        crate::initialize();
        let key = [0x42u8; 32];
        let ctx = "blake3-servil counter-range test";
        let starts = [0u32, 1, 0x7fff_fffa, u32::MAX - 20, u32::MAX - 1, u32::MAX];
        let platforms = supported_platforms();
        for (m_idx, mode) in [Mode::Plain, Mode::Keyed(&key), Mode::DeriveKey(ctx)].into_iter().enumerate() {
            let (kw, fl) = mode.key_and_flags();
            for tree in [TreeHasher::new(mode), TreeHasher::new(mode).on_this_core()] {
                for len in [4usize, 7, 16, 17, 32, 36, 37, 52, 53, 60, 63, 64] {
                    let mut tmpl = std::vec![0u8; len];
                    for (j, b) in tmpl.iter_mut().enumerate() {
                        *b = ((j.wrapping_mul(41).wrapping_add(len)) ^ 0x5a).to_le_bytes()[0];
                    }
                    let max_cw = (len / 4) - 1;
                    let mut cws = std::vec![0usize, max_cw];
                    if max_cw >= 2 {
                        cws.push(max_cw / 2);
                    }
                    cws.sort_unstable();
                    cws.dedup();
                    for &cw in &cws {
                        for count in [0usize, 1, 7, 8, 9, 10, 11, 15, 16, 17, 18, 19, 22, 24, 32, 36, 47] {
                            let start = starts[(len + cw + count) % starts.len()];
                            let want = official_counter_range(mode, &tmpl, cw, start, count);

                            let mut got = std::vec![[0u8; OUT_LEN]; count];
                            tree.hash_counter_range(&tmpl, cw, start, &mut got);
                            assert_eq!(got, want, "mode={m_idx} len={len} cw={cw} count={count} start={start:#x}");

                            let mut got_fn = std::vec![[0u8; OUT_LEN]; count];
                            hash_counter_range(mode, &tmpl, cw, start, &mut got_fn);
                            assert_eq!(got_fn, want, "fn mode={m_idx} len={len} cw={cw} count={count}");

                            let want16: std::vec::Vec<[u8; TRUNC16_LEN]> = want.iter().map(|d| d[..TRUNC16_LEN].try_into().unwrap()).collect();
                            let mut got16 = std::vec![[0u8; TRUNC16_LEN]; count];
                            tree.hash_counter_range_trunc16(&tmpl, cw, start, &mut got16);
                            assert_eq!(got16, want16, "trunc16 tree mode={m_idx} len={len} cw={cw} count={count}");

                            let mut got16_fn = std::vec![[0u8; TRUNC16_LEN]; count];
                            hash_counter_range_trunc16(mode, &tmpl, cw, start, &mut got16_fn);
                            if count == 24 || count == 47 {
                                let vt = ValidatedTemplate::new(&tmpl, cw);
                                for &p in &platforms {
                                    let mut got_p = std::vec![[0u8; OUT_LEN]; count];
                                    hash_counter_range_on(vt, start, &kw, fl, &mut got_p, p);
                                    assert_eq!(got_p, want, "platform={} len={len} cw={cw} count={count}", p.name());
                                    let mut got16_p = std::vec![[0u8; TRUNC16_LEN]; count];
                                    hash_counter_range_on(vt, start, &kw, fl, &mut got16_p, p);
                                    assert_eq!(got16_p, want16, "trunc16 platform={} len={len} cw={cw} count={count}", p.name());
                                }
                            }
                            assert_eq!(got16_fn, want16, "trunc16 fn mode={m_idx} len={len} cw={cw} count={count}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn find_counter_range_matches_official_across_tiers_modes_masks_and_capacities() {
        crate::initialize();
        let key = [0x91u8; 32];
        let ctx = "blake3-servil find-counter test";
        for mode in [Mode::Plain, Mode::Keyed(&key), Mode::DeriveKey(ctx)] {
            for tree in [TreeHasher::new(mode), TreeHasher::new(mode).on_this_core()] {
                for &(len, cw) in &[(4usize, 0usize), (16, 3), (20, 4), (36, 8), (40, 9), (52, 12), (56, 13), (64, 15)] {
                    let mut tmpl = std::vec![0u8; len];
                    for (j, b) in tmpl.iter_mut().enumerate() {
                        *b = ((j.wrapping_mul(53).wrapping_add(len)) ^ 0x3e).to_le_bytes()[0];
                    }
                    let start = u32::MAX - 17;
                    let count = 95usize;
                    let all = official_counter_range(mode, &tmpl, cw, start, count);
                    let mask_broad = 0x0000_0007u32;
                    let target_broad = 0x0000_0005u32;
                    let want_broad: std::vec::Vec<(u32, [u8; OUT_LEN])> = all
                        .iter()
                        .enumerate()
                        .filter_map(|(i, d)| {
                            let w0 = u32::from_le_bytes([d[0], d[1], d[2], d[3]]);
                            ((w0 & mask_broad) == target_broad).then_some((wrapping_add_usize(start, i), *d))
                        })
                        .collect();

                    for cap in [0usize, 1, 2, 5, 32] {
                        let mut got = std::vec![(0u32, [0u8; OUT_LEN]); cap];
                        let n = tree.find_counter_range(&tmpl, cw, start, count as u64, mask_broad, target_broad, &mut got);
                        let want_n = want_broad.len().min(cap);
                        assert_eq!(n, want_n, "tree len={len} cw={cw} cap={cap}");
                        assert_eq!(&got[..n], &want_broad[..want_n], "tree matches len={len} cap={cap}");

                        let mut got_fn = std::vec![(0u32, [0u8; OUT_LEN]); cap];
                        let n_fn = find_counter_range(mode, &tmpl, cw, start, count as u64, mask_broad, target_broad, &mut got_fn);
                        assert_eq!(n_fn, want_n, "fn len={len} cw={cw} cap={cap}");
                        assert_eq!(&got_fn[..n_fn], &want_broad[..want_n], "fn matches len={len} cap={cap}");
                    }

                    for idx in [0usize, 7, 8, 10, 11, 15, 16, 17, 18, 35, 36, 46, 94] {
                        let w0 = u32::from_le_bytes([all[idx][0], all[idx][1], all[idx][2], all[idx][3]]);
                        let mut got = [(0u32, [0u8; OUT_LEN]); 1];
                        let n = tree.find_counter_range(&tmpl, cw, start, count as u64, u32::MAX, w0, &mut got);
                        assert_eq!(n, 1, "exact hit len={len} idx={idx}");
                        assert_eq!(got[0], (wrapping_add_usize(start, idx), all[idx]), "exact hit val len={len} idx={idx}");
                    }

                    let mut miss_target = 0xdead_beefu32;
                    while all.iter().any(|d| u32::from_le_bytes([d[0], d[1], d[2], d[3]]) == miss_target) {
                        miss_target = miss_target.wrapping_add(1);
                    }
                    let mut got = [(0u32, [0u8; OUT_LEN]); 4];
                    assert_eq!(tree.find_counter_range(&tmpl, cw, start, count as u64, u32::MAX, miss_target, &mut got), 0);
                }
            }
        }

        // Exercise multi-group secondary SIMD `find` directly (AVX2 on AVX-512
        // hosts; NEON on SME2 hosts) so `g_idx * lanes`, `*found != matches.len()`,
        // and `rem < lanes` are tested at `g_idx >= 2` on both hit and miss paths.
        #[cfg(target_arch = "x86_64")]
        if let Some(avx2) = Platform::avx2() {
            let tmpl = [0x47u8; 36];
            let start = 100u32;
            let all = official_counter_range(Mode::Plain, &tmpl, 8, start, 35);
            let vt = ValidatedTemplate::new(&tmpl, 8);
            let w0 = u32::from_le_bytes(all[25][..4].try_into().unwrap());
            let mut got = [(0u32, [0u8; OUT_LEN]); 1];
            crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
            assert_eq!(find_counter_range_on(vt, start, 35, u32::MAX, w0, &crate::IV, 0, &mut got, avx2), 1);
            assert_eq!(got[0], (start + 25, all[25]));
            assert!(crate::RAN_ENTRIES.with(|r| r.borrow().contains(&(arch::avx2_find_kernel(36) as *const () as usize))));
            // Verify miss advances `off` by exact `4 * 8 = 32` (not `4 + 8 = 12`).
            let mut miss_t = 0xfeed_faceu32;
            while all.iter().any(|d| u32::from_le_bytes(d[..4].try_into().unwrap()) == miss_t) {
                miss_t = miss_t.wrapping_add(1);
            }
            let mut block = [0u8; BLOCK_LEN];
            block[..36].copy_from_slice(&tmpl);
            let words = words_from_block(&block);
            let ctx = FindHitCtx { counter_word: 8, cw_off: 32, blen_u8: 36, key: &crate::IV, root_flags: CHUNK_START + CHUNK_END + ROOT, platform: avx2 };
            let mut found = 0usize;
            assert_eq!(arch::find_counter_groups(&words, &ctx, start, 35, u32::MAX, miss_t, &mut block, &mut got, &mut found), 32);
            assert_eq!(found, 0);
            // Single exact group (`count == 8`) must run AVX2 `find`.
            crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
            assert_eq!(arch::find_counter_groups(&words, &ctx, start, 8, u32::MAX, miss_t, &mut block, &mut got, &mut found), 8);
            assert!(crate::RAN_ENTRIES.with(|r| r.borrow().contains(&(arch::avx2_find_kernel(36) as *const () as usize))));
        }
        #[cfg(target_arch = "aarch64")]
        if let Some(neon) = Platform::neon() {
            let tmpl = [0x47u8; 36];
            let start = 100u32;
            let all = official_counter_range(Mode::Plain, &tmpl, 8, start, 35);
            let mut miss_t = 0xfeed_faceu32;
            while all.iter().any(|d| u32::from_le_bytes(d[..4].try_into().unwrap()) == miss_t) {
                miss_t = miss_t.wrapping_add(1);
            }
            let mut block = [0u8; BLOCK_LEN];
            block[..36].copy_from_slice(&tmpl);
            let words = words_from_block(&block);
            let ctx = FindHitCtx { counter_word: 8, cw_off: 32, blen_u8: 36, key: &crate::IV, root_flags: CHUNK_START + CHUNK_END + ROOT, platform: neon };
            let mut got = [(0u32, [0u8; OUT_LEN]); 1];
            let mut found = 0usize;
            assert_eq!(arch::find_counter_groups(&words, &ctx, start, 35, u32::MAX, miss_t, &mut block, &mut got, &mut found), 33);
            assert_eq!(found, 0);
            crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
            assert_eq!(arch::find_counter_groups(&words, &ctx, start, 11, u32::MAX, miss_t, &mut block, &mut got, &mut found), 11);
            assert!(crate::RAN_ENTRIES.with(|r| r.borrow().contains(&(arch::neon_find_kernel(36) as *const () as usize))));
        }
    }

    #[test]
    fn find_counter_range_impossible_predicate_and_empty_short_circuit_without_kernels() {
        crate::initialize();
        let tmpl = [0x11u8; 52];
        let mut out = [(0u32, [0u8; OUT_LEN]); 4];
        crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
        // Bounded count (1000) so a mutant that skips `find_is_noop` fails
        // the `RAN_ENTRIES.is_empty()` assertion promptly instead of timing out.
        assert_eq!(find_counter_range(Mode::Plain, &tmpl, 12, 0, 1000, 0x0000_00ff, 0x0000_0100, &mut out), 0);
        assert_eq!(find_counter_range(Mode::Plain, &tmpl, 12, 0, 0, 0, 0, &mut out), 0);
        assert_eq!(find_counter_range(Mode::Plain, &tmpl, 12, 0, 1000, 0, 0, &mut []), 0);
        assert_eq!(TreeHasher::new(Mode::Plain).find_counter_range(&tmpl, 12, 0, 1000, 0x0000_00ff, 0x0000_0100, &mut out), 0);
        assert_eq!(TreeHasher::new(Mode::Plain).find_counter_range(&tmpl, 12, 0, 0, 0, 0, &mut out), 0);
        assert_eq!(TreeHasher::new(Mode::Plain).find_counter_range(&tmpl, 12, 0, 1000, 0, 0, &mut []), 0);
        assert!(crate::RAN_ENTRIES.with(|r| r.borrow().is_empty()), "short-circuited find must not launch kernels");
    }

    #[test]
    fn counter_range_covers_every_counter_word_0_to_15() {
        crate::initialize();
        let mut tmpl = [0u8; 64];
        for (j, b) in tmpl.iter_mut().enumerate() {
            *b = (j.wrapping_mul(23) ^ 0xc3).to_le_bytes()[0];
        }
        for cw in 0..16 {
            let start = u32::MAX - 9;
            let mut got = [[0u8; OUT_LEN]; 49];
            hash_counter_range(Mode::Plain, &tmpl, cw, start, &mut got);
            let want = official_counter_range(Mode::Plain, &tmpl, cw, start, 49);
            assert_eq!(&got[..], &want[..], "cw={cw}");
        }
        for count in [48usize, 54, 256] {
            let mut got = std::vec![[0u8; OUT_LEN]; count];
            TreeHasher::new(Mode::Plain).hash_counter_range(&tmpl[..52], 12, u32::MAX - 30, &mut got);
            let want = official_counter_range(Mode::Plain, &tmpl[..52], 12, u32::MAX - 30, count);
            assert_eq!(got, want, "multi-group count={count}");
        }
    }

    #[test]
    fn counter_and_find_dispatch_to_pruned_tier_kernels() {
        crate::initialize();
        let platform = Platform::detect();
        let group_len: usize = match platform {
            #[cfg(target_arch = "aarch64")]
            Platform::SME2 { .. } => 18,
            #[cfg(target_arch = "aarch64")]
            Platform::NEON { .. } => 11,
            #[cfg(target_arch = "x86_64")]
            Platform::AVX512 { .. } => 16,
            #[cfg(target_arch = "x86_64")]
            Platform::AVX2 { .. } => 8,
            #[cfg(target_arch = "x86_64")]
            Platform::SSE41 { .. } => return,
        };

        for &(len, cw) in &[(16usize, 3usize), (36, 8), (52, 12), (64, 15)] {
            let entry_counter: usize = match platform {
                #[cfg(target_arch = "aarch64")]
                Platform::SME2 { .. } => arch::sme2_counter_kernel(len as u8) as *const () as usize,
                #[cfg(target_arch = "aarch64")]
                Platform::NEON { .. } => arch::neon_counter_kernel(len as u8) as *const () as usize,
                #[cfg(target_arch = "x86_64")]
                Platform::AVX512 { .. } => arch::avx512_counter_kernel(len as u8) as *const () as usize,
                #[cfg(target_arch = "x86_64")]
                Platform::AVX2 { .. } => arch::avx2_counter_kernel(len as u8) as *const () as usize,
                #[cfg(target_arch = "x86_64")]
                Platform::SSE41 { .. } => unreachable!(),
            };
            let entry_find: usize = match platform {
                #[cfg(target_arch = "aarch64")]
                Platform::SME2 { .. } => arch::sme2_find_kernel(len as u8) as *const () as usize,
                #[cfg(target_arch = "aarch64")]
                Platform::NEON { .. } => arch::neon_find_kernel(len as u8) as *const () as usize,
                #[cfg(target_arch = "x86_64")]
                Platform::AVX512 { .. } => arch::avx512_find_kernel(len as u8) as *const () as usize,
                #[cfg(target_arch = "x86_64")]
                Platform::AVX2 { .. } => arch::avx2_find_kernel(len as u8) as *const () as usize,
                #[cfg(target_arch = "x86_64")]
                Platform::SSE41 { .. } => unreachable!(),
            };

            let tmpl = std::vec![0x3cu8; len];
            let mut out32 = std::vec![[0u8; OUT_LEN]; group_len];
            crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
            hash_counter_range(Mode::Plain, &tmpl, cw, 100, &mut out32);
            let ran = crate::RAN_ENTRIES.with(|r| r.borrow().clone());
            assert!(ran.contains(&entry_counter) && ran.len() == 1, "counter32 len={len}: {ran:?}");

            let mut out16 = std::vec![[0u8; TRUNC16_LEN]; 2 * group_len];
            crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
            TreeHasher::new(Mode::Plain).hash_counter_range_trunc16(&tmpl, cw, 100, &mut out16);
            let ran = crate::RAN_ENTRIES.with(|r| r.borrow().clone());
            assert!(ran.contains(&entry_counter) && ran.len() == 1, "counter16 len={len}: {ran:?}");

            let mut hits = [(0u32, [0u8; OUT_LEN]); 1];
            for find_count in [group_len, 2 * group_len] {
                crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
                let _ = find_counter_range(Mode::Plain, &tmpl, cw, 100, find_count as u64, u32::MAX, 0xdead_beef, &mut hits);
                let ran = crate::RAN_ENTRIES.with(|r| r.borrow().clone());
                assert!(ran.contains(&entry_find) && ran.len() == 1, "find len={len} count={find_count}: {ran:?}");
            }
        }

        #[cfg(target_arch = "aarch64")]
        if matches!(platform, Platform::SME2 { .. }) {
            let tmpl = [0x3cu8; 52];
            let neon_c = arch::neon_counter_kernel(52) as *const () as usize;
            let sme_c = arch::sme2_counter_kernel(52) as *const () as usize;
            let mut out = [[0u8; OUT_LEN]; 33];
            crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
            TreeHasher::new(Mode::Plain).on_this_core().hash_counter_range(&tmpl, 12, 100, &mut out);
            let ran = crate::RAN_ENTRIES.with(|r| r.borrow().clone());
            assert!(ran.contains(&neon_c) && !ran.contains(&sme_c), "on_this_core counter must route to NEON");

            let neon_f = arch::neon_find_kernel(52) as *const () as usize;
            let sme_f = arch::sme2_find_kernel(52) as *const () as usize;
            let mut hits = [(0u32, [0u8; OUT_LEN]); 1];
            crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
            let _ = TreeHasher::new(Mode::Plain).on_this_core().find_counter_range(&tmpl, 12, 100, 33, u32::MAX, 0xdead_beef, &mut hits);
            let ran = crate::RAN_ENTRIES.with(|r| r.borrow().clone());
            assert!(ran.contains(&neon_f) && !ran.contains(&sme_f), "on_this_core find must route to NEON");
        }
    }

    #[test]
    fn counter_range_and_trunc16_respect_guard_pages() {
        use crate::test::guard::Guarded;
        crate::initialize();
        let platform = Platform::detect();
        for len in [4usize, 16, 36, 52, 64] {
            for count in [8usize, 11, 16, 18, 22, 36, 47] {
                for at_end in [false, true] {
                    let mut tmpl = Guarded::new(len, at_end);
                    for (i, b) in tmpl.bytes().iter_mut().enumerate() {
                        *b = (i.wrapping_mul(31) ^ 0x6d).to_le_bytes()[0];
                    }
                    let cw = (len / 4) - 1;
                    let want = official_counter_range(Mode::Plain, tmpl.bytes(), cw, u32::MAX - 3, count);

                    let mut out32 = Guarded::new(count * OUT_LEN, at_end);
                    let out32_slice = out32.bytes().as_chunks_mut::<OUT_LEN>().0;
                    hash_counter_range_on(ValidatedTemplate::new(tmpl.bytes(), cw), u32::MAX - 3, &crate::IV, 0, out32_slice, platform);
                    assert_eq!(out32_slice, &want[..], "32B len={len} count={count} at_end={at_end}");

                    let mut out16 = Guarded::new(count * TRUNC16_LEN, at_end);
                    let out16_slice = out16.bytes().as_chunks_mut::<TRUNC16_LEN>().0;
                    hash_counter_range_on(ValidatedTemplate::new(tmpl.bytes(), cw), u32::MAX - 3, &crate::IV, 0, out16_slice, platform);
                    for (got, w) in out16_slice.iter().zip(want.iter()) {
                        assert_eq!(got, &w[..TRUNC16_LEN], "16B len={len} count={count} at_end={at_end}");
                    }
                }
            }
        }
    }

    #[test]
    #[should_panic(expected = "fit in one block")]
    fn counter_range_rejects_template_over_block_even_when_empty() {
        crate::initialize();
        hash_counter_range(Mode::Plain, &[0u8; 65], 0, 0, &mut []);
    }

    #[test]
    #[should_panic(expected = "fit in one block")]
    fn counter_range_trunc16_rejects_template_over_block_even_when_empty() {
        crate::initialize();
        hash_counter_range_trunc16(Mode::Plain, &[0u8; 65], 0, 0, &mut []);
    }

    #[test]
    #[should_panic(expected = "fit in one block")]
    fn find_counter_range_rejects_template_over_block_even_when_empty() {
        crate::initialize();
        let _ = find_counter_range(Mode::Plain, &[0u8; 65], 0, 0, 0, 0, 0, &mut []);
    }

    #[test]
    #[should_panic(expected = "out of bounds")]
    fn counter_range_rejects_counter_word_past_template_even_when_empty() {
        crate::initialize();
        hash_counter_range(Mode::Plain, &[0u8; 7], 1, 0, &mut []);
    }

    #[test]
    #[should_panic(expected = "out of bounds")]
    fn find_counter_range_rejects_counter_word_past_template_even_when_empty() {
        crate::initialize();
        let _ = TreeHasher::new(Mode::Plain).find_counter_range(&[0u8; 7], 1, 0, 0, 0, 0, &mut []);
    }

    #[test]
    #[should_panic(expected = "out of bounds")]
    fn counter_range_rejects_short_template_under_4_bytes_even_when_empty() {
        crate::initialize();
        TreeHasher::new(Mode::Plain).hash_counter_range(&[0u8; 3], 0, 0, &mut []);
    }

    #[test]
    #[should_panic(expected = "out of bounds")]
    fn counter_range_rejects_counter_word_16_even_when_empty() {
        crate::initialize();
        TreeHasher::new(Mode::Plain).hash_counter_range(&[0u8; 64], 16, 0, &mut []);
    }
}
