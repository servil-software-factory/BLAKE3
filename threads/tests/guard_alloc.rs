//! Trailing guard-page allocator suite for `blake3-servil` (Approved Default 4).
//!
//! Runs multithreaded `update_multithreaded` (above the 512 KiB per-subtree
//! pool-split threshold plus an odd tail) and `Pipeline` (`owned`, `lent`,
//! streamed `Room` commits, and `batch`) with every heap allocation placed
//! right-aligned before a mapped `PROT_NONE` trailing guard page.
#![cfg(unix)]

#[path = "../../tools/guard_alloc_common.rs"]
mod guard_alloc_common;

use blake3_servil::{Mode, convenience::Hasher, initialize_multithreaded, throughput::*};
use guard_alloc_common::{GuardAllocator, verify_guard_page_faults_on_overread};
use std::num::NonZeroUsize;

#[global_allocator]
static ALLOC: GuardAllocator = GuardAllocator;

#[inline]
fn low_byte(v: usize) -> u8 {
    v.to_le_bytes()[0]
}

#[test]
fn threads_pipeline_and_pool_never_overread_under_guard_pages() {
    // Verify the allocator's trailing guard page actually faults on a 1-byte overread
    // BEFORE spawning pool worker threads (so fork runs single-threaded).
    verify_guard_page_faults_on_overread();

    initialize_multithreaded(NonZeroUsize::new(4));

    // 1. Multithreaded Hasher update over a 2 MiB + 137 B exact-sized heap buffer
    //    (above the 512 KiB per-subtree pool-split threshold so workers genuinely
    //    split and hash subtrees under guard pages).
    let big_len: usize = 2 * 1024 * 1024 + 137;
    let big: Vec<u8> = (0..big_len).map(|i| low_byte(i.wrapping_mul(41) ^ (i >> 6))).collect();
    let mut h = Hasher::new(Mode::Plain, Some(big.len() as u64));
    update_multithreaded(&mut h, &big);
    assert_eq!(h.finalize().as_bytes(), blake3::hash(&big).as_bytes());

    // 2. Pipeline owned, lent, streamed Room commits, and Batch under guard pages,
    //    verifying exact FIFO end-order delivery across all 4 tags.
    let mut pipe = Pipeline::<usize, Vec<u8>>::with_owners(Mode::Plain, Limits { messages: 8, rooms: 8 });

    let owned_payload: Vec<u8> = (0..131_077usize).map(|i| low_byte(i * 17)).collect();
    let owned_want = *blake3::hash(&owned_payload).as_bytes();
    let m1 = block_on(pipe.message(1, Some(owned_payload.len() as u64)));
    pipe.owned(m1, owned_payload);

    let lent_payload: Vec<u8> = (0..98_305usize).map(|i| low_byte(i * 23)).collect();
    let lent_want = *blake3::hash(&lent_payload).as_bytes();
    let m2 = block_on(pipe.message(2, Some(lent_payload.len() as u64)));
    // Sound: `lent_payload` outlives `lent_guard`, dropped only after Event::Hashed(tag: 2).
    let lent_guard = unsafe { pipe.lent(m2, &lent_payload) };

    let stream_payload: Vec<u8> = (0..80_001usize).map(|i| low_byte(i * 7 + 3)).collect();
    let stream_want = *blake3::hash(&stream_payload).as_bytes();
    let mut msg = block_on(pipe.message(3, Some(stream_payload.len() as u64)));
    let mut off = 0;
    while off < stream_payload.len() {
        let mut room = block_on(pipe.room());
        let take = (stream_payload.len() - off).min(room.len());
        room.bytes_mut()[..take].copy_from_slice(&stream_payload[off..off + take]);
        pipe.commit(&mut msg, &mut room, take);
        off += take;
    }
    pipe.end(msg);

    let mut batch = Batch::new(256, 37);
    let mut batch_wants = Vec::new();
    for i in 0..37usize {
        let item: Vec<u8> = (0..256).map(|j| low_byte(i * 13 + j * 5)).collect();
        batch_wants.push(*blake3::hash(&item).as_bytes());
        batch.set(i, &item);
    }
    let batch_msg = block_on(pipe.message(4, None));
    let digests = vec![[0u8; 32]; 37].into_boxed_slice();
    pipe.batch(batch_msg, batch, digests);

    let mut seen_tags = Vec::with_capacity(4);
    let mut lent_guard = Some(lent_guard);
    while seen_tags.len() < 4 {
        if let Some(ev) = pipe.next_event() {
            match ev {
                Event::Hashed { tag: 1, hash, owner } => {
                    assert_eq!(hash.as_bytes(), &owned_want);
                    assert!(owner.is_some());
                    seen_tags.push(1);
                }
                Event::Hashed { tag: 2, hash, .. } => {
                    assert_eq!(hash.as_bytes(), &lent_want);
                    drop(lent_guard.take());
                    seen_tags.push(2);
                }
                Event::Hashed { tag: 3, hash, .. } => {
                    assert_eq!(hash.as_bytes(), &stream_want);
                    seen_tags.push(3);
                }
                Event::Batch { tag: 4, digests, .. } => {
                    assert_eq!(digests.len(), batch_wants.len());
                    for (got, want) in digests.iter().zip(&batch_wants) {
                        assert_eq!(got, want);
                    }
                    seen_tags.push(4);
                }
                _ => panic!("unexpected pipeline event"),
            }
        } else {
            block_on(pipe.ready());
        }
    }
    assert_eq!(seen_tags, vec![1, 2, 3, 4], "pipeline must deliver all four distinct events in FIFO end order");
}
