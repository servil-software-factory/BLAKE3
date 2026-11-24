//! `StackVec<T, N>`: a vector of at most `N` elements held inline. No heap,
//! no dependency, `no_std`.
//!
//! Its elements are `Copy`, so no destructor ever runs here: to shorten the
//! vector is to change its length, and a panic midway through a fill leaves
//! nothing to release.
//!
//! Invariant: `len <= N`, and `buf[..len]` is initialized.

use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};
use core::slice;

pub(crate) struct StackVec<T: Copy, const N: usize> {
    buf: [MaybeUninit<T>; N],
    len: usize,
}

impl<T: Copy, const N: usize> StackVec<T, N> {
    #[inline]
    pub(crate) fn new() -> Self {
        // The inline const leaves the array unwritten; `[MaybeUninit::uninit(); N]`
        // compiled to a memset of all N elements (seen in a release build).
        Self { buf: [const { MaybeUninit::uninit() }; N], len: 0 }
    }

    /// Panics when the vector is full (the index into `buf` is the check).
    #[inline]
    pub(crate) fn push(&mut self, value: T) {
        self.buf[self.len].write(value);
        self.len += 1;
    }

    /// Appends a copy of `other`. Panics when it does not fit (taking the
    /// free slots it needs is the check, made once).
    #[inline]
    #[cfg(any(blake3_sse41_ffi, test))]
    pub(crate) fn extend_from_slice(&mut self, other: &[T]) {
        let room = &mut self.buf[self.len..][..other.len()];
        for (slot, &item) in room.iter_mut().zip(other) {
            slot.write(item);
        }
        self.len += other.len();
    }

    #[inline]
    pub(crate) fn as_slice(&self) -> &[T] {
        // Sound: buf[..len] is initialized (the invariant).
        unsafe { slice::from_raw_parts(self.buf.as_ptr() as *const T, self.len) }
    }

    #[inline]
    pub(crate) fn as_mut_slice(&mut self) -> &mut [T] {
        // Sound: buf[..len] is initialized (the invariant).
        unsafe { slice::from_raw_parts_mut(self.buf.as_mut_ptr() as *mut T, self.len) }
    }
}

// What `each` alone calls, and `each` needs `std`.
#[cfg(feature = "std")]
impl<T: Copy, const N: usize> StackVec<T, N> {
    /// Whether the vector holds `N` elements.
    #[inline]
    pub(crate) fn is_full(&self) -> bool {
        self.len == N
    }

    #[inline]
    pub(crate) fn clear(&mut self) {
        self.len = 0;
    }
}

impl<T: Copy, const N: usize> Extend<T> for StackVec<T, N> {
    /// Panics when the iterator gives more elements than fit (the index
    /// into `buf` is the check).
    #[inline]
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        // The length stays in a local through the loop and is stored once
        // after it. Stored at every element, it was loaded and stored
        // again each time round (seen in a release build, filling a table
        // that a later call borrows). If the iterator panics the length
        // is the old one, and the elements written past it are bytes
        // nobody reads.
        let mut len = self.len;
        for item in iter {
            self.buf[len].write(item);
            len += 1;
        }
        self.len = len;
    }
}

impl<T: Copy, const N: usize> FromIterator<T> for StackVec<T, N> {
    /// Panics when the iterator gives more than `N` elements.
    #[inline]
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut out = Self::new();
        out.extend(iter);
        out
    }
}

impl<T: Copy, const N: usize> Default for StackVec<T, N> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy, const N: usize> Deref for StackVec<T, N> {
    type Target = [T];
    #[inline]
    fn deref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<T: Copy, const N: usize> DerefMut for StackVec<T, N> {
    #[inline]
    fn deref_mut(&mut self) -> &mut [T] {
        self.as_mut_slice()
    }
}

impl<'a, T: Copy, const N: usize> IntoIterator for &'a StackVec<T, N> {
    type Item = &'a T;
    type IntoIter = slice::Iter<'a, T>;
    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}
