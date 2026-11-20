//! Linux `io_uring` + `blake3::throughput::Pipeline` file hashing engine.
//!
//! Serves both multi-file hashing (`b3sum files...`) and checkfile verification
//! (`b3sum --check sums.b3`) through [`hash_paths`]:
//! - **Warm runs**: starts up to 12 threads; small files (`<= 16 KiB`) read via
//!   `preadv2(RWF_NOWAIT)` into a core-local `MaybeUninit<Block>` arena and hash
//!   16-at-a-time via `blake3::throughput::hash_each`; warm files `>= 4 MiB`
//!   loan `&map` zero-copy to `Pipeline::lent` across the worker pool.
//! - **Cold runs**: the first `RWF_NOWAIT` miss opens that worker's
//!   single-issuer `Uring` (`COOP_TASKRUN | SINGLE_ISSUER | DEFER_TASKRUN`).
//!   Files `< 4 MiB` go straight onto it (the kernel's `io_read` serves
//!   cached hits inline and dispatches NVMe reads asynchronously), and each
//!   is hashed on the core that read it, whether or not the pool runs.
//!   Cold files `>= 4 MiB` stream into 64 KiB `Pipeline::room()`s at
//!   ~20 MiB RSS; they are the only cold files that start the pool.

use crate::Args;
use crate::uring::{IoUringCqe, Uring};
use blake3::throughput::{Block, Event, Lent, Limits, Message, Pipeline, ROOM_LEN, Room, block_on};
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
const WARM_BATCH_MAX_FILE: usize = 16 * 1024;
const WARM_SLOT_BLOCKS: usize = (WARM_BATCH_MAX_FILE / 64) + 1;
const WARM_BATCH_SLOTS: usize = 16;

const RING_ENTRIES: u32 = 128;
const SLOTS_PER_RING_MULTI: usize = 48;
const SLOTS_PER_RING_SINGLE: usize = 64;
const LARGE_WINDOW: usize = 64;
const MAX_FILE_ROOMS: u64 = 48;
const USER_LARGE_BIT: u64 = 1 << 63;

pub(crate) fn applies_to_paths(args: &Args, paths: &[PathBuf]) -> bool {
    args.is_standard_32() && paths.len() > 1 && paths.iter().all(|p| !crate::is_stdin(p))
}

pub(crate) fn hash_single_cold_large_file_1t(args: &Args, file: &File, hint_size: u64) -> Option<io::Result<blake3::Hash>> {
    if !args.is_standard_32() {
        return None;
    }
    let mut ring = Uring::acquire(RING_ENTRIES).ok()?;
    crate::init_pool_once(1);
    let mut pipe = Pipeline::<usize, ()>::new(args.mode, Limits { messages: 2, rooms: LARGE_WINDOW });
    let mut deferred_cqes = VecDeque::new();
    let mut msg = block_on(pipe.message(0, Some(hint_size)));
    let out = match stream_fd_into_rooms(&mut ring, &mut deferred_cqes, &mut pipe, &mut msg, file.as_raw_fd(), hint_size) {
        Ok(()) => {
            pipe.end(msg);
            loop {
                if let Some(ev) = pipe.next_event() {
                    match ev {
                        Event::Hashed { hash, .. } => break Ok(hash),
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
            Err(e)
        }
    };
    ring.release();
    Some(out)
}

/// Pre-grow the kernel file-descriptor table while single-threaded, raising
/// `RLIMIT_NOFILE` up to `rlim_max` where needed and clamping to `rlim_cur - 1`
/// so `dup2` never fails with `EBADF` when `want > 1024` (which previously left
/// the table at 64 and caused four 10 ms `synchronize_rcu` stalls across threads).
fn grow_descriptor_table(descriptors: usize) {
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
        let target = want.min(cap).saturating_sub(1) as i32;
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

fn stream_fd_into_rooms(
    ring: &mut Uring,
    deferred_small_cqes: &mut VecDeque<IoUringCqe>,
    pipe: &mut Pipeline<usize, ()>,
    msg: &mut Message,
    fd: RawFd,
    hint_size: u64,
) -> io::Result<()> {
    let mut window: [Option<WindowSlot>; LARGE_WINDOW] = [const { None }; LARGE_WINDOW];
    let mut next_issue_seq: u64 = 0;
    let mut next_commit_seq: u64 = 0;
    let mut max_issue_seq: u64 = (hint_size / (ROOM_LEN as u64)) + 1;
    let mut eof_seq: Option<u64> = None;
    let mut in_flight_uring: usize = 0;
    let mut first_err: Option<io::Error> = None;

    loop {
        while first_err.is_none()
            && eof_seq.is_none()
            && next_issue_seq <= max_issue_seq
            && (next_issue_seq - next_commit_seq) < MAX_FILE_ROOMS
            && ring.sq_space_left() > 0
        {
            let slot_idx = (next_issue_seq as usize) % LARGE_WINDOW;
            if window[slot_idx].is_some() {
                break;
            }
            let mut room = if in_flight_uring == 0 && window[(next_commit_seq as usize) % LARGE_WINDOW].is_none() {
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
            // as Reading until then, and the ring stops the process if it is
            // dropped with a read in flight.
            let ok = unsafe { ring.push_read(fd, room.bytes_mut().as_mut_ptr(), ROOM_LEN, off, 0, user_data) };
            // In every build: a read the full ring did not take would be
            // counted in flight and waited for without end.
            assert!(ok, "the ring has room for every read in flight");
            window[slot_idx] = Some(WindowSlot::Reading(room));
            in_flight_uring += 1;
            next_issue_seq += 1;
        }

        let commit_ready = matches!(window[(next_commit_seq as usize) % LARGE_WINDOW], Some(WindowSlot::Ready(..) | WindowSlot::Eof));
        if ring.unsubmitted() > 0 || (!commit_ready && in_flight_uring > 0) {
            let min_wait = if commit_ready || in_flight_uring == 0 { 0 } else { 1 };
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
            in_flight_uring -= 1;

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
                    in_flight_uring += 1;
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
            wait_out_large_reads(ring, deferred_small_cqes, &mut window, &mut in_flight_uring);
            return Err(e);
        }

        let mut finished = false;
        loop {
            let idx = (next_commit_seq as usize) % LARGE_WINDOW;
            match window[idx].take() {
                Some(WindowSlot::Eof) => {
                    finished = true;
                    break;
                }
                Some(WindowSlot::Ready(mut room, len)) if len < ROOM_LEN => {
                    if len > 0 {
                        pipe.commit(msg, &mut room, len);
                    }
                    drop(room);
                    finished = true;
                    break;
                }
                Some(WindowSlot::Ready(mut room, len)) => {
                    debug_assert_eq!(len, ROOM_LEN);
                    pipe.commit(msg, &mut room, ROOM_LEN);
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
            wait_out_large_reads(ring, deferred_small_cqes, &mut window, &mut in_flight_uring);
            return Ok(());
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
    in_flight: &mut usize,
) {
    while *in_flight > 0 {
        ring.submit_and_wait(1);
        while let Some(cqe) = ring.pop_cqe() {
            if (cqe.user_data & USER_LARGE_BIT) == 0 {
                deferred_small_cqes.push_back(cqe);
                continue;
            }
            let seq = cqe.user_data & !USER_LARGE_BIT;
            let _ = window[(seq as usize) % LARGE_WINDOW].take();
            *in_flight -= 1;
        }
    }
    for slot in window {
        *slot = None;
    }
}

/// Where results go, until the listener says to stop: after `done` returns
/// false, nothing more is delivered.
struct Sink<'a> {
    done: &'a mut dyn FnMut(usize, anyhow::Result<blake3::Hash>) -> bool,
    open: bool,
}

impl Sink<'_> {
    fn put(&mut self, index: usize, result: anyhow::Result<blake3::Hash>) {
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
    fn slot_ptr_and_cap(&mut self, slot: usize) -> (*mut u8, usize) {
        let arena = self.arena.get_or_insert_with(|| Box::new_uninit_slice(WARM_BATCH_SLOTS * WARM_SLOT_BLOCKS));
        let start = slot * WARM_SLOT_BLOCKS;
        // Sound: `slot` is below WARM_BATCH_SLOTS (push_raw flushes at that
        // count), so the slot is inside the arena.
        let ptr = unsafe { arena.as_mut_ptr().add(start).cast::<u8>() };
        (ptr, WARM_SLOT_BLOCKS * 64)
    }

    #[inline]
    fn push_raw(&mut self, file_idx: usize, src_ptr: *const u8, byte_len: usize, mode: blake3::Mode<'_>, sink: &mut Sink<'_>) {
        // In every build: the copy below is into the slot. A file can have
        // grown past WARM_BATCH_MAX_FILE since its size was read (its
        // bytes come to here by a read into a slot's room), and no
        // further than the slot.
        assert!(byte_len <= WARM_SLOT_BLOCKS * 64, "a batched file fits its slot");
        let slot = self.len;
        let (dst, _) = self.slot_ptr_and_cap(slot);
        if byte_len > 0 {
            // Sound: the slot has room for `byte_len` bytes (checked above),
            // and `src_ptr` is that many bytes outside the arena (a caller's
            // buffer or the stack probe).
            unsafe { core::ptr::copy_nonoverlapping(src_ptr, dst, byte_len) };
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
            // read or by push_raw.
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

/// Read the file at `fd` from byte `at` into the `dst_cap` bytes at
/// `dst_ptr`: how far it got, and whether that is the file's end. The
/// first read is of the page cache alone, and where the cache cannot
/// give the next bytes without waiting, the answer is false and the
/// caller reads on (through the ring, where it has one). Only a read
/// that gives no byte says where a file ends: the size a file reports
/// chooses the buffer, never where the file ends (0.9.0's rule, which a
/// shortcut here broke: it took a read that stopped at the reported size
/// for the end, BLAKE3 #424). False too where the buffer is full.
#[inline]
fn try_pread_cached(fd: RawFd, dst_ptr: *mut u8, dst_cap: usize, at: usize) -> io::Result<(usize, bool)> {
    if force_miss() {
        return Ok((at, false));
    }
    cached_then_plain(dst_cap, at, |at, flags| {
        // Sound: `at` is below `dst_cap` here, and the callers give `dst_cap`
        // writable bytes at `dst_ptr`; the kernel writes at most the rest of
        // them.
        let iov = libc::iovec { iov_base: unsafe { dst_ptr.add(at).cast() }, iov_len: dst_cap - at };
        preadv2_asked(fd, &iov, at, flags)
    })
}

/// One `preadv2` with `flags`; the tests keep what was asked.
#[inline]
fn preadv2_asked(fd: RawFd, iov: &libc::iovec, at: usize, flags: libc::c_int) -> isize {
    #[cfg(test)]
    ASKED.with(|asked| asked.borrow_mut().push(flags));
    // Sound: the caller's one iovec over writable bytes.
    unsafe { libc::preadv2(fd, iov, 1, at as libc::off_t, flags) }
}

#[cfg(test)]
thread_local! {
    /// The flags of each read [`try_pread_cached`] made on this thread.
    static ASKED: core::cell::RefCell<Vec<libc::c_int>> = const { core::cell::RefCell::new(Vec::new()) };
}

/// [`try_pread_cached`]'s reads, each made by `read(at, flags)`: the
/// bytes it gave, or -1 with the error in `errno`, as `preadv2` answers
/// (the tests give a file of their own making).
///
/// The read that says a file has ended is one that may wait. A read of
/// the cache alone (`RWF_NOWAIT`) that gives no byte does not say it: on
/// Linux 5.9 and 5.10 such a read can give none where it would have had
/// to wait, with bytes still to come (preadv2(2), BUGS), and that answer
/// was taken for the end, so the hash printed would have been of the
/// bytes before it. So the first read asks the cache, and once it has
/// answered at all, with bytes or with none, the reads after it may
/// wait: a file in the cache costs the two reads it cost before, and
/// its end is said by the second.
#[inline]
fn cached_then_plain(dst_cap: usize, mut at: usize, mut read: impl FnMut(usize, libc::c_int) -> isize) -> io::Result<(usize, bool)> {
    let mut flags = libc::RWF_NOWAIT;
    loop {
        if at == dst_cap {
            return Ok((at, false));
        }
        let rc = read(at, flags);
        if rc < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            // The cache would have had to wait, or the file system has no
            // such reads; or a read that may wait failed, as the caller's
            // own read of the rest will, and that one reports it.
            return Ok((at, false));
        }
        if rc == 0 && flags == 0 {
            return Ok((at, true));
        }
        at += rc as usize;
        flags = 0;
    }
}

struct SmallSlot {
    file_idx: usize,
    file: File,
    buf: Vec<u8>,
    at: usize,
}

#[inline]
fn ensure_spare_cap(buf: &mut Vec<u8>, needed: usize) {
    buf.clear();
    if buf.capacity() < needed {
        buf.reserve(needed);
    }
}

/// Makes `buf` an empty buffer with room for a file of `hint` bytes, whose
/// first `n` bytes (`start`) were read already, and copies those in. The
/// room is for what was read and more, whatever the hint: the file may
/// have grown since its size was read. (One fallback sized its buffer by
/// the hint alone and copied 16,448 bytes into 4,096.)
fn buffer_with_start(buf: &mut Vec<u8>, hint: usize, start: Option<(*const u8, usize)>) -> usize {
    let n = start.map_or(0, |(_, n)| n);
    ensure_spare_cap(buf, (hint + 1).max(n + 4096));
    // In every build: the copy below is within the buffer.
    assert!(buf.capacity() >= n, "the buffer holds what was read");
    if let Some((src, n)) = start
        && n > 0
    {
        // Sound: `src` is `n` bytes read just now into a slot or buffer
        // that is not `buf` (the callers'), and `buf` has room for them.
        unsafe { core::ptr::copy_nonoverlapping(src, buf.as_mut_ptr(), n) };
    }
    n
}

fn read_rest_sync_uninit(file: &File, buf: &mut Vec<u8>, mut at: usize) -> io::Result<usize> {
    loop {
        if at == buf.capacity() {
            // Sound: every byte below `at` was written by a read (`at` counts
            // them), and `at` is the capacity here; the length goes back to 0
            // once the buffer has grown, as the callers expect.
            unsafe { buf.set_len(at) };
            buf.reserve(at.max(4096));
            unsafe { buf.set_len(0) };
        }
        let avail = buf.capacity() - at;
        // Sound: `avail` bytes from `at` are inside the capacity; the kernel
        // writes at most that many.
        let ptr = unsafe { buf.as_mut_ptr().add(at) };
        let rc = unsafe { libc::pread(file.as_raw_fd(), ptr.cast(), avail, at as libc::off_t) };
        if rc == 0 {
            // Sound: the reads wrote every byte below `at`.
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

struct WarmLargePipe {
    pipe: Pipeline<usize, ()>,
    maps: Vec<(usize, Lent<'static>, memmap2::Mmap)>,
    in_flight: usize,
}

impl WarmLargePipe {
    fn new(mode: blake3::Mode<'static>) -> Self {
        Self { pipe: Pipeline::new(mode, Limits { messages: 16, rooms: 0 }), maps: Vec::with_capacity(16), in_flight: 0 }
    }

    fn drain(&mut self, sink: &mut Sink<'_>) {
        while let Some(ev) = self.pipe.next_event() {
            self.in_flight -= 1;
            if let Event::Hashed { tag: idx, hash, .. } = ev {
                if let Some(pos) = self.maps.iter().position(|x| x.0 == idx) {
                    let (_, guard, map) = self.maps.swap_remove(pos);
                    drop(guard);
                    crate::hold(map);
                }
                sink.put(idx, Ok(hash));
            }
        }
    }

    fn submit_lent(&mut self, idx: usize, size: u64, map: memmap2::Mmap, sink: &mut Sink<'_>) {
        self.drain(sink);
        let m = block_on(self.pipe.message(idx, Some(size)));
        // Sound: the map is kept beside the guard in `maps` until the
        // message's event (drain), where the guard goes first; a guard dropped
        // before its message is hashed stops the process (Lent), and so does a
        // panic. The mapping is of a file: if another program cuts the file
        // short meanwhile, the read faults and the process stops (README,
        // --no-mmap).
        let slice: &'static [u8] = unsafe { core::slice::from_raw_parts(map.as_ptr(), map.len()) };
        let guard = unsafe { self.pipe.lent(m, slice) };
        self.maps.push((idx, guard, map));
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
        partial_src: Option<(*const u8, usize)>,
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
        // Sound: `partial_at` is inside the capacity (buffer_with_start).
        let ptr = unsafe { buf.as_mut_ptr().add(partial_at) };
        let room = buf.capacity() - partial_at;
        self.slots[sid] = Some(SmallSlot { file_idx, file, buf, at: partial_at });
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
            let slot_id = cqe.user_data as usize;
            let mut st = self.slots[slot_id].take().expect("active SmallSlot");
            self.ring_in_flight -= 1;

            if cqe.res < 0 {
                let errno = -cqe.res;
                if errno == libc::EINTR {
                    while self.ring.sq_space_left() == 0 {
                        self.ring.submit_and_wait(0);
                    }
                    let fd = st.file.as_raw_fd();
                    // Sound: `st.at` is below the capacity (the read that
                    // failed had room).
                    let ptr = unsafe { st.buf.as_mut_ptr().add(st.at) };
                    let room = st.buf.capacity() - st.at;
                    let off = st.at as u64;
                    self.slots[slot_id] = Some(st);
                    // Sound: as queue_file's read; the slot goes back into
                    // `slots`.
                    let ok = unsafe { self.ring.push_read(fd, ptr, room, off, 0, slot_id as u64) };
                    assert!(ok, "the ring has room for every read in flight");
                    self.ring_in_flight += 1;
                    continue;
                }
                if st.at == 0 && matches!(errno, libc::EINVAL | libc::EOPNOTSUPP | libc::EBADF) {
                    let res = read_rest_sync_uninit(&st.file, &mut st.buf, 0);
                    drop(st.file);
                    self.free_slot_ids.push(slot_id);
                    match res {
                        Ok(n) => {
                            if n <= WARM_BATCH_MAX_FILE {
                                warm_batch.push_raw(st.file_idx, st.buf.as_ptr(), n, mode, sink);
                            } else {
                                let h = crate::hash_slice_local(mode, &st.buf[..n]);
                                sink.put(st.file_idx, Ok(h));
                            }
                        }
                        Err(e) => {
                            sink.put(st.file_idx, Err(e.into()));
                        }
                    }
                    st.buf.clear();
                    if self.free_bufs.len() < self.max_slots * 2 && st.buf.capacity() <= (256 << 10) {
                        self.free_bufs.push(st.buf);
                    }
                    continue;
                }
                if st.buf.capacity() <= (256 << 10) && self.free_bufs.len() < self.max_slots * 2 {
                    self.free_bufs.push(st.buf);
                }
                self.free_slot_ids.push(slot_id);
                sink.put(st.file_idx, Err(io::Error::from_raw_os_error(errno).into()));
                continue;
            }

            let n = cqe.res as usize;
            st.at += n;
            // The file's end is where a read gives no byte, and nowhere
            // its size said (b3sum/README.md, "What b3sum reads").
            if n == 0 {
                let total_len = st.at;
                drop(st.file);
                self.free_slot_ids.push(slot_id);
                if sink.open {
                    // Sound: the reads wrote every byte below `st.at`.
                    unsafe { st.buf.set_len(total_len) };
                    if total_len <= WARM_BATCH_MAX_FILE {
                        warm_batch.push_raw(st.file_idx, st.buf.as_ptr(), total_len, mode, sink);
                    } else {
                        let h = crate::hash_slice_local(mode, &st.buf);
                        sink.put(st.file_idx, Ok(h));
                    }
                    st.buf.clear();
                }
                if st.buf.capacity() <= (256 << 10) && self.free_bufs.len() < self.max_slots * 2 {
                    self.free_bufs.push(st.buf);
                }
            } else {
                if st.at == st.buf.capacity() {
                    // Sound: as read_rest_sync_uninit's: all `st.at` bytes are
                    // written, kept across the growth.
                    unsafe { st.buf.set_len(st.at) };
                    st.buf.reserve(st.buf.capacity().max(8192));
                    unsafe { st.buf.set_len(0) };
                }
                while self.ring.sq_space_left() == 0 {
                    self.ring.submit_and_wait(0);
                }
                let fd = st.file.as_raw_fd();
                // Sound: `st.at` is below the capacity (grown above if it was
                // full).
                let ptr = unsafe { st.buf.as_mut_ptr().add(st.at) };
                let room = st.buf.capacity() - st.at;
                let off = st.at as u64;
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
pub(crate) fn hash_paths(args: &Args, paths: &[PathBuf], on_ordered: &mut dyn FnMut(usize, anyhow::Result<blake3::Hash>) -> bool) {
    hash_paths_inner(args, paths, false, on_ordered);
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

fn hash_paths_inner(args: &Args, paths: &[PathBuf], force_no_uring: bool, on_ordered: &mut dyn FnMut(usize, anyhow::Result<blake3::Hash>) -> bool) {
    let nfiles = paths.len();
    if nfiles == 0 {
        return;
    }
    let total_threads = crate::threads().get();
    let io_threads = readers(total_threads, nfiles);
    let slots_per_ring = if io_threads <= 4 { SLOTS_PER_RING_SINGLE } else { SLOTS_PER_RING_MULTI };

    grow_descriptor_table(io_threads * (slots_per_ring + 8) + 64);

    if io_threads == 1 {
        let next = AtomicUsize::new(0);
        let mut results: Vec<Option<anyhow::Result<blake3::Hash>>> = (0..nfiles).map(|_| None).collect();
        let mut emitted = 0usize;
        let mut stop = false;
        run_hybrid_worker(args, paths, slots_per_ring, total_threads, 1, force_no_uring, &next, &mut |idx, res| {
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
    let (done_tx, done_rx) = std::sync::mpsc::channel::<(usize, anyhow::Result<blake3::Hash>)>();

    std::thread::scope(|scope| {
        for _ in 1..io_threads {
            if next.load(Ordering::Relaxed) >= nfiles {
                break;
            }
            let (next_ref, tx) = (&next, done_tx.clone());
            scope.spawn(move || {
                run_hybrid_worker(args, paths, slots_per_ring, total_threads, io_threads, force_no_uring, next_ref, &mut |i, res| tx.send((i, res)).is_ok());
            });
        }
        drop(done_tx);

        let mut results: Vec<Option<anyhow::Result<blake3::Hash>>> = (0..nfiles).map(|_| None).collect();
        let mut emitted = 0usize;
        let mut stop = false;
        let mut take = |idx: usize, res: anyhow::Result<blake3::Hash>| -> bool {
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

        run_hybrid_worker(args, paths, slots_per_ring, total_threads, io_threads, force_no_uring, &next, &mut |i, res| {
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
    max_slots: usize,
    total_threads: usize,
    io_threads: usize,
    mut uring_unavailable: bool,
    next: &AtomicUsize,
    done: &mut dyn FnMut(usize, anyhow::Result<blake3::Hash>) -> bool,
) {
    let mut warm_batch = WarmBatch::new();
    let mut medium_buf: Vec<u8> = Vec::new();
    let mut warm_pipe: Option<WarmLargePipe> = None;
    let mut cold: Option<ColdRingState> = None;
    let mut probe_stack: [MaybeUninit<u8>; WARM_SLOT_BLOCKS * 64] = [MaybeUninit::uninit(); WARM_SLOT_BLOCKS * 64];
    let mut sink = Sink { done, open: true };

    while sink.open {
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

        let opened = File::open(&paths[i]).and_then(|f| Ok((f.metadata()?, f)));
        let (meta, file) = match opened {
            Ok(pair) => pair,
            Err(e) => {
                sink.put(i, Err(e.into()));
                continue;
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

        let size = meta.len();

        // Fast path once `cold` ring is already active on this worker: push `< 4 MiB`
        // files straight onto `Uring` without an extra `preadv2(RWF_NOWAIT)` syscall
        // (kernel `io_read` serves cached hits inline and arms async NVMe reads on misses!).
        if let Some(ref mut c) = cold
            && size < COLD_RING_MAX_FILE
        {
            c.queue_file(i, file, size as usize, None, args, &mut warm_batch, &mut sink);
            continue;
        }

        // 1. Small files `<= 16 KiB` before `cold` is active:
        if size <= WARM_BATCH_MAX_FILE as u64 {
            let hint = size as usize;
            let (slot_ptr, slot_cap, in_arena) = if warm_batch.has_arena() {
                let s = warm_batch.len;
                let (p, cap) = warm_batch.slot_ptr_and_cap(s);
                (p, cap, true)
            } else {
                (probe_stack.as_mut_ptr().cast::<u8>(), probe_stack.len(), false)
            };

            match try_pread_cached(file.as_raw_fd(), slot_ptr, slot_cap, 0) {
                Ok((n, true)) => {
                    drop(file);
                    if in_arena {
                        let s = warm_batch.len;
                        warm_batch.entries[s] = (i, n);
                        warm_batch.len += 1;
                        if warm_batch.len == WARM_BATCH_SLOTS {
                            warm_batch.flush(args.mode, &mut sink);
                        }
                    } else {
                        warm_batch.push_raw(i, slot_ptr, n, args.mode, &mut sink);
                    }
                    continue;
                }
                Ok((partial_at, false)) => {
                    if !uring_unavailable && cold.is_none() {
                        cold = ColdRingState::new(max_slots);
                        if cold.is_none() {
                            uring_unavailable = true;
                        }
                    }
                    if let Some(ref mut c) = cold {
                        c.queue_file(i, file, hint, Some((slot_ptr, partial_at)), args, &mut warm_batch, &mut sink);
                    } else {
                        let mut buf = Vec::new();
                        buffer_with_start(&mut buf, hint, Some((slot_ptr, partial_at)));
                        let res = read_rest_sync_uninit(&file, &mut buf, partial_at).map(|n| crate::hash_slice_32(args.mode, &buf[..n])).map_err(Into::into);
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
            let hint = size as usize;
            ensure_spare_cap(&mut medium_buf, hint + 1);
            match try_pread_cached(file.as_raw_fd(), medium_buf.as_mut_ptr(), medium_buf.capacity(), 0) {
                Ok((n, true)) => {
                    drop(file);
                    // Sound: try_pread_cached wrote the first `n` bytes.
                    unsafe { medium_buf.set_len(n) };
                    let h = crate::hash_slice_32(args.mode, &medium_buf);
                    medium_buf.clear();
                    sink.put(i, Ok(h));
                    continue;
                }
                Ok((partial_at, false)) => {
                    if !uring_unavailable && cold.is_none() {
                        cold = ColdRingState::new(max_slots);
                        if cold.is_none() {
                            uring_unavailable = true;
                        }
                    }
                    if let Some(ref mut c) = cold {
                        c.queue_file(i, file, hint, Some((medium_buf.as_ptr(), partial_at)), args, &mut warm_batch, &mut sink);
                    } else {
                        let res = read_rest_sync_uninit(&file, &mut medium_buf, partial_at)
                            .map(|n| crate::hash_slice_32(args.mode, &medium_buf[..n]))
                            .map_err(Into::into);
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
                wp.submit_lent(i, size, map, &mut sink);
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
                    c.queue_file(i, file, size as usize, None, args, &mut warm_batch, &mut sink);
                    continue;
                }
            } else {
                drop(map);
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

        if let Some(ref mut c) = cold {
            let pool_workers = if args.num_threads().is_some() { (total_threads - io_threads + 1).max(1) } else { total_threads.min(16) };
            crate::init_pool_once(pool_workers);
            let cpipe = c.cold_large_pipe.get_or_insert_with(|| Pipeline::<usize, ()>::new(args.mode, Limits { messages: 2, rooms: LARGE_WINDOW }));
            let mut m = block_on(cpipe.message(i, Some(size)));
            match stream_fd_into_rooms(&mut c.ring, &mut c.deferred_cqes, cpipe, &mut m, file.as_raw_fd(), size) {
                Ok(()) => {
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
                    sink.put(i, Err(e.into()));
                }
            }
        } else {
            sink.put(i, crate::hash_open_file(args, file));
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
        let dir = tempfile::tempdir_in(".").unwrap_or_else(|_| tempfile::tempdir().unwrap());
        let p = dir.path().join("grow");
        let content: Vec<u8> = (0..100_000u32).map(|i| (i * 7) as u8).collect();
        std::fs::write(&p, &content).unwrap();
        let f = std::fs::File::open(&p).unwrap();
        let mut buf = Vec::with_capacity(16);
        let n = super::read_rest_sync_uninit(&f, &mut buf, 0).unwrap();
        assert_eq!(n, content.len());
        assert_eq!(&buf[..], &content[..]);

        let mut slice_buf = vec![0u8; 5000];
        let m = super::pread_fill_rest(f.as_raw_fd(), &mut slice_buf, 123).unwrap();
        assert_eq!(m, 5000);
        assert_eq!(&slice_buf[..], &content[123..5123]);

        // Direct unit coverage of `applies_to_paths`, `WarmBatch` (count=1 and count=16), and `hash_single_cold_large_file_1t`.
        let a_two = crate::Args::for_test_plain(vec![p.clone(), p.clone()]);
        assert!(!super::applies_to_paths(&a_two, &[p.clone()]));
        assert!(!super::applies_to_paths(&a_two, &[p.clone(), PathBuf::from("-")]));
        assert!(super::applies_to_paths(&a_two, &[p.clone(), p.clone()]));
        if let Some(res) = super::hash_single_cold_large_file_1t(&a_two, &f, content.len() as u64) {
            assert_eq!(res.unwrap(), blake3::convenience::hash(&content));
        }

        let mut wb = super::WarmBatch::new();
        assert!(!wb.has_arena());
        let mut wb_out = Vec::new();
        {
            let mut collect = |i, r: anyhow::Result<blake3::Hash>| {
                wb_out.push((i, r.unwrap()));
                true
            };
            let mut sink = super::Sink { done: &mut collect, open: true };
            wb.push_raw(0, b"abc".as_ptr(), 3, blake3::Mode::Plain, &mut sink);
            assert!(wb.has_arena());
            wb.flush(blake3::Mode::Plain, &mut sink);
        }
        assert_eq!(wb_out, vec![(0, blake3::convenience::hash(b"abc"))]);

        wb_out.clear();
        {
            let mut collect = |i, r: anyhow::Result<blake3::Hash>| {
                wb_out.push((i, r.unwrap()));
                true
            };
            let mut sink = super::Sink { done: &mut collect, open: true };
            for k in 0..super::WARM_BATCH_SLOTS {
                let msg = vec![(k + 1) as u8; k * 13 + 1];
                wb.push_raw(k, msg.as_ptr(), msg.len(), blake3::Mode::Plain, &mut sink);
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
            let mut collect = |i, r: anyhow::Result<blake3::Hash>| {
                wb_out.push((i, r.unwrap()));
                true
            };
            let mut sink = super::Sink { done: &mut collect, open: true };
            let messages: Vec<Vec<u8>> = lens.iter().map(|&len| (0..len).map(|i| (i * 5 + len) as u8).collect()).collect();
            for (k, msg) in messages.iter().enumerate() {
                wb.push_raw(k, msg.as_ptr(), msg.len(), blake3::Mode::Plain, &mut sink);
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
        super::hash_paths_inner(&args, &paths, true, &mut |idx, res| {
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

        let dir = tempfile::tempdir_in(".").unwrap_or_else(|_| tempfile::tempdir().unwrap());
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
            super::stream_fd_into_rooms(&mut ring, &mut deferred, &mut pipe, &mut m, rf.as_raw_fd(), (len / 3) as u64).unwrap();
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
        let err = super::stream_fd_into_rooms(&mut ring, &mut deferred, &mut pipe, &mut m, dir_file.as_raw_fd(), 1_000_000).unwrap_err();
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
        super::hash_paths_inner(&args, paths, false, &mut |index, result| {
            assert_eq!(index, got.len(), "results arrive in order");
            got.push(result.unwrap());
            true
        });
        got
    }

    #[test]
    fn forced_misses_take_the_cold_paths_for_every_size_class() {
        blake3::initialize();
        let dir = tempfile::tempdir_in(".").unwrap_or_else(|_| tempfile::tempdir().unwrap());
        // 34 files: the wide form (32 files and more). Then 5: the narrow one.
        let (paths, want) = files_of_every_size(dir.path(), 34);
        assert_eq!(hashes_in_order(&paths), want, "from the page cache");
        super::FORCE_MISS.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(hashes_in_order(&paths), want, "every file taken for a miss");
        assert_eq!(hashes_in_order(&paths[12..17]), want[12..17], "five files of 1 MiB and more");

        // A listener that stops: it hears nothing after the result it refused.
        let args = crate::Args::for_test_plain(paths.clone());
        let mut heard = Vec::new();
        super::hash_paths_inner(&args, &paths, false, &mut |index, result| {
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
        super::hash_paths_inner(&args, &with_missing, false, &mut |index, result| {
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
        let dir = tempfile::tempdir_in(".").unwrap_or_else(|_| tempfile::tempdir().unwrap());
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
        let mut collect = |_, _: anyhow::Result<blake3::Hash>| true;
        let mut sink = super::Sink { done: &mut collect, open: true };
        let msg = vec![0u8; super::WARM_SLOT_BLOCKS * 64 + 1];
        wb.push_raw(0, msg.as_ptr(), msg.len(), blake3::Mode::Plain, &mut sink);
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
        let dir = tempfile::tempdir_in(std::env::current_exe().unwrap().parent().unwrap()).unwrap();
        let path = dir.path().join("f");
        let content: Vec<u8> = (0..100_000u32).map(|i| (i * 7) as u8).collect();
        std::fs::write(&path, &content).unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let mut byte = 0u8;
        if super::try_pread_cached(std::os::fd::AsRawFd::as_raw_fd(&file), &mut byte, 1, 0).unwrap().0 == 0 {
            eprintln!("no reads from the page cache alone in {}", dir.path().display());
            return;
        }
        for room in [1usize, 4095, 4096, 99_999, 100_000, 100_001, 300_000] {
            let mut buffer = vec![0u8; room];
            let (n, end) = super::try_pread_cached(std::os::fd::AsRawFd::as_raw_fd(&file), buffer.as_mut_ptr(), room, 0).unwrap();
            assert_eq!((n, end), (room.min(content.len()), room > content.len()), "room={room}");
            assert_eq!(buffer[..n], content[..n]);
            // From where a first read stopped, as a file that grew is read on.
            let mut rest = vec![0u8; content.len() + 1];
            let (m, end) = super::try_pread_cached(std::os::fd::AsRawFd::as_raw_fd(&file), rest.as_mut_ptr(), rest.len(), n).unwrap();
            assert_eq!((m, end), (content.len(), true), "room={room}");
            assert_eq!(rest[n..m], content[n..]);
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
    /// are more than the size said, and the buffer holds them all.
    #[test]
    fn a_buffer_holds_what_was_read_whatever_the_size_said() {
        let read: Vec<u8> = (0..super::WARM_SLOT_BLOCKS * 64).map(|i| i as u8).collect();
        let mut buf = Vec::new();
        let n = super::buffer_with_start(&mut buf, 10, Some((read.as_ptr(), read.len())));
        assert_eq!(n, read.len());
        assert!(buf.capacity() >= n + 4096 && buf.is_empty());
        unsafe { buf.set_len(n) };
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
        let dir = tempfile::tempdir_in(std::env::current_exe().unwrap().parent().unwrap()).unwrap();
        let path = dir.path().join("cached");
        std::fs::write(&path, [7u8; 5000]).unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let mut buffer = vec![0u8; 5001];
        super::ASKED.with(|asked| asked.borrow_mut().clear());
        let read = super::try_pread_cached(file.as_raw_fd(), buffer.as_mut_ptr(), buffer.len(), 0).unwrap();
        let asked = super::ASKED.with(|asked| asked.borrow().clone());
        if read == (0, false) {
            // A file system without reads of the cache alone: nothing to tell.
            assert_eq!(asked, [libc::RWF_NOWAIT]);
            return;
        }
        assert_eq!(read, (5000, true));
        assert_eq!(asked, [libc::RWF_NOWAIT, 0]);
        assert_eq!(&buffer[..5000], &[7u8; 5000]);
    }

    /// A read of the cache alone that gives no byte does not end a
    /// file: the kernels that give none where they would have to wait
    /// (Linux 5.9 and 5.10) are played here by `read`, whose answers
    /// are listed with what was asked. The end is where a read that may
    /// wait gives no byte.
    #[test]
    fn a_read_of_the_cache_that_gives_no_byte_does_not_end_the_file() {
        let content = b"0123456789";
        let wait = |would_wait: bool| -> isize {
            // Sound: this thread's errno.
            unsafe { *libc::__errno_location() = if would_wait { libc::EAGAIN } else { libc::EIO } };
            -1
        };
        let run = |room: usize, from: usize, cache: &dyn Fn(usize) -> isize, plain: &dyn Fn(usize) -> isize| {
            let mut asked = Vec::new();
            let got = super::cached_then_plain(room, from, |at, flags| {
                asked.push((at, flags != 0));
                if flags != 0 {
                    assert_eq!(flags, libc::RWF_NOWAIT);
                    cache(at)
                } else {
                    plain(at)
                }
            })
            .unwrap();
            (got, asked)
        };
        // A plain read: up to four bytes of the file at a time.
        let plain = |at: usize| (content.len() - at).min(4) as isize;
        // The cache gives no byte of a file that has ten.
        assert_eq!(run(64, 0, &|_| 0, &plain), ((10, true), vec![(0, true), (0, false), (4, false), (8, false), (10, false)]));
        // It gives six: the rest and the end come from reads that may wait.
        assert_eq!(run(64, 0, &|at| (6 - at) as isize, &plain), ((10, true), vec![(0, true), (6, false), (10, false)]));
        // It gives all ten: one read more, which may wait, says the end.
        assert_eq!(run(64, 0, &|at| (10 - at) as isize, &plain), ((10, true), vec![(0, true), (10, false)]));
        // A file of no bytes ends where a read that may wait says so.
        assert_eq!(run(64, 0, &|_| 0, &|_| 0), ((0, true), vec![(0, true), (0, false)]));
        // The cache would have to wait: the caller reads on, from the start.
        assert_eq!(run(64, 0, &|_| wait(true), &plain), ((0, false), vec![(0, true)]));
        // A full buffer is not the end.
        assert_eq!(run(10, 0, &|at| (10 - at) as isize, &plain), ((10, false), vec![(0, true)]));
        assert_eq!(run(8, 0, &|_| 0, &plain), ((8, false), vec![(0, true), (0, false), (4, false)]));
        // A read that may wait fails: the caller reads on from there.
        assert_eq!(run(64, 0, &|at| (6 - at) as isize, &|_| wait(false)), ((6, false), vec![(0, true), (6, false)]));
        // From where an earlier read stopped.
        assert_eq!(run(64, 6, &|_| 0, &plain), ((10, true), vec![(6, true), (6, false), (10, false)]));
    }

    /// Reads that come back short and reads that are interrupted come to
    /// the same bytes: with every other completion of a ring cut to half
    /// its bytes (1) or made an interruption (2), files of every size
    /// class, all taken for misses, hash as they do untouched. (Where no
    /// ring can be made the reads are of another kind, and this says
    /// nothing.)
    fn spoiled_ring_reads_come_to_the_same_hashes() {
        let dir = tempfile::tempdir_in(".").unwrap_or_else(|_| tempfile::tempdir().unwrap());
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
        let dir = tempfile::tempdir().unwrap();
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
        super::hash_paths_inner(&args, &paths, true, &mut |i, hash| {
            got.push((i, hash.unwrap()));
            true
        });
        super::set_force_miss(false);
        *super::AFTER_OPEN.lock().unwrap() = None;
        got.sort_by_key(|(i, _)| *i);
        let want = blake3::convenience::hash(&first);
        assert_eq!(got, vec![(0, want), (1, want)]);
    }
}
