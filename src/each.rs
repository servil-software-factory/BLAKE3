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

/// [`hash_each`], using other CPU cores where that is faster: the
/// same hashes, never slower. Collections of 512 KiB and more in all use
/// them, as [`hash_multithreaded`](crate::throughput::hash_multithreaded) does.
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
#[cfg(feature = "std")]
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

// Many messages arriving at once, such as a server's uploads, each with
// its own Hasher: one call takes the pieces that have arrived for all of
// them, which hash faster together than one Hasher::update each.

/// Feed each piece to its hasher: `(i, bytes)` does
/// `hashers[i].update(bytes)`, each hasher's pieces in their order in
/// `pieces`. Call it once with all the pieces your program has in hand
/// (one turn of its event loop): never slower than one
/// [`update`](Hasher::update) each: many messages arriving at once, such
/// as a server's uploads, each with its own [`Hasher`]. Requires each `i < hashers.len()`.
/// Finish the messages with [`finalize_each`]: on AArch64 a message of up
/// to 1024 bytes waits whole in its hasher, and those that finish together
/// hash side by side there.
///
/// # Key-dependent timing
///
/// Hashers may use different secret keys. Grouping, kernel selection, and
/// temporary storage sizes are independent of key values and key equality.
/// See the [key-independence contract](crate#key-dependent-timing).
///
/// ```
/// # blake3_servil::initialize();
/// use blake3_servil::{convenience::Hasher, throughput};
/// let mut hashers = vec![Hasher::new(blake3_servil::Mode::Plain, None), Hasher::new(blake3_servil::Mode::Plain, None)];
/// throughput::update_each(&mut hashers, &[(0, b"ab"), (1, b"xyz"), (0, b"c")]);
/// let mut out = [[0u8; 32]; 2];
/// throughput::finalize_each(&mut hashers, &[0, 1], &mut out);
/// let hash = blake3_servil::convenience::hash;
/// assert_eq!((&out[0], &out[1]), (hash(b"abc").as_bytes(), hash(b"xyz").as_bytes()));
/// ```
pub fn update_each(hashers: &mut [Hasher], pieces: &[(usize, &[u8])]) {
    update_turn(hashers, pieces, false);
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
#[cfg(feature = "std")]
pub fn update_each_multithreaded(hashers: &mut [Hasher], pieces: &[(usize, &[u8])]) {
    crate::lanes::require();
    update_turn(hashers, pieces, true);
}

/// The hash of each message `which` names: `out[k]` is what
/// `hashers[which[k]].finalize()` would give, and that hasher is then
/// reset for its next message (no length hint). Requires
/// `out.len() == which.len()`, each index `< hashers.len()`, and distinct indices.
/// Messages that finish in one call hash side by side where the CPU has
/// the lanes: sixteen at once on AVX-512, and on AArch64 the plain
/// messages of up to 1024 bytes fed by [`update_each`].
///
/// # Key-dependent timing
///
/// Selected hashers may use different secret keys. Key values and key equality
/// select neither the finalization path nor its amount of work.
/// See the [key-independence contract](crate#key-dependent-timing).
pub fn finalize_each(hashers: &mut [Hasher], which: &[usize], out: &mut [[u8; OUT_LEN]]) {
    assert_eq!(which.len(), out.len(), "one digest per message named");
    debug_assert!(which.iter().enumerate().all(|(at, i)| !which[..at].contains(i)), "each finalized index is distinct");
    #[cfg(all(blake3_avx512_ffi, target_os = "linux"))]
    if which.len() >= 16 && matches!(Platform::detect(), Platform::AVX512) {
        #[cfg(all(test, feature = "std"))]
        crate::counted(crate::Counted::OutputGroupTurn);
        for (which, out) in which.chunks(16).zip(out.chunks_mut(16)) {
            if which.len() == 16 {
                let hashed = {
                    let inputs: Option<arrayvec::ArrayVec<_, 16>> = which.iter().map(|&i| crate::ffi_output::input(&hashers[i])).collect();
                    match inputs {
                        Some(inputs) => {
                            // Sound: the detected platform supplies AVX-512F+VL;
                            // borrowed descriptors keep every block/CV readable.
                            unsafe { crate::ffi_output::hash_group(inputs.as_slice().try_into().unwrap(), out) };
                            true
                        }
                        None => false,
                    }
                };
                if hashed {
                    for &i in which {
                        hashers[i].reset(None);
                    }
                    continue;
                }
            }
            finish_each(hashers, which, out);
        }
        return;
    }
    // AArch64: the messages of up to a chunk that a turn left whole in
    // their stages (`Hasher::gather_part`) hash side by side by their
    // blocks, as a collection's small items do (`hash_by_blocks`: the NEON
    // lanes, or one SME2 call for a table of equal whole blocks). The
    // plain ones only: their key is the IV, the same for all, and a keyed
    // hasher's key is its own and secret, so it finishes alone (BLAKE3
    // #312). Which hashers qualify is their public shape: mode and length.
    #[cfg(blake3_neon)]
    {
        let whole_in_stage = |h: &Hasher| h.staged <= CHUNK_LEN && h.core.count() == 0 && h.core.initial_chunk_counter == 0 && h.core.chunk_state.flags == 0;
        if which.iter().filter(|&&i| whole_in_stage(&hashers[i])).nth(1).is_some() {
            let platform = Platform::detect();
            // BY_BLOCKS_AT_ONCE of them at a time, their places on the
            // stack: a finish allocates nothing.
            let mut small = (0..which.len()).filter(|&k| whole_in_stage(&hashers[which[k]])).peekable();
            while small.peek().is_some() {
                let part: arrayvec::ArrayVec<usize, BY_BLOCKS_AT_ONCE> = small.by_ref().take(BY_BLOCKS_AT_ONCE).collect();
                let bytes: arrayvec::ArrayVec<&[u8], BY_BLOCKS_AT_ONCE> = part.iter().map(|&k| hashers[which[k]].staged_bytes()).collect();
                let mut digests = [[0u8; OUT_LEN]; BY_BLOCKS_AT_ONCE];
                hash_by_blocks(&bytes, crate::IV, 0, &mut digests[..part.len()], platform);
                for (&k, digest) in part.iter().zip(&digests) {
                    out[k] = *digest;
                }
            }
            for (&i, out) in which.iter().zip(out.iter_mut()) {
                if !whole_in_stage(&hashers[i]) {
                    *out = *hashers[i].take_output().root_hash().as_bytes();
                }
                hashers[i].reset(None);
            }
            return;
        }
    }
    finish_each(hashers, which, out);
}

/// Each hasher `which` names finished in place and reset, its digest into
/// `out`: the path every message takes but a sixteen-lane group.
fn finish_each(hashers: &mut [Hasher], which: &[usize], out: &mut [[u8; OUT_LEN]]) {
    for (&i, out) in which.iter().zip(out) {
        *out = *hashers[i].take_output().root_hash().as_bytes();
        hashers[i].reset(None);
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
fn update_turn(hashers: &mut [Hasher], pieces: &[(usize, &[u8])], pooled: bool) {
    let mut held = arrayvec::ArrayVec::<usize, TABLE>::new();
    let mut waiting = arrayvec::ArrayVec::<(usize, &[u8]), TABLE>::new();
    for &(i, bytes) in pieces {
        if waiting.is_full() || held.is_full() {
            drain(hashers, &mut held, &mut waiting, pooled);
        }
        if hashers[i].held_group().is_some() {
            waiting.push((i, bytes));
            continue;
        }
        gather(hashers, i, bytes, pooled, &mut held, &mut waiting);
    }
    drain(hashers, &mut held, &mut waiting, pooled);
}

/// `bytes` into hasher `i`, which is not held: as much as fills its stage;
/// a filled stage is held and the rest of the piece waits.
fn gather<'a>(
    hashers: &mut [Hasher],
    i: usize,
    bytes: &'a [u8],
    pooled: bool,
    held: &mut arrayvec::ArrayVec<usize, TABLE>,
    waiting: &mut arrayvec::ArrayVec<(usize, &'a [u8]), TABLE>,
) {
    debug_assert!(hashers[i].held_group().is_none(), "a held hasher's bytes wait");
    let rest = hashers[i].gather_part(bytes, pooled, true);
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
fn drain<'a>(hashers: &mut [Hasher], held: &mut arrayvec::ArrayVec<usize, TABLE>, waiting: &mut arrayvec::ArrayVec<(usize, &'a [u8]), TABLE>, pooled: bool) {
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
                gather(hashers, i, bytes, pooled, held, waiting);
            }
        }
    }
}

/// Hash every queued stage, then return the bounded table to its caller.
fn flush_held(hashers: &mut [Hasher], held: &mut arrayvec::ArrayVec<usize, TABLE>) {
    hash_held(hashers, held);
    held.clear();
}

#[cfg(all(test, feature = "std"))]
std::thread_local! {
    // Actual cohorts passed to the shared-key body.
    static HELD_GROUPS: core::cell::RefCell<Vec<Vec<usize>>> = const { core::cell::RefCell::new(Vec::new()) };
}

/// Hash the whole groups `held` names, one in each of those hashers'
/// stages, and pass each group's subtree to its hasher's core. The plain
/// hashers' groups hash together; a keyed or key-derivation hasher's group
/// hashes alone, since its key is secret and must not decide the work
/// (BLAKE3 #312): the mode alone, public, groups them.
#[cfg(feature = "std")]
fn hash_held(hashers: &mut [Hasher], held: &[usize]) {
    // Plain's key is the IV, the same for every plain hasher.
    let mut plain = arrayvec::ArrayVec::<usize, TABLE>::new();
    plain.extend(held.iter().copied().filter(|&i| hashers[i].core.chunk_state.flags == 0));
    if !plain.is_empty() {
        hash_cohort::<TABLE>(hashers, &plain);
    }
    for &i in held {
        if hashers[i].core.chunk_state.flags != 0 {
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
    level: arrayvec::ArrayVec<[Block; STAGE_PARENTS], N>,
    next: arrayvec::ArrayVec<[Block; STAGE_PARENTS / 2], N>,
}

impl<const N: usize> HeldWorkspace<N> {
    /// Empty buffers, ready to initialize in their final place.
    #[inline(always)]
    pub(crate) fn new() -> Self {
        const {
            assert!(ALONE_CHUNKS == 16 && crate::STAGE_LEN == ALONE_CHUNKS * CHUNK_LEN);
        }
        Self { level: arrayvec::ArrayVec::new(), next: arrayvec::ArrayVec::new() }
    }

    /// Call once, with 1 <= count <= N: initialize through a borrow of the
    /// workspace's final place, then borrow its two CV levels.
    pub(crate) fn buffers(&mut self, count: usize) -> (&mut [CVBytes], &mut [CVBytes]) {
        debug_assert!(count > 0 && count <= N && self.level.is_empty() && self.next.is_empty());
        self.level.extend(core::iter::repeat_n([Block::ZERO; STAGE_PARENTS], count));
        self.next.extend(core::iter::repeat_n([Block::ZERO; STAGE_PARENTS / 2], count));
        let (level, _) = Block::bytes_mut(self.level.as_mut_slice().as_flattened_mut()).as_chunks_mut::<OUT_LEN>();
        let (next, _) = Block::bytes_mut(self.next.as_mut_slice().as_flattened_mut()).as_chunks_mut::<OUT_LEN>();
        (level, next)
    }
}

/// Hash 1..=N distinct held stages: plain hashers together, or one keyed
/// or derive-key hasher. Keep chunks and each parent level together so
/// one SME2 entry covers the cohort; contiguous parents use its direct
/// path. Spare descriptor slots repeat a readable block and stay unused.
#[cfg(feature = "std")]
// Keep the workspace frame out of empty turns and the other mode's caller.
#[inline(never)]
fn hash_cohort<const N: usize>(hashers: &mut [Hasher], alike: &[usize]) {
    use crate::{BLOCK_LEN, PARENT};
    let first = alike[0];
    #[cfg(all(test, feature = "std"))]
    HELD_GROUPS.with(|groups| groups.borrow_mut().push(alike.to_vec()));
    #[cfg(all(test, feature = "std"))]
    crate::counted(crate::Counted::HeldPass);
    let (key, flags) = (hashers[first].core.key, hashers[first].core.chunk_state.flags);
    let turn = crate::platform::Sme2Turn::take(hashers[first].core.chunk_state.platform, true);
    let platform = turn.platform();
    let mut work = HeldWorkspace::<N>::new();
    let (mut level, mut next) = work.buffers(alike.len());
    {
        let mut refs = arrayvec::ArrayVec::<[&[u8; CHUNK_LEN]; ALONE_CHUNKS], N>::new();
        refs.extend(alike.iter().map(|&i| {
            let stage = hashers[i].held_group().expect("a held group").1;
            core::array::from_fn(|j| stage[j * CHUNK_LEN..(j + 1) * CHUNK_LEN].try_into().unwrap())
        }));
        let refs = refs.as_slice().as_flattened();
        let mut counters = arrayvec::ArrayVec::<u64, N>::new();
        counters.extend(alike.iter().map(|&i| hashers[i].held_group().unwrap().0));
        #[cfg(blake3_sme2)]
        let on_sme2 = matches!(platform, Platform::SME2);
        #[cfg(not(blake3_sme2))]
        let on_sme2 = false;
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
    let mut per = ALONE_CHUNKS;
    while per > 1 {
        let count = alike.len() * per / 2;
        {
            let bytes = level[..2 * count].as_flattened();
            // Eight parent blocks per full stage. Derive descriptor rows
            // from that layout, including its readable final padding.
            let (full, tail) = bytes.as_chunks::<BLOCK_LEN>().0.as_chunks::<STAGE_PARENTS>();
            let mut rows = arrayvec::ArrayVec::<_, N>::new();
            rows.extend(full.iter().map(|row| row.each_ref()));
            if let Some(first) = tail.first() {
                rows.push(core::array::from_fn(|i| tail.get(i).unwrap_or(first)));
            }
            platform.hash_many(&rows.as_slice().as_flattened()[..count], &key, 0, IncrementCounter::No, flags | PARENT, 0, 0, next[..count].as_flattened_mut());
        }
        core::mem::swap(&mut level, &mut next);
        per /= 2;
    }
    drop(turn);
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
/// # blake3_servil::initialize();
/// use blake3_servil::{Mode, convenience, throughput::{self, Block}};
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
const BY_BLOCKS_AT_ONCE: usize = TABLE;
#[cfg(not(blake3_sme2))]
const BY_BLOCKS_AT_ONCE: usize = crate::lanes::MEMBERS;

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
fn hash_by_blocks(items: &[&[u8]], key: &CVWords, flags: u8, out: &mut [[u8; OUT_LEN]], platform: Platform) {
    assert_eq!(items.len(), out.len(), "one digest per item");
    assert!(items.iter().all(|i| i.len() <= CHUNK_LEN), "items of up to a chunk");
    for (items, out) in items.chunks(BY_BLOCKS_AT_ONCE).zip(out.chunks_mut(BY_BLOCKS_AT_ONCE)) {
        by_blocks_at_once(items, key, flags, out, platform);
    }
}

fn by_blocks_at_once(items: &[&[u8]], key: &CVWords, flags: u8, out: &mut [[u8; OUT_LEN]], platform: Platform) {
    const GROUP: usize = 16;
    // On SME2, a table of messages of one length, whole blocks, with at
    // least the work that pays for the SME2 turn: one call of the message
    // kernel for all of them (a turn's 1 KiB messages finishing together:
    // the NEON lanes took 30-64% longer on an Apple M4 Max, jobs 2016 and
    // 2019). A table of one-block messages never reaches the threshold
    // (BY_BLOCKS_AT_ONCE of 64 bytes is under SME2_SIZED_LEN).
    #[cfg(blake3_sme2)]
    {
        let len = items[0].len();
        if matches!(platform, Platform::SME2)
            && items.len() * len >= crate::SME2_SIZED_LEN
            && len % crate::BLOCK_LEN == 0
            && items.iter().all(|item| item.len() == len)
        {
            let turn = crate::platform::Sme2Turn::take(platform, true);
            if matches!(turn.platform(), Platform::SME2) {
                macro_rules! run {
                    ($n:expr) => {{
                        let mut lanes: arrayvec::ArrayVec<&[u8; $n], BY_BLOCKS_AT_ONCE> =
                            items.iter().map(|item| (*item).try_into().expect("equal whole-block lengths")).collect();
                        while lanes.len() % GROUP != 0 {
                            lanes.push(*lanes.last().unwrap());
                        }
                        crate::counted(crate::Counted::ByBlocksCall);
                        // Sound: detection and the turn provide SME2. Every
                        // typed lane holds its whole input; spare lanes
                        // repeat a valid input and write no output.
                        unsafe {
                            crate::sme2::hash_messages(&lanes, key, flags, CHUNK_START, CHUNK_END | crate::ROOT, crate::BLOCK_LEN, out.as_flattened_mut())
                        };
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
    let mut order: arrayvec::ArrayVec<usize, BY_BLOCKS_AT_ONCE> = (0..items.len()).collect();
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
                let inputs: arrayvec::ArrayVec<*const u8, GROUP> = group.iter().map(|&i| items[i].as_ptr()).collect();
                let mut last_blocks = [[0u8; crate::BLOCK_LEN]; GROUP];
                let mut lengths = [0u32; GROUP];
                for ((&i, block), len) in group.iter().zip(&mut last_blocks).zip(&mut lengths) {
                    let tail = &items[i][k * crate::BLOCK_LEN..];
                    block[..tail.len()].copy_from_slice(tail);
                    *len = tail.len() as u32;
                }
                let lasts: arrayvec::ArrayVec<*const u8, GROUP> = last_blocks[..group.len()].iter().map(|b| b.as_ptr()).collect();
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
                        flags,
                        CHUNK_START,
                        CHUNK_END | crate::ROOT,
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
                let inputs: arrayvec::ArrayVec<*const u8, GROUP> = group.iter().map(|&i| items[i].as_ptr()).collect();
                let mut lanes = [0u8; GROUP * OUT_LEN];
                crate::counted(crate::Counted::ByBlocksCall);
                // Sound: each item holds more than k whole blocks; the
                // caller selected this platform's block-group path.
                #[cfg(target_arch = "x86_64")]
                unsafe {
                    crate::pair::hash_many_blocks(platform, &inputs, k, key, 0, false, flags, CHUNK_START, 0, &mut lanes[..group.len() * OUT_LEN])
                };
                #[cfg(blake3_neon)]
                unsafe {
                    crate::neon_hybrid::hash_messages_raw(
                        inputs.as_ptr(),
                        group.len(),
                        k,
                        key,
                        0,
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
        platform.compress_in_place(&mut cv, &block, last as u8, 0, flags | start | CHUNK_END | crate::ROOT);
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
    let by_blocks = platform_takes && items.iter().filter(|i| i.bytes().len() <= CHUNK_LEN).nth(1).is_some();
    if by_blocks {
        // BY_BLOCKS_AT_ONCE of them at a time, their indices on the stack:
        // a collection of small items allocates nothing.
        let mut small = (0..items.len()).filter(|&i| items[i].bytes().len() <= CHUNK_LEN).peekable();
        while small.peek().is_some() {
            let part: arrayvec::ArrayVec<usize, BY_BLOCKS_AT_ONCE> = small.by_ref().take(BY_BLOCKS_AT_ONCE).collect();
            let bytes: arrayvec::ArrayVec<&[u8], BY_BLOCKS_AT_ONCE> = part.iter().map(|&i| items[i].bytes()).collect();
            let mut digests = [[0u8; OUT_LEN]; BY_BLOCKS_AT_ONCE];
            hash_by_blocks(&bytes, &key, flags, &mut digests[..part.len()], platform);
            for (&i, digest) in part.iter().zip(&digests) {
                out[i] = *digest;
            }
        }
    }
    // The messages of 2 to ALONE_CHUNKS - 1 chunks share the lanes, their
    // k-th whole chunks side by side across the whole collection (the more
    // messages a round holds, the fuller its lanes). Their places and chunk
    // values live in this thread's storage, kept between calls (SHARED).
    SHARED.with(|shared| {
        let mut shared = shared.try_borrow_mut().expect("one collection at a time on a thread");
        let Shared { messages, cvs } = &mut *shared;
        messages.clear();
        let mut slots = 0;
        let mut most_full = 0;
        for (i, item) in items.iter().map(Item::bytes).enumerate() {
            let chunks = item.len().div_ceil(CHUNK_LEN);
            if chunks <= 1 && by_blocks {
                continue;
            }
            if chunks <= 1 || chunks >= ALONE_CHUNKS {
                out[i] = *alone(item).as_bytes();
            } else {
                most_full = most_full.max(item.len() / CHUNK_LEN);
                messages.push((i, slots));
                slots += chunks;
            }
        }
        cvs.clear();
        cvs.resize(slots, [0u8; OUT_LEN]);
        for k in 0..most_full {
            crate::counted(crate::Counted::EachRound);
            // A round's lanes go to the kernels LANES_AT_ONCE at a time,
            // their chunks and places on the stack.
            let mut refs = arrayvec::ArrayVec::<&[u8; CHUNK_LEN], LANES_AT_ONCE>::new();
            let mut owners = arrayvec::ArrayVec::<usize, LANES_AT_ONCE>::new();
            let mut flush = |refs: &mut arrayvec::ArrayVec<&[u8; CHUNK_LEN], LANES_AT_ONCE>, owners: &mut arrayvec::ArrayVec<usize, LANES_AT_ONCE>| {
                let mut lane_out = [0u8; LANES_AT_ONCE * OUT_LEN];
                let lanes = &mut lane_out[..refs.len() * OUT_LEN];
                platform.hash_many(refs, &key, k as u64, IncrementCounter::No, flags, CHUNK_START, CHUNK_END, lanes);
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
        for &(i, at) in messages.iter() {
            let item = items[i].bytes();
            let chunks = item.len().div_ceil(CHUNK_LEN);
            let full = item.len() / CHUNK_LEN;
            if full < chunks {
                cvs[at + full] = ChunkState::new(&key, full as u64, flags, platform).update(&item[full * CHUNK_LEN..]).output().chaining_value();
            }
            out[i] = *root(&cvs[at..at + chunks], &key, flags, platform).as_bytes();
        }
    });
}

/// The lanes of one call to the kernels within a round of [`hash_each_on`].
const LANES_AT_ONCE: usize = TABLE;

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

/// The messages and chunk values a pool thread's storage holds from its
/// start: a range of a pooled collection is cut at about
/// `lanes::MAX_PIECE_LEN` (128 KiB) of items, so it holds under 64
/// messages of 2 chunks or more plus one, and under 144 + 65 of their
/// chunks. A pooled collection then allocates nothing on any thread.
const PREPARED_MESSAGES: usize = 128;
const PREPARED_CVS: usize = 256;

/// This thread's storage for the collections it hashes, made now: the
/// pool's threads at their start, and the thread that initializes the pool.
pub(crate) fn prepare_thread() {
    SHARED.with(|shared| {
        let mut shared = shared.borrow_mut();
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

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "each finalized index is distinct")]
    fn finalize_each_rejects_repeated_indices() {
        crate::initialize();
        let mut hashers = [Hasher::new(Mode::Plain, None)];
        finalize_each(&mut hashers, &[0, 0], &mut [[0; OUT_LEN]; 2]);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "each finalized index is distinct")]
    fn finalize_each_rejects_repeats_across_groups() {
        crate::initialize();
        let mut hashers: Vec<_> = (0..16).map(|_| Hasher::new(Mode::Plain, None)).collect();
        let which: Vec<_> = (0..16).chain(0..16).collect();
        finalize_each(&mut hashers, &which, &mut [[0; OUT_LEN]; 32]);
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

    /// hash_each_multithreaded takes the pool from MIN_SPLIT_LEN bytes of
    /// items on, exactly, and the calling thread below them. Both give the
    /// same digests, so only where the call ran tells (BLAKE3 #219).
    #[test]
    fn test_hash_each_multithreaded_splits_at_min_split_len_exactly() {
        let min = crate::lanes::MIN_SPLIT_LEN;
        let input = vec![7u8; min];
        for total in [min - 1, min] {
            // Items under the split each, so none hashes alone.
            let items: Vec<&[u8]> = input[..total].chunks(64 * 1024 - 3).collect();
            let (mut a, mut b) = (vec![[0u8; OUT_LEN]; items.len()], vec![[0u8; OUT_LEN]; items.len()]);
            crate::RAN.with(|r| r.set(0));
            crate::throughput::hash_each_multithreaded(Mode::Plain, &crate::test::items(&crate::test::messages(&items), &items), &mut a);
            if total < min {
                assert_eq!(crate::RAN.with(|r| r.get()) & crate::Ran::Pool as u32, 0, "{total} bytes of items stay on the calling thread");
            } else {
                crate::lanes::assert_past_the_split("MIN_SPLIT_LEN bytes of items");
            }
            crate::throughput::hash_each(Mode::Plain, &crate::test::items(&crate::test::messages(&items), &items), &mut b);
            assert_eq!(a, b);
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

    /// The pool's ranges of items (lanes::Work::Each), just past where the
    /// pool takes them, against the calling thread's.
    #[test]
    fn test_hash_each_multithreaded_past_the_split() {
        let mut input = vec![0u8; crate::lanes::MIN_SPLIT_LEN + 8 * 1024];
        crate::test::paint_test_input(&mut input);
        let items: Vec<&[u8]> = input.chunks(crate::lanes::MIN_SPLIT_LEN / 40 + 7).collect();
        let (mut a, mut b) = (vec![[0u8; OUT_LEN]; items.len()], vec![[0u8; OUT_LEN]; items.len()]);
        crate::RAN.with(|r| r.set(0));
        crate::throughput::hash_each_multithreaded(Mode::Plain, &crate::test::items(&crate::test::messages(&items), &items), &mut a);
        crate::lanes::assert_past_the_split("the items");
        crate::throughput::hash_each(Mode::Plain, &crate::test::items(&crate::test::messages(&items), &items), &mut b);
        assert_eq!(a, b);
    }

    /// A collection past the split (items around chunk, lane, and the
    /// split's own length, a few of 512 KiB and more), every mode: the
    /// multithreaded form gives hash_each_with's digests.
    /// Keep: it detects, by hanging, mutants the clock then stops
    /// (tools/mutants/timeouts-x86_64.txt).
    #[test]
    fn test_hash_each_multithreaded() {
        let mut input = vec![0u8; 3 << 20];
        crate::test::paint_test_input(&mut input);
        let lens = [0, 1, 1024, 1025, 3000, 15 * 1024, 16 * 1024, 100_000, 511 * 1024, 512 * 1024, 700_000];
        let items: Vec<&[u8]> = (0..400)
            .map(|k| {
                let n = lens[k * 7 % lens.len()].min(input.len() - k);
                &input[k..k + n]
            })
            .collect();
        let key = [9u8; 32];
        for mode in [Mode::Plain, Mode::Keyed(&key), Mode::DeriveKey("each mt")] {
            let (mut a, mut b) = (vec![[0u8; OUT_LEN]; items.len()], vec![[0u8; OUT_LEN]; items.len()]);
            crate::throughput::hash_each_multithreaded(mode, &crate::test::items(&crate::test::messages(&items), &items), &mut a);
            crate::throughput::hash_each(mode, &crate::test::items(&crate::test::messages(&items), &items), &mut b);
            assert_eq!(a, b);
            // Below the split, on the calling thread: the items of 16 KiB
            // or less, which sum to less than the split's length.
            let small: Vec<usize> = (0..items.len()).filter(|&k| items[k].len() <= 16 * 1024).take(30).collect();
            assert!(small.iter().map(|&k| items[k].len()).sum::<usize>() < crate::lanes::MIN_SPLIT_LEN);
            let small_items: Vec<&[u8]> = small.iter().map(|&k| items[k]).collect();
            let mut c = vec![[0u8; OUT_LEN]; small.len()];
            crate::throughput::hash_each_multithreaded(mode, &crate::test::items(&crate::test::messages(&small_items), &small_items), &mut c);
            assert!(small.iter().zip(&c).all(|(&k, d)| *d == b[k]));
        }
    }

    /// Many hashers fed in turns of interleaved pieces of many lengths:
    /// update_each, its multithreaded twin, and finalize_each give every
    /// message the hash one Hasher per message gives.
    /// Turns in which several hashers hold a group each, under two keys
    /// whose flags agree, beyond their first group and at it (nonzero
    /// counters, and zero): the groups of one key hash together, each
    /// lands in its own hasher.
    #[test]
    fn test_update_each_holds_groups_together() {
        for platform in crate::test::every_platform() {
            holds_groups_together(platform);
        }
    }

    /// Hashers of subtrees at input offsets in update_each's turns,
    /// beside hashers of whole messages: they hold no groups, and each
    /// gives the chaining value one update of its subtree gives.
    #[test]
    fn test_update_each_at_offsets() {
        use crate::test::SubtreeHasher;
        let mut input = vec![0u8; 1 << 20];
        crate::test::paint_test_input(&mut input);
        // (offset, length): subtrees within what their offsets allow.
        let subtrees = [(64 * CHUNK_LEN, 64 * CHUNK_LEN), (16 * CHUNK_LEN, 16 * CHUNK_LEN), (CHUNK_LEN, CHUNK_LEN), (128 * CHUNK_LEN, 100_000)];
        let whole = [70_000usize, 300_000];
        let mut hashers: Vec<Hasher> = subtrees
            .iter()
            .map(|&(offset, _)| {
                let mut h = Hasher::new(crate::Mode::Plain, None);
                h.set_input_offset(offset as u64);
                h
            })
            .collect();
        hashers.extend(whole.iter().map(|_| Hasher::new(crate::Mode::Plain, None)));
        let lens: Vec<usize> = subtrees.iter().map(|&(_, len)| len).chain(whole).collect();
        let mut done = vec![0usize; lens.len()];
        let pieces = [1448usize, 31, 16384, 4096, 1];
        let mut k = 0;
        while done.iter().zip(&lens).any(|(d, l)| d < l) {
            let mut turn: Vec<(usize, &[u8])> = Vec::new();
            for _ in 0..9 {
                let i = k % lens.len();
                let n = pieces[k % pieces.len()].min(lens[i] - done[i]);
                turn.push((i, &input[done[i]..done[i] + n]));
                done[i] += n;
                k += 1;
            }
            crate::throughput::update_each(&mut hashers, &turn);
        }
        for (i, &(offset, len)) in subtrees.iter().enumerate() {
            let mut one = Hasher::new(crate::Mode::Plain, None);
            one.set_input_offset(offset as u64).update(&input[..len]);
            assert_eq!(hashers[i].finalize_non_root(), one.finalize_non_root(), "subtree at {offset}, {len} bytes");
        }
        for (j, &len) in whole.iter().enumerate() {
            assert_eq!(hashers[subtrees.len() + j].clone().finalize(), crate::convenience::hash(&input[..len]), "whole message of {len} bytes");
        }
    }

    fn holds_groups_together(platform: crate::platform::Platform) {
        let mut input = vec![0u8; 3 * crate::STAGE_LEN];
        crate::test::paint_test_input(&mut input);
        let keys = [[1u8; 32], [2u8; 32]];
        let on = |mut h: Hasher| {
            h.core.chunk_state.platform = platform;
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

    /// A turn of many hashers on AArch64 keeps a fresh hasher's first
    /// chunk in its stage, for `finalize_each` to find; elsewhere, and in
    /// a single `update` everywhere, it goes to the core. The digest is
    /// the same either way, so the stage's length tells.
    #[test]
    fn a_turn_on_aarch64_stages_a_first_chunk_and_update_never_does() {
        crate::initialize();
        let bytes = [7u8; 300];
        let mut hashers = vec![Hasher::new(Mode::Plain, None), Hasher::new(Mode::Plain, None)];
        update_each(&mut hashers, &[(0, &bytes[..100]), (0, &bytes[100..])]);
        hashers[1].update(&bytes);
        let in_turn = if cfg!(blake3_neon) { (300, 0) } else { (0, 300) };
        assert_eq!((hashers[0].staged, hashers[0].core.count()), in_turn, "a turn's first chunk");
        assert_eq!((hashers[1].staged, hashers[1].core.count()), (0, 300), "an update's first chunk goes to the core");
        for h in hashers {
            assert_eq!(h.finalize(), crate::test::official::hash(Mode::Plain, &bytes));
        }
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

    /// update_each's held groups hash together by their public mode alone
    /// (BLAKE3 #312): plain hashers share passes, and keyed ones take the
    /// same passes whether their keys are equal or not.
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
        assert_eq!(passes(&[Mode::Plain; 3]), 1, "plain hashers hash together");
        assert_eq!(passes(&[Mode::Plain, Mode::Keyed(&a), Mode::Plain]), 2, "the keyed one alone");
        let same = passes(&[Mode::Keyed(&a); 3]);
        assert_eq!(same, passes(&[Mode::Keyed(&a), Mode::Keyed(&b), Mode::Keyed(&c)]), "equal keys take the passes distinct keys take");
        assert_eq!(same, 3);
        let derived = passes(&[Mode::DeriveKey("shared context"); 3]);
        assert_eq!(derived, passes(&[Mode::DeriveKey("context a"), Mode::DeriveKey("context b"), Mode::DeriveKey("context c")]));
        assert_eq!(derived, 3, "derived-key equality stays private");
        // The ready table bounds plain cohorts. Keyed cohorts iterate
        // one hasher at a time, independent of key equality.
        assert_eq!(passes(&[Mode::Keyed(&a); 600]), 600, "600 keyed hashers, one pass each");
        assert_eq!(passes(&[Mode::Plain; 600]), 600usize.div_ceil(TABLE) as u64, "600 plain hashers, bounded passes");
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
            let mut hashers: Vec<_> = (0..n).map(|_| Hasher::new(Mode::Plain, None)).collect();
            let mut pieces = Vec::with_capacity(n);
            for start in (0..2 * crate::STAGE_LEN).step_by(4096) {
                pieces.clear();
                pieces.extend(inputs.iter().enumerate().map(|(i, b)| (i, &b[start..start + 4096])));
                update_each(&mut hashers, &pieces);
                assert!(hashers.iter().all(|h| h.held_group().is_none()), "each turn completes its held stages");
            }
            for (h, input) in hashers.iter_mut().zip(&inputs) {
                h.update(&input[2 * crate::STAGE_LEN..]);
                assert_eq!(h.clone().finalize(), crate::test::official::hash(Mode::Plain, input));
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
                                h.core.chunk_state.platform = platform;
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

    #[test]
    fn test_update_each() {
        let mut input = vec![0u8; 1 << 20];
        crate::test::paint_test_input(&mut input);
        let lens = [0, 1, 63, 64, 1023, 1024, 1025, 2048, 4470, 8191, 16384, 16385, 65536, 100_000, 300_000];
        let piece_lens = [1, 1448, 4096, 1448, 16384, 31, 65536];
        for multithreaded in [false, true] {
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
                if multithreaded {
                    crate::throughput::update_each_multithreaded(&mut hashers, &pieces);
                } else {
                    crate::throughput::update_each(&mut hashers, &pieces);
                }
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
                assert_eq!(&out[k], want.as_bytes(), "len {} multithreaded {multithreaded}", lens[i]);
            }
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
}
