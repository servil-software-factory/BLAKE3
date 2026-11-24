//! Whole chunks and a partial one in one pass, side by side (x86-64): the
//! generated kernels of `c/blake3_partial_x86-64_*` (tools/gen_x86.py),
//! four lanes on SSE4.1, eight on AVX2. The partial chunk takes the lane
//! after the whole ones, its last block flagged CHUNK_END at its own step,
//! and its chaining value is the kernel's snapshot at that step; so the
//! longest chain of dependent blocks is a whole chunk's sixteen, where
//! hashing the partial chunk after the whole ones added its blocks to it.

use crate::{CHUNK_END, CHUNK_LEN, CHUNK_START, CVWords, IV, OUT_LEN};

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
    unsafe { kernel(&mut p) };
    for lane in 0..=n {
        let words = if lane < n { &p.h } else { &p.snap };
        for (i, word) in words.iter().enumerate() {
            out[lane * OUT_LEN + 4 * i..][..4].copy_from_slice(&word[lane].to_le_bytes());
        }
    }
}

/// [`hash_chunks_with_partial`] on four lanes: 1 to 3 whole chunks.
/// Unsafe: the CPU must have SSE4.1.
pub unsafe fn hash_chunks_with_partial_sse41(whole: &[&[u8; CHUNK_LEN]], partial: &[u8], key: &CVWords, counter: u64, flags: u8, out: &mut [u8]) {
    hash_chunks_with_partial::<4>(blake3_servil_partial_sse41, whole, partial, key, counter, flags, out)
}

/// [`hash_chunks_with_partial`] on eight lanes: 1 to 7 whole chunks.
/// Unsafe: the CPU must have AVX2.
pub unsafe fn hash_chunks_with_partial_avx2(whole: &[&[u8; CHUNK_LEN]], partial: &[u8], key: &CVWords, counter: u64, flags: u8, out: &mut [u8]) {
    hash_chunks_with_partial::<8>(blake3_servil_partial_avx2, whole, partial, key, counter, flags, out)
}

pub(crate) fn lane_widths(platform: crate::platform::Platform) -> &'static [usize] {
    match platform {
        crate::platform::Platform::SSE41 => &[4],
        #[cfg(blake3_avx2_ffi)]
        crate::platform::Platform::AVX2 => &[4, 8],
        #[cfg(blake3_avx512_ffi)]
        crate::platform::Platform::AVX512 => {
            if cfg!(target_os = "linux") && crate::platform::wide_avx512() {
                &[4, 8, 16, 32]
            } else {
                &[4, 8, 16]
            }
        }
        #[allow(unreachable_patterns)]
        _ => &[],
    }
}

/// The lanes `count` whole inputs run on when the x86 kernels' cascade would
/// take two or more passes for them (3 -> 4, 5..=7 -> 8, 9..=15 -> 16,
/// 17..=31 -> 32 on wide AVX-512). Every power of two from 4 through the
/// platform's widest kernel is present in `lane_widths`.
pub(crate) fn rounded_lanes(platform: crate::platform::Platform, count: usize) -> Option<usize> {
    if count <= 2 {
        return None;
    }
    lane_widths(platform).iter().copied().find(|&w| w >= count).filter(|&w| w != count)
}

/// Fast-path `platform.hash_many` on powers of two (every power of two from 4
/// through the platform's widest kernel is present in `lane_widths`, so powers
/// of two never pad and larger ones divide evenly by the widest kernel),
/// keeping the 1 KiB rounding scratch and extra dispatch copies out of hot
/// callers' stack frames and cold instruction cache lines (`#376`).
#[inline]
#[allow(clippy::too_many_arguments)]
pub(crate) fn hash_many_rounded<const N: usize>(
    platform: crate::platform::Platform,
    inputs: &[&[u8; N]],
    key: &CVWords,
    counter: u64,
    increment_counter: crate::IncrementCounter,
    flags: u8,
    flags_start: u8,
    flags_end: u8,
    out: &mut [u8],
) {
    if inputs.len().is_power_of_two() {
        platform.hash_many(inputs, key, counter, increment_counter, flags, flags_start, flags_end, out);
    } else {
        hash_many_rounded_slow(platform, inputs, key, counter, increment_counter, flags, flags_start, flags_end, out);
    }
}

#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn hash_many_rounded_slow<const N: usize>(
    platform: crate::platform::Platform,
    inputs: &[&[u8; N]],
    key: &CVWords,
    counter: u64,
    increment_counter: crate::IncrementCounter,
    flags: u8,
    flags_start: u8,
    flags_end: u8,
    out: &mut [u8],
) {
    let widest = lane_widths(platform).last().copied().unwrap_or(usize::MAX);
    let mut inputs = inputs;
    let mut out = out;
    let mut counter = counter;
    if inputs.len() > widest && rounded_lanes(platform, inputs.len() % widest).is_some() {
        while inputs.len() > widest {
            let (head, rest) = inputs.split_at(widest);
            let (head_out, rest_out) = out.split_at_mut(widest * OUT_LEN);
            platform.hash_many(head, key, counter, increment_counter, flags, flags_start, flags_end, head_out);
            if increment_counter.yes() {
                counter += widest as u64;
            }
            inputs = rest;
            out = rest_out;
        }
    }
    if let Some(lanes) = rounded_lanes(platform, inputs.len()) {
        crate::ran(crate::Ran::RoundedUp);
        let last = *inputs.last().expect("a count that rounds up is 3 or more");
        let mut padded = crate::stack_vec::StackVec::<&[u8; N], 32>::new();
        padded.extend_from_slice(inputs);
        while padded.len() < lanes {
            padded.push(last);
        }
        #[cfg(all(test, feature = "std"))]
        crate::ROUNDED_WIDTH.with(|w| w.set(w.get() | padded.len()));
        let mut scratch = [0u8; 32 * OUT_LEN];
        platform.hash_many(&padded, key, counter, increment_counter, flags, flags_start, flags_end, &mut scratch[..lanes * OUT_LEN]);
        out[..inputs.len() * OUT_LEN].copy_from_slice(&scratch[..inputs.len() * OUT_LEN]);
    } else {
        platform.hash_many(inputs, key, counter, increment_counter, flags, flags_start, flags_end, out);
    }
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
    }

    /// Every count of whole chunks each kernel takes, with partial chunks
    /// of every block count (one byte and whole) and at a counter whose low
    /// word carries, against the portable chunk values.
    #[test]
    fn test_partial_kernels_against_portable() {
        let mut input = [0u8; 8 * CHUNK_LEN];
        crate::test::paint_test_input(&mut input);
        let kernels: [(usize, unsafe fn(&[&[u8; CHUNK_LEN]], &[u8], &CVWords, u64, u8, &mut [u8]), bool); 2] =
            [(4, hash_chunks_with_partial_sse41, crate::platform::sse41_detected()), (8, hash_chunks_with_partial_avx2, crate::platform::avx2_detected())];
        for (w, kernel, present) in kernels {
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
                            let mut out = [0u8; 8 * OUT_LEN];
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
}
