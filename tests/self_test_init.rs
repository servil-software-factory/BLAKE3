//! Production-library integration tests for `initialize()` publication.
//!
//! Unlike crate-internal unit tests (where `Platform::detect()` auto-initializes
//! under `cfg(test)`), this integration test links the shipped library where
//! observing an unpublished `DETECTED == 0` panics immediately.

#[cfg(feature = "std")]
#[test]
fn fast_path_initialize_publishes_platform_to_preexisting_worker() {
    use core::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    // Empty `finalize_each` must return cleanly even before `initialize()`.
    blake3_servil_core::throughput::finalize_each(&mut [], &[], &mut []);

    let expected = blake3::hash(b"fast-path-publication");
    let worker_ready = Arc::new(AtomicBool::new(false));
    let init_done = Arc::new(AtomicBool::new(false));

    let wr = Arc::clone(&worker_ready);
    let id = Arc::clone(&init_done);
    let handle = std::thread::spawn(move || {
        wr.store(true, Ordering::Release);
        // Relaxed spin intentionally supplies no synchronization of its own:
        // the worker relies solely on `initialize()`'s Acquire load on `STATE`
        // to synchronize with `DETECTED` publication.
        while !id.load(Ordering::Relaxed) {
            core::hint::spin_loop();
        }
        blake3_servil_core::initialize();
        let got = blake3_servil_core::convenience::hash(b"fast-path-publication");
        assert_eq!(got.as_bytes(), expected.as_bytes());
    });

    while !worker_ready.load(Ordering::Acquire) {
        std::thread::yield_now();
    }
    blake3_servil_core::initialize();
    init_done.store(true, Ordering::Relaxed);
    handle.join().expect("preexisting worker must observe published Platform after fast-path initialize()");
}
