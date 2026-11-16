//! The one streaming API ([`Pipeline`]): messages fed through reusable
//! [`Room`]s, owned values (`owned`), borrowed slices (`lent`), or
//! [`Batch`]es, hashed across this crate's worker pool within a fixed
//! memory budget ([`Limits`]).
//!
//! Every call takes `&mut Pipeline`, and [`Message`], [`Room`], [`Lent`],
//! and [`Event`] hold no borrow of the pipeline. Bookkeeping on the
//! producer side needs no lock, tasks holding a pipeline and its tokens
//! move freely between threads, and worker threads write only into memory
//! the pipeline owns or took by value.
//!
//! Cross-CPU rings (message slots, rooms, short-message groups, piece nodes,
//! and source actions) cycle in FIFO order rather than LIFO, so cache lines
//! written or read by one worker core are reused only after a full lap of
//! the ring rather than immediately bounced to another core.

use crate::lanes::sources::{ACTION_SME2, Source};
use crate::lanes::{MEMBERS, Member, OwnLine, TASK_LEN, Task, pool_platform};
use crate::platform::Platform;
use crate::throughput::{Batch, Prepared};
use crate::{CHUNK_LEN, CVBytes, CVWords, Hash, Mode, OUT_LEN};
use core::future::Future;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::pin::Pin;
// The primitives the Pipeline runs on: the standard library's here, and
// Shuttle's in tests/shuttle_pipeline.rs, which compiles this same file
// beside its own `pipeline_sync` and a stand-in for the pool.
use crate::pipeline_sync::{AtomicBool, AtomicU64, AtomicUsize, Mutex, Ordering, UnsafeCell, process_static, thread};
use core::task::{Context, Poll, Waker};
use std::sync::Arc;

/// Bytes in each [`Room`]: sixty-four chunks (64 KiB), a whole BLAKE3
/// subtree on a 64-byte boundary. Chosen by empirical measurement across
/// x86-64 (AVX2 and AVX-512) and ARM64 (NEON and SME2), where 64 KiB
/// reaches both the 32-lane AVX-512 chunk kernel and SME2's flat subtree
/// walk with one mechanism (`1 whole Room == 1 worker piece`).
pub const ROOM_LEN: usize = 64 * 1024;

/// Minimum subtree size that benefits from the SME2 coprocessor (16 chunks).
const SME2_MIN_LEN: usize = 16 * CHUNK_LEN;

/// Maximum subtree pieces one in-memory message or batch splits into.
const MAX_SLOT_PIECES: usize = 64;

/// Initial reference credit placed on a [`Room`] when handed to the caller;
/// each `commit` consumes 1 credit locally on `&mut Room` without atomic RMW,
/// and `Room::drop` returns the unused credits in one atomic subtraction.
const ROOM_CREDITS: usize = 1 << 30;

const NONE: u32 = u32::MAX;

const KIND_GROUP: u64 = 0;
const KIND_ROOM_PIECE: u64 = 1;
const KIND_MULTI: u64 = 2;

/// Fixed-capacity FIFO ring buffer over pre-allocated `Box<[MaybeUninit<X>]>`;
/// never reallocates after construction.
struct Ring<X> {
    buf: Box<[MaybeUninit<X>]>,
    head: usize,
    len: usize,
}

impl<X> Ring<X> {
    fn new(cap: usize) -> Self {
        Ring { buf: (0..cap).map(|_| MaybeUninit::uninit()).collect(), head: 0, len: 0 }
    }

    #[inline(always)]
    fn len(&self) -> usize {
        self.len
    }

    #[inline(always)]
    fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline(always)]
    fn front(&self) -> Option<&X> {
        // Sound: the `len` items from `head` on are written (push_back) and
        // not yet read out.
        if self.len == 0 { None } else { Some(unsafe { self.buf[self.head].assume_init_ref() }) }
    }

    #[inline(always)]
    fn push_back(&mut self, item: X) {
        let cap = self.buf.len();
        // In every build: one item more than the ring holds would be
        // written over the oldest, a queued message or event lost in
        // silence. Each ring's users keep within it (a slot, a piece or a
        // room is in one ring once), and this holds them to that.
        assert!(self.len < cap, "Ring capacity exceeded");
        let idx = (self.head + self.len) % cap;
        self.buf[idx].write(item);
        self.len += 1;
    }

    #[inline(always)]
    fn pop_front(&mut self) -> Option<X> {
        if self.len == 0 {
            return None;
        }
        // Sound: as `front`; the item is read out once, since `head` moves
        // past it.
        let item = unsafe { self.buf[self.head].assume_init_read() };
        self.head = (self.head + 1) % self.buf.len();
        self.len -= 1;
        Some(item)
    }
}

impl<X> Drop for Ring<X> {
    fn drop(&mut self) {
        while self.pop_front().is_some() {}
    }
}

#[inline(always)]
fn pack_arg(kind: u64, a: u32, b: u32, sme2: bool) -> u64 {
    debug_assert!(kind < 4 && a < (1 << 29));
    let raw = (kind << 61) | ((a as u64) << 32) | (b as u64);
    if sme2 { raw | ACTION_SME2 } else { raw }
}

#[inline(always)]
fn unpack_arg(arg: u64) -> (u64, usize, usize) {
    let clean = arg & !ACTION_SME2;
    let kind = clean >> 61;
    let a = ((clean >> 32) & 0x1fff_ffff) as usize;
    let b = (clean as u32) as usize;
    (kind, a, b)
}

/// Fixed memory bounds for a [`Pipeline`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Messages open or in flight at once; as many finished [`Event`]s
    /// again may wait to be pulled.
    pub messages: usize,
    /// [`Room`]s of [`ROOM_LEN`] bytes for arriving message data.
    pub rooms: usize,
}

/// Completion of a message submitted to a [`Pipeline`].
pub enum Event<T, B = ()> {
    Hashed { tag: T, hash: Hash, owner: Option<B> },
    Aborted { tag: T, owner: Option<B> },
    Batch { tag: T, batch: Batch<'static>, digests: Box<[[u8; 32]]> },
}

/// An open message on a [`Pipeline`].
///
/// Every `Message` goes back to its Pipeline: to [`Pipeline::end`], or to
/// [`Pipeline::abort`] when its bytes will not all come. Dropping one
/// panics. In async code an early return through `?` drops what the task
/// holds, so give the message to `abort` before returning the error.
#[must_use = "end, abort, or hand the message's bytes over; dropping an unfinished Message stops the program"]
pub struct Message {
    pipeline_id: u64,
    slot: u32,
}

impl Message {
    #[inline(always)]
    fn take(self) -> (u64, usize) {
        let out = (self.pipeline_id, self.slot as usize);
        core::mem::forget(self);
        out
    }
}

impl Drop for Message {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            panic!("blake3-servil: a Pipeline Message was dropped without end, abort, owned, lent, or batch");
        }
    }
}

/// Guard for a message hashed in place on loan ([`Pipeline::lent`]).
///
/// Dropping this guard before its message's event exists aborts the
/// process so worker threads never read freed caller memory.
#[must_use = "hold the guard until the message's event; dropping an active loan aborts the process"]
pub struct Lent<'a> {
    completed: &'static AtomicU64,
    epoch: u64,
    _bytes: PhantomData<&'a [u8]>,
}

impl Drop for Lent<'_> {
    #[inline]
    fn drop(&mut self) {
        if self.completed.load(Ordering::Acquire) < self.epoch {
            stop_process("blake3-servil: a Pipeline Lent guard was dropped before its message finished hashing");
        }
    }
}

#[cold]
#[inline(never)]
fn stop_process(msg: &str) -> ! {
    use std::io::Write as _;
    let _ = std::writeln!(std::io::stderr(), "{msg}");
    std::process::abort()
}

use crate::waker::WakerSlot;

/// Lock-free pre-allocated rooms buffer and per-room credit reference counts.
/// Aligned to its own 128-byte cache-line pair.
#[repr(align(128))]
struct Rooms {
    buf: *mut u8,
    /// The model's alone (tests/shuttle_pipeline.rs): the allocation, to
    /// free at a run's end.
    #[cfg(shuttle)]
    #[allow(dead_code)]
    backing: (*mut u8, usize),
    count: usize,
    refs: Box<[OwnLine<AtomicUsize>]>,
    caller_held: OwnLine<AtomicUsize>,
}

// Sound: `buf` points into the rooms' own allocation, which is never freed or
// moved; what may touch which bytes is the credits' matter (Room,
// Shared::run_room_piece), not the pointer's.
unsafe impl Send for Rooms {}
unsafe impl Sync for Rooms {}

impl Rooms {
    fn new(count: usize) -> Self {
        // The rooms' bytes are given away for good, as the storage that
        // holds them is (PipeStorage::acquire), and kept as a pointer
        // alone: callers and workers write and read them through pointers
        // made from this one, which a Box held beside it would claim to
        // own alone.
        #[cfg(shuttle)]
        let mut whole = (core::ptr::null_mut(), 0);
        let buf = if count == 0 {
            core::ptr::NonNull::<u8>::dangling().as_ptr()
        } else {
            let backing: &'static mut [u8] = Box::leak(vec![0u8; count * ROOM_LEN + 64].into_boxed_slice());
            #[cfg(shuttle)]
            {
                whole = (backing.as_mut_ptr(), backing.len());
            }
            let align = backing.as_ptr().align_offset(64);
            // Sound: the allocation has 64 spare bytes, so the first 64-byte
            // boundary and `count` rooms after it are inside it.
            unsafe { backing.as_mut_ptr().add(align) }
        };
        Rooms {
            buf,
            #[cfg(shuttle)]
            backing: whole,
            count,
            refs: (0..count).map(|_| OwnLine(AtomicUsize::new(0))).collect(),
            caller_held: OwnLine(AtomicUsize::new(0)),
        }
    }

    #[inline(always)]
    fn room_ptr(&self, index: usize) -> *mut u8 {
        debug_assert!(index < self.count);
        // Sound: `index` is a room of this storage (the callers take it from a
        // Room, a piece or a group of this Pipeline), so the offset is inside
        // the rooms' allocation.
        unsafe { self.buf.add(index * ROOM_LEN) }
    }

    #[inline(always)]
    fn init_taken(&self, idx: u32) {
        self.refs[idx as usize].store(ROOM_CREDITS, Ordering::Relaxed);
        self.caller_held.fetch_add(1, Ordering::Relaxed);
    }

    #[inline(always)]
    fn release_n(&self, index: u32, n: usize) -> bool {
        self.refs[index as usize].fetch_sub(n, Ordering::SeqCst) == n
    }
}

#[derive(Clone, Copy)]
struct GroupMemberMeta {
    room: u32,
}

struct MemberGroup {
    task: UnsafeCell<Task>,
    metas: UnsafeCell<[GroupMemberMeta; MEMBERS]>,
    epoch: UnsafeCell<u64>,
    completed_epoch: OwnLine<AtomicU64>,
}

/// Recycled `'static` backing storage: worker source, rooms, short-message
/// groups, and multi-piece loan-completion lines. Never freed, so [`Room`]
/// and [`Lent`] hold plain `'static` references with zero `Arc` traffic.
struct PipeStorage {
    source: Source,
    waker: WakerSlot,
    rooms: Rooms,
    groups: Box<[MemberGroup]>,
    /// Used only by multi-piece (`> TASK_LEN`) `lent` messages; short `lent`
    /// messages piggyback directly on `groups[g].completed_epoch`.
    loan_completed: Box<[OwnLine<AtomicU64>]>,
    saved_loan_epoch: UnsafeCell<u64>,
    saved_group_epoch: UnsafeCell<u64>,
    /// The workers' view of the Pipeline that holds this storage, kept for
    /// the next one (the owner's alone to read and write: `create`).
    shared: UnsafeCell<*mut Shared>,
    /// Who holds this storage: its Pipeline, and each Room the caller has.
    holders: OwnLine<AtomicUsize>,
}

// Sound: the UnsafeCells here are the owner's alone to write, and only between
// tasks: `groups[g]` while group g is not in flight (acquire_free_group), the
// saved epochs and `shared` while no Pipeline holds the storage or in its own
// create and drop. Workers read a group's cells only from its dispatch to its
// `completed_epoch`.
unsafe impl Send for PipeStorage {}
unsafe impl Sync for PipeStorage {}

process_static! {
    /// Every storage no Pipeline or Room holds, for the next Pipeline: all of
    /// them, however many. A storage is never freed (workers may still read
    /// one), so one that is not kept here is lost for good: a list of 16 lost
    /// every storage past the sixteenth, and a server with a Pipeline per
    /// connection grew by one at each connection beyond 16 at once (BLAKE3
    /// #427). The process now holds as many as its most Pipelines at once.
    static FREE_STORAGES: Mutex<Vec<&'static PipeStorage>> = Mutex::new(Vec::new());
}

#[cfg(any(test, shuttle))]
process_static! {
    /// The storages made so far in this process.
    static STORAGES_MADE: AtomicUsize = AtomicUsize::new(0);
}

/// The model's alone (tests/shuttle_pipeline.rs): frees a run's storages
/// at its end. In a file of its own, outside the files the mutation
/// check changes: no build that check tests holds this code.
#[cfg(shuttle)]
#[allow(dead_code)]
#[path = "../tests/model/storages.rs"]
pub(crate) mod model_storages;

impl PipeStorage {
    /// Give up one hold on `st` (its Pipeline's, or a Room's): the last
    /// one puts it on the free list. A Room may outlive its Pipeline, so
    /// either may be the last.
    fn release(st: &'static PipeStorage) {
        if st.holders.fetch_sub(1, Ordering::AcqRel) == 1 {
            FREE_STORAGES.lock().unwrap().push(st);
        }
    }

    fn acquire(actions: usize, messages: usize, rooms: usize, groups: usize, key: &CVWords, flags: u8) -> &'static PipeStorage {
        let chosen = {
            let mut free = FREE_STORAGES.lock().unwrap();
            let fits = |st: &&'static PipeStorage| {
                st.source.actions_len() >= actions && st.loan_completed.len() >= messages && st.rooms.count == rooms && st.groups.len() >= groups
            };
            free.iter().position(fits).map(|at| free.swap_remove(at))
        };
        if let Some(st) = chosen {
            st.holders.store(1, Ordering::Release);
            for g in &st.groups[..groups] {
                // Sound: the storage came off the free list, so no Pipeline
                // holds it and every task that read these groups has finished
                // (Pipeline::drop stops the process otherwise).
                let t = unsafe { &mut *g.task.get() };
                t.key = *key;
                t.flags = flags;
            }
            return st;
        }
        let actions = actions.max(1024);
        let messages = messages.max(64);
        let groups = groups.max(64);
        let dummy = GroupMemberMeta { room: NONE };
        #[cfg(any(test, shuttle))]
        STORAGES_MADE.fetch_add(1, Ordering::Relaxed);
        Box::leak(Box::new(PipeStorage {
            source: Source::with_actions(actions),
            waker: WakerSlot::new(),
            rooms: Rooms::new(rooms),
            groups: (0..groups)
                .map(|_| MemberGroup {
                    task: UnsafeCell::new(Task::members(key, flags, None)),
                    metas: UnsafeCell::new([dummy; MEMBERS]),
                    epoch: UnsafeCell::new(0),
                    completed_epoch: OwnLine(AtomicU64::new(0)),
                })
                .collect(),
            loan_completed: (0..messages).map(|_| OwnLine(AtomicU64::new(0))).collect(),
            saved_loan_epoch: UnsafeCell::new(1),
            saved_group_epoch: UnsafeCell::new(1),
            shared: UnsafeCell::new(core::ptr::null_mut()),
            holders: OwnLine(AtomicUsize::new(1)),
        }))
    }
}

/// Reusable `ROOM_LEN`-byte buffer for arriving message bytes.
#[must_use = "commit from the room; a dropped Room gives back what is left of it"]
pub struct Room {
    pipeline_id: u64,
    index: u32,
    offset: u32,
    commits: u32,
    ptr: *mut u8,
    storage: &'static PipeStorage,
}

// Sound: a Room is the only way to its uncommitted bytes, and its other fields
// are plain values; `storage` is shared and Sync.
unsafe impl Send for Room {}
unsafe impl Sync for Room {}

#[allow(clippy::len_without_is_empty)]
impl Room {
    /// Remaining writable bytes in this room, starting on a 64-byte boundary.
    #[inline(always)]
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        let rem = ROOM_LEN - self.offset as usize;
        // Sound: the bytes from `offset` to the room's end are this Room's
        // alone: committed bytes lie before `offset`, and no other Room has
        // this index while this one's credits are out (take_credit).
        unsafe { core::slice::from_raw_parts_mut(self.as_mut_ptr(), rem) }
    }

    /// Pointer to the start of the remaining writable bytes.
    #[inline(always)]
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        // Sound: `offset` is at most ROOM_LEN, inside or one past the room.
        unsafe { self.ptr.add(self.offset as usize) }
    }

    /// Number of writable bytes remaining in this room.
    #[inline(always)]
    pub fn len(&self) -> usize {
        ROOM_LEN - self.offset as usize
    }

    /// Take one of this Room's credits for a commit: the pool counts a
    /// Room free when all ROOM_CREDITS are back, each commit's from the
    /// worker that finishes it and the rest from the Room's drop. A Room
    /// never gives out its last: before that it takes a new set, so the
    /// pool cannot count it free while the caller holds it. An empty
    /// commit moves the Room on by nothing, so one Room can take them
    /// without end (BLAKE3 #418: after ROOM_CREDITS of them a second Room
    /// was made over the same bytes).
    #[inline(always)]
    fn take_credit(&mut self) {
        if self.commits as usize == ROOM_CREDITS - 1 {
            self.storage.rooms.refs[self.index as usize].fetch_add(ROOM_CREDITS - 1, Ordering::Relaxed);
            self.commits = 0;
        }
        self.commits += 1;
    }
}

#[cfg(test)]
impl Room {
    /// Leave this Room `left` of its credits, as that many short of
    /// ROOM_CREDITS commits would.
    fn spend_all_but(&mut self, left: u32) {
        let spent = ROOM_CREDITS as u32 - left;
        self.storage.rooms.release_n(self.index, spent as usize);
        self.commits += spent;
    }
}

impl Drop for Room {
    #[inline]
    fn drop(&mut self) {
        self.storage.rooms.caller_held.fetch_sub(1, Ordering::Release);
        let unused = ROOM_CREDITS - (self.commits as usize);
        if self.storage.rooms.release_n(self.index, unused) {
            self.storage.waker.notify_after_seqcst();
        }
        PipeStorage::release(self.storage);
    }
}

struct PieceNode {
    room: UnsafeCell<u32>,
    offset: UnsafeCell<u32>,
    len: UnsafeCell<u32>,
    counter: UnsafeCell<u64>,
    out: OwnLine<UnsafeCell<[u8; 64]>>,
    done: OwnLine<AtomicBool>,
    next: UnsafeCell<u32>,
}

#[derive(Clone, Copy)]
struct MultiDesc {
    input: *const u8,
    total_len: usize,
    piece_len: usize,
    pieces: usize,
    batch: Option<(usize, usize, usize)>,
    digests: *mut [u8; OUT_LEN],
    loan_epoch: u64,
}

/// The last thing a worker does for a task: `publish` stores the task's
/// completion, then the owner is woken. From that store on the owner may
/// see every task done and drop the Pipeline, and its storage and its
/// `Shared` may serve the next Pipeline: so the worker writes nothing
/// after the store, and reads only the storage, taken before it. Neither
/// is ever freed (a `Shared` that was, once, was read after: #399), so a
/// late read finds memory; a late write would land in another Pipeline's
/// work. A wake that reaches the next Pipeline is a wake one may get at
/// any time (tests/shuttle_waker.rs has the slot under late finishers).
#[inline(always)]
fn finished(storage: &'static PipeStorage, publish: impl FnOnce()) {
    publish();
    storage.waker.notify_after_seqcst();
}

/// Worker-visible per-pipeline state. Never freed: it stays with its
/// storage for the next Pipeline of its size (`create`), as the storage
/// does, so nothing a worker can reach is memory an owner gives back.
struct Shared {
    key: CVWords,
    flags: u8,
    storage: &'static PipeStorage,
    hashes: Box<[UnsafeCell<[u8; OUT_LEN]>]>,
    cvs: Box<[UnsafeCell<[[u8; OUT_LEN]; MAX_SLOT_PIECES + 1]>]>,
    slot_done: Box<[OwnLine<AtomicBool>]>,
    tasks_left: Box<[OwnLine<AtomicUsize>]>,
    multi: Box<[UnsafeCell<MultiDesc>]>,
    pieces: Box<[PieceNode]>,
}

// Sound: each UnsafeCell here has one writer at a time, by the slot's or
// piece's state: the owner before it dispatches the work, the workers of that
// work until they publish it done, the owner again after. Nothing here is ever
// freed.
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

impl Shared {
    unsafe fn run_action(ctx: *const (), arg: u64, platform: Platform) {
        // Sound: `ctx` is the `*const Shared` its Pipeline dispatched with
        // (dispatch_one, dispatch_many), and a Shared is never freed.
        let shared = unsafe { &*(ctx as *const Shared) };
        let (kind, a, b) = unpack_arg(arg);
        match kind {
            KIND_GROUP => shared.run_group(a, platform),
            KIND_ROOM_PIECE => shared.run_room_piece(a, platform),
            KIND_MULTI => shared.run_multi_piece(a, b, platform),
            _ => unreachable!(),
        }
    }

    fn run_group(&self, group_idx: usize, platform: Platform) {
        let storage = self.storage;
        let group = &storage.groups[group_idx];
        // Sound: the owner wrote this group's task, epoch and metas before it
        // dispatched the group (publish_open), and writes them again only
        // after it has seen `completed_epoch` reach this epoch, which is this
        // function's last store.
        let task = unsafe { &*group.task.get() };
        let count = task.members;
        let epoch = unsafe { *group.epoch.get() };
        task.run_members(platform);
        let metas = unsafe { &*group.metas.get() };

        // Coalesce consecutive members sharing a Room into a single atomic subtraction.
        let mut cur_room = NONE;
        let mut cur_n = 0usize;
        for meta in &metas[..count] {
            if meta.room != NONE {
                if meta.room == cur_room {
                    cur_n += 1;
                } else {
                    if cur_room != NONE {
                        self.storage.rooms.release_n(cur_room, cur_n);
                    }
                    cur_room = meta.room;
                    cur_n = 1;
                }
            }
        }
        if cur_room != NONE {
            self.storage.rooms.release_n(cur_room, cur_n);
        }

        // Single SeqCst swap completes all 64 events AND all 64 short `Lent` guards!
        finished(storage, || {
            group.completed_epoch.swap(epoch, Ordering::SeqCst);
        });
    }

    fn run_room_piece(&self, pnode_idx: usize, platform: Platform) {
        let pnode = &self.pieces[pnode_idx];
        // Sound: the owner wrote the piece's fields before dispatching it
        // (commit_multi), and touches the node again only once `done` is set,
        // which is this function's last store.
        let room_idx = unsafe { *pnode.room.get() };
        let offset = unsafe { *pnode.offset.get() } as usize;
        let len = unsafe { *pnode.len.get() } as usize;
        let counter = unsafe { *pnode.counter.get() };
        // Sound: the piece is `len` committed bytes of its room from `offset`,
        // which the Room's credit keeps unwritten and unshared until this task
        // returns it below.
        let ptr = unsafe { self.storage.rooms.room_ptr(room_idx as usize).add(offset) };
        let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
        // Sound: `out` is this piece's alone until `done` (as its fields
        // above).
        let out = unsafe { &mut *pnode.out.0.get() };
        if counter == 0 && len > CHUNK_LEN {
            *out = crate::compress_subtree_to_parent_node(bytes, 0, &self.key, 0, self.flags, platform);
        } else {
            out[..OUT_LEN].copy_from_slice(&crate::hash_all_at_once(bytes, &self.key, counter, self.flags, platform).chaining_value());
        }
        let storage = self.storage;
        storage.rooms.release_n(room_idx, 1);
        finished(storage, || {
            pnode.done.swap(true, Ordering::SeqCst);
        });
    }

    fn run_multi_piece(&self, slot: usize, piece_idx: usize, platform: Platform) {
        // Sound: the owner wrote the description before dispatching the slot's
        // tasks (batch, submit_multi_slice), and writes it again only after
        // `slot_done`.
        let desc = unsafe { *self.multi[slot].get() };
        if let Some((message_len, slot_len, per_part)) = desc.batch {
            let first = piece_idx * per_part;
            let count = (desc.total_len - first).min(per_part);
            // Sound: the batch and its digests stay in the slot (`batches[s]`)
            // until the slot's event, which waits for `slot_done`; this task's
            // messages and digests are its own range of them (`first`,
            // `count`).
            let bytes = unsafe { core::slice::from_raw_parts(desc.input.add(first * slot_len), count * slot_len) };
            let digests = unsafe { core::slice::from_raw_parts_mut(desc.digests.add(first), count) };
            crate::many::hash_many_on(bytes, message_len, slot_len, &self.key, self.flags, digests, platform);
            let storage = self.storage;
            if self.tasks_left[slot].fetch_sub(1, Ordering::AcqRel) == 1 {
                finished(storage, || {
                    self.slot_done[slot].swap(true, Ordering::SeqCst);
                });
            }
            return;
        }

        let offset = piece_idx * desc.piece_len;
        let part_len = (desc.total_len - offset).min(desc.piece_len);
        let counter = (offset / CHUNK_LEN) as u64;
        // Sound: the message's bytes stay where they are until the slot is
        // done (an owner in `owners[s]`; a loan by the Lent guard, which waits
        // for `loan_completed`), and this piece reads its own range.
        let bytes = unsafe { core::slice::from_raw_parts(desc.input.add(offset), part_len) };
        let cvs_ptr = self.cvs[slot].get() as *mut [u8; OUT_LEN];
        if piece_idx == 0 {
            debug_assert!(part_len > CHUNK_LEN);
            let pair = crate::compress_subtree_to_parent_node(bytes, 0, &self.key, 0, self.flags, platform);
            // Sound: `cvs[slot]` has MAX_SLOT_PIECES + 1 values; piece 0
            // writes the first two and piece i the one at i + 1, each its own.
            unsafe {
                (*cvs_ptr).copy_from_slice(&pair[..OUT_LEN]);
                (*cvs_ptr.add(1)).copy_from_slice(&pair[OUT_LEN..]);
            }
        } else {
            let cv = crate::hash_all_at_once(bytes, &self.key, counter, self.flags, platform).chaining_value();
            // Sound: as piece 0's, above.
            unsafe {
                (*cvs_ptr.add(piece_idx + 1)).copy_from_slice(&cv);
            }
        }

        let storage = self.storage;
        if self.tasks_left[slot].fetch_sub(1, Ordering::AcqRel) == 1 {
            // Sound: this is the slot's last task (`tasks_left` reached zero),
            // so every piece's value is written and no one else reads or
            // writes the slot's values or hash until `slot_done` below.
            let cvs = unsafe { &*self.cvs[slot].get() };
            let hash = fold_multi_cvs(&self.key, self.flags, cvs, desc.pieces);
            unsafe { *self.hashes[slot].get() = *hash.as_bytes() };
            if desc.loan_epoch != 0 {
                self.storage.loan_completed[slot].0.store(desc.loan_epoch, Ordering::Release);
            }
            finished(storage, || {
                self.slot_done[slot].swap(true, Ordering::SeqCst);
            });
        }
    }
}

fn fold_multi_cvs(key: &CVWords, flags: u8, cvs: &[[u8; OUT_LEN]; MAX_SLOT_PIECES + 1], pieces: usize) -> Hash {
    let platform = Platform::detect();
    let parent = |l: &CVBytes, r: &CVBytes| crate::hasher::parent_output(l, r, key, flags, platform);
    debug_assert!(pieces >= 2);
    let mut stack: arrayvec::ArrayVec<CVBytes, 8> = arrayvec::ArrayVec::new();
    let cv0 = parent(&cvs[0], &cvs[1]).chaining_value();
    stack.push(cv0);
    for (idx, cv) in (1..pieces).zip(&cvs[2..=pieces]) {
        let n = idx as u64;
        while stack.len() > n.count_ones() as usize {
            let right = stack.pop().unwrap();
            let left = stack.pop().unwrap();
            stack.push(parent(&left, &right).chaining_value());
        }
        stack.push(*cv);
    }
    let mut right = stack.pop().unwrap();
    while stack.len() > 1 {
        right = parent(&stack.pop().unwrap(), &right).chaining_value();
    }
    parent(&stack.pop().unwrap(), &right).root_hash()
}

/// Compact hot-path slot state (24 bytes; multi-room state sits in `room_streams`).
#[derive(Clone, Copy)]
enum SlotKind {
    Free,
    OpenEmpty,
    OpenShort { room: u32, offset: u32, len: u32 },
    OpenShortDispatched { group: u32, epoch: u64 },
    OpenRooms { sealed: bool },
    WaitSingle { has_owner: bool, group: u32, epoch: u64 },
    WaitRoomMulti,
    WaitMulti { has_owner: bool },
    WaitBatch { group: u32, epoch: u64 },
    WaitAborted { group: u32, epoch: u64 },
}

#[derive(Clone, Copy)]
struct RoomStream {
    room_pieces: u64,
    head_piece: u32,
    tail_piece: u32,
    stack_len: usize,
    merged_pieces: u64,
}

impl RoomStream {
    const EMPTY: Self = RoomStream { room_pieces: 0, head_piece: NONE, tail_piece: NONE, stack_len: 0, merged_pieces: 0 };
}

struct SlotMeta<T> {
    tag: Option<T>,
    kind: SlotKind,
}

process_static! {
    static NEXT_PIPELINE_ID: AtomicU64 = AtomicU64::new(1);
}

/// Streaming multithreaded hasher with bounded memory and pull-based
/// [`Event`] delivery.
///
/// # Memory
///
/// A Pipeline's storage (its Rooms, about 64 KiB each, and its queues) is
/// kept when the Pipeline is dropped and given to the next Pipeline with
/// the same `Limits`: it is never returned to the system, so a process
/// holds as much as its most Pipelines alive at once. The storage is not
/// cleared: a keyed Pipeline's key and the bytes last written to its
/// Rooms stay in the process's memory until another Pipeline writes over
/// them. Where that matters, hash with a `Hasher` and the `zeroize`
/// feature.
///
/// # Giving up a message
///
/// The contract is the caller's, and a server's peers must not be able to
/// break it for the server:
///
/// - A message whose bytes will not all come (its client went away) goes
///   to [`abort`](Pipeline::abort). The Pipeline forgets it, gives its
///   [`Event::Aborted`], and goes on with the others. A `Message` that is
///   dropped instead panics.
/// - A Pipeline is dropped only when it has nothing left to hash: every
///   message ended or aborted, and every message's event taken with
///   [`next_event`](Pipeline::next_event). Dropped earlier, it stops the
///   whole process: its workers may still be reading its buffers.
///
/// So keep a Pipeline in a task that outlives its clients (one for a
/// thread's connections, not one a connection), and do not let a
/// time-out or a cancelled task drop it mid-message.
///
/// ```
/// # blake3_servil::initialize_multithreaded(None);
/// use blake3_servil::{Mode, convenience, throughput::{Event, Limits, Pipeline, block_on}};
/// // A client's next bytes, or the error of one that went away.
/// async fn read(client: u32, into: &mut [u8]) -> std::io::Result<usize> {
///     if client == 1 {
///         return Err(std::io::ErrorKind::ConnectionReset.into());
///     }
///     into[..5].copy_from_slice(b"hello");
///     Ok(5)
/// }
///
/// let mut pipeline = Pipeline::new(Mode::Plain, Limits { messages: 2, rooms: 2 });
/// block_on(async {
///     for client in [0u32, 1, 2] {
///         let mut message = pipeline.message(client, None).await;
///         let mut room = pipeline.room().await;
///         match read(client, room.bytes_mut()).await {
///             Ok(n) => {
///                 pipeline.commit(&mut message, &mut room, n);
///                 pipeline.end(message);
///             }
///             // Not `?`: the message goes back first.
///             Err(_) => pipeline.abort(message),
///         }
///     }
///     // Every event taken: the Pipeline may then be dropped.
///     let mut seen = Vec::new();
///     while seen.len() < 3 {
///         match pipeline.next_event() {
///             Some(Event::Hashed { tag, hash, .. }) => seen.push((tag, Some(hash))),
///             Some(Event::Aborted { tag, .. }) => seen.push((tag, None)),
///             Some(_) => unreachable!(),
///             None => pipeline.ready().await,
///         }
///     }
///     let hello = Some(convenience::hash(b"hello"));
///     assert_eq!(seen, [(0, hello), (1, None), (2, hello)]);
/// });
/// ```
pub struct Pipeline<T, B = ()> {
    id: u64,
    limits: Limits,
    group_count: usize,
    source_slot: Option<usize>,
    shared: *mut Shared,
    slots: Box<[SlotMeta<T>]>,
    room_streams: Box<[RoomStream]>,
    owners: Box<[UnsafeCell<MaybeUninit<B>>]>,
    batches: Box<[UnsafeCell<MaybeUninit<(Batch<'static>, Box<[[u8; OUT_LEN]]>)>>]>,
    /// FIFO ring of free message slots: slots cycle around the full ring
    /// before reuse so worker-written cache lines cool before another worker writes them.
    free_slots: Ring<u32>,
    open_messages: usize,
    end_queue: Ring<u32>,
    events: Ring<Event<T, B>>,
    /// FIFO ring of free rooms, refilled by a circular cursor scan over `rooms.refs`.
    local_free_rooms: Ring<u32>,
    room_cursor: usize,
    /// FIFO ring of free `PieceNode`s.
    free_pieces: Ring<u32>,
    /// Monotonic circular cursor over `0..group_count` (`MemberGroup`s cycle in FIFO order).
    group_cursor: usize,
    open_group: Option<(u32, u64)>,
    local_count: usize,
    local_bytes: usize,
    local_batch: Option<usize>,
    local_members: Box<[Member; MEMBERS]>,
    local_rooms: Box<[u32; MEMBERS]>,
    next_loan_epoch: u64,
    next_group_epoch: u64,
    assigned_group_epoch: Box<[u64]>,
    seen_group_epoch: Box<[u64]>,
    action_buf: [u64; MAX_SLOT_PIECES],
}

// Sound: the Pipeline owns its tags and owners (hence `T: Send, B: Send`); the
// raw pointers are to its never-freed Shared and into buffers it holds, which
// workers read only through tasks it dispatched.
unsafe impl<T: Send, B: Send> Send for Pipeline<T, B> {}

impl<T> Pipeline<T, ()> {
    /// Create a pipeline in `mode` with fixed memory `limits`.
    pub fn new(mode: Mode, limits: Limits) -> Self {
        Self::create(mode, limits)
    }
}

// `B: 'static`: the workers read an owner's bytes until its message is
// hashed, and a Pipeline can be forgotten (`core::mem::forget`) while they
// do, which no borrow outlives soundly. An owner that borrows nothing
// keeps its bytes for as long as the forgotten Pipeline holds it: for
// good (BLAKE3 #417: a `&[u8]` was an owner, and safe code could free the
// bytes under the workers). Borrowed bytes are [`Pipeline::lent`]'s, which
// is unsafe for that reason.
impl<T, B: AsRef<[u8]> + 'static> Pipeline<T, B> {
    /// Create a pipeline that also accepts owned message buffers of type `B`
    /// ([`Pipeline::owned`]): a type that owns its bytes, such as `Vec<u8>`
    /// or `Box<[u8]>`.
    ///
    /// ```
    /// use blake3_servil::{Mode, throughput::{Limits, Pipeline}};
    /// fn pipeline_of<B: AsRef<[u8]> + 'static>(_first: B) -> Pipeline<(), B> {
    ///     Pipeline::with_owners(Mode::Plain, Limits { messages: 1, rooms: 0 })
    /// }
    /// # blake3_servil::initialize_multithreaded(None);
    /// let _ = pipeline_of(vec![0u8; 64]);
    /// ```
    ///
    /// A borrowed slice is no owner (the same code but for the borrow, so
    /// this fails for that reason):
    ///
    /// ```compile_fail
    /// use blake3_servil::{Mode, throughput::{Limits, Pipeline}};
    /// fn pipeline_of<'a>(_first: &'a [u8]) -> Pipeline<(), &'a [u8]> {
    ///     Pipeline::with_owners(Mode::Plain, Limits { messages: 1, rooms: 0 })
    /// }
    /// ```
    pub fn with_owners(mode: Mode, limits: Limits) -> Self {
        Self::create(mode, limits)
    }

    /// Hash `owner`'s bytes in place and return `owner` in the message's
    /// [`Event::Hashed`].
    #[inline]
    pub fn owned(&mut self, message: Message, owner: B) {
        let (pid, s) = message.take();
        assert_eq!(pid, self.id, "Message belongs to a different Pipeline");
        self.assert_uncommitted(s);
        self.open_messages -= 1;

        // Sound: the slot is open and has no owner yet (assert_uncommitted);
        // the owner stays in the slot, where its bytes cannot move, until the
        // slot's event reads it out or the Pipeline's drop drops it.
        let b_ref = unsafe { (*self.owners[s].get()).write(owner) };
        let bytes: &[u8] = b_ref.as_ref();
        let (ptr, len) = (bytes.as_ptr(), bytes.len());
        self.submit_slice(s, ptr, len, true);
        self.end_queue.push_back(s as u32);
    }
}

impl<T, B> Pipeline<T, B> {
    #[inline(always)]
    fn shared(&self) -> &Shared {
        // Sound: `shared` is this Pipeline's Shared (create), never freed.
        unsafe { &*self.shared }
    }

    fn create(mode: Mode, limits: Limits) -> Self {
        crate::lanes::require();
        prepare_thread();
        assert!(limits.messages > 0, "Pipeline Limits.messages must be at least 1");
        let (key, flags) = mode.key_and_flags();
        let m = limits.messages;
        let p_cap = limits.rooms + m + 1;
        let g_cap = m.clamp(4, 256);
        let actions_cap = m * MAX_SLOT_PIECES + p_cap + g_cap;
        // An action names a slot, a piece or a group in 29 bits (pack_arg).
        assert!(m < 1 << 29 && p_cap < 1 << 29, "Pipeline Limits beyond what an action can name");

        let storage = PipeStorage::acquire(actions_cap, m, limits.rooms, g_cap, &key, flags);
        // Sound: this Pipeline now holds the storage alone
        // (PipeStorage::acquire), and these cells are its holder's.
        let next_loan_epoch = unsafe { *storage.saved_loan_epoch.get() };
        let next_group_epoch = unsafe { *storage.saved_group_epoch.get() };
        let source_slot = storage.source.register();
        crate::lanes::sources::opened();

        let mut local_free_rooms = Ring::new(limits.rooms.max(1));
        for i in 0..limits.rooms {
            local_free_rooms.push_back(i as u32);
        }
        // The storage's `Shared`, where it has one of this size: every task
        // of the Pipeline before is done (its drop saw to that), and only
        // the mode is this Pipeline's own.
        // Sound: as the saved epochs above. A Shared the storage keeps was the
        // Pipeline before's, whose every task is done (its drop saw to that),
        // so reading its lengths and writing its mode race with no worker.
        let held = unsafe { *storage.shared.get() };
        let shared: *mut Shared = if !held.is_null() && unsafe { (&(*held).hashes).len() >= m } {
            // A storage has one count of rooms, so pieces enough too.
            assert!(unsafe { (&(*held).pieces).len() } >= p_cap, "a Shared of as many messages has as many pieces");
            unsafe {
                (*held).key = key;
                (*held).flags = flags;
            }
            held
        } else {
            // One it replaces stays allocated: a worker of an earlier
            // Pipeline may still be reading it.
            let new = Box::into_raw(Box::new(Shared {
                key,
                flags,
                storage,
                hashes: (0..m).map(|_| UnsafeCell::new([0u8; OUT_LEN])).collect(),
                cvs: (0..m).map(|_| UnsafeCell::new([[0u8; OUT_LEN]; MAX_SLOT_PIECES + 1])).collect(),
                slot_done: (0..m).map(|_| OwnLine(AtomicBool::new(true))).collect(),
                tasks_left: (0..m).map(|_| OwnLine(AtomicUsize::new(0))).collect(),
                multi: (0..m)
                    .map(|_| {
                        UnsafeCell::new(MultiDesc {
                            input: core::ptr::null(),
                            total_len: 0,
                            piece_len: TASK_LEN,
                            pieces: 0,
                            batch: None,
                            digests: core::ptr::null_mut(),
                            loan_epoch: 0,
                        })
                    })
                    .collect(),
                pieces: (0..p_cap)
                    .map(|_| PieceNode {
                        room: UnsafeCell::new(0),
                        offset: UnsafeCell::new(0),
                        len: UnsafeCell::new(0),
                        counter: UnsafeCell::new(0),
                        out: OwnLine(UnsafeCell::new([0u8; 64])),
                        done: OwnLine(AtomicBool::new(true)),
                        next: UnsafeCell::new(NONE),
                    })
                    .collect(),
            }));
            // Sound: as above; the Shared is kept for the next Pipeline.
            unsafe { *storage.shared.get() = new };
            new
        };

        let mut free_slots = Ring::new(m);
        for i in 0..m {
            free_slots.push_back(i as u32);
        }
        let mut free_pieces = Ring::new(p_cap);
        for i in 0..p_cap {
            free_pieces.push_back(i as u32);
        }

        Pipeline {
            id: NEXT_PIPELINE_ID.fetch_add(1, Ordering::Relaxed),
            limits,
            group_count: g_cap,
            source_slot,
            shared,
            slots: (0..m).map(|_| SlotMeta { tag: None, kind: SlotKind::Free }).collect(),
            room_streams: vec![RoomStream::EMPTY; m].into_boxed_slice(),
            owners: (0..m).map(|_| UnsafeCell::new(MaybeUninit::uninit())).collect(),
            batches: (0..m).map(|_| UnsafeCell::new(MaybeUninit::uninit())).collect(),
            free_slots,
            open_messages: 0,
            end_queue: Ring::new(m),
            events: Ring::new(m),
            local_free_rooms,
            room_cursor: 0,
            free_pieces,
            group_cursor: 0,
            open_group: None,
            local_count: 0,
            local_bytes: 0,
            local_batch: None,
            local_members: Box::new([Member { input: core::ptr::null(), len: 0, out: core::ptr::null_mut() }; MEMBERS]),
            local_rooms: Box::new([NONE; MEMBERS]),
            next_loan_epoch,
            next_group_epoch,
            assigned_group_epoch: vec![0u64; g_cap].into_boxed_slice(),
            seen_group_epoch: vec![0u64; g_cap].into_boxed_slice(),
            action_buf: [0; MAX_SLOT_PIECES],
        }
    }

    /// Wait for a free message slot and open a [`Message`].
    #[inline(always)]
    pub fn message(&mut self, tag: T, length_hint: Option<u64>) -> impl Future<Output = Message> + '_ {
        let _ = length_hint;
        MessageFut { pipe: self, tag: Some(tag) }
    }

    fn poll_message(&mut self, cx: &mut Context<'_>) -> Poll<u32> {
        if let Some(slot) = self.free_slots.pop_front() {
            return Poll::Ready(slot);
        }
        self.reap();
        if let Some(slot) = self.free_slots.pop_front() {
            return Poll::Ready(slot);
        }
        self.assert_message_can_progress();
        if self.front_uses_open_group() {
            self.publish_open();
        }
        if in_block_on() {
            while self.shared().storage.source.run_waiting_action() {
                self.reap();
                if let Some(slot) = self.free_slots.pop_front() {
                    return Poll::Ready(slot);
                }
            }
            if block_on_spinning() {
                if self.source_slot.is_some() {
                    self.shared().storage.source.wake_for_waiting();
                }
                return Poll::Pending;
            }
        }
        self.shared().storage.waker.register(cx);
        self.reap();
        if let Some(slot) = self.free_slots.pop_front() {
            self.shared().storage.waker.clear();
            return Poll::Ready(slot);
        }
        self.assert_message_can_progress();
        if self.source_slot.is_some() {
            self.shared().storage.source.wake_for_waiting();
        }
        Poll::Pending
    }

    fn assert_message_can_progress(&self) {
        assert!(
            self.events.len() < self.limits.messages,
            "Pipeline::message cannot make progress: {} finished events wait to be pulled (call next_event)",
            self.limits.messages
        );
        assert!(
            self.open_messages < self.limits.messages,
            "Pipeline::message cannot make progress: all {} message slots are held open by the caller",
            self.limits.messages
        );
    }

    /// Wait for a [`Room`] of [`ROOM_LEN`] bytes.
    #[inline(always)]
    pub fn room(&mut self) -> impl Future<Output = Room> + '_ {
        RoomFut { pipe: self }
    }

    #[inline(always)]
    fn pop_cached_room(&mut self) -> Option<Room> {
        let idx = self.local_free_rooms.pop_front()?;
        self.room_cursor = (idx as usize + 1) % self.limits.rooms;
        self.shared().storage.rooms.init_taken(idx);
        Some(self.make_room(idx))
    }

    #[inline]
    fn try_take_room(&mut self) -> Option<Room> {
        if self.local_free_rooms.is_empty() {
            // Sound: as `shared()`.
            let shared = unsafe { &*self.shared };
            let count = shared.storage.rooms.count;
            let start = self.room_cursor;
            for step in 0..count {
                let i = (start + step) % count;
                if shared.storage.rooms.refs[i].load(Ordering::Acquire) == 0 {
                    self.local_free_rooms.push_back(i as u32);
                }
            }
        }
        self.pop_cached_room()
    }

    fn poll_room(&mut self, cx: &mut Context<'_>) -> Poll<Room> {
        if let Some(r) = self.try_take_room() {
            return Poll::Ready(r);
        }
        self.reap();
        self.flush_sealed_short_rooms();
        self.publish_open();
        if let Some(r) = self.try_take_room() {
            return Poll::Ready(r);
        }
        self.assert_room_can_progress();
        if in_block_on() {
            while self.shared().storage.source.run_waiting_action() {
                if let Some(r) = self.try_take_room() {
                    return Poll::Ready(r);
                }
            }
            if block_on_spinning() {
                if self.source_slot.is_some() {
                    self.shared().storage.source.wake_for_waiting();
                }
                return Poll::Pending;
            }
        }
        self.shared().storage.waker.register(cx);
        if let Some(r) = self.try_take_room() {
            self.shared().storage.waker.clear();
            return Poll::Ready(r);
        }
        self.assert_room_can_progress();
        if self.source_slot.is_some() {
            self.shared().storage.source.wake_for_waiting();
        }
        Poll::Pending
    }

    #[inline(always)]
    fn make_room(&self, index: u32) -> Room {
        let storage = self.shared().storage;
        storage.holders.fetch_add(1, Ordering::Relaxed);
        Room { pipeline_id: self.id, index, offset: 0, commits: 0, ptr: storage.rooms.room_ptr(index as usize), storage }
    }

    fn assert_room_can_progress(&self) {
        assert!(self.limits.rooms > 0, "Pipeline::room called on a Pipeline with Limits.rooms == 0");
        assert!(
            self.shared().storage.rooms.caller_held.load(Ordering::Acquire) < self.limits.rooms,
            "Pipeline::room cannot make progress: all {} rooms are held by the caller",
            self.limits.rooms
        );
    }

    /// Commit the first `written` bytes of `room` as `message`'s next bytes.
    ///
    /// `room` advances to the next 64-byte boundary. Every commit to a
    /// message except its last must be a whole room (`ROOM_LEN` bytes).
    #[inline(always)]
    pub fn commit(&mut self, message: &mut Message, room: &mut Room, written: usize) {
        assert_eq!(message.pipeline_id, self.id, "Message belongs to a different Pipeline");
        assert_eq!(room.pipeline_id, self.id, "Room belongs to a different Pipeline");
        assert!(written <= room.len(), "commit wrote {written} bytes into a Room with {} left", room.len());

        let s = message.slot as usize;
        if matches!(self.slots[s].kind, SlotKind::OpenEmpty) && written < ROOM_LEN {
            let start_offset = room.offset;
            room.offset = ((start_offset as usize + written).div_ceil(64) * 64).min(ROOM_LEN) as u32;
            room.take_credit();
            self.slots[s].kind = SlotKind::OpenShort { room: room.index, offset: start_offset, len: written as u32 };
            return;
        }
        self.commit_multi(s, room, written);
    }

    #[inline(never)]
    fn commit_multi(&mut self, s: usize, room: &mut Room, written: usize) {
        let start_offset = room.offset;
        let room_idx = room.index;

        match self.slots[s].kind {
            SlotKind::OpenEmpty => {
                debug_assert_eq!(written, ROOM_LEN);
                debug_assert_eq!(start_offset, 0);
                room.offset = ROOM_LEN as u32;
                self.room_streams[s] = RoomStream::EMPTY;
                self.slots[s].kind = SlotKind::OpenRooms { sealed: false };
            }
            SlotKind::OpenShort { .. } | SlotKind::OpenShortDispatched { .. } | SlotKind::OpenRooms { sealed: true } => {
                panic!("commit after a commit of less than ROOM_LEN: every commit to a message but its last is a whole room");
            }
            SlotKind::OpenRooms { ref mut sealed } => {
                if written == ROOM_LEN {
                    debug_assert_eq!(start_offset, 0);
                    room.offset = ROOM_LEN as u32;
                } else {
                    *sealed = true;
                    room.offset = ((start_offset as usize + written).div_ceil(64) * 64).min(ROOM_LEN) as u32;
                    if written == 0 {
                        return;
                    }
                }
            }
            _ => unreachable!("Message token refers to a non-open slot"),
        }

        room.take_credit();
        let counter = self.room_streams[s].room_pieces * (ROOM_LEN / CHUNK_LEN) as u64;
        self.room_streams[s].room_pieces += 1;

        self.drain_slot_pieces(s);
        let pnode = self.alloc_piece_node(s);
        // Sound: as `shared()`. The node came off the free list, so no task
        // uses it: its fields are the owner's to write before the dispatch
        // below.
        let shared = unsafe { &*self.shared };
        let node = &shared.pieces[pnode as usize];
        unsafe {
            *node.room.get() = room_idx;
            *node.offset.get() = start_offset;
            *node.len.get() = written as u32;
            *node.counter.get() = counter;
            *node.next.get() = NONE;
        }
        node.done.store(false, Ordering::Relaxed);

        let rs = &mut self.room_streams[s];
        if rs.tail_piece == NONE {
            rs.head_piece = pnode;
        } else {
            // Sound: `next` is the owner's alone (workers read a node's other
            // fields).
            unsafe { *shared.pieces[rs.tail_piece as usize].next.get() = pnode };
        }
        rs.tail_piece = pnode;

        let arg = pack_arg(KIND_ROOM_PIECE, pnode, 0, written >= SME2_MIN_LEN);
        self.dispatch_one(arg);
    }

    fn alloc_piece_node(&mut self, current_slot: usize) -> u32 {
        if let Some(p) = self.free_pieces.pop_front() {
            return p;
        }
        loop {
            self.drain_slot_pieces(current_slot);
            self.reap();
            if let Some(p) = self.free_pieces.pop_front() {
                return p;
            }
            if !self.shared().storage.source.run_waiting_action() {
                core::hint::spin_loop();
            }
        }
    }

    /// Finish `message` and queue its [`Event::Hashed`] in end order.
    #[inline(always)]
    pub fn end(&mut self, message: Message) {
        let (pid, s) = message.take();
        assert_eq!(pid, self.id, "Message belongs to a different Pipeline");
        self.open_messages -= 1;

        match self.slots[s].kind {
            SlotKind::OpenShort { room: room_idx, offset, len } => {
                // Sound: the offset is within the room (commit checked
                // `written` against what was left).
                let ptr = unsafe { self.shared().storage.rooms.room_ptr(room_idx as usize).add(offset as usize) };
                let out = self.shared().hashes[s].get() as *mut u8;
                let (group, epoch) = self.push_member(ptr, len as usize, out, room_idx, None);
                self.slots[s].kind = SlotKind::WaitSingle { has_owner: false, group, epoch };
                self.end_queue.push_back(s as u32);
            }
            _ => self.end_slow(s),
        }
    }

    #[inline(never)]
    fn end_slow(&mut self, s: usize) {
        match self.slots[s].kind {
            SlotKind::OpenEmpty => {
                let empty_hash = crate::hash_serial(&[], &self.shared().key, self.shared().flags);
                // Sound: no task runs for an empty message, so the slot's hash
                // is the owner's to write.
                unsafe { *self.shared().hashes[s].get() = *empty_hash.as_bytes() };
                self.slots[s].kind = SlotKind::WaitSingle { has_owner: false, group: NONE, epoch: 0 };
            }
            SlotKind::OpenShortDispatched { group, epoch } => {
                self.slots[s].kind = SlotKind::WaitSingle { has_owner: false, group, epoch };
            }
            SlotKind::OpenRooms { .. } => {
                self.shared().slot_done[s].store(false, Ordering::Relaxed);
                self.slots[s].kind = SlotKind::WaitRoomMulti;
                self.drain_slot_pieces(s);
            }
            _ => unreachable!(),
        }
        self.end_queue.push_back(s as u32);
    }

    /// Abort `message` and queue its [`Event::Aborted`] in end order.
    pub fn abort(&mut self, message: Message) {
        let (pid, s) = message.take();
        assert_eq!(pid, self.id, "Message belongs to a different Pipeline");
        self.open_messages -= 1;

        match self.slots[s].kind {
            SlotKind::OpenEmpty => {
                self.slots[s].kind = SlotKind::WaitAborted { group: NONE, epoch: 0 };
            }
            SlotKind::OpenShort { room: room_idx, .. } => {
                if self.shared().storage.rooms.release_n(room_idx, 1) {
                    self.shared().storage.waker.notify_after_seqcst();
                }
                self.slots[s].kind = SlotKind::WaitAborted { group: NONE, epoch: 0 };
            }
            SlotKind::OpenShortDispatched { group, epoch } => {
                self.slots[s].kind = SlotKind::WaitAborted { group, epoch };
            }
            SlotKind::OpenRooms { .. } => {
                self.slots[s].kind = SlotKind::WaitAborted { group: NONE, epoch: 0 };
                self.drain_slot_pieces(s);
            }
            _ => unreachable!(),
        }
        self.end_queue.push_back(s as u32);
    }

    /// Hash `bytes` in place on loan, returning a [`Lent`] guard.
    ///
    /// # Safety
    /// The returned [`Lent`] guard must be dropped and never leaked
    /// (`core::mem::forget`, `Box::leak`, a reference cycle, or a leaked
    /// task holding it).
    #[inline(always)]
    pub unsafe fn lent<'a>(&mut self, message: Message, bytes: &'a [u8]) -> Lent<'a> {
        let (pid, s) = message.take();
        assert_eq!(pid, self.id, "Message belongs to a different Pipeline");
        self.assert_uncommitted(s);
        self.open_messages -= 1;

        let (completed, epoch) = if bytes.len() <= TASK_LEN {
            let out = self.shared().hashes[s].get() as *mut u8;
            let (group, epoch) = self.push_member(bytes.as_ptr(), bytes.len(), out, NONE, None);
            self.slots[s].kind = SlotKind::WaitSingle { has_owner: false, group, epoch };
            (&self.shared().storage.groups[group as usize].completed_epoch.0, epoch)
        } else {
            let epoch = self.next_loan_epoch;
            self.next_loan_epoch += 1;
            self.submit_multi_slice(s, bytes.as_ptr(), bytes.len(), false, epoch);
            (&self.shared().storage.loan_completed[s].0, epoch)
        };
        self.end_queue.push_back(s as u32);
        Lent { completed, epoch, _bytes: PhantomData }
    }

    /// Hash `batch` into `digests` and return both in [`Event::Batch`].
    pub fn batch(&mut self, message: Message, batch: Batch<'static>, digests: Box<[[u8; OUT_LEN]]>) {
        let (pid, s) = message.take();
        assert_eq!(pid, self.id, "Message belongs to a different Pipeline");
        self.assert_uncommitted(s);
        crate::many::assert_slots(batch.bytes(), batch.message_len(), batch.slot(), digests.len());
        self.open_messages -= 1;

        // Sound: the slot is open and holds no batch (assert_uncommitted); the
        // batch and digests stay in the slot until its event reads them out or
        // the Pipeline's drop drops them.
        let hold = unsafe { (*self.batches[s].get()).write((batch, digests)) };
        let msg_len = hold.0.message_len();
        let slot_len = hold.0.slot();
        let bytes = hold.0.bytes();
        let count = hold.1.len();
        let digests_ptr = hold.1.as_mut_ptr();

        if count == 0 {
            self.shared().slot_done[s].store(true, Ordering::Relaxed);
            self.slots[s].kind = SlotKind::WaitBatch { group: NONE, epoch: 0 };
        } else if bytes.len() < TASK_LEN {
            let (group, epoch) = self.push_member(bytes.as_ptr(), bytes.len(), digests_ptr as *mut u8, NONE, Some(msg_len));
            self.slots[s].kind = SlotKind::WaitBatch { group, epoch };
        } else {
            self.slots[s].kind = SlotKind::WaitBatch { group: NONE, epoch: 0 };
            let per_part = (TASK_LEN / slot_len).max(1).max(count.div_ceil(MAX_SLOT_PIECES));
            let pieces = count.div_ceil(per_part);
            debug_assert!((1..=MAX_SLOT_PIECES).contains(&pieces));
            self.shared().slot_done[s].store(false, Ordering::Relaxed);
            self.shared().tasks_left[s].store(pieces, Ordering::Relaxed);
            // Sound: no task of this slot is in flight (it was open), so its
            // description is the owner's to write before the dispatch below.
            unsafe {
                *self.shared().multi[s].get() = MultiDesc {
                    input: bytes.as_ptr(),
                    total_len: count,
                    piece_len: 0,
                    pieces,
                    batch: Some((msg_len, slot_len, per_part)),
                    digests: digests_ptr,
                    loan_epoch: 0,
                }
            };
            let sme2 = per_part * slot_len >= SME2_MIN_LEN;
            for p in 0..pieces {
                self.action_buf[p] = pack_arg(KIND_MULTI, s as u32, p as u32, sme2);
            }
            self.dispatch_many(pieces);
        }
        self.end_queue.push_back(s as u32);
    }

    /// Pull the next finished [`Event`], if one is ready.
    #[inline(always)]
    pub fn next_event(&mut self) -> Option<Event<T, B>> {
        if let Some(ev) = self.events.pop_front() {
            return Some(ev);
        }
        self.try_reap_one()
    }

    /// Wait until [`Pipeline::next_event`] has an [`Event`] ready.
    #[inline(always)]
    pub fn ready(&mut self) -> impl Future<Output = ()> + '_ {
        ReadyFut { pipe: self }
    }

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        self.reap();
        if !self.events.is_empty() {
            return Poll::Ready(());
        }
        assert!(!self.end_queue.is_empty(), "Pipeline::ready cannot make progress: no ended messages are in flight");
        if self.front_uses_open_group() {
            self.publish_open();
            self.reap();
            if !self.events.is_empty() {
                return Poll::Ready(());
            }
        }
        if in_block_on() {
            while self.shared().storage.source.run_waiting_action() {
                self.reap();
                if !self.events.is_empty() {
                    return Poll::Ready(());
                }
            }
            if block_on_spinning() {
                if self.source_slot.is_some() {
                    self.shared().storage.source.wake_for_waiting();
                }
                return Poll::Pending;
            }
        }
        self.shared().storage.waker.register(cx);
        self.reap();
        if !self.events.is_empty() {
            self.shared().storage.waker.clear();
            return Poll::Ready(());
        }
        if self.source_slot.is_some() {
            self.shared().storage.source.wake_for_waiting();
        }
        Poll::Pending
    }

    #[inline(always)]
    fn assert_uncommitted(&self, s: usize) {
        match self.slots[s].kind {
            SlotKind::OpenEmpty => {}
            SlotKind::OpenShort { .. } | SlotKind::OpenShortDispatched { .. } | SlotKind::OpenRooms { .. } => {
                panic!("bytes given two ways: Message already has committed room bytes")
            }
            _ => unreachable!("Message token refers to a non-open slot"),
        }
    }

    #[inline]
    fn submit_slice(&mut self, s: usize, ptr: *const u8, len: usize, has_owner: bool) {
        if len <= TASK_LEN {
            let out = self.shared().hashes[s].get() as *mut u8;
            let (group, epoch) = self.push_member(ptr, len, out, NONE, None);
            self.slots[s].kind = SlotKind::WaitSingle { has_owner, group, epoch };
            return;
        }
        self.submit_multi_slice(s, ptr, len, has_owner, 0);
    }

    #[inline(never)]
    fn submit_multi_slice(&mut self, s: usize, ptr: *const u8, len: usize, has_owner: bool, loan_epoch: u64) {
        let mut piece_len = TASK_LEN;
        while len.div_ceil(piece_len) > MAX_SLOT_PIECES {
            piece_len *= 2;
        }
        let pieces = len.div_ceil(piece_len);
        debug_assert!((2..=MAX_SLOT_PIECES).contains(&pieces));
        self.shared().slot_done[s].store(false, Ordering::Relaxed);
        self.shared().tasks_left[s].store(pieces, Ordering::Relaxed);
        // Sound: as `batch`'s, above.
        unsafe {
            *self.shared().multi[s].get() = MultiDesc { input: ptr, total_len: len, piece_len, pieces, batch: None, digests: core::ptr::null_mut(), loan_epoch }
        };
        self.slots[s].kind = SlotKind::WaitMulti { has_owner };
        for p in 0..pieces {
            self.action_buf[p] = pack_arg(KIND_MULTI, s as u32, p as u32, true);
        }
        self.dispatch_many(pieces);
    }

    fn flush_sealed_short_rooms(&mut self) {
        for s in 0..self.slots.len() {
            if let SlotKind::OpenShort { room: room_idx, offset, len } = self.slots[s].kind {
                // Sound: as in `end`.
                let ptr = unsafe { self.shared().storage.rooms.room_ptr(room_idx as usize).add(offset as usize) };
                let out = self.shared().hashes[s].get() as *mut u8;
                let (group, epoch) = self.push_member(ptr, len as usize, out, room_idx, None);
                self.slots[s].kind = SlotKind::OpenShortDispatched { group, epoch };
            }
        }
    }

    /// Next free `MemberGroup` in FIFO ring order (`0, 1, ..., group_count - 1, 0, ...`),
    /// so a group completed on one worker CPU is not overwritten until a full lap later.
    fn acquire_free_group(&mut self) -> u32 {
        let count = self.group_count;
        loop {
            for _ in 0..count {
                let g = self.group_cursor;
                self.group_cursor = (g + 1) % count;
                let assigned = self.assigned_group_epoch[g];
                if assigned == 0 {
                    return g as u32;
                }
                if self.seen_group_epoch[g] < assigned {
                    self.seen_group_epoch[g] = self.shared().storage.groups[g].completed_epoch.load(Ordering::Acquire);
                }
                if self.seen_group_epoch[g] >= assigned {
                    self.assigned_group_epoch[g] = 0;
                    return g as u32;
                }
            }
            if !self.shared().storage.source.run_waiting_action() {
                core::hint::spin_loop();
            }
        }
    }

    #[inline(always)]
    fn push_member(&mut self, input: *const u8, len: usize, out: *mut u8, room: u32, batch: Option<usize>) -> (u32, u64) {
        if self.open_group.is_some() && self.local_batch != batch {
            self.publish_open();
        }
        let (g, epoch) = match self.open_group {
            Some(pair) => pair,
            None => self.open_new_group(batch),
        };
        let idx = self.local_count;
        self.local_members[idx] = Member { input, len, out };
        self.local_rooms[idx] = room;
        let after = idx + 1;
        let bytes = self.local_bytes + len;
        self.local_count = after;
        self.local_bytes = bytes;
        let byte_limit = if batch.is_some() { TASK_LEN } else { TASK_LEN / 4 };
        if after == MEMBERS || bytes >= byte_limit {
            self.publish_open();
        }
        (g, epoch)
    }

    #[inline(never)]
    fn open_new_group(&mut self, batch: Option<usize>) -> (u32, u64) {
        let g = self.acquire_free_group();
        let epoch = self.next_group_epoch;
        self.next_group_epoch += 1;
        self.local_count = 0;
        self.local_bytes = 0;
        self.local_batch = batch;
        self.open_group = Some((g, epoch));
        (g, epoch)
    }

    #[inline]
    fn front_uses_open_group(&self) -> bool {
        let Some((open_g, _)) = self.open_group else { return false };
        let Some(&front) = self.end_queue.front() else { return false };
        match self.slots[front as usize].kind {
            SlotKind::WaitSingle { group, .. } | SlotKind::WaitBatch { group, .. } | SlotKind::WaitAborted { group, .. } => group == open_g,
            _ => false,
        }
    }

    #[inline(never)]
    fn publish_open(&mut self) {
        let Some((group_idx, epoch)) = self.open_group.take() else { return };
        let count = self.local_count;
        let group = &self.shared().storage.groups[group_idx as usize];
        // Sound: the group is not in flight (acquire_free_group gave it), so
        // its task, metas and epoch are the owner's to write before the
        // dispatch below.
        unsafe {
            let task = &mut *group.task.get();
            task.batch = self.local_batch;
            task.len = self.local_bytes;
            task.members = count;
            task.member[..count].copy_from_slice(&self.local_members[..count]);
            let metas = &mut *group.metas.get();
            for i in 0..count {
                metas[i] = GroupMemberMeta { room: self.local_rooms[i] };
            }
            *group.epoch.get() = epoch;
        }
        self.assigned_group_epoch[group_idx as usize] = epoch;
        let arg = pack_arg(KIND_GROUP, group_idx, 0, false);
        self.dispatch_one(arg);
    }

    #[inline]
    fn dispatch_one(&mut self, arg: u64) {
        let ctx = self.shared as *const Shared as *const ();
        match self.source_slot {
            Some(_) => self.shared().storage.source.publish_actions(Shared::run_action, ctx, core::slice::from_ref(&arg)),
            // Sound: `ctx` is this Pipeline's Shared, as run_action requires.
            None => unsafe { Shared::run_action(ctx, arg, pool_platform()) },
        }
    }

    fn dispatch_many(&mut self, count: usize) {
        let ctx = self.shared as *const Shared as *const ();
        match self.source_slot {
            Some(_) => self.shared().storage.source.publish_actions(Shared::run_action, ctx, &self.action_buf[..count]),
            None => {
                for i in 0..count {
                    let arg = self.action_buf[i];
                    // Sound: as dispatch_one's.
                    unsafe { Shared::run_action(ctx, arg, pool_platform()) };
                }
            }
        }
    }

    #[inline(always)]
    fn group_ready(&mut self, group: u32, epoch: u64) -> bool {
        if group == NONE {
            return true;
        }
        let gi = group as usize;
        if self.seen_group_epoch[gi] >= epoch {
            return true;
        }
        let now = self.shared().storage.groups[gi].completed_epoch.load(Ordering::Acquire);
        self.seen_group_epoch[gi] = now;
        now >= epoch
    }

    fn drain_slot_pieces(&mut self, s: usize) {
        // Sound: as `shared()`.
        let shared = unsafe { &*self.shared };
        let (key, flags, platform) = (shared.key, shared.flags, Platform::detect());
        let parent = |l: &CVBytes, r: &CVBytes| crate::hasher::parent_output(l, r, &key, flags, platform);
        let ended = matches!(self.slots[s].kind, SlotKind::WaitRoomMulti);
        let aborted = matches!(self.slots[s].kind, SlotKind::WaitAborted { .. });

        loop {
            let head = self.room_streams[s].head_piece;
            if head == NONE || !shared.pieces[head as usize].done.load(Ordering::Acquire) {
                return;
            }
            // Sound: the piece is done (checked above), so its `next` and
            // `out` are the owner's again.
            let next = unsafe { *shared.pieces[head as usize].next.get() };
            let out = unsafe { *shared.pieces[head as usize].out.0.get() };
            self.free_pieces.push_back(head);

            let rs = &mut self.room_streams[s];
            rs.head_piece = next;
            if next == NONE {
                rs.tail_piece = NONE;
            }
            if aborted {
                continue;
            }

            if ended && rs.room_pieces == 1 {
                let root = parent(out[..OUT_LEN].try_into().unwrap(), out[OUT_LEN..].try_into().unwrap()).root_hash();
                // Sound: every piece of this slot is done and merged here, on
                // the owner's thread: the slot's hash and values have no other
                // reader or writer.
                unsafe { *shared.hashes[s].get() = *root.as_bytes() };
                shared.slot_done[s].store(true, Ordering::Relaxed);
                return;
            }

            let cv: CVBytes = if rs.merged_pieces == 0 {
                parent(out[..OUT_LEN].try_into().unwrap(), out[OUT_LEN..].try_into().unwrap()).chaining_value()
            } else {
                out[..OUT_LEN].try_into().unwrap()
            };
            // Sound: a room message's values are written here alone, on the
            // owner's thread (workers write a slice message's, never both for
            // one slot).
            let cvs = unsafe { &mut *shared.cvs[s].get() };
            while rs.stack_len > rs.merged_pieces.count_ones() as usize {
                let right = cvs[rs.stack_len - 1];
                let left = cvs[rs.stack_len - 2];
                cvs[rs.stack_len - 2] = parent(&left, &right).chaining_value();
                rs.stack_len -= 1;
            }
            cvs[rs.stack_len] = cv;
            rs.stack_len += 1;
            rs.merged_pieces += 1;

            if ended && rs.merged_pieces == rs.room_pieces {
                let mut right = cvs[rs.stack_len - 1];
                rs.stack_len -= 1;
                while rs.stack_len > 1 {
                    right = parent(&cvs[rs.stack_len - 1], &right).chaining_value();
                    rs.stack_len -= 1;
                }
                let root = parent(&cvs[0], &right).root_hash();
                // Sound: as the one-room root, above.
                unsafe { *shared.hashes[s].get() = *root.as_bytes() };
                shared.slot_done[s].store(true, Ordering::Relaxed);
                return;
            }
        }
    }

    #[inline(always)]
    fn try_reap_one(&mut self) -> Option<Event<T, B>> {
        let &front = self.end_queue.front()?;
        let s = front as usize;

        let ready = match self.slots[s].kind {
            SlotKind::WaitSingle { group, epoch, .. } => self.group_ready(group, epoch),
            SlotKind::WaitMulti { .. } => self.shared().slot_done[s].load(Ordering::Acquire),
            SlotKind::WaitBatch { group, epoch } => {
                if group != NONE {
                    self.group_ready(group, epoch)
                } else {
                    self.shared().slot_done[s].load(Ordering::Acquire)
                }
            }
            SlotKind::WaitRoomMulti => {
                self.drain_slot_pieces(s);
                self.shared().slot_done[s].load(Ordering::Acquire)
            }
            SlotKind::WaitAborted { group, epoch } => {
                self.drain_slot_pieces(s);
                let rooms_done = self.room_streams[s].head_piece == NONE;
                rooms_done && self.group_ready(group, epoch)
            }
            SlotKind::Free | SlotKind::OpenEmpty | SlotKind::OpenShort { .. } | SlotKind::OpenShortDispatched { .. } | SlotKind::OpenRooms { .. } => {
                unreachable!()
            }
        };
        if !ready {
            return None;
        }

        self.end_queue.pop_front();
        let tag = self.slots[s].tag.take().expect("ended slot holds its tag");
        let kind = core::mem::replace(&mut self.slots[s].kind, SlotKind::Free);
        let event = match kind {
            SlotKind::WaitAborted { .. } => Event::Aborted { tag, owner: None },
            SlotKind::WaitSingle { has_owner, .. } | SlotKind::WaitMulti { has_owner } => {
                // Sound: the slot's work is done (`ready`), so its hash has no
                // writer; `has_owner` says the slot holds an owner, read out
                // here once (the slot is then Free).
                let hash = Hash::from_bytes(unsafe { *self.shared().hashes[s].get() });
                let owner = if has_owner { Some(unsafe { (*self.owners[s].get()).assume_init_read() }) } else { None };
                Event::Hashed { tag, hash, owner }
            }
            SlotKind::WaitRoomMulti => {
                // Sound: as above.
                let hash = Hash::from_bytes(unsafe { *self.shared().hashes[s].get() });
                Event::Hashed { tag, hash, owner: None }
            }
            SlotKind::WaitBatch { .. } => {
                // Sound: a WaitBatch slot holds its batch and digests (batch
                // wrote them), read out here once.
                let (batch, digests) = unsafe { (*self.batches[s].get()).assume_init_read() };
                Event::Batch { tag, batch, digests }
            }
            _ => unreachable!(),
        };
        self.free_slots.push_back(s as u32);
        return Some(event);
    }

    fn reap(&mut self) {
        while self.events.len() < self.limits.messages {
            let Some(ev) = self.try_reap_one() else { break };
            self.events.push_back(ev);
        }
    }

    fn has_in_flight_work(&self) -> bool {
        let shared = self.shared();
        for g in 0..self.group_count {
            let assigned = self.assigned_group_epoch[g];
            if assigned != 0 && shared.storage.groups[g].completed_epoch.load(Ordering::Acquire) < assigned {
                return true;
            }
        }
        for p in &shared.pieces {
            if !p.done.load(Ordering::Acquire) {
                return true;
            }
        }
        for s in 0..self.slots.len() {
            match self.slots[s].kind {
                SlotKind::WaitMulti { .. } | SlotKind::WaitBatch { group: NONE, .. } => {
                    if !shared.slot_done[s].load(Ordering::Acquire) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }
}

impl<T, B> Drop for Pipeline<T, B> {
    fn drop(&mut self) {
        if self.has_in_flight_work() {
            stop_process("blake3-servil: Pipeline dropped while worker tasks are still in flight");
        }
        // Release any credits held on uncommitted short rooms sitting in un-ended slots
        // (e.g. during panic unwind after a precondition check failed with zero workers running).
        for s in 0..self.slots.len() {
            if let SlotKind::OpenShort { room: room_idx, .. } = self.slots[s].kind {
                self.shared().storage.rooms.release_n(room_idx, 1);
            }
        }
        if self.open_group.take().is_some() {
            for &r in &self.local_rooms[..self.local_count] {
                if r != NONE {
                    self.shared().storage.rooms.release_n(r, 1);
                }
            }
        }
        if let Some(slot) = self.source_slot {
            Source::unregister(slot);
        }
        crate::lanes::sources::closed();
        let storage = self.shared().storage;
        storage.waker.clear();
        // Sound: the Pipeline still holds the storage; these cells are its
        // holder's.
        unsafe {
            *storage.saved_loan_epoch.get() = self.next_loan_epoch;
            *storage.saved_group_epoch.get() = self.next_group_epoch;
        }

        while let Some(s_idx) = self.end_queue.pop_front() {
            let s = s_idx as usize;
            match core::mem::replace(&mut self.slots[s].kind, SlotKind::Free) {
                SlotKind::WaitSingle { has_owner: true, .. } | SlotKind::WaitMulti { has_owner: true } => {
                    // Sound: a slot waiting with `has_owner` holds an owner no
                    // event took.
                    unsafe { (*self.owners[s].get()).assume_init_drop() };
                }
                SlotKind::WaitBatch { .. } => {
                    // Sound: a WaitBatch slot holds its batch and digests.
                    unsafe { (*self.batches[s].get()).assume_init_drop() };
                }
                _ => {}
            }
        }
        PipeStorage::release(storage);
    }
}

struct MessageFut<'a, T, B> {
    pipe: &'a mut Pipeline<T, B>,
    tag: Option<T>,
}

impl<T, B> Unpin for MessageFut<'_, T, B> {}

impl<T, B> Future for MessageFut<'_, T, B> {
    type Output = Message;

    #[inline(always)]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Message> {
        let this = &mut *self;
        let slot = match this.pipe.free_slots.pop_front() {
            Some(s) => s,
            None => match this.pipe.poll_message(cx) {
                Poll::Ready(s) => s,
                Poll::Pending => return Poll::Pending,
            },
        };
        let sm = &mut this.pipe.slots[slot as usize];
        sm.tag = this.tag.take();
        sm.kind = SlotKind::OpenEmpty;
        this.pipe.open_messages += 1;
        Poll::Ready(Message { pipeline_id: this.pipe.id, slot })
    }
}

struct RoomFut<'a, T, B> {
    pipe: &'a mut Pipeline<T, B>,
}

impl<T, B> Unpin for RoomFut<'_, T, B> {}

impl<T, B> Future for RoomFut<'_, T, B> {
    type Output = Room;

    #[inline(always)]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Room> {
        if let Some(r) = self.pipe.pop_cached_room() {
            return Poll::Ready(r);
        }
        self.pipe.poll_room(cx)
    }
}

struct ReadyFut<'a, T, B> {
    pipe: &'a mut Pipeline<T, B>,
}

impl<T, B> Unpin for ReadyFut<'_, T, B> {}

impl<T, B> Future for ReadyFut<'_, T, B> {
    type Output = ();

    #[inline(always)]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if !self.pipe.events.is_empty() {
            return Poll::Ready(());
        }
        self.pipe.poll_ready(cx)
    }
}

struct ThreadPark(thread::Thread);

impl std::task::Wake for ThreadPark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

crate::pipeline_sync::thread_local! {
    static BLOCK_ON_WAKER: Waker = Waker::from(Arc::new(ThreadPark(thread::current())));
    static BLOCK_ON_STATE: core::cell::Cell<u8> = const { core::cell::Cell::new(0) };
}

#[inline(always)]
fn in_block_on() -> bool {
    BLOCK_ON_STATE.with(core::cell::Cell::get) != 0
}

#[inline(always)]
fn block_on_spinning() -> bool {
    BLOCK_ON_STATE.with(core::cell::Cell::get) == 1
}

fn prepare_thread() {
    BLOCK_ON_WAKER.with(|_| {});
}

/// Drive `future` to completion on the current thread, parking between polls.
///
/// # Examples
///
/// Two messages open at once, fed in turns, with `Pipeline` called again while
/// handling a completed event:
/// ```
/// # blake3_servil::initialize_multithreaded(None);
/// use blake3_servil::{Mode, convenience, throughput::{Event, Limits, Pipeline, block_on}};
///
/// let mut pipeline = Pipeline::new(Mode::Plain, Limits { messages: 4, rooms: 4 });
/// block_on(async {
///     let mut a = pipeline.message("A", None).await;
///     let mut b = pipeline.message("B", None).await;
///     let mut room = pipeline.room().await;
///     room.bytes_mut()[..3].copy_from_slice(b"foo");
///     pipeline.commit(&mut a, &mut room, 3);
///     room.bytes_mut()[..4].copy_from_slice(b"quux");
///     pipeline.commit(&mut b, &mut room, 4);
///     drop(room);
///     pipeline.end(a);
///     pipeline.end(b);
///     pipeline.ready().await;
///     let mut seen = Vec::new();
///     while seen.len() < 2 {
///         if let Some(Event::Hashed { tag, hash, .. }) = pipeline.next_event() {
///             seen.push((tag, hash));
///             if tag == "A" {
///                 let follow = pipeline.message("C", None).await;
///                 pipeline.end(follow);
///             }
///         } else {
///             pipeline.ready().await;
///         }
///     }
///     assert_eq!(seen[0], ("A", convenience::hash(b"foo")));
///     assert_eq!(seen[1], ("B", convenience::hash(b"quux")));
///     while pipeline.next_event().is_none() {
///         pipeline.ready().await;
///     }
/// });
/// ```
///
/// A task holding a `Pipeline`, `Message`, and `Room` across `.await` is `Send`
/// and `'static` (what `tokio::spawn` requires):
/// ```
/// # blake3_servil::initialize_multithreaded(None);
/// use blake3_servil::{Mode, throughput::{Limits, Pipeline, block_on}};
/// fn assert_send_static<F: core::future::Future + Send + 'static>(f: F) -> F { f }
/// async fn read(into: &mut [u8]) -> usize { into[..5].copy_from_slice(b"hello"); 5 }
///
/// let task = assert_send_static(async {
///     let mut pipeline = Pipeline::<u64, Vec<u8>>::with_owners(Mode::Plain, Limits { messages: 2, rooms: 4 });
///     let mut m = pipeline.message(7, None).await;
///     let mut room = pipeline.room().await;
///     let n = read(room.bytes_mut()).await;
///     pipeline.commit(&mut m, &mut room, n);
///     drop(room);
///     pipeline.end(m);
///     let whole = pipeline.message(8, None).await;
///     pipeline.owned(whole, vec![1, 2, 3]);
///     let mut got = 0;
///     while got < 2 {
///         if pipeline.next_event().is_some() { got += 1; } else { pipeline.ready().await; }
///     }
/// });
/// block_on(task);
/// ```
///
/// A message used after `end` is refused by the compiler:
/// ```compile_fail,E0382
/// use blake3_servil::throughput::{Pipeline, Room};
/// async fn bad(pipeline: &mut Pipeline<u64>, room: &mut Room) {
///     let mut m = pipeline.message(1, None).await;
///     pipeline.end(m);
///     pipeline.commit(&mut m, room, 1);
/// }
/// ```
///
/// A lent buffer cannot be freed while its [`Lent`] guard lives:
/// ```compile_fail,E0505
/// use blake3_servil::throughput::Pipeline;
/// async fn bad(pipeline: &mut Pipeline<u64>) {
///     let buffer = vec![0u8; 1024];
///     let m = pipeline.message(1, None).await;
///     let guard = unsafe { pipeline.lent(m, &buffer) };
///     drop(buffer);
///     pipeline.ready().await;
///     drop(guard);
/// }
/// ```
///
/// nor mutated while its [`Lent`] guard lives:
/// ```compile_fail,E0502
/// use blake3_servil::throughput::Pipeline;
/// async fn bad(pipeline: &mut Pipeline<u64>) {
///     let mut buffer = vec![0u8; 1024];
///     let m = pipeline.message(1, None).await;
///     let guard = unsafe { pipeline.lent(m, &buffer) };
///     buffer[0] = 1;
///     pipeline.ready().await;
///     drop(guard);
/// }
/// ```
///
/// Held until the event exists, `lent` compiles and drops cleanly before the buffer:
/// ```
/// # blake3_servil::initialize_multithreaded(None);
/// use blake3_servil::{Mode, convenience, throughput::{Event, Limits, Pipeline, block_on}};
/// let mut pipeline = Pipeline::new(Mode::Plain, Limits { messages: 2, rooms: 0 });
/// block_on(async {
///     let buffer = vec![9u8; 1 << 20];
///     let m = pipeline.message(1u64, None).await;
///     let _guard = unsafe { pipeline.lent(m, &buffer) };
///     pipeline.ready().await;
/// });
/// match pipeline.next_event() {
///     Some(Event::Hashed { hash, .. }) => assert_eq!(hash, convenience::hash(&vec![9u8; 1 << 20])),
///     _ => panic!("expected Hashed"),
/// }
/// ```
///
/// `Pipeline::new` (without owner type) exposes no `owned` method:
/// ```compile_fail,E0599
/// use blake3_servil::throughput::Pipeline;
/// async fn bad(pipeline: &mut Pipeline<u64>) {
///     let m = pipeline.message(1, None).await;
///     pipeline.owned(m, ());
/// }
/// ```
///
/// A batch over borrowed blocks is refused (`Batch<'static>` required):
/// ```compile_fail,E0521
/// use blake3_servil::throughput::{Batch, Block, Pipeline};
/// async fn bad(pipeline: &mut Pipeline<u64>, blocks: &[Block]) {
///     let batch = Batch::from_blocks(blocks, 64);
///     let m = pipeline.message(1, None).await;
///     pipeline.batch(m, batch, Box::new([]));
/// }
/// ```
///
/// Two concurrent waits on one pipeline are refused (`&mut Pipeline`):
/// ```compile_fail,E0499
/// use blake3_servil::throughput::Pipeline;
/// async fn bad(pipeline: &mut Pipeline<u64>) {
///     let slot = pipeline.message(1, None);
///     let ev = pipeline.ready();
///     let _ = (slot.await, ev.await);
/// }
/// ```
pub fn block_on<F: Future>(future: F) -> F::Output {
    struct Guard(u8);
    impl Drop for Guard {
        fn drop(&mut self) {
            BLOCK_ON_STATE.with(|f| f.set(self.0));
        }
    }
    let prev = BLOCK_ON_STATE.with(|f| f.replace(1));
    let _guard = Guard(prev);
    let mut future = core::pin::pin!(future);
    BLOCK_ON_WAKER.with(|waker| {
        let mut cx = Context::from_waker(waker);
        if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
            return output;
        }
        let started = crate::lanes::now();
        let mut yielded = started;
        loop {
            crate::lanes::poll_pause(&mut yielded);
            if crate::lanes::now() - started >= crate::lanes::SPIN_BEFORE_SLEEP {
                BLOCK_ON_STATE.with(|f| f.set(2));
                match future.as_mut().poll(&mut cx) {
                    Poll::Ready(output) => return output,
                    Poll::Pending => thread::park(),
                }
            } else if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
                return output;
            }
        }
    })
}

// The standard library's tests alone: under Shuttle
// (tests/shuttle_pipeline.rs) this file is compiled without them.
#[cfg(all(test, not(shuttle)))]
mod test {
    use super::*;

    #[test]
    fn aaa_unit_single_step_every_stage() {
        crate::initialize_multithreaded(None);
        let step_until = |source: &Source, cond: &dyn Fn() -> bool, what: &str| {
            for i in 0..200_000usize {
                let ran = source.run_waiting_action();
                if cond() {
                    return;
                }
                if !ran && i % 64 == 63 {
                    std::thread::yield_now();
                } else {
                    core::hint::spin_loop();
                }
            }
            panic!("bounded step failed: {what}");
        };

        assert!(!in_block_on() && !block_on_spinning());
        block_on(core::future::poll_fn(|_| {
            assert!(in_block_on() && block_on_spinning());
            Poll::Ready(())
        }));
        let _ = std::panic::catch_unwind(|| {
            block_on(async {
                assert!(in_block_on());
                panic!("unwind guard test");
            });
        });
        assert!(!in_block_on() && !block_on_spinning());

        let p_cap_check = Pipeline::<usize>::new(Mode::Plain, Limits { messages: 255, rooms: 10 });
        assert_eq!(p_cap_check.shared().storage.source.actions_len(), 16841);
        drop(p_cap_check);

        let mut p = Pipeline::<usize>::new(Mode::Plain, Limits { messages: 4, rooms: 2 });
        let Poll::Ready(r_first) = poll_once(p.room()) else { panic!("room 0 ready") };
        let Poll::Ready(mut r) = poll_once(p.room()) else { panic!("room 1 ready") };
        assert_eq!(p.room_cursor, 0, "room_cursor wraps modulo rooms");
        drop(r_first);
        let Poll::Ready(mut m) = poll_once(p.message(1, None)) else { panic!("msg ready") };
        r.bytes_mut()[..ROOM_LEN].fill(0x33);
        p.commit(&mut m, &mut r, ROOM_LEN);
        drop(r);
        let pnode = p.room_streams[0].head_piece as usize;
        let src = &p.shared().storage.source;
        let pieces_ref = &p.shared().pieces;
        step_until(src, &|| pieces_ref[pnode].done.load(Ordering::Acquire), "dispatch_one + run_room_piece");
        p.end(m);
        assert_eq!(p.room_streams[0].head_piece, NONE, "end drained room piece");
        p.reap();
        assert_eq!(p.events.len(), 1, "reap moved completed event");
        match p.next_event() {
            Some(Event::Hashed { tag: 1, hash, owner: None }) => assert_eq!(hash, convenience::hash(&vec![0x33; ROOM_LEN])),
            _ => panic!("unexpected event"),
        }

        let mut p_short = Pipeline::<usize, Vec<u8>>::with_owners(Mode::Plain, Limits { messages: 4, rooms: 0 });
        for round in 0..10usize {
            let Poll::Ready(ms) = poll_once(p_short.message(round, None)) else { panic!("short msg ready") };
            p_short.owned(ms, vec![round as u8; 128]);
            p_short.publish_open();
            let g = round % 4;
            let ep = (round + 1) as u64;
            let src = &p_short.shared().storage.source;
            let groups = &p_short.shared().storage.groups;
            step_until(src, &|| groups[g].completed_epoch.load(Ordering::Acquire) >= ep, "publish_open + run_group");
            assert!(p_short.group_ready(g as u32, ep));
            let Some(Event::Hashed { tag, hash, owner: Some(_) }) = p_short.try_reap_one() else {
                panic!("try_reap_one ready");
            };
            assert_eq!(tag, round);
            assert_eq!(hash, convenience::hash(&vec![round as u8; 128]));
        }

        let mut p_multi = Pipeline::<usize, Vec<u8>>::with_owners(Mode::Plain, Limits { messages: 2, rooms: 0 });
        let Poll::Ready(mm) = poll_once(p_multi.message(99, None)) else { panic!("multi msg ready") };
        p_multi.owned(mm, vec![0x44; MAX_SLOT_PIECES * TASK_LEN]);
        assert_eq!(unsafe { (*p_multi.shared().multi[0].get()).piece_len }, TASK_LEN);
        let src = &p_multi.shared().storage.source;
        let sd = &p_multi.shared().slot_done;
        step_until(src, &|| sd[0].load(Ordering::Acquire), "dispatch_many + run_multi_piece");
        assert!(p_multi.try_reap_one().is_some(), "run_multi_piece completed all pieces");
        drop(p_multi);

        let mut p_meta = Pipeline::<usize>::new(Mode::Plain, Limits { messages: 4, rooms: 1 });
        let Poll::Ready(ma) = poll_once(p_meta.message(1, None)) else { panic!() };
        assert_eq!(p_meta.open_messages, 1);
        p_meta.abort(ma);
        assert_eq!(p_meta.open_messages, 0);
        let _ = p_meta.next_event();

        let Poll::Ready(mb) = poll_once(p_meta.message(2, None)) else { panic!() };
        assert_eq!(p_meta.open_messages, 1);
        p_meta.batch(mb, Batch::new(1024, 128), vec![[0u8; 32]; 128].into_boxed_slice());
        assert_eq!(p_meta.open_messages, 0);
        assert!(matches!(p_meta.slots[1].kind, SlotKind::WaitBatch { group: NONE, .. }));
        assert_eq!(unsafe { (*p_meta.shared().multi[1].get()).batch.unwrap().2 }, 64);
        let src = &p_meta.shared().storage.source;
        let sd = &p_meta.shared().slot_done;
        step_until(src, &|| sd[1].load(Ordering::Acquire), "batch 128 multi");
        let _ = p_meta.try_reap_one();

        let Poll::Ready(mb64) = poll_once(p_meta.message(3, None)) else { panic!() };
        p_meta.batch(mb64, Batch::new(1024, 64), vec![[0u8; 32]; 64].into_boxed_slice());
        assert!(matches!(p_meta.slots[2].kind, SlotKind::WaitBatch { group: NONE, .. }));
        let src = &p_meta.shared().storage.source;
        let sd = &p_meta.shared().slot_done;
        step_until(src, &|| sd[2].load(Ordering::Acquire), "batch 64 multi");
        let _ = p_meta.try_reap_one();
        drop(p_meta);

        {
            let mut p_wb = Pipeline::<usize>::new(Mode::Plain, Limits { messages: 1, rooms: 0 });
            let Poll::Ready(m0) = poll_once(p_wb.message(0, None)) else { panic!() };
            p_wb.end(m0);
            p_wb.reap();
            let Poll::Ready(m1) = poll_once(p_wb.message(1, None)) else { panic!() };
            p_wb.batch(m1, Batch::new(64, 1), vec![[0u8; 32]; 1].into_boxed_slice());
            p_wb.publish_open();
            let src = &p_wb.shared().storage.source;
            let groups = &p_wb.shared().storage.groups;
            step_until(src, &|| groups[0].completed_epoch.load(Ordering::Acquire) >= 1, "WaitBatch drop setup");
        }
        {
            let mut p_forget = Pipeline::<usize>::new(Mode::Plain, Limits { messages: 2, rooms: 1 });
            let Poll::Ready(mut m0) = poll_once(p_forget.message(0, None)) else { panic!() };
            let Poll::Ready(mut r0) = poll_once(p_forget.room()) else { panic!() };
            p_forget.commit(&mut m0, &mut r0, 64);
            drop(r0);
            core::mem::forget(m0);
        }
    }

    use crate::convenience;

    fn drain_all<T, B>(pipeline: &mut Pipeline<T, B>, want: usize) -> Vec<Event<T, B>> {
        let mut out = Vec::with_capacity(want);
        while out.len() < want {
            if let Some(ev) = pipeline.next_event() {
                out.push(ev);
            } else {
                block_on(pipeline.ready());
            }
        }
        out
    }

    #[test]
    fn rooms_owned_lent_and_modes_match_reference_across_boundaries() {
        crate::initialize_multithreaded(None);
        let max_len = (5 << 20) + 123;
        let mut input = vec![0u8; max_len];
        crate::test::paint_test_input(&mut input);
        let key = [0x5Au8; 32];
        let ctx_key = crate::throughput::context_key("pipeline modes");
        let lens = [
            0,
            1,
            63,
            64,
            65,
            1023,
            1024,
            1025,
            16 * 1024,
            ROOM_LEN - 1,
            ROOM_LEN,
            ROOM_LEN + 1,
            2 * ROOM_LEN,
            3 * ROOM_LEN + 77,
            200_000,
            1 << 20,
            (4 << 20) + 7,
            max_len,
        ];

        // Identical Limits across all 4 modes also exercises PipeStorage recycling across Modes!
        for mode in [Mode::Plain, Mode::Keyed(&key), Mode::DeriveKey("pipeline derive"), Mode::DeriveKeyWith(&ctx_key)] {
            let mut pipe = Pipeline::new(mode, Limits { messages: 4, rooms: 3 });
            for (idx, &len) in lens.iter().enumerate() {
                block_on(async {
                    let mut m = pipe.message(idx, Some(len as u64)).await;
                    let mut pos = 0;
                    if len == 0 && idx % 2 == 1 {
                        let mut r = pipe.room().await;
                        pipe.commit(&mut m, &mut r, 0);
                    }
                    while pos < len {
                        let mut r = pipe.room().await;
                        let n = (len - pos).min(ROOM_LEN);
                        r.bytes_mut()[..n].copy_from_slice(&input[pos..pos + n]);
                        pipe.commit(&mut m, &mut r, n);
                        pos += n;
                    }
                    pipe.end(m);
                });
                if idx % 3 == 2 {
                    for ev in drain_all(&mut pipe, 3) {
                        match ev {
                            Event::Hashed { tag, hash, owner: None } => {
                                assert_eq!(hash, convenience::hash_with(mode, &input[..lens[tag]]), "room len={}", lens[tag]);
                            }
                            _ => panic!("unexpected event"),
                        }
                    }
                }
            }
            let rem = lens.len() % 3;
            for ev in drain_all(&mut pipe, rem) {
                if let Event::Hashed { tag, hash, .. } = ev {
                    assert_eq!(hash, convenience::hash_with(mode, &input[..lens[tag]]));
                } else {
                    panic!("unexpected event");
                }
            }
            drop(pipe);

            let mut owned_pipe = Pipeline::<usize, Vec<u8>>::with_owners(mode, Limits { messages: 6, rooms: 0 });
            for (idx, &len) in lens.iter().enumerate() {
                while owned_pipe.events.len() == 6 || owned_pipe.free_slots.is_empty() {
                    if let Some(Event::Hashed { tag, hash, owner: Some(buf) }) = owned_pipe.next_event() {
                        assert_eq!(buf.len(), lens[tag]);
                        assert_eq!(hash, convenience::hash_with(mode, &input[..lens[tag]]));
                    } else {
                        block_on(owned_pipe.ready());
                    }
                }
                let m = block_on(owned_pipe.message(idx, None));
                owned_pipe.owned(m, input[..len].to_vec());
            }
            while owned_pipe.end_queue.len() + owned_pipe.events.len() > 0 {
                if let Some(Event::Hashed { tag, hash, owner: Some(buf) }) = owned_pipe.next_event() {
                    assert_eq!(buf.len(), lens[tag]);
                    assert_eq!(hash, convenience::hash_with(mode, &input[..lens[tag]]));
                } else {
                    block_on(owned_pipe.ready());
                }
            }
            drop(owned_pipe);

            let mut lent_pipe = Pipeline::new(mode, Limits { messages: 2, rooms: 0 });
            for (idx, &len) in lens.iter().enumerate() {
                let slice = &input[..len];
                let m = block_on(lent_pipe.message(idx, None));
                let guard = unsafe { lent_pipe.lent(m, slice) };
                block_on(lent_pipe.ready());
                drop(guard);
                match lent_pipe.next_event() {
                    Some(Event::Hashed { tag, hash, owner: None }) => {
                        assert_eq!(tag, idx);
                        assert_eq!(hash, convenience::hash_with(mode, slice), "lent len={len}");
                    }
                    _ => panic!("expected Hashed"),
                }
            }
        }
    }

    #[test]
    fn inline_array_owner_never_aliases_or_corrupts() {
        let mut pipe = Pipeline::<usize, [u8; 100]>::with_owners(Mode::Plain, Limits { messages: 8, rooms: 0 });
        for i in 0..32usize {
            while pipe.free_slots.is_empty() {
                if let Some(Event::Hashed { tag, hash, owner: Some(arr) }) = pipe.next_event() {
                    assert_eq!(arr, [tag as u8; 100]);
                    assert_eq!(hash, convenience::hash(&[tag as u8; 100]));
                } else {
                    block_on(pipe.ready());
                }
            }
            let m = block_on(pipe.message(i, None));
            pipe.owned(m, [i as u8; 100]);
        }
        while pipe.end_queue.len() + pipe.events.len() > 0 {
            if let Some(Event::Hashed { tag, hash, owner: Some(arr) }) = pipe.next_event() {
                assert_eq!(arr, [tag as u8; 100]);
                assert_eq!(hash, convenience::hash(&[tag as u8; 100]));
            } else {
                block_on(pipe.ready());
            }
        }
    }

    #[test]
    fn shared_room_many_short_messages_and_fifo_room_cycling() {
        let mut pipe = Pipeline::new(Mode::Plain, Limits { messages: 8, rooms: 3 });
        let mut room = block_on(pipe.room());
        assert_eq!(room.len(), ROOM_LEN);
        assert_eq!(room.as_mut_ptr() as usize % 64, 0);

        let count = 1500usize;
        let mut pulled = Vec::with_capacity(count);
        for i in 0..count {
            let msg_len = (i * 13) % 200;
            if room.len() < msg_len {
                drop(room);
                room = block_on(pipe.room());
            }
            while pipe.free_slots.is_empty() {
                if let Some(ev) = pipe.next_event() {
                    pulled.push(ev);
                } else {
                    block_on(pipe.ready());
                }
            }
            let mut m = block_on(pipe.message((i, msg_len), None));
            let pat = (i as u8).wrapping_mul(7);
            room.bytes_mut()[..msg_len].fill(pat);
            pipe.commit(&mut m, &mut room, msg_len);
            assert_eq!(room.as_mut_ptr() as usize % 64, 0, "room stays 64-byte aligned");
            pipe.end(m);
        }
        drop(room);
        while pulled.len() < count {
            if let Some(ev) = pipe.next_event() {
                pulled.push(ev);
            } else {
                block_on(pipe.ready());
            }
        }
        for (idx, ev) in pulled.into_iter().enumerate() {
            match ev {
                Event::Hashed { tag: (i, len), hash, owner: None } => {
                    assert_eq!(i, idx);
                    let pat = (i as u8).wrapping_mul(7);
                    assert_eq!(hash, convenience::hash(&vec![pat; len]));
                }
                _ => panic!("unexpected event"),
            }
        }

        // FIFO ring cycling check: rooms cycle 0 -> 1 -> 2 -> 0 -> 1 -> 2, cooling cache lines before reuse.
        let mut seen = Vec::new();
        for _ in 0..6 {
            let r = block_on(pipe.room());
            seen.push(r.index);
            drop(r);
        }
        for w in seen.windows(3) {
            assert!(w[0] != w[1] && w[1] != w[2] && w[0] != w[2], "rooms cycle around all 3 before reuse: {seen:?}");
        }
    }

    #[test]
    fn interleaved_messages_deliver_in_end_order_and_handle_aborts() {
        let mut pipe = Pipeline::new(Mode::Plain, Limits { messages: 4, rooms: 4 });
        let data_a = vec![0xAAu8; 3 * ROOM_LEN + 500];
        let data_b = vec![0xBBu8; 900];
        let data_c = vec![0xCCu8; 2 * ROOM_LEN + 100];

        block_on(async {
            let mut a = pipe.message("A", None).await;
            let mut b = pipe.message("B", None).await;
            let mut c = pipe.message("C", None).await;

            let mut r = pipe.room().await;
            r.bytes_mut().copy_from_slice(&data_a[..ROOM_LEN]);
            pipe.commit(&mut a, &mut r, ROOM_LEN);
            drop(r);

            let mut r = pipe.room().await;
            r.bytes_mut().copy_from_slice(&data_c[..ROOM_LEN]);
            pipe.commit(&mut c, &mut r, ROOM_LEN);
            drop(r);

            let mut r = pipe.room().await;
            r.bytes_mut()[..data_b.len()].copy_from_slice(&data_b);
            pipe.commit(&mut b, &mut r, data_b.len());
            drop(r);

            pipe.end(b);
            pipe.abort(c);

            for chunk in data_a[ROOM_LEN..].chunks(ROOM_LEN) {
                let mut r = pipe.room().await;
                r.bytes_mut()[..chunk.len()].copy_from_slice(chunk);
                pipe.commit(&mut a, &mut r, chunk.len());
            }
            pipe.end(a);
        });

        let events = drain_all(&mut pipe, 3);
        match &events[0] {
            Event::Hashed { tag: "B", hash, .. } => assert_eq!(*hash, convenience::hash(&data_b)),
            _ => panic!("expected B first"),
        }
        match &events[1] {
            Event::Aborted { tag: "C", owner: None } => {}
            _ => panic!("expected C aborted second"),
        }
        match &events[2] {
            Event::Hashed { tag: "A", hash, .. } => assert_eq!(*hash, convenience::hash(&data_a)),
            _ => panic!("expected A third"),
        }
    }

    #[test]
    fn batches_empty_small_and_large_match_hash_many() {
        let mut pipe = Pipeline::new(Mode::Plain, Limits { messages: 8, rooms: 0 });
        for (tag, (msg_len, count)) in [(64, 0), (0, 5), (64, 16), (100, 33), (64, 2048), (300, 600)].into_iter().enumerate() {
            let mut batch = Batch::new(msg_len, count);
            for i in 0..count {
                let msg = vec![(i as u8).wrapping_add(tag as u8); msg_len];
                batch.set(i, &msg);
            }
            let digests = vec![[0u8; 32]; count].into_boxed_slice();
            let m = block_on(pipe.message((tag, msg_len, count), None));
            pipe.batch(m, batch, digests);
        }

        for ev in drain_all(&mut pipe, 6) {
            match ev {
                Event::Batch { tag: (tag, msg_len, count), batch, digests } => {
                    assert_eq!(batch.len(), count);
                    assert_eq!(digests.len(), count);
                    for i in 0..count {
                        let want = convenience::hash(&vec![(i as u8).wrapping_add(tag as u8); msg_len]);
                        assert_eq!(digests[i], *want.as_bytes());
                    }
                }
                _ => panic!("expected Batch"),
            }
        }
    }

    /// What a worker can reach is never given back: a Pipeline's `Shared`
    /// stays with its storage when the Pipeline is dropped, and the next
    /// Pipeline of that size takes both and hashes rightly in its own mode
    /// (a worker that read it late once read freed memory, #399). A
    /// larger Pipeline on the same storage gets one of its own size.
    #[test]
    fn a_pipelines_shared_state_outlives_it_and_serves_the_next() {
        crate::initialize_multithreaded(None);
        let limits = Limits { messages: 3, rooms: 2 };
        let first = Pipeline::<usize, Vec<u8>>::with_owners(Mode::Plain, limits);
        let (shared, storage) = (first.shared, first.shared().storage as *const PipeStorage);
        drop(first);
        assert_eq!(unsafe { *(*storage).shared.get() }, shared, "the storage keeps it");
        let key = [9u8; 32];
        let mut second = Pipeline::<usize, Vec<u8>>::with_owners(Mode::Keyed(&key), limits);
        assert_eq!((second.shared, second.shared().storage as *const PipeStorage), (shared, storage), "the next Pipeline of this size takes both");
        let m = block_on(second.message(0, None));
        second.owned(m, vec![5u8; 1 << 20]);
        match drain_all(&mut second, 1).pop().unwrap() {
            Event::Hashed { hash, .. } => assert_eq!(hash, convenience::hash_with(Mode::Keyed(&key), &vec![5u8; 1 << 20]), "in its own mode"),
            _ => panic!("expected Hashed"),
        }
        drop(second);
        // A larger Pipeline that the storage still fits: a Shared of its own
        // size, which holds every one of its messages.
        let mut larger = Pipeline::<usize, Vec<u8>>::with_owners(Mode::Plain, Limits { messages: 10, rooms: 2 });
        assert_eq!(larger.shared().storage as *const PipeStorage, storage, "the storage serves ten messages too");
        assert_ne!(larger.shared, shared, "the Shared of three messages does not");
        for tag in 0..10 {
            let m = block_on(larger.message(tag, None));
            larger.owned(m, vec![tag as u8; 300_000]);
        }
        for event in drain_all(&mut larger, 10) {
            match event {
                Event::Hashed { tag, hash, .. } => assert_eq!(hash, convenience::hash(&vec![tag as u8; 300_000])),
                _ => panic!("expected Hashed"),
            }
        }
    }

    #[test]
    fn event_backpressure_holds_excess_completions_in_slots_without_loss() {
        let mut pipe = Pipeline::<usize, Vec<u8>>::with_owners(Mode::Plain, Limits { messages: 2, rooms: 0 });
        for i in 0..2 {
            let m = block_on(pipe.message(i, None));
            pipe.owned(m, vec![i as u8; 128]);
        }
        block_on(pipe.ready());
        while pipe.events.len() < 2 {
            if !pipe.shared().storage.source.run_waiting_action() {
                pipe.shared().storage.source.wake_for_waiting();
                std::thread::yield_now();
            }
            pipe.reap();
        }
        for i in 2..4 {
            let m = block_on(pipe.message(i, None));
            pipe.owned(m, vec![i as u8; 128]);
        }
        pipe.publish_open();
        while pipe.has_in_flight_work() {
            if !pipe.shared().storage.source.run_waiting_action() {
                pipe.shared().storage.source.wake_for_waiting();
                std::thread::yield_now();
            }
        }
        pipe.reap();
        assert_eq!(pipe.events.len(), 2, "events capped at limits.messages");
        assert_eq!(pipe.end_queue.len(), 2, "wave 2 holds its slots until pulled");

        let all = drain_all(&mut pipe, 4);
        for (i, ev) in all.into_iter().enumerate() {
            match ev {
                Event::Hashed { tag, hash, owner: Some(v) } => {
                    assert_eq!(tag, i);
                    assert_eq!(hash, convenience::hash(&v));
                }
                _ => panic!("expected Hashed"),
            }
        }
    }

    #[test]
    fn guard_pages_catch_any_underread_or_overread_on_owned_and_lent() {
        struct GuardedBytes {
            _g: crate::test::guard::Guarded,
            ptr: *const u8,
            len: usize,
        }
        impl GuardedBytes {
            fn new(len: usize, at_end: bool) -> Self {
                let mut g = crate::test::guard::filled(len, at_end);
                let ptr = g.bytes().as_ptr();
                GuardedBytes { _g: g, ptr, len }
            }
        }
        impl AsRef<[u8]> for GuardedBytes {
            fn as_ref(&self) -> &[u8] {
                unsafe { core::slice::from_raw_parts(self.ptr, self.len) }
            }
        }

        let mut pipe = Pipeline::<usize, GuardedBytes>::with_owners(Mode::Plain, Limits { messages: 2, rooms: 0 });
        for at_end in [false, true] {
            for len in [0, 1, 63, 64, 65, 1024, 16 * 1024, ROOM_LEN, ROOM_LEN + 65] {
                let gb = GuardedBytes::new(len, at_end);
                let want = convenience::hash(gb.as_ref());

                let m = block_on(pipe.message(len, None));
                let guard = unsafe { pipe.lent(m, gb.as_ref()) };
                block_on(pipe.ready());
                drop(guard);
                match pipe.next_event() {
                    Some(Event::Hashed { hash, owner: None, .. }) => assert_eq!(hash, want, "lent len={len} at_end={at_end}"),
                    _ => panic!("expected Hashed"),
                }

                let m = block_on(pipe.message(len, None));
                pipe.owned(m, gb);
                block_on(pipe.ready());
                match pipe.next_event() {
                    Some(Event::Hashed { hash, owner: Some(back), .. }) => {
                        assert_eq!(back.as_ref().len(), len);
                        assert_eq!(hash, want, "owned len={len} at_end={at_end}");
                    }
                    _ => panic!("expected Hashed"),
                }
            }

            let count = 65;
            let gb_batch = GuardedBytes::new(count * CHUNK_LEN, at_end);
            let blocks: &'static [crate::throughput::Block] =
                unsafe { core::mem::transmute(crate::throughput::Block::from_bytes(gb_batch.as_ref()).expect("guard-page batch starts aligned")) };
            let m = block_on(pipe.message(count, None));
            pipe.batch(m, Batch::in_place(blocks, CHUNK_LEN), vec![[0u8; 32]; count].into_boxed_slice());
            block_on(pipe.ready());
            match pipe.next_event() {
                Some(Event::Batch { batch, digests, .. }) => {
                    for i in 0..count {
                        assert_eq!(digests[i], *convenience::hash(&gb_batch.as_ref()[i * CHUNK_LEN..][..CHUNK_LEN]).as_bytes());
                    }
                    drop(batch);
                }
                _ => panic!("expected Batch"),
            }
            drop(gb_batch);
        }
    }

    #[test]
    #[should_panic(expected = "Limits.messages must be at least 1")]
    fn contract_zero_messages_panics() {
        let _ = Pipeline::<()>::new(Mode::Plain, Limits { messages: 0, rooms: 1 });
    }

    fn poll_once<F: Future>(f: F) -> Poll<F::Output> {
        let mut cx = Context::from_waker(Waker::noop());
        core::pin::pin!(f).poll(&mut cx)
    }

    #[test]
    #[should_panic(expected = "all 1 message slots are held open")]
    fn contract_message_deadlock_all_open_panics() {
        let mut pipe = Pipeline::new(Mode::Plain, Limits { messages: 1, rooms: 0 });
        let _a = block_on(pipe.message(1, None));
        let _ = poll_once(pipe.message(2, None));
    }

    #[test]
    #[should_panic(expected = "finished events wait to be pulled")]
    fn contract_message_deadlock_events_full_panics() {
        let mut pipe = Pipeline::new(Mode::Plain, Limits { messages: 1, rooms: 0 });
        let a = block_on(pipe.message(1, None));
        pipe.end(a);
        block_on(pipe.ready());
        let b = block_on(pipe.message(2, None));
        pipe.end(b);
        let _ = poll_once(pipe.message(3, None));
    }

    /// A Room that takes empty commits never gives out its last credit:
    /// while the caller holds it, no second Room is made over its bytes
    /// (BLAKE3 #418: an empty commit moves the Room on by nothing, so one
    /// Room can take them without end, and after ROOM_CREDITS of them the
    /// pool counted it free). The Room here starts a few credits from the
    /// end, where a billion commits would bring it.
    #[test]
    fn a_room_held_through_its_last_credits_is_not_handed_out_again() {
        crate::initialize_multithreaded(None);
        let mut pipe = Pipeline::<u32>::new(Mode::Plain, Limits { messages: 2, rooms: 3 });
        block_on(async {
            let mut room = pipe.room().await;
            room.spend_all_but(4);
            // At each count of empty commits around the last credit: the
            // other two Rooms, taken and given back until the pool has
            // looked again at which are free.
            for n in 0..12u32 {
                let mut m = pipe.message(n, None).await;
                pipe.commit(&mut m, &mut room, 0);
                pipe.end(m);
                loop {
                    match pipe.next_event() {
                        Some(Event::Hashed { tag, hash, .. }) => {
                            assert_eq!((tag, hash), (n, crate::convenience::hash(b"")));
                            break;
                        }
                        Some(_) => {}
                        None => pipe.ready().await,
                    }
                }
                // Its commits all finished: the pool holds what the Room
                // has yet to give out, no more and no less (with more, the
                // Room would never be free again once dropped).
                let held = room.storage.rooms.refs[room.index as usize].load(Ordering::SeqCst);
                assert_eq!(held, ROOM_CREDITS - room.commits as usize, "after {} empty commits", n + 1);
                for _ in 0..4 {
                    let (a, b) = (pipe.room().await, pipe.room().await);
                    assert!(a.index != room.index && b.index != room.index, "two Rooms over the same bytes, after {} empty commits", n + 1);
                }
            }
            assert_eq!(room.len(), ROOM_LEN, "empty commits use none of it");
            drop(room);
            // All three are free again, each one Room.
            let (a, b, c) = (pipe.room().await, pipe.room().await, pipe.room().await);
            assert!(a.index != b.index && b.index != c.index && a.index != c.index);
        });
    }

    /// Storage is reused however many Pipelines were alive at once, and
    /// whichever of a Pipeline and its Rooms is dropped last: a second
    /// wave of as many Pipelines makes no new storage. (BLAKE3 #427: the
    /// free list kept 16, and a Room dropped after its Pipeline kept its
    /// storage from the list. Either way the storage was lost for good.)
    /// The Limits are ones no other test uses, so the storages counted
    /// are this test's.
    #[test]
    fn storage_is_reused_past_sixteen_pipelines_and_after_a_late_room() {
        crate::initialize_multithreaded(None);
        let limits = Limits { messages: 3, rooms: 5 };
        let wave = |late_rooms: bool| {
            let mut pipes: Vec<Pipeline<u32>> = (0..40).map(|_| Pipeline::new(Mode::Plain, limits)).collect();
            let rooms: Vec<Room> = if late_rooms { pipes.iter_mut().map(|p| block_on(p.room())).collect() } else { Vec::new() };
            drop(pipes);
            drop(rooms);
        };
        let before = STORAGES_MADE.load(Ordering::Relaxed);
        wave(false);
        assert_eq!(STORAGES_MADE.load(Ordering::Relaxed) - before, 40, "one each, the first time");
        wave(false);
        wave(true);
        wave(true);
        assert_eq!(STORAGES_MADE.load(Ordering::Relaxed) - before, 40, "and none after");
        // A storage back on the list is whole: its Rooms are all free.
        let mut pipe = Pipeline::<u32>::new(Mode::Plain, limits);
        let rooms: Vec<Room> = (0..4).map(|_| block_on(pipe.room())).collect();
        let mut seen: Vec<u32> = rooms.iter().map(|r| r.index).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 4);
    }

    #[test]
    #[should_panic(expected = "Limits.rooms == 0")]
    fn contract_room_with_zero_rooms_panics() {
        let mut pipe = Pipeline::<u32>::new(Mode::Plain, Limits { messages: 1, rooms: 0 });
        let _ = poll_once(pipe.room());
    }

    #[test]
    #[should_panic(expected = "all 1 rooms are held by the caller")]
    fn contract_room_deadlock_all_held_panics() {
        let mut pipe = Pipeline::<u32>::new(Mode::Plain, Limits { messages: 1, rooms: 1 });
        let _r1 = block_on(pipe.room());
        let _ = poll_once(pipe.room());
    }

    #[test]
    #[should_panic(expected = "no ended messages are in flight")]
    fn contract_ready_with_nothing_ended_panics() {
        let mut pipe = Pipeline::<u32>::new(Mode::Plain, Limits { messages: 2, rooms: 0 });
        block_on(async {
            let _open = pipe.message(1, None).await;
            pipe.ready().await;
        });
    }

    #[test]
    #[should_panic(expected = "every commit to a message but its last is a whole room")]
    fn contract_commit_after_partial_room_panics() {
        let mut pipe = Pipeline::new(Mode::Plain, Limits { messages: 1, rooms: 2 });
        block_on(async {
            let mut m = pipe.message(1, None).await;
            let mut r = pipe.room().await;
            pipe.commit(&mut m, &mut r, 64);
            pipe.commit(&mut m, &mut r, 64);
        });
    }

    #[test]
    #[should_panic(expected = "commit wrote 65537 bytes into a Room with 65536 left")]
    fn contract_commit_past_room_len_panics() {
        let mut pipe = Pipeline::new(Mode::Plain, Limits { messages: 1, rooms: 1 });
        block_on(async {
            let mut m = pipe.message(1, None).await;
            let mut r = pipe.room().await;
            pipe.commit(&mut m, &mut r, ROOM_LEN + 1);
        });
    }

    #[test]
    #[should_panic(expected = "bytes given two ways")]
    fn contract_bytes_given_two_ways_panics() {
        let mut pipe = Pipeline::<u32, Vec<u8>>::with_owners(Mode::Plain, Limits { messages: 1, rooms: 1 });
        block_on(async {
            let mut m = pipe.message(1, None).await;
            let mut r = pipe.room().await;
            pipe.commit(&mut m, &mut r, 10);
            pipe.owned(m, vec![1, 2, 3]);
        });
    }

    #[test]
    #[should_panic(expected = "Message belongs to a different Pipeline")]
    fn contract_foreign_message_panics() {
        let mut p1 = Pipeline::<u32>::new(Mode::Plain, Limits { messages: 1, rooms: 0 });
        let mut p2 = Pipeline::<u32>::new(Mode::Plain, Limits { messages: 1, rooms: 0 });
        let m1 = block_on(p1.message(1, None));
        p2.end(m1);
    }

    #[test]
    #[should_panic(expected = "Room belongs to a different Pipeline")]
    fn contract_foreign_room_panics() {
        let mut p1 = Pipeline::<u32>::new(Mode::Plain, Limits { messages: 1, rooms: 1 });
        let mut p2 = Pipeline::<u32>::new(Mode::Plain, Limits { messages: 1, rooms: 1 });
        let mut m1 = block_on(p1.message(1, None));
        let mut r2 = block_on(p2.room());
        p1.commit(&mut m1, &mut r2, 0);
    }

    #[test]
    #[should_panic(expected = "Message was dropped without end, abort, owned, lent, or batch")]
    fn contract_dropped_unfinished_message_panics() {
        let mut p = Pipeline::<u32>::new(Mode::Plain, Limits { messages: 1, rooms: 0 });
        let _m = block_on(p.message(1, None));
    }

    #[test]
    fn active_loan_drop_and_active_pipeline_drop_abort_the_process() {
        use std::os::unix::process::ExitStatusExt as _;
        const ENV: &str = "BLAKE3_PIPELINE_ABORT_CASE";
        if let Ok(which) = std::env::var(ENV) {
            match which.as_str() {
                "lent" => {
                    let completed = AtomicU64::new(0);
                    let static_completed: &'static AtomicU64 = Box::leak(Box::new(completed));
                    let guard = Lent { completed: static_completed, epoch: 1, _bytes: PhantomData };
                    drop(guard);
                }
                "pipeline" => {
                    let mut p = Pipeline::<u32>::new(Mode::Plain, Limits { messages: 1, rooms: 0 });
                    p.assigned_group_epoch[0] = u64::MAX;
                    drop(p);
                }
                "pipeline_multi" => {
                    let mut p = Pipeline::<u32>::new(Mode::Plain, Limits { messages: 1, rooms: 0 });
                    p.slots[0].kind = SlotKind::WaitMulti { has_owner: false };
                    p.shared().slot_done[0].store(false, Ordering::SeqCst);
                    drop(p);
                }
                "abort_on_unwind" => {
                    crate::lanes::drop_abort_on_unwind_for_test();
                }
                _ => unreachable!(),
            }
            std::process::exit(0);
        }

        for (case, needle) in [
            ("lent", "Lent guard was dropped before its message finished"),
            ("pipeline", "Pipeline dropped while worker tasks are still in flight"),
            ("pipeline_multi", "Pipeline dropped while worker tasks are still in flight"),
            ("abort_on_unwind", ""),
        ] {
            let out = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "pipeline::test::active_loan_drop_and_active_pipeline_drop_abort_the_process", "--nocapture"])
                .env(ENV, case)
                .output()
                .unwrap();
            assert_eq!(out.status.signal(), Some(6), "case {case} must SIGABRT: {:?}", out.status);
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(stderr.contains(needle), "case {case} stderr: {stderr}");
        }
    }

    #[test]
    fn unit_ring_pack_rooms_and_thread_park_primitives() {
        let arc = Arc::new(());
        {
            let mut ring = Ring::new(3);
            assert!(ring.is_empty() && ring.len() == 0 && ring.front().is_none() && ring.pop_front().is_none());
            ring.push_back((10, Arc::clone(&arc)));
            ring.push_back((20, Arc::clone(&arc)));
            assert_eq!(ring.len(), 2);
            assert_eq!(ring.front().unwrap().0, 10);
            assert_eq!(ring.pop_front().unwrap().0, 10);
            ring.push_back((30, Arc::clone(&arc)));
            ring.push_back((40, Arc::clone(&arc)));
            assert_eq!(ring.pop_front().unwrap().0, 20);
            assert_eq!(Arc::strong_count(&arc), 3);
        }
        assert_eq!(Arc::strong_count(&arc), 1, "Ring::drop must drop remaining elements");

        for kind in [KIND_GROUP, KIND_ROOM_PIECE, KIND_MULTI] {
            for sme2 in [false, true] {
                let raw = pack_arg(kind, 0x1234_5678, 0x9abc_def0, sme2);
                assert_eq!((raw & ACTION_SME2) != 0, sme2);
                assert_eq!(unpack_arg(raw), (kind, 0x1234_5678, 0x9abc_def0));
            }
        }

        let rooms = Rooms::new(2);
        rooms.init_taken(0);
        assert!(!rooms.release_n(0, 5));
        assert!(rooms.release_n(0, ROOM_CREDITS - 5));

        let tp = Arc::new(ThreadPark(std::thread::current()));
        std::thread::park_timeout(std::time::Duration::ZERO);
        std::task::Wake::wake_by_ref(&tp);
        let t0 = std::time::Instant::now();
        std::thread::park_timeout(std::time::Duration::from_millis(200));
        assert!(t0.elapsed() < std::time::Duration::from_millis(150));

        std::thread::park_timeout(std::time::Duration::ZERO);
        std::task::Wake::wake(tp);
        let t1 = std::time::Instant::now();
        std::thread::park_timeout(std::time::Duration::from_millis(200));
        assert!(t1.elapsed() < std::time::Duration::from_millis(150));
    }

    #[test]
    fn unit_internal_capacities_group_thresholds_aborts_and_drop_cleanup() {
        crate::initialize_multithreaded(None);

        let p_small = Pipeline::<usize>::new(Mode::Plain, Limits { messages: 2, rooms: 1 });
        assert_eq!(p_small.free_pieces.buf.len(), 1 + 2 + 1);
        assert_eq!(p_small.group_count, 4);
        assert!(p_small.shared().storage.source.actions_len() >= 2 * MAX_SLOT_PIECES + 4 + 4);
        let st_ptr1 = p_small.shared().storage as *const PipeStorage as usize;

        let mut p_held = Pipeline::<usize>::new(Mode::Plain, Limits { messages: 2, rooms: 3 });
        let held_ptr = p_held.shared().storage as *const PipeStorage as usize;
        let held_room = block_on(p_held.room());
        drop(p_held);
        let p_held2 = Pipeline::<usize>::new(Mode::Plain, Limits { messages: 2, rooms: 3 });
        assert_ne!(p_held2.shared().storage as *const PipeStorage as usize, held_ptr);
        drop(held_room);
        drop(p_held2);

        drop(p_small);
        let p_same = Pipeline::<usize>::new(Mode::Plain, Limits { messages: 2, rooms: 1 });
        assert_eq!(p_same.shared().storage as *const PipeStorage as usize, st_ptr1);
        let p_diff_rooms = Pipeline::<usize>::new(Mode::Plain, Limits { messages: 2, rooms: 2 });
        assert_ne!(p_diff_rooms.shared().storage as *const PipeStorage as usize, st_ptr1);
        let p_big_msgs = Pipeline::<usize>::new(Mode::Plain, Limits { messages: 200, rooms: 1 });
        assert_ne!(p_big_msgs.shared().storage as *const PipeStorage as usize, st_ptr1);
        drop((p_same, p_diff_rooms, p_big_msgs));

        let mut pipe = Pipeline::new(Mode::Plain, Limits { messages: 4, rooms: 2 });
        block_on(async {
            let mut m1 = pipe.message(1usize, None).await;
            let mut r0 = pipe.room().await;
            pipe.commit(&mut m1, &mut r0, ROOM_LEN);
            assert_eq!(r0.len(), 0);
            assert_eq!(r0.bytes_mut().len(), 0);
            drop(r0);

            let mut r1 = pipe.room().await;
            r1.bytes_mut()[..100].fill(0x55);
            pipe.commit(&mut m1, &mut r1, 100);
            assert_eq!(r1.len(), ROOM_LEN - 128);
            assert_eq!(r1.bytes_mut().len(), ROOM_LEN - 128);

            let mut m2 = pipe.message(2usize, None).await;
            r1.bytes_mut()[..50].fill(0x77);
            pipe.commit(&mut m2, &mut r1, 50);
            assert_eq!(r1.bytes_mut().len(), ROOM_LEN - 192);
            drop(r1);
            pipe.end(m1);
            pipe.end(m2);
        });
        let evs = drain_all(&mut pipe, 2);
        let mut expected_m1 = vec![0u8; ROOM_LEN];
        expected_m1.extend_from_slice(&[0x55; 100]);
        match &evs[0] {
            Event::Hashed { hash, .. } => assert_eq!(*hash, convenience::hash(&expected_m1)),
            _ => panic!("expected Hashed"),
        }
        match &evs[1] {
            Event::Hashed { hash, .. } => assert_eq!(*hash, convenience::hash(&[0x77; 50])),
            _ => panic!("expected Hashed"),
        }

        let mut p1 = Pipeline::new(Mode::Plain, Limits { messages: 1, rooms: 0 });
        let m_empty = block_on(p1.message(10usize, None));
        p1.abort(m_empty);
        assert!(matches!(p1.next_event(), Some(Event::Aborted { tag: 10, owner: None })));
        let m_batch = block_on(p1.message(11usize, None));
        p1.batch(m_batch, Batch::new(64, 2), vec![[0u8; 32]; 2].into_boxed_slice());
        block_on(p1.ready());
        assert!(matches!(p1.next_event(), Some(Event::Batch { tag: 11, .. })));
        let m_after_batch = block_on(p1.message(12usize, None));
        p1.end(m_after_batch);
        assert!(matches!(p1.next_event(), Some(Event::Hashed { tag: 12, .. })));

        let mut p_flush = Pipeline::new(Mode::Plain, Limits { messages: 3, rooms: 1 });
        block_on(async {
            let mut ma = p_flush.message(1usize, None).await;
            let mut mb = p_flush.message(2usize, None).await;
            let mut r = p_flush.room().await;
            r.bytes_mut()[..CHUNK_LEN].fill(0xAA);
            p_flush.commit(&mut ma, &mut r, CHUNK_LEN);
            r.bytes_mut()[..64].fill(0xBB);
            p_flush.commit(&mut mb, &mut r, 64);
            drop(r);
            let r_reclaimed = p_flush.room().await;
            drop(r_reclaimed);
            p_flush.end(ma);
            p_flush.abort(mb);
        });
        let fevs = drain_all(&mut p_flush, 2);
        match &fevs[0] {
            Event::Hashed { tag: 1, hash, .. } => assert_eq!(*hash, convenience::hash(&[0xAA; CHUNK_LEN])),
            _ => panic!("expected ma Hashed"),
        }
        assert!(matches!(fevs[1], Event::Aborted { tag: 2, owner: None }));

        let mut p_thresh = Pipeline::<usize, Vec<u8>>::with_owners(Mode::Plain, Limits { messages: 128, rooms: 0 });
        let m_sub = block_on(p_thresh.message(0, None));
        p_thresh.owned(m_sub, vec![1u8; TASK_LEN / 4 - 1]);
        assert!(p_thresh.open_group.is_some());
        assert!(p_thresh.front_uses_open_group());
        let m_tip = block_on(p_thresh.message(1, None));
        p_thresh.owned(m_tip, vec![2u8; 1]);
        assert!(p_thresh.open_group.is_none());
        let m_next = block_on(p_thresh.message(2, None));
        p_thresh.owned(m_next, vec![3u8; 64]);
        assert!(p_thresh.open_group.is_some());
        assert!(!p_thresh.front_uses_open_group());
        let _ = drain_all(&mut p_thresh, 3);

        let mut p_bound = Pipeline::<usize, Vec<u8>>::with_owners(Mode::Plain, Limits { messages: 8, rooms: 0 });
        let mb1 = block_on(p_bound.message(0, None));
        p_bound.batch(mb1, Batch::new(1024, 63), vec![[0u8; 32]; 63].into_boxed_slice());
        assert!(p_bound.open_group.is_some());
        let mb2 = block_on(p_bound.message(1, None));
        p_bound.batch(mb2, Batch::new(1024, 1), vec![[0u8; 32]; 1].into_boxed_slice());
        assert!(p_bound.open_group.is_none());
        let mb3 = block_on(p_bound.message(2, None));
        p_bound.batch(mb3, Batch::new(1024, 64), vec![[0u8; 32]; 64].into_boxed_slice());
        let _ = drain_all(&mut p_bound, 3);

        let owner_arc: Arc<[u8]> = Arc::from([9u8; 8]);
        {
            let mut p_drop = Pipeline::<usize, Arc<[u8]>>::with_owners(Mode::Plain, Limits { messages: 1, rooms: 0 });
            let m0 = block_on(p_drop.message(0, None));
            p_drop.end(m0);
            block_on(p_drop.ready());
            assert_eq!(p_drop.events.len(), 1);
            let m1 = block_on(p_drop.message(1, None));
            p_drop.owned(m1, Arc::clone(&owner_arc));
            p_drop.publish_open();
            while p_drop.has_in_flight_work() {
                if !p_drop.shared().storage.source.run_waiting_action() {
                    p_drop.shared().storage.source.wake_for_waiting();
                    std::thread::yield_now();
                }
            }
            assert_eq!(Arc::strong_count(&owner_arc), 2);
        }
        assert_eq!(Arc::strong_count(&owner_arc), 1);

        {
            let mut p_ab = Pipeline::<usize>::new(Mode::Plain, Limits { messages: 2, rooms: 1 });
            let pnode = p_ab.free_pieces.pop_front().unwrap();
            p_ab.shared().pieces[pnode as usize].done.store(false, Ordering::SeqCst);
            unsafe { *p_ab.shared().pieces[pnode as usize].next.get() = NONE };
            p_ab.room_streams[0] = RoomStream { room_pieces: 1, head_piece: pnode, tail_piece: pnode, stack_len: 0, merged_pieces: 0 };
            p_ab.slots[0] = SlotMeta { tag: Some(77), kind: SlotKind::WaitAborted { group: 0, epoch: 5 } };
            p_ab.free_slots.pop_front();
            p_ab.end_queue.push_back(0);
            assert!(p_ab.try_reap_one().is_none());
            p_ab.shared().pieces[pnode as usize].done.store(true, Ordering::SeqCst);
            assert!(p_ab.try_reap_one().is_none());
            p_ab.shared().storage.groups[0].completed_epoch.store(5, Ordering::SeqCst);
            assert!(matches!(p_ab.try_reap_one(), Some(Event::Aborted { tag: 77, owner: None })));
        }
    }
}
