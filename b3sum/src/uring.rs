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
}

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
        LOCAL_URING.with(|c| {
            c.set(Some(self));
        });
    }

    pub(crate) fn new(entries: u32) -> io::Result<Self> {
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
            unsafe { libc::mmap(ptr::null_mut(), sq_ring_len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, ring_fd, IORING_OFF_SQ_RING) };
        if sq_ring_ptr == libc::MAP_FAILED {
            let err = io::Error::last_os_error();
            unsafe { libc::close(ring_fd) };
            return Err(err);
        }

        let (cq_ring_ptr, cq_ring_len) = if single_mmap {
            (sq_ring_ptr, 0)
        } else {
            let p = unsafe { libc::mmap(ptr::null_mut(), cq_size, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, ring_fd, IORING_OFF_CQ_RING) };
            if p == libc::MAP_FAILED {
                let err = io::Error::last_os_error();
                unsafe {
                    libc::munmap(sq_ring_ptr, sq_ring_len);
                    libc::close(ring_fd);
                }
                return Err(err);
            }
            (p, cq_size)
        };

        let sqes_len = (params.sq_entries as usize) * size_of::<IoUringSqe>();
        let sqes_raw = unsafe { libc::mmap(ptr::null_mut(), sqes_len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, ring_fd, IORING_OFF_SQES) };
        if sqes_raw == libc::MAP_FAILED {
            let err = io::Error::last_os_error();
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

        let sq_head = unsafe { sq_base.add(params.sq_off.head as usize).cast::<AtomicU32>() };
        let sq_tail = unsafe { sq_base.add(params.sq_off.tail as usize).cast::<AtomicU32>() };
        let sq_mask = unsafe { *sq_base.add(params.sq_off.ring_mask as usize).cast::<u32>() };
        let sq_array = unsafe { sq_base.add(params.sq_off.array as usize).cast::<u32>() };
        for i in 0..params.sq_entries {
            unsafe { *sq_array.add(i as usize) = i };
        }

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
        })
    }

    #[inline]
    pub(crate) fn sq_space_left(&self) -> u32 {
        let head = unsafe { (*self.sq_head).load(Ordering::Acquire) };
        self.sq_entries - (self.local_sq_tail.wrapping_sub(head))
    }

    #[inline]
    pub(crate) fn unsubmitted(&self) -> u32 {
        self.unsubmitted
    }

    /// Queue an `IORING_OP_READ` SQE. Caller must ensure `buf[..len]` stays
    /// valid and exclusively writable by the kernel until the matching CQE is
    /// reaped.
    #[inline]
    pub(crate) unsafe fn push_read(&mut self, fd: RawFd, buf: *mut u8, len: u32, offset: u64, rw_flags: u32, user_data: u64) -> bool {
        if self.sq_space_left() == 0 {
            return false;
        }
        let idx = (self.local_sq_tail & self.sq_mask) as usize;
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
        true
    }

    /// Submit all queued SQEs and wait for at least `min_complete` CQEs.
    pub(crate) fn submit_and_wait(&mut self, min_complete: u32) -> io::Result<()> {
        if self.unsubmitted > 0 {
            unsafe { (*self.sq_tail).store(self.local_sq_tail, Ordering::Release) };
        }
        let mut to_submit = self.unsubmitted;
        self.unsubmitted = 0;

        let need_getevents = min_complete > 0 || (self.ring_flags & IORING_SETUP_DEFER_TASKRUN) != 0;
        if to_submit == 0 && min_complete == 0 && !need_getevents {
            return Ok(());
        }
        let enter_flags = if need_getevents { IORING_ENTER_GETEVENTS } else { 0 };

        loop {
            let rc = unsafe {
                libc::syscall(
                    libc::SYS_io_uring_enter,
                    self.fd,
                    to_submit,
                    min_complete,
                    enter_flags,
                    ptr::null::<libc::sigset_t>(),
                    size_of::<libc::sigset_t>(),
                )
            };
            if rc >= 0 {
                let submitted = rc as u32;
                to_submit = to_submit.saturating_sub(submitted);
                if to_submit == 0 {
                    return Ok(());
                }
                continue;
            }
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                let head = unsafe { (*self.sq_head).load(Ordering::Acquire) };
                to_submit = self.local_sq_tail.wrapping_sub(head);
                if min_complete == 0 && to_submit == 0 {
                    return Ok(());
                }
                continue;
            }
            return Err(err);
        }
    }

    #[inline]
    pub(crate) fn pop_cqe(&mut self) -> Option<IoUringCqe> {
        let head = unsafe { (*self.cq_head).load(Ordering::Relaxed) };
        let tail = unsafe { (*self.cq_tail).load(Ordering::Acquire) };
        if head == tail {
            return None;
        }
        let idx = (head & self.cq_mask) as usize;
        let cqe = unsafe { *self.cqes.add(idx) };
        unsafe { (*self.cq_head).store(head.wrapping_add(1), Ordering::Release) };
        Some(cqe)
    }
}

impl Drop for Uring {
    fn drop(&mut self) {
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

        ring.submit_and_wait(cap).unwrap();
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
        ring.submit_and_wait(1).unwrap();
        let err_cqe = ring.pop_cqe().expect("error CQE");
        assert_eq!(err_cqe.user_data, 0xDEAD_BEEF);
        assert_eq!(err_cqe.res, -libc::EBADF);
        ring.submit_and_wait(0).unwrap();

        let saved_fd = ring.fd;
        let saved_entries = ring.sq_entries;
        ring.release();
        // While cached in `LOCAL_URING`, `saved_fd` must remain open (not closed by Drop).
        assert!(unsafe { libc::fcntl(saved_fd, libc::F_GETFD) } >= 0, "Uring::release must cache the ring");
        let reused = Uring::acquire(saved_entries).unwrap();
        assert_eq!(reused.fd, saved_fd);
        reused.release();
        // Requesting more SQEs than cached must allocate a larger ring (`>=` vs `<`).
        let big = Uring::acquire(saved_entries + 16).unwrap();
        assert!(big.sq_entries >= saved_entries + 16);
    }

    #[test]
    fn uring_drop_closes_fd() {
        let Ok(ring) = Uring::new(8) else { return };
        let fd = ring.fd;
        drop(ring);
        let rc = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        assert_eq!(rc, -1, "Uring::drop must close the io_uring fd");
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::EBADF));
    }
}
