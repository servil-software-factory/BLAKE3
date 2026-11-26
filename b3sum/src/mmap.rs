//! Read-only file memory mapping over `libc::mmap`, replacing `memmap2`.

use core::ops::Deref;
use core::ptr::{self, NonNull};
use std::io;
use std::os::fd::AsRawFd;

pub(crate) struct Mmap {
    ptr: NonNull<u8>,
    len: usize,
}

// Sound: `Mmap` holds a read-only mapping (`PROT_READ`) for its lifetime and
// exposes only `&[u8]`; moving or sharing it across threads introduces no
// data race.
unsafe impl Send for Mmap {}
unsafe impl Sync for Mmap {}

impl Mmap {
    /// Map `file` read-only from offset 0 through its current length.
    ///
    /// # Safety
    ///
    /// The underlying file must not be modified or truncated concurrently
    /// while the mapping is live (see `b3sum/README.md`, `--no-mmap`).
    pub(crate) unsafe fn map(file: &impl AsRawFd) -> io::Result<Self> {
        let fd = file.as_raw_fd();
        // Sound: `libc::stat` is a plain C struct of integers initialized by `fstat`.
        let mut st: libc::stat = unsafe { core::mem::zeroed() };
        // Sound: `st` is valid for writes by `fstat(2)` on `fd`.
        if unsafe { libc::fstat(fd, &raw mut st) } != 0 {
            return Err(io::Error::last_os_error());
        }
        if st.st_size < 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "negative file size"));
        }
        let Ok(len) = usize::try_from(st.st_size) else {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "file size overflows usize"));
        };
        if len > isize::MAX as usize {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "file size overflows isize"));
        }
        if len == 0 {
            return Ok(Self { ptr: NonNull::dangling(), len: 0 });
        }
        // Sound: `len > 0` and fits in `isize`, `fd` is a live open descriptor,
        // and the mapping is read-only.
        let mapped = unsafe { libc::mmap(ptr::null_mut(), len, libc::PROT_READ, libc::MAP_SHARED, fd, 0) };
        if mapped == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        let Some(ptr) = NonNull::new(mapped.cast::<u8>()) else {
            // Sound: `mapped` came from a successful `mmap` of length `len`
            // (not `MAP_FAILED`), so unmap it before returning an error.
            unsafe {
                let _ = libc::munmap(mapped, len);
            }
            return Err(io::Error::other("mmap returned null"));
        };
        Ok(Self { ptr, len })
    }
}

impl Deref for Mmap {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &[u8] {
        // Sound: for `len == 0`, `ptr` is non-null and aligned for `u8`; for
        // `len > 0`, `ptr..ptr+len` is a valid read-only mapping owned by `self`
        // (under the caller's immutability contract on `Mmap::map`).
        unsafe { core::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

impl AsRef<[u8]> for Mmap {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl Drop for Mmap {
    fn drop(&mut self) {
        if self.len > 0 {
            // Sound: `self.ptr` and `self.len` came from a successful `mmap`
            // in `Mmap::map` and are unmapped once here.
            unsafe {
                let _ = libc::munmap(self.ptr.as_ptr().cast(), self.len);
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::fs::{self, File};
    use std::os::fd::RawFd;

    struct BadFd(RawFd);
    impl AsRawFd for BadFd {
        fn as_raw_fd(&self) -> RawFd {
            self.0
        }
    }

    #[test]
    fn maps_regular_and_empty_files_and_rejects_bad_fd() {
        let dir = crate::tempfile::tempdir().unwrap();
        let nonempty = dir.path().join("data");
        fs::write(&nonempty, b"hello mmap").unwrap();
        let f = File::open(&nonempty).unwrap();
        let m = unsafe { Mmap::map(&f) }.unwrap();
        assert_eq!(&*m, b"hello mmap");
        drop(m);

        let empty = dir.path().join("empty");
        fs::write(&empty, b"").unwrap();
        let ef = File::open(&empty).unwrap();
        let em = unsafe { Mmap::map(&ef) }.unwrap();
        assert_eq!(&*em, b"");

        assert!(unsafe { Mmap::map(&BadFd(-1)) }.is_err());
    }
}
