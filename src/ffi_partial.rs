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

/// Whether a fresh hasher in `update_each` (`hold`) on x86-64 should stream its
/// first chunk straight into `ChunkState`: keyed/derived hashers do so that
/// `finalize_each` finishes their final block side by side via 16-lane
/// `output16` when 16 eligible descriptors pack together; plain hashers stage
/// instead so `finalize_each` hashes 1 KiB chunks via `hash_many_rounded` and
/// multi-block `65..=1023 B` prefixes on AVX2/AVX-512 via `hash_by_blocks`
/// (#404). Non-consuming mid-chunk `finalize()` on a staged plain first chunk
/// rehashes the staged prefix fresh.
#[inline(always)]
pub(crate) fn first_chunk_to_core(stage_empty: bool, hold: bool, is_plain: bool, core_count: u64, input_len: usize) -> bool {
    stage_empty && hold && !is_plain && core_count + input_len as u64 <= CHUNK_LEN as u64
}

/// x86-64 `finalize_each` (called for `which.len() >= 2`): partition each
/// 128-hasher window into staged plain whole chunks (`== 1024 B`, full-width
/// `hash_many_rounded` when `>= 2`), staged plain multi-block short messages
/// (`65..=1023 B`, on AVX2/AVX-512 batched via `hash_by_blocks` for equal
/// prefix blocks with scalar final blocks when `>= 4` or `>= 2` with a message
/// `> 128 B`; SSE4.1 finishes staged `< 1 KiB` messages individually), and
/// remaining hashers (`<= 64 B` plain, keyed/derived, or multi-chunk; Linux
/// AVX-512 packs eligible `ffi_output::input` descriptors independently and
/// runs 16-lane `output16` on every complete 16-pack). Key values never select
/// grouping or dispatch (#404).
#[cfg(feature = "std")]
pub(crate) fn finalize_each(hashers: &mut [crate::Hasher], which: &[usize], out: &mut [[u8; OUT_LEN]]) {
    const FINALIZE_BATCH: usize = 128;
    const FULL_CHUNK_BATCH: usize = 32;
    const SHORT_PAIR_MIN_LEN: usize = 2 * crate::BLOCK_LEN;
    const { assert!(FINALIZE_BATCH <= u8::MAX as usize) };
    let platform = crate::platform::Platform::detect();
    let block_lanes = crate::pair::takes_platform(platform);
    #[cfg(all(blake3_avx512_ffi, target_os = "linux"))]
    let output16_lanes = matches!(platform, crate::platform::Platform::AVX512);
    #[cfg(all(test, blake3_avx512_ffi, target_os = "linux"))]
    let mut counted_output_turn = false;
    for (w_chunk, o_chunk) in which.chunks(FINALIZE_BATCH).zip(out.chunks_mut(FINALIZE_BATCH)) {
        let mut full_chunks = crate::stack_vec::StackVec::<u8, FINALIZE_BATCH>::new();
        let mut short_plain = crate::stack_vec::StackVec::<u8, FINALIZE_BATCH>::new();
        let mut short_plain_max_len = 0usize;
        #[cfg(all(blake3_avx512_ffi, target_os = "linux"))]
        let mut output16 = crate::stack_vec::StackVec::<u8, FINALIZE_BATCH>::new();
        let mut rest = crate::stack_vec::StackVec::<u8, FINALIZE_BATCH>::new();
        for (k, &i) in w_chunk.iter().enumerate() {
            let k = k.to_le_bytes()[0];
            if hashers[i].is_plain_single_chunk_staged() {
                let len = hashers[i].staged_bytes().len();
                if len == CHUNK_LEN {
                    full_chunks.push(k);
                    continue;
                }
                if block_lanes && len > crate::BLOCK_LEN {
                    short_plain.push(k);
                    short_plain_max_len = short_plain_max_len.max(len);
                    continue;
                }
            }
            #[cfg(all(blake3_avx512_ffi, target_os = "linux"))]
            if output16_lanes && crate::ffi_output::input(&hashers[i]).is_some() {
                output16.push(k);
                continue;
            }
            rest.push(k);
        }
        if full_chunks.len() >= 2 {
            for part in full_chunks.as_slice().chunks(FULL_CHUNK_BATCH) {
                let refs: crate::stack_vec::StackVec<&[u8; CHUNK_LEN], FULL_CHUNK_BATCH> =
                    part.iter().map(|&k| hashers[w_chunk[usize::from(k)]].staged_bytes().try_into().unwrap()).collect();
                let mut digests = [[0u8; OUT_LEN]; FULL_CHUNK_BATCH];
                crate::hash_many_rounded(
                    platform,
                    &refs,
                    crate::IV,
                    0,
                    crate::IncrementCounter::No,
                    0,
                    CHUNK_START,
                    CHUNK_END | crate::ROOT,
                    digests[..part.len()].as_flattened_mut(),
                );
                for (&k, digest) in part.iter().zip(&digests) {
                    o_chunk[usize::from(k)] = *digest;
                }
            }
        } else {
            for &k in full_chunks.as_slice() {
                let k = usize::from(k);
                o_chunk[k] = *hashers[w_chunk[k]].take_root_hash().as_bytes();
            }
        }
        // Two 1-prefix-block messages (`65..=128 B`) cost less scalar than
        // sorting/scattering through `hash_by_blocks`; 4+ or any >= 2-block
        // prefix (`> 128 B`) pays for SIMD prefix batching.
        if short_plain.len() >= 4 || (short_plain.len() >= 2 && short_plain_max_len > SHORT_PAIR_MIN_LEN) {
            for part in short_plain.as_slice().chunks(crate::each::BY_BLOCKS_AT_ONCE) {
                let bytes: crate::stack_vec::StackVec<&[u8], { crate::each::BY_BLOCKS_AT_ONCE }> =
                    part.iter().map(|&k| hashers[w_chunk[usize::from(k)]].staged_bytes()).collect();
                let mut digests = [[0u8; OUT_LEN]; crate::each::BY_BLOCKS_AT_ONCE];
                crate::each::hash_by_blocks(&bytes, crate::IV, 0, &mut digests[..part.len()], platform);
                for (&k, digest) in part.iter().zip(&digests) {
                    o_chunk[usize::from(k)] = *digest;
                }
            }
        } else {
            for &k in short_plain.as_slice() {
                let k = usize::from(k);
                o_chunk[k] = *hashers[w_chunk[k]].take_root_hash().as_bytes();
            }
        }
        #[cfg(all(blake3_avx512_ffi, target_os = "linux"))]
        {
            let done = if output16.len() >= 16 {
                #[cfg(test)]
                if !counted_output_turn {
                    crate::counted(crate::Counted::OutputGroupTurn);
                    counted_output_turn = true;
                }
                let mut done = 0usize;
                for slots in output16.as_slice().chunks_exact(16) {
                    let inputs: crate::stack_vec::StackVec<_, 16> =
                        slots.iter().map(|&k| crate::ffi_output::input(&hashers[w_chunk[usize::from(k)]]).unwrap()).collect();
                    let mut digests = [[0u8; OUT_LEN]; 16];
                    // Sound: `output16` is only populated when `platform` is `Platform::AVX512`;
                    // borrowed descriptors keep every block/CV readable and `digests` holds 16 slots.
                    unsafe { crate::ffi_output::hash_group::<16>(inputs.as_slice().try_into().unwrap(), &mut digests) };
                    for (&k, digest) in slots.iter().zip(&digests) {
                        o_chunk[usize::from(k)] = *digest;
                    }
                    done += 16;
                }
                done
            } else {
                0
            };
            for &k in &output16.as_slice()[done..] {
                let ku = usize::from(k);
                o_chunk[ku] = *hashers[w_chunk[ku]].take_root_hash().as_bytes();
            }
        }
        for &k in rest.as_slice() {
            let ku = usize::from(k);
            o_chunk[ku] = *hashers[w_chunk[ku]].take_root_hash().as_bytes();
        }
        for &i in w_chunk {
            hashers[i].reset();
        }
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
