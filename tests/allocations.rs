//! The crate's "Memory" section, held to its word: the single-threaded
//! calls allocate nothing, and `hash_each` only 32 bytes for each chunk of
//! its messages of 2 to 15 chunks and up to 128 for each such message. A
//! counting allocator, in a process of its own, counts the bytes each call
//! asks for on its calling thread.
#![cfg(feature = "std")]

#[path = "../tools/require_nextest.rs"]
mod require_nextest;

use blake3_servil::convenience::Hasher;
use blake3_servil::throughput::{self, Batch, Block};
use blake3_servil::{Mode, convenience};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Counting;

// The calls measured here are single-threaded; count their allocations
// separately from the test harness's other threads.
thread_local! {
    static BYTES: Cell<usize> = const { Cell::new(0) };
    static REQUESTS: Cell<usize> = const { Cell::new(0) };
}

// Sound: every call goes on to the system allocator, unchanged.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // A thread whose locals are gone counts nothing.
        let _ = BYTES.try_with(|b| b.set(b.get() + layout.size()));
        let _ = REQUESTS.try_with(|n| n.set(n.get() + 1));
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

/// The bytes `call` asks the allocator for, on this thread.
fn allocated(call: impl FnOnce()) -> usize {
    let before = BYTES.with(Cell::get);
    call();
    BYTES.with(Cell::get) - before
}

#[test]
fn allocation_counter_sees_a_callers_allocation() {
    let before = REQUESTS.with(Cell::get);
    assert_eq!(allocated(|| drop(std::hint::black_box(vec![0u8; 4096]))), 4096);
    assert_eq!(REQUESTS.with(Cell::get) - before, 1);
}

/// Only key bytes vary: caller preparation stays outside the allocator
/// counters, and two held stages cover zero and nonzero chunk counters.
#[test]
fn held_allocations_ignore_key_equality_and_prefixes() {
    blake3_servil::initialize();
    let input = vec![42u8; 2 * 16 * 1024];
    for n in [3usize, 8, 16, 17, 33] {
        for mixed in [false, true] {
            let mut reference = None;
            for prefix in (0..=8).rev() {
                let keys: Vec<[u8; 32]> = (0..n)
                    .map(|i| {
                        let mut key = [0x6du8; 32];
                        if prefix < 8 {
                            key[4 * prefix..4 * prefix + 4].copy_from_slice(&(i as u32 + 1).to_le_bytes());
                        }
                        key
                    })
                    .collect();
                let mode = |i: usize| {
                    if !mixed || i % 3 == 1 {
                        Mode::Keyed(&keys[i])
                    } else if i % 3 == 0 {
                        Mode::Plain
                    } else {
                        Mode::DeriveKey("held key-prefix regression")
                    }
                };
                let mut hashers: Vec<_> = (0..n).map(|i| Hasher::new(mode(i), None)).collect();
                let mut pieces = Vec::with_capacity(n);
                let requests = REQUESTS.with(Cell::get);
                let bytes = allocated(|| {
                    for piece in input.chunks(4096) {
                        pieces.clear();
                        pieces.extend((0..n).map(|i| (i, piece)));
                        throughput::update_each(&mut hashers, &pieces);
                    }
                });
                let work = (bytes, REQUESTS.with(Cell::get) - requests);
                assert!(work.0 > 0 && work.1 > 0, "held allocation work observed");
                if let Some(expected) = reference {
                    assert_eq!(work, expected, "allocation work independent of key equality and prefix: n={n} mixed={mixed} prefix={prefix}");
                } else {
                    reference = Some(work);
                }
                for (i, h) in hashers.iter().enumerate() {
                    let expected = match mode(i) {
                        Mode::Plain => *blake3::hash(&input).as_bytes(),
                        Mode::Keyed(key) => *blake3::keyed_hash(key, &input).as_bytes(),
                        Mode::DeriveKey(context) => blake3::derive_key(context, &input),
                        _ => unreachable!("only plain, keyed and derive-key test modes"),
                    };
                    assert_eq!(h.clone().finalize().as_bytes(), &expected);
                }
            }
        }
    }
}

fn blocks(len: usize, seed: u8) -> Vec<Block> {
    let mut blocks = vec![Block::ZERO; len.div_ceil(64).max(1)];
    for (i, b) in Block::bytes_mut(&mut blocks).iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(31).wrapping_add(seed);
    }
    blocks
}

#[test]
fn single_threaded_calls_allocate_as_the_memory_section_says() {
    blake3_servil::initialize();
    let input = blocks(64 * 1024, 1);
    let bytes = Block::bytes(&input);
    let mut out = [0u8; 100];
    for len in [0, 1, 64, 1024, 1025, 16 * 1024, 64 * 1024] {
        assert_eq!(
            allocated(|| {
                std::hint::black_box(convenience::hash(&bytes[..len]));
            }),
            0,
            "convenience::hash of {len} B"
        );
        assert_eq!(
            allocated(|| {
                std::hint::black_box(throughput::hash(Mode::Plain, &input, len));
            }),
            0,
            "throughput::hash of {len} B"
        );
        let mut hasher = Hasher::new(Mode::Plain, None);
        assert_eq!(
            allocated(|| {
                hasher.update(&bytes[..len]);
                std::hint::black_box(hasher.clone().finalize());
            }),
            0,
            "Hasher of {len} B"
        );
        assert_eq!(allocated(|| hasher.finalize_xof().fill(&mut out)), 0, "OutputReader after {len} B");
    }
    let batch = Batch::from_blocks(&input[..64], 64);
    let mut digests = vec![[0u8; 32]; batch.len()];
    assert_eq!(allocated(|| throughput::hash_many(Mode::Plain, &batch, &mut digests)), 0, "hash_many");

    // A collection: items of up to a chunk allocate nothing; a message of
    // 2 to 15 chunks, 32 bytes a chunk.
    let lens = [0usize, 1, 63, 64, 65, 300, 700, 1000, 1024];
    let small: Vec<Vec<Block>> = (0..100).map(|i| blocks(lens[i % lens.len()], i as u8)).collect();
    let items: Vec<(&[Block], usize)> = small.iter().zip((0..100).map(|i| lens[i % lens.len()])).map(|(b, l)| (&b[..], l)).collect();
    let mut digests = vec![[0u8; 32]; items.len()];
    assert_eq!(allocated(|| throughput::hash_each(Mode::Plain, &items, &mut digests)), 0, "hash_each of 100 items of up to a chunk");
    let long = blocks(5 * 1024, 9);
    let mut mixed = items.clone();
    mixed.push((&long[..], 5 * 1024));
    let mut digests = vec![[0u8; 32]; mixed.len()];
    let bytes = allocated(|| throughput::hash_each(Mode::Plain, &mixed, &mut digests));
    assert!(bytes <= 32 * 5 + 128, "hash_each with one message of 5 chunks: {bytes} B");
    // Forty such messages: 32 bytes a chunk, and the bookkeeping a message.
    let longs: Vec<(&[Block], usize)> = (0..40).map(|_| (&long[..], 5 * 1024)).collect();
    let mut digests = vec![[0u8; 32]; longs.len()];
    let bytes = allocated(|| throughput::hash_each(Mode::Plain, &longs, &mut digests));
    assert!(bytes <= 40 * (32 * 5 + 128), "hash_each of 40 messages of 5 chunks: {bytes} B");
}

// The pipeline's admission source uses the same counting allocator.
mod finish_sync {
    pub use std::sync::Mutex;
}
#[path = "../src/admission.rs"]
mod admission;

#[test]
fn resource_admission_allocates_nothing_when_ready_or_waiting() {
    use core::{
        future::Future,
        task::{Context, Poll},
    };
    let pool = std::sync::Arc::new(admission::Pool::new(vec![17]));
    let mut cx = Context::from_waker(std::task::Waker::noop());
    let bytes = allocated(|| {
        for _ in 0..1000 {
            let mut immediate = std::pin::pin!(pool.acquire());
            let held: admission::Lease<'_, usize> = match immediate.as_mut().poll(&mut cx) {
                Poll::Ready(value) => value,
                Poll::Pending => panic!("the buffer is free"),
            };
            let mut waiting = std::pin::pin!(pool.clone().acquire_owned());
            assert!(waiting.as_mut().poll(&mut cx).is_pending());
            held.release();
            let returned: admission::OwnedLease<usize> = match waiting.as_mut().poll(&mut cx) {
                Poll::Ready(value) => value,
                Poll::Pending => panic!("the buffer is reserved"),
            };
            assert_eq!(*returned, 17);
            // A request dropped while pending leaves the queue, within the
            // queue's reserved capacity.
            let mut withdrawn = std::pin::pin!(pool.acquire());
            assert!(withdrawn.as_mut().poll(&mut cx).is_pending());
            drop(withdrawn);
            returned.release();
        }
    });
    assert_eq!(bytes, 0, "ready, pending, notification and return use existing storage");
}
