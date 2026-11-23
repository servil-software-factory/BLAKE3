// The contract's check, in a process of its own: a call before initialize()
// panics, in every build, saying what the caller must do. (Its first line
// names initialize, so tools/call_initialize.py leaves it as it is.)

#[test]
#[should_panic(expected = "call blake3_servil_core::initialize() once, before any other call")]
fn a_call_before_initialize_panics() {
    // Deliberately without initialize(): this test is the contract's check.
    blake3_servil_core::convenience::hash(b"");
}

#[cfg(feature = "std")]
#[test]
fn every_platform_constructor_before_initialize_panics() {
    use blake3_servil_core::thread_support::Platform;
    // Explicit Platform constructors must enforce initialize() too so safe
    // callers cannot obtain a Platform value before the self-test runs.
    let assert_panics = |name: &str, f: fn() -> Option<Platform>| {
        let err = std::panic::catch_unwind(f);
        assert!(err.is_err(), "{name} must panic before initialize()");
    };
    #[cfg(target_arch = "x86_64")]
    {
        assert_panics("Platform::sse41", Platform::sse41);
        assert_panics("Platform::avx2", Platform::avx2);
        assert_panics("Platform::avx512", Platform::avx512);
    }
    #[cfg(target_arch = "aarch64")]
    {
        assert_panics("Platform::neon", Platform::neon);
    }
}
