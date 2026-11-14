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
pub use crate::block::{Batch, Block, Prepared};
#[cfg(feature = "std")]
pub use crate::each::{finalize_each, hash_each, update_each};

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

/// A subtree's chaining value: the 32 bytes a BLAKE3 tree passes from a
/// subtree up to its parent. It is not a hash of anything a program can
/// check alone, and has its own type so that it is never taken for one:
/// the root alone gives a [`struct@Hash`] ([`TreeHasher::root_hash`]).
///
/// Equality is constant-time, as [`struct@Hash`]'s is.
#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub struct Cv([u8; OUT_LEN]);

impl Cv {
    /// The chaining value's bytes.
    #[inline]
    pub const fn as_bytes(&self) -> &[u8; OUT_LEN] {
        &self.0
    }

    /// A chaining value from its bytes: one a program stored or received.
    #[inline]
    pub const fn from_bytes(bytes: [u8; OUT_LEN]) -> Self {
        Self(bytes)
    }
}

/// This implementation is constant-time.
impl PartialEq for Cv {
    #[inline]
    fn eq(&self, other: &Cv) -> bool {
        constant_time_eq::constant_time_eq_32(&self.0, &other.0)
    }
}

impl Eq for Cv {}

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
/// ```
/// # blake3_servil_core::initialize();
/// use blake3_servil_core::{Mode, convenience, throughput::{TreeHasher, left_len}};
/// let message = [7u8; 5000];
/// let tree = TreeHasher::new(Mode::Plain);
/// let split = left_len(message.len() as u64) as usize;
/// let left = tree.subtree_cv(0, &message[..split]);
/// let right = tree.subtree_cv((split / 1024) as u64, &message[split..]);
/// assert_eq!(tree.root_hash(&left, &right), convenience::hash(&message));
/// ```
///
/// # Key-dependent timing
///
/// A keyed `TreeHasher`'s work depends on the lengths it is given and on the
/// kind of its mode, never on the key's value.
/// See the [key-independence contract](crate#key-dependent-timing).
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

    /// Requires that `input` is a whole subtree whose first chunk is
    /// `first_chunk`: not empty, and no longer than the subtree aligned at
    /// that chunk can be (at chunk zero, any length).
    fn assert_subtree(first_chunk: u64, input: &[u8]) {
        assert!(!input.is_empty(), "a subtree holds at least one byte");
        assert!(first_chunk <= u64::MAX / CHUNK_LEN as u64, "the subtree's first chunk, {first_chunk}, is beyond a message's last");
        if let Some(max) = crate::max_subtree_len(first_chunk * CHUNK_LEN as u64) {
            assert!(input.len() as u64 <= max, "the subtree at chunk {first_chunk} holds at most {max} bytes (found {})", input.len());
        }
    }

    /// The chaining value of the subtree over `input`, whose first chunk
    /// is the message's chunk `first_chunk`.
    ///
    /// Requires a whole subtree: `input` is not empty, and is no longer
    /// than the subtree aligned at `first_chunk` can be (the largest power
    /// of two of chunks that divides `first_chunk`; at chunk zero, any
    /// length). The message's last subtree may end short of that.
    pub fn subtree_cv(&self, first_chunk: u64, input: &[u8]) -> Cv {
        Self::assert_subtree(first_chunk, input);
        let turn = self.turn(input.len());
        if input.len() <= CHUNK_LEN {
            return Cv(crate::ChunkState::new(&self.key, first_chunk, self.flags, turn.platform()).update(input).output().chaining_value());
        }
        let [left, right] =
            crate::tree::root_children(&crate::tree::PlatformKernels { key: &self.key, flags: self.flags, platform: turn.platform() }, input, 0, first_chunk);
        Cv(crate::hasher::parent_output(&left, &right, &self.key, self.flags, turn.platform()).chaining_value())
    }

    /// The subtree [`subtree_cv`](TreeHasher::subtree_cv) hashes, as the
    /// chaining values of its two halves: what a caller keeps when the
    /// subtree may turn out to be the whole message, whose root
    /// ([`root_hash`](TreeHasher::root_hash)) is made from its two children.
    ///
    /// Requires what `subtree_cv` does, and more than one chunk.
    pub fn subtree_halves(&self, first_chunk: u64, input: &[u8]) -> [Cv; 2] {
        Self::assert_subtree(first_chunk, input);
        assert!(input.len() > CHUNK_LEN, "a subtree of one chunk has no halves");
        let turn = self.turn(input.len());
        let [left, right] =
            crate::tree::root_children(&crate::tree::PlatformKernels { key: &self.key, flags: self.flags, platform: turn.platform() }, input, 0, first_chunk);
        [Cv(left), Cv(right)]
    }

    /// The chaining value of the parent of two subtrees.
    pub fn parent_cv(&self, left: &Cv, right: &Cv) -> Cv {
        Cv(crate::hasher::parent_output(&left.0, &right.0, &self.key, self.flags, crate::platform::Platform::detect()).chaining_value())
    }

    /// A layer of parents: `out[i]` is the parent of `children[2 * i]` and
    /// `children[2 * i + 1]`, side by side in the CPU's lanes.
    ///
    /// Requires an even number of children and half as many outputs.
    pub fn parent_cvs(&self, children: &[Cv], out: &mut [Cv]) {
        assert_eq!(children.len() % 2, 0, "parents take their children in pairs");
        assert_eq!(out.len(), children.len() / 2, "one parent per pair of children");
        let platform = crate::platform::Platform::detect();
        // As many pairs a call as the kernels take.
        let pairs = crate::platform::MAX_SIMD_DEGREE;
        for (children, out) in children.chunks(2 * pairs).zip(out.chunks_mut(pairs)) {
            if children.len() == 2 {
                out[0] = self.parent_cv(&children[0], &children[1]);
                continue;
            }
            // Sound: Cv is a transparent [u8; 32].
            let children: &[[u8; OUT_LEN]] = unsafe { core::slice::from_raw_parts(children.as_ptr().cast(), children.len()) };
            let out: &mut [[u8; OUT_LEN]] = unsafe { core::slice::from_raw_parts_mut(out.as_mut_ptr().cast(), out.len()) };
            let parents = crate::compress_parents_parallel(children.as_flattened(), &self.key, self.flags, platform, out.as_flattened_mut());
            debug_assert_eq!(parents, out.len());
        }
    }

    /// The hash of the message whose root has these two children: the
    /// tree's top, where the chaining values end.
    pub fn root_hash(&self, left: &Cv, right: &Cv) -> Hash {
        crate::hasher::parent_output(&left.0, &right.0, &self.key, self.flags, crate::platform::Platform::detect()).root_hash()
    }

    /// The extended output of the message whose root has these two
    /// children: its first 32 bytes are [`root_hash`](TreeHasher::root_hash)'s.
    pub fn root_output(&self, left: &Cv, right: &Cv) -> crate::OutputReader {
        crate::OutputReader::new(crate::hasher::parent_output(&left.0, &right.0, &self.key, self.flags, crate::platform::Platform::detect()))
    }
}

#[cfg(feature = "zeroize")]
impl zeroize::Zeroize for TreeHasher {
    fn zeroize(&mut self) {
        // Destructured, so that a new field is not forgotten here.
        let Self { key, flags, on_this_core } = self;
        key.zeroize();
        flags.zeroize();
        on_this_core.zeroize();
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
        use blake3::hazmat::{self, HasherExt};
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
        pub(super) fn parent(mode: crate::Mode, left: &[u8; 32], right: &[u8; 32], root: bool) -> [u8; 32] {
            let context_key;
            let mode = match mode {
                crate::Mode::Plain => hazmat::Mode::Hash,
                crate::Mode::Keyed(key) => hazmat::Mode::KeyedHash(key),
                crate::Mode::DeriveKey(context) => {
                    context_key = hazmat::hash_derive_key_context(context);
                    hazmat::Mode::DeriveKeyMaterial(&context_key)
                }
                crate::Mode::DeriveKeyWith(_) => unreachable!("the tests derive from a context string"),
            };
            if root { *hazmat::merge_subtrees_root(left, right, mode).as_bytes() } else { hazmat::merge_subtrees_non_root(left, right, mode) }
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
                        assert_eq!(*tree.subtree_cv(first_chunk, bytes).as_bytes(), want, "first_chunk={first_chunk} len={len}");
                        if len > CHUNK_LEN {
                            let [left, right] = tree.subtree_halves(first_chunk, bytes);
                            assert_eq!(*tree.parent_cv(&left, &right).as_bytes(), want, "the halves' parent, first_chunk={first_chunk} len={len}");
                            let split = left_len(len as u64) as usize;
                            assert_eq!(
                                *left.as_bytes(),
                                official::subtree_cv(mode, first_chunk, &bytes[..split]),
                                "the left half, first_chunk={first_chunk} len={len}"
                            );
                            assert_eq!(
                                *right.as_bytes(),
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

    /// A parent and the root from two chaining values, against official
    /// BLAKE3's merges; the root's extended output begins with its hash;
    /// and a layer of parents is each pair's parent, at every count around
    /// the kernels' widths.
    #[test]
    fn parents_and_the_root_match_official_blake3() {
        crate::initialize();
        let bytes = painted(32 * 300);
        let cvs: Vec<Cv> = bytes.as_chunks::<32>().0.iter().map(|b| Cv::from_bytes(*b)).collect();
        for mode in modes() {
            let tree = TreeHasher::new(mode);
            for pair in cvs[..40].chunks(2) {
                let (left, right) = (&pair[0], &pair[1]);
                assert_eq!(*tree.parent_cv(left, right).as_bytes(), official::parent(mode, left.as_bytes(), right.as_bytes(), false));
                assert_eq!(*tree.root_hash(left, right).as_bytes(), official::parent(mode, left.as_bytes(), right.as_bytes(), true));
                let mut out = [0u8; 150];
                tree.root_output(left, right).fill(&mut out);
                assert_eq!(out[..32], *tree.root_hash(left, right).as_bytes());
                assert_ne!(tree.parent_cv(left, right).as_bytes(), tree.root_hash(left, right).as_bytes(), "the root is not a chaining value");
            }
            for pairs in (0..=40).chain([63, 64, 65, 127, 128, 129, 150]) {
                let children = &cvs[..2 * pairs];
                let mut out = std::vec![Cv::from_bytes([0; 32]); pairs];
                tree.parent_cvs(children, &mut out);
                for (i, got) in out.iter().enumerate() {
                    assert_eq!(*got, tree.parent_cv(&children[2 * i], &children[2 * i + 1]), "pair {i} of {pairs}");
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
        fn cv(tree: &TreeHasher, input: &[u8], first_chunk: u64) -> Cv {
            if input.len() <= 16 * CHUNK_LEN {
                return tree.subtree_cv(first_chunk, input);
            }
            let split = left_len(input.len() as u64) as usize;
            let left = cv(tree, &input[..split], first_chunk);
            let right = cv(tree, &input[split..], first_chunk + (split / CHUNK_LEN) as u64);
            tree.parent_cv(&left, &right)
        }
        let input = painted((1 << 20) + 7);
        for mode in modes() {
            let tree = TreeHasher::new(mode).on_this_core();
            for len in [1025, 2048, 5000, 16 * 1024, 16 * 1024 + 1, 100_000, 1 << 19, (1 << 19) + 1, (1 << 20) + 7] {
                let message = &input[..len];
                let split = left_len(len as u64) as usize;
                let left = cv(&tree, &message[..split], 0);
                let right = cv(&tree, &message[split..], (split / CHUNK_LEN) as u64);
                assert_eq!(*tree.root_hash(&left, &right).as_bytes(), crate::test::official::hash(mode, message), "len={len}");
                let mut out = [0u8; 200];
                tree.root_output(&left, &right).fill(&mut out);
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

    /// Chaining values compare by their bytes, and a `TreeHasher` zeroized
    /// holds no key.
    #[test]
    fn chaining_values_compare_and_a_tree_zeroizes() {
        let (one, other) = (Cv::from_bytes([1; 32]), Cv::from_bytes([2; 32]));
        assert!(one == Cv::from_bytes([1; 32]));
        assert!(one != other);
        #[cfg(feature = "zeroize")]
        {
            use zeroize::Zeroize;
            let mut tree = TreeHasher::new(Mode::Keyed(&KEY)).on_this_core();
            assert_ne!((tree.key, tree.flags, tree.on_this_core), ([0; 8], 0, false));
            tree.zeroize();
            assert_eq!((tree.key, tree.flags, tree.on_this_core), ([0; 8], 0, false));
        }
    }

    #[test]
    #[should_panic(expected = "a chunk or less does not split")]
    fn left_len_requires_more_than_a_chunk() {
        left_len(1024);
    }

    #[test]
    #[should_panic(expected = "a subtree holds at least one byte")]
    fn a_subtree_is_not_empty() {
        crate::initialize();
        TreeHasher::new(Mode::Plain).subtree_cv(0, &[]);
    }

    #[test]
    #[should_panic(expected = "the subtree at chunk 6 holds at most 2048 bytes (found 2049)")]
    fn a_subtree_is_no_longer_than_its_place_allows() {
        crate::initialize();
        TreeHasher::new(Mode::Plain).subtree_cv(6, &[0; 2049]);
    }

    #[test]
    #[should_panic(expected = "a subtree of one chunk has no halves")]
    fn the_halves_of_a_chunk_do_not_exist() {
        crate::initialize();
        TreeHasher::new(Mode::Plain).subtree_halves(0, &[0; 1024]);
    }

    #[test]
    #[should_panic(expected = "parents take their children in pairs")]
    fn a_layer_of_parents_takes_pairs() {
        crate::initialize();
        let cvs = [Cv::from_bytes([1; 32]); 3];
        TreeHasher::new(Mode::Plain).parent_cvs(&cvs, &mut [Cv::from_bytes([0; 32])]);
    }

    #[test]
    #[should_panic(expected = "one parent per pair of children")]
    fn a_layer_of_parents_takes_one_output_a_pair() {
        crate::initialize();
        let cvs = [Cv::from_bytes([1; 32]); 4];
        TreeHasher::new(Mode::Plain).parent_cvs(&cvs, &mut [Cv::from_bytes([0; 32])]);
    }
}
