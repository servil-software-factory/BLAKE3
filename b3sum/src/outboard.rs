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
//! `DIR/<hash>.obao4`; an empty outboard is no file (Zooko, November
//! 14, 2026: creating a thousand empty files took longer than hashing
//! the thousand short files they were for).
//!
//! It is built on the library's tree API alone (`throughput::TreeHasher`):
//! each group's chaining value, on as many threads as the file has work
//! for, then the parents a layer at a time, side by side in the lanes.
//!
//! A long file that the page cache does not hold is read past the cache
//! (O_DIRECT, Linux), several reads at once: a set of large files hashed
//! once is the common case, and is too large to cache (Zooko, November
//! 14, 2026). The cache does not hold such a file afterwards.

use blake3::throughput::{Cv, TreeHasher};
use std::path::Path;

#[cfg(target_os = "linux")]
use crate::direct::PIECE;
#[cfg(target_os = "linux")]
pub(crate) use crate::direct::Piece;

/// A chunk group: sixteen chunks, bao-tree's block size 4.
pub(crate) const GROUP: usize = 16 * blake3::CHUNK_LEN;

/// A thread takes at least this many groups (4 MiB): a file with less
/// work than that a thread is hashed by fewer, since the threads are
/// started for the file and a start costs what hashing a megabyte does.
const GROUPS_A_THREAD: usize = 256;

/// A thread's rooms, kept from file to file: for a file's bytes, for its
/// tree's chaining values, and for a piece read directly.
pub(crate) struct Rooms {
    pub(crate) bytes: Vec<u8>,
    pub(crate) cvs: Vec<Cv>,
    /// Made by the first direct read: most runs have none.
    #[cfg(target_os = "linux")]
    pub(crate) piece: Option<Piece>,
}

impl Rooms {
    pub(crate) fn new() -> Rooms {
        Rooms {
            bytes: Vec::new(),
            cvs: Vec::new(),
            #[cfg(target_os = "linux")]
            piece: None,
        }
    }
}

/// The hash of `data` in `mode`, and its outboard appended to `out`. `cvs`
/// is the caller's room for the tree's chaining values, kept from file to
/// file.
pub(crate) fn outboard(mode: blake3::Mode, data: &[u8], threads: usize, cvs: &mut Vec<Cv>, out: &mut Vec<u8>) -> blake3::Hash {
    if data.len() <= GROUP {
        return blake3::convenience::hash_with(mode, data);
    }
    let done: Result<_, core::convert::Infallible> = build(mode, data.len(), cvs, out, |tree, cvs| {
        group_cvs(tree, data, threads, cvs);
        Ok(())
    });
    match done {
        Ok(hash) => hash,
    }
}

/// The hash and outboard of a message of `len` bytes, more than a group,
/// whose groups' chaining values `fill` writes.
fn build<E>(
    mode: blake3::Mode,
    len: usize,
    cvs: &mut Vec<Cv>,
    out: &mut Vec<u8>,
    fill: impl FnOnce(&TreeHasher, &mut [Cv]) -> Result<(), E>,
) -> Result<blake3::Hash, E> {
    assert!(len > GROUP);
    let tree = TreeHasher::new(mode);
    let groups = len.div_ceil(GROUP);
    // Every layer's values, one layer after another: the groups', then
    // each layer of parents, down to the root's two children. A layer
    // with an odd count passes its last value up as it is.
    // Under two values a group in all: each layer holds half the one
    // below, and one more where that one's count is odd.
    cvs.clear();
    cvs.resize(2 * groups + usize::BITS as usize, Cv::from_bytes([0; 32]));
    fill(&tree, &mut cvs[..groups])?;
    let mut layers: Vec<(usize, usize)> = vec![(0, groups)];
    while layers.last().unwrap().1 > 2 {
        let (at, count) = *layers.last().unwrap();
        let (children, parents) = cvs[at..].split_at_mut(count);
        let pairs = count / 2;
        tree.parent_cvs(&children[..2 * pairs], &mut parents[..pairs]);
        if count % 2 == 1 {
            parents[pairs] = children[count - 1];
        }
        layers.push((at + count, count.div_ceil(2)));
    }
    let (top, _) = *layers.last().unwrap();
    out.reserve(64 * (groups - 1));
    node(cvs, &layers, layers.len(), 0, out);
    Ok(tree.root_hash(&cvs[top], &cvs[top + 1]))
}

/// The hash and outboard of the open `file`, read past the page cache
/// (`direct::read`): None where that is not the way to read it or where a
/// read failed, and then `out` is as it was and the caller reads the file
/// as ever, and reports what fails then.
#[cfg(target_os = "linux")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn outboard_direct(
    mode: blake3::Mode,
    file: &std::fs::File,
    len: u64,
    threads: usize,
    mine: &mut Option<Piece>,
    cvs: &mut Vec<Cv>,
    out: &mut Vec<u8>,
) -> Option<blake3::Hash> {
    if !crate::direct::is_for(file, len) {
        return None;
    }
    direct(mode, file, usize::try_from(len).ok()?, threads, mine.get_or_insert_with(Piece::new), cvs, out)
}

/// [`outboard_direct`]'s reading and hashing, for a file of `len` bytes,
/// PIECE or more: each piece's groups' chaining values are a run of the
/// file's (a piece is 128 groups).
#[cfg(target_os = "linux")]
fn direct(
    mode: blake3::Mode,
    file: &std::fs::File,
    len: usize,
    threads: usize,
    mine: &mut Piece,
    cvs: &mut Vec<Cv>,
    out: &mut Vec<u8>,
) -> Option<blake3::Hash> {
    let before = out.len();
    let done = build(mode, len, cvs, out, |tree, cvs| {
        // Many of these run at once, one a core: each on its own core's kernels.
        let tree = tree.clone().on_this_core();
        crate::direct::read(file, len, threads, mine, cvs, PIECE / GROUP, |i, bytes, cvs| {
            for (g, cv) in cvs.iter_mut().enumerate() {
                let group = &bytes[g * GROUP..bytes.len().min((g + 1) * GROUP)];
                *cv = tree.subtree_cv(((i * PIECE + g * GROUP) / blake3::CHUNK_LEN) as u64, group);
            }
        })
        .ok_or(())
    });
    if done.is_err() {
        out.truncate(before);
    }
    done.ok()
}

/// Each group's chaining value into `cvs`, the groups shared out among
/// the threads.
fn group_cvs(tree: &TreeHasher, data: &[u8], threads: usize, cvs: &mut [Cv]) {
    let groups = cvs.len();
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

/// The node `index` of the layer under `layer` (`layers[layer - 1]`), and
/// the nodes below it, appended to `out` in pre-order: a parent as its two
/// children's values. A value its layer passed up as it was, and a
/// group's own, are no parent, and write nothing of their own. The root
/// is node 0 of the layer above the last.
fn node(cvs: &[Cv], layers: &[(usize, usize)], layer: usize, index: usize, out: &mut Vec<u8>) {
    if layer == 0 {
        return;
    }
    let (at, count) = layers[layer - 1];
    if 2 * index + 1 >= count {
        return node(cvs, layers, layer - 1, 2 * index, out);
    }
    out.extend_from_slice(cvs[at + 2 * index].as_bytes());
    out.extend_from_slice(cvs[at + 2 * index + 1].as_bytes());
    node(cvs, layers, layer - 1, 2 * index, out);
    node(cvs, layers, layer - 1, 2 * index + 1, out);
}

/// Where `hash`'s outboard goes in `dir`.
pub(crate) fn path_in(dir: &Path, hash: &blake3::Hash) -> std::path::PathBuf {
    dir.join(format!("{}.obao4", hash.to_hex()))
}

/// Write `hash`'s outboard, `nodes`, to its place in `dir`: whole, into a
/// new file of another name there, which then takes the place's name.
/// Nothing at that name is opened or written, so a link there is replaced
/// and what it points to is left alone, and a reader of an outboard
/// already there sees the old one whole or the new one whole (BLAKE3
/// #425: writing in place followed a link out of the directory, and
/// emptied an outboard that others were reading).
pub(crate) fn write_in(dir: &Path, hash: &blake3::Hash, nodes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let place = path_in(dir, hash);
    // A name no other writer has: this process's, and a count. A file a
    // stopped run left under it is not opened: the next name is tried.
    let (new, mut file) = loop {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let new = dir.join(format!(".{}.obao4.{}.{n}.new", hash.to_hex(), std::process::id()));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&new) {
            Ok(file) => break (new, file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    };
    let written = file.write_all(nodes).and_then(|()| {
        drop(file);
        std::fs::rename(&new, &place)
    });
    if written.is_err() {
        let _ = std::fs::remove_file(&new);
    }
    written
}

#[cfg(test)]
pub(crate) mod test {
    use super::*;

    #[cfg(target_os = "linux")]
    use crate::direct::{DIRECT_MIN, TAKE_FOR_COLD};

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

    /// A file read directly gives the hash and outboard its bytes in
    /// memory give: at lengths around a piece and ragged ones, on one
    /// thread and several, plain and keyed. A file whose pieces are not
    /// the ones its length when opened promised gives None and leaves
    /// `out` as it was. (Where the test's directory has no direct reads, as a
    /// tmpfs before Linux 6.6, there is nothing to compare.)
    #[test]
    #[cfg(target_os = "linux")]
    fn a_file_read_directly_gives_the_same_outboard() {
        use std::os::unix::fs::OpenOptionsExt;
        blake3::initialize();
        let dir = tempfile::tempdir().unwrap();
        let all = painted(5 * PIECE + 3 * GROUP + 17);
        let key = [0x42u8; 32];
        let (mut cvs, mut piece) = (Vec::new(), Piece::new());
        for len in [PIECE, PIECE + 1, 2 * PIECE, 2 * PIECE + 1, 3 * PIECE - GROUP + 5, 4 * PIECE + GROUP, all.len()] {
            let path = dir.path().join(format!("file-{len}"));
            std::fs::write(&path, &all[..len]).unwrap();
            if std::fs::OpenOptions::new().read(true).custom_flags(libc::O_DIRECT).open(&path).is_err() {
                eprintln!("no direct reads in {}", dir.path().display());
                return;
            }
            let file = std::fs::File::open(&path).unwrap();
            for mode in [blake3::Mode::Plain, blake3::Mode::Keyed(&key)] {
                let mut want = Vec::new();
                let hash = outboard(mode, &all[..len], 1, &mut cvs, &mut want);
                for threads in [1, 3] {
                    let mut got = vec![0xee; 5];
                    assert_eq!(direct(mode, &file, len, threads, &mut piece, &mut cvs, &mut got), Some(hash), "len={len} threads={threads}");
                    assert_eq!(got[..5], [0xee; 5]);
                    assert_eq!(got[5..], want, "len={len} threads={threads}");
                    for wrong in [len + 1, len - 1, len + PIECE, len - GROUP] {
                        // Bytes past a length that ends a piece are not read.
                        if wrong < len && wrong % PIECE == 0 {
                            continue;
                        }
                        let mut got = vec![0xee; 5];
                        assert_eq!(direct(mode, &file, wrong, threads, &mut piece, &mut cvs, &mut got), None, "len={len} read as {wrong}");
                        assert_eq!(got, [0xee; 5]);
                    }
                }
            }
        }
        // What is not read directly: a short file, and one the page cache holds.
        let path = dir.path().join(format!("file-{}", all.len()));
        assert!(all.len() as u64 >= DIRECT_MIN);
        let file = std::fs::File::open(&path).unwrap();
        let mut made = None;
        assert_eq!(
            outboard_direct(blake3::Mode::Plain, &file, all.len() as u64, 1, &mut made, &mut cvs, &mut Vec::new()),
            None,
            "just written: the cache holds it"
        );
        assert!(made.is_none(), "no piece is made for a file that is not read directly");
        // Taken for cold, it is read directly, and a file under DIRECT_MIN is not.
        TAKE_FOR_COLD.with(|c| c.set(true));
        let mut want = Vec::new();
        let hash = outboard(blake3::Mode::Plain, &all, 1, &mut cvs, &mut want);
        let mut got = Vec::new();
        assert_eq!(outboard_direct(blake3::Mode::Plain, &file, all.len() as u64, 2, &mut made, &mut cvs, &mut got), Some(hash));
        assert_eq!(got, want);
        assert!(made.is_some());
        let short = dir.path().join(format!("file-{}", PIECE + 1));
        let file = std::fs::File::open(&short).unwrap();
        assert!(((PIECE + 1) as u64) < DIRECT_MIN);
        assert_eq!(outboard_direct(blake3::Mode::Plain, &file, (PIECE + 1) as u64, 1, &mut made, &mut cvs, &mut got), None);
        assert_eq!(outboard_direct(blake3::Mode::Plain, &file, DIRECT_MIN - 1, 1, &mut made, &mut cvs, &mut got), None, "the length decides");
        TAKE_FOR_COLD.with(|c| c.set(false));
    }
}
