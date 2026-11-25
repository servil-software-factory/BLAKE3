//! `blake3-servil-core` is BLAKE3-Servil on one thread: a fast [BLAKE3]
//! implementation for 64-bit Arm and x86-64, its assembly kernels (each
//! proved against the specification) and every call that hashes on the
//! thread that makes it. It produces the same digests as official BLAKE3.
//! It starts no thread and uses no other core: the crate `blake3-servil`
//! is this one, whole, with the calls that do.
//!
//! # Examples
//!
//! ```
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Once per process, before any other call (see Startup self-test).
//! blake3_servil_core::initialize();
//!
//! // Hash an input all at once.
//! let hash1 = blake3_servil_core::convenience::hash(b"foobarbaz");
//!
//! // Hash an input incrementally.
//! let mut hasher = blake3_servil_core::convenience::Hasher::new(blake3_servil_core::Mode::Plain, None);
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
//! let mut hasher = blake3_servil_core::convenience::Hasher::new(blake3_servil_core::Mode::Plain, None);
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
//! use blake3_servil_core::{throughput::{self, Batch}, Mode};
//! let mut batch = Batch::new(64, 3);
//! for i in 0..3 {
//!     batch.set(i, &[i as u8; 64]);
//! }
//! let mut hashes = [[0u8; 32]; 3];
//! throughput::hash_many(Mode::Plain, &batch, &mut hashes);
//! assert_eq!(hashes[2], *blake3_servil_core::convenience::hash(&[2u8; 64]).as_bytes());
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
//!   hashing fast (batches, collections, many hashers fed in turns), tuned for
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
//! throughput call that takes it, `throughput::update_each` (with `std`),
//! is tuned as a throughput call.
//!
//! Start with the convenience calls; when a program needs more speed, it
//! moves to the throughput ones.
//!
//! | Your data | Call |
//! |---|---|
//! | One message in memory (now and then) | [`convenience::hash`] (or [`convenience::hash_with`]) |
//! | One message in memory, repeated call after call | [`throughput::hash`] (on aligned [`Block`](throughput::Block)s), or [`throughput::TreeHasher::hash`] directly on caller-owned `&[u8]` slices without copying into aligned blocks |
//! | One message arriving in pieces | [`convenience::Hasher`] |
//! | Many messages of one length, in one buffer | [`throughput::hash_many`] over a [`throughput::Batch`] |
//! | Many 16- or 32-byte messages (a Merkle layer's leaves or digests) | With `std`, [`throughput::hash_many`] over a [`throughput::Batch`] (fastest for already-prepared batches) or `throughput::Packed` (compact storage; expands on the stack internally, avoids refill copies in chains). Without `std`, a prepared [`throughput::TreeHasher`] per message. |
//! | Many messages of different lengths, in memory | `throughput::hash_each` (with `std`) |
//! | Many messages arriving at once (a server's uploads) | `throughput::update_each`, then `throughput::finalize_each` (with `std`) |
//!
//! Each call returns once its hash is done, on the calling thread. For
//! long inputs and large batches on several cores, the crate
//! `blake3-servil` adds a multithreaded form of each, and for messages
//! that keep arriving its Pipeline
//! (`throughput::update_each` says when each is the one to use).
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
//! about two thirds of it.
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
//! - **These single-threaded calls allocate nothing** (except on 512-bit SME2
//!   CPUs under `std`, where the first flat-walk tree of 32 KiB or more on
//!   each thread lazily allocates one reusable 64 KiB thread-local scratch
//!   block; without `std`, or during thread-local teardown after that block is
//!   destroyed, that 64 KiB scratch sits on the stack instead):
//!   [`convenience::hash`], [`throughput::hash`], [`throughput::hash_many`],
//!   [`Hasher::update`](convenience::Hasher::update) and
//!   [`Hasher::finalize`](convenience::Hasher::finalize), and [`OutputReader`].
//!   With `std`, `Hasher::update_reader` uses a 64 KiB stack buffer for the
//!   first read (even on immediate EOF), and promotes to a 1 MiB heap buffer
//!   once 65,536 bytes or more are read before EOF, freed when it returns.
//!   `throughput::hash_each` (with `std`) keeps reusable per-thread `Vec`
//!   storage for messages of 2 to 15 chunks (16 bytes per message and 32 bytes
//!   per chunk of payload, plus `Vec` capacity): its first such collection and
//!   a larger one grow that storage, and a repeated collection on a live
//!   thread allocates nothing; during thread-local teardown after that storage
//!   is destroyed, a collection with multi-chunk items falls back to
//!   call-local `Vec`s.
//! - **Grouped incremental calls** use bounded stack tables and tree buffers.
//!   `throughput::update_each` (with `std`) allocates and frees nothing beyond
//!   the one-time per-thread SME2 flat-walk scratch noted above if a fresh
//!   thread hashes a whole 32 KiB+ piece on a 512-bit SME2 CPU.
//! - **Once per process**: the startup self-test's buffers on the calling
//!   thread's stack while it runs (plus the SME2 flat-walk scratch noted above
//!   on 512-bit SME2 CPUs).
//! - **Nothing is wiped**: this crate does not clear what it held from
//!   memory, and offers no call that does. A key, the message bytes a
//!   [`Hasher`] holds and the chaining values made from them stay in the
//!   value, in the copies the kernels leave on the stack, and in the
//!   storage a thread keeps (`hash_each`'s, above; on CPUs with SME2,
//!   a 64 KiB block for the chaining values of the inputs the thread
//!   hashes, keyed ones included), until something else overwrites
//!   them. Official BLAKE3 offers `Zeroize` for its types.
//!
//! # Panics
//!
//! A broken precondition of a call panics on the calling thread, and
//! unwinds as usual.
//!
//! # Startup self-test
//!
//! **Every program calls [`initialize`] once, before hashing**: any call
//! that loads the detected platform (including [`convenience::hash`],
//! [`Hasher::new`](convenience::Hasher::new), `Mode::DeriveKey` context
//! hashing, and hashing methods on [`TreeHasher`](throughput::TreeHasher))
//! panics before `initialize` completes, in every build, saying so. Pure
//! constructors and `const` coordinate helpers that need no runtime platform
//! (such as [`TreeHasher::new`](throughput::TreeHasher::new) for `Plain`,
//! `Keyed`, or `DeriveKeyWith`, an empty `parent_cvs` call, and
//! [`throughput::left_len`]) do not load the platform. The check costs
//! nothing: it is part of the platform load made when selecting kernels.
//!
//! `initialize` checks the build and the CPU before the process first
//! hashes, in well under a millisecond. It runs the assembly kernels built
//! for this machine on fixed inputs and compares the outputs with fixed
//! values official BLAKE3 generated (on ARM64, all but the kernels for
//! batches of short messages of unequal lengths). A mismatch stops the process, which no
//! caller can catch: with `std` it writes a best-effort diagnostic directly to
//! file descriptor 2 (bypassing Rust's stderr lock and skipping an
//! already-full pipe) and aborts; without it, the CPU's trap instruction stops
//! it. Other threads that call `initialize` meanwhile wait for it (parked via
//! `std::sync::Once` under `std`; spinning without `std`, so `no_std` programs
//! with strict fixed-priority scheduling should call `initialize` before
//! starting higher-priority callers, and programs that `fork` should finish
//! `initialize` before forking).
//!
//! # Cargo Features
//!
//! The `std` feature (the only feature enabled by default) enables the
//! [`Write`] implementation and `Hasher::update_reader` for [`Hasher`],
//! the [`Read`] and [`Seek`] implementations for [`OutputReader`], owned
//! `Batch`/`Packed` builders, and the `hash_each`/`update_each` calls.
//!
//! All kernels are assembled directly by `rustc` via
//! `core::arch::global_asm!` (no external C compiler or `cc` build dependency
//! is required for the Rust libraries). This crate runs its assembly or not at
//! all: for portable Rust or other targets, use official BLAKE3.
//!
//! [BLAKE3]: https://blake3.io
//! [`Read`]: https://doc.rust-lang.org/std/io/trait.Read.html
//! [`Write`]: https://doc.rust-lang.org/std/io/trait.Write.html
//! [`Seek`]: https://doc.rust-lang.org/std/io/trait.Seek.html

#![cfg_attr(not(feature = "std"), no_std)]

// The tests use std in every build, the no_std one included.
#[cfg(all(test, not(feature = "std")))]
#[macro_use]
extern crate std;

#[cfg(all(test, feature = "std"))]
mod lifecycle_counts;
#[cfg(test)]
mod test;
#[cfg(test)]
mod test_support;

mod asm;
mod block;
pub mod convenience;
mod ct;
#[cfg(feature = "std")]
mod each;
pub mod throughput;

mod platform;

// Platform-specific implementations of the compression function. These
// BLAKE3-specific cfg flags are set in build.rs.
#[cfg(blake3_avx2_ffi)]
#[path = "ffi_avx2.rs"]
mod avx2;
#[cfg(blake3_avx512_ffi)]
#[path = "ffi_avx512.rs"]
mod avx512;
#[cfg(all(blake3_avx512_ffi, target_os = "linux"))]
#[path = "ffi_avx512_32.rs"]
mod avx512_32;
#[cfg(blake3_sse41_ffi)]
#[path = "ffi_chunk_root.rs"]
mod chunk_root;
mod ffi_counter;
#[cfg(all(feature = "std", blake3_avx512_ffi, target_os = "linux"))]
mod ffi_output;
#[cfg(all(feature = "std", blake3_neon))]
#[path = "ffi_output_aarch64.rs"]
mod ffi_output;
mod ffi_packed;
#[cfg(blake3_neon)]
#[path = "ffi_neon_hybrid.rs"]
mod neon_hybrid;
#[cfg(blake3_sme2)]
#[path = "ffi_sme2.rs"]
mod sme2;
// Whole chunks and a partial one in one pass, on x86-64 (generated by
// tools/gen_x86.py, built with the SSE4.1 and AVX2 assembly).
#[cfg(blake3_sse41_ffi)]
mod ffi_partial;
#[cfg(blake3_sse41_ffi)]
#[path = "ffi_sse41.rs"]
mod sse41;

#[cfg(feature = "std")]
mod io;
mod many;
mod pair;
mod self_test;
mod stack_vec;
mod tree;

/// A kind of kernel or path a call ran, for the tests that check which
/// path a call took. Each build notes the kinds it has.
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub(crate) enum Ran {
    /// The call went to the pool.
    Pool = 1,
    /// The call wanted SME2 and another thread held the turn.
    TurnTaken = 2,
    /// A task's members, all of one length, ran side by side on that
    /// length's multi-lane kernel (lanes::Task::run_equal_members).
    EqualMembers = 4,
    /// A task's members ran one at a time (lanes::Task::run_members).
    MemberAlone = 8,
    /// A batch of one-block messages ran on the kernels' table a run at a
    /// time (many::hash_many_on's first path).
    BlockRun = 16,
    /// A batch's messages of 2 to 15 chunks ran across messages
    /// (many::hash_across).
    Across = 32,
    /// A batch's messages ran one at a time (many::hash_long_or_single).
    MessageAlone = 64,
    /// A batch of messages of up to a chunk, two or more, ran on the
    /// kernels' table a run at a time (many::hash_many_on's short path).
    ShortRun = 128,
    /// Whole chunks, parents, or x86-64 multi-block batches were hashed on a
    /// wider kernel than their count, the spare lanes a copy of the last
    /// (rounded_lanes / round_x86_blocks).
    RoundedUp = 256,
    /// A table's one-block inputs were copied into one buffer for SME2's
    /// parent kernel (sme2::hash_many): a copy of the input that blocks
    /// handed over as the one buffer they lie in do not take
    /// (sme2::hash_block_run).
    Gathered = 512,
}

#[cfg(all(test, feature = "std"))]
std::thread_local! {
    /// The kinds this thread's calls ran since the test last cleared it.
    pub(crate) static RAN: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
    /// The lanes the rounded-up hash_many calls ran at since the test last
    /// cleared it, as a mask of widths (4 | 8 = 12: both).
    pub(crate) static ROUNDED_WIDTH: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
    /// `(blocks, inputs.len())` recorded at `Platform::hash_many` and
    /// `pair::hash_many_blocks`, for verifying actual kernel dispatch widths.
    pub(crate) static KERNEL_HASH_MANY_CALLS: core::cell::RefCell<std::vec::Vec<(usize, usize)>> = const { core::cell::RefCell::new(std::vec::Vec::new()) };
}

#[cfg(all(test, feature = "std"))]
#[inline(always)]
pub(crate) fn record_hash_many_call(blocks: usize, count: usize) {
    let _ = KERNEL_HASH_MANY_CALLS.try_with(|v| v.borrow_mut().push((blocks, count)));
}

/// Note that `kind` happened: in the tests' record, and nothing in any
/// other build (some of which have no kernel to note).
#[inline(always)]
#[allow(dead_code)]
pub(crate) fn ran(kind: Ran) {
    #[cfg(all(test, feature = "std"))]
    let _ = RAN.try_with(|r| r.set(r.get() | kind as u32));
    let _ = kind;
}

/// The work the tests count, for a mutant that only costs work (BLAKE3
/// #219): an exact count on the calling thread, the same on every machine,
/// where the digests are the same either way.
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub(crate) enum Counted {
    /// A round of hash_each_on's shared lanes: one hash_many call over the
    /// k-th chunk of every message that has one.
    EachRound = 0,
    /// One root output block computed alone (Output::root_output_block):
    /// OutputReader::fill's tails, where its whole blocks go to xof_many.
    RootBlock = 1,
    /// One group of short collection items hashed together by blocks.
    ByBlocksCall = 2,
    /// One x86-64 AVX-512/AVX2 finalize_each output dispatch turn (four or
    /// more eligible counter-zero hashers).
    OutputGroupTurn = 3,
    /// One pass of update_each's held groups hashed together.
    HeldPass = 4,
    /// One update of a block or less staged as a padded block
    /// (Hasher::update's path for an empty stage).
    PaddedFirstBlock = 5,
    /// One copy of bytes into a Hasher's stage (Hasher::stage): what an
    /// update's whole groups skip.
    Staged = 6,
    /// One whole subtree cut into a row for the Hasher's stack.
    Row = 7,
    /// One `finalize` that hashed the staged bytes apart and read the
    /// stack, the core not copied.
    TailApart = 8,
    /// One whole subtree a Hasher's update hashed on SME2's flat walk, to
    /// its one chaining value (the Hasher's hook `subtree_cv`).
    Flat = 9,
    /// One multi-lane SIMD `Platform::hash_many` dispatch call (#497).
    HashManyCall = 10,
}

#[cfg(all(test, feature = "std"))]
std::thread_local! {
    /// This thread's counts since the test last cleared them, by Counted.
    pub(crate) static COUNTED: [core::cell::Cell<u64>; 11] = const { [const { core::cell::Cell::new(0) }; 11] };
}

/// Count one of `what`: in the tests' record, and nothing in any other
/// build.
#[inline(always)]
#[allow(dead_code)]
pub(crate) fn counted(what: Counted) {
    #[cfg(all(test, feature = "std"))]
    let _ = COUNTED.try_with(|c| c[what as usize].set(c[what as usize].get() + 1));
    let _ = what;
}

#[cfg(all(test, feature = "std"))]
std::thread_local! {
    /// The assembly entry points this thread's calls ran since the test
    /// last cleared it, by address.
    pub(crate) static RAN_ENTRIES: core::cell::RefCell<std::collections::BTreeSet<usize>> = const { core::cell::RefCell::new(std::collections::BTreeSet::new()) };
}

/// Note that the assembly entry point at `entry` ran (in the tests'
/// record; nothing in any other build).
#[inline(always)]
#[allow(dead_code)]
pub(crate) fn ran_kernel(entry: *const ()) {
    #[cfg(all(test, feature = "std"))]
    let _ = RAN_ENTRIES.try_with(|r| r.borrow_mut().insert(entry as usize));
    let _ = entry;
}

use core::cmp;
use core::fmt;
use platform::{MAX_SIMD_DEGREE, Platform};
use stack_vec::StackVec;

/// This build's version: the crate's version, and for any build but a
/// release's own commit, the commit built and, from a dirty tree, the Git
/// tree id of its working tree, e.g. `0.8.0+dev.<commit>.dirty.<tree>`.
/// `b3sum --version` and the C library's `b3_version()` print it.
pub use blake3_servil_version::BUILD_VERSION;

/// The number of bytes in a [`struct@Hash`], 32.
pub const OUT_LEN: usize = 32;

/// The number of bytes in a key, 32.
pub const KEY_LEN: usize = 32;

/// The number of bytes in a block, 64.
///
/// You don't usually need to think about this number. One case where it matters is calling
/// [`OutputReader::fill`] in a loop, where using a `buf` argument that's a multiple of `BLOCK_LEN`
/// avoids repeating work.
pub const BLOCK_LEN: usize = 64;

/// The number of bytes in a chunk, 1024.
///
/// You don't usually need to think about this number, but it often comes up in benchmarks, because
/// the maximum degree of parallelism used by the implementation equals the number of chunks.
pub const CHUNK_LEN: usize = 1024;

pub(crate) const MAX_DEPTH: usize = 54; // 2^54 * CHUNK_LEN = 2^64

// A chunk's CV is words while its blocks compress (the kernels' form); the
// hash_many interface hashes input bytes and parent nodes alike, so its
// output CVs are bytes.
pub(crate) type CVWords = [u32; 8];
pub(crate) type CVBytes = [u8; 32]; // little-endian

pub(crate) const IV: &CVWords = &[0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19];

// These are the internal domain-separation flags in the low bits (0 through 6)
// of the block flags word in the compression function.
pub(crate) const CHUNK_START: u8 = 1 << 0;
pub(crate) const CHUNK_END: u8 = 1 << 1;
const PARENT: u8 = 1 << 2;
pub(crate) const ROOT: u8 = 1 << 3;
pub(crate) const KEYED_HASH: u8 = 1 << 4;
const DERIVE_KEY_CONTEXT: u8 = 1 << 5;
pub(crate) const DERIVE_KEY_MATERIAL: u8 = 1 << 6;

/// An output of the default size, 32 bytes, which provides constant-time
/// equality checking.
///
/// `Hash` implements [`From`] and [`Into`] for `[u8; 32]`, and it provides
/// [`from_bytes`] and [`as_bytes`] for explicit conversions between itself and
/// `[u8; 32]`. However, byte arrays and slices don't provide constant-time
/// equality checking, which is often a security requirement in software that
/// handles private data. `Hash` doesn't implement [`Deref`] or [`AsRef`], to
/// avoid situations where a type conversion happens implicitly and the
/// constant-time property is accidentally lost.
///
/// `Hash` provides the [`to_hex`] and [`from_hex`] methods for converting to
/// and from hexadecimal. It also implements [`Display`] and [`FromStr`].
///
/// [`From`]: https://doc.rust-lang.org/std/convert/trait.From.html
/// [`Into`]: https://doc.rust-lang.org/std/convert/trait.Into.html
/// [`as_bytes`]: #method.as_bytes
/// [`from_bytes`]: #method.from_bytes
/// [`Deref`]: https://doc.rust-lang.org/stable/std/ops/trait.Deref.html
/// [`AsRef`]: https://doc.rust-lang.org/std/convert/trait.AsRef.html
/// [`to_hex`]: #method.to_hex
/// [`from_hex`]: #method.from_hex
/// [`Display`]: https://doc.rust-lang.org/std/fmt/trait.Display.html
/// [`FromStr`]: https://doc.rust-lang.org/std/str/trait.FromStr.html
///
/// # Trait implementations
///
/// - `Clone` reproduces the digest byte for byte.
/// - `Copy` leaves the same digest available at both places.
/// - `Debug` includes the full digest in hexadecimal, even for a derived
///   key.
/// - `Display` prints all 32 bytes in lowercase hexadecimal, even for a
///   derived key.
/// - `Eq` treats digests with identical bytes as equal.
/// - `From` converts between the digest and its 32-byte array without
///   changing the bytes; array equality is not constant-time.
/// - `FromStr` decodes exactly 64 hexadecimal digits, accepting either
///   case.
/// - `Hash` feeds the length 32 and then the 32 digest bytes to whatever
///   `core::hash::Hasher` the caller supplies, whose running time may
///   depend on those bytes.
/// - `PartialEq` compares digests, arrays and slices in constant time for
///   32-byte operands; a different length is unequal.
#[derive(Clone, Copy, Eq)]
#[repr(transparent)]
pub struct Hash([u8; OUT_LEN]);

/// The 64 lowercase hexadecimal characters of a [`Hash`](struct@Hash), held
/// inline with no heap allocation.
///
/// Dereferences to `&str` and implements `Display`, `AsRef<str>`, `AsRef<[u8]>`,
/// and `PartialEq<&str>` / `PartialEq<str>`. Equality on `HexHash` is not
/// constant-time; compare [`Hash`](struct@Hash) values directly for secret tags or keys.
///
/// # Trait implementations
///
/// - `AsRef` borrows the 64 hexadecimal characters as `&str` or `&[u8]`.
/// - `Borrow` lends the 64 hexadecimal characters as `&str`, with the `str`'s own hash, equality and order, so a map keyed by `HexHash` finds a key by its `&str`.
/// - `Clone` copies the 64 ASCII bytes.
/// - `Copy` leaves the same 64 ASCII bytes available at both places.
/// - `Debug` formats the 64 hexadecimal characters as a quoted Rust string.
/// - `Deref` borrows the 64 hexadecimal characters as `&str`.
/// - `Display` formats the 64 lowercase hexadecimal characters as their `str` does, width and precision included.
/// - `Eq` reflects `PartialEq` over the 64 ASCII bytes (not constant-time).
/// - `Hash` feeds the hasher what the `str` of the 64 characters feeds it.
/// - `Ord` compares the 64 ASCII bytes lexicographically (not constant-time).
/// - `PartialEq` compares the 64 ASCII bytes against another `HexHash`, `str`, or `&str` (not constant-time).
/// - `PartialOrd` compares the 64 ASCII bytes lexicographically (not constant-time).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct HexHash([u8; 2 * OUT_LEN]);

impl HexHash {
    /// View the 64 hexadecimal characters as a `&str`.
    #[inline]
    pub fn as_str(&self) -> &str {
        // Sound: `Hash::to_hex` writes only ASCII '0'..='9' and 'a'..='f'.
        unsafe { core::str::from_utf8_unchecked(&self.0) }
    }

    /// View the 64 ASCII hexadecimal bytes.
    #[inline]
    pub fn as_bytes(&self) -> &[u8; 2 * OUT_LEN] {
        &self.0
    }
}

impl core::ops::Deref for HexHash {
    type Target = str;
    #[inline]
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for HexHash {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<[u8]> for HexHash {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl core::borrow::Borrow<str> for HexHash {
    #[inline]
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

// As the str hashes, which `Borrow<str>` requires: a derived hash of the
// byte array feeds the hasher other bytes, and a lookup by `&str` misses.
impl core::hash::Hash for HexHash {
    #[inline]
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.as_str().hash(state)
    }
}

impl fmt::Display for HexHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_str(), f)
    }
}

impl fmt::Debug for HexHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl PartialEq<str> for HexHash {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for HexHash {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl Hash {
    /// The raw bytes of the `Hash`. Note that byte arrays don't provide
    /// constant-time equality checking, so if you need to compare hashes,
    /// prefer the `Hash` type.
    #[inline]
    pub const fn as_bytes(&self) -> &[u8; OUT_LEN] {
        &self.0
    }

    /// Create a `Hash` from its raw bytes representation.
    pub const fn from_bytes(bytes: [u8; OUT_LEN]) -> Self {
        Self(bytes)
    }

    /// The raw bytes of the `Hash`, as a slice. Useful for serialization. Note that byte arrays
    /// don't provide constant-time equality checking, so if you need to compare hashes, prefer
    /// the `Hash` type.
    #[inline]
    pub const fn as_slice(&self) -> &[u8] {
        self.0.as_slice()
    }

    /// Create a `Hash` from its raw bytes representation as a slice.
    ///
    /// Returns an error if the slice is not exactly 32 bytes long.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, core::array::TryFromSliceError> {
        Ok(Self::from_bytes(bytes.try_into()?))
    }

    /// Encode a `Hash` in lowercase hexadecimal.
    ///
    /// Each character is computed from its four bits by arithmetic: no
    /// table is indexed by the hash and no branch is taken on it, so the
    /// call's time and memory accesses are the same for every hash (a
    /// hash may be a secret: a keyed hash's tag, a derived key).
    ///
    /// The returned [`HexHash`] holds the 64 ASCII hexadecimal bytes inline
    /// with no heap allocation and dereferences to `&str`. Note that
    /// [`HexHash`] doesn't provide constant-time equality checking, so if you
    /// need to compare hashes, prefer the [`Hash`](struct@Hash) type.
    pub fn to_hex(&self) -> HexHash {
        // 0..=9 are '0' + n; 10..=15 are 'a' + n - 10, which is '0' + n + 39:
        // (n + 6) >> 4 is 1 exactly for those.
        let nibble = |n: u8| b'0' + n + ((n + 6) >> 4) * 39;
        let mut bytes = [0u8; 2 * OUT_LEN];
        for (i, &b) in self.0.iter().enumerate() {
            bytes[2 * i] = nibble(b >> 4);
            bytes[2 * i + 1] = nibble(b & 0xf);
        }
        HexHash(bytes)
    }

    /// Decode a `Hash` from hexadecimal. Both uppercase and lowercase ASCII
    /// bytes are supported.
    ///
    /// Any byte outside the ranges `'0'...'9'`, `'a'...'f'`, and `'A'...'F'`
    /// results in an error. An input length other than 64 also results in an
    /// error.
    ///
    /// For 64 hexadecimal characters, the call's time and memory accesses
    /// are the same whatever they are: each is decoded by arithmetic, all
    /// 64 always. An input that is refused is then searched for the
    /// character the error names.
    ///
    /// Note that `Hash` also implements `FromStr`, so `Hash::from_hex("...")`
    /// is equivalent to `"...".parse()`.
    pub fn from_hex(hex: impl AsRef<[u8]>) -> Result<Self, HexError> {
        /// (the character's value, 0xff where it is a hex character and 0
        /// where not), with no branch: a difference below its range's size
        /// leaves the subtraction negative, whose sign fills the mask.
        fn hex_val(byte: u8) -> (u8, u8) {
            let digit = byte.wrapping_sub(b'0');
            let letter = (byte | 0x20).wrapping_sub(b'a');
            let is_digit = ((digit as i16 - 10) >> 8) as u8;
            let is_letter = ((letter as i16 - 6) >> 8) as u8;
            ((digit & is_digit) | (letter.wrapping_add(10) & is_letter), is_digit | is_letter)
        }
        let hex_bytes: &[u8] = hex.as_ref();
        if hex_bytes.len() != OUT_LEN * 2 {
            return Err(HexError(HexErrorInner::InvalidLen(hex_bytes.len())));
        }
        let mut hash_bytes: [u8; OUT_LEN] = [0; OUT_LEN];
        let mut all = 0xff;
        for i in 0..OUT_LEN {
            let (high, high_is) = hex_val(hex_bytes[2 * i]);
            let (low, low_is) = hex_val(hex_bytes[2 * i + 1]);
            hash_bytes[i] = high << 4 | low;
            all &= high_is & low_is;
        }
        if all != 0xff {
            // Refused: the first character that is none, for the error.
            let byte = *hex_bytes.iter().find(|&&b| hex_val(b).1 == 0).expect("a refused input holds a character that is no hex character");
            return Err(HexError(HexErrorInner::InvalidByte(byte)));
        }
        Ok(Hash::from(hash_bytes))
    }
}

impl From<[u8; OUT_LEN]> for Hash {
    #[inline]
    fn from(bytes: [u8; OUT_LEN]) -> Self {
        Self::from_bytes(bytes)
    }
}

impl From<Hash> for [u8; OUT_LEN] {
    #[inline]
    fn from(hash: Hash) -> Self {
        hash.0
    }
}

impl core::str::FromStr for Hash {
    type Err = HexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Hash::from_hex(s)
    }
}

/// This implementation is constant-time (branchless integer assembly; on Arm
/// it leaves `PSTATE.DIT` as the calling thread has it, matching hashing).
impl PartialEq for Hash {
    #[inline]
    fn eq(&self, other: &Hash) -> bool {
        ct::eq_32(&self.0, &other.0)
    }
}

impl core::hash::Hash for Hash {
    #[inline]
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

/// This implementation is constant-time (branchless integer assembly; on Arm
/// it leaves `PSTATE.DIT` as the calling thread has it, matching hashing).
impl PartialEq<[u8; OUT_LEN]> for Hash {
    #[inline]
    fn eq(&self, other: &[u8; OUT_LEN]) -> bool {
        ct::eq_32(&self.0, other)
    }
}

/// This implementation is constant-time if the target is 32 bytes long
/// (branchless integer assembly; on Arm it leaves `PSTATE.DIT` as the calling
/// thread has it, matching hashing).
impl PartialEq<[u8]> for Hash {
    #[inline]
    fn eq(&self, other: &[u8]) -> bool {
        ct::eq_slice_32(&self.0, other)
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // Formatting field as `&str` to reduce code size since the `Debug`
        // dynamic dispatch table for `&str` is likely needed elsewhere already,
        // but that for `HexHash` is not.
        let hex = self.to_hex();
        let hex: &str = hex.as_str();

        f.write_str(hex)
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // Formatting field as `&str` to reduce code size since the `Debug`
        // dynamic dispatch table for `&str` is likely needed elsewhere already,
        // but that for `HexHash` is not.
        let hex = self.to_hex();
        let hex: &str = hex.as_str();

        f.debug_tuple("Hash").field(&hex).finish()
    }
}

/// The error type for [`Hash::from_hex`].
///
/// The `.to_string()` representation of this error currently distinguishes between bad length
/// errors and bad character errors. This is to help with logging and debugging, but it isn't a
/// stable API detail, and it may change at any time.
///
/// # Trait implementations
///
/// - `Clone` copies the reason decoding failed.
/// - `Debug` shows the error variant and its invalid byte or length.
/// - `Display` describes the invalid byte or length without promising
///   stable wording.
/// - `Error` (with `std`) lets a caller report the decoding failure as a
///   standard error.
#[derive(Clone, Debug)]
pub struct HexError(HexErrorInner);

#[derive(Clone, Debug)]
enum HexErrorInner {
    InvalidByte(u8),
    InvalidLen(usize),
}

impl fmt::Display for HexError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.0 {
            HexErrorInner::InvalidByte(byte) => {
                if byte < 128 {
                    write!(f, "invalid hex character: {:?}", byte as char)
                } else {
                    write!(f, "invalid hex character: 0x{:x}", byte)
                }
            }
            HexErrorInner::InvalidLen(len) => {
                write!(f, "expected 64 hex bytes, received {}", len)
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for HexError {}

pub(crate) type Output = hasher::Output<Platform>;

impl Clone for Output {
    fn clone(&self) -> Self {
        hasher::Output {
            block: self.block,
            input_chaining_value: self.input_chaining_value,
            block_len: self.block_len,
            counter: self.counter,
            flags: self.flags,
            platform: self.platform,
        }
    }
}

impl Output {
    pub fn root_hash(&self) -> Hash {
        debug_assert_eq!(self.counter, 0);
        Hash(self.root_bytes())
    }

    fn root_output_block(&self) -> [u8; 2 * OUT_LEN] {
        counted(Counted::RootBlock);
        self.platform.compress_xof(&self.input_chaining_value, &self.block, self.block_len, self.counter, self.flags | ROOT)
    }
}

type ChunkState = hasher::ChunkState<Platform>;

impl hasher::Blocks for Platform {
    #[inline(always)]
    fn compress_in_place(&self, cv: &mut CVWords, block: &[u8; BLOCK_LEN], block_len: u8, counter: u64, flags: u8) {
        Platform::compress_in_place(self, cv, block, block_len, counter, flags)
    }

    #[inline(always)]
    fn compress_blocks(&self, cv: &mut CVWords, blocks: &[u8], count: usize, counter: u64, flags: u8, flags_start: u8) {
        Platform::compress_blocks(self, cv, blocks, count, counter, flags, flags_start)
    }
}

impl ChunkState {
    // Try to avoid buffering as much as possible, by compressing directly from
    // the input slice when full blocks are available (src/hasher.rs).
    #[inline(always)]
    fn update(&mut self, input: &[u8]) -> &mut Self {
        self.absorb(input);
        debug_assert!(self.count() <= CHUNK_LEN);
        self
    }
}

// IMPLEMENTATION NOTE
// ===================
/// Undocumented and unstable, for benchmarks only.
#[doc(hidden)]
#[derive(Clone, Copy)]
pub(crate) enum IncrementCounter {
    Yes,
    No,
}

impl IncrementCounter {
    #[inline]
    fn yes(&self) -> bool {
        match self {
            IncrementCounter::Yes => true,
            IncrementCounter::No => false,
        }
    }
}

/// The widths the platform's many-lane kernels run at, in one pass each,
/// rising: SSE4.1 four lanes; AVX2 eight (and four); AVX-512 sixteen (and
/// eight, four), and thirty-two where the 32-input kernel runs
/// (wide_avx512, Zen 5 on Linux). Below the smallest, one and two lanes
/// have passes of their own as cheap as one.
/// `platform.hash_many` over `inputs`, on `rounded_lanes` where a wider
/// kernel takes the count in one pass: the spare lanes hash a copy of the
/// last input, into scratch. Past the widest kernel, the whole groups of
/// its width first, then the rest rounded.
///
/// On x86-64, every power of two from 4 through the platform's widest kernel
/// is present in `lane_widths` (so powers of two never pad and larger powers
/// of two divide evenly by the widest kernel). Taking `platform.hash_many`
/// directly on powers of two keeps the 1 KiB scratch buffer and extra dispatch
/// copies out of hot callers' stack frames and cold instruction cache lines
/// (`#376`). Non-x86 architectures have no rounding tables and call
/// `platform.hash_many` directly with no extra branch.
#[inline]
#[allow(clippy::too_many_arguments)]
pub(crate) fn hash_many_rounded<const N: usize>(
    platform: Platform,
    inputs: &[&[u8; N]],
    key: &CVWords,
    counter: u64,
    increment_counter: IncrementCounter,
    flags: u8,
    flags_start: u8,
    flags_end: u8,
    out: &mut [u8],
) {
    #[cfg(blake3_sse41_ffi)]
    ffi_partial::hash_many_rounded(platform, inputs, key, counter, increment_counter, flags, flags_start, flags_end, out);
    #[cfg(not(blake3_sse41_ffi))]
    platform.hash_many(inputs, key, counter, increment_counter, flags, flags_start, flags_end, out);
}

#[cfg(all(test, blake3_sse41_ffi))]
use ffi_partial::{lane_widths, rounded_lanes};

// Use SIMD parallelism to hash up to MAX_SIMD_DEGREE chunks at the same time
// on a single thread. Write out the chunk chaining values and return the
// number of chunks hashed. These chunks are never the root and never empty;
// those cases use a different codepath.
fn compress_chunks_parallel(input: &[u8], key: &CVWords, chunk_counter: u64, flags: u8, platform: Platform, out: &mut [u8]) -> usize {
    debug_assert!(!input.is_empty(), "empty chunks below the root");
    debug_assert!(input.len() <= MAX_SIMD_DEGREE * CHUNK_LEN);

    let mut chunks_exact = input.chunks_exact(CHUNK_LEN);
    let mut chunks_array = StackVec::<&[u8; CHUNK_LEN], MAX_SIMD_DEGREE>::new();
    for chunk in &mut chunks_exact {
        chunks_array.push(chunk.try_into().unwrap());
    }
    // A whole chunk and a partial one, on AVX2 or AVX-512: their first
    // blocks side by side (pair.rs).
    if pair::takes(platform, input.len()) {
        out[..2 * OUT_LEN].copy_from_slice(&pair::parent(input, key, chunk_counter, flags, platform));
        return 2;
    }
    // Whole chunks that leave a lane spare, and a partial one: one pass,
    // the partial chunk in the spare lane.
    #[cfg(blake3_sse41_ffi)]
    if ffi_partial::try_hash_chunks_with_partial(platform, &chunks_array, chunks_exact.remainder(), key, chunk_counter, flags, out) {
        return chunks_array.len() + 1;
    }
    // Whole chunks and a partial one, on the NEON hybrids: where a q kernel
    // covers the count, the partial chunk runs on the scalar units beside
    // the whole chunks rather than after them.
    #[cfg(blake3_neon)]
    let hybrids = matches!(platform, Platform::NEON);
    #[cfg(blake3_sme2)]
    let hybrids = hybrids || matches!(platform, Platform::SME2);
    #[cfg(blake3_neon)]
    if !chunks_exact.remainder().is_empty()
        && hybrids
        && neon_hybrid::partial_pays(chunks_array.len(), chunks_exact.remainder().len())
        && neon_hybrid::sha3_detected()
    {
        // Sound: `partial_pays` verified the chunk count and `sha3_detected()` verified SHA-3 support; `out` holds enough room for `chunks_array.len() + 1` CVs.
        unsafe {
            neon_hybrid::hash_chunks_with_partial(&chunks_array, chunks_exact.remainder(), key, chunk_counter, flags, CHUNK_START, CHUNK_END, out);
        }
        return chunks_array.len() + 1;
    }
    hash_many_rounded(platform, &chunks_array, key, chunk_counter, IncrementCounter::Yes, flags, CHUNK_START, CHUNK_END, out);

    // Hash the remaining partial chunk, if there is one. Note that the empty
    // chunk (meaning the empty message) is a different codepath.
    let chunks_so_far = chunks_array.len();
    if !chunks_exact.remainder().is_empty() {
        let counter = chunk_counter + chunks_so_far as u64;
        let mut chunk_state = ChunkState::new(key, counter, flags, platform);
        chunk_state.update(chunks_exact.remainder());
        *<&mut [u8; OUT_LEN]>::try_from(&mut out[chunks_so_far * OUT_LEN..][..OUT_LEN]).unwrap() = chunk_state.output().chaining_value();
        chunks_so_far + 1
    } else {
        chunks_so_far
    }
}

// Use SIMD parallelism to hash up to MAX_SIMD_DEGREE parents at the same time
// on a single thread. Write out the parent chaining values and return the
// number of parents hashed. (If there's an odd input chaining value left over,
// return it as an additional output.) These parents are never the root and
// never empty; those cases use a different codepath.
#[doc(hidden)]
pub(crate) fn compress_parents_parallel(child_chaining_values: &[u8], key: &CVWords, flags: u8, platform: Platform, out: &mut [u8]) -> usize {
    debug_assert_eq!(child_chaining_values.len() % OUT_LEN, 0, "wacky hash bytes");
    let num_children = child_chaining_values.len() / OUT_LEN;
    debug_assert!(num_children >= 2, "not enough children");
    debug_assert!(num_children <= 2 * MAX_SIMD_DEGREE, "too many");

    let mut parents_exact = child_chaining_values.chunks_exact(BLOCK_LEN);
    let mut parents_array = StackVec::<&[u8; BLOCK_LEN], MAX_SIMD_DEGREE>::new();
    for parent in &mut parents_exact {
        parents_array.push(parent.try_into().unwrap());
    }
    let flags = flags | PARENT;
    #[cfg(blake3_sme2)]
    let on_sme2 = matches!(platform, Platform::SME2);
    #[cfg(not(blake3_sme2))]
    let on_sme2 = false;
    if on_sme2 {
        // SME2's parent kernel reads a group of sixteen blocks through
        // one pointer: the parents go to it as the run of memory they
        // are, not as the table of borrows made above (which the other
        // platforms take, and of which only the count is used here).
        #[cfg(blake3_sme2)]
        // Sound: the platform is SME2 only where detection found SME2
        // with 512-bit streaming vectors.
        unsafe {
            sme2::hash_block_run(child_chaining_values.as_chunks::<BLOCK_LEN>().0.as_flattened(), key, flags, out)
        };
    } else {
        hash_many_rounded(
            platform,
            &parents_array,
            key,
            0, // Parents always use counter 0.
            IncrementCounter::No,
            flags,
            0, // Parents have no start flags.
            0, // Parents have no end flags.
            out,
        );
    }

    // If there's an odd child left over, it becomes an output.
    let parents_so_far = parents_array.len();
    if !parents_exact.remainder().is_empty() {
        out[parents_so_far * OUT_LEN..][..OUT_LEN].copy_from_slice(parents_exact.remainder());
        parents_so_far + 1
    } else {
        parents_so_far
    }
}

// Hash a subtree with the walk (src/tree.rs, `children`), condensing its
// chaining values down to a single parent node. Don't compress that
// last parent node, however. Instead, return its message bytes (the
// concatenated chaining values of its children). This is necessary when the
// first call to update() supplies a complete subtree, because the topmost
// parent node of that subtree could end up being the root. It's also necessary
// for extended output in the general case.
//
// This function is not used on inputs of 1 chunk or less. That's a
// different codepath. Its arrays live in tree::root_children, out of line.
#[inline]
#[doc(hidden)]
pub(crate) fn compress_subtree_to_parent_node(input: &[u8], ahead: usize, key: &CVWords, chunk_counter: u64, flags: u8, platform: Platform) -> [u8; BLOCK_LEN] {
    debug_assert!(input.len() > CHUNK_LEN);
    let kernels = tree::PlatformKernels { key, flags, platform };
    let [left, right] = tree::root_children(&kernels, input, ahead, chunk_counter);
    let mut block = [0u8; BLOCK_LEN];
    block[..OUT_LEN].copy_from_slice(&left);
    block[OUT_LEN..].copy_from_slice(&right);
    block
}

// The root hash of an input of one chunk or less, in one kernel call. The
// whole blocks are read in place, the final one too when it is full; a
// short final block (and the empty block for the empty input) is copied
// into a zero-padded buffer, which is the copy ChunkState::update() makes
// too. The scalar kernel runs on every AArch64 core, so this path needs no
// platform check.
#[cfg(blake3_neon)]
#[inline]
fn hash_one_chunk_root(input: &[u8], key: &CVWords, flags: u8) -> Hash {
    debug_assert!(input.len() <= CHUNK_LEN);
    let blocks = cmp::max(1, input.len().div_ceil(BLOCK_LEN));
    let whole = (blocks - 1) * BLOCK_LEN;
    let tail = &input[whole..];
    /// The padded last block, 64-byte aligned as hasher::Output's is
    /// (src/hasher.rs explains).
    #[repr(align(64))]
    struct Aligned64([u8; BLOCK_LEN]);
    let padded;
    let last: &[u8; BLOCK_LEN] = match tail.try_into() {
        Ok(full) => full,
        Err(_) => {
            let mut block = Aligned64([0; BLOCK_LEN]);
            block.0[..tail.len()].copy_from_slice(tail);
            padded = block;
            &padded.0
        }
    };
    let mut out = [0u8; OUT_LEN];
    // Sound: `input` holds `(blocks - 1) * BLOCK_LEN` readable bytes before `tail`, `blocks` is in 1..=16, and `last` is 64 readable bytes.
    unsafe {
        neon_hybrid::hash_chunk(input.as_ptr(), blocks, last, tail.len().to_le_bytes()[0], key, 0, flags, CHUNK_START, CHUNK_END | ROOT, &mut out);
    }
    Hash(out)
}

/// Run the startup self-test and detect the CPU: every program calls this
/// once before hashing (any call that loads the detected platform before
/// `initialize` completes panics). Under 0.2 ms, once per process
/// (see the crate's documentation); later calls return at once.
///
/// ```
/// blake3_servil_core::initialize();
/// let hash = blake3_servil_core::convenience::hash(b"foo");
/// # let mut hasher = blake3_servil_core::convenience::Hasher::new(blake3_servil_core::Mode::Plain, None);
/// # hasher.update(b"foo");
/// # assert_eq!(hash, hasher.finalize());
/// ```
pub fn initialize() {
    self_test::ensure();
}

/// Scheduling and storage setup shared with `blake3-servil`'s workers.
/// Hashing goes through the checked calls on `throughput::TreeHasher`.
#[doc(hidden)]
#[cfg(feature = "std")]
pub mod thread_support {
    pub use crate::each::prepare_thread as prepare_collection_thread;
    pub use crate::many::{assert_slots, slot_len};
    pub use crate::platform::{MAX_SIMD_DEGREE, Platform, Sme2Turn};
    #[cfg(blake3_sme2)]
    pub use crate::sme2::prepare_thread_scratch as prepare_sme2_scratch;
}

/// An alias to distinguish [`context_key`](throughput::context_key) outputs from other keys.
/// It is a plain byte array, and so are its traits: its `Debug` prints the
/// key, and its `==` promises no constant time.
pub type ContextKey = [u8; KEY_LEN];

/// Prepared BLAKE3 mode (immutable key words + domain flags), private so
/// nothing outside `domain` can mutate them independently or pair non-IV
/// words with plain-mode flags.
pub(crate) mod domain {
    use super::{CVWords, DERIVE_KEY_MATERIAL, IV, KEYED_HASH, Mode, platform, throughput};

    #[derive(Clone, Copy)]
    pub(crate) struct Domain {
        key: CVWords,
        flags: u8,
    }

    impl Domain {
        #[inline(always)]
        pub(crate) const fn plain() -> Self {
            Self { key: *IV, flags: 0 }
        }

        #[inline]
        pub(crate) fn from_mode(mode: Mode<'_>) -> Self {
            match mode {
                Mode::Plain => Self::plain(),
                Mode::Keyed(key) => Self { key: platform::words_from_le_bytes_32(key), flags: KEYED_HASH },
                Mode::DeriveKey(context) => Self { key: platform::words_from_le_bytes_32(&throughput::context_key(context)), flags: DERIVE_KEY_MATERIAL },
                Mode::DeriveKeyWith(context_key) => Self { key: platform::words_from_le_bytes_32(context_key), flags: DERIVE_KEY_MATERIAL },
            }
        }

        #[inline(always)]
        pub(crate) const fn key(&self) -> &CVWords {
            &self.key
        }

        #[inline(always)]
        pub(crate) const fn flags(&self) -> u8 {
            self.flags
        }

        #[cfg(any(feature = "std", blake3_sse41_ffi))]
        #[inline(always)]
        pub(crate) const fn is_plain(&self) -> bool {
            self.flags == 0
        }
    }
}

pub(crate) use domain::Domain;

/// Which of BLAKE3's three functions a call computes: the plain hash
/// ([`hash`](convenience::hash)), the keyed hash, or key derivation. Every call that takes a mode runs each one at the same
/// speed, but for `DeriveKey`, which first hashes its context string (once
/// per call; [`DeriveKeyWith`](Mode::DeriveKeyWith) takes it hashed
/// once for many calls).
///
/// # Trait implementations
///
/// - `Clone` preserves the function selected and any borrowed key or
///   context.
/// - `Copy` copies the selection without copying the bytes it borrows.
#[derive(Clone, Copy)]
pub enum Mode<'a> {
    /// The plain hash function, as [`hash`](convenience::hash).
    Plain,
    /// The keyed hash function with this key.
    Keyed(&'a [u8; KEY_LEN]),
    /// The key derivation function with this context string; the input
    /// is the key material.
    DeriveKey(&'a str),
    /// The key derivation function with its context string already hashed
    /// ([`context_key`](throughput::context_key)); the input is the key material.
    DeriveKeyWith(&'a ContextKey),
}

impl Mode<'_> {
    /// The key words and flags this mode hashes with, the plain mode's IV
    /// where it is: a public call's one way to its key. A copy of the IV
    /// put 32 bytes of stores on the stack before the message's loads of
    /// every call (October 24, the i7's disassembly of a caller); a
    /// constant mode folds to its constants, inlined.
    #[inline]
    pub(crate) fn key(self, room: &mut core::mem::MaybeUninit<CVWords>) -> (&CVWords, u8) {
        match self {
            Mode::Plain => (IV, 0),
            _ => {
                let (key, flags) = self.key_and_flags();
                (room.write(key), flags)
            }
        }
    }

    /// The key words and flags this mode hashes with.
    #[inline]
    pub(crate) fn key_and_flags(self) -> (CVWords, u8) {
        let domain = Domain::from_mode(self);
        (*domain.key(), domain.flags())
    }
}

/// The whole input on the calling thread, in the given mode: the one-chunk
/// kernel call up to CHUNK_LEN, the subtree walk above. hash(),
/// hash() in its modes and hash_multithreaded()'s short path all come
/// through here.
#[inline]
pub(crate) fn hash_serial(input: &[u8], key: &CVWords, flags: u8) -> Hash {
    hash_serial_from(Platform::detect(), input, key, flags, false)
}

/// [`hash_serial`] for a call now and then (`convenience`'s): its kernels'
/// code prefetched first, where that helps a call whose code has left the
/// caches (AGENTS, "Cold calls and warm calls, by their API").
#[inline]
fn hash_serial_cold(input: &[u8], key: &CVWords, flags: u8) -> Hash {
    hash_serial_from(Platform::detect(), input, key, flags, true)
}

/// [`hash_serial`] from `platform`, and `cold` for a call now and then:
/// the startup self-test's way in, which has not run `Platform::detect`'s
/// check (it is that check). Always inlined, so each entry point has its
/// own copy with `cold` a constant, as hash_serial alone did before cold
/// calls had their path (one out-of-line copy taking `cold` as an argument
/// made convenience::hash of 512 B after other work 11.5% slower, the VM's
/// check).
#[inline(always)]
fn hash_serial_from(platform: Platform, input: &[u8], key: &CVWords, flags: u8, cold: bool) -> Hash {
    let turn = platform::Sme2Turn::take(platform, input.len() >= SME2_SIZED_LEN);
    // One comparison on the path of a chunk or less: everything else is
    // out of line, so that path's code stays as small as it was (a call
    // after other work fetches every line of it from DRAM).
    #[cfg(all(blake3_neon, feature = "std"))]
    if cold && (CHUNK_LEN + 1..PREFETCH_BELOW).contains(&input.len()) {
        prefetch_cold(input.len(), turn.platform(), false);
    }
    #[cfg(not(all(blake3_neon, feature = "std")))]
    let _ = cold;
    hash_serial_on(input, key, flags, turn.platform())
}

/// Below this length a call now and then (`convenience`'s, and a fresh
/// Hasher's first update) prefetches its kernels' code: after other work
/// the call then fetches it from DRAM in parallel, not one miss at a time
/// (M4 Max, after other work: 4 KiB x0.61, 8 KiB x0.51, 16 KiB x0.66;
/// 32 KiB and up level, the hashing's own time dwarfing it; jobs
/// 971-975). It costs a call whose code is near 2-4%, which is why the
/// throughput calls, for call after call, take no prefetch.
/// A cold call of 2-8 KiB spends about a third of its time on its code
/// (10-24 KB of kernels and glue); from 3 KiB what remains is the kernels
/// starting before their lines arrive. Tried and set aside: prefetching
/// for a chunk or less (2-10% gained, less than the pause check costs);
/// prefetching the 16 KiB of Rust around the kernels too (every call
/// 400-500 ns slower: prefetches beyond some number in flight are
/// dropped); a rolled four-lane NEON path for calls after a pause (1.3 KB,
/// 1.2-1.5x slower: a four-lane NEON BLAKE3 is bound by its chains'
/// latency, where the hybrids' integer lanes fill the idle issue slots).
#[cfg(all(blake3_neon, feature = "std"))]
const PREFETCH_BELOW: usize = 64 * 1024;

/// Prefetch the kernels hashing `len` bytes runs, for a call now and then:
/// a one-shot call's (prefetch_kernels), or with `fresh_hasher` a fresh
/// Hasher's first update's (prefetch_update_kernels).
#[cfg(all(blake3_neon, feature = "std"))]
// Cold for layout: the path of a chunk or less falls through past its
// call, on the lines it had before the call existed.
#[cold]
#[inline(never)]
fn prefetch_cold(len: usize, platform: Platform, fresh_hasher: bool) {
    if fresh_hasher {
        prefetch_update_kernels(len, platform);
    } else {
        prefetch_kernels(len, platform);
    }
}

/// Prefetch the code of the kernels hashing `len` bytes on `platform`
/// runs: SME2's from 16 chunks, else the hybrids', chunks to root.
#[cfg(all(blake3_neon, feature = "std"))]
fn prefetch_kernels(len: usize, platform: Platform) {
    #[cfg(blake3_sme2)]
    if matches!(platform, Platform::SME2) && len >= SME2_SIZED_LEN {
        sme2::prefetch_code();
        // Below the flat walk, sixteen chunks' values merge on the hybrids.
        if !sme2::flat_takes(len) {
            neon_hybrid::prefetch_parent_code(16);
        }
        return;
    }
    // Below 16 chunks the SME2 platform runs the hybrids too.
    #[cfg(blake3_sme2)]
    let hybrids = matches!(platform, Platform::NEON | Platform::SME2);
    #[cfg(not(blake3_sme2))]
    let hybrids = matches!(platform, Platform::NEON);
    if hybrids && neon_hybrid::sha3_detected() {
        let total_chunks = len.div_ceil(CHUNK_LEN);
        let whole = (len / CHUNK_LEN).min(16);
        let partial = if whole == 16 { 0 } else { len % CHUNK_LEN };
        neon_hybrid::prefetch_tree_code(whole, partial, total_chunks);
    }
}

/// Prefetch the code a fresh Hasher's update of `len` bytes runs: the
/// subtrees its loop cuts (hasher::subtree_len from counter 0, up to the last
/// chunk, which waits in the chunk state), each one's kernels.
#[cfg(all(blake3_neon, feature = "std"))]
fn prefetch_update_kernels(len: usize, platform: Platform) {
    if !neon_hybrid::sha3_detected() {
        return;
    }
    let (mut counter, mut left) = (0u64, len);
    while left > CHUNK_LEN {
        let subtree = hasher::subtree_len(counter, left);
        let chunks = subtree / CHUNK_LEN;
        #[cfg(blake3_sme2)]
        if matches!(platform, Platform::SME2) && chunks >= 16 {
            sme2::prefetch_code();
            if !sme2::flat_takes(subtree) {
                neon_hybrid::prefetch_parent_code(16);
            }
            counter += chunks as u64;
            left -= subtree;
            continue;
        }
        neon_hybrid::prefetch_tree_code(chunks.min(16), 0, chunks);
        counter += chunks as u64;
        left -= subtree;
    }
    let _ = platform;
}

/// The smallest input the SME2 kernels take part in: one group of sixteen
/// chunks. Below it the SME2 platform runs the NEON hybrids.
pub(crate) const SME2_SIZED_LEN: usize = 16 * CHUNK_LEN;

/// The fewest messages a batch has when the SME2 kernels take part: one
/// group of sixteen messages.
pub(crate) const SME2_SIZED_BATCH: usize = 16;

/// [`hash_serial`] with the tree's kernels chosen by the caller (a pool
/// piece runs the NEON hybrids); one chunk or less runs the scalar kernel
/// on every platform.
#[inline]
#[doc(hidden)]
pub(crate) fn hash_serial_on(input: &[u8], key: &CVWords, flags: u8, platform: Platform) -> Hash {
    Hash(tree::hash(&tree::PlatformKernels { key, flags, platform }, input))
}

/// [`hash_many`] in the mode of `key` and `flags`, on the calling thread.
#[inline]
#[doc(hidden)]
pub(crate) fn hash_many_serial(input: &[u8], message_len: usize, slot: usize, key: &CVWords, flags: u8, out: &mut [[u8; OUT_LEN]]) {
    hash_many_serial_from(Platform::detect(), input, message_len, slot, key, flags, out);
}

/// [`hash_many_serial`] from `platform` (the self-test's way in, as
/// [`hash_serial_from`]).
#[inline]
fn hash_many_serial_from(platform: Platform, input: &[u8], message_len: usize, slot: usize, key: &CVWords, flags: u8, out: &mut [[u8; OUT_LEN]]) {
    let turn = platform::Sme2Turn::take(platform, many::sme2_sized(message_len, out.len()));
    many::hash_many_on(input, message_len, slot, key, flags, out, turn.platform());
}

mod hasher;
use hasher::CvStack;

mod incremental;
pub(crate) use incremental::*;

/// An incremental reader for extended output, returned by
/// [`Hasher::finalize_xof`](convenience::Hasher::finalize_xof).
///
/// Shorter BLAKE3 outputs are prefixes of longer ones, and explicitly requesting a short output is
/// equivalent to truncating the default-length output. Note that this is a difference between
/// BLAKE2 and BLAKE3.
///
/// # Security notes
///
/// Outputs shorter than the default length of 32 bytes (256 bits) provide less security. An N-bit
/// BLAKE3 output is intended to provide N bits of first and second preimage resistance and N/2
/// bits of collision resistance, for any N up to 256. Longer outputs don't provide any additional
/// security.
///
/// Avoid relying on the secrecy of the output offset, that is, the number of output bytes read or
/// the arguments to [`seek`](struct.OutputReader.html#method.seek) or
/// [`set_position`](struct.OutputReader.html#method.set_position). [_Block-Cipher-Based Tree
/// Hashing_ by Aldo Gunsing](https://eprint.iacr.org/2022/283) shows that an attacker who knows
/// both the message and the key (if any) can easily determine the offset of an extended output.
/// For comparison, AES-CTR has a similar property: if you know the key, you can decrypt a block
/// from an unknown position in the output stream to recover its block index. Callers with strong
/// secret keys aren't affected in practice, but secret offsets are a [design
/// smell](https://en.wikipedia.org/wiki/Design_smell) in any case.
///
/// # Trait implementations
///
/// - `Clone` reads the same output from the same position, with an
///   independent cursor.
/// - `Debug` reports the position without disclosing the output or key.
/// - `Read` (with `std`) advances by the bytes returned, stopping at
///   position `u64::MAX`.
/// - `Seek` (with `std`) moves within the same output, clamping past its
///   end and refusing negative positions and seeks relative to the end.
#[derive(Clone)]
pub struct OutputReader {
    inner: Output,
    position_within_block: u8,
    cached_block: [u8; BLOCK_LEN],
    cache_valid: bool,
}

impl OutputReader {
    fn new(inner: Output) -> Self {
        Self { inner, position_within_block: 0, cached_block: [0; BLOCK_LEN], cache_valid: false }
    }

    // This helper function handles both the case where the output buffer is
    // shorter than one block, and the case where our position_within_block is
    // non-zero.
    fn fill_one_block(&mut self, buf: &mut &mut [u8]) {
        if !self.cache_valid {
            self.cached_block = self.inner.root_output_block();
            self.cache_valid = true;
        }
        let output_bytes = &self.cached_block[self.position_within_block as usize..];
        let take = cmp::min(buf.len(), output_bytes.len());
        buf[..take].copy_from_slice(&output_bytes[..take]);
        self.position_within_block += u8::try_from(take).expect("take <= BLOCK_LEN");
        if usize::from(self.position_within_block) == BLOCK_LEN {
            self.inner.counter += 1;
            self.position_within_block = 0;
            self.cache_valid = false;
        }
        // Advance the dest buffer. mem::take() is a borrowck workaround.
        *buf = &mut core::mem::take(buf)[take..];
    }

    /// Fill a buffer with output bytes and advance the position of the
    /// `OutputReader`. It always fills the entire buffer. [`Read::read`]
    /// does the same up to the output's end, and differs there: it reads
    /// the bytes that are left and after them 0 bytes, where this call
    /// panics (below).
    ///
    /// A partial output block is retained for subsequent short reads, so
    /// sequential reads don't recompute that block. Whole blocks continue
    /// through the vectorized output path. For streaming throughput, prefer
    /// a slice length that's a multiple of [`BLOCK_LEN`] (64 bytes).
    ///
    /// The maximum output size of BLAKE3 is 2<sup>64</sup>-1 bytes.
    ///
    /// # Panics
    ///
    /// If `buf` asks for bytes past that end (after seeking near it), in
    /// every build: the position cannot count them. [`Read::read`] reads
    /// up to the end instead, and then gives 0 bytes.
    ///
    /// [`Read::read`]: #method.read
    pub fn fill(&mut self, mut buf: &mut [u8]) {
        if buf.is_empty() {
            return;
        }
        // Past the end the block counter would overflow: the position
        // wrapped to 0 in a release build, and a seek relative to it then
        // gave the output's first bytes again (BLAKE3 #423).
        assert!(buf.len() as u64 <= self.remaining(), "BLAKE3's output ends at 2^64 - 1 bytes");

        // If we're partway through a block, try to get to a block boundary.
        if self.position_within_block != 0 {
            self.fill_one_block(&mut buf);
            if buf.is_empty() {
                return;
            }
        }

        // The whole blocks at once (xof_many takes none as none), then the
        // tail block alone. Each guard here spares a root block computed for
        // nothing, which test_fill_computes_each_block_once counts.
        let full_blocks = buf.len() / BLOCK_LEN;
        let full_blocks_len = full_blocks * BLOCK_LEN;
        debug_assert!(full_blocks == 0 || self.position_within_block == 0);
        self.inner.platform.xof_many(
            &self.inner.input_chaining_value,
            &self.inner.block,
            self.inner.block_len,
            self.inner.counter,
            self.inner.flags | ROOT,
            &mut buf[..full_blocks_len],
        );
        self.inner.counter += full_blocks as u64;
        buf = &mut buf[full_blocks_len..];

        if !buf.is_empty() {
            debug_assert!(buf.len() < BLOCK_LEN);
            self.fill_one_block(&mut buf);
            debug_assert!(buf.is_empty());
        }
    }

    /// Return the current read position in the output stream. This is
    /// equivalent to [`Seek::stream_position`], except that it doesn't return
    /// a `Result`. The position of a new `OutputReader` starts at 0, and each
    /// call to [`fill`] or [`Read::read`] moves the position forward by the
    /// number of bytes read.
    ///
    /// [`Seek::stream_position`]: #method.stream_position
    /// [`fill`]: #method.fill
    /// [`Read::read`]: #method.read
    pub fn position(&self) -> u64 {
        self.inner.counter * BLOCK_LEN as u64 + self.position_within_block as u64
    }

    /// The output bytes left from the position to the output's end.
    #[inline]
    fn remaining(&self) -> u64 {
        u64::MAX - self.position()
    }

    /// Seek to a new read position in the output stream. This is equivalent to
    /// calling [`Seek::seek`] with [`SeekFrom::Start`], except that it doesn't
    /// return a `Result`.
    ///
    /// [`Seek::seek`]: #method.seek
    /// [`SeekFrom::Start`]: https://doc.rust-lang.org/std/io/enum.SeekFrom.html
    pub fn set_position(&mut self, position: u64) {
        self.position_within_block = u8::try_from(position % BLOCK_LEN as u64).expect("remainder < 64");
        self.inner.counter = position / BLOCK_LEN as u64;
        self.cache_valid = false;
    }
}

// Don't derive(Debug), because the state may be secret.
impl fmt::Debug for OutputReader {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("OutputReader").field("position", &self.position()).finish()
    }
}

#[cfg(feature = "std")]
impl std::io::Read for OutputReader {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // Up to the output's end, where a read gives 0 bytes as a file's
        // does.
        let len = usize::try_from(cmp::min(buf.len() as u64, self.remaining())).expect("bounded by buf.len()");
        self.fill(&mut buf[..len]);
        Ok(len)
    }
}

#[cfg(feature = "std")]
impl std::io::Seek for OutputReader {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        let max_position = u64::MAX as i128;
        let target_position: i128 = match pos {
            std::io::SeekFrom::Start(x) => x as i128,
            std::io::SeekFrom::Current(x) => self.position() as i128 + x as i128,
            std::io::SeekFrom::End(_) => {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "seek from end not supported"));
            }
        };
        if target_position < 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "seek before start"));
        }
        self.set_position(u64::try_from(cmp::min(target_position, max_position)).expect("non-negative and <= u64::MAX"));
        Ok(self.position())
    }
}
