//! Single-publisher, multi-claimer atomic ticket ring for `Pipeline` actions.
//!
//! Used by `lanes::sources::Source` in production (on `core::sync::atomic`)
//! and compiled directly under `--cfg shuttle` in `tests/shuttle_source_ring.rs`
//! (on `shuttle::sync::atomic`) so Shuttle exercises the exact production
//! publish/claim wrap-around protocol without a hand-copied stand-in (M4).

use crate::action_ring_sync::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};

/// One action ticket in a `Source`'s ring buffer (24 B): the context pointer,
/// the encoded `ActionArg`, and the speculative local-cache wake hint.
pub(crate) struct Action {
    pub(crate) context: AtomicPtr<()>,
    pub(crate) arg: AtomicU64,
    pub(crate) near: AtomicBool,
}

impl Action {
    pub(crate) fn empty() -> Self {
        Self { context: AtomicPtr::new(core::ptr::null_mut()), arg: AtomicU64::new(0), near: AtomicBool::new(false) }
    }
}

/// Publish `args` in order into `actions` with a single `Release` store on
/// `published`. Panics if `args` would overwrite any ticket not yet claimed.
pub(crate) fn publish_slice(actions: &[Action], published: &AtomicUsize, claimed: &AtomicUsize, context: *const (), args: &[u64], near: bool) {
    // Debug-only: internal pool entry; callers only publish non-empty job slices.
    debug_assert!(!args.is_empty());
    let p = published.load(Ordering::Relaxed);
    let next = p.checked_add(args.len()).expect("Source action ticket counter exhausted");
    let cap = actions.len();
    assert!(p - claimed.load(Ordering::Acquire) + args.len() <= cap, "a source's actions hold every item not yet taken");
    for (k, &arg) in args.iter().enumerate() {
        let slot = &actions[(p + k) % cap];
        slot.context.store(context as *mut (), Ordering::Relaxed);
        slot.arg.store(arg, Ordering::Relaxed);
        slot.near.store(near, Ordering::Relaxed);
    }
    published.store(next, Ordering::Release);
}

/// Claim the next published action ticket, returning `(ticket_seq, context, arg)`.
/// Reads the slot speculatively before winning `claimed` with `AcqRel` CAS;
/// a slot cannot be overwritten until its ticket is claimed and the publisher
/// wraps `actions.len()` tickets later, so any winning CAS guarantees the
/// pre-read `(context, arg)` belonged to `ticket_seq`.
pub(crate) fn take_one(actions: &[Action], published: &AtomicUsize, claimed: &AtomicUsize) -> Option<(usize, *const (), u64)> {
    let mut c = claimed.load(Ordering::Acquire);
    let cap = actions.len();
    loop {
        let p = published.load(Ordering::Acquire);
        if c >= p {
            return None;
        }
        let slot = &actions[c % cap];
        let context = slot.context.load(Ordering::Relaxed) as *const ();
        let arg = slot.arg.load(Ordering::Relaxed);
        match claimed.compare_exchange_weak(c, c + 1, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return Some((c, context, arg)),
            Err(now) => c = now,
        }
    }
}
