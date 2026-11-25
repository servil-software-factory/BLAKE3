//! Lock-free epoch-guarded source registry (`EpochRegistry`):
//!
//! Each worker owns an exclusive [`ReaderToken`] bound to its cache-line-padded
//! [`ReaderCell`]. Entering a [`ReaderGuard`] announces a nonzero monotonic
//! epoch (`Release` store + `SeqCst` fence) before loading any slot pointer
//! with `Acquire`; dropping the guard clears the cell back to `0` (`Release`).
//!
//! [`EpochRegistry::unregister_and_drain`] removes a slot from `active`, stores
//! the unaligned `DRAINING` sentinel (`Release`), executes a `SeqCst` fence,
//! and waits for every active reader to clear its cell to `0` or advance to a
//! newer epoch before releasing the slot back to null. Any caller freeing an
//! already-unregistered source (such as a thread-local job source on thread
//! exit) calls [`EpochRegistry::drain_readers`] to run the same fence and wait.
//!
//! `tests/shuttle_registry.rs` compiles this exact file beside Shuttle's
//! atomics (`registry_sync`), and `tools/check-waker-order.py` compiles this
//! exact file on both Arm64 (`aarch64-apple-darwin`) and x86-64
//! (`x86_64-unknown-linux-gnu`) to verify both Dekker fences.

use super::registry_sync::{AtomicPtr, AtomicU64, Ordering, fence, pause_slow, spin};
use core::marker::PhantomData;

/// Value isolated on its own 128-byte cache line.
#[repr(align(128))]
pub(crate) struct Line<T>(pub(crate) T);

/// Per-worker reader epoch cell (`0` = idle, nonzero monotonic epoch = active).
pub(crate) struct ReaderCell(Line<AtomicU64>);

impl ReaderCell {
    pub(crate) fn new() -> Self {
        Self(Line(AtomicU64::new(0)))
    }

    #[cfg(test)]
    pub(crate) fn load_seqcst(&self) -> u64 {
        self.0.0.load(Ordering::SeqCst)
    }
}

/// Exclusive per-worker handle tied to one [`ReaderCell`]. Owns the worker's
/// monotonic epoch counter and borrows `&mut self` for the entire lifetime of
/// [`ReaderGuard`], making overlapping guards on one worker or guards over
/// unmonitored cells unwriteable.
pub(crate) struct ReaderToken<'a> {
    cell: &'a AtomicU64,
    seq: u64,
}

impl<'a> ReaderToken<'a> {
    #[inline]
    pub(crate) fn new(cell: &'a ReaderCell) -> Self {
        Self { cell: &cell.0.0, seq: 0 }
    }

    #[cfg(test)]
    pub(crate) fn set_seq_for_test(&mut self, seq: u64) {
        self.seq = seq;
    }

    #[inline(always)]
    pub(crate) fn enter(&mut self) -> ReaderGuard<'_> {
        let next = self.seq.wrapping_add(1);
        self.seq = if next == 0 { 1 } else { next };
        self.cell.store(self.seq, Ordering::Release);
        fence(Ordering::SeqCst);
        ReaderGuard { cell: self.cell, _exclusive: PhantomData }
    }
}

/// Type-enforced read-side guard over [`EpochRegistry`]. Every read of a
/// registered pointer requires `&ReaderGuard`, guaranteeing that the caller
/// announced a nonzero epoch with `Release` store + `SeqCst` fence before any
/// `Acquire` slot load.
pub(crate) struct ReaderGuard<'a> {
    cell: &'a AtomicU64,
    _exclusive: PhantomData<&'a mut ()>,
}

impl Drop for ReaderGuard<'_> {
    #[inline(always)]
    fn drop(&mut self) {
        self.cell.store(0, Ordering::Release);
    }
}

/// Short spin before blocking in [`EpochRegistry::drain_readers`]: spinning
/// keeps the common case fast while blocking lets a lower-priority worker
/// under fixed-priority scheduling (`SCHED_FIFO`) run and drop its guard.
#[inline]
pub(crate) fn drain_pause_with(spins: &mut u32, spin_fn: impl FnOnce(), sleep_fn: impl FnOnce()) {
    if *spins < 64 {
        *spins += 1;
        spin_fn();
    } else {
        sleep_fn();
    }
}

/// Lock-free fixed-capacity registry of `SLOTS` (`<= 64`) raw pointers guarded
/// by per-reader monotonic epochs.
pub(crate) struct EpochRegistry<T, const SLOTS: usize> {
    slots: [AtomicPtr<T>; SLOTS],
    active: AtomicU64,
}

impl<T, const SLOTS: usize> EpochRegistry<T, SLOTS> {
    /// Sentinel stored in `slots[slot]` while `unregister_and_drain` waits for
    /// active readers: invisible to scanners and un-reusable by new callers
    /// until the drain finishes.
    #[inline(always)]
    fn draining() -> *mut T {
        core::ptr::without_provenance_mut(1)
    }

    #[inline(always)]
    fn is_live(ptr: *mut T) -> bool {
        !ptr.is_null() && ptr != Self::draining()
    }

    #[cfg(not(shuttle))]
    pub(crate) const fn new() -> Self {
        const { assert!(SLOTS > 0 && SLOTS <= 64) };
        Self { slots: [const { AtomicPtr::new(core::ptr::null_mut()) }; SLOTS], active: AtomicU64::new(0) }
    }

    #[cfg(shuttle)]
    pub(crate) fn new() -> Self {
        const { assert!(SLOTS > 0 && SLOTS <= 64) };
        Self { slots: core::array::from_fn(|_| AtomicPtr::new(core::ptr::null_mut())), active: AtomicU64::new(0) }
    }

    #[inline(always)]
    pub(crate) fn any_active(&self) -> bool {
        self.active.load(Ordering::Acquire) != 0
    }

    /// Install `ptr` in a free slot and set its `active` bit.
    ///
    /// # Safety
    ///
    /// `*ptr` must remain valid at its address until the returned slot is
    /// passed to [`Self::unregister_and_drain`] (or [`Self::unregister`]
    /// followed by [`Self::drain_readers`]) and that call returns.
    pub(crate) unsafe fn register(&self, ptr: *mut T) -> Option<usize> {
        let slot = (0..SLOTS).find(|&i| self.slots[i].compare_exchange(core::ptr::null_mut(), ptr, Ordering::SeqCst, Ordering::Relaxed).is_ok())?;
        self.active.fetch_or(1 << slot, Ordering::SeqCst);
        Some(slot)
    }

    /// Clear `slot` without waiting for active readers. Valid only when the
    /// underlying `*ptr` remains alive until a subsequent [`Self::drain_readers`]
    /// (e.g., a thread-local one-shot job source between jobs).
    pub(crate) fn unregister(&self, slot: usize) {
        self.active.fetch_and(!(1 << slot), Ordering::SeqCst);
        self.slots[slot].store(core::ptr::null_mut(), Ordering::SeqCst);
    }

    /// Fence any prior pointer unpublication before sampling `readers`, then
    /// wait until every reader holding a nonzero pre-fence epoch has cleared
    /// its cell to `0` or advanced to a newer epoch.
    #[inline]
    pub(crate) fn drain_readers(readers: &[ReaderCell]) {
        fence(Ordering::SeqCst);
        for reader in readers {
            let initial = reader.0.0.load(Ordering::Acquire);
            if initial != 0 {
                let mut spins = 0u32;
                while reader.0.0.load(Ordering::Acquire) == initial {
                    drain_pause_with(&mut spins, spin, pause_slow);
                }
            }
        }
    }

    /// Unregister `slot` and wait until every reader in `readers` that could
    /// have observed `slot` before `DRAINING` was published has exited that
    /// [`ReaderGuard`]. Pass `None` only when no reader threads exist (e.g.,
    /// in a forked child process).
    pub(crate) fn unregister_and_drain(&self, slot: usize, readers: Option<&[ReaderCell]>) {
        self.active.fetch_and(!(1 << slot), Ordering::SeqCst);
        self.slots[slot].store(Self::draining(), Ordering::Release);
        if let Some(readers) = readers {
            Self::drain_readers(readers);
        }
        self.slots[slot].store(core::ptr::null_mut(), Ordering::Release);
    }

    /// Visit registered live slots starting at `start` (wrapping around),
    /// returning the first slot for which `f` returns `true`.
    #[inline(always)]
    pub(crate) fn find_from(&self, _guard: &ReaderGuard<'_>, start: usize, mut f: impl FnMut(&T) -> bool) -> Option<usize> {
        let mut active = self.active.load(Ordering::Acquire).rotate_right(u32::from(start.to_le_bytes()[0]));
        while active != 0 {
            let slot = (active.trailing_zeros() as usize + start) % SLOTS;
            active &= active - 1;
            let ptr = self.slots[slot].load(Ordering::Acquire);
            // Sound: `_guard` guarantees this thread published a nonzero epoch
            // with `Release` store + `SeqCst` fence before this `Acquire` load,
            // so `*ptr` remains live until `_guard` drops.
            if Self::is_live(ptr) && f(unsafe { &*ptr }) {
                return Some(slot);
            }
        }
        None
    }

    #[cfg(test)]
    pub(crate) fn is_active(&self, slot: usize) -> bool {
        self.active.load(Ordering::SeqCst) & (1 << slot) != 0
    }

    #[cfg(test)]
    pub(crate) fn is_registered(&self, slot: usize) -> bool {
        Self::is_live(self.slots[slot].load(Ordering::SeqCst))
    }

    #[cfg(test)]
    pub(crate) fn is_draining(&self, slot: usize) -> bool {
        let ptr = self.slots[slot].load(Ordering::SeqCst);
        !ptr.is_null() && ptr == Self::draining()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self, slot: usize) -> bool {
        self.slots[slot].load(Ordering::SeqCst).is_null()
    }
}

#[cfg(all(test, not(shuttle)))]
mod test {
    use super::*;

    #[test]
    fn reader_guard_clears_cell_and_skips_zero_on_wrap() {
        let cell = ReaderCell::new();
        let mut token = ReaderToken::new(&cell);
        {
            let _guard = token.enter();
            assert_eq!(cell.load_seqcst(), 1);
        }
        assert_eq!(token.seq, 1);
        assert_eq!(cell.load_seqcst(), 0);
        {
            let _guard = token.enter();
            assert_eq!(cell.load_seqcst(), 2);
        }
        assert_eq!(token.seq, 2);
        assert_eq!(cell.load_seqcst(), 0);
        token.seq = u64::MAX;
        {
            let _guard = token.enter();
            assert_eq!(cell.load_seqcst(), 1);
        }
        assert_eq!(token.seq, 1);
        assert_eq!(cell.load_seqcst(), 0);
    }

    #[test]
    fn drain_pause_spins_64_then_sleeps() {
        let mut spins = 0u32;
        let mut spin_calls = 0u32;
        let mut sleep_calls = 0u32;
        for _ in 0..66 {
            drain_pause_with(&mut spins, || spin_calls += 1, || sleep_calls += 1);
        }
        assert_eq!(spins, 64);
        assert_eq!(spin_calls, 64);
        assert_eq!(sleep_calls, 2);
    }
}
