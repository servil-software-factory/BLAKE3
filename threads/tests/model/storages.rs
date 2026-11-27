//! A child module of src/pipeline.rs under `--cfg shuttle` alone
//! (tests/shuttle_pipeline.rs): verifies that every `PipeStorage` allocated
//! during a run was freed via `PipeStorage::release_n` and every recycled
//! `PipeSlot` returned to `FREE_PIPE_SLOTS`, then frees the run's `PipeSlot`s
//! once all Shuttle threads have joined.

use super::{FREE_PIPE_SLOTS, FreeSlot, LIVE_STORAGES, MODEL_OWNED_PIPE_SLOTS, Ordering, OwnedPipeSlotPtr, PIPE_SLOTS_MADE, PipeSlot, PipeStorage, slot_tier};

fn slice_range<T>(s: &[T]) -> Option<(usize, usize)> {
    let bytes = core::mem::size_of_val(s);
    if bytes == 0 {
        return None;
    }
    let start = s.as_ptr() as usize;
    Some((start, start + bytes))
}

/// Address ranges owned by `st` (the struct itself and every boxed array and
/// room buffer inside it), recorded when `st` is retired under Shuttle so any
/// subsequent atomic, cell, or `SlotRef` read inside them panics immediately.
pub(crate) fn retired_ranges(st: *const PipeStorage) -> Vec<(usize, usize)> {
    // Sound: `st` was just retired and quarantined for the rest of this Shuttle
    // run, so its metadata pointers remain readable until `drain_quarantined_storages`.
    let s = unsafe { &*st };
    let mut ranges = Vec::with_capacity(12);
    let base = st as usize;
    ranges.push((base, base + core::mem::size_of::<PipeStorage>()));
    for r in [
        slice_range(&s.groups),
        slice_range(&s.slot_trees),
        slice_range(&s.hashes),
        slice_range(&s.cvs),
        slice_range(&s.slot_done),
        slice_range(&s.tasks_left),
        slice_range(&s.multi),
        slice_range(&s.pieces),
        slice_range(&s.rooms.refs),
    ]
    .into_iter()
    .flatten()
    {
        ranges.push(r);
    }
    // Sound: `s.rooms.backing` stays valid while `st` is quarantined until teardown.
    if let Some(r) = slice_range(unsafe { &*s.rooms.backing }) {
        ranges.push(r);
    }
    ranges
}

/// Monotonic multi-piece loan completion epoch on `pipe`'s `PipeSlot` for slot
/// `s`, used by `shuttle_pipeline` to verify saved loan epochs advance across
/// `PipeSlot` reuse.
pub(crate) fn loan_completed_epoch<T, B, P>(pipe: &super::Pipeline<T, B, P>, s: usize) -> u64 {
    pipe.pipe_slot.loan_completed[s].load(Ordering::Acquire)
}

/// At the end of one run of the model, verify every `PipeStorage` was freed
/// (`live == 0`) and every `PipeSlot` made in the run returned by identity to
/// `FREE_PIPE_SLOTS`, then free the run's `PipeSlot` allocations so Shuttle
/// atomics do not leak across runs.
pub(crate) fn free_storages_for_model() -> (usize, usize, usize) {
    let mut returned_addrs: Vec<usize> = Vec::new();
    {
        let mut tiers = FREE_PIPE_SLOTS.lock().unwrap();
        for (tier_idx, bucket) in tiers.iter_mut().enumerate() {
            for FreeSlot { slot, next_group_epoch, next_loan_epoch } in bucket.drain(..) {
                assert_eq!(slot_tier(slot.loan_completed.len() * super::MAX_SLOT_PIECES), tier_idx, "PipeSlot returned to wrong tier bucket");
                for g in slot.group_completed.iter() {
                    assert!(next_group_epoch > g.0.load(Ordering::Acquire), "saved next_group_epoch must exceed completed group epoch");
                }
                for l in slot.loan_completed.iter() {
                    assert!(next_loan_epoch > l.0.load(Ordering::Acquire), "saved next_loan_epoch must exceed completed loan epoch");
                }
                returned_addrs.push(slot as *const PipeSlot as usize);
            }
        }
    }
    let owned: Vec<OwnedPipeSlotPtr> = MODEL_OWNED_PIPE_SLOTS.lock().unwrap().drain(..).collect();
    let made = PIPE_SLOTS_MADE.swap(0, Ordering::SeqCst);
    let live = LIVE_STORAGES.swap(0, Ordering::SeqCst);
    returned_addrs.sort_unstable();
    let back = returned_addrs.len();
    let mut owned_addrs: Vec<usize> = owned.iter().map(|p| p.0 as usize).collect();
    owned_addrs.sort_unstable();
    assert_eq!(returned_addrs, owned_addrs, "PipeSlots returned must match PipeSlots created by identity");
    for OwnedPipeSlotPtr(ptr) in owned {
        // Sound: the model's threads have all joined and `ptr` is the original
        // owning pointer from `Box::into_raw` in `PipeSlot::acquire`.
        unsafe { drop(Box::from_raw(ptr)) };
    }
    (made, back, live)
}
