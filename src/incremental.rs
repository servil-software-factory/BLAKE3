//! The Hasher: a message fed in pieces, on the calling thread (the
//! convenience API exports it; the throughput API's free functions take
//! it to other cores and in groups).

use super::*;
use crate::throughput::{Cv, TreeHasher};

/// A message's hash, fed in pieces: [`update`](Hasher::update) with each
/// piece, in order, then [`finalize`](Hasher::finalize). The pieces may be
/// of any lengths; the hash is the same as [`hash`](convenience::hash) of them all at once.
/// Every call runs on the calling thread and returns with its input
/// incorporated. For other cores, the crate `blake3-servil` has
/// `throughput::update_multithreaded`;
/// for many messages at once, [`throughput::update_each`](crate::throughput::update_each).
///
/// A `Hasher` is 26 KiB (it holds short pieces until it has 16 KiB to
/// hash at once, and rows of chaining values, so that parents are hashed
/// side by side); box it where that matters. It touches what it uses: a
/// short message touches its first block.
///
/// # After a panic
///
/// Each call says what it refuses, and a callback of the caller's may
/// panic too. A call that panics leaves each hasher it was given holding
/// a part of its message that is not specified: of a turn's pieces some
/// may be in and others not, and after a callback's panic bytes that
/// earlier updates took may be gone. Its hash is then of no use: replace
/// such a hasher with a new one (or let
/// [`finalize_each`](crate::throughput::finalize_each) take that hash,
/// which resets the hasher) before its next message.
///
/// # Examples
///
/// ```
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// # blake3_servil_core::initialize();
/// // Hash an input incrementally.
/// let mut hasher = blake3_servil_core::convenience::Hasher::new(blake3_servil_core::Mode::Plain, None);
/// hasher.update(b"foo");
/// hasher.update(b"bar");
/// hasher.update(b"baz");
/// // finalize borrows the Hasher: more updates may follow it.
/// assert_eq!(hasher.finalize(), blake3_servil_core::convenience::hash(b"foobarbaz"));
///
/// // Extended output. OutputReader also implements Read and Seek.
/// # #[cfg(feature = "std")] {
/// let mut output = [0; 1000];
/// let mut hasher = blake3_servil_core::convenience::Hasher::new(blake3_servil_core::Mode::Plain, None);
/// hasher.update(b"foobarbaz");
/// let mut output_reader = hasher.finalize_xof();
/// output_reader.fill(&mut output);
/// assert_eq!(&output[..32], blake3_servil_core::convenience::hash(b"foobarbaz").as_bytes());
/// # }
/// # Ok(())
/// # }
/// ```
///
/// # Trait implementations
///
/// - `Clone` copies the hashing state (26 KiB: the key, the tree so far and
///   up to the last 16 KiB of the message's bytes), so both hashers go on
///   with the same message, each alone; nothing wipes either copy when it
///   is dropped.
/// - `Debug` shows the mode flags and platform, keeping the key and message
///   private.
/// - `Write` (with `std`) feeds every written byte in order; flushing
///   leaves the message unchanged.
pub struct Hasher {
    core: HasherCore,
    stage: stage::Stage,
    /// Set while one `finalize_each` call finishes this hasher, so that
    /// the call sees an index it is given twice (`each::name_each`).
    finishing: bool,
}

impl Clone for Hasher {
    #[inline]
    fn clone(&self) -> Self {
        Self { core: self.core.clone(), stage: self.stage.clone(), finishing: self.finishing }
    }

    #[inline]
    fn clone_from(&mut self, source: &Self) {
        self.core.clone_from(&source.core);
        self.stage.clone_from(&source.stage);
        self.finishing = source.finishing;
    }
}

/// The bytes a Hasher gathers before they hash side by side: 16 chunks,
/// the widest kernels' lanes.
pub(crate) const STAGE_LEN: usize = 16 * CHUNK_LEN;

/// The stage buffer and its initialized-prefix length, kept private so
/// `len <= STAGE_LEN`, `buf[..len]` initialized, and (when `len > 0`)
/// `buf[..BLOCK_LEN]` zero-padded past `len` hold by construction.
mod stage {
    use crate::{BLOCK_LEN, STAGE_LEN};

    pub(crate) struct Stage {
        len: usize,
        buf: [core::mem::MaybeUninit<u8>; STAGE_LEN],
    }

    impl Clone for Stage {
        #[inline]
        fn clone(&self) -> Self {
            Self { len: self.len, buf: self.buf }
        }

        #[inline]
        fn clone_from(&mut self, source: &Self) {
            // Copy the live bytes, and with any staged the whole first block
            // (`first_block`; an empty stage's is unwritten, and never read);
            // the rest of the capacity is overwritten before it becomes input.
            let prefix = &source.buf[..if source.len == 0 { 0 } else { source.len.max(BLOCK_LEN) }];
            self.len = source.len;
            self.buf[..prefix.len()].copy_from_slice(prefix);
        }
    }

    impl Stage {
        #[inline(always)]
        pub(crate) fn new() -> Self {
            Self { len: 0, buf: [core::mem::MaybeUninit::uninit(); STAGE_LEN] }
        }

        #[inline(always)]
        pub(crate) fn len(&self) -> usize {
            self.len
        }

        #[inline(always)]
        pub(crate) fn is_empty(&self) -> bool {
            self.len == 0
        }

        #[cfg(any(feature = "std", test))]
        #[inline(always)]
        pub(crate) fn clear(&mut self) {
            self.len = 0;
        }

        /// The message's first block, padded with zeros, while the message is
        /// a block or less and whole in the stage. An empty stage's is a block
        /// of zeros apart (a new Hasher's stage is unwritten, and a reset one's
        /// holds the last message).
        #[cfg(blake3_sse41_ffi)]
        #[inline(always)]
        pub(crate) fn first_block(&self) -> &[u8; BLOCK_LEN] {
            static EMPTY: [u8; BLOCK_LEN] = [0; BLOCK_LEN];
            if self.len == 0 {
                return &EMPTY;
            }
            // Sound: when `self.len > 0`, `append`, `pad_first_block`, and
            // `clone_from` initialize `buf[..BLOCK_LEN]` with zeros past `len`.
            unsafe { &*(self.buf.as_ptr() as *const [u8; BLOCK_LEN]) }
        }

        /// The bytes gathered in the stage.
        #[inline(always)]
        pub(crate) fn as_bytes(&self) -> &[u8] {
            // Sound: `self.len <= STAGE_LEN` and `buf[..self.len]` were written.
            unsafe { core::slice::from_raw_parts(self.buf.as_ptr() as *const u8, self.len) }
        }

        /// Copy `bytes` into the stage after what it holds. Under a block
        /// into an empty stage, the rest of the first block is zeroed.
        pub(crate) fn append(&mut self, bytes: &[u8]) {
            assert!(bytes.len() <= STAGE_LEN - self.len, "staged bytes exceed STAGE_LEN");
            crate::counted(crate::Counted::Staged);
            if self.len == 0 {
                // (From a block up there is nothing left of the block to zero.)
                self.buf[bytes.len().min(BLOCK_LEN)..BLOCK_LEN].fill(core::mem::MaybeUninit::new(0));
            }
            self.buf[self.len..self.len + bytes.len()].write_copy_of_slice(bytes);
            self.len += bytes.len();
        }

        /// Store `input` (at most `BLOCK_LEN` bytes) into an empty stage as one
        /// zero-padded block in the 16-byte pieces `finalize`'s kernel loads.
        #[cfg(blake3_sse41_ffi)]
        #[inline(always)]
        pub(crate) fn pad_first_block(&mut self, input: &[u8]) {
            assert!(self.len == 0 && input.len() <= BLOCK_LEN, "pad_first_block requires empty stage and at most one block");
            // Sound: the stage holds 64 bytes and more; the reads stay inside `input`.
            unsafe { crate::chunk_root::pad_block(self.buf.as_mut_ptr().cast(), input.as_ptr(), input.len()) };
            self.len = input.len();
        }

        /// Clear the stage length first (so a panic in `f` never leaves stale
        /// staged bytes), then pass the previously staged bytes to `f`.
        #[inline(always)]
        pub(crate) fn take_with(&mut self, f: impl FnOnce(&[u8])) {
            let len = core::mem::replace(&mut self.len, 0);
            // Sound: `buf[..len]` were written before `self.len` was cleared,
            // and `self.buf` is not mutated while `bytes` is borrowed by `f`.
            let bytes = unsafe { core::slice::from_raw_parts(self.buf.as_ptr() as *const u8, len) };
            f(bytes);
        }

        #[cfg(all(test, feature = "std"))]
        pub(crate) fn fill_from_for_test(&mut self, start: usize, byte: u8) {
            self.buf[start..].fill(core::mem::MaybeUninit::new(byte));
        }

        /// # Safety
        ///
        /// The test caller must have initialized `self.buf[i]` before reading it.
        #[cfg(all(test, feature = "std"))]
        pub(crate) unsafe fn byte_for_test(&self, i: usize) -> u8 {
            // Sound: caller's precondition.
            unsafe { self.buf[i].assume_init() }
        }
    }
}

/// The tree's state behind a [`Hasher`]: its immutable prepared mode,
/// its chunk, and the chaining values of its finished subtrees.
pub(crate) struct HasherCore {
    domain: Domain,
    chunk_state: ChunkState,
    // The stack size is MAX_DEPTH + 1 because we do lazy merging. For example,
    // with 7 chunks, we have 3 entries in the stack. Adding an 8th chunk
    // requires a 4th entry, rather than merging everything down to 1, because
    // we don't know whether more input is coming (official BLAKE3's
    // reference implementation merges eagerly instead).
    cv_stack: CvStack,
}

impl Clone for HasherCore {
    fn clone(&self) -> Self {
        Self { domain: self.domain, chunk_state: self.chunk_state.clone(), cv_stack: self.cv_stack.clone() }
    }

    /// In place, field by field: the stack copies the entries it holds
    /// (a clone made and then moved here would copy 10 KiB twice).
    fn clone_from(&mut self, source: &Self) {
        self.domain = source.domain;
        self.chunk_state.clone_from(&source.chunk_state);
        self.cv_stack.clone_from(&source.cv_stack);
    }
}

/// Check that `incoming` more bytes after `count` stay within BLAKE3's
/// `2^64 - 1` byte message domain.
#[inline(always)]
fn check_input_room(count: u64, incoming: usize) {
    assert!(incoming as u64 <= u64::MAX - count, "BLAKE3 messages contain at most 2^64 - 1 bytes (have {count}, adding {incoming})");
}

/// What the Hasher's kernels take from the library (hasher::Hooks): with
/// `pooled`, subtrees of the given size and more on the pool; whole
/// subtrees on `platform` (an SME2 turn's); `chunk_platform`, the chunk
/// state's, otherwise.
struct HasherHooks<'a> {
    pub(crate) key: &'a CVWords,
    flags: u8,
    chunk_platform: Platform,
    platform: Platform,
    pooled: Option<&'a Pool<'a>>,
}

/// The callback and threshold that one call of Hasher::update_with lends.
pub(crate) struct Pool<'a> {
    pub min_len: usize,
    pub halves: &'a dyn Fn(&TreeHasher, u32, u64, &[u8]) -> [Cv; 2],
}

impl hasher::Hooks for HasherHooks<'_> {
    #[inline(always)]
    #[allow(unused_variables)]
    fn subtree_cv(&self, input: &[u8], ahead: usize, counter: u64) -> Option<CVBytes> {
        // After the first input, no subtree is the root: on SME2 the flat
        // walk runs it down to its own chaining value, so one value goes
        // on the stack and about one merge runs on the core between SME2
        // kernels (about 0.2 us of other work between them puts the SME
        // unit in its slow state: `sme2`'s documentation).
        #[cfg(blake3_sme2)]
        if self.pooled.is_none() && matches!(self.platform, Platform::SME2) && sme2::flat_takes(input.len()) {
            crate::counted(crate::Counted::Flat);
            // Safe: the SME2 platform is selected only where the CPU has it.
            return Some(unsafe { sme2::compress_subtree_flat_to_cv(input, ahead, self.key, counter, self.flags) });
        }
        None
    }

    #[inline(always)]
    fn children(&self, input: &[u8], ahead: usize, counter: u64) -> [CVBytes; 2] {
        if let Some(pool) = self.pooled.filter(|pool| input.len() >= pool.min_len) {
            let level = crate::throughput::level_of(input.len() as u64);
            let index = counter >> level;
            assert!(index << level == counter, "a subtree of {} bytes starts at its own boundary, not chunk {counter}", input.len());
            let halves = (pool.halves)(&TreeHasher::of_words(self.key, self.flags), level, index, input);
            return Cv::halves_bytes(halves, level, index);
        }
        let pair = if self.pooled.is_some() {
            // Pooled, the subtrees below take the lock one at a time, and
            // the pool's own never use SME2.
            let own = platform::Sme2Turn::take(self.chunk_platform, input.len() >= SME2_SIZED_LEN);
            compress_subtree_to_parent_node(input, ahead, self.key, counter, self.flags, own.platform())
        } else {
            compress_subtree_to_parent_node(input, ahead, self.key, counter, self.flags, self.platform)
        };
        [pair[..OUT_LEN].try_into().unwrap(), pair[OUT_LEN..].try_into().unwrap()]
    }

    /// A whole group or more is cut, on the caller's own kernels: not
    /// pooled, and by kernels 4 to ROW lanes wide (the SME2 unit's are
    /// wider, and its subtrees are its own: `subtree_cv`).
    #[inline(always)]
    fn cuts(&self, len: usize) -> bool {
        self.pooled.is_none() && len >= STAGE_LEN && (4..=hasher::ROW).contains(&self.platform.simd_degree())
    }

    #[inline(always)]
    fn cut(&self, input: &[u8], ahead: usize, counter: u64, out: &mut [CVBytes; hasher::ROW]) -> usize {
        crate::counted(crate::Counted::Row);
        let kernels = crate::tree::PlatformKernels { key: self.key, flags: self.flags, platform: self.platform };
        crate::tree::wide(&kernels, input, ahead, counter, out)
    }

    #[inline(always)]
    fn parents(&self, children: &[CVBytes], out: &mut [CVBytes]) -> usize {
        compress_parents_parallel(children.as_flattened(), self.key, self.flags, self.platform, out.as_flattened_mut())
    }
}

/// The Hasher's kernels: chunks, nodes, and parents on the chunk state's
/// platform, the rest from `hooks`.
type HasherKernels<'a> = hasher::Kernels<'a, Platform, HasherHooks<'a>>;

/// A Hasher's kernels on its immutable domain and chunk state (see HasherHooks).
#[inline(always)]
fn kernels_on<'a>(domain: &'a Domain, chunk_state: &ChunkState, platform: Platform, pooled: Option<&'a Pool<'a>>) -> HasherKernels<'a> {
    hasher::Kernels {
        key: domain.key(),
        flags: domain.flags(),
        platform: chunk_state.platform(),
        hooks: HasherHooks { key: domain.key(), flags: domain.flags(), chunk_platform: chunk_state.platform(), platform, pooled },
    }
}

impl Hasher {
    #[inline(always)]
    pub(crate) fn from_core(core: HasherCore) -> Self {
        // One literal, with nothing written after it: a Hasher made in a
        // local and then written to (its first block zeroed) was copied
        // to its caller, 16 KiB of it, once the core held the stack's rows
        // (160 ns a 64-byte message where 41 had done). The empty stage's
        // first block is never read (`first_block`). Whether the compiler
        // builds a Hasher where it goes depends on its fields' layout too
        // (one more byte in the stack cost 13 ns; the stack aligned to 64
        // bytes, 68 ns), and no test of ours can see it: after any change
        // to these types, time `new` on x86-64 and on AArch64.
        Self { core, stage: stage::Stage::new(), finishing: false }
    }

    /// The prepared mode (immutable key words and domain flags).
    #[cfg(feature = "std")]
    #[inline(always)]
    pub(crate) fn domain(&self) -> &Domain {
        &self.core.domain
    }

    #[cfg(feature = "std")]
    #[inline(always)]
    pub(crate) fn is_plain(&self) -> bool {
        self.core.domain.is_plain()
    }

    #[cfg(feature = "std")]
    #[inline(always)]
    pub(crate) fn platform(&self) -> Platform {
        self.core.chunk_state.platform()
    }

    /// Mark this hasher as participating in an active `finalize_each` call;
    /// returns `true` if it was already marked.
    #[cfg(feature = "std")]
    #[inline(always)]
    pub(crate) fn mark_finishing(&mut self) -> bool {
        core::mem::replace(&mut self.finishing, true)
    }

    #[cfg(feature = "std")]
    #[inline(always)]
    pub(crate) fn clear_finishing(&mut self) {
        self.finishing = false;
    }

    #[cfg(all(test, feature = "std"))]
    pub(crate) fn set_platform_for_test(&mut self, platform: Platform) {
        self.core.chunk_state.set_platform(platform);
    }

    #[cfg(any(all(test, feature = "std"), all(blake3_avx512_ffi, target_os = "linux", feature = "std")))]
    #[inline(always)]
    pub(crate) fn staged_len(&self) -> usize {
        self.stage.len()
    }

    #[cfg(all(blake3_avx512_ffi, target_os = "linux", feature = "std"))]
    #[inline(always)]
    pub(crate) fn chunk_state(&self) -> &ChunkState {
        &self.core.chunk_state
    }

    #[cfg(all(blake3_avx512_ffi, target_os = "linux", feature = "std"))]
    #[inline(always)]
    pub(crate) fn stage_first_block(&self) -> &[u8; BLOCK_LEN] {
        self.stage.first_block()
    }

    /// True when the whole message is plain mode, at most one chunk, and
    /// sits completely in the stage (AArch64 `finalize_each` batching).
    #[cfg(all(blake3_neon, feature = "std"))]
    #[inline(always)]
    pub(crate) fn is_plain_single_chunk_staged(&self) -> bool {
        self.stage.len() <= CHUNK_LEN && self.core.count() == 0 && self.core.domain.is_plain()
    }

    /// The bytes gathered in the stage.
    #[cfg(all(blake3_neon, feature = "std"))]
    pub(crate) fn staged_bytes(&self) -> &[u8] {
        self.stage.as_bytes()
    }

    /// Copy `bytes` into the stage after what it holds.
    #[cfg(test)]
    pub(crate) fn stage(&mut self, bytes: &[u8]) {
        self.stage.append(bytes);
    }

    /// `update`'s gathering: an update of STAGE_LEN or more from a 16 KiB
    /// boundary of the message (or an empty stage) goes to the core whole;
    /// shorter ones gather in the stage until it reaches the next boundary,
    /// and the stage goes to the core as one update, its chunks side by
    /// side in the lanes. A message of under 16 KiB is therefore whole in
    /// the stage, where `finalize` hashes it as the one-shot call would.
    fn gather(&mut self, input: &[u8], pooled: Option<&Pool<'_>>) -> &mut Self {
        self.gather_part(input, pooled, false);
        self
    }

    /// [`gather`](Hasher::gather), but with `hold`, a stage that fills a
    /// whole group is kept for the caller to hash with others' (`held_group`),
    /// and the rest of `input` returned; else nothing is left.
    pub(crate) fn gather_part<'a>(&mut self, mut input: &'a [u8], pooled: Option<&Pool<'_>>, hold: bool) -> &'a [u8] {
        check_input_room(self.count(), input.len());
        while !input.is_empty() {
            // Whole groups skip the stage. So does a fresh hasher's first
            // chunk in a turn of many hashers (`hold`) on x86-64, where
            // `finalize_each` finishes the messages still in their chunk
            // states side by side (ffi_output::input). Everywhere else the
            // first chunk is staged: `finalize` then finds a short message
            // whole in the stage, on AArch64 `finalize_each` hashes those
            // side by side (each.rs), and a longer message's first chunk
            // joins its group of sixteen.
            let first_chunk_to_core = hold && !cfg!(blake3_neon) && self.core.count() + input.len() as u64 <= CHUNK_LEN as u64;
            if self.stage.is_empty() && first_chunk_to_core {
                self.core.update_pooled(input, pooled);
                return &[];
            }
            let at = self.count() % STAGE_LEN as u64;
            // On a 16 KiB boundary with nothing staged, the input's whole
            // groups go to the core as they lie; what follows them is
            // staged below, so the next groups start on a boundary too
            // (BLAKE3 #393: sent whole, an input's ragged end left the
            // core off the boundary, and every later piece hashed in the
            // narrow subtrees that fit there, at half the speed).
            if self.stage.is_empty() && at == 0 && input.len() >= STAGE_LEN {
                let groups = input.len() / STAGE_LEN * STAGE_LEN;
                // The loop's variant on this path: a group or more leaves the input.
                assert!(groups > 0, "no whole group went to the core");
                self.core.update_groups(&input[..groups], pooled);
                input = &input[groups..];
                continue;
            }
            let to_boundary = STAGE_LEN - at as usize;
            let take = cmp::min(to_boundary, input.len());
            // The loop's variant: every pass stages at least one byte of
            // the input, which is not empty (`at` is below STAGE_LEN).
            assert!(take > 0, "the stage took nothing");
            self.stage.append(&input[..take]);
            input = &input[take..];
            if take < to_boundary {
                return &[];
            }
            if hold && self.held_group().is_some() {
                return input;
            }
            self.stage.take_with(|bytes| self.core.update_groups(bytes, pooled));
        }
        &[]
    }

    /// A whole group in the stage: its first chunk's counter and its bytes.
    #[cfg(feature = "std")]
    pub(crate) fn held_group(&self) -> Option<(u64, &[u8; STAGE_LEN])> {
        (self.stage.len() == STAGE_LEN).then(|| (self.core.count() / CHUNK_LEN as u64, self.stage.as_bytes().try_into().unwrap()))
    }

    #[cfg(not(feature = "std"))]
    pub(crate) fn held_group(&self) -> Option<(u64, &[u8; STAGE_LEN])> {
        None
    }

    /// The held group's subtree, as [`HasherCore::push_group`] takes it
    /// (its two halves' values at chunk zero, else its own), into the
    /// core; the stage empty again.
    #[cfg(feature = "std")]
    pub(crate) fn take_held(&mut self, result: &[u8; BLOCK_LEN]) {
        assert_eq!(self.stage.len(), STAGE_LEN, "a held group fills the stage");
        self.stage.clear();
        self.core.push_group(result);
    }

    /// The key of a fresh core, as a kernel reads it: the chunk state's
    /// chaining value, which is the key until a block is compressed. Not
    /// `core.domain.key()`: `new` has just stored both, and it stores
    /// `domain.key` in 16-byte pieces that straddle the kernel's two
    /// 16-byte loads of it, which a CPU cannot forward to those loads, so
    /// the kernel's first round would wait for the stores to reach the
    /// cache (14 ns of a 64-byte message's 55 ns on a Zen 4 core,
    /// November 14, 2026). The chaining value is 64-byte aligned
    /// (ChunkState), stored as it is loaded.
    #[inline(always)]
    fn fresh_key(&self) -> &CVWords {
        debug_assert_eq!(self.core.count(), 0, "a fresh core's chaining value is its key");
        self.core.chunk_state.cv()
    }

    /// The root's output with the Hasher left as it is. With nothing
    /// staged it is the core's. Else, where the core holds whole groups
    /// (as `update` leaves it), the staged bytes are a whole subtree after
    /// them: a local tail chunk and stack hash them, on this frame, and
    /// their node is folded into the stack's entries, which are only read
    /// (`hasher::above`); the Hasher's core is not copied (10 KiB with the
    /// stack's rows). After a turn of many hashers that left a first
    /// chunk in the core (`gather_part`), a copy of the core takes the
    /// staged bytes.
    fn root_output(&self) -> Output {
        if self.stage.is_empty() {
            return self.core.final_output();
        }
        let core = &self.core;
        let counter = core.chunk_state.chunk_counter();
        if core.chunk_state.count() > 0 || counter % (STAGE_LEN / CHUNK_LEN) as u64 != 0 {
            let mut core = self.core.clone();
            core.update(self.stage.as_bytes());
            return core.final_output();
        }
        crate::counted(crate::Counted::TailApart);
        let kernels = core.kernels();
        let mut tail_chunk = ChunkState::new(core.domain.key(), counter, core.domain.flags(), core.chunk_state.platform());
        let mut tail_stack = CvStack::new();
        hasher::feed(&kernels, &mut tail_chunk, &mut tail_stack, counter, self.stage.as_bytes());
        let node = hasher::final_node(&kernels, &tail_chunk, &tail_stack);
        if counter == 0 {
            return node;
        }
        hasher::above(&kernels, &core.cv_stack, hasher::ones(counter), node)
    }

    /// The root hash, for a caller that then resets the Hasher
    /// (`finalize_each`): a message whole in the stage as `finalize` hashes
    /// it, any other finished in place, with no copy of the core.
    #[cfg(feature = "std")]
    pub(crate) fn take_root_hash(&mut self) -> Hash {
        if self.core.count() == 0 {
            return self.finalize();
        }
        self.take_output().root_hash()
    }

    /// The root's output, the state finished in place: the Hasher is then
    /// finished, to be reset or dropped (`finalize_each`, which resets it).
    #[cfg(feature = "std")]
    pub(crate) fn take_output(&mut self) -> Output {
        self.stage.take_with(|staged| {
            self.core.update(staged);
        });
        self.core.final_output()
    }

    /// Construct a new `Hasher` in a [`Mode`]: the plain hash, the keyed
    /// hash, or key derivation with a context string or a context key.
    ///
    /// `length_hint` is the message's likely length in bytes, where the
    /// program knows one (a file's size, a protocol's length field), or
    /// `None`: a scheduling hint, never part of the hash, so the digest is
    /// the same whatever it says and however many bytes then arrive. The
    /// Hasher does not use it today; it is here so that one that does
    /// changes no caller (AGENTS, "Our APIs freeze at 1.0").
    pub fn new(mode: Mode, length_hint: Option<u64>) -> Self {
        let _ = length_hint;
        Self::from_core(HasherCore::new_internal(Domain::from_mode(mode)))
    }

    /// The `Hasher` back in its initial state, its mode kept: what
    /// `finalize_each` leaves the hashers it finishes in.
    #[cfg(any(feature = "std", test))]
    pub(crate) fn reset(&mut self) {
        self.core.reset();
        self.stage.clear();
        self.finishing = false;
    }

    /// Add the message's next piece, of any length, on the calling thread.
    ///
    /// # Panics
    ///
    /// Panics if the cumulative input length would exceed BLAKE3's maximum
    /// message length of `2^64 - 1` bytes (this limit applies equally through
    /// [`update_with`](Hasher::update_with), [`update_reader`](Hasher::update_reader),
    /// [`Write`](std::io::Write), and grouped updates).
    ///
    /// **Speed**: pieces of any size hash at nearly the speed of whole
    /// buffers: short pieces are copied into the `Hasher` until it holds
    /// 16 KiB, which then hash at once. Pieces of 16 KiB and more, starting
    /// at a multiple of 16 KiB of the message, skip the copy (about 5% of the
    /// time on an Apple M4). For a whole message in memory, [`hash`](convenience::hash) is a
    /// little faster; for a file or reader, see
    /// [`update_reader`](#method.update_reader).
    #[inline]
    pub fn update(&mut self, input: &[u8]) -> &mut Self {
        // A block or less into an empty stage (a Hasher per short message,
        // above all) is staged here, in line, as one zero-padded block in
        // the 16-byte pieces `finalize`'s kernel loads (pad_block). When
        // `chunk_counter < 2^54 - 1`, at least 1,023 bytes remain in the
        // 2^64 - 1 byte domain even if the buffered chunk is full, so <= 64
        // bytes always fit; near the limit we fall through to `update_longer`
        // for the exact `check_input_room` check.
        #[cfg(blake3_sse41_ffi)]
        if input.len() <= BLOCK_LEN && self.stage.is_empty() && self.core.chunk_state.chunk_counter() < (1u64 << 54) - 1 {
            crate::counted(crate::Counted::PaddedFirstBlock);
            self.stage.pad_first_block(input);
            return self;
        }
        self.update_longer(input)
    }

    /// [`update`](Hasher::update) past its path for a block or less into an
    /// empty stage, out of line.
    #[inline(never)]
    fn update_longer(&mut self, input: &[u8]) -> &mut Self {
        // A fresh hasher's first update is a one-shot call's work (a
        // Hasher per message, the digest traits): it prefetches its code,
        // as convenience::hash does. A Hasher fed in pieces pays it once.
        // Under 16 KiB it waits whole in the stage for `finalize`, which
        // hashes it with the one-shot call's kernels; from 16 KiB it goes
        // to the core now.
        #[cfg(all(blake3_neon, feature = "std"))]
        if self.count() == 0 && (CHUNK_LEN + 1..PREFETCH_BELOW).contains(&input.len()) {
            prefetch_cold(input.len(), self.core.chunk_state.platform(), input.len() >= STAGE_LEN);
        }
        self.gather(input, None)
    }

    /// Add the next piece, letting the caller hash large whole subtrees.
    ///
    /// The Hasher can pass subtrees of at least `min_len` bytes to
    /// `halves`. For each call, return the same ordered values as
    /// `tree.subtree_halves(level, index, input)` in the given mode. The
    /// Hasher checks both places in every build; the callback is
    /// responsible for their bytes.
    ///
    /// The callback finishes its work before returning. It is borrowed
    /// only for this update, so it can use the caller's workers or scratch.
    /// Input that is not passed to it hashes on the calling thread.
    ///
    /// If the callback panics, the Hasher is as [after a panic](Hasher#after-a-panic):
    /// replace it.
    pub fn update_with(&mut self, input: &[u8], min_len: usize, halves: &dyn Fn(&TreeHasher, u32, u64, &[u8]) -> [Cv; 2]) -> &mut Self {
        self.gather(input, Some(&Pool { min_len, halves }))
    }

    /// The [`struct@Hash`] of everything updated so far. The Hasher is left
    /// as it was: more updates may follow, and a later `finalize` then
    /// covers them too, as with official BLAKE3's `Hasher`.
    ///
    /// **Speed**: when the entire message is still in the stage (under 16 KiB
    /// fed via `update`), it is hashed here as [`hash`](convenience::hash)
    /// would hash it. Otherwise, an empty stage reads the core in place, a
    /// staged tail after an aligned empty chunk hashes the tail apart without
    /// copying the core, and a fallback (such as staged bytes behind a partial
    /// chunk left in the core by `update_each` on x86-64) copies the core
    /// (~10 KiB).
    #[inline]
    pub fn finalize(&self) -> Hash {
        // A message whole in the stage, the empty one too: the core holds
        // none of it.
        if self.core.count() == 0 {
            // A block or less: one kernel call on the stage's first block.
            // Read the mode byte from `chunk_state` (same 64-byte-aligned
            // cache line as `fresh_key`, initialized from `domain`).
            #[cfg(blake3_sse41_ffi)]
            if self.stage.len() <= BLOCK_LEN {
                // Sound: the platform's instructions were detected.
                return Hash(unsafe {
                    crate::chunk_root::block_root(
                        self.core.chunk_state.platform(),
                        self.stage.first_block(),
                        self.stage.len(),
                        self.fresh_key(),
                        self.core.chunk_state.flags(),
                    )
                });
            }
            return crate::hash_serial_from(self.core.chunk_state.platform(), self.stage.as_bytes(), self.fresh_key(), self.core.domain.flags(), false);
        }
        self.root_output().root_hash()
    }

    /// An [`OutputReader`] over everything updated so far, which supplies
    /// any number of output bytes; the Hasher is left as it was, as with
    /// [`finalize`](Hasher::finalize).
    pub fn finalize_xof(&self) -> OutputReader {
        OutputReader::new(self.root_output())
    }

    /// The number of bytes passed to [`update`](Hasher::update) so far.
    pub(crate) fn count(&self) -> u64 {
        self.core.count() + self.stage.len() as u64
    }

    /// As [`update`](Hasher::update), but reading from a
    /// [`std::io::Read`](https://doc.rust-lang.org/std/io/trait.Read.html) implementation.
    ///
    /// [`Hasher`] implements
    /// [`std::io::Write`](https://doc.rust-lang.org/std/io/trait.Write.html), so it's possible to
    /// use [`std::io::copy`](https://doc.rust-lang.org/std/io/fn.copy.html) to update a [`Hasher`]
    /// from any reader. Unfortunately, this standard approach can limit performance, because
    /// `copy` currently uses an internal 8 KiB buffer that isn't big enough to take advantage of
    /// all SIMD instruction sets. (In particular, [AVX-512](https://en.wikipedia.org/wiki/AVX-512)
    /// needs a 16 KiB buffer.) `update_reader` avoids this performance problem and is slightly
    /// more convenient.
    ///
    /// The internal buffer size this method uses may change at any time, and it may be different
    /// for different targets. The only guarantee is that it will be large enough for all of this
    /// crate's SIMD implementations on the current platform.
    ///
    /// The most common implementer of
    /// [`std::io::Read`](https://doc.rust-lang.org/std/io/trait.Read.html) might be
    /// [`std::fs::File`](https://doc.rust-lang.org/std/fs/struct.File.html).
    ///
    /// This method requires the `std` Cargo feature, which is enabled by default.
    ///
    /// The buffer holds the message's last bytes when the call returns, and
    /// is not cleared. For a secret message, read into a buffer of your
    /// own, call [`update`](Hasher::update), and clear the buffer.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use std::fs::File;
    /// # use std::io;
    /// # fn main() -> io::Result<()> {
    /// # blake3_servil_core::initialize();
    /// // Hash standard input.
    /// let mut hasher = blake3_servil_core::convenience::Hasher::new(blake3_servil_core::Mode::Plain, None);
    /// hasher.update_reader(std::io::stdin().lock())?;
    /// println!("{}", hasher.finalize());
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "std")]
    pub fn update_reader(&mut self, reader: impl std::io::Read) -> std::io::Result<&mut Self> {
        io::copy_wide(reader, self)?;
        Ok(self)
    }
}

impl HasherCore {
    #[inline(always)]
    pub(crate) fn new_internal(domain: Domain) -> Self {
        Self::new_from(domain, Platform::detect())
    }

    /// [`HasherCore::new_internal`] on `platform` (the self-test's way in,
    /// as [`hash_serial_from`]).
    #[inline(always)]
    pub(crate) fn new_from(domain: Domain, platform: Platform) -> Self {
        Self { domain, chunk_state: ChunkState::new(domain.key(), 0, domain.flags(), platform), cv_stack: CvStack::new() }
    }

    /// Back to the empty message, with the same domain and platform.
    #[cfg(any(feature = "std", test))]
    pub(crate) fn reset(&mut self) -> &mut Self {
        self.chunk_state = ChunkState::new(self.domain.key(), 0, self.domain.flags(), self.chunk_state.platform());
        self.cv_stack.clear();
        self
    }

    // As described in push_cv() below, we do "lazy merging", delaying merges
    // until right before the next CV is about to be added (official BLAKE3's
    // reference implementation merges eagerly). Also, we aren't
    // always merging 1 chunk at a time. Instead, each CV might represent any
    // power-of-two number of chunks, as long as the smaller-above-larger stack
    // order is maintained. Instead of the "count the trailing 0-bits"
    // algorithm described in the spec (which assumes you're adding one chunk
    // at a time), we use a "count the total number of 1-bits" variant (which
    // doesn't assume that). The principle is the same: each CV that should
    // remain in the stack is represented by a 1-bit in the total number of
    // chunks (or bytes) so far.
    #[cfg(feature = "std")]
    fn merge_cv_stack(&mut self, chunk_counter: u64) {
        let post_merge_stack_len = hasher::ones(chunk_counter);
        let kernels = kernels_on(&self.domain, &self.chunk_state, self.chunk_state.platform(), None);
        self.cv_stack.merge(&kernels, post_merge_stack_len);
    }

    #[cfg(feature = "std")]
    fn push_cv(&mut self, new_cv: &CVBytes, chunk_counter: u64) {
        self.merge_cv_stack(chunk_counter);
        self.cv_stack.push(*new_cv);
    }

    /// [`update`](Hasher::update) with a held group (STAGE_LEN bytes from a
    /// multiple of STAGE_LEN, the core's chunk empty: Hasher::take_held,
    /// its one caller), whose subtree value `result` holds as
    /// the pool's task computes it: at chunk zero its two halves' values
    /// (the group may be the root's), elsewhere its own. The group is one
    /// whole subtree, so nothing of it reaches the chunk state.
    #[cfg(feature = "std")]
    pub fn push_group(&mut self, result: &[u8; BLOCK_LEN]) {
        const CHUNKS: u64 = (STAGE_LEN / CHUNK_LEN) as u64;
        check_input_room(self.count(), STAGE_LEN);
        let counter = self.chunk_state.chunk_counter();
        assert!(self.chunk_state.count() == 0 && counter % CHUNKS == 0, "a held group starts at a group boundary, the core's chunk empty");
        if counter == 0 {
            self.push_cv(result[..OUT_LEN].try_into().unwrap(), 0);
            self.push_cv(result[OUT_LEN..].try_into().unwrap(), CHUNKS / 2);
        } else {
            self.push_cv(result[..OUT_LEN].try_into().unwrap(), counter);
        }
        self.chunk_state.advance_chunks(CHUNKS);
    }

    /// What `gather_part` sends to the core (an input's whole groups, or
    /// the stage's bytes up to a group boundary), pooled only where the
    /// slice reaches the pool's threshold: shorter, it holds no subtree
    /// for the pool, and hashes exactly as the serial update hashes it.
    /// Kept pooled, it would lose what the hooks do only without a pool:
    /// the rows a whole group is cut into, and the SME2 turn over the
    /// whole slice. (64 open messages fed 16 KiB at a time took 9% to 15%
    /// longer through a pooled turn (update_each_multithreaded, a call
    /// since removed) than through update_each,
    /// on a Ryzen 9700X and an EPYC 9255: BLAKE3 #470.) The threshold is
    /// held against the slice, not against the update that completed it:
    /// a group gathered from shorter updates is the pool's where it
    /// reaches the threshold.
    ///
    /// The core's and not the Hasher's: the Hasher calls it with a view of
    /// its own stage, and a method on the Hasher would borrow the stage
    /// exclusively while the view reads it.
    #[inline(always)]
    pub(crate) fn update_groups(&mut self, groups: &[u8], pooled: Option<&Pool<'_>>) {
        self.update_pooled(groups, pooled.filter(|pool| groups.len() >= pool.min_len));
    }

    /// `update`'s loop; with `pooled`, whole subtrees at or above the
    /// callback's threshold may go to the pool.
    pub(crate) fn update_pooled(&mut self, input: &[u8], pooled: Option<&Pool<'_>>) -> &mut Self {
        check_input_room(self.count(), input.len());
        // The partial chunk first (hasher::fill), then whole subtrees and the
        // last chunk (hasher::feed), as src/hasher.rs describes.
        let fill_kernels = kernels_on(&self.domain, &self.chunk_state, self.chunk_state.platform(), pooled);
        let rest = hasher::fill(&fill_kernels, &mut self.chunk_state, &mut self.cv_stack, 0, input);
        // Pooled, the subtrees take the lock one at a time (children).
        let turn = platform::Sme2Turn::take(self.chunk_state.platform(), pooled.is_none() && rest.len() >= SME2_SIZED_LEN);
        let kernels = kernels_on(&self.domain, &self.chunk_state, turn.platform(), pooled);
        hasher::feed(&kernels, &mut self.chunk_state, &mut self.cv_stack, 0, rest);
        self
    }

    /// A Hasher's own kernels for its final output, with no pool.
    #[inline(always)]
    fn kernels(&self) -> HasherKernels<'_> {
        kernels_on(&self.domain, &self.chunk_state, self.chunk_state.platform(), None)
    }

    pub fn final_output(&self) -> Output {
        hasher::final_node(&self.kernels(), &self.chunk_state, &self.cv_stack)
    }

    /// The serial update: whole subtrees now, the last chunk kept.
    pub fn update(&mut self, input: &[u8]) -> &mut Self {
        self.update_pooled(input, None)
    }

    /// Bytes hashed into the core so far.
    pub fn count(&self) -> u64 {
        self.chunk_state.chunk_counter() * CHUNK_LEN as u64 + self.chunk_state.count() as u64
    }
}

// Don't derive(Debug), because the state may be secret.
impl fmt::Debug for Hasher {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Hasher").field("flags", &self.core.domain.flags()).field("platform", &self.core.chunk_state.platform()).finish()
    }
}

#[cfg(feature = "std")]
impl std::io::Write for Hasher {
    /// This is equivalent to [`update`](#method.update).
    #[inline]
    fn write(&mut self, input: &[u8]) -> std::io::Result<usize> {
        self.update(input);
        Ok(input.len())
    }

    #[inline]
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn update_with_one_wrong_half(wrong: usize) {
        crate::initialize();
        Hasher::new(Mode::Plain, None).update_with(&[0; STAGE_LEN], STAGE_LEN, &|tree, level, index, input| {
            let mut halves = tree.subtree_halves(level, index, input);
            halves[wrong] = halves[1 - wrong];
            halves
        });
    }

    #[test]
    #[should_panic(expected = "not the halves of level 4 index 0")]
    fn pool_rejects_a_wrong_left_half_alone() {
        update_with_one_wrong_half(0);
    }

    #[test]
    #[should_panic(expected = "not the halves of level 4 index 0")]
    fn pool_rejects_a_wrong_right_half_alone() {
        update_with_one_wrong_half(1);
    }

    /// A panic of the caller's callback comes out of `update_with`, both
    /// from a group the input holds whole and from one that a one-byte
    /// update completes in the stage; and a hasher replaced after it (as
    /// "After a panic" asks) hashes its next message. (A hook that caught
    /// the panic and hashed the subtree itself passed every test, once the
    /// turn's callback test went with `update_each_with`: BLAKE3 #476.)
    #[cfg(feature = "std")]
    #[test]
    fn a_callbacks_panic_comes_out_of_update_with() {
        crate::initialize();
        let mut input = vec![0u8; 2 * STAGE_LEN];
        crate::test::paint_test_input(&mut input);
        let panics = |_: &TreeHasher, _: u32, _: u64, _: &[u8]| -> [Cv; 2] { panic!("this callback panics") };
        for whole in [true, false] {
            let mut hasher = Hasher::new(Mode::Plain, None);
            let call = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
                if whole {
                    hasher.update_with(&input, STAGE_LEN, &panics);
                } else {
                    hasher.update(&input[..STAGE_LEN + 1]);
                    hasher.update(&input[STAGE_LEN + 1..2 * STAGE_LEN - 1]);
                    hasher.update_with(&input[2 * STAGE_LEN - 1..], STAGE_LEN, &panics);
                }
            }));
            let message = call.expect_err("the callback's panic did not come out");
            assert_eq!(message.downcast_ref::<&str>(), Some(&"this callback panics"), "whole: {whole}");
            let mut hasher = Hasher::new(Mode::Plain, None);
            hasher.update_with(&input, STAGE_LEN, &|tree, level, index, bytes| tree.subtree_halves(level, index, bytes));
            assert_eq!(*hasher.finalize().as_bytes(), crate::test::official::hash(Mode::Plain, &input), "whole: {whole}");
        }
    }

    /// Groups shorter than the pool's threshold go to the core as the
    /// serial update sends them: the callback is not asked, and their
    /// whole subtrees take the serial update's paths. Counted, since the
    /// digests are the same either way: the rows a whole group is cut
    /// into, and on SME2 the flat walks to one value. (Kept pooled, such
    /// an update cut no row, and no subtree of it walked flat to its one
    /// value: 64 open messages fed 16 KiB at a time took 9% to 15% longer
    /// through a pooled turn (`update_each_multithreaded`, a call since
    /// removed) than through `update_each`,
    /// BLAKE3 #470.) The threshold here
    /// is above every piece and every group the pieces fill in the stage;
    /// what is held to it is the group, not the piece that completes it
    /// (`a_staged_group_is_held_to_the_threshold_by_its_own_length`). At
    /// the threshold the callback is asked
    /// (`pool_halves_use_the_subtrees_nonzero_place`), and above it too.
    #[test]
    #[cfg(feature = "std")]
    fn an_update_below_the_pools_threshold_is_the_serial_update() {
        crate::initialize();
        let mut input = std::vec![0u8; 16 * STAGE_LEN];
        crate::test::paint_test_input(&mut input);
        let counts = || [crate::Counted::Row, crate::Counted::Flat].map(|what| crate::COUNTED.with(|c| c[what as usize].replace(0)));
        // The threshold of the library with threads: 512 KiB, above every
        // piece here and every group they fill in the stage.
        const THRESHOLD: usize = 32 * STAGE_LEN;
        let key = [0x52u8; 32];
        let platform = Platform::detect();
        let rows_here = (4..=hasher::ROW).contains(&platform.simd_degree());
        #[cfg(blake3_sme2)]
        let flat_here = matches!(platform, Platform::SME2);
        #[cfg(not(blake3_sme2))]
        let flat_here = false;
        for mode in [Mode::Plain, Mode::Keyed(&key), Mode::DeriveKey("below the pool's threshold")] {
            let (mut serial, mut pooled) = (Hasher::new(mode, None), Hasher::new(mode, None));
            let (mut fed, mut seen) = (0, [0u64; 2]);
            // A group; a group and a part; the rest of that group; a
            // group; then, from the 64 KiB boundary, four groups that are
            // one subtree; two groups and a part.
            for n in [STAGE_LEN, STAGE_LEN + 100, STAGE_LEN - 100, STAGE_LEN, 4 * STAGE_LEN, 2 * STAGE_LEN + 7] {
                let piece = &input[fed..fed + n];
                fed += n;
                counts();
                serial.update(piece);
                let want = counts();
                pooled.update_with(piece, THRESHOLD, &|_, _, _, _| panic!("an input below the threshold holds no subtree for the callback"));
                assert_eq!(counts(), want, "{fed} bytes in, a piece of {n}: the serial update's rows and flat walks");
                assert_eq!(pooled.finalize(), serial.finalize(), "{fed} bytes");
                seen = [seen[0] + want[0], seen[1] + want[1]];
            }
            // The comparison is of something wherever the kernels have
            // these paths: the pieces after the first group cut rows, and
            // the 64 KiB subtree walks flat.
            assert_eq!((seen[0] > 0, seen[1] > 0), (rows_here, flat_here), "rows and flat walks counted: {seen:?}");
            // Above the threshold the callback takes its subtrees: here
            // 64 KiB from the next 64 KiB boundary, one subtree, with a
            // threshold of a group.
            let calls = core::cell::Cell::new(0);
            assert_eq!(fed, 10 * STAGE_LEN + 7);
            pooled.update(&input[fed..12 * STAGE_LEN]);
            pooled.update_with(&input[12 * STAGE_LEN..], STAGE_LEN, &|tree, level, index, bytes| {
                calls.set(calls.get() + 1);
                tree.subtree_halves(level, index, bytes)
            });
            assert_eq!(calls.get(), 1, "one subtree of 64 KiB, above a threshold of 16 KiB");
            assert_eq!(*pooled.finalize().as_bytes(), crate::test::official::hash(mode, &input));
        }
    }

    /// The threshold is held against the slice that goes to the core, not
    /// against the update that completes it. A group gathered in the stage
    /// from shorter updates keeps the pool where it reaches the threshold,
    /// though its last piece is one byte: the callback is asked for it.
    /// One byte under the threshold it is the serial update's, cut into
    /// the rows the serial update cuts. (A filter on the update's own
    /// length would pass every other test here and take the staged groups
    /// from a caller of `update_with` whose threshold is a group; a gate
    /// one byte early would keep the pool for the second case, where the
    /// callback is silent either way, the hooks holding each subtree to
    /// the threshold again, and only the rows tell.)
    #[test]
    #[cfg(feature = "std")]
    fn a_staged_group_is_held_to_the_threshold_by_its_own_length() {
        crate::initialize();
        let input = painted();
        let input = &input[..2 * STAGE_LEN + 5];
        let asked = core::cell::RefCell::new(std::vec::Vec::new());
        let mut hasher = Hasher::new(Mode::Plain, None);
        hasher.update(&input[..STAGE_LEN - 1]);
        hasher.update_with(&input[STAGE_LEN - 1..STAGE_LEN], STAGE_LEN, &|tree, level, index, bytes| {
            asked.borrow_mut().push((level, index, bytes.len()));
            tree.subtree_halves(level, index, bytes)
        });
        assert_eq!(asked.borrow().as_slice(), &[(4, 0, STAGE_LEN)], "a group of the threshold's length, completed by one byte");
        hasher.update(&input[STAGE_LEN..]);
        assert_eq!(*hasher.finalize().as_bytes(), crate::test::official::hash(Mode::Plain, input));
        // One byte under the threshold, and the message's second group:
        // its first cuts no row on either path.
        let rows = || crate::COUNTED.with(|c| c[crate::Counted::Row as usize].replace(0));
        let (mut serial, mut pooled) = (Hasher::new(Mode::Plain, None), Hasher::new(Mode::Plain, None));
        serial.update(&input[..2 * STAGE_LEN - 1]);
        pooled.update(&input[..2 * STAGE_LEN - 1]);
        let last = &input[2 * STAGE_LEN - 1..2 * STAGE_LEN];
        rows();
        serial.update(last);
        let want = rows();
        pooled.update_with(last, STAGE_LEN + 1, &|_, _, _, _| panic!("a group one byte under the threshold holds no subtree for the callback"));
        assert_eq!(rows(), want, "a group one byte under the threshold: the serial update's rows");
        let rows_here = (4..=hasher::ROW).contains(&Platform::detect().simd_degree());
        assert_eq!(want > 0, rows_here, "the comparison is of something wherever the kernels cut rows");
        assert_eq!(pooled.finalize(), serial.finalize());
        assert_eq!(*pooled.finalize().as_bytes(), crate::test::official::hash(Mode::Plain, &input[..2 * STAGE_LEN]));
    }

    #[test]
    fn pool_halves_use_the_subtrees_nonzero_place() {
        let calls = core::cell::Cell::new(0);
        crate::initialize();
        let input = painted();
        let input = &input[..2 * STAGE_LEN];
        let key = [3; KEY_LEN];
        let mut hasher = Hasher::new(Mode::Keyed(&key), None);
        hasher.update(&input[..STAGE_LEN]);
        hasher.update_with(&input[STAGE_LEN..], STAGE_LEN, &|tree, level, index, input| {
            assert_eq!((level, index, input.len()), (4, 1, STAGE_LEN));
            calls.set(calls.get() + 1);
            tree.subtree_halves(level, index, input)
        });
        assert_eq!(calls.get(), 1);
        assert_eq!(hasher.finalize().as_bytes(), blake3::keyed_hash(&key, input).as_bytes());
    }

    fn painted() -> std::vec::Vec<u8> {
        let mut input = std::vec![0u8; 3 * STAGE_LEN];
        crate::test::paint_test_input(&mut input);
        input
    }

    /// The length hint is a scheduling hint and never part of the hash:
    /// every hint, accurate, short, long, zero or absent, gives the
    /// one-shot digest, in every mode, through `new` and through `reset`.
    /// A Hasher that one day uses the hint keeps this contract or fails here.
    #[test]
    fn test_the_length_hint_never_changes_the_digest() {
        let input = painted();
        let key = [3u8; 32];
        for mode in [Mode::Plain, Mode::Keyed(&key), Mode::DeriveKey("hint test")] {
            for len in [0, 1, CHUNK_LEN, CHUNK_LEN + 1, STAGE_LEN + 7, input.len()] {
                let message = &input[..len];
                let want = crate::convenience::hash_with(mode, message);
                let hints = [None, Some(0), Some(1), Some(len as u64), Some(len as u64 / 2), Some(2 * len as u64 + 1), Some(u64::MAX)];
                let mut reused = Hasher::new(mode, Some(7));
                for hint in hints {
                    let mut fresh = Hasher::new(mode, hint);
                    for piece in message.chunks(1000) {
                        fresh.update(piece);
                    }
                    assert_eq!(fresh.finalize(), want, "{len} bytes, hint {hint:?}");
                    reused.reset();
                    reused.update(message);
                    assert_eq!(reused.clone().finalize(), want, "{len} bytes, hint {hint:?}, after reset");
                }
            }
        }
    }

    /// In-place checkpointing overwrites the live stage prefix only. A
    /// full-capacity copy gives identical digests but changes the canary
    /// in unused destination storage, making the performance invariant
    /// deterministic rather than a timing assertion.
    #[test]
    #[cfg(feature = "std")]
    fn clone_from_copies_only_the_live_stage_prefix() {
        crate::initialize();
        let input = painted();
        let key = [0x42u8; 32];
        let other_key = [0x73u8; 32];
        for mode in [Mode::Plain, Mode::Keyed(&key), Mode::DeriveKey("in-place prefix checkpoint")] {
            for len in [0usize, 64, 1024, 1025, 2048, STAGE_LEN - 1, STAGE_LEN, 2 * STAGE_LEN + 17] {
                let mut source = Hasher::new(mode, None);
                for part in input[..len].chunks(17) {
                    source.update(part);
                }
                // The live part of the stage: the staged bytes, and with any
                // staged the whole first block (zero past a message that
                // ends inside it); nothing of an empty stage.
                let live = if source.stage.len() == 0 { 0 } else { source.stage.len().max(BLOCK_LEN) };
                source.stage.fill_from_for_test(live, 0xbb);
                let mut copy = Hasher::new(Mode::Keyed(&other_key), None);
                copy.update(&input[..2 * STAGE_LEN]);
                copy.stage.fill_from_for_test(0, 0xaa);
                copy.clone_from(&source);
                let expected = crate::test::official::hash(mode, &input[..len]);
                assert_eq!(copy.count(), source.count());
                assert_eq!(*copy.clone().finalize().as_bytes(), expected);
                assert_eq!(copy.stage.as_bytes(), source.stage.as_bytes());
                // Sound: every byte in `live..STAGE_LEN` was painted `0xaa` on `copy`,
                // and `source.stage.len()..live` was zero-padded for nonempty stages.
                assert!((live..STAGE_LEN).all(|i| unsafe { copy.stage.byte_for_test(i) } == 0xaa), "unused stage copied at len={len}");
                if source.core.count() == 0 {
                    assert!(
                        (source.stage.len()..live).all(|i| unsafe { copy.stage.byte_for_test(i) } == 0),
                        "the first block is zero past a message whole in the stage, len={len}"
                    );
                }
                copy.update(&input[len..len + 129]);
                assert_eq!(*copy.clone().finalize().as_bytes(), crate::test::official::hash(mode, &input[..len + 129]));
                assert_eq!(*source.finalize().as_bytes(), expected, "checkpoint source is untouched");
                let mut out = [0u8; 129];
                copy.finalize_xof().fill(&mut out);
                assert_eq!(out.as_slice(), crate::test::official::xof(mode, &input[..len + 129], 0, out.len()));
            }
        }
    }

    /// A whole group from a 16 KiB boundary goes to the core unstaged;
    /// shorter updates, a fresh hasher's first chunk among them, stage up
    /// to the next boundary, where the stage goes to the core. Staging
    /// gives the same digest either way, so only `staged` tells (BLAKE3
    /// #219).
    #[test]
    fn test_update_stages_exactly_what_it_must() {
        let input = painted();
        let mut h = Hasher::new(Mode::Plain, None);
        for (end, staged) in [(CHUNK_LEN, CHUNK_LEN), (CHUNK_LEN + 100, CHUNK_LEN + 100), (STAGE_LEN, 0), (2 * STAGE_LEN, 0)] {
            let from = h.count() as usize;
            h.update(&input[from..end]);
            assert_eq!(h.stage.len(), staged, "after {end} bytes");
        }
        assert_eq!(h.finalize(), crate::convenience::hash(&input[..2 * STAGE_LEN]));
    }

    /// `finalize` and `finalize_xof` leave the Hasher as it was: asked
    /// twice they answer alike, and updates after them extend the message,
    /// as with official BLAKE3's Hasher. At every length class of the
    /// first part (a block, a chunk, the stage, beyond it), fed whole and
    /// in pieces of 7 bytes, in every mode.
    #[test]
    #[cfg(feature = "std")]
    fn finalize_leaves_the_hasher_as_it_was_and_later_updates_extend_the_message() {
        crate::initialize();
        let input = painted();
        let key = [0x5a; 32];
        for mode in [Mode::Plain, Mode::Keyed(&key), Mode::DeriveKey("finalize, then more")] {
            for first in [0usize, 1, 17, 63, 64, 65, 1000, 1024, 1025, STAGE_LEN - 1, STAGE_LEN, STAGE_LEN + 1, 2 * STAGE_LEN - 300] {
                for more in [0usize, 1, 64, 1500, STAGE_LEN] {
                    for piece in [usize::MAX, 7] {
                        let what = std::format!("first={first} more={more} piece={piece}");
                        let mut h = Hasher::new(mode, None);
                        for part in input[..first].chunks(piece.min(first.max(1))) {
                            h.update(part);
                        }
                        let state = (h.stage.len(), h.core.count());
                        let want = crate::test::official::hash(mode, &input[..first]);
                        assert_eq!(*h.finalize().as_bytes(), want, "{what}");
                        let mut out = [0u8; 100];
                        h.finalize_xof().fill(&mut out);
                        assert_eq!(out.as_slice(), crate::test::official::xof(mode, &input[..first], 0, out.len()), "{what}");
                        assert_eq!(*h.finalize().as_bytes(), want, "asked again, {what}");
                        assert_eq!((h.stage.len(), h.core.count()), state, "untouched, {what}");
                        h.update(&input[first..first + more]);
                        assert_eq!(*h.finalize().as_bytes(), crate::test::official::hash(mode, &input[..first + more]), "extended, {what}");
                        h.finalize_xof().fill(&mut out);
                        assert_eq!(out.as_slice(), crate::test::official::xof(mode, &input[..first + more], 0, out.len()), "extended, {what}");
                    }
                }
            }
        }
    }

    /// A message of a block or less is hashed from the stage's first block
    /// where it lies, so the block must be zero past the message whatever
    /// the Hasher held before (`first_block`): after a longer message and a
    /// reset (`finalize_each`), after `clone_from` over a longer message,
    /// and when its bytes arrive by the path that skips `update`'s own
    /// first block (`gather`, as `update_with` calls it).
    #[test]
    #[cfg(feature = "std")]
    fn a_block_or_less_is_padded_whatever_the_hasher_held_before() {
        crate::initialize();
        let input = painted();
        let key = [0x77; 32];
        for mode in [Mode::Plain, Mode::Keyed(&key)] {
            for long in [0usize, 30, 64, 100, 5000, STAGE_LEN, STAGE_LEN + 300] {
                for short in [0usize, 1, 10, 31, 63, 64] {
                    let what = std::format!("long={long} short={short}");
                    let message = &input[1000..1000 + short];
                    let want = crate::test::official::hash(mode, message);
                    // Reset by finalize_each, then the short message, whole and in two parts.
                    for split in [0, short / 2] {
                        let mut hashers = std::vec![Hasher::new(mode, None)];
                        hashers[0].update(&input[..long]);
                        let mut out = [[0u8; OUT_LEN]];
                        crate::each::finalize_each(&mut hashers, &[0], &mut out);
                        assert_eq!(out[0], crate::test::official::hash(mode, &input[..long]), "{what}");
                        hashers[0].update(&message[..split]);
                        hashers[0].update(&message[split..]);
                        assert_eq!(*hashers[0].finalize().as_bytes(), want, "after a reset, {what} split={split}");
                        crate::each::finalize_each(&mut hashers, &[0], &mut out);
                        assert_eq!(out[0], want, "finished with others after a reset, {what} split={split}");
                    }
                    // clone_from over a Hasher that held the longer message.
                    let mut source = Hasher::new(mode, None);
                    source.update(message);
                    let mut copy = Hasher::new(mode, None);
                    copy.update(&input[..long]);
                    copy.clone_from(&source);
                    assert_eq!(*copy.finalize().as_bytes(), want, "after clone_from, {what}");
                    assert_eq!(*source.clone().finalize().as_bytes(), want, "a clone, {what}");
                    // The bytes by `gather`, in two parts.
                    let mut gathered = Hasher::new(mode, None);
                    gathered.gather(&message[..short / 2], None);
                    gathered.gather(&message[short / 2..], None);
                    assert_eq!(*gathered.finalize().as_bytes(), want, "gathered, {what}");
                }
            }
        }
    }

    /// The assembly entry points `f` runs on this thread.
    #[cfg(feature = "std")]
    fn entries_run(f: &mut dyn FnMut()) -> std::collections::BTreeSet<usize> {
        crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
        f();
        crate::RAN_ENTRIES.with(|r| r.borrow().clone())
    }

    /// A message whole in the stage is hashed by the kernels the one-shot
    /// call runs, and no others: by `finalize`, and by `finalize_each` when
    /// it finishes a hasher alone. Another path gives the same digest with
    /// more work (a copy of the core, a chunk state's block-by-block
    /// compression), so the kernels that ran are what tells (BLAKE3 #219).
    /// The control: a message a turn left in its chunk state on x86-64
    /// finishes by other kernels, so the record does tell.
    #[test]
    #[cfg(feature = "std")]
    fn a_message_whole_in_the_stage_runs_the_one_shot_calls_kernels() {
        crate::initialize();
        let input = painted();
        let key = [0x21; 32];
        for mode in [Mode::Plain, Mode::Keyed(&key)] {
            for len in [0usize, 1, 64, 65, 1000, 1024, 1025, 5000, STAGE_LEN - 1] {
                let message = &input[..len];
                let one_shot = entries_run(&mut || {
                    crate::convenience::hash_with(mode, message);
                });
                // The record names the kernels of a chunk or less on every
                // machine; of longer inputs, only some machines' kernels.
                assert!(len > CHUNK_LEN || !one_shot.is_empty(), "a kernel ran, len={len}");
                let mut hashers = std::vec![Hasher::new(mode, None)];
                assert!(
                    entries_run(&mut || {
                        hashers[0].update(message);
                    })
                    .is_empty(),
                    "staging runs no kernel, len={len}"
                );
                assert_eq!(
                    entries_run(&mut || {
                        hashers[0].finalize();
                    }),
                    one_shot,
                    "finalize, len={len}"
                );
                let mut out = [[0u8; OUT_LEN]];
                assert_eq!(entries_run(&mut || crate::each::finalize_each(&mut hashers, &[0], &mut out)), one_shot, "finalize_each alone, len={len}");
                #[cfg(not(blake3_neon))]
                if (65..=CHUNK_LEN).contains(&len) {
                    let mut turn = Hasher::new(mode, None);
                    turn.gather_part(message, None, true);
                    assert_ne!(
                        entries_run(&mut || {
                            turn.finalize();
                        }),
                        one_shot,
                        "the control: from a chunk state, len={len}"
                    );
                }
            }
        }
    }

    /// A block or less into an empty stage is stored as one padded block
    /// (`update`'s own path, which the kernel's loads are served from);
    /// more, or into a stage that holds bytes, takes the longer path. The
    /// digest is the same either way, so the count tells (BLAKE3 #219).
    #[test]
    #[cfg(all(feature = "std", blake3_sse41_ffi))]
    fn an_update_of_a_block_or_less_into_an_empty_stage_is_stored_padded() {
        crate::initialize();
        let input = painted();
        let padded = |hasher: &mut Hasher, bytes: &[u8]| {
            crate::COUNTED.with(|c| c[crate::Counted::PaddedFirstBlock as usize].set(0));
            hasher.update(bytes);
            crate::COUNTED.with(|c| c[crate::Counted::PaddedFirstBlock as usize].get())
        };
        for (len, expected) in [(0usize, 1), (1, 1), (63, 1), (64, 1), (65, 0), (1024, 0), (STAGE_LEN, 0)] {
            let mut h = Hasher::new(Mode::Plain, None);
            assert_eq!(padded(&mut h, &input[..len]), expected, "a fresh hasher, {len} bytes");
        }
        // Into a stage that holds bytes: the longer path. At the message's
        // next 16 KiB boundary the stage is empty again.
        let mut h = Hasher::new(Mode::Plain, None);
        assert_eq!(padded(&mut h, &input[..10]), 1);
        assert_eq!(padded(&mut h, &input[10..20]), 0, "the stage holds bytes");
        assert_eq!(padded(&mut h, &input[20..STAGE_LEN]), 0);
        assert_eq!(h.stage.len(), 0, "the stage went to the core");
        assert_eq!(padded(&mut h, &input[STAGE_LEN..STAGE_LEN + 64]), 1, "an empty stage again");
        assert_eq!(h.finalize(), crate::convenience::hash(&input[..STAGE_LEN + 64]));
    }

    /// What the stack's rows and the tail's own core are for, counted
    /// (the digests are the same without them): a message's first group
    /// goes to the core as its two halves, every later whole group or
    /// more as a row; `finalize` with bytes staged after whole groups
    /// hashes them apart and reads the stack, the Hasher's core not
    /// copied, and the Hasher goes on to the same digests after. In every
    /// mode, at lengths around the groups.
    #[test]
    #[cfg(feature = "std")]
    fn later_groups_are_rows_and_finalize_hashes_the_tail_apart() {
        crate::initialize();
        let mut input = std::vec![0u8; 8 * STAGE_LEN];
        crate::test::paint_test_input(&mut input);
        let count = |what: crate::Counted| crate::COUNTED.with(|c| c[what as usize].replace(0));
        let key = [0x51u8; 32];
        let rows_here = (4..=hasher::ROW).contains(&Platform::detect().simd_degree());
        for mode in [Mode::Plain, Mode::Keyed(&key), Mode::DeriveKey("rows and tails")] {
            let mut h = Hasher::new(mode, None);
            let (mut fed, mut rows) = (0, 0);
            // Pieces: a group; a group and a part; the rest of that group;
            // two groups and a part from the third boundary (the fourth
            // group, then the fifth: two subtrees); the rest of a group.
            for (n, new_rows) in [(STAGE_LEN, 0), (STAGE_LEN + 100, 1), (STAGE_LEN - 100, 1), (2 * STAGE_LEN + 7, 2), (STAGE_LEN - 7, 1)] {
                count(crate::Counted::Row);
                h.update(&input[fed..fed + n]);
                fed += n;
                rows += new_rows;
                assert_eq!(count(crate::Counted::Row), if rows_here { new_rows } else { 0 }, "{fed} bytes in: the update's subtrees after the first");
                count(crate::Counted::TailApart);
                assert_eq!(*h.finalize().as_bytes(), crate::test::official::hash(mode, &input[..fed]), "{fed} bytes");
                let apart = u64::from(h.stage.len() > 0);
                assert_eq!(count(crate::Counted::TailApart), apart, "{fed} bytes: {} staged", h.stage.len());
                let mut xof = [0u8; 100];
                h.finalize_xof().fill(&mut xof);
                assert_eq!(xof[..32], *h.finalize().as_bytes());
            }
            let _ = rows;
        }
        // A short piece staged into a stage that held another message:
        // the first block is the piece and zeros (`stage`), whatever was
        // there, on the path that copies (gather_part), which a first
        // update of a block or less on x86-64 does not take.
        let mut h = Hasher::new(Mode::Plain, None);
        h.update(&input[..3000]);
        h.reset();
        h.gather_part(&input[..5], None, false);
        assert_eq!((h.stage.len(), h.core.count()), (5, 0));
        assert_eq!(*h.finalize().as_bytes(), crate::test::official::hash(Mode::Plain, &input[..5]));
        h.gather_part(&input[5..70], None, false);
        assert_eq!(*h.finalize().as_bytes(), crate::test::official::hash(Mode::Plain, &input[..70]));
    }

    /// After a turn of many hashers left a message's first chunk in the
    /// core (x86-64) and later bytes in the stage, `finalize` takes the
    /// copy of the core: the digest is right, and no tail is hashed apart.
    #[test]
    #[cfg(feature = "std")]
    fn finalize_after_a_turn_that_left_a_chunk_in_the_core() {
        crate::initialize();
        let input = painted();
        let mut hashers = [Hasher::new(Mode::Plain, None), Hasher::new(Mode::Plain, None)];
        crate::throughput::update_each(&mut hashers, &[(0, &input[..700]), (1, &input[..64])]);
        crate::throughput::update_each(&mut hashers, &[(0, &input[700..2500]), (1, &input[64..65])]);
        crate::COUNTED.with(|c| c[crate::Counted::TailApart as usize].set(0));
        for (h, len) in hashers.iter().zip([2500, 65]) {
            assert_eq!(*h.finalize().as_bytes(), crate::test::official::hash(Mode::Plain, &input[..len]), "{len} bytes");
        }
        if !cfg!(blake3_neon) {
            assert!(hashers[0].core.chunk_state.count() > 0 && hashers[0].stage.len() > 0, "a chunk in the core, bytes in the stage");
            assert_eq!(crate::COUNTED.with(|c| c[crate::Counted::TailApart as usize].get()), 0);
        }
        // The same state made by hand, on every target: part of a chunk
        // in the core, or whole chunks short of a group, and bytes staged.
        for in_core in [700, 3 * CHUNK_LEN, STAGE_LEN + CHUNK_LEN] {
            let mut h = Hasher::new(Mode::Plain, None);
            h.core.update(&input[..in_core]);
            h.stage(&input[in_core..in_core + 900]);
            // The count with bytes in the core's chunk state. On AArch64
            // an update from offset zero leaves none there (it stages its
            // ragged end and sends the core whole groups), so no test of
            // such updates tells a count that subtracts them. This state,
            // made by hand, does.
            assert_eq!(h.count(), (in_core + 900) as u64, "{in_core} bytes in the core: the count");
            crate::COUNTED.with(|c| c[crate::Counted::TailApart as usize].set(0));
            assert_eq!(*h.finalize().as_bytes(), crate::test::official::hash(Mode::Plain, &input[..in_core + 900]), "{in_core} bytes in the core");
            assert_eq!(crate::COUNTED.with(|c| c[crate::Counted::TailApart as usize].get()), 0, "{in_core} bytes in the core: the copy");
        }
    }

    /// An update of 16 KiB or more from a boundary gives the core its
    /// whole groups and stages what follows them, so that later pieces
    /// start on a boundary again: three pieces of a third of 64 KiB, then
    /// pieces that end off and on the boundaries. Sent whole, such pieces
    /// hash to the same digest in narrower subtrees, at half the speed
    /// (BLAKE3 #393), so only `staged` tells (BLAKE3 #219).
    #[test]
    fn test_an_updates_whole_groups_skip_the_stage_and_its_end_is_staged() {
        let mut input = std::vec![0u8; 65536 + 6 * STAGE_LEN];
        crate::test::paint_test_input(&mut input);
        let mut h = Hasher::new(Mode::Plain, None);
        let third = 65536 / 3;
        // After each update: the bytes staged, and the copies into the
        // stage the update made (one for a ragged end; two where it first
        // fills the stage to the boundary; none for whole groups alone).
        for (end, staged, copies) in [
            (third, third - STAGE_LEN, 1),
            (2 * third, 2 * third - 2 * STAGE_LEN, 2),
            (65536, 0, 1),
            (65536 + STAGE_LEN + 5, 5, 1),
            (65536 + 2 * STAGE_LEN, 0, 1),
            (65536 + 2 * STAGE_LEN + 100, 100, 1),
            (65536 + 4 * STAGE_LEN + 100, 100, 2),
            (65536 + 5 * STAGE_LEN, 0, 1),
            (65536 + 6 * STAGE_LEN, 0, 0),
        ] {
            let from = h.count() as usize;
            #[cfg(feature = "std")]
            crate::COUNTED.with(|c| c[crate::Counted::Staged as usize].set(0));
            h.update(&input[from..end]);
            #[cfg(feature = "std")]
            assert_eq!(crate::COUNTED.with(|c| c[crate::Counted::Staged as usize].get()), copies, "copies into the stage, after {end} bytes");
            let _ = copies;
            assert_eq!(h.stage.len(), staged, "after {end} bytes");
            assert_eq!(h.finalize(), crate::convenience::hash(&input[..end]), "after {end} bytes");
        }
    }

    /// Construct a hasher whose chunk counter sits near the 2^64 - 1 byte
    /// BLAKE3 domain limit (without streaming 16 EiB) so unit tests can
    /// verify both acceptance of the final legal byte and refusal of the
    /// first byte past 2^64 - 1 on every ingress path.
    fn near_limit(remaining_bytes: u64) -> Hasher {
        assert!(remaining_bytes >= 1 && remaining_bytes <= CHUNK_LEN as u64 - 1);
        let mut h = Hasher::new(Mode::Plain, None);
        // Chunk counter (2^54 - 1) leaves 1023 bytes in the final legal chunk.
        h.core.chunk_state.set_chunk_counter((1u64 << 54) - 1);
        let fill = (CHUNK_LEN as u64 - 1 - remaining_bytes) as usize;
        if fill > 0 {
            h.core.chunk_state.update(&std::vec![0u8; fill]);
        }
        assert_eq!(u64::MAX - h.count(), remaining_bytes);
        h
    }

    #[test]
    fn accepts_last_legal_byte_at_u64_max() {
        crate::initialize();
        let mut h = near_limit(10);
        h.update(&[1u8; 9]);
        h.update(&[2u8; 1]);
        assert_eq!(h.count(), u64::MAX);
        // Zero-length update at u64::MAX is still valid.
        h.update(&[]);
        assert_eq!(h.count(), u64::MAX);
    }

    #[test]
    #[should_panic(expected = "BLAKE3 messages contain at most 2^64 - 1 bytes")]
    fn refuses_byte_past_u64_max_on_short_fast_path() {
        crate::initialize();
        let mut h = near_limit(1);
        h.update(&[0u8; 2]);
    }

    #[test]
    #[should_panic(expected = "BLAKE3 messages contain at most 2^64 - 1 bytes")]
    fn refuses_byte_past_u64_max_after_filling_to_limit() {
        crate::initialize();
        let mut h = near_limit(1);
        h.update(&[0u8; 1]);
        assert_eq!(h.count(), u64::MAX);
        h.update(&[0u8; 1]);
    }

    #[cfg(feature = "std")]
    #[test]
    #[should_panic(expected = "BLAKE3 messages contain at most 2^64 - 1 bytes")]
    fn push_group_refuses_group_past_u64_max() {
        crate::initialize();
        let mut h = Hasher::new(Mode::Plain, None);
        // Counter at the last 16-chunk boundary before 2^54: adding STAGE_LEN
        // (16 KiB) would reach 2^64 bytes exactly.
        h.core.chunk_state.set_chunk_counter((1u64 << 54) - (STAGE_LEN / CHUNK_LEN) as u64);
        h.core.push_group(&[0u8; BLOCK_LEN]);
    }

    #[test]
    #[should_panic(expected = "staged bytes exceed STAGE_LEN")]
    fn stage_append_checks_capacity() {
        crate::initialize();
        let mut h = Hasher::new(Mode::Plain, None);
        h.stage(&[0u8; STAGE_LEN]);
        h.stage(&[0u8; 1]);
    }

    /// With `hold`, a stage that fills a whole group from the message's
    /// start is kept (held_group) and the rest of the input handed back,
    /// for the caller to hash the group beside other hashers' groups.
    /// Holding is std's (update_each): without it held_group is None.
    #[test]
    #[cfg(feature = "std")]
    fn test_gather_part_holds_a_whole_group() {
        let input = painted();
        let mut h = Hasher::new(Mode::Plain, None);
        h.update(&input[..STAGE_LEN]);
        h.update(&input[STAGE_LEN..STAGE_LEN + 100]);
        let rest = h.gather_part(&input[STAGE_LEN + 100..2 * STAGE_LEN + 50], None, true);
        assert_eq!(rest.len(), 50, "the rest handed back");
        let (counter, bytes) = h.held_group().expect("a whole group held");
        assert_eq!((counter, &bytes[..]), ((STAGE_LEN / CHUNK_LEN) as u64, &input[STAGE_LEN..2 * STAGE_LEN]));
    }

    #[cfg(all(blake3_sme2, feature = "std"))]
    #[test]
    fn set_platform_for_test_updates_chunk_state_platform() {
        crate::initialize();
        let mut h = Hasher::new(Mode::Plain, None);
        h.set_platform_for_test(Platform::SME2);
        assert_eq!(h.platform(), Platform::SME2);
        h.set_platform_for_test(Platform::NEON);
        assert_eq!(h.platform(), Platform::NEON);
    }
}
