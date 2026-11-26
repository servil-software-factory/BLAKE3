//! Shuttle model check of `src/waker.rs` (`WakerSlot`): lost-wake freedom
//! under concurrent finishers and re-registrations.
//!
//!     RUSTFLAGS="--cfg shuttle" cargo test --release --test shuttle_waker
#![cfg(shuttle)]

mod waker_sync {
    pub use shuttle::sync::Mutex;
    pub use shuttle::sync::atomic::{AtomicBool, Ordering, fence};
}

#[path = "../src/waker.rs"]
mod waker;

use core::task::{Context, RawWaker, RawWakerVTable, Waker};
use shuttle::sync::Arc;
use shuttle::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use shuttle::thread;
use waker::WakerSlot;

struct ParkWake {
    waiter: thread::Thread,
    woken: AtomicBool,
}

unsafe fn clone_raw(ptr: *const ()) -> RawWaker {
    unsafe { Arc::increment_strong_count(ptr as *const ParkWake) };
    RawWaker::new(ptr, &VTABLE)
}
unsafe fn wake_raw(ptr: *const ()) {
    let arc = unsafe { Arc::from_raw(ptr as *const ParkWake) };
    arc.woken.store(true, Ordering::SeqCst);
    arc.waiter.unpark();
}
unsafe fn wake_ref_raw(ptr: *const ()) {
    let p = unsafe { &*(ptr as *const ParkWake) };
    p.woken.store(true, Ordering::SeqCst);
    p.waiter.unpark();
}
unsafe fn drop_raw(ptr: *const ()) {
    drop(unsafe { Arc::from_raw(ptr as *const ParkWake) });
}

static VTABLE: RawWakerVTable = RawWakerVTable::new(clone_raw, wake_raw, wake_ref_raw, drop_raw);

fn make_shuttle_waker(state: &Arc<ParkWake>) -> Waker {
    let raw = Arc::into_raw(Arc::clone(state)) as *const ();
    unsafe { Waker::from_raw(RawWaker::new(raw, &VTABLE)) }
}

#[test]
fn waker_slot_never_loses_a_wake_or_races_under_concurrent_finishers() {
    shuttle::check_pct(
        || {
            let slot = Arc::new(WakerSlot::new());
            let payload = Arc::new(AtomicUsize::new(0));
            let done = Arc::new(AtomicBool::new(false));

            let (s1, p1, d1) = (Arc::clone(&slot), Arc::clone(&payload), Arc::clone(&done));
            let worker = thread::spawn(move || {
                p1.store(42, Ordering::Relaxed);
                d1.swap(true, Ordering::SeqCst);
                s1.notify_after_seqcst();
            });

            let s2 = Arc::clone(&slot);
            let other = thread::spawn(move || {
                s2.notify_after_seqcst();
            });

            let wake_state = Arc::new(ParkWake { waiter: thread::current(), woken: AtomicBool::new(false) });
            let waker = make_shuttle_waker(&wake_state);
            let cx = Context::from_waker(&waker);

            loop {
                if done.load(Ordering::Acquire) {
                    break;
                }
                slot.register(&cx);
                if done.load(Ordering::Acquire) {
                    slot.clear();
                    break;
                }
                while !wake_state.woken.swap(false, Ordering::SeqCst) {
                    thread::park();
                }
            }

            assert_eq!(payload.load(Ordering::Relaxed), 42);
            worker.join().unwrap();
            other.join().unwrap();
        },
        3000,
        3,
    );
}

#[test]
fn overwrite_and_clear_drop_wakers_and_prevent_spurious_wakes() {
    shuttle::check_pct(
        || {
            let slot = Arc::new(WakerSlot::new());
            let s_worker = Arc::clone(&slot);
            let worker = thread::spawn(move || {
                s_worker.notify_after_seqcst();
            });

            let w1_state = Arc::new(ParkWake { waiter: thread::current(), woken: AtomicBool::new(false) });
            let w1 = make_shuttle_waker(&w1_state);
            slot.register(&Context::from_waker(&w1));

            let w2_state = Arc::new(ParkWake { waiter: thread::current(), woken: AtomicBool::new(false) });
            let w2 = make_shuttle_waker(&w2_state);
            slot.register(&Context::from_waker(&w2));
            slot.clear();
            worker.join().unwrap();

            drop((w1, w2));
            assert_eq!(Arc::strong_count(&w1_state), 1, "first waker dropped");
            assert_eq!(Arc::strong_count(&w2_state), 1, "cleared waker dropped");
        },
        2000,
        3,
    );
}

#[test]
fn superseded_waker_delivers_wake_to_newest_waiter() {
    use core::sync::atomic::AtomicUsize;
    use std::sync::Arc;
    use std::task::Wake;

    struct CountWake(AtomicUsize);
    impl Wake for CountWake {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    shuttle::check_pct(
        || {
            let slot = Arc::new(WakerSlot::new());
            let c1 = Arc::new(CountWake(AtomicUsize::new(0)));
            let c2 = Arc::new(CountWake(AtomicUsize::new(0)));
            let w1 = Waker::from(Arc::clone(&c1));
            let w2 = Waker::from(Arc::clone(&c2));
            slot.register(&Context::from_waker(&w1));
            slot.register(&Context::from_waker(&w2));
            let s_worker = Arc::clone(&slot);
            let worker = thread::spawn(move || s_worker.notify_after_seqcst());
            worker.join().unwrap();
            assert_eq!(
                [c1.0.load(Ordering::SeqCst), c2.0.load(Ordering::SeqCst)],
                [0, 1],
                "notify must wake the newest registered waiter, not a superseded one"
            );
            drop((w1, w2));
            assert_eq!(Arc::strong_count(&c1), 1, "superseded waker dropped");
            assert_eq!(Arc::strong_count(&c2), 1, "notified waker dropped");
        },
        100,
        2,
    );
}
