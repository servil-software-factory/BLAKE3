//! A pool of one thread has no thread but its caller's, so a child forked
//! from its process lacks nothing: the child's multithreaded calls hash
//! on the child's own thread, as the parent's do. (Where the pool has
//! workers the child has none of them and its calls panic:
//! tests/forked_child.rs.)
//!
//! Configured with `harness = false` in `Cargo.toml` so `libtest` spawns no
//! runner thread and the process is genuinely single-threaded at `fork()`
//! (verified on Linux via `/proc/self/status`).

use blake3_servil::throughput::{self, Block};
use blake3_servil::{Mode, convenience};

fn run_test() {
    blake3_servil::initialize_multithreaded(core::num::NonZeroUsize::new(1));
    // Under QEMU user-mode emulation (`BLAKE3_EMULATED=1`), QEMU maintains its
    // own internal signal/RCU thread so `/proc/self/status` reports 2 threads;
    // on native Linux this binary has genuinely 1 OS thread.
    #[cfg(target_os = "linux")]
    if std::env::var_os("BLAKE3_EMULATED").is_none() {
        let status = std::fs::read_to_string("/proc/self/status").expect("read /proc/self/status");
        let threads = status
            .lines()
            .find_map(|line| line.strip_prefix("Threads:"))
            .and_then(|v| v.trim().parse::<usize>().ok())
            .expect("Threads field in /proc/self/status");
        assert_eq!(threads, 1, "one-thread fork test must be genuinely single-threaded before fork()");
    }
    let blocks = vec![Block([7u8; 64]); (1 << 20) / 64];
    let bytes: Vec<u8> = blocks.iter().flat_map(|b| b.0).collect();
    let want = convenience::hash(&bytes);
    assert_eq!(throughput::hash_multithreaded(Mode::Plain, &blocks, bytes.len()), want, "the parent, before the fork");
    // Sound: this process has a single OS thread (`harness = false` and a
    // one-thread pool), so full Rust execution after `fork()` is POSIX-valid.
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "fork");
    if pid == 0 {
        let multi = std::panic::catch_unwind(|| throughput::hash_multithreaded(Mode::Plain, &blocks, bytes.len()));
        let flagged = blake3_servil::__is_forked();
        // Sound: `_exit` ends the child cleanly without duplicating parent stdio buffers.
        unsafe {
            libc::_exit(if flagged {
                3
            } else {
                match multi {
                    Ok(hash) if hash == want => 0,
                    Ok(_) => 2,
                    Err(_) => 1,
                }
            })
        }
    }
    let mut status = 0;
    // Sound: waits for the child above.
    assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
    assert!(libc::WIFEXITED(status), "the child exited (status {status})");
    assert_eq!(libc::WEXITSTATUS(status), 0, "1: the child's multithreaded call panicked, 2: its hash was wrong, 3: one-thread pool set FORKED");
    assert_eq!(throughput::hash_multithreaded(Mode::Plain, &blocks, bytes.len()), want, "the parent, after the fork");
}

fn main() {
    // Minimal libtest protocol so `cargo-nextest` discovers and runs this
    // single-threaded (`harness = false`) test cleanly without running the
    // fork during `--list` discovery or spawning a `libtest` worker thread.
    const TEST_NAME: &str = "a_child_forked_from_a_pool_of_one_thread_hashes_as_its_parent_does";
    let mut list = false;
    let mut ignored = false;
    let mut exact = false;
    let mut filters = Vec::new();
    let mut skips = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("Usage: forked_child_one_thread [--list] [--exact] [--ignored] [--skip PAT] [FILTER...]");
                return;
            }
            "--list" => list = true,
            "--ignored" => ignored = true,
            "--exact" => exact = true,
            "--nocapture" | "--show-output" | "--quiet" | "-q" | "--include-ignored" => {}
            "--skip" => {
                if let Some(pat) = args.next() {
                    skips.push(pat);
                }
            }
            s if s.starts_with("--skip=") => {
                skips.push(s["--skip=".len()..].to_string());
            }
            "--format" | "--test-threads" | "--color" => {
                let _ = args.next();
            }
            s if s.starts_with('-') => {}
            _ => filters.push(arg),
        }
    }
    let matches_pat = |pat: &str| if exact { TEST_NAME == pat } else { TEST_NAME.contains(pat) };
    let skipped = skips.iter().any(|s| matches_pat(s));
    let matched = !ignored && !skipped && (filters.is_empty() || filters.iter().any(|f| matches_pat(f)));
    if list {
        if matched {
            println!("{TEST_NAME}: test");
        }
        return;
    }
    if matched {
        run_test();
    }
}
