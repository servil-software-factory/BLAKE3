//! Trailing guard-page allocator suite for `blake3-servil-core` (Approved Default 4).
//!
//! Uses `tools/guard_alloc_common.rs` as `#[global_allocator]` so every heap
//! allocation sits right-aligned before a mapped `PROT_NONE` trailing guard
//! page (zero trailing slack when `size % align == 0`, including all `Vec<u8>`
//! buffers with exact capacity). Note that types with internal block/line
//! padding (such as `Packed`) still have logical padding before the allocation
//! end; this test targets exact-sized byte buffers and slice boundaries.
#![cfg(all(feature = "std", unix))]

#[path = "../tools/guard_alloc_common.rs"]
mod guard_alloc_common;

use blake3_servil_core::{Mode, convenience::Hasher, initialize, throughput::*};
use guard_alloc_common::{GuardAllocator, page_size, verify_guard_page_faults_on_overread};

#[global_allocator]
static ALLOC: GuardAllocator = GuardAllocator;

fn official(mode: Mode<'static>, bytes: &[u8]) -> [u8; 32] {
    match mode {
        Mode::Plain => *blake3::hash(bytes).as_bytes(),
        Mode::Keyed(k) => *blake3::keyed_hash(k, bytes).as_bytes(),
        Mode::DeriveKey(ctx) => blake3::derive_key(ctx, bytes),
        _ => unreachable!(),
    }
}

#[inline]
fn low_byte(v: usize) -> u8 {
    v.to_le_bytes()[0]
}

#[test]
fn guard_allocator_places_byte_buffers_flush_against_prot_none_page() {
    let ps = page_size();
    for len in [1usize, 13, 63, 64, 65, 1024, 16385] {
        let buf = vec![0xA5u8; len];
        let end = (buf.as_ptr() as usize) + buf.capacity();
        assert_eq!(end % ps, 0, "1-byte aligned buffer of len {len} must sit flush against trailing PROT_NONE page");
    }
    verify_guard_page_faults_on_overread();
}

#[test]
fn core_operations_never_overread_or_overwrite_under_guard_pages() {
    initialize();
    const KEY: [u8; 32] = [0x3Cu8; 32];
    let modes = [Mode::Plain, Mode::Keyed(&KEY), Mode::DeriveKey("guard-alloc-core-context")];
    let lengths = [0usize, 1, 63, 64, 65, 1023, 1024, 1025, 2048, 4095, 4096, 4097, 16384, 16385, 32768, 65537];

    for &mode in &modes {
        let tree = TreeHasher::new(mode);
        for &len in &lengths {
            let mut input = vec![0u8; len];
            for (i, b) in input.iter_mut().enumerate() {
                *b = low_byte(i.wrapping_mul(29) ^ (i >> 7));
            }
            let want = official(mode, &input);

            // 1. Direct one-shot hash and streaming Hasher across split boundaries.
            assert_eq!(blake3_servil_core::convenience::hash_with(mode, &input).as_bytes(), &want, "oneshot len={len}");
            let mut h = Hasher::new(mode, Some(len as u64));
            let split = len / 3;
            h.update(&input[..split]);
            h.update(&input[split..]);
            assert_eq!(h.finalize().as_bytes(), &want, "stream len={len}");

            // 2. XOF OutputReader into exact-sized heap buffers (zero slack!).
            let mut xof = h.finalize_xof();
            for out_len in [1usize, 32, 63, 64, 65, 191, 513] {
                let mut out = vec![0u8; out_len];
                xof.set_position(0);
                xof.fill(&mut out);
                let mut ref_h = match mode {
                    Mode::Plain => blake3::Hasher::new(),
                    Mode::Keyed(k) => blake3::Hasher::new_keyed(k),
                    Mode::DeriveKey(c) => blake3::Hasher::new_derive_key(c),
                    _ => unreachable!(),
                };
                ref_h.update(&input);
                let mut ref_out = vec![0u8; out_len];
                ref_h.finalize_xof().fill(&mut ref_out);
                assert_eq!(out, ref_out, "xof len={len} out={out_len}");
            }

            // 3. TreeHasher whole-message / subtree paths.
            if len > 1024 {
                let left_chunks = len.div_ceil(1024).next_power_of_two() / 2;
                let left_bytes = left_chunks * 1024;
                let level = left_chunks.trailing_zeros();
                let left = tree.subtree_cv(level, 0, &input[..left_bytes]);
                let right = tree.last_cv(left_chunks as u64, &input[left_bytes..]);
                assert_eq!(tree.root_hash(&left, &right, len as u64).as_bytes(), &want, "tree len={len}");
            }
        }
    }

    // 4. Grouped update_each / finalize_each / hash_each / Batch & Packed under guard pages.
    let mut hashers: Vec<Hasher> = (0..35).map(|i| Hasher::new(modes[i % 3], None)).collect();
    let payloads: Vec<Vec<u8>> = (0..35)
        .map(|i| {
            let n = lengths[i % lengths.len()];
            (0..n).map(|j| low_byte(i * 31 + j * 13)).collect()
        })
        .collect();
    let first_turn: Vec<(usize, &[u8])> = payloads.iter().enumerate().map(|(i, p)| (i, &p[..p.len() / 2])).collect();
    let second_turn: Vec<(usize, &[u8])> = payloads.iter().enumerate().map(|(i, p)| (i, &p[p.len() / 2..])).collect();
    update_each(&mut hashers, &first_turn);
    update_each(&mut hashers, &second_turn);
    let which: Vec<usize> = (0..hashers.len()).collect();
    let mut digests = vec![[0u8; 32]; hashers.len()];
    finalize_each(&mut hashers, &which, &mut digests);
    for (i, d) in digests.iter().enumerate() {
        assert_eq!(d, &official(modes[i % 3], &payloads[i]), "finalize_each {i}");
    }

    let block_payloads: Vec<(Vec<Block>, usize)> = payloads
        .iter()
        .map(|p| {
            let nblocks = p.len().div_ceil(64).max(1);
            let mut blocks = vec![Block::ZERO; nblocks];
            Block::bytes_mut(&mut blocks)[..p.len()].copy_from_slice(p);
            (blocks, p.len())
        })
        .collect();
    let items: Vec<(&[Block], usize)> = block_payloads.iter().map(|(b, l)| (b.as_slice(), *l)).collect();
    let mut each_out = vec![[0u8; 32]; items.len()];
    hash_each(Mode::Plain, &items, &mut each_out);
    for (i, d) in each_out.iter().enumerate() {
        assert_eq!(d, &official(Mode::Plain, &payloads[i]), "hash_each {i}");
    }

    let mut batch = Batch::new(1024, 33);
    for i in 0..33 {
        let mut msg = vec![0u8; 1024];
        for (j, b) in msg.iter_mut().enumerate() {
            *b = low_byte(i * 19 + j);
        }
        batch.set(i, &msg);
    }
    let mut batch_out = vec![[0u8; 32]; batch.len()];
    hash_many(Mode::Plain, &batch, &mut batch_out);
    for i in 0..batch.len() {
        let expected: Vec<u8> = (0..1024).map(|j| low_byte(i * 19 + j)).collect();
        assert_eq!(batch_out[i], official(Mode::Plain, &expected), "batch {i}");
    }
    let mut packed = Packed::<32>::new(33);
    for i in 0..33 {
        let expected: [u8; 32] = core::array::from_fn(|j| low_byte(i * 19 + j));
        packed[i].copy_from_slice(&expected);
    }
    let mut packed_out = vec![[0u8; 32]; packed.len()];
    hash_many(Mode::Plain, &packed, &mut packed_out);
    for i in 0..packed.len() {
        let expected: [u8; 32] = core::array::from_fn(|j| low_byte(i * 19 + j));
        assert_eq!(packed_out[i], official(Mode::Plain, &expected), "packed {i}");
    }
}
