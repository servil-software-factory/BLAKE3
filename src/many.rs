//! Many messages of one length, one digest each: [`crate::hash_many`]
//! and the pool's pieces of it.
//!
//! Each message sits in a slot of whole blocks ([`slot_len`]), zero past
//! its end, so every kernel reads whole blocks in place and records the
//! last block's true length. A message of up to a chunk is one chunk, the
//! shape the kernels hash many lanes at a time: sixteen per group on
//! SME2's 512-bit streaming vectors, several beside the integer units on
//! the NEON hybrids, up to [`TABLE`] per call so one entry into streaming
//! mode covers eight groups. Messages of 2 to 15 chunks go side by side on
//! SME2; others through the same code as [`crate::hash`], one at a time.

use crate::platform::Platform;
use crate::{BLOCK_LEN, CHUNK_END, CHUNK_LEN, CHUNK_START, CVWords, IncrementCounter, OUT_LEN, ROOT};

/// Most messages per platform call: eight SME2 groups per entry into
/// streaming mode, the same as the tree walk's `sme2::DEGREE`. Measured on
/// the 16-vCPU VM at 1024 one-block messages, ns per message: 128 → 9.7,
/// 256 → 10.2, 512 and 1024 → 12.5. The slowdown follows the run length
/// handed to the platform, with the kernel call's own group count, the
/// frame size, the buffers' alignment, and the loops below each ruled out
/// by measurement; what remains is scanning far ahead of the kernel. On an
/// M4 the entry into streaming mode costs about a microsecond, so a longer
/// run may pay there; measure before changing this.
pub(crate) const TABLE: usize = 128;

/// The bytes each message of `len` bytes takes in a batch: `len` rounded up
/// to whole blocks, one block for an empty message.
///
/// Panics if `len` cannot be rounded up to a multiple of [`BLOCK_LEN`] without
/// overflowing `usize`.
#[inline(always)]
pub fn slot_len(len: usize) -> usize {
    len.checked_next_multiple_of(BLOCK_LEN).expect("batch slot length overflow").max(BLOCK_LEN)
}

/// hash_many's contract on its input: `count` slots of `slot` bytes for
/// messages of `len` bytes (checked in every build: the kernels read
/// them), each slot's bytes past its message zero (checked in debug
/// builds: a Batch keeps them so). A slot is whole blocks ([`slot_len`]),
/// or the message itself, packed (`packed`: 16 or 32 bytes).
#[inline]
pub fn assert_slots(input: &[u8], len: usize, slot: usize, count: usize) {
    // In every build, both: the kernels take a slot's size from `len` and
    // read `slot * count` bytes from `input`. A slot of another size, or
    // an input of another length, would be read past its end, or leave
    // the kernels' table of slots unwritten (BLAKE3 #419: a 2-byte input
    // called a 64-byte message crashed 0.9.0). AGENTS: a check of a
    // mistake that would mismanage memory, at the cost of a comparison.
    assert!(slot == slot_len(len) || packed(len, slot), "a slot is whole blocks or a packed message");
    assert_eq!(Some(input.len()), slot.checked_mul(count), "input holds one slot per output");
    // Every caller's slots come from a Batch (throughput's calls, the
    // stream's batches) or from the crate's own buffers, which keep them
    // zero past each message (BLAKE3 #205): checked in debug builds.
    debug_assert!(
        len % BLOCK_LEN == 0 || slot == len || input.chunks_exact(slot).all(|s| s[len..].iter().all(|&b| b == 0)),
        "every slot's bytes past its message are zero"
    );
}

/// Whether messages of `len` bytes at a stride of `slot` are packed: the
/// stride is the message, one of the three supported packed widths (16, 32,
/// or 48 bytes), so the block's remaining bytes are not in memory (`Packed`
/// or `&[[u8; N]]`).
#[inline]
pub(crate) fn packed(len: usize, slot: usize) -> bool {
    matches!(len, 16 | 32 | 48) && slot == len
}

/// `outputs[i] = hash(input[i * slot..][..len])` for every message, with
/// `slot = slot_len(len)`, in the mode of `key` and `flags` (IV and 0 for
/// plain hashing), on `platform`: messages of up to a chunk go to
/// the kernels TABLE at a time, 2 to 15 chunks side by side on SME2, others
/// through the one-message path. Requires `input.len() == slot *
/// outputs.len()` and every slot's bytes past `len` zero (a Batch's, or
/// the crate's own).
pub fn hash_many_on(input: &[u8], len: usize, slot: usize, key: &CVWords, flags: u8, outputs: &mut [[u8; OUT_LEN]], platform: Platform) {
    // One-block messages first, on the path they took before slots (a
    // shared entry cost 64 B x 3 7% in the regression check).
    if len == BLOCK_LEN && outputs.len() >= 2 {
        crate::ran(crate::Ran::BlockRun);
        assert_slots(input, len, slot, outputs.len());
        for (messages, digests) in input.chunks(BLOCK_LEN * TABLE).zip(outputs.chunks_mut(TABLE)) {
            hash_run(messages, key, flags, digests, platform);
        }
        return;
    }
    assert_slots(input, len, slot, outputs.len());
    if packed(len, slot) {
        return hash_packed(input, len, key, flags, outputs, platform);
    }
    // Empty messages: the NEON plans take a last block of 1 to 64 bytes
    // (hash_many_in's contract), so without this arm a batch of them stops
    // there (#251: CI's Cortex-A72, and every NEON machine since the pair
    // kernels left).
    if len == 0 {
        outputs.fill(*crate::hash_serial_on(&[], key, flags, platform).as_bytes());
        return;
    }
    #[cfg(blake3_sme2)]
    if chunked(len) && platform_is_sme2(platform) && (outputs.len() >= 16 || outputs.len() >= sme2_chunked_min(len)) {
        return hash_chunked(input, len, key, flags, outputs, platform);
    }
    #[cfg(blake3_neon)]
    if (CHUNK_LEN + 1..=2 * CHUNK_LEN).contains(&len) && outputs.len() >= 2 {
        return hash_two_chunks(input, len, key, flags, outputs);
    }
    if outputs.len() < 2 || len > CHUNK_LEN {
        #[cfg(target_arch = "x86_64")]
        return hash_long_or_single(input, len, key, flags, outputs, platform);
        #[cfg(not(target_arch = "x86_64"))]
        {
            for (i, output) in outputs.iter_mut().enumerate() {
                *output = *crate::hash_serial_on(&input[i * slot..][..len], key, flags, platform).as_bytes();
            }
            return;
        }
    }
    crate::ran(crate::Ran::ShortRun);
    let last_len = len - (slot - BLOCK_LEN);
    #[cfg(all(blake3_avx512_ffi, target_os = "linux"))]
    if slot > BLOCK_LEN && last_len < BLOCK_LEN && outputs.len() >= X86_PAD8_MIN {
        let width = crate::ffi_output::lanes(platform);
        if width != 0 {
            multi_block_short_last(input, slot, key, flags, last_len, outputs, platform, width);
            return;
        }
    }
    let blocks = slot / BLOCK_LEN;
    for (messages, digests) in input.chunks(slot * TABLE).zip(outputs.chunks_mut(TABLE)) {
        if blocks == 1 {
            hash_run_short(messages, key, flags, digests, platform, last_len);
            continue;
        }
        #[cfg(blake3_sse41_ffi)]
        crate::sse41::hash_blocks_x86(messages, blocks, key, flags, digests, platform, last_len);
        #[cfg(blake3_neon)]
        match blocks {
            2 => hash_blocks::<{ 2 * BLOCK_LEN }>(messages, key, flags, digests, platform, last_len),
            3 => hash_blocks::<{ 3 * BLOCK_LEN }>(messages, key, flags, digests, platform, last_len),
            4 => hash_blocks::<{ 4 * BLOCK_LEN }>(messages, key, flags, digests, platform, last_len),
            5 => hash_blocks::<{ 5 * BLOCK_LEN }>(messages, key, flags, digests, platform, last_len),
            6 => hash_blocks::<{ 6 * BLOCK_LEN }>(messages, key, flags, digests, platform, last_len),
            7 => hash_blocks::<{ 7 * BLOCK_LEN }>(messages, key, flags, digests, platform, last_len),
            8 => hash_blocks::<{ 8 * BLOCK_LEN }>(messages, key, flags, digests, platform, last_len),
            9 => hash_blocks::<{ 9 * BLOCK_LEN }>(messages, key, flags, digests, platform, last_len),
            10 => hash_blocks::<{ 10 * BLOCK_LEN }>(messages, key, flags, digests, platform, last_len),
            11 => hash_blocks::<{ 11 * BLOCK_LEN }>(messages, key, flags, digests, platform, last_len),
            12 => hash_blocks::<{ 12 * BLOCK_LEN }>(messages, key, flags, digests, platform, last_len),
            13 => hash_blocks::<{ 13 * BLOCK_LEN }>(messages, key, flags, digests, platform, last_len),
            14 => hash_blocks::<{ 14 * BLOCK_LEN }>(messages, key, flags, digests, platform, last_len),
            15 => hash_blocks::<{ 15 * BLOCK_LEN }>(messages, key, flags, digests, platform, last_len),
            16 => hash_blocks::<{ 16 * BLOCK_LEN }>(messages, key, flags, digests, platform, last_len),
            _ => unreachable!("messages of up to a chunk"),
        }
    }
}

/// Packed messages (`len` bytes each, at a stride of `len`): full SIMD
/// groups run directly on the zero-padding-free packed kernels in
/// `ffi_packed` (#501) across SME2, AVX-512, AVX2, and NEON `8+3`; any
/// short remainder (or SSE4.1 fallback) expands up to 16 messages at a
/// time into a 1 KiB zero-padded stack buffer.
#[inline(never)]
fn hash_packed(input: &[u8], len: usize, key: &CVWords, flags: u8, outputs: &mut [[u8; OUT_LEN]], platform: Platform) {
    // Packed's three lengths, in every build: a block-length message sent
    // here would hash right and slow (a mutant of `packed` did).
    assert!(matches!(len, 16 | 32 | 48), "packed messages are 16, 32, or 48 bytes, not {len}");
    let done = crate::ffi_packed::hash_packed_groups(input, len, key, flags, outputs, platform);
    if done == outputs.len() {
        return;
    }
    let mut blocks = [crate::block::Block::ZERO; 16];
    for (messages, digests) in input[done * len..].chunks(len * 16).zip(outputs[done..].chunks_mut(16)) {
        let bytes = crate::block::Block::bytes_mut(&mut blocks);
        for (i, message) in messages.chunks_exact(len).enumerate() {
            bytes[i * BLOCK_LEN..][..len].copy_from_slice(message);
        }
        hash_run_short(&bytes[..digests.len() * BLOCK_LEN], key, flags, digests, platform, len);
    }
}

/// Minimum remaining one-block or short-last-block messages to pad a 16-lane
/// AVX-512 `output16` call instead of `output8` + scalar.
#[cfg(all(blake3_avx512_ffi, target_os = "linux"))]
const X86_PAD16_MIN: usize = 9;
/// Minimum remaining one-block or short-last-block messages to pad an 8-lane
/// AVX2 `output8` call instead of running serial `hash_serial_on` (4 wins on
/// every tested x86 CPU including Xeon 6330, where 3 read +12% slower).
#[cfg(all(blake3_avx512_ffi, target_os = "linux"))]
const X86_PAD8_MIN: usize = 4;

/// The whole and padded groups of `digests.len()` one-block messages (each
/// `len` bytes of a 64-byte slot in `bytes`, zero past them: a batch's
/// padding, or the remainder buffer `hash_packed` writes) that run on the
/// x86-64 independent-compression kernels side by side: how many messages
/// they took.
#[cfg(all(blake3_avx512_ffi, target_os = "linux"))]
#[inline(never)]
fn one_block_groups(bytes: &[u8], digests: &mut [[u8; OUT_LEN]], len: usize, key: &CVWords, flags: u8, platform: Platform) -> usize {
    // Check the descriptor length once, before narrowing it or filling
    // any lanes. Both short batches and expanded Packed remainders come here.
    assert!(len <= BLOCK_LEN, "an independent short message fits in one block");
    let len = len.to_le_bytes()[0];
    let lanes = crate::ffi_output::lanes(platform);
    if lanes == 0 {
        return 0;
    }
    let groups = digests.len() / lanes;
    for g in 0..groups {
        let (at, out) = (g * lanes, &mut digests[g * lanes..][..lanes]);
        let group = &bytes[at * BLOCK_LEN..];
        // Sound: lanes() names the platform's kernel width, which the CPU
        // runs (Platform::detect chose it).
        unsafe {
            if lanes == 16 {
                packed_group::<16>(group, out, len, key, flags);
            } else {
                packed_group::<8>(group, out, len, key, flags);
            }
        }
    }
    let mut done = groups * lanes;
    let rem = digests.len() - done;
    let tail = &bytes[done * BLOCK_LEN..];
    if lanes == 16 && rem >= X86_PAD16_MIN {
        // Sound: Platform::AVX512 supports output16; every lane reads a valid
        // 64-byte slot in `tail`, repeating the last message for spare lanes.
        unsafe { packed_tail::<16>(tail, &mut digests[done..], len, key, flags) };
        done += rem;
    } else if rem == 8 {
        // Sound: both Platform::AVX512 and Platform::AVX2 support AVX2 output8.
        unsafe { packed_group::<8>(tail, &mut digests[done..done + 8], len, key, flags) };
        done += 8;
    } else if rem >= X86_PAD8_MIN {
        // Sound: both Platform::AVX512 and Platform::AVX2 support AVX2 output8.
        unsafe { packed_tail::<8>(tail, &mut digests[done..], len, key, flags) };
        done += rem;
    }
    done
}

/// `W` one-block messages, each `len` bytes zero-padded to a block at the
/// start of `blocks`, on the `W`-lane independent-compression kernel.
/// Requires the CPU to run that kernel (`ffi_output::lanes`).
#[cfg(all(blake3_avx512_ffi, target_os = "linux"))]
unsafe fn packed_group<const W: usize>(blocks: &[u8], out: &mut [[u8; OUT_LEN]], len: u8, key: &CVWords, flags: u8) {
    let block = |i: usize| -> &[u8; BLOCK_LEN] { blocks[i * BLOCK_LEN..][..BLOCK_LEN].try_into().unwrap() };
    let inputs: [crate::ffi_output::Input<'_>; W] = core::array::from_fn(|i| crate::ffi_output::packed(block(i), key, len, flags));
    // Sound: the caller's CPU runs the W-lane kernel; every block is 64
    // readable bytes.
    unsafe { crate::ffi_output::hash_group::<W>(&inputs, out) };
}

/// `1..W` one-block messages padded to `W` lanes by repeating the last block
/// into spare lanes, writing into a stack scratch buffer and copying back
/// `out.len()` digests.
#[cfg(all(blake3_avx512_ffi, target_os = "linux"))]
unsafe fn packed_tail<const W: usize>(blocks: &[u8], out: &mut [[u8; OUT_LEN]], len: u8, key: &CVWords, flags: u8) {
    let count = out.len();
    assert!(count > 0 && count < W, "padded tail holds 1..W messages");
    let block = |i: usize| -> &[u8; BLOCK_LEN] {
        let idx = i.min(count - 1);
        blocks[idx * BLOCK_LEN..][..BLOCK_LEN].try_into().unwrap()
    };
    let inputs: [crate::ffi_output::Input<'_>; W] = core::array::from_fn(|i| crate::ffi_output::packed(block(i), key, len, flags));
    let mut scratch = [[0u8; OUT_LEN]; W];
    // Sound: the caller's CPU runs the W-lane kernel; every lane reads a valid
    // 64-byte slot and `scratch` holds all W outputs.
    unsafe { crate::ffi_output::hash_group::<W>(&inputs, &mut scratch) };
    out.copy_from_slice(&scratch[..count]);
}

/// Padded lane count for a single `<= TILE` tile of multi-block prefix
/// `hash_many_blocks`: round `9..15` lanes up to 16 on AVX-512 (`width == 16`)
/// and `5..7` lanes up to 8 on both AVX-512 and AVX2 so prefix hashing avoids
/// scalar tails.
#[cfg(all(blake3_avx512_ffi, target_os = "linux"))]
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
/// keeps the stack workspace small (`512 B` for `TILE = 8`, `1 KiB` for
/// `TILE = 16`) so small batches never evict the caller's L1d working set.
#[cfg(all(blake3_avx512_ffi, target_os = "linux"))]
#[inline(never)]
fn multi_block_short_last_tile<const TILE: usize>(
    messages: &[u8],
    slot: usize,
    key: &CVWords,
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
    let inputs: [crate::ffi_output::Input<'_>; TILE] = core::array::from_fn(|j| {
        let idx = j.min(count - 1);
        crate::ffi_output::final_block(last_block(idx), &cvs[idx], last_len_u8, flags)
    });
    if count == TILE {
        // Sound: `TILE` matches AVX-512 (16) or AVX2/AVX-512 (8); all inputs are valid.
        unsafe { crate::ffi_output::hash_group::<TILE>(&inputs, outputs) };
    } else {
        let mut scratch = [[0u8; OUT_LEN]; TILE];
        // Sound: `TILE` matches AVX-512 (16) or AVX2/AVX-512 (8); all inputs are valid.
        unsafe { crate::ffi_output::hash_group::<TILE>(&inputs, &mut scratch) };
        outputs.copy_from_slice(&scratch[..count]);
    }
}

/// Multi-block messages (`slot = 2..=16` blocks) whose final block is short
/// (`1..64` bytes, zero-padded in each slot) on x86-64 AVX-512 / AVX2: tile
/// by `width` (`16` on AVX-512, `8` on AVX2), dispatching `4..=8` through the
/// `512 B` 8-lane tile and `9..=16` through the `1 KiB` 16-lane tile, and
/// running trailing `1..3` messages serially.
#[cfg(all(blake3_avx512_ffi, target_os = "linux"))]
#[inline(never)]
fn multi_block_short_last(
    input: &[u8],
    slot: usize,
    key: &CVWords,
    flags: u8,
    last_len: usize,
    outputs: &mut [[u8; OUT_LEN]],
    platform: Platform,
    width: usize,
) {
    assert!(last_len > 0 && last_len < BLOCK_LEN, "short final block");
    let prefix_len = slot - BLOCK_LEN;
    let last_len_u8 = last_len.to_le_bytes()[0];
    for (messages, digests) in input.chunks(slot * width).zip(outputs.chunks_mut(width)) {
        let count = digests.len();
        if count < X86_PAD8_MIN {
            for (i, output) in digests.iter_mut().enumerate() {
                *output = *crate::hash_serial_on(&messages[i * slot..][..prefix_len + last_len], key, flags, platform).as_bytes();
            }
        } else if count <= 8 {
            multi_block_short_last_tile::<8>(messages, slot, key, flags, last_len_u8, digests, platform);
        } else {
            multi_block_short_last_tile::<16>(messages, slot, key, flags, last_len_u8, digests, platform);
        }
    }
}

/// Fewest messages of 2 to 16 blocks that go to SME2 as one more group of
/// sixteen, its spare lanes reading the last message again: 10 in a batch
/// of fewer than sixteen, 5 left over after the SME2 groups (where NEON
/// work beside the SME2 kernels pays the SME unit's slow state as well).
/// Fewer run on the integer + NEON parent plans. ns per message, the plans
/// against the extra group: VM 256 B, 9 messages 60 / 69, 10 68 / 62, 12
/// 65 / 52; 128 B and 1 KiB cross at the same counts. Mac P-core (jobs
/// 261-264), 21 x 256 B 67.5 / 58.3, 53 x 256 B 51.1 / 45.5, 37 x 1 KiB
/// 201 / 193; the VM level at 21. With the program's work between calls,
/// 5 after the groups is best or level against 3 and 1 at nearly every
/// count (Mac, 64 B to 1 KiB, 16-129 messages: 1 KiB x 19 345 ns/msg
/// against 430 and 431); 3 and 1 win only back to back.
pub(crate) const SME2_TAIL_MIN: usize = 10;
#[cfg(blake3_neon)]
pub(crate) const SME2_TAIL_MIN_AFTER_GROUPS: usize = 5;

/// Fewest messages of `len` bytes (whole blocks, 2 to 15 chunks) that SME2
/// hashes side by side as one group of sixteen, padded, by chunk count.
/// Fewer, alone or left over past whole groups, run through hash() one at
/// a time. The count at which a padded group's time equals that many
/// hash() calls, VM: 2 chunks 5.9, 3 8.4, 4 8.1, 6 9.5, 8 10.4, 12 11.4, 15
/// 11.6; Mac P-core (jobs 268-271) about the same or lower. Each entry is
/// the next count up.
const SME2_CHUNKED_MIN: [usize; 16] = [0, 0, 7, 9, 9, 10, 10, 11, 11, 12, 12, 12, 12, 12, 12, 12];

pub(crate) fn sme2_chunked_min(len: usize) -> usize {
    SME2_CHUNKED_MIN[len.div_ceil(CHUNK_LEN)]
}

/// Whether hash_many_on takes messages of `len` bytes side by side on
/// SME2: 2 to 15 chunks.
#[inline(always)]
fn chunked(len: usize) -> bool {
    cfg!(blake3_sme2) && (CHUNK_LEN + 1..=15 * CHUNK_LEN).contains(&len)
}

/// Messages of 2 to 15 chunks on SME2, sixteen side by side per group;
/// messages left over past whole groups make one more group, its spare
/// lanes repeating the last message, when there are sme2_chunked_min of
/// them, and otherwise run through hash() one at a time, first.
#[cfg(blake3_sme2)]
#[inline(never)]
fn hash_chunked(input: &[u8], len: usize, key: &CVWords, flags: u8, outputs: &mut [[u8; OUT_LEN]], platform: Platform) {
    const GROUP: usize = 16;
    let slot = slot_len(len);
    let left = outputs.len() % GROUP;
    // Past whole groups a group pays from two messages sooner: hash() calls
    // beside the SME2 kernels run in their slow state (VM, 22 x 2 KiB: 630
    // ns per message with six alone, 486 with a padded group).
    let min = if outputs.len() > GROUP { sme2_chunked_min(len) - 2 } else { sme2_chunked_min(len) };
    let alone = if left >= min { 0 } else { left };
    let grouped = outputs.len() - alone;
    let (groups, rest) = outputs.split_at_mut(grouped);
    for (i, output) in rest.iter_mut().enumerate() {
        *output = *crate::hash_serial_on(&input[(grouped + i) * slot..][..len], key, flags, platform).as_bytes();
    }
    for (g, digests) in groups.chunks_mut(GROUP).enumerate() {
        let mut lanes = [core::ptr::null::<u8>(); GROUP];
        for (i, lane) in lanes.iter_mut().enumerate() {
            let message = g * GROUP + i.min(digests.len() - 1);
            *lane = input[message * slot..][..slot].as_ptr();
        }
        // Sound: the caller found SME2 (platform_is_sme2), and every lane
        // points at a whole slot inside `input`, zero past its message.
        unsafe { crate::sme2::hash_chunked_messages(&lanes, len, key, flags, digests.as_flattened_mut()) };
    }
}

/// Messages of two chunks (1025 to 2048 bytes) side by side on the integer
/// + NEON parent plans, sixteen at a time: every message's first chunk in
/// one plan call at counter 0, every second chunk in another at counter 1,
/// then every root. hash() gives each message a NEON pair with a spare
/// lane (q1) or a pair alone (k2); side by side the plans fill their
/// lanes with the batch. VM, ns per message, one hash() each / side by
/// side: 2000 B x 2 1018 / 927, x 3 1036 / 653, x 4 1050 / 697, x 8 690
/// (SME2 from 7); 1100 B x 3 442 / 379. Commonware's four NEON lanes: 2000
/// B x 4 824 (Mac P-core 914 against servil's 1050 before, job 283).
#[cfg(blake3_neon)]
#[inline(never)]
fn hash_two_chunks(input: &[u8], len: usize, key: &CVWords, flags: u8, outputs: &mut [[u8; OUT_LEN]]) {
    use crate::PARENT;
    let slot = slot_len(len);
    let second = len - CHUNK_LEN;
    let second_blocks = second.div_ceil(BLOCK_LEN);
    let last_len = second - (second_blocks - 1) * BLOCK_LEN;
    // Sixteen at a time, as the plans take them: small arrays to zero.
    const GROUP: usize = 16;
    for (messages, digests) in input.chunks(slot * GROUP).zip(outputs.chunks_mut(GROUP)) {
        let count = digests.len();
        let mut firsts = crate::stack_vec::StackVec::<&[u8; CHUNK_LEN], GROUP>::new();
        let mut seconds = crate::stack_vec::StackVec::<*const u8, GROUP>::new();
        firsts.extend(messages.chunks_exact(slot).map(|m| m[..CHUNK_LEN].try_into().expect("a whole first chunk")));
        seconds.extend(messages.chunks_exact(slot).map(|m| m[CHUNK_LEN..].as_ptr()));
        assert_eq!(firsts.len(), count, "one two-chunk slot per output");
        assert_eq!(seconds.len(), count, "one second chunk per output");
        let mut cvs = [[0u8; OUT_LEN]; 2 * GROUP];
        let mut pairs = [[0u8; BLOCK_LEN]; GROUP];
        // Sound: `firsts` and `seconds` hold exactly `count` initialized
        // entries; each second-chunk pointer reaches `second_blocks` whole
        // blocks inside its slot, zero past the message; NEON is mandatory on
        // every AArch64 CPU.
        unsafe {
            crate::neon_hybrid::hash_many_last_len(
                &firsts,
                key,
                0,
                IncrementCounter::No,
                flags,
                CHUNK_START,
                CHUNK_END,
                BLOCK_LEN,
                cvs[..count].as_flattened_mut(),
            );
            crate::neon_hybrid::hash_messages_raw(
                seconds.as_ptr(),
                seconds.len(),
                second_blocks,
                key,
                1,
                flags,
                CHUNK_START,
                CHUNK_END,
                last_len,
                cvs[count..2 * count].as_flattened_mut(),
            );
        }
        for (i, pair) in pairs[..count].iter_mut().enumerate() {
            pair[..OUT_LEN].copy_from_slice(&cvs[i]);
            pair[OUT_LEN..].copy_from_slice(&cvs[count + i]);
        }
        let parents: crate::stack_vec::StackVec<&[u8; BLOCK_LEN], GROUP> = pairs[..count].iter().collect();
        // Sound: as above.
        unsafe {
            crate::neon_hybrid::hash_many_last_len(&parents, key, 0, IncrementCounter::No, flags | PARENT, 0, ROOT, BLOCK_LEN, digests.as_flattened_mut())
        };
    }
}

/// Keep long-message tree work outside the short-message dispatcher's frame.
#[cfg(target_arch = "x86_64")]
#[inline(never)]
fn hash_long_or_single(input: &[u8], len: usize, key: &CVWords, flags: u8, outputs: &mut [[u8; OUT_LEN]], platform: Platform) {
    hash_long_or_single_for(input, len, key, flags, outputs, platform, crate::platform::wide_avx512());
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn hash_long_or_single_for(input: &[u8], len: usize, key: &CVWords, flags: u8, outputs: &mut [[u8; OUT_LEN]], platform: Platform, wide_avx512: bool) {
    let done = if let Some(least) = across_from(len, platform)
        && outputs.len() >= least
    {
        hash_across_tiles(input, len, key, flags, outputs, platform, least, wide_avx512)
    } else {
        0
    };
    if done < outputs.len() {
        crate::ran(crate::Ran::MessageAlone);
        let slot = slot_len(len);
        for (i, output) in outputs[done..].iter_mut().enumerate() {
            *output = *crate::hash_serial_on(&input[(done + i) * slot..][..len], key, flags, platform).as_bytes();
        }
    }
}

/// Out of line so `StackVec` and `hash_group_across`'s 8 KiB CV table stay out
/// of `hash_long_or_single`'s frame on the one-message path.
#[cfg(target_arch = "x86_64")]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn hash_across_tiles(
    mut input: &[u8],
    len: usize,
    key: &CVWords,
    flags: u8,
    mut outputs: &mut [[u8; OUT_LEN]],
    platform: Platform,
    least: usize,
    wide_avx512: bool,
) -> usize {
    let total = outputs.len();
    let slot = slot_len(len);
    let lanes = across_lanes(platform);
    let chunks = len.div_ceil(CHUNK_LEN);
    while outputs.len() >= least {
        crate::ran(crate::Ran::Across);
        let take = across_take(outputs.len(), chunks, lanes, least, wide_avx512);
        let (head, rest) = outputs.split_at_mut(take);
        let (in_head, in_rest) = input.split_at(take * slot);
        let messages: crate::stack_vec::StackVec<&[u8], ACROSS_GROUP> = (0..take).map(|m| &in_head[m * slot..][..len]).collect();
        hash_group_across(&messages, len, key, flags, head, platform);
        input = in_rest;
        outputs = rest;
    }
    total - outputs.len()
}

/// The across-group lane width for `platform`: 8 on AVX2, 16 (`ACROSS_GROUP`)
/// on AVX-512 (never AVX-512's 32-lane single-tree degree).
#[cfg(target_arch = "x86_64")]
#[inline]
fn across_lanes(platform: Platform) -> usize {
    platform.simd_degree().min(ACROSS_GROUP)
}

/// The number of messages to take into one [`hash_group_across`] tile out of
/// `rem` remaining messages of `chunks` chunks on an across width of `lanes`
/// (8 on AVX2, 16 on AVX-512) with admission floor `least`:
/// - `ACROSS_GROUP` (16) whenever `rem >= ACROSS_GROUP`;
/// - `lanes` (8 on AVX2) when `lanes <= rem < ACROSS_GROUP` and the leftover
///   `rem - lanes < least` should run serially rather than in an underfilled
///   sub-group (when `rem - lanes >= least`, keeping all `rem` in one call
///   avoids re-initializing the 8 KiB CV table);
/// - on non-wide AVX-512 (`!wide_avx512`), 8 for `rem` in `9..=10` when
///   `chunks >= 4`, so 1–2 trailing messages run serially instead of padding a
///   16-lane pass (at 2–3 chunks, 9–10 messages padded to 16 beat 8 + serial);
/// - otherwise all `rem` messages together.
#[cfg(target_arch = "x86_64")]
#[inline]
fn across_take(rem: usize, chunks: usize, lanes: usize, least: usize, wide_avx512: bool) -> usize {
    if rem >= ACROSS_GROUP {
        ACROSS_GROUP
    } else if rem >= lanes && rem - lanes < least {
        lanes
    } else if chunks >= 4 && !wide_avx512 && matches!(rem, 9 | 10) {
        ACROSS_GROUP / 2
    } else {
        rem
    }
}

/// The chunks a message has below which [`hash_long_or_single`] hashes across
/// messages: from 16, a message's own chunks fill the widest lanes alone (as
/// each.rs's ALONE_CHUNKS).
#[cfg(target_arch = "x86_64")]
const ACROSS_CHUNKS: usize = 16;

/// The fewest messages of `len` bytes that one [`hash_group_across`] tile
/// beats serial hashing on `platform` (AVX2 and AVX-512, 2 to ACROSS_CHUNKS - 1
/// chunks):
/// - 2 for 2-chunk messages (1025..=2048 B) and partial 3-chunk messages
///   (2049..=3071 B), where even 2 messages fill more work across than one
///   message's short chain alone;
/// - 4 for longer partial-chunk messages with `chunks < lanes` (4..=7 chunks
///   on AVX2, 4..=15 on AVX-512);
/// - `lanes / 2` (4 on AVX2 at 3 KiB, 8 on AVX-512 at 3..=7 KiB) for whole-
///   chunk messages under half the across lanes;
/// - on AVX-512 (`lanes == ACROSS_GROUP`), for 8..=15 whole KiB: 14 at 8 KiB
///   (8 chunks fill an exact 8-lane AVX2 pass alone) and `chunks` at 9..=15
///   KiB (which already pad to 16 alone, so `chunks` messages across match or
///   beat serial);
/// - on AVX2 (`lanes == 8`), for 4..=15 whole KiB: 7 at 9..=14 KiB (where
///   serial needs two AVX2 passes so 7/8 padded across wins) and 8 otherwise.
#[cfg(target_arch = "x86_64")]
pub(crate) fn across_from(len: usize, platform: Platform) -> Option<usize> {
    let chunks = len.div_ceil(CHUNK_LEN);
    if !(2..ACROSS_CHUNKS).contains(&chunks) || !crate::pair::takes_platform(platform) {
        return None;
    }
    let lanes = across_lanes(platform);
    let half = lanes / 2;
    let partial = len % CHUNK_LEN != 0;
    Some(if chunks == 2 || (chunks == 3 && partial) {
        2
    } else if chunks < lanes && partial {
        4
    } else if chunks < half {
        half
    } else if lanes == ACROSS_GROUP {
        if chunks == half { 14 } else { chunks }
    } else if (9..=14).contains(&chunks) {
        7
    } else {
        lanes
    })
}

/// The most messages [`hash_group_across`] takes at once: the widest lanes.
#[cfg(target_arch = "x86_64")]
pub(crate) const ACROSS_GROUP: usize = 16;

/// Messages of one length, over a chunk and under ACROSS_CHUNKS chunks,
/// hashed across up to ACROSS_GROUP messages at every step of their tree (all
/// share one shape): each whole chunk index on the platform's hash_many;
/// the partial chunk's blocks but its last likewise, its last block for
/// each message alone (its chaining value so far is that message's own);
/// then each parent node, from the leaves up, across the messages, ROOT on
/// the top one. Every value on the stack: the batch allocates nothing.
#[cfg(target_arch = "x86_64")]
pub(crate) fn hash_group_across(messages: &[&[u8]], len: usize, key: &CVWords, flags: u8, digests: &mut [[u8; OUT_LEN]], platform: Platform) {
    const GROUP: usize = ACROSS_GROUP;
    let (full, chunks) = (len / CHUNK_LEN, len.div_ceil(CHUNK_LEN));
    assert!(full >= 1 && chunks >= 2 && chunks < ACROSS_CHUNKS, "messages of 2 chunks to under ACROSS_CHUNKS");
    assert!(
        messages.len() <= GROUP && messages.len() == digests.len() && messages.iter().all(|m| m.len() == len),
        "up to ACROSS_GROUP messages of len bytes, a digest each"
    );
    {
        let count = digests.len();
        let message = |m: usize| messages[m];
        // cvs[chunk][message]
        let mut cvs = [[[0u8; OUT_LEN]; GROUP]; ACROSS_CHUNKS];
        for k in 0..full {
            let refs: crate::stack_vec::StackVec<&[u8; CHUNK_LEN], GROUP> =
                (0..count).map(|m| message(m)[k * CHUNK_LEN..][..CHUNK_LEN].try_into().unwrap()).collect();
            crate::hash_many_rounded(platform, &refs, key, k as u64, IncrementCounter::No, flags, CHUNK_START, CHUNK_END, cvs[k][..count].as_flattened_mut());
        }
        if full < chunks {
            let partial = len - full * CHUNK_LEN;
            let before_last = (partial - 1) / BLOCK_LEN;
            let mut so_far = [[0u8; OUT_LEN]; GROUP];
            if before_last > 0 {
                // Sound: each pointer reaches before_last whole blocks of its message.
                let ptrs: crate::stack_vec::StackVec<*const u8, GROUP> = (0..count).map(|m| message(m)[full * CHUNK_LEN..].as_ptr()).collect();
                unsafe {
                    crate::pair::hash_many_blocks(
                        platform,
                        &ptrs,
                        before_last,
                        key,
                        full as u64,
                        false,
                        flags,
                        CHUNK_START,
                        0,
                        so_far[..count].as_flattened_mut(),
                    )
                };
            }
            for m in 0..count {
                let mut cv = if before_last > 0 { crate::platform::words_from_le_bytes_32(&so_far[m]) } else { *key };
                let last = &message(m)[full * CHUNK_LEN + before_last * BLOCK_LEN..];
                let mut block = [0u8; BLOCK_LEN];
                block[..last.len()].copy_from_slice(last);
                let start = if before_last == 0 { CHUNK_START } else { 0 };
                platform.compress_in_place(&mut cv, &block, last.len().to_le_bytes()[0], full as u64, flags | start | CHUNK_END);
                cvs[full][m] = crate::hasher::le_bytes(&cv);
            }
        }
        subtree_across(&mut cvs, 0, chunks, count, key, flags, platform, true);
        digests.copy_from_slice(&cvs[0][..count]);
    }
}

/// The chaining values (at the top, the root hashes) of the subtree of
/// chunks `first..first + n` of every message, into `cvs[first]`, from the
/// chunk values in `cvs` (per chunk, then message), each parent across the
/// messages.
#[cfg(target_arch = "x86_64")]
#[allow(clippy::too_many_arguments)]
fn subtree_across(
    cvs: &mut [[[u8; OUT_LEN]; 16]; ACROSS_CHUNKS],
    first: usize,
    n: usize,
    count: usize,
    key: &CVWords,
    flags: u8,
    platform: Platform,
    top: bool,
) {
    if n == 1 {
        return;
    }
    let split = crate::tree::left_len(n * CHUNK_LEN) / CHUNK_LEN;
    subtree_across(cvs, first, split, count, key, flags, platform, false);
    subtree_across(cvs, first + split, n - split, count, key, flags, platform, false);
    let mut blocks = [[0u8; BLOCK_LEN]; 16];
    for m in 0..count {
        blocks[m][..OUT_LEN].copy_from_slice(&cvs[first][m]);
        blocks[m][OUT_LEN..].copy_from_slice(&cvs[first + split][m]);
    }
    let lanes: crate::stack_vec::StackVec<&[u8; BLOCK_LEN], 16> = blocks[..count].iter().collect();
    let parent_flags = flags | crate::PARENT | if top { ROOT } else { 0 };
    platform.hash_many(&lanes, key, 0, IncrementCounter::No, parent_flags, 0, 0, cvs[first][..count].as_flattened_mut());
}

/// Whether a batch of `count` messages of `len` bytes runs SME2 kernels
/// (so takes the SME2 turn): sixteen messages or more; one-block messages
/// from ONE_BLOCK_PAD_MIN; messages of up to a chunk from SME2_TAIL_MIN;
/// messages of 2 to 15 chunks from sme2_chunked_min; and any message long
/// enough for hash() to take the turn itself.
pub(crate) fn sme2_sized(len: usize, count: usize) -> bool {
    count >= crate::SME2_SIZED_BATCH
        || (len > 0 && len <= BLOCK_LEN && count >= ONE_BLOCK_PAD_MIN)
        || (len > BLOCK_LEN && len <= CHUNK_LEN && count >= SME2_TAIL_MIN)
        || (chunked(len) && count >= sme2_chunked_min(len))
        || (len >= crate::SME2_SIZED_LEN && count >= 1)
}

/// Up to TABLE one-block messages (N = BLOCK_LEN, whole blocks) in
/// `messages`: whole SME2 groups on the message kernel, the last padded,
/// where the count calls for it; otherwise on SME2 as the run of blocks
/// they are (its parent kernel, `sme2::hash_block_run`), and elsewhere
/// the platform's `hash_many` (the parent kernels and the NEON plans).
/// Kept out of line:
/// with all sixteen lengths inlined into hash_many_on, batches of two
/// 64-byte messages took 30% longer (VM, bench-hashes); out of line they
/// cost what they did before.
#[inline(never)]
fn hash_run(messages: &[u8], key: &CVWords, flags: u8, outputs: &mut [[u8; OUT_LEN]], platform: Platform) {
    #[cfg(blake3_sme2)]
    if platform_is_sme2(platform) && outputs.len() % 16 >= if outputs.len() > 16 { ONE_BLOCK_PAD_AFTER_GROUPS } else { ONE_BLOCK_PAD_MIN } {
        return hash_run_padded(messages, key, flags, outputs, BLOCK_LEN);
    }
    #[cfg(blake3_sme2)]
    if platform_is_sme2(platform) {
        // The messages are one block each and lie one after another: a run of
        // blocks. SME2's parent kernel reads a group of sixteen through one
        // pointer, so they go to it as that run, not as a table of sixteen borrows.
        let run = messages.as_chunks::<BLOCK_LEN>().0[..outputs.len()].as_flattened();
        // Sound: the caller found SME2 (platform_is_sme2).
        return unsafe { crate::sme2::hash_block_run(run, key, flags | CHUNK_START | CHUNK_END | ROOT, outputs.as_flattened_mut()) };
    }
    let mut table = crate::stack_vec::StackVec::<&[u8; BLOCK_LEN], TABLE>::new();
    table.extend(messages.chunks_exact(BLOCK_LEN).map(|message| message.try_into().expect("one-block messages")));
    assert_eq!(table.len(), outputs.len(), "one message per output");
    whole_messages(&table, key, flags, outputs, platform);
}

/// The kernel call for one message of whole blocks in each lane. Both a
/// contiguous batch and a table of separately borrowed messages use it
/// (on SME2 a batch of one-block messages goes another way, padded or
/// as a run: [`hash_run`]).
#[inline(always)]
fn whole_messages<const N: usize>(lanes: &[&[u8; N]], key: &CVWords, flags: u8, out: &mut [[u8; OUT_LEN]], platform: Platform) {
    const { assert!(N >= BLOCK_LEN && N <= CHUNK_LEN && N % BLOCK_LEN == 0) };
    let (flags, start, end) = if N == BLOCK_LEN { (flags | CHUNK_START | CHUNK_END | ROOT, 0, 0) } else { (flags, CHUNK_START, CHUNK_END | ROOT) };
    platform.hash_many(lanes, key, 0, IncrementCounter::No, flags, start, end, out.as_flattened_mut());
}

/// Try the full-block lanes on separately borrowed messages. A mismatch
/// leaves the outputs alone, for the caller's one-message path. Each lane
/// checks its own length as it becomes a reference to a fixed-size array.
pub(crate) fn try_whole_group<'a>(
    count: usize,
    input: impl Fn(usize) -> &'a [u8],
    key: &CVWords,
    flags: u8,
    platform: Platform,
    output: impl FnMut(usize, [u8; OUT_LEN]),
) -> bool {
    fn run<'a, const N: usize>(
        count: usize,
        input: impl Fn(usize) -> &'a [u8],
        key: &CVWords,
        flags: u8,
        platform: Platform,
        mut output: impl FnMut(usize, [u8; OUT_LEN]),
    ) -> bool {
        // The messages up to the first that is not N bytes: all of them, or
        // this group goes the one-message way. The table is filled where it
        // stands. Collected, it was copied whole to its place, a kibibyte
        // whatever the group's size, and a group of two 64-byte messages
        // took half as long again (BLAKE3 PR #484).
        let mut table = crate::stack_vec::StackVec::<&[u8; N], TABLE>::new();
        table.extend((0..count).map_while(|i| input(i).try_into().ok()));
        if table.len() != count {
            return false;
        }
        // Only a group that takes the lanes needs temporary outputs.
        let mut digests = crate::stack_vec::StackVec::<[u8; OUT_LEN], TABLE>::new();
        digests.extend(core::iter::repeat_n([0; OUT_LEN], count));
        whole_messages(&table, key, flags, &mut digests, platform);
        for (i, &digest) in digests.iter().enumerate() {
            output(i, digest);
        }
        true
    }
    if !(2..=TABLE).contains(&count) {
        return false;
    }
    match input(0).len() {
        BLOCK_LEN => run::<BLOCK_LEN>(count, input, key, flags, platform, output),
        128 => run::<128>(count, input, key, flags, platform, output),
        256 => run::<256>(count, input, key, flags, platform, output),
        512 => run::<512>(count, input, key, flags, platform, output),
        CHUNK_LEN => run::<CHUNK_LEN>(count, input, key, flags, platform, output),
        _ => false,
    }
}

/// One-block messages on SME2, every group on the message kernel, the
/// last one padded, their last block `last_len` bytes.
#[cfg(blake3_sme2)]
#[inline(never)]
fn hash_run_padded(messages: &[u8], key: &CVWords, flags: u8, outputs: &mut [[u8; OUT_LEN]], last_len: usize) {
    let mut slots: Slots<BLOCK_LEN> = [core::mem::MaybeUninit::uninit(); TABLE + 16];
    let lanes = fill_table(&mut slots, messages, outputs.len(), outputs.len().next_multiple_of(16));
    // Sound: the caller found SME2 (platform_is_sme2).
    unsafe { crate::sme2::hash_messages::<BLOCK_LEN>(lanes, key, flags, CHUNK_START, CHUNK_END | ROOT, last_len, outputs.as_flattened_mut()) };
}

/// [`hash_run`] for messages of 1 to 63 bytes (one short block): on SME2
/// the message kernel from ONE_BLOCK_PAD_MIN messages (the parent kernel
/// records 64 for every block), else the NEON plans, or one call each
/// without them.
#[inline(never)]
fn hash_run_short(messages: &[u8], key: &CVWords, flags: u8, outputs: &mut [[u8; OUT_LEN]], platform: Platform, last_len: usize) {
    // One-block messages on x86-64 with the independent-compression kernels
    // (AVX2's eight lanes, AVX-512's sixteen): whole groups and padded tails
    // (4..=7 on output8, 9..=15 on output16), a batch's and a `Packed`'s alike;
    // trailing 1..=3 messages, and every other platform, run as before.
    #[cfg(all(blake3_avx512_ffi, target_os = "linux"))]
    let done = one_block_groups(messages, outputs, last_len, key, flags, platform);
    #[cfg(not(all(blake3_avx512_ffi, target_os = "linux")))]
    let done = 0;
    let (messages, outputs) = (&messages[done * BLOCK_LEN..], &mut outputs[done..]);
    #[cfg(blake3_sme2)]
    if platform_is_sme2(platform) && outputs.len() >= ONE_BLOCK_PAD_MIN {
        return hash_run_padded(messages, key, flags, outputs, last_len);
    }
    #[cfg(blake3_neon)]
    {
        let mut slots: Slots<BLOCK_LEN> = [core::mem::MaybeUninit::uninit(); TABLE + 16];
        let lanes = fill_table(&mut slots, messages, outputs.len(), outputs.len());
        neon_plans_last_len::<BLOCK_LEN>(lanes, key, flags, last_len, outputs);
    }
    #[cfg(not(blake3_neon))]
    {
        for (output, message) in outputs.iter_mut().zip(messages.chunks_exact(BLOCK_LEN)) {
            *output = *crate::hash_serial_on(&message[..last_len], key, flags, platform).as_bytes();
        }
    }
}

/// Fewest one-block messages that SME2 hashes as whole groups on the
/// message kernel, the last one padded: 11 in a batch of fewer than
/// sixteen (fewer run on the NEON parent plans), 5 left over past whole
/// groups (fewer left over run on the NEON plans). VM, ns per message,
/// before / padded: 10 18.3 / 20.9, 11 19.1 / 15.0, 15 18.2 / 10.8.
/// Past whole groups the padded group replaces the plans and the overlap
/// group. The plans after a group run fast while the SME unit stays in
/// its fast state and 2.5x slower when it sits in its slow one; the
/// padded group runs at one speed. Mac M4 Max P-cores, ns per message,
/// left-overs from 13 / from 5: in a sweep where the unit sat slow at
/// 20 and 22-28 messages (probe onepad-after5, jobs 279-281), 22-28
/// 25.5-31.5 / 11.4-15.7 and 53 18.4 / 12.2; slower at 21 (13.6 / 15.6)
/// and 37-40 (10.1-11.8 / 11.9-13.0). In the benchmark, every sample
/// fast (jobs 291-294), 24 10.6 / 13.3 (official 21.0-21.7). The VM, in
/// the plans' fast state, 22, 24, 26 12-16% slower (their slow state is
/// twice as slow). The worst case halves; the fast case pays up to 25%.
/// Re-probed on Apple M6 after NEON `pn10` (#502, November 24, 2026): raising
/// this threshold from 11 to 16 speeds warm `BriefIdleManyMessages|12` (-43.0%)
/// but regresses cold-cache `ManyMessages|12` (+225.3%, 3.25x) and
/// `IdleManyMessages|12` (+74.1%). Keep 11 as the general default and direct
/// interleaved scalar+batch callers to `TreeHasher::on_this_core()`.
pub(crate) const ONE_BLOCK_PAD_MIN: usize = 11;
#[cfg_attr(not(blake3_sme2), allow(dead_code))]
pub(crate) const ONE_BLOCK_PAD_AFTER_GROUPS: usize = 5;

/// Room for a pointer table: TABLE messages and a padded group's spare
/// lanes, uninitialised on the caller's stack.
#[cfg(blake3_neon)]
type Slots<'a, const N: usize> = [core::mem::MaybeUninit<&'a [u8; N]>; TABLE + 16];

/// Fill `slots` with pointers to the first `count` slots of N bytes in
/// `messages`, then spare lanes repeating the last up to `lanes`; the
/// table's filled part.
#[cfg(blake3_neon)]
#[inline(always)]
fn fill_table<'a, 's, const N: usize>(slots: &'s mut Slots<'a, N>, messages: &'a [u8], count: usize, lanes: usize) -> &'s [&'a [u8; N]] {
    assert!(0 < count && count <= lanes && lanes <= TABLE + 16 && messages.len() >= count * N, "a table of 1 to TABLE + 16 lanes over the messages");
    let mut last: &[u8; N] = &[0; N];
    for (slot, message) in slots[..count].iter_mut().zip(messages.chunks_exact(N)) {
        last = message.try_into().expect("slots of N bytes");
        slot.write(last);
    }
    for slot in &mut slots[count..lanes] {
        slot.write(last);
    }
    // Sound: the first `lanes` slots were written just above.
    unsafe { core::slice::from_raw_parts(slots.as_ptr() as *const &[u8; N], lanes) }
}

/// The integer + NEON parent plans over `lanes`, each message's last block
/// `last_len` bytes.
#[cfg(blake3_neon)]
#[inline(always)]
fn neon_plans_last_len<const N: usize>(lanes: &[&[u8; N]], key: &CVWords, flags: u8, last_len: usize, outputs: &mut [[u8; OUT_LEN]]) {
    // Sound: every AArch64 CPU has NEON.
    unsafe {
        crate::neon_hybrid::hash_many_last_len(lanes, key, 0, IncrementCounter::No, flags, CHUNK_START, CHUNK_END | ROOT, last_len, outputs.as_flattened_mut())
    };
}

/// [`hash_run`] for messages of 2 to 16 blocks (N), the last block
/// `last_len` bytes, on one of three plans:
///
/// - SME2, SME2_TAIL_MIN or more messages in all (below sixteen) or
///   SME2_TAIL_MIN_AFTER_GROUPS or more past the groups of sixteen: whole
///   groups, the last one padded, its spare lanes pointing at the last
///   message again (no bytes move; the kernel computes every lane anyway
///   and stores the real ones alone).
/// - SME2 with fewer left over past the groups: those on the integer +
///   NEON plans first, then the groups (NEON work right after the SME2
///   kernels runs in the SME unit's slow state on the VM: 17 x 256 B, 68
///   ns per message after, 44 before).
/// - The integer + NEON plans (no SME2, or fewer messages): all at once.
#[cfg(blake3_neon)]
#[inline(never)]
fn hash_blocks<const N: usize>(messages: &[u8], key: &CVWords, flags: u8, outputs: &mut [[u8; OUT_LEN]], platform: Platform, last_len: usize) {
    const GROUP: usize = 16;
    let count = outputs.len();
    if count == 0 {
        return;
    }
    let sme2 = platform_is_sme2(platform);
    let left = count % GROUP;
    let padded = sme2 && left >= if count > GROUP { SME2_TAIL_MIN_AFTER_GROUPS } else { SME2_TAIL_MIN };
    // Messages hashed on SME2 (the first `on_sme2`), and the table's lanes.
    let on_sme2 = if padded {
        count
    } else if sme2 {
        count - left
    } else {
        0
    };
    let lanes = if padded { count.next_multiple_of(GROUP) } else { count };
    let mut slots: Slots<N> = [core::mem::MaybeUninit::uninit(); TABLE + 16];
    let table = fill_table(&mut slots, messages, count, lanes);
    let (groups, rest) = outputs.split_at_mut(on_sme2);
    if !rest.is_empty() {
        let rest_lanes = &table[on_sme2..count];
        neon_plans_last_len::<N>(rest_lanes, key, flags, last_len, rest);
    }
    #[cfg(blake3_sme2)]
    if on_sme2 > 0 {
        // Sound: platform_is_sme2 means detect() found SME2 with 512-bit
        // streaming vectors.
        let sme2_lanes = &table[..if padded { lanes } else { on_sme2 }];
        unsafe { crate::sme2::hash_messages::<N>(sme2_lanes, key, flags, CHUNK_START, CHUNK_END | ROOT, last_len, groups.as_flattened_mut()) };
    }
    #[cfg(not(blake3_sme2))]
    let _ = groups;
}

#[cfg(blake3_neon)]
#[inline(always)]
fn platform_is_sme2(platform: Platform) -> bool {
    #[cfg(blake3_sme2)]
    return matches!(platform, Platform::SME2);
    #[cfg(not(blake3_sme2))]
    {
        let _ = platform;
        false
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::vec::Vec;

    #[cfg(all(blake3_avx512_ffi, target_os = "linux"))]
    #[test]
    #[should_panic(expected = "an independent short message fits in one block")]
    fn an_independent_batch_refuses_a_multiblock_length() {
        crate::initialize();
        one_block_groups(&[], &mut [], BLOCK_LEN + 1, crate::IV, 0, Platform::detect());
    }

    #[test]
    fn separately_borrowed_whole_blocks_take_the_batch_lanes() {
        crate::initialize();
        let platform = Platform::detect();
        let tree = crate::throughput::TreeHasher::new(crate::Mode::Plain);
        let input = [std::vec![9; CHUNK_LEN], std::vec![7; CHUNK_LEN], std::vec![2; CHUNK_LEN]];
        let mut out = [[0; OUT_LEN]; 3];
        let hash_many_calls = || crate::COUNTED.with(|c| c[crate::Counted::HashManyCall as usize].get());
        for len in [64, 65, 128, 192, 256, 512, 1024] {
            let before = out;
            let bytes = |i: usize| &input[i][..len];
            let grouped = matches!(len, 64 | 128 | 256 | 512 | 1024);
            let calls_before = hash_many_calls();
            assert_eq!(tree.try_hash_group_on(platform, out.len(), bytes, |i, digest| out[i] = digest), grouped, "{len} bytes");
            let calls_delta = hash_many_calls() - calls_before;
            assert_eq!(calls_delta, u64::from(grouped), "HashManyCall delta for {len} bytes");
            if grouped {
                for (i, digest) in out.iter().enumerate() {
                    assert_eq!(digest, blake3::hash(bytes(i)).as_bytes());
                }
            } else {
                assert_eq!(out, before, "a group without a kernel leaves no output");
            }
        }
        let before = out;
        let calls_before = hash_many_calls();
        assert!(!tree.try_hash_group_on(platform, out.len(), |i| &input[i][..if i == 0 { 64 } else { 63 }], |i, digest| out[i] = digest));
        assert!(!tree.try_hash_group_on(platform, 1, |i| &input[i][..64], |i, digest| out[i] = digest));
        assert_eq!(hash_many_calls(), calls_before, "rejected groups make no HashManyCall");
        assert_eq!(out, before, "a mixed group or one message leaves no output");
    }

    /// `count` deterministic messages of `len` bytes, each in its slot
    /// (slot_len), zero past its end: little-endian 64-bit words `len << 48
    /// | index`, so every block differs and a kernel that mixed up its lanes
    /// would be caught.
    fn messages(len: usize, count: usize) -> Vec<u8> {
        let slot = slot_len(len);
        let mut bytes: Vec<u8> = (0..(slot * count).div_ceil(8) as u64).flat_map(|i| ((len as u64) << 48 | i).to_le_bytes()).collect();
        bytes.truncate(slot * count);
        for message in bytes.chunks_exact_mut(slot) {
            message[len..].fill(0);
        }
        bytes
    }

    /// Message `i` of a batch of `len`-byte messages.
    fn message(input: &[u8], len: usize, i: usize) -> &[u8] {
        &input[i * slot_len(len)..][..len]
    }

    /// sme2_sized(len, count), exactly (BLAKE3 #219; a predicate every
    /// build computes, whose answer only an SME2 machine acts on): sixteen
    /// messages or more; one-block messages from ONE_BLOCK_PAD_MIN (11);
    /// messages over a block up to a chunk from SME2_TAIL_MIN (10); 2 to 15
    /// chunks from sme2_chunked_min, on an SME2 build alone; and one message
    /// or more of SME2_SIZED_LEN (16 chunks) or longer.
    #[test]
    fn test_sme2_sized_exactly() {
        assert!(sme2_sized(0, 16) && !sme2_sized(0, 15), "sixteen messages of any length, empty ones too; fewer empty ones never");
        assert!(sme2_sized(BLOCK_LEN, 11) && !sme2_sized(BLOCK_LEN, 10), "one-block messages from eleven");
        assert!(sme2_sized(1, 11) && !sme2_sized(1, 10), "a byte is a one-block message");
        assert!(!sme2_sized(0, 11), "an empty message is no block");
        assert!(sme2_sized(BLOCK_LEN + 1, 10) && !sme2_sized(BLOCK_LEN + 1, 9), "over a block, up to a chunk, from ten");
        assert!(sme2_sized(CHUNK_LEN, 10) && !sme2_sized(CHUNK_LEN, 9));
        assert!(!sme2_sized(BLOCK_LEN, 10) && sme2_sized(BLOCK_LEN, 16), "a block at ten: neither rule; sixteen: the batch rule");
        // 2 to 15 chunks: the chunked rule on an SME2 build; elsewhere only
        // the batch rule (sixteen) reaches them.
        let two_chunks_min = if cfg!(blake3_sme2) { sme2_chunked_min(2 * CHUNK_LEN) } else { 16 };
        assert!(sme2_sized(2 * CHUNK_LEN, two_chunks_min) && !sme2_sized(2 * CHUNK_LEN, two_chunks_min - 1));
        assert!(!sme2_sized(15 * CHUNK_LEN, 1) && !sme2_sized(CHUNK_LEN + 1, 1));
        assert!(sme2_sized(16 * CHUNK_LEN, 1) && !sme2_sized(16 * CHUNK_LEN - 1, 1), "sixteen chunks, one message: long enough alone");
        assert!(sme2_sized(1 << 20, 1) && !sme2_sized(1 << 20, 0), "a long message, none of them: nothing");
    }

    /// Actual kernel widths dispatched on x86-64 (recorded at `Platform::hash_many`
    /// and `pair::hash_many_blocks`): verifies coalesced full heads + rounded
    /// tails across `128 B`, `256 B`, `1024 B`, the unrounded `64 B` exception,
    /// `128 B` `n = 3, 9, 10` fast paths, and `hash_group_across` full chunks.
    #[test]
    #[cfg(target_arch = "x86_64")]
    fn test_hash_blocks_arrangement() {
        let calls = |f: &mut dyn FnMut()| -> Vec<(usize, usize)> {
            crate::KERNEL_HASH_MANY_CALLS.with(|v| v.borrow_mut().clear());
            f();
            crate::KERNEL_HASH_MANY_CALLS.with(|v| v.borrow().clone())
        };
        for platform in crate::test::every_platform() {
            let is_512 = {
                #[cfg(blake3_avx512_ffi)]
                {
                    matches!(platform, Platform::AVX512)
                }
                #[cfg(not(blake3_avx512_ffi))]
                {
                    false
                }
            };
            let is_2 = {
                #[cfg(blake3_avx2_ffi)]
                {
                    matches!(platform, Platform::AVX2)
                }
                #[cfg(not(blake3_avx2_ffi))]
                {
                    false
                }
            };
            let is_sse41 = {
                #[cfg(blake3_sse41_ffi)]
                {
                    matches!(platform, Platform::SSE41)
                }
                #[cfg(not(blake3_sse41_ffi))]
                {
                    false
                }
            };
            if !is_512 && !is_2 && !is_sse41 {
                continue;
            }
            // 1. 64 B stays unrounded (`hash_run::<64>`).
            let m64 = messages(BLOCK_LEN, 6);
            let mut out6 = vec![[0u8; OUT_LEN]; 6];
            assert_eq!(
                calls(&mut || hash_many_on(&m64, BLOCK_LEN, BLOCK_LEN, crate::IV, 0, &mut out6, platform)),
                vec![(1, 6)],
                "64 B unrounded on {platform:?}"
            );

            // 2. 128 B (2 blocks) via `hash_many_on` (the public batch dispatcher):
            // `n = 3` uses inline 4-lane scratch (with 0 single-block `CompressInPlace`
            // scalar fallbacks on SSE4.1), `5..=7 -> 8`, `9..=10` stay unrounded on
            // AVX-512, `11..=15 -> 16`, `31 -> 16 + 16`, and `131` crosses `TABLE = 128`.
            let m128 = messages(2 * BLOCK_LEN, TABLE + 3);
            for count in [2usize, 3, 4, 5, 7, 8, 9, 10, 11, 15, 16, 31, TABLE + 3] {
                let mut out = vec![[0u8; OUT_LEN]; count];
                crate::COUNTED.with(|c| c[crate::Counted::CompressInPlace as usize].set(0));
                crate::RAN_ENTRIES.with(|r| r.borrow_mut().clear());
                let got = calls(&mut || hash_many_on(&m128[..count * 2 * BLOCK_LEN], 2 * BLOCK_LEN, 2 * BLOCK_LEN, crate::IV, 0, &mut out, platform));
                let want_entry: usize = if is_512 {
                    crate::avx512::ffi::blake3_servil_hash_many_avx512 as *const () as usize
                } else if is_2 {
                    crate::avx2::ffi::blake3_servil_hash_many_avx2 as *const () as usize
                } else {
                    crate::sse41::ffi::blake3_servil_hash_many_sse41 as *const () as usize
                };
                let ran_entries: std::collections::BTreeSet<usize> = crate::RAN_ENTRIES.with(|r| r.borrow().clone());
                assert_eq!(
                    ran_entries,
                    std::collections::BTreeSet::from([want_entry]),
                    "128 B count={count} on {platform:?} must call only its own ISA hash_many assembly"
                );
                if !is_512 && !is_2 && count == 3 {
                    let single_compress = crate::COUNTED.with(|c| c[crate::Counted::CompressInPlace as usize].get());
                    assert_eq!(single_compress, 0, "SSE4.1 3x128 B must call SSE4.1 hash_many assembly, not scalar CompressInPlace");
                }
                let want: Vec<(usize, usize)> = if is_512 {
                    match count {
                        3 => vec![(2, 4)],
                        5 | 7 => vec![(2, 8)],
                        11 | 15 => vec![(2, 16)],
                        31 => vec![(2, 16), (2, 16)],
                        131 => vec![(2, 128), (2, 4)],
                        _ => vec![(2, count)],
                    }
                } else if is_2 {
                    match count {
                        3 => vec![(2, 4)],
                        5 | 7 => vec![(2, 8)],
                        11 => vec![(2, 8), (2, 4)],
                        15 => vec![(2, 8), (2, 8)],
                        31 => vec![(2, 24), (2, 8)],
                        131 => vec![(2, 128), (2, 4)],
                        _ => vec![(2, count)],
                    }
                } else {
                    match count {
                        3 => vec![(2, 4)],
                        7 => vec![(2, 4), (2, 4)],
                        11 => vec![(2, 8), (2, 4)],
                        15 => vec![(2, 12), (2, 4)],
                        31 => vec![(2, 28), (2, 4)],
                        131 => vec![(2, 128), (2, 4)],
                        _ => vec![(2, count)],
                    }
                };
                assert_eq!(got, want, "128 B count={count} on {platform:?}");
                for (i, o) in out.iter().enumerate() {
                    assert_eq!(*o, *blake3::hash(&m128[i * 2 * BLOCK_LEN..][..2 * BLOCK_LEN]).as_bytes(), "128 B count={count} msg {i}");
                }
            }

            // 2b. 192 B (3 blocks) via `hash_many_on`: `9..=10` remain unrounded
            // on AVX-512 (`blocks >= 4` boundary).
            let m192 = messages(3 * BLOCK_LEN, 10);
            for count in [9usize, 10] {
                let mut out = vec![[0u8; OUT_LEN]; count];
                let got = calls(&mut || hash_many_on(&m192[..count * 3 * BLOCK_LEN], 3 * BLOCK_LEN, 3 * BLOCK_LEN, crate::IV, 0, &mut out, platform));
                assert_eq!(got, vec![(3, count)], "192 B count={count} stays unrounded on {platform:?}");
                for (i, o) in out.iter().enumerate() {
                    assert_eq!(*o, *blake3::hash(&m192[i * 3 * BLOCK_LEN..][..3 * BLOCK_LEN]).as_bytes(), "192 B count={count} msg {i}");
                }
            }

            // 3. 256 B (4 blocks) via `hash_many_on`: full rounding including `9..=10 -> 16` on AVX-512.
            let m256 = messages(4 * BLOCK_LEN, 31);
            for count in [3usize, 5, 9, 10, 15, 16, 19, 31] {
                let mut out = vec![[0u8; OUT_LEN]; count];
                let got = calls(&mut || hash_many_on(&m256[..count * 4 * BLOCK_LEN], 4 * BLOCK_LEN, 4 * BLOCK_LEN, crate::IV, 0, &mut out, platform));
                let want: Vec<(usize, usize)> = if is_512 {
                    match count {
                        3 => vec![(4, 4)],
                        5 => vec![(4, 8)],
                        9 | 10 | 15 => vec![(4, 16)],
                        16 => vec![(4, 16)],
                        19 => vec![(4, 16), (4, 4)],
                        31 => vec![(4, 16), (4, 16)],
                        _ => unreachable!(),
                    }
                } else if is_2 {
                    match count {
                        3 => vec![(4, 4)],
                        5 => vec![(4, 8)],
                        9 | 10 | 16 => vec![(4, count)],
                        15 => vec![(4, 8), (4, 8)],
                        19 => vec![(4, 16), (4, 4)],
                        31 => vec![(4, 24), (4, 8)],
                        _ => unreachable!(),
                    }
                } else {
                    match count {
                        3 => vec![(4, 4)],
                        15 => vec![(4, 12), (4, 4)],
                        19 => vec![(4, 16), (4, 4)],
                        31 => vec![(4, 28), (4, 4)],
                        _ => vec![(4, count)],
                    }
                };
                assert_eq!(got, want, "256 B count={count} on {platform:?}");
                for (i, o) in out.iter().enumerate() {
                    assert_eq!(*o, *blake3::hash(&m256[i * 4 * BLOCK_LEN..][..4 * BLOCK_LEN]).as_bytes(), "256 B count={count} msg {i}");
                }
            }

            // 4. 1024 B (16 blocks) via `hash_many_on`: routes through `sse41::hash_blocks_x86` -> `hash_many_rounded` (Zen 5 wide 32 on 19/31).
            let m1k = messages(CHUNK_LEN, 31);
            let wide = is_512 && cfg!(target_os = "linux") && crate::platform::wide_avx512();
            for count in [9usize, 15, 19, 31] {
                let mut out = vec![[0u8; OUT_LEN]; count];
                let got = calls(&mut || hash_many_on(&m1k[..count * CHUNK_LEN], CHUNK_LEN, CHUNK_LEN, crate::IV, 0, &mut out, platform));
                let want: Vec<(usize, usize)> = if wide {
                    match count {
                        9 | 15 => vec![(16, 16)],
                        19 | 31 => vec![(16, 32)],
                        _ => unreachable!(),
                    }
                } else if is_512 {
                    match count {
                        9 | 15 => vec![(16, 16)],
                        19 => vec![(16, 16), (16, 4)],
                        31 => vec![(16, 16), (16, 16)],
                        _ => unreachable!(),
                    }
                } else if is_2 {
                    match count {
                        9 => vec![(16, 9)],
                        15 => vec![(16, 8), (16, 8)],
                        19 => vec![(16, 8), (16, 8), (16, 4)],
                        31 => vec![(16, 8), (16, 8), (16, 8), (16, 8)],
                        _ => unreachable!(),
                    }
                } else {
                    match count {
                        9 => vec![(16, 9)],
                        15 => vec![(16, 4); 4],
                        19 => vec![(16, 4); 5],
                        31 => vec![(16, 4); 8],
                        _ => unreachable!(),
                    }
                };
                assert_eq!(got, want, "1024 B count={count} on {platform:?}");
                for (i, o) in out.iter().enumerate() {
                    assert_eq!(*o, *blake3::hash(&m1k[i * CHUNK_LEN..][..CHUNK_LEN]).as_bytes(), "1024 B count={count} msg {i}");
                }
            }

            // 5. `hash_group_across` (2048 B) rounds full chunks via `hash_many_rounded`.
            if is_512 || is_2 {
                let m2k = messages(2 * CHUNK_LEN, 15);
                let mut out = vec![[0u8; OUT_LEN]; 15];
                let got = calls(&mut || hash_many_on(&m2k[..7 * 2 * CHUNK_LEN], 2 * CHUNK_LEN, 2 * CHUNK_LEN, crate::IV, 0, &mut out[..7], platform));
                assert_eq!(got, vec![(16, 8), (16, 8), (1, 7)], "2048 B n=7 across on {platform:?}");
                for i in 0..7 {
                    assert_eq!(out[i], *blake3::hash(message(&m2k, 2 * CHUNK_LEN, i)).as_bytes());
                }
                if is_512 {
                    let got15 = calls(&mut || hash_many_on(&m2k, 2 * CHUNK_LEN, 2 * CHUNK_LEN, crate::IV, 0, &mut out, platform));
                    assert_eq!(got15, vec![(16, 16), (16, 16), (1, 15)], "2048 B n=15 across on AVX-512");
                }
            }
        }
        // SSE4.1 rounds remainder-3 tails to 4 lanes through `round_x86_blocks`.
        #[cfg(blake3_sse41_ffi)]
        {
            assert_eq!(crate::sse41::round_x86_blocks(Platform::SSE41, 4, 7), (4, 4));
            assert_eq!(crate::sse41::round_x86_blocks(Platform::SSE41, 4, 6), (0, 0));
        }
    }

    /// The paths hash_many_on takes by message count (BLAKE3 #219): one-
    /// block messages run on the kernels' table from two messages, one
    /// alone goes the general way; messages of 2 to 15 chunks run across
    /// messages from across_from's count, and one at a time below it.
    #[test]
    #[cfg(all(target_arch = "x86_64", blake3_avx2_ffi))]
    fn test_hash_many_on_paths_by_count() {
        use crate::{IV, Ran};
        let Some(avx2) = Platform::avx2() else { return };
        let ran = |call: &mut dyn FnMut()| {
            crate::RAN.with(|r| r.set(0));
            call();
            crate::RAN.with(|r| r.get())
        };
        let has = |ran: u32, kind: Ran| ran & kind as u32 != 0;
        let mut out = vec![[0u8; OUT_LEN]; 16];
        let blocks = messages(BLOCK_LEN, 2);
        let r = ran(&mut || hash_many_on(&blocks[..BLOCK_LEN], BLOCK_LEN, BLOCK_LEN, IV, 0, &mut out[..1], avx2));
        assert!(!has(r, Ran::BlockRun), "one block alone: the general way");
        let r = ran(&mut || hash_many_on(&blocks, BLOCK_LEN, BLOCK_LEN, IV, 0, &mut out[..2], avx2));
        assert!(has(r, Ran::BlockRun), "two blocks: the table");
        // Up to a chunk, two messages or more: the short runs; one alone,
        // or longer than a chunk, the one-message way.
        for len in [65, 500, CHUNK_LEN] {
            let two = messages(len, 2);
            let r = ran(&mut || hash_many_on(&two[..slot_len(len)], len, slot_len(len), IV, 0, &mut out[..1], avx2));
            assert!(has(r, Ran::MessageAlone) && !has(r, Ran::ShortRun), "{len} B alone: one message");
            let r = ran(&mut || hash_many_on(&two, len, slot_len(len), IV, 0, &mut out[..2], avx2));
            assert!(has(r, Ran::ShortRun) && !has(r, Ran::MessageAlone), "two of {len} B: a short run");
        }
        let two = messages(CHUNK_LEN + 1, 2);
        let r = ran(&mut || hash_many_on(&two, CHUNK_LEN + 1, slot_len(CHUNK_LEN + 1), IV, 0, &mut out[..2], avx2));
        assert!(!has(r, Ran::ShortRun), "two of a chunk and a byte: past the short runs");
        let check_digests = |buf: &[u8], len: usize, out: &[[u8; OUT_LEN]]| {
            for (i, o) in out.iter().enumerate() {
                assert_eq!(*o, *crate::convenience::hash(message(buf, len, i)).as_bytes());
            }
        };
        // Three whole chunks: across from four messages on AVX2 (under half lanes).
        let len3 = 3 * CHUNK_LEN;
        assert_eq!(across_from(len3, avx2), Some(4));
        let four = messages(len3, 4);
        out.fill([0xAA; OUT_LEN]);
        let r = ran(&mut || hash_many_on(&four[..3 * slot_len(len3)], len3, slot_len(len3), IV, 0, &mut out[..3], avx2));
        assert!(has(r, Ran::MessageAlone) && !has(r, Ran::Across), "three of 3 KiB: one at a time");
        check_digests(&four, len3, &out[..3]);
        out.fill([0xAA; OUT_LEN]);
        let r = ran(&mut || hash_many_on(&four, len3, slot_len(len3), IV, 0, &mut out[..4], avx2));
        assert!(has(r, Ran::Across) && !has(r, Ran::MessageAlone), "four of 3 KiB: across");
        check_digests(&four, len3, &out[..4]);
        // Four whole chunks: across from eight messages on AVX2, and a 9th
        // message in the tail falls back to one-at-a-time instead of a 1-lane
        // across group.
        let len4 = 4 * CHUNK_LEN;
        assert_eq!(across_from(len4, avx2), Some(8));
        let nine = messages(len4, 9);
        out.fill([0xAA; OUT_LEN]);
        let r = ran(&mut || hash_many_on(&nine[..7 * slot_len(len4)], len4, slot_len(len4), IV, 0, &mut out[..7], avx2));
        assert!(has(r, Ran::MessageAlone) && !has(r, Ran::Across), "seven: one at a time");
        check_digests(&nine, len4, &out[..7]);
        out.fill([0xAA; OUT_LEN]);
        let r = ran(&mut || hash_many_on(&nine[..8 * slot_len(len4)], len4, slot_len(len4), IV, 0, &mut out[..8], avx2));
        assert!(has(r, Ran::Across) && !has(r, Ran::MessageAlone), "eight: across");
        check_digests(&nine, len4, &out[..8]);
        out.fill([0xAA; OUT_LEN]);
        let r = ran(&mut || hash_many_on(&nine, len4, slot_len(len4), IV, 0, &mut out[..9], avx2));
        assert!(has(r, Ran::Across) && has(r, Ran::MessageAlone), "nine on AVX2: 8 across + 1 serial tail");
        check_digests(&nine, len4, &out[..9]);
        #[cfg(blake3_avx512_ffi)]
        if let Some(avx512) = Platform::avx512() {
            let mut out26 = vec![[0u8; OUT_LEN]; 26];
            // 4 KiB on AVX-512: 7 serial, 8 across, 9 & 26 test both wide and
            // non-wide peeling directly, and 26 exercises peeling after a full 16 tile.
            let m26 = messages(len4, 26);
            out26.fill([0xAA; OUT_LEN]);
            let r = ran(&mut || hash_many_on(&m26[..7 * slot_len(len4)], len4, slot_len(len4), IV, 0, &mut out26[..7], avx512));
            assert!(has(r, Ran::MessageAlone) && !has(r, Ran::Across), "AVX-512 4 KiB x 7: serial");
            check_digests(&m26, len4, &out26[..7]);
            out26.fill([0xAA; OUT_LEN]);
            let r = ran(&mut || hash_many_on(&m26[..8 * slot_len(len4)], len4, slot_len(len4), IV, 0, &mut out26[..8], avx512));
            assert!(has(r, Ran::Across) && !has(r, Ran::MessageAlone), "AVX-512 4 KiB x 8: across");
            check_digests(&m26, len4, &out26[..8]);
            for wide in [false, true] {
                for count in [9usize, 26] {
                    out26.fill([0xAA; OUT_LEN]);
                    let r = ran(&mut || hash_long_or_single_for(&m26[..count * slot_len(len4)], len4, IV, 0, &mut out26[..count], avx512, wide));
                    assert!(has(r, Ran::Across) && has(r, Ran::MessageAlone) == !wide, "AVX-512 4 KiB x {count} wide={wide}");
                    check_digests(&m26, len4, &out26[..count]);
                }
            }
            // 3 KiB x 10 never peels (chunks < 4); 2 KiB x 10/19 run all across.
            let m10_3 = messages(len3, 10);
            out26.fill([0xAA; OUT_LEN]);
            let r = ran(&mut || hash_long_or_single_for(&m10_3, len3, IV, 0, &mut out26[..10], avx512, false));
            assert!(has(r, Ran::Across) && !has(r, Ran::MessageAlone), "AVX-512 3 KiB x 10: all across");
            check_digests(&m10_3, len3, &out26[..10]);
            let len2 = 2 * CHUNK_LEN;
            let m19 = messages(len2, 19);
            for count in [10usize, 19] {
                out26.fill([0xAA; OUT_LEN]);
                let r = ran(&mut || hash_long_or_single_for(&m19[..count * slot_len(len2)], len2, IV, 0, &mut out26[..count], avx512, false));
                assert!(has(r, Ran::Across) && !has(r, Ran::MessageAlone), "AVX-512 2 KiB x {count}: all across");
                check_digests(&m19, len2, &out26[..count]);
            }
            // 8 KiB on AVX-512: 13 serial, 14 across, 17 = 16 across + 1 serial.
            let len8 = 8 * CHUNK_LEN;
            let m17 = messages(len8, 17);
            out26.fill([0xAA; OUT_LEN]);
            let r = ran(&mut || hash_many_on(&m17[..13 * slot_len(len8)], len8, slot_len(len8), IV, 0, &mut out26[..13], avx512));
            assert!(has(r, Ran::MessageAlone) && !has(r, Ran::Across), "AVX-512 8 KiB x 13: serial");
            check_digests(&m17, len8, &out26[..13]);
            out26.fill([0xAA; OUT_LEN]);
            let r = ran(&mut || hash_many_on(&m17[..14 * slot_len(len8)], len8, slot_len(len8), IV, 0, &mut out26[..14], avx512));
            assert!(has(r, Ran::Across) && !has(r, Ran::MessageAlone), "AVX-512 8 KiB x 14: across");
            check_digests(&m17, len8, &out26[..14]);
            out26.fill([0xAA; OUT_LEN]);
            let r = ran(&mut || hash_many_on(&m17, len8, slot_len(len8), IV, 0, &mut out26[..17], avx512));
            assert!(has(r, Ran::Across) && has(r, Ran::MessageAlone), "AVX-512 8 KiB x 17: 16 across + 1 serial");
            check_digests(&m17, len8, &out26[..17]);
            // 10 KiB on AVX-512: 9 serial, 10 across (peels 8+2 on !wide), 15 across.
            let len10 = 10 * CHUNK_LEN;
            let m15 = messages(len10, 15);
            out26.fill([0xAA; OUT_LEN]);
            let r = ran(&mut || hash_many_on(&m15[..9 * slot_len(len10)], len10, slot_len(len10), IV, 0, &mut out26[..9], avx512));
            assert!(has(r, Ran::MessageAlone) && !has(r, Ran::Across), "AVX-512 10 KiB x 9: serial");
            check_digests(&m15, len10, &out26[..9]);
            out26.fill([0xAA; OUT_LEN]);
            let r = ran(&mut || hash_many_on(&m15, len10, slot_len(len10), IV, 0, &mut out26[..15], avx512));
            assert!(has(r, Ran::Across) && !has(r, Ran::MessageAlone), "AVX-512 10 KiB x 15: across");
            check_digests(&m15, len10, &out26[..15]);
        }
    }

    /// across_from(len, platform) and across_take(rem, chunks, lanes, wide)
    /// (BLAKE3 #219): exact admission floors and tile sizing on AVX2, AVX-512,
    /// and SSE4.1.
    #[test]
    #[cfg(all(target_arch = "x86_64", blake3_avx2_ffi))]
    fn test_across_from_exactly() {
        let avx2 = Platform::AVX2;
        assert_eq!(across_from(CHUNK_LEN, avx2), None, "one chunk");
        assert_eq!(across_from(CHUNK_LEN + 1, avx2), Some(2), "two chunks, one partial");
        assert_eq!(across_from(2 * CHUNK_LEN, avx2), Some(2), "two whole chunks");
        assert_eq!(across_from(2 * CHUNK_LEN + 1, avx2), Some(2), "three chunks, one partial");
        assert_eq!(across_from(3 * CHUNK_LEN, avx2), Some(4), "three whole chunks: under half AVX2 lanes");
        assert_eq!(across_from(3 * CHUNK_LEN + 1, avx2), Some(4), "four chunks, one partial");
        assert_eq!(across_from(4 * CHUNK_LEN, avx2), Some(8), "four whole chunks: half AVX2 lanes");
        assert_eq!(across_from(6 * CHUNK_LEN + 500, avx2), Some(4), "seven chunks, one partial, short of the lanes");
        assert_eq!(across_from(7 * CHUNK_LEN + 500, avx2), Some(8), "eight chunks fill the lanes, partial or not");
        assert_eq!(across_from(8 * CHUNK_LEN, avx2), Some(8), "eight whole chunks");
        assert_eq!(across_from(8 * CHUNK_LEN + 1, avx2), Some(7), "nine chunks: serial needs two AVX2 passes");
        assert_eq!(across_from(14 * CHUNK_LEN, avx2), Some(7), "fourteen chunks: 7/8 padded beats two AVX2 passes");
        assert_eq!(across_from(15 * CHUNK_LEN, avx2), Some(8), "fifteen chunks");
        assert_eq!(across_from(15 * CHUNK_LEN + 1, avx2), None, "sixteen chunks");
        assert_eq!(across_from(16 * CHUNK_LEN, avx2), None);
        assert_eq!(across_from(3 * CHUNK_LEN, Platform::SSE41), None, "SSE4.1 hashes message by message");
        // Tile sizing: ACROSS_GROUP when rem >= 16; peel `lanes` on AVX2 when
        // the leftover < least (keep `rem` together when leftover >= least);
        // peel 8 for 9..=10 on non-wide AVX-512 when chunks >= 4.
        assert_eq!(across_take(4, 4, 8, 4, false), 4);
        assert_eq!(across_take(5, 4, 8, 4, false), 5);
        assert_eq!(across_take(7, 4, 8, 4, false), 7);
        assert_eq!(across_take(8, 4, 8, 8, false), 8);
        assert_eq!(across_take(9, 3, 8, 4, false), 8);
        assert_eq!(across_take(11, 4, 8, 4, false), 8);
        assert_eq!(across_take(11, 4, 8, 8, false), 8);
        assert_eq!(across_take(12, 4, 8, 4, false), 12);
        assert_eq!(across_take(10, 2, 8, 2, false), 10);
        assert_eq!(across_take(15, 10, 8, 7, false), 15);
        assert_eq!(across_take(16, 4, 8, 8, false), 16);
        assert_eq!(across_take(17, 4, 8, 8, false), 16);
        assert_eq!(across_take(9, 3, 16, 8, false), 9);
        assert_eq!(across_take(10, 3, 16, 8, false), 10);
        assert_eq!(across_take(8, 4, 16, 8, false), 8);
        assert_eq!(across_take(9, 4, 16, 8, false), 8);
        assert_eq!(across_take(10, 4, 16, 8, false), 8);
        assert_eq!(across_take(11, 4, 16, 8, false), 11);
        assert_eq!(across_take(9, 4, 16, 8, true), 9);
        assert_eq!(across_take(10, 4, 16, 8, true), 10);
        assert_eq!(across_take(16, 4, 16, 15, false), 16);
        assert_eq!(across_take(17, 4, 16, 15, false), 16);
        #[cfg(blake3_avx512_ffi)]
        {
            // AVX-512 across lanes are capped at ACROSS_GROUP = 16 (never
            // simd_degree()'s 32 for single-tree dispatch). Pin literals so a
            // future simd_degree() change cannot silently drift the threshold.
            assert_eq!(across_from(2 * CHUNK_LEN, Platform::AVX512), Some(2), "two whole chunks");
            assert_eq!(across_from(2 * CHUNK_LEN + 1, Platform::AVX512), Some(2), "three chunks, one partial");
            assert_eq!(across_from(3 * CHUNK_LEN, Platform::AVX512), Some(8), "three whole chunks: under half of 16");
            assert_eq!(across_from(7 * CHUNK_LEN, Platform::AVX512), Some(8), "seven whole chunks: under half of 16");
            assert_eq!(across_from(8 * CHUNK_LEN, Platform::AVX512), Some(14), "eight whole chunks: fills 8-lane AVX2 alone");
            assert_eq!(across_from(8 * CHUNK_LEN + 1, Platform::AVX512), Some(4), "nine chunks, one partial, short of 16");
            assert_eq!(across_from(9 * CHUNK_LEN, Platform::AVX512), Some(9), "nine whole chunks: pads to 16 alone");
            assert_eq!(across_from(12 * CHUNK_LEN, Platform::AVX512), Some(12), "twelve whole chunks");
            assert_eq!(across_from(15 * CHUNK_LEN, Platform::AVX512), Some(15), "fifteen whole chunks");
        }
    }

    /// hash_many of `count` messages of
    /// `len` bytes against hash() one message at a time.
    fn check(len: usize, count: usize) {
        let input = messages(len, count);
        let mut out = vec![[0u8; OUT_LEN]; count];
        crate::throughput::hash_many(crate::Mode::Plain, &crate::throughput::Batch::in_place(&crate::test::blocks(&input), len), &mut out);
        for (i, digest) in out.iter().enumerate() {
            assert_eq!(*digest, *crate::convenience::hash(message(&input, len, i)).as_bytes(), "message {i} of {count}, {len} bytes");
        }
    }

    /// Messages of 2 to 16 whole blocks, at counts that fill SME2 groups
    /// and leave remainders.
    #[test]
    fn test_hash_many_whole_blocks() {
        for blocks in 2..=16 {
            for count in [0, 1, 2, 3, 4, 5, 6, 7, 15, 16, 17, 18, 31, 32, 33, 127, 128, 129, 130, 300] {
                check(blocks * BLOCK_LEN, count);
            }
        }
    }

    /// Whole-block x86 batches on every available SIMD implementation,
    /// including tails and table boundaries, against fixed values (official
    /// BLAKE3's digests) in all three modes, with output sentinels.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn test_whole_blocks_x86_full_width() {
        #[allow(unused_mut)]
        let mut platforms: Vec<_> = [Platform::sse41(), Platform::avx2()].into_iter().flatten().collect();
        #[cfg(blake3_avx512_ffi)]
        platforms.extend(Platform::avx512());
        let key = [0x5Au8; crate::KEY_LEN];
        let context = "Linux whole-block batch dispatch";
        let context_key = crate::throughput::context_key(context);
        let modes = [
            (*crate::IV, 0),
            (crate::platform::words_from_le_bytes_32(&key), crate::KEYED_HASH),
            (crate::platform::words_from_le_bytes_32(&context_key), crate::DERIVE_KEY_MATERIAL),
        ];
        let api_modes = [crate::Mode::Plain, crate::Mode::Keyed(&key), crate::Mode::DeriveKey(context)];
        let mut golden = crate::test::Golden::new("whole_blocks_x86_full_width");
        for len in [128, 256, 512, CHUNK_LEN, 2047, 2048, 2049, 4095, 4096, 4097] {
            for count in (0..=33).chain([63, 64, 127, 128, 129, 255]) {
                let bytes = messages(len, count);
                // The digests, once per mode, one message at a time, against
                // the fixed values: the input's alignment below changes
                // nothing about them.
                let wants: Vec<Vec<[u8; OUT_LEN]>> = api_modes
                    .iter()
                    .map(|&mode| (0..count).map(|i| *crate::convenience::hash_with(mode, message(&bytes, len, i)).as_bytes()).collect())
                    .collect();
                for (&mode, want) in api_modes.iter().zip(&wants) {
                    golden.add(want.as_flattened(), || (0..count).flat_map(|i| crate::test::official::hash(mode, message(&bytes, len, i))).collect());
                }
                // Four alignments of the input (a wrong pointer step shows
                // at any of them): 64 took the suite from 2 s to 16 s.
                for offset in [0, 1, 7, 63] {
                    let mut allocation = vec![0; offset + bytes.len()];
                    allocation[offset..].copy_from_slice(&bytes);
                    let input = &allocation[offset..];
                    for ((key_words, flags), want) in modes.iter().zip(&wants) {
                        for &platform in &platforms {
                            let mut out = vec![[0xAA; OUT_LEN]; count + 1];
                            hash_many_on(input, len, slot_len(len), key_words, *flags, &mut out[..count], platform);
                            assert_eq!(&out[..count], want, "{platform:?}, {count} x {len}, offset {offset}, flags {flags}");
                            assert_eq!(out[count], [0xAA; OUT_LEN]);
                        }
                    }
                }
            }
        }
        golden.check(0xba67a57a036242e7);
    }

    /// hash_across with partial chunks (1025, 3073, 7169 bytes: one whole
    /// chunk and a byte, three and a byte, seven and a byte), on every
    /// platform, against the published vectors
    /// (test_vectors/test_vectors.json, byte i = i % 251), each message in
    /// its zero-padded slot.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn across_partial_chunks_matches_published_vectors() {
        let key = b"whats the Elvish word for friend";
        let context = "BLAKE3 2019-12-27 16:29:52 test vectors context";
        let context_key = crate::throughput::context_key(context);
        let modes = [
            (*crate::IV, 0),
            (crate::platform::words_from_le_bytes_32(key), crate::KEYED_HASH),
            (crate::platform::words_from_le_bytes_32(&context_key), crate::DERIVE_KEY_MATERIAL),
        ];
        let cases = [
            (
                1025,
                [
                    "d00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444",
                    "357dc55de0c7e382c900fd6e320acc04146be01db6a8ce7210b7189bd664ea69",
                    "effaa245f065fbf82ac186839a249707c3bddf6d3fdda22d1b95a3c970379bcb",
                ],
            ),
            (
                3073,
                [
                    "7124b49501012f81cc7f11ca069ec9226cecb8a2c850cfe644e327d22d3e1cd3",
                    "68dede9bef00ba89e43f31a6825f4cf433389fedae75c04ee9f0cf16a427c95a",
                    "72613c9ec9ff7e40f8f5c173784c532ad852e827dba2bf85b2ab4b76f7079081",
                ],
            ),
            (
                7169,
                [
                    "a003fc7a51754a9b3c7fae0367ab3d782dccf28855a03d435f8cfe74605e7817",
                    "ed9b1a922c046fdb3d423ae34e143b05ca1bf28b710432857bf738bcedbfa511",
                    "554b0a5efea9ef183f2f9b931b7497995d9eb26f5c5c6dad2b97d62fc5ac31d9",
                ],
            ),
        ];
        for (len, anchors) in cases {
            let slot = slot_len(len);
            for count in &[4usize, 17] {
                let mut input = vec![0u8; slot * count];
                for m in 0..*count {
                    for (i, b) in input[m * slot..][..len].iter_mut().enumerate() {
                        *b = (i % 251) as u8;
                    }
                }
                for ((words, flags), anchor) in modes.iter().zip(anchors) {
                    let expected: [u8; OUT_LEN] = crate::Hash::from_hex(anchor).unwrap().into();
                    for platform in crate::test::every_platform() {
                        let mut out = vec![[0u8; OUT_LEN]; *count];
                        if crate::pair::takes_platform(platform) {
                            hash_many_on(&input, len, slot, words, *flags, &mut out, platform);
                        } else {
                            let msgs: Vec<&[u8]> = (0..*count).map(|m| &input[m * slot..][..len]).collect();
                            for (g, digests) in out.chunks_mut(ACROSS_GROUP).enumerate() {
                                hash_group_across(&msgs[g * ACROSS_GROUP..][..digests.len()], len, words, *flags, digests, platform);
                            }
                        }
                        assert!(out.iter().all(|digest| *digest == expected), "{platform:?}: {count} x {len} B");
                    }
                }
            }
        }
    }

    /// Published anchors exercise the two-chunk helper on every platform.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn two_chunk_batch_matches_published_vectors() {
        let key = b"whats the Elvish word for friend";
        let context = "BLAKE3 2019-12-27 16:29:52 test vectors context";
        let context_key = crate::throughput::context_key(context);
        let modes = [
            (*crate::IV, 0),
            (crate::platform::words_from_le_bytes_32(key), crate::KEYED_HASH),
            (crate::platform::words_from_le_bytes_32(&context_key), crate::DERIVE_KEY_MATERIAL),
        ];
        // test_vectors/test_vectors.json: 2048 bytes, byte i = i % 251.
        let anchors = [
            "e776b6028c7cd22a4d0ba182a8bf62205d2ef576467e838ed6f2529b85fba24a",
            "879cf1fa2ea0e79126cb1063617a05b6ad9d0b696d0d757cf053439f60a99dd1",
            "7b2945cb4fef70885cc5d78a87bf6f6207dd901ff239201351ffac04e1088a23",
        ];
        let message: Vec<u8> = (0..2 * CHUNK_LEN).map(|i| (i % 251) as u8).collect();
        for count in [4, 17] {
            let input = message.repeat(count);
            for ((words, flags), anchor) in modes.iter().zip(anchors) {
                let expected: [u8; OUT_LEN] = crate::Hash::from_hex(anchor).unwrap().into();
                for platform in crate::test::every_platform() {
                    let mut out = vec![[0u8; OUT_LEN]; count];
                    if crate::pair::takes_platform(platform) {
                        hash_many_on(&input, 2 * CHUNK_LEN, 2 * CHUNK_LEN, words, *flags, &mut out, platform);
                    } else {
                        let msgs: Vec<&[u8]> = (0..count).map(|m| &input[m * 2 * CHUNK_LEN..][..2 * CHUNK_LEN]).collect();
                        for (g, digests) in out.chunks_mut(ACROSS_GROUP).enumerate() {
                            hash_group_across(&msgs[g * ACROSS_GROUP..][..digests.len()], 2 * CHUNK_LEN, words, *flags, digests, platform);
                        }
                    }
                    assert!(out.iter().all(|digest| *digest == expected), "{platform:?}");
                }
            }
        }
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn four_chunk_batch_matches_published_vectors() {
        let key = b"whats the Elvish word for friend";
        let context = "BLAKE3 2019-12-27 16:29:52 test vectors context";
        let context_key = crate::throughput::context_key(context);
        let modes = [
            (*crate::IV, 0),
            (crate::platform::words_from_le_bytes_32(key), crate::KEYED_HASH),
            (crate::platform::words_from_le_bytes_32(&context_key), crate::DERIVE_KEY_MATERIAL),
        ];
        // Published test_vectors/test_vectors.json, 4096 bytes, i % 251.
        let anchors = [
            "015094013f57a5277b59d8475c0501042c0b642e531b0a1c8f58d2163229e969",
            "befc660aea2f1718884cd8deb9902811d332f4fc4a38cf7c7300d597a081bfc0",
            "1e0d7f3db8c414c97c6307cbda6cd27ac3b030949da8e23be1a1a924ad2f25b9",
        ];
        let message: Vec<u8> = (0..4 * CHUNK_LEN).map(|i| (i % 251) as u8).collect();
        for count in [4, 17] {
            let input = message.repeat(count);
            for ((words, flags), anchor) in modes.iter().zip(anchors) {
                let expected: [u8; OUT_LEN] = crate::Hash::from_hex(anchor).unwrap().into();
                for platform in crate::test::every_platform() {
                    let mut out = vec![[0u8; OUT_LEN]; count];
                    if crate::pair::takes_platform(platform) {
                        hash_many_on(&input, 4 * CHUNK_LEN, 4 * CHUNK_LEN, words, *flags, &mut out, platform);
                    } else {
                        let msgs: Vec<&[u8]> = (0..count).map(|m| &input[m * 4 * CHUNK_LEN..][..4 * CHUNK_LEN]).collect();
                        for (g, digests) in out.chunks_mut(ACROSS_GROUP).enumerate() {
                            hash_group_across(&msgs[g * ACROSS_GROUP..][..digests.len()], 4 * CHUNK_LEN, words, *flags, digests, platform);
                        }
                    }
                    assert!(out.iter().all(|digest| *digest == expected), "{platform:?}");
                }
            }
        }
    }

    /// Whole-block multi-message batches (`256 B`, 0..=40 outputs) across every
    /// platform this CPU supports (`sse41::hash_blocks_x86` on x86-64, NEON/SME2
    /// `hash_blocks` on AArch64), verifying digests and untouched trailing sentinels.
    #[test]
    fn test_hash_blocks_multi_count_sentinels() {
        let platforms = crate::test::every_platform();
        for count in 0..=40 {
            let input = messages(4 * BLOCK_LEN, count);
            for &platform in &platforms {
                let mut out = vec![[0xAAu8; OUT_LEN]; count + 16];
                #[cfg(blake3_sse41_ffi)]
                crate::sse41::hash_blocks_x86(&input, 4, crate::IV, 0, &mut out[..count], platform, BLOCK_LEN);
                #[cfg(blake3_neon)]
                hash_blocks::<{ 4 * BLOCK_LEN }>(&input, crate::IV, 0, &mut out[..count], platform, BLOCK_LEN);
                assert!(out[count..].iter().all(|d| *d == [0xAA; OUT_LEN]), "{platform:?}, {count}: a store past the last output");
                for (i, digest) in out[..count].iter().enumerate() {
                    assert_eq!(*digest, *crate::convenience::hash(message(&input, 256, i)).as_bytes(), "{platform:?}, message {i} of {count}");
                }
            }
        }
    }

    /// The keyed and derive-key modes through every batch path on every
    /// platform this CPU has (one-block runs, short blocks, whole blocks,
    /// two chunks, 2-15 chunks, longer messages),
    /// against keyed_hash() and derive_key() one message at a time.
    #[test]
    fn test_hash_many_modes_every_platform() {
        #[allow(unused_mut)]
        let mut platforms = vec![Platform::detect()];
        #[cfg(blake3_neon)]
        platforms.push(Platform::neon().expect("NEON on AArch64"));
        let key = [0x5Au8; crate::KEY_LEN];
        let context = "blake3-servil many modes test";
        let context_key = crate::throughput::context_key(context);
        let modes: [(CVWords, u8, &dyn Fn(&[u8]) -> [u8; OUT_LEN]); 2] = [
            (crate::platform::words_from_le_bytes_32(&key), crate::KEYED_HASH, &|m| *crate::convenience::hash_with(crate::Mode::Keyed(&key), m).as_bytes()),
            (crate::platform::words_from_le_bytes_32(&context_key), crate::DERIVE_KEY_MATERIAL, &|m| {
                *crate::convenience::hash_with(crate::Mode::DeriveKey(context), m).as_bytes()
            }),
        ];
        for len in [0, 1, 63, BLOCK_LEN, 100, 256, 1000, CHUNK_LEN, CHUNK_LEN + 1, 2000, 2 * CHUNK_LEN, 4000, 15 * CHUNK_LEN, 16 * CHUNK_LEN + 5] {
            for count in [1, 2, 3, 4, 5, 10, 11, 12, 16, 17, 21, 33, 129] {
                let input = messages(len, count);
                for (key_words, flags, one) in &modes {
                    let want: Vec<[u8; OUT_LEN]> = (0..count).map(|i| one(message(&input, len, i))).collect();
                    for &platform in &platforms {
                        let mut out = vec![[0u8; OUT_LEN]; count];
                        hash_many_on(&input, len, slot_len(len), key_words, *flags, &mut out, platform);
                        assert_eq!(out, want, "{platform:?}, flags {flags}, {count} x {len} B");
                    }
                }
            }
        }
    }

    /// Batches of 1 to 17 messages of up to a block after a pause (the pair
    /// kernel's path for 2 to 8, prefetched plans beyond), every mode: each
    /// digest is hash_with's.
    #[test]
    fn test_small_batches_after_a_pause() {
        let key = [7u8; 32];
        for mode in [crate::Mode::Plain, crate::Mode::Keyed(&key), crate::Mode::DeriveKey("after a pause")] {
            for len in [0, 1, 31, 63, 64] {
                for count in 1..=17 {
                    let slot = slot_len(len);
                    let mut input = vec![0u8; slot * count];
                    for (k, s) in input.chunks_mut(slot).enumerate() {
                        for (j, b) in s[..len].iter_mut().enumerate() {
                            *b = (k * 31 + j) as u8;
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_micros(300));
                    let mut out = vec![[0u8; OUT_LEN]; count];
                    crate::throughput::hash_many(mode, &crate::throughput::Batch::in_place(&crate::test::blocks(&input), len), &mut out);
                    for (k, digest) in out.iter().enumerate() {
                        assert_eq!(digest, crate::convenience::hash_with(mode, &input[k * slot..][..len]).as_bytes(), "len {len} count {count} message {k}");
                    }
                }
            }
        }
    }

    /// The length check holds in every build, inside the crate too: it
    /// stands before the kernels, which would read a short input past its
    /// end.
    #[test]
    #[should_panic(expected = "input holds one slot per output")]
    fn test_a_short_input_is_refused_before_the_kernels() {
        crate::initialize();
        hash_many_on(&[0u8; 64], 64, 64, crate::IV, 0, &mut [[0u8; OUT_LEN]; 2], crate::platform::Platform::detect());
    }

    /// The slot check too, in every build: this call crashed 0.9.0's
    /// release build, its two bytes named a 64-byte message (BLAKE3 #419).
    #[test]
    #[should_panic(expected = "a slot is whole blocks or a packed message")]
    fn test_a_slot_of_another_size_is_refused_before_the_kernels() {
        crate::initialize();
        hash_many_on(&[0u8; 2], 64, 1, crate::IV, 0, &mut [[0u8; OUT_LEN]; 2], crate::platform::Platform::detect());
    }

    #[test]
    #[should_panic(expected = "input holds one slot per output")]
    fn test_hash_many_needs_the_input_it_names() {
        crate::throughput::hash_many(
            crate::Mode::Plain,
            &crate::throughput::Batch::from_blocks(&[crate::throughput::Block::ZERO], 64),
            &mut [[0u8; OUT_LEN]; 2],
        );
    }

    #[test]
    fn test_assert_slots_and_slot_len_reject_invalid_shapes() {
        // Packed widths are strictly 16 and 32 bytes.
        assert_slots(&[0u8; 16], 16, 16, 1);
        assert_slots(&[0u8; 32], 32, 32, 1);
        assert_slots(&[0u8; 64], 0, 64, 1);
        for (len, slot, count) in [(0, 0, 1), (0, 0, 0), (17, 17, 1), (3, 3, 0), (63, 63, 1)] {
            assert!(
                std::panic::catch_unwind(|| assert_slots(&[0u8; 64][..slot * count], len, slot, count)).is_err(),
                "invalid packed shape ({len}, {slot}, {count}) must panic"
            );
        }
        assert!(std::panic::catch_unwind(|| slot_len(usize::MAX)).is_err(), "slot_len(usize::MAX) must panic instead of wrapping");
        assert!(std::panic::catch_unwind(|| slot_len(usize::MAX - 62)).is_err(), "slot_len near usize::MAX must panic");
    }
}
