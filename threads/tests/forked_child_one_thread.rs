//! A pool of one thread has no thread but its caller's, so a child forked
//! from its process lacks nothing: the child's multithreaded calls hash
//! on the child's own thread, as the parent's do. (Where the pool has
//! workers the child has none of them and its calls panic:
//! tests/forked_child.rs.) A binary of its own: the pool is the
//! process's.

use blake3_servil::throughput::{self, Block};
use blake3_servil::{Mode, convenience};

#[test]
fn a_child_forked_from_a_pool_of_one_thread_hashes_as_its_parent_does() {
    blake3_servil::initialize_multithreaded(core::num::NonZeroUsize::new(1));
    let blocks = vec![Block([7u8; 64]); (1 << 20) / 64];
    let bytes: Vec<u8> = blocks.iter().flat_map(|b| b.0).collect();
    let want = convenience::hash(&bytes);
    assert_eq!(throughput::hash_multithreaded(Mode::Plain, &blocks, bytes.len()), want, "the parent, before the fork");
    // Sound: the child only hashes, catches a panic and exits.
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "fork");
    if pid == 0 {
        let multi = std::panic::catch_unwind(|| throughput::hash_multithreaded(Mode::Plain, &blocks, bytes.len()));
        // Sound: _exit ends the child at once, with no handlers.
        unsafe {
            libc::_exit(match multi {
                Ok(hash) if hash == want => 0,
                Ok(_) => 2,
                Err(_) => 1,
            })
        }
    }
    let mut status = 0;
    // Sound: waits for the child above.
    assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
    assert!(libc::WIFEXITED(status), "the child exited (status {status})");
    assert_eq!(libc::WEXITSTATUS(status), 0, "1: the child's multithreaded call panicked, 2: its hash was wrong");
    assert_eq!(throughput::hash_multithreaded(Mode::Plain, &blocks, bytes.len()), want, "the parent, after the fork");
}
