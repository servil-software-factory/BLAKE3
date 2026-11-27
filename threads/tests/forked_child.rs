//! A child forked after the pool started its threads has none of them:
//! the `pthread_atfork` child handler sets the process-wide fork flag so
//! any multithreaded call stops rather than hanging on vanished workers.
//!
//! Because POSIX permits only async-signal-safe calls between `fork()` and
//! `_exit()` in a multithreaded process, the forked child here ONLY reads
//! the atomic fork flag (`__is_forked()`) and calls `_exit()`; the panic
//! message and single-threaded fallback under that flag are verified in a
//! normal un-forked subprocess (`lanes::test::marked_forked_state_rejects_multithreaded_and_allows_single_threaded`
//! and `pipeline::test::forked_child_stops_in_acquire_free_group_and_alloc_piece_node`).

use blake3_servil::throughput::{self, Block};
use blake3_servil::{Mode, convenience};

#[test]
fn a_forked_child_marks_the_fork_flag_while_the_parent_continues() {
    blake3_servil::initialize_multithreaded(core::num::NonZeroUsize::new(2));
    let blocks = vec![Block([7u8; 64]); (1 << 20) / 64];
    let bytes: Vec<u8> = blocks.iter().flat_map(|b| b.0).collect();
    let want = convenience::hash(&bytes);
    assert!(!blake3_servil::__is_forked(), "parent must not be marked forked before fork()");
    assert_eq!(throughput::hash_multithreaded(Mode::Plain, &blocks, bytes.len()), want, "the parent, before the fork");
    // Sound: in the child, only an atomic load (`__is_forked`) and async-signal-safe `_exit` run.
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "fork");
    if pid == 0 {
        let code = if blake3_servil::__is_forked() { 0 } else { 1 };
        // Sound: `_exit` is async-signal-safe and ends the child without running handlers.
        unsafe { libc::_exit(code) }
    }
    let mut status = 0;
    // Sound: waits for the child above.
    assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
    assert!(libc::WIFEXITED(status), "the child exited (status {status})");
    assert_eq!(libc::WEXITSTATUS(status), 0, "1: pthread_atfork child handler did not set FORKED");
    assert!(!blake3_servil::__is_forked(), "parent must remain unmarked after fork()");
    assert_eq!(throughput::hash_multithreaded(Mode::Plain, &blocks, bytes.len()), want, "the parent, after the fork");
}
