//! The throughput API, for power users: every way this crate makes
//! hashing fast (other cores, batches, collections, streams, a message fed
//! in pieces). The program takes on more of the work, so the calls can be
//! fast and simple:
//!
//! - **Inputs are [`Block`]s**: 64 bytes each on a 64-byte boundary, so
//!   every message starts a cache line and no block straddles two; the
//!   type holds it. A message is blocks and a length (`&[Block], len`),
//!   the bytes past its end ignored. A batch is a [`Batch`]: messages of
//!   one length, each in its own whole blocks, every byte past each one
//!   zero, which the batch kernels read in place, side by side; a `Batch`
//!   lets the program write only its messages' bytes, so those zeros
//!   cannot go wrong. Bytes already in place become blocks with no copy
//!   through [`Block::from_bytes`], and a batch through
//!   [`Batch::from_blocks`] where its messages fill their blocks.
//!   [`update_multithreaded`] and [`update_each`] alone take pieces of any
//!   length anywhere, into a [`Hasher`](crate::convenience::Hasher) (which
//!   copies short ones into its own buffer); an aligned long piece runs
//!   fastest.
//! - **Digests go where the program says**: into its slices and places.
//! - **Every call takes a [`Mode`]**: `Mode::Plain` for the plain hash.
//!
//! For one call per hash, any slice, the result returned, see the
//! [`convenience`](crate::convenience) API.

pub use crate::block::{Batch, Block};
#[cfg(feature = "std")]
pub use crate::each::{finalize_each, hash_each, hash_each_multithreaded, update_each, update_each_multithreaded};
#[cfg(feature = "std")]
pub use crate::stream::{Stream, stream};

/// The incremental background pipeline: messages fed piece by piece as
/// their bytes arrive, prepared batches, and completions delivered on the
/// producer's thread, with a fixed memory budget ([`pipeline::Limits`]).
/// Evolving: bench-hashes measures it against [`Stream`], which it is to
/// replace; its names may change before 1.0.
///
/// Its waits are futures, run by the caller's executor; a program with no
/// executor polls them on its own thread, parking between polls:
///
/// ```
/// use blake3_servil::throughput::pipeline::{Event, Limits, Stream};
/// use blake3_servil::{Hash, Mode};
/// use std::cell::RefCell;
/// use std::future::Future;
/// use std::task::{Context, Poll, Wake, Waker};
///
/// struct Unpark(std::thread::Thread);
/// impl Wake for Unpark {
///     fn wake(self: std::sync::Arc<Self>) {
///         self.0.unpark();
///     }
/// }
/// fn block_on<F: Future>(future: F) -> F::Output {
///     let waker = Waker::from(std::sync::Arc::new(Unpark(std::thread::current())));
///     let mut cx = Context::from_waker(&waker);
///     let mut future = std::pin::pin!(future);
///     loop {
///         match future.as_mut().poll(&mut cx) {
///             Poll::Ready(value) => return value,
///             Poll::Pending => std::thread::park(),
///         }
///     }
/// }
///
/// blake3_servil::initialize_multithreaded(None);
/// let messages: Vec<Vec<u8>> = (0..3).map(|i| vec![i as u8; 100_000 * (i + 1)]).collect();
/// let hashes: RefCell<Vec<Option<Hash>>> = RefCell::new(vec![None; messages.len()]);
/// let stream = Stream::new(Mode::Plain, Limits { buffers: 8, messages: 2 }, |event| {
///     if let Event::Hashed { tag, hash } = event {
///         hashes.borrow_mut()[tag] = Some(hash);
///     }
/// });
/// for (tag, bytes) in messages.iter().enumerate() {
///     // A message slot; while none is free, ended messages are delivered.
///     let mut message = block_on(stream.message(tag, Some(bytes.len() as u64)));
///     let mut rest = &bytes[..];
///     while !rest.is_empty() {
///         // Room for the next bytes, as they arrive (here, from a slice).
///         let mut input = block_on(message.input());
///         let n = input.bytes_mut().len().min(rest.len());
///         input.bytes_mut()[..n].copy_from_slice(&rest[..n]);
///         input.commit(n);
///         rest = &rest[n..];
///     }
///     message.end(0);
/// }
/// stream.finish();
/// for (bytes, hash) in messages.iter().zip(hashes.borrow().iter()) {
///     assert_eq!(hash.unwrap(), blake3_servil::convenience::hash(bytes));
/// }
/// ```
#[cfg(feature = "std")]
pub mod pipeline {
    pub use crate::stream::frontend::{Event, Input, Limits, Message, Stream};
}

use crate::{ContextKey, DERIVE_KEY_CONTEXT, Hash, IV, Mode, OUT_LEN};

/// [`Hasher::update`](crate::convenience::Hasher::update), using other CPU
/// cores where that is faster: the same result, never slower. Pieces of
/// 512 KiB and more use them, as [`hash_multithreaded`] does; shorter ones
/// hash on the calling thread. One `Hasher` may take both calls. When
/// reading the pieces keeps your thread busy, a [`stream`] is faster
/// ([`Stream::submit_reader`], or [`Stream::submit_with`]): it hashes each
/// piece while your thread reads the next. The program calls
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
#[cfg(feature = "std")]
pub fn update_multithreaded<'h>(hasher: &'h mut crate::convenience::Hasher, input: &[u8]) -> &'h mut crate::convenience::Hasher {
    hasher.update_pooled(input)
}

/// The hash in `mode` of the message of `len` bytes that starts `input`:
/// a batch of one. The message starts a block, so on a cache line; the
/// bytes past its end, in its last block and any after, are ignored.
/// Tuned for call after call: for a hash now and then amid other work,
/// [`convenience::hash`](crate::convenience::hash) is the faster call.
///
/// ```
/// # blake3_servil::initialize();
/// # #[cfg(feature = "std")] {
/// use blake3_servil::{Mode, convenience, throughput::{self, Batch}};
/// let mut message = Batch::new(100, 1);
/// message.set(0, &[7u8; 100]);
/// assert_eq!(throughput::hash(Mode::Plain, message.blocks(), 100), convenience::hash(&[7u8; 100]));
/// # }
/// ```
// Inlined, so a constant mode folds to its constants (as
// convenience::hash is: no mode stored for the call to read).
#[inline]
pub fn hash(mode: Mode, input: &[Block], len: usize) -> Hash {
    let mut room = core::mem::MaybeUninit::uninit();
    let (key, flags) = mode.key(&mut room);
    crate::hash_serial(&Block::bytes(input)[..len], key, flags)
}

/// The hashes of a [`Batch`]'s messages, in `mode`, on the calling thread:
/// `out[i]` becomes the [`hash`] of message `i`, one digest a message. This
/// is the shape of a Merkle tree's layer: leaves of one size, or nodes of
/// two 32-byte children, which the layer below writes straight into the
/// next layer's batch ([`Batch::digests_mut`]).
///
/// ```
/// # blake3_servil::initialize();
/// # #[cfg(feature = "std")] {
/// use blake3_servil::{Mode, convenience, throughput::{self, Batch}};
/// let mut leaves = Batch::new(256, 1000);
/// for i in 0..1000 {
///     leaves.set(i, &[7u8; 256]);
/// }
/// let mut hashes = vec![[0u8; 32]; 1000];
/// throughput::hash_many(Mode::Plain, &leaves, &mut hashes);
/// assert_eq!(hashes[3], *convenience::hash(&[7u8; 256]).as_bytes());
/// # }
/// ```
///
/// **Speed**: short messages hash several at a time, so a batch beats a
/// loop of [`hash`]: on an Apple M4 Max, 64-byte messages take about 10 to
/// 20 ns each in batches of 16 or more, against about 50 for one [`hash`]
/// each. Messages longer than 15 KiB cost what [`hash`] costs. For the
/// same hashes over several cores, see [`hash_many_multithreaded`].
// Inlined, so a constant mode folds to its constants (as
// convenience::hash is: no mode stored for the call to read).
#[inline(always)]
pub fn hash_many(mode: Mode, batch: &Batch, out: &mut [[u8; OUT_LEN]]) {
    let mut room = core::mem::MaybeUninit::uninit();
    let (key, flags) = mode.key(&mut room);
    crate::hash_many_serial(Block::bytes(batch.blocks()), batch.message_len(), key, flags, out);
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
#[cfg(feature = "std")]
// Inlined, so a constant mode folds to its constants (as
// convenience::hash is: no mode stored for the call to read).
#[inline(always)]
pub fn hash_many_multithreaded(mode: Mode, batch: &Batch, out: &mut [[u8; OUT_LEN]]) {
    crate::lanes::require();
    let mut room = core::mem::MaybeUninit::uninit();
    let (key, flags) = mode.key(&mut room);
    // Its length against the digests here, on the caller's thread, as
    // hash_many's kernels check it: a multiplication.
    crate::many::assert_slots(Block::bytes(batch.blocks()), batch.message_len(), out.len());
    crate::lanes::hash_many(Block::bytes(batch.blocks()), batch.message_len(), key, flags, out);
}

/// [`hash`], using other CPU cores where that is faster: the same
/// [`Hash`](struct@Hash), never slower. For a message whose bytes are not
/// in blocks (a mapped file, say), [`update_multithreaded`] into a
/// [`Hasher`](crate::convenience::Hasher) takes any slice, at the same
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
#[cfg(feature = "std")]
// Inlined, so a constant mode folds to its constants (as
// convenience::hash is: no mode stored for the call to read).
#[inline]
pub fn hash_multithreaded(mode: Mode, input: &[Block], len: usize) -> Hash {
    crate::lanes::require();
    let mut room = core::mem::MaybeUninit::uninit();
    let (key, flags) = mode.key(&mut room);
    crate::lanes::hash_with_key(&Block::bytes(input)[..len], key, flags)
}

/// Hash a key-derivation context string and return a [`ContextKey`].
///
/// This has the same security requirement as key derivation. **The context
/// string should be hardcoded, globally unique, and application-specific.**
///
/// Its one use is [`Mode::DeriveKeyWith`]: a program that derives many
/// keys from one context hashes the context once.
///
/// # Example
///
/// ```
/// # blake3_servil::initialize();
/// use blake3_servil::Mode;
///
/// let context_key = blake3_servil::throughput::context_key("foo");
/// let derived = blake3_servil::convenience::hash_with(Mode::DeriveKeyWith(&context_key), b"bar");
/// assert_eq!(derived, blake3_servil::convenience::hash_with(Mode::DeriveKey("foo"), b"bar"));
/// ```
pub fn context_key(context: &str) -> ContextKey {
    crate::hash_serial(context.as_bytes(), IV, DERIVE_KEY_CONTEXT).0
}
