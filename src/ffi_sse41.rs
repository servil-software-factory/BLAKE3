use crate::{BLOCK_LEN, CVWords, IncrementCounter, OUT_LEN};

// Unsafe because this may only be called on platforms supporting SSE4.1.
pub unsafe fn compress_in_place(cv: &mut CVWords, block: &[u8; BLOCK_LEN], block_len: u8, counter: u64, flags: u8) {
    // Sound: caller verified SSSE3+SSE4.1 support; `cv` and `block` are valid non-overlapping fixed-size references.
    unsafe { ffi::blake3_servil_compress_in_place_sse41(cv.as_mut_ptr(), block.as_ptr(), block_len, counter, flags) }
}

// Unsafe because this may only be called on platforms supporting SSE4.1.
pub unsafe fn compress_xof(cv: &CVWords, block: &[u8; BLOCK_LEN], block_len: u8, counter: u64, flags: u8) -> [u8; 64] {
    // Sound: caller verified SSSE3+SSE4.1 support; `cv`, `block`, and `out` are valid non-overlapping fixed-size buffers.
    unsafe {
        let mut out = [0u8; 64];
        ffi::blake3_servil_compress_xof_sse41(cv.as_ptr(), block.as_ptr(), block_len, counter, flags, out.as_mut_ptr());
        out
    }
}

// Unsafe because this may only be called on platforms supporting SSE4.1.
pub unsafe fn hash_many<const N: usize>(
    inputs: &[&[u8; N]],
    key: &CVWords,
    counter: u64,
    increment_counter: IncrementCounter,
    flags: u8,
    flags_start: u8,
    flags_end: u8,
    out: &mut [u8],
) {
    const { assert!(N > 0 && N % BLOCK_LEN == 0, "hash_many inputs are whole blocks") };
    // Sound: caller verified SSSE3+SSE4.1 support; `inputs` holds valid `N`-byte pointers and `out` is checked below to hold `inputs.len() * OUT_LEN` bytes.
    unsafe {
        // Check `out` capacity before calling assembly without multiplying
        // `inputs.len()`.
        assert!(inputs.len() <= out.len() / OUT_LEN);
        ffi::blake3_servil_hash_many_sse41(
            inputs.as_ptr() as *const *const u8,
            inputs.len(),
            N / BLOCK_LEN,
            key.as_ptr(),
            counter,
            increment_counter.yes(),
            flags,
            flags_start,
            flags_end,
            out.as_mut_ptr(),
        )
    }
}

/// Single non-generic x86-64 dispatcher for `2..=16`-block slot widths
/// (`128..=1024 B`): short-last batches (`last_len < BLOCK_LEN`, when not
/// already taken by AVX `multi_block_short_last`) run serially over their
/// logical length, whole-chunk (`16`-block) runs delegate to
/// `hash_many_rounded`, and whole-block `2..=15`-block runs dispatch through
/// `pair::hash_many_blocks`.
#[inline(never)]
pub(crate) fn hash_blocks_x86(
    messages: &[u8],
    blocks: usize,
    key: &CVWords,
    flags: u8,
    outputs: &mut [[u8; OUT_LEN]],
    platform: crate::platform::Platform,
    last_len: usize,
) {
    let count = outputs.len();
    assert!((2..=16).contains(&blocks) && (1..=BLOCK_LEN).contains(&last_len), "x86 multi-block shape out of range");
    let slot = blocks * BLOCK_LEN;
    assert_eq!(messages.len(), count.checked_mul(slot).expect("slot byte count fits"), "exact slot bytes for count outputs");
    if count == 0 {
        return;
    }
    if last_len != BLOCK_LEN {
        let len = slot - BLOCK_LEN + last_len;
        for (output, message) in outputs.iter_mut().zip(messages.chunks_exact(slot)) {
            *output = *crate::hash_serial_on(&message[..len], key, flags, platform).as_bytes();
        }
        return;
    }
    let flags_end = crate::CHUNK_END | crate::ROOT;
    if blocks == 16 {
        // Whole-chunk batches delegate to `hash_many_rounded` (preserving
        // Zen 5 `hash32` for 17..=31).
        let mut lanes = crate::stack_vec::StackVec::<&[u8; crate::CHUNK_LEN], { crate::many::TABLE }>::new();
        lanes.extend(messages.chunks_exact(crate::CHUNK_LEN).map(|m| m.try_into().expect("CHUNK_LEN chunk")));
        crate::hash_many_rounded(platform, &lanes, key, 0, crate::IncrementCounter::No, flags, crate::CHUNK_START, flags_end, outputs.as_flattened_mut());
        return;
    }
    if count == 3 {
        let last = messages[2 * slot..].as_ptr();
        let spare = [messages.as_ptr(), messages[slot..].as_ptr(), last, last];
        let mut scratch = [[0u8; OUT_LEN]; 4];
        // Sound: every pointer in `spare` reaches `blocks` whole blocks inside `messages`, and `platform` is detected.
        unsafe { crate::pair::hash_many_blocks(platform, &spare, blocks, key, 0, false, flags, crate::CHUNK_START, flags_end, scratch.as_flattened_mut()) };
        outputs.copy_from_slice(&scratch[..3]);
        return;
    }
    let mut ptrs = crate::stack_vec::StackVec::<*const u8, { crate::many::TABLE }>::new();
    ptrs.extend(messages.chunks_exact(slot).map(|m| m.as_ptr()));
    assert_eq!(ptrs.len(), count, "one message per output");
    let (full, target) = round_x86_blocks(platform, blocks, count);
    if target == 0 {
        // Sound: every pointer in `ptrs` reaches `blocks` whole blocks inside `messages`, and `platform` is a detected x86-64 ISA.
        unsafe {
            crate::pair::hash_many_blocks(platform, &ptrs, blocks, key, 0, false, flags, crate::CHUNK_START, flags_end, outputs.as_flattened_mut());
        }
        return;
    }
    // Sound: every pointer in `ptrs` reaches `blocks` whole blocks inside `messages`, and `platform` is a detected x86-64 ISA.
    unsafe { hash_blocks_rounded_x86(&ptrs, blocks, full, target, key, flags, flags_end, outputs, platform) };
}

/// Compute `(full_prefix_lanes, rounded_tail_width)` once for x86-64 multi-block
/// (`2..=15` blocks) batches across AVX-512, AVX2, and SSE4.1. Returns `(0, 0)`
/// when no tail rounding applies (exact widths, `1..=2` remainders, or
/// `128..192 B` `n = 9..=10` remainders on AVX-512 where `8 + 1/2` avoids
/// wide-kernel setup overhead).
pub(crate) fn round_x86_blocks(platform: crate::platform::Platform, blocks: usize, count: usize) -> (usize, usize) {
    let widest = match platform {
        #[cfg(target_arch = "x86_64")]
        crate::platform::Platform::AVX512 => 16,
        #[cfg(target_arch = "x86_64")]
        crate::platform::Platform::AVX2 => 8,
        crate::platform::Platform::SSE41 => 4,
    };
    let rem = count % widest;
    let target = match rem {
        3 => 4,
        5..=7 => 8,
        9..=10 if blocks >= 4 => 16,
        11..=15 => 16,
        _ => 0,
    };
    if target == 0 { (0, 0) } else { (count - rem, target) }
}

/// Out-of-line x86-64 rounded-tail batch helper (`2..=15` blocks, `target > 0`)
/// built over `pair::hash_many_blocks` with width-sized stack scratch (`4/8/16`),
/// entered only when rounding a tail so unrounded calls stay inline in
/// `hash_blocks_x86`.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
unsafe fn hash_blocks_rounded_x86(
    ptrs: &[*const u8],
    blocks: usize,
    full: usize,
    target: usize,
    key: &CVWords,
    flags: u8,
    flags_end: u8,
    outputs: &mut [[u8; OUT_LEN]],
    platform: crate::platform::Platform,
) {
    if full != 0 {
        // Sound: same pointer/platform guarantees on the leading `full` lanes.
        unsafe {
            crate::pair::hash_many_blocks(
                platform,
                &ptrs[..full],
                blocks,
                key,
                0,
                false,
                flags,
                crate::CHUNK_START,
                flags_end,
                outputs[..full].as_flattened_mut(),
            );
        }
    }
    let tail = &ptrs[full..];
    let tail_out = &mut outputs[full..];
    let last = *tail.last().expect("rounded tail has 3 or more lanes");
    let mut padded = crate::stack_vec::StackVec::<*const u8, 16>::new();
    padded.extend_from_slice(tail);
    padded.extend(core::iter::repeat_n(last, target - tail.len()));
    crate::ran(crate::Ran::RoundedUp);
    let mut run = |scratch: &mut [u8]| {
        // Sound: `padded` holds `target` valid `blocks`-block pointers and `scratch` holds `target` CVs.
        unsafe {
            crate::pair::hash_many_blocks(platform, &padded, blocks, key, 0, false, flags, crate::CHUNK_START, flags_end, scratch);
        }
        tail_out.as_flattened_mut().copy_from_slice(&scratch[..tail.len() * OUT_LEN]);
    };
    match target {
        4 => run(&mut [0u8; 4 * OUT_LEN]),
        8 => run(&mut [0u8; 8 * OUT_LEN]),
        16 => run(&mut [0u8; 16 * OUT_LEN]),
        _ => unreachable!("x86 multi-block target width is 4, 8, or 16"),
    }
}

pub mod ffi {
    unsafe extern "C" {
        pub fn blake3_servil_compress_in_place_sse41(cv: *mut u32, block: *const u8, block_len: u8, counter: u64, flags: u8);
        pub fn blake3_servil_compress_xof_sse41(cv: *const u32, block: *const u8, block_len: u8, counter: u64, flags: u8, out: *mut u8);
        pub fn blake3_servil_hash_many_sse41(
            inputs: *const *const u8,
            num_inputs: usize,
            blocks: usize,
            key: *const u32,
            counter: u64,
            increment_counter: bool,
            flags: u8,
            flags_start: u8,
            flags_end: u8,
            out: *mut u8,
        );
    }
}
