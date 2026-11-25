//! Shuttle model check of the production lock-free epoch source registry
//! (`src/registry.rs`). Compiles the exact production file beside Shuttle's
//! atomics (`registry_sync`), verifying:
//! 1. Concurrent workers scanning under bounded bursts (`BURST = 2`) with a
//!    continuously regenerating source never read a freed/poisoned source after
//!    `unregister_and_drain` and never starve a drainer.
//! 2. An already-unregistered thread-local source freed after `drain_readers`
//!    never suffers a post-drain dereference by any scanning worker.

#![cfg(shuttle)]

mod registry_sync {
    pub use shuttle::sync::atomic::{AtomicPtr, AtomicU64, Ordering, fence};
    #[inline]
    pub(crate) fn spin() {
        shuttle::thread::yield_now();
    }
    #[inline]
    pub(crate) fn pause_slow() {
        shuttle::thread::yield_now();
    }
}

#[allow(dead_code)]
#[path = "../src/registry.rs"]
mod registry;

use registry::{EpochRegistry, ReaderCell, ReaderToken};
use shuttle::sync::Arc;
use shuttle::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use shuttle::thread;

fn check(test: impl Fn() + Send + Sync + 'static) {
    let runner = shuttle::Runner::new(shuttle::scheduler::PctScheduler::new(3, 500), shuttle::Config::default());
    runner.run(test);
}

struct DummySource {
    waiting: AtomicUsize,
    regenerating: bool,
    freed: AtomicBool,
}

#[test]
fn unregister_and_drain_never_reads_freed_source_or_starves_under_continuous_work() {
    const SLOTS: usize = 4;
    const BURST: usize = 2;

    check(|| {
        let reg = Arc::new(EpochRegistry::<DummySource, SLOTS>::new());
        let readers = Arc::new([ReaderCell::new(), ReaderCell::new()]);
        let stop = Arc::new(AtomicBool::new(false));
        let quarantined = Arc::new(shuttle::sync::Mutex::new(Vec::<usize>::new()));

        // One continuously regenerating source stays registered while callers
        // register and drain other slots; an unbounded worker burst would
        // starve callers' drains here.
        let busy = Box::into_raw(Box::new(DummySource { waiting: AtomicUsize::new(1), regenerating: true, freed: AtomicBool::new(false) }));
        // Sound: `busy` stays quarantined until all workers join.
        let busy_slot = unsafe { reg.register(busy) }.expect("busy slot");

        let workers: Vec<_> = (0..2)
            .map(|me| {
                let reg = Arc::clone(&reg);
                let readers = Arc::clone(&readers);
                let stop = Arc::clone(&stop);
                thread::spawn(move || {
                    let mut token = ReaderToken::new(&readers[me]);
                    while !stop.load(Ordering::Acquire) {
                        if reg.any_active() {
                            let guard = token.enter();
                            for _ in 0..BURST {
                                let found = reg
                                    .find_from(&guard, 0, |src| {
                                        assert!(!src.freed.load(Ordering::Acquire), "worker dereferenced freed Source before read");
                                        if src.waiting.load(Ordering::Acquire) > 0 {
                                            if !src.regenerating {
                                                let _ = src.waiting.fetch_update(Ordering::AcqRel, Ordering::Relaxed, |w| w.checked_sub(1));
                                            }
                                            assert!(!src.freed.load(Ordering::Acquire), "worker observed freed Source mid-read");
                                            true
                                        } else {
                                            assert!(!src.freed.load(Ordering::Acquire), "worker observed freed Source after empty check");
                                            false
                                        }
                                    })
                                    .is_some();
                                if !found {
                                    break;
                                }
                            }
                        }
                        thread::yield_now();
                    }
                })
            })
            .collect();

        let callers: Vec<_> = (0..2)
            .map(|_| {
                let reg = Arc::clone(&reg);
                let readers = Arc::clone(&readers);
                let quarantined = Arc::clone(&quarantined);
                thread::spawn(move || {
                    for _ in 0..3 {
                        let ptr = Box::into_raw(Box::new(DummySource { waiting: AtomicUsize::new(2), regenerating: false, freed: AtomicBool::new(false) }));
                        // Sound: `ptr` stays quarantined until all workers join.
                        if let Some(slot) = unsafe { reg.register(ptr) } {
                            thread::yield_now();
                            reg.unregister_and_drain(slot, Some(&readers[..]));
                        }
                        // Poison before quarantine: any worker reading `ptr`
                        // after `unregister_and_drain` immediately trips the assert.
                        unsafe { (*ptr).freed.store(true, Ordering::Release) };
                        quarantined.lock().unwrap().push(ptr as usize);
                    }
                })
            })
            .collect();

        for c in callers {
            c.join().unwrap();
        }
        reg.unregister_and_drain(busy_slot, Some(&readers[..]));
        unsafe { (*busy).freed.store(true, Ordering::Release) };
        quarantined.lock().unwrap().push(busy as usize);
        stop.store(true, Ordering::Release);
        for w in workers {
            w.join().unwrap();
        }
        for addr in core::mem::take(&mut *quarantined.lock().unwrap()) {
            // Sound: all threads have joined.
            unsafe { drop(Box::from_raw(addr as *mut DummySource)) };
        }
    });
}

#[test]
fn unregister_then_drain_readers_never_reads_freed_job_source() {
    const SLOTS: usize = 4;

    check(|| {
        let reg = Arc::new(EpochRegistry::<DummySource, SLOTS>::new());
        let readers = Arc::new([ReaderCell::new(), ReaderCell::new()]);
        let stop = Arc::new(AtomicBool::new(false));
        let quarantined = Arc::new(shuttle::sync::Mutex::new(Vec::<usize>::new()));

        let workers: Vec<_> = (0..2)
            .map(|me| {
                let reg = Arc::clone(&reg);
                let readers = Arc::clone(&readers);
                let stop = Arc::clone(&stop);
                thread::spawn(move || {
                    let mut token = ReaderToken::new(&readers[me]);
                    while !stop.load(Ordering::Acquire) {
                        if reg.any_active() {
                            let guard = token.enter();
                            let _ = reg.find_from(&guard, 0, |src| {
                                assert!(!src.freed.load(Ordering::Acquire), "worker dereferenced freed job Source");
                                let _ = src.waiting.load(Ordering::Acquire);
                                assert!(!src.freed.load(Ordering::Acquire), "worker observed freed job Source after read");
                                false
                            });
                        }
                        thread::yield_now();
                    }
                })
            })
            .collect();

        let caller = {
            let reg = Arc::clone(&reg);
            let readers = Arc::clone(&readers);
            let quarantined = Arc::clone(&quarantined);
            thread::spawn(move || {
                // Simulate a thread-local job source: fast register/unregister
                // across two jobs, then `drain_readers` once before freeing on
                // thread exit.
                let ptr = Box::into_raw(Box::new(DummySource { waiting: AtomicUsize::new(1), regenerating: false, freed: AtomicBool::new(false) }));
                for _ in 0..2 {
                    // Sound: `ptr` stays quarantined until all threads join.
                    if let Some(slot) = unsafe { reg.register(ptr) } {
                        thread::yield_now();
                        reg.unregister(slot);
                    }
                }
                EpochRegistry::<DummySource, SLOTS>::drain_readers(&readers[..]);
                unsafe { (*ptr).freed.store(true, Ordering::Release) };
                quarantined.lock().unwrap().push(ptr as usize);
            })
        };

        caller.join().unwrap();
        stop.store(true, Ordering::Release);
        for w in workers {
            w.join().unwrap();
        }
        for addr in core::mem::take(&mut *quarantined.lock().unwrap()) {
            // Sound: all threads have joined.
            unsafe { drop(Box::from_raw(addr as *mut DummySource)) };
        }
    });
}

#[test]
fn registry_basics_wrap_pause_and_draining_slot_protection() {
    let mut spins = 0u32;
    let mut spin_calls = 0u32;
    let mut sleep_calls = 0u32;
    for _ in 0..66 {
        registry::drain_pause_with(&mut spins, || spin_calls += 1, || sleep_calls += 1);
    }
    assert_eq!(spins, 64);
    assert_eq!(spin_calls, 64);
    assert_eq!(sleep_calls, 2);

    shuttle::check_random(
        || {
            let cell = ReaderCell::new();
            let mut token = ReaderToken::new(&cell);
            {
                let _g = token.enter();
                assert_eq!(cell.load_seqcst(), 1);
            }
            assert_eq!(cell.load_seqcst(), 0);
            token.set_seq_for_test(u64::MAX);
            {
                let _g = token.enter();
                assert_eq!(cell.load_seqcst(), 1);
            }
            assert_eq!(cell.load_seqcst(), 0);

            let reg = Arc::new(EpochRegistry::<DummySource, 4>::new());
            assert!(!reg.any_active());
            let a = Box::into_raw(Box::new(DummySource { waiting: AtomicUsize::new(1), regenerating: false, freed: AtomicBool::new(false) }));
            let b = Box::into_raw(Box::new(DummySource { waiting: AtomicUsize::new(2), regenerating: false, freed: AtomicBool::new(false) }));
            let slot_a = unsafe { reg.register(a) }.expect("slot a");
            let slot_b = unsafe { reg.register(b) }.expect("slot b");
            assert!(reg.any_active());
            assert!(reg.is_active(slot_a) && reg.is_active(slot_b));
            assert!(reg.is_registered(slot_a) && reg.is_registered(slot_b));
            {
                let g = token.enter();
                assert_eq!(reg.find_from(&g, 0, |s| s.waiting.load(Ordering::Relaxed) == 2), Some(slot_b));
                assert_eq!(reg.find_from(&g, 1, |s| s.waiting.load(Ordering::Relaxed) == 1), Some(slot_a));
                assert_eq!(reg.find_from(&g, 0, |s| s.waiting.load(Ordering::Relaxed) == 99), None);
            }
            reg.unregister(slot_a);
            assert!(!reg.is_active(slot_a) && !reg.is_registered(slot_a));
            assert!(reg.is_active(slot_b) && reg.is_registered(slot_b));
            assert!(reg.any_active());
            reg.unregister(slot_b);
            assert!(!reg.any_active());

            let readers = Arc::new([ReaderCell::new()]);
            let slot0 = unsafe { reg.register(a) }.expect("slot 0");
            let slot1 = unsafe { reg.register(b) }.expect("slot 1");
            assert_eq!((slot0, slot1), (0, 1));
            let mut rtok = ReaderToken::new(&readers[0]);
            let guard = rtok.enter();
            let reg_d = Arc::clone(&reg);
            let readers_d = Arc::clone(&readers);
            let drainer = thread::spawn(move || {
                reg_d.unregister_and_drain(slot1, Some(&readers_d[..]));
            });
            while !reg.is_draining(slot1) {
                thread::yield_now();
            }
            assert!(!reg.is_active(slot1), "draining slot must be cleared from active");
            assert!(reg.is_active(slot0), "sibling slot must remain active");
            assert!(!reg.is_registered(slot1) && !reg.is_empty(slot1));
            let c = Box::into_raw(Box::new(DummySource { waiting: AtomicUsize::new(3), regenerating: false, freed: AtomicBool::new(false) }));
            let other = unsafe { reg.register(c) }.expect("slot while 1 draining");
            assert_ne!(other, slot1, "draining slot must not be reused before drain completes");
            drop(guard);
            drainer.join().unwrap();
            assert!(reg.is_empty(slot1));
            assert!(reg.is_active(slot0));
            reg.unregister(other);
            reg.unregister(slot0);
            assert!(!reg.any_active());
            unsafe {
                drop(Box::from_raw(a));
                drop(Box::from_raw(b));
                drop(Box::from_raw(c));
            }
        },
        1,
    );
}
