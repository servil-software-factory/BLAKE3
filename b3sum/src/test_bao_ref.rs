//! Minimal, independent reference implementation of BLAKE3 and `bao-tree`'s
//! pre-order outboard format (`BlockSize::from_chunk_log(4)`, 16 KiB chunk
//! groups), anchored in `outboard::test` against frozen golden vectors from
//! `bao-tree 0.17.0`.
//!
//! Uses only textbook scalar BLAKE3 compression and recursive left-heavy tree
//! construction so tests share no code with `blake3-servil` or `outboard.rs`.

const IV: [u32; 8] = [0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19];
const MSG_PERMUTATION: [usize; 16] = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];

const CHUNK_START: u32 = 1 << 0;
const CHUNK_END: u32 = 1 << 1;
const PARENT: u32 = 1 << 2;
const ROOT: u32 = 1 << 3;
const KEYED_HASH: u32 = 1 << 4;

const BLOCK_LEN: usize = 64;
const CHUNK_LEN: usize = 1024;
const GROUP_CHUNKS: usize = 16;
const GROUP_LEN: usize = GROUP_CHUNKS * CHUNK_LEN;

/// Output of the reference pre-order outboard builder: the 32-byte BLAKE3 root
/// hash and the concatenated 64-byte `(left_cv, right_cv)` parent nodes in
/// pre-order (empty for inputs of at most one 16 KiB chunk group).
pub(crate) struct PreOrderOutboard {
    pub(crate) root: [u8; 32],
    pub(crate) data: Vec<u8>,
}

impl PreOrderOutboard {
    pub(crate) fn root_hex(&self) -> String {
        to_hex(&self.root)
    }
}

pub(crate) fn create(input: &[u8]) -> PreOrderOutboard {
    build(input, IV, 0)
}

pub(crate) fn create_keyed(input: &[u8], key: &[u8; 32]) -> PreOrderOutboard {
    build(input, bytes_to_words(key), KEYED_HASH)
}

#[allow(dead_code)]
pub(crate) fn hash_plain(input: &[u8]) -> [u8; 32] {
    build_subtree_chunks(input, 0, IV, 0, true)
}

pub(crate) fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn build(input: &[u8], key_words: [u32; 8], base_flags: u32) -> PreOrderOutboard {
    if input.len() <= GROUP_LEN {
        return PreOrderOutboard { root: build_subtree_chunks(input, 0, key_words, base_flags, true), data: Vec::new() };
    }
    let n_groups = input.len().div_ceil(GROUP_LEN);
    let mut data = Vec::with_capacity(64 * (n_groups - 1));
    let root = build_groups(input, 0, n_groups, key_words, base_flags, true, &mut data);
    PreOrderOutboard { root, data }
}

/// Recursive pre-order Bao tree over `n_groups` 16 KiB chunk groups starting
/// at `start_group`. Splits at the largest power of two strictly less than
/// `n_groups` and writes `(left_cv, right_cv)` into `out` before recursing.
fn build_groups(input: &[u8], start_group: usize, n_groups: usize, key_words: [u32; 8], base_flags: u32, is_root: bool, out: &mut Vec<u8>) -> [u8; 32] {
    if n_groups == 1 {
        let start = start_group * GROUP_LEN;
        let end = (start + GROUP_LEN).min(input.len());
        return build_subtree_chunks(&input[start..end], (start_group * GROUP_CHUNKS) as u64, key_words, base_flags, is_root);
    }
    let left_groups = largest_power_of_two_less_than(n_groups);
    let slot = out.len();
    out.resize(slot + 64, 0);
    let left_cv = build_groups(input, start_group, left_groups, key_words, base_flags, false, out);
    let right_cv = build_groups(input, start_group + left_groups, n_groups - left_groups, key_words, base_flags, false, out);
    out[slot..slot + 32].copy_from_slice(&left_cv);
    out[slot + 32..slot + 64].copy_from_slice(&right_cv);
    parent_cv(&left_cv, &right_cv, key_words, base_flags, is_root)
}

/// Recursive BLAKE3 subtree chaining value (or root hash when `is_root`) over
/// `input` starting at chunk index `start_chunk`.
fn build_subtree_chunks(input: &[u8], start_chunk: u64, key_words: [u32; 8], base_flags: u32, is_root: bool) -> [u8; 32] {
    if input.len() <= CHUNK_LEN {
        return chunk_cv(input, start_chunk, key_words, base_flags, is_root);
    }
    let n_chunks = input.len().div_ceil(CHUNK_LEN);
    let left_chunks = largest_power_of_two_less_than(n_chunks);
    let split = left_chunks * CHUNK_LEN;
    let left_cv = build_subtree_chunks(&input[..split], start_chunk, key_words, base_flags, false);
    let right_cv = build_subtree_chunks(&input[split..], start_chunk + left_chunks as u64, key_words, base_flags, false);
    parent_cv(&left_cv, &right_cv, key_words, base_flags, is_root)
}

fn largest_power_of_two_less_than(n: usize) -> usize {
    debug_assert!(n > 1);
    1usize << (usize::BITS - 1 - (n - 1).leading_zeros())
}

fn chunk_cv(chunk: &[u8], chunk_counter: u64, key_words: [u32; 8], base_flags: u32, is_root: bool) -> [u8; 32] {
    let mut cv = key_words;
    let n_blocks = chunk.len().div_ceil(BLOCK_LEN).max(1);
    for b in 0..n_blocks {
        let start = b * BLOCK_LEN;
        let end = (start + BLOCK_LEN).min(chunk.len());
        let slice = &chunk[start..end];
        let mut block = [0u8; BLOCK_LEN];
        block[..slice.len()].copy_from_slice(slice);
        let mut flags = base_flags;
        if b == 0 {
            flags |= CHUNK_START;
        }
        if b + 1 == n_blocks {
            flags |= CHUNK_END;
            if is_root {
                flags |= ROOT;
            }
        }
        let block_len = u32::try_from(slice.len()).expect("block length <= 64");
        let words = compress(&cv, &block_to_words(&block), chunk_counter, block_len, flags);
        cv.copy_from_slice(&words[..8]);
    }
    words_to_bytes(&cv)
}

fn parent_cv(left: &[u8; 32], right: &[u8; 32], key_words: [u32; 8], base_flags: u32, is_root: bool) -> [u8; 32] {
    let mut block = [0u8; BLOCK_LEN];
    block[..32].copy_from_slice(left);
    block[32..].copy_from_slice(right);
    let flags = base_flags | PARENT | if is_root { ROOT } else { 0 };
    let words = compress(&key_words, &block_to_words(&block), 0, u32::try_from(BLOCK_LEN).expect("block length <= 64"), flags);
    let mut out = [0u32; 8];
    out.copy_from_slice(&words[..8]);
    words_to_bytes(&out)
}

fn g(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize, mx: u32, my: u32) {
    state[a] = state[a].wrapping_add(state[b]).wrapping_add(mx);
    state[d] = (state[d] ^ state[a]).rotate_right(16);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_right(12);
    state[a] = state[a].wrapping_add(state[b]).wrapping_add(my);
    state[d] = (state[d] ^ state[a]).rotate_right(8);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_right(7);
}

fn round(state: &mut [u32; 16], m: &[u32; 16]) {
    g(state, 0, 4, 8, 12, m[0], m[1]);
    g(state, 1, 5, 9, 13, m[2], m[3]);
    g(state, 2, 6, 10, 14, m[4], m[5]);
    g(state, 3, 7, 11, 15, m[6], m[7]);
    g(state, 0, 5, 10, 15, m[8], m[9]);
    g(state, 1, 6, 11, 12, m[10], m[11]);
    g(state, 2, 7, 8, 13, m[12], m[13]);
    g(state, 3, 4, 9, 14, m[14], m[15]);
}

fn compress(cv: &[u32; 8], block_words: &[u32; 16], counter: u64, block_len: u32, flags: u32) -> [u32; 16] {
    let counter_bytes = counter.to_le_bytes();
    let counter_low = u32::from_le_bytes([counter_bytes[0], counter_bytes[1], counter_bytes[2], counter_bytes[3]]);
    let counter_high = u32::from_le_bytes([counter_bytes[4], counter_bytes[5], counter_bytes[6], counter_bytes[7]]);
    let mut state = [cv[0], cv[1], cv[2], cv[3], cv[4], cv[5], cv[6], cv[7], IV[0], IV[1], IV[2], IV[3], counter_low, counter_high, block_len, flags];
    let mut m = *block_words;
    for r in 0..7 {
        round(&mut state, &m);
        if r < 6 {
            let prev = m;
            for i in 0..16 {
                m[i] = prev[MSG_PERMUTATION[i]];
            }
        }
    }
    for i in 0..8 {
        state[i] ^= state[i + 8];
        state[i + 8] ^= cv[i];
    }
    state
}

fn bytes_to_words(bytes: &[u8; 32]) -> [u32; 8] {
    let mut out = [0u32; 8];
    for (i, chunk) in bytes.chunks_exact(4).enumerate() {
        out[i] = u32::from_le_bytes(chunk.try_into().unwrap());
    }
    out
}

fn block_to_words(block: &[u8; 64]) -> [u32; 16] {
    let mut out = [0u32; 16];
    for (i, chunk) in block.chunks_exact(4).enumerate() {
        out[i] = u32::from_le_bytes(chunk.try_into().unwrap());
    }
    out
}

fn words_to_bytes(words: &[u32; 8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, &w) in words.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&w.to_le_bytes());
    }
    out
}
