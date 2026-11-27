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
    let assert_panic_contains = |name: &str, expected: &str, f: fn()| {
        let err = std::panic::catch_unwind(f).expect_err(&format!("{name} must panic"));
        let msg = err.downcast_ref::<String>().map(String::as_str).or_else(|| err.downcast_ref::<&str>().copied()).unwrap_or("");
        assert!(msg.contains(expected), "{name} panicked with {msg:?}, expected {expected:?}");
    };
    // Empty counter ranges must validate bounds first, then enforce initialize()
    // on both the free function and `TreeHasher::hash_counter_range`.
    assert_panic_contains("empty hash_counter_range (invalid template)", "fit in one block", || {
        blake3_servil_core::throughput::hash_counter_range(blake3_servil_core::Mode::Plain, &[0u8; 65], 0, 0, &mut []);
    });
    assert_panic_contains("empty hash_counter_range (invalid word)", "out of bounds", || {
        blake3_servil_core::throughput::hash_counter_range(blake3_servil_core::Mode::Plain, &[0u8; 4], 1, 0, &mut []);
    });
    assert_panic_contains("empty hash_counter_range (valid)", "initialize()", || {
        blake3_servil_core::throughput::hash_counter_range(blake3_servil_core::Mode::Plain, &[0u8; 4], 0, 0, &mut []);
    });
    assert_panic_contains("empty TreeHasher::hash_counter_range (invalid template)", "fit in one block", || {
        let tree = blake3_servil_core::throughput::TreeHasher::new(blake3_servil_core::Mode::Plain);
        tree.hash_counter_range(&[0u8; 65], 0, 0, &mut []);
    });
    assert_panic_contains("empty TreeHasher::hash_counter_range (invalid word)", "out of bounds", || {
        let tree = blake3_servil_core::throughput::TreeHasher::new(blake3_servil_core::Mode::Plain);
        tree.hash_counter_range(&[0u8; 4], 1, 0, &mut []);
    });
    assert_panic_contains("empty TreeHasher::hash_counter_range (valid)", "initialize()", || {
        let tree = blake3_servil_core::throughput::TreeHasher::new(blake3_servil_core::Mode::Plain);
        tree.hash_counter_range(&[0u8; 4], 0, 0, &mut []);
    });
    assert_panic_contains("TreeHasher::hash (short)", "initialize()", || {
        let tree = blake3_servil_core::throughput::TreeHasher::new(blake3_servil_core::Mode::Plain);
        let _ = tree.hash(&[0u8; 48]);
    });
    assert_panic_contains("TreeHasher::on_this_core().hash (short)", "initialize()", || {
        let tree = blake3_servil_core::throughput::TreeHasher::new(blake3_servil_core::Mode::Plain).on_this_core();
        let _ = tree.hash(&[0u8; 48]);
    });
}
