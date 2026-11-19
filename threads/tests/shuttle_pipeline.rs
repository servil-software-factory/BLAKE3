//! The Pipeline (src/pipeline.rs) under Shuttle. This file compiles the
//! library's own pipeline.rs and waker.rs beside Shuttle's primitives and
//! a stand-in for the pool, so what is checked is the code a program
//! runs; a copy would drift. Run it so:
//!
//!     RUSTFLAGS="--cfg shuttle" cargo test --release --test shuttle_pipeline
//!
//! Without the cfg it compiles to nothing.
//!
//! What is real: every line of the Pipeline, its Rooms and credits, its
//! groups, pieces and slots, its storage and that storage's reuse, the
//! waker slot, `block_on`, and the hashing itself.
//!
//! What stands in: the pool. The Pipeline hands the pool actions through
//! `Source` (publish_actions) and runs waiting ones itself
//! (run_waiting_action). Here a Source is a queue under a lock, and the
//! model's worker threads take actions from every registered Source and
//! run them, as the pool's threads do. `Task::run_members` hashes each
//! member alone, where the pool's picks a side-by-side kernel for some
//! lengths: the same digests.
//!
//! How a fault shows: every atomic operation, lock and cell access is a
//! point where Shuttle may switch threads. A wake lost between a worker's
//! finishing and the caller's sleeping leaves every thread blocked, which
//! Shuttle reports as a deadlock. A result read before its worker wrote
//! it, a Room's bytes written while a worker reads them, or a group
//! reused while in flight gives a digest that is not the message's, and
//! each scenario compares every digest with the one-thread library's. At
//! the end of each run every storage made in it must be free again.
//!
//! tools/pipeline_model_controls.py plants such faults in pipeline.rs one
//! at a time and requires this model to fail for each. Three things here
//! are there because a planted fault passed without them: the workers
//! sleep when idle (Shuttle's park may wake by itself, so a caller no one
//! wakes is a deadlock only if no thread can run); a worker is sometimes
//! held back with its action in hand (hold_back); and both schedulers run.
//!
//! What it cannot see: Shuttle treats every atomic operation as SeqCst,
//! so a fault that needs a weaker ordering to show is outside it. It
//! checks the schedules it tries, not all. The pool itself is not here:
//! its handshakes have their own models (shuttle_finish, shuttle_park).
//! Two things the real pool does and this stand-in does not (BLAKE3
//! #445, from the 0.10.0 review): it sends actions marked for SME2 to
//! the thread that holds that unit (here every worker runs every action,
//! the mark dropped), and a caller spins for 200 microseconds before it
//! sleeps (here it sleeps at once: Shuttle has no clock, and the sleep is
//! where a lost wake shows).
#![cfg(shuttle)]
#![allow(dead_code, unused_imports)]

/// Shuttle's primitives, under the names src/pipeline.rs imports. A cell
/// access is a point where threads may switch: the Pipeline's cells are
/// handed between the caller and the workers by its atomics, and a
/// handing-over that is wrong must be able to show.
mod pipeline_sync {
    pub use shuttle::sync::Mutex;
    pub use shuttle::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    pub use shuttle::thread_local;
    /// Shuttle's threads. A caller about to sleep lets the held-back
    /// workers go (lanes::sources::hold_back): they are who would wake it.
    pub mod thread {
        pub use shuttle::thread::{Thread, current};
        pub fn park() {
            crate::lanes::sources::caller_sleeps(true);
            shuttle::thread::park();
            crate::lanes::sources::caller_sleeps(false);
        }
    }

    /// A static for one run of the model: Shuttle's primitives must not
    /// pass from one run to the next.
    macro_rules! process_static {
        ($(#[$attribute:meta])* static $name:ident: $kind:ty = $first:expr;) => {
            shuttle::lazy_static! {
                $(#[$attribute])*
                static ref $name: $kind = $first;
            }
        };
    }
    pub(crate) use process_static;

    #[repr(transparent)]
    pub struct UnsafeCell<T>(core::cell::UnsafeCell<T>);
    impl<T> UnsafeCell<T> {
        pub const fn new(v: T) -> Self {
            Self(core::cell::UnsafeCell::new(v))
        }
        pub fn get(&self) -> *mut T {
            shuttle::thread::yield_now();
            self.0.get()
        }
    }
}

/// As tests/shuttle_waker.rs has it.
mod waker_sync {
    pub use shuttle::sync::atomic::{AtomicBool, AtomicUsize, Ordering, fence};
    pub struct UnsafeCell<T> {
        busy: AtomicBool,
        inner: core::cell::UnsafeCell<T>,
    }
    impl<T> UnsafeCell<T> {
        pub fn new(v: T) -> Self {
            Self { busy: AtomicBool::new(false), inner: core::cell::UnsafeCell::new(v) }
        }
    }
    pub fn spin() {
        shuttle::thread::yield_now();
    }
    pub unsafe fn with_mut<T, R>(cell: &UnsafeCell<T>, f: impl FnOnce(&mut T) -> R) -> R {
        assert!(!cell.busy.swap(true, Ordering::SeqCst), "concurrent mutable access to WakerSlot::waker");
        shuttle::thread::yield_now();
        let out = f(unsafe { &mut *cell.inner.get() });
        assert!(cell.busy.swap(false, Ordering::SeqCst));
        out
    }
}

#[path = "../src/waker.rs"]
mod waker;

#[path = "../src/pipeline.rs"]
mod pipeline;

// What src/pipeline.rs takes from its crate's root: the core library's
// own items, as threads/src/lib.rs names them.
pub(crate) use blake3_servil_core::thread_support::{Platform, assert_slots, slot_len};
pub(crate) use blake3_servil_core::{CHUNK_LEN, Hash, Mode, OUT_LEN, convenience};
pub(crate) mod throughput {
    pub use blake3_servil_core::throughput::*;
}

/// The pool's stand-in.
mod lanes {
    use crate::Platform;
    use crate::throughput::TreeHasher;
    use shuttle::sync::Mutex;
    use std::collections::VecDeque;

    pub(crate) const MEMBERS: usize = 64;
    pub(crate) const TASK_LEN: usize = 64 * crate::CHUNK_LEN;
    pub(crate) const SPIN_BEFORE_SLEEP: std::time::Duration = std::time::Duration::ZERO;

    #[repr(align(128))]
    pub(crate) struct OwnLine<T>(pub(crate) T);
    impl<T> std::ops::Deref for OwnLine<T> {
        type Target = T;
        fn deref(&self) -> &T {
            &self.0
        }
    }

    #[derive(Clone, Copy)]
    pub(crate) struct Member {
        pub(crate) input: *const u8,
        pub(crate) len: usize,
        pub(crate) out: *mut u8,
    }
    const NO_MEMBER: Member = Member { input: core::ptr::null(), len: 0, out: core::ptr::null_mut() };

    pub(crate) struct Task {
        pub(crate) len: usize,
        pub(crate) batch: Option<usize>,
        pub(crate) members: usize,
        pub(crate) member: [Member; MEMBERS],
    }
    unsafe impl Send for Task {}

    impl Task {
        pub(crate) fn members(batch: Option<usize>) -> Task {
            Task { len: 0, batch, members: 0, member: [NO_MEMBER; MEMBERS] }
        }

        /// As the pool's, each member alone.
        pub(crate) fn run_members(&self, tree: &TreeHasher, platform: Platform) {
            for m in &self.member[..self.members] {
                // A worker may be switched away from before each member.
                shuttle::thread::yield_now();
                let bytes = unsafe { core::slice::from_raw_parts(m.input, m.len) };
                if let Some(message_len) = self.batch {
                    let slot = crate::slot_len(message_len);
                    let digests = unsafe { core::slice::from_raw_parts_mut(m.out as *mut [u8; crate::OUT_LEN], m.len / slot) };
                    tree.hash_slots_on(platform, bytes, message_len, slot, digests);
                } else {
                    let hash = tree.hash_on(platform, bytes);
                    unsafe { core::ptr::copy_nonoverlapping(hash.as_bytes().as_ptr(), m.out, crate::OUT_LEN) };
                }
            }
        }
    }

    pub(crate) fn pool_platform() -> Platform {
        Platform::detect()
    }
    pub(crate) fn require() {}
    /// The stand-in pool has workers; a Pipeline with no slot (NO_SLOT)
    /// models the unregistered path a one-thread pool takes.
    pub(crate) fn pool_has_workers() -> bool {
        true
    }
    /// The process's end, as the pool's own: the model counts it a fault shown.
    pub(crate) fn stop(why: &str) -> ! {
        eprint!("{why}");
        std::process::abort()
    }
    pub(crate) fn now() -> std::time::Instant {
        static BASE: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        *BASE.get_or_init(std::time::Instant::now)
    }
    pub(crate) fn poll_pause(_: &mut std::time::Instant) {
        shuttle::thread::yield_now();
    }

    pub(crate) mod sources {
        use super::*;

        pub(crate) const ACTION_SME2: u64 = 1 << 63;

        type Action = (unsafe fn(*const (), u64, Platform), usize, u64);

        /// A Pipeline's actions waiting for a thread.
        pub(crate) struct Source {
            waiting: Mutex<VecDeque<Action>>,
            actions: usize,
        }

        shuttle::lazy_static! {
            /// Set by a scenario: the pool has no slot free for a Pipeline.
            pub(crate) static ref NO_SLOT: shuttle::sync::atomic::AtomicBool = shuttle::sync::atomic::AtomicBool::new(false);
            /// The Sources a worker serves: those of the Pipelines open now.
            static ref REGISTERED: Mutex<Vec<usize>> = Mutex::new(Vec::new());
            static ref NEWS: Mutex<u64> = Mutex::new(0);
            static ref NEWS_CAME: shuttle::sync::Condvar = shuttle::sync::Condvar::new();
        }

        impl Source {
            pub(crate) fn with_actions(actions: usize) -> Source {
                Source { waiting: Mutex::new(VecDeque::new()), actions }
            }
            pub(crate) fn actions_len(&self) -> usize {
                self.actions
            }
            pub(crate) fn register(&'static self) -> Option<usize> {
                // The pool has 64 slots, and a Pipeline that finds none
                // free runs each action itself, where it dispatches it.
                if NO_SLOT.load(shuttle::sync::atomic::Ordering::SeqCst) {
                    return None;
                }
                let at = self as *const Source as usize;
                REGISTERED.lock().unwrap().push(at);
                Some(at)
            }
            pub(crate) fn unregister(slot: usize) {
                REGISTERED.lock().unwrap().retain(|&at| at != slot);
            }
            pub(crate) fn publish_actions(&self, run: unsafe fn(*const (), u64, Platform), context: *const (), args: &[u64]) {
                let mut waiting = self.waiting.lock().unwrap();
                assert!(waiting.len() + args.len() <= self.actions, "more actions than the Source was made for");
                waiting.extend(args.iter().map(|&arg| (run, context as usize, arg)));
                drop(waiting);
                news();
            }
            pub(crate) fn run_waiting_action(&self) -> bool {
                let Some((run, context, arg)) = self.waiting.lock().unwrap().pop_front() else { return false };
                unsafe { run(context as *const (), arg & !ACTION_SME2, pool_platform()) };
                true
            }
            pub(crate) fn wake_for_waiting(&self) {
                shuttle::thread::yield_now();
            }
        }

        /// How many times there has been news for the workers (actions
        /// published, or the model's end), and the workers' sleep until
        /// the next. A worker with nothing to run sleeps: were it to spin,
        /// Shuttle would see a thread that can always run, and a caller
        /// asleep with no one to wake it would not be the deadlock it is.

        pub(crate) fn news() {
            *NEWS.lock().unwrap() += 1;
            NEWS_CAME.notify_all();
        }
        pub(crate) fn news_so_far() -> u64 {
            *NEWS.lock().unwrap()
        }
        pub(crate) fn sleep_until_news_after(seen: u64) {
            let mut news = NEWS.lock().unwrap();
            while *news == seen {
                news = NEWS_CAME.wait(news).unwrap();
            }
        }

        pub(crate) fn opened() {}
        pub(crate) fn closed() {}

        /// One turn of a worker: an action of some registered Source, run.
        /// A Source is in a storage that is never freed, so its address
        /// stays good after its Pipeline is gone.
        pub(crate) fn run_one() -> bool {
            let all: Vec<usize> = REGISTERED.lock().unwrap().clone();
            for at in all {
                let source = unsafe { &*(at as *const Source) };
                let Some((run, context, arg)) = source.waiting.lock().unwrap().pop_front() else { continue };
                hold_back();
                unsafe { run(context as *const (), arg & !ACTION_SME2, pool_platform()) };
                BY_WORKERS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return true;
            }
            false
        }

        shuttle::lazy_static! {
            /// The callers asleep now, and whether the model has ended.
            static ref ASLEEP: Mutex<(usize, bool)> = Mutex::new((0, false));
            static ref ASLEEP_CHANGED: shuttle::sync::Condvar = shuttle::sync::Condvar::new();
        }

        /// A worker with an action in hand is, one time in four, held back
        /// for as long as no caller is asleep (and the model has not
        /// ended): a slow worker, as slow as can be without hanging the
        /// caller. Everything else that can happen first then does, with
        /// this action taken and not yet run. Without it a schedule must
        /// starve the worker by chance for hundreds of steps, which the
        /// schedulers seldom do, and planted faults that need it went
        /// unseen. It waits on what is so (a caller is asleep), not on a
        /// signal: the first form waited for the next caller to fall
        /// asleep, and a worker that began to hold just after the last one
        /// had was never let go, which this model then reported as the
        /// Pipeline's deadlock (once in some two hundred runs).
        ///
        /// A scenario's own thread that gives something back to a waiting
        /// caller (a Room dropped on another thread) is held back the
        /// same way: left to the schedulers it is done within a few
        /// steps, always before the caller has fallen asleep.
        pub(crate) fn hold_back() {
            use shuttle::rand::Rng;
            if shuttle::rand::thread_rng().gen_ratio(1, 4) {
                let mut asleep = ASLEEP.lock().unwrap();
                while asleep.0 == 0 && !asleep.1 {
                    asleep = ASLEEP_CHANGED.wait(asleep).unwrap();
                }
            }
        }
        /// A caller is about to sleep (`true`), or has woken.
        pub(crate) fn caller_sleeps(now: bool) {
            let mut asleep = ASLEEP.lock().unwrap();
            if now {
                asleep.0 += 1;
            } else {
                asleep.0 -= 1;
            }
            drop(asleep);
            ASLEEP_CHANGED.notify_all();
        }
        /// The model's end: no worker is held back any more.
        pub(crate) fn ended() {
            ASLEEP.lock().unwrap().1 = true;
            ASLEEP_CHANGED.notify_all();
        }
        /// Actions the workers ran, over every schedule: a model whose
        /// caller ran them all would check nothing.
        pub(crate) static BY_WORKERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    }
}

use pipeline::{Event, Limits, Pipeline, ROOM_LEN, block_on};
use shuttle::sync::Arc;
use shuttle::sync::atomic::{AtomicBool, Ordering};
use shuttle::thread;

/// `model` under two of Shuttle's schedulers: PCT, which finds the faults
/// that need a few switches at the right places, and the random one,
/// which switches often. The seeds are fixed, so every run tries the same
/// schedules and a failure here can be had again. To try others, set
/// SHUTTLE_RANDOM_SEED to a number: Shuttle then seeds both with it, and
/// a failure prints its schedule.
fn check(model: impl Fn() + Send + Sync + 'static) {
    use shuttle::scheduler::{PctScheduler, RandomScheduler};
    let model = std::sync::Arc::new(model);
    let again = std::sync::Arc::clone(&model);
    shuttle::Runner::new(PctScheduler::new_from_seed(0xb1a4_e300, 3, 2000), Default::default()).run(move || again());
    shuttle::Runner::new(RandomScheduler::new_from_seed(0xb1a4_e301, 5000), Default::default()).run(move || model());
}

/// Runs `caller` with `workers` threads serving the Sources until it
/// returns.
fn with_workers(workers: usize, caller: impl FnOnce()) {
    let stop = Arc::new(AtomicBool::new(false));
    let threads: Vec<_> = (0..workers)
        .map(|_| {
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                loop {
                    let seen = lanes::sources::news_so_far();
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    if !lanes::sources::run_one() {
                        lanes::sources::sleep_until_news_after(seen);
                    }
                }
            })
        })
        .collect();
    caller();
    stop.store(true, Ordering::SeqCst);
    lanes::sources::ended();
    lanes::sources::news();
    for t in threads {
        t.join().unwrap();
    }
    // Every storage made in this run is free again: none is lost, and
    // none is held by a Pipeline or a Room that is gone.
    let (made, back) = pipeline::model_storages::free_storages_for_model();
    assert_eq!(made, back, "storages made in this run, and those free at its end");
}

fn input(len: usize, salt: u8) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8 ^ salt).collect()
}

fn reference(bytes: &[u8]) -> Hash {
    convenience::hash(bytes)
}

/// Events of `pipe` until `left` have come, each digest checked against
/// `want` by its tag, the caller sleeping where none is ready.
fn drain<B>(pipe: &mut Pipeline<usize, B>, mut left: usize, want: &[Hash]) {
    while left > 0 {
        match pipe.next_event() {
            Some(Event::Hashed { tag, hash, .. }) => {
                assert_eq!(hash, want[tag], "message {tag}'s digest");
                left -= 1;
            }
            Some(_) => panic!("an event that is no digest"),
            None => block_on(pipe.ready()),
        }
    }
}

/// `inputs` through Rooms, at most `messages` in the Pipeline at once.
fn through_rooms(pipe: &mut Pipeline<usize, ()>, messages: usize, inputs: &[Vec<u8>], want: &[Hash]) {
    let mut outstanding = 0;
    for (tag, bytes) in inputs.iter().enumerate() {
        if outstanding == messages {
            drain(pipe, 1, want);
            outstanding -= 1;
        }
        let mut message = block_on(pipe.message(tag, Some(bytes.len() as u64)));
        let mut at = 0;
        while at < bytes.len() {
            let mut room = block_on(pipe.room());
            let n = room.bytes_mut().len().min(bytes.len() - at);
            room.bytes_mut()[..n].copy_from_slice(&bytes[at..at + n]);
            pipe.commit(&mut message, &mut room, n);
            at += n;
        }
        pipe.end(message);
        outstanding += 1;
    }
    drain(pipe, outstanding, want);
}

fn inputs_of(lens: &[usize]) -> (Vec<Vec<u8>>, Vec<Hash>) {
    let inputs: Vec<Vec<u8>> = lens.iter().enumerate().map(|(i, &len)| input(len, i as u8)).collect();
    let want = inputs.iter().map(|b| reference(b)).collect();
    (inputs, want)
}

fn actions_by_workers() -> usize {
    lanes::sources::BY_WORKERS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Short messages gathered into groups and messages of several Rooms,
/// more Rooms' worth than the Pipeline has: a Room comes back to the
/// caller only when the worker that read it is done with it.
fn messages_through_rooms_complete_with_their_own_digests() {
    let before = actions_by_workers();
    check(|| {
        with_workers(2, || {
            let (inputs, want) = inputs_of(&[0, 1, 64, 65, 1024, 3 * ROOM_LEN / 2, 3 * ROOM_LEN, 7, 2 * ROOM_LEN + 1]);
            let mut pipe = Pipeline::<usize, ()>::new(Mode::Plain, Limits { messages: 3, rooms: 2 });
            through_rooms(&mut pipe, 3, &inputs, &want);
        });
    });
    assert!(actions_by_workers() > before, "the workers ran no action: the caller alone checks nothing");
}

/// Messages hashed in place from their owners, with no Room: the caller
/// is woken by the finishing of the work alone.
fn owned_messages_complete_and_wake_the_caller() {
    check(|| {
        with_workers(2, || {
            let (inputs, want) = inputs_of(&[0, 100, 1024, 5 * ROOM_LEN + 3, 64]);
            let mut pipe = Pipeline::<usize, Vec<u8>>::with_owners(Mode::Plain, Limits { messages: 2, rooms: 0 });
            let mut outstanding = 0;
            for (tag, bytes) in inputs.iter().enumerate() {
                if outstanding == 2 {
                    drain(&mut pipe, 1, &want);
                    outstanding -= 1;
                }
                let message = block_on(pipe.message(tag, None));
                pipe.owned(message, bytes.clone());
                outstanding += 1;
            }
            drain(&mut pipe, outstanding, &want);
        });
    });
}

/// One Pipeline after another of the same size: the second takes the
/// first's storage, with the first's workers perhaps still on their way
/// out of its last tasks.
fn a_pipeline_after_another_reuses_its_storage_soundly() {
    check(|| {
        with_workers(2, || {
            for round in 0..3u8 {
                let lens = [1usize + round as usize, 2 * ROOM_LEN, 64, 300];
                let inputs: Vec<Vec<u8>> = lens.iter().enumerate().map(|(i, &len)| input(len, round * 16 + i as u8)).collect();
                let want: Vec<Hash> = inputs.iter().map(|b| reference(b)).collect();
                let mut pipe = Pipeline::<usize, ()>::new(Mode::Plain, Limits { messages: 2, rooms: 2 });
                through_rooms(&mut pipe, 2, &inputs, &want);
            }
        });
    });
}

/// A Room held past its Pipeline's end, written and given back while
/// two more Pipelines of that size are at work on two threads: the
/// storage the Room holds is no one else's until the Room is gone.
fn a_room_outlives_its_pipeline() {
    check(|| {
        with_workers(1, || {
            let first = Pipeline::<usize, ()>::new(Mode::Plain, Limits { messages: 2, rooms: 2 });
            let mut first = first;
            let mut held = block_on(first.room());
            drop(first);
            let giver = thread::spawn(move || {
                held.bytes_mut().fill(0xa5);
                drop(held);
            });
            let others: Vec<_> = (0..2u8)
                .map(|which| {
                    thread::spawn(move || {
                        let lens = [ROOM_LEN + 5, 9 + which as usize, 2 * ROOM_LEN];
                        let inputs: Vec<Vec<u8>> = lens.iter().enumerate().map(|(i, &len)| input(len, which * 16 + i as u8)).collect();
                        let want: Vec<Hash> = inputs.iter().map(|b| reference(b)).collect();
                        let mut pipe = Pipeline::<usize, ()>::new(Mode::Plain, Limits { messages: 2, rooms: 2 });
                        through_rooms(&mut pipe, 2, &inputs, &want);
                    })
                })
                .collect();
            giver.join().unwrap();
            for other in others {
                other.join().unwrap();
            }
        });
    });
}

/// A wait for a Room while the Pipeline's one Room is alive on another
/// thread, with bytes committed from it that a worker may still be
/// reading: the wait ends when the Room has been dropped there and the
/// worker is done, whichever comes last, and each wakes the caller (a
/// wake lost at either leaves the caller asleep for good, which Shuttle
/// reports). The thread that drops the Room is sometimes held back until
/// the caller sleeps, as a worker is. `room()` refused this wait until
/// November 19, 2026, by a count of the Rooms the calling program held.
fn a_wait_for_a_room_ends_when_another_thread_drops_one() {
    check(|| {
        with_workers(1, || {
            let (inputs, want) = inputs_of(&[ROOM_LEN + 7]);
            let mut pipe = Pipeline::<usize, ()>::new(Mode::Plain, Limits { messages: 1, rooms: 1 });
            let mut message = block_on(pipe.message(0, None));
            let mut room = block_on(pipe.room());
            room.bytes_mut().copy_from_slice(&inputs[0][..ROOM_LEN]);
            pipe.commit(&mut message, &mut room, ROOM_LEN);
            let giver = thread::spawn(move || {
                lanes::sources::hold_back();
                drop(room);
            });
            let mut room = block_on(pipe.room());
            giver.join().unwrap();
            room.bytes_mut()[..7].copy_from_slice(&inputs[0][ROOM_LEN..]);
            pipe.commit(&mut message, &mut room, 7);
            drop(room);
            pipe.end(message);
            drain(&mut pipe, 1, &want);
        });
    });
}

/// Messages lent by the caller: when a message's event has come, its
/// guard can be dropped (a guard dropped before its loan is complete
/// stops the process).
fn lent_messages_are_complete_when_their_events_come() {
    check(|| {
        with_workers(2, || {
            let (inputs, want) = inputs_of(&[3 * ROOM_LEN + 1, 0, 200, 6 * ROOM_LEN]);
            let mut pipe = Pipeline::<usize, ()>::new(Mode::Plain, Limits { messages: 2, rooms: 0 });
            let mut guards: Vec<Option<pipeline::Lent<'_>>> = inputs.iter().map(|_| None).collect();
            let mut outstanding = 0;
            let take = |pipe: &mut Pipeline<usize, ()>, guards: &mut Vec<Option<pipeline::Lent<'_>>>| loop {
                match pipe.next_event() {
                    Some(Event::Hashed { tag, hash, .. }) => {
                        assert_eq!(hash, want[tag], "message {tag}'s digest");
                        drop(guards[tag].take().expect("an event once a message"));
                        return;
                    }
                    Some(_) => panic!("an event that is no digest"),
                    None => block_on(pipe.ready()),
                }
            };
            for (tag, bytes) in inputs.iter().enumerate() {
                if outstanding == 2 {
                    take(&mut pipe, &mut guards);
                    outstanding -= 1;
                }
                let message = block_on(pipe.message(tag, None));
                // Sound: `inputs` outlives the Pipeline and every guard.
                guards[tag] = Some(unsafe { pipe.lent(message, bytes) });
                outstanding += 1;
            }
            for _ in 0..outstanding {
                take(&mut pipe, &mut guards);
            }
        });
    });
}

/// Batches of short messages, a small one (one group's task) and one
/// cut into several tasks, among single messages: each batch's digests
/// are all written when its event comes.
fn batches_are_whole_when_their_events_come() {
    check(|| {
        with_workers(2, || {
            let mut pipe = Pipeline::<usize, ()>::new(Mode::Plain, Limits { messages: 3, rooms: 1 });
            let shapes = [(100usize, 3usize), (64, 5000), (1, 70)];
            let mut outstanding = 0;
            let check_event = |pipe: &mut Pipeline<usize, ()>| loop {
                match pipe.next_event() {
                    Some(Event::Batch { tag, batch, digests }) => {
                        let (message_len, count) = shapes[tag];
                        assert_eq!((batch.message_len(), digests.len()), (message_len, count));
                        let slot = message_len.next_multiple_of(64).max(64);
                        let bytes = throughput::Block::bytes(batch.blocks());
                        for (i, digest) in digests.iter().enumerate() {
                            assert_eq!(*digest, *reference(&bytes[i * slot..i * slot + message_len]).as_bytes(), "batch {tag}'s digest {i}");
                        }
                        return;
                    }
                    Some(_) => panic!("an event that is no batch"),
                    None => block_on(pipe.ready()),
                }
            };
            for (tag, &(message_len, count)) in shapes.iter().enumerate() {
                if outstanding == 3 {
                    check_event(&mut pipe);
                    outstanding -= 1;
                }
                let mut batch = throughput::Batch::new(message_len, count);
                for i in 0..count {
                    batch.set(i, &input(message_len, (tag * 31 + i) as u8));
                }
                let message = block_on(pipe.message(tag, None));
                pipe.batch(message, batch, vec![[0u8; 32]; count].into_boxed_slice());
                outstanding += 1;
            }
            for _ in 0..outstanding {
                check_event(&mut pipe);
            }
        });
    });
}

/// A Pipeline for which the pool has no slot free: it runs each action
/// itself, where it would have handed it to the pool, and no worker
/// takes part. Its messages complete all the same, and it never waits
/// for a wake no one will send.
fn a_pipeline_with_no_pool_slot_does_its_own_work() {
    check(|| {
        with_workers(1, || {
            lanes::sources::NO_SLOT.store(true, Ordering::SeqCst);
            let (inputs, want) = inputs_of(&[0, 64, 1024, 2 * ROOM_LEN + 1, 5]);
            let mut pipe = Pipeline::<usize, ()>::new(Mode::Plain, Limits { messages: 2, rooms: 2 });
            through_rooms(&mut pipe, 2, &inputs, &want);
            let (owned, owned_want) = inputs_of(&[100, 5 * ROOM_LEN + 3]);
            let mut owners = Pipeline::<usize, Vec<u8>>::with_owners(Mode::Plain, Limits { messages: 2, rooms: 0 });
            for (tag, bytes) in owned.iter().enumerate() {
                let message = block_on(owners.message(tag, None));
                owners.owned(message, bytes.clone());
            }
            drain(&mut owners, owned.len(), &owned_want);
        });
    });
}

/// A message aborted with Rooms of it at the workers, among others that
/// complete.
fn an_aborted_message_leaves_the_others_whole() {
    check(|| {
        with_workers(2, || {
            let (inputs, want) = inputs_of(&[2 * ROOM_LEN, 77, ROOM_LEN + 9]);
            let mut pipe = Pipeline::<usize, ()>::new(Mode::Plain, Limits { messages: 3, rooms: 2 });
            let mut gone = block_on(pipe.message(99, None));
            let mut room = block_on(pipe.room());
            room.bytes_mut().fill(1);
            let all = room.bytes_mut().len();
            pipe.commit(&mut gone, &mut room, all);
            drop(room);
            pipe.abort(gone);
            let mut seen_abort = false;
            let mut outstanding = 1;
            let mut hashed = 0;
            let mut take = |pipe: &mut Pipeline<usize, ()>| loop {
                match pipe.next_event() {
                    Some(Event::Hashed { tag, hash, .. }) => {
                        assert_eq!(hash, want[tag], "message {tag}'s digest");
                        hashed += 1;
                        return;
                    }
                    Some(Event::Aborted { tag, .. }) => {
                        assert!(tag == 99 && !seen_abort, "one abort, of the aborted message");
                        seen_abort = true;
                        return;
                    }
                    Some(_) => panic!("an event of another kind"),
                    None => block_on(pipe.ready()),
                }
            };
            for (tag, bytes) in inputs.iter().enumerate() {
                if outstanding == 3 {
                    take(&mut pipe);
                    outstanding -= 1;
                }
                let mut message = block_on(pipe.message(tag, None));
                let mut at = 0;
                while at < bytes.len() {
                    let mut room = block_on(pipe.room());
                    let n = room.bytes_mut().len().min(bytes.len() - at);
                    room.bytes_mut()[..n].copy_from_slice(&bytes[at..at + n]);
                    pipe.commit(&mut message, &mut room, n);
                    at += n;
                }
                pipe.end(message);
                outstanding += 1;
            }
            for _ in 0..outstanding {
                take(&mut pipe);
            }
            assert!(seen_abort && hashed == inputs.len());
        });
    });
}

/// The scenarios, one after another in one test: the Pipeline's free
/// storages and the Sources' registry are the process's, as in a program,
/// and two models at once on two test threads would share them.
#[test]
fn the_pipeline_under_shuttle() {
    blake3_servil_core::initialize();
    let scenarios: [(&str, fn()); 9] = [
        ("messages_through_rooms_complete_with_their_own_digests", messages_through_rooms_complete_with_their_own_digests),
        ("owned_messages_complete_and_wake_the_caller", owned_messages_complete_and_wake_the_caller),
        ("a_pipeline_after_another_reuses_its_storage_soundly", a_pipeline_after_another_reuses_its_storage_soundly),
        ("a_room_outlives_its_pipeline", a_room_outlives_its_pipeline),
        ("a_wait_for_a_room_ends_when_another_thread_drops_one", a_wait_for_a_room_ends_when_another_thread_drops_one),
        ("lent_messages_are_complete_when_their_events_come", lent_messages_are_complete_when_their_events_come),
        ("an_aborted_message_leaves_the_others_whole", an_aborted_message_leaves_the_others_whole),
        ("batches_are_whole_when_their_events_come", batches_are_whole_when_their_events_come),
        ("a_pipeline_with_no_pool_slot_does_its_own_work", a_pipeline_with_no_pool_slot_does_its_own_work),
    ];
    for (name, scenario) in scenarios {
        eprintln!("shuttle_pipeline: {name}");
        scenario();
    }
}
