//! Shuttle model check of `src/waker.rs` (`WakerSlot`): causal race-freedom
//! on `UnsafeCell<Option<Waker>>` and lost-wake freedom under concurrent
//! finishers and re-registrations.
//!
//!     RUSTFLAGS="--cfg shuttle" cargo test --release --test shuttle_waker
#![cfg(shuttle)]

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
fn register_overwrite_wakes_replacement_and_clear_suppresses_wake() {
    use core::sync::atomic::AtomicUsize;
    struct CountWake(AtomicUsize);
    fn make_count_waker(state: &Arc<CountWake>) -> Waker {
        unsafe fn clone_fn(ptr: *const ()) -> RawWaker {
            let arc = unsafe { Arc::from_raw(ptr as *const CountWake) };
            let cloned = Arc::clone(&arc);
            core::mem::forget(arc);
            RawWaker::new(Arc::into_raw(cloned) as *const (), &VTABLE)
        }
        unsafe fn wake_fn(ptr: *const ()) {
            let arc = unsafe { Arc::from_raw(ptr as *const CountWake) };
            arc.0.fetch_add(1, Ordering::SeqCst);
        }
        unsafe fn wake_by_ref_fn(ptr: *const ()) {
            let arc = unsafe { Arc::from_raw(ptr as *const CountWake) };
            arc.0.fetch_add(1, Ordering::SeqCst);
            core::mem::forget(arc);
        }
        unsafe fn drop_fn(ptr: *const ()) {
            drop(unsafe { Arc::from_raw(ptr as *const CountWake) });
        }
        static VTABLE: RawWakerVTable = RawWakerVTable::new(clone_fn, wake_fn, wake_by_ref_fn, drop_fn);
        let raw = RawWaker::new(Arc::into_raw(Arc::clone(state)) as *const (), &VTABLE);
        unsafe { Waker::from_raw(raw) }
    }

    shuttle::check_dfs(
        || {
            let slot = WakerSlot::new();
            let a = Arc::new(CountWake(AtomicUsize::new(0)));
            let b = Arc::new(CountWake(AtomicUsize::new(0)));
            let wa = make_count_waker(&a);
            let wb = make_count_waker(&b);

            // Replacing A with distinct B must deliver a subsequent wake to B only.
            slot.register(&Context::from_waker(&wa));
            slot.register(&Context::from_waker(&wb));
            slot.notify_after_seqcst();
            assert_eq!(a.0.load(Ordering::SeqCst), 0, "replaced waker must not be woken");
            assert_eq!(b.0.load(Ordering::SeqCst), 1, "replacement waker must be woken once");

            // Clearing a registered waker must prevent any subsequent wake.
            slot.register(&Context::from_waker(&wa));
            slot.clear();
            slot.notify_after_seqcst();
            assert_eq!(a.0.load(Ordering::SeqCst), 0, "cleared waker must not be woken");
            assert_eq!(b.0.load(Ordering::SeqCst), 1, "previous replacement must stay at 1");

            drop((wa, wb));
            assert_eq!(Arc::strong_count(&a), 1);
            assert_eq!(Arc::strong_count(&b), 1);
        },
        None,
    );
}
