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

use crate::lanes::places::{Kind as PlaceKind, Plan};
use crate::lanes::sources::{ACTION_SME2, Source};
use crate::lanes::{MEMBERS, Member, OwnLine, TASK_LEN, Task, pool_platform};
use crate::throughput::{Batch, Cv, LastCv, Prepared, TreeHasher, level_of};
use crate::{CHUNK_LEN, Hash, Mode, OUT_LEN, Platform};
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
const ROOM_LEVEL: u32 = level_of(ROOM_LEN as u64);

/// Minimum subtree size that benefits from the SME2 coprocessor (16 chunks).
const SME2_MIN_LEN: usize = 16 * CHUNK_LEN;

/// Maximum subtree pieces one in-memory message or batch splits into.
const MAX_SLOT_PIECES: usize = 64;

/// Initial reference credit placed on a [`Room`] when handed to the caller;
/// each `commit` consumes 1 credit locally on `&mut Room` without atomic RMW,
/// and `Room::drop` returns the unused credits in one atomic subtraction.
const ROOM_CREDITS: usize = 1 << 30;
/// High bit stamped onto `Rooms::refs[i]` by `Pipeline::drop` after all worker
/// tasks have finished, marking any still-live [`Room`] that outlives its
/// `Pipeline` so `Room::drop` releases `PipeStorage::holders` only in that rare
/// case and `make_room`/`Room::drop` pay zero `holders` atomics while the
/// `Pipeline` is alive (#494).
const ROOM_CLOSING_BIT: usize = 1usize << (usize::BITS - 1);

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
        // Recursive drop guard ensures remaining initialized elements still
        // drop if dropping an earlier element unwinds.
        struct Rest<'a, X>(&'a mut Ring<X>);
        impl<X> Drop for Rest<'_, X> {
            fn drop(&mut self) {
                while let Some(item) = self.0.pop_front() {
                    let _rest = Rest(&mut *self.0);
                    drop(item);
                    core::mem::forget(_rest);
                }
            }
        }
        let _ = Rest(self);
    }
}

/// Move-insensitive heap array of uninitialized interior-mutable slots
/// (matching `Rooms::backing`): held by raw pointer rather than `Box<[...]>`,
/// so moving `Pipeline` by value never retags pointers derived from inline
/// values stored inside the array. The caller drops every initialized element
/// before dropping `RawSlots`; `Drop` frees the backing allocation itself.
struct RawSlots<T> {
    ptr: *mut UnsafeCell<MaybeUninit<T>>,
    len: usize,
}

// Sound: `RawSlots<T>` exclusively owns its heap allocation of `len` slots.
unsafe impl<T: Send> Send for RawSlots<T> {}

impl<T> RawSlots<T> {
    fn new(len: usize) -> Self {
        let v: Vec<UnsafeCell<MaybeUninit<T>>> = (0..len).map(|_| UnsafeCell::new(MaybeUninit::uninit())).collect();
        let ptr = Box::into_raw(v.into_boxed_slice()) as *mut UnsafeCell<MaybeUninit<T>>;
        RawSlots { ptr, len }
    }

    #[inline(always)]
    fn slot(&self, index: usize) -> *mut MaybeUninit<T> {
        assert!(index < self.len);
        // Sound: `index < self.len`, within the owned allocation.
        unsafe { (*self.ptr.add(index)).get() }
    }
}

impl<T> Drop for RawSlots<T> {
    fn drop(&mut self) {
        // Sound: `ptr` and `len` came from `Box::into_raw(boxed_slice)`; all
        // initialized elements have already been dropped (or leaked if the
        // Pipeline was forgotten), and `MaybeUninit<T>` has no drop glue.
        unsafe {
            drop(Box::from_raw(core::ptr::slice_from_raw_parts_mut(self.ptr, self.len)));
        }
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
    let a = usize::try_from((clean >> 32) & 0x1fff_ffff).expect("29-bit field fits usize");
    let b = u32::from_le_bytes(clean.to_le_bytes()[..4].try_into().unwrap()) as usize;
    (kind, a, b)
}

/// Fixed memory bounds for a [`Pipeline`].
///
/// # Trait implementations
///
/// - `Clone` duplicates both capacity bounds.
/// - `Copy` preserves both counts without reserving any storage.
/// - `Debug` reports the message and room counts.
/// - `Eq` identifies limits with equal bounds.
/// - `PartialEq` compares both counts by value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Messages open or in flight at once; as many finished [`Event`]s
    /// again may wait to be pulled.
    pub messages: usize,
    /// [`Room`]s of [`ROOM_LEN`] bytes for arriving message data.
    pub rooms: usize,
}

/// Completion of a message submitted to a [`Pipeline`].
pub enum Event<T, B = (), P = Batch<'static>> {
    Hashed { tag: T, hash: Hash, owner: Option<B> },
    Aborted { tag: T, owner: Option<B> },
    Batch { tag: T, batch: P, digests: Box<[[u8; 32]]> },
}

/// An open message on a [`Pipeline`].
///
/// Every `Message` goes back to its Pipeline: to [`Pipeline::end`], or to
/// [`Pipeline::abort`] when its bytes will not all come. Dropping one
/// panics. In async code an early return through `?` drops what the task
/// holds, so give the message to `abort` before returning the error.
///
/// # Trait implementations
///
/// - `Drop` panics on an unfinished message unless the thread is already
///   unwinding; dropping is not cancellation.
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
/// Hold this guard until its message's event has been taken. Dropped
/// while a worker may still read the bytes (before the message is
/// hashed), it stops the process: no worker reads freed memory.
///
/// # Trait implementations
///
/// - `Drop` stops the process (an abort, not a panic) while the loan's
///   hashing has not finished, even where no worker has been given it yet
///   or its Pipeline is gone; once it has finished, the guard may be
///   dropped before its event is taken. The guard reads a count of its
///   storage's finished work: after its Pipeline was dropped with the
///   loan still not handed to a worker, and a later Pipeline has used the
///   same storage, the drop may pass though the loan was never hashed,
///   and no worker reads its bytes then either.
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
            stop_process("blake3-servil: a Pipeline Lent guard was dropped before its message finished hashing\n");
        }
    }
}

/// Say why, and end the process: a breach after which a worker would read
/// freed memory. As the pool's own stop, the words go straight to
/// descriptor 2: `std::io::stderr` takes a lock, and a thread that holds
/// it while it waits for this one would wait for ever.
#[cold]
#[inline(never)]
fn stop_process(why: &str) -> ! {
    crate::lanes::stop(why)
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
    refs: Box<[AtomicUsize]>,
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
            refs: (0..count).map(|_| AtomicUsize::new(0)).collect(),
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
    custom_tree: UnsafeCell<Option<TreeHasher>>,
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
    /// Who holds this storage: its Pipeline while alive, plus teardown
    /// reservations and any surviving Rooms stamped with `ROOM_CLOSING_BIT`.
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
    /// #427). The process now holds, for Pipelines of the same Limits, as
    /// many as its most such Pipelines at once (`acquire` gives a storage
    /// only to a Pipeline of as many rooms whose actions, messages and
    /// groups all fit in it: one of another size takes a new storage. A
    /// storage is made with 1,024 actions, 64 messages and 64 groups at
    /// the least, and the actions bind first: with one Room, 15 messages
    /// fit a storage made for one, and 16 do not).
    static FREE_STORAGES: Mutex<Vec<&'static PipeStorage>> = Mutex::new(Vec::new());
}

#[cfg(any(test, shuttle))]
process_static! {
    /// The storages made so far in this process.
    static STORAGES_MADE: AtomicUsize = AtomicUsize::new(0);
}

#[cfg(shuttle)]
struct OwnedStoragePtr(*mut PipeStorage);

#[cfg(shuttle)]
// Sound: only pushed at creation and drained in `free_storages_for_model`
// after all Shuttle worker threads have joined.
unsafe impl Send for OwnedStoragePtr {}

#[cfg(shuttle)]
process_static! {
    /// Original `Box::into_raw` owning pointers for model teardown so
    /// `free_storages_for_model` never casts `&'static PipeStorage` to `*mut`.
    static MODEL_OWNED_STORAGES: Mutex<Vec<OwnedStoragePtr>> = Mutex::new(Vec::new());
}

/// The model's alone (tests/shuttle_pipeline.rs): frees a run's storages
/// at its end. In a file of its own, outside the files the mutation
/// check changes: no build that check tests holds this code.
#[cfg(shuttle)]
#[allow(dead_code)]
#[path = "../tests/model/storages.rs"]
pub(crate) mod model_storages;

impl PipeStorage {
    /// Give up `n` holds on `st` (its Pipeline's teardown holds, or a
    /// surviving Room's): the last hold puts it on the free list. A Room may
    /// outlive its Pipeline, so either may be the last.
    fn release_n(st: &'static PipeStorage, n: usize) {
        if st.holders.fetch_sub(n, Ordering::AcqRel) == n {
            FREE_STORAGES.lock().unwrap().push(st);
        }
    }

    fn release(st: &'static PipeStorage) {
        Self::release_n(st, 1);
    }

    fn acquire(actions: usize, messages: usize, rooms: usize, groups: usize) -> &'static PipeStorage {
        let chosen = {
            let mut free = FREE_STORAGES.lock().unwrap();
            let fits = |st: &&'static PipeStorage| {
                st.source.actions_len() >= actions && st.loan_completed.len() >= messages && st.rooms.count == rooms && st.groups.len() >= groups
            };
            free.iter().position(fits).map(|at| free.swap_remove(at))
        };
        if let Some(st) = chosen {
            st.holders.store(1, Ordering::Release);
            return st;
        }
        let actions = actions.max(1024);
        let messages = messages.max(64);
        let groups = groups.max(64);
        let dummy = GroupMemberMeta { room: NONE };
        #[cfg(any(test, shuttle))]
        STORAGES_MADE.fetch_add(1, Ordering::Relaxed);
        let raw = Box::into_raw(Box::new(PipeStorage {
            source: Source::with_actions(actions),
            waker: WakerSlot::new(),
            rooms: Rooms::new(rooms),
            groups: (0..groups)
                .map(|_| MemberGroup {
                    task: UnsafeCell::new(Task::members(None)),
                    metas: UnsafeCell::new([dummy; MEMBERS]),
                    custom_tree: UnsafeCell::new(None),
                    epoch: UnsafeCell::new(0),
                    completed_epoch: OwnLine(AtomicU64::new(0)),
                })
                .collect(),
            loan_completed: (0..messages).map(|_| OwnLine(AtomicU64::new(0))).collect(),
            saved_loan_epoch: UnsafeCell::new(1),
            saved_group_epoch: UnsafeCell::new(1),
            shared: UnsafeCell::new(core::ptr::null_mut()),
            holders: OwnLine(AtomicUsize::new(1)),
        }));
        #[cfg(shuttle)]
        MODEL_OWNED_STORAGES.lock().unwrap().push(OwnedStoragePtr(raw));
        // Sound: `raw` is a valid leaked allocation (freed only after all
        // Shuttle workers join in `model_storages::free_storages_for_model`).
        unsafe { &*raw }
    }
}

/// Reusable `ROOM_LEN`-byte buffer for arriving message bytes.
///
/// # Trait implementations
///
/// - `Drop` returns the unused credits; workers retain access to committed
///   bytes until they finish.
/// - `Send` lets another thread take the token and its exclusive access to
///   the writable bytes.
/// - `Sync` permits sharing a reference; writing still requires an
///   exclusive borrow.
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
        let unused = ROOM_CREDITS - (self.commits as usize);
        let prev = self.storage.rooms.refs[self.index as usize].fetch_sub(unused, Ordering::SeqCst);
        if prev & ROOM_CLOSING_BIT != 0 {
            // Pipeline::drop stamped ROOM_CLOSING_BIT after observing all
            // dispatched work complete and reserved a `holders` ref for this
            // Room; clear the slot back to 0 before releasing that hold
            // (`AcqRel` establishes happens-before with the storage returning
            // to FREE_STORAGES).
            assert_eq!(prev & !ROOM_CLOSING_BIT, unused, "surviving room ref count");
            self.storage.rooms.refs[self.index as usize].store(0, Ordering::Release);
            PipeStorage::release(self.storage);
        } else if prev == unused {
            // Even if Pipeline::drop races right after our `fetch_sub` and
            // recycles `&'static PipeStorage`, waking a WakerSlot is benign.
            self.storage.waker.notify_after_seqcst();
        }
    }
}

struct PieceNode {
    slot: UnsafeCell<u32>,
    room: UnsafeCell<u32>,
    offset: UnsafeCell<u32>,
    len: UnsafeCell<u32>,
    index: UnsafeCell<u64>,
    out: OwnLine<UnsafeCell<RoomValue>>,
    done: OwnLine<AtomicBool>,
    next: UnsafeCell<u32>,
}

#[derive(Clone, Copy)]
enum RoomValue {
    // A first room might be the whole message. Keep both children of its root.
    First([Cv; 2]),
    Whole(Cv),
    Last(LastCv),
}

struct PieceValues {
    whole: [Cv; MAX_SLOT_PIECES],
    last: Option<LastCv>,
}

#[derive(Clone, Copy)]
struct MultiDesc {
    input: *const u8,
    total_len: usize,
    piece_level: u32,
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
    /// This Pipeline's default mode. In a cell: the next Pipeline to take this
    /// `Shared` writes its own mode here (`create`), and a worker of the
    /// Pipeline before may still be returning from its last task, its
    /// `&Shared` alive. Workers read per-slot or per-group modes only while
    /// hashing, before they publish a task done.
    tree: UnsafeCell<TreeHasher>,
    /// Per-slot prepared mode, written by the owner only when a slot opens via
    /// `message_with` (`MessageWithFut::poll`) and read-only by workers until
    /// the slot finishes. Default `message()` slots leave this untouched.
    slot_trees: Box<[UnsafeCell<TreeHasher>]>,
    /// Whether `slot_trees[slot]` overrides `self.tree` for the active message
    /// in `slot` (written when a slot opens, read-only while tasks run).
    slot_custom: Box<[UnsafeCell<bool>]>,
    storage: &'static PipeStorage,
    hashes: Box<[UnsafeCell<[u8; OUT_LEN]>]>,
    cvs: Box<[UnsafeCell<PieceValues>]>,
    slot_done: Box<[OwnLine<AtomicBool>]>,
    tasks_left: Box<[OwnLine<AtomicUsize>]>,
    multi: Box<[UnsafeCell<MultiDesc>]>,
    pieces: Box<[PieceNode]>,
}

// Sound: each UnsafeCell here has one writer at a time, by the slot's or
// piece's state: the owner before it dispatches the work, the workers of that
// work until they publish it done, the owner again after. The mode's cell
// (`tree`) has its own rule, at `Shared::tree`: written by `create` alone,
// between Pipelines. Nothing here is ever freed.
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

impl Shared {
    /// The Pipeline's default mode, copied into new slots opened with `message`.
    #[inline(always)]
    fn tree(&self) -> &TreeHasher {
        // Sound: `create` alone writes the default mode, when every task of the
        // Pipeline before is published done; the owner reads it while its
        // Pipeline lives.
        unsafe { &*self.tree.get() }
    }

    /// Slot `slot`'s prepared mode, for hashing its bytes or folding its values.
    #[inline(always)]
    fn slot_tree(&self, slot: usize) -> &TreeHasher {
        // Sound: the owner writes `slot_custom[slot]` (and `slot_trees[slot]`
        // when true) when `slot` opens, before dispatching any task for `slot`,
        // and does not write them again until `slot` returns to `free_slots`.
        if unsafe { *self.slot_custom[slot].get() } { unsafe { &*self.slot_trees[slot].get() } } else { self.tree() }
    }

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
        // Sound: the owner wrote this group's task, custom_tree, epoch and
        // metas before it dispatched the group (publish_open), keeping every
        // member's input and output valid until `completed_epoch` reaches this
        // epoch.
        let task = unsafe { &*group.task.get() };
        let tree = unsafe { (*group.custom_tree.get()).as_ref() }.unwrap_or_else(|| self.tree());
        let count = task.members;
        let epoch = unsafe { *group.epoch.get() };
        unsafe { task.run_members(tree, platform) };
        // Sound: `metas` was written before dispatch and stays read-only until `completed_epoch` publishes.
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
        let slot = unsafe { *pnode.slot.get() } as usize;
        let room_idx = unsafe { *pnode.room.get() };
        let offset = unsafe { *pnode.offset.get() } as usize;
        let len = unsafe { *pnode.len.get() } as usize;
        let index = unsafe { *pnode.index.get() };
        let rooms = &self.storage.rooms;
        assert!((room_idx as usize) < rooms.count && offset <= ROOM_LEN && len <= ROOM_LEN - offset, "a piece stays within its room");
        // Sound: these are just the committed bytes. The Room's credit keeps
        // them from being written again until this task releases them below.
        let ptr = unsafe { rooms.room_ptr(room_idx as usize).add(offset) };
        let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
        let tree = self.slot_tree(slot);
        let value = if index == 0 {
            RoomValue::First(tree.subtree_halves_on(platform, ROOM_LEVEL, index, bytes))
        } else if len == ROOM_LEN {
            RoomValue::Whole(tree.subtree_cv_on(platform, ROOM_LEVEL, index, bytes))
        } else {
            RoomValue::Last(tree.last_cv_on(platform, index << ROOM_LEVEL, bytes))
        };
        // Sound: this piece's output is its worker's alone until `done`.
        unsafe { *pnode.out.0.get() = value };
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
        assert!(piece_idx < desc.pieces, "the action names a piece of its submission");
        if let Some((message_len, slot_len, per_part)) = desc.batch {
            let first = piece_idx * per_part;
            let count = (desc.total_len - first).min(per_part);
            // Sound: the batch and its digests stay in the slot (`batches[s]`)
            // until the slot's event, which waits for `slot_done`; this task's
            // messages and digests are its own range of them (`first`,
            // `count`).
            let bytes = unsafe { core::slice::from_raw_parts(desc.input.add(first * slot_len), count * slot_len) };
            let digests = unsafe { core::slice::from_raw_parts_mut(desc.digests.add(first), count) };
            // Sound: `desc.batch` came from a validated `Prepared` batch
            // (including zero-length messages padded to `BLOCK_LEN`) held alive
            // in `Pipeline::batches[slot]` until `slot_done`.
            let part = unsafe { crate::throughput::Part::from_checked_slots(bytes, message_len, slot_len) };
            self.slot_tree(slot).hash_many_on(platform, &part, digests);
            let storage = self.storage;
            if self.tasks_left[slot].fetch_sub(1, Ordering::AcqRel) == 1 {
                finished(storage, || {
                    self.slot_done[slot].swap(true, Ordering::SeqCst);
                });
            }
            return;
        }

        // Sound: the owner, or the Lent guard, keeps all the message's
        // bytes in place until the slot is done. The chunks of this slice
        // are the same partition the submitter put in the action list.
        let input = unsafe { core::slice::from_raw_parts(desc.input, desc.total_len) };
        let level = desc.piece_level;
        let bytes = input.chunks(CHUNK_LEN << level).nth(piece_idx).expect("a piece within its message");
        let values = self.cvs[slot].get();
        let tree = self.slot_tree(slot);
        if piece_idx + 1 == desc.pieces {
            let value = tree.last_cv_on(platform, (piece_idx as u64) << level, bytes);
            // Sound: only the last piece writes this field.
            unsafe { (*values).last = Some(value) };
        } else {
            let value = tree.subtree_cv_on(platform, level, piece_idx as u64, bytes);
            // Sound: a separate slot for each index (submit_multi_slice
            // limits the count); form no borrow of other workers' slots.
            unsafe { core::ptr::addr_of_mut!((*values).whole).cast::<Cv>().add(piece_idx).write(value) };
        }

        let storage = self.storage;
        if self.tasks_left[slot].fetch_sub(1, Ordering::AcqRel) == 1 {
            // Sound: this is the slot's last task (`tasks_left` reached zero),
            // so every piece's value is written and no one else reads or
            // writes the slot's values or hash until `slot_done` below.
            let values = unsafe { &mut *values };
            let last = values.last.take().expect("the last piece supplied its value");
            let hash = fold_multi_cvs(tree, &values.whole[..desc.pieces - 1], last, desc.total_len as u64);
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

/// Settle the lazy merges of `count` equal-size pieces. The 1-bits of
/// `count` name the whole subtrees left on the stack, largest first.
fn merge_pieces(tree: &TreeHasher, stack: &mut [Cv], mut used: usize, count: u64) -> usize {
    let merges = used.saturating_sub(count.count_ones() as usize);
    for _ in 0..merges {
        stack[used - 2] = tree.parent_cv(&stack[used - 2], &stack[used - 1]);
        used -= 1;
    }
    used
}

/// Add a whole piece. The same fold serves slice pieces and rooms.
fn push_piece(tree: &TreeHasher, stack: &mut [Cv], used: &mut usize, preceding: u64, piece: Cv) {
    let n = merge_pieces(tree, stack, *used, preceding);
    stack[n] = piece;
    *used = n + 1;
}

/// The whole subtrees meet the final piece on the right edge. Check the
/// root against the submitted length.
fn finish_pieces(tree: &TreeHasher, stack: &mut [Cv], used: usize, count: u64, mut last: LastCv, len: u64) -> Hash {
    let used = merge_pieces(tree, stack, used, count);
    let (first, rest) = stack[..used].split_first().expect("a whole piece before the last");
    for left in rest.iter().rev() {
        last = tree.last_parent_cv(left, &last);
    }
    tree.root_hash(first, &last, len)
}

fn fold_multi_cvs(tree: &TreeHasher, whole: &[Cv], last: LastCv, len: u64) -> Hash {
    // There is always a separate last piece. Below a power-of-two cap,
    // no push needs more slots than the cap's base-two logarithm.
    const { assert!(MAX_SLOT_PIECES.is_power_of_two()) };
    let mut stack = [Cv::unset(); MAX_SLOT_PIECES.ilog2() as usize];
    let mut used = 0;
    for (i, &value) in whole.iter().enumerate() {
        push_piece(tree, &mut stack, &mut used, i as u64, value);
    }
    finish_pieces(tree, &mut stack, used, whole.len() as u64, last, len)
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
    len: u64,
    head_piece: u32,
    tail_piece: u32,
    stack_len: usize,
    merged_pieces: u64,
}

impl RoomStream {
    const EMPTY: Self = RoomStream { len: 0, head_piece: NONE, tail_piece: NONE, stack_len: 0, merged_pieces: 0 };
}

/// Public mode provenance of a slot for short-message group batching.
/// Never inspects or compares secret key bytes: keyed and derived modes
/// always use `Custom` and are never grouped across messages.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SlotMode {
    Default,
    Plain,
    Custom,
}

struct SlotMeta<T> {
    tag: Option<T>,
    kind: SlotKind,
    mode: SlotMode,
}

process_static! {
    static NEXT_PIPELINE_ID: AtomicU64 = AtomicU64::new(1);
}

/// Streaming multithreaded hasher with bounded memory and pull-based
/// [`Event`] delivery.
///
/// In a pool of one thread (`initialize_multithreaded` asked for one, or
/// a machine with one CPU) there is no worker, and the calling thread
/// hashes: in the calls that hand work over
/// ([`commit`](Pipeline::commit), [`end`](Pipeline::end),
/// [`owned`](Pipeline::owned), [`lent`](Pipeline::lent) and
/// [`batch`](Pipeline::batch)), and in the waits
/// ([`ready`](Pipeline::ready), [`room`](Pipeline::room) and
/// [`message`](Pipeline::message)). Short messages are gathered into
/// groups, and a group that is not yet full is hashed by a later call or
/// by a wait: so a short message's event may come only after `ready` has
/// been awaited, there as with workers, and its [`Lent`] guard is held
/// until its event.
///
/// Pipelines, and the other multithreaded calls made beside them, share
/// the pool's workers. A worker looks for work among them in one order,
/// and after every 16 takings it takes once from the next of them
/// that has any, going round (a taking is one action, or a run of a
/// call's pieces): a Pipeline that streams without a pause slows
/// another's messages, and does not keep them waiting while it streams.
/// The thread kept for SME2, where there is one, takes in the one order
/// only.
///
/// # Memory
///
/// A Pipeline's storage (its Rooms, about 64 KiB each, and what it
/// shares with the pool's workers) is never returned to the system. It
/// is in use while its Pipeline or any [`Room`] of that Pipeline is
/// alive, and is then kept for a later Pipeline. A process whose
/// Pipelines all have the same [`Limits`] so holds as many storages as
/// it has had in use at once: a Room kept past its Pipeline's drop keeps
/// that Pipeline's storage, and the next Pipeline takes another. No
/// more than that is promised. A kept storage serves a Pipeline of
/// other Limits only where it has as many Rooms and room in each of its
/// queues for that Pipeline's messages, which a small change of
/// `messages` may or may not leave it (and what the storage shares with
/// the workers may then be made anew, the old kept beside it); any
/// other Pipeline is given new storage, and the old is kept. So a
/// process that varies its Limits holds more than its most in use at
/// once. The storage is not cleared: a keyed Pipeline's key and the
/// bytes last written to its Rooms stay in the process's memory until
/// another Pipeline writes over them, and no call of this crate wipes
/// them.
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
///   [`next_event`](Pipeline::next_event). Dropped while a worker may
///   still be reading its buffers, it stops the whole process. (Dropped
///   earlier than this rule allows at a moment when nothing it has handed
///   to the pool is unfinished, it drops what it holds, and what it had
///   not handed over is never hashed.)
///
/// So keep a Pipeline in a task that outlives its clients (one for a
/// thread's connections, not one a connection), and do not let a
/// time-out or a cancelled task drop it mid-message.
///
/// # Panics
///
/// Each call says what it refuses. When `owned`, `lent`, or `batch` panics
/// on a [`Message`] of this Pipeline (bytes given two ways, a batch of the
/// wrong shape, or an owner's `as_ref` panicking), the rejected message's
/// slot is aborted cleanly (releasing any Room credit, dropping any
/// unwritten owner, and queuing [`Event::Aborted`]) so the Pipeline stays
/// usable and later [`message`](Pipeline::message) calls do not hang.
/// A `Message` given by value to another Pipeline is refused before that
/// Pipeline touches its own slots (so the refusing Pipeline goes on as it
/// was, while the other Pipeline's slot remains open with no `Message` left
/// to name it, and that other Pipeline is good only for dropping).
/// ([`commit`](Pipeline::commit) borrows its `Message`: after its panic
/// the `Message` is still the program's, and its drop panics as any open
/// Message's does, unless the thread is unwinding already.)
/// A [`Waker`](core::task::Waker) that panics when it is cloned or dropped
/// may leave the Pipeline usable only for dropping; a waker that panics
/// when it is woken ends the process, on whichever thread it is woken: the
/// wake comes in the middle of other work, which a panic would leave half
/// done.
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
///
/// # Trait implementations
///
/// - `Drop` stops the process (an abort) if work it has handed to the
///   pool is unfinished, whether a worker is hashing it or it still waits
///   for one; otherwise it drops quietly, with the messages not yet handed
///   over, the events not taken and their owners, and keeps its buffers
///   for a later Pipeline.
/// - `Send` lets a pipeline move between threads when its tags, owners, and
///   batches are `Send`, keeping the same messages and tokens.
pub struct Pipeline<T, B = (), P = Batch<'static>> {
    id: u64,
    limits: Limits,
    group_count: usize,
    source_slot: Option<usize>,
    shared: *mut Shared,
    slots: Box<[SlotMeta<T>]>,
    room_streams: Box<[RoomStream]>,
    owners: RawSlots<B>,
    batches: RawSlots<(P, Box<[[u8; OUT_LEN]]>)>,
    /// Per-slot adaptive locality plan for split `Pipeline::batch` submissions
    /// (`[TASK_LEN, places::FAR_LEN)`).
    batch_plans: Box<[Option<Plan>]>,
    batches_in_flight: usize,
    /// FIFO ring of free message slots: slots cycle around the full ring
    /// before reuse so worker-written cache lines cool before another worker writes them.
    free_slots: Ring<u32>,
    open_messages: usize,
    end_queue: Ring<u32>,
    events: Ring<Event<T, B, P>>,
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
    local_batch: Option<(usize, usize)>,
    local_group_mode: SlotMode,
    local_custom_tree: Option<TreeHasher>,
    local_members: Box<[Member; MEMBERS]>,
    local_rooms: Box<[u32; MEMBERS]>,
    next_loan_epoch: u64,
    next_group_epoch: u64,
    assigned_group_epoch: Box<[u64]>,
    seen_group_epoch: Box<[u64]>,
    action_buf: [u64; MAX_SLOT_PIECES],
}

// Sound: the Pipeline owns its tags, owners, and batches (hence
// `T: Send, B: Send, P: Send`); the raw pointers are to its never-freed Shared
// and into buffers it holds, which workers read only through tasks it dispatched.
unsafe impl<T: Send, B: Send, P: Send> Send for Pipeline<T, B, P> {}

impl<T> Pipeline<T, (), Batch<'static>> {
    /// Create a pipeline in `mode` with fixed memory `limits`.
    ///
    /// # Panics
    ///
    /// If `limits.messages` is zero, or if `limits.messages` and
    /// `limits.rooms` together reach 2^29 - 1 (the Pipeline numbers its
    /// work in 29 bits). Before
    /// [`initialize_multithreaded`](crate::initialize_multithreaded), and
    /// in a child process forked after that call started the pool's
    /// threads.
    pub fn new(mode: Mode, limits: Limits) -> Self {
        Self::create(mode, limits)
    }
}

impl<T, P: Prepared + 'static> Pipeline<T, (), P> {
    /// Create a pipeline whose [`Pipeline::batch`] accepts owned prepared
    /// batches of type `P`, such as [`Packed<16>`](crate::throughput::Packed),
    /// [`Packed<32>`](crate::throughput::Packed),
    /// [`Packed<48>`](crate::throughput::Packed), `Vec<[u8; 32]>`,
    /// `Box<[[u8; 32]]>`, or [`Batch<'static>`].
    ///
    /// # Panics
    ///
    /// As [`new`](Pipeline::new).
    pub fn with_batches(mode: Mode, limits: Limits) -> Self {
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
impl<T, B: AsRef<[u8]> + 'static> Pipeline<T, B, Batch<'static>> {
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
    ///
    /// # Panics
    ///
    /// As [`new`](Pipeline::new).
    pub fn with_owners(mode: Mode, limits: Limits) -> Self {
        Self::create(mode, limits)
    }
}

impl<T, B: AsRef<[u8]> + 'static, P: Prepared + 'static> Pipeline<T, B, P> {
    /// Create a pipeline that accepts both owned message buffers of type `B`
    /// ([`Pipeline::owned`]) and owned prepared batches of type `P`
    /// ([`Pipeline::batch`]).
    ///
    /// # Panics
    ///
    /// As [`new`](Pipeline::new).
    pub fn with_owners_and_batches(mode: Mode, limits: Limits) -> Self {
        Self::create(mode, limits)
    }
}

impl<T, B: AsRef<[u8]> + 'static, P> Pipeline<T, B, P> {
    /// Hash `owner`'s bytes in place and return `owner` in the message's
    /// [`Event::Hashed`].
    ///
    /// # Panics
    ///
    /// If `message` is another Pipeline's, or has had a commit from a
    /// Room already, even one of no bytes: a message's bytes come one way.
    #[inline]
    pub fn owned(&mut self, message: Message, owner: B) {
        assert_eq!(message.pipeline_id, self.id, "Message belongs to a different Pipeline");
        let (_, s) = message.take();
        self.require_uncommitted(s);

        // Sound: the slot is open and has no owner yet (require_uncommitted);
        // the owner stays in the move-insensitive raw slot, where its bytes
        // cannot move or be retagged by moving `Pipeline`, until the slot's
        // event reads it out or the Pipeline's drop drops it.
        let slot: *mut B = unsafe { (*self.owners.slot(s)).write(owner) };
        let pipe: *mut Self = self;
        // `as_ref` is the program's code. Until the slice is submitted
        // nothing records that the slot holds an owner, so if `as_ref`
        // unwinds, drop the owner in place and abort slot `s` cleanly so
        // `open_messages` stays exact and the Pipeline remains usable (#465).
        // The pointer fields (`pipe`, `slot`) are raw pointers: an owner may
        // hold its bytes inside itself (`[u8; 8]`), and forgetting a guard
        // holding `&mut` would retag it after the bytes' pointer was taken
        // (Miri, November 20).
        struct Unrecorded<T, B, P> {
            pipe: *mut Pipeline<T, B, P>,
            slot: *mut B,
            s: usize,
        }
        impl<T, B, P> Drop for Unrecorded<T, B, P> {
            fn drop(&mut self) {
                // Sound: the owner was written just above and nothing else
                // holds or records it; `self.pipe` points to this live
                // Pipeline; this runs only while `as_ref` unwinds (the guard
                // is forgotten once `as_ref` returns).
                unsafe {
                    core::ptr::drop_in_place(self.slot);
                    (*self.pipe).abort_slot(self.s);
                }
            }
        }
        let unrecorded = Unrecorded { pipe, slot, s };
        // Sound: the owner just written, in its slot, read through a
        // shared borrow; nothing else names it.
        let bytes: &[u8] = unsafe { &*slot }.as_ref();
        let (ptr, len) = (bytes.as_ptr(), bytes.len());
        core::mem::forget(unrecorded);
        self.open_messages -= 1;
        self.submit_slice(s, ptr, len, true);
        self.end_queue.push_back(u32::try_from(s).expect("slot fits u32"));
    }
}

impl<T, B, P: Prepared + 'static> Pipeline<T, B, P> {
    /// Hash `batch` into `digests` and return both in [`Event::Batch`].
    ///
    /// Accepts any owned [`Prepared`] batch type `P` chosen when the pipeline
    /// was constructed ([`Batch<'static>`] by default, or
    /// [`Packed<N>`](crate::throughput::Packed) / `Vec<[u8; N]>` /
    /// `Box<[[u8; N]]>` via [`Pipeline::with_batches`] or
    /// [`Pipeline::with_owners_and_batches`]).
    ///
    /// # Panics
    ///
    /// If `message` is another Pipeline's, or has had a commit from a
    /// Room already (even one of no bytes); or if `digests` has not one place
    /// for each message of `batch`.
    pub fn batch(&mut self, message: Message, batch: P, digests: Box<[[u8; OUT_LEN]]>) {
        assert_eq!(message.pipeline_id, self.id, "Message belongs to a different Pipeline");
        let (_, s) = message.take();
        self.require_uncommitted(s);
        struct AbortOnUnwind<'a, T, B, P> {
            pipe: &'a mut Pipeline<T, B, P>,
            s: usize,
        }
        impl<T, B, P> Drop for AbortOnUnwind<'_, T, B, P> {
            fn drop(&mut self) {
                self.pipe.abort_slot(self.s);
            }
        }
        let guard = AbortOnUnwind { pipe: self, s };
        crate::assert_slots(batch.bytes(), batch.message_len(), batch.slot(), digests.len());
        core::mem::forget(guard);
        self.open_messages -= 1;

        // Sound: the slot is open and holds no batch (require_uncommitted); the
        // batch and digests stay in the move-insensitive raw slot until its
        // event reads them out or the Pipeline's drop drops them.
        let hold = unsafe { (*self.batches.slot(s)).write((batch, digests)) };
        let msg_len = hold.0.message_len();
        let slot_len = hold.0.slot();
        let bytes = hold.0.bytes();
        let count = hold.1.len();
        let digests_ptr = hold.1.as_mut_ptr();
        let work_slot = slot_len.max(crate::BLOCK_LEN);
        let work_bytes = count.saturating_mul(work_slot);

        if count == 0 {
            self.shared().slot_done[s].store(true, Ordering::Relaxed);
            self.slots[s].kind = SlotKind::WaitBatch { group: NONE, epoch: 0 };
        } else if work_bytes < TASK_LEN {
            let (group, epoch) = self.push_member(s, bytes.as_ptr(), bytes.len(), digests_ptr as *mut u8, NONE, Some((msg_len, slot_len)));
            self.slots[s].kind = SlotKind::WaitBatch { group, epoch };
        } else {
            self.slots[s].kind = SlotKind::WaitBatch { group: NONE, epoch: 0 };
            let per_part = (TASK_LEN / work_slot).max(1).max(count.div_ceil(MAX_SLOT_PIECES));
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
                    piece_level: 0,
                    pieces,
                    batch: Some((msg_len, slot_len, per_part)),
                    digests: digests_ptr,
                    loan_epoch: 0,
                }
            };
            let s32 = u32::try_from(s).expect("slot fits u32");
            for p in 0..pieces {
                let first = p.saturating_mul(per_part);
                let piece_n = (count - first).min(per_part);
                let sme2 = piece_n.saturating_mul(work_slot) >= SME2_MIN_LEN;
                self.action_buf[p] = pack_arg(KIND_MULTI, s32, u32::try_from(p).expect("piece fits u32"), sme2);
            }
            let near = if self.source_slot.is_some() {
                self.batches_in_flight += 1;
                let mut plan = Plan::new(PlaceKind::PipelineBatch, work_bytes, 1);
                plan.in_flight = self.batches_in_flight;
                let near = plan.near();
                self.batch_plans[s] = Some(plan);
                near
            } else {
                false
            };
            self.dispatch_many(pieces, near);
        }
        self.end_queue.push_back(u32::try_from(s).expect("slot fits u32"));
    }
}

impl<T, B, P> Pipeline<T, B, P> {
    #[inline(always)]
    fn shared(&self) -> &Shared {
        // Sound: `shared` is this Pipeline's Shared (create), never freed.
        unsafe { &*self.shared }
    }

    fn create(mode: Mode, limits: Limits) -> Self {
        crate::lanes::require();
        prepare_thread();
        assert!(limits.messages > 0, "Pipeline Limits.messages must be at least 1");
        let tree = TreeHasher::new(mode);
        let m = limits.messages;
        // Each limit alone first: their sum, next, could wrap round to a
        // small one (`rooms: usize::MAX` passed the check below so).
        assert!(m < 1 << 29 && limits.rooms < 1 << 29, "Pipeline Limits beyond what an action can name");
        let p_cap = limits.rooms + m + 1;
        let g_cap = m.clamp(4, 256);
        let actions_cap = m * MAX_SLOT_PIECES + p_cap + g_cap;
        // An action names a slot, a piece or a group in 29 bits (pack_arg).
        assert!(m < 1 << 29 && p_cap < 1 << 29, "Pipeline Limits beyond what an action can name");
        // Reserve a non-wrapping identity before acquiring storage or a source
        // slot, and keep exhaustion sticky at u64::MAX so a later caller cannot
        // wrap back to previously issued IDs.
        let id = loop {
            let cur = NEXT_PIPELINE_ID.load(Ordering::Relaxed);
            assert!(cur < u64::MAX, "Pipeline ID counter exhausted");
            if NEXT_PIPELINE_ID.compare_exchange(cur, cur + 1, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
                break cur;
            }
        };

        let storage = PipeStorage::acquire(actions_cap, m, limits.rooms, g_cap);
        // Sound: this Pipeline now holds the storage alone
        // (PipeStorage::acquire), and these cells are its holder's.
        let next_loan_epoch = unsafe { *storage.saved_loan_epoch.get() };
        let next_group_epoch = unsafe { *storage.saved_group_epoch.get() };
        // A pool of one thread has no worker to take a published action,
        // and only this crate's block_on would run it: an outside executor
        // waited for ever. Unregistered, every action runs where it is
        // dispatched, as when no slot is free.
        let source_slot = if crate::lanes::pool_has_workers() { storage.source.register() } else { None };
        crate::lanes::sources::opened();

        let mut local_free_rooms = Ring::new(limits.rooms.max(1));
        for i in 0..limits.rooms {
            local_free_rooms.push_back(u32::try_from(i).expect("room index fits u32"));
        }
        // The storage's `Shared`, where it has one of this size: every task
        // of the Pipeline before is done (its drop saw to that), and only
        // the mode is this Pipeline's own.
        // Sound: as the saved epochs above. A Shared the storage keeps was the
        // Pipeline before's, whose every task is published done (its drop saw
        // to that), so reading its lengths races with no worker.
        let held = unsafe { *storage.shared.get() };
        let shared: *mut Shared = if !held.is_null() && unsafe { (&(*held).hashes).len() >= m } {
            // A storage has one count of rooms, so pieces enough too.
            assert!(unsafe { (&(*held).pieces).len() } >= p_cap, "a Shared of as many messages has as many pieces");
            // Sound: nor does writing its mode. A worker may still be
            // returning from that last task with its `&Shared`: the mode is
            // in a cell, and the worker's borrow of it ended before it
            // published the task (Shared::tree).
            unsafe {
                *(*held).tree.get() = tree;
            }
            held
        } else {
            // One it replaces stays allocated: a worker of an earlier
            // Pipeline may still be reading it.
            let new = Box::into_raw(Box::new(Shared {
                tree: UnsafeCell::new(tree.clone()),
                slot_trees: (0..m).map(|_| UnsafeCell::new(tree.clone())).collect(),
                slot_custom: (0..m).map(|_| UnsafeCell::new(false)).collect(),
                storage,
                hashes: (0..m).map(|_| UnsafeCell::new([0u8; OUT_LEN])).collect(),
                cvs: (0..m).map(|_| UnsafeCell::new(PieceValues { whole: [Cv::unset(); MAX_SLOT_PIECES], last: None })).collect(),
                slot_done: (0..m).map(|_| OwnLine(AtomicBool::new(true))).collect(),
                tasks_left: (0..m).map(|_| OwnLine(AtomicUsize::new(0))).collect(),
                multi: (0..m)
                    .map(|_| {
                        UnsafeCell::new(MultiDesc {
                            input: core::ptr::null(),
                            total_len: 0,
                            piece_level: level_of(TASK_LEN as u64),
                            pieces: 0,
                            batch: None,
                            digests: core::ptr::null_mut(),
                            loan_epoch: 0,
                        })
                    })
                    .collect(),
                pieces: (0..p_cap)
                    .map(|_| PieceNode {
                        slot: UnsafeCell::new(0),
                        room: UnsafeCell::new(0),
                        offset: UnsafeCell::new(0),
                        len: UnsafeCell::new(0),
                        index: UnsafeCell::new(0),
                        out: OwnLine(UnsafeCell::new(RoomValue::Whole(Cv::unset()))),
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
            free_slots.push_back(u32::try_from(i).expect("slot index fits u32"));
        }
        let mut free_pieces = Ring::new(p_cap);
        for i in 0..p_cap {
            free_pieces.push_back(u32::try_from(i).expect("piece index fits u32"));
        }

        Pipeline {
            id,
            limits,
            group_count: g_cap,
            source_slot,
            shared,
            slots: (0..m).map(|_| SlotMeta { tag: None, kind: SlotKind::Free, mode: SlotMode::Default }).collect(),
            room_streams: vec![RoomStream::EMPTY; m].into_boxed_slice(),
            owners: RawSlots::new(m),
            batches: RawSlots::new(m),
            batch_plans: vec![None; m].into_boxed_slice(),
            batches_in_flight: 0,
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
            local_group_mode: SlotMode::Default,
            local_custom_tree: None,
            local_members: Box::new([Member { input: core::ptr::null(), len: 0, out: core::ptr::null_mut() }; MEMBERS]),
            local_rooms: Box::new([NONE; MEMBERS]),
            next_loan_epoch,
            next_group_epoch,
            assigned_group_epoch: vec![0u64; g_cap].into_boxed_slice(),
            seen_group_epoch: vec![0u64; g_cap].into_boxed_slice(),
            action_buf: [0; MAX_SLOT_PIECES],
        }
    }

    /// Wait for a free message slot and open a [`Message`] in this pipeline's
    /// default [`Mode`].
    ///
    /// # Panics
    ///
    /// If no slot is free and waiting cannot free one: as many messages
    /// as `Limits.messages` are open, or as many events wait to be taken
    /// ([`next_event`](Pipeline::next_event)). Only this task could end
    /// one or take one, and it is here.
    #[inline(always)]
    pub fn message(&mut self, tag: T, length_hint: Option<u64>) -> impl Future<Output = Message> + '_ {
        let _ = length_hint;
        MessageFut { pipe: self, tag: Some(tag), registered: false }
    }

    /// Wait for a free message slot and open a [`Message`] hashed in `mode`,
    /// overriding this pipeline's default [`Mode`] for that message alone.
    ///
    /// `mode` is prepared immediately, so the returned future does not borrow
    /// `mode`'s key or context string. Short messages opened with
    /// [`Pipeline::message`] or `message_with(Mode::Plain, ..)` batch by
    /// public provenance; keyed and derived per-message modes are never
    /// grouped across messages so dispatch never depends on secret key bytes.
    ///
    /// # Panics
    ///
    /// Same as [`Pipeline::message`].
    #[inline(always)]
    pub fn message_with<'p>(&'p mut self, mode: Mode<'_>, tag: T, length_hint: Option<u64>) -> impl Future<Output = Message> + use<'p, T, B, P> {
        let _ = length_hint;
        let is_plain = matches!(mode, Mode::Plain);
        MessageWithFut { pipe: self, tag: Some(tag), tree: Some(TreeHasher::new(mode)), is_plain, registered: false }
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
    ///
    /// A Room is free again when it has been dropped, on any thread, and
    /// the bytes committed from it have been hashed. (A message's last
    /// commit of less than a whole Room may keep its Room until the
    /// message is ended or aborted, or until a wait here sends it on.) A
    /// program that keeps every Room alive waits here for ever.
    ///
    /// # Panics
    ///
    /// If the Pipeline has no Rooms (`Limits.rooms` is 0).
    #[inline(always)]
    pub fn room(&mut self) -> impl Future<Output = Room> + '_ {
        RoomFut { pipe: self, registered: false }
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
                    self.local_free_rooms.push_back(u32::try_from(i).expect("room index fits u32"));
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
        Room { pipeline_id: self.id, index, offset: 0, commits: 0, ptr: storage.rooms.room_ptr(index as usize), storage }
    }

    /// A Pipeline with no Rooms has none to wait for. One whose Rooms are
    /// all alive may be waited on: a Room is `Send`, and another thread's
    /// drop of one ends the wait (`Room::drop` wakes it).
    fn assert_room_can_progress(&self) {
        assert!(self.limits.rooms > 0, "Pipeline::room called on a Pipeline with Limits.rooms == 0");
    }

    /// Commit the first `written` bytes of `room` as `message`'s next bytes.
    ///
    /// `room` advances to the next 64-byte boundary. Every commit to a
    /// message except its last must be a whole room (`ROOM_LEN` bytes).
    ///
    /// # Panics
    ///
    /// If `message` or `room` is another Pipeline's; if `written` is more
    /// than `room` has left ([`Room::len`]); if an earlier commit to
    /// `message` was of less than a whole Room.
    #[inline(always)]
    pub fn commit(&mut self, message: &mut Message, room: &mut Room, written: usize) {
        assert_eq!(message.pipeline_id, self.id, "Message belongs to a different Pipeline");
        assert_eq!(room.pipeline_id, self.id, "Room belongs to a different Pipeline");
        assert!(written <= room.len(), "commit wrote {written} bytes into a Room with {} left", room.len());

        let s = message.slot as usize;
        if matches!(self.slots[s].kind, SlotKind::OpenEmpty) && written < ROOM_LEN {
            let start_offset = room.offset;
            room.offset = u32::try_from(((start_offset as usize + written).div_ceil(64) * 64).min(ROOM_LEN)).expect("bounded by ROOM_LEN");
            room.take_credit();
            self.slots[s].kind = SlotKind::OpenShort { room: room.index, offset: start_offset, len: u32::try_from(written).expect("written < ROOM_LEN") };
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
                room.offset = u32::try_from(ROOM_LEN).unwrap();
                self.room_streams[s] = RoomStream::EMPTY;
                self.slots[s].kind = SlotKind::OpenRooms { sealed: false };
            }
            SlotKind::OpenShort { .. } | SlotKind::OpenShortDispatched { .. } | SlotKind::OpenRooms { sealed: true } => {
                panic!("commit after a commit of less than ROOM_LEN: every commit to a message but its last is a whole room");
            }
            SlotKind::OpenRooms { ref mut sealed } => {
                if written == ROOM_LEN {
                    debug_assert_eq!(start_offset, 0);
                    room.offset = u32::try_from(ROOM_LEN).unwrap();
                } else {
                    *sealed = true;
                    room.offset = u32::try_from(((start_offset as usize + written).div_ceil(64) * 64).min(ROOM_LEN)).expect("bounded by ROOM_LEN");
                    if written == 0 {
                        return;
                    }
                }
            }
            _ => unreachable!("Message token refers to a non-open slot"),
        }

        room.take_credit();
        let rs = &mut self.room_streams[s];
        // The earlier commits all filled a whole room. A short one
        // seals the message, so there can be no commit after it.
        let index = rs.len / ROOM_LEN as u64;
        rs.len = rs.len.checked_add(written as u64).expect("a message fits in u64 bytes");

        self.drain_slot_pieces(s);
        let pnode = self.alloc_piece_node(s);
        // Sound: as `shared()`. The node came off the free list, so no task
        // uses it: its fields are the owner's to write before the dispatch
        // below.
        let shared = unsafe { &*self.shared };
        let node = &shared.pieces[pnode as usize];
        unsafe {
            *node.slot.get() = u32::try_from(s).expect("slot fits u32");
            *node.room.get() = room_idx;
            *node.offset.get() = start_offset;
            *node.len.get() = u32::try_from(written).expect("written <= ROOM_LEN");
            *node.index.get() = index;
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
            if self.free_pieces.is_empty() {
                // Every node may be another open message's: its later
                // pieces finished while a worker was on an earlier one,
                // and only a commit to it or its end would fold them.
                // (A slot with no pieces has none to fold.)
                for other in 0..self.slots.len() {
                    if other != current_slot {
                        self.drain_pieces(other, true);
                    }
                }
            }
            if let Some(p) = self.free_pieces.pop_front() {
                return p;
            }
            crate::lanes::require();
            if !self.shared().storage.source.run_waiting_action() {
                core::hint::spin_loop();
            }
        }
    }

    /// Finish `message` and queue its [`Event::Hashed`] in end order.
    ///
    /// # Panics
    ///
    /// If `message` is another Pipeline's.
    #[inline(always)]
    pub fn end(&mut self, message: Message) {
        assert_eq!(message.pipeline_id, self.id, "Message belongs to a different Pipeline");
        let (_, s) = message.take();
        self.open_messages -= 1;

        match self.slots[s].kind {
            SlotKind::OpenShort { room: room_idx, offset, len } => {
                // Sound: the offset is within the room (commit checked
                // `written` against what was left).
                let ptr = unsafe { self.shared().storage.rooms.room_ptr(room_idx as usize).add(offset as usize) };
                let out = self.shared().hashes[s].get() as *mut u8;
                let (group, epoch) = self.push_member(s, ptr, len as usize, out, room_idx, None);
                self.slots[s].kind = SlotKind::WaitSingle { has_owner: false, group, epoch };
                self.end_queue.push_back(u32::try_from(s).expect("slot fits u32"));
            }
            _ => self.end_slow(s),
        }
    }

    #[inline(never)]
    fn end_slow(&mut self, s: usize) {
        match self.slots[s].kind {
            SlotKind::OpenEmpty => {
                let empty_hash = self.shared().slot_tree(s).hash(&[]);
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
        self.end_queue.push_back(u32::try_from(s).expect("slot fits u32"));
    }

    /// Abort `message` and queue its [`Event::Aborted`] in end order.
    ///
    /// # Panics
    ///
    /// If `message` is another Pipeline's.
    pub fn abort(&mut self, message: Message) {
        assert_eq!(message.pipeline_id, self.id, "Message belongs to a different Pipeline");
        let (_, s) = message.take();
        self.abort_slot(s);
    }

    fn abort_slot(&mut self, s: usize) {
        self.open_messages -= 1;

        match self.slots[s].kind {
            SlotKind::OpenEmpty => {
                self.slots[s].kind = SlotKind::WaitAborted { group: NONE, epoch: 0 };
            }
            SlotKind::OpenShort { room: room_idx, .. } => {
                // The record first, whole, and the wake last: the wake runs
                // the program's waker in the middle of this call. (A waker
                // that panics ends the process, in the waker slot's wake.
                // Until it did, such a panic, caught, could leave a slot
                // that still said it held the room's credit, to be given
                // back a second time.)
                self.slots[s].kind = SlotKind::WaitAborted { group: NONE, epoch: 0 };
                self.end_queue.push_back(u32::try_from(s).expect("slot fits u32"));
                if self.shared().storage.rooms.release_n(room_idx, 1) {
                    self.shared().storage.waker.notify_after_seqcst();
                }
                return;
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
        self.end_queue.push_back(u32::try_from(s).expect("slot fits u32"));
    }

    /// Hash `bytes` in place on loan, returning a [`Lent`] guard.
    ///
    /// # Safety
    /// The returned [`Lent`] guard must be dropped and never leaked
    /// (`core::mem::forget`, `Box::leak`, a reference cycle, or a leaked
    /// task holding it).
    ///
    /// # Panics
    ///
    /// If `message` is another Pipeline's, or has had a commit from a
    /// Room already, even one of no bytes: a message's bytes come one way.
    #[inline(always)]
    pub unsafe fn lent<'a>(&mut self, message: Message, bytes: &'a [u8]) -> Lent<'a> {
        assert_eq!(message.pipeline_id, self.id, "Message belongs to a different Pipeline");
        let (_, s) = message.take();
        self.require_uncommitted(s);
        self.open_messages -= 1;

        let (completed, epoch) = if bytes.len() <= TASK_LEN {
            let out = self.shared().hashes[s].get() as *mut u8;
            let (group, epoch) = self.push_member(s, bytes.as_ptr(), bytes.len(), out, NONE, None);
            self.slots[s].kind = SlotKind::WaitSingle { has_owner: false, group, epoch };
            (&self.shared().storage.groups[group as usize].completed_epoch.0, epoch)
        } else {
            let epoch = self.next_loan_epoch;
            self.next_loan_epoch = epoch.checked_add(1).expect("Pipeline loan epoch counter exhausted");
            self.submit_multi_slice(s, bytes.as_ptr(), bytes.len(), false, epoch);
            (&self.shared().storage.loan_completed[s].0, epoch)
        };
        self.end_queue.push_back(u32::try_from(s).expect("slot fits u32"));
        Lent { completed, epoch, _bytes: PhantomData }
    }

    /// Pull the next finished [`Event`], if one is ready.
    ///
    /// This takes what is finished and starts nothing. A message's work
    /// may wait for [`ready`](Pipeline::ready) (or another wait of this
    /// Pipeline) to hand it to the workers, so a loop that calls only
    /// this may never be given an event: where this gives `None`, await
    /// `ready`.
    #[inline(always)]
    pub fn next_event(&mut self) -> Option<Event<T, B, P>> {
        if let Some(ev) = self.events.pop_front() {
            return Some(ev);
        }
        self.try_reap_one()
    }

    /// Wait until [`Pipeline::next_event`] has an [`Event`] ready.
    ///
    /// # Panics
    ///
    /// If no message has been ended (or aborted, or handed over whole)
    /// whose event is still to come: nothing would end the wait.
    #[inline(always)]
    pub fn ready(&mut self) -> impl Future<Output = ()> + '_ {
        ReadyFut { pipe: self, registered: false }
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
    fn require_uncommitted(&mut self, s: usize) {
        match self.slots[s].kind {
            SlotKind::OpenEmpty => {}
            SlotKind::OpenShort { .. } | SlotKind::OpenShortDispatched { .. } | SlotKind::OpenRooms { .. } => {
                self.abort_slot(s);
                panic!("bytes given two ways: Message already has committed room bytes")
            }
            _ => unreachable!("Message token refers to a non-open slot"),
        }
    }

    #[inline]
    fn submit_slice(&mut self, s: usize, ptr: *const u8, len: usize, has_owner: bool) {
        if len <= TASK_LEN {
            let out = self.shared().hashes[s].get() as *mut u8;
            let (group, epoch) = self.push_member(s, ptr, len, out, NONE, None);
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
        assert!((2..=MAX_SLOT_PIECES).contains(&pieces), "a split message fits its piece slots");
        self.shared().slot_done[s].store(false, Ordering::Relaxed);
        self.shared().tasks_left[s].store(pieces, Ordering::Relaxed);
        // Sound: as `batch`'s, above.
        unsafe {
            *self.shared().multi[s].get() = MultiDesc {
                input: ptr,
                total_len: len,
                piece_level: level_of(piece_len as u64),
                pieces,
                batch: None,
                digests: core::ptr::null_mut(),
                loan_epoch,
            }
        };
        self.slots[s].kind = SlotKind::WaitMulti { has_owner };
        let s32 = u32::try_from(s).expect("slot fits u32");
        for p in 0..pieces {
            self.action_buf[p] = pack_arg(KIND_MULTI, s32, u32::try_from(p).expect("piece fits u32"), true);
        }
        self.dispatch_many(pieces, false);
    }

    fn flush_sealed_short_rooms(&mut self) {
        for s in 0..self.slots.len() {
            if let SlotKind::OpenShort { room: room_idx, offset, len } = self.slots[s].kind {
                // Sound: as in `end`.
                let ptr = unsafe { self.shared().storage.rooms.room_ptr(room_idx as usize).add(offset as usize) };
                let out = self.shared().hashes[s].get() as *mut u8;
                let (group, epoch) = self.push_member(s, ptr, len as usize, out, room_idx, None);
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
                    return u32::try_from(g).expect("group fits u32");
                }
                if self.seen_group_epoch[g] < assigned {
                    self.seen_group_epoch[g] = self.shared().storage.groups[g].completed_epoch.load(Ordering::Acquire);
                }
                if self.seen_group_epoch[g] >= assigned {
                    self.assigned_group_epoch[g] = 0;
                    return u32::try_from(g).expect("group fits u32");
                }
            }
            crate::lanes::require();
            if !self.shared().storage.source.run_waiting_action() {
                core::hint::spin_loop();
            }
        }
    }

    #[inline(always)]
    fn push_member(&mut self, slot: usize, input: *const u8, len: usize, out: *mut u8, room: u32, batch: Option<(usize, usize)>) -> (u32, u64) {
        let slot_mode = self.slots[slot].mode;
        if self.open_group.is_some() && (self.local_batch != batch || slot_mode == SlotMode::Custom || self.local_group_mode != slot_mode) {
            self.publish_open();
        }
        let (g, epoch) = match self.open_group {
            Some(pair) => pair,
            None => self.open_new_group(slot, batch),
        };
        let idx = self.local_count;
        self.local_members[idx] = Member { input, len, out };
        self.local_rooms[idx] = room;
        let after = idx + 1;
        let work_len = match batch {
            Some((_, slot)) => (len / slot).saturating_mul(slot.max(crate::BLOCK_LEN)),
            None => len,
        };
        let bytes = self.local_bytes.saturating_add(work_len);
        self.local_count = after;
        self.local_bytes = bytes;
        let byte_limit = if batch.is_some() { TASK_LEN } else { TASK_LEN / 4 };
        if slot_mode == SlotMode::Custom || after == MEMBERS || bytes >= byte_limit {
            self.publish_open();
        }
        (g, epoch)
    }

    #[inline(never)]
    fn open_new_group(&mut self, slot: usize, batch: Option<(usize, usize)>) -> (u32, u64) {
        let g = self.acquire_free_group();
        let epoch = self.next_group_epoch;
        self.next_group_epoch = epoch.checked_add(1).expect("Pipeline group epoch counter exhausted");
        self.local_count = 0;
        self.local_bytes = 0;
        self.local_batch = batch;
        let slot_mode = self.slots[slot].mode;
        self.local_group_mode = slot_mode;
        self.local_custom_tree = if slot_mode == SlotMode::Default { None } else { Some(self.shared().slot_tree(slot).clone()) };
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
            *group.custom_tree.get() = self.local_custom_tree.take();
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
            // Sound: this Pipeline exclusively publishes to its own action
            // source, and `ctx` stays valid for the lifetime of `Shared`.
            Some(_) => unsafe { self.shared().storage.source.publish_actions(Shared::run_action, ctx, core::slice::from_ref(&arg), false) },
            // Sound: `ctx` is this Pipeline's Shared, as run_action requires.
            None => unsafe { Shared::run_action(ctx, arg, pool_platform()) },
        }
    }

    fn dispatch_many(&mut self, count: usize, near: bool) {
        let ctx = self.shared as *const Shared as *const ();
        match self.source_slot {
            // Sound: as dispatch_one's.
            Some(_) => unsafe { self.shared().storage.source.publish_actions(Shared::run_action, ctx, &self.action_buf[..count], near) },
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
        self.drain_pieces(s, false);
    }

    /// Fold slot `s`'s finished pieces, in order, up to the first still
    /// being hashed. `keep_last` leaves the newest piece: an open
    /// message may end with it, and the last piece folds as the root.
    fn drain_pieces(&mut self, s: usize, keep_last: bool) {
        // Sound: as `shared()`.
        let shared = unsafe { &*self.shared };
        let tree = shared.slot_tree(s);
        let ended = matches!(self.slots[s].kind, SlotKind::WaitRoomMulti);
        let aborted = matches!(self.slots[s].kind, SlotKind::WaitAborted { .. });

        loop {
            let head = self.room_streams[s].head_piece;
            if head == NONE || (keep_last && head == self.room_streams[s].tail_piece) || !shared.pieces[head as usize].done.load(Ordering::Acquire) {
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

            // Sound: the caller alone writes a room message's stack
            // (workers write a slice message's slots, never both for one slot).
            let cvs = unsafe { &mut (*shared.cvs[s].get()).whole };
            if ended && rs.merged_pieces + 1 == rs.len.div_ceil(ROOM_LEN as u64) {
                let root = match out {
                    RoomValue::First([left, right]) => tree.root_hash(&left, &right.into(), rs.len),
                    RoomValue::Whole(cv) => finish_pieces(tree, cvs, rs.stack_len, rs.merged_pieces, cv.into(), rs.len),
                    RoomValue::Last(last) => finish_pieces(tree, cvs, rs.stack_len, rs.merged_pieces, last, rs.len),
                };
                // Sound: the room message is done, with no other writer.
                unsafe { *shared.hashes[s].get() = *root.as_bytes() };
                shared.slot_done[s].store(true, Ordering::Relaxed);
                return;
            }
            let cv = match out {
                RoomValue::First([left, right]) => tree.parent_cv(&left, &right),
                RoomValue::Whole(cv) => cv,
                RoomValue::Last(_) => panic!("a short room is the last piece"),
            };
            push_piece(tree, cvs, &mut rs.stack_len, rs.merged_pieces, cv);
            rs.merged_pieces += 1;
        }
    }

    #[inline(always)]
    fn try_reap_one(&mut self) -> Option<Event<T, B, P>> {
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
                let owner = if has_owner { Some(unsafe { (*self.owners.slot(s)).assume_init_read() }) } else { None };
                Event::Hashed { tag, hash, owner }
            }
            SlotKind::WaitRoomMulti => {
                // Sound: as above.
                let hash = Hash::from_bytes(unsafe { *self.shared().hashes[s].get() });
                Event::Hashed { tag, hash, owner: None }
            }
            SlotKind::WaitBatch { .. } => {
                if let Some(plan) = self.batch_plans[s].take() {
                    self.batches_in_flight -= 1;
                    plan.done();
                }
                // Sound: a WaitBatch slot holds its batch and digests (batch
                // wrote them), read out here once.
                let (batch, digests) = unsafe { (*self.batches.slot(s)).assume_init_read() };
                Event::Batch { tag, batch, digests }
            }
            _ => unreachable!(),
        };
        self.free_slots.push_back(u32::try_from(s).expect("slot fits u32"));
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

impl<T, B, P> Drop for Pipeline<T, B, P> {
    fn drop(&mut self) {
        if self.has_in_flight_work() {
            stop_process("blake3-servil: Pipeline dropped while worker tasks are still in flight\n");
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
        // Pre-reserve one hold per room slot before stamping ROOM_CLOSING_BIT so
        // any surviving Room dropped concurrently on another thread cannot drop
        // `storage.holders` to 0 while this loop is still running.
        let room_count = storage.rooms.count;
        storage.holders.fetch_add(room_count, Ordering::Relaxed);
        let mut survivors = 0usize;
        for r in &storage.rooms.refs {
            let prev = r.fetch_or(ROOM_CLOSING_BIT, Ordering::SeqCst);
            if prev != 0 {
                survivors += 1;
            } else {
                r.store(0, Ordering::Relaxed);
            }
        }
        let release_holds = (room_count - survivors) + 1;
        // Sound: the Pipeline still holds the storage; these cells are its
        // holder's. Save epochs before any user drop (waker or owner) can unwind.
        unsafe {
            *storage.saved_loan_epoch.get() = self.next_loan_epoch;
            *storage.saved_group_epoch.get() = self.next_group_epoch;
        }
        struct ReleaseStorage(&'static PipeStorage, usize);
        impl Drop for ReleaseStorage {
            fn drop(&mut self) {
                PipeStorage::release_n(self.0, self.1);
            }
        }
        struct DrainPending<'a, T, B, P>(&'a mut Pipeline<T, B, P>);
        impl<T, B, P> Drop for DrainPending<'_, T, B, P> {
            fn drop(&mut self) {
                while let Some(s_idx) = self.0.end_queue.pop_front() {
                    let s = s_idx as usize;
                    match core::mem::replace(&mut self.0.slots[s].kind, SlotKind::Free) {
                        SlotKind::WaitSingle { has_owner: true, .. } | SlotKind::WaitMulti { has_owner: true } => {
                            let ptr = self.0.owners.slot(s);
                            let _rest = DrainPending(&mut *self.0);
                            // Sound: a slot waiting with `has_owner` holds an owner no
                            // event took.
                            unsafe { (*ptr).assume_init_drop() };
                            core::mem::forget(_rest);
                        }
                        SlotKind::WaitBatch { .. } => {
                            let ptr = self.0.batches.slot(s);
                            let _rest = DrainPending(&mut *self.0);
                            // Sound: a WaitBatch slot holds its batch and digests.
                            unsafe { (*ptr).assume_init_drop() };
                            core::mem::forget(_rest);
                        }
                        _ => {}
                    }
                }
            }
        }
        // Guards run in LIFO order (`_drain` then `_release`), so even if
        // dropping the saved waker or one owner unwinds, every remaining owner
        // and batch still drops and the storage returns to FREE_STORAGES.
        let _release = ReleaseStorage(storage, release_holds);
        let _drain = DrainPending(self);
        storage.waker.clear();
    }
}

struct MessageFut<'a, T, B, P> {
    pipe: &'a mut Pipeline<T, B, P>,
    tag: Option<T>,
    registered: bool,
}

impl<T, B, P> Unpin for MessageFut<'_, T, B, P> {}

impl<T, B, P> Drop for MessageFut<'_, T, B, P> {
    fn drop(&mut self) {
        if self.registered {
            self.pipe.shared().storage.waker.clear();
        }
    }
}

impl<T, B, P> Future for MessageFut<'_, T, B, P> {
    type Output = Message;

    #[inline(always)]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Message> {
        let this = &mut *self;
        assert!(this.tag.is_some(), "Pipeline::message future polled after completion");
        let slot = match this.pipe.free_slots.pop_front() {
            Some(s) => s,
            None => match this.pipe.poll_message(cx) {
                Poll::Ready(s) => s,
                Poll::Pending => {
                    this.registered = true;
                    return Poll::Pending;
                }
            },
        };
        this.registered = false;
        // Sound: `slot` just came off `free_slots`, so no worker task reads
        // `slot_custom[slot]` until the owner dispatches work for this slot.
        unsafe { *this.pipe.shared().slot_custom[slot as usize].get() = false };
        let sm = &mut this.pipe.slots[slot as usize];
        sm.tag = this.tag.take();
        sm.kind = SlotKind::OpenEmpty;
        sm.mode = SlotMode::Default;
        this.pipe.open_messages += 1;
        Poll::Ready(Message { pipeline_id: this.pipe.id, slot })
    }
}

struct MessageWithFut<'a, T, B, P> {
    pipe: &'a mut Pipeline<T, B, P>,
    tag: Option<T>,
    tree: Option<TreeHasher>,
    is_plain: bool,
    registered: bool,
}

impl<T, B, P> Unpin for MessageWithFut<'_, T, B, P> {}

impl<T, B, P> Drop for MessageWithFut<'_, T, B, P> {
    fn drop(&mut self) {
        if self.registered {
            self.pipe.shared().storage.waker.clear();
        }
    }
}

impl<T, B, P> Future for MessageWithFut<'_, T, B, P> {
    type Output = Message;

    #[inline(always)]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Message> {
        let this = &mut *self;
        assert!(this.tag.is_some(), "Pipeline::message_with future polled after completion");
        let slot = match this.pipe.free_slots.pop_front() {
            Some(s) => s,
            None => match this.pipe.poll_message(cx) {
                Poll::Ready(s) => s,
                Poll::Pending => {
                    this.registered = true;
                    return Poll::Pending;
                }
            },
        };
        this.registered = false;
        let tree = this.tree.take().expect("message_with tree present before completion");
        // Sound: `slot` just came off `free_slots`, so no worker task reads
        // `slot_trees[slot]` or `slot_custom[slot]` until work is dispatched.
        unsafe {
            *this.pipe.shared().slot_trees[slot as usize].get() = tree;
            *this.pipe.shared().slot_custom[slot as usize].get() = true;
        }
        let sm = &mut this.pipe.slots[slot as usize];
        sm.tag = this.tag.take();
        sm.kind = SlotKind::OpenEmpty;
        sm.mode = if this.is_plain { SlotMode::Plain } else { SlotMode::Custom };
        this.pipe.open_messages += 1;
        Poll::Ready(Message { pipeline_id: this.pipe.id, slot })
    }
}

struct RoomFut<'a, T, B, P> {
    pipe: &'a mut Pipeline<T, B, P>,
    registered: bool,
}

impl<T, B, P> Unpin for RoomFut<'_, T, B, P> {}

impl<T, B, P> Drop for RoomFut<'_, T, B, P> {
    fn drop(&mut self) {
        if self.registered {
            self.pipe.shared().storage.waker.clear();
        }
    }
}

impl<T, B, P> Future for RoomFut<'_, T, B, P> {
    type Output = Room;

    #[inline(always)]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Room> {
        if let Some(r) = self.pipe.pop_cached_room() {
            self.registered = false;
            return Poll::Ready(r);
        }
        match self.pipe.poll_room(cx) {
            Poll::Ready(r) => {
                self.registered = false;
                Poll::Ready(r)
            }
            Poll::Pending => {
                self.registered = true;
                Poll::Pending
            }
        }
    }
}

struct ReadyFut<'a, T, B, P> {
    pipe: &'a mut Pipeline<T, B, P>,
    registered: bool,
}

impl<T, B, P> Unpin for ReadyFut<'_, T, B, P> {}

impl<T, B, P> Drop for ReadyFut<'_, T, B, P> {
    fn drop(&mut self) {
        if self.registered {
            self.pipe.shared().storage.waker.clear();
        }
    }
}

impl<T, B, P> Future for ReadyFut<'_, T, B, P> {
    type Output = ();

    #[inline(always)]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if !self.pipe.events.is_empty() {
            self.registered = false;
            return Poll::Ready(());
        }
        match self.pipe.poll_ready(cx) {
            Poll::Ready(()) => {
                self.registered = false;
                Poll::Ready(())
            }
            Poll::Pending => {
                self.registered = true;
                Poll::Pending
            }
        }
    }
}

/// `block_on`'s waker: it wakes its thread, and remembers that it did.
struct ThreadPark {
    thread: thread::Thread,
    /// Set by a wake; taken by `block_on` before it sleeps. The thread's
    /// park token alone would not do: another wait on the same thread (a
    /// `block_on` inside a poll, a blocking receive) takes the token, and
    /// the wake with it.
    woken: AtomicBool,
}

impl std::task::Wake for ThreadPark {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.woken.store(true, Ordering::Release);
        self.thread.unpark();
    }
}

crate::pipeline_sync::thread_local! {
    static BLOCK_ON_PARK: Arc<ThreadPark> = Arc::new(ThreadPark { thread: thread::current(), woken: AtomicBool::new(false) });
    static BLOCK_ON_WAKER: Waker = BLOCK_ON_PARK.with(|park| Waker::from(Arc::clone(park)));
    static BLOCK_ON_STATE: core::cell::Cell<u8> = const { core::cell::Cell::new(0) };
}

#[inline(always)]
fn in_block_on() -> bool {
    BLOCK_ON_STATE.try_with(core::cell::Cell::get).unwrap_or(0) != 0
}

#[cfg(all(test, not(shuttle)))]
#[inline(always)]
fn block_on_spinning() -> bool {
    BLOCK_ON_STATE.with(core::cell::Cell::get) == 1
}

fn prepare_thread() {
    let _ = BLOCK_ON_WAKER.try_with(|_| {});
}

/// Drive `future` to completion on the current thread, parking between polls.
///
/// A `block_on` may run inside a poll of another on the same thread (a
/// future whose poll calls a helper built on this function). The two
/// share the thread's waker, and the inner one may take a wake that was
/// the outer one's: so when the inner one has taken a wake (whether it
/// slept for it or found it noted already), the outer future is polled
/// once more before it sleeps, whether or not the wake was its own. A
/// future whose every poll runs a `block_on` that takes a wake is
/// therefore polled again after each, with no wake of its own.
///
/// # Panics
///
/// On a thread that is ending, once its thread-local values are being
/// destroyed: `block_on` keeps its waker and its state in thread-local
/// storage, and a call from the destructor of another thread-local value
/// may find them gone (the standard library's panic for that).
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
    /// The state of the `block_on` this one runs inside (0: none), and
    /// whether this one took a wake.
    struct Guard(u8, bool);
    impl Drop for Guard {
        fn drop(&mut self) {
            BLOCK_ON_STATE.with(|f| f.set(self.0));
            if self.0 != 0 && self.1 {
                // This `block_on` ran inside a poll of another, on the one
                // waker the two share: a wake it took may have been the
                // other's, which polls once more before it sleeps. Only
                // where it took one: an outer future that runs a
                // `block_on` in each poll would otherwise never sleep.
                BLOCK_ON_PARK.with(|park| park.woken.store(true, Ordering::Release));
            }
        }
    }
    // Spinning, unless this `block_on` runs inside a poll of one that
    // sleeps between its polls: that one sleeps on whatever its poll left
    // registered, so a wait under this one keeps its waker from the first
    // poll. (An inner `block_on` whose future polled a wait once and gave
    // the `Poll` back as its value "spun", the wait kept no waker for it,
    // and the outer one slept through the Room being given back.)
    let prev = BLOCK_ON_STATE.with(core::cell::Cell::get);
    BLOCK_ON_STATE.with(|f| f.set(if prev == 2 { 2 } else { 1 }));
    let mut guard = Guard(prev, false);
    let mut future = core::pin::pin!(future);
    BLOCK_ON_PARK.with(|park| {
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
                    if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
                        return output;
                    }
                    // Asleep until a wake, one that came during the poll
                    // included, whoever took the thread's park token since.
                    while !park.woken.swap(false, Ordering::Acquire) {
                        thread::park();
                    }
                    guard.1 = true;
                } else if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
                    return output;
                }
            }
        })
    })
}

// The standard library's tests alone: under Shuttle
// (tests/shuttle_pipeline.rs) this file is compiled without them.
#[cfg(all(test, not(shuttle)))]
mod test {
    use super::*;

    #[test]
    #[should_panic(expected = "the children are not siblings")]
    fn the_piece_fold_refuses_swapped_values() {
        crate::initialize();
        let tree = TreeHasher::new(Mode::Plain);
        let left = Cv::at(ROOM_LEVEL, 0, [1; OUT_LEN]);
        let next = Cv::at(ROOM_LEVEL, 1, [2; OUT_LEN]);
        let last = LastCv::at(2 * (ROOM_LEN / CHUNK_LEN) as u64, 1, [3; OUT_LEN]);
        fold_multi_cvs(&tree, &[next, left], last, 2 * ROOM_LEN as u64 + 1);
    }

    #[test]
    #[should_panic(expected = "the root's children hold")]
    fn the_piece_fold_checks_the_submitted_length() {
        crate::initialize();
        let tree = TreeHasher::new(Mode::Plain);
        let left = Cv::at(ROOM_LEVEL, 0, [1; OUT_LEN]);
        let last = LastCv::at((ROOM_LEN / CHUNK_LEN) as u64, 1, [2; OUT_LEN]);
        fold_multi_cvs(&tree, &[left], last, ROOM_LEN as u64 + 2);
    }

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
        assert_eq!(unsafe { (*p_multi.shared().multi[0].get()).piece_level }, level_of(TASK_LEN as u64));
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

    fn drain_all<T, B, P>(pipeline: &mut Pipeline<T, B, P>, want: usize) -> Vec<Event<T, B, P>> {
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

        // The same limits across modes exercise the reuse of PipeStorage.
        for mode in [Mode::Plain, Mode::Keyed(&key), Mode::DeriveKey("pipeline derive"), Mode::DeriveKeyWith(&ctx_key)] {
            let expected = lens.map(|len| {
                let bytes = &input[..len];
                match mode {
                    Mode::Plain => *blake3::hash(bytes).as_bytes(),
                    Mode::Keyed(key) => *blake3::keyed_hash(key, bytes).as_bytes(),
                    Mode::DeriveKey(context) => blake3::derive_key(context, bytes),
                    Mode::DeriveKeyWith(_) => blake3::derive_key("pipeline modes", bytes),
                }
            });
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
                                assert_eq!(*hash.as_bytes(), expected[tag], "room len={}", lens[tag]);
                            }
                            _ => panic!("unexpected event"),
                        }
                    }
                }
            }
            let rem = lens.len() % 3;
            for ev in drain_all(&mut pipe, rem) {
                if let Event::Hashed { tag, hash, .. } = ev {
                    assert_eq!(*hash.as_bytes(), expected[tag]);
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
                        assert_eq!(*hash.as_bytes(), expected[tag]);
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
                    assert_eq!(*hash.as_bytes(), expected[tag]);
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
                        assert_eq!(*hash.as_bytes(), expected[tag], "lent len={len}");
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

    #[test]
    fn contract_limits_beyond_what_an_action_names_panic() {
        // The last of these wraps the sum of the two limits round to 1.
        for limits in [Limits { messages: 1 << 29, rooms: 0 }, Limits { messages: 1, rooms: (1 << 29) - 2 }, Limits { messages: 1, rooms: usize::MAX }] {
            let refused = std::panic::catch_unwind(|| Pipeline::<()>::new(Mode::Plain, limits)).err().expect("limits past 29 bits are refused");
            let why = refused.downcast_ref::<&str>().copied().map(String::from).or_else(|| refused.downcast_ref::<String>().cloned()).unwrap_or_default();
            assert!(why.contains("Pipeline Limits beyond what an action can name"), "{limits:?}: {why}");
        }
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

    /// Multiple Rooms outliving their Pipeline hold `PipeStorage::holders`
    /// via `ROOM_CLOSING_BIT` until every survivor drops, clear their ref
    /// slots back to 0, and return the storage to `FREE_STORAGES` cleanly (#494).
    #[test]
    fn surviving_rooms_return_storage_after_pipeline_drop() {
        crate::initialize_multithreaded(None);
        let mut pipe = Pipeline::<u32>::new(Mode::Plain, Limits { messages: 2, rooms: 4 });
        let storage: *const PipeStorage = pipe.shared().storage;
        let (r0, r1) = block_on(async { (pipe.room().await, pipe.room().await) });
        let (i0, i1) = (r0.index as usize, r1.index as usize);
        assert_eq!(r0.storage.holders.load(Ordering::SeqCst), 1, "live Rooms do not bump holders while Pipeline lives");
        drop(pipe);
        assert_eq!(r0.storage.holders.load(Ordering::SeqCst), 2, "two surviving Rooms hold the storage");
        for i in 0..4 {
            let v = r0.storage.rooms.refs[i].load(Ordering::SeqCst);
            if i == i0 || i == i1 {
                assert_eq!(v, ROOM_CLOSING_BIT | ROOM_CREDITS, "surviving room {i} stamped");
            } else {
                assert_eq!(v, 0, "idle room {i} cleared to 0");
            }
        }
        drop(r0);
        assert_eq!(r1.storage.holders.load(Ordering::SeqCst), 1);
        assert_eq!(r1.storage.rooms.refs[i0].load(Ordering::SeqCst), 0);
        drop(r1);
        let next = Pipeline::<u32>::new(Mode::Plain, Limits { messages: 2, rooms: 4 });
        let next_storage: *const PipeStorage = next.shared().storage;
        assert_eq!(next_storage, storage, "storage returned to FREE_STORAGES once last surviving Room dropped");
        for i in 0..4 {
            assert_eq!(next.shared().storage.rooms.refs[i].load(Ordering::SeqCst), 0);
        }
    }

    /// A commit that finds no piece node free takes back the finished ones
    /// of every open message, not only its own. One message's later
    /// pieces finish while a worker is still on an earlier one, and they
    /// then hold every node; a commit to another message waited for ever,
    /// even once all of them had finished (the 0.10 delta audit, BLAKE3
    /// #461). The earlier piece is held back here by its done flag.
    /// Without the taking back, the commit to the second message never
    /// returns, and the run's clock ends the test (.config/nextest.toml).
    #[test]
    fn a_commit_takes_back_piece_nodes_another_open_message_holds() {
        crate::initialize_multithreaded(None);
        let data: Vec<u8> = (0..5 * ROOM_LEN).map(|i| (i % 251) as u8).collect();
        let mut pipe = Pipeline::<u32>::new(Mode::Plain, Limits { messages: 2, rooms: 2 });
        block_on(async {
            let mut a = pipe.message(0, None).await;
            let mut b = pipe.message(1, None).await;
            let mut held = NONE;
            for part in data.chunks_exact(ROOM_LEN) {
                let mut room = pipe.room().await;
                room.bytes_mut().copy_from_slice(part);
                pipe.commit(&mut a, &mut room, ROOM_LEN);
                if held == NONE {
                    held = pipe.room_streams.iter().map(|stream| stream.head_piece).find(|&head| head != NONE).unwrap();
                    // A lone action waits for its owner's next await
                    // (publish_actions wakes a worker at two): run it
                    // here, unless a worker that was awake has it.
                    let done = &pipe.shared().pieces[held as usize].done;
                    while !done.load(Ordering::SeqCst) {
                        if !pipe.shared().storage.source.run_waiting_action() {
                            std::thread::yield_now();
                        }
                    }
                    done.store(false, Ordering::SeqCst);
                }
            }
            assert!(pipe.free_pieces.is_empty(), "the first message holds every node");
            pipe.shared().pieces[held as usize].done.store(true, Ordering::SeqCst);
            // Two whole rooms: a piece each.
            for part in data[ROOM_LEN - 5..].chunks(ROOM_LEN).take(2) {
                let mut room = pipe.room().await;
                room.bytes_mut()[..part.len()].copy_from_slice(part);
                pipe.commit(&mut b, &mut room, part.len());
            }
            pipe.end(b);
            pipe.end(a);
        });
        let events = drain_all(&mut pipe, 2);
        let hashes: Vec<_> = events.iter().map(|event| if let Event::Hashed { tag, hash, .. } = event { (*tag, *hash) } else { panic!("a hash") }).collect();
        assert_eq!(hashes, [(1, crate::convenience::hash(&data[ROOM_LEN - 5..3 * ROOM_LEN - 5])), (0, crate::convenience::hash(&data[..5 * ROOM_LEN]))]);
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
    fn a_wait_for_a_room_ends_when_another_thread_drops_one() {
        let mut pipe = Pipeline::<u32>::new(Mode::Plain, Limits { messages: 1, rooms: 1 });
        let held = block_on(pipe.room());
        let index = held.index;
        // Every Room is alive, and the wait is no deadlock: a Room is
        // Send, and whoever holds this one may drop it.
        assert!(poll_once(pipe.room()).is_pending());
        std::thread::spawn(move || drop(held)).join().unwrap();
        assert_eq!(block_on(pipe.room()).index, index);
    }

    /// A message of `p` ended and, for all `p` can see, still being
    /// hashed: its slot.
    fn message_in_flight(p: &mut Pipeline<u32>) -> usize {
        let (_, s) = block_on(p.message(7, None)).take();
        p.open_messages -= 1;
        p.slots[s].kind = SlotKind::WaitMulti { has_owner: false };
        p.shared().slot_done[s].store(false, Ordering::SeqCst);
        p.end_queue.push_back(s as u32);
        s
    }

    #[test]
    fn every_pending_wait_registers_its_waker_even_during_block_on_spin() {
        struct Count(AtomicUsize);
        impl std::task::Wake for Count {
            fn wake(self: Arc<Self>) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        fn notify(p: &Pipeline<u32>) {
            p.shared().storage.waker.notify_after_seqcst();
        }
        let wakes = Arc::new(Count(AtomicUsize::new(0)));
        let other = Waker::from(Arc::clone(&wakes));
        let own_woken = || BLOCK_ON_PARK.with(|park| park.woken.swap(false, Ordering::SeqCst));

        let mut p = Pipeline::<u32>::new(Mode::Plain, Limits { messages: 1, rooms: 1 });
        let mut room = Some(block_on(p.room()));
        let s = message_in_flight(&mut p);
        block_on(core::future::poll_fn(|own| {
            assert!(block_on_spinning());
            // Even while block_on spins, every Pending poll registers its
            // waker so composed outer futures that gate child polls never lose
            // a wake.
            own_woken();
            assert!(p.poll_ready(own).is_pending());
            notify(&p);
            assert!(own_woken(), "ready() must register waker on first Pending poll");
            assert!(p.poll_message(own).is_pending());
            notify(&p);
            assert!(own_woken(), "message() must register waker on first Pending poll");
            assert!(p.poll_room(own).is_pending());
            notify(&p);
            assert!(own_woken(), "room() must register waker on first Pending poll");
            // Under another waker: also kept and woken.
            let mut cx = Context::from_waker(&other);
            assert!(p.poll_ready(&mut cx).is_pending());
            notify(&p);
            assert_eq!(wakes.0.load(Ordering::SeqCst), 1, "ready() kept no waker");
            assert!(p.poll_message(&mut cx).is_pending());
            notify(&p);
            assert_eq!(wakes.0.load(Ordering::SeqCst), 2, "message() kept no waker");
            assert!(p.poll_room(&mut cx).is_pending());
            // A Room's drop is the notice a wait for a Room has.
            drop(room.take());
            assert_eq!(wakes.0.load(Ordering::SeqCst), 3, "room() kept no waker");
            // Once block_on sleeps between polls, its own waker is kept.
            BLOCK_ON_STATE.with(|state| state.set(2));
            assert!(p.poll_ready(own).is_pending());
            notify(&p);
            assert!(own_woken(), "a wait kept no waker though block_on sleeps between polls");
            Poll::Ready(())
        }));
        // The hashing ends, and the Pipeline is as any other.
        p.shared().slot_done[s].store(true, Ordering::SeqCst);
        assert!(matches!(p.next_event(), Some(Event::Hashed { tag: 7, .. })));
    }

    /// A composed outer future under `block_on` that polls `room()` once during
    /// spin and then waits on a producer flag (set right before dropping the
    /// Room) must be woken when the Room drops rather than sleeping forever.
    #[test]
    fn composed_wait_under_block_on_receives_room_wake() {
        let mut p = Pipeline::<u32>::new(Mode::Plain, Limits { messages: 1, rooms: 1 });
        let room = block_on(p.room());
        let first_poll = Arc::new(AtomicBool::new(false));
        let first_poll_p = Arc::clone(&first_poll);
        let flag = Arc::new(AtomicBool::new(false));
        let flag_p = Arc::clone(&flag);
        let producer = std::thread::spawn(move || {
            while !first_poll_p.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            std::thread::sleep(5 * crate::lanes::SPIN_BEFORE_SLEEP);
            flag_p.store(true, Ordering::Release);
            drop(room);
        });
        let mut polled_child = false;
        let got = block_on(core::future::poll_fn(|cx| {
            if !polled_child {
                polled_child = true;
                assert!(p.poll_room(cx).is_pending());
                first_poll.store(true, Ordering::Release);
                return Poll::Pending;
            }
            if !flag.load(Ordering::Acquire) {
                return Poll::Pending;
            }
            p.poll_room(cx)
        }));
        producer.join().unwrap();
        drop(got);
    }

    #[test]
    fn pack_unpack_arg_roundtrips_every_field() {
        for sme2 in [false, true] {
            for (kind, a, b) in [(0u64, 0u32, 0u32), (1, 7, 13), (2, 12345, 67890), (3, (1 << 29) - 1, u32::MAX)] {
                let arg = pack_arg(kind, a, b, sme2);
                assert_eq!((arg & ACTION_SME2 != 0), sme2);
                assert_eq!(unpack_arg(arg), (kind, usize::try_from(a).unwrap(), usize::try_from(b).unwrap()));
            }
        }
    }

    /// Dropping a cancelled `room()`, `message()`, or `ready()` future must
    /// immediately release the installed Waker so no reference cycle lingers.
    #[test]
    fn cancelling_wait_future_releases_its_waker() {
        struct Track(AtomicUsize);
        impl std::task::Wake for Track {
            fn wake(self: Arc<Self>) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let track = Arc::new(Track(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&track));
        let mut cx = Context::from_waker(&waker);
        let mut p = Pipeline::<u32>::new(Mode::Plain, Limits { messages: 1, rooms: 1 });
        let room = block_on(p.room());
        {
            let mut fut = core::pin::pin!(p.room());
            assert!(fut.as_mut().poll(&mut cx).is_pending());
            assert_eq!(Arc::strong_count(&track), 3, "room(): track + waker + WakerSlot");
        }
        assert_eq!(Arc::strong_count(&track), 2, "dropping cancelled room() future must clear WakerSlot");
        // Now hold the single message slot in flight and cancel both message() and ready().
        let s = message_in_flight(&mut p);
        {
            let mut fut = core::pin::pin!(p.message(9, None));
            assert!(fut.as_mut().poll(&mut cx).is_pending());
            assert_eq!(Arc::strong_count(&track), 3, "message(): track + waker + WakerSlot");
        }
        assert_eq!(Arc::strong_count(&track), 2, "dropping cancelled message() future must clear WakerSlot");
        {
            let mut fut = core::pin::pin!(p.ready());
            assert!(fut.as_mut().poll(&mut cx).is_pending());
            assert_eq!(Arc::strong_count(&track), 3, "ready(): track + waker + WakerSlot");
        }
        assert_eq!(Arc::strong_count(&track), 2, "dropping cancelled ready() future must clear WakerSlot");
        p.shared().slot_done[s].store(true, Ordering::SeqCst);
        assert!(matches!(p.next_event(), Some(Event::Hashed { tag: 7, .. })));
        drop(room);
    }

    /// If dropping one unreaped owner panics during `Pipeline::drop`, all
    /// remaining owners must still drop and `PipeStorage` must still return to
    /// `FREE_STORAGES`.
    #[test]
    fn pipeline_drop_cleans_remaining_owners_and_storage_after_panicking_owner() {
        struct DropOwner {
            data: [u8; 8],
            drops: Arc<AtomicUsize>,
            panic_on_drop: bool,
        }
        impl AsRef<[u8]> for DropOwner {
            fn as_ref(&self) -> &[u8] {
                &self.data
            }
        }
        impl Drop for DropOwner {
            fn drop(&mut self) {
                self.drops.fetch_add(1, Ordering::SeqCst);
                if self.panic_on_drop {
                    panic!("deliberate owner drop panic");
                }
            }
        }
        let drops = Arc::new(AtomicUsize::new(0));
        let mut p = Pipeline::<u32, DropOwner>::with_owners(Mode::Plain, Limits { messages: 2, rooms: 0 });
        let st = p.shared().storage;
        let holders_before = st.holders.load(Ordering::Acquire);
        let m1 = block_on(p.message(1, None));
        let m2 = block_on(p.message(2, None));
        // Short messages stay buffered locally in open_group (not dispatched),
        // so Pipeline::drop drops them quietly without aborting.
        p.owned(m1, DropOwner { data: [1; 8], drops: Arc::clone(&drops), panic_on_drop: true });
        p.owned(m2, DropOwner { data: [2; 8], drops: Arc::clone(&drops), panic_on_drop: false });
        let res = std::panic::catch_unwind(core::panic::AssertUnwindSafe(move || drop(p)));
        assert!(res.is_err(), "first owner panic should unwind");
        assert_eq!(drops.load(Ordering::SeqCst), 2, "second owner must still be dropped on unwind");
        assert_eq!(st.holders.load(Ordering::Acquire), holders_before - 1, "storage must be released back to free list");
    }

    /// Polling a completed `Pipeline::message` future a second time panics
    /// before popping another free slot.
    #[test]
    fn repolling_completed_message_future_panics_without_taking_slot() {
        let mut p = Pipeline::<u32>::new(Mode::Plain, Limits { messages: 2, rooms: 0 });
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        let msg = {
            let mut fut = core::pin::pin!(p.message(42, None));
            let Poll::Ready(msg) = fut.as_mut().poll(&mut cx) else { panic!("first poll must succeed") };
            let res = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
                let _ = fut.as_mut().poll(&mut cx);
            }));
            assert!(res.is_err(), "repolling completed MessageFut must panic");
            msg
        };
        assert_eq!(p.open_messages, 1, "repoll must not consume a second slot");
        p.end(msg);
        block_on(p.ready());
        assert!(matches!(p.next_event(), Some(Event::Hashed { tag: 42, .. })));
    }

    /// Moving a `Pipeline` holding pending inline (`[u8; 64]`) owners by value
    /// to another thread before publishing/reaping must preserve exact digests
    /// and owners via the move-insensitive `RawSlots` backing.
    #[test]
    fn moving_pipeline_with_pending_inline_owners_across_threads() {
        let mut p = Pipeline::<u32, [u8; 64]>::with_owners(Mode::Plain, Limits { messages: 4, rooms: 0 });
        for i in 0..4u32 {
            let m = block_on(p.message(i, None));
            p.owned(m, [i as u8 + 1; 64]);
        }
        let results = std::thread::spawn(move || {
            let mut out = Vec::new();
            while out.len() < 4 {
                if let Some(Event::Hashed { tag, hash, owner }) = p.next_event() {
                    out.push((tag, hash, owner.unwrap()));
                } else {
                    block_on(p.ready());
                }
            }
            out
        })
        .join()
        .unwrap();
        for (tag, hash, owner) in results {
            assert_eq!(owner, [tag as u8 + 1; 64]);
            assert_eq!(hash, crate::convenience::hash(&owner));
        }
    }

    #[test]
    fn a_wake_is_kept_though_another_wait_takes_the_park_token() {
        let mut polls = 0;
        block_on(core::future::poll_fn(|cx| {
            if block_on_spinning() {
                return Poll::Pending;
            }
            // From here block_on sleeps between polls.
            polls += 1;
            if polls == 1 {
                cx.waker().wake_by_ref();
                // A wait of the poll's own (a blocking receive, say) takes
                // the park token that wake left: this returns at once.
                std::thread::park();
                return Poll::Pending;
            }
            Poll::Ready(())
        }));
        assert_eq!(polls, 2);
    }

    #[test]
    fn a_block_on_inside_a_poll_leaves_the_outer_one_its_wake() {
        let mut polls = 0;
        block_on(core::future::poll_fn(|cx| {
            if block_on_spinning() {
                return Poll::Pending;
            }
            polls += 1;
            if polls == 1 {
                // This future's wake comes, and then a block_on that
                // sleeps: the two share the thread's waker, and the inner
                // one takes the wake for its own.
                cx.waker().wake_by_ref();
                // The inner future is ready only once that wake is gone:
                // it stays pending until the inner block_on has slept on
                // it. (An inner one that only spun would leave the wake
                // where it was, and prove nothing of the giving back.)
                let woken = || BLOCK_ON_PARK.with(|park| park.woken.load(Ordering::SeqCst));
                assert!(woken());
                block_on(core::future::poll_fn(|_| if woken() { Poll::Pending } else { Poll::Ready(()) }));
                assert!(in_block_on() && !block_on_spinning(), "the outer block_on still sleeps between polls");
                assert!(BLOCK_ON_PARK.with(|park| park.woken.load(Ordering::SeqCst)), "the inner block_on took the wake and left none");
                return Poll::Pending;
            }
            Poll::Ready(())
        }));
        assert_eq!(polls, 2);
    }

    /// A `block_on` inside a poll of one that sleeps between its polls
    /// keeps a wait's waker from its first poll: the outer one sleeps on
    /// it. Here the inner one's future polls the wait for a Room once and
    /// gives the `Poll` back as its value, with the only Room out; the
    /// notice a Room's return sends then wakes the thread. (The inner
    /// `block_on` spun, the wait kept no waker for a `block_on` that
    /// spins, and the outer one slept on with the Room free.)
    #[test]
    fn a_wait_polled_once_by_a_block_on_inside_a_sleeping_one_keeps_its_waker() {
        let own_woken = || BLOCK_ON_PARK.with(|park| park.woken.swap(false, Ordering::SeqCst));
        let mut p = Pipeline::<u32>::new(Mode::Plain, Limits { messages: 1, rooms: 1 });
        let room = block_on(p.room());
        let mut polls = 0;
        block_on(core::future::poll_fn(|_| {
            if block_on_spinning() {
                return Poll::Pending;
            }
            // From here the outer block_on sleeps between polls.
            polls += 1;
            own_woken();
            let pending = block_on(core::future::poll_fn(|cx| Poll::Ready(p.poll_room(cx).is_pending())));
            assert!(pending, "the only Room is out");
            assert!(in_block_on() && !block_on_spinning(), "the outer block_on still sleeps between polls");
            p.shared().storage.waker.notify_after_seqcst();
            assert!(own_woken(), "the wait kept no waker, and the outer block_on would sleep on");
            Poll::Ready(())
        }));
        assert_eq!(polls, 1);
        drop(room);
    }

    /// An owner whose `as_ref` panics is dropped and its slot aborted, keeping
    /// the Pipeline usable.
    #[test]
    fn an_owner_whose_as_ref_panics_is_dropped() {
        struct Owner {
            _held: Arc<()>,
        }
        impl AsRef<[u8]> for Owner {
            fn as_ref(&self) -> &[u8] {
                panic!("an owner that cannot give its bytes")
            }
        }
        let held = Arc::new(());
        let mut p = Pipeline::<u32, Owner>::with_owners(Mode::Plain, Limits { messages: 1, rooms: 0 });
        let message = block_on(p.message(7, None));
        let owner = Owner { _held: Arc::clone(&held) };
        let refused = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| p.owned(message, owner)));
        assert!(refused.is_err());
        assert_eq!(Arc::strong_count(&held), 1, "the owner was not dropped");
        drop(p);
        assert_eq!(Arc::strong_count(&held), 1);
    }

    #[test]
    fn a_block_on_that_took_no_wake_leaves_the_outer_one_none() {
        // The outer block_on's poll runs one that is ready at once: it
        // took no wake, and notes none. (It noted one whenever it ended
        // inside another, and an outer future that runs a block_on in
        // each poll was then polled without end and never slept.)
        let mut polls = 0;
        block_on(core::future::poll_fn(|cx| {
            if block_on_spinning() {
                return Poll::Pending;
            }
            polls += 1;
            block_on(async {});
            assert!(in_block_on() && !block_on_spinning(), "the outer block_on still sleeps between polls");
            if polls == 1 {
                assert!(!BLOCK_ON_PARK.with(|park| park.woken.load(Ordering::SeqCst)), "a block_on that took no wake noted one");
                // This future's one wake: its next poll is its last.
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            Poll::Ready(())
        }));
        assert_eq!(polls, 2);
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
            // Each case aborts on purpose: keep the kernel's crash
            // handler (Ubuntu's apport: 1 s of D-Bus timeout a child,
            // serialized across shards) from inspecting a deliberate
            // SIGABRT under the mutation profile's 15 s clock.
            #[cfg(target_os = "linux")]
            {
                // Sound: PR_SET_DUMPABLE(0) affects only this child process.
                let rc = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0 as libc::c_ulong, 0 as libc::c_ulong, 0 as libc::c_ulong, 0 as libc::c_ulong) };
                assert_eq!(rc, 0, "prctl(PR_SET_DUMPABLE, 0) failed");
            }
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
                "worker_start" => {
                    crate::lanes::fail_worker_start_for_test();
                }
                // A waker that panics ends the process, on the owner's
                // thread too: an abort of a short message frees its room
                // and wakes the wait before the call has returned.
                "waker_panics" => {
                    struct Panics;
                    impl std::task::Wake for Panics {
                        fn wake(self: Arc<Self>) {
                            panic!("this waker panics");
                        }
                    }
                    let mut p = Pipeline::<u32>::new(Mode::Plain, Limits { messages: 2, rooms: 1 });
                    let mut message = block_on(p.message(1, None));
                    let mut room = block_on(p.room());
                    p.commit(&mut message, &mut room, 3);
                    drop(room);
                    let _other = message_in_flight(&mut p);
                    let waker = Waker::from(Arc::new(Panics));
                    assert!(p.poll_ready(&mut Context::from_waker(&waker)).is_pending());
                    let _ = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| p.abort(message)));
                    // Not reached: the panic did not unwind into the catch.
                    std::process::exit(0);
                }
                // The stop takes no lock: this thread holds stderr's and
                // waits for the one that stops.
                "lent_stderr_locked" => {
                    let _held = std::io::stderr().lock();
                    let breach = || {
                        let completed: &'static AtomicU64 = Box::leak(Box::new(AtomicU64::new(0)));
                        drop(Lent { completed, epoch: 1, _bytes: PhantomData });
                    };
                    std::thread::spawn(breach).join().unwrap();
                }
                // The stop does not wait for its words: stderr is a full
                // pipe here, and a write to it would never return.
                "worker_start_stderr_full" => {
                    crate::lanes::fill_stderr_for_test();
                    crate::lanes::fail_worker_start_for_test();
                }
                _ => unreachable!(),
            }
            std::process::exit(0);
        }

        for (case, needle) in [
            ("lent", "Lent guard was dropped before its message finished"),
            ("lent_stderr_locked", "Lent guard was dropped before its message finished"),
            ("waker_panics", "this waker panics"),
            ("pipeline", "Pipeline dropped while worker tasks are still in flight"),
            ("pipeline_multi", "Pipeline dropped while worker tasks are still in flight"),
            ("abort_on_unwind", ""),
            ("worker_start", "BLAKE3: start a worker thread"),
            ("worker_start_stderr_full", ""),
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

    /// A stop says why where a person reads it: on a terminal. A stop
    /// writes only where `poll` says descriptor 2 takes words, and what
    /// `poll` says of a terminal is each system's own (the manual of
    /// macOS says its `poll` does not support devices).
    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn a_stop_says_why_on_a_terminal() {
        use std::io::{Read as _, Write as _};
        use std::os::fd::FromRawFd as _;
        use std::os::unix::fs::OpenOptionsExt as _;
        use std::os::unix::process::ExitStatusExt as _;
        unsafe extern "C" {
            fn posix_openpt(flags: i32) -> i32;
            fn grantpt(fd: i32) -> i32;
            fn unlockpt(fd: i32) -> i32;
            fn ptsname(fd: i32) -> *const core::ffi::c_char;
        }
        const O_RDWR: i32 = 2;
        #[cfg(target_os = "linux")]
        const O_NOCTTY: i32 = 0o400;
        #[cfg(target_os = "macos")]
        const O_NOCTTY: i32 = 0x20000;
        // Sound: the descriptor posix_openpt returns is this function's
        // own, and ptsname's string is copied before any call that could
        // write over it.
        let (mut ours, name) = unsafe {
            let fd = posix_openpt(O_RDWR | O_NOCTTY);
            assert!(fd >= 0, "this test needs a pseudo-terminal: {}", std::io::Error::last_os_error());
            assert_eq!((grantpt(fd), unlockpt(fd)), (0, 0), "{}", std::io::Error::last_os_error());
            let name = ptsname(fd);
            assert!(!name.is_null(), "{}", std::io::Error::last_os_error());
            (std::fs::File::from_raw_fd(fd), core::ffi::CStr::from_ptr(name).to_str().unwrap().to_owned())
        };
        // The terminal as a program has it: the child's descriptor 2. It
        // stays open here until the words are read: a terminal that
        // nobody has open any more may drop what was written to it.
        let mut terminal = std::fs::OpenOptions::new().read(true).write(true).custom_flags(O_NOCTTY).open(&name).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "pipeline::test::active_loan_drop_and_active_pipeline_drop_abort_the_process", "--nocapture"])
            .env("BLAKE3_PIPELINE_ABORT_CASE", "worker_start")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(terminal.try_clone().unwrap())
            .status()
            .unwrap();
        assert_eq!(status.signal(), Some(6), "{status:?}");
        // A mark after whatever the child said, so that the reading ends.
        terminal.write_all(b"\nTHE END\n").unwrap();
        let mut said = Vec::new();
        let mut some = [0u8; 256];
        while !said.windows(7).any(|bytes| bytes == b"THE END") {
            let count = ours.read(&mut some).unwrap();
            assert_ne!(count, 0, "the terminal closed");
            said.extend_from_slice(&some[..count]);
        }
        let said = String::from_utf8_lossy(&said);
        assert!(said.contains("BLAKE3: start a worker thread"), "the terminal got {said:?}");
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

        let tp = Arc::new(ThreadPark { thread: std::thread::current(), woken: AtomicBool::new(false) });
        std::thread::park_timeout(std::time::Duration::ZERO);
        std::task::Wake::wake_by_ref(&tp);
        assert!(tp.woken.swap(false, Ordering::SeqCst), "a wake is remembered");
        let t0 = std::time::Instant::now();
        std::thread::park_timeout(std::time::Duration::from_millis(200));
        assert!(t0.elapsed() < std::time::Duration::from_millis(150));

        std::thread::park_timeout(std::time::Duration::ZERO);
        std::task::Wake::wake(Arc::clone(&tp));
        assert!(tp.woken.load(Ordering::SeqCst), "a wake is remembered");
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
            p_ab.room_streams[0] = RoomStream { len: ROOM_LEN as u64, head_piece: pnode, tail_piece: pnode, stack_len: 0, merged_pieces: 0 };
            p_ab.slots[0] = SlotMeta { tag: Some(77), kind: SlotKind::WaitAborted { group: 0, epoch: 5 }, mode: SlotMode::Default };
            p_ab.free_slots.pop_front();
            p_ab.end_queue.push_back(0);
            assert!(p_ab.try_reap_one().is_none());
            p_ab.shared().pieces[pnode as usize].done.store(true, Ordering::SeqCst);
            assert!(p_ab.try_reap_one().is_none());
            p_ab.shared().storage.groups[0].completed_epoch.store(5, Ordering::SeqCst);
            assert!(matches!(p_ab.try_reap_one(), Some(Event::Aborted { tag: 77, owner: None })));
        }
    }

    #[test]
    fn per_message_modes_match_reference_across_all_paths() {
        let key_a = [0x11u8; 32];
        let key_b = [0x22u8; 32];
        let ctx_a = "pipeline per-message mode test A";
        let ctx_b = "pipeline per-message mode test B";
        let dk_a = [0x33u8; 32];
        let dk_b = [0x44u8; 32];

        let ref_hash = |mode: Mode, data: &[u8]| -> [u8; 32] {
            let mut h = blake3_servil_core::convenience::Hasher::new(mode, None);
            h.update(data);
            *h.finalize().as_bytes()
        };

        let collect_hashes = |pipe: &mut Pipeline<usize, Vec<u8>>, n: usize| -> Vec<(usize, [u8; 32])> {
            drain_all(pipe, n)
                .into_iter()
                .map(|ev| match ev {
                    Event::Hashed { tag, hash, .. } => (tag, *hash.as_bytes()),
                    _ => panic!("expected Event::Hashed"),
                })
                .collect()
        };

        let mut pipe = Pipeline::<usize, Vec<u8>>::with_owners(Mode::Plain, Limits { messages: 16, rooms: 4 });
        let mut expected: Vec<(usize, [u8; 32])> = Vec::new();
        let mut tag = 0usize;

        // 1. Empty messages across all four Mode variants (including default `message()`).
        let modes =
            [None, Some(Mode::Plain), Some(Mode::Keyed(&key_a)), Some(Mode::Keyed(&key_b)), Some(Mode::DeriveKey(ctx_a)), Some(Mode::DeriveKeyWith(&dk_b))];
        for om in modes {
            let m = match om {
                None => block_on(pipe.message(tag, None)),
                Some(mode) => block_on(pipe.message_with(mode, tag, None)),
            };
            pipe.end(m);
            expected.push((tag, ref_hash(om.unwrap_or(Mode::Plain), &[])));
            tag += 1;
        }
        assert_eq!(collect_hashes(&mut pipe, expected.len()), expected);
        expected.clear();

        // 2. Short owned messages: alternating modes AND consecutive same-mode (tests ModeKey batching + flush).
        let short_cases: &[(Option<Mode>, usize)] = &[
            (None, 32),
            (Some(Mode::Plain), 64),
            (Some(Mode::Keyed(&key_a)), 48),
            (Some(Mode::Keyed(&key_a)), 80),
            (Some(Mode::Keyed(&key_b)), 48),
            (Some(Mode::DeriveKeyWith(&dk_a)), 96),
            (Some(Mode::DeriveKeyWith(&dk_a)), 128),
            (Some(Mode::DeriveKey(ctx_b)), 64),
            (Some(Mode::DeriveKey(ctx_b)), 64),
            (None, 1024),
        ];
        for (i, &(om, len)) in short_cases.iter().enumerate() {
            let data: Vec<u8> = (0..len).map(|j| ((i * 31 + j * 7) & 0xff) as u8).collect();
            let exp = ref_hash(om.unwrap_or(Mode::Plain), &data);
            let m = match om {
                None => block_on(pipe.message(tag, Some(len as u64))),
                Some(mode) => block_on(pipe.message_with(mode, tag, Some(len as u64))),
            };
            pipe.owned(m, data);
            expected.push((tag, exp));
            tag += 1;
        }
        assert_eq!(collect_hashes(&mut pipe, expected.len()), expected);
        expected.clear();

        // 3. Multi-piece owned, lent, and streaming room messages with distinct per-message modes.
        let long_len = 2 * ROOM_LEN + 137;
        let owned_long: Vec<u8> = (0..long_len).map(|j| ((j * 13 + 3) & 0xff) as u8).collect();
        let lent_long: Vec<u8> = (0..long_len).map(|j| ((j * 17 + 9) & 0xff) as u8).collect();
        let stream_long: Vec<u8> = (0..long_len).map(|j| ((j * 23 + 5) & 0xff) as u8).collect();
        let stream_short: Vec<u8> = (0..120).map(|j| ((j * 29 + 1) & 0xff) as u8).collect();

        let m_owned = block_on(pipe.message_with(Mode::Keyed(&key_a), tag, Some(long_len as u64)));
        expected.push((tag, ref_hash(Mode::Keyed(&key_a), &owned_long)));
        pipe.owned(m_owned, owned_long);
        tag += 1;

        let m_lent = block_on(pipe.message_with(Mode::DeriveKeyWith(&dk_b), tag, Some(long_len as u64)));
        expected.push((tag, ref_hash(Mode::DeriveKeyWith(&dk_b), &lent_long)));
        let guard = unsafe { pipe.lent(m_lent, &lent_long) };
        tag += 1;

        let mut m_stream = block_on(pipe.message_with(Mode::DeriveKey(ctx_a), tag, Some(long_len as u64)));
        expected.push((tag, ref_hash(Mode::DeriveKey(ctx_a), &stream_long)));
        for chunk in stream_long.chunks(ROOM_LEN) {
            let mut r = block_on(pipe.room());
            r.bytes_mut()[..chunk.len()].copy_from_slice(chunk);
            pipe.commit(&mut m_stream, &mut r, chunk.len());
        }
        pipe.end(m_stream);
        tag += 1;

        let mut m_short_room = block_on(pipe.message_with(Mode::Keyed(&key_b), tag, None));
        expected.push((tag, ref_hash(Mode::Keyed(&key_b), &stream_short)));
        {
            let mut r = block_on(pipe.room());
            r.bytes_mut()[..stream_short.len()].copy_from_slice(&stream_short);
            pipe.commit(&mut m_short_room, &mut r, stream_short.len());
        }
        pipe.end(m_short_room);
        tag += 1;

        let mut got = Vec::new();
        while got.len() < expected.len() {
            if let Some(ev) = pipe.next_event() {
                if let Event::Hashed { tag: t, hash, .. } = ev {
                    got.push((t, *hash.as_bytes()));
                }
            } else {
                block_on(pipe.ready());
            }
        }
        drop(guard);
        assert_eq!(got, expected);

        // 4. Short and multi-piece batches with per-message modes.
        for &(mode, count, msg_len) in &[(Mode::Keyed(&key_a), 8usize, 64usize), (Mode::DeriveKeyWith(&dk_b), 2048usize, 64usize)] {
            let mut b = Batch::new(msg_len, count);
            for i in 0..count {
                b.message_mut(i).fill(((i * 5 + 1) & 0xff) as u8);
            }
            let digests = vec![[0u8; 32]; count].into_boxed_slice();
            let bm = block_on(pipe.message_with(mode, tag, None));
            pipe.batch(bm, b, digests);
            tag += 1;
            let ev = loop {
                if let Some(ev) = pipe.next_event() {
                    break ev;
                }
                block_on(pipe.ready());
            };
            let Event::Batch { mut batch, digests, .. } = ev else {
                panic!("expected Event::Batch");
            };
            for i in 0..count {
                assert_eq!(digests[i], ref_hash(mode, batch.message_mut(i)));
            }
        }

        // 5. Key-independent group dispatch: identical vs distinct secret keys
        //    must publish the exact same number of MemberGroups.
        let mut p_same = Pipeline::<usize, Vec<u8>>::with_owners(Mode::Plain, Limits { messages: 8, rooms: 0 });
        let e0 = p_same.next_group_epoch;
        for i in 0..2 {
            let m = block_on(p_same.message_with(Mode::Keyed(&key_a), i, None));
            p_same.owned(m, vec![i as u8; 64]);
            assert!(p_same.open_group.is_none(), "Custom message must publish immediately");
        }
        let same_groups = p_same.next_group_epoch - e0;
        let _ = drain_all(&mut p_same, 2);

        let mut p_diff = Pipeline::<usize, Vec<u8>>::with_owners(Mode::Plain, Limits { messages: 8, rooms: 0 });
        let e1 = p_diff.next_group_epoch;
        let m0 = block_on(p_diff.message_with(Mode::Keyed(&key_a), 0, None));
        p_diff.owned(m0, vec![0u8; 64]);
        let m1 = block_on(p_diff.message_with(Mode::Keyed(&key_b), 1, None));
        p_diff.owned(m1, vec![1u8; 64]);
        let diff_groups = p_diff.next_group_epoch - e1;
        let _ = drain_all(&mut p_diff, 2);
        assert_eq!(same_groups, diff_groups, "keyed group dispatch must not depend on secret key equality");
        assert_eq!(same_groups, 2);

        // 6. RPIT lifetime independence (`+ use<'p, T, B>`) and unpolled drop.
        fn open_with_local_ctx<'p>(p: &'p mut Pipeline<usize, Vec<u8>>, tag: usize) -> impl Future<Output = Message> + use<'p> {
            let local_ctx = String::from("ephemeral local derive context");
            p.message_with(Mode::DeriveKey(&local_ctx), tag, None)
        }
        // Drop an unpolled message_with future without affecting pipeline state.
        let _unpolled = pipe.message_with(Mode::Keyed(&key_a), 9999, None);
        drop(_unpolled);
        let m_local = block_on(open_with_local_ctx(&mut pipe, tag));
        pipe.owned(m_local, vec![0xAB; 64]);
        let got_local = collect_hashes(&mut pipe, 1);
        assert_eq!(got_local, vec![(tag, ref_hash(Mode::DeriveKey("ephemeral local derive context"), &[0xAB; 64]))]);

        // 7. Keyed-default Pipeline: adjacent same-length Default -> Plain and
        //    Default -> Custom transitions must flush open groups (killing both
        //    `|| -> &&` mutants in `push_member`), plus room-pressure flush.
        let mut p_keyed = Pipeline::<usize, Vec<u8>>::with_owners(Mode::Keyed(&key_a), Limits { messages: 8, rooms: 1 });
        let md0 = block_on(p_keyed.message(0, None));
        p_keyed.owned(md0, vec![0x11; 64]);
        let mp1 = block_on(p_keyed.message_with(Mode::Plain, 1, None));
        p_keyed.owned(mp1, vec![0x22; 64]);
        let mc2 = block_on(p_keyed.message_with(Mode::Keyed(&key_b), 2, None));
        p_keyed.owned(mc2, vec![0x33; 64]);
        let got_k = collect_hashes(&mut p_keyed, 3);
        assert_eq!(got_k[0], (0, ref_hash(Mode::Keyed(&key_a), &[0x11; 64])));
        assert_eq!(got_k[1], (1, ref_hash(Mode::Plain, &[0x22; 64])));
        assert_eq!(got_k[2], (2, ref_hash(Mode::Keyed(&key_b), &[0x33; 64])));

        // Room-pressure flush on Keyed-default Pipeline with explicit Plain room.
        let mut mr0 = block_on(p_keyed.message(10, None));
        let mut mr1 = block_on(p_keyed.message_with(Mode::Plain, 11, None));
        {
            let mut r = block_on(p_keyed.room());
            r.bytes_mut()[..64].fill(0x44);
            p_keyed.commit(&mut mr0, &mut r, 64);
            r.bytes_mut()[..64].fill(0x55);
            p_keyed.commit(&mut mr1, &mut r, 64);
        }
        // Re-acquiring the only room forces flush_sealed_short_rooms before end().
        let r2 = block_on(p_keyed.room());
        drop(r2);
        p_keyed.end(mr1);
        p_keyed.end(mr0);
        let got_r = collect_hashes(&mut p_keyed, 2);
        assert_eq!(got_r[0], (11, ref_hash(Mode::Plain, &[0x55; 64])));
        assert_eq!(got_r[1], (10, ref_hash(Mode::Keyed(&key_a), &[0x44; 64])));

        // 8. Dropping a Pending MessageWithFut must clear the registered Waker.
        struct NoopWake;
        impl std::task::Wake for NoopWake {
            fn wake(self: Arc<Self>) {}
        }
        let mut p1 = Pipeline::<usize, Vec<u8>>::with_owners(Mode::Plain, Limits { messages: 1, rooms: 0 });
        let cur_epoch = p1.shared().storage.groups[0].completed_epoch.load(Ordering::SeqCst);
        let pending_epoch = cur_epoch + 1;
        p1.slots[0] = SlotMeta { tag: Some(0), kind: SlotKind::WaitAborted { group: 0, epoch: pending_epoch }, mode: SlotMode::Default };
        p1.free_slots.pop_front();
        p1.end_queue.push_back(0);
        let wake_arc = Arc::new(NoopWake);
        {
            let waker = Waker::from(Arc::clone(&wake_arc));
            let mut cx = Context::from_waker(&waker);
            let mut fut = Box::pin(p1.message_with(Mode::Keyed(&key_a), 1, None));
            assert!(fut.as_mut().poll(&mut cx).is_pending());
            drop(waker);
            assert_eq!(Arc::strong_count(&wake_arc), 2, "Pending poll must register waker");
            drop(fut);
        }
        assert_eq!(Arc::strong_count(&wake_arc), 1, "Dropping Pending MessageWithFut must clear waker");
        p1.shared().storage.groups[0].completed_epoch.store(pending_epoch, Ordering::SeqCst);
        assert!(matches!(p1.try_reap_one(), Some(Event::Aborted { tag: 0, owner: None })));
    }

    #[test]
    fn packed_batches_in_pipeline_match_reference() {
        use crate::throughput::Packed;
        let key = [0x66u8; 32];
        let fill_msg = |dst: &mut [u8], tag: usize, idx: usize| {
            // Encode full u32 index + position-dependent bytes so no period-256
            // aliasing hides offset bugs across multi-piece batch tasks.
            let idx_bytes = (idx as u32).to_le_bytes();
            let tag_byte = (tag as u8).wrapping_mul(31).wrapping_add(7);
            for (j, b) in dst.iter_mut().enumerate() {
                *b = if j < 4 {
                    idx_bytes[j] ^ tag_byte
                } else {
                    ((idx.wrapping_mul(17).wrapping_add(j.wrapping_mul(13)).wrapping_add(tag.wrapping_mul(5))) & 0xff) as u8
                };
            }
        };

        // 1. Packed<32> (inferred without turbofish!): empty, small (MemberGroup),
        //    multi-piece BLINCS batch (6,144 msgs -> 6 pieces), and 1,025-msg tail
        //    checking per-piece sme2 flag (piece 0: 1,024 msgs >= SME2_MIN_LEN,
        //    piece 1: 1 msg < SME2_MIN_LEN).
        let mut p32_pipe = Pipeline::with_batches(Mode::Plain, Limits { messages: 4, rooms: 0 });
        for &(tag, count, mode) in &[(0usize, 0usize, Mode::Plain), (1, 37, Mode::Plain), (2, 6144, Mode::Keyed(&key)), (3, 1025, Mode::Plain)] {
            let mut batch = Packed::<32>::new(count);
            for i in 0..count {
                fill_msg(&mut batch[i], tag, i);
            }
            let digests = vec![[0u8; 32]; count].into_boxed_slice();
            let m = block_on(p32_pipe.message_with(mode, tag, None));
            let slot_idx = m.slot as usize;
            p32_pipe.batch(m, batch, digests);
            let got_pieces = if count >= 1024 { unsafe { (*p32_pipe.shared().multi[slot_idx].get()).pieces } } else { 0 };
            let tail_sme2 = if count == 1025 { Some((p32_pipe.action_buf[0] & ACTION_SME2 != 0, p32_pipe.action_buf[1] & ACTION_SME2 != 0)) } else { None };
            let ev = loop {
                if let Some(ev) = p32_pipe.next_event() {
                    break ev;
                }
                block_on(p32_pipe.ready());
            };
            if count == 6144 {
                // Verify 6,144 x 32 B split into 6 compression-work pieces (1,024 msgs each).
                assert_eq!(got_pieces, 6);
            }
            if let Some((p0_sme2, p1_sme2)) = tail_sme2 {
                assert_eq!(got_pieces, 2);
                assert_eq!(p0_sme2, 1024 * crate::BLOCK_LEN >= SME2_MIN_LEN);
                assert!(!p1_sme2, "1-message tail piece must not claim Sme2Turn");
            }
            let Event::Batch { tag: got_tag, batch, digests } = ev else {
                panic!("expected Event::Batch");
            };
            assert_eq!(got_tag, tag);
            for i in 0..count {
                let want = *blake3_servil_core::convenience::hash_with(mode, &batch[i]).as_bytes();
                assert_eq!(digests[i], want, "Packed<32> tag={tag} msg={i}");
            }
        }

        // 2. Compression-work flushing in push_member: 1,023 x 32 B (32,736 raw B,
        //    65,472 work B < 65,536) stays in open_group, then +1 x 32 B pushes
        //    work bytes to 65,536 == TASK_LEN and flushes open_group immediately
        //    (raw bytes would only be 32,768 < TASK_LEN).
        {
            let mut flush_pipe = Pipeline::with_batches(Mode::Plain, Limits { messages: 4, rooms: 0 });
            let mut b1 = Packed::<32>::new(1023);
            for i in 0..1023 {
                fill_msg(&mut b1[i], 10, i);
            }
            let m1 = block_on(flush_pipe.message(10usize, None));
            flush_pipe.batch(m1, b1, vec![[0u8; 32]; 1023].into_boxed_slice());
            assert!(flush_pipe.open_group.is_some(), "1023 x 32 B (< TASK_LEN work) stays buffered");
            let mut b2 = Packed::<32>::new(1);
            fill_msg(&mut b2[0], 11, 0);
            let m2 = block_on(flush_pipe.message(11usize, None));
            flush_pipe.batch(m2, b2, vec![[0u8; 32]; 1].into_boxed_slice());
            assert!(flush_pipe.open_group.is_none(), "1023 + 1 x 32 B reaches TASK_LEN compression work and flushes");
            let events = drain_all(&mut flush_pipe, 2);
            for ev in events {
                let Event::Batch { batch, digests, .. } = ev else { panic!("expected Event::Batch") };
                for i in 0..batch.len() {
                    assert_eq!(digests[i], *blake3_servil_core::convenience::hash(&batch[i]).as_bytes());
                }
            }
        }

        // 3. Packed<16> and Packed<48> (small + multi-piece) and owned Vec<[u8; 32]>.
        let mut p16_pipe = Pipeline::<usize, (), Packed<16>>::with_batches(Mode::Plain, Limits { messages: 2, rooms: 0 });
        for &count in &[17usize, 2051] {
            let mut batch = Packed::<16>::new(count);
            for i in 0..count {
                fill_msg(&mut batch[i], 4, i);
            }
            let m = block_on(p16_pipe.message(count, None));
            p16_pipe.batch(m, batch, vec![[0u8; 32]; count].into_boxed_slice());
            let Event::Batch { batch, digests, .. } = drain_all(&mut p16_pipe, 1).pop().unwrap() else {
                panic!("expected Event::Batch");
            };
            for i in 0..count {
                assert_eq!(digests[i], *blake3_servil_core::convenience::hash(&batch[i]).as_bytes());
            }
        }

        let mut p48_pipe = Pipeline::<usize, (), Packed<48>>::with_batches(Mode::Keyed(&key), Limits { messages: 2, rooms: 0 });
        for &count in &[19usize, 1500] {
            let mut batch = Packed::<48>::new(count);
            for i in 0..count {
                fill_msg(&mut batch[i], 5, i);
            }
            let m = block_on(p48_pipe.message(count, None));
            p48_pipe.batch(m, batch, vec![[0u8; 32]; count].into_boxed_slice());
            let Event::Batch { batch, digests, .. } = drain_all(&mut p48_pipe, 1).pop().unwrap() else {
                panic!("expected Event::Batch");
            };
            for i in 0..count {
                assert_eq!(digests[i], *blake3_servil_core::convenience::hash_with(Mode::Keyed(&key), &batch[i]).as_bytes());
            }
        }

        let mut vec_pipe = Pipeline::<usize, Vec<u8>, Vec<[u8; 32]>>::with_owners_and_batches(Mode::Plain, Limits { messages: 2, rooms: 0 });
        let vec_batch: Vec<[u8; 32]> = (0..1200)
            .map(|i| {
                let mut msg = [0u8; 32];
                fill_msg(&mut msg, 6, i);
                msg
            })
            .collect();
        let m = block_on(vec_pipe.message(1, None));
        vec_pipe.batch(m, vec_batch, vec![[0u8; 32]; 1200].into_boxed_slice());
        let Event::Batch { batch, digests, .. } = drain_all(&mut vec_pipe, 1).pop().unwrap() else {
            panic!("expected Event::Batch");
        };
        for i in 0..1200 {
            assert_eq!(digests[i], *blake3_servil_core::convenience::hash(&batch[i]).as_bytes());
        }

        // Verify Pipeline::with_owners still infers P = Batch<'static> with no batch() call,
        // and Box<[[u8; 32]]> works through with_batches.
        let mut inferred_owners = Pipeline::with_owners(Mode::Plain, Limits { messages: 1, rooms: 0 });
        let mo = block_on(inferred_owners.message(9usize, None));
        inferred_owners.owned(mo, vec![0x42u8; 64]);
        let Event::Hashed { tag: 9, hash, .. } = drain_all(&mut inferred_owners, 1).pop().unwrap() else {
            panic!("expected Event::Hashed");
        };
        assert_eq!(hash, blake3_servil_core::convenience::hash(&[0x42u8; 64]));

        let mut box_pipe = Pipeline::with_batches(Mode::Plain, Limits { messages: 2, rooms: 0 });
        let boxed: Box<[[u8; 32]]> = (0..600)
            .map(|i| {
                let mut msg = [0u8; 32];
                fill_msg(&mut msg, 7, i);
                msg
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let mb = block_on(box_pipe.message(7usize, None));
        box_pipe.batch(mb, boxed, vec![[0u8; 32]; 600].into_boxed_slice());
        let Event::Batch { tag: 7, batch, digests } = drain_all(&mut box_pipe, 1).pop().unwrap() else {
            panic!("expected Event::Batch");
        };
        for i in 0..600 {
            assert_eq!(digests[i], *blake3_servil_core::convenience::hash(&batch[i]).as_bytes());
        }
    }

    /// Panicking inside an owner's `AsRef::as_ref` drops the owner in place,
    /// aborts the slot cleanly, keeps `open_messages` exact, and lets the
    /// single-slot Pipeline keep serving subsequent messages (#465).
    #[test]
    fn panicking_as_ref_aborts_slot_and_keeps_pipeline_usable() {
        struct MaybePanicOwner {
            bytes: [u8; 16],
            drops: Arc<AtomicUsize>,
            panic: bool,
        }
        impl AsRef<[u8]> for MaybePanicOwner {
            fn as_ref(&self) -> &[u8] {
                if self.panic {
                    panic!("deliberate as_ref panic");
                }
                &self.bytes
            }
        }
        impl Drop for MaybePanicOwner {
            fn drop(&mut self) {
                self.drops.fetch_add(1, Ordering::SeqCst);
            }
        }
        let drops = Arc::new(AtomicUsize::new(0));
        let mut p = Pipeline::<u32, MaybePanicOwner>::with_owners(Mode::Plain, Limits { messages: 1, rooms: 0 });
        let m1 = block_on(p.message(1, None));
        let res = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
            p.owned(m1, MaybePanicOwner { bytes: [0xaa; 16], drops: Arc::clone(&drops), panic: true });
        }));
        assert!(res.is_err(), "panicking as_ref must unwind");
        assert_eq!(drops.load(Ordering::SeqCst), 1, "owner must be dropped on as_ref unwind");
        assert_eq!(p.open_messages, 0, "open_messages must reflect aborted slot");
        block_on(p.ready());
        assert!(matches!(p.next_event(), Some(Event::Aborted { tag: 1, owner: None })), "aborted slot must yield Event::Aborted");

        // Slot must be back on free_slots so a subsequent message succeeds without hanging.
        let m2 = block_on(p.message(2, None));
        p.owned(m2, MaybePanicOwner { bytes: [0x55; 16], drops: Arc::clone(&drops), panic: false });
        let Event::Hashed { tag: 2, hash, owner } = drain_all(&mut p, 1).pop().unwrap() else {
            panic!("expected Event::Hashed");
        };
        assert_eq!(hash, blake3_servil_core::convenience::hash(&[0x55; 16]));
        drop(owner);
        assert_eq!(drops.load(Ordering::SeqCst), 2);
    }

    /// Rejected `owned`, `lent`, and `batch` calls (bytes given two ways or
    /// mismatched batch digest count) abort the slot and release any Room
    /// credit so a 1-message / 1-room Pipeline stays usable after `catch_unwind` (#465).
    #[test]
    fn rejected_owned_lent_and_batch_abort_slot_and_release_room() {
        let mut p = Pipeline::<u32, Vec<u8>>::with_owners(Mode::Plain, Limits { messages: 1, rooms: 1 });

        // 1. owned after Room commit releases room and aborts slot.
        let mut m1 = block_on(p.message(1, None));
        let mut r1 = block_on(p.room());
        r1.bytes_mut()[..8].copy_from_slice(b"12345678");
        p.commit(&mut m1, &mut r1, 8);
        drop(r1);
        let res = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
            p.owned(m1, vec![1, 2, 3]);
        }));
        assert!(res.is_err(), "owned after commit must panic");
        block_on(p.ready());
        assert!(matches!(p.next_event(), Some(Event::Aborted { tag: 1, owner: None })));

        // 2. lent after Room commit releases room and aborts slot.
        let mut m2 = block_on(p.message(2, None));
        let mut r2 = block_on(p.room());
        r2.bytes_mut()[..4].copy_from_slice(b"abcd");
        p.commit(&mut m2, &mut r2, 4);
        drop(r2);
        let res = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
            let _ = unsafe { p.lent(m2, b"efgh") };
        }));
        assert!(res.is_err(), "lent after commit must panic");
        block_on(p.ready());
        assert!(matches!(p.next_event(), Some(Event::Aborted { tag: 2, owner: None })));

        // 3. owned after whole-room OpenRooms commit releases room and aborts slot.
        let mut m3 = block_on(p.message(3, None));
        let mut r3 = block_on(p.room());
        r3.bytes_mut().fill(0xab);
        p.commit(&mut m3, &mut r3, ROOM_LEN);
        drop(r3);
        assert!(matches!(p.slots[m3.slot as usize].kind, SlotKind::OpenRooms { .. }));
        let res = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
            p.owned(m3, vec![9, 8, 7]);
        }));
        assert!(res.is_err(), "owned after OpenRooms commit must panic");
        block_on(p.ready());
        assert!(matches!(p.next_event(), Some(Event::Aborted { tag: 3, owner: None })));

        // 4. lent after OpenShortDispatched (flushed short room) aborts cleanly.
        let mut m4 = block_on(p.message(4, None));
        let mut r4 = block_on(p.room());
        r4.bytes_mut()[..12].copy_from_slice(b"short-in-flt");
        p.commit(&mut m4, &mut r4, 12);
        drop(r4);
        p.flush_sealed_short_rooms();
        assert!(matches!(p.slots[m4.slot as usize].kind, SlotKind::OpenShortDispatched { .. }));
        let res = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
            let _ = unsafe { p.lent(m4, b"extra") };
        }));
        assert!(res.is_err(), "lent after OpenShortDispatched must panic");
        block_on(p.ready());
        assert!(matches!(p.next_event(), Some(Event::Aborted { tag: 4, owner: None })));

        // 5. batch with bytes given two ways (after Room commit) aborts cleanly.
        let mut m5 = block_on(p.message(5, None));
        let mut r5 = block_on(p.room());
        r5.bytes_mut()[..4].copy_from_slice(b"room");
        p.commit(&mut m5, &mut r5, 4);
        drop(r5);
        let batch_two_ways = Batch::new(1, 64);
        let res = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
            p.batch(m5, batch_two_ways, vec![[0u8; 32]; 1].into_boxed_slice());
        }));
        assert!(res.is_err(), "batch after commit must panic");
        block_on(p.ready());
        assert!(matches!(p.next_event(), Some(Event::Aborted { tag: 5, owner: None })));

        // 6. batch with mismatched digest count aborts slot cleanly.
        let m6 = block_on(p.message(6, None));
        let bad_batch = Batch::new(2, 64);
        let res = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
            p.batch(m6, bad_batch, vec![[0u8; 32]; 1].into_boxed_slice());
        }));
        assert!(res.is_err(), "batch with wrong digest len must panic");
        block_on(p.ready());
        assert!(matches!(p.next_event(), Some(Event::Aborted { tag: 6, owner: None })));

        // 7. Both the single message slot and single room must now work end to end.
        let mut m7 = block_on(p.message(7, None));
        let mut r7 = block_on(p.room());
        r7.bytes_mut()[..5].copy_from_slice(b"hello");
        p.commit(&mut m7, &mut r7, 5);
        drop(r7);
        p.end(m7);
        let Event::Hashed { tag: 7, hash, .. } = drain_all(&mut p, 1).pop().unwrap() else {
            panic!("expected Event::Hashed");
        };
        assert_eq!(hash, blake3_servil_core::convenience::hash(b"hello"));
    }

    /// Forked children entering `acquire_free_group` or `alloc_piece_node`
    /// while waiting on work must stop via `lanes::require()` instead of
    /// spinning forever (#465).
    #[cfg(unix)]
    #[test]
    fn forked_child_stops_in_acquire_free_group_and_alloc_piece_node() {
        const ENV: &str = "BLAKE3_PIPELINE_FORK_WAIT_CASE";
        if let Ok(which) = std::env::var(ENV) {
            crate::initialize_multithreaded(core::num::NonZeroUsize::new(2));
            let mut p = Pipeline::<u32>::new(Mode::Plain, Limits { messages: 2, rooms: 1 });
            // Mark every group / piece unavailable before simulating a forked child.
            if which == "group" {
                for slot in &mut p.assigned_group_epoch {
                    *slot = u64::MAX;
                }
            } else {
                while p.free_pieces.pop_front().is_some() {}
            }
            // Sound: test runs in an isolated child process and triggers the
            // fork flag so `acquire_free_group`/`alloc_piece_node` hits `require()`.
            unsafe {
                let pid = libc::fork();
                assert!(pid >= 0, "fork failed");
                if pid == 0 {
                    let res = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
                        if which == "group" {
                            let _ = p.acquire_free_group();
                        } else {
                            let _ = p.alloc_piece_node(0);
                        }
                    }));
                    libc::_exit(if res.is_err() { 0 } else { 1 });
                }
                let mut status: libc::c_int = 0;
                assert_eq!(libc::waitpid(pid, &mut status, 0), pid);
                // Clear synthetic busy state before parent drops `p`.
                for slot in &mut p.assigned_group_epoch {
                    *slot = 0;
                }
                assert!(libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0, "child failed: status={status}");
            }
            return;
        }
        let exe = std::env::current_exe().unwrap();
        for case in ["group", "piece"] {
            let out = std::process::Command::new(&exe)
                .env(ENV, case)
                .args(["--exact", "pipeline::test::forked_child_stops_in_acquire_free_group_and_alloc_piece_node", "--nocapture"])
                .output()
                .unwrap();
            let stdout = std::string::String::from_utf8_lossy(&out.stdout);
            assert!(out.status.success() && stdout.contains("1 passed"), "fork wait case {case} failed: status={:?}\n{stdout}", out.status);
        }
    }

    #[cfg(not(shuttle))]
    #[test]
    fn batch_plans_and_non_batch_dispatch_locality_contract() {
        crate::initialize();
        let mut pipe = Pipeline::<u32, Vec<u8>>::with_owners(Mode::Plain, Limits { messages: 4, rooms: 2 });
        let drain_one = |pipe: &mut Pipeline<u32, Vec<u8>>| loop {
            if let Some(ev) = pipe.next_event() {
                return ev;
            }
            block_on(pipe.ready());
        };
        let is_registered = pipe.source_slot.is_some();

        // Split batch (>= TASK_LEN = 64 KiB) records a BatchPlan when
        // registered, decrements batches_in_flight on reap, and produces
        // exact digests.
        let split_count = TASK_LEN / 64; // 1,024 x 64 B = 64 KiB
        let mut batch = Batch::new(64, split_count);
        for i in 0..split_count {
            batch.message_mut(i).fill((i & 0xff) as u8);
        }
        let msg = block_on(pipe.message(1, None));
        pipe.batch(msg, batch, vec![[0u8; OUT_LEN]; split_count].into_boxed_slice());
        let recorded_in_flight = pipe.batches_in_flight;
        let ev = drain_one(&mut pipe);
        assert_eq!(recorded_in_flight, usize::from(is_registered));
        assert_eq!(pipe.batches_in_flight, 0);
        let Event::Batch { batch, digests, .. } = ev else { panic!("expected Batch event") };
        assert_eq!(digests[0], *crate::convenience::hash(&batch.bytes()[..64]).as_bytes());

        // Short group (KIND_GROUP), full room piece (KIND_ROOM_PIECE), and
        // split non-batch owned message (128 KiB) never set batch_plans and
        // never mark their queued actions near.
        let mut m_short = block_on(pipe.message(2, Some(64)));
        let mut room = block_on(pipe.room());
        room.bytes_mut()[..64].fill(0x33);
        pipe.commit(&mut m_short, &mut room, 64);
        drop(room);
        pipe.end(m_short);
        let short_front_near = pipe.shared().storage.source.front_action_near();
        let _ = drain_one(&mut pipe);
        assert!(!short_front_near);
        assert_eq!(pipe.batches_in_flight, 0);

        let mut m_room = block_on(pipe.message(3, Some((ROOM_LEN * 2) as u64)));
        let mut r1 = block_on(pipe.room());
        r1.bytes_mut().fill(0x44);
        pipe.commit(&mut m_room, &mut r1, ROOM_LEN);
        drop(r1);
        let room_front_near = pipe.shared().storage.source.front_action_near();
        let mut r2 = block_on(pipe.room());
        r2.bytes_mut().fill(0x55);
        pipe.commit(&mut m_room, &mut r2, ROOM_LEN);
        drop(r2);
        pipe.end(m_room);
        let _ = drain_one(&mut pipe);
        assert!(!room_front_near);
        assert_eq!(pipe.batches_in_flight, 0);

        let m_owned = block_on(pipe.message(4, Some((TASK_LEN * 2) as u64)));
        pipe.owned(m_owned, vec![0x66u8; TASK_LEN * 2]);
        let owned_front_near = pipe.shared().storage.source.front_action_near();
        let _ = drain_one(&mut pipe);
        assert!(!owned_front_near);
        assert_eq!(pipe.batches_in_flight, 0);
    }
}
