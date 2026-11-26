//! Shared 24-byte descriptor, eligibility check, and startup fixture for the
//! independent counter-zero compression kernels (`ffi_output` on x86-64 and
//! `ffi_output_aarch64` on AArch64).

use crate::{BLOCK_LEN, CHUNK_END, CHUNK_START, Hasher, OUT_LEN, ROOT};

#[derive(Clone, Copy)]
#[repr(C)]
pub(crate) struct Input<'a> {
    pub(crate) block: &'a [u8; BLOCK_LEN],
    pub(crate) cv: &'a [u32; 8],
    pub(crate) len: u8,
    pub(crate) flags: u8,
}

const _: () = {
    assert!(core::mem::size_of::<Input<'static>>() == 24);
    assert!(core::mem::offset_of!(Input<'static>, block) == 0);
    assert!(core::mem::offset_of!(Input<'static>, cv) == 8);
    assert!(core::mem::offset_of!(Input<'static>, len) == 16);
    assert!(core::mem::offset_of!(Input<'static>, flags) == 17);
};

/// A whole message whose root is one compression away: still in its first
/// chunk, with no staged suffix or stacked subtree, its final block already
/// padded by `ChunkState` (a turn's messages, `update_each`); or a block or
/// less waiting in the stage, padded there (`Stage::first_block`). Other
/// states keep `Hasher`'s existing finalization path.
pub(crate) fn input(hasher: &Hasher) -> Option<Input<'_>> {
    let c = hasher.chunk_state();
    // Its first chunk, from the message's start: the counter counts every
    // chunk finished since chunk zero.
    if c.chunk_counter() != 0 {
        return None;
    }
    let staged = hasher.staged_len();
    if staged == 0 {
        let (block, cv, len, flags) = c.output_parts();
        return Some(Input { block, cv, len, flags: flags + ROOT });
    }
    const WHOLE_ROOT_FLAGS: u8 = CHUNK_START + CHUNK_END + ROOT;
    // Staged bytes behind a chunk state that holds none: the whole message.
    (staged <= BLOCK_LEN && c.count() == 0).then(|| Input {
        block: hasher.stage_first_block(),
        cv: c.cv(),
        len: u8::try_from(staged).expect("staged length <= 64"),
        flags: hasher.domain().flags() + WHOLE_ROOT_FLAGS,
    })
}

pub(crate) const SELF_TEST_EXPECTED: u64 = 0x153e0dfbeac2bb44;

pub(crate) fn self_test_shape(mut hash: impl FnMut(&[Input<'_>; 16], &mut [[u8; OUT_LEN]])) -> u64 {
    let mut lens: [u8; 16] = [0, 1, 31, 63, 64, 7, 32, 55, 3, 15, 48, 60, 2, 33, 62, 64];
    let flags: [u8; 16] = core::array::from_fn(|i| {
        let shape = [3u8, 2, 4][i % 3];
        if shape == 4 {
            lens[i] = 64;
        }
        shape + [0u8, 16, 32, 64][i % 4]
    });
    let cvs: [[u32; 8]; 16] = core::array::from_fn(|i| {
        let i = u32::try_from(i).unwrap();
        core::array::from_fn(|w| {
            let w = u32::try_from(w).unwrap();
            (0x1234abcdu32.wrapping_add(i * 0x9e37).wrapping_add(w * 0x1020304)).wrapping_mul(0x85ebca6b)
        })
    });
    let blocks: [[u8; 64]; 16] =
        core::array::from_fn(|i| core::array::from_fn(|j| if j < lens[i] as usize { crate::low_byte((i * 37 + j * 17) ^ 0x5a) } else { 0 }));
    let inputs = core::array::from_fn(|i| Input { block: &blocks[i], cv: &cvs[i], len: lens[i], flags: flags[i] + if (i / 4) % 2 == 0 { ROOT } else { 0 } });
    let mut out = [[0u8; OUT_LEN]; 16];
    hash(&inputs, &mut out);
    crate::self_test::fold(out.as_flattened())
}

pub(crate) fn check_self_test_value(answer: u64) -> bool {
    answer == SELF_TEST_EXPECTED
}

pub(crate) fn check_self_test(answer: u64) {
    if !check_self_test_value(answer) {
        crate::self_test::fail(format_args!("blake3-servil startup self-test failed: independent counter-zero compressions"));
    }
}
