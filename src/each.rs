//! [`hash_each`]: a collection of messages of any lengths, each hashed,
//! with short messages' chunks side by side in the SIMD lanes.
//!
//! Chunk k of every message has chunk counter k, so the full chunks k of
//! different messages hash in one call of the many-inputs kernel, at counter
//! k for every lane. Messages of up to a chunk share lanes by their block
//! counts (`hash_by_blocks`); NEON with SHA-3 includes their last blocks in
//! the same pass. Messages of 16 chunks or more fill the lanes alone; a
//! longer message's last partial chunk hashes on its own. Each message's
//! chunk chaining values then merge into its root as BLAKE3's tree places them.

use crate::many::TABLE;
use crate::platform::Platform;
use crate::throughput::Block;
use crate::{CHUNK_END, CHUNK_LEN, CHUNK_START, CVBytes, CVWords, ChunkState, Hash, Hasher, IncrementCounter, Mode, OUT_LEN};

// Many messages arriving at once, such as a server's uploads, each with
// its own Hasher: one call takes the pieces that have arrived for all of
// them, which hash faster together than one Hasher::update each.

/// Feed each piece to its hasher: `(i, bytes)` does
/// `hashers[i].update(bytes)`, each hasher's pieces in their order in
/// `pieces`. Call it once with all the pieces your program has in hand
/// (one turn of its event loop): never slower than one
/// [`update`](Hasher::update) each: many messages arriving at once, such
/// as a server's uploads, each with its own [`Hasher`]. Requires each `i < hashers.len()`:
/// a turn that panics on one that is not leaves its hashers as
/// [after a panic](Hasher#after-a-panic).
/// Finish the messages with [`finalize_each`]: plain messages of up to 1024
/// bytes wait whole in their stage (on both x86-64 and AArch64), so those
/// that finish together hash side by side there. Because a staged plain
/// first chunk has not yet compressed into `ChunkState`, calling a
/// non-consuming `Hasher::finalize()` mid-chunk rehashes that staged prefix
/// afresh; callers that finish each message once via `finalize_each` avoid
/// that cost.
///
/// # Or the Pipeline
///
/// A program with threads to give, whose messages keep arriving, has a
/// faster way in the crate `blake3-servil` (this library with its thread
/// pool): its `throughput::Pipeline` splits across worker cores (historically
/// beating pre-batching `update_each` on 1,024 open 1 KiB messages on Ryzen
/// 9700X / EPYC 9255, November 22, 2026, BLAKE3 #470, and taking a third of
/// the time for 64 open messages of 1 MiB). It holds a room of 64 KiB for
/// each open message. This call stays the right one:
///
/// - on one thread;
/// - for more open messages than the program has 64 KiB each for (a
///   `Hasher` is 26 KiB, and touches only what it uses);
/// - where a message's running state is wanted: a clone, a `finalize`
///   part-way;
/// - for a small burst after a gap, about 256 KiB a turn, where the
///   Pipeline's workers must first wake: there it read from 6% faster to
///   20% slower than this call on the same two machines.
///
/// # Key-dependent timing
///
/// Hashers may use different secret keys. Grouping, kernel selection, and
/// temporary storage sizes are independent of key values and key equality.
/// See the [key-independence contract](crate#key-dependent-timing).
///
/// ```
/// # blake3_servil_core::initialize();
/// use blake3_servil_core::{convenience::Hasher, throughput};
/// let mut hashers = vec![Hasher::new(blake3_servil_core::Mode::Plain, None), Hasher::new(blake3_servil_core::Mode::Plain, None)];
/// throughput::update_each(&mut hashers, &[(0, b"ab"), (1, b"xyz"), (0, b"c")]);
/// let mut out = [[0u8; 32]; 2];
/// throughput::finalize_each(&mut hashers, &[0, 1], &mut out);
/// let hash = blake3_servil_core::convenience::hash;
/// assert_eq!((&out[0], &out[1]), (hash(b"abc").as_bytes(), hash(b"xyz").as_bytes()));
/// ```
pub fn update_each(hashers: &mut [Hasher], pieces: &[(usize, &[u8])]) {
    update_turn(hashers, pieces);
}

/// RAII guard for `finalize_each`: records how many hashers in `which[..count]`
/// have been marked `finishing`, and clears those marks on unwind (whether a
/// bad/duplicate index fails preflight or an internal panic occurs mid-finish).
struct Marked<'a> {
    hashers: &'a mut [Hasher],
    which: &'a [usize],
    count: usize,
    active: bool,
}

impl Drop for Marked<'_> {
    fn drop(&mut self) {
        if self.active {
            for &i in &self.which[..self.count] {
                self.hashers[i].clear_finishing();
            }
        }
    }
}

/// The hash of each message `which` names: `out[k]` is what
/// `hashers[which[k]].finalize()` would give, and that hasher is then
/// reset for its next message (no length hint). Requires
/// `out.len() == which.len()`, each index `< hashers.len()`, and distinct
/// indices: it panics on an index named twice, in every build. A call
/// that is refused hashes nothing and leaves every hasher as it was.
/// Messages that finish in one call hash side by side where the CPU has
/// the lanes (with at least two eligible hashers in a 128-hasher window):
/// on x86-64, staged plain 1 KiB chunks via `hash_many_rounded` (ceiling
/// 32 lanes on Zen 5 / 16 on AVX-512 / 8 on AVX2 / 4 on SSE4.1), staged
/// plain multi-block `65..=1023 B` messages on AVX2/AVX-512 via
/// `hash_by_blocks` (equal-prefix-length blocks batched in SIMD, each
/// message's final block compressed individually; SSE4.1 finishes staged
/// `< 1 KiB` messages individually), and 16-lane `output16` for every 16
/// eligible counter-zero final blocks packed from remaining hashers on Linux
/// AVX-512; on AArch64, staged plain messages of up to 1024 bytes via
/// `hash_by_blocks` and counter-zero final blocks via NEON `output8`.
///
/// # Key-dependent timing
///
/// Selected hashers may use different secret keys. Key values and key equality
/// select neither the finalization path nor its amount of work.
/// See the [key-independence contract](crate#key-dependent-timing).
pub fn finalize_each(hashers: &mut [Hasher], which: &[usize], out: &mut [[u8; OUT_LEN]]) {
    assert_eq!(which.len(), out.len(), "one digest per message named");
    let mut marked = Marked { hashers, which, count: 0, active: true };
    for &i in which {
        assert!(i < marked.hashers.len(), "finalize_each names hasher {i} of {}", marked.hashers.len());
        assert!(!marked.hashers[i].mark_finishing(), "each finalized index is distinct");
        marked.count += 1;
    }
    let hashers = &mut *marked.hashers;
    if which.is_empty() {
        marked.active = false;
        return;
    }
    if which.len() == 1 {
        let i = which[0];
        out[0] = *hashers[i].take_root_hash().as_bytes();
        hashers[i].reset();
        marked.active = false;
        return;
    }
    #[cfg(blake3_sse41_ffi)]
    {
        crate::ffi_partial::finalize_each(hashers, which, out);
        marked.active = false;
        return;
    }
    // AArch64: partition each 128-hasher slice in a single pass into plain
    // staged first chunks (`hash_by_blocks` when >= 2), remaining counter-zero
    // single-chunk hashers (keyed/derived/mixed `<= 64 B` staged or ChunkState-
    // resident -> NEON `output8` when >= 8, #336), and general hashers. Key
    // values never select grouping or dispatch.
    #[cfg(blake3_neon)]
    {
        const FINALIZE_BATCH: usize = 128;
        const { assert!(FINALIZE_BATCH <= u8::MAX as usize) };
        let platform = Platform::detect();
        for (w_chunk, o_chunk) in which.chunks(FINALIZE_BATCH).zip(out.chunks_mut(FINALIZE_BATCH)) {
            let mut plain = crate::stack_vec::StackVec::<u8, FINALIZE_BATCH>::new();
            let mut keyed = crate::stack_vec::StackVec::<u8, FINALIZE_BATCH>::new();
            let mut rest = crate::stack_vec::StackVec::<u8, FINALIZE_BATCH>::new();
            for (k, &i) in w_chunk.iter().enumerate() {
                let k = k.to_le_bytes()[0];
                if hashers[i].is_plain_single_chunk_staged() {
                    plain.push(k);
                } else if crate::ffi_output::input(&hashers[i]).is_some() {
                    keyed.push(k);
                } else {
                    rest.push(k);
                }
            }
            if plain.len() >= 2 {
                for part in plain.as_slice().chunks(BY_BLOCKS_AT_ONCE) {
                    let bytes: crate::stack_vec::StackVec<&[u8], BY_BLOCKS_AT_ONCE> =
                        part.iter().map(|&k| hashers[w_chunk[usize::from(k)]].staged_bytes()).collect();
                    let mut digests = [[0u8; OUT_LEN]; BY_BLOCKS_AT_ONCE];
                    hash_by_blocks(&bytes, crate::IV, 0, &mut digests[..part.len()], platform);
                    for (&k, digest) in part.iter().zip(&digests) {
                        o_chunk[usize::from(k)] = *digest;
                    }
                }
            } else {
                for &k in plain.as_slice() {
                    let k = usize::from(k);
                    o_chunk[k] = *hashers[w_chunk[k]].take_root_hash().as_bytes();
                }
            }
            let done = if keyed.len() >= 8 {
                let mut inputs = crate::stack_vec::StackVec::<crate::ffi_output::Input<'_>, FINALIZE_BATCH>::new();
                for &k in keyed.as_slice() {
                    inputs.push(crate::ffi_output::input(&hashers[w_chunk[usize::from(k)]]).unwrap());
                }
                let mut digests = [[0u8; OUT_LEN]; FINALIZE_BATCH];
                let done = crate::ffi_output::hash_groups_arm(inputs.as_slice(), &mut digests[..keyed.len()]);
                for (&k, digest) in keyed.as_slice()[..done].iter().zip(&digests[..done]) {
                    o_chunk[usize::from(k)] = *digest;
                }
                done
            } else {
                0
            };
            for &k in &keyed.as_slice()[done..] {
                let k = usize::from(k);
                o_chunk[k] = *hashers[w_chunk[k]].take_root_hash().as_bytes();
            }
            for &k in rest.as_slice() {
                let k = usize::from(k);
                o_chunk[k] = *hashers[w_chunk[k]].take_root_hash().as_bytes();
            }
            for &i in w_chunk {
                hashers[i].reset();
            }
        }
        marked.active = false;
        return;
    }
    #[cfg(not(any(blake3_neon, blake3_sse41_ffi)))]
    {
        finish_each(hashers, which, out);
        marked.active = false;
    }
}

/// Each hasher `which` names finished in place and reset, its digest into
/// `out`: fallback for portable targets.
#[cfg(not(any(blake3_neon, blake3_sse41_ffi)))]
fn finish_each(hashers: &mut [Hasher], which: &[usize], out: &mut [[u8; OUT_LEN]]) {
    for (&i, out) in which.iter().zip(out) {
        *out = *hashers[i].take_root_hash().as_bytes();
        hashers[i].reset();
    }
}

/// A turn of pieces for many hashers, in caller order: each piece
/// gathered into its hasher's stage; a stage that fills a whole group is
/// held, and its hasher's later bytes (the rest of that piece, and its
/// later pieces) wait, in order, until the held groups hash together, up
/// to TABLE of them in one pass (one SME2 call for all; elsewhere a call
/// per group, then each parent level in one call). A full table, a full
/// waiting list, or the turn's end hashes what is held and gathers what
/// waited, pass after pass, until nothing waits. Outside a drain every
/// waiting piece's hasher is held (it entered waiting because the hasher
/// was held, or as the rest of the piece that filled the stage; only a
/// drain un-holds, and a drain leaves nothing waiting), so a hasher that
/// is not held has no earlier bytes waiting, and takes its piece now. Both
/// lists live on the stack: a turn allocates nothing.
fn update_turn(hashers: &mut [Hasher], pieces: &[(usize, &[u8])]) {
    /// The stages a turn holds, hashed however the turn ends. A turn that
    /// panics part-way (an index that names no hasher) unwinds through
    /// here: a full stage that no list named
    /// any more would have no room for its hasher's next update, and its
    /// next turn would wait for ever for a pass to hash it. `flush_held`
    /// detaches the list before hashing so an unwind out of `hash_held`
    /// itself cannot make `Drop` retry the failing call.
    struct Held<'h> {
        hashers: &'h mut [Hasher],
        held: crate::stack_vec::StackVec<usize, TABLE>,
    }
    impl Drop for Held<'_> {
        fn drop(&mut self) {
            flush_held(self.hashers, &mut self.held);
        }
    }
    let mut turn = Held { hashers, held: crate::stack_vec::StackVec::new() };
    let mut waiting = crate::stack_vec::StackVec::<(usize, &[u8]), TABLE>::new();
    for &(i, bytes) in pieces {
        if waiting.is_full() || turn.held.is_full() {
            drain(turn.hashers, &mut turn.held, &mut waiting);
        }
        if turn.hashers[i].held_group().is_some() {
            waiting.push((i, bytes));
            continue;
        }
        gather(turn.hashers, i, bytes, &mut turn.held, &mut waiting);
    }
    drain(turn.hashers, &mut turn.held, &mut waiting);
}

/// `bytes` into hasher `i`, which is not held: as much as fills its stage;
/// a filled stage is held and the rest of the piece waits.
fn gather<'a>(
    hashers: &mut [Hasher],
    i: usize,
    bytes: &'a [u8],
    held: &mut crate::stack_vec::StackVec<usize, TABLE>,
    waiting: &mut crate::stack_vec::StackVec<(usize, &'a [u8]), TABLE>,
) {
    debug_assert!(hashers[i].held_group().is_none(), "a held hasher's bytes wait");
    let rest = hashers[i].gather_part(bytes, None, true);
    assert!(rest.len() < bytes.len() || bytes.is_empty(), "a turn's pass gathered nothing");
    if hashers[i].held_group().is_some() {
        held.push(i);
        if !rest.is_empty() {
            waiting.push((i, rest));
        }
    }
}

/// Hash the held groups, then gather what waited, in order, pass after
/// pass, until nothing waits. Each pass starts with every hasher free, so
/// the first waiting piece of each hasher gathers and shrinks the bytes
/// in play; a hasher filled again in the pass sends its later pieces back
/// to wait. A pass holds at most one group per hasher and keeps at most
/// as many pieces waiting as it took, so neither list overflows.
fn drain<'a>(hashers: &mut [Hasher], held: &mut crate::stack_vec::StackVec<usize, TABLE>, waiting: &mut crate::stack_vec::StackVec<(usize, &'a [u8]), TABLE>) {
    loop {
        flush_held(hashers, held);
        if waiting.is_empty() {
            return;
        }
        let batch = core::mem::take(waiting);
        for &(i, bytes) in &batch {
            if hashers[i].held_group().is_some() {
                waiting.push((i, bytes));
            } else {
                gather(hashers, i, bytes, held, waiting);
            }
        }
        assert!(waiting.len() < batch.len() || !held.is_empty(), "drain made no progress");
    }
}

/// The fewest held stages that take the independent parent lanes. Two
/// keyed hashers each hash alone at full width instead: on an Ice Lake
/// Xeon the lanes cost a pair 3.5% of a whole cycle (16 and 32 KiB
/// messages in 4 KiB pieces, November 10, 2026), and from eight up they
/// gain 2-3% there and 7-11% on an EPYC 9255. The count is public; no
/// key decides the route.
#[cfg(any(all(blake3_avx512_ffi, target_os = "linux"), blake3_neon))]
const INDEPENDENT_MIN: usize = 3;

/// Hash every queued stage after detaching the table so an unwind inside
/// `hash_held` cannot make `Held::drop` retry the same failing call.
fn flush_held(hashers: &mut [Hasher], held: &mut crate::stack_vec::StackVec<usize, TABLE>) {
    if held.is_empty() {
        return;
    }
    let list = core::mem::take(held);
    hash_held(hashers, &list);
}

#[cfg(all(test, feature = "std"))]
std::thread_local! {
    // Actual cohorts passed to the shared-key or independent-parent body.
    static HELD_GROUPS: core::cell::RefCell<Vec<Vec<usize>>> = const { core::cell::RefCell::new(Vec::new()) };
}

/// On x86-64 Linux and AArch64, hash the held stages with an independent key
/// per parent lane (and on SME2, one multi-group keyed leaf entry), when any
/// of them is keyed and there are enough of them, and say so. The mode, the
/// count and the platform, all public, choose; key values never determine
/// grouping, allocation or dispatch. Every other target compiles `hash_held`
/// as it was.
#[cfg(all(feature = "std", any(all(blake3_avx512_ffi, target_os = "linux"), blake3_neon)))]
fn independent_lanes(hashers: &mut [Hasher], held: &[usize]) -> bool {
    let Some(&first) = held.first() else { return false };
    let platform = hashers[first].platform();
    #[cfg(all(blake3_avx512_ffi, target_os = "linux"))]
    if !matches!(platform, Platform::AVX2 | Platform::AVX512) {
        return false;
    }
    // Public plain mode fixes every key to the IV: those share one pass.
    if held.len() < INDEPENDENT_MIN || held.iter().all(|&i| hashers[i].is_plain()) {
        return false;
    }
    crate::ffi_output::held_groups(hashers, held, platform);
    #[cfg(test)]
    let _ = HELD_GROUPS.try_with(|groups| groups.borrow_mut().push(held.to_vec()));
    crate::counted(crate::Counted::HeldPass);
    true
}

#[cfg(all(test, feature = "std"))]
std::thread_local! {
    /// Most recent `hash_cohort::<N>` workspace tier used on this thread.
    static LAST_COHORT_TIER: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

/// Hash the whole groups `held` names, one in each of those hashers'
/// stages, and pass each group's subtree to its hasher's core. The plain
/// hashers' groups hash together; a keyed or key-derivation hasher's group
/// hashes alone, since its key is secret and must not decide the work
/// (BLAKE3 #312): the mode alone, public, groups them.
#[cfg(feature = "std")]
fn hash_held(hashers: &mut [Hasher], held: &[usize]) {
    #[cfg(any(all(blake3_avx512_ffi, target_os = "linux"), blake3_neon))]
    if independent_lanes(hashers, held) {
        return;
    }
    // Plain's key is the IV, the same for every plain hasher. Tile larger
    // cohorts in 16-stage L1D-hot blocks (`HeldWorkspace<16>`, ~15 KiB stack)
    // under one shared `Sme2Turn` so even 17..=128 plain hashers fit inside a
    // 64 KiB worker stack without reserving a 116 KiB `TABLE`-sized frame.
    const PLAIN_TILE: usize = 16;
    let mut plain = crate::stack_vec::StackVec::<usize, TABLE>::new();
    plain.extend(held.iter().copied().filter(|&i| hashers[i].is_plain()));
    if !plain.is_empty() {
        #[cfg(test)]
        let _ = HELD_GROUPS.try_with(|groups| groups.borrow_mut().push(plain.as_slice().to_vec()));
        #[cfg(test)]
        crate::counted(crate::Counted::HeldPass);
        let turn = crate::platform::Sme2Turn::take(hashers[plain[0]].platform(), true);
        let platform = turn.platform();
        if plain.len() == 1 {
            hash_cohort_on::<1>(hashers, &plain, platform);
        } else {
            for tile in plain.as_slice().chunks(PLAIN_TILE) {
                hash_cohort_on::<PLAIN_TILE>(hashers, tile, platform);
            }
        }
    }
    for &i in held {
        if !hashers[i].is_plain() {
            hash_cohort::<1>(hashers, &[i]);
        }
    }
}

/// Parent blocks supplied by a full staged message's pairs of CVs.
const STAGE_PARENTS: usize = ALONE_CHUNKS / 2;

/// Contiguous parent inputs retain the kernels' direct path. Both CV
/// levels live on the stack; ping-pong leaves the final two children in
/// the other buffer. Block supplies the existing 64-byte alignment.
pub(crate) struct HeldWorkspace<const N: usize> {
    level: crate::stack_vec::StackVec<[Block; STAGE_PARENTS], N>,
    next: crate::stack_vec::StackVec<[Block; STAGE_PARENTS / 2], N>,
}

impl<const N: usize> HeldWorkspace<N> {
    /// Empty buffers, ready to initialize in their final place.
    #[inline(always)]
    pub(crate) fn new() -> Self {
        const {
            assert!(ALONE_CHUNKS == 16 && crate::STAGE_LEN == ALONE_CHUNKS * CHUNK_LEN);
        }
        Self { level: crate::stack_vec::StackVec::new(), next: crate::stack_vec::StackVec::new() }
    }

    /// Call once, with 1 <= count <= N: initialize through a borrow of the
    /// workspace's final place, then borrow its two CV levels.
    pub(crate) fn buffers(&mut self, count: usize) -> (&mut [CVBytes], &mut [CVBytes]) {
        assert!(count > 0 && count <= N && self.level.is_empty() && self.next.is_empty(), "one fresh HeldWorkspace per cohort");
        self.level.extend(core::iter::repeat_n([Block::ZERO; STAGE_PARENTS], count));
        self.next.extend(core::iter::repeat_n([Block::ZERO; STAGE_PARENTS / 2], count));
        let (level, _) = Block::bytes_mut(self.level.as_mut_slice().as_flattened_mut()).as_chunks_mut::<OUT_LEN>();
        let (next, _) = Block::bytes_mut(self.next.as_mut_slice().as_flattened_mut()).as_chunks_mut::<OUT_LEN>();
        (level, next)
    }
}

/// Hash 1..=N distinct held stages: plain hashers together, or one keyed
/// or derive-key hasher. Acquires an SME2 turn for the cohort and records
/// test pass counters, then delegates to [`hash_cohort_on`].
#[cfg(feature = "std")]
#[inline(never)]
fn hash_cohort<const N: usize>(hashers: &mut [Hasher], alike: &[usize]) {
    let first = alike[0];
    #[cfg(all(test, feature = "std"))]
    let _ = HELD_GROUPS.try_with(|groups| groups.borrow_mut().push(alike.to_vec()));
    #[cfg(all(test, feature = "std"))]
    crate::counted(crate::Counted::HeldPass);
    let turn = crate::platform::Sme2Turn::take(hashers[first].platform(), true);
    hash_cohort_on::<N>(hashers, alike, turn.platform());
}

/// Hash one tile of 1..=N distinct held stages on `platform` (with any SME2
/// turn held by the caller across tiles). Keeps chunks and each parent level
/// in a bounded `HeldWorkspace<N>` on the stack.
#[cfg(feature = "std")]
// Keep the workspace frame out of empty turns and the other mode's caller.
#[inline(never)]
fn hash_cohort_on<const N: usize>(hashers: &mut [Hasher], alike: &[usize], platform: Platform) {
    use crate::{BLOCK_LEN, PARENT};
    let first = alike[0];
    #[cfg(all(test, feature = "std"))]
    let _ = LAST_COHORT_TIER.try_with(|t| t.set(N));
    let domain = hashers[first].domain();
    let flags = domain.flags();
    // Plain cohorts hash under the constant IV directly (fault isolation),
    // while a keyed or derive-key hasher hashes alone here under its own key.
    let key = if domain.is_plain() { *crate::IV } else { *domain.key() };
    let mut work = HeldWorkspace::<N>::new();
    let (mut level, mut next) = work.buffers(alike.len());
    #[cfg(blake3_sme2)]
    let on_sme2 = matches!(platform, Platform::SME2);
    #[cfg(not(blake3_sme2))]
    let on_sme2 = false;
    {
        let mut refs = crate::stack_vec::StackVec::<[&[u8; CHUNK_LEN]; ALONE_CHUNKS], N>::new();
        refs.extend(alike.iter().map(|&i| {
            let hasher = &hashers[i];
            // The shared key serves plain hashers only. Check the public
            // mode as we take each stage, without comparing key bytes.
            assert!(alike.len() == 1 || hasher.is_plain(), "only plain hashers share a cohort");
            let stage = hasher.held_group().expect("a held group").1;
            core::array::from_fn(|j| stage[j * CHUNK_LEN..(j + 1) * CHUNK_LEN].try_into().unwrap())
        }));
        let refs = refs.as_slice().as_flattened();
        let mut counters = crate::stack_vec::StackVec::<u64, N>::new();
        counters.extend(alike.iter().map(|&i| hashers[i].held_group().unwrap().0));
        if on_sme2 {
            #[cfg(blake3_sme2)]
            // Sound: the turn detects SME2. Borrowed stages and stack output
            // are disjoint, with one counter per full 16-chunk stage.
            unsafe {
                crate::sme2::hash_groups_at(refs, &counters, &key, flags, CHUNK_START, CHUNK_END, level.as_flattened_mut())
            };
        } else {
            for (k, &counter) in counters.iter().enumerate() {
                platform.hash_many(
                    &refs[k * ALONE_CHUNKS..(k + 1) * ALONE_CHUNKS],
                    &key,
                    counter,
                    IncrementCounter::Yes,
                    flags,
                    CHUNK_START,
                    CHUNK_END,
                    level[k * ALONE_CHUNKS..(k + 1) * ALONE_CHUNKS].as_flattened_mut(),
                );
            }
        }
    }
    let parent_flags = flags | PARENT;
    let mut per = ALONE_CHUNKS;
    while per > 1 {
        let count = alike.len() * per / 2;
        let bytes = level[..2 * count].as_flattened();
        if on_sme2 {
            // SME2's parent kernel reads a group of sixteen blocks through
            // one pointer: the level goes to it as the run of memory it
            // is, not as a table of sixteen borrows.
            #[cfg(blake3_sme2)]
            // Sound: the turn's platform is SME2 only where detection
            // found it.
            unsafe {
                crate::sme2::hash_block_run(bytes, &key, parent_flags, next[..count].as_flattened_mut())
            };
        } else {
            // Eight parent blocks per full stage. Derive descriptor rows
            // from that layout, including its readable final padding.
            let (full, tail) = bytes.as_chunks::<BLOCK_LEN>().0.as_chunks::<STAGE_PARENTS>();
            let mut rows = crate::stack_vec::StackVec::<_, N>::new();
            rows.extend(full.iter().map(|row| row.each_ref()));
            if let Some(first) = tail.first() {
                rows.push(core::array::from_fn(|i| tail.get(i).unwrap_or(first)));
            }
            platform.hash_many(&rows.as_slice().as_flattened()[..count], &key, 0, IncrementCounter::No, parent_flags, 0, 0, next[..count].as_flattened_mut());
        }
        core::mem::swap(&mut level, &mut next);
        per /= 2;
    }
    for (k, &i) in alike.iter().enumerate() {
        let (counter, _) = hashers[i].held_group().unwrap();
        let mut result = [0u8; BLOCK_LEN];
        if counter == 0 {
            result.copy_from_slice(next[2 * k..2 * k + 2].as_flattened());
        } else {
            result[..OUT_LEN].copy_from_slice(&level[k]);
        }
        hashers[i].take_held(&result);
    }
}

#[cfg(not(feature = "std"))]
fn hash_held(_: &mut [Hasher], held: &[usize]) {
    assert!(held.is_empty(), "nothing held without std");
}

/// Messages of this many chunks or more fill the lanes alone.
const ALONE_CHUNKS: usize = 16;

/// The hash of each message, of any lengths, on the calling thread:
/// `out[i]` is [`hash`](crate::throughput::hash)`(mode, items[i].0,
/// items[i].1)`, each item a message of `items[i].1` bytes starting its
/// blocks (the bytes past its end ignored). Short messages hash together,
/// so a collection of small items (a directory's files, a repository's
/// objects) takes less time than one call each. Requires `out.len() ==
/// items.len()`.
///
/// ```
/// # blake3_servil_core::initialize();
/// use blake3_servil_core::{Mode, convenience, throughput::{self, Block}};
/// let blocks: Vec<Vec<Block>> = (0..100).map(|i| vec![Block([i as u8; 64]); (100 * i as usize).div_ceil(64)]).collect();
/// let items: Vec<(&[Block], usize)> = blocks.iter().enumerate().map(|(i, b)| (&b[..], 100 * i)).collect();
/// let mut out = vec![[0u8; 32]; items.len()];
/// throughput::hash_each(Mode::Plain, &items, &mut out);
/// assert_eq!(&out[42], convenience::hash(&[42u8; 4200]).as_bytes());
/// ```
pub fn hash_each(mode: Mode, items: &[(&[Block], usize)], out: &mut [[u8; OUT_LEN]]) {
    assert_eq!(items.len(), out.len(), "one digest per item");
    let mut room = core::mem::MaybeUninit::uninit();
    let (key, flags) = mode.key(&mut room);
    hash_each_on(key, flags, items, out, Platform::detect(), |item| crate::hash_serial(item, key, flags));
}

/// The most items [`hash_by_blocks`] sorts at once: a stream task's
/// members, or with SME2 its message kernel's table.
#[cfg(blake3_sme2)]
pub(crate) const BY_BLOCKS_AT_ONCE: usize = TABLE;
#[cfg(not(blake3_sme2))]
pub(crate) const BY_BLOCKS_AT_ONCE: usize = 64;

/// The digest of each of `items`, every one of up to a chunk, of any
/// lengths, by their blocks: those with as many whole blocks before their
/// last (k) side by side on the platform's hash_many, up to sixteen at a
/// time. On NEON with SHA-3 their last blocks run in the same lanes,
/// each with its own length (the l kernels). Other platforms hash the
/// prefix with CHUNK_START on the first block and no end, leaving each its
/// chaining value; then each one's last block (1 to 64 bytes, zero-padded;
/// an empty message's is empty) compressed on its own with CHUNK_END and
/// ROOT, CHUNK_START too where k is 0. Up to BY_BLOCKS_AT_ONCE items are
/// sorted at once, on the stack. Requires AVX2 or AVX-512 on x86-64; NEON on AArch64.
pub(crate) fn hash_by_blocks(items: &[&[u8]], key: &CVWords, flags: u8, out: &mut [[u8; OUT_LEN]], platform: Platform) {
    hash_chunks_by_blocks(items, key, 0, flags, CHUNK_END | crate::ROOT, out, platform);
}

fn hash_chunks_by_blocks(items: &[&[u8]], key: &CVWords, counter: u64, flags: u8, flags_end: u8, out: &mut [[u8; OUT_LEN]], platform: Platform) {
    assert_eq!(items.len(), out.len(), "one digest per item");
    assert!(items.iter().all(|i| i.len() <= CHUNK_LEN), "items of up to a chunk");
    for (items, out) in items.chunks(BY_BLOCKS_AT_ONCE).zip(out.chunks_mut(BY_BLOCKS_AT_ONCE)) {
        by_blocks_at_once(items, key, counter, flags, flags_end, out, platform);
    }
}

fn by_blocks_at_once(items: &[&[u8]], key: &CVWords, counter: u64, flags: u8, flags_end: u8, out: &mut [[u8; OUT_LEN]], platform: Platform) {
    const GROUP: usize = 16;
    // On SME2, a table of messages of one length, whole blocks, at counter 0,
    // with at least the work that pays for the SME2 turn: one call of the message
    // kernel for all of them (a turn's 1 KiB messages finishing together:
    // the NEON lanes took 30-64% longer on an Apple M4 Max, jobs 2016 and
    // 2019). A table of one-block messages never reaches the threshold
    // (BY_BLOCKS_AT_ONCE of 64 bytes is under SME2_SIZED_LEN).
    #[cfg(blake3_sme2)]
    {
        let len = items[0].len();
        if counter == 0
            && matches!(platform, Platform::SME2)
            && items.len() * len >= crate::SME2_SIZED_LEN
            && len % crate::BLOCK_LEN == 0
            && items.iter().all(|item| item.len() == len)
        {
            let turn = crate::platform::Sme2Turn::take(platform, true);
            if matches!(turn.platform(), Platform::SME2) {
                macro_rules! run {
                    ($n:expr) => {{
                        let mut lanes: crate::stack_vec::StackVec<&[u8; $n], BY_BLOCKS_AT_ONCE> =
                            items.iter().map(|item| (*item).try_into().expect("equal whole-block lengths")).collect();
                        while lanes.len() % GROUP != 0 {
                            lanes.push(*lanes.last().unwrap());
                        }
                        crate::counted(crate::Counted::ByBlocksCall);
                        // Sound: detection and the turn provide SME2. Every
                        // typed lane holds its whole input; spare lanes
                        // repeat a valid input and write no output.
                        unsafe { crate::sme2::hash_messages(&lanes, key, flags, CHUNK_START, flags_end, crate::BLOCK_LEN, out.as_flattened_mut()) };
                    }};
                }
                match len {
                    128 => run!(128),
                    192 => run!(192),
                    256 => run!(256),
                    320 => run!(320),
                    384 => run!(384),
                    448 => run!(448),
                    512 => run!(512),
                    576 => run!(576),
                    640 => run!(640),
                    704 => run!(704),
                    768 => run!(768),
                    832 => run!(832),
                    896 => run!(896),
                    960 => run!(960),
                    1024 => run!(1024),
                    _ => unreachable!("an SME2-sized table of whole blocks of up to a chunk"),
                }
                return;
            }
        }
    }
    let whole = |item: &[u8]| item.len().saturating_sub(1) / crate::BLOCK_LEN;
    let mut order: crate::stack_vec::StackVec<usize, BY_BLOCKS_AT_ONCE> = (0..items.len()).collect();
    order.sort_unstable_by_key(|&i| whole(items[i]));
    // With SHA-3 the last block stays in the lanes too; other AArch64
    // CPUs keep the prefix kernels followed by scalar last blocks.
    #[cfg(blake3_neon)]
    let last_lanes = crate::neon_hybrid::sha3_detected();
    #[cfg(not(blake3_neon))]
    let last_lanes = false;
    let mut cvs = [*key; BY_BLOCKS_AT_ONCE];
    // chunk_by always advances and returns nonempty groups, even if a
    // changed predicate groups the wrong items; no mutable loop bound.
    for same_blocks in order.chunk_by(|&a, &b| whole(items[a]) == whole(items[b])) {
        let k = whole(items[same_blocks[0]]);
        #[cfg(blake3_neon)]
        if last_lanes {
            for group in same_blocks.chunks(GROUP) {
                let inputs: crate::stack_vec::StackVec<*const u8, GROUP> = group.iter().map(|&i| items[i].as_ptr()).collect();
                let mut last_blocks = [[0u8; crate::BLOCK_LEN]; GROUP];
                let mut lengths = [0u32; GROUP];
                for ((&i, block), len) in group.iter().zip(&mut last_blocks).zip(&mut lengths) {
                    let tail = &items[i][k * crate::BLOCK_LEN..];
                    block[..tail.len()].copy_from_slice(tail);
                    *len = u32::from(tail.len().to_le_bytes()[0]);
                }
                let lasts: crate::stack_vec::StackVec<*const u8, GROUP> = last_blocks[..group.len()].iter().map(|b| b.as_ptr()).collect();
                let mut digests = [0u8; GROUP * OUT_LEN];
                crate::counted(crate::Counted::ByBlocksCall);
                // Sound: SHA-3 is present; these items share k whole blocks
                // before their separately padded last block, 0 to 64 bytes.
                unsafe {
                    crate::neon_hybrid::hash_last_lanes(
                        &inputs,
                        &lasts,
                        &lengths[..group.len()],
                        k + 1,
                        key,
                        counter,
                        flags,
                        CHUNK_START,
                        flags_end,
                        &mut digests[..group.len() * OUT_LEN],
                    );
                }
                for (&i, digest) in group.iter().zip(digests.chunks_exact(OUT_LEN)) {
                    out[i].copy_from_slice(digest);
                }
            }
            continue;
        }
        if k > 0 {
            for group in same_blocks.chunks(GROUP) {
                let inputs: crate::stack_vec::StackVec<*const u8, GROUP> = group.iter().map(|&i| items[i].as_ptr()).collect();
                let mut lanes = [0u8; GROUP * OUT_LEN];
                crate::counted(crate::Counted::ByBlocksCall);
                // Sound: each item holds more than k whole blocks; the
                // caller selected this platform's block-group path.
                #[cfg(target_arch = "x86_64")]
                unsafe {
                    crate::pair::hash_many_blocks(platform, &inputs, k, key, counter, false, flags, CHUNK_START, 0, &mut lanes[..group.len() * OUT_LEN])
                };
                #[cfg(blake3_neon)]
                unsafe {
                    crate::neon_hybrid::hash_messages_raw(
                        inputs.as_ptr(),
                        group.len(),
                        k,
                        key,
                        counter,
                        flags,
                        CHUNK_START,
                        0,
                        crate::BLOCK_LEN,
                        &mut lanes[..group.len() * OUT_LEN],
                    )
                };
                for (n, &i) in group.iter().enumerate() {
                    cvs[i] = crate::platform::words_from_le_bytes_32(lanes[n * OUT_LEN..][..OUT_LEN].try_into().unwrap());
                }
            }
        }
    }
    if last_lanes {
        return;
    }
    for &i in &order {
        let item = items[i];
        let k = whole(item);
        let mut cv = cvs[i];
        let mut block = [0u8; crate::BLOCK_LEN];
        let last = item.len() - k * crate::BLOCK_LEN;
        block[..last].copy_from_slice(&item[k * crate::BLOCK_LEN..]);
        let start = if k == 0 { CHUNK_START } else { 0 };
        platform.compress_in_place(&mut cv, &block, last.to_le_bytes()[0], counter, flags | start | flags_end);
        out[i] = crate::hasher::le_bytes(&cv);
    }
}

/// An item of a collection, as hash_each_on reads it: its bytes. The
/// public calls' items are messages in blocks; the pool's ranges, bytes.
pub(crate) trait Item {
    fn bytes(&self) -> &[u8];
}

impl Item for &[u8] {
    #[inline(always)]
    fn bytes(&self) -> &[u8] {
        self
    }
}

impl Item for (&[Block], usize) {
    #[inline(always)]
    fn bytes(&self) -> &[u8] {
        &Block::bytes(self.0)[..self.1]
    }
}

/// [`hash_each`] in the mode of `key` and `flags`, the lanes on
/// `platform`, each message that fills the lanes alone (or is a chunk or
/// less) through `alone`.
pub(crate) fn hash_each_on<I: Item>(key: &CVWords, flags: u8, items: &[I], out: &mut [[u8; OUT_LEN]], platform: Platform, alone: impl Fn(&[u8]) -> Hash) {
    let key = *key;
    // Items of up to a chunk, two or more, by their blocks (hash_by_blocks).
    #[cfg(target_arch = "x86_64")]
    let platform_takes = crate::pair::takes_platform(platform);
    // Every supported AArch64 platform has NEON; the kernel family
    // selects SHA-3 or plain instructions itself.
    #[cfg(target_arch = "aarch64")]
    let platform_takes = true;
    let by_blocks = platform_takes && items.iter().filter(|i| i.bytes().len() <= CHUNK_LEN).take(2).count() == 2;
    if by_blocks {
        // BY_BLOCKS_AT_ONCE of them at a time, their indices on the stack:
        // a collection of small items allocates nothing and returns before
        // borrowing SHARED or scanning items again.
        let mut small_done = 0usize;
        let mut small = (0..items.len()).filter(|&i| items[i].bytes().len() <= CHUNK_LEN).peekable();
        while small.peek().is_some() {
            let part: crate::stack_vec::StackVec<usize, BY_BLOCKS_AT_ONCE> = small.by_ref().take(BY_BLOCKS_AT_ONCE).collect();
            small_done += part.len();
            let bytes: crate::stack_vec::StackVec<&[u8], BY_BLOCKS_AT_ONCE> = part.iter().map(|&i| items[i].bytes()).collect();
            let mut digests = [[0u8; OUT_LEN]; BY_BLOCKS_AT_ONCE];
            hash_by_blocks(&bytes, &key, flags, &mut digests[..part.len()], platform);
            for (&i, digest) in part.iter().zip(&digests) {
                out[i] = *digest;
            }
        }
        if small_done == items.len() {
            return;
        }
    }
    if items.is_empty() {
        return;
    }
    // The messages of 2 to ALONE_CHUNKS - 1 chunks share the lanes, their
    // k-th whole chunks side by side across the whole collection (the more
    // messages a round holds, the fuller its lanes). Their places and chunk
    // values live in this thread's storage, kept between calls (SHARED),
    // falling back to call-local storage during thread-local teardown.
    if SHARED
        .try_with(|shared| {
            let mut shared = shared.try_borrow_mut().expect("one collection at a time on a thread");
            hash_each_middle(&mut shared, &key, flags, items, out, platform, by_blocks, platform_takes, &alone);
        })
        .is_err()
    {
        let mut local = Shared { messages: Vec::new(), cvs: Vec::new() };
        hash_each_middle(&mut local, &key, flags, items, out, platform, by_blocks, platform_takes, &alone);
    }
}

/// Shared-lane rounds for a nonempty collection (handles single/large items
/// through `alone` and `2..ALONE_CHUNKS`-chunk items through `shared`).
#[allow(clippy::too_many_arguments)]
fn hash_each_middle<I: Item>(
    shared: &mut Shared,
    key: &CVWords,
    flags: u8,
    items: &[I],
    out: &mut [[u8; OUT_LEN]],
    platform: Platform,
    by_blocks: bool,
    platform_takes: bool,
    alone: &impl Fn(&[u8]) -> Hash,
) {
    let Shared { messages, cvs } = shared;
    messages.clear();
    #[cfg(blake3_neon)]
    let last_lanes = crate::neon_hybrid::sha3_detected();
    #[cfg(not(blake3_neon))]
    let last_lanes = false;
    let min_k = usize::from(!last_lanes);
    let mut slots = 0;
    let mut most_full = 0;
    let mut max_chunks = 0usize;
    let mut partial_any = [false; ALONE_CHUNKS];
    let mut partial_seen = [0u16; ALONE_CHUNKS];
    let mut partial_shared = [false; ALONE_CHUNKS];
    for (i, item) in items.iter().map(Item::bytes).enumerate() {
        let chunks = item.len().div_ceil(CHUNK_LEN);
        if chunks <= 1 && by_blocks {
            continue;
        }
        if chunks <= 1 || chunks >= ALONE_CHUNKS {
            out[i] = *alone(item).as_bytes();
        } else {
            let full = item.len() / CHUNK_LEN;
            most_full = most_full.max(full);
            max_chunks = max_chunks.max(chunks);
            if full != chunks {
                partial_any[full] = true;
                let tail = &item[full * CHUNK_LEN..];
                let k = (tail.len() - 1) / crate::BLOCK_LEN;
                if k >= min_k {
                    let bit = 1u16 << k;
                    if (partial_seen[full] & bit) != 0 {
                        partial_shared[full] = true;
                    }
                    partial_seen[full] |= bit;
                }
            }
            messages.push((i, slots));
            slots += chunks;
        }
    }
    if messages.is_empty() {
        return;
    }
    cvs.clear();
    cvs.resize(slots, [0u8; OUT_LEN]);
    // Partial last chunks (1,025..=15,360 B middle messages) run before taking
    // an SME2 turn (NEON `hash_last_lanes` and x86 `hash_many_blocks` do not
    // use SME2, and each partial chunk writes only `cvs[at + c]`, disjoint from
    // full chunks `cvs[at..at + c]`). Batch through `hash_chunks_by_blocks`
    // only when at least two tails at counter `c` share a SIMD-eligible
    // prefix-block length, avoiding sort/scatter overhead on incompatible pairs.
    for c in 1..ALONE_CHUNKS {
        if !partial_any[c] {
            continue;
        }
        if platform_takes && partial_shared[c] {
            let mut slices = crate::stack_vec::StackVec::<&[u8], BY_BLOCKS_AT_ONCE>::new();
            let mut owners = crate::stack_vec::StackVec::<usize, BY_BLOCKS_AT_ONCE>::new();
            let mut flush_partial = |slices: &mut crate::stack_vec::StackVec<&[u8], BY_BLOCKS_AT_ONCE>,
                                     owners: &mut crate::stack_vec::StackVec<usize, BY_BLOCKS_AT_ONCE>| {
                let mut digests = [[0u8; OUT_LEN]; BY_BLOCKS_AT_ONCE];
                hash_chunks_by_blocks(slices, key, c as u64, flags, CHUNK_END, &mut digests[..slices.len()], platform);
                for (&slot, digest) in owners.iter().zip(&digests) {
                    cvs[slot] = *digest;
                }
                slices.clear();
                owners.clear();
            };
            for &(i, at) in messages.iter() {
                let item = items[i].bytes();
                let full = item.len() / CHUNK_LEN;
                if full == c && item.len() > c * CHUNK_LEN {
                    slices.push(&item[c * CHUNK_LEN..]);
                    owners.push(at + c);
                    if slices.is_full() {
                        flush_partial(&mut slices, &mut owners);
                    }
                }
            }
            if !slices.is_empty() {
                flush_partial(&mut slices, &mut owners);
            }
        } else {
            for &(i, at) in messages.iter() {
                let item = items[i].bytes();
                let full = item.len() / CHUNK_LEN;
                if full == c && item.len() > c * CHUNK_LEN {
                    cvs[at + c] = ChunkState::new(key, c as u64, flags, platform).update(&item[c * CHUNK_LEN..]).output().chaining_value();
                }
            }
        }
    }
    // Acquire one best-effort SME2 turn across the shared full-chunk rounds and
    // batched Merkle parent/root rounds whenever either the chunk/root rounds
    // (`messages.len() >= 16`) or the first non-root parent round
    // (`first_parent_pairs >= 16`) can reach an SME2 group of 16 lanes.
    #[cfg(blake3_sme2)]
    let turn = crate::sme2::turn_for_each_middle(platform, items, messages);
    #[cfg(blake3_sme2)]
    let round_platform = turn.platform();
    #[cfg(not(blake3_sme2))]
    let round_platform = platform;
    for k in 0..most_full {
        crate::counted(crate::Counted::EachRound);
        // A round's lanes go to the kernels LANES_AT_ONCE at a time,
        // their chunks and places on the stack.
        let mut refs = crate::stack_vec::StackVec::<&[u8; CHUNK_LEN], LANES_AT_ONCE>::new();
        let mut owners = crate::stack_vec::StackVec::<usize, LANES_AT_ONCE>::new();
        let mut flush = |refs: &mut crate::stack_vec::StackVec<&[u8; CHUNK_LEN], LANES_AT_ONCE>,
                         owners: &mut crate::stack_vec::StackVec<usize, LANES_AT_ONCE>| {
            let mut lane_out = [0u8; LANES_AT_ONCE * OUT_LEN];
            let lanes = &mut lane_out[..refs.len() * OUT_LEN];
            round_platform.hash_many(refs, key, k as u64, IncrementCounter::No, flags, CHUNK_START, CHUNK_END, lanes);
            for (j, &slot) in owners.iter().enumerate() {
                cvs[slot].copy_from_slice(&lanes[j * OUT_LEN..(j + 1) * OUT_LEN]);
            }
            refs.clear();
            owners.clear();
        };
        for &(i, at) in messages.iter() {
            if let Some(chunk) = items[i].bytes().get(k * CHUNK_LEN..(k + 1) * CHUNK_LEN) {
                refs.push(chunk.try_into().unwrap());
                owners.push(at + k);
                if refs.is_full() {
                    flush(&mut refs, &mut owners);
                }
            }
        }
        if !refs.is_empty() {
            flush(&mut refs, &mut owners);
        }
    }
    if messages.len() == 1 {
        let (i, at) = messages[0];
        let chunks = items[i].bytes().len().div_ceil(CHUNK_LEN);
        out[i] = *root(&cvs[at..at + chunks], key, flags, platform).as_bytes();
    } else {
        // Reduce every message's chunk CVs bottom-up in place with no pair
        // assembly/copy in `each.rs`: left-to-right adjacent pairing never
        // crosses BLAKE3's power-of-two left-subtree boundary, and each pair
        // sits contiguously as a 64-byte parent block in `cvs`. For
        // `max_chunks` in 2..=15, `parent_rounds` (0..=3) leaves every message
        // with its 2 root-input CVs at `cvs[at..at + 2]`. After `round`
        // reductions of `chunks` CVs, the live count is `((chunks - 1) >> round) + 1`.
        let parent_flags = flags | crate::PARENT;
        let parent_rounds = usize::BITS - (max_chunks - 1).leading_zeros() - 1;
        for round in 0..parent_rounds {
            let mut jobs = crate::stack_vec::StackVec::<(usize, usize), LANES_AT_ONCE>::new();
            let mut flushed = false;
            let mut flush_parents = |jobs: &mut crate::stack_vec::StackVec<(usize, usize), LANES_AT_ONCE>, cvs: &mut [CVBytes]| {
                flushed = true;
                let mut lane_out = [0u8; LANES_AT_ONCE * OUT_LEN];
                {
                    let mut refs = crate::stack_vec::StackVec::<&[u8; crate::BLOCK_LEN], LANES_AT_ONCE>::new();
                    for &(src, _) in jobs.iter() {
                        refs.push(cvs[src..src + 2].as_flattened().try_into().unwrap());
                    }
                    round_platform.hash_many(&refs, key, 0, IncrementCounter::No, parent_flags, 0, 0, &mut lane_out[..refs.len() * OUT_LEN]);
                }
                for (j, &(_, dst)) in jobs.iter().enumerate() {
                    cvs[dst].copy_from_slice(&lane_out[j * OUT_LEN..(j + 1) * OUT_LEN]);
                }
                jobs.clear();
            };
            for &(i, at) in messages.iter() {
                let chunks = items[i].bytes().len().div_ceil(CHUNK_LEN);
                let n = ((chunks - 1) >> round) + 1;
                if n <= 2 {
                    continue;
                }
                for j in 0..(n / 2) {
                    jobs.push((at + 2 * j, at + j));
                    if jobs.is_full() {
                        flush_parents(&mut jobs, cvs);
                    }
                }
            }
            if !jobs.is_empty() {
                flush_parents(&mut jobs, cvs);
            }
            assert!(flushed, "each scheduled parent round reduces at least one pair");
            for &(i, at) in messages.iter() {
                let chunks = items[i].bytes().len().div_ceil(CHUNK_LEN);
                let n = ((chunks - 1) >> round) + 1;
                if n <= 2 {
                    continue;
                }
                if (n & 1) == 1 {
                    let pairs = n / 2;
                    cvs[at + pairs] = cvs[at + 2 * pairs];
                }
            }
        }
        let root_flags = flags | crate::PARENT | crate::ROOT;
        for chunk in messages.chunks(LANES_AT_ONCE) {
            let mut refs = crate::stack_vec::StackVec::<&[u8; crate::BLOCK_LEN], LANES_AT_ONCE>::new();
            for &(_, at) in chunk {
                refs.push(cvs[at..at + 2].as_flattened().try_into().unwrap());
            }
            let mut lane_out = [0u8; LANES_AT_ONCE * OUT_LEN];
            round_platform.hash_many(&refs, key, 0, IncrementCounter::No, root_flags, 0, 0, &mut lane_out[..refs.len() * OUT_LEN]);
            for (j, &(i, _)) in chunk.iter().enumerate() {
                out[i].copy_from_slice(&lane_out[j * OUT_LEN..(j + 1) * OUT_LEN]);
            }
        }
    }
    #[cfg(blake3_sme2)]
    drop(turn);
    messages.clear();
    cvs.clear();
}

/// The lanes of one call to the kernels within a round of [`hash_each_on`].
pub(crate) const LANES_AT_ONCE: usize = TABLE;

/// What [`hash_each_on`] keeps on a thread between calls, for a
/// collection's messages of 2 to ALONE_CHUNKS - 1 chunks: each one's index
/// and where its chunk values start (16 bytes a message), and the chunk
/// values (32 bytes a chunk). It grows at the thread's first such
/// collection and at a larger one, and a repeated collection allocates
/// nothing (BLAKE3 #325; each call took and freed them before).
struct Shared {
    messages: Vec<(usize, usize)>,
    cvs: Vec<CVBytes>,
}

std::thread_local! {
    static SHARED: core::cell::RefCell<Shared> = const { core::cell::RefCell::new(Shared { messages: Vec::new(), cvs: Vec::new() }) };
}

/// The messages and chunk values a pool thread's middle-message scratch
/// holds from its start: one cut range of a pooled collection is cut at
/// `lanes::MAX_PIECE_LEN` (128 KiB) of items, so in the worst ratio it can
/// take up to 127 two-chunk items of 1,025 bytes (130,175 bytes, 254 CVs)
/// plus one 15-chunk overshooting item (128 messages, 269 CVs). Rounding
/// up to 144 messages and 288 CVs keeps every pooled cut range's
/// middle-message scratch allocation-free on a prepared thread (separate
/// from caller piece scratch in `lanes` and the no-spare-pool fallback).
const PREPARED_MESSAGES: usize = 144;
const PREPARED_CVS: usize = 288;

/// This thread's storage for the collections it hashes, made now: the
/// pool's threads at their start, and the thread that initializes the pool.
pub fn prepare_thread() {
    let _ = SHARED.try_with(|shared| {
        let mut shared = shared.borrow_mut();
        shared.messages.clear();
        shared.cvs.clear();
        shared.messages.reserve(PREPARED_MESSAGES);
        shared.cvs.reserve(PREPARED_CVS);
    });
}

/// The root of a message of two or more chunks, from its chunk values: the
/// left subtree takes the largest power of two of chunks below the count.
fn root(cvs: &[CVBytes], key: &CVWords, flags: u8, platform: Platform) -> Hash {
    let split = left_len(cvs.len());
    let left = subtree(&cvs[..split], key, flags, platform);
    let right = subtree(&cvs[split..], key, flags, platform);
    crate::hasher::parent_output(&left, &right, key, flags, platform).root_hash()
}

/// A non-root subtree's chaining value.
fn subtree(cvs: &[CVBytes], key: &CVWords, flags: u8, platform: Platform) -> CVBytes {
    if cvs.len() == 1 {
        return cvs[0];
    }
    let split = left_len(cvs.len());
    let left = subtree(&cvs[..split], key, flags, platform);
    let right = subtree(&cvs[split..], key, flags, platform);
    crate::hasher::parent_output(&left, &right, key, flags, platform).chaining_value()
}

/// The largest power of two below `n` (n of 2 or more).
fn left_len(n: usize) -> usize {
    debug_assert!(n >= 2);
    1 << (usize::BITS - 1 - (n - 1).leading_zeros())
}

#[cfg(test)]
mod test {
    use super::*;

    #[cfg(feature = "std")]
    #[test]
    #[should_panic(expected = "only plain hashers share a cohort")]
    fn a_shared_cohort_refuses_a_keyed_stage() {
        crate::initialize();
        let key = [7; crate::KEY_LEN];
        let mut hashers = [Hasher::new(Mode::Plain, None), Hasher::new(Mode::Keyed(&key), None)];
        for h in &mut hashers {
            h.stage(&[0; crate::STAGE_LEN]);
        }
        hash_cohort::<2>(&mut hashers, &[0, 1]);
    }

    /// Plain hashers always carry IV by `Domain` construction, and keyed or
    /// derive-key hashers beside them in the same turn never leak their keys
    /// into the plain cohort.
    #[cfg(feature = "std")]
    #[test]
    fn a_plain_cohort_keeps_iv_beside_keyed_hashers() {
        crate::initialize();
        let mut input = vec![0u8; 4 * crate::STAGE_LEN];
        crate::test::paint_test_input(&mut input);
        let key = [0x5a; 32];
        let mut hashers = [
            Hasher::new(Mode::Plain, None),
            Hasher::new(Mode::Keyed(&key), None),
            Hasher::new(Mode::Plain, None),
            Hasher::new(Mode::DeriveKey("cohort separation"), None),
        ];
        assert_eq!(hashers[0].domain().key(), crate::IV);
        assert_eq!(hashers[2].domain().key(), crate::IV);
        for piece in input.chunks(4096) {
            update_each(&mut hashers, &[(0, piece), (1, piece), (2, piece), (3, piece)]);
        }
        assert_eq!(hashers[0].clone().finalize(), crate::test::official::hash(Mode::Plain, &input));
        assert_eq!(hashers[1].clone().finalize(), crate::test::official::hash(Mode::Keyed(&key), &input));
        assert_eq!(hashers[2].clone().finalize(), crate::test::official::hash(Mode::Plain, &input));
        assert_eq!(hashers[3].clone().finalize(), crate::test::official::hash(Mode::DeriveKey("cohort separation"), &input));
        // Exercise `hash_cohort` directly on a shared plain cohort as well
        // (independent lanes handle mixed x86 AVX2/AVX-512 turns above).
        let mut plain_cohort = [Hasher::new(Mode::Plain, None), Hasher::new(Mode::Plain, None)];
        for h in &mut plain_cohort {
            h.stage(&input[..crate::STAGE_LEN]);
        }
        hash_cohort::<2>(&mut plain_cohort, &[0, 1]);
        for h in &plain_cohort {
            assert_eq!(h.clone().finalize(), crate::test::official::hash(Mode::Plain, &input[..crate::STAGE_LEN]));
        }
    }

    #[test]
    fn prepare_thread_reserves_collection_storage() {
        // nextest gives this test a fresh thread-local store.
        SHARED.with(|s| assert_eq!((s.borrow().messages.capacity(), s.borrow().cvs.capacity()), (0, 0)));
        prepare_thread();
        SHARED.with(|s| {
            let s = s.borrow();
            assert!(s.messages.capacity() >= PREPARED_MESSAGES);
            assert!(s.cvs.capacity() >= PREPARED_CVS);
            assert!(s.messages.is_empty() && s.cvs.is_empty());
        });
    }

    #[cfg(all(blake3_avx512_ffi, target_os = "linux", feature = "std"))]
    #[test]
    fn finalize_each_groups_only_full_output_lanes() {
        crate::initialize();
        let expected = crate::test::official::hash(Mode::Plain, b"output selector");
        for n in [0, 1, 2, 15, 16, 17, 31, 32, 33] {
            let mut hashers: Vec<_> = (0..n)
                .map(|_| {
                    let mut h = Hasher::new(Mode::Plain, None);
                    h.update(b"output selector");
                    h
                })
                .collect();
            let which: Vec<_> = (0..n).collect();
            let mut out = vec![[0u8; OUT_LEN]; n];
            crate::COUNTED.with(|c| c[crate::Counted::OutputGroupTurn as usize].set(0));
            finalize_each(&mut hashers, &which, &mut out);
            let turns = crate::COUNTED.with(|c| c[crate::Counted::OutputGroupTurn as usize].get());
            let expected_turns = u64::from(n >= 16 && matches!(Platform::detect(), Platform::AVX512));
            assert_eq!(turns, expected_turns, "full output group dispatch n={n}");
            assert!(out.iter().all(|digest| *digest == expected));
            assert!(hashers.iter().all(|h| h.count() == 0));
        }
    }

    #[test]
    #[should_panic(expected = "each finalized index is distinct")]
    fn finalize_each_rejects_repeated_indices() {
        crate::initialize();
        let mut hashers = [Hasher::new(Mode::Plain, None)];
        finalize_each(&mut hashers, &[0, 0], &mut [[0; OUT_LEN]; 2]);
    }

    /// Every path out of a finish clears the mark: a hasher can be named
    /// again in the next call, and one left marked would panic there.
    #[test]
    fn finalize_each_leaves_no_hasher_marked() {
        crate::initialize();
        for n in [1, 2, 3, 15, 16, 17, 33] {
            for len in [0, 1, 64, 1024, 1025, 5000, 40_000] {
                let mut hashers: Vec<_> = (0..n).map(|k| Hasher::new(if k % 3 == 2 { Mode::Keyed(&[7; 32]) } else { Mode::Plain }, None)).collect();
                let which: Vec<_> = (0..n).rev().collect();
                let mut out = vec![[0u8; OUT_LEN]; n];
                for round in 0..2 {
                    let message = vec![round as u8; len];
                    let pieces: Vec<(usize, &[u8])> = (0..n).map(|i| (i, &message[..])).collect();
                    update_each(&mut hashers, &pieces);
                    finalize_each(&mut hashers, &which, &mut out);
                    assert!(
                        hashers.iter_mut().all(|h| {
                            let was = h.mark_finishing();
                            h.clear_finishing();
                            !was
                        }),
                        "n={n} len={len}"
                    );
                    assert_eq!(out[n - 1], *crate::convenience::hash(&message).as_bytes(), "n={n} len={len}");
                }
            }
        }
    }

    #[test]
    #[should_panic(expected = "each finalized index is distinct")]
    fn finalize_each_rejects_repeats_across_groups() {
        crate::initialize();
        let mut hashers: Vec<_> = (0..16).map(|_| Hasher::new(Mode::Plain, None)).collect();
        let which: Vec<_> = (0..16).chain(0..16).collect();
        finalize_each(&mut hashers, &which, &mut [[0; OUT_LEN]; 32]);
    }

    #[test]
    #[should_panic(expected = "finalize_each names hasher 9 of 2")]
    fn finalize_each_refuses_an_index_that_names_no_hasher() {
        crate::initialize();
        let mut hashers = [Hasher::new(Mode::Plain, None), Hasher::new(Mode::Plain, None)];
        finalize_each(&mut hashers, &[0, 9], &mut [[0; OUT_LEN]; 2]);
    }

    /// A refused call leaves no mark: a program that catches the panic
    /// may name the same hashers in its next call.
    #[cfg(feature = "std")]
    #[test]
    fn a_refused_finalize_each_leaves_no_hasher_marked() {
        crate::initialize();
        for refused in [&[0usize, 1, 0][..], &[0, 1, 9]] {
            let mut hashers = [Hasher::new(Mode::Plain, None), Hasher::new(Mode::Plain, None)];
            update_each(&mut hashers, &[(0, b"abc"), (1, b"xyz")]);
            let call = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| finalize_each(&mut hashers, refused, &mut [[0; OUT_LEN]; 3])));
            assert!(call.is_err(), "{refused:?} was accepted");
            assert!(
                hashers.iter_mut().all(|h| {
                    let was = h.mark_finishing();
                    h.clear_finishing();
                    !was
                }),
                "{refused:?} left a mark"
            );
            let mut out = [[0; OUT_LEN]; 2];
            finalize_each(&mut hashers, &[1, 0], &mut out);
            assert_eq!(out, [*crate::convenience::hash(b"xyz").as_bytes(), *crate::convenience::hash(b"abc").as_bytes()]);
        }
    }

    /// A turn that panics part-way leaves no hasher holding a full stage:
    /// the hasher's next update has room, and its next turn ends.
    #[cfg(feature = "std")]
    #[test]
    fn a_turn_that_unwinds_leaves_no_stage_held() {
        crate::initialize();
        let bytes: Vec<u8> = (0..2 * crate::STAGE_LEN).map(|i| (i % 251) as u8).collect();
        let half = crate::STAGE_LEN / 2;
        let then = |hashers: &mut [Hasher; 2], after: &str| {
            assert!(hashers[0].held_group().is_none(), "{after}: hasher 0 is left holding a full stage");
            // What the turn fed hasher 0 before it stopped is in: the
            // stage that filled, and not the byte that waited behind it.
            assert_eq!(hashers[0].count(), crate::STAGE_LEN as u64, "{after}");
            hashers[0].update(b"y");
            hashers[1].reset();
            update_each(hashers, &[(0, b"z"), (1, b"abc")]);
            let mut fed = bytes[..crate::STAGE_LEN].to_vec();
            fed.extend_from_slice(b"yz");
            assert_eq!(hashers[0].finalize(), crate::convenience::hash(&fed), "{after}");
            assert_eq!(hashers[1].finalize(), crate::convenience::hash(b"abc"), "{after}");
        };

        // An index that names no hasher, after hasher 0's stage has filled.
        let mut hashers = [Hasher::new(Mode::Plain, None), Hasher::new(Mode::Plain, None)];
        let turn = [(0, &bytes[..half]), (0, &bytes[half..2 * half + 1]), (7, &bytes[..1])];
        let call = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| update_each(&mut hashers, &turn)));
        assert!(call.is_err(), "an index that names no hasher was accepted");
        then(&mut hashers, "an index out of range");
    }

    // Expected routes are independent of the implementation's predicate:
    // all supported ARM platforms, AVX2 and AVX-512, but not SSE4.1.
    fn block_groups_expected(platform: Platform) -> bool {
        #[cfg(blake3_sse41_ffi)]
        return !matches!(platform, Platform::SSE41);
        #[cfg(not(blake3_sse41_ffi))]
        {
            let _ = platform;
            true
        }
    }

    /// A collection of small items of mixed lengths (empty to a chunk, more
    /// than BY_BLOCKS_AT_ONCE of them, a long one among them): those of up
    /// to a chunk by their blocks where the platform takes them (a
    /// hash_many call counted), every digest hash()'s, held to a golden
    /// fold.
    #[test]
    fn test_hash_each_small_items_by_blocks() {
        let mut golden = crate::test::Golden::new("hash_each_small_items");
        let mut x = 0x9e37_79b9u64;
        for count in [2usize, 7, 63, 64, 65, 200] {
            let mut lens: Vec<usize> = (0..count)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    (x % (CHUNK_LEN as u64 + 1)) as usize
                })
                .collect();
            lens[count / 2] = 5 * CHUNK_LEN + 77;
            let data: Vec<Vec<u8>> = lens.iter().enumerate().map(|(m, &n)| (0..n).map(|i| ((m * 37 + i) % 249) as u8).collect()).collect();
            let items: Vec<&[u8]> = data.iter().map(|d| &d[..]).collect();
            let mut out = vec![[0u8; OUT_LEN]; count];
            for platform in crate::test::every_platform() {
                crate::COUNTED.with(|c| c[crate::Counted::ByBlocksCall as usize].set(0));
                hash_each_on(crate::IV, 0, &items, &mut out, platform, |item| crate::hash_serial(item, crate::IV, 0));
                let calls = crate::COUNTED.with(|c| c[crate::Counted::ByBlocksCall as usize].get());
                // A call where two or more items are of up to a chunk and one of
                // them has a whole block before its last.
                let small: Vec<usize> = lens.iter().copied().filter(|&n| n <= CHUNK_LEN).collect();
                #[cfg(blake3_neon)]
                let last_lanes = crate::neon_hybrid::sha3_detected();
                #[cfg(not(blake3_neon))]
                let last_lanes = false;
                let calls_expected: u64 = if block_groups_expected(platform) && small.len() >= 2 {
                    small
                        .chunks(BY_BLOCKS_AT_ONCE)
                        .map(|part| {
                            let mut buckets = [0u64; 16];
                            for &len in part {
                                buckets[len.saturating_sub(1) / crate::BLOCK_LEN] += 1;
                            }
                            buckets.into_iter().enumerate().filter(|&(k, _)| last_lanes || k > 0).map(|(_, n)| n.div_ceil(16)).sum::<u64>()
                        })
                        .sum()
                } else {
                    0
                };
                assert_eq!(calls, calls_expected, "{count} items on {platform:?}: exact block-group calls");
                let expected: Vec<[u8; OUT_LEN]> = items.iter().map(|i| *crate::convenience::hash(i).as_bytes()).collect();
                assert_eq!(out, expected, "{count} items");
            }
            golden.add(out.as_flattened(), || items.iter().flat_map(|i| crate::test::official::hash(Mode::Plain, i)).collect());
        }
        golden.check(0x8fa73f9dac40c559);
    }

    /// hash_each_on sends a message to `alone` exactly when it has
    /// ALONE_CHUNKS or more, or one chunk or fewer where the platform takes
    /// no items by their blocks (elsewhere two or more such go
    /// by their blocks), and gives the others' lanes as many rounds as the
    /// longest of them has whole chunks. Every other choice gives the same
    /// digests, so only the counts tell (BLAKE3 #219).
    #[test]
    fn test_hash_each_on_counts_its_alone_messages_and_rounds() {
        let input = vec![5u8; (ALONE_CHUNKS + 1) * CHUNK_LEN];
        let lens = [
            0,
            1,
            CHUNK_LEN,
            CHUNK_LEN + 1,
            2 * CHUNK_LEN,
            (ALONE_CHUNKS - 1) * CHUNK_LEN,
            (ALONE_CHUNKS - 1) * CHUNK_LEN + 1,
            ALONE_CHUNKS * CHUNK_LEN,
            ALONE_CHUNKS * CHUNK_LEN + 1,
        ];
        let items: Vec<&[u8]> = lens.iter().map(|&n| &input[..n]).collect();
        let alone = core::cell::RefCell::new(Vec::new());
        let mut out = vec![[0u8; OUT_LEN]; items.len()];
        crate::COUNTED.with(|c| c[crate::Counted::EachRound as usize].set(0));
        hash_each_on(crate::IV, 0, &items, &mut out, Platform::detect(), |item| {
            alone.borrow_mut().push(item.len());
            crate::hash_serial(item, crate::IV, 0)
        });
        let rounds = crate::COUNTED.with(|c| c[crate::Counted::EachRound as usize].get());
        let long = [(ALONE_CHUNKS - 1) * CHUNK_LEN + 1, ALONE_CHUNKS * CHUNK_LEN, ALONE_CHUNKS * CHUNK_LEN + 1];
        let expected: Vec<usize> = if block_groups_expected(Platform::detect()) { long.to_vec() } else { [0, 1, CHUNK_LEN].into_iter().chain(long).collect() };
        assert_eq!(alone.into_inner(), expected);
        // The shared: 1, 2 and ALONE_CHUNKS - 1 whole chunks.
        assert_eq!(rounds, (ALONE_CHUNKS - 1) as u64);
        for (item, out) in items.iter().zip(&out) {
            assert_eq!(out, crate::convenience::hash(item).as_bytes(), "{} bytes", item.len());
        }
    }

    /// A round with more lanes than one call to the kernels takes
    /// (LANES_AT_ONCE) goes in several calls, and every chunk value lands
    /// in its message's place: 2 * LANES_AT_ONCE + 10 messages of 2 to
    /// ALONE_CHUNKS - 1 chunks, short and long ones between them, hashed
    /// twice over (the thread's storage, used again), match the one-message
    /// hash, in as many rounds as the longest has whole chunks.
    #[test]
    fn test_hash_each_on_takes_a_round_of_many_lanes_in_several_calls() {
        crate::initialize();
        let mut input = vec![0u8; (ALONE_CHUNKS + 1) * CHUNK_LEN];
        crate::test::paint_test_input(&mut input);
        let shared_lens = [2 * CHUNK_LEN, 3 * CHUNK_LEN + 5, CHUNK_LEN + 1, 7 * CHUNK_LEN, (ALONE_CHUNKS - 1) * CHUNK_LEN - 3];
        let mut lens = Vec::new();
        for j in 0..2 * LANES_AT_ONCE + 10 {
            lens.push(shared_lens[j % shared_lens.len()]);
            if j % 7 == 0 {
                lens.extend([33, ALONE_CHUNKS * CHUNK_LEN + 9]);
            }
        }
        let items: Vec<&[u8]> = lens.iter().map(|&n| &input[..n]).collect();
        for pass in 0..2 {
            let mut out = vec![[0xAAu8; OUT_LEN]; items.len()];
            crate::COUNTED.with(|c| c[crate::Counted::EachRound as usize].set(0));
            hash_each_on(crate::IV, 0, &items, &mut out, Platform::detect(), |item| crate::hash_serial(item, crate::IV, 0));
            let rounds = crate::COUNTED.with(|c| c[crate::Counted::EachRound as usize].get());
            assert_eq!(rounds, (ALONE_CHUNKS - 2) as u64, "the longest holds fourteen whole chunks (pass {pass})");
            for (item, out) in items.iter().zip(&out) {
                assert_eq!(out, crate::convenience::hash(item).as_bytes(), "{} bytes (pass {pass})", item.len());
            }
        }
    }

    /// Turns in which several hashers hold a group each, under two keys
    /// whose flags agree, beyond their first group and at it (nonzero
    /// counters, and zero), on each platform: each held group's bytes
    /// land in its own hasher.
    #[test]
    fn test_update_each_holds_groups_together() {
        for platform in crate::test::every_platform() {
            holds_groups_together(platform);
        }
    }

    fn holds_groups_together(platform: crate::platform::Platform) {
        let mut input = vec![0u8; 3 * crate::STAGE_LEN];
        crate::test::paint_test_input(&mut input);
        let keys = [[1u8; 32], [2u8; 32]];
        let on = |mut h: Hasher| {
            h.set_platform_for_test(platform);
            h
        };
        let mut hashers: Vec<Hasher> = (0..6).map(|i| on(Hasher::new(crate::Mode::Keyed(&keys[i % 2]), None))).collect();
        // Each hasher a group in, its message at a different offset into the
        // input; then one turn of two pieces each, which stage its second
        // group from its start and fill it (held), and 7 bytes more.
        let start = |i: usize| i * 1000;
        let group = crate::STAGE_LEN;
        for (i, h) in hashers.iter_mut().enumerate() {
            h.update(&input[start(i)..start(i) + group]);
        }
        let turn: Vec<(usize, &[u8])> = (0..6)
            .flat_map(|i| [(i, &input[start(i) + group..start(i) + group + 100]), (i, &input[start(i) + group + 100..start(i) + 2 * group + 7])])
            .collect();
        crate::throughput::update_each(&mut hashers, &turn);
        for (i, h) in hashers.iter().enumerate() {
            let message = &input[start(i)..start(i) + 2 * crate::STAGE_LEN + 7];
            assert_eq!(h.clone().finalize(), crate::convenience::hash_with(crate::Mode::Keyed(&keys[i % 2]), message), "hasher {i}, {platform:?}");
        }
        // And each hasher's first group (chunk counter zero, which hands
        // its core the group's two halves), six at once: a first piece
        // longer than a chunk, so it is staged from offset zero.
        let mut fresh: Vec<Hasher> = (0..6).map(|i| on(Hasher::new(crate::Mode::Keyed(&keys[i % 2]), None))).collect();
        let turn: Vec<(usize, &[u8])> =
            (0..6).flat_map(|i| [(i, &input[start(i)..start(i) + 1100]), (i, &input[start(i) + 1100..start(i) + group + 7])]).collect();
        crate::throughput::update_each(&mut fresh, &turn);
        for (i, h) in fresh.iter().enumerate() {
            assert_eq!(
                h.clone().finalize(),
                crate::convenience::hash_with(crate::Mode::Keyed(&keys[i % 2]), &input[start(i)..start(i) + group + 7]),
                "fresh hasher {i}, {platform:?}"
            );
        }
    }

    /// A turn of many hashers keeps a fresh plain hasher's first chunk in its
    /// stage everywhere (for `finalize_each` SIMD batching), while a keyed
    /// hasher's first chunk goes to the core on x86-64 (for `output16`) and
    /// stays staged on AArch64; a single `update` stages everywhere.
    #[test]
    fn a_turn_stages_a_first_chunk_on_aarch64_and_an_update_everywhere() {
        crate::initialize();
        let bytes = [7u8; 300];
        let key = [0x42u8; 32];
        let mut hashers = vec![Hasher::new(Mode::Plain, None), Hasher::new(Mode::Plain, None), Hasher::new(Mode::Keyed(&key), None)];
        update_each(&mut hashers, &[(0, &bytes[..100]), (0, &bytes[100..]), (2, &bytes[..100]), (2, &bytes[100..])]);
        hashers[1].update(&bytes);
        assert_eq!((hashers[0].staged_len(), hashers[0].count() - hashers[0].staged_len() as u64), (300, 0), "a plain turn's first chunk stays staged");
        assert_eq!((hashers[1].staged_len(), hashers[1].count() - hashers[1].staged_len() as u64), (300, 0), "an update's first chunk waits in the stage");
        let keyed_in_turn = if cfg!(blake3_neon) { (300, 0) } else { (0, 300) };
        assert_eq!((hashers[2].staged_len(), hashers[2].count() - hashers[2].staged_len() as u64), keyed_in_turn, "a keyed turn's first chunk");
        assert_eq!(hashers[0].finalize(), crate::test::official::hash(Mode::Plain, &bytes));
        assert_eq!(hashers[1].finalize(), crate::test::official::hash(Mode::Plain, &bytes));
        assert_eq!(hashers[2].finalize(), crate::test::official::hash(Mode::Keyed(&key), &bytes));
    }

    /// x86-64 `finalize_each` exact dispatch boundaries for staged plain full
    /// chunks (`== 1024 B` -> `hash_many_rounded` when `>= 2`), staged plain
    /// multi-block short messages (`65..=1023 B` -> `hash_by_blocks` on AVX2/AVX-512
    /// when `>= 4` or `>= 2` with length `> 128 B`), `<= 64 B` remaining in `rest`,
    /// and 128-index window boundaries.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn x86_finalize_each_batches_staged_plain_chunks_and_short_blocks() {
        crate::initialize();
        let run_case = |lens: &[usize]| -> (u64, u64, bool) {
            let data: Vec<Vec<u8>> = lens.iter().enumerate().map(|(i, &n)| (0..n).map(|j| ((i * 41 + j * 13) & 0xff) as u8).collect()).collect();
            let mut hashers: Vec<_> = (0..lens.len()).map(|_| Hasher::new(Mode::Plain, None)).collect();
            for offset in (0..1024).step_by(256) {
                let pieces: Vec<_> =
                    data.iter().enumerate().filter(|(_, d)| offset < d.len()).map(|(i, d)| (i, &d[offset..(offset + 256).min(d.len())])).collect();
                if !pieces.is_empty() {
                    update_each(&mut hashers, &pieces);
                }
            }
            let which: Vec<usize> = (0..lens.len()).collect();
            let mut out = vec![[0u8; OUT_LEN]; lens.len()];
            crate::COUNTED.with(|c| {
                c[crate::Counted::HashManyCall as usize].set(0);
                c[crate::Counted::ByBlocksCall as usize].set(0);
            });
            crate::ROUNDED_WIDTH.with(|w| w.set(0));
            finalize_each(&mut hashers, &which, &mut out);
            for (d, got) in data.iter().zip(&out) {
                assert_eq!(*got, crate::test::official::hash(Mode::Plain, d), "len={}", d.len());
            }
            assert!(hashers.iter().all(|h| h.count() == 0));
            let rounded = crate::ROUNDED_WIDTH.with(|w| w.get()) != 0;
            (
                crate::COUNTED.with(|c| c[crate::Counted::HashManyCall as usize].get()),
                crate::COUNTED.with(|c| c[crate::Counted::ByBlocksCall as usize].get()),
                rounded,
            )
        };
        let block_lanes = matches!(Platform::detect(), Platform::AVX2 | Platform::AVX512);
        assert_eq!(run_case(&[1024]), (0, 0, false), "1 full chunk takes scalar root");
        assert_eq!(run_case(&[1024, 1024]), (1, 0, false), "2 full chunks call hash_many_rounded without padding");
        assert_eq!(run_case(&[1024; 3]), (1, 0, true), "3 full chunks round up via hash_many_rounded");
        assert_eq!(run_case(&[1024; 35]), (2, 0, true), "35 full chunks split into 32 + 3");
        assert_eq!(run_case(&[512]), (0, 0, false), "1 short plain message skips hash_by_blocks");
        assert_eq!(run_case(&[64, 128]), (0, 0, false), "64 B stays in rest so single 128 B skips hash_by_blocks");
        assert_eq!(run_case(&[64, 64, 64, 65]), (0, 0, false), "64 B stays in rest so lone 65 B skips hash_by_blocks");
        assert_eq!(run_case(&[65, 65]), (0, 0, false), "pair of 1-prefix-block 65 B messages stays scalar");
        assert_eq!(run_case(&[128, 128]), (0, 0, false), "pair of 128 B messages stays scalar");
        assert_eq!(run_case(&[65; 4]), (0, u64::from(block_lanes), false), "4 x 65 B batches via hash_by_blocks");
        assert_eq!(run_case(&[129, 129]), (0, u64::from(block_lanes), false), "pair of 129 B (2 prefix blocks) batches");
        assert_eq!(run_case(&[512, 512]), (0, u64::from(block_lanes), false), "pair of 512 B batches via hash_by_blocks");
        assert_eq!(run_case(&[1023, 1023]), (0, u64::from(block_lanes), false), "high endpoint 1023 B batches via hash_by_blocks");
        assert_eq!(run_case(&[65, 129, 512, 1023]), (0, if block_lanes { 4 } else { 0 }, false), "4 distinct prefix buckets batch each bucket");
        assert_eq!(run_case(&[512; 65]), (0, if block_lanes { 5 } else { 0 }, false), "65 short plain messages split across BY_BLOCKS_AT_ONCE");
        // Straddling the 128-index window: 128 full chunks (4 x 32) + 3 rounded full chunks in window 2.
        assert_eq!(run_case(&[1024; 131]), (5, 0, true), "131 full chunks cross the 128-index window and round window 2");
        // Straddling with a singleton in window 2 (128 full + 1 full): window 2 finishes scalar.
        assert_eq!(run_case(&[1024; 129]), (4, 0, false), "129 full chunks finish the 129th scalar in window 2");
    }

    /// Messages fed by turns and finished together match the official
    /// implementation at every length around a block, a chunk and a
    /// group, in every mode, in pieces short and long, and their hashers
    /// are reset: a second finish gives the empty message's hash.
    #[test]
    fn messages_fed_by_turns_finish_together_and_reset() {
        crate::initialize();
        let key = [37; 32];
        for n in [2, 16, 17, 64, 128, 130] {
            for len in [0, 1, 63, 64, 65, 255, 1023, 1024, 1025, 2048, 16385] {
                for width in [17, 256] {
                    for mode in [Mode::Plain, Mode::Keyed(&key), Mode::DeriveKey("grouped first chunk")] {
                        let data: Vec<Vec<u8>> = (0..n).map(|i| (0..len).map(|j| (i * 37 + j * 13) as u8).collect()).collect();
                        let mut hashers: Vec<_> = (0..n).map(|_| Hasher::new(mode, None)).collect();
                        for start in (0..len).step_by(width) {
                            let pieces: Vec<_> = data.iter().enumerate().map(|(i, d)| (i, &d[start..(start + width).min(len)])).collect();
                            update_each(&mut hashers, &pieces);
                        }
                        assert!(hashers.iter().all(|h| h.count() == len as u64));
                        let which: Vec<_> = (0..n).rev().collect();
                        let mut out = vec![[0; 32]; n];
                        finalize_each(&mut hashers, &which, &mut out);
                        for (&i, got) in which.iter().zip(&out) {
                            assert_eq!(*got, crate::test::official::hash(mode, &data[i]), "n={n}, len={len}, width={width}");
                        }
                        assert!(hashers.iter().all(|h| h.count() == 0));
                        finalize_each(&mut hashers, &which, &mut out);
                        assert!(out.iter().all(|got| *got == crate::test::official::hash(mode, &[])));
                    }
                }
            }
        }
    }

    /// AArch64: of the hashers one finish names, the plain messages of up
    /// to a chunk still whole in their stages go by their blocks together,
    /// whatever stands between them: a keyed message, a longer one, one
    /// that a single `update` sent to the core. Each of those finishes
    /// alone, every digest lands in its own place, and every hasher is
    /// reset. One such message alone takes no block-group call.
    #[cfg(blake3_neon)]
    #[test]
    fn a_finish_takes_the_staged_first_chunks_from_among_the_others() {
        crate::initialize();
        let key = [91u8; 32];
        // The first and third messages (plain, in every count below) hold
        // whole blocks before their last, so they take a group call on a
        // CPU without SHA-3 too, whose shortest messages take none.
        let lens = [511usize, 1, 1024, 64, 65, 0, 63];
        for n in [3usize, 17, 128, 200] {
            let data: Vec<Vec<u8>> =
                (0..n).map(|i| (0..if i % 5 == 3 { 3000 } else { lens[i % lens.len()] }).map(|j| (i * 37 + j * 13) as u8).collect()).collect();
            let mode = |i: usize| if i % 5 == 1 { Mode::Keyed(&key) } else { Mode::Plain };
            let mut hashers: Vec<_> = (0..n).map(|i| Hasher::new(mode(i), None)).collect();
            for (i, d) in data.iter().enumerate() {
                if i % 5 == 4 {
                    hashers[i].update(d);
                }
            }
            for from in (0..3000).step_by(17) {
                let pieces: Vec<_> =
                    data.iter().enumerate().filter(|(i, _)| i % 5 != 4).map(|(i, d)| (i, &d[from.min(d.len())..(from + 17).min(d.len())])).collect();
                update_each(&mut hashers, &pieces);
            }
            let which: Vec<_> = (0..n).rev().collect();
            let mut out = vec![[0; 32]; n];
            crate::COUNTED.with(|c| c[crate::Counted::ByBlocksCall as usize].set(0));
            finalize_each(&mut hashers, &which, &mut out);
            for (&i, got) in which.iter().zip(&out) {
                assert_eq!(*got, crate::test::official::hash(mode(i), &data[i]), "n={n}, message {i}");
            }
            assert!(crate::COUNTED.with(|c| c[crate::Counted::ByBlocksCall as usize].get()) > 0, "the staged first chunks went by their blocks");
            assert!(hashers.iter().all(|h| h.count() == 0));
        }
        // One staged first chunk beside a long message: nothing to group.
        let mut hashers = vec![Hasher::new(Mode::Plain, None), Hasher::new(Mode::Plain, None)];
        let long = [5u8; 3000];
        update_each(&mut hashers, &[(0, &long[..700]), (1, &long)]);
        let mut out = [[0; 32]; 2];
        crate::COUNTED.with(|c| c[crate::Counted::ByBlocksCall as usize].set(0));
        finalize_each(&mut hashers, &[0, 1], &mut out);
        assert_eq!(out, [crate::test::official::hash(Mode::Plain, &long[..700]), crate::test::official::hash(Mode::Plain, &long)]);
        assert_eq!(crate::COUNTED.with(|c| c[crate::Counted::ByBlocksCall as usize].get()), 0, "one message of up to a chunk finishes alone");
    }

    /// On SME2, a table of messages of one length, whole blocks, with
    /// SME2_SIZED_LEN of work or more, takes the message kernel in one
    /// call; a shorter table, or one with a message of another length,
    /// keeps the NEON lanes. The digests match the official implementation
    /// in every mode.
    #[cfg(blake3_sme2)]
    #[test]
    fn a_sized_table_of_whole_block_messages_takes_the_sme2_message_kernel() {
        if !crate::platform::sme2_detected() {
            return;
        }
        crate::initialize();
        let secret = [0x9du8; crate::KEY_LEN];
        let entry = crate::sme2::ffi::blake3_servil_sme2_hash16_messages_512 as *const () as usize;
        let ran = || crate::RAN_ENTRIES.with(|r| r.borrow().contains(&entry));
        for mode in [Mode::Plain, Mode::Keyed(&secret), Mode::DeriveKey("a table of whole blocks")] {
            let mut room = core::mem::MaybeUninit::uninit();
            let (key, flags) = mode.key(&mut room);
            // Every whole-block length of up to a chunk: each has its own
            // typed call of the kernel.
            for len in (64usize..=1024).step_by(64) {
                for count in [1usize, 15, 16, 17, 31, 32, 63, 64, 127, 128] {
                    let data: Vec<Vec<u8>> = (0..count).map(|i| (0..len).map(|j| (i * 37 + j * 11) as u8).collect()).collect();
                    let items: Vec<&[u8]> = data.iter().map(|d| &d[..]).collect();
                    let expected: Vec<_> = items.iter().map(|i| crate::test::official::hash(mode, i)).collect();
                    let mut out = vec![[0u8; OUT_LEN]; count];
                    crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
                    hash_by_blocks(&items, key, flags, &mut out, Platform::SME2);
                    assert_eq!(out, expected, "{count} x {len}");
                    assert_eq!(ran(), count * len >= crate::SME2_SIZED_LEN, "the message kernel at {count} x {len}");
                }
            }
            // One message a byte short: the table is no longer of one length.
            let mut data: Vec<Vec<u8>> = (0..128).map(|i| (0..1024).map(|j| (i * 37 + j * 11) as u8).collect()).collect();
            data[1].pop();
            let items: Vec<&[u8]> = data.iter().map(|d| &d[..]).collect();
            let expected: Vec<_> = items.iter().map(|i| crate::test::official::hash(mode, i)).collect();
            let mut out = vec![[0u8; OUT_LEN]; 128];
            crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
            hash_by_blocks(&items, key, flags, &mut out, Platform::SME2);
            assert_eq!(out, expected);
            assert!(!ran(), "the message kernel takes messages of one length");
        }
    }

    /// A piece that outlasts its hasher's stage waits for the turn's flush
    /// instead of forcing one: 256 plain hashers, each holding part of a
    /// chunk, each given a 16 KiB piece in caller order (the benchmark's
    /// interleaved cell), fill 256 stages that hash in two full passes of
    /// the table, with the rest of each piece gathered after. A flush at
    /// each outlasting piece would take a pass per hasher. The digests
    /// agree with the official implementation.
    #[cfg(feature = "std")]
    #[test]
    fn outlasting_pieces_wait_for_a_full_table() {
        crate::initialize();
        const N: usize = 256;
        let mut input = vec![0u8; 1448 + crate::STAGE_LEN];
        crate::test::paint_test_input(&mut input);
        let mut hashers: Vec<Hasher> = (0..N).map(|_| Hasher::new(Mode::Plain, None)).collect();
        let first: Vec<(usize, &[u8])> = (0..N).map(|i| (i, &input[..1448])).collect();
        update_each(&mut hashers, &first);
        let second: Vec<(usize, &[u8])> = (0..N).map(|i| (i, &input[1448..])).collect();
        crate::COUNTED.with(|c| c[crate::Counted::HeldPass as usize].set(0));
        update_each(&mut hashers, &second);
        let passes = crate::COUNTED.with(|c| c[crate::Counted::HeldPass as usize].get());
        assert_eq!(passes, (N / TABLE) as u64, "every stage hashed in full passes of the table");
        for h in &hashers {
            assert_eq!(h.clone().finalize(), crate::test::official::hash(Mode::Plain, &input));
        }
        // Two pieces of one hasher in one turn apply in order: the second
        // waits while the first's rest does.
        let mut pair = vec![Hasher::new(Mode::Plain, None), Hasher::new(Mode::Plain, None)];
        let (a, b) = input.split_at(crate::STAGE_LEN + 100);
        update_each(&mut pair, &[(0, a), (1, &input[..7]), (0, b), (1, &input[7..])]);
        assert_eq!(pair[0].clone().finalize(), crate::test::official::hash(Mode::Plain, &input));
        assert_eq!(pair[1].clone().finalize(), crate::test::official::hash(Mode::Plain, &input));
    }

    /// Public mode and independent-parent availability choose the held
    /// passes; equal and distinct secret keys always take the same work.
    #[cfg(feature = "std")]
    #[test]
    fn held_groups_pass_by_mode_not_key() {
        crate::initialize();
        let mut input = vec![0u8; crate::STAGE_LEN];
        crate::test::paint_test_input(&mut input);
        let (a, b, c) = ([1u8; 32], [2u8; 32], [3u8; 32]);
        let passes = |modes: &[Mode]| {
            let mut hashers: Vec<Hasher> = modes.iter().map(|&m| Hasher::new(m, None)).collect();
            // Pieces shorter than a group, so each hasher's group fills in
            // its stage and is held for the turn.
            let pieces: Vec<(usize, &[u8])> = input.chunks(4096).flat_map(|piece| (0..modes.len()).map(move |i| (i, piece))).collect();
            crate::COUNTED.with(|c| c[crate::Counted::HeldPass as usize].set(0));
            update_each(&mut hashers, &pieces);
            let passes = crate::COUNTED.with(|c| c[crate::Counted::HeldPass as usize].get());
            for (h, &m) in hashers.iter().zip(modes) {
                assert_eq!(h.clone().finalize(), crate::test::official::hash(m, &input));
            }
            passes
        };
        #[cfg(any(all(blake3_avx512_ffi, target_os = "linux"), blake3_neon))]
        let independent = crate::ffi_output::lanes(Platform::detect()) != 0;
        #[cfg(not(any(all(blake3_avx512_ffi, target_os = "linux"), blake3_neon)))]
        let independent = false;
        assert_eq!(passes(&[Mode::Plain; 3]), 1, "plain hashers hash together");
        assert_eq!(passes(&[Mode::Plain, Mode::Keyed(&a), Mode::Plain]), if independent { 1 } else { 2 }, "public mixed-mode route");
        let same = passes(&[Mode::Keyed(&a); 3]);
        assert_eq!(same, passes(&[Mode::Keyed(&a), Mode::Keyed(&b), Mode::Keyed(&c)]), "equal keys take the passes distinct keys take");
        assert_eq!(same, if independent { 1 } else { 3 });
        // Two held hashers stay below the lanes' count: each keyed one alone.
        assert_eq!(passes(&[Mode::Keyed(&a), Mode::Keyed(&b)]), 2, "a pair of keyed hashers, on either route");
        assert_eq!(passes(&[Mode::Keyed(&a); 2]), 2, "equal keys take the same two passes");
        let derived = passes(&[Mode::DeriveKey("shared context"); 3]);
        assert_eq!(derived, passes(&[Mode::DeriveKey("context a"), Mode::DeriveKey("context b"), Mode::DeriveKey("context c")]));
        assert_eq!(derived, if independent { 1 } else { 3 }, "derived-key equality stays private");
        // Public routing and the ready-table bound fix the pass count.
        let bounded = 600usize.div_ceil(TABLE) as u64;
        assert_eq!(passes(&[Mode::Keyed(&a); 600]), if independent { bounded } else { 600 }, "600 keyed hashers, bounded public route");
        assert_eq!(passes(&[Mode::Plain; 600]), bounded, "600 plain hashers, bounded passes");
    }

    /// Parent rows cross messages and the ready-table boundary. Distinct
    /// input bytes make a pointer borrowed from the wrong message visible.
    #[test]
    fn held_workspace_preserves_distinct_message_rows() {
        crate::initialize();
        for n in [2usize, 15, 16, 17, 31, 32, 33, 127, 128, 129, 257] {
            let count = n.min(TABLE);
            let mut workspace = HeldWorkspace::<TABLE>::new();
            let (level, next) = workspace.buffers(count);
            assert_eq!((level.len(), next.len()), (16 * count, 8 * count), "workspace CV capacities");
            let inputs: Vec<Vec<u8>> =
                (0..n).map(|i| (0..2 * crate::STAGE_LEN + i % 71).map(|j| ((i * 37 + j * 17 + (j >> 8)) % 251) as u8).collect()).collect();
            let keys: Vec<[u8; 32]> = (0..n).map(|i| core::array::from_fn(|j| (i * 37 + j * 11) as u8)).collect();
            for flavor in 0..4 {
                let mode = |i: usize| match flavor {
                    0 => Mode::Plain,
                    1 => Mode::Keyed(&keys[0]),
                    2 => Mode::Keyed(&keys[i]),
                    _ => match i % 3 {
                        0 => Mode::Plain,
                        1 => Mode::Keyed(&keys[i]),
                        _ => Mode::DeriveKey("distinct held rows"),
                    },
                };
                let mut hashers: Vec<_> = (0..n).map(|i| Hasher::new(mode(i), None)).collect();
                let mut pieces = Vec::with_capacity(n);
                for start in (0..2 * crate::STAGE_LEN).step_by(4096) {
                    pieces.clear();
                    pieces.extend(inputs.iter().enumerate().map(|(i, b)| (i, &b[start..start + 4096])));
                    update_each(&mut hashers, &pieces);
                    assert!(hashers.iter().all(|h| h.held_group().is_none()), "each turn completes its held stages");
                }
                for (i, (h, input)) in hashers.iter_mut().zip(&inputs).enumerate() {
                    h.update(&input[2 * crate::STAGE_LEN..]);
                    assert_eq!(h.clone().finalize(), crate::test::official::hash(mode(i), input), "n={n} flavor={flavor} row={i}");
                }
            }
        }
    }

    /// With public shape fixed, changing key equality or the number of
    /// matching leading words changes neither cohorts nor kernel entries.
    /// This checks dispatch, not instruction-level constant-time execution.
    #[cfg(feature = "std")]
    #[test]
    fn held_dispatch_ignores_key_equality_and_prefixes() {
        crate::initialize();
        let input = vec![42u8; 2 * crate::STAGE_LEN];
        for platform in crate::test::every_platform() {
            for n in [3usize, 8, 16, 17, 33] {
                for mixed in [false, true] {
                    let mut reference = None;
                    // Eight words means identical keys. Otherwise exactly
                    // `prefix` leading words match between every pair.
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
                        let mut hashers: Vec<_> = (0..n)
                            .map(|i| {
                                let mut h = Hasher::new(mode(i), None);
                                h.set_platform_for_test(platform);
                                h
                            })
                            .collect();
                        HELD_GROUPS.with(|g| g.borrow_mut().clear());
                        crate::RAN_ENTRIES.with(|e| e.borrow_mut().clear());
                        crate::COUNTED.with(|c| c[crate::Counted::HeldPass as usize].set(0));
                        for piece in input.chunks(4096) {
                            let pieces: Vec<_> = (0..n).map(|i| (i, piece)).collect();
                            update_each(&mut hashers, &pieces);
                        }
                        // Capture before finalization and the official oracle.
                        let groups = HELD_GROUPS.with(|g| g.borrow().clone());
                        let entries = crate::RAN_ENTRIES.with(|e| e.borrow().clone());
                        let passes = crate::COUNTED.with(|c| c[crate::Counted::HeldPass as usize].get());
                        assert_eq!(groups.iter().map(Vec::len).sum::<usize>(), 2 * n, "both held stages observed");
                        let work = (groups, entries, passes);
                        if let Some(expected) = &reference {
                            assert_eq!(
                                &work, expected,
                                "dispatch work independent of key equality and prefix: {platform:?} n={n} mixed={mixed} prefix={prefix}"
                            );
                        } else {
                            reference = Some(work);
                        }
                        for (i, h) in hashers.iter().enumerate() {
                            assert_eq!(h.clone().finalize(), crate::test::official::hash(mode(i), &input));
                        }
                    }
                }
            }
        }
    }

    /// Keyed and mixed-mode turns use independent parent lanes; plain
    /// turns share the fixed IV. Key values never select a different route.
    #[cfg(any(all(blake3_avx512_ffi, target_os = "linux"), blake3_neon))]
    #[test]
    fn test_held_key_groups_take_one_partition_pass() {
        let entry = |width| if width == 8 { crate::ffi_output::entry8() } else { crate::ffi_output::entry() };
        for platform in crate::test::every_platform() {
            let width = crate::ffi_output::lanes(platform);
            if width == 0 {
                continue;
            }
            for n in [2usize, 7, 8, 9, 15, 16, 17, 31, 32, 33] {
                for flavor in 0..4 {
                    let keys: Vec<[u8; 32]> = (0..n).map(|i| core::array::from_fn(|j| (i * 37 + j * 11) as u8)).collect();
                    let modes: Vec<Mode<'_>> = (0..n)
                        .map(|i| match flavor {
                            0 => Mode::Plain,
                            1 => Mode::Keyed(&keys[0]),
                            2 => Mode::Keyed(&keys[i]),
                            _ => match i % 3 {
                                0 => Mode::Plain,
                                1 => Mode::Keyed(&keys[i]),
                                _ => Mode::DeriveKey("held parent lanes"),
                            },
                        })
                        .collect();
                    let offsets: Vec<_> = (0..n).map(|i| (i % 3) * crate::STAGE_LEN).collect();
                    let data: Vec<Vec<u8>> =
                        (0..n).map(|i| (0..offsets[i] + crate::STAGE_LEN + 97).map(|j| ((i * 37 + j * 17) % 251) as u8).collect()).collect();
                    let mut hashers: Vec<_> = (0..n)
                        .map(|i| {
                            let mut h = Hasher::new(modes[i], None);
                            h.set_platform_for_test(platform);
                            h.update(&data[i][..offsets[i]]);
                            h
                        })
                        .collect();
                    crate::COUNTED.with(|c| c[crate::Counted::HeldPass as usize].set(0));
                    crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
                    for start in (0..crate::STAGE_LEN).step_by(4096) {
                        let pieces: Vec<_> = (0..n).map(|i| (i, &data[i][offsets[i] + start..offsets[i] + start + 4096])).collect();
                        update_each(&mut hashers, &pieces);
                    }
                    let parent_entries = crate::RAN_ENTRIES.with(|r| r.borrow().clone());
                    for i in 0..n {
                        let end = offsets[i] + crate::STAGE_LEN;
                        assert_eq!(hashers[i].staged_len(), 0);
                        assert_eq!(hashers[i].count(), end as u64);
                        assert_eq!(
                            *hashers[i].clone().finalize().as_bytes(),
                            crate::test::official::hash(modes[i], &data[i][..end]),
                            "{platform:?} n={n} lane={i} flavor={flavor}"
                        );
                        hashers[i].update(&data[i][end..]);
                        assert_eq!(*hashers[i].clone().finalize().as_bytes(), crate::test::official::hash(modes[i], &data[i]));
                        let mut reader = hashers[i].clone().finalize_xof();
                        reader.set_position(13);
                        let mut out = [0u8; 129];
                        reader.fill(&mut out);
                        assert_eq!(out.as_slice(), crate::test::official::xof(modes[i], &data[i], 13, out.len()));
                    }
                    let partitions = crate::COUNTED.with(|c| c[crate::Counted::HeldPass as usize].get());
                    // Below INDEPENDENT_MIN a keyed hasher hashes alone, and the plain ones together.
                    let lanes = flavor != 0 && n >= INDEPENDENT_MIN;
                    assert_eq!(partitions, if flavor != 0 && !lanes { n as u64 } else { 1 }, "held partitions: {platform:?} n={n} flavor={flavor}");
                    #[cfg(all(blake3_avx512_ffi, target_os = "linux"))]
                    {
                        assert_eq!(parent_entries.contains(&(entry(width) as usize)), lanes, "independent parent entry: {platform:?} n={n}");
                        if lanes && platform == crate::platform::Platform::AVX512 && crate::platform::avx2_detected() {
                            let eight_needed = (1..=4).any(|shift| ((n * (crate::STAGE_LEN / crate::CHUNK_LEN)) >> shift) % 16 >= 8);
                            assert_eq!(
                                parent_entries.contains(&(entry(8) as usize)),
                                eight_needed,
                                "eight-wide parent remainder: {platform:?} n={n} flavor={flavor}"
                            );
                        }
                    }
                    #[cfg(blake3_neon)]
                    {
                        #[cfg(blake3_sme2)]
                        if matches!(platform, crate::platform::Platform::SME2) {
                            assert_eq!(
                                parent_entries.contains(&(crate::ffi_output::entry_chunks_keyed() as usize)),
                                lanes,
                                "SME2 chunks_keyed entry: n={n} flavor={flavor}"
                            );
                        }
                        assert_eq!(parent_entries.contains(&(entry(8) as usize)), lanes, "NEON output8 parent entry: {platform:?} n={n} flavor={flavor}");
                    }
                }
            }
        }
    }

    /// Large same-key and distinct-key turns take the same passes. A
    /// platform without independent parent lanes uses public-mode cohorts.
    #[cfg(feature = "std")]
    #[test]
    fn held_passes_do_not_depend_on_keys() {
        crate::initialize();
        let input = vec![42u8; crate::STAGE_LEN];
        let keys: Vec<[u8; 32]> = (0..600u64)
            .map(|i| {
                let mut key = [7u8; 32];
                key[..8].copy_from_slice(&i.to_le_bytes());
                key
            })
            .collect();
        for platform in crate::test::every_platform() {
            #[cfg(any(all(blake3_avx512_ffi, target_os = "linux"), blake3_neon))]
            let independent = crate::ffi_output::lanes(platform) != 0;
            #[cfg(not(any(all(blake3_avx512_ffi, target_os = "linux"), blake3_neon)))]
            let independent = false;
            for n in [3, 600] {
                for flavor in 0..3 {
                    let mode = |i| match flavor {
                        0 => Mode::Plain,
                        1 => Mode::Keyed(&keys[0]),
                        _ => Mode::Keyed(&keys[i]),
                    };
                    let mut hashers: Vec<_> = (0..n)
                        .map(|i| {
                            let mut h = Hasher::new(mode(i), None);
                            h.set_platform_for_test(platform);
                            h
                        })
                        .collect();
                    crate::COUNTED.with(|c| c[crate::Counted::HeldPass as usize].set(0));
                    for piece in input.chunks(4096) {
                        let pieces: Vec<_> = (0..n).map(|i| (i, piece)).collect();
                        update_each(&mut hashers, &pieces);
                    }
                    let passes = crate::COUNTED.with(|c| c[crate::Counted::HeldPass as usize].get());
                    assert_eq!(
                        passes,
                        if independent || flavor == 0 { n.div_ceil(TABLE) as u64 } else { n as u64 },
                        "key-independent passes: {platform:?} n={n} flavor={flavor}"
                    );
                    for (i, h) in hashers.iter().enumerate() {
                        assert_eq!(h.clone().finalize(), crate::test::official::hash(mode(i), &input));
                    }
                }
            }
        }
    }

    #[test]
    fn test_update_each() {
        let mut input = vec![0u8; 1 << 20];
        crate::test::paint_test_input(&mut input);
        let lens = [0, 1, 63, 64, 1023, 1024, 1025, 2048, 4470, 8191, 16384, 16385, 65536, 100_000, 300_000];
        let piece_lens = [1, 1448, 4096, 1448, 16384, 31, 65536];
        let key = [5u8; 32];
        // Every third hasher keyed: a turn's held groups in two kinds.
        let mut hashers: Vec<Hasher> =
            (0..lens.len()).map(|i| if i % 3 == 1 { Hasher::new(crate::Mode::Keyed(&key), None) } else { Hasher::new(crate::Mode::Plain, None) }).collect();
        let mut done = vec![0usize; lens.len()];
        let mut k = 0;
        while done.iter().zip(&lens).any(|(d, l)| d < l) {
            let mut pieces: Vec<(usize, &[u8])> = Vec::new();
            for _ in 0..23 {
                let i = k % lens.len();
                let n = piece_lens[k % piece_lens.len()].min(lens[i] - done[i]);
                pieces.push((i, &input[done[i]..done[i] + n]));
                done[i] += n;
                k += 1;
            }
            crate::throughput::update_each(&mut hashers, &pieces);
        }
        let which: Vec<usize> = (0..lens.len()).rev().collect();
        let mut out = vec![[0u8; OUT_LEN]; lens.len()];
        crate::throughput::finalize_each(&mut hashers, &which, &mut out);
        for (k, &i) in which.iter().enumerate() {
            let want = if i % 3 == 1 {
                crate::convenience::hash_with(crate::Mode::Keyed(&key), &input[..lens[i]])
            } else {
                crate::convenience::hash(&input[..lens[i]])
            };
            assert_eq!(&out[k], want.as_bytes(), "len {}", lens[i]);
        }
    }

    /// Every length around chunk and lane boundaries, in every mode, mixed in
    /// one call, against `hash_with` on each.
    /// Keep: it detects, by hanging, mutants the clock then stops
    /// (tools/mutants/timeouts-x86_64.txt).
    #[test]
    fn test_hash_each_matches_hash_with() {
        let mut input = vec![0u8; 40 * CHUNK_LEN];
        crate::test::paint_test_input(&mut input);
        let mut lens = vec![
            0,
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
            3000,
            4096,
            5000,
            15 * CHUNK_LEN,
            15 * CHUNK_LEN + 1,
            16 * CHUNK_LEN - 1,
            16 * CHUNK_LEN,
            16 * CHUNK_LEN + 1,
            40 * CHUNK_LEN,
        ];
        for n in 2..16 {
            lens.push(n * CHUNK_LEN);
            lens.push(n * CHUNK_LEN - 7);
        }
        let items: Vec<&[u8]> = lens.iter().enumerate().map(|(j, &l)| &input[j % 3..j % 3 + l.min(input.len() - 2)]).collect();
        let key = [9u8; 32];
        for mode in [Mode::Plain, Mode::Keyed(&key), Mode::DeriveKey("hash_each test")] {
            let mut out = vec![[0u8; OUT_LEN]; items.len()];
            crate::throughput::hash_each(mode, &crate::test::items(&crate::test::messages(&items), &items), &mut out);
            for (item, digest) in items.iter().zip(&out) {
                assert_eq!(digest, crate::convenience::hash_with(mode, item).as_bytes(), "length {}", item.len());
            }
        }
    }

    /// Calling `hash_each` (and `prepare_thread`) from an earlier-initialized
    /// thread-local destructor after `SHARED` has already been destroyed must
    /// not panic and must produce exact digests for both empty and multi-chunk
    /// collections.
    #[test]
    fn hash_each_works_during_thread_local_teardown() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        crate::initialize();
        let ran = Arc::new(AtomicBool::new(false));
        let ok = Arc::new(AtomicBool::new(false));
        struct OnExit {
            ran: Arc<AtomicBool>,
            ok: Arc<AtomicBool>,
        }
        impl Drop for OnExit {
            fn drop(&mut self) {
                self.ran.store(true, Ordering::SeqCst);
                let tree = crate::throughput::TreeHasher::new(Mode::Plain);
                // Empty collection during teardown.
                let mut empty: [[u8; OUT_LEN]; 0] = [];
                tree.hash_each(&[], &mut empty);
                // Middle-sized multi-chunk collection during teardown (forces
                // the `SHARED.try_with` fallback once `SHARED` is destroyed).
                let msg_a = [0x11u8; 2 * CHUNK_LEN + 7];
                let msg_b = [0x22u8; 3 * CHUNK_LEN + 13];
                let slices = [&msg_a[..], &msg_b[..]];
                let blocks = crate::test::messages(&slices);
                let items = crate::test::items(&blocks, &slices);
                let mut out = [[0u8; OUT_LEN]; 2];
                prepare_thread();
                tree.hash_each(&items, &mut out);
                let matched = out[0] == *blake3::hash(&msg_a).as_bytes() && out[1] == *blake3::hash(&msg_b).as_bytes();
                self.ok.store(matched, Ordering::SeqCst);
            }
        }
        std::thread_local! {
            static EARLIER: core::cell::RefCell<Option<OnExit>> = const { core::cell::RefCell::new(None) };
        }
        let ran_c = Arc::clone(&ran);
        let ok_c = Arc::clone(&ok);
        std::thread::spawn(move || {
            // Initialize EARLIER first so LIFO TLS teardown destroys SHARED
            // before EARLIER's Drop runs.
            EARLIER.with(|slot| *slot.borrow_mut() = Some(OnExit { ran: ran_c, ok: ok_c }));
            // Now initialize SHARED via a middle-sized hash_each call.
            let msg = [0x33u8; 2 * CHUNK_LEN];
            let slices = [&msg[..]];
            let blocks = crate::test::messages(&slices);
            let items = crate::test::items(&blocks, &slices);
            let mut out = [[0u8; OUT_LEN]; 1];
            crate::throughput::hash_each(Mode::Plain, &items, &mut out);
            assert_eq!(out[0], *blake3::hash(&msg).as_bytes());
        })
        .join()
        .expect("thread teardown with hash_each must not panic");
        assert!(ran.load(Ordering::SeqCst), "TLS destructor must have run");
        assert!(ok.load(Ordering::SeqCst), "TLS destructor digests must match");
    }

    /// `hash_held` picks the smallest `hash_cohort_on::<N>` workspace tier (1
    /// for a singleton, 16-stage L1D-hot tiles for 2..=128) for plain cohorts
    /// across every `std` platform.
    #[cfg(feature = "std")]
    #[test]
    fn hash_held_selects_smallest_cohort_tier() {
        crate::initialize();
        let data = std::vec![0x42u8; crate::STAGE_LEN + 17];
        let expected = *blake3::hash(&data).as_bytes();
        for (count, expected_tier) in [(1usize, 1usize), (2, 16), (16, 16), (17, 16), (33, 16)] {
            let mut hashers: std::vec::Vec<Hasher> = (0..count).map(|_| Hasher::new(Mode::Plain, None)).collect();
            LAST_COHORT_TIER.with(|t| t.set(0));
            for piece in data.chunks(4096) {
                let pieces: std::vec::Vec<(usize, &[u8])> = (0..count).map(|i| (i, piece)).collect();
                update_each(&mut hashers[..], &pieces);
            }
            assert_eq!(LAST_COHORT_TIER.with(|t| t.get()), expected_tier, "count={count}");
            assert_eq!(*hashers[0].finalize().as_bytes(), expected);
        }
    }

    /// Feeding plain (including 17 and 33 hashers fed sixteen 1 KiB pieces) or
    /// mixed keyed+plain (3..=16) cohorts of boxed hashers through a 16 KiB
    /// stage in `update_each` uses bounded 1/16-stage workspaces (~1.5–15 KiB)
    /// rather than a 116 KiB 128-stage frame, so all succeed on a 64 KiB worker
    /// thread stack.
    #[cfg(any(target_arch = "x86_64", target_os = "macos"))]
    #[test]
    fn small_held_cohorts_fit_in_64kib_stack() {
        crate::initialize();
        let data = std::vec![0x42u8; crate::STAGE_LEN + 97];
        let expected_plain = *blake3::hash(&data).as_bytes();
        let key = [0x5Au8; 32];
        let expected_keyed = *blake3::keyed_hash(&key, &data).as_bytes();
        for (count, keyed_first, piece_len) in
            [(1usize, false, 4096usize), (2, false, 4096), (3, true, 4096), (16, true, 4096), (17, false, 1024), (33, false, 1024)]
        {
            let data_c = data.clone();
            let mut hashers: std::vec::Vec<Hasher> =
                (0..count).map(|i| if keyed_first && i == 0 { Hasher::new(Mode::Keyed(&key), None) } else { Hasher::new(Mode::Plain, None) }).collect();
            let (first, last) = std::thread::Builder::new()
                .stack_size(64 * 1024)
                .spawn(move || {
                    for piece in data_c.chunks(piece_len) {
                        let pieces: std::vec::Vec<(usize, &[u8])> = (0..count).map(|i| (i, piece)).collect();
                        update_each(&mut hashers[..], &pieces);
                    }
                    (*hashers[0].finalize().as_bytes(), *hashers[count - 1].finalize().as_bytes())
                })
                .expect("spawn 64 KiB thread")
                .join()
                .expect("held cohort must fit in 64 KiB stack");
            assert_eq!(first, if keyed_first { expected_keyed } else { expected_plain }, "cohort of {count} (keyed={keyed_first}) first");
            assert_eq!(last, expected_plain, "cohort of {count} (keyed={keyed_first}) last");
        }
    }

    /// Every middle chunk count (`2..=15`, exact and partial tails `1, 63, 64,
    /// 65, 512, 1023 B`), collection counts spanning singleton (`1`), small
    /// (`2`, `3`), and over-full (`LANES_AT_ONCE + 5`, exercising mid-round
    /// parent flushes and odd-leaf compaction), and all three `Mode`s match the
    /// reference implementation and exercise batched partial-chunk and
    /// parent/root rounds.
    #[cfg(feature = "std")]
    #[test]
    fn hash_each_middle_all_chunk_counts_partial_tails_and_modes() {
        crate::initialize();
        let key = [0x3cu8; crate::KEY_LEN];
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut fill = |n: usize| -> std::vec::Vec<u8> {
            (0..n)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    x as u8
                })
                .collect()
        };

        // Mixed collection with every chunk count 2..=15 (both exact and ragged
        // partial tails) plus empty, small (<= 1 KiB), and large (>= 16 KiB)
        // items, repeated past LANES_AT_ONCE so both partial-chunk flushes and
        // mid-round parent flushes with odd trailing leaves trigger.
        let tails = [0usize, 1, 63, 64, 65, 512, 1023];
        let mut raw: std::vec::Vec<std::vec::Vec<u8>> = std::vec![fill(0), fill(1), fill(64), fill(1024), fill(16 * CHUNK_LEN), fill(16 * CHUNK_LEN + 37)];
        for chunks in 2..ALONE_CHUNKS {
            for &tail in &tails {
                let len = if tail == 0 { chunks * CHUNK_LEN } else { (chunks - 1) * CHUNK_LEN + tail };
                raw.push(fill(len));
            }
        }
        // Extra 5-chunk ragged items past LANES_AT_ONCE to stress flush + odd compaction.
        while raw.len() < LANES_AT_ONCE + 20 {
            raw.push(fill(4 * CHUNK_LEN + 333));
        }
        let slices: std::vec::Vec<&[u8]> = raw.iter().map(|v| v.as_slice()).collect();
        let blocks = crate::test::messages(&slices);
        let items = crate::test::items(&blocks, &slices);
        for (m, mode) in [Mode::Plain, Mode::Keyed(&key), Mode::DeriveKey("each middle simd")].into_iter().enumerate() {
            let mut out = std::vec![[0u8; OUT_LEN]; items.len()];
            crate::throughput::hash_each(mode, &items, &mut out);
            for (i, s) in slices.iter().enumerate() {
                assert_eq!(out[i], crate::test::official::hash(mode, s), "mode={m} item {i} len={}", s.len());
            }
        }

        // Singleton middle message (`messages.len() == 1`) and 15-chunk multi-round
        // reduction (`15 -> 8 -> 4 -> 2 -> 1`, 3 non-root parent rounds + 1 root round).
        #[cfg(target_arch = "x86_64")]
        let block_lanes = crate::pair::takes_platform(Platform::detect());
        #[cfg(target_arch = "aarch64")]
        let block_lanes = true;
        #[cfg(blake3_neon)]
        let last_lanes = crate::neon_hybrid::sha3_detected();
        #[cfg(not(blake3_neon))]
        let last_lanes = false;

        for count in [1usize, 2, 3, 17] {
            let bufs: std::vec::Vec<std::vec::Vec<u8>> = (0..count).map(|k| fill(14 * CHUNK_LEN + 100 + k * 7)).collect();
            let s: std::vec::Vec<&[u8]> = bufs.iter().map(|v| v.as_slice()).collect();
            let b = crate::test::messages(&s);
            let it = crate::test::items(&b, &s);
            let mut out = std::vec![[0u8; OUT_LEN]; count];
            crate::COUNTED.with(|c| {
                c[crate::Counted::ByBlocksCall as usize].set(0);
                c[crate::Counted::HashManyCall as usize].set(0);
            });
            crate::throughput::hash_each(Mode::Plain, &it, &mut out);
            for (i, msg) in s.iter().enumerate() {
                assert_eq!(out[i], crate::test::official::hash(Mode::Plain, msg), "15-chunk count={count} i={i}");
            }
            let by_blocks = crate::COUNTED.with(|c| c[crate::Counted::ByBlocksCall as usize].get());
            let hash_many = crate::COUNTED.with(|c| c[crate::Counted::HashManyCall as usize].get());
            if count == 1 {
                assert_eq!(by_blocks, 0, "single partial tail skips hash_chunks_by_blocks");
                assert_eq!(hash_many, 14, "single 15-chunk message runs 14 chunk rounds only");
            } else {
                // 14 full-chunk rounds + 3 parent rounds (15->8, 8->4, 4->2) + 1 root round.
                assert_eq!(hash_many, 18, "count={count} 15-chunk messages batch all 3 parent + 1 root rounds");
                if block_lanes {
                    assert!(by_blocks > 0, "count={count} >= 2 shared-prefix partial tails batch via hash_chunks_by_blocks");
                }
            }
        }

        // Partial-tail boundary & exact group-width checks at counter 1 & 2:
        // - incompatible prefix lengths (1,127 B = k=1, 1,191 B = k=2) skip SIMD
        // - block-boundary pair (1024+511, 1024+512 -> both k=7) runs 1 SIMD call
        // - adjacent-boundary pair (1024+512 -> k=7, 1024+513 -> k=8) skips SIMD
        // - k=0 pair plus singleton k=1 (1024+32, 1024+32, 1024+103) skips SIMD on !last_lanes
        //   (kills `k > 0 -> k >= 0`) and runs 2 calls (k=0 + k=1) on SHA-3 NEON
        // - exact c-chunk message placed LAST alongside shared & single counter-c tails
        //   (exercises `item.len() > c * CHUNK_LEN` in both branches; `>=` panics OOB)
        // - BY_BLOCKS_AT_ONCE + 1 identical-k items at counter 1 exercises `slices.is_full()`
        //   (64/16 = 4 calls in first flush + 1 call in trailing flush = 5 calls).
        let partial_cases: &[(&str, &[usize], u64)] = &[
            ("incompatible k=1,2", &[CHUNK_LEN + 103, CHUNK_LEN + 167], 0),
            ("shared k=1 pair", &[CHUNK_LEN + 103, CHUNK_LEN + 103], if block_lanes { 1 } else { 0 }),
            ("shared k=7 at 511/512 boundary", &[CHUNK_LEN + 511, CHUNK_LEN + 512], if block_lanes { 1 } else { 0 }),
            ("split k=7/8 at 512/513 boundary", &[CHUNK_LEN + 512, CHUNK_LEN + 513], 0),
            ("k=0 pair + singleton k=1", &[CHUNK_LEN + 32, CHUNK_LEN + 32, CHUNK_LEN + 103], if block_lanes && last_lanes { 2 } else { 0 }),
            ("shared c=2 tails + trailing exact 2-chunk", &[2 * CHUNK_LEN + 512, 2 * CHUNK_LEN + 512, 2 * CHUNK_LEN], if block_lanes { 1 } else { 0 }),
            ("single c=2 tail + trailing exact 2-chunk", &[2 * CHUNK_LEN + 512, 2 * CHUNK_LEN], 0),
        ];
        for &(label, lens, want_calls) in partial_cases {
            let bufs: std::vec::Vec<std::vec::Vec<u8>> = lens.iter().map(|&n| fill(n)).collect();
            let s: std::vec::Vec<&[u8]> = bufs.iter().map(|v| v.as_slice()).collect();
            let b = crate::test::messages(&s);
            let it = crate::test::items(&b, &s);
            let mut out = std::vec![[0u8; OUT_LEN]; lens.len()];
            crate::COUNTED.with(|c| c[crate::Counted::ByBlocksCall as usize].set(0));
            crate::throughput::hash_each(Mode::Plain, &it, &mut out);
            for (i, msg) in s.iter().enumerate() {
                assert_eq!(out[i], crate::test::official::hash(Mode::Plain, msg), "{label} i={i}");
            }
            let got_calls = crate::COUNTED.with(|c| c[crate::Counted::ByBlocksCall as usize].get());
            assert_eq!(got_calls, want_calls, "{label}: exact ByBlocksCall count");
        }

        // All-small collection returns before borrowing SHARED (catches `small_done` bugs).
        if block_lanes {
            let small_bufs = [fill(64), fill(128)];
            let s: std::vec::Vec<&[u8]> = small_bufs.iter().map(|v| v.as_slice()).collect();
            let b = crate::test::messages(&s);
            let it = crate::test::items(&b, &s);
            let mut out = [[0u8; OUT_LEN]; 2];
            SHARED.with(|shared| {
                let _held = shared.borrow_mut();
                crate::throughput::hash_each(Mode::Plain, &it, &mut out);
            });
            for (i, msg) in s.iter().enumerate() {
                assert_eq!(out[i], crate::test::official::hash(Mode::Plain, msg), "all-small fast exit i={i}");
            }
        }

        // BY_BLOCKS_AT_ONCE + 1 items at the same counter (c = 1, k = 7 -> 64/16 + 1 = 5 calls).
        let over_full = BY_BLOCKS_AT_ONCE + 1;
        let bufs: std::vec::Vec<std::vec::Vec<u8>> = (0..over_full).map(|_| fill(CHUNK_LEN + 512)).collect();
        let s: std::vec::Vec<&[u8]> = bufs.iter().map(|v| v.as_slice()).collect();
        let b = crate::test::messages(&s);
        let it = crate::test::items(&b, &s);
        let mut out = std::vec![[0u8; OUT_LEN]; over_full];
        crate::COUNTED.with(|c| c[crate::Counted::ByBlocksCall as usize].set(0));
        crate::throughput::hash_each(Mode::Plain, &it, &mut out);
        for (i, msg) in s.iter().enumerate() {
            assert_eq!(out[i], crate::test::official::hash(Mode::Plain, msg), "over_full i={i}");
        }
        let got_calls = crate::COUNTED.with(|c| c[crate::Counted::ByBlocksCall as usize].get());
        let want_over_full = if block_lanes { (BY_BLOCKS_AT_ONCE as u64 / 16) + 1 } else { 0 };
        assert_eq!(got_calls, want_over_full, "BY_BLOCKS_AT_ONCE + 1 same-counter partial tails flush exactly");
    }
}
