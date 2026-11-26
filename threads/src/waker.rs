//! The pipeline's single-waiter Dekker handshake (`WakerSlot`):
//! a waiting caller stores its `Waker` under a `Mutex` and publishes
//! `registered = true` in `SeqCst` before re-checking completion. A `SeqCst`
//! fence orders publication before the re-check's acquire-only loads,
//! including on Arm RCpc. A worker finishes its item with a `SeqCst` operation
//! and then checks `registered` in `SeqCst`, taking the mutex only after
//! observing registration.
//!
//! `tests/shuttle_waker.rs` compiles this same file beside Shuttle's
//! atomics and `Mutex` (`waker_sync`), checking freedom from lost wakes.

use super::waker_sync::{AtomicBool, Mutex, Ordering, fence};
use core::task::{Context, Waker};

#[repr(align(128))]
pub(crate) struct Line<T>(pub(crate) T);

/// Ends the process if it is dropped: held across a call that must not
/// unwind, and forgotten when the call returns.
struct EndOnUnwind;

impl Drop for EndOnUnwind {
    fn drop(&mut self) {
        std::process::abort()
    }
}

pub(crate) struct WakerSlot {
    registered: Line<AtomicBool>,
    waker: Mutex<Option<Waker>>,
}

impl WakerSlot {
    pub(crate) fn new() -> Self {
        let slot = WakerSlot { registered: Line(AtomicBool::new(false)), waker: Mutex::new(None) };
        // Eagerly initialize platform-lazy mutexes (such as macOS OnceBox
        // pthread mutexes) at slot construction so the first waiter
        // registration never allocates after `Pipeline::new`.
        drop(slot.waker.lock());
        slot
    }

    /// Publish interest before the caller rechecks the completion predicate.
    /// When an equivalent waker is already installed (`will_wake`), skips
    /// cloning/dropping while still publishing `SeqCst` before the caller's
    /// completion re-check.
    pub(crate) fn register(&self, cx: &Context<'_>) {
        let old = self.install_waker(cx);
        // The completion re-check may load with Acquire (RCpc
        // on Arm). It must not move before publishing the waiter.
        fence(Ordering::SeqCst);
        drop(old);
    }

    #[inline(always)]
    fn install_waker(&self, cx: &Context<'_>) -> Option<Waker> {
        {
            let slot = self.waker.lock().expect("WakerSlot mutex poisoned");
            // `Waker::will_wake` compares raw data/vtable pointers and runs no
            // user code while the mutex is held.
            if slot.as_ref().is_some_and(|w| w.will_wake(cx.waker())) {
                self.registered.0.store(true, Ordering::SeqCst);
                return None;
            }
        }
        // Clone outside the mutex so a panicking user `Waker::clone` cannot
        // poison the slot.
        let next = cx.waker().clone();
        let mut slot = self.waker.lock().expect("WakerSlot mutex poisoned");
        let old = slot.replace(next);
        self.registered.0.store(true, Ordering::SeqCst);
        old
    }

    pub(crate) fn clear(&self) {
        if !self.registered.0.load(Ordering::SeqCst) {
            return;
        }
        let old = {
            let mut slot = self.waker.lock().expect("WakerSlot mutex poisoned");
            let old = slot.take();
            self.registered.0.store(false, Ordering::SeqCst);
            old
        };
        drop(old);
    }

    #[inline(always)]
    pub(crate) fn notify_after_seqcst(&self) {
        if self.registered.0.load(Ordering::SeqCst) {
            self.wake_slow();
        }
    }

    #[cold]
    #[inline(never)]
    fn wake_slow(&self) {
        let waker = {
            let mut slot = self.waker.lock().expect("WakerSlot mutex poisoned");
            let waker = slot.take();
            self.registered.0.store(false, Ordering::SeqCst);
            waker
        };
        if let Some(w) = waker {
            // The waker is the program's code, and it runs here in the
            // middle of whatever gave the notice: a worker's task, or
            // a call of the Pipeline's owner that has just freed a
            // room and not yet recorded what it freed. A panic out of
            // it would leave that half done for the program to find (a
            // room's credit given back twice, a room handed out while
            // in use). It ends the process, on whichever thread, as a
            // panic on a worker does; the panic's own words come first.
            let ends = EndOnUnwind;
            w.wake();
            core::mem::forget(ends);
        }
    }
}
