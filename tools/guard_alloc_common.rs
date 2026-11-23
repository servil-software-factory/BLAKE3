//! Shared trailing guard-page `GlobalAlloc` for Unix test binaries (Approved Default 4).
//!
//! Each nonzero allocation receives its own private `mmap` region followed by a
//! mapped, inaccessible (`PROT_NONE`) trailing guard page, with the user pointer
//! placed as high as alignment permits (`(guard - size) & !(align - 1)`).
//! Therefore an allocation has zero writable trailing slack whenever
//! `size % align == 0` (in particular every `align == 1` byte buffer), and any
//! 1-byte overread or overwrite past a flush allocation faults immediately in
//! hardware.

use std::alloc::{GlobalAlloc, Layout};
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct GuardAllocator;

static PAGE_SIZE: AtomicUsize = AtomicUsize::new(0);

#[inline]
pub fn page_size() -> usize {
    let cached = PAGE_SIZE.load(Ordering::Relaxed);
    if cached != 0 {
        return cached;
    }
    // Sound: `sysconf(_SC_PAGESIZE)` touches no memory.
    let raw = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    let ps = usize::try_from(raw).expect("positive page size");
    PAGE_SIZE.store(ps, Ordering::Relaxed);
    ps
}

const HEADER_BYTES: usize = 2 * core::mem::size_of::<usize>();

// Sound: `GuardAllocator` satisfies `GlobalAlloc`'s alignment, non-overlap, and
// deallocation contracts by placing each allocation in its own `mmap` region
// right-aligned before a mapped `PROT_NONE` guard page and storing `[base, total]`
// in the `HEADER_BYTES` bytes immediately before the returned pointer.
unsafe impl GlobalAlloc for GuardAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size();
        let align = layout.align();
        if size == 0 {
            return align as *mut u8;
        }
        let page = page_size();
        let Some(need) = size.checked_add(align).and_then(|n| n.checked_add(HEADER_BYTES)) else {
            return ptr::null_mut();
        };
        let Some(data_bytes) = need.div_ceil(page).checked_mul(page) else {
            return ptr::null_mut();
        };
        let Some(total) = data_bytes.checked_add(page) else {
            return ptr::null_mut();
        };
        // Sound: anonymous private mapping of `total` bytes.
        let base = unsafe { libc::mmap(ptr::null_mut(), total, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_ANON, -1, 0) };
        if base == libc::MAP_FAILED {
            return ptr::null_mut();
        }
        let guard = (base as usize) + data_bytes;
        // Sound: `guard..guard + page` is the trailing page of this fresh mapping.
        if unsafe { libc::mprotect(guard as *mut libc::c_void, page, libc::PROT_NONE) } != 0 {
            // Sound: releasing the mapping just created on error.
            unsafe { libc::munmap(base, total) };
            return ptr::null_mut();
        }
        let user_addr = (guard - size) & !(align - 1);
        debug_assert!(user_addr >= (base as usize) + HEADER_BYTES);
        // Sound: `[base, total]` sits strictly inside `base..user_addr`.
        unsafe {
            ptr::write_unaligned((user_addr - HEADER_BYTES) as *mut [usize; 2], [base as usize, total]);
        }
        user_addr as *mut u8
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if layout.size() == 0 {
            return;
        }
        // Sound: `ptr` was returned by `alloc` with `[base, total]` stored at `ptr - HEADER_BYTES`.
        let [base, total] = unsafe { ptr::read_unaligned((ptr as usize - HEADER_BYTES) as *const [usize; 2]) };
        // Sound: releasing the exact mapping created by `alloc`.
        let rc = unsafe { libc::munmap(base as *mut libc::c_void, total) };
        debug_assert_eq!(rc, 0);
    }
}

/// Positive control: prove in a forked child process that reading 1 byte past
/// a flush `Vec<u8>` actually hits the `PROT_NONE` guard page and terminates
/// via `SIGSEGV` or `SIGBUS` (not a silent pass if `mprotect` were omitted).
#[allow(dead_code)]
pub fn verify_guard_page_faults_on_overread() {
    let ps = page_size();
    let buf = vec![0x5Au8; 13];
    let end_ptr = unsafe { buf.as_ptr().add(buf.capacity()) };
    assert_eq!((end_ptr as usize) % ps, 0, "1-byte aligned buffer must sit flush against PROT_NONE page");
    // Sound: single-threaded test process forks to observe a deliberate page fault in the child.
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "fork failed");
    if pid == 0 {
        // Sound (child-only): intentionally reading the PROT_NONE guard page to trigger SIGSEGV/SIGBUS.
        let _ = unsafe { core::ptr::read_volatile(end_ptr) };
        unsafe { libc::_exit(0) };
    }
    let mut status: libc::c_int = 0;
    loop {
        // Sound: waiting on our own child pid with valid `status` storage.
        let rc = unsafe { libc::waitpid(pid, &mut status, 0) };
        if rc == pid {
            break;
        }
        assert_eq!(std::io::Error::last_os_error().kind(), std::io::ErrorKind::Interrupted, "waitpid failed");
    }
    assert!(libc::WIFSIGNALED(status), "reading 1 byte past buffer must terminate by signal (status={status:#x})");
    let sig = libc::WTERMSIG(status);
    assert!(sig == libc::SIGSEGV || sig == libc::SIGBUS, "expected SIGSEGV or SIGBUS, got signal {sig}");
    drop(buf);
}
