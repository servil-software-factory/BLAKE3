//! `--outboard DIR`: beside each file's hash, its outboard in bao-tree's
//! form, for verified streaming of the file (iroh).
//!
//! The outboard holds the BLAKE3 tree above the file's chunk groups of
//! 16 KiB: each parent node as its two children's chaining values (64
//! bytes), the nodes in pre-order (a parent, its left subtree, its right
//! subtree), with no length before them. A file of one group or less has
//! no parent, and an empty outboard. It is what bao-tree's
//! `PreOrderMemOutboard::create(data, BlockSize::from_chunk_log(4))`
//! holds, byte for byte (the tests compare the two), written to
//! `DIR/<hash>.obao4`.
//!
//! It is built on the library's tree API alone (`throughput::TreeHasher`): each
//! group's chaining value, on as many threads as the file has work for,
//! then the parents.

use blake3::throughput::{Cv, TreeHasher, left_len};
use std::path::Path;

/// A chunk group: sixteen chunks, bao-tree's block size 4.
pub(crate) const GROUP: usize = 16 * blake3::CHUNK_LEN;

/// A thread takes at least this many groups (4 MiB): a file with less
/// work than that a thread is hashed by fewer, since the threads are
/// started for the file and a start costs what hashing a megabyte does.
const GROUPS_A_THREAD: usize = 256;

/// The hash of `data` in `mode`, and its outboard appended to `out`. `cvs`
/// is the caller's room for the groups' chaining values, kept from file to
/// file.
pub(crate) fn outboard(mode: blake3::Mode, data: &[u8], threads: usize, cvs: &mut Vec<Cv>, out: &mut Vec<u8>) -> blake3::Hash {
    if data.len() <= GROUP {
        return blake3::convenience::hash_with(mode, data);
    }
    let tree = TreeHasher::new(mode);
    group_cvs(&tree, data, threads, cvs);
    let (left, right) = children(&tree, data.len(), cvs, out);
    tree.root_hash(&left, &right)
}

/// Each group's chaining value into `cvs`, the groups shared out among
/// the threads.
fn group_cvs(tree: &TreeHasher, data: &[u8], threads: usize, cvs: &mut Vec<Cv>) {
    let groups = data.len().div_ceil(GROUP);
    cvs.clear();
    cvs.resize(groups, Cv::from_bytes([0; 32]));
    let threads = threads.min(groups / GROUPS_A_THREAD).max(1);
    let a_thread = groups.div_ceil(threads);
    // Many of these run at once, one a core: each on its own core's kernels.
    let tree = tree.clone().on_this_core();
    let hash = |first: usize, cvs: &mut [Cv]| {
        for (i, cv) in cvs.iter_mut().enumerate() {
            let group = first + i;
            let bytes = &data[group * GROUP..data.len().min((group + 1) * GROUP)];
            *cv = tree.subtree_cv((group * GROUP / blake3::CHUNK_LEN) as u64, bytes);
        }
    };
    if threads == 1 {
        hash(0, cvs);
    } else {
        std::thread::scope(|scope| {
            for (t, part) in cvs.chunks_mut(a_thread).enumerate() {
                let hash = &hash;
                scope.spawn(move || hash(t * a_thread, part));
            }
        });
    }
}

/// The two children of the subtree over `len` bytes (more than a group)
/// whose groups' chaining values are `cvs`; its node, then its subtrees'
/// nodes, appended to `out`.
fn children(tree: &TreeHasher, len: usize, cvs: &[Cv], out: &mut Vec<u8>) -> (Cv, Cv) {
    let at = out.len();
    out.extend_from_slice(&[0u8; 64]);
    // The left subtree is whole groups: a power of two of chunks, sixteen or more.
    let split = left_len(len as u64) as usize;
    let left = subtree(tree, split, &cvs[..split / GROUP], out);
    let right = subtree(tree, len - split, &cvs[split / GROUP..], out);
    out[at..at + 32].copy_from_slice(left.as_bytes());
    out[at + 32..at + 64].copy_from_slice(right.as_bytes());
    (left, right)
}

/// The chaining value of the subtree over `len` bytes, its nodes appended
/// to `out`: a group's own, or its two children's parent.
fn subtree(tree: &TreeHasher, len: usize, cvs: &[Cv], out: &mut Vec<u8>) -> Cv {
    if len <= GROUP {
        return cvs[0];
    }
    let (left, right) = children(tree, len, cvs, out);
    tree.parent_cv(&left, &right)
}

/// Where `hash`'s outboard goes in `dir`.
pub(crate) fn path_in(dir: &Path, hash: &blake3::Hash) -> std::path::PathBuf {
    dir.join(format!("{}.obao4", hash.to_hex()))
}

#[cfg(test)]
mod test {
    use super::*;

    fn painted(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i.wrapping_mul(31) ^ (i >> 9)) as u8).collect()
    }

    /// The outboard and the hash are bao-tree's own, byte for byte: at
    /// every length around a group, around powers of two of groups, and at
    /// ragged ones; on one thread and on several; and the keyed one too.
    #[test]
    fn the_outboard_is_bao_trees_byte_for_byte() {
        blake3::initialize();
        // Long enough for three threads' worth of groups, and ragged.
        let all = painted(3 * GROUPS_A_THREAD * GROUP + 5 * GROUP + 17);
        let key = [0x42u8; 32];
        let mut cvs = Vec::new();
        let block_size = bao_tree::BlockSize::from_chunk_log(4);
        let mut lens = vec![0usize, 1, 1024, GROUP - 1, GROUP, GROUP + 1, 2 * GROUP - 1, 2 * GROUP, 2 * GROUP + 1, 3 * GROUP, 50_000, 5 * GROUP + 1];
        lens.extend([63 * GROUP, 64 * GROUP, 64 * GROUP + 1, 65 * GROUP + 100, 1 << 20, (1 << 20) + 1]);
        lens.extend([2 * GROUPS_A_THREAD * GROUP - 1, 2 * GROUPS_A_THREAD * GROUP, 2 * GROUPS_A_THREAD * GROUP + 1, all.len()]);
        for len in lens {
            let data = &all[..len];
            for threads in [1, 3, 16] {
                let mut plain = Vec::new();
                let hash = outboard(blake3::Mode::Plain, data, threads, &mut cvs, &mut plain);
                let theirs = bao_tree::io::outboard::PreOrderMemOutboard::create(data, block_size);
                assert_eq!(hash.as_bytes(), theirs.root.as_bytes(), "the hash, len={len} threads={threads}");
                assert_eq!(hash, blake3::convenience::hash(data), "len={len}");
                assert_eq!(plain, theirs.data, "the outboard, len={len} threads={threads}");
                assert_eq!(plain.len(), 64 * len.div_ceil(GROUP).saturating_sub(1), "a node a group but one, len={len}");
                let mut keyed = Vec::new();
                let hash = outboard(blake3::Mode::Keyed(&key), data, threads, &mut cvs, &mut keyed);
                let theirs = bao_tree::io::outboard::PreOrderMemOutboard::create_keyed(data, block_size, &key);
                assert_eq!(hash.as_bytes(), theirs.root.as_bytes(), "the keyed hash, len={len} threads={threads}");
                assert_eq!(keyed, theirs.data, "the keyed outboard, len={len} threads={threads}");
            }
        }
    }
}
