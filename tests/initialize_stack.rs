//! Production-library integration test for cold `initialize()` stack budget.
//!
//! Runs in its own process with `initialize()` never called beforehand,
//! invoking public `blake3_servil_core::initialize()` from a fresh 128 KiB
//! worker thread (matching BLINCS's stack-budget test, #509). Applies to
//! optimized builds (`[profile.test]` `opt-level = 3` and `--release`).

#[cfg(feature = "std")]
#[test]
fn cold_initialize_fits_in_128kib_thread_stack() {
    let expected = blake3::hash(b"cold-init-128k");
    let got = std::thread::Builder::new()
        .name("cold-init-128k".into())
        .stack_size(128 * 1024)
        .spawn(|| {
            blake3_servil_core::initialize();
            blake3_servil_core::convenience::hash(b"cold-init-128k")
        })
        .expect("spawn 128 KiB stack thread")
        .join()
        .expect("cold initialize() must not overflow a 128 KiB thread stack");
    assert_eq!(got.as_bytes(), expected.as_bytes());
}
