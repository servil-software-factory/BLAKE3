//! A cold file read past the page cache (Linux, O_DIRECT), several reads
//! at once, each thread hashing the piece it has just read.
//!
//! A drive answers several reads at once faster than one after another,
//! and a direct read skips the page cache's copy: on a Ryzen 9700X's NVMe
//! drives a cold 1 GiB file is read at 10 GB/s this way (16 threads; a
//! ring at the same depth reads no faster), at 7.6 GB/s through the cache
//! by a ring, at 5 GB/s through a map. The page cache does not hold the
//! file afterwards: a large set of files hashed once is the common case,
//! and is too large to cache (Zooko, November 14, 2026).

use blake3::throughput::{Cv, TreeHasher, level_of};

/// A cold file is read in pieces of this many bytes, each by one direct
/// read: 128 groups, so a piece starts on a group and its groups'
/// chaining values are a run of the file's.
pub(crate) const PIECE: usize = 2 << 20;

/// A file of at least this many bytes that the page cache does not hold
/// is read directly.
pub(crate) const DIRECT_MIN: u64 = 2 * PIECE as u64;

/// A buffer a direct read can fill: PIECE bytes, 4096-byte aligned.
pub(crate) struct Piece(*mut u8);

// Sound: the buffer is the Piece's own.
unsafe impl Send for Piece {}

impl Piece {
    const LAYOUT: std::alloc::Layout = match std::alloc::Layout::from_size_align(PIECE, 4096) {
        Ok(layout) => layout,
        Err(_) => panic!(),
    };
    pub(crate) fn new() -> Piece {
        // Sound: the layout's size is not zero. Not zeroed: zeroing would
        // touch every page (512 page faults, 0.7 ms on a Ryzen 7700X), and
        // only bytes that a read has filled are ever read.
        let at = unsafe { std::alloc::alloc(Self::LAYOUT) };
        if at.is_null() {
            std::alloc::handle_alloc_error(Self::LAYOUT);
        }
        Piece(at)
    }
}

impl Drop for Piece {
    fn drop(&mut self) {
        // Sound: allocated in `new` with this layout.
        unsafe { std::alloc::dealloc(self.0, Self::LAYOUT) }
    }
}

std::thread_local! {
    /// How many files this thread has set out to hash directly.
    #[cfg(test)]
    pub(crate) static HASHED: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
    /// The tests' seam: set, this thread takes every file for cold (a
    /// file just written is in the cache).
    #[cfg(test)]
    pub(crate) static TAKE_FOR_COLD: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

/// Whether `file`, of `len` bytes, is one to read directly: DIRECT_MIN
/// bytes or more, and its first byte not in the page cache (a read that
/// may not wait says; a file system without such reads, as a tmpfs, says
/// something else: its files are in memory).
pub(crate) fn is_for(file: &std::fs::File, len: u64) -> bool {
    use std::os::fd::AsRawFd;
    if len < DIRECT_MIN {
        return false;
    }
    let mut byte = 0u8;
    let iov = libc::iovec { iov_base: (&raw mut byte).cast(), iov_len: 1 };
    // Sound: one iovec over one writable byte.
    let got = unsafe { libc::preadv2(file.as_raw_fd(), &iov, 1, 0, libc::RWF_NOWAIT) };
    let cold = got == -1 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock;
    #[cfg(test)]
    let cold = cold || TAKE_FOR_COLD.with(|c| c.get()) || crate::linux_pipe::force_miss();
    cold
}

/// The open `file`, of `len` bytes, read in pieces of PIECE bytes by
/// `threads` threads, each piece handed, with its number, to `each` on the
/// thread that read it, beside its share of `outs`: `per` of them a piece
/// (the last piece's, what is left). None where the file cannot be opened
/// for direct reads, or a read fails or comes back short, or the file
/// does not end at `len` (a last piece with fewer bytes or more, or bytes
/// after a last whole piece): the caller then reads the file another way.
/// The reads say where the file ends, and `len` only where to look.
pub(crate) fn read<T: Send>(
    file: &std::fs::File,
    len: usize,
    threads: usize,
    mine: &mut Piece,
    outs: &mut [T],
    per: usize,
    each: impl Fn(usize, &[u8], &mut [T]) + Sync,
) -> Option<()> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    assert!(per > 0, "direct::read requires at least one output slot per piece");
    let expected_pieces = len.div_ceil(PIECE);
    assert!(expected_pieces > 0, "direct::read requires a non-empty length");
    assert_eq!(outs.chunks_mut(per).len(), expected_pieces, "direct::read output slice must cover every piece");
    let opened = file;
    // Open a new description of this very file, with independent flags and
    // offset. Reopening its old pathname could hash a different file after
    // a rename; changing flags on a dup would change the caller's too.
    // Without procfs, the caller uses its original descriptor another way.
    let direct = format!("/proc/self/fd/{}", file.as_raw_fd());
    let file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_DIRECT).open(direct).ok()?;
    let threads = threads.min(len.div_ceil(PIECE)).max(1);
    let pieces = std::sync::Mutex::new(outs.chunks_mut(per).enumerate());
    let failed = std::sync::atomic::AtomicBool::new(false);
    let work = |buffer: &mut Piece| {
        loop {
            let Some((i, outs)) = pieces.lock().unwrap().next() else { return };
            let want = PIECE.min(len - i * PIECE);
            // Sound: the buffer holds PIECE writable bytes. A piece
            // starts on a multiple of PIECE, so of the drive's blocks.
            let got = unsafe { libc::pread(file.as_raw_fd(), buffer.0.cast(), PIECE, (i * PIECE) as libc::off_t) };
            if got != want as isize {
                return failed.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            // Sound: `want` initialized bytes of the buffer.
            each(i, unsafe { core::slice::from_raw_parts(buffer.0, want) }, outs);
        }
    };
    if threads == 1 {
        work(mine);
    } else {
        std::thread::scope(|scope| {
            for _ in 1..threads {
                scope.spawn(|| work(&mut Piece::new()));
            }
            work(mine);
        });
    }
    if failed.into_inner() {
        return None;
    }
    // The reads above found `len` bytes; one read more says there are no
    // others. A last piece that is whole was asked for no byte more than
    // it has, so its read does not say the file ends there (BLAKE3 #424:
    // a file that had grown past a multiple of PIECE gave the hash of its
    // first `len` bytes, with no sign of the rest). A last piece that is
    // short was one read that stopped, and a read may stop before a
    // file's end. The read is of one byte at `len`, of the file as the
    // caller opened it: a direct read starts on a block, and `len` need
    // not.
    let mut byte = 0u8;
    loop {
        // Sound: one writable byte.
        let more = unsafe { libc::pread(opened.as_raw_fd(), (&raw mut byte).cast(), 1, len as libc::off_t) };
        if more == -1 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        return (more == 0).then_some(());
    }
}

/// The hash of the open `file`, of `len` bytes (PIECE or more), read
/// directly: each piece's subtree hashed by the thread that read it, on
/// its own core's kernels, then the pieces' parents a layer at a time.
/// None as [`read`] says.
pub(crate) fn hash(mode: blake3::Mode, file: &std::fs::File, len: usize, threads: usize, mine: &mut Piece) -> Option<blake3::Hash> {
    assert!(len > PIECE, "two pieces or more");
    #[cfg(test)]
    HASHED.with(|h| h.set(h.get() + 1));
    // Checked when this compiles: a piece is a whole subtree's size.
    const LEVEL: u32 = level_of(PIECE as u64);
    let tree = TreeHasher::new(mode);
    let pieces = len.div_ceil(PIECE);
    // A slot a piece, as `read` shares them out. The last piece is the
    // message's last subtree, a value of its own kind: its slot stays unset.
    let mut cvs = vec![Cv::unset(); pieces];
    let last = std::sync::OnceLock::new();
    {
        let tree = tree.clone().on_this_core();
        read(file, len, threads, mine, &mut cvs, 1, |i, bytes, cv| {
            if i + 1 == pieces {
                last.set(tree.last_cv((i << LEVEL) as u64, bytes)).expect("one thread reads the last piece");
            } else {
                cv[0] = tree.subtree_cv(LEVEL, i as u64, bytes);
            }
        })?;
    }
    let mut last = last.into_inner().expect("the last piece was read");
    cvs.truncate(pieces - 1);
    // A layer's whole values pair up. An odd one out is the last
    // subtree's left sibling.
    let mut next = vec![Cv::unset(); cvs.len() / 2];
    while cvs.len() > 1 {
        let pairs = cvs.len() / 2;
        tree.parent_cvs(&cvs[..2 * pairs], &mut next[..pairs]);
        if cvs.len() % 2 == 1 {
            last = tree.last_parent_cv(&cvs[2 * pairs], &last);
        }
        cvs.truncate(pairs);
        cvs.copy_from_slice(&next[..pairs]);
    }
    Some(tree.root_hash(&cvs[0], &last, len as u64))
}

#[cfg(test)]
mod test {
    use super::*;
    use std::os::unix::fs::OpenOptionsExt;

    /// A file read directly hashes as its bytes do: at two pieces, an odd
    /// count of pieces, a ragged last piece and a last piece of one byte;
    /// on one thread and on several; plain and keyed. A length that the
    /// file's pieces do not bear gives None. (Where the test's directory
    /// has no direct reads, as a tmpfs before Linux 6.6, there is nothing
    /// to compare.)
    #[test]
    fn a_file_read_directly_hashes_as_its_bytes_do() {
        blake3::initialize();
        let dir = crate::tempfile::tempdir().unwrap();
        let all: Vec<u8> = (0..7 * PIECE + 12345).map(|i: usize| (i.wrapping_mul(37) ^ (i >> 11)) as u8).collect();
        let key = [0x33u8; 32];
        let mut piece = Piece::new();
        for len in [PIECE + 1, 2 * PIECE, 2 * PIECE + 1, 3 * PIECE, 4 * PIECE - 1, 5 * PIECE + 4096, all.len()] {
            let path = dir.path().join(format!("file-{len}"));
            std::fs::write(&path, &all[..len]).unwrap();
            if std::fs::OpenOptions::new().read(true).custom_flags(libc::O_DIRECT).open(&path).is_err() {
                eprintln!("no direct reads in {}", dir.path().display());
                return;
            }
            let file = std::fs::File::open(&path).unwrap();
            for mode in [blake3::Mode::Plain, blake3::Mode::Keyed(&key)] {
                let want = blake3::convenience::hash_with(mode, &all[..len]);
                for threads in [1, 2, 5] {
                    assert_eq!(hash(mode, &file, len, threads, &mut piece), Some(want), "len={len} threads={threads}");
                }
                // Any other length, one that ends a piece too: the file's
                // end is where its reads say.
                for wrong in [len + 1, len - 1, len + PIECE, len - PIECE, len.next_multiple_of(PIECE) - PIECE] {
                    if wrong == len || wrong <= PIECE {
                        continue;
                    }
                    assert_eq!(hash(mode, &file, wrong, 2, &mut piece), None, "len={len} read as {wrong}");
                }
            }
        }
        // What is read directly: a cold file of DIRECT_MIN bytes or more.
        let path = dir.path().join(format!("file-{}", all.len()));
        let file = std::fs::File::open(&path).unwrap();
        assert!(!is_for(&file, all.len() as u64), "just written: the cache holds it");
        TAKE_FOR_COLD.with(|c| c.set(true));
        assert!(is_for(&file, DIRECT_MIN) && !is_for(&file, DIRECT_MIN - 1));
        TAKE_FOR_COLD.with(|c| c.set(false));
    }

    /// Replacing a pathname after open must not switch which file is
    /// hashed. Rename is the synchronization: no racing threads or sleeps.
    #[test]
    fn direct_reads_keep_the_open_file_when_its_path_is_replaced() {
        blake3::initialize();
        if crate::threads().get() == 1 {
            return;
        }
        let dir = crate::tempfile::tempdir().unwrap();
        let path = dir.path().join("input");
        let original = vec![0x35u8; DIRECT_MIN as usize];
        std::fs::write(&path, &original).unwrap();
        if std::fs::OpenOptions::new().read(true).custom_flags(libc::O_DIRECT).open(&path).is_err() {
            return;
        }
        let file = std::fs::File::open(&path).unwrap();
        std::fs::rename(&path, dir.path().join("moved")).unwrap();
        std::fs::write(&path, vec![0x97u8; original.len()]).unwrap();
        let args = crate::Args::for_test_plain(vec![path.clone()]);
        crate::linux_pipe::set_force_miss(true);
        let got = crate::hash_open_file(&args, file).unwrap();
        crate::linux_pipe::set_force_miss(false);
        assert_eq!(got, blake3::convenience::hash(&original), "hash the open file, not a replacement at its old name");
    }

    /// b3sum's one-file path reads a cold file of DIRECT_MIN or more
    /// directly, and no other: with every file taken for cold (the engine's
    /// seam), a file of DIRECT_MIN goes this way and one a byte shorter
    /// does not, and both hash as their bytes do.
    #[test]
    fn the_one_file_path_reads_a_cold_long_file_directly() {
        blake3::initialize();
        let dir = crate::tempfile::tempdir().unwrap();
        let all: Vec<u8> = (0..DIRECT_MIN as usize + 4096).map(|i: usize| (i.wrapping_mul(41) ^ (i >> 9)) as u8).collect();
        let probe = dir.path().join("probe");
        std::fs::write(&probe, &all[..4096]).unwrap();
        if std::fs::OpenOptions::new().read(true).custom_flags(libc::O_DIRECT).open(&probe).is_err() || crate::threads().get() == 1 {
            return;
        }
        crate::linux_pipe::set_force_miss(true);
        for (len, direct) in [(DIRECT_MIN as usize - 1, 0), (DIRECT_MIN as usize, 1), (all.len(), 1)] {
            let path = dir.path().join(format!("file-{len}"));
            std::fs::write(&path, &all[..len]).unwrap();
            let args = crate::Args::for_test_plain(vec![path.clone()]);
            HASHED.with(|h| h.set(0));
            assert_eq!(
                crate::hash_path_hash(&args, &path, &crate::file_end::StdinContext::current(), None).unwrap(),
                blake3::convenience::hash(&all[..len]),
                "len={len}"
            );
            assert_eq!(HASHED.with(|h| h.get()), direct, "len={len}");
        }
        crate::linux_pipe::set_force_miss(false);
    }
}
