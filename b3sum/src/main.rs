//! b3sum: BLAKE3's command-line hasher.
//!
//! On Linux, multi-file runs and uncached large files stream through
//! `io_uring` and `blake3::throughput::Pipeline` (`linux_pipe`):
//! - Small files (`< 512 KiB`) are read asynchronously through `io_uring` into
//!   recycled buffers and gathered into 64-member multi-buffer SIMD tasks on
//!   the worker pool, with zero steady-state heap allocation.
//! - Cached large files (`>= 512 KiB`, `mincore` resident) are mapped and
//!   loaned zero-copy to `Pipeline::lent`, then held mapped up to `HOLD_PAGES`
//!   until exit to avoid `munmap` TLB shootdowns.
//! - Cold large files (`>= 512 KiB`, not resident in page cache) stream
//!   directly from storage into 64 KiB `Pipeline::room()`s via batched
//!   `IORING_OP_READ`s, overlapping NVMe DMA with all-core subtree hashing.
//!
//! Single small files read into a stack buffer with zero `mmap` or ring setup,
//! and standard 32-byte digests format on the stack into a 32 KiB buffered
//! writer without cloning an 18 KiB `Hasher` per file.

use anyhow::{bail, ensure};
use clap::Parser;
use std::cmp;
use std::fs::File;
use std::io;
use std::io::prelude::*;
use std::path::{Path, PathBuf};

#[cfg(test)]
mod unit_tests;

const NAME: &str = "b3sum";

#[cfg(target_os = "linux")]
mod linux_pipe;
mod outboard;
#[cfg(target_os = "linux")]
mod uring;

const DERIVE_KEY_ARG: &str = "derive_key";
const KEYED_ARG: &str = "keyed";
const LENGTH_ARG: &str = "length";
const NO_NAMES_ARG: &str = "no_names";
const RAW_ARG: &str = "raw";
const TAG_ARG: &str = "tag";
const CHECK_ARG: &str = "check";

#[derive(Parser)]
#[command(
    version = blake3::BUILD_VERSION,
    max_term_width(100),
    after_help = "More information: b3sum/README.md at https://github.com/servil-software-factory/BLAKE3/blob/servil/b3sum/README.md"
)]
struct Inner {
    /// Files to hash, or checkfiles to check
    file: Vec<PathBuf>,

    /// Use the keyed mode, reading the 32-byte key from stdin
    #[arg(long, requires("file"))]
    keyed: bool,

    /// Use the key derivation mode, with the given context string
    #[arg(long, value_name("CONTEXT"), conflicts_with(KEYED_ARG))]
    derive_key: Option<String>,

    /// The number of output bytes, before hex or sigil encoding
    #[arg(
        short,
        long,
        default_value_t = blake3::OUT_LEN as u64,
        value_name("LEN")
    )]
    length: u64,

    /// The starting output byte offset, before hex or sigil encoding
    #[arg(long, default_value_t = 0, value_name("SEEK"))]
    seek: u64,

    /// The most threads hashing at once [default: as fast as b3sum can]
    #[arg(long, value_name("NUM"))]
    num_threads: Option<std::num::NonZeroUsize>,

    /// No effect: b3sum chooses between reading and mapping each file
    #[arg(long)]
    no_mmap: bool,

    /// Omit filenames in the output
    #[arg(long)]
    no_names: bool,

    /// Write raw output bytes to stdout, rather than hex
    #[arg(long)]
    raw: bool,

    /// Write each hash as a sigil (25 characters for 32 bytes; see
    /// johnservil/sigil), rather than hex; with --check, read sigils as
    /// well as hex
    #[arg(long, conflicts_with(RAW_ARG))]
    sigilcode: bool,

    /// Output BSD-style checksums: BLAKE3 ([FILE]) = [HASH]
    #[arg(long)]
    tag: bool,

    /// Read BLAKE3 sums from the [FILE]s and check them
    #[arg(
        short,
        long,
        conflicts_with(DERIVE_KEY_ARG),
        conflicts_with(KEYED_ARG),
        conflicts_with(LENGTH_ARG),
        conflicts_with(RAW_ARG),
        conflicts_with(TAG_ARG),
        conflicts_with(NO_NAMES_ARG)
    )]
    check: bool,

    /// Skip printing OK for each checked file
    #[arg(long, requires(CHECK_ARG))]
    quiet: bool,

    /// Also write each file's outboard to DIR/<HASH>.obao4: its BLAKE3
    /// tree over 16 KiB chunk groups in bao-tree's pre-order form, for
    /// verified streaming
    #[arg(
        long,
        value_name("DIR"),
        conflicts_with(CHECK_ARG),
        conflicts_with(DERIVE_KEY_ARG),
        conflicts_with(LENGTH_ARG),
        conflicts_with(RAW_ARG),
        conflicts_with("seek")
    )]
    outboard: Option<PathBuf>,
}

struct Args {
    inner: Inner,
    file_args: Vec<PathBuf>,
    mode: blake3::Mode<'static>,
    base_hasher: blake3::convenience::Hasher,
}

impl Args {
    fn parse() -> anyhow::Result<Self> {
        let inner = Inner::parse_from(std::env::args_os());
        let file_args = if !inner.file.is_empty() { inner.file.clone() } else { vec!["-".into()] };
        if inner.raw && file_args.len() > 1 {
            bail!("Only one filename can be provided when using --raw");
        }
        if inner.sigilcode && inner.length == 0 && !inner.check {
            bail!("--sigilcode writes at least one output byte; --length is 0");
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
            sigilcode: false,
            tag: false,
            check: false,
            quiet: false,
            outboard: None,
        };
        Self { inner, file_args, mode: blake3::Mode::Plain, base_hasher: blake3::convenience::Hasher::new(blake3::Mode::Plain, None) }
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

    fn sigilcode(&self) -> bool {
        self.inner.sigilcode
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

    fn keyed(&self) -> bool {
        self.inner.keyed
    }

    fn quiet(&self) -> bool {
        self.inner.quiet
    }
}

/// 32 KiB buffered writer over stdout so multi-file runs make one `write`
/// syscall per 32 KiB rather than one `LineWriter` flush per filename, while
/// still hitting `SIGPIPE` on closed pipes (`test_reader_gone`).
pub(crate) struct OutBuf {
    buf: Vec<u8>,
}

const OUT_BUF_CAP: usize = 32 * 1024;

impl OutBuf {
    fn new() -> Self {
        Self { buf: Vec::with_capacity(OUT_BUF_CAP) }
    }

    #[inline]
    fn write_bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.buf.len() + bytes.len() > OUT_BUF_CAP {
            self.flush()?;
            if bytes.len() >= OUT_BUF_CAP {
                return write_all_stdout(bytes);
            }
        }
        self.buf.extend_from_slice(bytes);
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.buf.is_empty() {
            write_all_stdout(&self.buf)?;
            self.buf.clear();
        }
        Ok(())
    }
}

fn write_all_stdout(mut bytes: &[u8]) -> io::Result<()> {
    #[cfg(unix)]
    {
        while !bytes.is_empty() {
            let rc = unsafe { libc::write(1, bytes.as_ptr().cast(), bytes.len()) };
            if rc < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            bytes = &bytes[rc as usize..];
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let stdout = io::stdout();
        let mut handle = stdout.lock();
        handle.write_all(bytes)
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

/// Fast standard-mode (`seek == 0 && len <= 32`) single-path hasher: avoids
/// cloning an 18 KiB `Hasher` on warm mappings and small files, uses a 64 KiB
/// stack buffer for small files (zero `8 MiB` `BUFFERS` fault), and streams
/// cold large Linux files via `io_uring` + `Pipeline`.
pub(crate) fn hash_path_hash(args: &Args, path: &Path) -> anyhow::Result<blake3::Hash> {
    if path == Path::new("-") {
        if args.keyed() {
            bail!("Cannot open `-` in keyed mode");
        }
        let mut hasher = args.base_hasher.clone();
        update_from(&mut hasher, io::stdin())?;
        return Ok(hasher.finalize());
    }

    let mut file = File::open(path)?;
    if let Ok(meta) = file.metadata()
        && meta.is_file()
    {
        let size = meta.len();
        if size < MAP_LEN as u64 {
            let cap = (size as usize + 1).max(4096);
            let mut buf = Vec::with_capacity(cap);
            let n = read_all_uninit(&mut file, &mut buf)?;
            return Ok(hash_slice_32(args.mode, &buf[..n]));
        }

        #[cfg(unix)]
        if let Ok(len) = usize::try_from(size)
            && len >= MAP_LEN
            && let Ok(map) = unsafe { memmap2::Mmap::map(&file) }
        {
            if resident(&map) {
                let h = hash_slice_32(args.mode, &map);
                hold(map);
                return Ok(h);
            }
            #[cfg(target_os = "linux")]
            if threads().get() == 1 {
                drop(map);
                if size >= POOL_START_LEN as u64
                    && let Some(res) = linux_pipe::hash_single_cold_large_file_1t(args, &file, size)
                {
                    return Ok(res?);
                }
                let mut buf = Vec::with_capacity(len + 1);
                let n = read_all_uninit(&mut file, &mut buf)?;
                return Ok(hash_slice_32(args.mode, &buf[..n]));
            } else {
                // Multi-threaded single cold file (`>= 512 KiB`): start pool and fault
                // disjoint subtrees in parallel across all NUMA nodes and NVMe RAID stripes.
                init_pool_once(pool_threads().get());
                let h = hash_slice_32(args.mode, &map);
                hold(map);
                return Ok(h);
            }
            #[cfg(not(target_os = "linux"))]
            drop(map);
        }
    }

    let mut hasher = args.base_hasher.clone();
    update_from(&mut hasher, file)?;
    Ok(hasher.finalize())
}

fn hash_path(args: &Args, path: &Path) -> anyhow::Result<blake3::OutputReader> {
    let mut hasher = args.base_hasher.clone();
    if path == Path::new("-") {
        if args.keyed() {
            bail!("Cannot open `-` in keyed mode");
        }
        update_from(&mut hasher, io::stdin())?;
    } else {
        let mut file = File::open(path)?;
        if let Ok(meta) = file.metadata()
            && meta.is_file()
            && meta.len() < 64 * 1024
        {
            let mut stack_buf = [0u8; 64 * 1024];
            let n = fill(&mut file, &mut stack_buf)?;
            update(&mut hasher, &stack_buf[..n]);
            if n == stack_buf.len() {
                update_from(&mut hasher, file)?;
            }
        } else {
            #[cfg(unix)]
            if let Some(map) = mapped(&file) {
                update(&mut hasher, &map);
                hold(map);
            } else {
                update_from(&mut hasher, file)?;
            }
            #[cfg(not(unix))]
            update_from(&mut hasher, file)?;
        }
    }
    let mut output_reader = hasher.finalize_xof();
    output_reader.set_position(args.seek());
    Ok(output_reader)
}

#[cfg(unix)]
const HOLD_PAGES: u64 = 1 << 18;

#[cfg(unix)]
pub(crate) fn hold(map: memmap2::Mmap) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static HELD: AtomicU64 = AtomicU64::new(0);
    let page = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).expect("the page size is positive");
    let pages = (map.len() as u64).div_ceil(page);
    if HELD.fetch_add(pages, Ordering::Relaxed) + pages <= HOLD_PAGES {
        std::mem::forget(map);
    } else {
        HELD.fetch_sub(pages, Ordering::Relaxed);
    }
}

#[cfg(unix)]
fn mapped(file: &File) -> Option<memmap2::Mmap> {
    let len = usize::try_from(file.metadata().ok()?.len()).ok()?;
    if len < MAP_LEN {
        return None;
    }
    let map = unsafe { memmap2::Mmap::map(file) }.ok()?;
    if cfg!(target_os = "linux") {
        return Some(map);
    }
    resident(&map).then_some(map)
}

#[cfg(unix)]
pub(crate) fn resident(map: &memmap2::Mmap) -> bool {
    let mut vec = 0u8;
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

static POOL_ONCE: std::sync::Once = std::sync::Once::new();

pub(crate) fn init_pool_once(n: usize) {
    POOL_ONCE.call_once(|| {
        blake3::initialize_multithreaded(std::num::NonZeroUsize::new(n));
    });
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

fn read_all_uninit(file: &mut File, buf: &mut Vec<u8>) -> io::Result<usize> {
    let mut at = 0usize;
    loop {
        if at == buf.capacity() {
            unsafe { buf.set_len(at) };
            buf.reserve(at.max(4096));
            unsafe { buf.set_len(0) };
        }
        let avail = buf.capacity() - at;
        #[cfg(unix)]
        let rc = {
            use std::os::fd::AsRawFd;
            unsafe { libc::read(file.as_raw_fd(), buf.as_mut_ptr().add(at).cast(), avail) }
        };
        #[cfg(not(unix))]
        let rc = {
            buf.resize(at + avail, 0);
            match file.read(&mut buf[at..]) {
                Ok(n) => n as isize,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => -1,
                Err(e) => return Err(e),
            }
        };
        if rc == 0 {
            unsafe { buf.set_len(at) };
            return Ok(at);
        }
        if rc < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        at += rc as usize;
    }
}

static THREADS: std::sync::OnceLock<std::num::NonZeroUsize> = std::sync::OnceLock::new();

pub(crate) fn threads() -> std::num::NonZeroUsize {
    *THREADS.get_or_init(|| std::thread::available_parallelism().unwrap_or(std::num::NonZeroUsize::MIN))
}

static FILE_THREADS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);

pub(crate) fn pool_threads() -> std::num::NonZeroUsize {
    let file_threads = FILE_THREADS.load(std::sync::atomic::Ordering::Relaxed);
    std::num::NonZeroUsize::new(threads().get() - file_threads + 1).expect("at most threads() file threads")
}

fn update(hasher: &mut blake3::convenience::Hasher, input: &[u8]) {
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

fn fill(reader: &mut impl Read, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

fn update_from(hasher: &mut blake3::convenience::Hasher, mut reader: impl Read + Send) -> io::Result<()> {
    BUFFERS.with_borrow_mut(|[first, second]| {
        let mut len = fill(&mut reader, first)?;
        if len < PIECE {
            update(hasher, &first[..len]);
            return Ok(());
        }
        if threads().get() == 1 {
            loop {
                update(hasher, &first[..len]);
                if len < PIECE {
                    return Ok(());
                }
                len = fill(&mut reader, first)?;
            }
        }
        std::thread::scope(|scope| {
            let (free_tx, free_rx) = std::sync::mpsc::sync_channel::<&mut [u8]>(1);
            let (full_tx, full_rx) = std::sync::mpsc::sync_channel::<(&mut [u8], io::Result<usize>)>(1);
            let reader = &mut reader;
            scope.spawn(move || {
                for buffer in free_rx {
                    let read = fill(reader, buffer);
                    let last = !matches!(read, Ok(n) if n == buffer.len());
                    if full_tx.send((buffer, read)).is_err() || last {
                        break;
                    }
                }
            });
            free_tx.send(&mut second[..]).expect("the reader waits for a buffer");
            let mut current: &mut [u8] = &mut first[..];
            loop {
                update(hasher, current);
                let (next, read) = full_rx.recv().expect("the reader sends every piece");
                let len = read?;
                if len < next.len() {
                    update(hasher, &next[..len]);
                    return Ok(());
                }
                free_tx.send(current).expect("the reader waits for a buffer");
                current = next;
            }
        })
    })
}

fn write_hash_text(hash: &blake3::Hash, args: &Args, out: &mut OutBuf) -> anyhow::Result<()> {
    let take = args.len() as usize;
    if args.sigilcode() {
        let s = sigilcode::encode(&hash.as_bytes()[..take]);
        out.write_bytes(s.as_bytes())?;
    } else {
        let mut hex_buf = [0u8; 2 * blake3::OUT_LEN];
        encode_hex_stack(&hash.as_bytes()[..take], &mut hex_buf[..2 * take]);
        out.write_bytes(&hex_buf[..2 * take])?;
    }
    Ok(())
}

pub(crate) fn print_hash_output(hash: blake3::Hash, path: &Path, args: &Args, out: &mut OutBuf) -> anyhow::Result<()> {
    let take = args.len() as usize;
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

fn write_hex_output(mut output: blake3::OutputReader, args: &Args, out: &mut OutBuf) -> anyhow::Result<()> {
    let mut len = args.len();
    let mut block = [0u8; blake3::BLOCK_LEN];
    let mut hex_buf = [0u8; 2 * blake3::BLOCK_LEN];
    while len > 0 {
        output.fill(&mut block);
        let take_bytes = cmp::min(len, block.len() as u64) as usize;
        encode_hex_stack(&block[..take_bytes], &mut hex_buf[..2 * take_bytes]);
        out.write_bytes(&hex_buf[..2 * take_bytes])?;
        len -= take_bytes as u64;
    }
    Ok(())
}

fn write_sigil_output(mut output: blake3::OutputReader, args: &Args, out: &mut OutBuf) -> anyhow::Result<()> {
    let mut bytes = vec![0; args.len() as usize];
    output.fill(&mut bytes);
    let encoded = sigilcode::encode(&bytes);
    out.write_bytes(encoded.as_bytes())?;
    Ok(())
}

fn write_text_output(output: blake3::OutputReader, args: &Args, out: &mut OutBuf) -> anyhow::Result<()> {
    if args.sigilcode() { write_sigil_output(output, args, out) } else { write_hex_output(output, args, out) }
}

fn write_raw_output(output: blake3::OutputReader, args: &Args, out: &mut OutBuf) -> anyhow::Result<()> {
    out.flush()?;
    let mut output = output.take(args.len());
    let stdout = std::io::stdout();
    let mut handler = stdout.lock();
    std::io::copy(&mut output, &mut handler)?;
    Ok(())
}

fn print_output(output: blake3::OutputReader, path: &Path, args: &Args, out: &mut OutBuf) -> anyhow::Result<()> {
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

fn read_key_from_stdin() -> anyhow::Result<[u8; blake3::KEY_LEN]> {
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

fn hex_half_byte(c: char) -> anyhow::Result<u8> {
    if '0' <= c && c <= '9' {
        return Ok(c as u8 - b'0');
    }
    if 'a' <= c && c <= 'f' {
        return Ok(c as u8 - b'a' + 10);
    }
    bail!("Invalid hex");
}

fn check_for_invalid_characters(utf8_path: &str) -> anyhow::Result<()> {
    if utf8_path.contains('\0') {
        bail!("Null character in path");
    }
    if utf8_path.contains('\u{FFFD}') {
        bail!("Unicode replacement character in path");
    }
    Ok(())
}

fn unescape(mut path: &str) -> anyhow::Result<String> {
    let mut unescaped = String::with_capacity(2 * path.len());
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

fn parse_check_line(mut line: &str, sigils: bool) -> anyhow::Result<ParsedCheckLine> {
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
    if let Some((left, right)) = split_untagged_check_line(line_after_slash) {
        hash_text = left;
        file_str = right;
    } else if let Some((left, right)) = split_tagged_check_line(line_after_slash) {
        file_str = left;
        hash_text = right;
    } else {
        bail!("Invalid check line format");
    }

    let mut hash_bytes = [0; blake3::OUT_LEN];
    if sigils && !hash_text.chars().all(|c| c.is_ascii_hexdigit()) {
        let decoded = sigilcode::decode(hash_text, blake3::OUT_LEN).map_err(|e| anyhow::anyhow!("Invalid hash, read as a sigil: {e}"))?;
        hash_bytes.copy_from_slice(&decoded);
    } else {
        if !sigils && !hash_text.is_ascii() {
            bail!("Invalid hash: not hex (a sigil? check it with --sigilcode)");
        }
        ensure!(hash_text.len() == 2 * blake3::OUT_LEN, "Invalid hash length");
        let mut hex_chars = hash_text.chars();
        for byte in &mut hash_bytes {
            let high_char = hex_chars.next().unwrap();
            let low_char = hex_chars.next().unwrap();
            *byte = 16 * hex_half_byte(high_char)? + hex_half_byte(low_char)?;
        }
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
fn hash_paths_32(args: &Args, paths: &[PathBuf], on_ordered: &mut dyn FnMut(usize, anyhow::Result<blake3::Hash>) -> bool) {
    if paths.is_empty() {
        return;
    }
    if paths.len() == 1 {
        let _ = on_ordered(0, hash_path_hash(args, &paths[0]));
        return;
    }

    #[cfg(target_os = "linux")]
    if linux_pipe::applies_to_paths(args, paths) {
        linux_pipe::hash_paths(args, paths, on_ordered);
        return;
    }

    let threads = threads().get().min(paths.len());
    if args.num_threads().is_some() {
        FILE_THREADS.store(threads, std::sync::atomic::Ordering::Relaxed);
    }
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
                    if i >= paths.len() || done_tx.send((i, hash_path_hash(args, &paths[i]))).is_err() {
                        break;
                    }
                }
            });
        }
        drop(done_tx);

        let mut emitted = 0usize;
        let mut stop = false;
        let mut results: Vec<Option<anyhow::Result<blake3::Hash>>> = (0..paths.len()).map(|_| None).collect();
        let mut take = |i: usize, output: anyhow::Result<blake3::Hash>| -> bool {
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
            if i >= paths.len() || !take(i, hash_path_hash(args, &paths[i])) {
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
fn outboard_inputs(args: &Args, dir: &Path, out: &mut OutBuf) -> u64 {
    use std::sync::{Mutex, mpsc};
    let paths = &args.file_args;
    let workers = threads().get().min(paths.len());
    let in_a_file = (threads().get() / paths.len().max(1)).max(1);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let done = Mutex::new(Vec::<(usize, anyhow::Result<blake3::Hash>)>::new());
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
        if let Err(e) = std::fs::write(outboard::path_in(dir, &hash), &nodes) {
            unwritten.lock().unwrap().push((i, e));
        }
        if nodes.capacity() > 0 {
            // No worker may be left to take it.
            let _ = to_workers.send(nodes);
        }
    };
    let work = |hand: &dyn Fn(Made)| {
        let mut rooms = outboard::Rooms::new();
        let mut mine = Vec::new();
        loop {
            let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let Some(path) = paths.get(i) else { break };
            let mut nodes = spare.lock().unwrap().try_recv().unwrap_or_default();
            nodes.clear();
            let hashed = outboard_of(args.mode, path, in_a_file, &mut rooms, &mut nodes);
            if let Ok(hash) = &hashed {
                hand((i, *hash, nodes));
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
    let mut results: Vec<Option<anyhow::Result<blake3::Hash>>> = (0..paths.len()).map(|_| None).collect();
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
            eprintln!("{}: {}: {}", NAME, path.to_string_lossy(), e);
        }
    }
    failed
}

/// The hash of the input at `path` (`-`: standard input), and its
/// outboard appended to `nodes`. A file under MAP_LEN is read into the
/// thread's room by one read; a longer one that the page cache does not
/// hold is read directly, where the system has such reads (Linux); any
/// other is mapped where it can be (the mapping then held, as every
/// mapping b3sum makes: `hold`), else read whole.
fn outboard_of(mode: blake3::Mode, path: &Path, threads: usize, rooms: &mut outboard::Rooms, nodes: &mut Vec<u8>) -> anyhow::Result<blake3::Hash> {
    let bytes = &mut rooms.bytes;
    bytes.clear();
    if path == Path::new("-") {
        io::stdin().lock().read_to_end(bytes)?;
        return Ok(outboard::outboard(mode, bytes, threads, &mut rooms.cvs, nodes));
    }
    let mut file = File::open(path)?;
    let meta = file.metadata()?;
    #[cfg(unix)]
    if meta.is_file() && meta.len() < MAP_LEN as u64 {
        // One read of the length the file has now: what a longer read
        // or a second would say is that it has since grown.
        let len = meta.len() as usize;
        bytes.resize(len, 0);
        let mut got = 0;
        while got < len {
            match file.read(&mut bytes[got..])? {
                0 => break,
                n => got += n,
            }
        }
        return Ok(outboard::outboard(mode, &bytes[..got], threads, &mut rooms.cvs, nodes));
    }
    #[cfg(target_os = "linux")]
    if meta.is_file()
        && let Some(hash) = outboard::outboard_direct(mode, path, &file, meta.len(), threads, &mut rooms.piece, &mut rooms.cvs, nodes)
    {
        return Ok(hash);
    }
    #[cfg(unix)]
    if meta.len() > 0
        && let Ok(map) = unsafe { memmap2::Mmap::map(&file) }
    {
        let hash = outboard::outboard(mode, &map, threads, &mut rooms.cvs, nodes);
        hold(map);
        return Ok(hash);
    }
    file.read_to_end(bytes)?;
    Ok(outboard::outboard(mode, bytes, threads, &mut rooms.cvs, nodes))
}

fn hash_inputs(args: &Args, out: &mut OutBuf) -> u64 {
    if let Some(dir) = args.outboard() {
        return outboard_inputs(args, dir, out);
    }
    if args.is_standard_32() {
        let mut failed = 0u64;
        hash_paths_32(args, &args.file_args, &mut |idx, res| {
            let path = &args.file_args[idx];
            if let Err(e) = res.and_then(|h| print_hash_output(h, path, args, out)) {
                failed = failed.saturating_add(1);
                let _ = out.flush();
                eprintln!("{}: {}: {}", NAME, path.to_string_lossy(), e);
            }
            true
        });
        return failed;
    }

    let threads = if args.file_args.len() == 1 { 1 } else { threads().get().min(args.file_args.len()) };
    hash_inputs_with(args, threads, out, hash_some, print_output)
}

fn hash_inputs_with<O: Send>(
    args: &Args,
    threads: usize,
    out: &mut OutBuf,
    work: impl Fn(&Args, &std::sync::atomic::AtomicUsize, &mut dyn FnMut(usize, anyhow::Result<O>) -> bool) + Sync,
    print_one: impl Fn(O, &Path, &Args, &mut OutBuf) -> anyhow::Result<()>,
) -> u64 {
    if args.num_threads().is_some() {
        FILE_THREADS.store(threads, std::sync::atomic::Ordering::Relaxed);
    }
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
        let mut results: Vec<Option<anyhow::Result<O>>> = (0..paths.len()).map(|_| None).collect();
        let mut take = |i: usize, output: anyhow::Result<O>| {
            results[i] = Some(output);
            while printed < paths.len() {
                let Some(item) = results[printed].take() else { break };
                if let Err(e) = item.and_then(|output| print_one(output, &paths[printed], args, out)) {
                    failed = failed.saturating_add(1);
                    let _ = out.flush();
                    eprintln!("{}: {}: {}", NAME, paths[printed].to_string_lossy(), e);
                }
                printed += 1;
            }
            true
        };
        work(args, &next, &mut take);
        for (i, output) in done_rx {
            take(i, output);
        }
        failed
    })
}

fn hash_some(args: &Args, next: &std::sync::atomic::AtomicUsize, done: &mut dyn FnMut(usize, anyhow::Result<blake3::OutputReader>) -> bool) {
    loop {
        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if i >= args.file_args.len() || !done(i, hash_path(args, &args.file_args[i])) {
            return;
        }
    }
}

fn emit_checked_entry(parsed: ParsedCheckLine, res: anyhow::Result<blake3::Hash>, args: &Args, out: &mut OutBuf) -> bool {
    let ParsedCheckLine { file_string, is_escaped, expected_hash, .. } = parsed;
    let display = if is_escaped { "\\".to_string() + &file_string } else { file_string };
    let found_hash = match res {
        Ok(h) => h,
        Err(e) => {
            let _ = out.write_bytes(format!("{}: FAILED ({})\n", display, e).as_bytes());
            return false;
        }
    };
    // Constant-time comparison (`blake3::Hash::eq`).
    if expected_hash == found_hash {
        if !args.quiet() {
            let _ = out.write_bytes(display.as_bytes());
            let _ = out.write_bytes(b": OK\n");
        }
        true
    } else {
        let _ = out.write_bytes(display.as_bytes());
        let _ = out.write_bytes(b": FAILED\n");
        false
    }
}

fn check_one_checkfile(path: &Path, args: &Args, files_failed: &mut u64, out: &mut OutBuf) -> anyhow::Result<()> {
    const CHECK_CHUNK: usize = 4096;
    enum Entry {
        ParseErr(anyhow::Error),
        Stdin(ParsedCheckLine),
        File { parsed: ParsedCheckLine, batch_idx: usize },
    }

    let mut file;
    let stdin;
    let mut stdin_lock;
    let mut bufreader: io::BufReader<&mut dyn Read>;
    if path == Path::new("-") {
        stdin = io::stdin();
        stdin_lock = stdin.lock();
        bufreader = io::BufReader::new(&mut stdin_lock);
    } else {
        file = File::open(path)?;
        bufreader = io::BufReader::new(&mut file);
    }

    let mut line = String::new();
    let mut entries = Vec::new();
    let mut batch_paths = Vec::new();
    let mut eof = false;

    while !eof {
        entries.clear();
        batch_paths.clear();
        while entries.len() < CHECK_CHUNK {
            line.clear();
            let n = bufreader.read_line(&mut line)?;
            if n == 0 {
                eof = true;
                break;
            }
            match parse_check_line(&line, args.sigilcode()) {
                Err(e) => entries.push(Entry::ParseErr(e)),
                Ok(parsed) if parsed.file_path == Path::new("-") => entries.push(Entry::Stdin(parsed)),
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

        let mut batch_results: Vec<Option<anyhow::Result<blake3::Hash>>> = (0..batch_paths.len()).map(|_| None).collect();
        hash_paths_32(args, &batch_paths, &mut |idx, res| {
            batch_results[idx] = Some(res);
            true
        });

        for entry in entries.drain(..) {
            let ok = match entry {
                Entry::ParseErr(e) => {
                    let _ = out.flush();
                    eprintln!("{}: {}", NAME, e);
                    false
                }
                Entry::Stdin(parsed) => {
                    let res = hash_path_hash(args, &parsed.file_path);
                    emit_checked_entry(parsed, res, args, out)
                }
                Entry::File { parsed, batch_idx } => {
                    let res = batch_results[batch_idx].take().expect("every batch path hashed");
                    emit_checked_entry(parsed, res, args, out)
                }
            };
            if !ok {
                *files_failed = files_failed.saturating_add(1);
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn explain_sigbus() {
    static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    extern "C" fn on_sigbus(_: libc::c_int) {
        if SAID.swap(true, std::sync::atomic::Ordering::SeqCst) {
            loop {
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
    unsafe { libc::signal(libc::SIGBUS, handler as libc::sighandler_t) };
}

fn main() -> anyhow::Result<()> {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL)
    };
    #[cfg(unix)]
    explain_sigbus();
    blake3::initialize();
    let args = Args::parse()?;
    if let Some(threads) = args.num_threads() {
        THREADS.set(threads).expect("main sets the threads first");
    }
    let mut out = OutBuf::new();
    let mut files_failed = 0u64;
    if args.check() {
        for path in &args.file_args {
            if let Err(e) = check_one_checkfile(path, &args, &mut files_failed, &mut out) {
                let _ = out.flush();
                return Err(e);
            }
        }
    } else {
        files_failed = hash_inputs(&args, &mut out);
    }
    let _ = out.flush();
    if args.check() && files_failed > 0 {
        eprintln!("{}: WARNING: {} computed checksum{} did NOT match", NAME, files_failed, if files_failed == 1 { "" } else { "s" },);
    }
    let status = if files_failed > 0 { 1 } else { 0 };
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    unsafe { libc::_exit(status) }
}

#[cfg(test)]
mod test {
    include!("../../tools/require_nextest.rs");
    use clap::CommandFactory;

    #[test]
    fn test_args() {
        blake3::initialize();
        crate::Inner::command().debug_assert();
    }
}
