//! A child module of src/pipeline.rs under `--cfg shuttle` alone
//! (tests/shuttle_pipeline.rs): verifies that every `PipeStorage` allocated
//! during a run was freed via `PipeStorage::release_n` and every recycled
//! `PipeSlot` returned to `FREE_PIPE_SLOTS`, then frees the run's `PipeSlot`s
//! once all Shuttle threads have joined.

use super::{FREE_PIPE_SLOTS, LIVE_STORAGES, MODEL_OWNED_PIPE_SLOTS, Ordering, OwnedPipeSlotPtr, PIPE_SLOTS_MADE, PipeSlot};

/// At the end of one run of the model, verify every `PipeStorage` was freed
/// (`live == 0`) and every `PipeSlot` made in the run returned to
/// `FREE_PIPE_SLOTS`, then free the run's `PipeSlot` allocations so Shuttle
/// atomics do not leak across runs.
pub(crate) fn free_storages_for_model() -> (usize, usize, usize) {
    let free: Vec<&'static PipeSlot> = FREE_PIPE_SLOTS.lock().unwrap().drain(..).collect();
    let owned: Vec<OwnedPipeSlotPtr> = MODEL_OWNED_PIPE_SLOTS.lock().unwrap().drain(..).collect();
    let made = PIPE_SLOTS_MADE.swap(0, Ordering::SeqCst);
    let live = LIVE_STORAGES.swap(0, Ordering::SeqCst);
    let back = free.len();
    drop(free);
    for OwnedPipeSlotPtr(ptr) in owned {
        // Sound: the model's threads have all joined and `ptr` is the original
        // owning pointer from `Box::into_raw` in `PipeSlot::acquire`.
        unsafe { drop(Box::from_raw(ptr)) };
    }
    (made, back, live)
}
