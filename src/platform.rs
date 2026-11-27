use crate::{BLOCK_LEN, CHUNK_LEN, CVWords, IncrementCounter};

// The most chunks one call hashes: AVX-512's 32 on x86-64, SME2's 128 on
// AArch64. Every platform hashes at least four, so the walk's arrays
// (src/tree.rs) hold two values or more.
#[cfg(target_arch = "x86_64")]
pub const MAX_SIMD_DEGREE: usize = 32;
#[cfg(target_arch = "aarch64")]
// See sme2::DEGREE.
pub const MAX_SIMD_DEGREE: usize = crate::sme2::DEGREE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
/// Each platform's discriminant is fixed, the same in every build (0 means
/// "not yet recorded" to [`Platform::detect`]); tools/verify's whole-call
/// proofs start from the value `initialize` records.
///
/// A Platform is permission to run its kernels' instructions, so only
/// this crate makes one, and only after asking the CPU (`detect`,
/// `own_core`, a turn's, and the tests' `sse41()` to `sme2()`). Each
/// variant is `non_exhaustive`, which keeps another crate from writing
/// one: `Platform::AVX512` there does not compile (BLAKE3 #419: with it,
/// safe code ran AVX-512 or SME2 kernels on a CPU without them).
///
/// Another crate can ask which platform it was given:
///
/// ```
/// use blake3_servil_core::thread_support::Platform;
/// blake3_servil_core::initialize();
/// #[cfg(target_arch = "x86_64")]
/// let _ = matches!(Platform::detect(), Platform::AVX2 { .. });
/// #[cfg(target_arch = "aarch64")]
/// let _ = matches!(Platform::detect(), Platform::NEON { .. });
/// ```
///
/// and cannot make one (the same names, so this fails for that reason):
///
/// ```compile_fail,E0603
/// use blake3_servil_core::thread_support::Platform;
/// #[cfg(target_arch = "x86_64")]
/// let _ = Platform::AVX2;
/// #[cfg(target_arch = "aarch64")]
/// let _ = Platform::NEON;
/// ```
pub enum Platform {
    #[cfg(target_arch = "x86_64")]
    #[non_exhaustive]
    SSE41 = 3,
    #[cfg(target_arch = "x86_64")]
    #[non_exhaustive]
    AVX2 = 4,
    #[cfg(target_arch = "x86_64")]
    #[non_exhaustive]
    AVX512 = 5,
    #[cfg(target_arch = "aarch64")]
    #[non_exhaustive]
    NEON = 6,
    #[cfg(target_arch = "aarch64")]
    #[non_exhaustive]
    SME2 = 7,
}

/// One SME2 call at a time per process (all but always: two calls taking
/// the turn at the same instant can both get it): the turn a call takes
/// before it runs SME2 kernels, and the platform it runs on. A call finding
/// another SME2 call running takes NEON instead. Two SME2 threads of a process can
/// share one SME unit (macOS keeps a thread group on one P-cluster), each
/// at half speed or less, slower than NEON (n threads each hashing 1 MiB:
/// 0.170, 0.176, 0.322, 0.47 ns/B a thread on SME2 for n = 1, 2, 4, 8,
/// NEON 0.25 throughout); and SME2 calls running together sent the
/// process's other threads to E-cores on an M4 Max (4-10% of a sleeping
/// thread's following work; the turn took the benchmark's small cells'
/// E-core samples from 5.1% to none). Calls too small for the SME2 kernels
/// (`sme2_sized` false) leave the turn alone. The flag is a plain load and
/// store, no read-modify-write: two calls taking it at one instant can both
/// get it, which costs only what running SME2 at once always cost. Its
/// accepted costs (Zooko, September 2026): two programs sharing the machine
/// lose the cells where each held an SME unit, and their small batches
/// 6-16%.
pub struct Sme2Turn {
    platform: Platform,
    #[cfg(target_arch = "aarch64")]
    held: bool,
}

/// The turn's flag, on a 128-byte line of its own (Apple's cache line), so
/// no other state's writes invalidate it.
#[cfg(target_arch = "aarch64")]
#[repr(align(128))]
struct Busy(core::sync::atomic::AtomicBool);

#[cfg(target_arch = "aarch64")]
static SME2_BUSY: Busy = Busy(core::sync::atomic::AtomicBool::new(false));

impl Sme2Turn {
    #[inline]
    pub fn take(platform: Platform, sme2_sized: bool) -> Self {
        #[cfg(target_arch = "aarch64")]
        if sme2_sized && matches!(platform, Platform::SME2) {
            use core::sync::atomic::Ordering;
            // A load, and a store when the turn is free: two calls that
            // look at the same instant can both take it, which costs what
            // running SME2 at once always did, and nothing else depends on
            // the flag. An atomic swap cost 3-9% for a batch of 24
            // messages (on the VM, 282 ns against 273, and 9-10% in the
            // regression check); acquire and release ordering about 1 µs
            // per call beside the SME2 kernels (256 one-block messages:
            // 3.43 µs against 2.50).
            let held = !SME2_BUSY.0.load(Ordering::Relaxed);
            if held {
                SME2_BUSY.0.store(true, Ordering::Relaxed);
            } else {
                crate::ran(crate::Ran::TurnTaken);
            }
            return Sme2Turn { platform: if held { Platform::SME2 } else { Platform::NEON }, held };
        }
        let _ = sme2_sized;
        Sme2Turn {
            platform,
            #[cfg(target_arch = "aarch64")]
            held: false,
        }
    }

    #[inline]
    pub fn platform(&self) -> Platform {
        self.platform
    }
}

impl Drop for Sme2Turn {
    #[inline]
    fn drop(&mut self) {
        #[cfg(target_arch = "aarch64")]
        if self.held {
            SME2_BUSY.0.store(false, core::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// What this process has detected. The low byte: the platform `initialize`
/// detected, as its discriminant (1 and up), or 0 before it has run. Bits 8
/// and 9: whether the cores are Apple's, and whether that has been asked
/// (`apple_cores_cached`: AArch64's kernel plans differ by it, the self-test's
/// too, which runs before the platform is recorded). One word, private to this
/// module so no other module can corrupt the low byte that licenses unsafe
/// instructions, while keeping both answers on one cache line (#395).
static DETECTED: core::sync::atomic::AtomicU16 = core::sync::atomic::AtomicU16::new(0);

/// Read the high-byte auxiliary cache bits (`bits 8..15`) of `DETECTED`
/// without exposing the low platform byte.
#[cfg(all(target_arch = "aarch64", not(target_vendor = "apple")))]
#[inline]
pub(crate) fn high_bits() -> u16 {
    DETECTED.load(core::sync::atomic::Ordering::Relaxed) & 0xff00
}

/// Set auxiliary cache bits inside the high byte (`bits 8..15`) of `DETECTED`
/// without ever touching the low platform byte that licenses unsafe kernels.
#[cfg(all(target_arch = "aarch64", not(target_vendor = "apple")))]
#[inline]
pub(crate) fn set_high_bits(bits: u16) {
    assert_eq!(bits & 0x00ff, 0, "high-byte cache write must not touch low platform byte");
    DETECTED.fetch_or(bits, core::sync::atomic::Ordering::Relaxed);
}

#[cfg(test)]
pub(crate) fn seed_high_bits_for_test(bits: u16) {
    assert_eq!(bits & 0x00ff, 0, "high-bit test seed must not touch low platform byte");
    DETECTED.store(bits, core::sync::atomic::Ordering::Release);
}

#[cfg(test)]
pub(crate) fn high_bits_for_test() -> u16 {
    DETECTED.load(core::sync::atomic::Ordering::Acquire) & 0xff00
}

/// DETECTED's word with `platform` in its low byte and its other bits as
/// they were.
const fn with_platform(detected: u16, platform: u8) -> u16 {
    (detected & 0xff00) + platform as u16
}

/// Whether `word` (loaded from `DETECTED`) holds a recorded platform in its
/// low byte, independent of any high-byte family/width cache bits.
#[inline]
pub(crate) const fn is_initialized_word(word: u16) -> bool {
    crate::low_byte_u16(word) != 0
}

/// Whether [`crate::initialize`] has finished recording the process platform.
/// `Acquire` pairs with `Platform::record`'s `Release` so callers on weak-
/// memory CPUs observe the recorded platform before proceeding.
#[inline]
pub(crate) fn is_initialized() -> bool {
    is_initialized_word(DETECTED.load(core::sync::atomic::Ordering::Acquire))
}

/// A call before `initialize`: the caller broke the contract. The crate's
/// own unit tests, many small programs in one, initialize here instead.
#[cold]
#[inline(never)]
fn not_initialized() -> u8 {
    #[cfg(test)]
    {
        crate::initialize();
        DETECTED.load(core::sync::atomic::Ordering::Relaxed) as u8
    }
    #[cfg(not(test))]
    panic!("blake3-servil: call blake3_servil_core::initialize() once, before any other call: it runs the startup self-test and detects the CPU")
}

/// x86-64's kernels for a CPU with the given features: the widest whose full
/// prerequisite chain is present (`SSSE3+SSE4.1` <= `AVX2` <= `AVX-512`), or
/// None for a CPU without SSSE3 + SSE4.1, which is not supported.
#[cfg(target_arch = "x86_64")]
const fn x86_platform(avx512: bool, avx2: bool, sse41: bool) -> Option<Platform> {
    if sse41 && avx2 && avx512 {
        Some(Platform::AVX512)
    } else if sse41 && avx2 {
        Some(Platform::AVX2)
    } else if sse41 {
        Some(Platform::SSE41)
    } else {
        None
    }
}

impl Platform {
    #[cfg(test)]
    #[inline]
    fn require_initialized() {
        if !is_initialized() {
            let _ = not_initialized();
        }
    }
    /// The platform [`initialize`](crate::initialize) detected for this
    /// process. The caller must have called `initialize` (or
    /// `initialize_multithreaded`) before: a call before it panics, in every
    /// build. The check costs nothing extra: it is the load of the platform,
    /// which every call makes anyway.
    #[inline]
    pub fn detect() -> Self {
        let mut code = crate::low_byte_u16(DETECTED.load(core::sync::atomic::Ordering::Relaxed));
        if code == 0 {
            code = not_initialized();
        }
        // Sound: DETECTED's low byte holds 0 or a Platform's discriminant (`record`).
        unsafe { core::mem::transmute::<u8, Platform>(code) }
    }

    /// The detected platform's kernels that are a core's own: on a CPU
    /// whose one SME2 accelerator serves every core, the NEON hybrids,
    /// which no other core's work slows down; elsewhere the detected
    /// platform (throughput::TreeHasher::on_this_core, and the pool's pieces).
    #[inline]
    pub fn own_core() -> Self {
        let detected = Self::detect();
        #[cfg(target_arch = "aarch64")]
        if matches!(detected, Platform::SME2) {
            return Platform::NEON;
        }
        detected
    }

    /// Make `self` what [`Platform::detect`] returns from now on, in every
    /// thread: `initialize` does, once its self-test has passed.
    pub(crate) fn record(self) {
        // The other bits stay (the cores, asked by now or later). `Release`
        // pairs with `is_initialized()`'s `Acquire` load.
        use core::sync::atomic::Ordering::{Relaxed, Release};
        let mut detected = DETECTED.load(Relaxed);
        while let Err(now) = DETECTED.compare_exchange_weak(detected, with_platform(detected, self as u8), Release, Relaxed) {
            detected = now;
        }
    }

    /// [`Platform::detect`] without the startup self-test: an assembly
    /// platform on every supported machine, or the process stops.
    #[allow(unreachable_code)]
    pub(crate) fn detect_unchecked() -> Self {
        // Every x86-64 build assembles every kernel (build.rs), so a CPU
        // runs the widest it has; one without SSE4.1 is not supported, and
        // the process stops here, in the startup self-test, as a build for
        // an unsupported target stops (official BLAKE3 runs there).
        #[cfg(target_arch = "x86_64")]
        match x86_platform(avx512_detected(), avx2_detected(), sse41_detected()) {
            Some(platform) => return platform,
            None => crate::self_test::unsupported(format_args!(
                "blake3-servil runs on x86-64 CPUs with SSSE3 and SSE4.1 (Intel since 2008, AMD since 2011); this CPU lacks them: use official BLAKE3 (https://github.com/BLAKE3-team/BLAKE3)"
            )),
        }
        // SME2 is detected at runtime; the kernels also require a 512-bit
        // streaming vector length, which sme2_detected() checks. Without
        // SME2 the NEON backend (integer + vector hybrid kernels) is the
        // selection, and it is a real result in its own right; a benchmark
        // that cares which one ran reads it back from detect().
        #[cfg(target_arch = "aarch64")]
        {
            if sme2_detected() {
                return Platform::SME2;
            }
        }
        // NEON is mandatory on AArch64.
        #[cfg(target_arch = "aarch64")]
        {
            return Platform::NEON;
        }
        unreachable!("build.rs builds for x86-64 and AArch64 alone")
    }

    /// The selection's name, for reports.
    pub fn name(&self) -> &'static str {
        match self {
            #[cfg(target_arch = "x86_64")]
            Platform::SSE41 => "SSE4.1",
            #[cfg(target_arch = "x86_64")]
            Platform::AVX2 => "AVX2",
            #[cfg(target_arch = "x86_64")]
            Platform::AVX512 => "AVX-512",
            #[cfg(target_arch = "aarch64")]
            Platform::NEON => "NEON",
            #[cfg(target_arch = "aarch64")]
            Platform::SME2 => "SME2",
        }
    }

    pub fn simd_degree(&self) -> usize {
        #[cfg(target_arch = "x86_64")]
        const SSE41_DEGREE: usize = 4;
        #[cfg(target_arch = "x86_64")]
        const AVX2_DEGREE: usize = 8;
        #[cfg(target_arch = "x86_64")]
        const AVX512_DEGREE: usize = 32;
        #[cfg(target_arch = "aarch64")]
        const NEON_DEGREE: usize = 16;
        #[cfg(target_arch = "x86_64")]
        const {
            assert!(SSE41_DEGREE >= 2 && SSE41_DEGREE <= MAX_SIMD_DEGREE && SSE41_DEGREE.is_power_of_two());
            assert!(AVX2_DEGREE >= 2 && AVX2_DEGREE <= MAX_SIMD_DEGREE && AVX2_DEGREE.is_power_of_two());
            assert!(AVX512_DEGREE >= 2 && AVX512_DEGREE <= MAX_SIMD_DEGREE && AVX512_DEGREE.is_power_of_two());
        }
        #[cfg(target_arch = "aarch64")]
        const {
            assert!(NEON_DEGREE >= 2 && NEON_DEGREE <= MAX_SIMD_DEGREE && NEON_DEGREE.is_power_of_two());
            assert!(crate::sme2::DEGREE >= 2 && crate::sme2::DEGREE <= MAX_SIMD_DEGREE && crate::sme2::DEGREE.is_power_of_two());
        }
        match self {
            #[cfg(target_arch = "x86_64")]
            Platform::SSE41 => SSE41_DEGREE,
            #[cfg(target_arch = "x86_64")]
            Platform::AVX2 => AVX2_DEGREE,
            #[cfg(target_arch = "x86_64")]
            Platform::AVX512 => AVX512_DEGREE,
            // Sixteen chunks per call reach the k10 kernel (see
            // neon_hybrid), in both its families.
            #[cfg(target_arch = "aarch64")]
            Platform::NEON => NEON_DEGREE,
            #[cfg(target_arch = "aarch64")]
            Platform::SME2 => crate::sme2::DEGREE,
        }
    }

    #[inline(always)]
    fn compress_in_place_raw(&self, cv: &mut CVWords, block: &[u8; BLOCK_LEN], block_len: u8, counter: u64, flags: u8) {
        match self {
            // Sound: `Platform::SSE41`/`AVX2` is constructed only after CPUID verification.
            #[cfg(target_arch = "x86_64")]
            Platform::SSE41 | Platform::AVX2 => unsafe { crate::sse41::compress_in_place(cv, block, block_len, counter, flags) },
            // Sound: `Platform::AVX512` is constructed only after CPUID + XCR0 verification.
            #[cfg(target_arch = "x86_64")]
            Platform::AVX512 => unsafe { crate::avx512::compress_in_place(cv, block, block_len, counter, flags) },
            // The scalar kernel k1 keeps the state in registers; it runs on
            // every AArch64 core, so the NEON and SME2 platforms share it.
            #[cfg(target_arch = "aarch64")]
            Platform::NEON => crate::neon_hybrid::compress_in_place(cv, block, block_len, counter, flags),
            #[cfg(target_arch = "aarch64")]
            Platform::SME2 => crate::neon_hybrid::compress_in_place(cv, block, block_len, counter, flags),
        }
    }

    pub(crate) fn compress_in_place(&self, cv: &mut CVWords, block: &[u8; BLOCK_LEN], block_len: u8, counter: u64, flags: u8) {
        crate::counted(crate::Counted::CompressInPlace);
        self.compress_in_place_raw(cv, block, block_len, counter, flags);
    }

    /// Compress `count` whole blocks of one chunk into `cv`: the first block
    /// with `flags | flags_start`, the rest with `flags`, all with `counter`.
    /// `blocks` holds at least `count * BLOCK_LEN` bytes; `count` is 1..=16.
    pub(crate) fn compress_blocks(&self, cv: &mut CVWords, blocks: &[u8], count: usize, counter: u64, flags: u8, flags_start: u8) {
        crate::counted(crate::Counted::CompressBlocks);
        assert!((1..=16).contains(&count));
        assert!(blocks.len() >= count * BLOCK_LEN);
        // The integer-only kernel runs on every AArch64 core.
        #[cfg(target_arch = "aarch64")]
        {
            // Sound: `count` is asserted in `1..=16` and `blocks` holds at least `count * BLOCK_LEN` readable bytes.
            unsafe {
                crate::neon_hybrid::compress_blocks(cv, blocks.as_ptr(), count, counter, flags, flags_start);
            }
            return;
        }
        #[allow(unreachable_code)]
        {
            let mut block_flags = flags | flags_start;
            for block in blocks[..count * BLOCK_LEN].chunks_exact(BLOCK_LEN) {
                self.compress_in_place_raw(cv, block.try_into().unwrap(), u8::try_from(BLOCK_LEN).unwrap(), counter, block_flags);
                block_flags = flags;
            }
        }
    }

    pub(crate) fn compress_xof(&self, cv: &CVWords, block: &[u8; BLOCK_LEN], block_len: u8, counter: u64, flags: u8) -> [u8; 64] {
        match self {
            // Sound: `Platform::SSE41`/`AVX2` is constructed only after CPUID verification.
            #[cfg(target_arch = "x86_64")]
            Platform::SSE41 | Platform::AVX2 => unsafe { crate::sse41::compress_xof(cv, block, block_len, counter, flags) },
            // Sound: `Platform::AVX512` is constructed only after CPUID + XCR0 verification.
            #[cfg(target_arch = "x86_64")]
            Platform::AVX512 => unsafe { crate::avx512::compress_xof(cv, block, block_len, counter, flags) },
            // The one-block extended-output kernel (neon_hybrid::xof_block).
            #[cfg(target_arch = "aarch64")]
            Platform::NEON => crate::neon_hybrid::xof_block(cv, block, block_len, counter, flags),
            #[cfg(target_arch = "aarch64")]
            Platform::SME2 => crate::neon_hybrid::xof_block(cv, block, block_len, counter, flags),
        }
    }

    // IMPLEMENTATION NOTE
    // ===================
    // hash_many() applies two optimizations. The critically important
    // optimization is the high-performance parallel SIMD hashing mode,
    // described in detail in the spec. This more than doubles throughput per
    // thread. Another optimization is keeping the state vectors transposed
    // from block to block within a chunk. When state vectors are transposed
    // after every block, there's a small but measurable performance loss.
    // Compressing chunks with a dedicated loop avoids this.

    pub(crate) fn hash_many<const N: usize>(
        &self,
        inputs: &[&[u8; N]],
        key: &CVWords,
        counter: u64,
        increment_counter: IncrementCounter,
        flags: u8,
        flags_start: u8,
        flags_end: u8,
        out: &mut [u8],
    ) {
        // Every kernel hashes whole blocks: an input of another length
        // would lose its tail, so it cannot compile.
        const { assert!(N > 0 && N % BLOCK_LEN == 0, "hash_many inputs are whole blocks") };
        // The shapes the crate makes, which every kernel takes: whole chunks
        // at counters counting up, or 1 to 16 whole blocks at one counter.
        assert!(
            if increment_counter.yes() { N == CHUNK_LEN } else { N <= CHUNK_LEN },
            "hash_many takes whole chunks at incrementing counters, or 1 to 16 blocks at one counter"
        );
        #[cfg(test)]
        {
            crate::counted(crate::Counted::HashManyCall);
            crate::record_hash_many_call(N / BLOCK_LEN, inputs.len());
        }
        match self {
            // Sound: `Platform::SSE41` is constructed only after CPUID verification.
            #[cfg(target_arch = "x86_64")]
            Platform::SSE41 => unsafe { crate::sse41::hash_many(inputs, key, counter, increment_counter, flags, flags_start, flags_end, out) },
            // Sound: `Platform::AVX2` is constructed only after CPUID + XCR0 verification.
            #[cfg(target_arch = "x86_64")]
            Platform::AVX2 => unsafe { crate::avx2::hash_many(inputs, key, counter, increment_counter, flags, flags_start, flags_end, out) },
            // Sound: `Platform::AVX512` is constructed only after CPUID + XCR0 verification.
            #[cfg(target_arch = "x86_64")]
            Platform::AVX512 => unsafe { crate::avx512::hash_many(inputs, key, counter, increment_counter, flags, flags_start, flags_end, out) },
            // Sound: `Platform::NEON` is built only on AArch64 where NEON is baseline.
            #[cfg(target_arch = "aarch64")]
            Platform::NEON => unsafe { crate::neon_hybrid::hash_many(inputs, key, counter, increment_counter, flags, flags_start, flags_end, out) },
            // Sound: startup verified FEAT_SME2; SME2 assembly checks the calling thread's SVL at entry and falls back to NEON if != 512 bits.
            #[cfg(target_arch = "aarch64")]
            Platform::SME2 => unsafe { crate::sme2::hash_many(inputs, key, counter, increment_counter, flags, flags_start, flags_end, out) },
        }
    }

    pub(crate) fn xof_many(&self, cv: &CVWords, block: &[u8; BLOCK_LEN], block_len: u8, mut counter: u64, flags: u8, out: &mut [u8]) {
        assert_eq!(0, out.len() % BLOCK_LEN, "whole blocks only");
        if out.is_empty() {
            // The current assembly implementation always outputs at least 1 block.
            return;
        }
        match self {
            // Sound: `Platform::AVX512` is constructed only after CPUID + XCR0 verification.
            #[cfg(target_arch = "x86_64")]
            Platform::AVX512 => unsafe { crate::avx512::xof_many(cv, block, block_len, counter, flags, out) },
            _ => {
                // On SME2, whole groups of sixteen blocks on the extended-
                // output kernel, under the process's turn; the rest below.
                #[cfg(target_arch = "aarch64")]
                let out = {
                    let turn = Sme2Turn::take(*self, out.len() >= 16 * BLOCK_LEN);
                    let done = if matches!(turn.platform(), Platform::SME2) {
                        // Sound: startup verified FEAT_SME2; SME2 xof_many checks the calling thread's SVL at entry and declines if != 512 bits.
                        unsafe { crate::sme2::xof_many(cv, block, block_len, counter, flags, out) }
                    } else {
                        0
                    };
                    counter += (done / BLOCK_LEN) as u64;
                    &mut out[done..]
                };
                // Then four or eight at a time on NEON (AArch64): CPUs
                // without SME2, and SME2's leftovers; the last one to three
                // blocks after them (neon_hybrid::xof_tail).
                #[cfg(target_arch = "aarch64")]
                {
                    let done = crate::neon_hybrid::xof_many(cv, block, block_len, counter, flags, out);
                    if done < out.len() {
                        crate::neon_hybrid::xof_tail(cv, block, block_len, counter.wrapping_add((done / BLOCK_LEN) as u64), flags, &mut out[done..]);
                    }
                }
                // SSE4.1 and AVX2, without an xof_many kernel: block by block.
                #[cfg(target_arch = "x86_64")]
                for out_array in out.as_chunks_mut::<BLOCK_LEN>().0 {
                    *out_array = self.compress_xof(cv, block, block_len, counter, flags);
                    counter += 1;
                }
            }
        }
    }
}

// Explicit platform constructors for in-crate unit tests. Placed in a
// `#[cfg(test)]` impl block (with separate per-arch cfgs) so production code
// never sees them and cargo-mutants excludes test-only helpers cleanly.
#[cfg(test)]
impl Platform {
    /// The SSE41 platform where this build has its kernels and the CPU its
    /// instructions.
    #[cfg(target_arch = "x86_64")]
    pub(crate) fn sse41() -> Option<Self> {
        Self::require_initialized();
        if sse41_detected() {
            return Some(Self::SSE41);
        }
        None
    }

    /// The AVX2 platform where this build has its kernels and the CPU its
    /// instructions.
    #[cfg(target_arch = "x86_64")]
    pub(crate) fn avx2() -> Option<Self> {
        Self::require_initialized();
        if avx2_detected() {
            return Some(Self::AVX2);
        }
        None
    }

    #[cfg(target_arch = "x86_64")]
    pub(crate) fn avx512() -> Option<Self> {
        Self::require_initialized();
        if avx512_detected() { Some(Self::AVX512) } else { None }
    }

    #[cfg(target_arch = "aarch64")]
    pub(crate) fn neon() -> Option<Self> {
        Self::require_initialized();
        Some(Self::NEON)
    }

    #[cfg(target_arch = "aarch64")]
    pub(crate) fn sme2() -> Option<Self> {
        // Issue Platform::SME2 only when the startup self-test actually tested
        // SME2 (recorded as the process platform) and this thread's SVL is 512 bits.
        if sme2_allowed(Self::detect(), sme2_detected()) { Some(Self::SME2) } else { None }
    }
}

#[cfg(test)]
#[cfg(target_arch = "aarch64")]
#[inline(always)]
const fn sme2_allowed(detected: Platform, thread_ok: bool) -> bool {
    matches!(detected, Platform::SME2) && thread_ok
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
const fn sme2_svl_ok(lanes: u64) -> bool {
    lanes == crate::sme2::GROUP as u64
}

/// The x86-64 features the platforms check, as bits: [`SSE41`], [`AVX2`],
/// [`AVX512`] (F and VL, the two AVX-512 featuresets the kernels use), and
/// [`READ`] once the bits are read. Every check reads them from CPUID once
/// per process: CPUID costs about 640 ns (i7-12700K), and cpufeatures read
/// three leaves per feature checked, six on an AVX2 machine's first
/// detection where two serve.
#[cfg(target_arch = "x86_64")]
mod x86_bits {
    pub const SSE41: u8 = 2;
    pub const AVX2: u8 = 4;
    pub const AVX512: u8 = 8;
    /// AVX-512 on a CPU where the 32-input kernels pay ([`super::wide_cpu`]).
    pub const WIDE: u8 = 16;
    pub const READ: u8 = 128;
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
// Read by the x86 kernels' dispatch and the tests.
fn x86_features() -> u8 {
    use core::sync::atomic::{AtomicU8, Ordering};
    static FEATURES: AtomicU8 = AtomicU8::new(0);
    let features = FEATURES.load(Ordering::Relaxed);
    if features != 0 {
        return features;
    }
    let features = read_x86_features();
    FEATURES.store(features, Ordering::Relaxed);
    features
}

/// What CPUID and XCR0 say of this CPU, as the words [`features_of`]
/// decides by. Reading is all this does: every decision but one is
/// `features_of`'s, a function of these words alone, which its test tries
/// on every CPU the words can describe. The one decision here is the
/// order of reading: XCR0 and leaf 7 are read only where leaf 1 says the
/// OS saves AVX's registers ([`os_saves_avx`]); XGETBV faults elsewhere.
#[cfg(target_arch = "x86_64")]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct CpuWords {
    /// CPUID leaf 1's EAX (the family) and ECX (SSE4.1, XSAVE, OSXSAVE, AVX).
    leaf1_eax: u32,
    leaf1_ecx: u32,
    /// XCR0's low half: the register sets the OS saves. Zero where unread.
    xcr0: u32,
    /// CPUID leaf 7's EBX (AVX2, AVX-512 F and VL). Zero where unread.
    leaf7_ebx: u32,
    /// The vendor is "AuthenticAMD" (leaf 0). False where unread.
    amd: bool,
}

/// Leaf 1's ECX says the CPU has XSAVE and AVX and the OS enabled XGETBV
/// (OSXSAVE): bits 26, 27 and 28.
#[cfg(target_arch = "x86_64")]
const fn os_saves_avx(leaf1_ecx: u32) -> bool {
    let avx = (1 << 26) | (1 << 27) | (1 << 28);
    leaf1_ecx & avx == avx
}

/// Whether CPUID reports basic or extended `leaf` as supported given the
/// corresponding maximum leaf (`leaf0.eax` for basic leaves, `0x8000_0000.eax`
/// for extended leaves).
#[cfg(target_arch = "x86_64")]
pub(crate) const fn cpuid_has_leaf(max_leaf: u32, leaf: u32) -> bool {
    max_leaf >= leaf
}

#[cfg(target_arch = "x86_64")]
#[cold]
fn read_cpu_words() -> CpuWords {
    use core::arch::x86_64 as arch;
    let leaf0 = arch::__cpuid(0);
    if !cpuid_has_leaf(leaf0.eax, 1) {
        return CpuWords::default();
    }
    let leaf1 = arch::__cpuid(1);
    let mut words = CpuWords { leaf1_eax: leaf1.eax, leaf1_ecx: leaf1.ecx, ..CpuWords::default() };
    if os_saves_avx(leaf1.ecx) {
        // Sound: OSXSAVE (bit 27) says the OS enabled XGETBV.
        words.xcr0 = u32::from_le_bytes(unsafe { arch::_xgetbv(0) }.to_le_bytes()[..4].try_into().unwrap());
        words.amd = (leaf0.ebx, leaf0.edx, leaf0.ecx) == (0x6874_7541, 0x6974_6e65, 0x444d_4163); // "AuthenticAMD"
        if cpuid_has_leaf(leaf0.eax, 7) {
            words.leaf7_ebx = arch::__cpuid_count(7, 0).ebx;
        }
    }
    words
}

/// [`x86_features`] of a CPU, from its words: SSSE3 + SSE4.1 by leaf 1;
/// AVX2 (requiring SSSE3 + SSE4.1) and AVX-512 F+VL (requiring AVX2) by
/// leaf 7 where the CPU has AVX and the OS saves the registers each needs
/// (XSAVE, OSXSAVE, AVX, then XCR0's YMM bits, and for AVX-512 its opmask
/// and ZMM bits too); the wide kernels' bit with AVX-512 on the CPUs
/// [`wide_cpu`] names. No reading, no state: its test runs it on every
/// combination of the bits it looks at.
#[cfg(target_arch = "x86_64")]
const fn features_of(words: CpuWords) -> u8 {
    use x86_bits::*;
    let mut features = READ;
    let sse41_bits = (1 << 9) | (1 << 19);
    let has_sse41 = words.leaf1_ecx & sse41_bits == sse41_bits;
    if has_sse41 {
        features |= SSE41;
    }
    if has_sse41 && os_saves_avx(words.leaf1_ecx) {
        let (ymm, zmm) = (0b110, 0b1110_0110);
        let has_avx2 = words.xcr0 & ymm == ymm && words.leaf7_ebx & (1 << 5) != 0;
        if has_avx2 {
            features |= AVX2;
            let f_vl = (1 << 16) | (1 << 31);
            if words.xcr0 & zmm == zmm && words.leaf7_ebx & f_vl == f_vl {
                features |= AVX512;
                if wide_cpu(words.amd, family(words.leaf1_eax)) {
                    features |= WIDE;
                }
            }
        }
    }
    features
}

/// [`x86_features`] from CPUID: [`features_of`] this CPU's words.
#[cfg(target_arch = "x86_64")]
#[cold]
// Read by the x86 kernels' dispatch and the tests.
fn read_x86_features() -> u8 {
    features_of(read_cpu_words())
}

/// The display family of CPUID leaf 1's EAX: the base family, plus the
/// extended family where the base is 0xF.
#[cfg(target_arch = "x86_64")]
const fn family(eax: u32) -> u32 {
    let base = (eax >> 8) & 0xF;
    if base == 0xF { base + ((eax >> 20) & 0xFF) } else { base }
}

/// Whether the 32-input AVX-512 kernels pay on this CPU: AMD from family
/// 0x1A (Zen 5). Measured against the 16-input kernel for one thread on
/// inputs of 32 KiB and more (BLAKE3 #125): about 20% less time on Zen 5
/// (EPYC 9255), but on Zen 4 (EPYC 9124, family 0x19) 4-5% more from
/// memory, and on Intel Ice Lake (Xeon Gold 6330) 3-4.5% more. Other
/// CPUs join the list when measured.
#[cfg(target_arch = "x86_64")]
const fn wide_cpu(amd: bool, family: u32) -> bool {
    amd && family >= 0x1A
}

/// Whether `hash_many` dispatches to the 32-input AVX-512 kernels
/// (Linux): AVX-512 on a CPU where they pay ([`wide_cpu`]).
#[cfg(target_arch = "x86_64")]
#[inline(always)]
pub fn wide_avx512() -> bool {
    // The bit comes only with AVX-512 (read_x86_features).
    x86_features() & x86_bits::WIDE != 0
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
pub fn avx512_detected() -> bool {
    x86_features() & x86_bits::AVX512 != 0
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
// Read by the x86 kernels' dispatch and the tests.
pub fn avx2_detected() -> bool {
    x86_features() & x86_bits::AVX2 != 0
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
// Read by the x86 kernels' dispatch and the tests.
pub fn sse41_detected() -> bool {
    x86_features() & x86_bits::SSE41 != 0
}

/// Prefetch code from `start` to `end` into the L2 cache, a line (128
/// bytes on Apple's cores) at a time. A call after other work otherwise
/// fetches its kernel's lines from DRAM one miss after another: hash() of
/// 8 KiB took 4.8 us so on an M4 Max, and 2.8 with this prefetch (job 971),
/// where code already near costs it a few cycles a line.
#[cfg(target_arch = "aarch64")]
#[inline]
pub(crate) fn prefetch_code((start, end): (usize, usize)) {
    // Debug-only: `prfm` never faults and is skipped when `start >= end`; this
    // sanity-checks symbol-pair bounds passed by our own code-range helpers.
    debug_assert!(start < end && end - start < 64 * 1024, "a kernel's code");
    // Four lines a step (past `end` by up to three: harmless).
    let mut line = start;
    #[cfg(test)]
    let mut steps = 0;
    while line < end {
        #[cfg(test)]
        {
            steps += 1;
            // A loop that never ends (a mutant's) fails here, at once, not by the clock.
            assert!(steps < 1 << 16, "prefetch_code without end");
        }
        // Sound: a prefetch never faults, whatever the address.
        unsafe {
            core::arch::asm!(
                "prfm pldl2keep, [{0}]",
                "prfm pldl2keep, [{0}, #128]",
                "prfm pldl2keep, [{0}, #256]",
                "prfm pldl2keep, [{0}, #384]",
                in(reg) line,
                options(nostack, readonly, preserves_flags),
            )
        };
        line += 512;
    }
    #[cfg(test)]
    let _ = PREFETCHED.try_with(|p| {
        // Recorded only while a counting test asks (test::prefetch_counts): the
        // other tests prefetch without end, as calls do.
        if let Some(p) = &mut *p.borrow_mut() {
            // A plan that never ends (a mutant's) fails here, not by taking every byte of memory.
            assert!(p.len() < 100_000, "prefetch_code called without end");
            p.push(((start, end), steps));
        }
    });
}

#[cfg(all(test, target_arch = "aarch64"))]
std::thread_local! {
    /// The tests' record of prefetch_code on this thread since they last
    /// took it, while one asks (Some): each range, and the steps of four
    /// lines it took. None, as in every other test, records nothing.
    pub(crate) static PREFETCHED: core::cell::RefCell<Option<std::vec::Vec<((usize, usize), usize)>>> = const { core::cell::RefCell::new(None) };
}

// The little-endian words of a block or a key, and back: 4-byte chunks
// (as_chunks), with no index arithmetic to get wrong (each unrolled form's
// 16 lines of `bytes[i * 4..]` were 16 places for a mutant; October 26).
#[inline(always)]
pub fn words_from_le_bytes_32(bytes: &[u8; 32]) -> [u32; 8] {
    let mut out = [0; 8];
    for (word, chunk) in out.iter_mut().zip(bytes.as_chunks::<4>().0) {
        *word = u32::from_le_bytes(*chunk);
    }
    out
}

/// True when the CPU reports SME2 and the calling thread's streaming vector
/// length is 512 bits, the only length the SME2 kernels support. Detection is
/// per platform: `hw.optional.arm.FEAT_SME2` on Apple systems and
/// `HWCAP2_SME2` on Linux.
///
/// The hardware check is cached once per process. On Apple silicon the
/// streaming vector length is fixed hardware-wide and cached with it; on
/// Linux SVL is per-thread (`prctl(PR_SME_SET_VL)`), so the calling thread's
/// SVL is queried via side-effect-free `rdsvl` (without entering streaming
/// mode or touching ZA).
#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub fn sme2_detected() -> bool {
    use core::sync::atomic::{AtomicU8, Ordering};

    // 0 = unknown, 1 = no, 2 = yes.
    static CACHE: AtomicU8 = AtomicU8::new(0);

    let cached = match CACHE.load(Ordering::Relaxed) {
        2 => true,
        1 => false,
        _ => {
            let detected = sme2_detect_uncached();
            CACHE.store(if detected { 2 } else { 1 }, Ordering::Relaxed);
            detected
        }
    };
    // Sound: `cached` is true only after `sme2_reported()` confirmed HWCAP2_SME2, so `rdsvl` is legal.
    #[cfg(target_os = "linux")]
    return if cached { sme2_svl_ok(unsafe { crate::sme2::svl_lanes() }) } else { false };
    #[cfg(not(target_os = "linux"))]
    cached
}

#[cfg(target_arch = "aarch64")]
fn sme2_detect_uncached() -> bool {
    if !sme2_reported() {
        return false;
    }

    #[cfg(target_os = "linux")]
    return true;
    #[cfg(not(target_os = "linux"))]
    {
        // Sound: `sme2_reported()` confirmed FEAT_SME2 above; side-effect-free `rdsvl` never enters streaming mode or touches ZA.
        sme2_svl_ok(unsafe { crate::sme2::svl_lanes() })
    }
}

#[cfg(all(target_arch = "aarch64", target_vendor = "apple"))]
fn sme2_reported() -> bool {
    use core::ffi::{c_char, c_int, c_void};
    #[link(name = "c")]
    unsafe extern "C" {
        fn sysctlbyname(name: *const c_char, oldp: *mut c_void, oldlenp: *mut usize, newp: *mut c_void, newlen: usize) -> c_int;
    }
    let mut value: u32 = 0;
    let mut size = core::mem::size_of::<u32>();
    // Sound: `c"hw.optional.arm.FEAT_SME2"` is NUL-terminated, `value` and
    // `size` are live writable stack storage for a 4-byte sysctl query, and
    // `newp` is null with length 0.
    let rc = unsafe { sysctlbyname(c"hw.optional.arm.FEAT_SME2".as_ptr(), (&raw mut value).cast::<c_void>(), &raw mut size, core::ptr::null_mut(), 0) };
    rc == 0 && value != 0
}

#[cfg(all(target_arch = "aarch64", target_os = "linux"))]
fn sme2_reported() -> bool {
    use core::ffi::c_ulong;
    #[cfg_attr(all(target_env = "musl", target_feature = "crt-static"), link(name = "c", kind = "static", modifiers = "-bundle"))]
    #[cfg_attr(not(all(target_env = "musl", target_feature = "crt-static")), link(name = "c"))]
    unsafe extern "C" {
        fn getauxval(kind: c_ulong) -> c_ulong;
    }
    // AT_HWCAP2 from <elf.h> and HWCAP2_SME2 from <asm/hwcap.h> (Linux 6.4+).
    const AT_HWCAP2: c_ulong = 26;
    const HWCAP2_SME2: c_ulong = 1 << 37;
    // Sound: `getauxval` reads the immutable auxiliary vector without dereferencing caller pointers.
    let hwcap2 = unsafe { getauxval(AT_HWCAP2) };
    hwcap2 & HWCAP2_SME2 != 0
}

#[cfg(test)]
mod detected_test {
    /// Recording a platform changes the word's low byte alone: the bits
    /// above it (AArch64's answer about the cores) stay, whichever
    /// platform was there before.
    #[test]
    fn test_a_recorded_platform_keeps_the_words_other_bits() {
        crate::initialize();
        assert!(super::is_initialized());
        assert_eq!(super::with_platform(0, 3), 3);
        assert_eq!(super::with_platform(0x0300, 3), 0x0303);
        assert_eq!(super::with_platform(0x0305, 2), 0x0302, "the platform before goes");
        assert_eq!(super::with_platform(0x02ff, 0x41), 0x0241);
    }

    /// `Platform::xof_many` rejects non-block-multiple output slices in every
    /// build, on both AVX-512 and non-AVX-512 paths.
    #[test]
    #[should_panic(expected = "whole blocks only")]
    fn test_xof_many_rejects_partial_block_output() {
        let mut out = [0u8; 63];
        #[cfg(target_arch = "x86_64")]
        let platform = super::Platform::sse41().unwrap_or_else(super::Platform::detect);
        #[cfg(not(target_arch = "x86_64"))]
        let platform = super::Platform::detect();
        platform.xof_many(crate::IV, &[0u8; 64], 0, 0, crate::ROOT, &mut out);
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn test_sme2_policy_helpers() {
        assert!(super::sme2_svl_ok(crate::sme2::GROUP as u64));
        assert!(!super::sme2_svl_ok(0));
        assert!(!super::sme2_svl_ok((crate::sme2::GROUP as u64) / 2));
        assert!(!super::sme2_svl_ok((crate::sme2::GROUP as u64) * 2));
        {
            assert!(super::sme2_allowed(super::Platform::SME2, true));
            assert!(!super::sme2_allowed(super::Platform::SME2, false));
            assert!(!super::sme2_allowed(super::Platform::NEON, true));
            assert!(!super::sme2_allowed(super::Platform::NEON, false));
            assert_eq!(super::Platform::sme2().is_some(), matches!(super::Platform::detect(), super::Platform::SME2));
        }
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn test_avx512_xof_many_rejects_partial_block_output() {
        if !super::avx512_detected() {
            return;
        }
        let err = std::panic::catch_unwind(|| {
            let mut out = [0u8; 63];
            unsafe { crate::avx512::xof_many(crate::IV, &[0u8; 64], 0, 0, crate::ROOT, &mut out) };
        });
        assert!(err.is_err(), "avx512::xof_many must reject partial-block output in release builds");
    }
}

#[cfg(all(test, target_arch = "x86_64"))]
mod x86_degree_test {
    use super::*;

    /// The detected platform hashes as many chunks at once as the widest
    /// kernel this build and CPU have (the standard library's detection, an
    /// independent reading, says which the CPU has): a narrower one would
    /// hash correctly, more slowly.
    #[test]
    fn test_detected_degree_is_the_widest_kernel() {
        let has_sse41 = std::is_x86_feature_detected!("ssse3") && std::is_x86_feature_detected!("sse4.1");
        let has_avx2 = has_sse41 && std::is_x86_feature_detected!("avx2");
        let has_avx512 = has_avx2 && std::is_x86_feature_detected!("avx512f") && std::is_x86_feature_detected!("avx512vl");
        let expected = if has_avx512 {
            32
        } else if has_avx2 {
            8
        } else {
            4
        };
        assert_eq!(Platform::detect().simd_degree(), expected);
    }
}

#[cfg(all(test, target_arch = "x86_64"))]
mod x86_features_test {
    /// The CPUID read finds what the standard library's detection, an
    /// independent reading, finds (under the crate's prerequisite lattice).
    #[test]
    fn test_x86_features_match_std() {
        use super::x86_bits::*;
        let features = super::read_x86_features();
        let has_sse41 = std::is_x86_feature_detected!("ssse3") && std::is_x86_feature_detected!("sse4.1");
        let has_avx2 = has_sse41 && std::is_x86_feature_detected!("avx2");
        let has_avx512 = has_avx2 && std::is_x86_feature_detected!("avx512f") && std::is_x86_feature_detected!("avx512vl");
        assert_eq!(features & READ, READ);
        assert_eq!(features & SSE41 != 0, has_sse41);
        assert_eq!(features & AVX2 != 0, has_avx2);
        assert_eq!(features & AVX512 != 0, has_avx512);
    }

    /// Every CPU the words can describe: each combination of the bits the
    /// detection looks at, against the rule written out feature by feature,
    /// with every other bit clear and then set (they decide nothing). On any
    /// machine: no CPU has to be the one that shows a wrong bit.
    #[test]
    fn test_features_of_every_combination_of_the_bits() {
        use super::x86_bits::*;
        use super::{CpuWords, features_of};
        let bit = |word: u32, n: u32| word & (1 << n) != 0;
        let spread = |chosen: u32, places: &[u32], rest: bool| {
            let mut word = if rest { !0u32 } else { 0 };
            for (i, &place) in places.iter().enumerate() {
                word = if chosen & (1 << i) != 0 { word | (1 << place) } else { word & !(1 << place) };
            }
            word
        };
        // The family: Intel's 6, AMD's Zen 4 (0x19), Zen 5 (0x1A) and the next (0x1B).
        let families = [(0x6u32, 0u32, 0x6u32), (0xF, 0x0A, 0x19), (0xF, 0x0B, 0x1A), (0xF, 0x0C, 0x1B)];
        let mut tried = 0;
        for rest in [false, true] {
            for ecx_bits in 0..32 {
                for xcr0_bits in 0..32 {
                    for ebx_bits in 0..8 {
                        for amd in [false, true] {
                            for (base, ext, family) in families {
                                let ecx = spread(ecx_bits, &[9, 19, 26, 27, 28], rest);
                                let xcr0 = spread(xcr0_bits, &[1, 2, 5, 6, 7], rest);
                                let ebx = spread(ebx_bits, &[5, 16, 31], rest);
                                // Everything of EAX but the two family fields follows `rest`.
                                let eax = (if rest { !0u32 & !(0xFF << 20) & !(0xF << 8) } else { 0 }) | (ext << 20) | (base << 8);
                                let got = features_of(CpuWords { leaf1_eax: eax, leaf1_ecx: ecx, xcr0, leaf7_ebx: ebx, amd });
                                // The rule, enforcing the full prerequisite lattice.
                                let sse41 = bit(ecx, 9) && bit(ecx, 19);
                                let avx = bit(ecx, 26) && bit(ecx, 27) && bit(ecx, 28);
                                let avx2 = sse41 && avx && bit(xcr0, 1) && bit(xcr0, 2) && bit(ebx, 5);
                                let avx512 = avx2 && [1, 2, 5, 6, 7].iter().all(|&n| bit(xcr0, n)) && bit(ebx, 16) && bit(ebx, 31);
                                let wide = avx512 && amd && family >= 0x1A;
                                let want = READ
                                    | if sse41 { SSE41 } else { 0 }
                                    | if avx2 { AVX2 } else { 0 }
                                    | if avx512 { AVX512 } else { 0 }
                                    | if wide { WIDE } else { 0 };
                                assert_eq!(got, want, "ecx {ecx:#x} xcr0 {xcr0:#x} ebx {ebx:#x} amd {amd} family {family:#x}");
                                tried += 1;
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(tried, 2 * 32 * 32 * 8 * 2 * 4);
    }

    /// The words are read where they may be: XCR0 and leaf 7 only where
    /// leaf 1 says the OS saves AVX's registers, and they agree with what
    /// the standard library finds here.
    #[test]
    fn test_cpu_words_are_read_where_leaf_one_allows() {
        use super::{os_saves_avx, read_cpu_words};
        for (ecx, saves) in [
            (0u32, false),
            (1 << 26, false),
            ((1 << 26) | (1 << 27), false),
            ((1 << 27) | (1 << 28), false),
            ((1 << 26) | (1 << 28), false),
            ((1 << 26) | (1 << 27) | (1 << 28), true),
            (!0, true),
            (!(1u32 << 27), false),
        ] {
            assert_eq!(os_saves_avx(ecx), saves, "ecx {ecx:#x}");
        }
        // Capped basic/extended CPUID ranges must not read absent leaves.
        assert!(!super::cpuid_has_leaf(0, 1));
        assert!(super::cpuid_has_leaf(1, 1));
        assert!(!super::cpuid_has_leaf(6, 7));
        assert!(super::cpuid_has_leaf(7, 7));
        assert!(!super::cpuid_has_leaf(0x8000_0000, 0x8000_0001));
        assert!(super::cpuid_has_leaf(0x8000_0001, 0x8000_0001));
        let words = read_cpu_words();
        assert_eq!(words.leaf1_ecx & (1 << 19) != 0, std::is_x86_feature_detected!("sse4.1"));
        assert_eq!(os_saves_avx(words.leaf1_ecx), std::is_x86_feature_detected!("avx"), "an OS that saves AVX's registers is how std finds AVX");
        if os_saves_avx(words.leaf1_ecx) {
            assert_eq!(words.xcr0 & 0b110, 0b110, "the OS saves XMM and YMM where std finds AVX");
            assert_eq!(words.leaf7_ebx & (1 << 5) != 0, std::is_x86_feature_detected!("avx2"));
            assert_eq!(words.leaf7_ebx & (1 << 3) != 0, std::is_x86_feature_detected!("bmi1"), "leaf 7's EBX, by a bit the kernels do not use");
        } else {
            assert_eq!((words.xcr0, words.leaf7_ebx, words.amd), (0, 0, false), "unread words are zero");
        }
    }

    /// The wide kernels' CPUs: AMD from family 0x1A (Zen 5), from leaf 1's
    /// family fields as CPUID packs them.
    #[test]
    fn test_wide_cpu_and_family() {
        let eax = |base: u32, ext: u32| (ext << 20) | (base << 8);
        assert_eq!(super::family(eax(0xF, 0x0B)), 0x1A, "Zen 5");
        assert_eq!(super::family(eax(0xF, 0x0A)), 0x19, "Zen 4");
        assert_eq!(super::family(eax(0x6, 0x0F)), 0x6, "Intel: the extended family counts only with base 0xF");
        for (amd, family, wide) in [(true, 0x1A, true), (true, 0x1B, true), (true, 0x19, false), (true, 0x17, false), (false, 0x1A, false), (false, 0x6, false)]
        {
            assert_eq!(super::wide_cpu(amd, family), wide, "amd {amd}, family {family:#x}");
        }
    }

    /// The wide bit agrees with the kernel's own reading of this CPU
    /// (/proc/cpuinfo's vendor and family), and comes only with AVX-512.
    #[test]
    #[cfg(target_os = "linux")]
    fn test_wide_bit_matches_cpuinfo() {
        use super::x86_bits::*;
        let features = super::read_x86_features();
        let info = std::fs::read_to_string("/proc/cpuinfo").expect("/proc/cpuinfo");
        let field = |name: &str| info.lines().find(|l| l.starts_with(name)).and_then(|l| l.split(':').nth(1)).map(|v| v.trim().to_string()).expect(name);
        let amd = field("vendor_id") == "AuthenticAMD";
        let family: u32 = field("cpu family").parse().expect("cpu family");
        let want = features & AVX512 != 0 && amd && family >= 0x1A;
        assert_eq!(features & WIDE != 0, want, "vendor {amd}, family {family:#x}");
        // The two words only an AMD CPU with AVX-512 puts to use, read
        // rightly on every CPU: a vendor or a family misread on an Intel
        // machine changed no feature there, and no test saw it.
        let words = super::read_cpu_words();
        if super::os_saves_avx(words.leaf1_ecx) {
            assert_eq!(words.amd, amd, "the vendor CPUID's leaf 0 names");
        }
        assert_eq!(super::family(words.leaf1_eax), family, "leaf 1's family");
        // What the dispatch reads, against the same independent reading.
        #[cfg(target_arch = "x86_64")]
        assert_eq!(super::wide_avx512(), want, "wide_avx512 on vendor {amd}, family {family:#x}");
    }

    /// The detection functions answer as the standard library does, each
    /// one on its own: detect() asks for the widest kernel first, so it
    /// never consults the narrower ones' answers on a CPU that has a wider
    /// one (AVX2's on an AVX-512 machine, SSE4.1's on an AVX2 one), and the
    /// guard-page tests' platform lists come from these same functions.
    #[test]
    fn test_x86_detection_matches_std() {
        let has_sse41 = std::is_x86_feature_detected!("ssse3") && std::is_x86_feature_detected!("sse4.1");
        let has_avx2 = has_sse41 && std::is_x86_feature_detected!("avx2");
        let has_avx512 = has_avx2 && std::is_x86_feature_detected!("avx512f") && std::is_x86_feature_detected!("avx512vl");
        assert_eq!(super::sse41_detected(), has_sse41, "sse41_detected");
        assert_eq!(super::avx2_detected(), has_avx2, "avx2_detected");
        #[cfg(target_arch = "x86_64")]
        assert_eq!(super::avx512_detected(), has_avx512, "avx512_detected");
    }
    /// Each CPU runs the widest kernel whose full prerequisite chain it has;
    /// a CPU without SSSE3 + SSE4.1 gets none, and the process stops.
    #[test]
    fn test_a_cpu_runs_its_widest_kernel_and_one_without_sse41_none() {
        use super::{Platform, x86_platform};
        for (avx512, avx2, sse41, want) in [
            (true, true, true, Some(Platform::AVX512)),
            (true, false, true, Some(Platform::SSE41)),
            (true, true, false, None),
            (true, false, false, None),
            (false, true, true, Some(Platform::AVX2)),
            (false, true, false, None),
            (false, false, true, Some(Platform::SSE41)),
            (false, false, false, None),
        ] {
            assert_eq!(x86_platform(avx512, avx2, sse41), want, "avx512 {avx512}, avx2 {avx2}, sse41 {sse41}");
        }
    }
}
