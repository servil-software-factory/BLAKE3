//! The tests' helpers that both crates' tests use: this crate's unit
//! tests (as `crate::test`'s own) and the thread crate's, which compiles
//! this file as its `crate::test`. Every path here is one both crates
//! have.

/// Dropped, it releases a thread a test holds on its channel (`recv`): so
/// a fault in the test fails it at once where it would strand the held
/// thread, and the test with it, until the clock.
#[cfg(feature = "std")]
#[allow(dead_code)] // The thread crate's tests hold threads with it.
pub(crate) struct Released(pub(crate) std::sync::mpsc::Sender<()>);
#[cfg(feature = "std")]
impl Drop for Released {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

/// `bytes` copied into whole blocks, zero after them: a message as the
/// throughput calls take one.
#[cfg(feature = "std")]
pub(crate) fn message(bytes: &[u8]) -> std::vec::Vec<crate::throughput::Block> {
    let mut blocks = std::vec![crate::throughput::Block::ZERO; bytes.len().div_ceil(crate::BLOCK_LEN).max(1)];
    crate::throughput::Block::bytes_mut(&mut blocks)[..bytes.len()].copy_from_slice(bytes);
    blocks
}

/// Each of `items` copied into blocks (`message`).
#[cfg(feature = "std")]
pub(crate) fn messages(items: &[&[u8]]) -> std::vec::Vec<std::vec::Vec<crate::throughput::Block>> {
    items.iter().map(|i| message(i)).collect()
}

/// `owned`'s messages (from `message`) as hash_each's items, `items`'
/// lengths.
#[cfg(feature = "std")]
pub(crate) fn items<'a>(owned: &'a [std::vec::Vec<crate::throughput::Block>], items: &[&[u8]]) -> std::vec::Vec<(&'a [crate::throughput::Block], usize)> {
    owned.iter().zip(items).map(|(b, i)| (&b[..], i.len())).collect()
}

/// `bytes` (whole blocks) copied into blocks, as the batch calls take them.
pub(crate) fn blocks(bytes: &[u8]) -> std::vec::Vec<crate::throughput::Block> {
    assert_eq!(bytes.len() % crate::BLOCK_LEN, 0, "whole blocks");
    let mut blocks = std::vec![crate::throughput::Block::ZERO; bytes.len() / crate::BLOCK_LEN];
    crate::throughput::Block::bytes_mut(&mut blocks).copy_from_slice(bytes);
    blocks
}

// Golden, the fixed values' check, shared with the integration tests.
#[cfg(feature = "std")]
#[path = "../tests/common/golden.rs"]
mod golden;
#[cfg(feature = "std")]
pub(crate) use golden::Golden;

/// Official BLAKE3 1.8.7, the generator of the fixed values: its public
/// calls, and its hidden portable kernels (`blake3::platform`) for the
/// compression-level sweeps.
#[cfg(feature = "std")]
pub(crate) mod official {
    use std::vec::Vec;
    fn portable() -> blake3::platform::Platform {
        blake3::platform::Platform::portable()
    }
    pub(crate) fn hash(mode: crate::Mode, input: &[u8]) -> [u8; 32] {
        match mode {
            crate::Mode::Plain => *blake3::hash(input).as_bytes(),
            crate::Mode::Keyed(key) => *blake3::keyed_hash(key, input).as_bytes(),
            crate::Mode::DeriveKey(context) => blake3::derive_key(context, input),
            crate::Mode::DeriveKeyWith(_) => unreachable!("a context key's own mode is DeriveKey"),
        }
    }
    /// `len` bytes of extended output from byte `skip`.
    pub(crate) fn xof(mode: crate::Mode, input: &[u8], skip: u64, len: usize) -> Vec<u8> {
        let mut hasher = match mode {
            crate::Mode::Plain => blake3::Hasher::new(),
            crate::Mode::Keyed(key) => blake3::Hasher::new_keyed(key),
            crate::Mode::DeriveKey(context) => blake3::Hasher::new_derive_key(context),
            crate::Mode::DeriveKeyWith(_) => unreachable!("a context key's own mode is DeriveKey"),
        };
        hasher.update(input);
        let mut reader = hasher.finalize_xof();
        reader.set_position(skip);
        let mut out = std::vec![0u8; len];
        reader.fill(&mut out);
        out
    }
    /// Merge two chaining values without depending on our tree kernels.
    pub(crate) fn parent(mode: crate::Mode, left: &[u8; 32], right: &[u8; 32], root: bool) -> [u8; 32] {
        use blake3::hazmat;
        let key;
        let mode = match mode {
            crate::Mode::Plain => hazmat::Mode::Hash,
            crate::Mode::Keyed(key) => hazmat::Mode::KeyedHash(key),
            crate::Mode::DeriveKey(context) => {
                key = hazmat::hash_derive_key_context(context);
                hazmat::Mode::DeriveKeyMaterial(&key)
            }
            crate::Mode::DeriveKeyWith(key) => hazmat::Mode::DeriveKeyMaterial(key),
        };
        if root { *hazmat::merge_subtrees_root(left, right, mode).as_bytes() } else { hazmat::merge_subtrees_non_root(left, right, mode) }
    }
    pub(crate) fn compress_xof(cv: &[u32; 8], block: &[u8; 64], block_len: u8, counter: u64, flags: u8) -> [u8; 64] {
        portable().compress_xof(cv, block, block_len, counter, flags)
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn hash_many<const N: usize>(inputs: &[&[u8; N]], key: &[u32; 8], counter: u64, increment: bool, flags: u8, start: u8, end: u8, out: &mut [u8]) {
        let increment = if increment { blake3::IncrementCounter::Yes } else { blake3::IncrementCounter::No };
        portable().hash_many::<N>(inputs, key, counter, increment, flags, start, end, out)
    }
    /// Each chunk's chaining value (a chunk of the plain hash, not the root).
    pub(crate) fn chunk_cvs(input: &[u8], counter: u64) -> Vec<u8> {
        input
            .chunks(crate::CHUNK_LEN)
            .enumerate()
            .flat_map(|(i, c)| {
                use blake3::hazmat::HasherExt;
                blake3::Hasher::new().set_input_offset((counter + i as u64) * crate::CHUNK_LEN as u64).update(c).finalize_non_root()
            })
            .collect()
    }
}

pub fn paint_test_input(buf: &mut [u8]) {
    for (i, b) in buf.iter_mut().enumerate() {
        *b = (i % 251) as u8;
    }
}

/// Buffers between inaccessible pages, for the tests that place inputs
/// and outputs flush against one (each crate's guard-page tests).
#[cfg(all(feature = "std", any(target_arch = "aarch64", target_arch = "x86_64"), any(target_vendor = "apple", target_os = "linux")))]
#[allow(dead_code)]
pub(crate) mod guard {
    /// `len` bytes with an inaccessible page before and after them, the
    /// bytes flush against the page after (`at_end`) or before.
    pub(crate) struct Guarded {
        pub(crate) base: *mut u8,
        pub(crate) total: usize,
        pub(crate) start: usize,
        pub(crate) len: usize,
    }

    impl Guarded {
        pub(crate) fn new(len: usize, at_end: bool) -> Self {
            let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as usize;
            let body = len.div_ceil(page).max(1) * page;
            let total = body + 2 * page;
            let base = unsafe { libc::mmap(core::ptr::null_mut(), total, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, -1, 0) };
            assert_ne!(base, libc::MAP_FAILED, "mmap");
            let base = base as *mut u8;
            unsafe {
                assert_eq!(libc::mprotect(base as *mut _, page, libc::PROT_NONE), 0);
                assert_eq!(libc::mprotect(base.add(page + body) as *mut _, page, libc::PROT_NONE), 0);
            }
            let start = if at_end { page + body - len } else { page };
            Guarded { base, total, start, len }
        }

        pub(crate) fn bytes(&mut self) -> &mut [u8] {
            unsafe { core::slice::from_raw_parts_mut(self.base.add(self.start), self.len) }
        }

        /// Build shared views first; after this call the body is read-only.
        #[cfg(all(target_os = "linux", blake3_avx512_ffi))]
        pub(crate) fn read_only(&mut self) {
            let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as usize;
            assert_eq!(unsafe { libc::mprotect(self.base.add(page).cast(), self.total - 2 * page, libc::PROT_READ) }, 0);
        }
    }

    impl Drop for Guarded {
        fn drop(&mut self) {
            unsafe { libc::munmap(self.base as *mut _, self.total) };
        }
    }

    pub(crate) fn filled(len: usize, at_end: bool) -> Guarded {
        let mut g = Guarded::new(len, at_end);
        crate::test::paint_test_input(g.bytes());
        g
    }

    /// `count` digests' places flush against a guard page (after them, or
    /// before): a call writes no byte outside its outputs.
    pub(crate) fn places(count: usize, at_end: bool) -> (Guarded, &'static mut [crate::Hash]) {
        let mut g = Guarded::new(count * crate::OUT_LEN, at_end);
        // Sound: Hash is repr(transparent) over [u8; 32], align 1, and the
        // pages live until the Guarded drops, after every use here.
        let out = unsafe { core::slice::from_raw_parts_mut(g.bytes().as_mut_ptr() as *mut crate::Hash, count) };
        (g, out)
    }

    /// The same places as digests' bytes, for the calls that write those.
    pub(crate) fn digests(count: usize, at_end: bool) -> (Guarded, &'static mut [[u8; crate::OUT_LEN]]) {
        let mut g = Guarded::new(count * crate::OUT_LEN, at_end);
        let out = unsafe { core::slice::from_raw_parts_mut(g.bytes().as_mut_ptr() as *mut [u8; crate::OUT_LEN], count) };
        (g, out)
    }
}
