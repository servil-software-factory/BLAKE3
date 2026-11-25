//! The pipeline's single-waiter lockless Dekker handshake (`WakerSlot`):
//! a waiting caller stores its `Waker` and publishes `REGISTERED` in `SeqCst`
//! before re-checking completion. A `SeqCst` fence orders publication
//! before the re-check's acquire-only loads, including on Arm RCpc.
//! A worker finishes its item with a `SeqCst` operation and then checks
//! `state == REGISTERED` in `SeqCst`.
//!
//! `tests/shuttle_waker.rs` compiles this same file beside Shuttle's
//! atomics and `UnsafeCell` (`waker_sync`), checking both causal race-freedom
//! on the stored `Waker` and freedom from lost wakes.

use super::waker_sync::{AtomicUsize, Ordering, UnsafeCell, fence, spin, with_mut};
use core::task::{Context, Waker};

const IDLE: usize = 0;
const WRITING: usize = 1;
const REGISTERED: usize = 2;
const WAKING: usize = 3;

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
    state: Line<AtomicUsize>,
    waker: UnsafeCell<Option<Waker>>,
}

// Sound: the cell is the one part that is not an atomic, and it is
// reached only by the thread whose compare-and-swap moved `state` to
// WRITING (`register`, `clear`) or to WAKING (`wake_slow`). That is one
// thread at a time, and it leaves the state by its own store when it has
// done with the cell. The `Waker` in it is `Send` and `Sync` itself.
unsafe impl Send for WakerSlot {}
// Sound: as above. Every method takes `&self` and goes through `state`.
unsafe impl Sync for WakerSlot {}

impl WakerSlot {
    pub(crate) fn new() -> Self {
        WakerSlot { state: Line(AtomicUsize::new(IDLE)), waker: UnsafeCell::new(None) }
    }

    /// Publish interest before the caller rechecks the completion predicate.
    /// When an equivalent waker is already installed (`will_wake`), skips
    /// cloning/dropping while still issuing the SeqCst fence before the
    /// caller's completion re-check.
    pub(crate) fn register(&self, cx: &Context<'_>) {
        let old = self.install_waker(cx);
        // The completion re-check may load with Acquire (RCpc
        // on Arm). It must not move before publishing the waiter.
        fence(Ordering::SeqCst);
        drop(old);
    }

    #[inline(always)]
    fn install_waker(&self, cx: &Context<'_>) -> Option<Waker> {
        // Fast path: if already REGISTERED with a waker that `will_wake` the
        // caller's, keep it without cloning or running user code under WRITING.
        if self.state.0.compare_exchange(REGISTERED, WRITING, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
            // Sound: WRITING is held exclusively; `Waker::will_wake` compares
            // raw data/vtable pointers and invokes no user code.
            let same = unsafe { with_mut(&self.waker, |slot| slot.as_ref().is_some_and(|w| w.will_wake(cx.waker()))) };
            self.state.0.store(REGISTERED, Ordering::SeqCst);
            if same {
                return None;
            }
        }
        let next = cx.waker().clone();
        loop {
            let s = self.state.0.load(Ordering::SeqCst);
            if (s == IDLE || s == REGISTERED) && self.state.0.compare_exchange(s, WRITING, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                // Sound: this thread's compare-and-swap made the state
                // WRITING, and no other touches the cell until the store
                // below ends that.
                let old = unsafe { with_mut(&self.waker, |slot| slot.replace(next)) };
                self.state.0.store(REGISTERED, Ordering::SeqCst);
                return old;
            }
            spin();
        }
    }

    pub(crate) fn clear(&self) {
        loop {
            let s = self.state.0.load(Ordering::SeqCst);
            if s == IDLE {
                return;
            }
            if s == REGISTERED && self.state.0.compare_exchange(REGISTERED, WRITING, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                // Sound: as in `register`: WRITING is this thread's, until
                // the store below.
                let old = unsafe { with_mut(&self.waker, Option::take) };
                self.state.0.store(IDLE, Ordering::SeqCst);
                drop(old);
                return;
            }
            spin();
        }
    }

    /// Take the registered waker, if any, after a `SeqCst` completion store.
    /// Callers holding a worker `ReaderGuard` or `PipeStorage` reference must
    /// drop those guards BEFORE calling [`wake_or_abort`] on the returned
    /// waker so an inline executor or lock-acquiring waker cannot deadlock with
    /// `Pipeline::drop`.
    #[inline(always)]
    pub(crate) fn take_after_seqcst(&self) -> Option<Waker> {
        if self.state.0.load(Ordering::SeqCst) == REGISTERED { self.take_slow() } else { None }
    }

    #[cold]
    #[inline(never)]
    fn take_slow(&self) -> Option<Waker> {
        if self.state.0.compare_exchange(REGISTERED, WAKING, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
            // Sound: this thread's compare-and-swap made the state
            // WAKING, which keeps `register` and `clear` out of the cell
            // (they spin) until the store below.
            let waker = unsafe { with_mut(&self.waker, Option::take) };
            self.state.0.store(IDLE, Ordering::SeqCst);
            waker
        } else {
            None
        }
    }

    #[inline(always)]
    pub(crate) fn notify_after_seqcst(&self) {
        if let Some(w) = self.take_after_seqcst() {
            wake_or_abort(w);
        }
    }
}

/// Invoke a taken user `Waker` outside all library guards/locks; a panic out
/// of a waker aborts the process so it never unwinds through a worker or
/// leaves half-updated Pipeline state.
#[cold]
#[inline(never)]
pub(crate) fn wake_or_abort(w: Waker) {
    let ends = EndOnUnwind;
    w.wake();
    core::mem::forget(ends);
}
