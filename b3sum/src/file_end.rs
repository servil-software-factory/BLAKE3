//! Unified file-end, descriptor-identity, and bounded-read state machines for `b3sum` (#468).
//!
//! Ordinary reading paths in `b3sum` share three invariants enforced here
//! (memory-mapped paths hash the mapped extent, and `--outboard`'s non-mapped
//! fallback still buffers the whole input):
//! 1. **Only a waiting read returning 0 ends a file.** A non-blocking cache
//!    probe (`RWF_NOWAIT`) returning `0` or an error (such as `EAGAIN` or
//!    sandbox `EPERM`) never ends a file; a real blocking read error (`EIO`,
//!    etc.) is propagated rather than swallowed as a cache miss.
//! 2. **Initialized prefix length lives in `Vec::len()`.** Buffers grow safely
//!    without callers carrying an unchecked raw offset, and small-hint regular
//!    files promote to incremental streaming once they exceed [`SMALL_MEM_CAP`]
//!    so a virtual or growing file reporting a tiny size cannot exhaust memory.
//! 3. **Descriptor identity is validated by the OS (`fstat` / `stat`), never
//!    by lexical path equality alone.** Relative paths under `/dev` and `/proc`
//!    (including from `/`) are nominated lexically and confirmed via `stat`;
//!    only genuine consumable streams (FIFOs/pipes, Unix sockets, terminals —
//!    not block devices or cloning char devices) reuse fd 0 directly; on macOS,
//!    verified `fd(4)` regular-file aliases (`/dev/stdin`, `/dev/fd/0`) hash
//!    fd 0 from byte 0 via positioned `pread` or `mmap` without moving fd 0's
//!    shared offset.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

#[cfg(unix)]
use std::os::fd::{AsRawFd, RawFd};

/// Ceiling for reading a small-hinted file into a single contiguous `Vec<u8>`.
/// Files whose reads pass this bound promote to incremental streaming hashing.
pub(crate) const SMALL_MEM_CAP: usize = 4 << 20;

/// Device and inode pair identifying an open Unix file description's target.
#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StreamId {
    pub(crate) dev: u64,
    pub(crate) ino: u64,
}

/// File semantics relevant to descriptor aliasing and offset ownership.
#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileKind {
    /// Regular file (`S_IFREG`).
    Regular,
    /// Consumable non-seekable stream: pipe/FIFO (`S_IFIFO`), socket
    /// (`S_IFSOCK`), or a terminal (`S_IFCHR` where `isatty(fd) == 1`).
    /// Excludes seekable block devices (`S_IFBLK`) and cloning devices such as
    /// `/dev/ptmx`, whose named open yields an independent description.
    Stream,
    /// Any other special file (directories, block devices, non-tty char devices).
    Other,
}

/// Metadata queried directly from an open file descriptor via `fstat` (needs
/// no spare file descriptor).
#[cfg(unix)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct FdMeta {
    pub(crate) id: StreamId,
    pub(crate) kind: FileKind,
    pub(crate) mode_type: libc::mode_t,
    pub(crate) len: u64,
}

#[cfg(unix)]
impl FdMeta {
    #[inline]
    pub(crate) fn is_file(self) -> bool {
        self.kind == FileKind::Regular
    }
}

/// Query `(dev, ino, kind, len)` directly on `fd` using `fstat`, without
/// duplicating the descriptor.
#[cfg(unix)]
pub(crate) fn fstat_fd(fd: RawFd) -> io::Result<FdMeta> {
    let mut st = core::mem::MaybeUninit::<libc::stat>::zeroed();
    // Sound: `st` is valid writable storage for `libc::stat`.
    let rc = unsafe { libc::fstat(fd, st.as_mut_ptr()) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    // Sound: `fstat` returned 0 and initialized `st`.
    let st = unsafe { st.assume_init() };
    let mode_type = st.st_mode & libc::S_IFMT;
    let kind = match mode_type {
        libc::S_IFREG => FileKind::Regular,
        libc::S_IFIFO | libc::S_IFSOCK => FileKind::Stream,
        // Sound: `isatty` queries an open fd without touching memory.
        libc::S_IFCHR if unsafe { libc::isatty(fd) } == 1 => FileKind::Stream,
        _ => FileKind::Other,
    };
    Ok(FdMeta { id: StreamId { dev: st.st_dev as u64, ino: st.st_ino as u64 }, kind, mode_type, len: st.st_size.max(0) as u64 })
}

/// Snapshot of process standard input and working directory used to classify
/// operands and checkfile entries identically without repeated `getcwd` calls.
#[derive(Clone, Debug)]
pub(crate) struct StdinContext {
    #[cfg(unix)]
    pub(crate) stdin: Option<FdMeta>,
    #[cfg(unix)]
    cwd: Option<PathBuf>,
    #[cfg(unix)]
    cwd_in_dev_or_proc: bool,
    #[cfg(unix)]
    cwd_is_root: bool,
}

/// How an operand or checkfile entry relates to standard input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InputRole {
    /// Literal `-`.
    StdinDash,
    /// Another pathname under `/dev` or `/proc` (absolute or relative) whose
    /// OS `stat` confirms it names the same consumable stdin stream (pipe,
    /// FIFO, Unix socket, or terminal). Must read from fd 0 directly rather
    /// than reopening the pathname.
    #[cfg(unix)]
    StdinStreamAlias,
    /// macOS `/dev/stdin` or `/dev/fd/0` when stdin is a regular file and OS
    /// lookup confirms the path resolves to Darwin's `fd(4)` entry for fd 0
    /// and matches fd 0's `(dev, ino)`: hashed from offset 0 via positioned
    /// reads (`pread`) or `mmap` without moving fd 0's cursor.
    #[cfg(target_os = "macos")]
    DarwinRegularFd0,
    /// Ordinary pathname opened normally.
    Path,
}

impl StdinContext {
    pub(crate) fn current() -> Self {
        #[cfg(unix)]
        {
            let stdin = fstat_fd(0).ok();
            let cwd = std::env::current_dir().ok();
            let cwd_in_dev_or_proc = cwd.as_ref().is_some_and(|p| p.starts_with("/dev") || p.starts_with("/proc"));
            let cwd_is_root = cwd.as_deref() == Some(Path::new("/"));
            Self { stdin, cwd, cwd_in_dev_or_proc, cwd_is_root }
        }
        #[cfg(not(unix))]
        {
            Self {}
        }
    }

    /// The `(dev, ino)` of stdin if and only if stdin is a consumable stream.
    #[cfg(unix)]
    pub(crate) fn stdin_stream_id(&self) -> Option<StreamId> {
        self.stdin.filter(|m| m.kind == FileKind::Stream).map(|m| m.id)
    }

    /// Fast lexical nomination: whether `path` could plausibly land under
    /// `/dev` or `/proc` (including relative names from `/`, `/dev`, `/proc`,
    /// or paths with `..`). This ONLY filters which operands get an OS `stat`
    /// check; it NEVER overrides the OS's resolution.
    #[cfg(unix)]
    fn might_be_dev_or_proc(&self, path: &Path) -> bool {
        let has_parent_dir = path.components().any(|c| matches!(c, Component::ParentDir));
        let needs_check = if path.is_absolute() {
            path.starts_with("/dev") || path.starts_with("/proc") || has_parent_dir
        } else {
            self.cwd_in_dev_or_proc || has_parent_dir || (self.cwd_is_root && (path.starts_with("dev") || path.starts_with("proc")))
        };
        if !needs_check {
            return false;
        }
        normalize_lexical(path, self.cwd.as_deref()).is_some_and(|norm| norm.starts_with("/dev") || norm.starts_with("/proc"))
    }

    /// Classify `path` consistently across hashing operands, `--outboard`,
    /// XOF mode, and `--check` entries. Every alias decision is verified by
    /// the OS against the original operand (never by lexical text alone).
    pub(crate) fn classify(&self, path: &Path) -> InputRole {
        if crate::is_stdin(path) {
            return InputRole::StdinDash;
        }
        #[cfg(unix)]
        if let Some(stdin) = self.stdin
            && self.might_be_dev_or_proc(path)
        {
            match stdin.kind {
                FileKind::Stream => {
                    if path_matches_fd_meta(path, stdin) {
                        return InputRole::StdinStreamAlias;
                    }
                }
                FileKind::Regular =>
                {
                    #[cfg(target_os = "macos")]
                    if is_verified_darwin_fd0(path, stdin) {
                        return InputRole::DarwinRegularFd0;
                    }
                }
                FileKind::Other => {}
            }
        }
        InputRole::Path
    }
}

/// Lexically normalize `path` against `cwd` without touching the filesystem.
/// Used only to decide whether to inspect a path with OS metadata calls.
#[cfg(unix)]
pub(crate) fn normalize_lexical(path: &Path, cwd: Option<&Path>) -> Option<PathBuf> {
    let mut out = if path.is_absolute() { PathBuf::new() } else { cwd?.to_path_buf() };
    for comp in path.components() {
        match comp {
            Component::Prefix(_) => {}
            Component::RootDir => {
                out.clear();
                out.push("/");
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if out.parent().is_some() {
                    out.pop();
                }
            }
            Component::Normal(part) => out.push(part),
        }
    }
    Some(out)
}

/// Verify via OS resolution that `path` actually resolves to Darwin's `fd(4)`
/// descriptor 0 entry (`/dev/stdin` -> `fd/0` or `/dev/fd/0`) AND that
/// `stat(path)` confirms it names the same regular file as fd 0 (preserving
/// trailing-slash errors, directory-symlink `..` semantics, and lookup errors).
#[cfg(target_os = "macos")]
fn is_verified_darwin_fd0(path: &Path, stdin: FdMeta) -> bool {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    // Reject trailing slash (regular file is not a directory) and verify `stat` matches fd 0.
    if path.as_os_str().as_bytes().ends_with(b"/") {
        return false;
    }
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() || (StreamId { dev: meta.dev(), ino: meta.ino() }) != stdin.id {
        return false;
    }
    // Resolve the parent directory via the OS kernel (`canonicalize`) so
    // symlinks and `..` follow kernel rules rather than lexical cancellation.
    let Some(file_name) = path.file_name() else {
        return false;
    };
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let Ok(real_parent) = std::fs::canonicalize(parent) else {
        return false;
    };
    if real_parent == Path::new("/dev/fd") && file_name == "0" {
        return true;
    }
    if real_parent == Path::new("/dev") {
        let entry = real_parent.join(file_name);
        if let Ok(target) = std::fs::read_link(&entry) {
            return target == Path::new("fd/0") || target == Path::new("/dev/fd/0");
        }
    }
    false
}

/// Whether `path` currently resolves via OS `stat` to the exact same `(dev, ino)`
/// AND file type (`S_IFMT`) as `target` (preserving symlinks, `..`, and trailing
/// directory requirements).
#[cfg(unix)]
pub(crate) fn path_matches_fd_meta(path: &Path, target: FdMeta) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).is_ok_and(|meta| {
        let mode_type = meta.mode() & u32::from(libc::S_IFMT);
        mode_type == u32::from(target.mode_type) && StreamId { dev: meta.dev(), ino: meta.ino() } == target.id
    })
}

/// Whether `path` currently resolves via OS `stat` to a stream matching `stream`.
#[cfg(unix)]
pub(crate) fn path_matches_stream(path: &Path, stream: StreamId) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).is_ok_and(|meta| {
        let mode_type = meta.mode() & u32::from(libc::S_IFMT);
        (mode_type == u32::from(libc::S_IFIFO) || mode_type == u32::from(libc::S_IFSOCK) || mode_type == u32::from(libc::S_IFCHR))
            && StreamId { dev: meta.dev(), ino: meta.ino() } == stream
    })
}

/// Open `path` and immediately check its `fstat` identity against an optional
/// forbidden consumable stream (e.g., the checkfile's own pipe/FIFO). Checks
/// `stat(path)` first so rejecting the checkfile's own FIFO does not block
/// waiting for another writer, then re-verifies `fstat(fd)` after open to
/// close pathname-swap TOCTOU races.
#[cfg(unix)]
pub(crate) fn open_checked(path: &Path, forbidden: Option<StreamId>) -> anyhow::Result<(File, FdMeta)> {
    if let Some(forbid) = forbidden
        && path_matches_stream(path, forbid)
    {
        anyhow::bail!("this checkfile is a stream, and cannot also be a file to check");
    }
    let file = File::open(path)?;
    let meta = fstat_fd(file.as_raw_fd())?;
    if let Some(forbid) = forbidden
        && meta.kind == FileKind::Stream
        && meta.id == forbid
    {
        anyhow::bail!("this checkfile is a stream, and cannot also be a file to check");
    }
    Ok((file, meta))
}

/// Outcome of hashing an open descriptor, carrying exact byte count and
/// whether a cursor-consuming read path ran.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HashRead {
    pub(crate) hash: blake3::Hash,
    pub(crate) bytes_hashed: u64,
    pub(crate) cursor_moved: bool,
}

/// Result of a Linux page-cache probe followed by waiting positional reads.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CacheRead {
    /// Waiting read returned 0: EOF reached with `usize` bytes total in buffer.
    Eof(usize),
    /// Optional non-blocking probe unavailable (`EAGAIN`/`EPERM`/`EOPNOTSUPP`/...)
    /// or buffer full before EOF: fall back to waiting reads on the open fd.
    NeedMore(usize),
    /// Descriptor does not support positional reads (`ESPIPE`); fall back to sequential `read`.
    NonSeekable(usize),
}

/// Run the single rule for cached positional reads:
/// 1. The first read asks the page cache alone (`RWF_NOWAIT`).
/// 2. A non-blocking 0-byte reply does NOT mean EOF (Linux 5.9/5.10 bug): it
///    switches to waiting reads (`flags == 0`).
/// 3. Any non-`EINTR`, non-`ESPIPE` failure of the optional `RWF_NOWAIT` probe
///    (including `EAGAIN`, `EOPNOTSUPP`, `ENOSYS`, or sandbox `EPERM`) returns
///    `NeedMore` so the caller performs the real waiting read on the open fd.
/// 4. On waiting reads (`flags == 0`), `ESPIPE` returns `NonSeekable`, while
///    **any real blocking error (`EIO`, etc.) is returned as `Err`, never
///    swallowed as a cache miss.**
/// 5. Only a waiting read (`flags == 0`) returning `0` returns `CacheRead::Eof`.
#[cfg(target_os = "linux")]
#[inline]
pub(crate) fn cached_then_plain(dst_cap: usize, mut at: usize, mut read: impl FnMut(usize, libc::c_int) -> isize) -> io::Result<CacheRead> {
    let mut flags = libc::RWF_NOWAIT;
    loop {
        if at == dst_cap {
            return Ok(CacheRead::NeedMore(at));
        }
        let rc = read(at, flags);
        if rc < 0 {
            let err = io::Error::last_os_error();
            let raw = err.raw_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if raw == Some(libc::ESPIPE) {
                return Ok(CacheRead::NonSeekable(at));
            }
            if flags != 0 {
                return Ok(CacheRead::NeedMore(at));
            }
            return Err(err);
        }
        if rc == 0 && flags == 0 {
            return Ok(CacheRead::Eof(at));
        }
        at += rc as usize;
        flags = 0;
    }
}

/// Read into `buf` (preserving `buf[..buf.len()]` as the initialized prefix)
/// sequentially up to `cap`. Returns `Ok(true)` if EOF was reached within
/// `cap`, or `Ok(false)` when `buf.len() >= cap` and the caller must promote
/// to incremental streaming hashing.
pub(crate) fn read_sequential_capped(file: &mut File, buf: &mut Vec<u8>, cap: usize) -> io::Result<bool> {
    loop {
        if buf.len() >= cap {
            return Ok(false);
        }
        if buf.len() == buf.capacity() {
            let grow = buf.len().max(4096).min(cap - buf.len());
            buf.reserve(grow);
        }
        let avail = (buf.capacity() - buf.len()).min(cap - buf.len());
        #[cfg(unix)]
        let rc = {
            // Sound: `buf.spare_capacity_mut()[..avail]` is valid writable
            // storage inside `buf`'s allocation; the kernel writes at most `avail` bytes.
            unsafe { libc::read(file.as_raw_fd(), buf.as_mut_ptr().add(buf.len()).cast(), avail) }
        };
        #[cfg(not(unix))]
        let rc = {
            let old = buf.len();
            buf.resize(old + avail, 0);
            match file.read(&mut buf[old..]) {
                Ok(n) => {
                    buf.truncate(old + n);
                    if n == 0 {
                        return Ok(true);
                    }
                    continue;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                    buf.truncate(old);
                    continue;
                }
                Err(e) => {
                    buf.truncate(old);
                    return Err(e);
                }
            }
        };
        if rc == 0 {
            return Ok(true);
        }
        if rc < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        let n = rc as usize;
        assert!(n <= avail, "kernel read did not exceed requested length");
        // Sound: `libc::read` initialized `n` bytes starting at `buf.len()`.
        unsafe { buf.set_len(buf.len() + n) };
    }
}

/// Read the rest of `file` starting at `buf.len()` using `pread` up to `cap`,
/// falling back to sequential `read` on `ESPIPE` when `buf.is_empty()`.
/// Returns `Ok((reached_eof, used_sequential))`.
#[cfg(target_os = "linux")]
pub(crate) fn pread_rest_capped(file: &mut File, buf: &mut Vec<u8>, cap: usize) -> io::Result<(bool, bool)> {
    loop {
        if buf.len() >= cap {
            return Ok((false, false));
        }
        if buf.len() == buf.capacity() {
            let grow = buf.len().max(4096).min(cap - buf.len());
            buf.reserve(grow);
        }
        let avail = (buf.capacity() - buf.len()).min(cap - buf.len());
        let off = buf.len() as libc::off_t;
        // Sound: `avail` writable bytes sit at `buf.as_mut_ptr().add(buf.len())`.
        let rc = unsafe { libc::pread(file.as_raw_fd(), buf.as_mut_ptr().add(buf.len()).cast(), avail, off) };
        if rc == 0 {
            return Ok((true, false));
        }
        if rc < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if e.raw_os_error() == Some(libc::ESPIPE) && buf.is_empty() {
                let eof = read_sequential_capped(file, buf, cap)?;
                return Ok((eof, true));
            }
            return Err(e);
        }
        let n = rc as usize;
        assert!(n <= avail, "kernel pread did not exceed requested length");
        // Sound: `libc::pread` initialized `n` bytes starting at `buf.len()`.
        unsafe { buf.set_len(buf.len() + n) };
    }
}

/// A `Read` adapter that reads sequentially from a fixed starting offset on a
/// Unix `RawFd` using `pread`, never modifying the underlying open file
/// description's cursor. Used both for streaming promotion after positional
/// reads and for macOS `/dev/stdin` / `/dev/fd/0` on regular files.
#[cfg(unix)]
pub(crate) struct PreadReader {
    fd: RawFd,
    offset: u64,
}

#[cfg(unix)]
impl PreadReader {
    pub(crate) fn new(fd: RawFd, offset: u64) -> Self {
        Self { fd, offset }
    }
}

#[cfg(unix)]
impl Read for PreadReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            // Sound: `buf` is a live exclusive mutable slice for the kernel to fill.
            let rc = unsafe { libc::pread(self.fd, buf.as_mut_ptr().cast(), buf.len(), self.offset as libc::off_t) };
            if rc < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            let n = rc as usize;
            self.offset += n as u64;
            return Ok(n);
        }
    }
}

/// Copy all bytes of `fd` from offset 0 into `out` via `pread` (never advancing
/// `fd`'s shared file offset), for `--outboard` on macOS `/dev/stdin` / `/dev/fd/0`.
#[cfg(target_os = "macos")]
pub(crate) fn pread_all_into(fd: RawFd, out: &mut Vec<u8>) -> io::Result<()> {
    let mut reader = PreadReader::new(fd, 0);
    reader.read_to_end(out)?;
    Ok(())
}

/// Hash `prefix` followed by the remainder of `file` starting at `prefix.len()`
/// via positioned `pread` (or sequentially if `sequential` is true), keeping
/// memory bounded regardless of reported size.
#[cfg(unix)]
pub(crate) fn finish_hash_streaming_from_prefix(mode: blake3::Mode<'_>, prefix: &[u8], file: &mut File, sequential: bool) -> io::Result<HashRead> {
    let mut hasher = blake3::convenience::Hasher::new(mode, None);
    crate::update(&mut hasher, prefix);
    if sequential {
        let extra = crate::update_from_counted(&mut hasher, file)?;
        Ok(HashRead { hash: hasher.finalize(), bytes_hashed: prefix.len() as u64 + extra, cursor_moved: true })
    } else {
        let mut reader = PreadReader::new(file.as_raw_fd(), prefix.len() as u64);
        let extra = crate::update_from_counted(&mut hasher, &mut reader)?;
        Ok(HashRead { hash: hasher.finalize(), bytes_hashed: prefix.len() as u64 + extra, cursor_moved: false })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn normalize_lexical_resolves_relative_and_dotdot_dev_paths() {
        #[cfg(unix)]
        {
            assert_eq!(normalize_lexical(Path::new("stdin"), Some(Path::new("/dev"))), Some(PathBuf::from("/dev/stdin")));
            assert_eq!(normalize_lexical(Path::new("./stdin"), Some(Path::new("/dev"))), Some(PathBuf::from("/dev/stdin")));
            assert_eq!(normalize_lexical(Path::new("0"), Some(Path::new("/dev/fd"))), Some(PathBuf::from("/dev/fd/0")));
            assert_eq!(normalize_lexical(Path::new("dev/stdin"), Some(Path::new("/"))), Some(PathBuf::from("/dev/stdin")));
            assert_eq!(normalize_lexical(Path::new("proc/self/fd/0"), Some(Path::new("/"))), Some(PathBuf::from("/proc/self/fd/0")));
            assert_eq!(normalize_lexical(Path::new("../dev/stdin"), Some(Path::new("/tmp"))), Some(PathBuf::from("/dev/stdin")));
            assert_eq!(normalize_lexical(Path::new("/tmp/../dev/./fd/0"), Some(Path::new("/"))), Some(PathBuf::from("/dev/fd/0")));
            assert_eq!(normalize_lexical(Path::new("0"), Some(Path::new("/proc/self/fd"))), Some(PathBuf::from("/proc/self/fd/0")));
        }
    }

    #[test]
    fn root_cwd_nominates_relative_dev_and_proc_candidates() {
        #[cfg(unix)]
        {
            let ctx = StdinContext { stdin: None, cwd: Some(PathBuf::from("/")), cwd_in_dev_or_proc: false, cwd_is_root: true };
            assert!(ctx.might_be_dev_or_proc(Path::new("dev/stdin")));
            assert!(ctx.might_be_dev_or_proc(Path::new("proc/self/fd/0")));
            assert!(!ctx.might_be_dev_or_proc(Path::new("tmp/file")));
        }
    }

    #[test]
    fn small_read_capped_promotes_above_cap_without_oom() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big-virtual");
        let data: Vec<u8> = (0..100_000usize).map(|i| (i * 17) as u8).collect();
        std::fs::write(&path, &data).unwrap();
        let mut file = File::open(&path).unwrap();
        let mut buf = Vec::with_capacity(64);
        let reached_eof = read_sequential_capped(&mut file, &mut buf, 4096).unwrap();
        assert!(!reached_eof, "must stop at cap and promote");
        assert_eq!(buf.len(), 4096);
        assert_eq!(&buf[..], &data[..4096]);
        let mut rest = Vec::new();
        file.read_to_end(&mut rest).unwrap();
        assert_eq!(&rest[..], &data[4096..]);
    }
}
