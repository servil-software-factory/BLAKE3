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
        // The Rust hash_many implementations do bounds checking on the `out`
        // array, but the assembly implementations don't. Even though this is
        // an unsafe function, assert the bounds without multiplying `inputs.len()`.
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

/// Single non-generic x86-64 dispatcher for `2..=15`-block slot widths
/// (`128..=960 B`), eliminating 14 monomorphized copies of `hash_blocks` +
/// `hash_four_at_a_time` while preserving the 4-lane fast path for `count == 3`
/// and SSE4.1 / AVX2 / AVX-512 assembly dispatch via `pair::hash_many_blocks`.
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
    assert!((2..=15).contains(&blocks) && (1..=BLOCK_LEN).contains(&last_len), "x86 multi-block shape out of range");
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
    if count == 3 {
        let base = messages.as_ptr();
        // Sound: `messages.len() == 3 * slot` was asserted above.
        let (second, last) = unsafe { (base.add(slot), base.add(2 * slot)) };
        let spare = [base, second, last, last];
        let mut scratch = [[0u8; OUT_LEN]; 4];
        // Sound: every pointer in `spare` reaches `blocks` whole blocks inside `messages`, and `platform` is detected.
        unsafe {
            crate::pair::hash_many_blocks(
                platform,
                &spare,
                blocks,
                key,
                0,
                false,
                flags,
                crate::CHUNK_START,
                crate::CHUNK_END | crate::ROOT,
                scratch.as_flattened_mut(),
            )
        };
        outputs.copy_from_slice(&scratch[..3]);
        return;
    }
    let mut ptrs = crate::stack_vec::StackVec::<*const u8, { crate::many::TABLE }>::new();
    ptrs.extend(messages.chunks_exact(slot).map(|m| m.as_ptr()));
    assert_eq!(ptrs.len(), count, "one message per output");
    if crate::pair::takes_platform(platform) {
        // Sound: every pointer in `ptrs` reaches `blocks` whole blocks inside `messages`.
        unsafe { hash_blocks_rounded_x86(&ptrs, blocks, key, flags, outputs, platform) };
        return;
    }
    if count % 4 != 3 {
        // Sound: as above.
        unsafe {
            crate::pair::hash_many_blocks(
                platform,
                &ptrs,
                blocks,
                key,
                0,
                false,
                flags,
                crate::CHUNK_START,
                crate::CHUNK_END | crate::ROOT,
                outputs.as_flattened_mut(),
            )
        };
        return;
    }
    // `count == 3` returned above, so `count % 4 == 3` implies `count >= 7` and `fours >= 4`.
    let fours = count / 4 * 4;
    // Sound: as above.
    unsafe {
        crate::pair::hash_many_blocks(
            platform,
            &ptrs[..fours],
            blocks,
            key,
            0,
            false,
            flags,
            crate::CHUNK_START,
            crate::CHUNK_END | crate::ROOT,
            outputs[..fours].as_flattened_mut(),
        )
    };
    let last = ptrs[fours + 2];
    let spare = [ptrs[fours], ptrs[fours + 1], last, last];
    let mut scratch = [[0u8; OUT_LEN]; 4];
    // Sound: as above.
    unsafe {
        crate::pair::hash_many_blocks(
            platform,
            &spare,
            blocks,
            key,
            0,
            false,
            flags,
            crate::CHUNK_START,
            crate::CHUNK_END | crate::ROOT,
            scratch.as_flattened_mut(),
        )
    };
    outputs[fours..].copy_from_slice(&scratch[..3]);
}

/// Compute `(full_prefix_lanes, rounded_tail_width)` once for x86-64 AVX2 /
/// AVX-512 multi-block (`2..=15` blocks) batches. Returns `(0, 0)` when no
/// tail rounding applies (exact widths, `1..=2` remainders, or `128..192 B`
/// `n = 9..=10` remainders on AVX-512 where `8 + 1/2` avoids wide-kernel
/// setup overhead).
pub(crate) fn round_x86_blocks(platform: crate::platform::Platform, blocks: usize, count: usize) -> (usize, usize) {
    let widest = match platform {
        #[cfg(blake3_avx512_ffi)]
        crate::platform::Platform::AVX512 => 16,
        #[cfg(blake3_avx2_ffi)]
        crate::platform::Platform::AVX2 => 8,
        _ => return (0, 0),
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

/// Non-generic x86-64 AVX2/AVX-512 rounded multi-block batch helper (`2..=15`
/// blocks) built over `pair::hash_many_blocks` with width-sized stack scratch
/// (`4/8/16`), avoiding per-length monomorphization bloat in `hash_blocks`.
#[inline(never)]
unsafe fn hash_blocks_rounded_x86(
    ptrs: &[*const u8],
    blocks: usize,
    key: &CVWords,
    flags: u8,
    outputs: &mut [[u8; OUT_LEN]],
    platform: crate::platform::Platform,
) {
    let count = outputs.len();
    let (full, target) = round_x86_blocks(platform, blocks, count);
    if target == 0 {
        // Sound: caller verified AVX2/AVX-512 and every pointer reaches `blocks` blocks.
        unsafe {
            crate::pair::hash_many_blocks(
                platform,
                ptrs,
                blocks,
                key,
                0,
                false,
                flags,
                crate::CHUNK_START,
                crate::CHUNK_END | crate::ROOT,
                outputs.as_flattened_mut(),
            );
        }
        return;
    }
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
                crate::CHUNK_END | crate::ROOT,
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
            crate::pair::hash_many_blocks(platform, &padded, blocks, key, 0, false, flags, crate::CHUNK_START, crate::CHUNK_END | crate::ROOT, scratch);
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
