//! Whole chunks and a partial one in one pass, side by side (x86-64): the
//! five generated kernels of `c/blake3_partial_x86-64_unix.S` (`tools/gen_x86.py`),
//! four lanes on SSE4.1 (`partial_sse41`), eight on AVX2 (`partial_avx2`), and
//! zero-spill EVEX AVX-512F+VL kernels at four (horizontal `zmm`), eight
//! (vertical `ymm`), and sixteen (vertical `zmm`) lanes (`partial_avx512_{4,8,16}`).
//! The partial chunk takes the lane after the whole ones, its last block flagged
//! CHUNK_END at its own step, and its chaining value is the kernel's snapshot at
//! that step; so the longest chain of dependent blocks is a whole chunk's
//! sixteen, where hashing the partial chunk after the whole ones added its
//! blocks to it.

use crate::platform::Platform;
use crate::{BLOCK_LEN, CHUNK_END, CHUNK_LEN, CHUNK_START, CVWords, IV, OUT_LEN};

/// The kernels' parameter block for `W` lanes, laid out as gen_x86.py's
/// `layout` says (the test below checks the offsets): every value the
/// kernel reads per lane and per block, and its two outputs.
#[repr(C, align(64))]
pub struct Params<const W: usize> {
    msg: [*const u8; W],
    key: [[u32; W]; 8],
    iv: [[u32; W]; 4],
    rot16: [u32; W],
    rot8: [u32; W],
    ctr: [[u32; W]; 2],
    lens: [[u32; W]; 16],
    flags: [[u32; W]; 16],
    last: [u32; 16],
    h: [[u32; W]; 8],
    snap: [[u32; W]; 8],
}

unsafe extern "C" {
    fn blake3_servil_partial_sse41(params: *mut Params<4>);
    fn blake3_servil_partial_avx2(params: *mut Params<8>);
    fn blake3_servil_partial_avx512_4(params: *mut Params<4>);
    fn blake3_servil_partial_avx512_8(params: *mut Params<8>);
    fn blake3_servil_partial_avx512_16(params: *mut Params<16>);
}

/// The byte shuffle (pshufb) that rotates each 32-bit word right by `r`
/// bytes, as `W` little-endian words; each 16-byte lane of a ymm register
/// shuffles within itself, so the pattern repeats every four words.
fn rotation<const W: usize>(r: u32) -> [u32; W] {
    core::array::from_fn(|k| {
        let base = 4 * u32::from((k % 4).to_le_bytes()[0]);
        (0..4).map(|i| (base + (i + r) % 4) << (8 * i)).fold(0, |a, b| a | b)
    })
}

/// The chaining values of `whole.len()` whole chunks and one partial chunk
/// after them (1 to CHUNK_LEN - 1 bytes), at `counter` and the counters
/// after it, into `out`, `(whole.len() + 1) * OUT_LEN` bytes. Requires 1 to
/// `W - 1` whole chunks.
fn hash_chunks_with_partial<const W: usize>(
    kernel: unsafe extern "C" fn(*mut Params<W>),
    whole: &[&[u8; CHUNK_LEN]],
    partial: &[u8],
    key: &CVWords,
    counter: u64,
    flags: u8,
    out: &mut [u8],
) {
    let n = whole.len();
    assert!((1..W).contains(&n) && (1..CHUNK_LEN).contains(&partial.len()) && out.len() >= (n + 1) * OUT_LEN);
    // The partial chunk zero-padded to a whole one, so its lane reads
    // within bounds after its last block; the lanes past it read it too.
    let mut padded = [0u8; CHUNK_LEN];
    padded[..partial.len()].copy_from_slice(partial);
    let last = partial.len().div_ceil(64) - 1;
    let last_len = u32::from((partial.len() - 64 * last).to_le_bytes()[0]);
    let flags = flags as u32;
    let mut p = Params::<W> {
        msg: [padded.as_ptr(); W],
        key: core::array::from_fn(|i| [key[i]; W]),
        iv: core::array::from_fn(|i| [IV[i]; W]),
        rot16: rotation(2),
        rot8: rotation(1),
        ctr: [
            core::array::from_fn(|j| u32::from_le_bytes((counter + j as u64).to_le_bytes()[..4].try_into().unwrap())),
            core::array::from_fn(|j| u32::from_le_bytes((counter + j as u64).to_le_bytes()[4..].try_into().unwrap())),
        ],
        lens: [[64; W]; 16],
        flags: [[flags; W]; 16],
        last: core::array::from_fn(|i| if i == 0 { 64 * u32::from(last.to_le_bytes()[0]) } else { 0 }),
        h: [[0; W]; 8],
        snap: [[0; W]; 8],
    };
    // Every block is whole and carries `flags`, but for three places: each
    // lane's first block starts a chunk, its sixteenth ends one, and the
    // partial lane's last block holds `last_len` bytes and ends its chunk.
    // Filled whole and patched, not entry by entry: built entry by entry,
    // the 16-by-4 tables unrolled into 20 KB of code that a cold call
    // fetches (#119).
    for lane in 0..W {
        p.flags[0][lane] |= CHUNK_START as u32;
        p.flags[15][lane] |= CHUNK_END as u32;
    }
    p.lens[last][n] = last_len;
    p.flags[last][n] |= CHUNK_END as u32;
    for (lane, chunk) in whole.iter().enumerate() {
        p.msg[lane] = chunk.as_ptr();
    }
    // Sound: every lane's pointer reaches 16 blocks (a whole chunk, or the
    // padded copy), and the caller's CPU has the kernel's instructions.
    unsafe {
        crate::ran_kernel(kernel as *const ());
        kernel(&mut p);
    }
    for lane in 0..=n {
        let words = if lane < n { &p.h } else { &p.snap };
        for (i, word) in words.iter().enumerate() {
            out[lane * OUT_LEN + 4 * i..][..4].copy_from_slice(&word[lane].to_le_bytes());
        }
    }
}

/// The lanes of the one pass that hashes `whole` whole chunks with a
/// partial one of `partial` bytes after them, where the pass costs less
/// than the partial chunk's blocks after the whole ones:
/// - 3 whole chunks in 4 lanes on SSE4.1/AVX2, and on AVX-512 horizontal zmm
///   (all partials on wide AVX-512 such as Zen 5, > 1 block on non-wide AVX-512
///   such as Ice Lake and Zen 4);
/// - 2 whole chunks in 4 lanes on AVX-512 horizontal zmm (all partials on wide
///   AVX-512, > 4 blocks on non-wide AVX-512 so Zen 4's 2177..2304 B stay
///   regression-free);
/// - 6 or 7 whole chunks (or 5 with > 5 partial blocks) in 8 lanes on AVX2;
///   5..=7 whole chunks (or 4 past 2 blocks on wide AVX-512 / 13 blocks on
///   non-wide AVX-512) in 8 lanes on AVX-512;
/// - 9..=15 whole chunks (all partials on wide AVX-512, > 9 blocks on non-wide
///   AVX-512) and 8 whole chunks (> 2 blocks on wide AVX-512) in 16 lanes on
///   AVX-512.
/// One whole chunk plus a partial chunk of two blocks or more is handled
/// earlier by `pair::takes`.
#[inline]
pub(crate) fn spare_lane(platform: Platform, whole: usize, partial: usize) -> Option<usize> {
    #[cfg(target_os = "linux")]
    let wide = crate::platform::wide_avx512();
    #[cfg(not(target_os = "linux"))]
    let wide = false;
    spare_lane_for(platform, wide, whole, partial)
}

#[inline]
pub(crate) fn spare_lane_for(platform: Platform, wide: bool, whole: usize, partial: usize) -> Option<usize> {
    match (platform, whole) {
        (Platform::SSE41 | Platform::AVX2, 3) => Some(4),
        (Platform::AVX2, 6..=7) => Some(8),
        (Platform::AVX2, 5) if partial > 5 * BLOCK_LEN => Some(8),
        (Platform::AVX512, 2) if wide || partial > 4 * BLOCK_LEN => Some(4),
        (Platform::AVX512, 3) if wide || partial > BLOCK_LEN => Some(4),
        (Platform::AVX512, 5..=7) => Some(8),
        (Platform::AVX512, 4) if partial > (if wide { 2 } else { 13 }) * BLOCK_LEN => Some(8),
        (Platform::AVX512, 9..=15) if wide || partial > 9 * BLOCK_LEN => Some(16),
        (Platform::AVX512, 8) if wide && partial > 2 * BLOCK_LEN => Some(16),
        _ => None,
    }
}

/// Hash `whole` chunks plus a non-empty `partial` chunk in one spare-lane pass
/// when [`spare_lane`] selects one for `platform`, returning `true` if handled.
#[inline]
pub(crate) fn try_hash_chunks_with_partial(
    platform: Platform,
    whole: &[&[u8; CHUNK_LEN]],
    partial: &[u8],
    key: &CVWords,
    counter: u64,
    flags: u8,
    out: &mut [u8],
) -> bool {
    if partial.is_empty() {
        return false;
    }
    let Some(lanes) = spare_lane(platform, whole.len(), partial.len()) else {
        return false;
    };
    // Sound: `spare_lane` admits each width only on a platform whose CPU
    // supplies the matching SSE4.1, AVX2, or AVX-512F+VL instructions.
    unsafe {
        match (platform, lanes) {
            (Platform::AVX512, 4) => hash_chunks_with_partial_avx512_4(whole, partial, key, counter, flags, out),
            (Platform::AVX512, 8) => hash_chunks_with_partial_avx512_8(whole, partial, key, counter, flags, out),
            (Platform::AVX512, 16) => hash_chunks_with_partial_avx512_16(whole, partial, key, counter, flags, out),
            (_, 4) => hash_chunks_with_partial_sse41(whole, partial, key, counter, flags, out),
            _ => hash_chunks_with_partial_avx2(whole, partial, key, counter, flags, out),
        }
    }
    true
}

/// [`hash_chunks_with_partial`] on four lanes: 1 to 3 whole chunks.
/// Unsafe: the CPU must have SSE4.1.
#[inline(never)]
pub unsafe fn hash_chunks_with_partial_sse41(whole: &[&[u8; CHUNK_LEN]], partial: &[u8], key: &CVWords, counter: u64, flags: u8, out: &mut [u8]) {
    hash_chunks_with_partial::<4>(blake3_servil_partial_sse41, whole, partial, key, counter, flags, out)
}

/// [`hash_chunks_with_partial`] on eight lanes: 1 to 7 whole chunks.
/// Unsafe: the CPU must have AVX2.
#[inline(never)]
pub unsafe fn hash_chunks_with_partial_avx2(whole: &[&[u8; CHUNK_LEN]], partial: &[u8], key: &CVWords, counter: u64, flags: u8, out: &mut [u8]) {
    hash_chunks_with_partial::<8>(blake3_servil_partial_avx2, whole, partial, key, counter, flags, out)
}

/// [`hash_chunks_with_partial`] on four EVEX AVX-512VL lanes: 1 to 3 whole chunks.
/// Unsafe: the CPU must have AVX-512F and AVX-512VL.
#[inline(never)]
pub unsafe fn hash_chunks_with_partial_avx512_4(whole: &[&[u8; CHUNK_LEN]], partial: &[u8], key: &CVWords, counter: u64, flags: u8, out: &mut [u8]) {
    hash_chunks_with_partial::<4>(blake3_servil_partial_avx512_4, whole, partial, key, counter, flags, out)
}

/// [`hash_chunks_with_partial`] on eight EVEX AVX-512VL lanes: 1 to 7 whole chunks.
/// Unsafe: the CPU must have AVX-512F and AVX-512VL.
#[inline(never)]
pub unsafe fn hash_chunks_with_partial_avx512_8(whole: &[&[u8; CHUNK_LEN]], partial: &[u8], key: &CVWords, counter: u64, flags: u8, out: &mut [u8]) {
    hash_chunks_with_partial::<8>(blake3_servil_partial_avx512_8, whole, partial, key, counter, flags, out)
}

/// [`hash_chunks_with_partial`] on sixteen EVEX AVX-512F lanes: 1 to 15 whole chunks.
/// Unsafe: the CPU must have AVX-512F and AVX-512VL.
#[inline(never)]
pub unsafe fn hash_chunks_with_partial_avx512_16(whole: &[&[u8; CHUNK_LEN]], partial: &[u8], key: &CVWords, counter: u64, flags: u8, out: &mut [u8]) {
    hash_chunks_with_partial::<16>(blake3_servil_partial_avx512_16, whole, partial, key, counter, flags, out)
}

const SELF_TEST_WHOLE: usize = 9;
const SELF_TEST_PARTIAL: usize = 640;
const SELF_TEST_COUNTER: u64 = 0xffff_fffe;
/// Official BLAKE3 1.8.7 FNV-1a fold of the 10 chaining values produced by
/// `partial_avx512_16` on `SELF_TEST_WHOLE` whole chunks + `SELF_TEST_PARTIAL`
/// bytes at `SELF_TEST_COUNTER` with `IV` and `KEYED_HASH`.
const SELF_TEST_PARTIAL16_EXPECTED: u64 = 0x572e_b865_aabf_f677;
/// Official BLAKE3 1.8.7 FNV-1a fold of the plain hash of 5 whole chunks from
/// the same input buffer (rounds 5 up to 8 lanes on AVX-512, preserving
/// startup coverage of `blake3_hash_many_avx512`'s 8-lane `ymm` loop now that
/// `self_test::CASES` case 4 routes to `partial_avx512_8`).
const SELF_TEST_HASH_MANY8_EXPECTED: u64 = 0x3676_3a5e_5677_8d91;

fn self_test_buf() -> [u8; SELF_TEST_WHOLE * CHUNK_LEN + SELF_TEST_PARTIAL] {
    let mut buf = [0u8; SELF_TEST_WHOLE * CHUNK_LEN + SELF_TEST_PARTIAL];
    for (i, b) in buf.iter_mut().enumerate() {
        *b = i.wrapping_mul(37).wrapping_add(11).to_le_bytes()[0];
    }
    buf
}

/// Startup self-test for `partial_avx512_16` and `hash_many_avx512`'s 8-lane
/// (`ymm`) loop (`partial_avx512_4` and `partial_avx512_8` are covered by
/// `self_test::CASES` on `3*KIB + 100` and `5*KIB + 321`). Compares both
/// outputs against checked-in official BLAKE3 1.8.7 folds (build-independent,
/// no runtime reference pass). Runs on `Platform::AVX512` only; zero cost
/// elsewhere.
#[inline(never)]
pub(crate) fn self_test(platform: crate::platform::Platform) -> usize {
    if !matches!(platform, crate::platform::Platform::AVX512) {
        return 0;
    }
    let buf = self_test_buf();
    let chunks: [&[u8; CHUNK_LEN]; SELF_TEST_WHOLE] = core::array::from_fn(|i| buf[i * CHUNK_LEN..(i + 1) * CHUNK_LEN].try_into().unwrap());
    let rest = &buf[SELF_TEST_WHOLE * CHUNK_LEN..];
    let mut out = [0u8; (SELF_TEST_WHOLE + 1) * OUT_LEN];
    // Sound: `platform` is `Platform::AVX512`, `SELF_TEST_WHOLE < 16`,
    // `SELF_TEST_PARTIAL < CHUNK_LEN`, and `out` holds `(SELF_TEST_WHOLE + 1) * OUT_LEN` bytes.
    unsafe { hash_chunks_with_partial_avx512_16(&chunks, rest, crate::IV, SELF_TEST_COUNTER, crate::KEYED_HASH, &mut out) };
    if crate::self_test::fold(&out) != SELF_TEST_PARTIAL16_EXPECTED {
        crate::self_test::fail(format_args!("blake3-servil startup self-test failed: partial_avx512_16 (platform {})", platform.name()));
    }
    // Five whole chunks (no partial) round up to 8 lanes on AVX-512, keeping
    // `blake3_hash_many_avx512`'s 8-lane `ymm` loop covered at startup.
    let five = crate::hash_serial_from(platform, &buf[..5 * CHUNK_LEN], crate::IV, 0, false);
    if crate::self_test::fold(five.as_bytes()) != SELF_TEST_HASH_MANY8_EXPECTED {
        crate::self_test::fail(format_args!("blake3-servil startup self-test failed: hash_many_avx512 8-lane (platform {})", platform.name()));
    }
    2
}

/// Exported entry point of the 4-lane SSE4.1 partial-chunk kernel for tests
/// checking non-AVX-512 `compress_chunks_parallel` dispatch.
#[cfg(all(test, feature = "std"))]
pub(crate) fn entry_sse41() -> *const () {
    blake3_servil_partial_sse41 as *const ()
}

/// Exported entry points of the AVX-512 partial-chunk kernels for
/// `self_test::test_self_test_run_checks_every_case`.
#[cfg(all(test, feature = "std"))]
pub(crate) fn entries_avx512() -> [(&'static str, *const ()); 3] {
    [
        ("partial_avx512_4", blake3_servil_partial_avx512_4 as *const ()),
        ("partial_avx512_8", blake3_servil_partial_avx512_8 as *const ()),
        ("partial_avx512_16", blake3_servil_partial_avx512_16 as *const ()),
    ]
}

#[cfg(test)]
mod test {
    use super::*;

    /// The parameter block's offsets are the generator's (gen_x86.py's
    /// `layout`: each field in turn, no padding but the struct's own).
    #[test]
    fn test_params_layout_is_the_generators() {
        fn check<const W: usize>() {
            let v = 4 * W;
            let want = [
                8 * W,
                8 * W + 8 * v,
                8 * W + 12 * v,
                8 * W + 13 * v,
                8 * W + 14 * v,
                8 * W + 16 * v,
                8 * W + 32 * v,
                8 * W + 48 * v,
                8 * W + 48 * v + 64,
                8 * W + 56 * v + 64,
            ];
            let got = [
                core::mem::offset_of!(Params<W>, key),
                core::mem::offset_of!(Params<W>, iv),
                core::mem::offset_of!(Params<W>, rot16),
                core::mem::offset_of!(Params<W>, rot8),
                core::mem::offset_of!(Params<W>, ctr),
                core::mem::offset_of!(Params<W>, lens),
                core::mem::offset_of!(Params<W>, flags),
                core::mem::offset_of!(Params<W>, last),
                core::mem::offset_of!(Params<W>, h),
                core::mem::offset_of!(Params<W>, snap),
            ];
            assert_eq!(got, want, "{W} lanes");
        }
        check::<4>();
        check::<8>();
        check::<16>();
    }

    /// Every count of whole chunks each kernel takes, with partial chunks
    /// of every block count (one byte and whole) and at a counter whose low
    /// word carries, against the portable chunk values.
    #[test]
    fn test_partial_kernels_against_portable() {
        let mut input = [0u8; 16 * CHUNK_LEN];
        crate::test::paint_test_input(&mut input);
        let kernels: &[(usize, unsafe fn(&[&[u8; CHUNK_LEN]], &[u8], &CVWords, u64, u8, &mut [u8]), bool)] = &[
            (4, hash_chunks_with_partial_sse41, crate::platform::sse41_detected()),
            (8, hash_chunks_with_partial_avx2, crate::platform::avx2_detected()),
            (4, hash_chunks_with_partial_avx512_4, crate::platform::avx512_detected()),
            (8, hash_chunks_with_partial_avx512_8, crate::platform::avx512_detected()),
            (16, hash_chunks_with_partial_avx512_16, crate::platform::avx512_detected()),
        ];
        for &(w, kernel, present) in kernels {
            if !present {
                continue;
            }
            for whole in 1..w {
                for blocks in 1..=16 {
                    for partial in [64 * blocks - 63, 64 * blocks] {
                        if partial >= CHUNK_LEN {
                            continue;
                        }
                        for counter in [0, (1 << 32) - 2] {
                            let chunks: std::vec::Vec<&[u8; CHUNK_LEN]> = input.chunks_exact(CHUNK_LEN).take(whole).map(|c| c.try_into().unwrap()).collect();
                            let rest = &input[whole * CHUNK_LEN..][..partial];
                            let mut out = [0u8; 16 * OUT_LEN];
                            // Sound: the CPU has the kernel's instructions.
                            unsafe { kernel(&chunks, rest, crate::IV, counter, crate::KEYED_HASH, &mut out) };
                            for (i, chunk) in input[..whole * CHUNK_LEN + partial].chunks(CHUNK_LEN).enumerate() {
                                let mut state = crate::ChunkState::new(crate::IV, counter + i as u64, crate::KEYED_HASH, crate::platform::Platform::SSE41);
                                state.update(chunk);
                                assert_eq!(
                                    &out[i * OUT_LEN..][..OUT_LEN],
                                    &state.output().chaining_value(),
                                    "{w} lanes: {whole} whole, {partial} B partial, counter {counter}, chunk {i}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// Official BLAKE3 1.8.7 check for `SELF_TEST_PARTIAL16_EXPECTED` and
    /// `SELF_TEST_HASH_MANY8_EXPECTED`.
    #[test]
    fn self_test_expected_folds_match_official_blake3() {
        use blake3::hazmat::HasherExt;
        let buf = self_test_buf();
        let mut iv_bytes = [0u8; 32];
        for (i, &w) in crate::IV.iter().enumerate() {
            iv_bytes[4 * i..4 * i + 4].copy_from_slice(&w.to_le_bytes());
        }
        let mut cvs = [0u8; (SELF_TEST_WHOLE + 1) * OUT_LEN];
        for (i, chunk) in buf.chunks(CHUNK_LEN).enumerate() {
            let mut h = blake3::Hasher::new_keyed(&iv_bytes);
            h.set_input_offset((SELF_TEST_COUNTER + i as u64) * CHUNK_LEN as u64);
            h.update(chunk);
            cvs[i * OUT_LEN..(i + 1) * OUT_LEN].copy_from_slice(&h.finalize_non_root());
        }
        let got_partial16 = crate::self_test::fold(&cvs);
        let official_five = blake3::hash(&buf[..5 * CHUNK_LEN]);
        let got_hash_many8 = crate::self_test::fold(official_five.as_bytes());
        assert_eq!(SELF_TEST_PARTIAL16_EXPECTED, got_partial16, "SELF_TEST_PARTIAL16_EXPECTED should be {got_partial16:#018x}");
        assert_eq!(SELF_TEST_HASH_MANY8_EXPECTED, got_hash_many8, "SELF_TEST_HASH_MANY8_EXPECTED should be {got_hash_many8:#018x}");
    }
}
