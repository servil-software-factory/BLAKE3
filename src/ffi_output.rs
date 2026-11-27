//! Eight or sixteen independent counter-zero compressions, one full-round compression
//! per lane. Descriptors borrow blocks and CVs; keys, lengths and flags may
//! differ between lanes. ROOT selects root digests; other blocks produce CVs.
#[cfg(test)]
use crate::hasher;
use crate::output_common::self_test_shape;
pub(crate) use crate::output_common::{Input, check_self_test, input};
use crate::{BLOCK_LEN, CHUNK_END, CHUNK_LEN, CHUNK_START, Hasher, OUT_LEN, ROOT, platform::Platform};

#[repr(C, align(64))]
struct Params8 {
    msg: [*const u8; 8],
    cvs: [[u32; 8]; 8],
    lens: [u32; 8],
    flags: [u32; 8],
}
const _: () = {
    assert!(core::mem::size_of::<Params8>() == 384);
    assert!(core::mem::offset_of!(Params8, cvs) == 64);
    assert!(core::mem::offset_of!(Params8, lens) == 320);
    assert!(core::mem::offset_of!(Params8, flags) == 352);
};
unsafe extern "C" {
    fn blake3_servil_output16(inputs: *const Input<'_>, out: *mut u8);
    fn blake3_servil_output8(params: *const Params8, out: *mut u8);
}

/// A packed message's descriptor (`Packed<16>`, `Packed<32>`): its block
/// zero-padded past `len` bytes, the key as its chaining value, and the
/// flags of a whole one-chunk root (CHUNK_START, CHUNK_END and ROOT with
/// the mode's). The caller checks `len <= BLOCK_LEN` for the batch.
pub(crate) fn packed<'a>(block: &'a [u8; BLOCK_LEN], key: &'a [u32; 8], len: u8, flags: u8) -> Input<'a> {
    Input { block, cv: key, len, flags: flags | CHUNK_START | CHUNK_END | ROOT }
}

/// Final block descriptor for a multi-block message inside chunk 0 whose
/// preceding full blocks already compressed into `cv`.
pub(crate) fn final_block<'a>(block: &'a [u8; BLOCK_LEN], cv: &'a [u32; 8], len: u8, flags: u8) -> Input<'a> {
    Input { block, cv, len, flags: flags | CHUNK_END | ROOT }
}

/// Arbitrary counter-zero final block descriptor (for variable-length items in
/// `hash_each`, where `k == 0` includes `CHUNK_START` and `k > 0` does not).
pub(crate) fn item_last<'a>(block: &'a [u8; BLOCK_LEN], cv: &'a [u32; 8], len: u8, flags: u8) -> Input<'a> {
    Input { block, cv, len, flags }
}

/// Minimum remaining one-block or short-last-block messages to pad a 16-lane
/// AVX-512 `output16` call instead of `output8` + scalar.
pub(crate) const X86_PAD16_MIN: usize = 9;
/// Minimum remaining one-block or short-last-block messages to pad an 8-lane
/// AVX2 `output8` call instead of running serial compression (4 wins on every
/// tested x86 CPU including Xeon 6330, where 3 read +12% slower).
pub(crate) const X86_PAD8_MIN: usize = 4;

/// Partition `count` items across `width`-lane (`16` on AVX-512, `8` on AVX2)
/// full and padded output groups, calling `run(start, len, lane_width)` for
/// each group (`lane_width` in `{8, 16}`, `1 <= len <= lane_width`) and
/// returning the total number of items dispatched (leaving `0..4` trailing
/// items for the caller's serial tail when `width > 0`, or `count` when
/// `width == 0`).
#[inline]
pub(crate) fn for_each_output_group(count: usize, width: usize, mut run: impl FnMut(usize, usize, usize)) -> usize {
    if width == 0 {
        return 0;
    }
    let groups = count / width;
    for g in 0..groups {
        run(g * width, width, width);
    }
    let done = groups * width;
    let rem = count - done;
    if rem < X86_PAD8_MIN {
        return done;
    }
    // `rem >= 9` is reachable only when `width == 16` (`rem < width`).
    run(done, rem, if rem >= X86_PAD16_MIN { 16 } else { 8 });
    count
}

/// Compress `1..=W` independent descriptors on the `W`-lane (`8` or `16`)
/// output kernel directly into a caller-supplied `W`-digest buffer `out`,
/// padding spare lanes by repeating the last descriptor when `count < W`.
/// Use this when the caller already owns a `W`-slot scratch buffer (e.g.
/// scatter callers) to avoid a second stack buffer and copy.
#[inline]
pub(crate) fn hash_padded_group_into<'a, const W: usize>(platform: Platform, count: usize, make: impl Fn(usize) -> Input<'a>, out: &mut [[u8; OUT_LEN]; W]) {
    assert!((W == 8 && lanes(platform) >= 8) || (W == 16 && lanes(platform) == 16), "platform does not support {W}-lane output kernel");
    assert!(count > 0 && count <= W, "padded output group holds 1..=W messages");
    let inputs: [Input<'_>; W] = core::array::from_fn(|j| make(j.min(count - 1)));
    // Sound: asserted above that `platform` supports `W`-lane output16/output8;
    // `out` holds all `W` digests.
    unsafe { hash_group::<W>(&inputs, out) };
}

/// Compress `1..=W` independent descriptors on the `W`-lane (`8` or `16`)
/// output kernel into an exact-length `out` slice (`out.len() >= count`),
/// writing directly when `count == W` or through a `W`-digest stack buffer
/// when `count < W` so bytes past `out[..count]` are never clobbered.
#[inline]
pub(crate) fn hash_padded_group<'a, const W: usize>(platform: Platform, count: usize, make: impl Fn(usize) -> Input<'a>, out: &mut [[u8; OUT_LEN]]) {
    assert!(out.len() >= count, "output slice holds at least count digests");
    if count == W {
        assert!((W == 8 && lanes(platform) >= 8) || (W == 16 && lanes(platform) == 16), "platform does not support {W}-lane output kernel");
        let inputs: [Input<'_>; W] = core::array::from_fn(|j| make(j));
        // Sound: asserted above that `platform` supports `W`-lane output16/output8;
        // `out[..W]` holds `W` digests.
        unsafe { hash_group::<W>(&inputs, &mut out[..W]) };
    } else {
        let mut scratch = [[0u8; OUT_LEN]; W];
        hash_padded_group_into::<W>(platform, count, make, &mut scratch);
        out[..count].copy_from_slice(&scratch[..count]);
    }
}

/// The whole and padded groups of `digests.len()` one-block messages (each
/// `len` bytes of a 64-byte slot in `bytes`, zero past them: a batch's
/// padding, or the remainder buffer `hash_packed` writes) that run on the
/// x86-64 independent-compression kernels side by side: how many messages
/// they took.
#[inline(never)]
pub(crate) fn one_block_groups(bytes: &[u8], digests: &mut [[u8; OUT_LEN]], len: usize, key: &crate::CVWords, flags: u8, platform: Platform) -> usize {
    assert!(len <= BLOCK_LEN, "an independent short message fits in one block");
    let len = u8::try_from(len).expect("len <= BLOCK_LEN");
    let width = lanes(platform);
    for_each_output_group(digests.len(), width, |start, count, lane_width| {
        let group = &bytes[start * BLOCK_LEN..];
        let out = &mut digests[start..start + count];
        let make = |i: usize| {
            let block: &[u8; BLOCK_LEN] = group[i * BLOCK_LEN..][..BLOCK_LEN].try_into().unwrap();
            packed(block, key, len, flags)
        };
        if lane_width == 16 {
            hash_padded_group::<16>(platform, count, make, out);
        } else {
            hash_padded_group::<8>(platform, count, make, out);
        }
    })
}

/// Padded lane count for a single `<= TILE` tile of multi-block prefix
/// `hash_many_blocks`: round `9..15` lanes up to 16 on AVX-512 (`width == 16`)
/// and `5..7` lanes up to 8 on both AVX-512 and AVX2 so prefix hashing avoids
/// scalar tails.
pub(crate) fn prefix_padded_count(count: usize, width: usize) -> usize {
    if width == 16 && count % 16 >= X86_PAD16_MIN {
        count.next_multiple_of(16)
    } else if count % 8 >= 5 {
        count.next_multiple_of(8)
    } else {
        count
    }
}

/// One `4..=TILE` (`TILE` in `{8, 16}`) tile of multi-block short-last messages:
/// sizes the intermediate CV arrays (`cv_bytes` + `cvs`: `512 B` for `TILE = 8`,
/// `1 KiB` for `TILE = 16`) to the active tile width rather than the full 128-lane
/// table.
#[inline(never)]
fn multi_block_short_last_tile<const TILE: usize>(
    messages: &[u8],
    slot: usize,
    key: &crate::CVWords,
    flags: u8,
    last_len_u8: u8,
    outputs: &mut [[u8; OUT_LEN]],
    platform: Platform,
) {
    let count = outputs.len();
    assert!((X86_PAD8_MIN..=TILE).contains(&count), "tile holds 4..=TILE messages");
    let prefix_len = slot - BLOCK_LEN;
    let prefix_blocks = prefix_len / BLOCK_LEN;
    let padded_count = prefix_padded_count(count, TILE);
    let mut ptrs = crate::stack_vec::StackVec::<*const u8, TILE>::new();
    for i in 0..count {
        ptrs.push(messages[i * slot..].as_ptr());
    }
    let last_ptr = ptrs[count - 1];
    ptrs.extend(core::iter::repeat_n(last_ptr, padded_count - count));
    let mut cv_bytes = [[0u8; OUT_LEN]; TILE];
    // Sound: every pointer in `ptrs` reaches `prefix_blocks` whole blocks inside
    // its slot in `messages`, and `cv_bytes[..padded_count]` holds `padded_count` CVs.
    unsafe {
        crate::pair::hash_many_blocks(platform, &ptrs, prefix_blocks, key, 0, false, flags, CHUNK_START, 0, cv_bytes[..padded_count].as_flattened_mut());
    }
    let mut cvs = [[0u32; 8]; TILE];
    for i in 0..count {
        cvs[i] = core::array::from_fn(|w| u32::from_le_bytes(cv_bytes[i][4 * w..4 * w + 4].try_into().unwrap()));
    }
    let last_block = |i: usize| -> &[u8; BLOCK_LEN] { messages[i * slot + prefix_len..][..BLOCK_LEN].try_into().unwrap() };
    hash_padded_group::<TILE>(platform, count, |idx| final_block(last_block(idx), &cvs[idx], last_len_u8, flags), outputs);
}

/// Multi-block messages (`slot = 2..=16` blocks) whose final block is short
/// (`1..64` bytes, zero-padded in each slot) on x86-64 AVX-512 / AVX2: tile
/// by `width` (`16` on AVX-512, `8` on AVX2), dispatching `4..=8` through the
/// `512 B` 8-lane tile and `9..=16` through the `1 KiB` 16-lane tile, and
/// running trailing `1..3` messages serially.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn multi_block_short_last(
    input: &[u8],
    slot: usize,
    key: &crate::CVWords,
    flags: u8,
    last_len: usize,
    outputs: &mut [[u8; OUT_LEN]],
    platform: Platform,
    width: usize,
) {
    assert!(last_len > 0 && last_len < BLOCK_LEN, "short final block");
    let prefix_len = slot - BLOCK_LEN;
    let last_len_u8 = u8::try_from(last_len).expect("last block <= 64");
    let done = for_each_output_group(outputs.len(), width, |start, count, lane_width| {
        let messages = &input[start * slot..(start + count) * slot];
        let digests = &mut outputs[start..start + count];
        if lane_width == 16 {
            multi_block_short_last_tile::<16>(messages, slot, key, flags, last_len_u8, digests, platform);
        } else {
            multi_block_short_last_tile::<8>(messages, slot, key, flags, last_len_u8, digests, platform);
        }
    });
    for (i, output) in outputs[done..].iter_mut().enumerate() {
        *output = *crate::hash_serial_on(&input[(done + i) * slot..][..prefix_len + last_len], key, flags, platform).as_bytes();
    }
}

/// Padded lane count for a `<= 16`-item prefix bucket in `each::by_blocks_at_once`:
/// rounds `9..=15 -> 16` on AVX-512, `5..=7 -> 8` on AVX2/AVX-512, and
/// `13..=15 -> 16` on AVX2 (avoiding a 5..7-lane scalar tail after 8 lanes).
pub(crate) fn by_blocks_prefix_padded(n: usize, width: usize) -> usize {
    if width == 16 && n >= X86_PAD16_MIN {
        16
    } else if width != 0 && (5..8).contains(&n) {
        8
    } else if width == 8 && (13..16).contains(&n) {
        16
    } else {
        n
    }
}

/// Pad `inputs` in place for a `<= 16`-item prefix bucket in
/// `each::by_blocks_at_once`, returning the padded lane count.
pub(crate) fn pad_by_blocks_inputs<const GROUP: usize>(inputs: &mut crate::stack_vec::StackVec<*const u8, GROUP>, platform: Platform) -> usize {
    let n = inputs.len();
    let target = by_blocks_prefix_padded(n, lanes(platform));
    let last = *inputs.last().unwrap();
    inputs.extend(core::iter::repeat_n(last, target - n));
    target
}

/// Finish counter-zero final blocks in `each::by_blocks_at_once` using
/// 16-lane `output16` and 8-lane `output8` for chunks of `>= 4` items,
/// returning how many leading entries of `order` were written into `out`.
pub(crate) fn finish_last_blocks_order(
    order: &[u8],
    items: &[&[u8]],
    cvs: &[crate::CVWords],
    counter: u64,
    flags: u8,
    flags_end: u8,
    out: &mut [[u8; OUT_LEN]],
    platform: Platform,
) -> usize {
    let width = lanes(platform);
    if counter != 0 || width == 0 {
        return 0;
    }
    for_each_output_group(order.len(), width, |start, len, lane_width| {
        let chunk = &order[start..start + len];
        if lane_width == 16 {
            finish_last_blocks_simd::<16>(platform, chunk, items, cvs, flags, flags_end, out);
        } else {
            finish_last_blocks_simd::<8>(platform, chunk, items, cvs, flags, flags_end, out);
        }
    })
}

/// Compress `4..=W` (`W` in `{8, 16}`) counter-zero final blocks for
/// `each::by_blocks_at_once` in a single `output8`/`output16` call, padding
/// spare lanes by repeating the last item in `chunk`.
#[inline(never)]
fn finish_last_blocks_simd<const W: usize>(
    platform: Platform,
    chunk: &[u8],
    items: &[&[u8]],
    cvs: &[crate::CVWords],
    flags: u8,
    flags_end: u8,
    out: &mut [[u8; OUT_LEN]],
) {
    assert!((W == 8 && lanes(platform) >= 8) || (W == 16 && lanes(platform) == 16), "platform does not support {W}-lane output kernel");
    let count = chunk.len();
    let mut blocks = [[0u8; BLOCK_LEN]; W];
    let mut lens = [0u8; W];
    let mut item_flags = [0u8; W];
    for (j, &i) in chunk.iter().enumerate() {
        let item = items[usize::from(i)];
        let k = item.len().saturating_sub(1) / BLOCK_LEN;
        let last = item.len() - k * BLOCK_LEN;
        blocks[j][..last].copy_from_slice(&item[k * BLOCK_LEN..]);
        lens[j] = u8::try_from(last).expect("last block <= 64");
        let start = if k == 0 { CHUNK_START } else { 0 };
        item_flags[j] = flags | start | flags_end;
    }
    let mut scratch = [[0u8; OUT_LEN]; W];
    hash_padded_group_into::<W>(platform, count, |idx| item_last(&blocks[idx], &cvs[usize::from(chunk[idx])], lens[idx], item_flags[idx]), &mut scratch);
    for (j, &i) in chunk.iter().enumerate() {
        out[usize::from(i)] = scratch[j];
    }
}

/// Finish packed counter-zero eligible `slots` from `each::finalize_each`
/// using 16-lane `output16` (full 16s and padded 9..=15 tails on AVX-512) and
/// 8-lane `output8` (full 8s and padded 4..=7 tails on both AVX-512 and AVX2),
/// returning how many leading entries of `slots` were written into `o_chunk`.
pub(crate) fn finalize_output_slots(hashers: &[Hasher], w_chunk: &[usize], slots: &[u8], o_chunk: &mut [[u8; OUT_LEN]], platform: Platform) -> usize {
    let width = lanes(platform);
    for_each_output_group(slots.len(), width, |start, len, lane_width| {
        let part = &slots[start..start + len];
        let make = |j: usize| input(&hashers[w_chunk[usize::from(part[j])]]).unwrap();
        if lane_width == 16 {
            let mut digests = [[0u8; OUT_LEN]; 16];
            hash_padded_group_into::<16>(platform, len, make, &mut digests);
            for (&k, digest) in part.iter().zip(&digests[..len]) {
                o_chunk[usize::from(k)] = *digest;
            }
        } else {
            let mut digests = [[0u8; OUT_LEN]; 8];
            hash_padded_group_into::<8>(platform, len, make, &mut digests);
            for (&k, digest) in part.iter().zip(&digests[..len]) {
                o_chunk[usize::from(k)] = *digest;
            }
        }
    })
}

pub(crate) fn lanes(platform: Platform) -> usize {
    match platform {
        Platform::AVX512 => 16,
        Platform::AVX2 => 8,
        _ => 0,
    }
}

#[cfg(test)]
std::thread_local! {
    /// Widths (`8` or `16`) passed to `hash_group` on this thread, for dispatch tests.
    pub(crate) static OUTPUT_CALLS: core::cell::RefCell<std::vec::Vec<usize>> = const { core::cell::RefCell::new(std::vec::Vec::new()) };
}

/// Requires AVX2 for W=8, AVX-512F+VL for W=16, on Linux. Every descriptor
/// borrows a readable padded block/CV, length 0..64. All counters are zero.
pub(crate) unsafe fn hash_group<const W: usize>(inputs: &[Input<'_>; W], out: &mut [[u8; OUT_LEN]]) {
    assert_eq!(out.len(), W, "one compression output per lane");
    #[cfg(test)]
    let _ = OUTPUT_CALLS.try_with(|c| c.borrow_mut().push(W));
    if W == 16 {
        crate::ran_kernel(blake3_servil_output16 as *const ());
        // Sound: caller supplies the ISA; fixed descriptors and checked
        // output provide readable inputs and room for every store.
        unsafe { blake3_servil_output16(inputs.as_ptr(), out.as_mut_ptr().cast()) };
    } else {
        assert_eq!(W, 8, "eight or sixteen compression lanes");
        let p = Params8 {
            msg: core::array::from_fn(|i| inputs[i].block.as_ptr()),
            cvs: core::array::from_fn(|w| core::array::from_fn(|i| inputs[i].cv[w])),
            lens: core::array::from_fn(|i| inputs[i].len as u32),
            flags: core::array::from_fn(|i| inputs[i].flags as u32),
        };
        crate::ran_kernel(blake3_servil_output8 as *const ());
        // Sound: caller supplies AVX2; all borrowed blocks reach64bytes,
        // and the parameter layout and output size are checked above.
        unsafe { blake3_servil_output8(&p, out.as_mut_ptr().cast()) };
    }
}

const HELD_TILE: usize = 16;

/// Hash complete held stages without grouping by their keys. Leaf calls
/// already fill their chunk lanes; parents borrow each message's own key
/// and flags in the existing independent-compression descriptors. The
/// caller supplies 1..=TABLE stages and a detected AVX2/AVX512 platform.
/// All turns tile in `HELD_TILE` (16-stage) chunks (`held_groups_in`),
/// avoiding a 128-stage stack frame (~84 KiB / 85,952 B smaller frame on
/// x86-64 release CGU=1 builds).
pub(crate) fn held_groups(hashers: &mut [Hasher], held: &[usize], platform: Platform) {
    for tile in held.chunks(HELD_TILE) {
        held_groups_in(hashers, tile, platform);
    }
}

#[inline(never)]
fn held_groups_in(hashers: &mut [Hasher], held: &[usize], platform: Platform) {
    let width = lanes(platform);
    assert!(!held.is_empty() && width != 0, "held_groups_in requires non-empty held slice and SIMD width");
    const PER: usize = crate::STAGE_LEN / CHUNK_LEN;
    const { assert!(PER == 16) };
    let n = held.len();
    let mut work = crate::each::HeldWorkspace::<HELD_TILE>::new();
    let (mut level, mut next) = work.buffers(n);
    crate::each::hash_held_leaves_serial(hashers, held, platform, level);
    for depth in 0..4 {
        let per_message = PER >> (depth + 1);
        let count = n * per_message;
        let input = |j: usize| {
            let h = &hashers[held[j / per_message]];
            let domain = h.domain();
            Input {
                block: level.as_flattened()[j * BLOCK_LEN..(j + 1) * BLOCK_LEN].try_into().unwrap(),
                cv: domain.key(),
                len: u8::try_from(BLOCK_LEN).unwrap(),
                flags: domain.flags() | crate::PARENT,
            }
        };
        let mut full = count / width * width;
        for base in (0..full).step_by(width) {
            // Sound: width matches the detected ISA. All blocks and keys
            // are readable; next supplies disjoint complete outputs.
            if width == 16 {
                let inputs = core::array::from_fn(|j| input(base + j));
                unsafe { hash_group::<16>(&inputs, &mut next[base..base + 16]) };
            } else {
                let inputs = core::array::from_fn(|j| input(base + j));
                unsafe { hash_group::<8>(&inputs, &mut next[base..base + 8]) };
            }
        }
        if width == 16 && count - full >= 8 {
            let inputs = core::array::from_fn(|j| input(full + j));
            // Sound: `width == 16` means `Platform::AVX512`, which implies AVX2;
            // eight real parent blocks and keys remain, and next has disjoint room.
            unsafe { hash_group::<8>(&inputs, &mut next[full..full + 8]) };
            full += 8;
        }
        for (j, out) in next.iter_mut().enumerate().take(count).skip(full) {
            let input = input(j);
            let mut cv = *input.cv;
            platform.compress_in_place(&mut cv, input.block, input.len, 0, input.flags);
            *out = crate::hasher::le_bytes(&cv);
        }
        core::mem::swap(&mut level, &mut next);
    }
    crate::each::commit_held_groups(hashers, held, level, next);
}

unsafe fn hash_eight_groups(inputs: &[Input<'_>; 16], out: &mut [[u8; OUT_LEN]]) {
    for (inputs, out) in inputs.chunks_exact(8).zip(out.chunks_mut(8)) {
        // Sound: caller verified AVX2/AVX-512 support; each chunk has exact length 8 matching `hash_group::<8>`.
        unsafe { hash_group::<8>(inputs.try_into().unwrap(), out) };
    }
}
unsafe fn hash_on_platform(platform: Platform, inputs: &[Input<'_>; 16], out: &mut [[u8; OUT_LEN]]) {
    if lanes(platform) == 16 {
        // Sound: `lanes(platform) == 16` guarantees AVX-512F+VL support; `inputs` and `out` hold 16 entries.
        unsafe { hash_group::<16>(inputs, out) };
    } else {
        assert_eq!(lanes(platform), 8);
        // Sound: `lanes(platform) == 8` guarantees AVX2 support; `inputs` and `out` hold 16 entries.
        unsafe { hash_eight_groups(inputs, out) };
    }
}
/// Planted corruption selector for subprocess fail-stop tests:
/// 1 = corrupt last output byte after 16-lane pass (before 8-lane overwrite),
/// 2 = corrupt last output byte of the final pass.
#[cfg(test)]
pub(crate) static PLANTED_WRONG_OUTPUT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

#[inline(never)]
pub(crate) fn self_test(platform: Platform) -> usize {
    if lanes(platform) == 0 {
        return 0;
    }
    let mut checked = 1;
    let answer = self_test_shape(|inputs, out| {
        // Sound: initialize detected this platform before testing its kernels.
        unsafe { hash_on_platform(platform, inputs, out) };
        if lanes(platform) == 16 {
            #[cfg(test)]
            if PLANTED_WRONG_OUTPUT.load(core::sync::atomic::Ordering::Relaxed) == 1 {
                out[15][OUT_LEN - 1] = !out[15][OUT_LEN - 1];
            }
            // Check the wide output and poison `out` before the remainder
            // kernel writes into it so a silent no-op cannot pass on stale bytes.
            check_self_test(crate::self_test::fold(out.as_flattened()));
            out.fill([0xa5; OUT_LEN]);
            // Sound: `lanes(platform) == 16` means `Platform::AVX512`, which implies AVX2; `inputs` and `out` hold 16 entries.
            unsafe { hash_eight_groups(inputs, out) };
            checked = 2;
        }
        #[cfg(test)]
        if PLANTED_WRONG_OUTPUT.load(core::sync::atomic::Ordering::Relaxed) == 2 {
            out[15][OUT_LEN - 1] = !out[15][OUT_LEN - 1];
        }
    });
    check_self_test(answer);
    checked
}
#[cfg(test)]
pub(crate) fn entry() -> *const () {
    blake3_servil_output16 as *const ()
}
#[cfg(test)]
pub(crate) fn entry8() -> *const () {
    blake3_servil_output8 as *const ()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output_common::{SELF_TEST_EXPECTED, check_self_test_value};
    #[test]
    fn startup_answers_match_independent_compression() {
        crate::initialize();
        let answer = self_test_shape(|inputs, out| {
            for (input, digest) in inputs.iter().zip(out) {
                let mut cv = *input.cv;
                unsafe { crate::sse41::compress_in_place(&mut cv, input.block, input.len, 0, input.flags) };
                *digest = hasher::le_bytes(&cv);
            }
        });
        assert_eq!(answer, SELF_TEST_EXPECTED);
        if lanes(Platform::detect()) != 0 {
            assert_eq!(self_test_shape(|inputs, out| unsafe { hash_on_platform(Platform::detect(), inputs, out) }), answer);
        }
    }
    #[test]
    fn independent_blocks_cvs_descriptors_and_output_against_guard_pages() {
        use crate::test::guard::Guarded;
        crate::initialize();
        if lanes(Platform::detect()) == 0 {
            return;
        }
        for at_end in [false, true] {
            let answer = self_test_shape(|inputs, out| {
                let blocks: Vec<_> = (0..16)
                    .map(|i| {
                        let mut g = Guarded::new(64, at_end);
                        g.bytes().copy_from_slice(inputs[i].block);
                        g.read_only()
                    })
                    .collect();
                let cvs: Vec<_> = (0..16)
                    .map(|i| {
                        let mut g = Guarded::new(32, at_end);
                        g.bytes().copy_from_slice(&hasher::le_bytes(inputs[i].cv));
                        g.read_only()
                    })
                    .collect();
                let mut table = Guarded::new(core::mem::size_of_val(inputs), at_end);
                let table_ptr = table.bytes().as_mut_ptr().cast::<Input<'_>>();
                for i in 0..16 {
                    // Page placement aligns CVs to u32 and descriptors to
                    // Input. The read-only guarded mappings outlive every view.
                    let block: &[u8; 64] = blocks[i].as_bytes().try_into().unwrap();
                    let cv = unsafe { &*cvs[i].as_bytes().as_ptr().cast::<[u32; 8]>() };
                    unsafe { table_ptr.add(i).write(Input { block, cv, len: inputs[i].len, flags: inputs[i].flags }) };
                }
                let table = table.read_only();
                let descriptors = unsafe { &*table.as_ptr().cast::<[Input<'_>; 16]>() };
                let mut output = Guarded::new(16 * OUT_LEN, at_end);
                let guarded_out = output.digests();
                unsafe { hash_on_platform(Platform::detect(), descriptors, guarded_out) };
                let expected = crate::self_test::fold(guarded_out.as_flattened());
                if crate::platform::avx2_detected() {
                    guarded_out.fill([0xa5; OUT_LEN]);
                    unsafe { hash_eight_groups(descriptors, guarded_out) };
                    assert_eq!(crate::self_test::fold(guarded_out.as_flattened()), expected);
                }
                out.copy_from_slice(guarded_out);
            });
            assert_eq!(answer, SELF_TEST_EXPECTED, "guard placement at_end={at_end}");
        }
    }

    /// Short one-block batches (1 to 63 bytes) and multi-block short-last
    /// batches (65 to 1023 bytes) on Linux x86-64 take `output16` / `output8`
    /// for whole groups and padded tails (`4..=8` on `output8`, `9..=16` on
    /// `output16`), while `1..=3` remainders stay serial. Packed `16/32/48`-byte
    /// remainders of `10..=15` on AVX-512 bypass `avx2_packed` to finish in one
    /// padded `output16` call.
    #[test]
    fn packed_groups_take_the_independent_kernel() {
        use crate::block::Prepared;
        use crate::throughput::hash_many;
        let platform = Platform::detect();
        let width = lanes(platform);
        if width == 0 {
            return;
        }
        let e16 = entry() as usize;
        let e8 = entry8() as usize;
        // Counts 1..=3 stay serial; 4..=8 take output8; on AVX-512, 9..=16 take
        // output16, and 17..=24 take output16 plus optional output8.
        let cases: &[(usize, Option<usize>, usize)] = if width == 16 {
            &[(3, None, 0), (4, Some(e8), 1), (8, Some(e8), 1), (9, Some(e16), 1), (15, Some(e16), 1), (16, Some(e16), 1), (32, Some(e16), 1)]
        } else {
            &[(3, None, 0), (4, Some(e8), 1), (7, Some(e8), 1), (8, Some(e8), 1), (16, Some(e8), 1)]
        };
        for &(count, expected_entry, expected_kernels) in cases {
            let mut out = std::vec![[0u8; OUT_LEN]; count];
            for len in [1, 33, 63] {
                let mut batch = crate::throughput::Batch::new(len, count);
                for i in 0..count {
                    batch.message_mut(i).fill(i as u8);
                }
                crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
                hash_many(crate::Mode::Plain, &batch, &mut out);
                let ran = crate::RAN_ENTRIES.with(|r| r.borrow().clone());
                match expected_entry {
                    Some(k) => {
                        assert!(ran.contains(&k), "{count} batch messages of {len} bytes should reach {k:#x}: {ran:?}");
                        assert_eq!(ran.len(), expected_kernels, "{count} batch messages of {len} bytes on the group kernel alone: {ran:?}");
                    }
                    None => {
                        assert!(!ran.contains(&e16) && !ran.contains(&e8), "{count} batch messages of {len} bytes should stay serial: {ran:?}");
                    }
                }
                for i in 0..count {
                    assert_eq!(out[i], *crate::convenience::hash(&std::vec![i as u8; len]).as_bytes(), "{len} bytes, message {i}");
                }
            }
        }
        // Verify Packed<16/32/48> remainder dispatch on AVX-512 vs AVX2:
        // on AVX-512 Linux std, 8..=9 remainders take `avx2_packed`, while
        // 10..=15 remainders bypass `avx2_packed` and finish in one `output16`.
        let avx2_packed = crate::ffi_packed::entries_avx2();
        let avx512_packed = crate::ffi_packed::entries_avx512();
        for (idx, len) in [16usize, 32, 48].into_iter().enumerate() {
            let p2 = avx2_packed[idx].1 as usize;
            let p512 = avx512_packed[idx].1 as usize;
            for count in [8usize, 9, 10, 11, 15, 24, 25, 26, 27, 31] {
                let mut bytes = std::vec![0u8; count * len];
                for i in 0..count {
                    for j in 0..len {
                        bytes[i * len + j] = ((i * 29 + j * 13 + len) & 0xff) as u8;
                    }
                }
                let mut out = std::vec![[0u8; OUT_LEN]; count];
                crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
                crate::many::hash_many_on(&bytes, len, len, crate::IV, 0, &mut out, platform);
                let ran = crate::RAN_ENTRIES.with(|r| r.borrow().clone());
                if width == 16 {
                    let rem = count % 16;
                    assert_eq!(ran.contains(&p512), count >= 16, "Packed<{len}> count={count} avx512_packed");
                    assert_eq!(ran.contains(&p2), (8..=9).contains(&rem), "Packed<{len}> count={count} avx2_packed");
                    assert_eq!(ran.contains(&e16), (10..=15).contains(&rem), "Packed<{len}> count={count} output16");
                } else {
                    assert!(ran.contains(&p2), "AVX2 Packed<{len}> count={count} avx2_packed");
                    let rem = count % 8;
                    assert_eq!(ran.contains(&e8), rem >= 4, "AVX2 Packed<{len}> count={count} output8");
                }
                for i in 0..count {
                    assert_eq!(out[i], *blake3::hash(&bytes[i * len..][..len]).as_bytes(), "Packed<{len}> count={count} msg {i}");
                }
            }
        }
        // Multi-block messages with a short last block (65..1023 B) tile by
        // platform width across every supported platform, exercising tile-2
        // `< 4` serial remainders (`17..19`, `129..132`) with distinct bytes
        // per slot and mixed-width `output16` + `output8` (`24` on AVX-512).
        let key = [0x42u8; crate::KEY_LEN];
        for plat in crate::test::every_platform() {
            let pw = lanes(plat);
            for len in [65, 100, 200, 500, 1000] {
                for count in [3, 4, 7, 8, 9, 15, 16, 17, 18, 19, 24, 33, 129, 130, 131, 132] {
                    let mut batch = crate::throughput::Batch::new(len, count);
                    for i in 0..count {
                        let msg = batch.message_mut(i);
                        for (j, b) in msg.iter_mut().enumerate() {
                            *b = ((i * 31 + j * 17 + len) & 0xff) as u8;
                        }
                    }
                    let mut out = std::vec![[0u8; OUT_LEN]; count];
                    let slot = batch.slot();
                    for mode in [crate::Mode::Plain, crate::Mode::Keyed(&key), crate::Mode::DeriveKey("x86-short-last")] {
                        let (kw, fl) = match mode {
                            crate::Mode::Plain => (*crate::IV, 0u8),
                            crate::Mode::Keyed(k) => (crate::platform::words_from_le_bytes_32(k), crate::KEYED_HASH),
                            crate::Mode::DeriveKey(ctx) => (
                                crate::platform::words_from_le_bytes_32(crate::hash_serial(ctx.as_bytes(), crate::IV, crate::DERIVE_KEY_CONTEXT).as_bytes()),
                                crate::DERIVE_KEY_MATERIAL,
                            ),
                            crate::Mode::DeriveKeyWith(_) => unreachable!(),
                        };
                        crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
                        crate::many::hash_many_on(batch.bytes(), len, slot, &kw, fl, &mut out, plat);
                        let ran = crate::RAN_ENTRIES.with(|r| r.borrow().clone());
                        if pw == 0 || count < 4 {
                            assert!(!ran.contains(&e16) && !ran.contains(&e8), "{plat:?} {count} x {len} B should stay serial");
                        } else if pw == 16 {
                            let need16 = count >= 9;
                            let need8 = (count < 9) || (4..=8).contains(&(count % 16));
                            assert_eq!(ran.contains(&e16), need16, "{plat:?} {count} x {len} B output16");
                            assert_eq!(ran.contains(&e8), need8, "{plat:?} {count} x {len} B output8");
                        } else {
                            assert!(ran.contains(&e8), "{plat:?} {count} x {len} B should reach output8");
                            assert!(!ran.contains(&e16), "{plat:?} {count} x {len} B should not reach output16");
                        }
                        for i in 0..count {
                            let msg = &batch.bytes()[i * slot..][..len];
                            let expected = match mode {
                                crate::Mode::Plain => *blake3::hash(msg).as_bytes(),
                                crate::Mode::Keyed(k) => *blake3::keyed_hash(k, msg).as_bytes(),
                                crate::Mode::DeriveKey(ctx) => blake3::derive_key(ctx, msg),
                                crate::Mode::DeriveKeyWith(_) => unreachable!(),
                            };
                            assert_eq!(out[i], expected, "{plat:?} {count} x {len} B, msg {i}");
                        }
                    }
                }
            }
        }
        // Pin prefix_padded_count boundaries for AVX2 (8) and AVX-512 (16).
        for (count, w, want) in [
            (4, 8, 4),
            (5, 8, 8),
            (7, 8, 8),
            (8, 8, 8),
            (12, 8, 12),
            (13, 8, 16),
            (4, 16, 4),
            (5, 16, 8),
            (8, 16, 8),
            (9, 16, 16),
            (15, 16, 16),
            (16, 16, 16),
            (20, 16, 20),
            (21, 16, 24),
            (24, 16, 24),
            (25, 16, 32),
        ] {
            assert_eq!(prefix_padded_count(count, w), want, "prefix_padded_count({count}, {w})");
        }
    }

    /// Pin the pure `for_each_output_group` schedule across widths 0/8/16 on
    /// every x86 CPU (including SSE4.1-only hosts where `lanes(detect) == 0`).
    #[test]
    fn for_each_output_group_schedule_all_widths() {
        let schedule = |count: usize, width: usize| -> (usize, std::vec::Vec<(usize, usize, usize)>) {
            let mut got = std::vec::Vec::new();
            let done = for_each_output_group(count, width, |s, l, w| got.push((s, l, w)));
            (done, got)
        };
        assert_eq!(schedule(16, 0), (0, std::vec![]));
        assert_eq!(schedule(3, 8), (0, std::vec![]));
        assert_eq!(schedule(4, 8), (4, std::vec![(0, 4, 8)]));
        assert_eq!(schedule(11, 8), (8, std::vec![(0, 8, 8)]));
        assert_eq!(schedule(12, 8), (12, std::vec![(0, 8, 8), (8, 4, 8)]));
        assert_eq!(schedule(3, 16), (0, std::vec![]));
        assert_eq!(schedule(8, 16), (8, std::vec![(0, 8, 8)]));
        assert_eq!(schedule(9, 16), (9, std::vec![(0, 9, 16)]));
        assert_eq!(schedule(24, 16), (24, std::vec![(0, 16, 16), (16, 8, 8)]));
        assert_eq!(schedule(25, 16), (25, std::vec![(0, 16, 16), (16, 9, 16)]));
    }

    #[test]
    fn startup_covers_each_supported_platform() {
        crate::initialize();
        for platform in crate::test::every_platform() {
            crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
            let checked = self_test(platform);
            let width = lanes(platform);
            let eight = width == 8 || (width == 16 && crate::platform::avx2_detected());
            let sixteen = width == 16;
            assert_eq!(checked, usize::from(eight) + usize::from(sixteen), "startup families on {platform:?}");
            let entries = crate::RAN_ENTRIES.with(|r| r.borrow().clone());
            assert_eq!(entries.contains(&(entry8() as usize)), eight, "eight-lane startup on {platform:?}");
            assert_eq!(entries.contains(&(entry() as usize)), sixteen, "sixteen-lane startup on {platform:?}");
        }
    }

    #[test]
    fn startup_rejects_corrupted_answer() {
        assert!(!check_self_test_value(SELF_TEST_EXPECTED ^ 1));
    }

    #[test]
    fn incremental_roots_batch_all_modes_and_selected_indices() {
        crate::initialize();
        for count in [0usize, 1, 15, 16, 17, 31, 32, 65] {
            let mut hashers = Vec::new();
            let mut expected = Vec::new();
            let mut empty = Vec::new();
            let mut next = Vec::new();
            for i in 0..count {
                let len = [0usize, 1, 31, 63, 64, 65, 127, 255, 511, 1023, 1024][i % 11];
                let data: Vec<u8> = (0..len).map(|j| (i * 37 + j * 17) as u8).collect();
                let key = [i as u8; 32];
                let mode = match i % 3 {
                    0 => crate::Mode::Plain,
                    1 => crate::Mode::Keyed(&key),
                    _ => crate::Mode::DeriveKey("BLAKE3-Servil bulk-root test"),
                };
                // Keyed/derived turns wait in ChunkState; plain turns and
                // <= 64 B updates wait in the stage. Eligible descriptors pack
                // into `output16` when 16 remain in `rest`.
                let mut h = Hasher::new(mode, None);
                if len <= BLOCK_LEN && i % 2 == 0 {
                    for part in data.chunks(17) {
                        h.update(part);
                    }
                    hashers.push(h);
                } else {
                    hashers.push(h);
                    for part in data.chunks(17) {
                        crate::each::update_each(&mut hashers, &[(i, part)]);
                    }
                }
                expected.push(crate::test::official::hash(mode, &data));
                empty.push(crate::test::official::hash(mode, b""));
                next.push(crate::test::official::hash(mode, b"next"));
            }
            let which: Vec<usize> = (0..count).rev().collect();
            let mut out = vec![[0u8; 32]; count];
            crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
            crate::each::finalize_each(&mut hashers, &which, &mut out);
            let wanted: Vec<_> = which.iter().map(|&i| expected[i]).collect();
            assert_eq!(out, wanted);
            // Plain multi-block items (> 64 B) are routed to `hash_many_rounded` /
            // `hash_by_blocks` on AVX2/AVX-512; `hash_by_blocks` calls `output16`
            // for chunks of >= 9 (`output8` for 4..=8), and `output_slots` packs
            // remaining eligible counter-zero descriptors the same way.
            let rest_count = (0..count)
                .filter(|&i| {
                    let len = [0usize, 1, 31, 63, 64, 65, 127, 255, 511, 1023, 1024][i % 11];
                    i % 3 != 0 || len <= BLOCK_LEN
                })
                .count();
            let short_plain_count = (0..count)
                .filter(|&i| {
                    let len = [0usize, 1, 31, 63, 64, 65, 127, 255, 511, 1023, 1024][i % 11];
                    i % 3 == 0 && len > BLOCK_LEN && len < CHUNK_LEN
                })
                .count();
            let reached = crate::RAN_ENTRIES.with(|r| r.borrow().contains(&(entry() as usize)));
            assert_eq!(reached, (rest_count >= 9 || short_plain_count >= 9) && matches!(Platform::detect(), Platform::AVX512));
            assert!(hashers.iter().all(|h| h.count() == 0), "selected hashers reset");
            crate::each::finalize_each(&mut hashers, &which, &mut out);
            let wanted: Vec<_> = which.iter().map(|&i| empty[i]).collect();
            assert_eq!(out, wanted, "reset preserves every key and mode");
            for h in &mut hashers {
                h.update(b"next");
            }
            let selected: Vec<_> = (0..count).step_by(2).collect();
            let mut out = vec![[0u8; 32]; selected.len()];
            crate::each::finalize_each(&mut hashers, &selected, &mut out);
            let wanted: Vec<_> = selected.iter().map(|&i| next[i]).collect();
            assert_eq!(out, wanted);
            for (i, h) in hashers.iter().enumerate() {
                assert_eq!(h.count(), if i % 2 == 0 { 0 } else { 4 }, "only selected states reset");
            }
        }
    }

    /// A keyed message partly in its chunk state (a turn's bytes) and partly in
    /// the stage (an update's after them) is not eligible for `output16`, and a
    /// plain message follows its staged path: every digest matches official BLAKE3.
    #[test]
    fn a_turn_then_an_update_finishes_outside_the_group() {
        crate::initialize();
        let key = [0x19; 32];
        for mode in [crate::Mode::Plain, crate::Mode::Keyed(&key)] {
            for (in_turn, updated) in [(100usize, 10usize), (10, 30), (64, 64), (0, 40)] {
                let data: Vec<Vec<u8>> = (0..32).map(|i| (0..in_turn + updated).map(|j| (i * 29 + j * 7) as u8).collect()).collect();
                let mut hashers: Vec<_> = (0..32).map(|_| Hasher::new(mode, None)).collect();
                for (i, d) in data.iter().enumerate() {
                    crate::each::update_each(&mut hashers, &[(i, &d[..in_turn])]);
                    hashers[i].update(&d[in_turn..]);
                }
                let which: Vec<usize> = (0..32).collect();
                let mut out = vec![[0u8; 32]; 32];
                crate::each::finalize_each(&mut hashers, &which, &mut out);
                for (d, got) in data.iter().zip(&out) {
                    assert_eq!(*got, crate::test::official::hash(mode, d), "in_turn={in_turn} updated={updated}");
                }
            }
        }
    }

    #[test]
    fn staged_long_roots_keep_the_existing_path() {
        crate::initialize();
        let mut hashers = Vec::new();
        let mut expected = Vec::new();
        for i in 0..32 {
            let data = vec![i as u8; if i == 3 || i == 20 { 4097 } else { 129 }];
            let mut h = Hasher::new(crate::Mode::Plain, None);
            for piece in data.chunks(23) {
                h.update(piece);
            }
            expected.push(crate::test::official::hash(crate::Mode::Plain, &data));
            hashers.push(h);
        }
        let which: Vec<_> = (0..32).collect();
        let mut out = vec![[0u8; 32]; 32];
        crate::COUNTED.with(|c| c[crate::Counted::OutputGroupTurn as usize].set(0));
        crate::each::finalize_each(&mut hashers, &which, &mut out);
        assert_eq!(out, expected);
        // None of the 32 hashers enters `output_slots`: 129 B plain takes
        // `short_plain -> hash_by_blocks`, and 4097 B takes `rest`.
        assert_eq!(crate::COUNTED.with(|c| c[crate::Counted::OutputGroupTurn as usize].get()), 0);
    }

    /// Eligible `output_slots` descriptors pack independently around ineligible
    /// ones on both AVX-512 (`output16` + `output8`) and AVX2 (`output8`), and
    /// `OutputGroupTurn` counts once per `finalize_each` call even across
    /// multiple 128-slot windows.
    #[test]
    fn mixed_eligible_output_slots_pack_around_ineligible_messages() {
        crate::initialize();
        let key = [0x4Du8; 32];
        let width = lanes(Platform::detect());
        let mut hashers = Vec::new();
        let mut expected = Vec::new();
        for i in 0..17 {
            if i == 8 {
                let data = vec![0x77u8; 1025];
                let mut h = Hasher::new(crate::Mode::Plain, None);
                h.update(&data);
                hashers.push(h);
                expected.push(crate::test::official::hash(crate::Mode::Plain, &data));
            } else {
                let data = vec![i as u8; 32];
                hashers.push(Hasher::new(crate::Mode::Keyed(&key), None));
                crate::each::update_each(&mut hashers, &[(i, &data)]);
                expected.push(crate::test::official::hash(crate::Mode::Keyed(&key), &data));
            }
        }
        let which: Vec<usize> = (0..17).collect();
        let mut out = vec![[0u8; 32]; 17];
        OUTPUT_CALLS.with(|c| c.borrow_mut().clear());
        crate::COUNTED.with(|c| c[crate::Counted::OutputGroupTurn as usize].set(0));
        crate::each::finalize_each(&mut hashers, &which, &mut out);
        assert_eq!(out, expected);
        let calls = OUTPUT_CALLS.with(|c| c.borrow().clone());
        let want_calls: Vec<usize> = match width {
            16 => vec![16],
            8 => vec![8, 8],
            _ => vec![],
        };
        assert_eq!(calls, want_calls, "packed 16 eligible + 1 ineligible width={width}");
        assert_eq!(crate::COUNTED.with(|c| c[crate::Counted::OutputGroupTurn as usize].get()), u64::from(width != 0));

        // 256 eligible keyed hashers spanning two 128-slot windows still count one OutputGroupTurn.
        let mut many: Vec<_> = (0..256).map(|_| Hasher::new(crate::Mode::Keyed(&key), None)).collect();
        let pieces: Vec<_> = (0..256).map(|i| (i, b"short-keyed-msg".as_slice())).collect();
        crate::each::update_each(&mut many, &pieces);
        let which256: Vec<usize> = (0..256).collect();
        let mut out256 = vec![[0u8; 32]; 256];
        crate::COUNTED.with(|c| c[crate::Counted::OutputGroupTurn as usize].set(0));
        crate::each::finalize_each(&mut many, &which256, &mut out256);
        assert_eq!(crate::COUNTED.with(|c| c[crate::Counted::OutputGroupTurn as usize].get()), u64::from(width != 0));
    }
}
