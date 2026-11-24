//! Arithmetic work-count lifecycle tests (#497, Approved Default 10).
//!
//! BLAKE3's work is countable: for a message or batch of a given shape, the
//! exact number of rows cut, flat walks, held passes, staged copies, shared
//! rounds, and `Platform::hash_many` SIMD calls is fixed by arithmetic. These
//! tests assert those exact counts alongside digest equality on every machine,
//! with no clock.
#![cfg(all(test, feature = "std"))]

use crate::{Counted, Hasher, Mode, STAGE_LEN, platform::Platform, throughput};

fn clear_counts() {
    crate::COUNTED.with(|c| {
        for slot in c {
            slot.set(0);
        }
    });
}

fn count(what: Counted) -> u64 {
    crate::COUNTED.with(|c| c[what as usize].get())
}

#[inline]
fn low_byte(v: usize) -> u8 {
    v.to_le_bytes()[0]
}

/// #497 first case (from review `0d1743a8.report.md` finding 4):
/// Eight hashers fed distinct, aligned 16 KiB pieces in successive turns of
/// `update_each`.
/// - `gather_part` sends aligned whole groups directly to the core without
///   copying into the stage (`Staged == 0`).
/// - On the second 16 KiB turn, platforms whose `simd_degree()` fits `ROW`
///   (x86-64 and NEON) cut exactly 8 rows with zero flat walks and zero held
///   passes (`[8, 0, 0]`). On SME2 (`DEGREE > ROW`, `FLAT_MIN_CHUNKS == 32`),
///   16 KiB is below `flat_takes`, so the second 32 KiB turn exercises SME2's
///   exact 8 flat walks (`[0, 8, 0]`).
#[test]
fn update_each_aligned_16k_turns_cut_exact_rows() {
    crate::initialize();
    const N: usize = 8;
    const PIECE: usize = STAGE_LEN; // 16 KiB
    let pieces_a: Vec<Vec<u8>> = (0..N).map(|h| (0..PIECE).map(|i| low_byte(i.wrapping_mul(31) ^ (h * 17 + 0x5A))).collect()).collect();
    let pieces_b: Vec<Vec<u8>> = (0..N).map(|h| (0..PIECE).map(|i| low_byte(i.wrapping_mul(17) ^ (h * 29 + 0xA5))).collect()).collect();
    let wants: Vec<[u8; 32]> = (0..N)
        .map(|h| {
            let mut full = Vec::with_capacity(2 * PIECE);
            full.extend_from_slice(&pieces_a[h]);
            full.extend_from_slice(&pieces_b[h]);
            crate::test::official::hash(Mode::Plain, &full)
        })
        .collect();

    let mut hashers: Vec<Hasher> = (0..N).map(|_| Hasher::new(Mode::Plain, None)).collect();
    let turn1: Vec<(usize, &[u8])> = (0..N).map(|i| (i, pieces_a[i].as_slice())).collect();
    let turn2: Vec<(usize, &[u8])> = (0..N).map(|i| (i, pieces_b[i].as_slice())).collect();

    throughput::update_each(&mut hashers, &turn1);
    clear_counts();
    throughput::update_each(&mut hashers, &turn2);
    let triple = [count(Counted::Row), count(Counted::Flat), count(Counted::HeldPass)];
    assert_eq!(count(Counted::Staged), 0, "aligned whole 16 KiB turn must not copy into stage");

    #[cfg(blake3_sme2)]
    let on_sme2 = matches!(Platform::detect(), Platform::SME2);
    #[cfg(not(blake3_sme2))]
    let on_sme2 = false;
    let rows_here = (4..=crate::hasher::ROW).contains(&Platform::detect().simd_degree());

    if rows_here {
        assert_eq!(triple, [N as u64, 0, 0], "aligned 16 KiB second turn must cut N rows with zero flat/held passes");
    } else {
        assert_eq!(triple, [0, 0, 0], "SME2 16 KiB is below flat_takes (32 KiB) and above ROW");
    }
    for (i, h) in hashers.iter().enumerate() {
        assert_eq!(h.finalize(), wants[i], "hasher {i}");
    }

    // Also test aligned 32 KiB turns with distinct per-hasher inputs (exercises SME2 flat_takes == 32 KiB).
    let big_a: Vec<Vec<u8>> = (0..N).map(|h| (0..(2 * PIECE)).map(|i| low_byte(i.wrapping_mul(13) ^ (h * 11 + 0x33))).collect()).collect();
    let big_b: Vec<Vec<u8>> = (0..N).map(|h| (0..(2 * PIECE)).map(|i| low_byte(i.wrapping_mul(29) ^ (h * 23 + 0xCC))).collect()).collect();
    let big_wants: Vec<[u8; 32]> = (0..N)
        .map(|h| {
            let mut full = Vec::with_capacity(4 * PIECE);
            full.extend_from_slice(&big_a[h]);
            full.extend_from_slice(&big_b[h]);
            crate::test::official::hash(Mode::Plain, &full)
        })
        .collect();
    let mut big_hashers: Vec<Hasher> = (0..N).map(|_| Hasher::new(Mode::Plain, None)).collect();
    let big_t1: Vec<(usize, &[u8])> = (0..N).map(|i| (i, big_a[i].as_slice())).collect();
    let big_t2: Vec<(usize, &[u8])> = (0..N).map(|i| (i, big_b[i].as_slice())).collect();
    throughput::update_each(&mut big_hashers, &big_t1);
    clear_counts();
    throughput::update_each(&mut big_hashers, &big_t2);
    let big_triple = [count(Counted::Row), count(Counted::Flat), count(Counted::HeldPass)];
    assert_eq!(count(Counted::Staged), 0, "aligned whole 32 KiB turn must not copy into stage");
    if on_sme2 {
        assert_eq!(big_triple, [0, N as u64, 0], "SME2 aligned 32 KiB second turn must take N flat walks");
    } else if rows_here {
        assert_eq!(big_triple, [N as u64, 0, 0], "aligned 32 KiB second turn must cut N rows");
    }
    for (i, h) in big_hashers.iter().enumerate() {
        assert_eq!(h.finalize(), big_wants[i], "big hasher {i}");
    }
}

/// Arithmetic work bounds for streaming `Hasher::update`:
/// - On x86-64 (`blake3_sse41_ffi`), an initial <= 64 byte update takes
///   `PaddedFirstBlock` once; on Arm it takes a single ordinary `Staged` copy.
/// - After an initial 16 KiB group, feeding `3 * 16 KiB + 100 B` cuts exact
///   subtrees (2 rows on x86-64/NEON, or 1 flat walk for the 32 KiB subtree on
///   SME2), copies only the 100-byte tail once (`Staged == 1`), and finalizes
///   with `TailApart == 1`.
#[test]
fn streaming_hasher_work_counts_match_arithmetic() {
    crate::initialize();
    #[cfg(blake3_sme2)]
    let on_sme2 = matches!(Platform::detect(), Platform::SME2);
    #[cfg(not(blake3_sme2))]
    let on_sme2 = false;

    // 1. Short <= 64 byte first update.
    clear_counts();
    let mut small = Hasher::new(Mode::Plain, None);
    small.update(&[0x42u8; 37]);
    #[cfg(blake3_sse41_ffi)]
    {
        assert_eq!(count(Counted::PaddedFirstBlock), 1);
        assert_eq!(count(Counted::Staged), 0);
    }
    #[cfg(not(blake3_sse41_ffi))]
    {
        assert_eq!(count(Counted::PaddedFirstBlock), 0);
        assert_eq!(count(Counted::Staged), 1);
    }
    assert_eq!(count(Counted::Row), 0);
    assert_eq!(count(Counted::Flat), 0);
    assert_eq!(small.finalize(), crate::test::official::hash(Mode::Plain, &[0x42u8; 37]));

    // 2. Subsequent 3 * 16 KiB + 100 B update after an initial 16 KiB group.
    let head: Vec<u8> = (0..STAGE_LEN).map(|i| low_byte(i * 11)).collect();
    let tail_data: Vec<u8> = (0..(3 * STAGE_LEN + 100)).map(|i| low_byte(i * 19)).collect();
    let mut all = Vec::with_capacity(head.len() + tail_data.len());
    all.extend_from_slice(&head);
    all.extend_from_slice(&tail_data);

    let mut big = Hasher::new(Mode::Plain, None);
    big.update(&head);
    clear_counts();
    big.update(&tail_data);
    let rows = count(Counted::Row);
    let flats = count(Counted::Flat);
    let rows_here = (4..=crate::hasher::ROW).contains(&Platform::detect().simd_degree());
    if on_sme2 {
        assert_eq!((rows, flats), (0, 1), "SME2 32 KiB aligned subtree takes 1 flat walk (16 KiB is below flat_takes)");
    } else if rows_here {
        assert_eq!((rows, flats), (2, 0), "16 KiB + 32 KiB after initial 16 KiB must cut exactly 2 rows");
    }
    assert_eq!(count(Counted::Staged), 1, "only the trailing 100-byte tail copies into stage");
    clear_counts();
    let got = big.finalize();
    assert_eq!(count(Counted::TailApart), 1, "finalizing a staged tail after whole subtrees takes TailApart once");
    assert_eq!(got, crate::test::official::hash(Mode::Plain, &all));
}

/// Exact grouped dispatch work counts for `hash_each` over uniform multi-chunk
/// messages: `M = 16` messages of `CHUNKS = 3` whole chunks run exactly
/// `CHUNKS` shared full-chunk rounds plus 1 batched Merkle parent round and 1
/// batched Merkle root round, totaling `M.div_ceil(LANES_AT_ONCE) * (CHUNKS + 2)`
/// `Platform::hash_many` calls (catches both missing rounds and ungrouped
/// per-input dispatches on every CPU width!).
#[test]
fn hash_each_shared_rounds_and_hash_many_calls_match_arithmetic() {
    crate::initialize();
    const M: usize = 16;
    const CHUNKS: usize = 3;
    const LEN: usize = CHUNKS * crate::CHUNK_LEN;
    let block_payloads: Vec<Vec<throughput::Block>> = (0..M)
        .map(|i| {
            let mut blocks = vec![throughput::Block::ZERO; LEN / 64];
            for (j, b) in throughput::Block::bytes_mut(&mut blocks).iter_mut().enumerate() {
                *b = low_byte(i * 7 + j);
            }
            blocks
        })
        .collect();
    let items: Vec<(&[throughput::Block], usize)> = block_payloads.iter().map(|b| (b.as_slice(), LEN)).collect();
    let mut out = [[0u8; 32]; M];

    clear_counts();
    throughput::hash_each(Mode::Plain, &items, &mut out);
    let rounds = count(Counted::EachRound);
    let hash_many_calls = count(Counted::HashManyCall);
    // 3 full-chunk rounds + 1 non-root parent round (3 -> 2 CVs) + 1 root round (2 -> 1).
    let expected_calls = (M.div_ceil(crate::each::LANES_AT_ONCE) * (CHUNKS + 2)) as u64;
    assert_eq!(rounds, CHUNKS as u64, "exact shared round count for {CHUNKS}-chunk messages");
    assert_eq!(
        hash_many_calls,
        expected_calls,
        "exact Platform::hash_many call count for M={M}, CHUNKS={CHUNKS}, LANES_AT_ONCE={}",
        crate::each::LANES_AT_ONCE
    );
    for (i, d) in out.iter().enumerate() {
        assert_eq!(d, &crate::test::official::hash(Mode::Plain, throughput::Block::bytes(&block_payloads[i])), "hash_each {i}");
    }
}
