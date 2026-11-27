// The contract's check, in a process of its own: a call before initialize()
// panics, in every build, saying what the caller must do. (Its first line
// names initialize, so tools/call_initialize.py leaves it as it is.)

#[test]
#[should_panic(expected = "call blake3_servil_core::initialize() once, before any other call")]
fn a_call_before_initialize_panics() {
    // Deliberately without initialize(): this test is the contract's check.
    blake3_servil_core::convenience::hash(b"");
}

#[test]
fn every_platform_constructor_before_initialize_panics() {
    use blake3_servil_core::thread_support::Platform;
    // Public Platform accessors must enforce initialize() so safe callers
    // cannot obtain a Platform value before the self-test runs.
    let assert_panics = |name: &str, f: fn() -> Platform| {
        let err = std::panic::catch_unwind(f);
        assert!(err.is_err(), "{name} must panic before initialize()");
    };
    assert_panics("Platform::detect", Platform::detect);
    assert_panics("Platform::own_core", Platform::own_core);
    // Even an empty counter range must enforce initialize() (after validating
    // template/word bounds) before returning.
    let err = std::panic::catch_unwind(|| {
        blake3_servil_core::throughput::hash_counter_range(blake3_servil_core::Mode::Plain, &[0u8; 4], 0, 0, &mut []);
    });
    assert!(err.is_err(), "empty hash_counter_range must panic before initialize()");
}
