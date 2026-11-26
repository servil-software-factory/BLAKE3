//! Linux `io_uring` + `blake3::throughput::Pipeline` file hashing engine.
//!
//! Serves both multi-file hashing (`b3sum files...`) and checkfile verification
//! (`b3sum --check sums.b3`) through [`hash_paths`]:
//! - **Warm runs**: starts up to 12 threads; small files (`<= 16 KiB`) read via
//!   `preadv2(RWF_NOWAIT)` into a core-local `MaybeUninit<Block>` arena and hash
//!   16-at-a-time via `blake3::throughput::hash_each`; warm files `>= 4 MiB`
//!   hand owned `Mmap`s zero-copy to `Pipeline::owned` across the worker pool.
//! - **Cold runs**: the first `RWF_NOWAIT` miss opens that worker's
//!   single-issuer `Uring` (`COOP_TASKRUN | SINGLE_ISSUER | DEFER_TASKRUN`).
//!   Files `< 4 MiB` go straight onto it (the kernel's `io_read` serves
//!   cached hits inline and dispatches NVMe reads asynchronously), and each
//!   is hashed on the core that read it, whether or not the pool runs.
//!   Cold files `>= 32 MiB` with at least 4 direct-I/O threads available per
//!   file worker read past the page cache via `direct::hash` (`O_DIRECT`,
//!   2 MiB pieces; its CV table scales with file size at ~20 MiB/TiB, as on
//!   the single-file path) before starting the pool; cold files in
//!   `4 MiB..32 MiB` (or with fewer threads, or when direct I/O is
//!   unavailable) stream into 64 KiB `Pipeline::room()`s at ~20 MiB RSS, and
//!   are the only cold files that start the pool.

use crate::Args;
use crate::file_end::{self, CacheRead, InputRole, SMALL_MEM_CAP, StdinContext, StreamId};
use crate::uring::{IoUringCqe, Uring};
use blake3::throughput::{Block, Event, Limits, Message, Pipeline, ROOM_LEN, Room, block_on};
use core::cell::Cell;
use core::future::Future;
use core::mem::MaybeUninit;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use std::collections::VecDeque;
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

const SMALL: u64 = crate::MAP_LEN as u64;
const COLD_RING_MAX_FILE: u64 = 4 << 20;
const COLD_DIRECT_MIN: u64 = 32 << 20;
const MIN_MULTI_DIRECT_PER_FILE: usize = 4;
const WARM_BATCH_MAX_FILE: usize = 16 * 1024;
const WARM_SLOT_BLOCKS: usize = (WARM_BATCH_MAX_FILE / 64) + 1;
const WARM_BATCH_SLOTS: usize = 16;

const RING_ENTRIES: u32 = 128;
const SLOTS_PER_RING_MULTI: usize = 48;
const SLOTS_PER_RING_SINGLE: usize = 64;
const LARGE_WINDOW: usize = 64;
const MAX_FILE_ROOMS: u64 = 48;
const USER_LARGE_BIT: u64 = 1 << 63;

pub(crate) fn applies_to_paths(args: &Args, paths: &[PathBuf], stdin_ctx: &StdinContext) -> bool {
    args.is_standard_32() && paths.len() > 1 && paths.iter().all(|p| stdin_ctx.classify(p) == InputRole::Path)
}

pub(crate) fn hash_single_cold_large_file_1t(args: &Args, file: &File, hint_size: u64) -> Option<io::Result<(blake3::Hash, u64)>> {
    if !args.is_standard_32() {
        return None;
    }
    let mut ring = Uring::acquire(RING_ENTRIES).ok()?;
    crate::init_pool_once(1);
    let mut pipe = Pipeline::<usize, ()>::new(args.mode, Limits { messages: 2, rooms: LARGE_WINDOW });
    let mut deferred_cqes = VecDeque::new();
    let mut msg = block_on(pipe.message(0, Some(hint_size)));
    let (res, committed_any) = stream_fd_into_rooms(&mut ring, &mut deferred_cqes, &mut pipe, &mut msg, file.as_raw_fd(), hint_size);
    let out = match res {
        Ok(committed_bytes) => {
            pipe.end(msg);
            loop {
                if let Some(ev) = pipe.next_event() {
                    match ev {
                        Event::Hashed { hash, .. } => break Some(Ok((hash, committed_bytes))),
                        _ => unreachable!(),
                    }
                }
                block_on(pipe.ready());
            }
        }
        Err(e) => {
            pipe.abort(msg);
            while pipe.next_event().is_none() {
                block_on(pipe.ready());
            }
            if !committed_any && matches!(e.raw_os_error(), Some(libc::EINVAL | libc::EOPNOTSUPP | libc::ESPIPE)) { None } else { Some(Err(e)) }
        }
    };
    ring.release();
    out
}

/// Pre-grow the kernel file-descriptor table while single-threaded, raising
/// `RLIMIT_NOFILE` up to `rlim_max` where needed and clamping to `rlim_cur - 1`
/// so `dup2` never fails with `EBADF` when `want > 1024`. Returns the
/// effective `RLIMIT_NOFILE` soft limit so callers can size worker count and
/// ring slot depth within the process descriptor budget.
fn grow_descriptor_table(descriptors: usize) -> usize {
    let want = descriptors.next_power_of_two().max(64) as libc::rlim_t;
    // Sound: system calls on plain values and on descriptors this function
    // opened itself, each closed once.
    unsafe {
        let mut rlim = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        let cap = if libc::getrlimit(libc::RLIMIT_NOFILE, &mut rlim) == 0 {
            if rlim.rlim_cur < want && rlim.rlim_max > rlim.rlim_cur {
                let raised = want.min(rlim.rlim_max);
                let try_rlim = libc::rlimit { rlim_cur: raised, rlim_max: rlim.rlim_max };
                if libc::setrlimit(libc::RLIMIT_NOFILE, &try_rlim) == 0 {
                    rlim.rlim_cur = raised;
                }
            }
            rlim.rlim_cur
        } else {
            1024
        };
        let target = i32::try_from(want.min(cap).saturating_sub(1).min(i32::MAX as libc::rlim_t)).expect("clamped to i32::MAX");
        if target >= 63 {
            // F_DUPFD takes the lowest free number from `target` up, so it
            // never lands on a descriptor the caller gave this process
            // (`b3sum /dev/fd/255 ...`). dup2 did: it closed that file
            // without a word, and a later open could take its number.
            let fd = libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC);
            if fd >= 0 {
                let high = libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, target);
                if high >= 0 {
                    libc::close(high);
                }
                libc::close(fd);
            }
        }
        usize::try_from(cap).unwrap_or(usize::MAX)
    }
}

#[inline]
fn poll_once<F: Future>(f: F) -> Poll<F::Output> {
    let mut cx = Context::from_waker(Waker::noop());
    Pin::new(&mut core::pin::pin!(f)).poll(&mut cx)
}

enum WindowSlot {
    Reading(Room),
    Ready(Room, usize),
    Eof,
}

fn pread_fill_rest(fd: RawFd, mut dst: &mut [u8], mut offset: u64) -> io::Result<usize> {
    let mut total = 0usize;
    while !dst.is_empty() {
        // Sound: the read is into `dst`, a slice this function holds alone.
        let rc = unsafe { libc::pread(fd, dst.as_mut_ptr().cast(), dst.len(), offset as libc::off_t) };
        if rc == 0 {
            break;
        }
        if rc < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        let n = rc as usize;
        total += n;
        offset += n as u64;
        dst = &mut dst[n..];
    }
    Ok(total)
}

/// Unwind safety guard: declared *after* `window` in `stream_fd_into_rooms`
/// so that in unwind-enabled builds any unexpected panic aborts BEFORE
/// `window` drops a `Reading(Room)` whose buffer the kernel may still write.
struct AbortIfInFlight<'a>(&'a Cell<usize>);

impl Drop for AbortIfInFlight<'_> {
    fn drop(&mut self) {
        if self.0.get() != 0 {
            use std::io::Write;
            let _ = writeln!(std::io::stderr(), "b3sum: unwinding with {} io_uring room reads in flight; aborting", self.0.get());
            std::process::abort();
        }
    }
}

fn stream_fd_into_rooms(
    ring: &mut Uring,
    deferred_small_cqes: &mut VecDeque<IoUringCqe>,
    pipe: &mut Pipeline<usize, ()>,
    msg: &mut Message,
    fd: RawFd,
    hint_size: u64,
) -> (io::Result<u64>, bool) {
    let in_flight_uring = Cell::new(0usize);
    let mut window: [Option<WindowSlot>; LARGE_WINDOW] = [const { None }; LARGE_WINDOW];
    let _unwind_guard = AbortIfInFlight(&in_flight_uring);
    let mut next_issue_seq: u64 = 0;
    let mut next_commit_seq: u64 = 0;
    let mut max_issue_seq: u64 = (hint_size / (ROOM_LEN as u64)) + 1;
    let mut eof_seq: Option<u64> = None;
    let mut first_err: Option<io::Error> = None;
    let mut committed_bytes: u64 = 0;
    let mut committed_any = false;

    loop {
        while first_err.is_none()
            && eof_seq.is_none()
            && next_issue_seq <= max_issue_seq
            && (next_issue_seq - next_commit_seq) < MAX_FILE_ROOMS
            && ring.sq_space_left() > 0
        {
            let slot_idx = usize::try_from(next_issue_seq).expect("64-bit target") % LARGE_WINDOW;
            if window[slot_idx].is_some() {
                break;
            }
            let mut room = if in_flight_uring.get() == 0 && window[usize::try_from(next_commit_seq).expect("64-bit target") % LARGE_WINDOW].is_none() {
                block_on(pipe.room())
            } else {
                match poll_once(pipe.room()) {
                    Poll::Ready(r) => r,
                    Poll::Pending => break,
                }
            };
            let off = next_issue_seq * (ROOM_LEN as u64);
            let user_data = USER_LARGE_BIT | next_issue_seq;
            // Sound: the read's buffer stays where it is, written by no one
            // else, until its completion is reaped: the room sits in `window`
            // as Reading until then, and `_unwind_guard` aborts before `window`
            // drops if anything unwinds while a read is in flight.
            let ok = unsafe { ring.push_read(fd, room.bytes_mut().as_mut_ptr(), ROOM_LEN, off, 0, user_data) };
            // In every build: a read the full ring did not take would be
            // counted in flight and waited for without end.
            assert!(ok, "the ring has room for every read in flight");
            window[slot_idx] = Some(WindowSlot::Reading(room));
            in_flight_uring.set(in_flight_uring.get() + 1);
            next_issue_seq += 1;
        }

        let commit_ready =
            matches!(window[usize::try_from(next_commit_seq).expect("64-bit target") % LARGE_WINDOW], Some(WindowSlot::Ready(..) | WindowSlot::Eof));
        if ring.unsubmitted() > 0 || (!commit_ready && in_flight_uring.get() > 0) {
            let min_wait = if commit_ready || in_flight_uring.get() == 0 { 0 } else { 1 };
            ring.submit_and_wait(min_wait);
        }

        while let Some(cqe) = ring.pop_cqe() {
            if (cqe.user_data & USER_LARGE_BIT) == 0 {
                deferred_small_cqes.push_back(cqe);
                continue;
            }
            let seq = cqe.user_data & !USER_LARGE_BIT;
            let idx = (seq as usize) % LARGE_WINDOW;
            let Some(WindowSlot::Reading(mut room)) = window[idx].take() else {
                unreachable!("large CQE without Reading room");
            };
            in_flight_uring.set(in_flight_uring.get() - 1);

            if cqe.res < 0 {
                let errno = -cqe.res;
                if errno == libc::EINTR && first_err.is_none() {
                    let off = seq * (ROOM_LEN as u64);
                    while ring.sq_space_left() == 0 {
                        ring.submit_and_wait(0);
                    }
                    // Sound: as the first read of this room, above; it goes
                    // back into `window`.
                    let ok = unsafe { ring.push_read(fd, room.bytes_mut().as_mut_ptr(), ROOM_LEN, off, 0, USER_LARGE_BIT | seq) };
                    assert!(ok, "the ring has room for every read in flight");
                    window[idx] = Some(WindowSlot::Reading(room));
                    in_flight_uring.set(in_flight_uring.get() + 1);
                    continue;
                }
                if first_err.is_none() {
                    first_err = Some(io::Error::from_raw_os_error(errno));
                }
                drop(room);
                continue;
            }

            let n = cqe.res as usize;
            if n == 0 {
                drop(room);
                eof_seq = Some(eof_seq.map_or(seq, |e| e.min(seq)));
                window[idx] = Some(WindowSlot::Eof);
            } else if n < ROOM_LEN {
                let extra = match pread_fill_rest(fd, &mut room.bytes_mut()[n..ROOM_LEN], seq * (ROOM_LEN as u64) + (n as u64)) {
                    Ok(k) => k,
                    Err(e) => {
                        if first_err.is_none() {
                            first_err = Some(e);
                        }
                        drop(room);
                        continue;
                    }
                };
                let total = n + extra;
                if total < ROOM_LEN {
                    eof_seq = Some(eof_seq.map_or(seq, |e| e.min(seq)));
                } else if seq >= max_issue_seq {
                    max_issue_seq = seq + 2;
                }
                window[idx] = Some(WindowSlot::Ready(room, total));
            } else {
                if seq >= max_issue_seq {
                    max_issue_seq = seq + 2;
                }
                window[idx] = Some(WindowSlot::Ready(room, ROOM_LEN));
            }
        }

        if let Some(e) = first_err {
            wait_out_large_reads(ring, deferred_small_cqes, &mut window, &in_flight_uring);
            return (Err(e), committed_any);
        }

        let mut finished = false;
        loop {
            let idx = usize::try_from(next_commit_seq).expect("64-bit target") % LARGE_WINDOW;
            match window[idx].take() {
                Some(WindowSlot::Eof) => {
                    finished = true;
                    break;
                }
                Some(WindowSlot::Ready(mut room, len)) if len < ROOM_LEN => {
                    if len > 0 {
                        pipe.commit(msg, &mut room, len);
                        committed_bytes += len as u64;
                        committed_any = true;
                    }
                    drop(room);
                    finished = true;
                    break;
                }
                Some(WindowSlot::Ready(mut room, len)) => {
                    debug_assert_eq!(len, ROOM_LEN);
                    pipe.commit(msg, &mut room, ROOM_LEN);
                    committed_bytes += ROOM_LEN as u64;
                    committed_any = true;
                    drop(room);
                    next_commit_seq += 1;
                }
                other => {
                    window[idx] = other;
                    break;
                }
            }
        }

        if finished {
            wait_out_large_reads(ring, deferred_small_cqes, &mut window, &in_flight_uring);
            return (Ok(committed_bytes), committed_any);
        }
    }
}

/// Waits for every large read still in flight and lets its room go: the
/// one way out of `stream_fd_into_rooms`, at the file's end and after an
/// error alike. Each turn enters the kernel and waits for a completion.
/// (The error's way out once looped without entering, and with
/// `IORING_SETUP_DEFER_TASKRUN` no completion arrives unless the thread
/// enters: it spun for ever. BLAKE3 #437.)
fn wait_out_large_reads(
    ring: &mut Uring,
    deferred_small_cqes: &mut VecDeque<IoUringCqe>,
    window: &mut [Option<WindowSlot>; LARGE_WINDOW],
    in_flight: &Cell<usize>,
) {
    while in_flight.get() > 0 {
        ring.submit_and_wait(1);
        while let Some(cqe) = ring.pop_cqe() {
            if (cqe.user_data & USER_LARGE_BIT) == 0 {
                deferred_small_cqes.push_back(cqe);
                continue;
            }
            let seq = cqe.user_data & !USER_LARGE_BIT;
            let _ = window[(seq as usize) % LARGE_WINDOW].take();
            in_flight.set(in_flight.get() - 1);
        }
    }
    for slot in window {
        *slot = None;
    }
}

/// Where results go, until the listener says to stop: after `done` returns
/// false, nothing more is delivered.
struct Sink<'a> {
    done: &'a mut dyn FnMut(usize, crate::err::Result<blake3::Hash>) -> bool,
    open: bool,
}

impl Sink<'_> {
    fn put(&mut self, index: usize, result: crate::err::Result<blake3::Hash>) {
        if self.open && !(self.done)(index, result) {
            self.open = false;
        }
    }
}

struct WarmBatch {
    arena: Option<Box<[MaybeUninit<Block>]>>,
    entries: [(usize, usize); WARM_BATCH_SLOTS],
    len: usize,
}

impl WarmBatch {
    fn new() -> Self {
        Self { arena: None, entries: [(0, 0); WARM_BATCH_SLOTS], len: 0 }
    }

    #[inline]
    fn has_arena(&self) -> bool {
        self.arena.is_some()
    }

    #[inline]
    fn slot_bytes_mut(&mut self, slot: usize) -> &mut [MaybeUninit<u8>] {
        assert!(slot < WARM_BATCH_SLOTS, "warm batch slot out of range");
        let arena = self.arena.get_or_insert_with(|| Box::new_uninit_slice(WARM_BATCH_SLOTS * WARM_SLOT_BLOCKS));
        let start = slot * WARM_SLOT_BLOCKS;
        let blocks = &mut arena[start..start + WARM_SLOT_BLOCKS];
        // Sound: `MaybeUninit<Block>` is 64 bytes with no padding or validity
        // restrictions; reinterpreting it as `WARM_SLOT_BLOCKS * 64` uninit
        // bytes stays strictly within the checked slice `blocks`.
        unsafe { core::slice::from_raw_parts_mut(blocks.as_mut_ptr().cast::<MaybeUninit<u8>>(), WARM_SLOT_BLOCKS * 64) }
    }

    #[inline]
    fn push_slice(&mut self, file_idx: usize, src: &[u8], mode: blake3::Mode<'_>, sink: &mut Sink<'_>) {
        // In every build: the copy below is into the slot. A file can have
        // grown past WARM_BATCH_MAX_FILE since its size was read, and no
        // further than the slot.
        let byte_len = src.len();
        assert!(byte_len <= WARM_SLOT_BLOCKS * 64, "a batched file fits its slot");
        let slot = self.len;
        let dst = self.slot_bytes_mut(slot);
        if byte_len > 0 {
            // Sound: `src` is `&[u8]` of `byte_len` initialized bytes and `dst`
            // is a disjoint `&mut [MaybeUninit<u8>]` of at least `byte_len`.
            unsafe { core::ptr::copy_nonoverlapping(src.as_ptr(), dst.as_mut_ptr().cast::<u8>(), byte_len) };
        }
        self.entries[slot] = (file_idx, byte_len);
        self.len += 1;
        if self.len == WARM_BATCH_SLOTS {
            self.flush(mode, sink);
        }
    }

    fn flush(&mut self, mode: blake3::Mode<'_>, sink: &mut Sink<'_>) {
        let count = self.len;
        if count == 0 {
            return;
        }
        self.len = 0;
        if !sink.open {
            return;
        }
        let arena = self.arena.as_mut().expect("arena allocated before len > 0");
        if count == 1 {
            let (file_idx, byte_len) = self.entries[0];
            let ptr = arena.as_ptr().cast::<u8>();
            // Sound: the first `byte_len` bytes of slot 0 were written by a
            // read or by push_slice.
            let bytes = unsafe { core::slice::from_raw_parts(ptr, byte_len) };
            let h = crate::hash_slice_32(mode, bytes);
            sink.put(file_idx, Ok(h));
            return;
        }
        for k in 0..count {
            let (_, byte_len) = self.entries[k];
            let blocks_needed = byte_len.div_ceil(64);
            let padded = blocks_needed * 64;
            if padded > byte_len {
                // Sound: slot k is inside the arena, and `padded` is at most
                // the slot's length (`byte_len` is, and the slot is whole
                // blocks).
                let start_ptr = unsafe { arena.as_mut_ptr().add(k * WARM_SLOT_BLOCKS).cast::<u8>() };
                unsafe { start_ptr.add(byte_len).write_bytes(0, padded - byte_len) };
            }
        }
        let arena_ref: &[MaybeUninit<Block>] = &*arena;
        let mut items: [(&[Block], usize); WARM_BATCH_SLOTS] = [(&[], 0); WARM_BATCH_SLOTS];
        let mut digests = [[0u8; blake3::OUT_LEN]; WARM_BATCH_SLOTS];
        for k in 0..count {
            let (_, byte_len) = self.entries[k];
            let blocks_needed = byte_len.div_ceil(64);
            let start = k * WARM_SLOT_BLOCKS;
            // Sound: the slot's first `blocks_needed` blocks are written:
            // `byte_len` bytes by the read or copy, zeros to the block's end
            // just above. A Block is 64 plain bytes.
            let slice = unsafe { core::slice::from_raw_parts(arena_ref.as_ptr().add(start).cast::<Block>(), blocks_needed) };
            items[k] = (slice, byte_len);
        }
        blake3::throughput::hash_each(mode, &items[..count], &mut digests[..count]);
        for k in 0..count {
            let (file_idx, _) = self.entries[k];
            sink.put(file_idx, Ok(blake3::Hash::from(digests[k])));
        }
    }
}

/// Set by a test to take every file for one the page cache lacks, so that
/// the cold paths run whatever the cache holds.
#[cfg(test)]
static FORCE_MISS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Count of cold `>= COLD_DIRECT_MIN` files hashed and accepted through
/// `direct::hash` in `run_hybrid_worker`, and the thread count passed to it.
#[cfg(test)]
static DIRECT_MULTI_SUCCEEDED: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
static DIRECT_MULTI_LAST_THREADS: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
pub(crate) fn force_miss() -> bool {
    FORCE_MISS.load(Ordering::Relaxed)
}

/// The tests' seam: called with each file's path just after the file is
/// opened, where a test replaces what the path names.
#[cfg(test)]
pub(crate) static AFTER_OPEN: std::sync::Mutex<Option<Box<dyn Fn(&std::path::Path) + Send>>> = std::sync::Mutex::new(None);

/// The tests' seam: with it set, every file is taken for one the page
/// cache does not hold.
#[cfg(test)]
pub(crate) fn set_force_miss(on: bool) {
    FORCE_MISS.store(on, Ordering::Relaxed)
}

#[cfg(not(test))]
#[inline(always)]
fn force_miss() -> bool {
    false
}

/// Read the file at `fd` from byte `at` into `dst`: delegates to the shared
/// `file_end::cached_then_plain` state machine so only a waiting 0-byte read
/// ends a file, cache misses return `CacheRead::NeedMore`, non-seekable fds
/// return `CacheRead::NonSeekable`, and real blocking `EIO` errors propagate.
#[inline]
fn try_pread_cached(fd: RawFd, dst: &mut [MaybeUninit<u8>], at: usize) -> io::Result<CacheRead> {
    assert!(at <= dst.len(), "cached read start within destination slice");
    if force_miss() {
        return Ok(CacheRead::NeedMore(at));
    }
    let dst_cap = dst.len();
    file_end::cached_then_plain(dst_cap, at, |pos, flags| preadv2_asked(fd, &mut dst[pos..], pos, flags))
}

/// One `preadv2` into `dst` with `flags`; the tests keep what was asked.
#[inline]
fn preadv2_asked(fd: RawFd, dst: &mut [MaybeUninit<u8>], at: usize, flags: libc::c_int) -> isize {
    #[cfg(test)]
    ASKED.with(|asked| asked.borrow_mut().push(flags));
    let iov = libc::iovec { iov_base: dst.as_mut_ptr().cast(), iov_len: dst.len() };
    // Sound: `iov` points directly into the exclusively borrowed `dst` slice.
    unsafe { libc::preadv2(fd, &iov, 1, at as libc::off_t, flags) }
}

#[cfg(test)]
thread_local! {
    /// The flags of each read [`try_pread_cached`] made on this thread.
    static ASKED: core::cell::RefCell<Vec<libc::c_int>> = const { core::cell::RefCell::new(Vec::new()) };
}

struct SmallSlot {
    file_idx: usize,
    file: File,
    /// `buf.len()` is always the initialized prefix length read so far.
    buf: Vec<u8>,
}

#[inline]
fn ensure_spare_cap(buf: &mut Vec<u8>, needed: usize) {
    buf.clear();
    let capped = needed.min(SMALL_MEM_CAP + 1);
    if buf.capacity() < capped {
        buf.reserve(capped);
    }
}

/// Resets `buf` so it holds `start` (if any) as its initialized prefix
/// (`buf.len() == start.len()`) with room for a file of `hint` bytes (capped at
/// `SMALL_MEM_CAP + 1` so virtual/growing files promote to streaming rather
/// than exhausting memory).
fn buffer_with_start(buf: &mut Vec<u8>, hint: usize, start: Option<&[u8]>) -> usize {
    let prefix = start.unwrap_or(&[]);
    let need = hint.saturating_add(1).max(prefix.len().saturating_add(4096)).min(SMALL_MEM_CAP + 1);
    buf.clear();
    if buf.capacity() < need {
        buf.reserve(need);
    }
    buf.extend_from_slice(prefix);
    buf.len()
}

/// Finish reading and hashing `file` starting from the initialized prefix
/// already in `buf`, using `pread` (falling back to sequential `read` on
/// `ESPIPE` when `buf.is_empty()`) and promoting to incremental streaming
/// hashing if the file grows past `SMALL_MEM_CAP`.
fn finish_sync_hash(mode: blake3::Mode<'_>, file: &mut File, buf: &mut Vec<u8>) -> io::Result<blake3::Hash> {
    let (reached_eof, sequential) = file_end::pread_rest_capped(file, buf, SMALL_MEM_CAP)?;
    if reached_eof { Ok(crate::hash_slice_32(mode, buf)) } else { Ok(file_end::finish_hash_streaming_from_prefix(mode, buf, file, sequential)?.hash) }
}

struct WarmLargePipe {
    pipe: Pipeline<usize, crate::mmap::Mmap>,
    in_flight: usize,
}

impl WarmLargePipe {
    fn new(mode: blake3::Mode<'static>) -> Self {
        Self { pipe: Pipeline::with_owners(mode, Limits { messages: 16, rooms: 0 }), in_flight: 0 }
    }

    fn drain(&mut self, sink: &mut Sink<'_>) {
        while let Some(ev) = self.pipe.next_event() {
            self.in_flight -= 1;
            if let Event::Hashed { tag: idx, hash, owner } = ev {
                if let Some(map) = owner {
                    crate::hold(map);
                }
                sink.put(idx, Ok(hash));
            }
        }
    }

    fn submit_owned(&mut self, idx: usize, size: u64, map: crate::mmap::Mmap, sink: &mut Sink<'_>) {
        self.drain(sink);
        let m = block_on(self.pipe.message(idx, Some(size)));
        self.pipe.owned(m, map);
        self.in_flight += 1;
    }

    fn finish(&mut self, sink: &mut Sink<'_>) {
        while self.in_flight > 0 {
            self.drain(sink);
            if self.in_flight > 0 {
                block_on(self.pipe.ready());
            }
        }
    }
}

struct ColdRingState {
    ring: Uring,
    cold_large_pipe: Option<Pipeline<usize, ()>>,
    slots: Vec<Option<SmallSlot>>,
    free_slot_ids: Vec<usize>,
    free_bufs: Vec<Vec<u8>>,
    deferred_cqes: VecDeque<IoUringCqe>,
    ring_in_flight: usize,
    max_slots: usize,
}

impl ColdRingState {
    fn new(max_slots: usize) -> Option<Self> {
        let ring = Uring::acquire(RING_ENTRIES).ok()?;
        Some(Self {
            ring,
            cold_large_pipe: None,
            slots: (0..max_slots).map(|_| None).collect(),
            free_slot_ids: (0..max_slots).rev().collect(),
            free_bufs: Vec::with_capacity(max_slots * 2),
            deferred_cqes: VecDeque::new(),
            ring_in_flight: 0,
            max_slots,
        })
    }

    fn queue_file(
        &mut self,
        file_idx: usize,
        file: File,
        hint: usize,
        partial_src: Option<&[u8]>,
        args: &Args,
        warm_batch: &mut WarmBatch,
        sink: &mut Sink<'_>,
    ) {
        while self.free_slot_ids.is_empty() || self.ring.sq_space_left() == 0 {
            self.step_ring(true, args, warm_batch, sink);
        }
        let mut buf = self.free_bufs.pop().unwrap_or_default();
        let partial_at = buffer_with_start(&mut buf, hint, partial_src);
        let sid = self.free_slot_ids.pop().unwrap();
        let fd = file.as_raw_fd();
        let spare = buf.spare_capacity_mut();
        let ptr = spare.as_mut_ptr().cast::<u8>();
        let room = spare.len();
        self.slots[sid] = Some(SmallSlot { file_idx, file, buf });
        // Sound: the read's buffer stays where it is, written by no one else,
        // until its completion is reaped: the buffer goes into `slots[sid]`
        // and is not touched until then.
        let ok = unsafe { self.ring.push_read(fd, ptr, room, partial_at as u64, 0, sid as u64) };
        // In every build, as the large file's read above.
        assert!(ok, "the ring has room for every read in flight");
        self.ring_in_flight += 1;
    }

    fn step_ring(&mut self, wait: bool, args: &Args, warm_batch: &mut WarmBatch, sink: &mut Sink<'_>) {
        if self.deferred_cqes.is_empty() {
            let min_wait = if wait && self.ring_in_flight > 0 { 1 } else { 0 };
            if min_wait == 0 && self.ring.unsubmitted() == 0 {
                return;
            }
            self.ring.submit_and_wait(min_wait);
        }

        let mode = args.mode;
        loop {
            let cqe = match self.deferred_cqes.pop_front() {
                Some(c) => c,
                None => match self.ring.pop_cqe() {
                    Some(c) => c,
                    None => break,
                },
            };
            let slot_id = usize::try_from(cqe.user_data).expect("64-bit target");
            let mut st = self.slots[slot_id].take().expect("active SmallSlot");
            self.ring_in_flight -= 1;

            if cqe.res < 0 {
                let errno = -cqe.res;
                if errno == libc::EINTR {
                    while self.ring.sq_space_left() == 0 {
                        self.ring.submit_and_wait(0);
                    }
                    let fd = st.file.as_raw_fd();
                    let off = st.buf.len() as u64;
                    let spare = st.buf.spare_capacity_mut();
                    let ptr = spare.as_mut_ptr().cast::<u8>();
                    let room = spare.len();
                    self.slots[slot_id] = Some(st);
                    // Sound: as queue_file's read; the slot goes back into
                    // `slots`.
                    let ok = unsafe { self.ring.push_read(fd, ptr, room, off, 0, slot_id as u64) };
                    assert!(ok, "the ring has room for every read in flight");
                    self.ring_in_flight += 1;
                    continue;
                }
                // Unsupported ring READ (`EINVAL`/`EOPNOTSUPP`/`EBADF`): finish via
                // waiting `pread` from `st.buf.len()` even when a cache prefix was
                // already read. `ESPIPE` falls back to sequential `read` at offset 0.
                if matches!(errno, libc::EINVAL | libc::EOPNOTSUPP | libc::EBADF) || (st.buf.is_empty() && errno == libc::ESPIPE) {
                    let res = finish_sync_hash(mode, &mut st.file, &mut st.buf);
                    drop(st.file);
                    self.free_slot_ids.push(slot_id);
                    match res {
                        Ok(h) => sink.put(st.file_idx, Ok(h)),
                        Err(e) => sink.put(st.file_idx, Err(e.into())),
                    }
                    st.buf.clear();
                    if self.free_bufs.len() < self.max_slots * 2 && st.buf.capacity() <= (256 << 10) {
                        self.free_bufs.push(st.buf);
                    }
                    continue;
                }
                st.buf.clear();
                if st.buf.capacity() <= (256 << 10) && self.free_bufs.len() < self.max_slots * 2 {
                    self.free_bufs.push(st.buf);
                }
                self.free_slot_ids.push(slot_id);
                sink.put(st.file_idx, Err(io::Error::from_raw_os_error(errno).into()));
                continue;
            }

            let n = cqe.res as usize;
            // The file's end is where a read gives no byte, and nowhere
            // its size said (b3sum/README.md, "What b3sum reads").
            if n == 0 {
                drop(st.file);
                self.free_slot_ids.push(slot_id);
                if sink.open {
                    if st.buf.len() <= WARM_BATCH_MAX_FILE {
                        warm_batch.push_slice(st.file_idx, &st.buf, mode, sink);
                    } else {
                        let h = crate::hash_slice_local(mode, &st.buf);
                        sink.put(st.file_idx, Ok(h));
                    }
                }
                st.buf.clear();
                if st.buf.capacity() <= (256 << 10) && self.free_bufs.len() < self.max_slots * 2 {
                    self.free_bufs.push(st.buf);
                }
            } else {
                assert!(n <= st.buf.capacity() - st.buf.len(), "kernel read did not exceed spare capacity");
                // Sound: the ring read wrote `n` initialized bytes into `st.buf.spare_capacity_mut()[..n]`.
                unsafe { st.buf.set_len(st.buf.len() + n) };

                // Bounded memory: if a small-hinted or virtual regular file grows
                // past SMALL_MEM_CAP, promote to streaming incremental hashing.
                if st.buf.len() >= SMALL_MEM_CAP {
                    let res = file_end::finish_hash_streaming_from_prefix(mode, &st.buf, &mut st.file, false).map(|hr| hr.hash);
                    drop(st.file);
                    self.free_slot_ids.push(slot_id);
                    sink.put(st.file_idx, res.map_err(Into::into));
                    continue;
                }

                if st.buf.len() == st.buf.capacity() {
                    let grow = st.buf.capacity().max(8192).min(SMALL_MEM_CAP - st.buf.len());
                    st.buf.reserve(grow);
                }
                while self.ring.sq_space_left() == 0 {
                    self.ring.submit_and_wait(0);
                }
                let fd = st.file.as_raw_fd();
                let off = st.buf.len() as u64;
                let spare = st.buf.spare_capacity_mut();
                let ptr = spare.as_mut_ptr().cast::<u8>();
                let room = spare.len();
                self.slots[slot_id] = Some(st);
                // Sound: as queue_file's read; the slot goes back into
                // `slots`.
                let ok = unsafe { self.ring.push_read(fd, ptr, room, off, 0, slot_id as u64) };
                assert!(ok, "the ring has room for every read in flight");
                self.ring_in_flight += 1;
            }
        }
    }

    fn finish(mut self, args: &Args, warm_batch: &mut WarmBatch, sink: &mut Sink<'_>) {
        while self.ring_in_flight > 0 || !self.deferred_cqes.is_empty() {
            self.step_ring(true, args, warm_batch, sink);
        }
        drop(self.cold_large_pipe);
        self.ring.release();
    }
}

/// Hash `paths` in parallel and deliver `(index, result)` in exact `0..paths.len()`
/// order to `on_ordered` (which returns `false` to stop early).
pub(crate) fn hash_paths(
    args: &Args,
    paths: &[PathBuf],
    forbidden: Option<StreamId>,
    on_ordered: &mut dyn FnMut(usize, crate::err::Result<blake3::Hash>) -> bool,
) {
    hash_paths_inner(args, paths, forbidden, false, on_ordered);
}

/// The threads that read and hash `nfiles` files, of `total_threads`:
/// four for up to 63 files, one more for every 32 files from 64, twelve
/// at the most. Each thread costs its start and its ring, which a hundred
/// small files do not repay. A hundred 16 KiB files, milliseconds with 6
/// threads and with 12: on 2x EPYC 7443 (96 CPUs) 2.47 and 3.13 in the page
/// cache, 3.54 and 4.67 from storage; on a Ryzen 9700X 0.61 and 0.70, and
/// 1.08 both. A thousand want the twelve: from storage on the 9700X 3.57 ms
/// with 12 and 4.34 with 6.
fn readers(total_threads: usize, nfiles: usize) -> usize {
    (3 + nfiles / 32).clamp(4, 12).min(nfiles).min(total_threads)
}

/// Direct-I/O helper threads a multi-file worker asks for on one cold file of
/// at least [`COLD_DIRECT_MIN`] bytes: targets [`crate::DIRECT_THREADS`] (16)
/// concurrent direct reads across the planned `io_threads`, bounded by
/// `total_threads / io_threads`.
fn multi_file_direct_threads(total_threads: usize, io_threads: usize) -> usize {
    let workers = io_threads.max(1);
    (crate::DIRECT_THREADS / workers).max(1).min((total_threads / workers).max(1))
}

fn hash_paths_inner(
    args: &Args,
    paths: &[PathBuf],
    forbidden: Option<StreamId>,
    force_no_uring: bool,
    on_ordered: &mut dyn FnMut(usize, crate::err::Result<blake3::Hash>) -> bool,
) {
    let nfiles = paths.len();
    if nfiles == 0 {
        return;
    }
    let total_threads = crate::threads().get();
    let max_readers = if args.num_threads().is_some() { crate::max_file_threads(total_threads) } else { total_threads };
    let desired_io_threads = readers(max_readers, nfiles);
    let desired_slots = if desired_io_threads <= 4 { SLOTS_PER_RING_SINGLE } else { SLOTS_PER_RING_MULTI };

    let nofile_cap = grow_descriptor_table(desired_io_threads * (desired_slots + 8) + 64);
    // Keep cross-worker descriptor usage within `RLIMIT_NOFILE`: reserve 6 fds
    // for stdio/ring/misc, collapse to 1 reader when tight so EMFILE draining
    // always reclaims its own queued slots, and cap per-ring slots accordingly.
    let usable_fds = nofile_cap.saturating_sub(6);
    let io_threads = if usable_fds < 16 { 1 } else { desired_io_threads.min((usable_fds / 4).max(1)) };
    let slots_per_ring = desired_slots.min((usable_fds / io_threads).saturating_sub(1).max(1));

    let _file_threads_guard = args.num_threads().map(|_| crate::FileThreadsGuard::enter(io_threads));

    if io_threads == 1 {
        let next = AtomicUsize::new(0);
        let mut results: Vec<Option<crate::err::Result<blake3::Hash>>> = (0..nfiles).map(|_| None).collect();
        let mut emitted = 0usize;
        let mut stop = false;
        run_hybrid_worker(args, paths, forbidden, slots_per_ring, total_threads, 1, force_no_uring, &next, &mut |idx, res| {
            results[idx] = Some(res);
            while emitted < nfiles {
                let Some(item) = results[emitted].take() else { break };
                if !on_ordered(emitted, item) {
                    stop = true;
                    return false;
                }
                emitted += 1;
            }
            !stop
        });
        return;
    }

    let next = AtomicUsize::new(0);
    let (done_tx, done_rx) = std::sync::mpsc::channel::<(usize, crate::err::Result<blake3::Hash>)>();

    std::thread::scope(|scope| {
        for _ in 1..io_threads {
            if next.load(Ordering::Relaxed) >= nfiles {
                break;
            }
            let (next_ref, tx) = (&next, done_tx.clone());
            scope.spawn(move || {
                run_hybrid_worker(args, paths, forbidden, slots_per_ring, total_threads, io_threads, force_no_uring, next_ref, &mut |i, res| {
                    tx.send((i, res)).is_ok()
                });
            });
        }
        drop(done_tx);

        let mut results: Vec<Option<crate::err::Result<blake3::Hash>>> = (0..nfiles).map(|_| None).collect();
        let mut emitted = 0usize;
        let mut stop = false;
        let mut take = |idx: usize, res: crate::err::Result<blake3::Hash>| -> bool {
            results[idx] = Some(res);
            while emitted < nfiles {
                let Some(item) = results[emitted].take() else { break };
                if !on_ordered(emitted, item) {
                    stop = true;
                    return false;
                }
                emitted += 1;
            }
            !stop
        };

        run_hybrid_worker(args, paths, forbidden, slots_per_ring, total_threads, io_threads, force_no_uring, &next, &mut |i, res| {
            while let Ok((wi, wres)) = done_rx.try_recv() {
                if !take(wi, wres) {
                    return false;
                }
            }
            take(i, res)
        });
        for (i, res) in done_rx {
            if !take(i, res) {
                break;
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn run_hybrid_worker(
    args: &Args,
    paths: &[PathBuf],
    forbidden: Option<StreamId>,
    max_slots: usize,
    total_threads: usize,
    io_threads: usize,
    mut uring_unavailable: bool,
    next: &AtomicUsize,
    done: &mut dyn FnMut(usize, crate::err::Result<blake3::Hash>) -> bool,
) {
    let mut warm_batch = WarmBatch::new();
    let mut medium_buf: Vec<u8> = Vec::new();
    let mut warm_pipe: Option<WarmLargePipe> = None;
    let mut cold: Option<ColdRingState> = None;
    let mut direct_piece: Option<crate::direct::Piece> = None;
    let mut probe_stack: [MaybeUninit<u8>; WARM_SLOT_BLOCKS * 64] = [MaybeUninit::uninit(); WARM_SLOT_BLOCKS * 64];
    let mut sink = Sink { done, open: true };

    'files: while sink.open {
        if let Some(ref mut wp) = warm_pipe {
            wp.drain(&mut sink);
        }
        if let Some(ref mut c) = cold {
            let submit_batch = if c.ring_in_flight <= 8 { 4 } else { 16 };
            if c.ring.unsubmitted() >= submit_batch {
                c.step_ring(false, args, &mut warm_batch, &mut sink);
            }
            while sink.open && (c.free_slot_ids.is_empty() || c.ring.sq_space_left() == 0) {
                c.step_ring(true, args, &mut warm_batch, &mut sink);
            }
            if !sink.open {
                break;
            }
        }

        let i = next.fetch_add(1, Ordering::Relaxed);
        if i >= paths.len() {
            break;
        }

        // Open and check `fstat` identity on the actual descriptor; on EMFILE/ENFILE
        // while we hold queued ring descriptors, drain completions until a slot
        // closes and retry so a tight RLIMIT_NOFILE makes progress instead of
        // failing queued files spuriously.
        let (mut file, meta) = loop {
            match file_end::open_checked(&paths[i], forbidden) {
                Ok(pair) => break pair,
                Err(e) => {
                    let is_fd_limit = matches!(e.raw_os_error(), Some(libc::EMFILE | libc::ENFILE));
                    if is_fd_limit
                        && let Some(ref mut c) = cold
                        && c.ring_in_flight > 0
                    {
                        let before_free = c.free_slot_ids.len();
                        while c.ring_in_flight > 0 && c.free_slot_ids.len() == before_free {
                            c.step_ring(true, args, &mut warm_batch, &mut sink);
                        }
                        if c.free_slot_ids.len() > before_free {
                            continue;
                        }
                    }
                    sink.put(i, Err(e));
                    continue 'files;
                }
            }
        };

        #[cfg(test)]
        if let Some(hook) = AFTER_OPEN.lock().unwrap().as_ref() {
            hook(&paths[i]);
        }

        // From here on the file is the one opened above, on every path:
        // nothing opens its name again, which may name another file by now.
        if !meta.is_file() {
            sink.put(i, crate::hash_open_file(args, file));
            continue;
        }

        let size = meta.len;

        // Fast path once `cold` ring is already active on this worker: push `< 4 MiB`
        // files straight onto `Uring` without an extra `preadv2(RWF_NOWAIT)` syscall
        // (kernel `io_read` serves cached hits inline and arms async NVMe reads on misses!).
        if let Some(ref mut c) = cold
            && size < COLD_RING_MAX_FILE
        {
            c.queue_file(i, file, usize::try_from(size).expect("size < COLD_RING_MAX_FILE"), None, args, &mut warm_batch, &mut sink);
            continue;
        }

        // 1. Small files `<= 16 KiB` before `cold` is active:
        if size <= WARM_BATCH_MAX_FILE as u64 {
            let hint = usize::try_from(size).expect("size <= WARM_BATCH_MAX_FILE");
            let in_arena = warm_batch.has_arena();
            let dst = if in_arena {
                let s = warm_batch.len;
                warm_batch.slot_bytes_mut(s)
            } else {
                &mut probe_stack[..]
            };

            match try_pread_cached(file.as_raw_fd(), dst, 0) {
                Ok(CacheRead::Eof(n)) => {
                    drop(file);
                    if in_arena {
                        let s = warm_batch.len;
                        warm_batch.entries[s] = (i, n);
                        warm_batch.len += 1;
                        if warm_batch.len == WARM_BATCH_SLOTS {
                            warm_batch.flush(args.mode, &mut sink);
                        }
                    } else {
                        // Sound: `try_pread_cached` initialized `probe_stack[..n]`.
                        let bytes = unsafe { core::slice::from_raw_parts(probe_stack.as_ptr().cast::<u8>(), n) };
                        warm_batch.push_slice(i, bytes, args.mode, &mut sink);
                    }
                    continue;
                }
                Ok(CacheRead::NonSeekable(0)) => {
                    sink.put(i, crate::hash_open_file(args, file));
                    continue;
                }
                Ok(CacheRead::NeedMore(partial_at) | CacheRead::NonSeekable(partial_at)) => {
                    // Sound: `try_pread_cached` initialized `dst[..partial_at]`.
                    let partial_slice = unsafe { core::slice::from_raw_parts(dst.as_ptr().cast::<u8>(), partial_at) };
                    if !uring_unavailable && cold.is_none() {
                        cold = ColdRingState::new(max_slots);
                        if cold.is_none() {
                            uring_unavailable = true;
                        }
                    }
                    if let Some(ref mut c) = cold {
                        c.queue_file(i, file, hint, Some(partial_slice), args, &mut warm_batch, &mut sink);
                    } else {
                        let mut buf = Vec::new();
                        buffer_with_start(&mut buf, hint, Some(partial_slice));
                        let res = finish_sync_hash(args.mode, &mut file, &mut buf).map_err(Into::into);
                        sink.put(i, res);
                    }
                    continue;
                }
                Err(e) => {
                    sink.put(i, Err(e.into()));
                    continue;
                }
            }
        }

        // 2. Medium files `16 KiB < size < 512 KiB` before `cold` is active:
        if size < SMALL {
            let hint = usize::try_from(size).expect("size < SMALL");
            ensure_spare_cap(&mut medium_buf, hint + 1);
            match try_pread_cached(file.as_raw_fd(), medium_buf.spare_capacity_mut(), 0) {
                Ok(CacheRead::Eof(n)) => {
                    drop(file);
                    // Sound: try_pread_cached wrote the first `n` bytes.
                    unsafe { medium_buf.set_len(n) };
                    let h = crate::hash_slice_32(args.mode, &medium_buf);
                    medium_buf.clear();
                    sink.put(i, Ok(h));
                    continue;
                }
                Ok(CacheRead::NonSeekable(0)) => {
                    sink.put(i, crate::hash_open_file(args, file));
                    continue;
                }
                Ok(CacheRead::NeedMore(partial_at) | CacheRead::NonSeekable(partial_at)) => {
                    // Sound: try_pread_cached wrote the first `partial_at` bytes.
                    unsafe { medium_buf.set_len(partial_at) };
                    if !uring_unavailable && cold.is_none() {
                        cold = ColdRingState::new(max_slots);
                        if cold.is_none() {
                            uring_unavailable = true;
                        }
                    }
                    if let Some(ref mut c) = cold {
                        c.queue_file(i, file, hint, Some(&medium_buf), args, &mut warm_batch, &mut sink);
                        medium_buf.clear();
                    } else {
                        let res = finish_sync_hash(args.mode, &mut file, &mut medium_buf).map_err(Into::into);
                        medium_buf.clear();
                        sink.put(i, res);
                    }
                    continue;
                }
                Err(e) => {
                    sink.put(i, Err(e.into()));
                    continue;
                }
            }
        }

        // 3. Large files `>= 512 KiB`: check `resident(&map)` BEFORE creating any `Uring`!
        warm_batch.flush(args.mode, &mut sink);
        let mut mapped_cold = false;
        if let Some(map) = args.map(&file) {
            if !force_miss() && crate::resident(&map) {
                if size < (4 << 20) {
                    let h = crate::hash_slice_32(args.mode, &map);
                    crate::hold(map);
                    sink.put(i, Ok(h));
                    continue;
                }
                let pool_workers = if args.num_threads().is_some() { (total_threads - io_threads + 1).max(1) } else { total_threads };
                crate::init_pool_once(pool_workers);
                let wp = warm_pipe.get_or_insert_with(|| WarmLargePipe::new(args.mode));
                wp.submit_owned(i, size, map, &mut sink);
                continue;
            }

            // Cold `512 KiB .. 4 MiB`: queue into `ColdRingState` sliding window if possible!
            if size < COLD_RING_MAX_FILE {
                drop(map);
                if !uring_unavailable && cold.is_none() {
                    cold = ColdRingState::new(max_slots);
                    if cold.is_none() {
                        uring_unavailable = true;
                    }
                }
                if let Some(ref mut c) = cold {
                    c.queue_file(i, file, usize::try_from(size).expect("size < COLD_RING_MAX_FILE"), None, args, &mut warm_batch, &mut sink);
                    continue;
                }
            } else {
                drop(map);
                mapped_cold = true;
            }
        }

        // Cold `>= 32 MiB` when this worker can lease at least 4 direct-I/O
        // readers: read past the page cache (`O_DIRECT`, 2 MiB pieces) before
        // opening an `io_uring` or starting the global pool. Using `resident`
        // above avoids a `preadv2(RWF_NOWAIT)` probe on the descriptor before
        // an `io_uring` fallback (`--no-mmap` falls back to `direct::is_for`).
        // Honors `--num-threads` via `DirectPermits`.
        let want_direct = multi_file_direct_threads(total_threads, io_threads);
        if size >= COLD_DIRECT_MIN
            && want_direct >= MIN_MULTI_DIRECT_PER_FILE
            && let Ok(len) = usize::try_from(size)
            && (mapped_cold || crate::direct::is_for(&file, size))
        {
            let direct_hash = {
                let permits = crate::DirectPermits::acquire(want_direct.saturating_sub(1));
                let direct_threads = 1 + permits.extra;
                if direct_threads >= MIN_MULTI_DIRECT_PER_FILE {
                    let h = crate::direct::hash(args.mode, &file, len, direct_threads, direct_piece.get_or_insert_with(crate::direct::Piece::new));
                    #[cfg(test)]
                    if h.is_some() {
                        DIRECT_MULTI_SUCCEEDED.fetch_add(1, Ordering::Relaxed);
                        DIRECT_MULTI_LAST_THREADS.store(direct_threads, Ordering::Relaxed);
                    }
                    h
                } else {
                    None
                }
            };
            if let Some(h) = direct_hash {
                sink.put(i, Ok(h));
                continue;
            }
        }

        // Cold `>= 4 MiB`: stream into 64 KiB `Pipeline` rooms
        // at ~20 MiB RSS, overlapping NVMe DMA with multi-core hashing!
        if !uring_unavailable && cold.is_none() {
            cold = ColdRingState::new(max_slots);
            if cold.is_none() {
                uring_unavailable = true;
            }
        }

        let mut retire_cold_ring = false;
        if let Some(ref mut c) = cold {
            let pool_workers = if args.num_threads().is_some() { (total_threads - io_threads + 1).max(1) } else { total_threads.min(16) };
            crate::init_pool_once(pool_workers);
            let cpipe = c.cold_large_pipe.get_or_insert_with(|| Pipeline::<usize, ()>::new(args.mode, Limits { messages: 2, rooms: LARGE_WINDOW }));
            let mut m = block_on(cpipe.message(i, Some(size)));
            let (stream_res, committed_any) = stream_fd_into_rooms(&mut c.ring, &mut c.deferred_cqes, cpipe, &mut m, file.as_raw_fd(), size);
            match stream_res {
                Ok(_committed_bytes) => {
                    cpipe.end(m);
                    let h = loop {
                        if let Some(Event::Hashed { hash, .. }) = cpipe.next_event() {
                            break hash;
                        }
                        block_on(cpipe.ready());
                    };
                    sink.put(i, Ok(h));
                }
                Err(e) => {
                    cpipe.abort(m);
                    while cpipe.next_event().is_none() {
                        block_on(cpipe.ready());
                    }
                    if !committed_any && matches!(e.raw_os_error(), Some(libc::EINVAL | libc::EOPNOTSUPP | libc::ESPIPE)) {
                        if matches!(e.raw_os_error(), Some(libc::EINVAL | libc::EOPNOTSUPP)) {
                            uring_unavailable = true;
                            retire_cold_ring = true;
                        }
                        sink.put(i, crate::hash_open_file(args, file));
                    } else {
                        sink.put(i, Err(e.into()));
                    }
                }
            }
        } else {
            sink.put(i, crate::hash_open_file(args, file));
        }
        if retire_cold_ring && let Some(c) = cold.take() {
            c.finish(args, &mut warm_batch, &mut sink);
        }
    }

    if let Some(c) = cold {
        c.finish(args, &mut warm_batch, &mut sink);
    }
    warm_batch.flush(args.mode, &mut sink);
    if let Some(mut wp) = warm_pipe {
        wp.finish(&mut sink);
    }
}

#[cfg(test)]
mod test {
    use std::os::fd::AsRawFd;
    use std::path::PathBuf;
    use std::sync::atomic::Ordering;

    #[test]
    fn the_readers_grow_with_the_files() {
        // (threads, files) -> readers: never more than the files or the
        // threads; four up to 63 files, then one more every 32, to twelve.
        for (threads, files, want) in [
            (1, 1000, 1),
            (16, 1, 1),
            (16, 3, 3),
            (16, 4, 4),
            (16, 31, 4),
            (16, 63, 4),
            (16, 64, 5),
            (16, 100, 6),
            (16, 287, 11),
            (16, 288, 12),
            (16, 100_000, 12),
            (8, 1000, 8),
            (2, 100, 2),
        ] {
            assert_eq!(super::readers(threads, files), want, "{threads} threads, {files} files");
        }
    }

    #[test]
    fn sync_fallback_pread_and_uring_disabled_mode_match_reference() {
        blake3::initialize();
        let dir = crate::tempfile::tempdir_in(".").unwrap_or_else(|_| crate::tempfile::tempdir().unwrap());
        let p = dir.path().join("grow");
        let content: Vec<u8> = (0..100_000u32).map(|i| (i * 7) as u8).collect();
        std::fs::write(&p, &content).unwrap();
        let mut f = std::fs::File::open(&p).unwrap();
        let mut buf = Vec::with_capacity(16);
        let (eof, seq) = crate::file_end::pread_rest_capped(&mut f, &mut buf, crate::file_end::SMALL_MEM_CAP).unwrap();
        assert!(eof && !seq);
        assert_eq!(buf.len(), content.len());
        assert_eq!(&buf[..], &content[..]);

        let mut slice_buf = vec![0u8; 5000];
        let m = super::pread_fill_rest(f.as_raw_fd(), &mut slice_buf, 123).unwrap();
        assert_eq!(m, 5000);
        assert_eq!(&slice_buf[..], &content[123..5123]);

        // Direct unit coverage of `applies_to_paths`, `WarmBatch` (count=1 and count=16), and `hash_single_cold_large_file_1t`.
        let stdin_ctx = crate::file_end::StdinContext::current();
        let a_two = crate::Args::for_test_plain(vec![p.clone(), p.clone()]);
        assert!(!super::applies_to_paths(&a_two, &[p.clone()], &stdin_ctx));
        assert!(!super::applies_to_paths(&a_two, &[p.clone(), PathBuf::from("-")], &stdin_ctx));
        assert!(super::applies_to_paths(&a_two, &[p.clone(), p.clone()], &stdin_ctx));
        if let Some(res) = super::hash_single_cold_large_file_1t(&a_two, &f, content.len() as u64) {
            let (h, bytes) = res.unwrap();
            assert_eq!(h, blake3::convenience::hash(&content));
            assert_eq!(bytes, content.len() as u64);
        }
        // Understated size hint still hashes the full file and reports exact committed bytes!
        if let Some(res) = super::hash_single_cold_large_file_1t(&a_two, &f, (content.len() / 3) as u64) {
            let (h, bytes) = res.unwrap();
            assert_eq!(h, blake3::convenience::hash(&content));
            assert_eq!(bytes, content.len() as u64);
        }

        let mut wb = super::WarmBatch::new();
        assert!(!wb.has_arena());
        let mut wb_out = Vec::new();
        {
            let mut collect = |i, r: crate::err::Result<blake3::Hash>| {
                wb_out.push((i, r.unwrap()));
                true
            };
            let mut sink = super::Sink { done: &mut collect, open: true };
            wb.push_slice(0, b"abc", blake3::Mode::Plain, &mut sink);
            assert!(wb.has_arena());
            wb.flush(blake3::Mode::Plain, &mut sink);
        }
        assert_eq!(wb_out, vec![(0, blake3::convenience::hash(b"abc"))]);

        wb_out.clear();
        {
            let mut collect = |i, r: crate::err::Result<blake3::Hash>| {
                wb_out.push((i, r.unwrap()));
                true
            };
            let mut sink = super::Sink { done: &mut collect, open: true };
            for k in 0..super::WARM_BATCH_SLOTS {
                let msg = vec![(k + 1) as u8; k * 13 + 1];
                wb.push_slice(k, &msg, blake3::Mode::Plain, &mut sink);
            }
        }
        assert_eq!(wb_out.len(), super::WARM_BATCH_SLOTS);
        for (k, &(idx, h)) in wb_out.iter().enumerate() {
            assert_eq!(idx, k);
            assert_eq!(h, blake3::convenience::hash(&vec![(k + 1) as u8; k * 13 + 1]));
        }

        // A file that grew past the batch's 16 KiB after its size was
        // read, and still fits its slot, hashes as its bytes do: alone in
        // a batch and beside others.
        let slot = super::WARM_SLOT_BLOCKS * 64;
        for lens in [&[super::WARM_BATCH_MAX_FILE + 1][..], &[slot][..], &[7, super::WARM_BATCH_MAX_FILE + 1, slot - 1, slot, 0][..]] {
            wb_out.clear();
            let mut collect = |i, r: crate::err::Result<blake3::Hash>| {
                wb_out.push((i, r.unwrap()));
                true
            };
            let mut sink = super::Sink { done: &mut collect, open: true };
            let messages: Vec<Vec<u8>> = lens.iter().map(|&len| (0..len).map(|i| (i * 5 + len) as u8).collect()).collect();
            for (k, msg) in messages.iter().enumerate() {
                wb.push_slice(k, msg, blake3::Mode::Plain, &mut sink);
            }
            wb.flush(blake3::Mode::Plain, &mut sink);
            let want: Vec<_> = messages.iter().enumerate().map(|(k, msg)| (k, blake3::convenience::hash(msg))).collect();
            assert_eq!(wb_out, want, "{lens:?}");
        }

        // Fault injection: exercise `force_no_uring = true` across cold evicted files of all size classes!
        let mut paths: Vec<PathBuf> = Vec::new();
        let mut want: Vec<blake3::Hash> = Vec::new();
        for (i, &len) in [0usize, 19, 4096, 16384, 90_000, 600_000].iter().enumerate() {
            let fp = dir.path().join(format!("nouring-{i}"));
            let data: Vec<u8> = (0..len).map(|k| (k * 11 + i) as u8).collect();
            {
                use std::io::Write;
                let mut wf = std::fs::File::create(&fp).unwrap();
                wf.write_all(&data).unwrap();
                wf.sync_all().unwrap();
            }
            let rf = std::fs::File::open(&fp).unwrap();
            unsafe { libc::posix_fadvise(rf.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED) };
            want.push(blake3::convenience::hash(&data));
            paths.push(fp);
        }
        let args = crate::Args::for_test_plain(paths.clone());
        let mut got = Vec::new();
        super::hash_paths_inner(&args, &paths, None, true, &mut |idx, res| {
            assert_eq!(idx, got.len());
            got.push(res.unwrap());
            true
        });
        assert_eq!(got, want);
    }

    #[test]
    fn stream_fd_into_rooms_handles_boundaries_and_read_errors_cleanly() {
        blake3::initialize();
        let Ok(mut ring) = crate::uring::Uring::new(64) else { return };
        crate::init_pool_once(2);

        let dir = crate::tempfile::tempdir_in(".").unwrap_or_else(|_| crate::tempfile::tempdir().unwrap());
        for &len in &[0usize, 1, 65_535, 65_536, 65_537, 600_000, 4_500_000] {
            let p = dir.path().join(format!("large-{len}"));
            let data: Vec<u8> = (0..len).map(|i| (i.wrapping_mul(31) ^ (i >> 8)) as u8).collect();
            {
                use std::io::Write;
                let mut wf = std::fs::File::create(&p).unwrap();
                wf.write_all(&data).unwrap();
                wf.sync_all().unwrap();
            }
            let rf = std::fs::File::open(&p).unwrap();
            unsafe { libc::posix_fadvise(rf.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED) };

            let mut pipe = blake3::throughput::Pipeline::<usize, ()>::new(
                blake3::Mode::DeriveKey("b3sum-uring-room-test"),
                blake3::throughput::Limits { messages: 2, rooms: super::LARGE_WINDOW },
            );
            let mut deferred = std::collections::VecDeque::new();
            let mut m = blake3::throughput::block_on(pipe.message(0, Some(len as u64)));
            super::stream_fd_into_rooms(&mut ring, &mut deferred, &mut pipe, &mut m, rf.as_raw_fd(), (len / 3) as u64).0.unwrap();
            pipe.end(m);
            let got = loop {
                if let Some(blake3::throughput::Event::Hashed { hash, .. }) = pipe.next_event() {
                    break hash;
                }
                blake3::throughput::block_on(pipe.ready());
            };
            let mut want_h = blake3::convenience::Hasher::new(blake3::Mode::DeriveKey("b3sum-uring-room-test"), None);
            want_h.update(&data);
            assert_eq!(got, want_h.finalize(), "room stream len={len}");
        }

        // Fault injection: passing a directory fd (`EISDIR`) or `-1` (`EBADF`) to `stream_fd_into_rooms`
        // drains all in-flight rooms cleanly and returns `Err` without aborting `Pipeline`!
        let dir_file = std::fs::File::open(dir.path()).unwrap();
        let mut pipe =
            blake3::throughput::Pipeline::<usize, ()>::new(blake3::Mode::Plain, blake3::throughput::Limits { messages: 2, rooms: super::LARGE_WINDOW });
        let mut deferred = std::collections::VecDeque::new();
        let mut m = blake3::throughput::block_on(pipe.message(9, Some(1_000_000)));
        let (res, committed) = super::stream_fd_into_rooms(&mut ring, &mut deferred, &mut pipe, &mut m, dir_file.as_raw_fd(), 1_000_000);
        let err = res.unwrap_err();
        assert!(!committed);
        assert_eq!(err.raw_os_error(), Some(libc::EISDIR));
        pipe.abort(m);
        while pipe.next_event().is_none() {
            blake3::throughput::block_on(pipe.ready());
        }
    }

    /// Files of every size class, `count` of them, and their hashes.
    fn files_of_every_size(dir: &std::path::Path, count: usize) -> (Vec<PathBuf>, Vec<blake3::Hash>) {
        let sizes = [0usize, 1, 63, 64, 65, 4096, 16_384, 16_385, 100_000, 524_287, 524_288, 600_000, 1_048_579, 4_194_303, 4_194_304, 4_300_000, 9_000_000];
        let mut paths = Vec::new();
        let mut want = Vec::new();
        for i in 0..count {
            let len = sizes[i % sizes.len()];
            let path = dir.join(format!("size-{i:03}"));
            let data: Vec<u8> = (0..len).map(|k| (k.wrapping_mul(29) ^ (k >> 9) ^ i) as u8).collect();
            std::fs::write(&path, &data).unwrap();
            want.push(blake3::convenience::hash(&data));
            paths.push(path);
        }
        (paths, want)
    }

    /// Every result of `hash_paths_inner`, which must arrive in order.
    fn hashes_in_order(paths: &[PathBuf]) -> Vec<blake3::Hash> {
        let args = crate::Args::for_test_plain(paths.to_vec());
        let mut got = Vec::new();
        super::hash_paths_inner(&args, paths, None, false, &mut |index, result| {
            assert_eq!(index, got.len(), "results arrive in order");
            got.push(result.unwrap());
            true
        });
        got
    }

    #[test]
    fn forced_misses_take_the_cold_paths_for_every_size_class() {
        blake3::initialize();
        let dir = crate::tempfile::tempdir_in(".").unwrap_or_else(|_| crate::tempfile::tempdir().unwrap());
        // 34 files: the wide form (32 files and more). Then 5: the narrow one.
        let (paths, want) = files_of_every_size(dir.path(), 34);
        assert_eq!(hashes_in_order(&paths), want, "from the page cache");
        super::FORCE_MISS.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(hashes_in_order(&paths), want, "every file taken for a miss");
        assert_eq!(hashes_in_order(&paths[12..17]), want[12..17], "five files of 1 MiB and more");

        // A listener that stops: it hears nothing after the result it refused.
        let args = crate::Args::for_test_plain(paths.clone());
        let mut heard = Vec::new();
        super::hash_paths_inner(&args, &paths, None, false, &mut |index, result| {
            assert_eq!(result.unwrap(), want[index]);
            heard.push(index);
            index < 7
        });
        assert_eq!(heard, (0..=7).collect::<Vec<_>>());

        // A file that cannot be read is an error at its own index, and the rest hash.
        let mut with_missing = paths[..6].to_vec();
        with_missing.insert(3, dir.path().join("no-such-file"));
        let args = crate::Args::for_test_plain(with_missing.clone());
        let mut results = Vec::new();
        super::hash_paths_inner(&args, &with_missing, None, false, &mut |index, result| {
            assert_eq!(index, results.len());
            results.push(result.ok());
            true
        });
        let expected: Vec<Option<blake3::Hash>> = vec![Some(want[0]), Some(want[1]), Some(want[2]), None, Some(want[3]), Some(want[4]), Some(want[5])];
        assert_eq!(results, expected);
    }

    #[test]
    fn forced_misses_with_one_thread_hash_every_size_class() {
        blake3::initialize();
        crate::THREADS.set(std::num::NonZeroUsize::MIN).expect("this test's process sets the thread count first");
        let dir = crate::tempfile::tempdir_in(".").unwrap_or_else(|_| crate::tempfile::tempdir().unwrap());
        let (paths, want) = files_of_every_size(dir.path(), 34);
        super::FORCE_MISS.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(hashes_in_order(&paths), want);
    }

    /// More bytes than a batch's slot holds are refused before they are
    /// copied into it, in every build.
    #[test]
    #[should_panic(expected = "a batched file fits its slot")]
    fn a_file_beyond_its_slot_is_refused_before_the_copy() {
        blake3::initialize();
        let mut wb = super::WarmBatch::new();
        let mut collect = |_, _: crate::err::Result<blake3::Hash>| true;
        let mut sink = super::Sink { done: &mut collect, open: true };
        let msg = vec![0u8; super::WARM_SLOT_BLOCKS * 64 + 1];
        wb.push_slice(0, &msg, blake3::Mode::Plain, &mut sink);
    }

    /// A file is read to where a read gives no byte, whatever room its
    /// reported size chose: room for less than the file is filled and
    /// not called the end, and room for more ends at the file's last
    /// byte. (0.9.0's test of the rule, for the read that replaced
    /// that release's.)
    #[test]
    fn a_cached_read_ends_only_where_a_read_gives_no_byte() {
        // Beside the test's executable: a temporary directory in memory
        // (a tmpfs) has no reads from the page cache alone to test.
        let dir = crate::tempfile::tempdir_in(std::env::current_exe().unwrap().parent().unwrap()).unwrap();
        let path = dir.path().join("f");
        let content: Vec<u8> = (0..100_000u32).map(|i| (i * 7) as u8).collect();
        std::fs::write(&path, &content).unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let mut probe = [core::mem::MaybeUninit::new(0u8); 1];
        if matches!(super::try_pread_cached(std::os::fd::AsRawFd::as_raw_fd(&file), &mut probe, 0).unwrap(), crate::file_end::CacheRead::NeedMore(0)) {
            eprintln!("no reads from the page cache alone in {}", dir.path().display());
            return;
        }
        for room in [1usize, 4095, 4096, 99_999, 100_000, 100_001, 300_000] {
            let mut buffer = vec![core::mem::MaybeUninit::new(0u8); room];
            let got = super::try_pread_cached(std::os::fd::AsRawFd::as_raw_fd(&file), &mut buffer, 0).unwrap();
            let (n, end) = match got {
                crate::file_end::CacheRead::Eof(n) => (n, true),
                crate::file_end::CacheRead::NeedMore(n) => (n, false),
                crate::file_end::CacheRead::NonSeekable(_) => panic!("regular file is seekable"),
            };
            assert_eq!((n, end), (room.min(content.len()), room > content.len()), "room={room}");
            let init = unsafe { core::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), n) };
            assert_eq!(init, &content[..n]);
            // From where a first read stopped, as a file that grew is read on.
            let mut rest = vec![core::mem::MaybeUninit::new(0u8); content.len() + 1];
            let got2 = super::try_pread_cached(std::os::fd::AsRawFd::as_raw_fd(&file), &mut rest, n).unwrap();
            assert_eq!(got2, crate::file_end::CacheRead::Eof(content.len()), "room={room}");
            let rest_init = unsafe { core::slice::from_raw_parts(rest.as_ptr().cast::<u8>(), content.len()) };
            assert_eq!(&rest_init[n..content.len()], &content[n..]);
        }
    }

    /// Growing the descriptor table leaves a descriptor the caller holds
    /// at the very number it aims for open (`b3sum /dev/fd/255 other`).
    #[test]
    fn growing_the_descriptor_table_closes_no_descriptor() {
        use std::os::fd::AsRawFd;
        let file = std::fs::File::open("/dev/null").unwrap();
        // 200 descriptors ask for a table of 256: the aim is number 255.
        let held = unsafe { libc::dup2(file.as_raw_fd(), 255) };
        assert_eq!(held, 255);
        super::grow_descriptor_table(200);
        let still = unsafe { libc::fcntl(255, libc::F_GETFD) };
        unsafe { libc::close(255) };
        assert_ne!(still, -1, "the caller's descriptor 255 was closed");
    }

    /// A file that grew after its size was read: the bytes already read
    /// are more than the size said, and the buffer holds them all as its
    /// initialized `len()`.
    #[test]
    fn a_buffer_holds_what_was_read_whatever_the_size_said() {
        let read: Vec<u8> = (0..super::WARM_SLOT_BLOCKS * 64).map(|i| i as u8).collect();
        let mut buf = Vec::new();
        let n = super::buffer_with_start(&mut buf, 10, Some(&read));
        assert_eq!(n, read.len());
        assert!(buf.capacity() >= n + 4096 && buf.len() == n);
        assert_eq!(buf, read);
        let mut none = vec![1u8; 7];
        assert_eq!(super::buffer_with_start(&mut none, 10_000, None), 0);
        assert!(none.capacity() > 10_000 && none.is_empty());
    }

    /// The system's own reads follow that order: the first asks the
    /// cache alone, the one that finds the end may wait. (A reviewer's
    /// planted fault gave every real read `RWF_NOWAIT` and passed the
    /// test below, which plays the system itself.)
    #[test]
    fn the_read_that_ends_a_cached_file_may_wait() {
        use std::os::fd::AsRawFd;
        // Beside the test's own program: /tmp is often a file system
        // that has no reads of the cache alone.
        let dir = crate::tempfile::tempdir_in(std::env::current_exe().unwrap().parent().unwrap()).unwrap();
        let path = dir.path().join("cached");
        std::fs::write(&path, [7u8; 5000]).unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let mut buffer = vec![core::mem::MaybeUninit::new(0u8); 5001];
        super::ASKED.with(|asked| asked.borrow_mut().clear());
        let read = super::try_pread_cached(file.as_raw_fd(), &mut buffer, 0).unwrap();
        let asked = super::ASKED.with(|asked| asked.borrow().clone());
        if read == crate::file_end::CacheRead::NeedMore(0) {
            // A file system without reads of the cache alone: nothing to tell.
            assert_eq!(asked, [libc::RWF_NOWAIT]);
            return;
        }
        assert_eq!(read, crate::file_end::CacheRead::Eof(5000));
        assert_eq!(asked, [libc::RWF_NOWAIT, 0]);
        let init = unsafe { core::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), 5000) };
        assert_eq!(init, &[7u8; 5000]);
    }

    /// A read of the cache alone that gives no byte does not end a
    /// file: the kernels that give none where they would have to wait
    /// (Linux 5.9 and 5.10) are played here by `read`, whose answers
    /// are listed with what was asked. The end is where a read that may
    /// wait gives no byte; a real blocking `EIO` error propagates instead
    /// of being swallowed as a cache miss; `ESPIPE` selects sequential fallback.
    #[test]
    fn a_read_of_the_cache_that_gives_no_byte_does_not_end_the_file() {
        use crate::file_end::{CacheRead, cached_then_plain};
        let content = b"0123456789";
        let fail_with = |errno: libc::c_int| -> isize {
            // Sound: this thread's errno.
            unsafe { *libc::__errno_location() = errno };
            -1
        };
        let run = |room: usize, from: usize, cache: &dyn Fn(usize) -> isize, plain: &dyn Fn(usize) -> isize| {
            let mut asked = Vec::new();
            let got = cached_then_plain(room, from, |at, flags| {
                asked.push((at, flags != 0));
                if flags != 0 {
                    assert_eq!(flags, libc::RWF_NOWAIT);
                    cache(at)
                } else {
                    plain(at)
                }
            });
            (got, asked)
        };
        // A plain read: up to four bytes of the file at a time.
        let plain = |at: usize| (content.len() - at).min(4) as isize;
        // The cache gives no byte of a file that has ten.
        let (got, asked) = run(64, 0, &|_| 0, &plain);
        assert_eq!((got.unwrap(), asked), (CacheRead::Eof(10), vec![(0, true), (0, false), (4, false), (8, false), (10, false)]));
        // It gives six: the rest and the end come from reads that may wait.
        let (got, asked) = run(64, 0, &|at| (6 - at) as isize, &plain);
        assert_eq!((got.unwrap(), asked), (CacheRead::Eof(10), vec![(0, true), (6, false), (10, false)]));
        // It gives all ten: one read more, which may wait, says the end.
        let (got, asked) = run(64, 0, &|at| (10 - at) as isize, &plain);
        assert_eq!((got.unwrap(), asked), (CacheRead::Eof(10), vec![(0, true), (10, false)]));
        // A file of no bytes ends where a read that may wait says so.
        let (got, asked) = run(64, 0, &|_| 0, &|_| 0);
        assert_eq!((got.unwrap(), asked), (CacheRead::Eof(0), vec![(0, true), (0, false)]));
        // The cache would have to wait: the caller reads on, from the start.
        let (got, asked) = run(64, 0, &|_| fail_with(libc::EAGAIN), &plain);
        assert_eq!((got.unwrap(), asked), (CacheRead::NeedMore(0), vec![(0, true)]));
        // A full buffer is not the end.
        let (got, asked) = run(10, 0, &|at| (10 - at) as isize, &plain);
        assert_eq!((got.unwrap(), asked), (CacheRead::NeedMore(10), vec![(0, true)]));
        let (got, asked) = run(8, 0, &|_| 0, &plain);
        assert_eq!((got.unwrap(), asked), (CacheRead::NeedMore(8), vec![(0, true), (0, false), (4, false)]));
        // A blocking read returning EIO must propagate as Err (never swallowed as cache miss!).
        let (got, asked) = run(64, 0, &|at| (6 - at) as isize, &|_| fail_with(libc::EIO));
        assert_eq!(got.unwrap_err().raw_os_error(), Some(libc::EIO));
        assert_eq!(asked, vec![(0, true), (6, false)]);
        // ESPIPE on a non-seekable regular file returns NonSeekable so caller reads sequentially.
        let (got, asked) = run(64, 0, &|_| fail_with(libc::ESPIPE), &plain);
        assert_eq!((got.unwrap(), asked), (CacheRead::NonSeekable(0), vec![(0, true)]));
        // From where an earlier read stopped.
        let (got, asked) = run(64, 6, &|_| 0, &plain);
        assert_eq!((got.unwrap(), asked), (CacheRead::Eof(10), vec![(6, true), (6, false), (10, false)]));
    }

    /// Reads that come back short and reads that are interrupted come to
    /// the same bytes: with every other completion of a ring cut to half
    /// its bytes (1) or made an interruption (2), files of every size
    /// class, all taken for misses, hash as they do untouched. (Where no
    /// ring can be made the reads are of another kind, and this says
    /// nothing.)
    fn spoiled_ring_reads_come_to_the_same_hashes() {
        let dir = crate::tempfile::tempdir_in(".").unwrap_or_else(|_| crate::tempfile::tempdir().unwrap());
        let (paths, want) = files_of_every_size(dir.path(), 34);
        super::FORCE_MISS.store(true, std::sync::atomic::Ordering::Relaxed);
        for spoil in [1, 2] {
            crate::uring::SPOIL.store(spoil, std::sync::atomic::Ordering::Relaxed);
            let before = crate::uring::SPOILED.load(std::sync::atomic::Ordering::Relaxed);
            assert_eq!(hashes_in_order(&paths), want, "spoil {spoil}");
            assert_eq!(hashes_in_order(&paths[12..17]), want[12..17], "spoil {spoil}: five files of 1 MiB and more");
            let spoiled = crate::uring::SPOILED.load(std::sync::atomic::Ordering::Relaxed) - before;
            if crate::uring::Uring::new(8).is_ok() {
                assert!(spoiled > 20, "spoil {spoil}: only {spoiled} completions were changed");
            }
        }
        crate::uring::SPOIL.store(0, std::sync::atomic::Ordering::Relaxed);
    }

    #[test]
    fn short_and_interrupted_ring_reads_come_to_the_same_hashes() {
        blake3::initialize();
        spoiled_ring_reads_come_to_the_same_hashes();
    }

    #[test]
    fn short_and_interrupted_ring_reads_on_one_thread_come_to_the_same_hashes() {
        blake3::initialize();
        crate::THREADS.set(std::num::NonZeroUsize::MIN).expect("this test's process sets the thread count first");
        spoiled_ring_reads_come_to_the_same_hashes();
    }

    /// The file opened is the file hashed, where there is no io_uring too:
    /// each path is replaced by another file just after it is opened, and
    /// the hashes are the first files'. (That fallback opened the name a
    /// second time, and hashed whatever it named by then.)
    #[test]
    fn without_a_ring_the_file_opened_is_the_file_hashed() {
        blake3::initialize();
        let dir = crate::tempfile::tempdir().unwrap();
        // A regular file past the small files' reads, and a pipe-like
        // input that is no regular file: the two places that opened again.
        let first: Vec<u8> = (0..crate::MAP_LEN + 4096).map(|i| (i * 13) as u8).collect();
        let paths: Vec<PathBuf> = (0..2).map(|i| dir.path().join(format!("f{i}"))).collect();
        for path in &paths {
            std::fs::write(path, &first).unwrap();
        }
        *super::AFTER_OPEN.lock().unwrap() = Some(Box::new(|path| {
            std::fs::rename(path, path.with_extension("moved")).unwrap();
            std::fs::write(path, b"another file").unwrap();
        }));
        super::set_force_miss(true);
        let args = crate::Args::for_test_plain(paths.clone());
        let mut got = Vec::new();
        super::hash_paths_inner(&args, &paths, None, true, &mut |i, hash| {
            got.push((i, hash.unwrap()));
            true
        });
        super::set_force_miss(false);
        *super::AFTER_OPEN.lock().unwrap() = None;
        got.sort_by_key(|(i, _)| *i);
        let want = blake3::convenience::hash(&first);
        assert_eq!(got, vec![(0, want), (1, want)]);
    }

    /// When a ring sets up but `IORING_OP_READ` is unsupported (e.g. Linux 5.1..5.5),
    /// `Uring::new` rejects the ring cleanly and multi-file cold hashing falls
    /// back to synchronous reads with exact digests.
    #[test]
    fn unsupported_op_read_falls_back_cleanly() {
        blake3::initialize();
        let dir = crate::tempfile::tempdir_in(".").unwrap_or_else(|_| crate::tempfile::tempdir().unwrap());
        let (paths, want) = files_of_every_size(dir.path(), 6);
        crate::uring::REFUSE_OP_READ.store(true, std::sync::atomic::Ordering::Relaxed);
        super::FORCE_MISS.store(true, std::sync::atomic::Ordering::Relaxed);
        let got = hashes_in_order(&paths);
        crate::uring::REFUSE_OP_READ.store(false, std::sync::atomic::Ordering::Relaxed);
        super::FORCE_MISS.store(false, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(got, want);
    }

    /// A path whose opened descriptor turns out to be the forbidden checkfile
    /// stream (e.g. swapped via symlink after parsing) is refused before any
    /// byte is read from it.
    #[test]
    fn forbidden_stream_checked_on_opened_descriptor() {
        use std::io::Write;
        use std::os::fd::AsRawFd;
        blake3::initialize();
        let dir = crate::tempfile::tempdir().unwrap();
        let fifo = dir.path().join("manifest.fifo");
        assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
        // Open O_RDWR on the FIFO so a reader and writer stay open without blocking
        // when `hash_paths_inner` opens `&fifo` a second time.
        let mut rw = std::fs::OpenOptions::new().read(true).write(true).open(&fifo).unwrap();
        rw.write_all(b"untouched-manifest-bytes").unwrap();
        let reader = std::fs::File::open(&fifo).unwrap();
        let meta = crate::file_end::fstat_fd(reader.as_raw_fd()).unwrap();
        let other = dir.path().join("other");
        std::fs::write(&other, b"ok").unwrap();
        let paths = vec![fifo.clone(), other.clone()];
        let args = crate::Args::for_test_plain(paths.clone());
        let mut results = Vec::new();
        super::hash_paths_inner(&args, &paths, Some(meta.id), true, &mut |idx, res| {
            assert_eq!(idx, results.len());
            results.push(res);
            true
        });
        assert!(results[0].is_err(), "opened descriptor matching forbidden stream must be refused");
        assert_eq!(results[1].as_ref().unwrap(), &blake3::convenience::hash(b"ok"));
        // Close the write end (O_RDWR handle) and verify none of the FIFO bytes were consumed!
        drop(rw);
        let mut remaining = String::new();
        std::io::Read::read_to_string(&mut &reader, &mut remaining).unwrap();
        assert_eq!(remaining, "untouched-manifest-bytes");
    }

    #[test]
    fn multi_file_direct_threads_bounds_total_readers() {
        for total in [2usize, 4, 8, 16, 48, 112] {
            for workers in [1usize, 2, 3, 4, 6, 8, 12] {
                if workers > total {
                    continue;
                }
                let per = super::multi_file_direct_threads(total, workers);
                assert!(per >= 1, "total={total} workers={workers}");
                assert!(per * workers <= crate::DIRECT_THREADS, "total={total} workers={workers} per={per}");
                assert!(per * workers <= total, "total={total} workers={workers} per={per}");
            }
        }
        assert_eq!(super::multi_file_direct_threads(16, 4), 4);
        assert_eq!(super::multi_file_direct_threads(56, 4), 4);
        assert_eq!(super::multi_file_direct_threads(16, 2), 8);
        assert_eq!(super::multi_file_direct_threads(4, 4), 1);
        assert_eq!(super::multi_file_direct_threads(48, 12), 1);
    }

    #[test]
    fn cold_large_multi_file_uses_direct_io_and_matches_reference() {
        use std::os::unix::fs::OpenOptionsExt;
        blake3::initialize();
        // Force a 16-thread budget so 3 files -> io_threads=3 -> want_direct=5 >= 4
        // deterministically even on 4-CPU CI runners.
        crate::THREADS.set(std::num::NonZeroUsize::new(16).unwrap()).expect("fresh test process");
        let dir = crate::tempfile::tempdir_in(".").unwrap_or_else(|_| crate::tempfile::tempdir().unwrap());
        // Pin literal 32 MiB threshold (and 32 MiB - 1 below it).
        let at_32m: usize = 32 * 1024 * 1024;
        let below_32m: usize = at_32m - 1;
        let make = |seed: u8, len: usize| -> Vec<u8> {
            let mut out = vec![0u8; len];
            blake3::convenience::Hasher::new(blake3::Mode::Plain, None).update(&[seed]).finalize_xof().fill(&mut out);
            out
        };
        let a = make(1, at_32m);
        let b = make(2, at_32m + 4096 + 13);
        let c = make(3, below_32m);
        let paths = vec![dir.path().join("big-a"), dir.path().join("big-b"), dir.path().join("below-c")];
        std::fs::write(&paths[0], &a).unwrap();
        std::fs::write(&paths[1], &b).unwrap();
        std::fs::write(&paths[2], &c).unwrap();
        let want = vec![blake3::convenience::hash(&a), blake3::convenience::hash(&b), blake3::convenience::hash(&c)];

        let supports_direct = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_DIRECT).open(&paths[0]).is_ok();
        super::DIRECT_MULTI_SUCCEEDED.store(0, Ordering::Relaxed);
        super::DIRECT_MULTI_LAST_THREADS.store(0, Ordering::Relaxed);
        super::set_force_miss(true);
        let got_cold = hashes_in_order(&paths);
        super::set_force_miss(false);
        assert_eq!(got_cold, want);
        if supports_direct {
            assert_eq!(super::DIRECT_MULTI_SUCCEEDED.load(Ordering::Relaxed), 2, "only the two >= 32 MiB cold files should succeed on O_DIRECT");
            assert_eq!(super::DIRECT_MULTI_LAST_THREADS.load(Ordering::Relaxed), 5, "3 workers on 16 threads get 16/3 = 5 direct threads");
        }

        // Warm files (just written, FORCE_MISS = false) must not enter O_DIRECT.
        let before_warm = super::DIRECT_MULTI_SUCCEEDED.load(Ordering::Relaxed);
        let got_warm = hashes_in_order(&paths);
        assert_eq!(got_warm, want);
        assert_eq!(super::DIRECT_MULTI_SUCCEEDED.load(Ordering::Relaxed), before_warm, "warm >= 32 MiB files must stay on mapped WarmLargePipe");
    }

    #[test]
    fn cold_large_multi_file_skips_direct_with_few_threads() {
        blake3::initialize();
        crate::THREADS.set(std::num::NonZeroUsize::new(4).unwrap()).expect("fresh test process");
        let dir = crate::tempfile::tempdir_in(".").unwrap_or_else(|_| crate::tempfile::tempdir().unwrap());
        let at_32m: usize = 32 * 1024 * 1024;
        let mut a = vec![0u8; at_32m];
        let mut b = vec![0u8; at_32m + 17];
        blake3::convenience::Hasher::new(blake3::Mode::Plain, None).update(b"f1").finalize_xof().fill(&mut a);
        blake3::convenience::Hasher::new(blake3::Mode::Plain, None).update(b"f2").finalize_xof().fill(&mut b);
        let paths = vec![dir.path().join("f-a"), dir.path().join("f-b")];
        std::fs::write(&paths[0], &a).unwrap();
        std::fs::write(&paths[1], &b).unwrap();
        let want = vec![blake3::convenience::hash(&a), blake3::convenience::hash(&b)];

        let args = crate::Args::for_test_plain_threads(paths.clone(), 4);
        super::DIRECT_MULTI_SUCCEEDED.store(0, Ordering::Relaxed);
        super::set_force_miss(true);
        let mut got = vec![None; 2];
        super::hash_paths_inner(&args, &paths, None, false, &mut |i, res| {
            got[i] = Some(res.unwrap());
            true
        });
        super::set_force_miss(false);
        assert_eq!(got.into_iter().map(Option::unwrap).collect::<Vec<_>>(), want);
        assert_eq!(super::DIRECT_MULTI_SUCCEEDED.load(Ordering::Relaxed), 0, "4 threads across 2 workers (2/worker < 4) must skip O_DIRECT");
    }

    #[test]
    fn cold_large_multi_file_honors_direct_permits_budget() {
        use std::os::unix::fs::OpenOptionsExt;
        blake3::initialize();
        // With 8 total threads and 2 files, `max_file_threads(8) = 4`, `io_threads = 2`,
        // so `want_direct = 4`: each worker leases 3 extra permits -> 2 + 2*3 = 8 total.
        crate::THREADS.set(std::num::NonZeroUsize::new(8).unwrap()).expect("fresh test process");
        let dir = crate::tempfile::tempdir_in(".").unwrap_or_else(|_| crate::tempfile::tempdir().unwrap());
        let at_32m: usize = 32 * 1024 * 1024;
        let mut a = vec![0u8; at_32m];
        let mut b = vec![0u8; at_32m + 17];
        blake3::convenience::Hasher::new(blake3::Mode::Plain, None).update(b"p1").finalize_xof().fill(&mut a);
        blake3::convenience::Hasher::new(blake3::Mode::Plain, None).update(b"p2").finalize_xof().fill(&mut b);
        let paths = vec![dir.path().join("p-a"), dir.path().join("p-b")];
        std::fs::write(&paths[0], &a).unwrap();
        std::fs::write(&paths[1], &b).unwrap();
        let want = vec![blake3::convenience::hash(&a), blake3::convenience::hash(&b)];
        let supports_direct = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_DIRECT).open(&paths[0]).is_ok();

        let args = crate::Args::for_test_plain_threads(paths.clone(), 8);
        super::DIRECT_MULTI_SUCCEEDED.store(0, Ordering::Relaxed);
        super::DIRECT_MULTI_LAST_THREADS.store(0, Ordering::Relaxed);
        super::set_force_miss(true);
        let mut got = vec![None; 2];
        super::hash_paths_inner(&args, &paths, None, false, &mut |i, res| {
            got[i] = Some(res.unwrap());
            true
        });
        super::set_force_miss(false);
        assert_eq!(got.into_iter().map(Option::unwrap).collect::<Vec<_>>(), want);
        if supports_direct {
            assert_eq!(super::DIRECT_MULTI_SUCCEEDED.load(Ordering::Relaxed), 2);
            assert_eq!(super::DIRECT_MULTI_LAST_THREADS.load(Ordering::Relaxed), 4);
        }

        // Pre-lease 4 of the 6 spare permits under a strict guard: each worker can
        // lease at most 2 extra (1 + 2 = 3 < 4), so both must skip O_DIRECT.
        let _strict = crate::FileThreadsGuard::enter(2);
        let held = crate::DirectPermits::acquire(4);
        assert_eq!(held.extra, 4, "all permits released after previous run");
        super::DIRECT_MULTI_SUCCEEDED.store(0, Ordering::Relaxed);
        super::set_force_miss(true);
        let mut got2 = vec![None; 2];
        super::hash_paths_inner(&args, &paths, None, false, &mut |i, res| {
            got2[i] = Some(res.unwrap());
            true
        });
        super::set_force_miss(false);
        drop(held);
        assert_eq!(got2.into_iter().map(Option::unwrap).collect::<Vec<_>>(), want);
        assert_eq!(super::DIRECT_MULTI_SUCCEEDED.load(Ordering::Relaxed), 0, "exhausted DirectPermits must prevent O_DIRECT oversubscription");
    }
}
