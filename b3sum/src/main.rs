//! b3sum: BLAKE3's command-line hasher.
//!
//! On Linux, multi-file runs and uncached large files stream through
//! `io_uring` and `blake3::throughput::Pipeline` (`linux_pipe`):
//! - Warm small files (`<= 16 KiB`) read via `preadv2(RWF_NOWAIT)` into a
//!   core-local 16-slot `Block` arena and hash locally via `hash_each`.
//! - Cold small/medium files (`< 4 MiB`) stream asynchronously through
//!   `io_uring` into recycled buffers (promoting to streaming hashing if a
//!   small-hinted file grows past 4 MiB).
//! - Cached large files (`>= 512 KiB`, first page resident via `mincore`) are
//!   mapped and loaned zero-copy to `Pipeline::lent`, then held mapped up to
//!   `HOLD_PAGES` until exit to avoid `munmap` TLB shootdowns.
//! - Cold large files (`>= 4 MiB`, not resident in page cache) stream
//!   directly from storage into 64 KiB `Pipeline::room()`s via batched
//!   `IORING_OP_READ`s, overlapping NVMe DMA with all-core subtree hashing.
//! - One cold file of 4 MiB or more, with more than one thread, is read
//!   past the page cache (`O_DIRECT`), 2 MiB a read on up to 16 threads
//!   (`direct`); the cache does not hold the file afterwards.
//!
//! Single small files read into a bounded `Vec` (promoting to incremental
//! streaming above 4 MiB) with zero `mmap` or ring setup, and standard
//! 32-byte digests format on the stack into a 32 KiB buffered writer without
//! cloning a 26 KiB `Hasher` per file. Unified file-end rules and descriptor
//! identity checks live in `file_end`.

mod args;
mod err;
#[cfg(unix)]
mod mmap;
#[cfg(test)]
#[path = "test_temp.rs"]
mod tempfile;
use args::{ParseOutcome, ParsedArgs as Inner};
use std::cmp;
use std::fs::File;
use std::io;
use std::io::prelude::*;
use std::path::{Path, PathBuf};

#[cfg(test)]
mod unit_tests;

const NAME: &str = "b3sum";

#[cfg(target_os = "linux")]
mod direct;
mod file_end;
#[cfg(target_os = "linux")]
mod linux_pipe;
mod outboard;
#[cfg(target_os = "linux")]
mod uring;

#[cfg(unix)]
use file_end::StreamId;
use file_end::{HashRead, InputRole, SMALL_MEM_CAP, StdinContext};

/// Best-effort diagnostic line on stderr: never panics on write errors such as
/// `ENOSPC` or `EBADF`, so normal status-1 failures keep their exit status
/// instead of aborting (a closed pipe on stderr still follows default `SIGPIPE`).
fn eprint_line(args: std::fmt::Arguments<'_>) {
    let mut stderr = io::stderr().lock();
    let _ = stderr.write_fmt(args);
    let _ = stderr.write_all(b"\n");
}

macro_rules! eprintln_safe {
    ($($arg:tt)*) => { eprint_line(format_args!($($arg)*)) };
}

struct Args {
    inner: Inner,
    file_args: Vec<PathBuf>,
    mode: blake3::Mode<'static>,
    base_hasher: blake3::convenience::Hasher,
}

impl Args {
    fn parse() -> err::Result<Self> {
        let inner = match args::parse_from(std::env::args_os()) {
            Ok(ParseOutcome::Parsed(inner)) => inner,
            Ok(ParseOutcome::Help) => {
                let mut stdout = io::stdout().lock();
                if let Err(e) = stdout.write_all(args::HELP_TEXT.as_bytes()).and_then(|()| stdout.flush()) {
                    let _ = writeln!(io::stderr().lock(), "{NAME}: {e}");
                    std::process::exit(1);
                }
                std::process::exit(0);
            }
            Ok(ParseOutcome::Version) => {
                let mut stdout = io::stdout().lock();
                if let Err(e) = writeln!(stdout, "b3sums {}", blake3::BUILD_VERSION).and_then(|()| stdout.flush()) {
                    let _ = writeln!(io::stderr().lock(), "{NAME}: {e}");
                    std::process::exit(1);
                }
                std::process::exit(0);
            }
            Err(msg) => {
                let mut stderr = io::stderr().lock();
                let _ = writeln!(stderr, "error: {msg}\n\nUsage: b3sum [OPTIONS] [FILE]...\n\nFor more information, try '--help'.");
                let _ = stderr.flush();
                std::process::exit(2);
            }
        };
        let file_args = if !inner.file.is_empty() { inner.file.clone() } else { vec!["-".into()] };
        if inner.raw && file_args.len() > 1 {
            bail!("Only one filename can be provided when using --raw");
        }
        // BLAKE3's output is 2^64 - 1 bytes long. A request past its end
        // was a panic for text, and for --raw fewer bytes than asked for,
        // with success.
        if inner.seek.checked_add(inner.length).is_none() {
            bail!("--seek {} and --length {} ask for output past its end, at 2^64 - 1 bytes", inner.seek, inner.length);
        }
        // Crash-only: leak the single key or context string once for `'static` Mode.
        let mode: blake3::Mode<'static> = if inner.keyed {
            let key: &'static [u8; blake3::KEY_LEN] = Box::leak(Box::new(read_key_from_stdin()?));
            blake3::Mode::Keyed(key)
        } else if let Some(ref context) = inner.derive_key {
            let ctx: &'static str = Box::leak(context.clone().into_boxed_str());
            blake3::Mode::DeriveKey(ctx)
        } else {
            blake3::Mode::Plain
        };
        let base_hasher = blake3::convenience::Hasher::new(mode, None);
        Ok(Self { inner, file_args, mode, base_hasher })
    }

    #[cfg(test)]
    pub(crate) fn for_test_plain(file_args: Vec<PathBuf>) -> Self {
        let inner = Inner {
            file: file_args.clone(),
            keyed: false,
            derive_key: None,
            length: blake3::OUT_LEN as u64,
            seek: 0,
            num_threads: None,
            no_mmap: false,
            no_names: false,
            raw: false,
            tag: false,
            check: false,
            quiet: false,
            outboard: None,
        };
        Self { inner, file_args, mode: blake3::Mode::Plain, base_hasher: blake3::convenience::Hasher::new(blake3::Mode::Plain, None) }
    }

    #[cfg(test)]
    pub(crate) fn for_test_plain_threads(file_args: Vec<PathBuf>, num_threads: usize) -> Self {
        let mut args = Self::for_test_plain(file_args);
        args.inner.num_threads = std::num::NonZeroUsize::new(num_threads);
        args
    }

    #[inline(always)]
    fn is_standard_32(&self) -> bool {
        self.inner.seek == 0 && self.inner.length <= blake3::OUT_LEN as u64
    }

    fn num_threads(&self) -> Option<std::num::NonZeroUsize> {
        self.inner.num_threads
    }

    fn check(&self) -> bool {
        self.inner.check
    }

    fn outboard(&self) -> Option<&Path> {
        self.inner.outboard.as_deref()
    }

    fn raw(&self) -> bool {
        self.inner.raw
    }

    fn tag(&self) -> bool {
        self.inner.tag
    }

    fn no_names(&self) -> bool {
        self.inner.no_names
    }

    fn len(&self) -> u64 {
        self.inner.length
    }

    fn seek(&self) -> u64 {
        self.inner.seek
    }

    /// The map of `file`, unless `--no-mmap` asked for none or the file
    /// cannot be mapped: the one place b3sum maps a file to hash it. A
    /// mapped file must not be modified in place while being hashed (see
    /// `b3sum/README.md`; pass `--no-mmap` for live or untrusted files). A
    /// shortened file stops the process when its lost pages are read
    /// (`explain_sigbus`). (BLAKE3 #432: the option was accepted and did
    /// nothing.)
    #[cfg(unix)]
    pub(crate) fn map(&self, file: &impl std::os::fd::AsRawFd) -> Option<mmap::Mmap> {
        if self.inner.no_mmap {
            return None;
        }
        #[cfg(test)]
        MAPPED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // Sound: the mapping is read-only and the CLI documents that files
        // must not be modified concurrently while hashed (pass `--no-mmap`
        // for mutable inputs; truncation triggers `explain_sigbus`).
        unsafe { mmap::Mmap::map(file) }.ok()
    }

    fn keyed(&self) -> bool {
        self.inner.keyed
    }

    fn quiet(&self) -> bool {
        self.inner.quiet
    }
}

/// 32 KiB buffered writer over stdout so multi-file runs make one `write`
/// syscall per 32 KiB rather than one `LineWriter` flush per filename, while
/// still hitting `SIGPIPE` on closed pipes (`test_reader_gone`). Tracks
/// partial writes and poisons itself on fatal errors so already-delivered
/// prefixes are never duplicated on later flushes.
pub(crate) struct OutBuf {
    buf: Vec<u8>,
    poisoned: bool,
}

const OUT_BUF_CAP: usize = 32 * 1024;

impl OutBuf {
    fn new() -> Self {
        Self { buf: Vec::with_capacity(OUT_BUF_CAP), poisoned: false }
    }

    #[inline]
    fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    #[inline]
    fn write_bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.poisoned {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "stdout previously failed"));
        }
        if self.buf.len() + bytes.len() > OUT_BUF_CAP {
            self.flush()?;
            if bytes.len() >= OUT_BUF_CAP {
                let (_, res) = write_all_stdout(bytes);
                if res.is_err() {
                    self.poisoned = true;
                }
                return res;
            }
        }
        self.buf.extend_from_slice(bytes);
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.poisoned {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "stdout previously failed"));
        }
        if !self.buf.is_empty() {
            let (written, res) = write_all_stdout(&self.buf);
            if written > 0 {
                self.buf.drain(..written);
            }
            if let Err(e) = res {
                self.poisoned = true;
                return Err(e);
            }
        }
        Ok(())
    }
}

/// Write `bytes` using `write_once`, advancing past short writes, retrying
/// `Interrupted`, and reporting `WriteZero` if `write_once` returns `Ok(0)`.
/// Returns `(bytes_written, result)` so callers never re-emit written prefixes.
fn write_all_with(mut bytes: &[u8], mut write_once: impl FnMut(&[u8]) -> io::Result<usize>) -> (usize, io::Result<()>) {
    let mut written = 0usize;
    while !bytes.is_empty() {
        match write_once(bytes) {
            Ok(0) => {
                return (written, Err(io::Error::new(io::ErrorKind::WriteZero, "failed to write whole buffer to stdout")));
            }
            Ok(n) => {
                let n = n.min(bytes.len());
                written += n;
                bytes = &bytes[n..];
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return (written, Err(e)),
        }
    }
    (written, Ok(()))
}

fn write_all_stdout(bytes: &[u8]) -> (usize, io::Result<()>) {
    #[cfg(unix)]
    {
        write_all_with(bytes, |chunk| {
            // Sound: a write of `chunk`, a slice held here, to standard output.
            let rc = unsafe { libc::write(1, chunk.as_ptr().cast(), chunk.len()) };
            if rc < 0 { Err(io::Error::last_os_error()) } else { Ok(rc as usize) }
        })
    }
    #[cfg(not(unix))]
    {
        let stdout = io::stdout();
        let mut handle = stdout.lock();
        write_all_with(bytes, |chunk| handle.write(chunk))
    }
}

#[inline]
fn encode_hex_stack(src: &[u8], dst: &mut [u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for (i, &b) in src.iter().enumerate() {
        dst[2 * i] = HEX[(b >> 4) as usize];
        dst[2 * i + 1] = HEX[(b & 0x0f) as usize];
    }
}

#[inline]
pub(crate) fn hash_slice_32(mode: blake3::Mode<'_>, bytes: &[u8]) -> blake3::Hash {
    if use_pool_for(bytes.len()) {
        let mut hasher = blake3::convenience::Hasher::new(mode, Some(bytes.len() as u64));
        blake3::throughput::update_multithreaded(&mut hasher, bytes);
        hasher.finalize()
    } else {
        hash_slice_local(mode, bytes)
    }
}

/// Hashes on the calling thread, whether or not the pool is running.
pub(crate) fn hash_slice_local(mode: blake3::Mode<'_>, bytes: &[u8]) -> blake3::Hash {
    if matches!(mode, blake3::Mode::Plain) {
        blake3::convenience::hash(bytes)
    } else {
        let mut hasher = blake3::convenience::Hasher::new(mode, Some(bytes.len() as u64));
        hasher.update(bytes);
        hasher.finalize()
    }
}

/// Whether `path` is the operand that stands for standard input: `-`
/// itself. (Paths compare by their parts, so `-/` and `-/.` were taken
/// for it too, and a file of that name was never opened.)
pub(crate) fn is_stdin(path: &Path) -> bool {
    path.as_os_str() == "-"
}

/// Whether standard input is named more than once (either literally `-` or
/// by a stream alias under `/dev` or `/proc`, including relative spellings
/// resolved against the working directory). A stream is read once in operand
/// order rather than split across file worker threads.
pub(crate) fn stdin_named_twice(paths: &[PathBuf], stdin_ctx: &StdinContext) -> bool {
    let mut stream_refs = 0usize;
    #[cfg(target_os = "macos")]
    let mut dash_refs = 0usize;
    #[cfg(target_os = "macos")]
    let mut darwin_dev_fd_refs = 0usize;
    for path in paths {
        match stdin_ctx.classify(path) {
            InputRole::StdinDash => {
                #[cfg(target_os = "macos")]
                {
                    dash_refs += 1;
                }
                stream_refs += 1;
            }
            #[cfg(unix)]
            InputRole::StdinStreamAlias => {
                stream_refs += 1;
            }
            #[cfg(target_os = "macos")]
            InputRole::DarwinRegularDevFd => {
                darwin_dev_fd_refs += 1;
            }
            InputRole::Path => {}
        }
    }
    #[cfg(target_os = "macos")]
    if dash_refs > 0 && darwin_dev_fd_refs > 0 {
        return true;
    }
    stream_refs > 1
}

/// Hash a regular-file Darwin `/dev/fd/<N>` (or `/dev/stdin`) operand from
/// offset 0 via `mmap` or positioned `pread`, after opening `path` normally to
/// preserve OS lookup errors and permissions, without moving the underlying
/// open-file description's shared offset (#515).
#[cfg(target_os = "macos")]
fn hash_darwin_regular_dev_fd_hash(args: &Args, path: &Path) -> err::Result<blake3::Hash> {
    use std::os::fd::AsRawFd;
    let file = File::open(path)?;
    let fd = file.as_raw_fd();
    let meta = file_end::fstat_fd(fd)?;
    file_end::ensure_byte_stream_fd(fd, meta.mode_type)?;
    if meta.len < MAP_LEN as u64 {
        let mut buf = Vec::with_capacity((usize::try_from(meta.len).expect("len < MAP_LEN") + 1).max(4096).min(SMALL_MEM_CAP + 1));
        let mut reader = file_end::PreadReader::new(fd, 0);
        let mut chunk = [0u8; 64 * 1024];
        loop {
            let n = reader.read(&mut chunk)?;
            if n == 0 {
                return Ok(hash_slice_32(args.mode, &buf));
            }
            if buf.len() + n > SMALL_MEM_CAP {
                let mut hasher = args.base_hasher.clone();
                update(&mut hasher, &buf);
                update(&mut hasher, &chunk[..n]);
                update_from(&mut hasher, reader)?;
                return Ok(hasher.finalize());
            }
            buf.extend_from_slice(&chunk[..n]);
        }
    }
    if let Ok(len) = usize::try_from(meta.len)
        && len >= MAP_LEN
        && let Some(map) = args.map(&file)
    {
        if resident(&map) {
            let h = hash_slice_32(args.mode, &map);
            hold(map);
            return Ok(h);
        }
        drop(map);
    }
    let mut hasher = args.base_hasher.clone();
    update_from(&mut hasher, file_end::PreadReader::new(fd, 0))?;
    Ok(hasher.finalize())
}

/// Fast standard-mode (`seek == 0 && len <= 32`) single-path hasher.
pub(crate) fn hash_path_hash(args: &Args, path: &Path, stdin_ctx: &StdinContext, forbidden: Option<StreamId>) -> err::Result<blake3::Hash> {
    match stdin_ctx.classify(path) {
        InputRole::StdinDash => {
            if args.keyed() {
                bail!("Cannot open `-` in keyed mode");
            }
            #[cfg(unix)]
            if let Some(forbid) = forbidden
                && stdin_ctx.stdin_stream_id() == Some(forbid)
            {
                bail!("standard input is the checkfile; it cannot also be a file to check");
            }
            // `b3sum < file`: a regular file at offset 0 is hashed as a named
            // file; if hashing did not move the descriptor cursor (mmap/pread/
            // direct/ring), advance fd 0 to the exact byte count hashed and
            // propagate any seek failure.
            #[cfg(unix)]
            if let Some(file) = stdin_file() {
                let hr = hash_open_file_ex(args, file)?;
                if !hr.cursor_moved && hr.bytes_hashed > 0 {
                    // Sound: `lseek` on open descriptor 0 touches no memory.
                    let rc = unsafe { libc::lseek(0, hr.bytes_hashed as libc::off_t, libc::SEEK_SET) };
                    if rc < 0 {
                        return Err(io::Error::last_os_error().into());
                    }
                }
                return Ok(hr.hash);
            }
            #[cfg(unix)]
            if let Ok(meta) = file_end::fstat_fd(0) {
                file_end::ensure_byte_stream_fd(0, meta.mode_type)?;
            }
            let mut hasher = args.base_hasher.clone();
            update_from(&mut hasher, io::stdin())?;
            Ok(hasher.finalize())
        }
        #[cfg(unix)]
        InputRole::StdinStreamAlias => {
            if args.keyed() {
                bail!("Cannot open `-` in keyed mode");
            }
            if let Some(forbid) = forbidden
                && stdin_ctx.stdin_stream_id() == Some(forbid)
            {
                bail!("standard input is the checkfile; it cannot also be a file to check");
            }
            // Reuse fd 0 directly instead of reopening `/dev/stdin` (which
            // fails with ENXIO on inherited Unix sockets and can block
            // forever on a closed named FIFO).
            if let Ok(meta) = file_end::fstat_fd(0) {
                file_end::ensure_byte_stream_fd(0, meta.mode_type)?;
            }
            let mut hasher = args.base_hasher.clone();
            update_from(&mut hasher, io::stdin())?;
            Ok(hasher.finalize())
        }
        #[cfg(target_os = "macos")]
        InputRole::DarwinRegularDevFd => hash_darwin_regular_dev_fd_hash(args, path),
        InputRole::Path => {
            #[cfg(unix)]
            {
                let (file, _) = file_end::open_checked(path, forbidden)?;
                Ok(hash_open_file_ex(args, file)?.hash)
            }
            #[cfg(not(unix))]
            {
                let _ = forbidden;
                Ok(hash_open_file_ex(args, File::open(path)?)?.hash)
            }
        }
    }
}

/// Standard input as a file, where it is a regular file read from its
/// start (`b3sum < file`): the same file, its position shared (the
/// caller moves it past the consumed bytes once hashed). Uses `fstat(0)`
/// and `lseek(0, 0, SEEK_CUR)` first so no spare descriptor is touched
/// unless needed.
#[cfg(unix)]
fn stdin_file() -> Option<File> {
    use std::os::fd::AsFd;
    let meta = file_end::fstat_fd(0).ok()?;
    if !meta.is_file() {
        return None;
    }
    // Sound: querying current offset of open descriptor 0 touches no memory.
    let pos = unsafe { libc::lseek(0, 0, libc::SEEK_CUR) };
    if pos != 0 {
        return None;
    }
    Some(File::from(io::stdin().as_fd().try_clone_to_owned().ok()?))
}

/// [`hash_path_hash`] for an open file. Every path hashes this file,
/// including when its original pathname has since been replaced.
#[cfg(target_os = "linux")]
pub(crate) fn hash_open_file(args: &Args, file: File) -> err::Result<blake3::Hash> {
    Ok(hash_open_file_ex(args, file)?.hash)
}

/// Hash an already-opened `file`, returning the digest along with exact byte
/// count and whether the file descriptor's cursor was advanced.
pub(crate) fn hash_open_file_ex(args: &Args, mut file: File) -> err::Result<HashRead> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        if let Ok(fd_meta) = file_end::fstat_fd(file.as_raw_fd()) {
            file_end::ensure_byte_stream_fd(file.as_raw_fd(), fd_meta.mode_type)?;
        }
    }
    if let Ok(meta) = file.metadata()
        && meta.is_file()
    {
        let size = meta.len();
        if size < MAP_LEN as u64 {
            let cap = (usize::try_from(size).expect("size < MAP_LEN") + 1).max(4096).min(SMALL_MEM_CAP + 1);
            let mut buf = Vec::with_capacity(cap);
            let reached_eof = file_end::read_sequential_capped(&mut file, &mut buf, SMALL_MEM_CAP)?;
            if reached_eof {
                return Ok(HashRead { hash: hash_slice_32(args.mode, &buf), bytes_hashed: buf.len() as u64, cursor_moved: true });
            }
            return Ok(file_end::finish_hash_streaming_from_prefix(args.mode, &buf, &mut file, true)?);
        }

        #[cfg(unix)]
        if let Ok(len) = usize::try_from(size)
            && len >= MAP_LEN
            && let Some(map) = args.map(&file)
        {
            let warm = resident(&map);
            // The tests' seam: a file just written is in the cache.
            #[cfg(all(test, target_os = "linux"))]
            let warm = warm && !linux_pipe::force_miss();
            if warm {
                // Release the open file descriptor once the mapping is known
                // warm so tight FD limits (e.g. RLIMIT_NOFILE=4) still have a
                // spare FD if `hash_slice_32` triggers lazy NUMA sysfs reads.
                drop(file);
                let h = hash_slice_32(args.mode, &map);
                let map_len = map.len() as u64;
                hold(map);
                return Ok(HashRead { hash: h, bytes_hashed: map_len, cursor_moved: false });
            }
            #[cfg(target_os = "linux")]
            if threads().get() == 1 {
                drop(map);
                if size < POOL_START_LEN as u64 {
                    let mut buf = Vec::with_capacity((len + 1).min(SMALL_MEM_CAP + 1));
                    let reached_eof = file_end::read_sequential_capped(&mut file, &mut buf, SMALL_MEM_CAP)?;
                    if reached_eof {
                        return Ok(HashRead { hash: hash_slice_32(args.mode, &buf), bytes_hashed: buf.len() as u64, cursor_moved: true });
                    }
                    return Ok(file_end::finish_hash_streaming_from_prefix(args.mode, &buf, &mut file, true)?);
                }
                if let Some(res) = linux_pipe::hash_single_cold_large_file_1t(args, &file, size) {
                    let (hash, bytes_hashed) = res?;
                    return Ok(HashRead { hash, bytes_hashed, cursor_moved: false });
                }
                // No ring to read it through (a sandbox that refuses
                // io_uring, a kernel without IORING_OP_READ): stream below.
            } else {
                // A cold file of 4 MiB or more: read past the page cache,
                // several reads at once, each thread hashing its piece
                // (direct.rs). Under `--num-threads`, concurrent file workers
                // lease extra direct threads from one shared spare permit pool
                // so total active threads stay within `--num-threads`.
                let want_total = if args.num_threads().is_some() { pool_threads().get() } else { threads().get() }.min(DIRECT_THREADS);
                let permits = DirectPermits::acquire(want_total.saturating_sub(1));
                let direct_threads = 1 + permits.extra;
                if direct::is_for(&file, size)
                    && let Some(h) = direct::hash(args.mode, &file, len, direct_threads, &mut direct::Piece::new())
                {
                    drop(permits);
                    drop(map);
                    return Ok(HashRead { hash: h, bytes_hashed: len as u64, cursor_moved: false });
                }
                drop(permits);
                // Multi-threaded single cold file (`>= 512 KiB`): start the pool,
                // whose workers fault disjoint subtrees in parallel.
                init_pool_once(pool_threads().get());
                let h = hash_slice_32(args.mode, &map);
                let map_len = map.len() as u64;
                hold(map);
                return Ok(HashRead { hash: h, bytes_hashed: map_len, cursor_moved: false });
            }
            #[cfg(not(target_os = "linux"))]
            drop(map);
        }
    }

    let mut hasher = args.base_hasher.clone();
    let bytes_hashed = update_from_counted(&mut hasher, file)?;
    Ok(HashRead { hash: hasher.finalize(), bytes_hashed, cursor_moved: true })
}

fn hash_path(args: &Args, path: &Path, stdin_ctx: &StdinContext) -> err::Result<blake3::OutputReader> {
    let mut hasher = args.base_hasher.clone();
    match stdin_ctx.classify(path) {
        InputRole::StdinDash => {
            if args.keyed() {
                bail!("Cannot open `-` in keyed mode");
            }
            #[cfg(unix)]
            if let Ok(meta) = file_end::fstat_fd(0) {
                file_end::ensure_byte_stream_fd(0, meta.mode_type)?;
            }
            update_from(&mut hasher, io::stdin())?;
        }
        #[cfg(unix)]
        InputRole::StdinStreamAlias => {
            if args.keyed() {
                bail!("Cannot open `-` in keyed mode");
            }
            if let Ok(meta) = file_end::fstat_fd(0) {
                file_end::ensure_byte_stream_fd(0, meta.mode_type)?;
            }
            update_from(&mut hasher, io::stdin())?;
        }
        #[cfg(target_os = "macos")]
        InputRole::DarwinRegularDevFd => {
            use std::os::fd::AsRawFd;
            let file = File::open(path)?;
            let fd = file.as_raw_fd();
            let meta = file_end::fstat_fd(fd)?;
            file_end::ensure_byte_stream_fd(fd, meta.mode_type)?;
            if meta.len >= MAP_LEN as u64
                && let Some(map) = args.map(&file)
                && resident(&map)
            {
                update(&mut hasher, &map);
                hold(map);
            } else {
                update_from(&mut hasher, file_end::PreadReader::new(fd, 0))?;
            }
        }
        InputRole::Path => {
            let mut file = File::open(path)?;
            #[cfg(unix)]
            {
                use std::os::fd::AsRawFd;
                if let Ok(fd_meta) = file_end::fstat_fd(file.as_raw_fd()) {
                    file_end::ensure_byte_stream_fd(file.as_raw_fd(), fd_meta.mode_type)?;
                }
            }
            if let Ok(meta) = file.metadata()
                && meta.is_file()
                && meta.len() < 64 * 1024
            {
                let mut stack_buf = [0u8; 64 * 1024];
                let (n, eof) = fill(&mut file, &mut stack_buf)?;
                update(&mut hasher, &stack_buf[..n]);
                if !eof {
                    update_from(&mut hasher, file)?;
                }
            } else {
                #[cfg(unix)]
                if let Some(map) = mapped(args, &file) {
                    update(&mut hasher, &map);
                    hold(map);
                } else {
                    update_from(&mut hasher, file)?;
                }
                #[cfg(not(unix))]
                update_from(&mut hasher, file)?;
            }
        }
    }
    let mut output_reader = hasher.finalize_xof();
    output_reader.set_position(args.seek());
    Ok(output_reader)
}

#[cfg(unix)]
const HOLD_PAGES: u64 = 1 << 18;

#[cfg(unix)]
pub(crate) fn hold(map: mmap::Mmap) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static HELD: AtomicU64 = AtomicU64::new(0);
    // Sound: a query with no memory argument.
    let page = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).expect("the page size is positive");
    let pages = (map.len() as u64).div_ceil(page);
    if HELD.fetch_add(pages, Ordering::Relaxed) + pages <= HOLD_PAGES {
        std::mem::forget(map);
    } else {
        HELD.fetch_sub(pages, Ordering::Relaxed);
    }
}

/// The files this process has mapped (`Args::map`). Atomic so multi-threaded
/// `linux_pipe::hash_paths` counts maps across all worker threads (#504).
#[cfg(test)]
pub(crate) static MAPPED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(unix)]
fn mapped(args: &Args, file: &File) -> Option<mmap::Mmap> {
    let len = usize::try_from(file.metadata().ok()?.len()).ok()?;
    if len < MAP_LEN {
        return None;
    }
    let map = args.map(file)?;
    if cfg!(target_os = "linux") {
        return Some(map);
    }
    resident(&map).then_some(map)
}

#[cfg(unix)]
pub(crate) fn resident(map: &mmap::Mmap) -> bool {
    let mut vec = 0u8;
    // Sound: one page of the map (a map is at least one byte here, the
    // callers') and one byte for the answer.
    let rc = unsafe { libc::mincore(map.as_ptr() as *mut libc::c_void, 1, (&mut vec as *mut u8).cast()) };
    rc == 0 && vec & 1 == 1
}

#[cfg(unix)]
pub(crate) const MAP_LEN: usize = POOL_LEN;

const POOL_LEN: usize = 512 << 10;
/// Starting the thread pool in a one-shot CLI process costs ~15-25 us per worker
/// thread (~0.3-2 ms), which exceeds the whole hash time below ~4 MiB; once
/// started, inputs from `POOL_LEN` up use it.
const POOL_START_LEN: usize = 4 << 20;

/// The threads that read one cold file directly, each a read at a time:
/// a drive's answer grows little past this many reads at once (a Ryzen
/// 9700X's: 5.7 GB/s at 2, 8.8 at 4, 10.3 at 8, 10.7 at 16).
#[cfg(target_os = "linux")]
pub(crate) const DIRECT_THREADS: usize = 16;

/// Hard ceiling on effective `--num-threads` so absurd values (e.g. `usize::MAX`)
/// are bounded cleanly as an upper limit instead of aborting in pool allocation.
const MAX_THREADS: usize = 1024;

static POOL_ONCE: std::sync::Once = std::sync::Once::new();

struct ThreadBudget {
    file_threads: usize,
    pool_threads: usize,
    leased_direct: usize,
    strict: bool,
}

static THREAD_BUDGET: std::sync::Mutex<ThreadBudget> =
    std::sync::Mutex::new(ThreadBudget { file_threads: 1, pool_threads: 0, leased_direct: 0, strict: false });

fn lock_budget() -> std::sync::MutexGuard<'static, ThreadBudget> {
    THREAD_BUDGET.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn init_pool_once(n: usize) {
    POOL_ONCE.call_once(|| {
        let clamped = {
            let mut b = lock_budget();
            let limit = if b.strict { threads().get().saturating_sub(b.file_threads.saturating_add(b.leased_direct)).saturating_add(1) } else { MAX_THREADS };
            let chosen = n.min(limit).clamp(1, MAX_THREADS);
            b.pool_threads = chosen;
            chosen
        };
        blake3::initialize_multithreaded(std::num::NonZeroUsize::new(clamped));
    });
}

/// Cap on file-worker threads for a batch, respecting any background worker
/// threads already started in the global pool so total simultaneous hashing
/// threads stay within `--num-threads` across multiple batches/checkfiles.
pub(crate) fn max_file_threads(total: usize) -> usize {
    let bg_workers = lock_budget().pool_threads.saturating_sub(1);
    total.saturating_sub(bg_workers).max(1)
}

#[inline]
fn use_pool_for(len: usize) -> bool {
    if POOL_ONCE.is_completed() {
        len >= POOL_LEN && pool_threads().get() > 1
    } else if len >= POOL_START_LEN && pool_threads().get() > 1 {
        init_pool_once(pool_threads().get());
        true
    } else {
        false
    }
}

fn cap_requested_threads(requested: std::num::NonZeroUsize) -> std::num::NonZeroUsize {
    let avail = std::thread::available_parallelism().map_or(64, |n| n.get());
    let cap = avail.saturating_mul(4).clamp(64, MAX_THREADS);
    std::num::NonZeroUsize::new(requested.get().min(cap).max(1)).expect("capped thread count is nonzero")
}

#[cfg(test)]
thread_local! {
    /// For the tests: the inputs this thread read in pieces ([`update_from`]).
    pub(crate) static STREAMED: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
}

static THREADS: std::sync::OnceLock<std::num::NonZeroUsize> = std::sync::OnceLock::new();

#[cfg(all(test, target_os = "linux"))]
pub(crate) static CONFINEMENT_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(target_os = "linux")]
fn confine_cli_once() {
    static CONFINED: std::sync::Once = std::sync::Once::new();
    CONFINED.call_once(|| {
        #[cfg(test)]
        CONFINEMENT_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        blake3::__confine_cli_to_one_node();
    });
}

/// Apply `--num-threads` before hashing begins. Runs with `--num-threads > 1`
/// confine to one Linux memory node eagerly before `available_parallelism()`
/// caps the request; `--num-threads 1` and default runs defer confinement to
/// the first `threads()` demand (so single-threaded paths that never ask for
/// workers skip `/sys/devices/system` NUMA discovery: BLAKE3 #469).
pub(crate) fn configure_threads(args: &Args) {
    if let Some(requested) = args.num_threads() {
        #[cfg(target_os = "linux")]
        if requested.get() > 1 {
            confine_cli_once();
        }
        THREADS.set(cap_requested_threads(requested)).expect("main sets the threads first");
        lock_budget().strict = true;
    }
}

pub(crate) fn threads() -> std::num::NonZeroUsize {
    *THREADS.get_or_init(|| {
        // Confine lazily on the first default `threads()` demand so single-
        // threaded paths that never query worker count skip per-process
        // `/sys/devices/system` NUMA discovery on multi-socket Linux hosts (#469).
        #[cfg(target_os = "linux")]
        confine_cli_once();
        std::thread::available_parallelism().unwrap_or(std::num::NonZeroUsize::MIN)
    })
}

/// Scoped accounting for active file-worker threads under `--num-threads`.
/// Shares one mutex-serialized hashing thread budget across file workers,
/// global pool background workers, and `direct::hash` helper threads (#515).
pub(crate) struct FileThreadsGuard {
    prev_file: usize,
    prev_strict: bool,
}

impl FileThreadsGuard {
    pub(crate) fn enter(file_threads: usize) -> Self {
        let capped = file_threads.clamp(1, threads().get());
        let mut b = lock_budget();
        let prev_file = core::mem::replace(&mut b.file_threads, capped);
        let prev_strict = core::mem::replace(&mut b.strict, true);
        Self { prev_file, prev_strict }
    }
}

impl Drop for FileThreadsGuard {
    fn drop(&mut self) {
        let mut b = lock_budget();
        b.strict = self.prev_strict;
        b.file_threads = self.prev_file;
    }
}

/// Shared lease of extra helper threads for `direct::hash` under strict
/// `--num-threads` budgeting, serialized atomically with pool initialization.
#[cfg(any(target_os = "linux", test))]
pub(crate) struct DirectPermits {
    pub(crate) extra: usize,
    leased: bool,
}

#[cfg(any(target_os = "linux", test))]
impl DirectPermits {
    pub(crate) fn acquire(want_extra: usize) -> Self {
        if want_extra == 0 {
            return Self { extra: 0, leased: false };
        }
        let mut b = lock_budget();
        if !b.strict {
            return Self { extra: want_extra, leased: false };
        }
        let used = b.file_threads.saturating_add(b.pool_threads.saturating_sub(1)).saturating_add(b.leased_direct);
        let take = threads().get().saturating_sub(used).min(want_extra);
        if take == 0 {
            return Self { extra: 0, leased: false };
        }
        b.leased_direct += take;
        Self { extra: take, leased: true }
    }
}

#[cfg(any(target_os = "linux", test))]
impl Drop for DirectPermits {
    fn drop(&mut self) {
        if self.leased && self.extra > 0 {
            let mut b = lock_budget();
            b.leased_direct = b.leased_direct.saturating_sub(self.extra);
        }
    }
}

pub(crate) fn pool_threads() -> std::num::NonZeroUsize {
    let b = lock_budget();
    let leased_direct = if b.strict { b.leased_direct } else { 0 };
    std::num::NonZeroUsize::new(threads().get().saturating_sub(b.file_threads.saturating_add(leased_direct)).saturating_add(1)).expect("at least one thread")
}

pub(crate) fn update(hasher: &mut blake3::convenience::Hasher, input: &[u8]) {
    if use_pool_for(input.len()) {
        blake3::throughput::update_multithreaded(hasher, input);
    } else {
        hasher.update(input);
    }
}

const PIECE: usize = 4 << 20;

thread_local! {
    static BUFFERS: std::cell::RefCell<[Vec<u8>; 2]> = std::cell::RefCell::new([vec![0; PIECE], vec![0; PIECE]]);
}

/// Minimum read request length so Linux `pipe2(O_DIRECT)` packet-mode pipes
/// (which buffer up to one kernel `PAGE_SIZE` per packet in `fs/pipe.c` and
/// discard any unread tail on a short `read`) never silently truncate a packet
/// on 4 KiB, 16 KiB, or 64 KiB page kernels (#515).
pub(crate) fn min_packet_read_len() -> usize {
    #[cfg(target_os = "linux")]
    {
        static PAGE_SIZE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
        *PAGE_SIZE.get_or_init(|| {
            // Sound: `sysconf(_SC_PAGESIZE)` queries an immutable OS constant.
            let ps = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
            usize::try_from(ps).ok().filter(|&n| n >= 4096).unwrap_or(4096)
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        4096
    }
}

/// Fill `buffer` from `reader`, returning `(bytes_read, reached_eof)`.
/// Keeps every read request `>= min_packet_read_len()` (at least one kernel
/// page on Linux) when filling a piece buffer so `pipe2(O_DIRECT)` packet-mode
/// pipes never discard a packet tail on a short trailing read (#515);
/// `reached_eof` is true only when `reader.read` actually returns `Ok(0)`.
fn fill(reader: &mut impl Read, buffer: &mut [u8]) -> io::Result<(usize, bool)> {
    let min_read = min_packet_read_len();
    let mut filled = 0;
    while filled < buffer.len() {
        if filled > 0 && buffer.len() >= min_read && buffer.len() - filled < min_read {
            return Ok((filled, false));
        }
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => return Ok((filled, true)),
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok((filled, false))
}

fn update_from(hasher: &mut blake3::convenience::Hasher, reader: impl Read + Send) -> io::Result<()> {
    update_from_counted(hasher, reader).map(|_| ())
}

pub(crate) fn update_from_counted(hasher: &mut blake3::convenience::Hasher, mut reader: impl Read + Send) -> io::Result<u64> {
    #[cfg(test)]
    STREAMED.with(|streamed| streamed.set(streamed.get() + 1));
    BUFFERS.with_borrow_mut(|[first, second]| {
        let (mut len, mut eof) = fill(&mut reader, first)?;
        let mut total = len as u64;
        if eof {
            update(hasher, &first[..len]);
            return Ok(total);
        }
        if threads().get() == 1 {
            loop {
                update(hasher, &first[..len]);
                if eof {
                    return Ok(total);
                }
                let next = fill(&mut reader, first)?;
                len = next.0;
                eof = next.1;
                total += len as u64;
            }
        }
        std::thread::scope(|scope| {
            let (free_tx, free_rx) = std::sync::mpsc::sync_channel::<&mut [u8]>(1);
            let (full_tx, full_rx) = std::sync::mpsc::sync_channel::<(&mut [u8], io::Result<(usize, bool)>)>(1);
            let reader = &mut reader;
            scope.spawn(move || {
                for buffer in free_rx {
                    let read = fill(reader, buffer);
                    let last = !matches!(read, Ok((_, false)));
                    if full_tx.send((buffer, read)).is_err() || last {
                        break;
                    }
                }
            });
            free_tx.send(&mut second[..]).expect("the reader waits for a buffer");
            let mut current: &mut [u8] = &mut first[..];
            let mut cur_len = len;
            loop {
                update(hasher, &current[..cur_len]);
                let (next, read) = full_rx.recv().expect("the reader sends every piece");
                let (next_len, next_eof) = read?;
                total += next_len as u64;
                if next_eof {
                    update(hasher, &next[..next_len]);
                    return Ok(total);
                }
                free_tx.send(current).expect("the reader waits for a buffer");
                current = next;
                cur_len = next_len;
            }
        })
    })
}

fn write_hash_text(hash: &blake3::Hash, args: &Args, out: &mut OutBuf) -> err::Result<()> {
    let take = usize::try_from(args.len()).expect("standard 32-byte mode len <= 32");
    let mut hex_buf = [0u8; 2 * blake3::OUT_LEN];
    encode_hex_stack(&hash.as_bytes()[..take], &mut hex_buf[..2 * take]);
    out.write_bytes(&hex_buf[..2 * take])?;
    Ok(())
}

pub(crate) fn print_hash_output(hash: blake3::Hash, path: &Path, args: &Args, out: &mut OutBuf) -> err::Result<()> {
    let take = usize::try_from(args.len()).expect("standard 32-byte mode len <= 32");
    if args.raw() {
        out.write_bytes(&hash.as_bytes()[..take])?;
        return Ok(());
    }
    if args.no_names() {
        write_hash_text(&hash, args, out)?;
        out.write_bytes(b"\n")?;
        return Ok(());
    }

    // Fast zero-allocation path when valid UTF-8 (or ASCII) without `\`, `\n`, `\r`!
    if let Some(s) = path.to_str()
        && !s.as_bytes().iter().any(|&b| matches!(b, b'\\' | b'\n' | b'\r'))
    {
        if args.tag() {
            out.write_bytes(b"BLAKE3 (")?;
            out.write_bytes(s.as_bytes())?;
            out.write_bytes(b") = ")?;
            write_hash_text(&hash, args, out)?;
            out.write_bytes(b"\n")?;
        } else {
            write_hash_text(&hash, args, out)?;
            out.write_bytes(b"  ")?;
            out.write_bytes(s.as_bytes())?;
            out.write_bytes(b"\n")?;
        }
        return Ok(());
    }

    let FilepathString { filepath_string, is_escaped } = filepath_to_string(path);
    if is_escaped {
        out.write_bytes(b"\\")?;
    }
    if args.tag() {
        out.write_bytes(b"BLAKE3 (")?;
        out.write_bytes(filepath_string.as_bytes())?;
        out.write_bytes(b") = ")?;
        write_hash_text(&hash, args, out)?;
        out.write_bytes(b"\n")?;
    } else {
        write_hash_text(&hash, args, out)?;
        out.write_bytes(b"  ")?;
        out.write_bytes(filepath_string.as_bytes())?;
        out.write_bytes(b"\n")?;
    }
    Ok(())
}

fn write_hex_output(mut output: blake3::OutputReader, args: &Args, out: &mut OutBuf) -> err::Result<()> {
    let mut len = args.len();
    let mut block = [0u8; blake3::BLOCK_LEN];
    let mut hex_buf = [0u8; 2 * blake3::BLOCK_LEN];
    while len > 0 {
        // Only the bytes asked for: the output's last bytes are no whole
        // block, and filling one past its end is a panic.
        let take_bytes = usize::try_from(cmp::min(len, block.len() as u64)).expect("bounded by block.len()");
        output.fill(&mut block[..take_bytes]);
        encode_hex_stack(&block[..take_bytes], &mut hex_buf[..2 * take_bytes]);
        out.write_bytes(&hex_buf[..2 * take_bytes])?;
        len -= take_bytes as u64;
    }
    Ok(())
}

fn write_text_output(output: blake3::OutputReader, args: &Args, out: &mut OutBuf) -> err::Result<()> {
    write_hex_output(output, args, out)
}

fn write_raw_output(output: blake3::OutputReader, args: &Args, out: &mut OutBuf) -> err::Result<()> {
    out.flush()?;
    let mut output = output.take(args.len());
    let stdout = std::io::stdout();
    let mut handler = stdout.lock();
    std::io::copy(&mut output, &mut handler)?;
    Ok(())
}

fn print_output(output: blake3::OutputReader, path: &Path, args: &Args, out: &mut OutBuf) -> err::Result<()> {
    if args.raw() {
        write_raw_output(output, args, out)?;
        return Ok(());
    }
    if args.no_names() {
        write_text_output(output, args, out)?;
        out.write_bytes(b"\n")?;
        return Ok(());
    }
    let FilepathString { filepath_string, is_escaped } = filepath_to_string(path);
    if is_escaped {
        out.write_bytes(b"\\")?;
    }
    if args.tag() {
        out.write_bytes(b"BLAKE3 (")?;
        out.write_bytes(filepath_string.as_bytes())?;
        out.write_bytes(b") = ")?;
        write_text_output(output, args, out)?;
        out.write_bytes(b"\n")?;
        return Ok(());
    }
    write_text_output(output, args, out)?;
    out.write_bytes(b"  ")?;
    out.write_bytes(filepath_string.as_bytes())?;
    out.write_bytes(b"\n")?;
    Ok(())
}

fn read_key_from_stdin() -> err::Result<[u8; blake3::KEY_LEN]> {
    #[cfg(unix)]
    if let Ok(meta) = file_end::fstat_fd(0) {
        file_end::ensure_byte_stream_fd(0, meta.mode_type)?;
    }
    let mut bytes = Vec::with_capacity(blake3::KEY_LEN + 1);
    let n = std::io::stdin().lock().take(blake3::KEY_LEN as u64 + 1).read_to_end(&mut bytes)?;
    if n < blake3::KEY_LEN {
        bail!("expected {} key bytes from stdin, found {}", blake3::KEY_LEN, n,)
    } else if n > blake3::KEY_LEN {
        bail!("read more than {} key bytes from stdin", blake3::KEY_LEN)
    } else {
        Ok(bytes[..blake3::KEY_LEN].try_into().unwrap())
    }
}

struct FilepathString {
    filepath_string: String,
    is_escaped: bool,
}

fn filepath_to_string(filepath: &Path) -> FilepathString {
    let unicode_cow = filepath.to_string_lossy();
    let mut filepath_string = unicode_cow.to_string();
    let mut is_escaped = false;
    if filepath_string.contains(['\\', '\n', '\r']) {
        filepath_string = filepath_string.replace('\\', "\\\\").replace('\n', "\\n").replace('\r', "\\r");
        is_escaped = true;
    }
    FilepathString { filepath_string, is_escaped }
}

fn hex_half_byte(b: u8) -> err::Result<u8> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        _ => bail!("Invalid hex"),
    }
}

fn check_for_invalid_characters(utf8_path: &str) -> err::Result<()> {
    if utf8_path.contains('\0') {
        bail!("Null character in path");
    }
    if utf8_path.contains('\u{FFFD}') {
        bail!("Unicode replacement character in path");
    }
    Ok(())
}

fn unescape(mut path: &str) -> err::Result<String> {
    let mut unescaped = String::with_capacity(path.len());
    while let Some(i) = path.find('\\') {
        ensure!(i < path.len() - 1, "Invalid backslash escape");
        unescaped.push_str(&path[..i]);
        match path[i + 1..].chars().next().unwrap() {
            'n' => unescaped.push('\n'),
            'r' => unescaped.push('\r'),
            '\\' => unescaped.push('\\'),
            _ => bail!("Invalid backslash escape"),
        }
        path = &path[i + 2..];
    }
    unescaped.push_str(path);
    Ok(unescaped)
}

#[derive(Debug)]
struct ParsedCheckLine {
    file_string: String,
    is_escaped: bool,
    file_path: PathBuf,
    expected_hash: blake3::Hash,
}

fn split_untagged_check_line(line_after_slash: &str) -> Option<(&str, &str)> {
    line_after_slash.split_once("  ")
}

fn split_tagged_check_line(line_after_slash: &str) -> Option<(&str, &str)> {
    let prefix = "BLAKE3 (";
    if !line_after_slash.starts_with(prefix) {
        return None;
    }
    line_after_slash[prefix.len()..].rsplit_once(") = ")
}

fn parse_check_line(mut line: &str) -> err::Result<ParsedCheckLine> {
    line = line.trim_end_matches(['\r', '\n']);
    let Some(first) = line.chars().next() else {
        bail!("Empty line");
    };
    let line_after_slash;
    let is_escaped;
    if first == '\\' {
        is_escaped = true;
        line_after_slash = &line[1..];
    } else {
        is_escaped = false;
        line_after_slash = line;
    }

    let hash_text;
    let file_str;
    // The tagged form first: it is told by its start and its last
    // `) = `, and its name may hold two spaces, which is how the other
    // form is cut (a file named `a  b` made a line of --tag that --check
    // called an invalid hash).
    if let Some((left, right)) = split_tagged_check_line(line_after_slash) {
        file_str = left;
        hash_text = right;
    } else if let Some((left, right)) = split_untagged_check_line(line_after_slash) {
        hash_text = left;
        file_str = right;
    } else {
        bail!("Invalid check line format");
    }

    let mut hash_bytes = [0; blake3::OUT_LEN];
    let hex_bytes = hash_text.as_bytes();
    ensure!(hex_bytes.len() == 2 * blake3::OUT_LEN, "Invalid hash length");
    for (byte, pair) in hash_bytes.iter_mut().zip(hex_bytes.chunks_exact(2)) {
        *byte = 16 * hex_half_byte(pair[0])? + hex_half_byte(pair[1])?;
    }
    let expected_hash: blake3::Hash = hash_bytes.into();

    let file_path_string = if is_escaped { unescape(file_str)? } else { file_str.to_string() };
    ensure!(!file_path_string.is_empty(), "empty file path");
    check_for_invalid_characters(&file_path_string)?;

    Ok(ParsedCheckLine { file_string: file_str.to_string(), is_escaped, file_path: file_path_string.into(), expected_hash })
}

/// Hash `paths` in standard 32-byte mode (`seek == 0 && len <= 32`) across
/// cores (using `linux_pipe` on Linux and worker threads elsewhere), invoking
/// `on_ordered(idx, result)` in exact `0..paths.len()` order.
fn hash_paths_32(
    args: &Args,
    paths: &[PathBuf],
    stdin_ctx: &StdinContext,
    #[cfg(unix)] forbidden: Option<StreamId>,
    #[cfg(not(unix))] forbidden: Option<()>,
    on_ordered: &mut dyn FnMut(usize, err::Result<blake3::Hash>) -> bool,
) {
    if paths.is_empty() {
        return;
    }
    if paths.len() == 1 {
        let _ = on_ordered(0, hash_path_hash(args, &paths[0], stdin_ctx, forbidden));
        return;
    }
    if stdin_named_twice(paths, stdin_ctx) {
        for (i, path) in paths.iter().enumerate() {
            if !on_ordered(i, hash_path_hash(args, path, stdin_ctx, forbidden)) {
                break;
            }
        }
        return;
    }

    #[cfg(target_os = "linux")]
    if linux_pipe::applies_to_paths(args, paths, stdin_ctx) {
        linux_pipe::hash_paths(args, paths, forbidden, on_ordered);
        return;
    }

    let total = threads().get();
    let max_workers = if args.num_threads().is_some() { max_file_threads(total) } else { total };
    let threads = max_workers.min(paths.len());
    let _file_threads_guard = args.num_threads().map(|_| FileThreadsGuard::enter(threads));
    let next = std::sync::atomic::AtomicUsize::new(0);
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        for _ in 1..threads {
            if next.load(std::sync::atomic::Ordering::Relaxed) >= paths.len() {
                break;
            }
            let (next, done_tx) = (&next, done_tx.clone());
            scope.spawn(move || {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if i >= paths.len() || done_tx.send((i, hash_path_hash(args, &paths[i], stdin_ctx, forbidden))).is_err() {
                        break;
                    }
                }
            });
        }
        drop(done_tx);

        let mut emitted = 0usize;
        let mut stop = false;
        let mut results: Vec<Option<err::Result<blake3::Hash>>> = (0..paths.len()).map(|_| None).collect();
        let mut take = |i: usize, output: err::Result<blake3::Hash>| -> bool {
            results[i] = Some(output);
            while emitted < paths.len() {
                let Some(item) = results[emitted].take() else { break };
                if !on_ordered(emitted, item) {
                    stop = true;
                    return false;
                }
                emitted += 1;
            }
            !stop
        };
        loop {
            let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if i >= paths.len() || !take(i, hash_path_hash(args, &paths[i], stdin_ctx, forbidden)) {
                break;
            }
        }
        for (i, output) in done_rx {
            if !take(i, output) {
                break;
            }
        }
    });
}

/// The outboards' files are created by this many threads, apart from the
/// ones that hash: creating a file takes its directory's lock, so more
/// threads than a few only wait for one another, and the hashing threads
/// would wait with them.
const OUTBOARD_WRITERS: usize = 2;

/// `--outboard DIR`: each file's hash, printed as ever, and its outboard
/// written to DIR/<hash>.obao4 (outboard.rs).
///
/// The files are shared out among threads started once for the run, each
/// with rooms of its own for a file's bytes and chaining values; where
/// there are fewer files than threads, each file takes several. A thread
/// hands each outboard to the writers and takes the next file. The lines
/// are printed in the files' order, once all are done. One file is hashed
/// and written by the caller's thread alone.
fn outboard_inputs(args: &Args, dir: &Path, stdin_ctx: &StdinContext, out: &mut OutBuf) -> u64 {
    use std::sync::{Mutex, mpsc};
    let paths = &args.file_args;
    // Before any file is hashed: a run of short files alone would write
    // nothing, and say nothing of a directory that is not there.
    if !dir.is_dir() {
        eprintln_safe!("{}: {}: not a directory", NAME, dir.to_string_lossy());
        return paths.len().max(1) as u64;
    }
    let total = threads().get();
    let max_workers = if args.num_threads().is_some() { max_file_threads(total) } else { total };
    let workers = if stdin_named_twice(paths, stdin_ctx) { 1 } else { max_workers.min(paths.len()) };
    let in_a_file = (total / workers.max(1)).max(1);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let done = Mutex::new(Vec::<(usize, err::Result<blake3::Hash>)>::new());
    // To the writers: a file's index, its hash, its outboard; no more
    // waiting than a few a writer, so that outboards are not made faster
    // than they are written. Back from them: the outboards' buffers, for
    // the next files.
    type Made = (usize, blake3::Hash, Vec<u8>);
    let (to_writers, made) = mpsc::sync_channel::<Made>(8 * OUTBOARD_WRITERS);
    let (to_workers, spare) = mpsc::channel::<Vec<u8>>();
    let (made, spare) = (Mutex::new(made), Mutex::new(spare));
    let unwritten = Mutex::new(Vec::<(usize, io::Error)>::new());
    let write = |(i, hash, nodes): Made| {
        if let Err(e) = outboard::write_in(dir, &hash, &nodes) {
            unwritten.lock().unwrap().push((i, e));
        }
        // No worker may be left to take it.
        let _ = to_workers.send(nodes);
    };
    let work = |hand: &dyn Fn(Made)| {
        let mut rooms = outboard::Rooms::new();
        let mut mine = Vec::new();
        let mut nodes = Vec::new();
        loop {
            let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let Some(path) = paths.get(i) else { break };
            nodes.clear();
            let hashed = outboard_of(args, path, stdin_ctx, in_a_file, &mut rooms, &mut nodes);
            // An empty outboard (a file of one chunk group or less) is no
            // file: there is nothing in it to read, and creating a file
            // costs more than hashing a short one.
            if let Ok(hash) = &hashed
                && !nodes.is_empty()
            {
                hand((i, *hash, core::mem::take(&mut nodes)));
                nodes = spare.lock().unwrap().try_recv().unwrap_or_default();
            }
            mine.push((i, hashed));
        }
        done.lock().unwrap().append(&mut mine);
    };
    if workers <= 1 {
        work(&write);
    } else {
        std::thread::scope(|scope| {
            for _ in 0..OUTBOARD_WRITERS {
                scope.spawn(|| {
                    loop {
                        // The lock is let go before the file is written.
                        let next = made.lock().unwrap().recv();
                        let Ok(next) = next else { break };
                        write(next);
                    }
                });
            }
            let work = &work;
            for _ in 1..workers {
                let to_writers = to_writers.clone();
                scope.spawn(move || work(&|made| to_writers.send(made).expect("the writers run until every sender is done")));
            }
            work(&|made| to_writers.send(made).expect("the writers run until every sender is done"));
            // The writers end when the channel does: when every thread's
            // sender is gone.
            drop(to_writers);
        });
    }
    let unwritten = unwritten.into_inner().unwrap();
    let mut results: Vec<Option<err::Result<blake3::Hash>>> = (0..paths.len()).map(|_| None).collect();
    for (i, result) in done.into_inner().unwrap() {
        results[i] = Some(result);
    }
    for (i, e) in unwritten {
        results[i] = Some(Err(e.into()));
    }
    let mut failed = 0u64;
    for (path, result) in paths.iter().zip(results) {
        let result = result.expect("every file has its result");
        if let Err(e) = result.and_then(|hash| print_hash_output(hash, path, args, out)) {
            failed = failed.saturating_add(1);
            let _ = out.flush();
            eprintln_safe!("{}: {}: {}", NAME, path.to_string_lossy(), e);
            if out.is_poisoned() {
                break;
            }
        }
    }
    failed
}

/// Read all of `file` into `bytes` until a waiting read returns `Ok(0)`:
/// asks for `says + 1` bytes first (a positive short read is NOT EOF, so
/// the loop continues until `Ok(0)` or the initial buffer fills, then
/// `read_to_end` collects any bytes past `says`; BLAKE3 #424).
fn read_to_its_end(file: &mut impl Read, says: usize, bytes: &mut Vec<u8>) -> io::Result<()> {
    bytes.resize(says + 1, 0);
    let mut got = 0;
    while got <= says {
        match file.read(&mut bytes[got..]) {
            Ok(0) => {
                bytes.truncate(got);
                return Ok(());
            }
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    file.read_to_end(bytes)?;
    Ok(())
}

/// The hash of the input at `path` (`-`: standard input), and its
/// outboard appended to `nodes`. A file under MAP_LEN is read into the
/// thread's room by one read; a longer one that the page cache does not
/// hold is read directly, where the system has such reads (Linux); any
/// other is mapped where it can be (the mapping then held, as every
/// mapping b3sum makes: `hold`), else read whole into memory.
fn outboard_of(
    args: &Args,
    path: &Path,
    stdin_ctx: &StdinContext,
    threads: usize,
    rooms: &mut outboard::Rooms,
    nodes: &mut Vec<u8>,
) -> err::Result<blake3::Hash> {
    let mode = args.mode;
    let bytes = &mut rooms.bytes;
    bytes.clear();
    match stdin_ctx.classify(path) {
        InputRole::StdinDash => {
            // The key was standard input, read to its end: no message is
            // left there, and its tree would be the empty message's.
            if matches!(mode, blake3::Mode::Keyed(_)) {
                bail!("Cannot open `-` in keyed mode");
            }
            #[cfg(unix)]
            if let Ok(meta) = file_end::fstat_fd(0) {
                file_end::ensure_byte_stream_fd(0, meta.mode_type)?;
            }
            file_end::read_stream_to_end(&mut io::stdin().lock(), bytes)?;
            return Ok(outboard::outboard(mode, bytes, threads, &mut rooms.cvs, nodes));
        }
        #[cfg(unix)]
        InputRole::StdinStreamAlias => {
            if matches!(mode, blake3::Mode::Keyed(_)) {
                bail!("Cannot open `-` in keyed mode");
            }
            if let Ok(meta) = file_end::fstat_fd(0) {
                file_end::ensure_byte_stream_fd(0, meta.mode_type)?;
            }
            file_end::read_stream_to_end(&mut io::stdin().lock(), bytes)?;
            return Ok(outboard::outboard(mode, bytes, threads, &mut rooms.cvs, nodes));
        }
        #[cfg(target_os = "macos")]
        InputRole::DarwinRegularDevFd => {
            use std::os::fd::AsRawFd;
            let file = File::open(path)?;
            let fd = file.as_raw_fd();
            let meta = file_end::fstat_fd(fd)?;
            file_end::ensure_byte_stream_fd(fd, meta.mode_type)?;
            if meta.len >= MAP_LEN as u64
                && let Some(map) = args.map(&file)
            {
                let hash = outboard::outboard(mode, &map, threads, &mut rooms.cvs, nodes);
                hold(map);
                return Ok(hash);
            }
            file_end::pread_all_into(fd, bytes)?;
            return Ok(outboard::outboard(mode, bytes, threads, &mut rooms.cvs, nodes));
        }
        InputRole::Path => {}
    }
    let mut file = File::open(path)?;
    let meta = file.metadata()?;
    #[cfg(unix)]
    if meta.is_file() && meta.len() < MAP_LEN as u64 {
        read_to_its_end(&mut file, usize::try_from(meta.len()).expect("len < MAP_LEN"), bytes)?;
        return Ok(outboard::outboard(mode, bytes, threads, &mut rooms.cvs, nodes));
    }
    #[cfg(target_os = "linux")]
    if meta.is_file()
        && let Some(hash) = outboard::outboard_direct(mode, &file, meta.len(), threads, &mut rooms.piece, &mut rooms.cvs, nodes)
    {
        return Ok(hash);
    }
    #[cfg(unix)]
    if meta.len() > 0
        && let Some(map) = args.map(&file)
    {
        let hash = outboard::outboard(mode, &map, threads, &mut rooms.cvs, nodes);
        hold(map);
        return Ok(hash);
    }
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        if let Ok(fd_meta) = file_end::fstat_fd(file.as_raw_fd()) {
            file_end::ensure_byte_stream_fd(file.as_raw_fd(), fd_meta.mode_type)?;
        }
    }
    if meta.is_file() {
        file.read_to_end(bytes)?;
    } else {
        file_end::read_stream_to_end(&mut file, bytes)?;
    }
    Ok(outboard::outboard(mode, bytes, threads, &mut rooms.cvs, nodes))
}

fn hash_inputs(args: &Args, stdin_ctx: &StdinContext, out: &mut OutBuf) -> u64 {
    if let Some(dir) = args.outboard() {
        return outboard_inputs(args, dir, stdin_ctx, out);
    }
    if args.is_standard_32() {
        let mut failed = 0u64;
        hash_paths_32(args, &args.file_args, stdin_ctx, None, &mut |idx, res| {
            let path = &args.file_args[idx];
            if let Err(e) = res.and_then(|h| print_hash_output(h, path, args, out)) {
                failed = failed.saturating_add(1);
                let _ = out.flush();
                eprintln_safe!("{}: {}: {}", NAME, path.to_string_lossy(), e);
                if out.is_poisoned() {
                    return false;
                }
            }
            true
        });
        return failed;
    }

    let total = threads().get();
    let max_workers = if args.num_threads().is_some() { max_file_threads(total) } else { total };
    let threads = if args.file_args.len() == 1 || stdin_named_twice(&args.file_args, stdin_ctx) { 1 } else { max_workers.min(args.file_args.len()) };
    hash_inputs_with(args, threads, out, |args, next, done| hash_some(args, stdin_ctx, next, done), print_output)
}

fn hash_inputs_with<O: Send>(
    args: &Args,
    threads: usize,
    out: &mut OutBuf,
    work: impl Fn(&Args, &std::sync::atomic::AtomicUsize, &mut dyn FnMut(usize, err::Result<O>) -> bool) + Sync,
    print_one: impl Fn(O, &Path, &Args, &mut OutBuf) -> err::Result<()>,
) -> u64 {
    let _file_threads_guard = args.num_threads().map(|_| FileThreadsGuard::enter(threads));
    let paths = &args.file_args;
    let next = std::sync::atomic::AtomicUsize::new(0);
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        for _ in 1..threads {
            if next.load(std::sync::atomic::Ordering::Relaxed) >= paths.len() {
                break;
            }
            let (next, done_tx, work) = (&next, done_tx.clone(), &work);
            scope.spawn(move || work(args, next, &mut |i, output| done_tx.send((i, output)).is_ok()));
        }
        drop(done_tx);
        let mut failed = 0u64;
        let mut printed = 0usize;
        let mut results: Vec<Option<err::Result<O>>> = (0..paths.len()).map(|_| None).collect();
        let mut take = |i: usize, output: err::Result<O>| {
            results[i] = Some(output);
            while printed < paths.len() {
                let Some(item) = results[printed].take() else { break };
                if let Err(e) = item.and_then(|output| print_one(output, &paths[printed], args, out)) {
                    failed = failed.saturating_add(1);
                    let _ = out.flush();
                    eprintln_safe!("{}: {}: {}", NAME, paths[printed].to_string_lossy(), e);
                    if out.is_poisoned() {
                        return false;
                    }
                }
                printed += 1;
            }
            true
        };
        work(args, &next, &mut take);
        for (i, output) in done_rx {
            if !take(i, output) {
                break;
            }
        }
        failed
    })
}

fn hash_some(
    args: &Args,
    stdin_ctx: &StdinContext,
    next: &std::sync::atomic::AtomicUsize,
    done: &mut dyn FnMut(usize, err::Result<blake3::OutputReader>) -> bool,
) {
    loop {
        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if i >= args.file_args.len() || !done(i, hash_path(args, &args.file_args[i], stdin_ctx)) {
            return;
        }
    }
}

fn emit_checked_entry(parsed: ParsedCheckLine, res: err::Result<blake3::Hash>, args: &Args, out: &mut OutBuf) -> err::Result<bool> {
    let ParsedCheckLine { file_string, is_escaped, expected_hash, .. } = parsed;
    let display = if is_escaped { "\\".to_string() + &file_string } else { file_string };
    let found_hash = match res {
        Ok(h) => h,
        Err(e) => {
            out.write_bytes(format!("{}: FAILED ({})\n", display, e).as_bytes())?;
            return Ok(false);
        }
    };
    // Constant-time comparison (`blake3::Hash::eq`).
    if expected_hash == found_hash {
        if !args.quiet() {
            out.write_bytes(display.as_bytes())?;
            out.write_bytes(b": OK\n")?;
        }
        Ok(true)
    } else {
        out.write_bytes(display.as_bytes())?;
        out.write_bytes(b": FAILED\n")?;
        Ok(false)
    }
}

/// Hard byte cap on one `--check` line (1 MiB, well above any OS path limit +
/// escapes + digest) so pointing `--check` at a giant newline-free file cannot
/// exhaust memory before reporting a parse error.
const MAX_CHECK_LINE_BYTES: usize = 1 << 20;

/// Read one line from `reader` with a hard byte bound `max_bytes`, draining
/// any oversized remainder up to `\n` or EOF without allocating, and decoding
/// UTF-8 per record so one invalid-UTF-8 line does not discard earlier valid
/// entries in the same batch.
fn read_check_line_bounded(reader: &mut impl BufRead, raw: &mut Vec<u8>, max_bytes: usize) -> io::Result<Option<Result<String, err::Error>>> {
    raw.clear();
    let mut too_long = false;
    let mut saw_any = false;
    loop {
        let avail = match reader.fill_buf() {
            Ok(buf) => buf,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if avail.is_empty() {
            break;
        }
        saw_any = true;
        if let Some(pos) = avail.iter().position(|&b| b == b'\n') {
            let take = pos + 1;
            if !too_long {
                if raw.len() + take > max_bytes {
                    too_long = true;
                } else {
                    raw.extend_from_slice(&avail[..take]);
                }
            }
            reader.consume(take);
            break;
        }
        let n = avail.len();
        if !too_long {
            if raw.len() + n > max_bytes {
                too_long = true;
            } else {
                raw.extend_from_slice(avail);
            }
        }
        reader.consume(n);
    }
    if !saw_any {
        return Ok(None);
    }
    if too_long {
        return Ok(Some(Err(format_err!("Check line exceeds maximum length"))));
    }
    match std::str::from_utf8(raw) {
        Ok(s) => Ok(Some(Ok(s.to_owned()))),
        Err(_) => Ok(Some(Err(format_err!("Invalid UTF-8 in check line")))),
    }
}

fn check_one_checkfile(path: &Path, args: &Args, stdin_ctx: &StdinContext, files_failed: &mut u64, out: &mut OutBuf) -> err::Result<()> {
    const CHECK_CHUNK: usize = 4096;
    enum Entry {
        ParseErr(err::Error),
        Own(ParsedCheckLine),
        Stdin(ParsedCheckLine),
        File { parsed: ParsedCheckLine, batch_idx: usize },
    }

    let mut file;
    let stdin;
    let mut stdin_lock;
    #[cfg(target_os = "macos")]
    let mut darwin_pread = file_end::PreadReader::new(0, 0);
    let mut bufreader: io::BufReader<&mut dyn Read>;
    #[cfg(unix)]
    let own: Option<StreamId>;
    #[cfg(not(unix))]
    let own: Option<()> = None;
    let checkfile_is_stdin;
    #[cfg(target_os = "macos")]
    let mut darwin_stdin_dash_pread = false;
    let check_buf_cap = min_packet_read_len().max(64 * 1024);
    match stdin_ctx.classify(path) {
        InputRole::StdinDash => {
            checkfile_is_stdin = true;
            #[cfg(unix)]
            let _stdin_meta = {
                let meta = file_end::fstat_fd(0)?;
                file_end::ensure_byte_stream_fd(0, meta.mode_type)?;
                own = (meta.kind == file_end::FileKind::Stream).then_some(meta.id);
                meta
            };
            #[cfg(target_os = "macos")]
            {
                if _stdin_meta.kind == file_end::FileKind::Regular {
                    // Sound: `lseek(SEEK_CUR)` queries fd 0's current offset without touching memory.
                    let start = unsafe { libc::lseek(0, 0, libc::SEEK_CUR) }.max(0) as u64;
                    darwin_pread = file_end::PreadReader::new(0, start);
                    darwin_stdin_dash_pread = true;
                    bufreader = io::BufReader::with_capacity(check_buf_cap, &mut darwin_pread);
                } else {
                    stdin = io::stdin();
                    stdin_lock = stdin.lock();
                    bufreader = io::BufReader::with_capacity(check_buf_cap, &mut stdin_lock);
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                stdin = io::stdin();
                stdin_lock = stdin.lock();
                bufreader = io::BufReader::with_capacity(check_buf_cap, &mut stdin_lock);
            }
        }
        #[cfg(unix)]
        InputRole::StdinStreamAlias => {
            checkfile_is_stdin = true;
            let meta = file_end::fstat_fd(0)?;
            file_end::ensure_byte_stream_fd(0, meta.mode_type)?;
            own = (meta.kind == file_end::FileKind::Stream).then_some(meta.id);
            stdin = io::stdin();
            stdin_lock = stdin.lock();
            bufreader = io::BufReader::with_capacity(check_buf_cap, &mut stdin_lock);
        }
        #[cfg(target_os = "macos")]
        InputRole::DarwinRegularDevFd => {
            use std::os::fd::AsRawFd;
            checkfile_is_stdin = false;
            own = None;
            file = File::open(path)?;
            let meta = file_end::fstat_fd(file.as_raw_fd())?;
            file_end::ensure_byte_stream_fd(file.as_raw_fd(), meta.mode_type)?;
            darwin_pread = file_end::PreadReader::new(file.as_raw_fd(), 0);
            bufreader = io::BufReader::with_capacity(check_buf_cap, &mut darwin_pread);
        }
        InputRole::Path => {
            file = File::open(path)?;
            #[cfg(unix)]
            let _meta = {
                use std::os::fd::AsRawFd;
                let meta = file_end::fstat_fd(file.as_raw_fd())?;
                file_end::ensure_byte_stream_fd(file.as_raw_fd(), meta.mode_type)?;
                own = (meta.kind == file_end::FileKind::Stream).then_some(meta.id);
                checkfile_is_stdin = own.is_some() && own == stdin_ctx.stdin_stream_id();
                meta
            };
            #[cfg(not(unix))]
            {
                checkfile_is_stdin = false;
            }
            #[cfg(target_os = "macos")]
            {
                use std::os::fd::AsRawFd;
                if _meta.kind == file_end::FileKind::Regular {
                    // On macOS, opening `/dev/fd/N` shares the underlying open
                    // file description offset (`fd(4)`). Reading a regular
                    // checkfile via positioned `pread` prevents any `-` or
                    // `/dev/fd/M` entry from advancing the parser cursor behind
                    // its back (#515).
                    // Sound: `lseek(SEEK_CUR)` queries the current offset without touching memory.
                    let start = unsafe { libc::lseek(file.as_raw_fd(), 0, libc::SEEK_CUR) }.max(0) as u64;
                    darwin_pread = file_end::PreadReader::new(file.as_raw_fd(), start);
                    bufreader = io::BufReader::with_capacity(check_buf_cap, &mut darwin_pread);
                } else {
                    bufreader = io::BufReader::with_capacity(check_buf_cap, &mut file);
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                bufreader = io::BufReader::with_capacity(check_buf_cap, &mut file);
            }
        }
    }

    // The checkfile's own stream, by any name (few checkfiles are
    // streams: for those every entry is looked at, AND the actual opened
    // descriptor is checked again against `own` before any bytes are read).
    let is_the_checkfile = |entry: &Path| {
        match stdin_ctx.classify(entry) {
            InputRole::StdinDash => return checkfile_is_stdin,
            #[cfg(unix)]
            InputRole::StdinStreamAlias => return checkfile_is_stdin,
            _ => {}
        }
        #[cfg(unix)]
        if own.is_some_and(|own| file_end::path_matches_stream(entry, own)) {
            return true;
        }
        false
    };

    let mut raw_line = Vec::new();
    let mut entries = Vec::new();
    let mut batch_paths = Vec::new();
    let mut eof = false;
    let mut saw_line = false;

    while !eof {
        entries.clear();
        batch_paths.clear();
        while entries.len() < CHECK_CHUNK {
            let Some(line_res) = read_check_line_bounded(&mut bufreader, &mut raw_line, MAX_CHECK_LINE_BYTES)? else {
                eof = true;
                break;
            };
            saw_line = true;
            let parsed_res = line_res.and_then(|line| parse_check_line(&line));
            match parsed_res {
                Err(e) => entries.push(Entry::ParseErr(e)),
                Ok(parsed) if is_the_checkfile(&parsed.file_path) => entries.push(Entry::Own(parsed)),
                Ok(parsed) if matches!(stdin_ctx.classify(&parsed.file_path), InputRole::StdinDash) => entries.push(Entry::Stdin(parsed)),
                #[cfg(unix)]
                Ok(parsed) if matches!(stdin_ctx.classify(&parsed.file_path), InputRole::StdinStreamAlias) => entries.push(Entry::Stdin(parsed)),
                Ok(parsed) => {
                    let batch_idx = batch_paths.len();
                    batch_paths.push(parsed.file_path.clone());
                    entries.push(Entry::File { parsed, batch_idx });
                }
            }
        }
        if entries.is_empty() {
            break;
        }

        let mut batch_results: Vec<Option<err::Result<blake3::Hash>>> = (0..batch_paths.len()).map(|_| None).collect();
        hash_paths_32(args, &batch_paths, stdin_ctx, own, &mut |idx, res| {
            batch_results[idx] = Some(res);
            true
        });

        for entry in entries.drain(..) {
            let ok = match entry {
                Entry::ParseErr(e) => {
                    let _ = out.flush();
                    eprintln_safe!("{}: {}", NAME, e);
                    false
                }
                Entry::Own(parsed) => {
                    let what = if checkfile_is_stdin { "standard input is the checkfile; it" } else { "this checkfile is a stream, and" };
                    emit_checked_entry(parsed, Err(format_err!("{what} cannot also be a file to check")), args, out)?
                }
                Entry::Stdin(parsed) => {
                    let res = hash_path_hash(args, &parsed.file_path, stdin_ctx, own);
                    emit_checked_entry(parsed, res, args, out)?
                }
                Entry::File { parsed, batch_idx } => {
                    let res = batch_results[batch_idx].take().expect("every batch path hashed");
                    emit_checked_entry(parsed, res, args, out)?
                }
            };
            if !ok {
                *files_failed = files_failed.saturating_add(1);
            }
        }
    }
    #[cfg(target_os = "macos")]
    if darwin_stdin_dash_pread {
        drop(bufreader);
        // Sound: `lseek` updates fd 0's offset to match the consumed checkfile bytes.
        unsafe { libc::lseek(0, darwin_pread.offset() as libc::off_t, libc::SEEK_SET) };
    }
    ensure!(saw_line, "{}: no checksum entries", path.display());
    Ok(())
}

#[cfg(unix)]
fn explain_sigbus() {
    static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    extern "C" fn on_sigbus(_: libc::c_int) {
        if SAID.swap(true, std::sync::atomic::Ordering::SeqCst) {
            loop {
                // Sound: calls a signal handler may make (pause, write,
                // _exit), on a constant message.
                unsafe { libc::pause() };
            }
        }
        const MESSAGE: &[u8] = b"b3sum: a bus error while reading a mapped file; most likely another program truncated it, so no digest of it would be right. Hash the files again once nothing writes to them.\n";
        unsafe {
            libc::write(2, MESSAGE.as_ptr().cast(), MESSAGE.len());
            libc::_exit(1);
        }
    }
    let handler: extern "C" fn(libc::c_int) = on_sigbus;
    // Sound: the handler makes only calls a signal handler may make, and
    // SIGBUS is this program's to handle.
    unsafe { libc::signal(libc::SIGBUS, handler as libc::sighandler_t) };
}

fn main() -> err::Result<()> {
    #[cfg(unix)]
    // Sound: before any thread starts; a broken pipe then ends b3sum, as it
    // does other tools.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL)
    };
    #[cfg(unix)]
    explain_sigbus();
    blake3::initialize();
    let args = Args::parse()?;
    configure_threads(&args);
    let stdin_ctx = StdinContext::current();
    let mut out = OutBuf::new();
    let mut files_failed = 0u64;
    if args.check() {
        for path in &args.file_args {
            if let Err(e) = check_one_checkfile(path, &args, &stdin_ctx, &mut files_failed, &mut out) {
                let _ = out.flush();
                return Err(e);
            }
        }
    } else {
        files_failed = hash_inputs(&args, &stdin_ctx, &mut out);
    }
    out.flush()?;
    std::io::stdout().flush()?;
    if args.check() && files_failed > 0 {
        eprintln_safe!("{}: WARNING: {} computed checksum{} did NOT match", NAME, files_failed, if files_failed == 1 { "" } else { "s" },);
    }
    let status = if files_failed > 0 { 1 } else { 0 };
    use std::io::Write;
    let _ = std::io::stderr().flush();
    // Sound: ends the process; standard output and error were flushed above.
    unsafe { libc::_exit(status) }
}

#[cfg(test)]
mod test {
    include!("../../tools/require_nextest.rs");
    use crate::args::{ParseOutcome, parse_from};

    #[test]
    fn test_args_basics_and_conflicts() {
        blake3::initialize();
        assert_eq!(parse_from(["b3sum", "--help"]), Ok(ParseOutcome::Help));
        assert_eq!(parse_from(["b3sum", "-h"]), Ok(ParseOutcome::Help));
        assert_eq!(parse_from(["b3sum", "--version"]), Ok(ParseOutcome::Version));
        assert_eq!(parse_from(["b3sum", "-V"]), Ok(ParseOutcome::Version));

        let Ok(ParseOutcome::Parsed(p)) = parse_from(["b3sum", "-l64", "--seek=8", "--num-threads", "2", "--", "-weird"]) else {
            panic!("expected parsed");
        };
        assert_eq!(p.length, 64);
        assert_eq!(p.seek, 8);
        assert_eq!(p.num_threads.unwrap().get(), 2);
        assert_eq!(p.file, vec![std::path::PathBuf::from("-weird")]);

        // Conflicts & requirements
        assert!(parse_from(["b3sum", "--keyed"]).is_err());
        assert!(parse_from(["b3sum", "--keyed", "--derive-key", "ctx", "f"]).is_err());
        assert!(parse_from(["b3sum", "--quiet"]).is_err());
        assert!(parse_from(["b3sum", "--check", "--seek", "0"]).is_err());
        assert!(parse_from(["b3sum", "--check", "--length", "32"]).is_err());
        assert!(parse_from(["b3sum", "--check", "--raw"]).is_err());
        assert!(parse_from(["b3sum", "--check", "--tag"]).is_err());
        assert!(parse_from(["b3sum", "--check", "--no-names"]).is_err());
        assert!(parse_from(["b3sum", "--check", "--keyed", "f"]).is_err());
        assert!(parse_from(["b3sum", "--check", "--derive-key", "x"]).is_err());
        assert!(parse_from(["b3sum", "--check", "--outboard", "d"]).is_err());
        assert!(parse_from(["b3sum", "--outboard", "d", "--length", "32"]).is_err());
        assert!(parse_from(["b3sum", "--outboard", "d", "--seek", "0"]).is_err());
        assert!(parse_from(["b3sum", "--outboard", "d", "--raw"]).is_err());
        assert!(parse_from(["b3sum", "--outboard", "d", "--derive-key", "x"]).is_err());
        // Check + quiet IS allowed; outboard + keyed + file IS allowed
        assert!(matches!(parse_from(["b3sum", "-c", "--quiet"]), Ok(ParseOutcome::Parsed(_))));
        assert!(matches!(parse_from(["b3sum", "--outboard", "d", "--keyed", "f"]), Ok(ParseOutcome::Parsed(_))));
        // Duplicate flags rejected
        assert!(parse_from(["b3sum", "--raw", "--raw"]).is_err());
        assert!(parse_from(["b3sum", "-l", "16", "--length", "32"]).is_err());
        // Missing / invalid values
        assert!(parse_from(["b3sum", "--length"]).is_err());
        assert!(parse_from(["b3sum", "--num-threads", "0"]).is_err());
        assert!(parse_from(["b3sum", "--unknown"]).is_err());
    }
}
