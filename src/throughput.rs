//! The throughput API, for power users: every way this crate makes
//! hashing fast (batches, collections, many hashers fed in turns). The program takes on more of the work, so the calls can be
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
//!   [`update_each`] alone takes pieces of any
//!   length anywhere, into a [`Hasher`](crate::convenience::Hasher) (which
//!   copies short ones into its own buffer); an aligned long piece runs
//!   fastest.
//! - **Digests go where the program says**: into its slices and places.
//! - **Every call takes a [`Mode`]**: `Mode::Plain` for the plain hash.
//!
//! For one call per hash, any slice, the result returned, see the
//! [`convenience`](crate::convenience) API.

#[cfg(feature = "std")]
pub use crate::block::Packed;
pub use crate::block::{Batch, Block, Part, Prepared};
#[cfg(feature = "std")]
pub use crate::each::{finalize_each, hash_each, update_each, update_each_with};

use crate::{CHUNK_LEN, ContextKey, DERIVE_KEY_CONTEXT, Hash, IV, Mode, OUT_LEN};

/// The hash in `mode` of the message of `len` bytes that starts `input`:
/// a batch of one. The message starts a block, so on a cache line; the
/// bytes past its end, in its last block and any after, are ignored.
/// Tuned for call after call: for a hash now and then amid other work,
/// [`convenience::hash`](crate::convenience::hash) is the faster call.
///
/// ```
/// # blake3_servil_core::initialize();
/// # #[cfg(feature = "std")] {
/// use blake3_servil_core::{Mode, convenience, throughput::{self, Batch}};
/// let mut message = Batch::new(100, 1);
/// message.set(0, &[7u8; 100]);
/// assert_eq!(throughput::hash(Mode::Plain, message.blocks(), 100), convenience::hash(&[7u8; 100]));
/// # }
/// ```
// Inlined, so a constant mode folds to its constants (as
// convenience::hash is: no mode stored for the call to read). Always:
// as the short path of the threads crate's hash_multithreaded it was
// left a call (64 B there 5 to 7% slower, a Ryzen 9700X, instructions
// level within the half percent the check reports).
#[inline(always)]
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
/// Such a tree is one of your own design, not BLAKE3's own tree: each
/// node here is the hash of a whole 64-byte message, where BLAKE3's
/// parents are [`TreeHasher::parent_cvs`]'. In your design, hash the
/// leaves and the nodes apart, with two modes (two
/// [`Mode::DeriveKey`] contexts, say) or leaves that are never 64 bytes.
/// Hashed alike, a 64-byte leaf made of two digests passes for a node,
/// and a proof for one tree's leaf proves another tree's node.
///
/// ```
/// # blake3_servil_core::initialize();
/// # #[cfg(feature = "std")] {
/// use blake3_servil_core::{Mode, convenience, throughput::{self, Batch}};
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
/// same hashes over several cores, the crate `blake3-servil` has
/// `hash_many_multithreaded`.
///
/// A [`Packed`] of 16- or 32-byte messages is a batch too, as the program
/// holds them, with no copy into blocks:
///
/// ```
/// # blake3_servil_core::initialize();
/// # #[cfg(feature = "std")] {
/// use blake3_servil_core::{Mode, convenience, throughput::{self, Packed}};
/// let mut leaves = Packed::<32>::new(1000);
/// for (i, leaf) in leaves.iter_mut().enumerate() {
///     leaf[0] = i as u8;
/// }
/// let mut digests = Packed::<32>::new(1000); // the next layer's input
/// throughput::hash_many(Mode::Plain, &leaves, &mut digests);
/// assert_eq!(digests[3], *convenience::hash(&leaves[3]).as_bytes());
/// # }
/// ```
// Inlined, so a constant mode folds to its constants (as
// convenience::hash is: no mode stored for the call to read).
#[inline(always)]
pub fn hash_many<P: Prepared>(mode: Mode, batch: &P, out: &mut [[u8; OUT_LEN]]) {
    let mut room = core::mem::MaybeUninit::uninit();
    let (key, flags) = mode.key(&mut room);
    crate::hash_many_serial(batch.bytes(), batch.message_len(), batch.slot(), key, flags, out);
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
/// # blake3_servil_core::initialize();
/// use blake3_servil_core::Mode;
///
/// let context_key = blake3_servil_core::throughput::context_key("foo");
/// let derived = blake3_servil_core::convenience::hash_with(Mode::DeriveKeyWith(&context_key), b"bar");
/// assert_eq!(derived, blake3_servil_core::convenience::hash_with(Mode::DeriveKey("foo"), b"bar"));
/// ```
pub fn context_key(context: &str) -> ContextKey {
    crate::hash_serial(context.as_bytes(), IV, DERIVE_KEY_CONTEXT).0
}

const CHUNK: u64 = CHUNK_LEN as u64;

/// The highest level of a whole subtree: 2^53 chunks, 2^63 bytes, the left
/// half of the longest message.
const MAX_LEVEL: u32 = 53;

// A whole subtree's place in one word: its level plus one above its index.
// The word of zeroed memory is then no place, so a slot that no call
// filled is refused, whether a program wrote `Cv::unset()` or the
// allocator's zeros are there.
const LEVEL_SHIFT: u32 = 56;
const INDEX_MASK: u64 = (1 << LEVEL_SHIFT) - 1;
const UNSET: u64 = 0;

/// The level of whole subtrees of `len` bytes: `len` is `1024 << level`.
///
/// Requires a power of two of chunks. In a `const`, another size stops the
/// build; elsewhere it panics.
///
/// ```compile_fail,E0080
/// use blake3_servil_core::throughput::level_of;
/// const PART: u64 = 5 << 20; // 5 MiB is no subtree's size
/// const LEVEL: u32 = level_of(PART);
/// let _ = LEVEL;
/// ```
pub const fn level_of(len: u64) -> u32 {
    assert!(len >= CHUNK && len % CHUNK == 0 && (len / CHUNK).is_power_of_two(), "a whole subtree holds a power of two of chunks");
    (len / CHUNK).trailing_zeros()
}

/// A whole subtree's place, packed.
const fn place(level: u32, index: u64) -> u64 {
    assert!(level <= MAX_LEVEL, "a whole subtree's level is at most 53");
    // It ends at byte (index + 1) << (level + 10), within a message's 2^64 - 1.
    assert!(index < u64::MAX >> (level + 10), "the subtree ends beyond a message's last byte");
    // The index is under 2^54, below the level's bits: the sum joins them.
    ((level as u64 + 1) << LEVEL_SHIFT) + index
}

/// The place and size of the bytes a whole-subtree call was given.
#[inline]
fn subtree_place(level: u32, index: u64, len: usize) -> u64 {
    let at = place(level, index);
    assert!(len as u64 == CHUNK << level, "a whole subtree at level {level} holds {} bytes (found {len})", CHUNK << level);
    at
}

/// The two halves of a valid whole subtree above level zero.
const fn half_places(level: u32, index: u64) -> [u64; 2] {
    let left = ((level as u64) << LEVEL_SHIFT) + 2 * index;
    [left, left + 1]
}

/// The place of the parent of the whole subtree at `place` (an even index).
const fn parent_place(place: u64) -> u64 {
    (place & !INDEX_MASK) + (1 << LEVEL_SHIFT) + ((place & INDEX_MASK) >> 1)
}

/// The most bytes the last subtree that begins at `first_chunk` can hold:
/// the whole subtree aligned there, as many chunks as the largest power of
/// two that divides `first_chunk` (at chunk 0, a whole message), and no
/// more than a message has left.
const fn last_max(first_chunk: u64) -> u64 {
    assert!(first_chunk <= u64::MAX / CHUNK, "the subtree starts beyond a message's last byte");
    let room = u64::MAX - first_chunk * CHUNK;
    if first_chunk == 0 {
        return room;
    }
    // first_chunk is under 2^54, so its whole subtree is 2^63 bytes or less.
    let whole = CHUNK << first_chunk.trailing_zeros();
    if whole < room { whole } else { room }
}

/// The chaining value of a **whole** subtree, with its place: 2^level
/// chunks (1024 bytes each), the `index`-th subtree of that size in the
/// message. It is the only kind a left child can be.
///
/// A chaining value is the 32 bytes a BLAKE3 tree passes from a subtree up
/// to its parent. It is not a hash of anything a program can check alone,
/// and has its own type so that it is never taken for one: the root alone
/// gives a [`struct@Hash`] ([`TreeHasher::root_hash`]).
///
/// Equality compares the place and, in constant time, the bytes. `Debug`
/// shows the place alone, so that a refusal's message carries no chaining
/// value of a keyed tree.
///
/// # Trait implementations
///
/// - `Clone` reproduces both the value and its place, including an unset
///   slot.
/// - `Copy` copies the bytes and place together.
/// - `Debug` shows the place or the unset state without exposing chaining
///   bytes.
/// - `Eq` identifies values with equal bytes and place.
/// - `PartialEq` compares the place and compares the chaining bytes in
///   constant time.
#[derive(Clone, Copy)]
pub struct Cv {
    /// The chaining value as the kernels take it: 32 bytes.
    value: crate::CVBytes,
    place: u64,
}

impl Cv {
    /// A slot not yet filled, for an array that a call will fill: every
    /// call that takes a `Cv` as a subtree's value refuses it
    /// ([`as_bytes`](Cv::as_bytes) gives its 32 zero bytes). Zeroed memory
    /// is the same value.
    pub const fn unset() -> Self {
        Cv { value: [0; OUT_LEN], place: UNSET }
    }

    /// A chaining value a program stored or received, for the whole
    /// subtree at `level` and `index`. The place is the program's word:
    /// bytes of another subtree give a wrong hash, as wrong input does.
    ///
    /// Requires a level of at most 53 and a subtree that ends
    /// within a message's 2^64 - 1 bytes.
    pub const fn at(level: u32, index: u64, bytes: [u8; OUT_LEN]) -> Self {
        Cv { value: bytes, place: place(level, index) }
    }

    /// The chaining value's bytes.
    #[inline]
    pub const fn as_bytes(&self) -> &[u8; OUT_LEN] {
        &self.value
    }

    /// The subtree holds 2^level chunks.
    ///
    /// Requires a value that was set.
    #[inline]
    pub const fn level(&self) -> u32 {
        (self.set() >> LEVEL_SHIFT) as u32 - 1
    }

    /// The subtree is the `index`-th of its size.
    ///
    /// Requires a value that was set.
    #[inline]
    pub const fn index(&self) -> u64 {
        self.set() & INDEX_MASK
    }

    /// The two values returned to a Hasher by its pool. Check their
    /// places before the stack takes only the bytes.
    pub(crate) fn halves_bytes([left, right]: [Self; 2], level: u32, index: u64) -> [crate::CVBytes; 2] {
        let [a, b] = half_places(level, index);
        assert!(left.place == a && right.place == b, "not the halves of level {level} index {index}: {left:?} and {right:?}");
        [left.value, right.value]
    }

    /// The place, of a value that has one.
    #[inline]
    const fn set(&self) -> u64 {
        assert!(self.place != UNSET, "a chaining value that was never set");
        self.place
    }
}

impl core::fmt::Debug for Cv {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.place == UNSET { write!(f, "a value never set") } else { write!(f, "the whole subtree at level {} index {}", self.level(), self.index()) }
    }
}

impl PartialEq for Cv {
    #[inline]
    fn eq(&self, other: &Cv) -> bool {
        (self.place == other.place) & constant_time_eq::constant_time_eq_32(&self.value, &other.value)
    }
}

impl Eq for Cv {}

/// The chaining value of a message's **last** subtree, with its place and
/// length: the right child at some level of the tree's right edge. It may
/// be shorter than its left sibling, and is never a left child: no call
/// takes one on the left, so a tree with a short subtree inside it does
/// not compile.
///
/// ```compile_fail,E0308
/// # blake3_servil_core::initialize();
/// use blake3_servil_core::{Mode, throughput::TreeHasher};
/// let tree = TreeHasher::new(Mode::Plain);
/// let end = tree.last_cv(1, &[7u8; 100]);
/// let next = tree.subtree_cv(0, 2, &[7u8; 1024]);
/// let _ = tree.parent_cv(&end, &next); // a short subtree on the left
/// ```
///
/// A whole subtree that ends the message becomes one by `LastCv::from`:
/// the program says that the message ends there.
///
/// Equality and `Debug` are as [`Cv`]'s.
///
/// # Trait implementations
///
/// - `Clone` reproduces the value, starting chunk and length.
/// - `Copy` copies those coordinates along with the chaining bytes.
/// - `Debug` shows the starting chunk and length without exposing chaining
///   bytes.
/// - `Eq` identifies values with equal coordinates and bytes.
/// - `From` (of a `Cv`) keeps a whole subtree's bytes and records its first
///   chunk and full length, on the caller's word that the message ends
///   there; it panics on a `Cv` that was never set.
/// - `PartialEq` compares both coordinates and compares the chaining bytes
///   in constant time.
#[derive(Clone, Copy)]
pub struct LastCv {
    value: crate::CVBytes,
    first_chunk: u64,
    len: u64,
}

impl LastCv {
    /// A chaining value a program stored or received, for the last
    /// subtree: the message's last `len` bytes, which begin at its chunk
    /// `first_chunk`.
    ///
    /// Requires what [`TreeHasher::last_cv`] does of the place and the
    /// length.
    pub const fn at(first_chunk: u64, len: u64, bytes: [u8; OUT_LEN]) -> Self {
        assert!(len >= 1 && len <= last_max(first_chunk), "the last subtree holds from one byte to the whole subtree aligned at its first chunk");
        LastCv { value: bytes, first_chunk, len }
    }

    /// The chaining value's bytes.
    #[inline]
    pub const fn as_bytes(&self) -> &[u8; OUT_LEN] {
        &self.value
    }

    /// The message's chunk the subtree begins at.
    #[inline]
    pub const fn first_chunk(&self) -> u64 {
        self.first_chunk
    }

    /// The subtree's bytes: one or more.
    #[inline]
    #[allow(clippy::len_without_is_empty)]
    pub const fn len(&self) -> u64 {
        self.len
    }
}

impl core::fmt::Debug for LastCv {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "the last subtree, {} bytes from chunk {}", self.len, self.first_chunk)
    }
}

impl PartialEq for LastCv {
    #[inline]
    fn eq(&self, other: &LastCv) -> bool {
        (self.first_chunk == other.first_chunk) & (self.len == other.len) & constant_time_eq::constant_time_eq_32(&self.value, &other.value)
    }
}

impl Eq for LastCv {}

/// A whole subtree that ends the message: the program says so.
impl From<Cv> for LastCv {
    fn from(cv: Cv) -> LastCv {
        let (level, index) = (cv.level(), cv.index());
        LastCv { value: cv.value, first_chunk: index << level, len: CHUNK << level }
    }
}

/// A [`Mode`] prepared for a program that builds or verifies a BLAKE3 tree
/// itself (verified streaming, an outboard, a tree hashed in parts on
/// other machines): the subtrees' chaining values, their parents, and the
/// root. The mode's key is derived once, here; each call then costs its
/// hashing alone.
///
/// A message of more than one chunk (1024 bytes) is a binary tree: its left
/// subtree holds the largest power of two of chunks below its count
/// ([`left_len`]), its right subtree the rest, and so on down. A message
/// of one chunk or less is its root alone: [`hash`] or
/// [`convenience::hash_with`](crate::convenience::hash_with).
///
/// **A tree that is not BLAKE3's has no way through these calls.** Every
/// left child is a *whole* subtree: a power of two of chunks, starting at a
/// multiple of its own size. Its right sibling begins where it ends and is
/// no longer. The message's *last* subtree, the right child at every level
/// down the tree's right edge, is the only one that may be shorter.
///
/// | Part of the rule | Kept by | When |
/// |---|---|---|
/// | A whole subtree's size is a power of two of chunks | its name, `level`: no other size can be said. For a size in bytes, [`level_of`] in a `const` | the build stops |
/// | A whole subtree starts at a multiple of its size | its name, `index`, counted in subtrees of its size | cannot be written |
/// | A left child is whole | two types: [`Cv`] is whole, [`LastCv`] is the last. A left child is a `Cv` | the build stops |
/// | A root's right child ends the message | [`root_hash`](TreeHasher::root_hash) takes a `LastCv` on the right | the build stops |
/// | The bytes handed over are the subtree named | [`subtree_cv`](TreeHasher::subtree_cv) takes exactly `1024 << level` bytes | panics, in every build |
/// | The last subtree is no longer than its left sibling | [`last_cv`](TreeHasher::last_cv) takes from 1 byte to the whole subtree aligned at its first chunk | panics, in every build |
/// | Two children are siblings, in order | each value carries its place, and every call that joins two compares the places | panics, in every build |
/// | The tree holds the whole message | `root_hash` takes the message's length and compares it with what the two children hold | panics, in every build |
///
/// What no call can check: that the bytes handed over are the message's
/// bytes at the place named. Wrong bytes give a wrong hash, as they do in
/// any hash call. Nor that a value was made in this mode and under this
/// key: a [`Cv`] carries its place and not its mode, so one kept from a
/// `TreeHasher` of another mode or key joins at the same place without a
/// panic, and the root is then not promised to be the message's hash under
/// either. A
/// program that keeps values (a cache of subtrees) keeps them by mode and
/// key as well as by place.
///
/// A message hashed in parts of 4 KiB (the part size is checked when this
/// compiles; parts of 5 MiB would stop the build):
///
/// ```
/// # blake3_servil_core::initialize();
/// use blake3_servil_core::{Mode, convenience, throughput::{TreeHasher, level_of}};
/// const PART: usize = 4096;
/// const LEVEL: u32 = level_of(PART as u64);
/// let message = [7u8; 3 * PART + 100];
/// let tree = TreeHasher::new(Mode::Plain);
/// let part = |i: usize| &message[i * PART..message.len().min((i + 1) * PART)];
/// let (p0, p1, p2) = (tree.subtree_cv(LEVEL, 0, part(0)), tree.subtree_cv(LEVEL, 1, part(1)), tree.subtree_cv(LEVEL, 2, part(2)));
/// let end = tree.last_cv((3 * PART / 1024) as u64, part(3));
/// let left = tree.parent_cv(&p0, &p1);
/// let right = tree.last_parent_cv(&p2, &end);
/// assert_eq!(tree.root_hash(&left, &right, message.len() as u64), convenience::hash(&message));
/// ```
///
/// A root's right child is one that ends the message, so two whole
/// subtrees are no root's children until the program says the second is
/// the last (`right.into()`):
///
/// ```compile_fail,E0308
/// # blake3_servil_core::initialize();
/// use blake3_servil_core::{Mode, throughput::TreeHasher};
/// let tree = TreeHasher::new(Mode::Plain);
/// let (left, right) = (tree.subtree_cv(0, 0, &[7u8; 1024]), tree.subtree_cv(0, 1, &[7u8; 1024]));
/// let _ = tree.root_hash(&left, &right, 2048);
/// ```
///
/// ```
/// # blake3_servil_core::initialize();
/// use blake3_servil_core::{Mode, convenience, throughput::TreeHasher};
/// let tree = TreeHasher::new(Mode::Plain);
/// let (left, right) = (tree.subtree_cv(0, 0, &[7u8; 1024]), tree.subtree_cv(0, 1, &[7u8; 1024]));
/// assert_eq!(tree.root_hash(&left, &right.into(), 2048), convenience::hash(&[7u8; 2048]));
/// ```
///
/// # Key-dependent timing
///
/// A keyed `TreeHasher`'s work depends on the lengths it is given and on the
/// kind of its mode, never on the key's value.
/// See the [key-independence contract](crate#key-dependent-timing).
///
/// # Trait implementations
///
/// - `Clone` prepares an independent tree hasher with the same key, mode
///   and core selection.
#[derive(Clone)]
pub struct TreeHasher {
    key: crate::CVWords,
    flags: u8,
    on_this_core: bool,
}

impl TreeHasher {
    /// The tree of messages hashed in `mode`.
    pub fn new(mode: Mode) -> Self {
        let (key, flags) = mode.key_and_flags();
        Self { key, flags, on_this_core: false }
    }

    /// The `TreeHasher` of a Hasher's key words and flags, for its large
    /// subtrees' callback (Hasher::update_with).
    pub(crate) fn of_words(key: &crate::CVWords, flags: u8) -> Self {
        Self { key: *key, flags, on_this_core: false }
    }

    /// The kernels of this `TreeHasher`'s parents: a core's own, or the
    /// detected platform's.
    #[inline]
    fn parents_platform(&self) -> crate::platform::Platform {
        if self.on_this_core { crate::platform::Platform::own_core() } else { crate::platform::Platform::detect() }
    }

    /// [`subtree_cv`](TreeHasher::subtree_cv) on `platform`'s kernels: for
    /// the threads crate's caller, which holds the shared accelerator's
    /// turn for a whole job (docs/hazmat-removal.md, D).
    #[doc(hidden)]
    pub fn subtree_cv_on(&self, platform: crate::platform::Platform, level: u32, index: u64, input: &[u8]) -> Cv {
        let place = subtree_place(level, index, input.len());
        Cv { value: self.cv_on(platform, index << level, input), place }
    }

    /// A subtree's chaining value, on `platform`'s kernels.
    #[inline]
    fn cv_on(&self, platform: crate::platform::Platform, first_chunk: u64, input: &[u8]) -> crate::CVBytes {
        if input.len() <= CHUNK_LEN {
            return crate::ChunkState::new(&self.key, first_chunk, self.flags, platform).update(input).output().chaining_value();
        }
        let [left, right] = self.halves_on(platform, first_chunk, input);
        crate::hasher::parent_output(&left, &right, &self.key, self.flags, platform).chaining_value()
    }

    /// The values of a subtree's children, not yet labelled.
    #[inline]
    fn halves_on(&self, platform: crate::platform::Platform, first_chunk: u64, input: &[u8]) -> [crate::CVBytes; 2] {
        crate::tree::root_children(&crate::tree::PlatformKernels { key: &self.key, flags: self.flags, platform }, input, 0, first_chunk)
    }

    /// This `TreeHasher`, its calls kept to the calling core's own kernels: for
    /// a program that runs many of them at once on different cores. On
    /// Apple M4 and later one accelerator serves the whole chip, and a
    /// call that hashes 16 KiB or more takes it when it is free; calls on
    /// many cores at once would share it, each slower than on its own
    /// core's kernels. On other CPUs this changes nothing.
    pub fn on_this_core(mut self) -> Self {
        self.on_this_core = true;
        self
    }

    /// The kernels for a call over `len` bytes: the detected platform's,
    /// with the shared accelerator's turn where the call may take it.
    #[inline]
    fn turn(&self, len: usize) -> crate::platform::Sme2Turn {
        if self.on_this_core {
            return crate::platform::Sme2Turn::take(crate::platform::Platform::own_core(), false);
        }
        crate::platform::Sme2Turn::take(crate::platform::Platform::detect(), len >= crate::SME2_SIZED_LEN)
    }

    /// The kernels for a call that may take the shared accelerator's turn
    /// when `sme2_sized` says its work is the accelerator's size.
    #[inline]
    fn turn_if(&self, sme2_sized: bool) -> crate::platform::Sme2Turn {
        if self.on_this_core {
            return crate::platform::Sme2Turn::take(crate::platform::Platform::own_core(), false);
        }
        crate::platform::Sme2Turn::take(crate::platform::Platform::detect(), sme2_sized)
    }

    /// The hash of the whole message `input`: what
    /// [`convenience::hash_with`](crate::convenience::hash_with) returns
    /// for this `TreeHasher`'s mode, the mode's key not derived again. A
    /// program that hashes many messages under one key-derivation context
    /// pays for the context once, in [`new`](TreeHasher::new).
    ///
    /// ```
    /// # blake3_servil_core::initialize();
    /// use blake3_servil_core::{Mode, convenience, throughput::TreeHasher};
    /// let tree = TreeHasher::new(Mode::DeriveKey("example.com 2026-11-16 session keys"));
    /// assert_eq!(tree.hash(b"material"), convenience::hash_with(Mode::DeriveKey("example.com 2026-11-16 session keys"), b"material"));
    /// ```
    pub fn hash(&self, input: &[u8]) -> Hash {
        let turn = self.turn(input.len());
        self.hash_on(turn.platform(), input)
    }

    /// A worker already holds the turn for these kernels.
    #[doc(hidden)]
    #[inline]
    pub fn hash_on(&self, platform: crate::platform::Platform, input: &[u8]) -> Hash {
        crate::hash_serial_on(input, &self.key, self.flags, platform)
    }

    /// The hashes of a batch's messages, as [`hash_many`] gives them, in
    /// this `TreeHasher`'s mode: `out[i]` becomes the hash of message `i`.
    ///
    /// Requires one output a message.
    pub fn hash_many<P: Prepared>(&self, batch: &P, out: &mut [[u8; OUT_LEN]]) {
        let turn = self.turn_if(crate::many::sme2_sized(batch.message_len(), out.len()));
        self.hash_slots_on(turn.platform(), batch.bytes(), batch.message_len(), batch.slot(), out);
    }

    /// A worker's slice of a batch. Slot shape and output count are
    /// checked by the same code as `hash_many`.
    ///
    /// Requires each slot's bytes past its message to be zero, as a
    /// `Batch` and a `Packed` keep them (checked in debug builds only: a
    /// kernel that takes whole blocks hashes those bytes too). Outside the
    /// documented API: the thread library passes its batches' slices here,
    /// until it passes a checked view (BLAKE3 #458).
    #[doc(hidden)]
    #[inline]
    pub fn hash_slots_on(&self, platform: crate::platform::Platform, bytes: &[u8], len: usize, slot: usize, out: &mut [[u8; OUT_LEN]]) {
        crate::many::hash_many_on(bytes, len, slot, &self.key, self.flags, out, platform);
    }

    /// Try the whole-block lanes with separate readable inputs. Returns
    /// false without writing if this group takes the one-message path.
    #[doc(hidden)]
    #[cfg(feature = "std")]
    pub fn try_hash_group_on<'a>(
        &self,
        platform: crate::platform::Platform,
        count: usize,
        input: impl Fn(usize) -> &'a [u8],
        output: impl FnMut(usize, [u8; OUT_LEN]),
    ) -> bool {
        crate::many::try_whole_group(count, input, &self.key, self.flags, platform, output)
    }

    /// The hash of each message, of any lengths, as [`hash_each`] gives
    /// them, in this `TreeHasher`'s mode: `out[i]` becomes the hash of the
    /// message of `items[i].1` bytes that starts `items[i].0`.
    ///
    /// Requires `out.len() == items.len()`.
    #[cfg(feature = "std")]
    pub fn hash_each(&self, items: &[(&[Block], usize)], out: &mut [[u8; OUT_LEN]]) {
        assert_eq!(items.len(), out.len(), "one digest per item");
        let platform = if self.on_this_core { crate::platform::Platform::own_core() } else { crate::platform::Platform::detect() };
        crate::each::hash_each_on(&self.key, self.flags, items, out, platform, |item| self.hash(item));
    }

    /// The chaining value of the parent of two subtrees.
    fn bare_parent_cv(&self, left: &crate::CVBytes, right: &crate::CVBytes) -> crate::CVBytes {
        crate::hasher::parent_output(left, right, &self.key, self.flags, self.parents_platform()).chaining_value()
    }

    /// A layer of parents: `out[i]` is the parent of `children[2 * i]` and
    /// `children[2 * i + 1]`, side by side in the CPU's lanes.
    ///
    /// Requires an even number of children and half as many outputs.
    fn bare_parent_cvs(&self, children: &[crate::CVBytes], out: &mut [crate::CVBytes]) {
        assert_eq!(children.len() % 2, 0, "parents take their children in pairs");
        assert_eq!(out.len(), children.len() / 2, "one parent per pair of children");
        let platform = self.parents_platform();
        // As many pairs a call as the kernels take.
        let pairs = crate::platform::MAX_SIMD_DEGREE;
        for (children, out) in children.chunks(2 * pairs).zip(out.chunks_mut(pairs)) {
            if children.len() == 2 {
                out[0] = self.bare_parent_cv(&children[0], &children[1]);
                continue;
            }
            let parents = crate::compress_parents_parallel(children.as_flattened(), &self.key, self.flags, platform, out.as_flattened_mut());
            debug_assert_eq!(parents, out.len());
        }
    }

    /// The hash of the message whose root has these two children: the
    /// tree's top, where the chaining values end.
    fn bare_root_hash(&self, left: &crate::CVBytes, right: &crate::CVBytes) -> Hash {
        crate::hasher::parent_output(left, right, &self.key, self.flags, self.parents_platform()).root_hash()
    }

    /// The extended output of the message whose root has these two
    /// children: its first 32 bytes are [`bare_root_hash`](TreeHasher::bare_root_hash)'s.
    fn bare_root_output(&self, left: &crate::CVBytes, right: &crate::CVBytes) -> crate::OutputReader {
        crate::OutputReader::new(crate::hasher::parent_output(left, right, &self.key, self.flags, self.parents_platform()))
    }
    /// The chaining value of a whole subtree: the `index`-th of 2^`level`
    /// chunks.
    ///
    /// Requires `input.len() == 1024 << level`, checked in every build.
    pub fn subtree_cv(&self, level: u32, index: u64, input: &[u8]) -> Cv {
        let place = subtree_place(level, index, input.len());
        let turn = self.turn(input.len());
        Cv { value: self.cv_on(turn.platform(), index << level, input), place }
    }

    /// The chaining values of a whole subtree's left and right halves,
    /// in that order. Keep the halves when the subtree might turn out to
    /// be the whole message: its root needs both children.
    ///
    /// Requires a positive `level` and the same whole subtree as
    /// [`subtree_cv`](Self::subtree_cv), checked in every build.
    pub fn subtree_halves(&self, level: u32, index: u64, input: &[u8]) -> [Cv; 2] {
        let turn = self.turn(input.len());
        self.subtree_halves_on(turn.platform(), level, index, input)
    }

    /// The same checked call for a worker that already holds its turn.
    #[doc(hidden)]
    pub fn subtree_halves_on(&self, platform: crate::platform::Platform, level: u32, index: u64, input: &[u8]) -> [Cv; 2] {
        subtree_place(level, index, input.len());
        assert!(level > 0, "a subtree of one chunk has no halves");
        let [a, b] = half_places(level, index);
        let [left, right] = self.halves_on(platform, index << level, input);
        [Cv { value: left, place: a }, Cv { value: right, place: b }]
    }

    /// The chaining value of the message's last subtree: `input` is the
    /// message from its chunk `first_chunk` to its end.
    ///
    /// Requires from one byte to the whole subtree aligned at
    /// `first_chunk` (as many chunks as the largest power of two that
    /// divides it), checked in every build. That is what makes it no
    /// longer than the left sibling it will be given.
    pub fn last_cv(&self, first_chunk: u64, input: &[u8]) -> LastCv {
        let turn = self.turn(input.len());
        self.last_cv_on(turn.platform(), first_chunk, input)
    }

    /// The same checked call for a worker that already holds its turn.
    #[doc(hidden)]
    pub fn last_cv_on(&self, platform: crate::platform::Platform, first_chunk: u64, input: &[u8]) -> LastCv {
        let (len, max) = (input.len() as u64, last_max(first_chunk));
        assert!(len >= 1 && len <= max, "the last subtree at chunk {first_chunk} holds 1 to {max} bytes (found {len})");
        LastCv { value: self.cv_on(platform, first_chunk, input), first_chunk, len }
    }

    /// The parent of two whole subtrees: whole, a level up.
    ///
    /// Requires siblings in order (`left` at an even index, `right` the
    /// next of its size), checked in every build.
    pub fn parent_cv(&self, left: &Cv, right: &Cv) -> Cv {
        let place = left.set();
        assert!(place & 1 == 0 && right.place == place + 1, "the children are not siblings: {left:?} and {right:?}");
        Cv { value: self.bare_parent_cv(&left.value, &right.value), place: parent_place(place) }
    }

    /// The parent of a whole subtree and the message's last subtree: the
    /// last subtree a level up.
    ///
    /// Requires `left` at an even index and `right` beginning where it
    /// ends, checked in every build. (A last subtree that begins there is
    /// no longer than `left`: [`last_cv`](TreeHasher::last_cv) saw to it.)
    pub fn last_parent_cv(&self, left: &Cv, right: &LastCv) -> LastCv {
        let (level, index) = (left.level(), left.index());
        assert!(index & 1 == 0 && right.first_chunk == (index + 1) << level, "the children are not siblings: {left:?} and {right:?}");
        LastCv { value: self.bare_parent_cv(&left.value, &right.value), first_chunk: index << level, len: (CHUNK << level) + right.len }
    }

    /// A layer of parents: `out[i]` becomes the parent of `children[2 * i]`
    /// and `children[2 * i + 1]`, side by side in the CPU's lanes.
    ///
    /// The places cost 3 to 5 ns a parent over its hashing (the values are
    /// gathered for the kernels and labelled after): 2% of `b3sum
    /// --outboard` on a 1 GiB file in memory (Ryzen 9700X), which makes a
    /// parent for every 16 KiB.
    ///
    /// Requires an even number of children, half as many outputs, and the
    /// children consecutive whole subtrees of one size from an even index,
    /// checked in every build.
    pub fn parent_cvs(&self, children: &[Cv], out: &mut [Cv]) {
        assert_eq!(children.len() % 2, 0, "parents take their children in pairs");
        assert_eq!(out.len(), children.len() / 2, "one parent per pair of children");
        let Some(first) = children.first() else { return };
        let base = first.set();
        assert!(base & 1 == 0, "the first child is at an odd index: {first:?}");
        for (k, child) in children.iter().enumerate() {
            assert!(child.place == base + k as u64, "child {k} is not the next subtree of its size: {child:?}");
        }
        // The kernels read bare values side by side: gather, hash, label.
        // 32 children are the widest kernel's two rounds, and the two
        // arrays are 1.5 KiB of the stack.
        const PAIRS: usize = 16;
        let mut values = [[0u8; OUT_LEN]; 2 * PAIRS];
        let mut parents = [[0u8; OUT_LEN]; PAIRS];
        for (children, out) in children.chunks(2 * PAIRS).zip(out.chunks_mut(PAIRS)) {
            for (value, child) in values.iter_mut().zip(children) {
                *value = child.value;
            }
            self.bare_parent_cvs(&values[..children.len()], &mut parents[..out.len()]);
            for (i, parent) in out.iter_mut().enumerate() {
                *parent = Cv { value: parents[i], place: parent_place(children[2 * i].place) };
            }
        }
    }

    /// The root's two children, checked: `left` begins the message,
    /// `right` begins where it ends, and the two hold `len` bytes.
    fn root_children(left: &Cv, right: &LastCv, len: u64) {
        let (level, index) = (left.level(), left.index());
        assert!(index == 0 && right.first_chunk == 1 << level, "not a root's children: {left:?} and {right:?}");
        let held = (CHUNK << level) + right.len;
        assert!(held == len, "the root's children hold {held} bytes, and the message {len}");
    }

    /// The hash of the message of `len` bytes whose root has these two
    /// children: the tree's top, where the chaining values end.
    ///
    /// Requires `left` at the message's start, `right` beginning where it
    /// ends, and the two holding `len` bytes, checked in every build: a
    /// tree that leaves out the message's end is refused, where it would
    /// give the hash of the bytes before it.
    pub fn root_hash(&self, left: &Cv, right: &LastCv, len: u64) -> Hash {
        Self::root_children(left, right, len);
        self.bare_root_hash(&left.value, &right.value)
    }

    /// The extended output of the message of `len` bytes whose root has
    /// these two children: its first 32 bytes are
    /// [`root_hash`](TreeHasher::root_hash)'s.
    ///
    /// Requires what `root_hash` does.
    pub fn root_output(&self, left: &Cv, right: &LastCv, len: u64) -> crate::OutputReader {
        Self::root_children(left, right, len);
        self.bare_root_output(&left.value, &right.value)
    }
}

/// The bytes of the left subtree of a message, or of a subtree, of `len`
/// bytes: the largest power of two of chunks below its chunk count. The
/// right subtree holds the rest.
///
/// Requires more than one chunk: a chunk or less does not split.
pub const fn left_len(len: u64) -> u64 {
    assert!(len > CHUNK_LEN as u64, "a chunk or less does not split");
    let chunks = (len - 1) / CHUNK_LEN as u64;
    (1u64 << (u64::BITS - 1 - chunks.leading_zeros())) * CHUNK_LEN as u64
}

#[cfg(all(test, feature = "std"))]
mod tree_test {
    use super::*;
    use std::vec::Vec;

    /// Official BLAKE3's hazmat: the independent implementation of the
    /// tree's pieces.
    mod official {
        pub(super) use crate::test::official::parent;
        use blake3::hazmat::HasherExt;
        pub(super) fn hasher(mode: crate::Mode) -> blake3::Hasher {
            match mode {
                crate::Mode::Plain => blake3::Hasher::new(),
                crate::Mode::Keyed(key) => blake3::Hasher::new_keyed(key),
                crate::Mode::DeriveKey(context) => blake3::Hasher::new_derive_key(context),
                crate::Mode::DeriveKeyWith(_) => unreachable!("the tests derive from a context string"),
            }
        }
        pub(super) fn subtree_cv(mode: crate::Mode, first_chunk: u64, input: &[u8]) -> [u8; 32] {
            let mut hasher = hasher(mode);
            hasher.set_input_offset(first_chunk * 1024);
            hasher.update(input);
            hasher.finalize_non_root()
        }
    }

    const KEY: [u8; 32] = [0x6b; 32];

    fn modes() -> [Mode<'static>; 3] {
        [Mode::Plain, Mode::Keyed(&KEY), Mode::DeriveKey("blake3-servil tree api test")]
    }

    fn painted(len: usize) -> Vec<u8> {
        let mut input = std::vec![0u8; len];
        crate::test::paint_test_input(&mut input);
        input
    }

    /// A subtree's chaining value, at every kind of place and length: at
    /// chunk zero and at offsets whose alignment allows 1, 2, 4, 8, 16 and
    /// many chunks, low and beyond 2^32 chunks; lengths around a block, a
    /// chunk, a group of sixteen and powers of two up to what the place
    /// allows; in each mode, with the shared accelerator and on this core.
    /// Against official BLAKE3's hazmat.
    #[test]
    fn a_subtrees_chaining_value_matches_official_blake3_at_every_place() {
        crate::initialize();
        let input = painted(70 * CHUNK_LEN + 100);
        let mut checked = 0;
        for mode in modes() {
            for tree in [TreeHasher::new(mode), TreeHasher::new(mode).on_this_core()] {
                for first_chunk in [0u64, 1, 2, 4, 6, 8, 16, 48, 64, 1 << 20, (1 << 33) + 4, 3 << 40] {
                    let max = if first_chunk == 0 { input.len() } else { ((1u64 << first_chunk.trailing_zeros()) as usize * CHUNK_LEN).min(70 * CHUNK_LEN) };
                    for len in [
                        1,
                        63,
                        64,
                        65,
                        1023,
                        1024,
                        1025,
                        2047,
                        2048,
                        2049,
                        4096,
                        5000,
                        8 * 1024,
                        16 * 1024 - 1,
                        16 * 1024,
                        16 * 1024 + 1,
                        33 * 1024 - 5,
                        64 * 1024,
                        70 * 1024,
                    ] {
                        if len > max {
                            continue;
                        }
                        let bytes = &input[100..100 + len];
                        checked += 1;
                        let want = official::subtree_cv(mode, first_chunk, bytes);
                        assert_eq!(*tree.last_cv(first_chunk, bytes).as_bytes(), want, "first_chunk={first_chunk} len={len}");
                        if len > CHUNK_LEN {
                            let turn = tree.turn(len);
                            let [left, right] = tree.halves_on(turn.platform(), first_chunk, bytes);
                            assert_eq!(tree.bare_parent_cv(&left, &right), want, "the halves' parent, first_chunk={first_chunk} len={len}");
                            let split = left_len(len as u64) as usize;
                            assert_eq!(left, official::subtree_cv(mode, first_chunk, &bytes[..split]), "the left half, first_chunk={first_chunk} len={len}");
                            assert_eq!(
                                right,
                                official::subtree_cv(mode, first_chunk + (split / CHUNK_LEN) as u64, &bytes[split..]),
                                "the right half, first_chunk={first_chunk} len={len}"
                            );
                        }
                    }
                }
            }
        }
        // The 19 lengths where the place allows them all, fewer at the
        // narrower places: 164 a mode and tree.
        assert_eq!(checked, 3 * 2 * 164, "the places and lengths checked");
    }

    #[test]
    fn whole_subtree_halves_keep_their_places_and_values() {
        crate::initialize();
        let input = painted(64 * CHUNK_LEN);
        for mode in modes() {
            for tree in [TreeHasher::new(mode), TreeHasher::new(mode).on_this_core()] {
                for (level, index) in [(1, 0), (2, 3), (5, 7), (6, (1 << 30) + 1)] {
                    let bytes = &input[..CHUNK_LEN << level];
                    let (head, tail) = bytes.split_at(bytes.len() / 2);
                    let first = index << level;
                    let want_left = official::subtree_cv(mode, first, head);
                    let want_right = official::subtree_cv(mode, first + (head.len() / CHUNK_LEN) as u64, tail);
                    assert_eq!(
                        tree.subtree_halves(level, index, bytes),
                        [Cv::at(level - 1, 2 * index, want_left), Cv::at(level - 1, 2 * index + 1, want_right)]
                    );
                }
            }
        }
    }

    /// A parent and the root from two chaining values, against official
    /// BLAKE3's merges; the root's extended output begins with its hash;
    /// and a layer of parents is each pair's parent, at every count around
    /// the kernels' widths.
    #[test]
    fn parents_and_the_root_match_official_blake3() {
        crate::initialize();
        let bytes = painted(32 * 300);
        let cvs: Vec<crate::CVBytes> = bytes.as_chunks::<32>().0.to_vec();
        for mode in modes() {
            let tree = TreeHasher::new(mode);
            for pair in cvs[..40].chunks(2) {
                let (left, right) = (&pair[0], &pair[1]);
                assert_eq!(tree.bare_parent_cv(left, right), official::parent(mode, left, right, false));
                assert_eq!(*tree.bare_root_hash(left, right).as_bytes(), official::parent(mode, left, right, true));
                let mut out = [0u8; 150];
                tree.bare_root_output(left, right).fill(&mut out);
                assert_eq!(out[..32], *tree.bare_root_hash(left, right).as_bytes());
                assert_ne!(&tree.bare_parent_cv(left, right), tree.bare_root_hash(left, right).as_bytes(), "the root is not a chaining value");
            }
            for pairs in (0..=40).chain([63, 64, 65, 127, 128, 129, 150]) {
                let children = &cvs[..2 * pairs];
                let mut out = std::vec![[0u8; 32]; pairs];
                tree.bare_parent_cvs(children, &mut out);
                for (i, got) in out.iter().enumerate() {
                    assert_eq!(*got, tree.bare_parent_cv(&children[2 * i], &children[2 * i + 1]), "pair {i} of {pairs}");
                }
            }
        }
    }

    /// A whole message's hash built the way a tree builder builds it:
    /// split at `left_len` down to groups of sixteen chunks, each group's
    /// chaining value, each parent, and the root from its two children.
    /// And its extended output, against the Hasher's.
    #[test]
    fn a_tree_built_from_its_pieces_gives_the_messages_hash() {
        crate::initialize();
        fn cv(tree: &TreeHasher, input: &[u8], first_chunk: u64) -> crate::CVBytes {
            if input.len() <= 16 * CHUNK_LEN {
                return tree.last_cv(first_chunk, input).value;
            }
            let split = left_len(input.len() as u64) as usize;
            let left = cv(tree, &input[..split], first_chunk);
            let right = cv(tree, &input[split..], first_chunk + (split / CHUNK_LEN) as u64);
            tree.bare_parent_cv(&left, &right)
        }
        let input = painted((1 << 20) + 7);
        for mode in modes() {
            let tree = TreeHasher::new(mode).on_this_core();
            for len in [1025, 2048, 5000, 16 * 1024, 16 * 1024 + 1, 100_000, 1 << 19, (1 << 19) + 1, (1 << 20) + 7] {
                let message = &input[..len];
                let split = left_len(len as u64) as usize;
                let left = cv(&tree, &message[..split], 0);
                let right = cv(&tree, &message[split..], (split / CHUNK_LEN) as u64);
                assert_eq!(*tree.bare_root_hash(&left, &right).as_bytes(), crate::test::official::hash(mode, message), "len={len}");
                let mut out = [0u8; 200];
                tree.bare_root_output(&left, &right).fill(&mut out);
                assert_eq!(out.as_slice(), crate::test::official::xof(mode, message, 0, out.len()), "len={len}");
            }
        }
    }

    /// Where a message splits, against official BLAKE3's, at every length
    /// around each power of two of chunks up to 2^40.
    #[test]
    fn left_len_is_official_blake3s() {
        for power in 0..=40u32 {
            let chunks = 1u64 << power;
            for len in [chunks * 1024 - 1, chunks * 1024, chunks * 1024 + 1, chunks * 1024 + 1024, chunks * 1536] {
                if len > CHUNK_LEN as u64 {
                    assert_eq!(left_len(len), blake3::hazmat::left_subtree_len(len), "len={len}");
                }
            }
        }
    }

    #[test]
    #[should_panic(expected = "a chunk or less does not split")]
    fn left_len_requires_more_than_a_chunk() {
        left_len(1024);
    }

    #[test]
    #[should_panic(expected = "the last subtree at chunk 0 holds 1 to")]
    fn a_subtree_is_not_empty() {
        crate::initialize();
        TreeHasher::new(Mode::Plain).last_cv(0, &[]);
    }

    #[test]
    #[should_panic(expected = "the last subtree at chunk 6 holds 1 to 2048 bytes (found 2049)")]
    fn a_subtree_is_no_longer_than_its_place_allows() {
        crate::initialize();
        TreeHasher::new(Mode::Plain).last_cv(6, &[0; 2049]);
    }

    #[test]
    #[should_panic(expected = "a subtree of one chunk has no halves")]
    fn the_halves_of_a_chunk_do_not_exist() {
        crate::initialize();
        TreeHasher::new(Mode::Plain).subtree_halves(0, 0, &[0; 1024]);
    }

    #[test]
    #[should_panic(expected = "a whole subtree at level 1 holds 2048 bytes (found 2047)")]
    fn halves_need_the_whole_subtree() {
        crate::initialize();
        TreeHasher::new(Mode::Plain).subtree_halves(1, 0, &[0; 2047]);
    }

    #[test]
    #[should_panic(expected = "parents take their children in pairs")]
    fn a_layer_of_parents_takes_pairs() {
        crate::initialize();
        let cvs = [[1u8; 32]; 3];
        TreeHasher::new(Mode::Plain).bare_parent_cvs(&cvs, &mut [[0u8; 32]]);
    }

    #[test]
    #[should_panic(expected = "one parent per pair of children")]
    fn a_layer_of_parents_takes_one_output_a_pair() {
        crate::initialize();
        let cvs = [[1u8; 32]; 4];
        TreeHasher::new(Mode::Plain).bare_parent_cvs(&cvs, &mut [[0u8; 32]]);
    }

    /// The hash of a whole message, a batch and a collection in a prepared
    /// mode: official BLAKE3's hashes, in each mode, with the shared
    /// accelerator and on this core, at lengths from empty to past the
    /// accelerator's size and the batch sizes its turn starts at.
    #[test]
    fn a_whole_message_a_batch_and_a_collection_match_official_blake3() {
        crate::initialize();
        let input = painted(40 * CHUNK_LEN + 7);
        let lens = [0, 1, 63, 64, 65, 1023, 1024, 1025, 2048, 3 * 1024 + 5, 15 * 1024, 16 * 1024 - 1, 16 * 1024, 16 * 1024 + 1, 40 * 1024 + 7];
        for mode in modes() {
            let want = |message: &[u8]| *official::hasher(mode).update(message).finalize().as_bytes();
            for tree in [TreeHasher::new(mode), TreeHasher::new(mode).on_this_core()] {
                for len in lens {
                    assert_eq!(*tree.hash(&input[..len]).as_bytes(), want(&input[..len]), "a message of {len} bytes");
                }
                // Batches: one message and many, a block or less, a chunk
                // or less, several chunks, and the accelerator's size.
                for (len, count) in [(1, 1), (32, 300), (64, 3), (64, 40), (100, 17), (1024, 9), (5000, 4), (5000, 20), (16 * 1024, 2)] {
                    let mut batch = Batch::new(len, count);
                    for i in 0..count {
                        batch.set(i, &input[i..i + len]);
                    }
                    let mut out = std::vec![[0u8; 32]; count];
                    tree.hash_many(&batch, &mut out);
                    for (i, digest) in out.iter().enumerate() {
                        assert_eq!(*digest, want(&input[i..i + len]), "message {i} of {count} of {len} bytes");
                    }
                }
                let mut packed = Packed::<32>::new(50);
                for (i, message) in packed.iter_mut().enumerate() {
                    message.copy_from_slice(&input[i..i + 32]);
                }
                let mut out = std::vec![[0u8; 32]; 50];
                tree.hash_many(&packed, &mut out);
                for (i, digest) in out.iter().enumerate() {
                    assert_eq!(*digest, want(&input[i..i + 32]), "packed message {i}");
                }
                // A collection of every length above, twice over, so the
                // short ones go by their blocks and the middle ones share
                // lanes.
                let blocks: Vec<Vec<Block>> = lens
                    .iter()
                    .chain(&lens)
                    .map(|&len| {
                        let mut blocks = std::vec![Block([0; 64]); len.div_ceil(64)];
                        Block::bytes_mut(&mut blocks)[..len].copy_from_slice(&input[..len]);
                        blocks
                    })
                    .collect();
                let items: Vec<(&[Block], usize)> = blocks.iter().zip(lens.iter().chain(&lens)).map(|(b, &len)| (&b[..], len)).collect();
                let mut out = std::vec![[0u8; 32]; items.len()];
                tree.hash_each(&items, &mut out);
                for (digest, &len) in out.iter().zip(lens.iter().chain(&lens)) {
                    assert_eq!(*digest, want(&input[..len]), "an item of {len} bytes");
                }
            }
        }
    }

    #[test]
    #[should_panic(expected = "one digest per item")]
    fn a_collection_takes_one_output_an_item() {
        crate::initialize();
        let block = [Block([0; 64])];
        TreeHasher::new(Mode::Plain).hash_each(&[(&block[..], 3), (&block[..], 4)], &mut [[0u8; 32]]);
    }

    /// A batch's parts hash as the batch does, for messages in blocks and
    /// packed ones, at every cut; and the caller's kernels, named, give
    /// the subtree's own chaining value.
    #[test]
    fn a_batchs_parts_hash_as_the_batch_does() {
        crate::initialize();
        let input = painted(64 * 1024);
        let tree = TreeHasher::new(Mode::Keyed(&KEY));
        let mut batch = Batch::new(100, 9);
        let mut packed = Packed::<16>::new(9);
        for i in 0..9 {
            batch.set(i, &input[i..i + 100]);
            packed[i].copy_from_slice(&input[i..i + 16]);
        }
        let (mut whole, mut whole_packed) = ([[0u8; 32]; 9], [[0u8; 32]; 9]);
        tree.hash_many(&batch, &mut whole);
        tree.hash_many(&packed, &mut whole_packed);
        for cut in 0..=9 {
            let (mut parts, mut parts_packed) = ([[0u8; 32]; 9], [[0u8; 32]; 9]);
            tree.hash_many(&batch.part(0..cut), &mut parts[..cut]);
            tree.hash_many(&batch.part(cut..9), &mut parts[cut..]);
            tree.hash_many(&packed.part(0..cut), &mut parts_packed[..cut]);
            // A part of a part counts from the part's first message.
            tree.hash_many(&packed.part(1.min(cut)..9).part(cut - 1.min(cut)..9 - 1.min(cut)), &mut parts_packed[cut..]);
            assert_eq!((whole, whole_packed), (parts, parts_packed), "cut at {cut}");
        }
        for level in [0, 1, 4, 6] {
            let len = 1024 << level;
            // Chunk 64 is the place of index `64 >> level` at every level here.
            let on = tree.subtree_cv_on(crate::platform::Platform::detect(), level, 64 >> level, &input[..len]);
            assert!(on == tree.subtree_cv(level, 64 >> level, &input[..len]), "{len} bytes");
            assert_eq!(*on.as_bytes(), official::subtree_cv(Mode::Keyed(&KEY), 64, &input[..len]), "{len} bytes");
        }
    }

    #[test]
    #[should_panic(expected = "lies within the batch")]
    fn a_part_lies_within_its_batch() {
        let batch = Batch::new(100, 9);
        let _ = batch.part(3..10);
    }

    #[test]
    #[should_panic(expected = "lies within the batch")]
    fn a_part_runs_forward() {
        let batch = Batch::new(100, 9);
        #[allow(clippy::reversed_empty_ranges)]
        let _ = batch.part(3..2);
    }
}
