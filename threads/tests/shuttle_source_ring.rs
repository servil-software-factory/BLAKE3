//! Shuttle concurrency model of the production `action_ring` publish/claim
//! ticket ring (M4). Compiles the real `src/action_ring.rs` against Shuttle's
//! atomics (`--cfg shuttle`), exercising batched publishes across multiple
//! wrap-around laps with competing claimers.
#![cfg(shuttle)]

mod action_ring_sync {
    pub use shuttle::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};
}

#[path = "../src/action_ring.rs"]
mod action_ring;

use action_ring::{Action, publish_slice, take_one};
use shuttle::sync::Arc;
use shuttle::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[test]
fn action_ring_publish_claim_two_laps_under_shuttle() {
    // Tiny ring (cap = 4), 10 tickets (> 2 full wrap-around laps), published in
    // variable-size batches [1, 3, 2, 1, 3] with 2 competing claimers: checks
    // that every sequence number `seq` is claimed exactly once with the exact
    // `(context, arg)` published for `seq`, even while the publisher wraps and
    // reuses slots immediately after `claimed` advances.
    const CAP: usize = 4;
    const BATCHES: &[usize] = &[1, 3, 2, 1, 3];
    const TOTAL: usize = 10;

    shuttle::check_random(
        || {
            let actions: Arc<Box<[Action]>> = Arc::new((0..CAP).map(|_| Action::empty()).collect::<Vec<_>>().into_boxed_slice());
            let published = Arc::new(AtomicUsize::new(0));
            let claimed = Arc::new(AtomicUsize::new(0));
            let done_pub = Arc::new(AtomicBool::new(false));
            let seen: Arc<Vec<AtomicUsize>> = Arc::new((0..TOTAL).map(|_| AtomicUsize::new(0)).collect());

            let mut workers = Vec::new();
            for _ in 0..2 {
                let actions = Arc::clone(&actions);
                let published = Arc::clone(&published);
                let claimed = Arc::clone(&claimed);
                let done_pub = Arc::clone(&done_pub);
                let seen = Arc::clone(&seen);
                workers.push(shuttle::thread::spawn(move || {
                    loop {
                        if let Some((seq, ctx, arg)) = take_one(&actions, &published, &claimed) {
                            assert!(seq < TOTAL, "corrupt sequence number {seq}");
                            // Derive expected `(batch, arg, ctx)` strictly from
                            // `seq` and `BATCHES`, never from the returned `arg`.
                            let (expected_batch, expected_arg, expected_ctx) = {
                                let mut base = 0usize;
                                let mut found = None;
                                for (b, &len) in BATCHES.iter().enumerate() {
                                    if seq < base + len {
                                        found = Some((b, ((b as u64) << 32) | (seq as u64), (0x1000usize + b * 0x100) as *const ()));
                                        break;
                                    }
                                    base += len;
                                }
                                found.expect("seq < TOTAL lies within BATCHES")
                            };
                            assert_eq!(arg, expected_arg, "arg mismatch for ticket {seq} (batch {expected_batch})");
                            assert_eq!(ctx, expected_ctx, "context mismatch for ticket {seq} (batch {expected_batch})");
                            assert_eq!(seen[seq].fetch_add(1, Ordering::SeqCst), 0, "ticket {seq} claimed twice");
                        } else if done_pub.load(Ordering::Acquire) && claimed.load(Ordering::Acquire) == TOTAL {
                            break;
                        } else {
                            shuttle::thread::yield_now();
                        }
                    }
                }));
            }

            let mut next_seq = 0usize;
            for (batch_idx, &batch_len) in BATCHES.iter().enumerate() {
                while published.load(Ordering::Relaxed) - claimed.load(Ordering::Acquire) + batch_len > CAP {
                    shuttle::thread::yield_now();
                }
                let args: Vec<u64> = (0..batch_len).map(|k| ((batch_idx as u64) << 32) | ((next_seq + k) as u64)).collect();
                let ctx = (0x1000usize + batch_idx * 0x100) as *const ();
                publish_slice(&actions, &published, &claimed, ctx, &args, batch_idx % 2 == 0);
                next_seq += batch_len;
            }
            assert_eq!(next_seq, TOTAL);
            done_pub.store(true, Ordering::Release);

            for w in workers {
                w.join().unwrap();
            }
            for seq in 0..TOTAL {
                assert_eq!(seen[seq].load(Ordering::SeqCst), 1, "ticket {seq} missing");
            }
        },
        4_000,
    );
}
