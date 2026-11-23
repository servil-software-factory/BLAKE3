//! A child module of src/pipeline.rs under `--cfg shuttle` alone
//! (tests/shuttle_pipeline.rs): it reaches the Pipeline's private storage
//! list to free a run's storages at the run's end.

use super::{FREE_STORAGES, MODEL_OWNED_STORAGES, Ordering, OwnedStoragePtr, PipeStorage, STORAGES_MADE};

/// At the end of one run of the model, every storage made in it is free
/// again (returned: how many made, how many free), and is freed here through
/// its original owning `*mut PipeStorage` allocation pointer (never casting a
/// shared `&PipeStorage` reference back to `*mut`). A program never frees one;
/// a model that ran thousands of schedules without freeing would hold them all,
/// and Shuttle's atomics must not pass from one run to the next.
pub(crate) fn free_storages_for_model() -> (usize, usize) {
    let free: Vec<&'static PipeStorage> = FREE_STORAGES.lock().unwrap().drain(..).collect();
    let owned: Vec<OwnedStoragePtr> = MODEL_OWNED_STORAGES.lock().unwrap().drain(..).collect();
    let made = STORAGES_MADE.swap(0, Ordering::SeqCst);
    let back = free.len();
    drop(free);
    for OwnedStoragePtr(st_ptr) in owned {
        // Sound: the model's threads have all ended, nothing holds a storage
        // that was on the free list, and `st_ptr` is the original owning
        // pointer from `Box::into_raw` in `PipeStorage::acquire`.
        unsafe {
            let st = &*st_ptr;
            let shared = *st.shared.get();
            if !shared.is_null() {
                drop(Box::from_raw(shared));
            }
            let (at, len) = st.rooms.backing;
            if !at.is_null() {
                drop(Box::from_raw(core::ptr::slice_from_raw_parts_mut(at, len)));
            }
            drop(Box::from_raw(st_ptr));
        }
    }
    (made, back)
}
