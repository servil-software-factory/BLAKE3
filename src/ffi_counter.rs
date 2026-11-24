//! Register-only SIMD kernels for [`crate::throughput::hash_counter_range`] and
//! [`crate::throughput::TreeHasher::hash_counter_range`] across AVX-512, AVX2,
//! Apple/AArch64 SME2, and AArch64 NEON `8+3` (#511).
//!
//! Each backend broadcasts the single-block (`4..=64` byte) template once
//! before the group loop and updates only `counter_word` per group in
//! registers / L1 stack storage, eliminating input RAM reads, message
//! transposes, caller stamping, and Apple P-core -> SME2 dirty cache-line
//! coherency stalls.

use crate::{BLOCK_LEN, CHUNK_END, CHUNK_START, CVWords, OUT_LEN, ROOT, platform::Platform};

/// Validate the `template` length and `counter_word` bounds upfront in every
/// build, including when `out` is empty.
#[inline]
pub(crate) fn validate_counter_range(template: &[u8], counter_word: usize) {
    assert!(template.len() <= BLOCK_LEN, "hash_counter_range template must fit in one block ({BLOCK_LEN} bytes), got {}", template.len(),);
    assert!(
        counter_word < 16 && counter_word * 4 + 4 <= template.len(),
        "hash_counter_range counter_word {counter_word} (bytes {}..{}) out of bounds for {}-byte template",
        counter_word.saturating_mul(4),
        counter_word.saturating_mul(4).saturating_add(4),
        template.len(),
    );
}

#[inline]
pub(crate) fn words_from_block(block: &[u8; BLOCK_LEN]) -> [u32; 16] {
    let mut words = [0u32; 16];
    for (word, chunk) in words.iter_mut().zip(block.as_chunks::<4>().0) {
        *word = u32::from_le_bytes(*chunk);
    }
    words
}

#[inline]
fn bytes_from_cv(cv: &CVWords) -> [u8; OUT_LEN] {
    let mut out = [0u8; OUT_LEN];
    for (chunk, &word) in out.as_chunks_mut::<4>().0.iter_mut().zip(cv.iter()) {
        *chunk = word.to_le_bytes();
    }
    out
}

#[inline]
fn wrapping_add_usize(start: u32, delta: usize) -> u32 {
    let bytes = delta.to_le_bytes();
    start.wrapping_add(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

#[inline]
#[cfg(any(blake3_sme2, blake3_neon, blake3_avx2_ffi, all(blake3_avx512_ffi, target_arch = "x86_64")))]
fn base_and_word(base: u32, counter_word: usize) -> u64 {
    u64::from(base) + ((counter_word as u64) << 32)
}

/// Hash every full SIMD group of counter-range single-block messages on
/// `platform`, returning the number of digests written to `out`.
fn hash_counter_groups(
    tmpl_words: &[u32; 16],
    blen_u8: u8,
    counter_word: usize,
    start: u32,
    key: &CVWords,
    root_flags: u8,
    out: &mut [[u8; OUT_LEN]],
    platform: Platform,
) -> usize {
    #[cfg(any(blake3_sme2, blake3_neon, blake3_avx2_ffi, all(blake3_avx512_ffi, target_arch = "x86_64")))]
    let flags_and_len = u64::from(root_flags) + (u64::from(blen_u8) << 8);
    let mut done = 0usize;

    #[cfg(blake3_sme2)]
    if matches!(platform, Platform::SME2 { .. }) {
        let groups = out.len() / 16;
        if groups > 0 {
            let kernel: unsafe extern "C" fn(*const u32, *const u32, u64, u64, *mut u8, u64) -> u64 = ffi_sme2::blake3_servil_sme2_counter;
            crate::ran_kernel(kernel as *const ());
            // Sound: `Platform::SME2` verified SME2 support; `tmpl_words` holds
            // 16 readable `u32` words, `key` holds 8 readable `u32` words,
            // `counter_word < 16`, and `out` holds at least `16 * groups`
            // writable 32-byte slots. The kernel checks SVL == 16 and
            // `TPIDR2_EL0` reserved bytes before touching ZA/streaming state
            // and returns 16 only when all `16 * groups` digests were written.
            let svl =
                unsafe { kernel(tmpl_words.as_ptr(), key.as_ptr(), base_and_word(start, counter_word), flags_and_len, out.as_mut_ptr().cast(), groups as u64) };
            if svl == 16 {
                done += groups * 16;
            }
        }
        // Any remainder (< 16, SVL != 16, or nonzero TPIDR2_EL0 reserved bytes)
        // falls through to NEON 8+3 below.
    }

    #[cfg(blake3_neon)]
    if matches!(platform, Platform::NEON { .. }) || {
        #[cfg(blake3_sme2)]
        {
            matches!(platform, Platform::SME2 { .. })
        }
        #[cfg(not(blake3_sme2))]
        {
            false
        }
    } {
        let rem = out.len() - done;
        let groups = rem / 11;
        if groups > 0 {
            let kernel: unsafe extern "C" fn(*const u32, *const u32, u64, u64, *mut u8, u64) = ffi_neon::blake3_servil_neon_p11_counter;
            crate::ran_kernel(kernel as *const ());
            let cur = wrapping_add_usize(start, done);
            // Sound: AArch64 NEON is mandatory on `Platform::NEON`/`SME2`;
            // `tmpl_words` holds 16 readable `u32` words, `key` holds 8
            // readable `u32` words, `counter_word < 16`, and `out[done..]`
            // holds at least `11 * groups` writable 32-byte slots.
            unsafe {
                kernel(tmpl_words.as_ptr(), key.as_ptr(), base_and_word(cur, counter_word), flags_and_len, out[done..].as_mut_ptr().cast(), groups as u64);
            }
            done += groups * 11;
        }
        return done;
    }

    #[cfg(all(blake3_avx512_ffi, target_arch = "x86_64"))]
    if matches!(platform, Platform::AVX512 { .. }) {
        let groups = out.len() / 16;
        if groups > 0 {
            let kernel: unsafe extern "C" fn(*const u32, u64, *const u32, u64, u64, *mut u8) = ffi_x86::blake3_servil_avx512_counter;
            crate::ran_kernel(kernel as *const ());
            // Sound: `Platform::AVX512` verified AVX-512F+VL; `tmpl_words`
            // holds 16 readable `u32` words, `key` holds 8 readable `u32`
            // words, `counter_word < 16`, and `out` holds at least
            // `16 * groups` writable 32-byte slots.
            unsafe {
                kernel(tmpl_words.as_ptr(), groups as u64, key.as_ptr(), base_and_word(start, counter_word), flags_and_len, out.as_mut_ptr().cast());
            }
            done += groups * 16;
        }
    }

    #[cfg(blake3_avx2_ffi)]
    if matches!(platform, Platform::AVX2 { .. }) || {
        #[cfg(all(blake3_avx512_ffi, target_arch = "x86_64"))]
        {
            matches!(platform, Platform::AVX512 { .. })
        }
        #[cfg(not(all(blake3_avx512_ffi, target_arch = "x86_64")))]
        {
            false
        }
    } {
        let rem = out.len() - done;
        let groups = rem / 8;
        if groups > 0 {
            let kernel: unsafe extern "C" fn(*const u32, u64, *const u32, u64, u64, *mut u8) = ffi_x86::blake3_servil_avx2_counter;
            crate::ran_kernel(kernel as *const ());
            let cur = wrapping_add_usize(start, done);
            // Sound: `Platform::AVX2` and `Platform::AVX512` guarantee AVX2;
            // `tmpl_words` holds 16 readable `u32` words, `key` holds 8
            // readable `u32` words, `counter_word < 16`, and `out[done..]`
            // holds at least `8 * groups` writable 32-byte slots.
            unsafe {
                kernel(tmpl_words.as_ptr(), groups as u64, key.as_ptr(), base_and_word(cur, counter_word), flags_and_len, out[done..].as_mut_ptr().cast());
            }
            done += groups * 8;
        }
    }

    let _ = (tmpl_words, blen_u8, counter_word, start, key, root_flags, out, platform);
    done
}

/// Hash `out.len()` single-block messages derived from `template` on
/// `platform`, replacing the 4-byte little-endian word at `counter_word * 4`
/// with `start.wrapping_add(i)` (modulo 2^32) for `i in 0..out.len()`.
pub(crate) fn hash_counter_range_on(template: &[u8], counter_word: usize, start: u32, key: &CVWords, flags: u8, out: &mut [[u8; OUT_LEN]], platform: Platform) {
    validate_counter_range(template, counter_word);
    if out.is_empty() {
        return;
    }

    let mut block = [0u8; BLOCK_LEN];
    block[..template.len()].copy_from_slice(template);
    let tmpl_words = words_from_block(&block);

    const ROOT_BLOCK_FLAGS: u8 = CHUNK_START + CHUNK_END + ROOT;
    let root_flags = flags + ROOT_BLOCK_FLAGS;
    let blen_u8 = u8::try_from(template.len()).expect("validated <= BLOCK_LEN");
    let done = hash_counter_groups(&tmpl_words, blen_u8, counter_word, start, key, root_flags, out, platform);

    // Serial tail (< 16 / < 11 / < 8, or SSE4.1 / portable builds).
    let cw_off = counter_word * 4;
    let mut cur = wrapping_add_usize(start, done);
    for slot in &mut out[done..] {
        block[cw_off..cw_off + 4].copy_from_slice(&cur.to_le_bytes());
        let mut cv = *key;
        platform.compress_in_place(&mut cv, &block, blen_u8, 0, root_flags);
        *slot = bytes_from_cv(&cv);
        cur = cur.wrapping_add(1);
    }
}

const SELF_TEST_COUNT: usize = 43;
const SELF_TEST_LEN: usize = 52;
const SELF_TEST_CW: usize = 12;
const SELF_TEST_START: u32 = u32::MAX - 5;
const SELF_TEST_KEY: [u8; 32] = [
    0x17, 0x2b, 0x3f, 0x53, 0x67, 0x7b, 0x8f, 0xa3, 0xb7, 0xcb, 0xdf, 0xf3, 0x09, 0x1d, 0x31, 0x45, 0x59, 0x6d, 0x81, 0x95, 0xa9, 0xbd, 0xd1, 0xe5, 0xf9, 0x0e,
    0x22, 0x36, 0x4a, 0x5e, 0x72, 0x86,
];

fn self_test_key_words() -> CVWords {
    let mut words = [0u32; 8];
    for i in 0..8 {
        words[i] = u32::from_le_bytes([SELF_TEST_KEY[4 * i], SELF_TEST_KEY[4 * i + 1], SELF_TEST_KEY[4 * i + 2], SELF_TEST_KEY[4 * i + 3]]);
    }
    words
}

fn self_test_template() -> [u8; SELF_TEST_LEN] {
    let mut tmpl = [0u8; SELF_TEST_LEN];
    for (j, b) in tmpl.iter_mut().enumerate() {
        *b = (j.wrapping_mul(37) ^ 0x9a).to_le_bytes()[0];
    }
    tmpl
}

/// Fold of `43` keyed 52-byte counter-range digests starting at `u32::MAX - 5`
/// (`counter_word = 12`) from official BLAKE3 1.8.7.
/// `43 = 2*16 + 11 = 2*16 + 8 + 3 = 5*8 + 3 = 3*11 + 10`, exercising multi-group
/// counter wrapping and remainder dispatch on every ISA kernel in one call.
const SELF_TEST_EXPECTED: u64 = 0xb837_cd90_1a80_d68b;

pub(crate) fn self_test(platform: Platform) -> usize {
    let key = self_test_key_words();
    let tmpl = self_test_template();
    let mut out = [[0u8; OUT_LEN]; SELF_TEST_COUNT];
    hash_counter_range_on(&tmpl, SELF_TEST_CW, SELF_TEST_START, &key, crate::KEYED_HASH, &mut out, platform);
    let got = crate::self_test::fold(out.as_flattened());
    if got != SELF_TEST_EXPECTED {
        crate::self_test::fail(format_args!(
            "blake3-servil startup self-test failed: hash_counter_range (platform {}): \
             expected {:#018x}, got {:#018x}",
            platform.name(),
            SELF_TEST_EXPECTED,
            got,
        ));
    }
    1
}

/// Exported entry points of the counter-range kernels compiled for this target,
/// for `self_test::test_self_test_runs_every_kernel`.
#[cfg(all(test, feature = "std", blake3_neon))]
pub(crate) fn entries_neon() -> [(&'static str, *const ()); 1] {
    [("neon_p11_counter", ffi_neon::blake3_servil_neon_p11_counter as *const ())]
}

#[cfg(all(test, feature = "std", blake3_sme2))]
pub(crate) fn entries_sme2() -> [(&'static str, *const ()); 1] {
    [("sme2_counter", ffi_sme2::blake3_servil_sme2_counter as *const ())]
}

#[cfg(all(test, feature = "std", blake3_avx2_ffi))]
pub(crate) fn entries_avx2() -> [(&'static str, *const ()); 1] {
    [("avx2_counter", ffi_x86::blake3_servil_avx2_counter as *const ())]
}

#[cfg(all(test, feature = "std", blake3_avx512_ffi, target_arch = "x86_64"))]
pub(crate) fn entries_avx512() -> [(&'static str, *const ()); 1] {
    [("avx512_counter", ffi_x86::blake3_servil_avx512_counter as *const ())]
}

#[cfg(all(test, feature = "std", blake3_sme2))]
pub(crate) unsafe fn call_sme2_counter(template: *const u32, key: *const u32, base_and_word: u64, flags_and_len: u64, out: *mut u8, groups: u64) -> u64 {
    // Sound: caller verified SME2 support and provided valid buffers covering
    // 16 `u32` template words, 8 `u32` key words, and `16 * groups` outputs.
    unsafe { ffi_sme2::blake3_servil_sme2_counter(template, key, base_and_word, flags_and_len, out, groups) }
}

#[cfg(blake3_avx2_ffi)]
mod ffi_x86 {
    unsafe extern "C" {
        #[cfg(blake3_avx512_ffi)]
        pub fn blake3_servil_avx512_counter(template: *const u32, groups: u64, key: *const u32, base_and_word: u64, flags_and_len: u64, out: *mut u8);
        pub fn blake3_servil_avx2_counter(template: *const u32, groups: u64, key: *const u32, base_and_word: u64, flags_and_len: u64, out: *mut u8);
    }
}

#[cfg(blake3_sme2)]
mod ffi_sme2 {
    unsafe extern "C" {
        pub fn blake3_servil_sme2_counter(template: *const u32, key: *const u32, base_and_word: u64, flags_and_len: u64, out: *mut u8, groups: u64) -> u64;
    }
}

#[cfg(blake3_neon)]
mod ffi_neon {
    unsafe extern "C" {
        pub fn blake3_servil_neon_p11_counter(template: *const u32, key: *const u32, base_and_word: u64, flags_and_len: u64, out: *mut u8, groups: u64);
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::Mode;
    use crate::throughput::{TreeHasher, hash_counter_range};

    fn official_counter_range(mode: Mode, template: &[u8], counter_word: usize, start: u32, count: usize) -> std::vec::Vec<[u8; OUT_LEN]> {
        let mut msg = template.to_vec();
        let off = counter_word * 4;
        (0..count)
            .map(|i| {
                let c = wrapping_add_usize(start, i);
                msg[off..off + 4].copy_from_slice(&c.to_le_bytes());
                crate::test::official::hash(mode, &msg)
            })
            .collect()
    }

    #[test]
    fn counter_self_test_expected_matches_official_reference() {
        let tmpl = self_test_template();
        let out = official_counter_range(Mode::Keyed(&SELF_TEST_KEY), &tmpl, SELF_TEST_CW, SELF_TEST_START, SELF_TEST_COUNT);
        let computed = crate::self_test::fold(out.as_flattened());
        assert_eq!(SELF_TEST_EXPECTED, computed, "SELF_TEST_EXPECTED should be {computed:#018x}");
    }

    #[test]
    fn counter_range_matches_official_across_lengths_words_wraps_and_modes() {
        crate::initialize();
        let key = [0x42u8; 32];
        let ctx = "blake3-servil counter-range test";
        let starts = [0u32, 1, 0x7fff_fffa, u32::MAX - 20, u32::MAX - 1, u32::MAX];
        for (m_idx, mode) in [Mode::Plain, Mode::Keyed(&key), Mode::DeriveKey(ctx)].into_iter().enumerate() {
            for tree in [TreeHasher::new(mode), TreeHasher::new(mode).on_this_core()] {
                for len in [4usize, 7, 16, 32, 36, 52, 60, 63, 64] {
                    let mut tmpl = std::vec![0u8; len];
                    for (j, b) in tmpl.iter_mut().enumerate() {
                        *b = ((j.wrapping_mul(41).wrapping_add(len)) ^ 0x5a).to_le_bytes()[0];
                    }
                    let max_cw = (len / 4) - 1;
                    let mut cws = std::vec![0usize, max_cw];
                    if max_cw >= 2 {
                        cws.push(max_cw / 2);
                    }
                    cws.sort_unstable();
                    cws.dedup();
                    for &cw in &cws {
                        for count in [0usize, 1, 7, 8, 9, 10, 11, 12, 15, 16, 17, 22, 24, 32, 43] {
                            let start = starts[(len + cw + count) % starts.len()];
                            let mut got = std::vec![[0u8; OUT_LEN]; count];
                            tree.hash_counter_range(&tmpl, cw, start, &mut got);
                            let want = official_counter_range(mode, &tmpl, cw, start, count);
                            assert_eq!(got, want, "mode={m_idx} len={len} cw={cw} count={count} start={start:#x}");

                            let mut got_fn = std::vec![[0u8; OUT_LEN]; count];
                            hash_counter_range(mode, &tmpl, cw, start, &mut got_fn);
                            assert_eq!(got_fn, want, "fn mode={m_idx} len={len} cw={cw} count={count}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn counter_range_covers_every_counter_word_0_to_15() {
        crate::initialize();
        let mut tmpl = [0u8; 64];
        for (j, b) in tmpl.iter_mut().enumerate() {
            *b = (j.wrapping_mul(23) ^ 0xc3).to_le_bytes()[0];
        }
        for cw in 0..16 {
            let start = u32::MAX - 9;
            let mut got = [[0u8; OUT_LEN]; 49];
            hash_counter_range(Mode::Plain, &tmpl, cw, start, &mut got);
            let want = official_counter_range(Mode::Plain, &tmpl, cw, start, 49);
            assert_eq!(&got[..], &want[..], "cw={cw}");
        }
        // Grinding-size multi-group counts (3+ groups across every ISA width).
        for count in [48usize, 49, 256] {
            let mut got = std::vec![[0u8; OUT_LEN]; count];
            TreeHasher::new(Mode::Plain).hash_counter_range(&tmpl[..52], 12, u32::MAX - 30, &mut got);
            let want = official_counter_range(Mode::Plain, &tmpl[..52], 12, u32::MAX - 30, count);
            assert_eq!(got, want, "multi-group count={count}");
        }
    }

    #[test]
    fn counter_range_whole_groups_take_direct_kernels() {
        crate::initialize();
        let platform = Platform::detect();
        let (group_len, entry): (usize, usize) = match platform {
            #[cfg(blake3_sme2)]
            Platform::SME2 { .. } => (16, ffi_sme2::blake3_servil_sme2_counter as *const () as usize),
            #[cfg(blake3_neon)]
            Platform::NEON { .. } => (11, ffi_neon::blake3_servil_neon_p11_counter as *const () as usize),
            #[cfg(all(blake3_avx512_ffi, target_arch = "x86_64"))]
            Platform::AVX512 { .. } => (16, ffi_x86::blake3_servil_avx512_counter as *const () as usize),
            #[cfg(blake3_avx2_ffi)]
            Platform::AVX2 { .. } => (8, ffi_x86::blake3_servil_avx2_counter as *const () as usize),
            #[cfg(blake3_sse41_ffi)]
            Platform::SSE41 { .. } => return,
        };

        let tmpl = [0x3cu8; 52];
        let mut block = [0u8; BLOCK_LEN];
        block[..52].copy_from_slice(&tmpl);
        let tmpl_words = words_from_block(&block);
        let root_flags = CHUNK_START | CHUNK_END | ROOT;
        for (count, reached) in [(group_len, true), (2 * group_len, true), (3 * group_len, true), (group_len - 1, false)] {
            let mut out = std::vec![[0u8; OUT_LEN]; count];
            crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
            let done = hash_counter_groups(&tmpl_words, 52, 12, 100, &crate::IV, root_flags, &mut out, platform);
            let ran = crate::RAN_ENTRIES.with(|r| r.borrow().clone());
            assert_eq!(ran.contains(&entry), reached, "count={count}");
            if reached {
                assert_eq!(done, count, "exact group count handled wholly by SIMD");
                assert_eq!(ran.len(), 1, "exact groups run direct counter kernel alone: {ran:?}");
            }
        }
        // Verify public API dispatch (both `hash_counter_range` and `TreeHasher::hash_counter_range`),
        // including `TreeHasher::on_this_core()` routing away from SME2 to NEON 8+3.
        for count in [group_len, 3 * group_len, 256] {
            let mut out = std::vec![[0u8; OUT_LEN]; count];
            crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
            hash_counter_range(Mode::Plain, &tmpl, 12, 100, &mut out);
            assert!(crate::RAN_ENTRIES.with(|r| r.borrow().contains(&entry)), "public fn count={count}");

            crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
            TreeHasher::new(Mode::Plain).hash_counter_range(&tmpl, 12, 100, &mut out);
            assert!(crate::RAN_ENTRIES.with(|r| r.borrow().contains(&entry)), "TreeHasher count={count}");
        }
        #[cfg(blake3_sme2)]
        if matches!(platform, Platform::SME2 { .. }) {
            let neon_entry = ffi_neon::blake3_servil_neon_p11_counter as *const () as usize;
            let mut out = std::vec![[0u8; OUT_LEN]; 33];
            crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
            TreeHasher::new(Mode::Plain).on_this_core().hash_counter_range(&tmpl, 12, 100, &mut out);
            let ran = crate::RAN_ENTRIES.with(|r| r.borrow().clone());
            assert!(ran.contains(&neon_entry), "on_this_core must dispatch to NEON 8+3");
            assert!(!ran.contains(&entry), "on_this_core must not enter SME2");
        }
        let (mixed, want_done) = match platform {
            #[cfg(blake3_sme2)]
            Platform::SME2 { .. } => (43usize, 43usize),
            #[cfg(blake3_neon)]
            Platform::NEON { .. } => (30usize, 22usize),
            #[cfg(all(blake3_avx512_ffi, target_arch = "x86_64"))]
            Platform::AVX512 { .. } => (27usize, 24usize),
            #[cfg(blake3_avx2_ffi)]
            Platform::AVX2 { .. } => (23usize, 16usize),
            #[cfg(blake3_sse41_ffi)]
            Platform::SSE41 { .. } => (23usize, 0usize),
        };
        let mut out = std::vec![[0u8; OUT_LEN]; mixed];
        let done = hash_counter_groups(&tmpl_words, 52, 12, 100, &crate::IV, root_flags, &mut out, platform);
        assert_eq!(done, want_done, "mixed group count={mixed}");
    }

    #[test]
    fn counter_range_respects_guard_pages() {
        use crate::test::guard::Guarded;
        crate::initialize();
        let platform = Platform::detect();
        for len in [4usize, 36, 52, 64] {
            for count in [8usize, 11, 16, 22, 32, 43] {
                for at_end in [false, true] {
                    let mut tmpl = Guarded::new(len, at_end);
                    for (i, b) in tmpl.bytes().iter_mut().enumerate() {
                        *b = (i.wrapping_mul(31) ^ 0x6d).to_le_bytes()[0];
                    }
                    let mut out = Guarded::new(count * OUT_LEN, at_end);
                    let out_slice = out.bytes().as_chunks_mut::<OUT_LEN>().0;
                    let cw = (len / 4) - 1;
                    hash_counter_range_on(tmpl.bytes(), cw, u32::MAX - 3, &crate::IV, 0, out_slice, platform);
                    let want = official_counter_range(Mode::Plain, tmpl.bytes(), cw, u32::MAX - 3, count);
                    assert_eq!(out_slice, &want[..], "len={len} count={count} at_end={at_end}");
                }
            }
        }
    }

    #[test]
    #[should_panic(expected = "fit in one block")]
    fn counter_range_rejects_template_over_block_even_when_empty() {
        crate::initialize();
        hash_counter_range(Mode::Plain, &[0u8; 65], 0, 0, &mut []);
    }

    #[test]
    #[should_panic(expected = "out of bounds")]
    fn counter_range_rejects_counter_word_past_template_even_when_empty() {
        crate::initialize();
        hash_counter_range(Mode::Plain, &[0u8; 7], 1, 0, &mut []);
    }

    #[test]
    #[should_panic(expected = "out of bounds")]
    fn counter_range_rejects_short_template_under_4_bytes_even_when_empty() {
        crate::initialize();
        TreeHasher::new(Mode::Plain).hash_counter_range(&[0u8; 3], 0, 0, &mut []);
    }

    #[test]
    #[should_panic(expected = "out of bounds")]
    fn counter_range_rejects_counter_word_16_even_when_empty() {
        crate::initialize();
        TreeHasher::new(Mode::Plain).hash_counter_range(&[0u8; 64], 16, 0, &mut []);
    }
}
