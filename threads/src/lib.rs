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
//! | One message in memory (now and then) | [`convenience::hash`], or for large inputs across cores [`throughput::hash_multithreaded`] |
//! | One message in memory, repeated call after call | [`throughput::hash_multithreaded`] (on aligned [`Block`](throughput::Block)s), or on one thread [`throughput::TreeHasher::hash`] directly on caller-owned `&[u8]` slices without copying into aligned blocks (use [`throughput::update_multithreaded`] for large unaligned slices across cores) |
//! | One message arriving in pieces | [`convenience::Hasher`], or for speed [`throughput::update_multithreaded`] |
//! | Many messages of one length, in one buffer | [`throughput::hash_many_multithreaded`] over a [`throughput::Batch`] |
//! | Many 16- or 32-byte messages (a Merkle layer's leaves or digests) | [`throughput::hash_many_multithreaded`] over a [`throughput::Batch`] (fastest for already-prepared batches) or [`throughput::Packed`] (compact storage; expands on the stack internally, avoids refill copies in chains) |
//! | Many messages of different lengths, in memory | [`throughput::hash_each_multithreaded`] |
//! | Many messages arriving at once (a server's uploads) | [`throughput::Pipeline`]; on one thread, or holding each message's running state, [`throughput::update_each`] |
//! | Continuous stream of messages or batches | [`throughput::Pipeline`] |
//!
//! Each call returns once its hash is done, and uses other CPU cores
//! where that is faster (the forms without `multithreaded` stay on the
//! calling thread). As a current heuristic (not a frozen contract),
//! [`throughput::hash_multithreaded`] and
//! [`throughput::hash_many_multithreaded`] stay on the calling thread
//! below 512 KiB of stored input bytes — raw input length for single
//! messages, `count × max(64, round_up(message_len, 64))` for a
//! [`throughput::Batch`], and `count × N` for a [`throughput::Packed<N>`] —
//! and at or above 512 KiB are eligible to use available worker threads
//! (batches also require at least two messages).
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
//! - **A [`Hasher`] is 26 KiB**; box it where that matters.
//! - **These single-threaded calls allocate nothing**: [`convenience::hash`],
//!   [`throughput::hash`], [`throughput::hash_many`], [`Hasher::update`](convenience::Hasher::update)
//!   and [`Hasher::finalize`](convenience::Hasher::finalize), and [`OutputReader`].
//!   [`Hasher::update_reader`](convenience::Hasher::update_reader) uses a 64 KiB
//!   stack buffer for the first read (even on immediate EOF), and promotes to a
//!   1 MiB heap buffer once 65,536 bytes or more are read before EOF, freed when
//!   it returns. [`throughput::hash_each`] keeps, on the
//!   thread that calls it, 32 bytes for each chunk of the collection's
//!   messages of 2 to 15 chunks and 16 for each such message: its first
//!   collection and a larger one grow that storage, and a repeated
//!   collection allocates nothing.
//! - **Grouped incremental calls** use bounded stack tables and tree buffers.
//!   [`throughput::update_each`] allocates and frees nothing.
//! - **[`throughput::Pipeline`]** pre-allocates its [`Limits`](throughput::Limits) rooms and message slots once at construction and allocates nothing thereafter on the calling thread or the workers.
//! - **[`throughput::hash_multithreaded`]** allocates nothing below 512 KiB,
//!   where it hashes on the calling thread. Above that length it keeps a list
//!   of the pieces it hands out, 56 bytes for each piece (most are of up
//!   to 128 KiB).
//!   The list is the calling thread's and stays with it:
//!   [`initialize_multithreaded`] makes 56 KiB of it ready on its own
//!   thread (1,024 pieces), a call that needs more pieces grows it, and
//!   the thread keeps it at that size (after 1 GiB on 16 CPUs without
//!   SME2, 578 KiB).
//!   A call that needs no more pieces than an earlier one on that thread
//!   allocates nothing. The pieces follow the cut and not the length
//!   alone: 1 GiB less one byte is cut into more pieces than 1 GiB.
//! - **Once per process**: the startup self-test's buffers, about 45 KiB
//!   of the calling thread's stack while it runs (it allocates nothing),
//!   and the threads: by default a worker per CPU beyond the first, each with the
//!   standard library's stack, started by [`initialize_multithreaded`].
//! - **Nothing is wiped**: this crate does not clear what it held from
//!   memory, and offers no call that does. A key, the message bytes a
//!   [`Hasher`] or a Pipeline holds and the chaining values made from
//!   them stay in the value, in the copies the kernels leave on the
//!   stacks, and in the pool's and the Pipelines' storage, until
//!   something else overwrites them. Official BLAKE3 offers `Zeroize` for
//!   its types.
//!
//! # Panics
//!
//! A broken precondition of a call panics on the calling thread, and
//! unwinds as usual. This crate itself ends the whole process in the
//! cases below, with a line on standard error where that takes one; a
//! program cannot catch them. (Rust's own rules end a process too: a
//! panic while another unwinds, as in an owner's destructor.)
//!
//! - Two breaches, since worker threads may be reading memory that the
//!   unwinding would free: a [`Pipeline`](throughput::Pipeline) dropped
//!   while its messages are still being hashed, and a
//!   [`Lent`](throughput::Lent) guard dropped while its message's hashing
//!   has not finished. A server keeps both from happening when a client
//!   goes away: see "Giving up a message" on the Pipeline. In a child
//!   forked while such work was in flight, either drop ends the child
//!   ([`initialize_multithreaded`]).
//! - A worker thread that the system refuses to start, and a panic on a
//!   worker thread, which speaks through the panic hook first
//!   ([`initialize_multithreaded`] tells both).
//! - A [`Waker`](core::task::Waker) of the program's that panics when it
//!   is woken, on whichever thread (the Pipeline's "Panics").
//! - A startup self-test that fails, and an x86-64 CPU without SSE4.1
//!   ([`initialize`]).
//! - A panic out of this crate's own hashing on a calling thread while
//!   workers hold that call's job: a fault of the crate's, which no
//!   input is known to cause.
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
//! caller can catch: with `std` it writes a best-effort diagnostic directly to
//! file descriptor 2 (bypassing Rust's stderr lock and skipping an
//! already-full pipe) and aborts; without it, the CPU's trap instruction stops
//! it. Other threads that call `initialize` meanwhile wait for it (parked via
//! `std::sync::Once`).
//!
//! # Cargo Features
//!
//! This crate needs the standard library; the core alone builds without
//! it. The `serde` feature (off by default) is the core's, passed on:
//! `Serialize` and `Deserialize` for [`Hash`](struct@Hash).
//!
//! The build needs a C compiler that assembles the kernels (GCC 12 or
//! later, or Clang); without one it stops and says so. This crate runs its
//! assembly or not at all: for portable Rust, use official BLAKE3.
//!
//! [BLAKE3]: https://blake3.io

#![allow(clippy::too_many_arguments)]

pub use blake3_servil_core::{
    BLOCK_LEN, BUILD_VERSION, CHUNK_LEN, ContextKey, Hash, HexError, HexHash, KEY_LEN, Mode, OUT_LEN, OutputReader, convenience, initialize,
};

pub(crate) use blake3_servil_core::convenience::Hasher;
pub(crate) use blake3_servil_core::thread_support::{MAX_SIMD_DEGREE, Platform, Sme2Turn, assert_slots, prepare_collection_thread, slot_len};
#[allow(unused_imports)]
pub(crate) mod block {
    pub use blake3_servil_core::throughput::{Batch, Block};
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
    pub use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering, fence};
    pub use std::thread;
}
mod waker_sync {
    pub use core::cell::UnsafeCell;
    pub use std::sync::atomic::{AtomicUsize, Ordering, fence};
    #[inline(always)]
    pub fn spin() {
        core::hint::spin_loop();
    }
    #[inline(always)]
    pub unsafe fn with_mut<T, R>(cell: &UnsafeCell<T>, f: impl FnOnce(&mut T) -> R) -> R {
        f(unsafe { &mut *cell.get() })
    }
}
/// The primitives src/pipeline.rs runs on: the standard library's here,
/// and Shuttle's in tests/shuttle_pipeline.rs, which compiles that file
/// beside its own `pipeline_sync`.
mod pipeline_sync {
    pub use core::cell::UnsafeCell;
    pub use std::sync::Mutex;
    pub use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    pub use std::thread;
    pub use std::thread_local;
    /// A static for the life of the process. (The model's is for one run.)
    macro_rules! process_static {
        ($(#[$attribute:meta])* static $name:ident: $kind:ty = $first:expr;) => {
            $(#[$attribute])*
            static $name: $kind = $first;
        };
    }
    pub(crate) use process_static;
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
/// (Apple M4) the SME2 thread too. `None` uses the available logical CPUs,
/// limited on Linux to the allowed CPUs of one memory node (below). With
/// 1, calls hash on their calling
/// thread alone: no workers start, nor the SME2 thread. Otherwise it
/// starts the first worker and returns; the first worker starts the others
/// (each 20-30 us on Linux x86-64), and calls run on the workers started
/// so far. The
/// threads wait asleep whenever no call has work for them (their memory:
/// see [Memory](crate#memory)). Later calls return at once, and must ask
/// for the same number: another `Some` panics. A panic on a worker thread
/// (a bug in this crate, or a waker of the program's that panics when a
/// worker wakes it) ends the process, once the process's panic hook has
/// returned (the standard one prints the panic's message, and waits
/// where standard error does not take it), rather than leaving the call
/// that waits for that worker's piece waiting forever. A worker thread
/// that the system refuses to start ends the process too, for the same
/// reason, with one line on standard error: a limit on the process's
/// threads (`RLIMIT_NPROC`, a container's `pids.max`) or a lack of
/// memory can refuse one. The first worker is started by this call and
/// the others by the first worker, so the end can come shortly after
/// this call has returned, and the program cannot catch it: one that
/// runs under such a limit passes a count of threads that fits.
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
/// threads. The count is also the one setting for energy against speed:
/// every thread started may be woken by a call, and while a `Pipeline` is
/// open one that finds no work looks for 5 us before it sleeps.
///
/// On Linux where the node topology is available and the system allows
/// it, workers stay on one memory node: the node of the initializing
/// thread's CPU or, where another node holds at least twice as many of
/// the CPUs that thread is allowed, the node that holds the most of
/// them. (The CPUs a
/// process is allowed need not lie evenly over the nodes, and its first
/// thread may start on a node that has one of them.)
/// On a machine with more than one last-level cache, each worker is kept
/// to a cache within that node.
/// A call wakes workers under its own cache first where it has any there.
/// Calling threads keep their own affinity and their buffer placement,
/// the initializing thread too: it may run on another node than the
/// workers.
/// To do this, on a Linux machine with more than one memory node, the
/// call reads the nodes' CPUs from `/sys/devices/system`, and the first
/// worker binds itself to the chosen node's with `sched_setaffinity`
/// before it starts the others, which inherit them (the SME2 thread
/// binds itself too).
/// All of this is for speed, and no call's result rests on it: what the
/// system does not tell or allow is left undone, each part by itself,
/// and the calls work as anywhere. Where the system shows no list of
/// memory nodes, or keeps from the process that list, the node of the
/// initializing CPU, that node's CPUs or the thread's own (a sandbox
/// may), or shows parts that do not agree (a node that is not online,
/// or does not hold its CPU), no node is chosen, and the workers start
/// with the initializing thread's CPUs. Another node is taken in place
/// of the initializing CPU's only where what the system shows of that
/// node agrees in the same way. Where it refuses a thread's
/// binding, to the node or later to a cache, that thread keeps the CPUs
/// it has. (A pool of one thread asked for by number,
/// `NonZeroUsize::new(1)`, has no workers: it reads no topology and
/// binds nothing.)
/// For each kind and size of call under 256 MiB, a caller with local
/// workers learns whether they or all the workers finish sooner.
///
/// The count `None` takes is the standard library's estimate
/// ([`std::thread::available_parallelism`], or 1 where it gives none)
/// and, where a node is chosen, the smaller of that and the node's
/// allowed CPUs. It is taken before any thread is bound, and a binding
/// refused afterwards does not change it. Where the system will not tell
/// a process its CPUs (it refuses `sched_getaffinity`), the estimate can
/// be more than the process may use, as many as the machine has online:
/// a program kept to fewer CPUs there does better to pass its number. A
/// request for `n` threads starts `n - 1` workers whether or not the
/// node, or the process, has `n` CPUs.
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
/// every multithreaded call panics, saying so (the child has none of
/// them), and so does a new [`Pipeline`](throughput::Pipeline). A
/// Pipeline made before the fork is not to be used in the child: its
/// calls panic the same way once one needs a worker, and one made while
/// the parent's workers were hashing for it may wait for ever instead.
/// Nor is it to be dropped there with work in flight at the fork, which
/// the child never sees finished: that drop, or the drop of a
/// [`Lent`](throughput::Lent) guard for such work, stops the child (a
/// child that leaves by `_exit` or `exec` drops neither). A
/// program that forks initializes the pool in the process that hashes,
/// after any fork; its single-threaded calls work anywhere.
///
/// Each thread that makes multithreaded calls keeps a little storage for
/// them (its cut of the input, 56 KiB for up to 1,024 pieces, and its
/// job record), grown on its first call and on the first call that
/// outgrows it, and otherwise untouched: a call that needs no more
/// pieces than an earlier one allocates nothing. This call prepares the calling thread's; a call from another
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
    let started = *STARTED.get_or_init(|| {
        initialize();
        lanes::initialize(threads)
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

/// Keep the command-line tool's main thread on one memory node before
/// it starts its file readers and hash workers, where the system tells
/// the node and allows the binding (as [`initialize_multithreaded`]
/// does for its workers).
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub fn __confine_cli_to_one_node() {
    lanes::places::confine_cli_thread();
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
    use crate::{Hash, Hasher, Mode, OUT_LEN};
    pub use blake3_servil_core::throughput::*;

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
    pub fn update_multithreaded<'h>(hasher: &'h mut Hasher, input: &[u8]) -> &'h mut Hasher {
        crate::lanes::require();
        hasher.update_with(input, crate::lanes::MIN_SPLIT_LEN, &crate::lanes::subtree_halves)
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
    /// **Speed**: below 512 KiB of stored input bytes (`count × max(64,
    /// round_up(message_len, 64))` for a [`Batch`], so a `Batch` of
    /// 4,096 × 36 B stays serial while 8,192 × 36 B qualifies; `count × N`
    /// for a [`Packed<N>`], so `Packed<32>` qualifies at 16,384 messages; an
    /// implementation heuristic, not a frozen contract), or with fewer than
    /// two messages, it runs on the calling thread at [`hash_many`]'s speed.
    /// Larger batches are eligible to use available worker threads (on an
    /// Apple M4 Max, 64-byte messages from about 10 down to 3 ns each). It
    /// shares the cores with other callers as [`hash_multithreaded`] does.
    // Inlined, so a constant mode folds to its constants (as
    // convenience::hash is: no mode stored for the call to read).
    #[inline(always)]
    pub fn hash_many_multithreaded<P: Prepared>(mode: Mode, batch: &P, out: &mut [[u8; OUT_LEN]]) {
        crate::lanes::require();
        // The short path first and inline, so a small batch costs what
        // hash_many costs (which checks its length against the digests);
        // everything the pool needs is behind the call after.
        if out.len() < 2 || batch.bytes().len() < crate::lanes::MIN_SPLIT_LEN {
            return hash_many(mode, batch, out);
        }
        // The whole batch as a part, its length checked against the
        // digests first, as hash_many checks it: a division here would
        // drop a ragged end (blocks borrowed that hold no whole number of
        // messages) where hash_many refuses it.
        assert_eq!(Some(batch.bytes().len()), batch.slot().checked_mul(out.len()), "input holds one slot per output");
        let (kind, key, context) = crate::lanes::parts(mode);
        crate::lanes::hash_many_in(kind, key, context, batch.part(0..out.len()), out);
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
    /// **Speed**, on an Apple M4 Max (16 cores): below 512 KiB (an
    /// implementation heuristic, not a frozen contract) it hashes on the
    /// calling thread, at [`hash`]'s speed; from 512 KiB it is eligible to
    /// use available worker threads too: about 14 GB/s at 1 MiB, 21 GB/s
    /// at 4 MiB, and 26 to 29 GB/s from 16 MiB, 2 to 5 times
    /// [`hash`](crate::convenience::hash). Idle workers park between calls
    /// and use no CPU once parked.
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
        // The short path first and inline, so a small input costs what
        // hash costs; everything the pool needs is behind the call after.
        if len < crate::lanes::MIN_SPLIT_LEN {
            return hash(mode, input, len);
        }
        let (kind, key, context) = crate::lanes::parts(mode);
        crate::lanes::hash_in(kind, key, context, &Block::bytes(input)[..len])
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
        let tree = TreeHasher::new(mode);
        let alone = |len: usize| len >= crate::lanes::MIN_SPLIT_LEN;
        // The items that hash alone are set aside in the collection's
        // call and hashed after, each over the whole pool.
        if items.iter().map(|i| i.1).filter(|&len| !alone(len)).sum::<usize>() < crate::lanes::MIN_SPLIT_LEN {
            crate::lanes::hash_each_small(&tree, items, out);
        } else {
            crate::lanes::hash_each(&tree, items, out);
        }
        for (item, out) in items.iter().zip(out.iter_mut()) {
            if alone(item.1) {
                *out = *crate::lanes::hash_over_pool(&tree, &Block::bytes(item.0)[..item.1]).as_bytes();
            }
        }
    }
}
