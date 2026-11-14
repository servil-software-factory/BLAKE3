//! `blake3-servil` is a fast [BLAKE3] implementation for 64-bit Arm and
//! x86-64. It produces the same digests as official BLAKE3, with a call
//! for each way programs hash (the table below).
//!
//! It is two crates in one: [`blake3_servil_core`], which hashes on the
//! calling thread (the assembly kernels, each proved against the
//! specification, and every one-thread call), re-exported here whole; and
//! this crate's own part, a pool of threads and the multithreaded form of
//! each call. A program that wants no thread started depends on the core
//! alone.
//!
//! # Examples
//!
//! ```
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Once per process, before any other call (see Startup self-test).
//! blake3_servil::initialize();
//!
//! // Hash an input all at once.
//! let hash1 = blake3_servil::convenience::hash(b"foobarbaz");
//!
//! // Hash an input incrementally.
//! let mut hasher = blake3_servil::convenience::Hasher::new(blake3_servil::Mode::Plain, None);
//! hasher.update(b"foo");
//! hasher.update(b"bar");
//! hasher.update(b"baz");
//! // finalize leaves the Hasher as it is: more updates may follow it.
//! let hash2 = hasher.finalize();
//! assert_eq!(hash1, hash2);
//!
//! // Extended output. OutputReader also implements Read and Seek.
//! # #[cfg(feature = "std")] {
//! let mut output = [0; 1000];
//! let mut hasher = blake3_servil::convenience::Hasher::new(blake3_servil::Mode::Plain, None);
//! hasher.update(b"foobarbaz");
//! let mut output_reader = hasher.finalize_xof();
//! output_reader.fill(&mut output);
//! assert_eq!(hash1, output[..32]);
//! # }
//!
//! // Print a hash as hex.
//! println!("{}", hash1);
//!
//! // Many short messages in one call: a Batch lays them out in Blocks.
//! # #[cfg(feature = "std")] {
//! use blake3_servil::{throughput::{self, Batch}, Mode};
//! let mut batch = Batch::new(64, 3);
//! for i in 0..3 {
//!     batch.set(i, &[i as u8; 64]);
//! }
//! let mut hashes = [[0u8; 32]; 3];
//! throughput::hash_many(Mode::Plain, &batch, &mut hashes);
//! assert_eq!(hashes[2], *blake3_servil::convenience::hash(&[2u8; 64]).as_bytes());
//! # }
//! # Ok(())
//! # }
//! ```
//!
//! # Two kinds of call
//!
//! - **[`convenience`]**: one call per hash, every feature of BLAKE3 (the
//!   plain hash, the keyed hash, key derivation, extended output), any
//!   slice, the result returned, and the [`Hasher`]
//!   for a message fed in pieces. Tuned for a hash now and then, amid
//!   other work: where it helps, a call fetches its code back into the
//!   caches before it starts.
//! - **[`throughput`]**, for power users: every way this crate makes
//!   hashing fast (other cores, batches, collections), tuned for
//!   call after call: an occasional call amid other work may run slower
//!   than a convenience call would. The
//!   program takes on more of the work: inputs come as
//!   [`Block`](throughput::Block)s (64 bytes on a 64-byte boundary, so
//!   alignment is the type's), digests go into the program's places, and
//!   every call takes a [`Mode`].
//!
//! The [`Hasher`] runs on the calling thread; its
//! first update is treated as a call now and then (its code fetched first
//! where that helps), and its later updates as call after call. The
//! throughput calls that take it, [`throughput::update_multithreaded`]
//! and [`throughput::update_each`], are tuned as throughput calls.
//!
//! Start with the convenience calls; when a program needs more speed, it
//! moves to the throughput ones.
//!
//! | Your data | Call |
//! |---|---|
//! | One message in memory | [`convenience::hash`], or for speed [`throughput::hash_multithreaded`] |
//! | One message arriving in pieces | [`convenience::Hasher`], or for speed [`throughput::update_multithreaded`] |
//! | Many messages of one length, in one buffer | [`throughput::hash_many_multithreaded`] |
//! | Many 16- or 32-byte messages (a Merkle layer's leaves or digests) | [`throughput::hash_many_multithreaded`] over a [`throughput::Packed`] |
//! | Many messages of different lengths, in memory | [`throughput::hash_each_multithreaded`] |
//! | Many messages arriving at once (a server's uploads) | [`throughput::update_each`], or across cores [`throughput::Pipeline`] |
//! | Continuous stream of messages or batches | [`throughput::Pipeline`] |
//!
//! Each call returns once its hash is done, and uses other CPU cores
//! where that is faster (the forms without `multithreaded` stay on the
//! calling thread).
//!
//! Short messages hash fastest together: 1024 messages of 64 bytes in one
//! [`throughput::hash_many`] call take about a fifth of the time of 1024
//! [`convenience::hash`] calls.
//!
//! Select **keyed hashing or key derivation** with [`Mode`], supplied to
//! throughput calls and the [`Hasher`] constructor. See [Key-dependent
//! timing](#key-dependent-timing) for the key-independence contract.
//!
//! **Hashing now and then** (rather than nonstop) is slower per byte on
//! every hash function, since its code and data must come back into the
//! CPU's caches and a resting core runs slower at first: hashing 64 KiB
//! after a pause takes about three times as long as nonstop on an Apple M4
//! Max.
//!
//! **Apple M4 and later** have one matrix unit that this crate uses for
//! inputs of 16 KiB and more; when several threads of a program hash such
//! inputs at the same moment, one runs at full speed and the others at
//! about two thirds of it. The multithreaded calls share the work among
//! the cores themselves, so a program gets the most from one thread making
//! them.
//!
//! # Key-dependent timing
//!
//! Hashing code treats secret key bytes, including derived keys, as data:
//! they select neither branches, memory-access addresses, nor kernels.
//! For a fixed public call shape, grouped calls do the same work whether
//! their hashers' keys are equal or different. Key equality selects neither
//! groups nor temporary storage sizes.
//!
//! Public call shape includes the mode kind (plain, keyed, or key derivation),
//! message and output lengths, input layout, update boundaries, hasher indices,
//! and size hints. Dispatch may also depend on CPU features and available
//! processing resources. A batch supplied with one [`Mode`] shares a key by
//! construction. Plain mode's fixed key is public.
//!
//! This is a software-level key-independence contract. Elapsed time varies
//! with public call shape, scheduling, clock speed, and machine load. Caller
//! callbacks and I/O have their own timing properties. Applications choose
//! which lengths, modes, and operation patterns they expose to observers.
//!
//! # Memory
//!
//! - **A [`Hasher`] is about 18 KiB**; box it where that matters.
//! - **These single-threaded calls allocate nothing**: [`convenience::hash`],
//!   [`throughput::hash`], [`throughput::hash_many`], [`Hasher::update`](convenience::Hasher::update)
//!   and [`Hasher::finalize`](convenience::Hasher::finalize), and [`OutputReader`].
//!   [`Hasher::update_reader`](convenience::Hasher::update_reader) uses a 1 MiB buffer while it reads past
//!   64 KiB, freed when it returns. [`throughput::hash_each`] keeps, on the
//!   thread that calls it, 32 bytes for each chunk of the collection's
//!   messages of 2 to 15 chunks and 16 for each such message: its first
//!   collection and a larger one grow that storage, and a repeated
//!   collection allocates nothing.
//! - **Grouped incremental calls** use bounded stack tables and tree buffers.
//!   [`throughput::update_each`] allocates and frees nothing.
//!   [`throughput::update_each_multithreaded`] may allocate worker-job lists
//!   when a large piece uses other cores.
//! - **[`throughput::Pipeline`]** pre-allocates its [`Limits`](throughput::Limits) rooms and message slots once at construction and allocates nothing thereafter on the calling thread or the workers.
//! - **[`throughput::hash_multithreaded`]** allocates nothing below 512 KiB,
//!   where it hashes on the calling thread. Above that length it keeps a list
//!   of pieces it hands out, freed when it returns: 48 bytes for each 128 KiB of
//!   input, and at most 200 bytes for each CPU (1 GiB on 16 CPUs: 390 KiB).
//! - **Once per process**: the startup self-test's buffers, about 45 KiB
//!   of the calling thread's stack while it runs (it allocates nothing),
//!   and the threads: by default a worker per CPU beyond the first, each with the
//!   standard library's stack, started by [`initialize_multithreaded`].
//!
//! # Panics
//!
//! A broken precondition of a call panics on the calling thread, and
//! unwinds as usual.
//!
//! # Startup self-test
//!
//! **Every program calls [`initialize`] (or [`initialize_multithreaded`])
//! once, before any other call of this crate**: a call before it panics, in
//! every build, saying so. The check costs nothing: it is part of the
//! platform load every call makes. **A program that makes multithreaded
//! calls calls [`initialize_multithreaded`]**: a multithreaded call
//! before it panics (one load per call).
//!
//! `initialize` checks the build and the CPU before the process first
//! hashes, in well under a millisecond. It runs every assembly kernel built
//! for this machine on fixed inputs and compares the outputs with fixed
//! values official BLAKE3 generated. A mismatch stops the process, which no
//! caller can catch: with `std` it names the code path on stderr and
//! aborts; without it, the CPU's trap instruction stops it. Other threads
//! that call `initialize` meanwhile wait for it.
//!
//! # Cargo Features
//!
//! This crate needs the standard library; the core alone builds without
//! it. The `zeroize` and `serde` features (off by default) are the core's,
//! passed on: `Zeroize` for its types, and `Serialize` and `Deserialize`
//! for [`Hash`](struct@Hash).
//!
//! The build needs a C compiler that assembles the kernels (GCC 12 or
//! later, or Clang); without one it stops and says so. This crate runs its
//! assembly or not at all: for portable Rust, use official BLAKE3.
//!
//! [BLAKE3]: https://blake3.io

#![allow(clippy::too_many_arguments)]

pub use blake3_servil_core::{BLOCK_LEN, BUILD_VERSION, CHUNK_LEN, ContextKey, Hash, HexError, KEY_LEN, Mode, OUT_LEN, OutputReader, convenience, initialize};

// The core's inside, under the names this crate's files know it by.
#[allow(unused_imports)]
pub(crate) use blake3_servil_core::IncrementCounter;
#[allow(unused_imports)]
pub(crate) use blake3_servil_core::convenience::Hasher;
#[allow(unused_imports)]
pub(crate) use blake3_servil_core::hazmat::platform::MAX_SIMD_DEGREE;
#[allow(unused_imports)]
pub(crate) use blake3_servil_core::hazmat::{
    CHUNK_END, CHUNK_START, CVBytes, CVWords, DERIVE_KEY_MATERIAL, IV, KEYED_HASH, MAX_DEPTH, Output, ROOT, compress_parents_parallel,
    compress_subtree_to_parent_node, hash_all_at_once, hash_many_serial, hash_serial, hash_serial_on, hasher, incremental, many, platform, tree,
};
#[allow(unused_imports)]
pub(crate) mod block {
    pub use blake3_servil_core::throughput::{Batch, Block};
}
pub(crate) mod each {
    pub use blake3_servil_core::hazmat::each::*;
}

/// The primitives src/finish.rs runs on: the standard library's here, and
/// Shuttle's in tests/shuttle_finish.rs, which compiles that file beside its own
/// `finish_sync`.
mod finish_sync {
    pub use std::sync::Mutex;
    pub use std::sync::atomic::{AtomicUsize, Ordering};
    pub use std::thread;
}
mod finish;
/// The primitives src/park.rs runs on: the standard library's here, and
/// Shuttle's in tests/shuttle_park.rs, which compiles that file beside its
/// own `park_sync`.
mod park_sync {
    pub use std::sync::OnceLock;
    pub use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    pub use std::thread;
}
mod waker_sync {
    pub use core::cell::UnsafeCell;
    pub use std::sync::atomic::{AtomicUsize, Ordering};
    #[inline(always)]
    pub fn spin() {
        core::hint::spin_loop();
    }
    #[inline(always)]
    pub unsafe fn with_mut<T, R>(cell: &UnsafeCell<T>, f: impl FnOnce(&mut T) -> R) -> R {
        f(unsafe { &mut *cell.get() })
    }
}
mod lanes;
mod park;
mod pipeline;
mod waker;

#[cfg(test)]
mod forms_test;
/// The tests' helpers, the core's own file: this crate's tests use the
/// same messages, fixed values and guard pages.
#[cfg(test)]
#[path = "../../src/test_support.rs"]
#[allow(dead_code)] // Some serve the core's tests alone.
pub(crate) mod test;

/// A kind of path a call of this crate's ran, for its tests (the core
/// has its own record).
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub(crate) enum Ran {
    Pool = 1,
    EqualMembers = 4,
    MemberAlone = 8,
}
#[cfg(test)]
std::thread_local! {
    pub(crate) static RAN: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
}
#[inline(always)]
#[allow(dead_code)]
pub(crate) fn ran(kind: Ran) {
    #[cfg(test)]
    RAN.with(|r| r.set(r.get() | kind as u32));
    let _ = kind;
}

/// [`initialize`], and start the threads that the multithreaded calls
/// use: every program that makes such calls calls this once,
/// before them, in place of `initialize`. It starts `threads - 1` workers,
/// and each multithreaded call hashes on its calling thread and the
/// workers (calls made at once share the workers), and on CPUs with SME2
/// (Apple M4) the SME2 thread too; `None` means one thread
/// a CPU (`std::thread::available_parallelism`, which follows the CPU
/// affinity the program runs under). With 1, calls hash on their calling
/// thread alone: no workers start, nor the SME2 thread. Otherwise it
/// starts the first worker and returns; the first worker starts the others
/// (each 20-30 us on Linux x86-64), and calls run on the workers started
/// so far. The
/// threads wait asleep whenever no call has work for them (their memory:
/// see [Memory](crate#memory)). Later calls return at once, and must ask
/// for the same number: another `Some` panics. A panic on a worker thread
/// (a bug in this crate) ends the process, its message printed, rather
/// than leaving the call that waits for that worker's piece waiting
/// forever.
///
/// `None` counts logical CPUs, so on a CPU with SMT (Hyper-Threading) both
/// threads of each core, and that is the faster default. On an EPYC 9255,
/// the 16-input kernels leave much of a core waiting on its loads, and a
/// core's second thread fills that time: two threads hash 1.4-1.7 times
/// what one does, as with official BLAKE3. The 32-input AVX-512 kernel
/// (Linux) keeps a core busier on its own, so a second thread adds less
/// (1.1-1.3 times); even so, on 1 GiB 24 threads, one a core, took 4-14%
/// longer than 48. A thread slowed by the other thread on its core, by an
/// efficiency core or by another program takes fewer of a call's pieces,
/// and the pieces shrink toward the end of the input, so the slowest thread
/// finishes holding a small one. To leave CPUs to other work, pass fewer
/// threads.
///
/// On Linux, on a machine with more than one last-level cache (several
/// sockets, or an EPYC, whose cores share one in groups), each worker is
/// kept to the CPUs under one of them (`sched_setaffinity`), among the CPUs
/// the process may run on, and a call wakes the workers under its own
/// thread's cache first: what the caller has just written is in that
/// cache, and a worker under another fetches it across the machine. A
/// call of under 48 MiB stays with those workers. The calling threads'
/// own affinity is never set. On 2x EPYC 7443 a 1 MiB message read into
/// a buffer and hashed takes 74 us this way; with the workers wherever
/// the system put them it took 437 us, more than the 236 us of one thread.
///
/// Several programs each with a full pool share the machine without
/// trouble, and unevenly. On an i7-12700K (20 CPUs), N programs each
/// streaming 64 KiB messages hashed 26 GB/s alone, 37-41 together at 2
/// to 10, and 21-28 at 20 to 80 (400 to 1600 threads): the machine's
/// total holds, and oversubscription costs it 31% at one program a core
/// and 48% at four. What each program gets is the OS's choice among
/// equal threads: at 20 programs the luckiest got 2.6 times the
/// unluckiest, at 80, 8 times (bench-hashes #116, November 2026). A
/// program that needs a predictable share beside others sizes its pool
/// to its share of the machine.
///
/// The threads are this process's: in a child forked after they started,
/// every multithreaded call and stream panics, saying so (the child has
/// none of them). A program that forks initializes the pool in the process
/// that hashes, after any fork; its single-threaded calls work anywhere.
///
/// Each thread that makes multithreaded calls keeps a little storage for
/// them (its cut of the input, 48 KiB for inputs up to 64 MiB, and its
/// job record), grown on its first call and on the first call that
/// outgrows it, and otherwise untouched: after that, a call allocates
/// nothing. This call prepares the calling thread's; a call from another
/// thread prepares that thread's the first time.
///
/// ```
/// blake3_servil::initialize_multithreaded(None);
/// let message = vec![blake3_servil::throughput::Block::ZERO; (1 << 20) / 64];
/// let hash = blake3_servil::throughput::hash_multithreaded(blake3_servil::Mode::Plain, &message, 1 << 20);
/// assert_eq!(hash, blake3_servil::convenience::hash(&[0u8; 1 << 20]));
/// ```
pub fn initialize_multithreaded(threads: Option<core::num::NonZeroUsize>) {
    static STARTED: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let cpus = || std::thread::available_parallelism().map_or(1, |n| n.get());
    let started = *STARTED.get_or_init(|| {
        let threads = threads.map_or_else(cpus, |n| n.get());
        initialize();
        lanes::initialize(threads);
        threads
    });
    lanes::prepare_thread();
    if let Some(threads) = threads {
        assert_eq!(
            started,
            threads.get(),
            "blake3-servil: the multithreaded calls were started with {started} threads; initialize_multithreaded cannot change that to {threads}"
        );
    }
}

/// Whether every worker of the multithreaded calls' pool is parked (false
/// before [`initialize_multithreaded`]). Undocumented and unstable, for
/// tests only: a test that needs the pool idle waits for this, not for a
/// sleep that a loaded machine could outlast.
#[doc(hidden)]
pub fn __all_workers_parked() -> bool {
    lanes::all_workers_parked()
}

/// The throughput API: the core's, and the calls that use other cores.
pub mod throughput {
    pub use crate::pipeline::{Event, Lent, Limits, Message, Pipeline, ROOM_LEN, Room, block_on};
    use crate::platform::Platform;
    use crate::{Hash, Hasher, Mode, OUT_LEN};
    use blake3_servil_core::hazmat::each::{Item, hash_each_on};
    pub use blake3_servil_core::throughput::*;

    /// This crate's pool, for the core's Hasher: its whole subtrees of
    /// MIN_SPLIT_LEN and more.
    const POOL: crate::incremental::Pool = crate::incremental::Pool { min_len: crate::lanes::MIN_SPLIT_LEN, children: crate::lanes::subtree_children };

    /// [`Hasher::update`](crate::convenience::Hasher::update), using other CPU
    /// cores where that is faster: the same result, never slower. Pieces of
    /// 512 KiB and more use them, as [`hash_multithreaded`] does; shorter ones
    /// hash on the calling thread. One `Hasher` may take both calls. The
    /// program calls
    /// [`initialize_multithreaded`](crate::initialize_multithreaded) first.
    ///
    /// ```
    /// # blake3_servil::initialize_multithreaded(None);
    /// use blake3_servil::{Mode, convenience::Hasher, throughput};
    /// let input = vec![7u8; 3 << 20];
    /// let mut hasher = Hasher::new(Mode::Plain, Some(input.len() as u64));
    /// for piece in input.chunks(64 * 1024) {
    ///     throughput::update_multithreaded(&mut hasher, piece);
    /// }
    /// assert_eq!(hasher.finalize(), blake3_servil::convenience::hash(&input));
    /// ```
    pub fn update_multithreaded<'h>(hasher: &'h mut crate::convenience::Hasher, input: &[u8]) -> &'h mut crate::convenience::Hasher {
        crate::lanes::require();
        hasher.update_on(input, POOL)
    }

    /// [`hash_many`], using other CPU cores where that is faster: the same
    /// hashes, never slower.
    ///
    /// ```
    /// # blake3_servil::initialize_multithreaded(None);
    /// use blake3_servil::Mode;
    /// let mut leaves = blake3_servil::throughput::Batch::new(256, 4096);
    /// (0..4096).for_each(|i| leaves.set(i, &[7u8; 256]));
    /// let mut hashes = vec![[0u8; 32]; 4096];
    /// blake3_servil::throughput::hash_many_multithreaded(Mode::Plain, &leaves, &mut hashes);
    /// assert!(hashes.iter().all(|h| h == blake3_servil::convenience::hash(&[7u8; 256]).as_bytes()));
    /// ```
    ///
    /// **Speed**: below 512 KiB in all it runs on the calling thread, at
    /// [`hash_many`]'s speed; larger batches use the other cores too (on an
    /// Apple M4 Max, 64-byte messages from about 10 down to 3 ns each). It
    /// shares the cores with other callers as [`hash_multithreaded`] does.
    // Inlined, so a constant mode folds to its constants (as
    // convenience::hash is: no mode stored for the call to read).
    #[inline(always)]
    pub fn hash_many_multithreaded<P: Prepared>(mode: Mode, batch: &P, out: &mut [[u8; OUT_LEN]]) {
        crate::lanes::require();
        let mut room = core::mem::MaybeUninit::uninit();
        let (key, flags) = mode.key(&mut room);
        // Its length against the digests here, on the caller's thread, as
        // hash_many's kernels check it: a multiplication.
        crate::many::assert_slots(batch.bytes(), batch.message_len(), batch.slot(), out.len());
        crate::lanes::hash_many(batch.bytes(), batch.message_len(), batch.slot(), key, flags, out);
    }

    /// [`hash`], using other CPU cores where that is faster: the same
    /// [`Hash`](struct@Hash), never slower. For a message whose bytes are not
    /// in blocks (a mapped file, say), [`update_multithreaded`] into a
    /// [`Hasher`] takes any slice, at the same
    /// speed on the i7-12700K from 512 KiB to 1 GiB (about 1% slower at 1 MiB).
    ///
    /// ```
    /// # blake3_servil::initialize_multithreaded(None);
    /// use blake3_servil::{Mode, throughput::{self, Block}};
    /// let message = vec![Block::ZERO; 1 << 14];
    /// let hash = throughput::hash_multithreaded(Mode::Plain, &message, 1 << 20);
    /// assert_eq!(hash, blake3_servil::convenience::hash(&[0u8; 1 << 20]));
    /// ```
    ///
    /// **Speed**, on an Apple M4 Max (16 cores): below 512 KiB it hashes on the
    /// calling thread, at [`hash`](crate::convenience::hash)'s speed; from 512 KiB it uses the other cores
    /// too: about 14 GB/s at 1 MiB, 21 GB/s at 4 MiB, and 26 to 29 GB/s from
    /// 16 MiB, 2 to 5 times [`hash`](crate::convenience::hash). The extra threads wait asleep between calls and
    /// use no CPU then.
    ///
    /// **Several callers at once**: calls from several threads of one program
    /// share the cores in turn, each getting a fair part; a call that finds the
    /// cores all busy hashes on its own thread, at [`hash`](crate::convenience::hash)'s speed.
    ///
    /// **CPUs with performance and efficiency cores** (Intel's since Alder
    /// Lake): a multithreaded call or stream uses every core, and afterwards
    /// the OS sometimes leaves the calling thread on an efficiency core for a
    /// while, where single-threaded hashing runs at about half speed (AVX2's
    /// 256-bit operations run as two halves there). Measured on an i7-12700K
    /// under Linux 7.0: beside nonstop multithreaded streams, 7% of 16 KiB
    /// single-threaded hashes ran partly on an efficiency core, 2.2 times
    /// slower, about 6% on their average. A program that alternates the two
    /// and needs its single-threaded speed steady can keep that thread on the
    /// performance cores (`sched_setaffinity` on Linux).
    ///
    /// The program calls [`initialize_multithreaded`](crate::initialize_multithreaded) first; a call before it
    /// panics.
    // Inlined, so a constant mode folds to its constants (as
    // convenience::hash is: no mode stored for the call to read).
    #[inline]
    pub fn hash_multithreaded(mode: Mode, input: &[Block], len: usize) -> Hash {
        crate::lanes::require();
        let mut room = core::mem::MaybeUninit::uninit();
        let (key, flags) = mode.key(&mut room);
        crate::lanes::hash_with_key(&Block::bytes(input)[..len], key, flags)
    }

    /// [`hash_each`], using other CPU cores where that is faster: the
    /// same hashes, never slower. Collections of 512 KiB and more in all use
    /// them, as [`hash_multithreaded`] does.
    /// Requires `out.len() == items.len()`.
    ///
    /// ```
    /// # blake3_servil::initialize_multithreaded(None);
    /// use blake3_servil::{Mode, throughput::{self, Block}};
    /// let blocks: Vec<Vec<Block>> = (0..100).map(|i| vec![Block([i as u8; 64]); (1000 * i as usize).div_ceil(64)]).collect();
    /// let items: Vec<(&[Block], usize)> = blocks.iter().enumerate().map(|(i, b)| (&b[..], 1000 * i)).collect();
    /// let (mut a, mut b) = (vec![[0u8; 32]; 100], vec![[0u8; 32]; 100]);
    /// throughput::hash_each_multithreaded(Mode::Plain, &items, &mut a);
    /// throughput::hash_each(Mode::Plain, &items, &mut b);
    /// assert_eq!(a, b);
    /// ```
    pub fn hash_each_multithreaded(mode: Mode, items: &[(&[Block], usize)], out: &mut [[u8; OUT_LEN]]) {
        crate::lanes::require();
        assert_eq!(items.len(), out.len(), "one digest per item");
        let mut room = core::mem::MaybeUninit::uninit();
        let (key, flags) = mode.key(&mut room);
        let alone_len = |len: usize| len >= crate::lanes::MIN_SPLIT_LEN;
        let alone = |item: &[u8]| alone_len(item.len());
        // The items that hash alone are set aside in the pool's collection
        // call (a zero stand-in) and hashed after, each over the whole pool.
        if items.iter().map(|i| i.bytes().len()).filter(|&len| !alone_len(len)).sum::<usize>() < crate::lanes::MIN_SPLIT_LEN {
            hash_each_on(key, flags, items, out, Platform::detect(), |item| {
                if alone(item) { Hash::from_bytes([0; OUT_LEN]) } else { crate::hash_serial(item, key, flags) }
            });
        } else {
            crate::lanes::hash_each(key, flags, items, out);
        }
        for (item, out) in items.iter().map(Item::bytes).zip(out.iter_mut()) {
            if alone(item) {
                *out = *crate::lanes::hash_with_key(item, key, flags).as_bytes();
            }
        }
    }

    /// [`update_each`], using other CPU cores where
    /// that is faster (pieces of 512 KiB and more): the same result, never
    /// slower.
    /// Requires each `i < hashers.len()`.
    ///
    /// # Key-dependent timing
    ///
    /// The same [key-independence contract](crate#key-dependent-timing) as
    /// [`update_each`] applies when workers participate.
    pub fn update_each_multithreaded(hashers: &mut [Hasher], pieces: &[(usize, &[u8])]) {
        crate::lanes::require();
        blake3_servil_core::hazmat::each::update_turn(hashers, pieces, Some(POOL));
    }
}
