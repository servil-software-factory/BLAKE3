//! Fused multi-step 32-byte hash-chain SIMD kernels for
//! [`crate::throughput::TreeHasher::hash_chains_32_trunc16`] and
//! [`crate::throughput::TreeHasher::hash_chains_varlen_32_trunc16`] across
//! AArch64 18-lane (16 SME2 + 2 integer) hybrid, AArch64 NEON 8+3 (11 lanes),
//! x86-64 AVX-512 (16 lanes), and x86-64 AVX2 (8 lanes) (#584).
//!
//! Each 32-byte input slot holds `[prefix12 || start_step_u32_le || state16]`.
//! Each step `s in 0..steps` advances `state` inside one kernel call (kept in
//! registers on AVX-512 and SME2 vector lanes, or L1 stack slots on AVX2, NEON,
//! and scalar lanes) without returning to caller buffers or repeating
//! `smstart`/`smstop` or zero-word (`8..15`) message additions:
//!
//! ```text
//! state_{s+1} = H_mode(prefix12 || (start_step.wrapping_add(s)) || state_s)[..16]
//! ```
//!
//! All dispatch, padding, and longest-processing-time (LPT) lane-refilling
//! decisions depend solely on `init.len()`, `steps`, `Mode` discriminant, and
//! `Platform` — never on key bytes or chain values.

use crate::{BLOCK_LEN, CHUNK_END, CHUNK_START, CVWords, ROOT, platform::Platform};

#[cfg(target_arch = "aarch64")]
#[path = "ffi_chain_aarch64.rs"]
mod arch;
#[cfg(target_arch = "x86_64")]
#[path = "ffi_chain_x86.rs"]
mod arch;

#[cfg(all(test, target_arch = "aarch64"))]
pub(crate) use arch::{call_sme2_chain, entries_neon, entries_sme2};
#[cfg(all(test, target_arch = "x86_64"))]
pub(crate) use arch::{entries_avx2, entries_avx512};

/// Width of each 32-byte chain input `[prefix12 || step_u32_le || state16]`.
pub(crate) const CHAIN_IN_LEN: usize = 32;
/// Width of each 16-byte truncated chain state / digest.
pub(crate) const CHAIN_OUT_LEN: usize = 16;

/// Minimum chain count to enter the 18-lane (16 SME2 + 2 integer) hybrid kernel.
pub(crate) const SME2_CHAIN_MIN: usize = 18;

/// Minimum total compressions for `hash_chains_varlen_32_trunc16` to engage
/// SME2 (18-lane hybrid) rather than staying on NEON (11-lane) and avoiding
/// `smstart`/`smstop` entry cost on short verification batches (Apple M6
/// `host_lab`: NEON wins WOTS+C 21 [597 comps, 6.6 vs 9.3 us] and U-PORS 27
/// [762 comps, 8.8 vs 9.6 us], while SME2 wins Batch 126 [3,687 comps,
/// 36.3 vs 40.3 us]).
pub(crate) const SME2_VARLEN_MIN_WORK: u64 = 18 * 64;

const ROOT_BLOCK_FLAGS: u8 = CHUNK_START + CHUNK_END + ROOT;

#[inline]
fn root_flags_u32(flags: u8) -> u32 {
    u32::from(flags + ROOT_BLOCK_FLAGS)
}

#[inline]
fn advance_step_in_slot(slot: &mut [u8; CHAIN_IN_LEN], delta: u32) {
    let cur = u32::from_le_bytes([slot[12], slot[13], slot[14], slot[15]]);
    let next = cur.wrapping_add(delta).to_le_bytes();
    slot[12..16].copy_from_slice(&next);
}

#[inline]
fn hash_chain_serial_one(init: &[u8; CHAIN_IN_LEN], steps: u32, key: &CVWords, flags: u8, platform: Platform) -> [u8; CHAIN_OUT_LEN] {
    if steps == 0 {
        return [
            init[16], init[17], init[18], init[19], init[20], init[21], init[22], init[23], init[24], init[25], init[26], init[27], init[28], init[29],
            init[30], init[31],
        ];
    }
    let mut block = [0u8; BLOCK_LEN];
    block[..CHAIN_IN_LEN].copy_from_slice(init);
    let mut step = u32::from_le_bytes([init[12], init[13], init[14], init[15]]);
    let root_flags = flags + ROOT_BLOCK_FLAGS;
    for _ in 0..steps {
        block[12..16].copy_from_slice(&step.to_le_bytes());
        let mut cv = *key;
        platform.compress_in_place(&mut cv, &block, 32, 0, root_flags);
        let bytes = crate::hasher::le_bytes(&cv);
        block[16..32].copy_from_slice(&bytes[..CHAIN_OUT_LEN]);
        step = step.wrapping_add(1);
    }
    let mut out = [0u8; CHAIN_OUT_LEN];
    out.copy_from_slice(&block[16..32]);
    out
}

/// Evaluate `init.len()` fixed-step 32-byte hash chains on `platform`, writing
/// each chain's final 16-byte state to `out`.
pub(crate) fn hash_chains_32_trunc16_on(
    init: &[[u8; CHAIN_IN_LEN]],
    steps: u32,
    key: &CVWords,
    flags: u8,
    out: &mut [[u8; CHAIN_OUT_LEN]],
    platform: Platform,
) {
    assert_eq!(init.len(), out.len(), "one 16-byte output per chain");
    if init.is_empty() {
        return;
    }
    if steps == 0 {
        for (src, dst) in init.iter().zip(out.iter_mut()) {
            dst.copy_from_slice(&src[16..32]);
        }
        return;
    }

    let rflags = root_flags_u32(flags);
    let done = arch::hash_chain_groups(init, steps, key, rflags, out, platform);

    for (src, dst) in init[done..].iter().zip(out[done..].iter_mut()) {
        *dst = hash_chain_serial_one(src, steps, key, flags, platform);
    }
}

/// Whether `hash_chains_varlen_32_trunc16` has enough active chains and total
/// compressions to justify taking the shared SME2 turn.
#[inline]
pub(crate) fn varlen_sme2_sized(steps: &[u32]) -> bool {
    let mut active = 0usize;
    let mut total = 0u64;
    for &s in steps {
        if s > 0 {
            active += 1;
            total = total.saturating_add(u64::from(s));
        }
    }
    active >= SME2_CHAIN_MIN && total >= SME2_VARLEN_MIN_WORK
}

/// Native single-group SIMD lane width used by the continuous LPT scheduler.
#[inline]
fn varlen_lane_width(platform: Platform, active_count: usize, total_work: u64) -> usize {
    arch::varlen_lane_width(platform, active_count, total_work)
}

/// Evaluate `init.len()` variable-step 32-byte hash chains on `platform` using
/// a longest-processing-time-first (LPT) continuous lane-refilling scheduler
/// over fused multi-step SIMD bursts of `min(remaining_steps)` iterations.
///
/// Uses stack storage (zero heap allocations) for up to 128 nonzero chains,
/// covering typical hash-based signature verification waves (e.g., 21 WOTS+C,
/// 27 U-PORS, or 126 batched chains).
pub(crate) fn hash_chains_varlen_32_trunc16_on(
    init: &[[u8; CHAIN_IN_LEN]],
    steps: &[u32],
    key: &CVWords,
    flags: u8,
    out: &mut [[u8; CHAIN_OUT_LEN]],
    platform: Platform,
) {
    assert_eq!(init.len(), steps.len(), "one step count per chain");
    assert_eq!(init.len(), out.len(), "one 16-byte output per chain");
    if init.is_empty() {
        return;
    }

    const STACK_QUEUE_CAP: usize = 128;
    let mut stack_order = [0usize; STACK_QUEUE_CAP];
    let mut heap_order = std::vec::Vec::new();
    let mut nonzero = 0usize;
    let mut total_work = 0u64;

    for (i, &st) in steps.iter().enumerate() {
        if st == 0 {
            out[i].copy_from_slice(&init[i][16..32]);
        } else {
            total_work = total_work.saturating_add(u64::from(st));
            if nonzero < STACK_QUEUE_CAP && heap_order.is_empty() {
                stack_order[nonzero] = i;
            } else {
                if heap_order.is_empty() {
                    heap_order.reserve(init.len());
                    heap_order.extend_from_slice(&stack_order[..nonzero]);
                }
                heap_order.push(i);
            }
            nonzero += 1;
        }
    }
    if nonzero == 0 {
        return;
    }

    let order: &mut [usize] = if heap_order.is_empty() { &mut stack_order[..nonzero] } else { &mut heap_order[..] };
    // LPT schedule (longest chains first): depends solely on public `steps`,
    // keeping SIMD lanes balanced until the final few steps of the wave.
    order.sort_unstable_by(|&a, &b| steps[b].cmp(&steps[a]));

    let width = varlen_lane_width(platform, nonzero, total_work);
    if width == 1 {
        for &idx in order.iter() {
            out[idx] = hash_chain_serial_one(&init[idx], steps[idx], key, flags, platform);
        }
        return;
    }

    let mut active_buf = [[0u8; CHAIN_IN_LEN]; 18];
    let mut active_out = [[0u8; CHAIN_OUT_LEN]; 18];
    let mut active_rem = [0u32; 18];
    let mut active_idx = [0usize; 18];
    let mut active_len = width.min(order.len());
    let mut pending = order[active_len..].iter().copied();

    for (slot, &idx) in order[..active_len].iter().enumerate() {
        active_buf[slot] = init[idx];
        active_rem[slot] = steps[idx];
        active_idx[slot] = idx;
    }

    // Keep lanes packed into fused multi-step SIMD bursts while at least
    // `arch::PAD_SIMD_MIN` chains remain active; drain any smaller tail
    // serially after the loop.
    while active_len >= arch::PAD_SIMD_MIN {
        let d = active_rem[..active_len].iter().copied().min().expect("active_len >= PAD_SIMD_MIN");

        hash_chains_32_trunc16_on(&active_buf[..active_len], d, key, flags, &mut active_out[..active_len], platform);

        // Scan in reverse so swap-removing a finished slot moves an already-
        // updated slot into `j` without re-visiting it.
        for j in (0..active_len).rev() {
            let left = active_rem[j].checked_sub(d).expect("d is min of active_rem");
            if left == 0 {
                out[active_idx[j]] = active_out[j];
                if let Some(idx) = pending.next() {
                    active_buf[j] = init[idx];
                    active_rem[j] = steps[idx];
                    active_idx[j] = idx;
                } else {
                    active_len -= 1;
                    active_buf[j] = active_buf[active_len];
                    active_rem[j] = active_rem[active_len];
                    active_idx[j] = active_idx[active_len];
                }
            } else {
                active_buf[j][16..32].copy_from_slice(&active_out[j]);
                advance_step_in_slot(&mut active_buf[j], d);
                active_rem[j] = left;
            }
        }
    }

    for j in 0..active_len {
        out[active_idx[j]] = hash_chain_serial_one(&active_buf[j], active_rem[j], key, flags, platform);
    }
}

#[cfg(test)]
pub(crate) static PLANTED_WRONG_CHAIN: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

const SELF_TEST_COUNT: usize = 40;
const SELF_TEST_STEPS: u32 = 5;
const SELF_TEST_KEY: [u8; 32] = [
    0x23, 0x37, 0x4b, 0x5f, 0x73, 0x87, 0x9b, 0xaf, 0xc3, 0xd7, 0xeb, 0xff, 0x13, 0x27, 0x3b, 0x4f, 0x63, 0x77, 0x8b, 0x9f, 0xb3, 0xc7, 0xdb, 0xef, 0x05, 0x19,
    0x2d, 0x41, 0x55, 0x69, 0x7d, 0x91,
];

fn self_test_init() -> [[u8; CHAIN_IN_LEN]; SELF_TEST_COUNT] {
    let mut init = [[0u8; CHAIN_IN_LEN]; SELF_TEST_COUNT];
    for (i, slot) in init.iter_mut().enumerate() {
        for (j, b) in slot.iter_mut().enumerate() {
            *b = crate::low_byte(i.wrapping_mul(29) ^ j.wrapping_mul(17) ^ 0x6b);
        }
        // Start steps straddling u32::MAX to exercise wrapping u32 step overflow on every ISA.
        let start = u32::MAX.wrapping_sub(2).wrapping_add(u32::try_from(i % 5).expect("i % 5 fits u32"));
        slot[12..16].copy_from_slice(&start.to_le_bytes());
    }
    init
}

/// Fold of `40` keyed 5-step 32-byte hash chains starting near `u32::MAX - 2`
/// from official BLAKE3 1.8.7.
/// `40 = 2*18 + 4` (SME2 multi-group + padded NEON), `3*11 + 7` (NEON),
/// `2*16 + 8` (AVX-512 multi-group + AVX2), `5*8` (AVX2),
/// exercising every fused chain kernel on its platform in one call.
const SELF_TEST_EXPECTED: u64 = 0x6aba_f1ba_bf9a_20a9;

#[inline(never)]
pub(crate) fn self_test(platform: Platform) -> usize {
    let key = crate::platform::words_from_le_bytes_32(&SELF_TEST_KEY);
    let init = self_test_init();
    let mut out = [[0u8; CHAIN_OUT_LEN]; SELF_TEST_COUNT];
    hash_chains_32_trunc16_on(&init, SELF_TEST_STEPS, &key, crate::KEYED_HASH, &mut out, platform);
    #[cfg(test)]
    if PLANTED_WRONG_CHAIN.load(core::sync::atomic::Ordering::Relaxed) {
        out[0][0] = !out[0][0];
    }
    let got = crate::self_test::fold(out.as_flattened());
    if got != SELF_TEST_EXPECTED {
        crate::self_test::fail(format_args!(
            "blake3-servil startup self-test failed: hash_chains_32_trunc16 (platform {}): \
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
    use crate::throughput::{TreeHasher, hash_chains_32_trunc16, hash_chains_varlen_32_trunc16};

    fn official_chain(mode: Mode, init: &[u8; CHAIN_IN_LEN], steps: u32) -> [u8; CHAIN_OUT_LEN] {
        let mut buf = *init;
        let mut step = u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
        for _ in 0..steps {
            buf[12..16].copy_from_slice(&step.to_le_bytes());
            let h = crate::test::official::hash(mode, &buf);
            buf[16..32].copy_from_slice(&h[..CHAIN_OUT_LEN]);
            step = step.wrapping_add(1);
        }
        buf[16..32].try_into().unwrap()
    }

    fn make_init(count: usize) -> std::vec::Vec<[u8; CHAIN_IN_LEN]> {
        (0..count)
            .map(|i| {
                let mut slot = [0u8; CHAIN_IN_LEN];
                slot[..4].copy_from_slice(&(i as u32).to_le_bytes());
                for (j, b) in slot[4..].iter_mut().enumerate() {
                    *b = ((i.wrapping_mul(43) ^ j.wrapping_mul(19) ^ 0x5d) & 0xff) as u8;
                }
                if i % 7 == 0 {
                    slot[12..16].copy_from_slice(&(u32::MAX - 2).to_le_bytes());
                }
                slot
            })
            .collect()
    }

    #[test]
    fn chain_self_test_expected_matches_official_reference() {
        let init = self_test_init();
        let expected_out: std::vec::Vec<[u8; CHAIN_OUT_LEN]> = init.iter().map(|s| official_chain(Mode::Keyed(&SELF_TEST_KEY), s, SELF_TEST_STEPS)).collect();
        let want = crate::self_test::fold(expected_out.as_flattened());
        assert_eq!(SELF_TEST_EXPECTED, want, "update SELF_TEST_EXPECTED to {want:#018x}");
    }

    #[test]
    fn fixed_chains_match_official_across_counts_steps_and_modes() {
        crate::initialize();
        let key = [0x51u8; 32];
        let ctx = "blake3-servil fused hash chain test";
        let detected = Platform::detect();
        let own = Platform::own_core();
        let platforms: &[Platform] = if own == detected { &[detected] } else { &[detected, own] };
        for mode in [Mode::Plain, Mode::Keyed(&key), Mode::DeriveKey(ctx)] {
            let tree = TreeHasher::new(mode);
            let local = TreeHasher::new(mode).on_this_core();
            for count in [0usize, 1, 3, 4, 7, 8, 10, 11, 15, 16, 17, 18, 19, 22, 29, 36, 47, 65] {
                let init = make_init(count);
                for steps in [0u32, 1, 2, 5, 13, 59] {
                    let want: std::vec::Vec<[u8; CHAIN_OUT_LEN]> = init.iter().map(|s| official_chain(mode, s, steps)).collect();
                    let mut out = std::vec![[0u8; CHAIN_OUT_LEN]; count];
                    tree.hash_chains_32_trunc16(&init, steps, &mut out);
                    assert_eq!(out, want, "TreeHasher count={count} steps={steps}");
                    let mut out_local = std::vec![[0u8; CHAIN_OUT_LEN]; count];
                    local.hash_chains_32_trunc16(&init, steps, &mut out_local);
                    assert_eq!(out_local, want, "on_this_core count={count} steps={steps}");
                    if steps <= 5 {
                        let mut out_free = std::vec![[0u8; CHAIN_OUT_LEN]; count];
                        hash_chains_32_trunc16(mode, &init, steps, &mut out_free);
                        assert_eq!(out_free, want, "free fn count={count} steps={steps}");
                    }
                    let (kw, fl) = mode.key_and_flags();
                    for &p in platforms {
                        let mut out_p = std::vec![[0u8; CHAIN_OUT_LEN]; count];
                        hash_chains_32_trunc16_on(&init, steps, &kw, fl, &mut out_p, p);
                        assert_eq!(out_p, want, "platform={} count={count} steps={steps}", p.name());
                    }
                }
            }
        }
    }

    #[test]
    fn varlen_chains_match_official_across_shapes_and_modes() {
        crate::initialize();
        let key = [0x93u8; 32];
        let ctx = "blake3-servil varlen hash chain test";
        for mode in [Mode::Plain, Mode::Keyed(&key), Mode::DeriveKey(ctx)] {
            let tree = TreeHasher::new(mode);
            let local = TreeHasher::new(mode).on_this_core();
            // Cover empty, all-zero, single, small, BLINCS 21-WOTS+C / 27-U-PORS /
            // 126-batched shapes, and >128 heap-queue spillover (160 chains).
            for count in [0usize, 1, 3, 4, 8, 11, 16, 18, 21, 27, 47, 126, 160] {
                let init = make_init(count);
                let steps: std::vec::Vec<u32> = (0..count)
                    .map(|i| match i % 9 {
                        0 => 0,
                        1 => 1,
                        2 => 2,
                        3 => 5,
                        4 => 11,
                        5 => 23,
                        6 => 37,
                        7 => 59,
                        _ => (i as u32) % 60,
                    })
                    .collect();
                let want: std::vec::Vec<[u8; CHAIN_OUT_LEN]> = init.iter().zip(steps.iter()).map(|(s, &n)| official_chain(mode, s, n)).collect();
                let mut out = std::vec![[0u8; CHAIN_OUT_LEN]; count];
                tree.hash_chains_varlen_32_trunc16(&init, &steps, &mut out);
                assert_eq!(out, want, "varlen TreeHasher count={count}");
                let mut out_local = std::vec![[0u8; CHAIN_OUT_LEN]; count];
                local.hash_chains_varlen_32_trunc16(&init, &steps, &mut out_local);
                assert_eq!(out_local, want, "varlen on_this_core count={count}");
                let mut out_free = std::vec![[0u8; CHAIN_OUT_LEN]; count];
                hash_chains_varlen_32_trunc16(mode, &init, &steps, &mut out_free);
                assert_eq!(out_free, want, "varlen free fn count={count}");
            }
            // All-zero steps fast path.
            let init = make_init(21);
            let zeros = std::vec![0u32; 21];
            let want: std::vec::Vec<[u8; CHAIN_OUT_LEN]> = init.iter().map(|s| s[16..32].try_into().unwrap()).collect();
            let mut out = std::vec![[0u8; CHAIN_OUT_LEN]; 21];
            tree.hash_chains_varlen_32_trunc16(&init, &zeros, &mut out);
            assert_eq!(out, want);
            // Exact STACK_QUEUE_CAP (128) and +1 (129) all-nonzero boundary.
            for exact in [128usize, 129] {
                let init_b = make_init(exact);
                let steps_b: std::vec::Vec<u32> = (0..exact).map(|i| 1 + (i as u32 % 5)).collect();
                let want_b: std::vec::Vec<[u8; CHAIN_OUT_LEN]> = init_b.iter().zip(steps_b.iter()).map(|(s, &n)| official_chain(mode, s, n)).collect();
                let mut out_b = std::vec![[0u8; CHAIN_OUT_LEN]; exact];
                tree.hash_chains_varlen_32_trunc16(&init_b, &steps_b, &mut out_b);
                assert_eq!(out_b, want_b, "varlen exact stack boundary={exact}");
            }
        }
    }

    #[test]
    fn chain_kernels_dispatch_and_respect_guard_pages() {
        use crate::test::guard::Guarded;
        crate::initialize();
        let platform = Platform::detect();
        for count in [1usize, 4, 8, 11, 16, 18, 22, 36, 47] {
            for at_end in [false, true] {
                let mut init = Guarded::new(count * CHAIN_IN_LEN, at_end);
                for (i, slot) in init.bytes().chunks_exact_mut(CHAIN_IN_LEN).enumerate() {
                    slot[..4].copy_from_slice(&(i as u32).to_le_bytes());
                    for (j, b) in slot[4..].iter_mut().enumerate() {
                        *b = ((i.wrapping_mul(31) ^ j.wrapping_mul(13) ^ 0x42) & 0xff) as u8;
                    }
                }
                let init_slice = init.bytes().as_chunks::<CHAIN_IN_LEN>().0;
                let mut out = Guarded::new(count * CHAIN_OUT_LEN, at_end);
                let out_slice = out.bytes().as_chunks_mut::<CHAIN_OUT_LEN>().0;
                hash_chains_32_trunc16_on(init_slice, 5, &crate::IV, 0, out_slice, platform);
                for (i, (src, got)) in init_slice.iter().zip(out_slice.iter()).enumerate() {
                    assert_eq!(*got, official_chain(Mode::Plain, src, 5), "count={count} at_end={at_end} i={i}");
                }
            }
        }
    }

    #[test]
    #[should_panic(expected = "one 16-byte output per chain")]
    fn fixed_chains_reject_mismatched_out_len() {
        crate::initialize();
        let init = make_init(3);
        let mut out = [[0u8; CHAIN_OUT_LEN]; 2];
        TreeHasher::new(Mode::Plain).hash_chains_32_trunc16(&init, 1, &mut out);
    }

    #[test]
    #[should_panic(expected = "one step count per chain")]
    fn varlen_chains_reject_mismatched_steps_len() {
        crate::initialize();
        let init = make_init(3);
        let mut out = [[0u8; CHAIN_OUT_LEN]; 3];
        TreeHasher::new(Mode::Plain).hash_chains_varlen_32_trunc16(&init, &[1, 2], &mut out);
    }

    #[test]
    #[should_panic(expected = "one 16-byte output per chain")]
    fn varlen_chains_reject_mismatched_out_len() {
        crate::initialize();
        let init = make_init(3);
        let mut out = [[0u8; CHAIN_OUT_LEN]; 2];
        TreeHasher::new(Mode::Plain).hash_chains_varlen_32_trunc16(&init, &[1, 2, 3], &mut out);
    }

    #[test]
    fn varlen_sme2_threshold_and_lane_width_boundaries() {
        // Exact active-count and total-work boundaries for SME2 entry.
        assert!(!varlen_sme2_sized(&[]));
        assert!(!varlen_sme2_sized(&[100; 17]));
        let mut seventeen_active = [100u32; 18];
        seventeen_active[5] = 0;
        assert!(!varlen_sme2_sized(&seventeen_active));
        assert!(!varlen_sme2_sized(&[63; 18])); // 1134 < 1152
        let mut barely_under = [64u32; 18];
        barely_under[0] = 63; // 1151 < 1152
        assert!(!varlen_sme2_sized(&barely_under));
        assert!(varlen_sme2_sized(&[64; 18])); // exact 18 & 1152
        assert!(varlen_sme2_sized(&[u32::MAX; 18])); // saturating_add overflow safety

        #[cfg(target_arch = "x86_64")]
        {
            if let Some(p) = Platform::avx512() {
                assert_eq!(varlen_lane_width(p, 18, 1152), 16);
            }
            if let Some(p) = Platform::avx2() {
                assert_eq!(varlen_lane_width(p, 18, 1152), 8);
            }
            let sse = Platform::sse41().expect("SSE4.1 platform");
            assert_eq!(varlen_lane_width(sse, 18, 1152), 1);
            // Also verify SSE4.1 serial fallback produces exact digests for fixed & varlen.
            crate::initialize();
            let init = make_init(5);
            let mut out = [[0u8; CHAIN_OUT_LEN]; 5];
            hash_chains_32_trunc16_on(&init, 3, &crate::IV, 0, &mut out, sse);
            let want: std::vec::Vec<_> = init.iter().map(|s| official_chain(Mode::Plain, s, 3)).collect();
            assert_eq!(out.as_slice(), want.as_slice());
            let mut vout = [[0u8; CHAIN_OUT_LEN]; 5];
            hash_chains_varlen_32_trunc16_on(&init, &[0, 1, 2, 3, 4], &crate::IV, 0, &mut vout, sse);
            let vwant: std::vec::Vec<_> = init.iter().enumerate().map(|(i, s)| official_chain(Mode::Plain, s, i as u32)).collect();
            assert_eq!(vout.as_slice(), vwant.as_slice());
        }
        #[cfg(target_arch = "aarch64")]
        {
            assert_eq!(varlen_lane_width(Platform::neon().expect("NEON platform"), 18, 1152), 11);
            if let Some(sme2) = Platform::sme2() {
                assert_eq!(varlen_lane_width(sme2, 17, 2000), 11);
                assert_eq!(varlen_lane_width(sme2, 18, 1151), 11);
                assert_eq!(varlen_lane_width(sme2, 18, 1152), 18);
            }
        }
    }

    #[test]
    fn chain_simd_dispatch_thresholds_hit_expected_kernels() {
        crate::initialize();
        let platform = Platform::detect();
        let init = make_init(36);
        let mut out = [[0u8; CHAIN_OUT_LEN]; 36];

        // `arch::PAD_SIMD_MIN - 1` chains stay on serial (do not enter padded SIMD).
        let below_pad = arch::PAD_SIMD_MIN - 1;
        let at_pad = arch::PAD_SIMD_MIN;
        let below_steps = std::vec![2u32; below_pad];
        let at_steps = std::vec![2u32; at_pad];
        crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
        hash_chains_32_trunc16_on(&init[..below_pad], 2, &crate::IV, 0, &mut out[..below_pad], platform);
        #[cfg(target_arch = "x86_64")]
        {
            let avx2 = entries_avx2()[0].1 as usize;
            let avx512 = entries_avx512()[0].1 as usize;
            assert!(!crate::RAN_ENTRIES.with(|r| r.borrow().contains(&avx2)));
            assert!(!crate::RAN_ENTRIES.with(|r| r.borrow().contains(&avx512)));
            if matches!(platform, Platform::AVX2 { .. } | Platform::AVX512 { .. }) {
                crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
                hash_chains_32_trunc16_on(&init[..at_pad], 2, &crate::IV, 0, &mut out[..at_pad], platform);
                assert!(crate::RAN_ENTRIES.with(|r| r.borrow().contains(&avx2)), "PAD_SIMD_MIN chains must pad into AVX2");
                assert!(!crate::RAN_ENTRIES.with(|r| r.borrow().contains(&avx512)));
                crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
                hash_chains_varlen_32_trunc16_on(&init[..at_pad], &at_steps, &crate::IV, 0, &mut out[..at_pad], platform);
                assert!(crate::RAN_ENTRIES.with(|r| r.borrow().contains(&avx2)), "PAD_SIMD_MIN varlen chains must pad into AVX2");
                crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
                hash_chains_varlen_32_trunc16_on(&init[..below_pad], &below_steps, &crate::IV, 0, &mut out[..below_pad], platform);
                assert!(!crate::RAN_ENTRIES.with(|r| r.borrow().contains(&avx2)), "below-PAD_SIMD_MIN varlen chains stay serial");
                crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
                hash_chains_32_trunc16_on(&init[..8], 2, &crate::IV, 0, &mut out[..8], platform);
                assert!(crate::RAN_ENTRIES.with(|r| r.borrow().contains(&avx2)), "8 chains must run AVX2");
            }
            if matches!(platform, Platform::AVX512 { .. }) {
                crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
                hash_chains_32_trunc16_on(&init[..9], 2, &crate::IV, 0, &mut out[..9], platform);
                assert!(crate::RAN_ENTRIES.with(|r| r.borrow().contains(&avx512)), "9 chains must pad into AVX-512");
                assert!(!crate::RAN_ENTRIES.with(|r| r.borrow().contains(&avx2)));
                crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
                hash_chains_32_trunc16_on(&init[..16], 2, &crate::IV, 0, &mut out[..16], platform);
                assert!(crate::RAN_ENTRIES.with(|r| r.borrow().contains(&avx512)), "16 chains must run AVX-512");
                assert!(!crate::RAN_ENTRIES.with(|r| r.borrow().contains(&avx2)));
            }
        }
        #[cfg(target_arch = "aarch64")]
        {
            let neon = entries_neon()[0].1 as usize;
            let sme2 = entries_sme2()[0].1 as usize;
            assert!(!crate::RAN_ENTRIES.with(|r| r.borrow().contains(&neon)));
            assert!(!crate::RAN_ENTRIES.with(|r| r.borrow().contains(&sme2)));
            crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
            hash_chains_32_trunc16_on(&init[..at_pad], 2, &crate::IV, 0, &mut out[..at_pad], platform);
            assert!(crate::RAN_ENTRIES.with(|r| r.borrow().contains(&neon)), "PAD_SIMD_MIN chains must pad into NEON p11");
            assert!(!crate::RAN_ENTRIES.with(|r| r.borrow().contains(&sme2)));
            crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
            hash_chains_varlen_32_trunc16_on(&init[..at_pad], &at_steps, &crate::IV, 0, &mut out[..at_pad], platform);
            assert!(crate::RAN_ENTRIES.with(|r| r.borrow().contains(&neon)), "PAD_SIMD_MIN varlen chains must pad into NEON p11");
            crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
            hash_chains_varlen_32_trunc16_on(&init[..below_pad], &below_steps, &crate::IV, 0, &mut out[..below_pad], platform);
            assert!(!crate::RAN_ENTRIES.with(|r| r.borrow().contains(&neon)), "below-PAD_SIMD_MIN varlen chains stay serial");
            crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
            hash_chains_32_trunc16_on(&init[..11], 2, &crate::IV, 0, &mut out[..11], platform);
            assert!(crate::RAN_ENTRIES.with(|r| r.borrow().contains(&neon)), "11 chains must run NEON p11");
            if matches!(platform, Platform::SME2 { .. }) {
                crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
                hash_chains_32_trunc16_on(&init[..18], 2, &crate::IV, 0, &mut out[..18], platform);
                assert!(crate::RAN_ENTRIES.with(|r| r.borrow().contains(&sme2)), "18 chains must run SME2 18-lane");
                assert!(!crate::RAN_ENTRIES.with(|r| r.borrow().contains(&neon)));
            }
        }
    }
}
