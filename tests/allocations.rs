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
use std::os::unix::thread::RawPthread;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Counting;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Counts {
    allocations: usize,
    reallocations: usize,
    deallocations: usize,
    bytes: usize,
}

impl Counts {
    fn since(self, before: Self) -> Self {
        Self {
            allocations: self.allocations - before.allocations,
            reallocations: self.reallocations - before.reallocations,
            deallocations: self.deallocations - before.deallocations,
            bytes: self.bytes - before.bytes,
        }
    }
    fn plus(self, other: Self) -> Self {
        Self {
            allocations: self.allocations + other.allocations,
            reallocations: self.reallocations + other.reallocations,
            deallocations: self.deallocations + other.deallocations,
            bytes: self.bytes + other.bytes,
        }
    }
}

struct AtomicCounts {
    allocations: AtomicUsize,
    reallocations: AtomicUsize,
    deallocations: AtomicUsize,
    bytes: AtomicUsize,
}
impl AtomicCounts {
    fn get(&self) -> Counts {
        Counts {
            allocations: self.allocations.load(Ordering::Relaxed),
            reallocations: self.reallocations.load(Ordering::Relaxed),
            deallocations: self.deallocations.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
        }
    }
    fn add(&self, c: Counts) {
        self.allocations.fetch_add(c.allocations, Ordering::Relaxed);
        self.reallocations.fetch_add(c.reallocations, Ordering::Relaxed);
        self.deallocations.fetch_add(c.deallocations, Ordering::Relaxed);
        self.bytes.fetch_add(c.bytes, Ordering::Relaxed);
    }
}
static WORKER_COUNTS: AtomicCounts =
    AtomicCounts { allocations: AtomicUsize::new(0), reallocations: AtomicUsize::new(0), deallocations: AtomicUsize::new(0), bytes: AtomicUsize::new(0) };

unsafe extern "C" {
    fn pthread_self() -> RawPthread;
    fn pthread_getname_np(thread: RawPthread, name: *mut std::ffi::c_char, len: usize) -> std::ffi::c_int;
}

// These are the library's OS-thread names (lanes.rs and gpu.rs). Reading
// them uses no Rust allocation; the harness's threads have other names.
// This observes Rust GlobalAlloc requests, not native allocators/drivers.
fn library_worker() -> bool {
    let mut name = [0u8; 64];
    // Sound: the calling thread is live and name is 64 writable bytes.
    if unsafe { pthread_getname_np(pthread_self(), name.as_mut_ptr().cast(), name.len()) } != 0 {
        std::process::abort();
    }
    name.starts_with(b"blake3-")
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
        let before = c.get();
        let mut value = before;
        change(&mut value);
        c.set(value);
        if library_worker() {
            WORKER_COUNTS.add(value.since(before));
        }
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
    after.since(before)
}

/// A complete synchronous call, including this process's named library
/// workers. The call waits for all of its work before returning. Each
/// nextest process owns its window; the harness's other threads stay out.
fn counts_with_workers(call: impl FnOnce()) -> Counts {
    let before = WORKER_COUNTS.get();
    let caller = counts(call);
    caller.plus(WORKER_COUNTS.get().since(before))
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

#[test]
fn worker_counter_sees_an_owned_background_allocation() {
    use std::sync::Arc;
    let flags = Arc::new([AtomicBool::new(false), AtomicBool::new(false), AtomicBool::new(false), AtomicBool::new(false)]);
    let f = flags.clone();
    let worker = std::thread::Builder::new()
        .name("blake3-worker-meter".into())
        .spawn(move || {
            f[0].store(true, Ordering::Release);
            while !f[1].load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            drop(std::hint::black_box(vec![0u8; 4096]));
            f[2].store(true, Ordering::Release);
            while !f[3].load(Ordering::Acquire) {
                std::thread::yield_now();
            }
        })
        .unwrap();
    while !flags[0].load(Ordering::Acquire) {
        std::thread::yield_now();
    }
    let work = counts_with_workers(|| {
        flags[1].store(true, Ordering::Release);
        while !flags[2].load(Ordering::Acquire) {
            std::thread::yield_now();
        }
    });
    flags[3].store(true, Ordering::Release);
    worker.join().unwrap();
    assert_eq!(work, Counts { allocations: 1, deallocations: 1, bytes: 4096, ..Counts::default() });
}

/// Setup precedes the first measured cycle. Grouped whole-message updates
/// reach the pool as well as the caller; output consumption and the
/// resets remain inside three complete cycles (#325).
#[test]
fn prepared_multithreaded_cycles_include_the_workers() {
    pool_ready(4);
    let inputs: Vec<_> = (0..3).map(|i| vec![37u8 + i; 4 << 20]).collect();
    let keys = [[19u8; 32], [91u8; 32]];
    for shared in [true, false] {
        let mode = |i| match i {
            0 => Mode::Plain,
            2 => Mode::DeriveKey("prepared worker allocation cycles"),
            _ => Mode::Keyed(&keys[usize::from(!shared && i == 3)]),
        };
        let mut hashers: Vec<_> = (0..4).map(|i| Hasher::new(mode(i), None)).collect();
        let which = [0usize, 1, 2, 3];
        let mut pieces = Vec::with_capacity(4);
        let mut outputs = vec![[0u8; 32]; 3 * 4];
        let work = counts_with_workers(|| {
            for (input, out) in inputs.iter().zip(outputs.chunks_mut(4)) {
                pieces.clear();
                pieces.extend((0..4).map(|i| (i, &input[..])));
                throughput::update_each_multithreaded(&mut hashers, &pieces);
                throughput::finalize_each(&mut hashers, &which, out);
                let consumed = out.as_flattened().iter().fold(0usize, |sum, &b| sum.wrapping_add(b as usize));
                std::hint::black_box(consumed);
            }
        });
        assert_eq!(work, Counts::default(), "prepared worker cycles, shared={shared}");
        for (input, out) in inputs.iter().zip(outputs.chunks(4)) {
            for (i, digest) in out.iter().enumerate() {
                let expected = match mode(i) {
                    Mode::Plain => *blake3::hash(input).as_bytes(),
                    Mode::Keyed(k) => *blake3::keyed_hash(k, input).as_bytes(),
                    Mode::DeriveKey(c) => blake3::derive_key(c, input),
                    _ => unreachable!("the test's three modes"),
                };
                assert_eq!(*digest, expected);
            }
        }
    }
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

/// The pipeline's batch cycle (#325, #317): two batches go round one
/// pipeline, each refilled, submitted, hashed by the workers, collected
/// with its places, and refilled again, as a continuous batch producer
/// does. After the first round, which prepares the receiver, a cycle
/// allocates nothing on the producer or the workers.
#[test]
fn a_pipeline_batch_cycle_allocates_nothing_after_the_first() {
    use std::cell::RefCell;
    use throughput::pipeline::{Event, Limits, Stream};
    fn run<F: std::future::Future>(future: F) -> F::Output {
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        let mut future = std::pin::pin!(future);
        loop {
            if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut cx) {
                return value;
            }
            std::thread::yield_now();
        }
    }
    pool_ready(4);
    let fill = |batch: &mut Batch<'_>, round: u8| {
        for i in 0..batch.len() {
            batch.set(i, &[round.wrapping_mul(7).wrapping_add(i as u8); 64]);
        }
    };
    let mut places = [vec![[0u8; 32]; 32], vec![[0u8; 32]; 32]];
    let returned: RefCell<Vec<(u8, Batch<'_>, &mut [[u8; 32]])>> = RefCell::new(Vec::with_capacity(2));
    let stream = Stream::new(Mode::Plain, Limits { buffers: 2, messages: 2 }, |event| match event {
        Event::Batch { tag, batch, places } => returned.borrow_mut().push((tag, batch, places)),
        _ => unreachable!("batches alone"),
    });
    let [pa, pb] = &mut places;
    let (mut a, mut b) = (Batch::new(64, 32), Batch::new(64, 32));
    fill(&mut a, 0);
    fill(&mut b, 1);
    run(stream.submit_batch(a, pa, 0));
    run(stream.submit_batch(b, pb, 1));
    let mut rounds = [0u8; 2];
    // One batch's cycle: wait for a returned batch, check its digests,
    // refill it and submit it again.
    let mut cycle = |check: bool| {
        while returned.borrow().is_empty() {
            run(stream.ready());
            stream.collect();
        }
        let (tag, mut batch, places) = returned.borrow_mut().pop().unwrap();
        let round = rounds[tag as usize] * 2 + tag;
        if check {
            for (i, place) in places.iter().enumerate() {
                assert_eq!(*place, *blake3::hash(&[round.wrapping_mul(7).wrapping_add(i as u8); 64]).as_bytes(), "round {round}, message {i}");
            }
        }
        rounds[tag as usize] += 1;
        fill(&mut batch, rounds[tag as usize] * 2 + tag);
        run(stream.submit_batch(batch, places, tag));
    };
    cycle(true);
    cycle(true);
    let work = counts_with_workers(|| {
        for _ in 0..12 {
            cycle(false);
        }
    });
    cycle(true);
    cycle(true);
    stream.finish();
    assert_eq!(work, Counts::default(), "twelve batch cycles after the first round");
}

/// The pipeline's message cycle: begin, write, end, collect, a message at
/// a time. After the first two, a cycle allocates nothing on the producer
/// or the workers.
#[test]
fn a_pipeline_message_cycle_allocates_nothing_after_the_first() {
    use std::cell::RefCell;
    use throughput::pipeline::{Event, Limits, Stream};
    fn run<F: std::future::Future>(future: F) -> F::Output {
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        let mut future = std::pin::pin!(future);
        loop {
            if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut cx) {
                return value;
            }
            std::thread::yield_now();
        }
    }
    pool_ready(4);
    let hashed: RefCell<Vec<(u8, blake3_servil::Hash)>> = RefCell::new(Vec::with_capacity(2));
    let stream = Stream::new(Mode::Plain, Limits { buffers: 2, messages: 2 }, |event| match event {
        Event::Hashed { tag, hash } => hashed.borrow_mut().push((tag, hash)),
        _ => unreachable!("ended messages alone"),
    });
    let cycle = |round: u8, check: bool| {
        let mut message = run(stream.message(round, Some(1024)));
        let mut input = run(message.input());
        input.bytes_mut()[..1024].fill(round);
        input.commit(1024);
        message.end(0);
        while hashed.borrow().is_empty() {
            run(stream.ready());
            stream.collect();
        }
        let (tag, hash) = hashed.borrow_mut().pop().unwrap();
        if check {
            assert_eq!(*hash.as_bytes(), *blake3::hash(&[tag; 1024]).as_bytes(), "message {tag}");
        }
    };
    cycle(0, true);
    cycle(1, true);
    let work = counts_with_workers(|| {
        for round in 2..14 {
            cycle(round, false);
        }
    });
    cycle(14, true);
    stream.finish();
    assert_eq!(work, Counts::default(), "twelve message cycles after the first two");
}

/// The pool of `threads`, started and at rest: the first worker starts
/// the others after `initialize_multithreaded` returns (their thread
/// names are allocations on `blake3-` threads), so a measured window
/// opens once every worker is parked.
fn pool_ready(threads: usize) {
    blake3_servil::initialize_multithreaded(std::num::NonZeroUsize::new(threads));
    while !blake3_servil::__all_workers_parked() {
        std::thread::yield_now();
    }
}

/// The other pool calls, after preparation and above the length that
/// reaches the pool (512 KiB): a whole message, a batch of messages, and
/// a collection of items of a chunk (items of 2 to 15 chunks allocate as
/// the Memory section says, pooled or not), each three times, on the
/// caller and the workers.
#[test]
fn prepared_pool_calls_allocate_nothing() {
    pool_ready(4);
    let message = blocks(4 << 20, 5);
    let batch = Batch::from_blocks(&message[..(2 << 20) / 64], 16 * 1024);
    let mut digests = vec![[0u8; 32]; batch.len()];
    let items: Vec<(&[Block], usize)> = message.chunks(1024 / 64).map(|b| (b, 1024)).collect();
    let mut out = vec![[0u8; 32]; items.len()];
    for round in 0..3 {
        let whole = counts_with_workers(|| {
            std::hint::black_box(throughput::hash_multithreaded(Mode::Plain, &message, 4 << 20));
        });
        assert_eq!(whole, Counts::default(), "a whole message over the pool, round {round}");
        let many = counts_with_workers(|| throughput::hash_many_multithreaded(Mode::Plain, &batch, &mut digests));
        assert_eq!(many, Counts::default(), "a batch over the pool, round {round}");
        let each = counts_with_workers(|| throughput::hash_each_multithreaded(Mode::Plain, &items, &mut out));
        assert_eq!(each, Counts::default(), "a collection over the pool, round {round}");
    }
    assert_eq!(throughput::hash_multithreaded(Mode::Plain, &message, 4 << 20), convenience::hash(Block::bytes(&message)));
    assert_eq!(digests[3], *blake3::hash(&Block::bytes(&message)[3 * 16 * 1024..4 * 16 * 1024]).as_bytes());
    assert_eq!(out[7], *blake3::hash(&Block::bytes(&message)[7 * 1024..8 * 1024]).as_bytes());
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
