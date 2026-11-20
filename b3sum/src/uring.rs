//! Minimal Linux `io_uring` wrapper over `libc` (`SYS_io_uring_setup`,
//! `SYS_io_uring_enter`, and the ring mappings). Zero external dependencies.

use core::cell::Cell;
use std::io;
use std::os::fd::RawFd;
use std::ptr;
use std::sync::atomic::{AtomicU32, Ordering};

const IORING_OP_READ: u8 = 22;

const IORING_OFF_SQ_RING: i64 = 0;
const IORING_OFF_CQ_RING: i64 = 0x800_0000;
const IORING_OFF_SQES: i64 = 0x1000_0000;

const IORING_FEAT_SINGLE_MMAP: u32 = 1 << 0;
const IORING_ENTER_GETEVENTS: u32 = 1 << 0;

const IORING_SETUP_CQSIZE: u32 = 1 << 3;
const IORING_SETUP_COOP_TASKRUN: u32 = 1 << 8;
const IORING_SETUP_SINGLE_ISSUER: u32 = 1 << 12;
const IORING_SETUP_DEFER_TASKRUN: u32 = 1 << 13;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct IoSqrOffsets {
    head: u32,
    tail: u32,
    ring_mask: u32,
    ring_entries: u32,
    flags: u32,
    dropped: u32,
    array: u32,
    resv1: u32,
    user_addr: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct IoCqrOffsets {
    head: u32,
    tail: u32,
    ring_mask: u32,
    ring_entries: u32,
    overflow: u32,
    cqes: u32,
    flags: u32,
    resv1: u32,
    user_addr: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct IoUringParams {
    sq_entries: u32,
    cq_entries: u32,
    flags: u32,
    sq_thread_cpu: u32,
    sq_thread_idle: u32,
    features: u32,
    wq_fd: u32,
    resv: [u32; 3],
    sq_off: IoSqrOffsets,
    cq_off: IoCqrOffsets,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct IoUringSqe {
    opcode: u8,
    flags: u8,
    ioprio: u16,
    fd: i32,
    off: u64,
    addr: u64,
    len: u32,
    rw_flags: u32,
    user_data: u64,
    buf_group: u16,
    personality: u16,
    splice_fd_in: i32,
    pad2: [u64; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct IoUringCqe {
    pub user_data: u64,
    pub res: i32,
    pub flags: u32,
}

pub(crate) struct Uring {
    fd: RawFd,
    ring_flags: u32,
    sq_ring_ptr: *mut libc::c_void,
    sq_ring_len: usize,
    cq_ring_ptr: *mut libc::c_void,
    cq_ring_len: usize,
    single_mmap: bool,
    sqes_ptr: *mut IoUringSqe,
    sqes_len: usize,

    sq_head: *const AtomicU32,
    sq_tail: *const AtomicU32,
    sq_mask: u32,
    sq_entries: u32,
    sq_array: *mut u32,
    local_sq_tail: u32,
    unsubmitted: u32,

    cq_head: *const AtomicU32,
    cq_tail: *const AtomicU32,
    cq_mask: u32,
    cqes: *const IoUringCqe,
    /// Reads queued and not yet reaped: the kernel may still write their
    /// buffers. The ring is not dropped or set aside while there are any.
    in_flight: u32,
}

// Sound: the pointers are into this ring's own mappings, which go where
// the Uring goes and are unmapped by its drop alone.
unsafe impl Send for Uring {}

thread_local! {
    /// Per-thread cached `Uring`: `IORING_SETUP_SINGLE_ISSUER` binds each ring
    /// to the `tid` that created it, so reuse stays strictly thread-local.
    static LOCAL_URING: Cell<Option<Uring>> = const { Cell::new(None) };
}

impl Uring {
    /// Acquire this thread's cached `Uring` with at least `entries` SQEs, or create one.
    pub(crate) fn acquire(entries: u32) -> io::Result<Self> {
        if let Some(ring) = LOCAL_URING.with(|c| c.take()) {
            if ring.sq_entries >= entries {
                return Ok(ring);
            }
            drop(ring);
        }
        Self::new(entries)
    }

    /// Cache an idle `Uring` on the thread that created it so subsequent
    /// `--check` batches on this thread reuse it without `io_uring_setup`.
    pub(crate) fn release(self) {
        assert_eq!(self.in_flight, 0, "a ring is set aside with no read in flight");
        LOCAL_URING.with(|c| {
            c.set(Some(self));
        });
    }

    pub(crate) fn new(entries: u32) -> io::Result<Self> {
        #[cfg(test)]
        if REFUSED.load(Ordering::Relaxed) {
            return Err(io::Error::from_raw_os_error(libc::ENOSYS));
        }
        let try_flags = [
            IORING_SETUP_CQSIZE | IORING_SETUP_COOP_TASKRUN | IORING_SETUP_SINGLE_ISSUER | IORING_SETUP_DEFER_TASKRUN,
            IORING_SETUP_CQSIZE | IORING_SETUP_COOP_TASKRUN,
            0,
        ];
        let mut params = IoUringParams::default();
        let mut ring_fd: RawFd = -1;
        let mut chosen_flags = 0u32;
        let mut last_err = io::Error::from_raw_os_error(libc::ENOSYS);

        for &flags in &try_flags {
            params = IoUringParams { cq_entries: if flags & IORING_SETUP_CQSIZE != 0 { entries * 2 } else { 0 }, flags, ..IoUringParams::default() };
            // Sound: kernel initializes `params` and returns a new cloexec fd or -1.
            let rc = unsafe { libc::syscall(libc::SYS_io_uring_setup, entries, &mut params as *mut IoUringParams) };
            if rc >= 0 {
                ring_fd = rc as RawFd;
                chosen_flags = flags;
                break;
            }
            last_err = io::Error::last_os_error();
        }
        if ring_fd < 0 {
            return Err(last_err);
        }

        let sq_size = params.sq_off.array as usize + (params.sq_entries as usize) * size_of::<u32>();
        let cq_size = params.cq_off.cqes as usize + (params.cq_entries as usize) * size_of::<IoUringCqe>();
        let single_mmap = (params.features & IORING_FEAT_SINGLE_MMAP) != 0;
        let sq_ring_len = if single_mmap { sq_size.max(cq_size) } else { sq_size };

        // Sound: mapping the ring descriptor at kernel-defined offsets.
        let sq_ring_ptr =
            // Sound: a new mapping of the ring's descriptor at the kernel's
            // offset for it; the kernel chooses the address.
            unsafe { libc::mmap(ptr::null_mut(), sq_ring_len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, ring_fd, IORING_OFF_SQ_RING) };
        if sq_ring_ptr == libc::MAP_FAILED {
            let err = io::Error::last_os_error();
            // Sound: the descriptor is this function's, closed once on each
            // way out.
            unsafe { libc::close(ring_fd) };
            return Err(err);
        }

        let (cq_ring_ptr, cq_ring_len) = if single_mmap {
            (sq_ring_ptr, 0)
        } else {
            // Sound: as the first mapping.
            let p = unsafe { libc::mmap(ptr::null_mut(), cq_size, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, ring_fd, IORING_OFF_CQ_RING) };
            if p == libc::MAP_FAILED {
                let err = io::Error::last_os_error();
                // Sound: this function's own mapping and descriptor, released
                // once.
                unsafe {
                    libc::munmap(sq_ring_ptr, sq_ring_len);
                    libc::close(ring_fd);
                }
                return Err(err);
            }
            (p, cq_size)
        };

        let sqes_len = (params.sq_entries as usize) * size_of::<IoUringSqe>();
        // Sound: as the first mapping.
        let sqes_raw = unsafe { libc::mmap(ptr::null_mut(), sqes_len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, ring_fd, IORING_OFF_SQES) };
        if sqes_raw == libc::MAP_FAILED {
            let err = io::Error::last_os_error();
            // Sound: this function's own mappings and descriptor, released
            // once.
            unsafe {
                if !single_mmap {
                    libc::munmap(cq_ring_ptr, cq_ring_len);
                }
                libc::munmap(sq_ring_ptr, sq_ring_len);
                libc::close(ring_fd);
            }
            return Err(err);
        }

        let sq_base = sq_ring_ptr.cast::<u8>();
        let cq_base = cq_ring_ptr.cast::<u8>();
        let sqes_ptr = sqes_raw.cast::<IoUringSqe>();

        // Sound: each offset is one the kernel gave for this ring (`params`),
        // inside the mapping of the length it also gave; the heads and tails
        // are u32s the kernel reads and writes atomically, as AtomicU32 does.
        let sq_head = unsafe { sq_base.add(params.sq_off.head as usize).cast::<AtomicU32>() };
        let sq_tail = unsafe { sq_base.add(params.sq_off.tail as usize).cast::<AtomicU32>() };
        let sq_mask = unsafe { *sq_base.add(params.sq_off.ring_mask as usize).cast::<u32>() };
        let sq_array = unsafe { sq_base.add(params.sq_off.array as usize).cast::<u32>() };
        for i in 0..params.sq_entries {
            // Sound: the array has `sq_entries` slots.
            unsafe { *sq_array.add(i as usize) = i };
        }

        // Sound: as the submission ring's, above.
        let cq_head = unsafe { cq_base.add(params.cq_off.head as usize).cast::<AtomicU32>() };
        let cq_tail = unsafe { cq_base.add(params.cq_off.tail as usize).cast::<AtomicU32>() };
        let cq_mask = unsafe { *cq_base.add(params.cq_off.ring_mask as usize).cast::<u32>() };
        let cqes = unsafe { cq_base.add(params.cq_off.cqes as usize).cast::<IoUringCqe>() };
        let local_sq_tail = unsafe { (*sq_tail).load(Ordering::Relaxed) };

        Ok(Self {
            fd: ring_fd,
            ring_flags: chosen_flags,
            sq_ring_ptr,
            sq_ring_len,
            cq_ring_ptr,
            cq_ring_len,
            single_mmap,
            sqes_ptr,
            sqes_len,
            sq_head,
            sq_tail,
            sq_mask,
            sq_entries: params.sq_entries,
            sq_array,
            local_sq_tail,
            unsubmitted: 0,
            cq_head,
            cq_tail,
            cq_mask,
            cqes,
            in_flight: 0,
        })
    }

    #[inline]
    pub(crate) fn sq_space_left(&self) -> u32 {
        // Sound: `sq_head` points into the ring's mapping, which lives as long
        // as the Uring.
        let head = unsafe { (*self.sq_head).load(Ordering::Acquire) };
        self.sq_entries - (self.local_sq_tail.wrapping_sub(head))
    }

    #[inline]
    pub(crate) fn unsubmitted(&self) -> u32 {
        self.unsubmitted
    }

    /// Queue an `IORING_OP_READ` SQE for a buffer with `room` bytes left:
    /// it asks for all of them, up to [`READ_MAX`] ([`read_len`]). Caller
    /// must ensure `buf[..room]` stays valid and exclusively writable by
    /// the kernel until the matching CQE is reaped.
    #[inline]
    pub(crate) unsafe fn push_read(&mut self, fd: RawFd, buf: *mut u8, room: usize, offset: u64, rw_flags: u32, user_data: u64) -> bool {
        if self.sq_space_left() == 0 {
            return false;
        }
        let len = read_len(room);
        let idx = (self.local_sq_tail & self.sq_mask) as usize;
        // Sound: `idx` is below `sq_entries` (masked), so both writes are
        // inside the ring's array and its SQEs; the kernel reads an SQE only
        // after the tail moves past it (submit_and_wait).
        unsafe {
            *self.sq_array.add(idx) = idx as u32;
            *self.sqes_ptr.add(idx) = IoUringSqe {
                opcode: IORING_OP_READ,
                flags: 0,
                ioprio: 0,
                fd,
                off: offset,
                addr: buf as usize as u64,
                len,
                rw_flags,
                user_data,
                buf_group: 0,
                personality: 0,
                splice_fd_in: 0,
                pad2: [0; 2],
            };
        }
        self.local_sq_tail = self.local_sq_tail.wrapping_add(1);
        self.unsubmitted += 1;
        self.in_flight += 1;
        true
    }

    /// Submit all queued SQEs and wait for at least `min_complete` CQEs.
    /// A failed enter stops the process: some reads may already have been
    /// submitted, so unwinding could free buffers the kernel still writes.
    /// A read's error CQE, in contrast, has finished using that read's buffer.
    pub(crate) fn submit_and_wait(&mut self, min_complete: u32) {
        if self.unsubmitted > 0 {
            // Sound: into the ring's mapping, as sq_space_left's.
            unsafe { (*self.sq_tail).store(self.local_sq_tail, Ordering::Release) };
        }
        let mut to_submit = self.unsubmitted;
        self.unsubmitted = 0;

        let need_getevents = min_complete > 0 || (self.ring_flags & IORING_SETUP_DEFER_TASKRUN) != 0;
        if to_submit == 0 && min_complete == 0 && !need_getevents {
            return;
        }
        let enter_flags = if need_getevents { IORING_ENTER_GETEVENTS } else { 0 };

        loop {
            if let Some(submitted) = enter(self.fd, to_submit, min_complete, enter_flags) {
                to_submit = to_submit.saturating_sub(submitted);
                if to_submit == 0 {
                    return;
                }
            } else {
                // Interrupted: the kernel may have consumed some SQEs.
                // Sound: as above.
                let head = unsafe { (*self.sq_head).load(Ordering::Acquire) };
                to_submit = self.local_sq_tail.wrapping_sub(head);
                if min_complete == 0 && to_submit == 0 {
                    return;
                }
            }
        }
    }

    #[inline]
    pub(crate) fn pop_cqe(&mut self) -> Option<IoUringCqe> {
        // Sound: into the ring's mapping. Only this thread moves the head.
        let head = unsafe { (*self.cq_head).load(Ordering::Relaxed) };
        let tail = unsafe { (*self.cq_tail).load(Ordering::Acquire) };
        if head == tail {
            return None;
        }
        let idx = (head & self.cq_mask) as usize;
        // Sound: `idx` is masked to the CQEs' count, and the kernel has
        // finished writing the entry before it moved the tail past it.
        let cqe = unsafe { *self.cqes.add(idx) };
        unsafe { (*self.cq_head).store(head.wrapping_add(1), Ordering::Release) };
        self.in_flight -= 1;
        #[cfg(test)]
        let cqe = spoiled(cqe);
        Some(cqe)
    }
}

/// The most one read is asked for: less than the 4 GiB that a request's
/// 32-bit length can say.
pub(crate) const READ_MAX: usize = 1 << 30;

/// The length a read is asked for, where its buffer has `room` bytes
/// left: all of them, up to [`READ_MAX`]. A buffer of gigabytes (a file
/// that reported a small size and has more, or that grew) can have 4 GiB
/// of room and more, and a request's length is 32 bits: the room cast to
/// them asked for its low bits alone, so for exactly 4 GiB of room it
/// asked for no byte, the read gave none, and no byte is how a file's end
/// is known. The hash printed was then of the file's first 4 GiB. So the
/// room comes here as it is, and no caller narrows it. A request is for
/// at least one byte, in every build: nothing else may end a file.
fn read_len(room: usize) -> u32 {
    assert!(room > 0, "a read is asked for at least one byte");
    room.min(READ_MAX) as u32
}

/// For the tests: no ring can be made, as where a sandbox refuses
/// io_uring.
#[cfg(test)]
pub(crate) static REFUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// For the tests: what becomes of the completions [`Uring::pop_cqe`]
/// hands out. 1: every other read that gave more than one byte gave half
/// as many (a short read; the bytes past them are in the buffer and are
/// not to be used). 2: every other read was interrupted. The reads'
/// callers must come to the same bytes either way. [`SPOILED`] counts the
/// completions so changed.
#[cfg(test)]
pub(crate) static SPOIL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
pub(crate) static SPOILED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
fn spoiled(mut cqe: IoUringCqe) -> IoUringCqe {
    static SEEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let this_one = || SEEN.fetch_add(1, Ordering::Relaxed) % 2 == 0;
    let before = cqe.res;
    match SPOIL.load(Ordering::Relaxed) {
        1 if cqe.res > 1 && this_one() => cqe.res /= 2,
        2 if cqe.res >= 0 && this_one() => cqe.res = -libc::EINTR,
        _ => {}
    }
    if cqe.res != before {
        SPOILED.fetch_add(1, Ordering::Relaxed);
    }
    cqe
}

/// None means interrupted; every other error is fatal. Keep this decision
/// at the syscall, before any caller can release an in-flight buffer.
#[inline]
fn enter(fd: RawFd, to_submit: u32, min_complete: u32, flags: u32) -> Option<u32> {
    // Sound: a system call with plain values; no signal mask is passed.
    let rc = unsafe { libc::syscall(libc::SYS_io_uring_enter, fd, to_submit, min_complete, flags, ptr::null::<libc::sigset_t>(), size_of::<libc::sigset_t>()) };
    if rc >= 0 {
        return Some(rc as u32);
    }
    let err = io::Error::last_os_error();
    if err.kind() == io::ErrorKind::Interrupted {
        return None;
    }
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "b3sum: io_uring_enter failed: {err}; stopping before pending reads' buffers can be released");
    std::process::abort()
}

impl Drop for Uring {
    fn drop(&mut self) {
        // Closing a ring does not wait for its reads: the kernel may write
        // a queued read's buffer after this returns, and whoever drops the
        // ring frees those buffers next. So a ring dropped with a read in
        // flight (a return or a panic past it) stops the process here.
        if self.in_flight != 0 {
            use std::io::Write;
            let _ = writeln!(
                std::io::stderr(),
                "b3sum: an io_uring was dropped with {} reads in flight; stopping before their buffers can be released",
                self.in_flight
            );
            std::process::abort()
        }
        // Sound: the three mappings and the descriptor are this ring's own
        // (new), each released once, here.
        unsafe {
            libc::munmap(self.sqes_ptr.cast(), self.sqes_len);
            if !self.single_mmap {
                libc::munmap(self.cq_ring_ptr, self.cq_ring_len);
            }
            libc::munmap(self.sq_ring_ptr, self.sq_ring_len);
            libc::close(self.fd);
        }
    }
}

#[cfg(test)]
mod test {
    use super::Uring;
    use std::os::fd::AsRawFd;

    /// A ring's read is asked for at least one byte and at most what its
    /// 32-bit length can say, whatever room the buffer has: at 4 GiB of
    /// room and at its multiples the cast length was 0, which reads like
    /// the file's end.
    #[test]
    fn a_rings_read_asks_for_bytes_whatever_the_room() {
        for spare in [1, 4096, super::READ_MAX - 1, super::READ_MAX, super::READ_MAX + 1, (1 << 32) - 1, 1 << 32, (1 << 32) + 1, 1 << 33, 3 << 32, usize::MAX] {
            let len = super::read_len(spare) as usize;
            assert!(0 < len && len <= spare && len <= super::READ_MAX, "{spare}: {len}");
            assert_eq!(len, spare.min(super::READ_MAX), "{spare}");
        }
    }

    /// Through a ring: a read whose buffer has 4 GiB of room gets the
    /// file's bytes. (The room is address space alone, and the file has
    /// 100 bytes; where the system will not lend 4 GiB of addresses, this
    /// says nothing.)
    #[test]
    fn a_read_with_four_gib_of_room_gets_the_files_bytes() {
        let Ok(mut ring) = Uring::new(8) else { return };
        let mut buffer: Vec<u8> = Vec::new();
        if buffer.try_reserve_exact(1 << 32).is_err() {
            eprintln!("no 4 GiB of address space to try this with");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        std::fs::write(&path, [7u8; 100]).unwrap();
        let file = std::fs::File::open(&path).unwrap();
        // Sound: the buffer has the 4 GiB of room said, and is not touched
        // until the read's completion is taken, below.
        assert!(unsafe { ring.push_read(file.as_raw_fd(), buffer.as_mut_ptr(), 1 << 32, 0, 0, 5) });
        ring.submit_and_wait(1);
        let cqe = ring.pop_cqe().unwrap();
        assert_eq!((cqe.user_data, cqe.res), (5, 100));
    }

    /// A ring dropped while a read is in flight stops the process: the
    /// read here is of a pipe no one writes, so it stays in flight.
    #[test]
    fn a_ring_dropped_with_a_read_in_flight_stops_the_process() {
        use std::os::unix::process::ExitStatusExt;
        const CHILD: &str = "B3SUM_TEST_RING_DROPPED";
        if std::env::var_os(CHILD).is_some() {
            let Ok(mut ring) = Uring::new(8) else { std::process::exit(77) };
            let mut ends = [0i32; 2];
            assert_eq!(unsafe { libc::pipe(ends.as_mut_ptr()) }, 0);
            let mut buffer = [0u8; 64];
            assert!(unsafe { ring.push_read(ends[0], buffer.as_mut_ptr(), 64, 0, 0, 1) });
            ring.submit_and_wait(0);
            drop(ring);
            std::process::exit(0);
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "uring::test::a_ring_dropped_with_a_read_in_flight_stops_the_process", "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        if output.status.code() == Some(77) {
            eprintln!("no io_uring here");
            return;
        }
        assert_eq!(output.status.signal(), Some(libc::SIGABRT), "{:?}: {:?}", output.status, output.stderr);
        assert!(String::from_utf8_lossy(&output.stderr).contains("dropped with 1 reads in flight"));
    }

    /// A real failed syscall must abort, even inside catch_unwind. This
    /// needs no io_uring permission: an invalid fd (or a denied syscall)
    /// deterministically takes the production error path.
    #[test]
    fn failed_enter_stops_before_unwinding() {
        use std::os::unix::process::ExitStatusExt;
        const CHILD: &str = "B3SUM_TEST_FAILED_ENTER";
        if std::env::var_os(CHILD).is_some() {
            let _ = std::panic::catch_unwind(|| super::enter(-1, 1, 0, 0));
            std::process::exit(0);
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "uring::test::failed_enter_stops_before_unwinding", "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert_eq!(output.status.signal(), Some(libc::SIGABRT), "{:?}: {:?}", output.status, output.stderr);
        assert!(String::from_utf8_lossy(&output.stderr).contains("stopping before pending reads' buffers can be released"));
    }

    #[test]
    fn uring_reads_offsets_saturation_and_error_cqes() {
        blake3::initialize();
        let Ok(mut ring) = Uring::new(8) else { return };
        assert!(ring.pop_cqe().is_none());
        assert_eq!(ring.unsubmitted(), 0);
        let cap = ring.sq_space_left();
        assert!(cap >= 8);

        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f0");
        let data: Vec<u8> = (0..10_000usize).map(|k| (k * 13 + 7) as u8).collect();
        std::fs::write(&p, &data).unwrap();
        let f = std::fs::File::open(&p).unwrap();

        // 1. Non-zero offset read + saturation up to `cap` (`push_read` returns false when full!)
        let mut bufs: Vec<Vec<u8>> = (0..cap as usize).map(|_| vec![0u8; 256]).collect();
        for (i, b) in bufs.iter_mut().enumerate() {
            let off = (i * 111) as u64;
            assert!(unsafe { ring.push_read(f.as_raw_fd(), b.as_mut_ptr(), 256, off, 0, i as u64) });
            assert_eq!(ring.unsubmitted(), (i + 1) as u32);
        }
        assert_eq!(ring.sq_space_left(), 0);
        let mut extra = [0u8; 16];
        assert!(!unsafe { ring.push_read(f.as_raw_fd(), extra.as_mut_ptr(), 16, 0, 0, 999) }, "ring must report full");

        ring.submit_and_wait(cap);
        assert_eq!(ring.unsubmitted(), 0);
        assert_eq!(ring.sq_space_left(), cap);

        let mut seen = vec![false; cap as usize];
        while let Some(cqe) = ring.pop_cqe() {
            let i = cqe.user_data as usize;
            assert_eq!(cqe.res, 256);
            let off = i * 111;
            assert_eq!(&bufs[i][..], &data[off..off + 256]);
            seen[i] = true;
        }
        assert!(seen.into_iter().all(|x| x));
        assert!(ring.pop_cqe().is_none());

        // 2. Fault injection: invalid fd `-1` produces `-EBADF` CQE without corrupting ring state,
        // and `acquire` / `release` recycles the idle ring cleanly.
        assert!(unsafe { ring.push_read(-1, extra.as_mut_ptr(), 16, 0, 0, 0xDEAD_BEEF) });
        ring.submit_and_wait(1);
        let err_cqe = ring.pop_cqe().expect("error CQE");
        assert_eq!(err_cqe.user_data, 0xDEAD_BEEF);
        assert_eq!(err_cqe.res, -libc::EBADF);
        ring.submit_and_wait(0);

        let saved_fd = ring.fd;
        ring.release();
        let reused = Uring::acquire(8).unwrap();
        assert_eq!(reused.fd, saved_fd);
    }
}
