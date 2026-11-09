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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Counts {
    allocations: usize,
    reallocations: usize,
    deallocations: usize,
    bytes: usize,
}

// Count the calling thread separately from the harness's other threads.
thread_local! {
    static COUNTS: Cell<Counts> = const { Cell::new(Counts {
        allocations: 0, reallocations: 0, deallocations: 0, bytes: 0,
    }) };
}

fn record(change: impl FnOnce(&mut Counts)) {
    // Allocations during thread-local destruction still reach System.
    let _ = COUNTS.try_with(|c| {
        let mut value = c.get();
        change(&mut value);
        c.set(value);
    });
}

// Sound: each allocator operation forwards its arguments to System unchanged.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(|c| {
            c.allocations += 1;
            c.bytes += layout.size();
        });
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(|c| {
            c.allocations += 1;
            c.bytes += layout.size();
        });
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record(|c| {
            c.reallocations += 1;
            c.bytes += new_size;
        });
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        record(|c| c.deallocations += 1);
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

/// The allocator work `call` requests on this thread.
fn counts(call: impl FnOnce()) -> Counts {
    let before = COUNTS.with(Cell::get);
    call();
    let after = COUNTS.with(Cell::get);
    Counts {
        allocations: after.allocations - before.allocations,
        reallocations: after.reallocations - before.reallocations,
        deallocations: after.deallocations - before.deallocations,
        bytes: after.bytes - before.bytes,
    }
}

/// The bytes `call` asks the allocator for, on this thread.
fn allocated(call: impl FnOnce()) -> usize {
    counts(call).bytes
}

#[test]
fn allocation_counter_sees_a_callers_allocation() {
    assert_eq!(counts(|| drop(std::hint::black_box(vec![0u8; 4096]))), Counts { allocations: 1, deallocations: 1, bytes: 4096, ..Counts::default() });
    let mut buffer = vec![0u8; 8];
    assert_eq!(counts(|| buffer.reserve_exact(4096)), Counts { reallocations: 1, bytes: 4104, ..Counts::default() });
    assert_eq!(counts(|| drop(buffer)), Counts { deallocations: 1, ..Counts::default() });
}

/// Only key bytes vary: caller preparation stays outside the allocator
/// counters. Three complete cycles cover first use, reset and storage reuse;
/// each input has two held stages (zero and nonzero chunk counters).
#[test]
fn grouped_cycles_reuse_storage_for_every_key_prefix() {
    blake3_servil::initialize();
    let inputs: Vec<_> = (0..3).map(|i| vec![42u8 + i; 2 * 16 * 1024]).collect();
    for n in [3usize, 8, 16, 17, 33] {
        for flavor in 0..3 {
            for prefix in (0..=8).rev() {
                if flavor == 0 && prefix != 8 {
                    continue;
                }
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
                    if flavor == 0 || (flavor == 2 && i % 3 == 0) {
                        Mode::Plain
                    } else if flavor == 1 || i % 3 == 1 {
                        Mode::Keyed(&keys[i])
                    } else {
                        Mode::DeriveKey("held key-prefix regression")
                    }
                };
                let mut hashers: Vec<_> = (0..n).map(|i| Hasher::new(mode(i), None)).collect();
                let mut pieces = Vec::with_capacity(n);
                let which: Vec<_> = (0..n).collect();
                let mut outputs = vec![[0u8; 32]; n * inputs.len()];
                let work = counts(|| {
                    for (input, outputs) in inputs.iter().zip(outputs.chunks_mut(n)) {
                        for piece in input.chunks(4096) {
                            pieces.clear();
                            pieces.extend((0..n).map(|i| (i, piece)));
                            throughput::update_each(&mut hashers, &pieces);
                        }
                        throughput::finalize_each(&mut hashers, &which, outputs);
                    }
                });
                assert_eq!(work, Counts::default(), "prepared grouped cycles: n={n} flavor={flavor} prefix={prefix}");
                for (input, outputs) in inputs.iter().zip(outputs.chunks(n)) {
                    for (i, output) in outputs.iter().enumerate() {
                        let expected = match mode(i) {
                            Mode::Plain => *blake3::hash(input).as_bytes(),
                            Mode::Keyed(key) => *blake3::keyed_hash(key, input).as_bytes(),
                            Mode::DeriveKey(context) => blake3::derive_key(context, input),
                            _ => unreachable!("only plain, keyed and derive-key test modes"),
                        };
                        assert_eq!(*output, expected);
                    }
                }
            }
        }
    }
}

/// Repeated indices, empty pieces and uneven initial stages around the
/// ready table's boundary, in adjacent and interleaved caller order.
#[test]
fn grouped_fragmentation_reuses_storage_at_table_boundaries() {
    blake3_servil::initialize_multithreaded(None);
    let input: Vec<u8> = (0..3 * 16 * 1024 + 137).map(|i| (i % 251) as u8).collect();
    let key = [93u8; 32];
    let mode = |i| match i % 3 {
        0 => Mode::Plain,
        1 => Mode::Keyed(&key),
        _ => Mode::DeriveKey("bounded grouped storage"),
    };
    let expected = [*blake3::hash(&input).as_bytes(), *blake3::keyed_hash(&key, &input).as_bytes(), blake3::derive_key("bounded grouped storage", &input)];
    for n in [1usize, 127, 128, 129, 257] {
        for interleaved in [false, true] {
            for pooled in [false, true] {
                for piece_len in [4093, 4096] {
                    let mut hashers: Vec<_> = (0..n)
                        .map(|i| {
                            let mut h = Hasher::new(mode(i), None);
                            h.update(&input[..i % 3 * 511]);
                            h
                        })
                        .collect();
                    let mut pieces = Vec::new();
                    for i in 0..n {
                        pieces.push((i, &input[..0]));
                        pieces.extend(input[i % 3 * 511..].chunks(piece_len).map(|p| (i, p)));
                    }
                    if interleaved {
                        pieces.sort_by_key(|(i, p)| (p.as_ptr() as usize - input.as_ptr() as usize, *i));
                    }
                    let which: Vec<_> = (0..n).collect();
                    let mut outputs = vec![[0; 32]; n];
                    let work = counts(|| {
                        if pooled {
                            throughput::update_each_multithreaded(&mut hashers, &pieces);
                        } else {
                            throughput::update_each(&mut hashers, &pieces);
                        }
                        throughput::finalize_each(&mut hashers, &which, &mut outputs);
                    });
                    assert_eq!(work, Counts::default(), "n={n} interleaved={interleaved} pooled={pooled} piece_len={piece_len}");
                    for (i, output) in outputs.iter().enumerate() {
                        assert_eq!(*output, expected[i % 3]);
                    }
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
