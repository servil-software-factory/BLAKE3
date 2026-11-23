//! Pool, worker-wake, thread-count, CPU-affinity, and official-vector
//! integration tests for [`blake3_servil::throughput::Pipeline`].
//!
//! Under `cargo nextest`, each test runs in its own OS process so each can
//! configure `initialize_multithreaded` or CPU affinity independently.

#[path = "../../tools/require_nextest.rs"]
mod require_nextest;

use blake3_servil::throughput::{Batch, Event, Limits, Pipeline, ROOM_LEN, block_on};
use blake3_servil::{Mode, convenience};
use core::future::Future;
use core::num::NonZeroUsize;
use core::task::{Context, Poll, Waker};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::Wake;

const KEY: &[u8; 32] = b"whats the Elvish word for friend";
const CONTEXT: &str = "BLAKE3 2019-12-27 16:29:52 test vectors context";

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

#[path = "../../test_vectors/src/parse.rs"]
mod vectors;

fn official_vectors() -> Vec<(usize, [[u8; 32]; 3])> {
    let data = vectors::parse_json(include_str!("../../test_vectors/test_vectors.json"));
    let hex32 = |s: &str| -> [u8; 32] { vectors::hex_decode(&s[..64]).try_into().unwrap() };
    data.cases.iter().map(|c| (c.input_len, [hex32(&c.hash), hex32(&c.keyed_hash), hex32(&c.derive_key)])).collect()
}

#[test]
fn official_test_vectors_match_across_rooms_owned_and_lent() {
    blake3_servil::initialize_multithreaded(None);
    let cases = official_vectors();
    let max_len = cases.iter().map(|&(n, _)| n).max().unwrap_or(0);
    let data = pattern(max_len);

    for (mode_idx, mode) in [Mode::Plain, Mode::Keyed(KEY), Mode::DeriveKey(CONTEXT)].into_iter().enumerate() {
        let mut pipe = Pipeline::<usize, Vec<u8>>::with_owners(mode, Limits { messages: 8, rooms: 4 });
        let mut verified = 0usize;
        for (idx, &(len, _)) in cases.iter().enumerate() {
            while let Some(Event::Hashed { tag, hash, .. }) = pipe.next_event() {
                assert_eq!(*hash.as_bytes(), cases[tag].1[mode_idx], "len={}", cases[tag].0);
                verified += 1;
            }
            block_on(async {
                while idx - verified == 8 {
                    pipe.ready().await;
                    while let Some(Event::Hashed { tag, hash, .. }) = pipe.next_event() {
                        assert_eq!(*hash.as_bytes(), cases[tag].1[mode_idx], "len={}", cases[tag].0);
                        verified += 1;
                    }
                }
                match idx % 3 {
                    0 => {
                        let mut m = pipe.message(idx, Some(len as u64)).await;
                        for part in data[..len].chunks(ROOM_LEN) {
                            let mut r = pipe.room().await;
                            r.bytes_mut()[..part.len()].copy_from_slice(part);
                            pipe.commit(&mut m, &mut r, part.len());
                        }
                        pipe.end(m);
                    }
                    1 => {
                        let m = pipe.message(idx, Some(len as u64)).await;
                        pipe.owned(m, data[..len].to_vec());
                    }
                    _ => {
                        let m = pipe.message(idx, Some(len as u64)).await;
                        let guard = unsafe { pipe.lent(m, &data[..len]) };
                        while verified <= idx {
                            if let Some(Event::Hashed { tag, hash, .. }) = pipe.next_event() {
                                assert_eq!(*hash.as_bytes(), cases[tag].1[mode_idx], "len={}", cases[tag].0);
                                verified += 1;
                            } else {
                                pipe.ready().await;
                            }
                        }
                        drop(guard);
                    }
                }
            });
        }
        block_on(async {
            while verified < cases.len() {
                if let Some(Event::Hashed { tag, hash, .. }) = pipe.next_event() {
                    assert_eq!(*hash.as_bytes(), cases[tag].1[mode_idx], "len={}", cases[tag].0);
                    verified += 1;
                } else {
                    pipe.ready().await;
                }
            }
        });
    }
}

#[test]
fn pipelines_on_several_threads_keep_their_own_order() {
    blake3_servil::initialize_multithreaded(None);
    let data = Arc::new(pattern(300_000));
    let handles: Vec<_> = (0..4usize)
        .map(|tid| {
            let data = Arc::clone(&data);
            std::thread::spawn(move || {
                let mut pipe = Pipeline::new(Mode::Plain, Limits { messages: 16, rooms: 4 });
                let lens: Vec<usize> = (0..80).map(|i| (i * 997 + tid * 131) % data.len()).collect();
                let mut got = Vec::with_capacity(lens.len());
                block_on(async {
                    for (idx, &len) in lens.iter().enumerate() {
                        while let Some(Event::Hashed { tag, hash, .. }) = pipe.next_event() {
                            got.push((tag, hash));
                        }
                        if idx - got.len() == 16 {
                            pipe.ready().await;
                            while let Some(Event::Hashed { tag, hash, .. }) = pipe.next_event() {
                                got.push((tag, hash));
                            }
                        }
                        let mut m = pipe.message(idx, Some(len as u64)).await;
                        for part in data[..len].chunks(ROOM_LEN) {
                            let mut r = pipe.room().await;
                            r.bytes_mut()[..part.len()].copy_from_slice(part);
                            pipe.commit(&mut m, &mut r, part.len());
                        }
                        pipe.end(m);
                    }
                    while got.len() < lens.len() {
                        if let Some(Event::Hashed { tag, hash, .. }) = pipe.next_event() {
                            got.push((tag, hash));
                        } else {
                            pipe.ready().await;
                        }
                    }
                });
                for (expected_idx, (tag, hash)) in got.into_iter().enumerate() {
                    assert_eq!(tag, expected_idx, "thread {tid} order");
                    assert_eq!(hash, convenience::hash(&data[..lens[tag]]));
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
}

fn run_saturation_rounds(pipe: &mut Pipeline<usize>, data: &[u8]) {
    let lens = [0, 64, 1500, ROOM_LEN, ROOM_LEN + 500, 3 * ROOM_LEN];
    let total = 48usize;
    let mut got = 0usize;
    block_on(async {
        for i in 0..total {
            while let Some(Event::Hashed { tag, hash, .. }) = pipe.next_event() {
                assert_eq!(hash, convenience::hash(&data[..lens[tag % lens.len()]]));
                got += 1;
            }
            if i - got == 4 {
                pipe.ready().await;
                while let Some(Event::Hashed { tag, hash, .. }) = pipe.next_event() {
                    assert_eq!(hash, convenience::hash(&data[..lens[tag % lens.len()]]));
                    got += 1;
                }
            }
            let len = lens[i % lens.len()];
            let mut m = pipe.message(i, Some(len as u64)).await;
            for part in data[..len].chunks(ROOM_LEN) {
                let mut r = pipe.room().await;
                r.bytes_mut()[..part.len()].copy_from_slice(part);
                pipe.commit(&mut m, &mut r, part.len());
            }
            pipe.end(m);
        }
        while got < total {
            if let Some(Event::Hashed { tag, hash, .. }) = pipe.next_event() {
                assert_eq!(hash, convenience::hash(&data[..lens[tag % lens.len()]]));
                got += 1;
            } else {
                pipe.ready().await;
            }
        }
    });
}

#[test]
fn one_thread_pool_makes_progress_without_workers() {
    blake3_servil::initialize_multithreaded(Some(NonZeroUsize::new(1).unwrap()));
    let data = pattern(3 * ROOM_LEN);
    let mut pipe = Pipeline::new(Mode::Plain, Limits { messages: 4, rooms: 2 });
    run_saturation_rounds(&mut pipe, &data);
}

#[test]
fn two_thread_pool_makes_progress_with_one_worker() {
    blake3_servil::initialize_multithreaded(Some(NonZeroUsize::new(2).unwrap()));
    let data = pattern(3 * ROOM_LEN);
    let mut pipe = Pipeline::new(Mode::Plain, Limits { messages: 4, rooms: 2 });
    run_saturation_rounds(&mut pipe, &data);
}

struct FlagWake {
    thread: std::thread::Thread,
    woken: AtomicBool,
}

impl Wake for FlagWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.woken.store(true, Ordering::SeqCst);
        self.thread.unpark();
    }
}

/// Drive a future with an external async `Waker` (outside `block_on`, so the
/// calling thread never steals its own actions and ONLY a worker thread can
/// complete the work and fire the waker!).
fn drive_external<F: Future>(future: F) -> F::Output {
    let state = Arc::new(FlagWake { thread: std::thread::current(), woken: AtomicBool::new(false) });
    let waker = Waker::from(Arc::clone(&state));
    let mut cx = Context::from_waker(&waker);
    let mut future = core::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(out) => return out,
            Poll::Pending => {
                while !state.woken.swap(false, Ordering::SeqCst) {
                    std::thread::park();
                }
            }
        }
    }
}

#[test]
fn external_async_waiter_wakes_a_parked_worker_for_a_lone_task() {
    blake3_servil::initialize_multithreaded(Some(NonZeroUsize::new(4).unwrap()));
    if std::thread::available_parallelism().map_or(1, |n| n.get()) < 2 {
        return;
    }
    let data = vec![7u8; 3000];
    let want = convenience::hash(&data);
    let mut pipe = Pipeline::new(Mode::Plain, Limits { messages: 2, rooms: 2 });

    for _ in 0..20 {
        while !blake3_servil::__all_workers_parked() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        drive_external(async {
            let mut m = pipe.message((), None).await;
            let mut r = pipe.room().await;
            r.bytes_mut()[..data.len()].copy_from_slice(&data);
            pipe.commit(&mut m, &mut r, data.len());
            drop(r);
            pipe.end(m);
            pipe.ready().await;
        });
        match pipe.next_event() {
            Some(Event::Hashed { hash, .. }) => assert_eq!(hash, want),
            _ => panic!("expected Hashed"),
        }
    }
}

#[test]
fn block_on_finishes_lone_tasks_and_large_batches_after_pauses() {
    blake3_servil::initialize_multithreaded(Some(NonZeroUsize::new(4).unwrap()));
    let data = vec![9u8; 3000];
    let want = convenience::hash(&data);
    let mut pipe = Pipeline::new(Mode::Plain, Limits { messages: 8, rooms: 2 });

    for _ in 0..20 {
        while !blake3_servil::__all_workers_parked() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        block_on(async {
            let mut m = pipe.message(0usize, None).await;
            let mut r = pipe.room().await;
            r.bytes_mut()[..data.len()].copy_from_slice(&data);
            pipe.commit(&mut m, &mut r, data.len());
            drop(r);
            pipe.end(m);

            let mut batch = Batch::new(64, 2048);
            batch.set(0, &[0xAB; 64]);
            let digests = vec![[0u8; 32]; 2048].into_boxed_slice();
            let bm = pipe.message(1usize, None).await;
            pipe.batch(bm, batch, digests);

            let mut seen = 0;
            while seen < 2 {
                if let Some(ev) = pipe.next_event() {
                    seen += 1;
                    match ev {
                        Event::Hashed { hash, .. } => assert_eq!(hash, want),
                        Event::Batch { digests, .. } => assert_eq!(digests[0], *convenience::hash(&[0xAB; 64]).as_bytes()),
                        _ => panic!("unexpected event"),
                    }
                } else {
                    pipe.ready().await;
                }
            }
        });
    }
}

#[cfg(target_os = "linux")]
#[test]
fn pipeline_completes_every_path_when_pinned_to_one_cpu() {
    unsafe extern "C" {
        fn sched_setaffinity(pid: i32, cpusetsize: usize, mask: *const core::ffi::c_ulong) -> i32;
    }
    let mut mask = [0 as core::ffi::c_ulong; 16];
    mask[0] = 1;
    assert_eq!(unsafe { sched_setaffinity(0, core::mem::size_of_val(&mask), mask.as_ptr()) }, 0);

    blake3_servil::initialize_multithreaded(None);
    let data = pattern(3 * ROOM_LEN + 111);
    let mut pipe = Pipeline::<usize, Vec<u8>>::with_owners(Mode::Plain, Limits { messages: 4, rooms: 2 });
    block_on(async {
        let mut m0 = pipe.message(0, None).await;
        for part in data.chunks(ROOM_LEN) {
            let mut r = pipe.room().await;
            r.bytes_mut()[..part.len()].copy_from_slice(part);
            pipe.commit(&mut m0, &mut r, part.len());
        }
        pipe.end(m0);

        let m1 = pipe.message(1, None).await;
        pipe.owned(m1, data[..5000].to_vec());

        let m2 = pipe.message(2, None).await;
        let guard = unsafe { pipe.lent(m2, &data[..2 * ROOM_LEN]) };

        let mut batch = Batch::new(64, 128);
        batch.set(3, &[0x42; 64]);
        let digests = vec![[0u8; 32]; 128].into_boxed_slice();
        let m3 = pipe.message(3, None).await;
        pipe.batch(m3, batch, digests);

        let mut got = 0;
        while got < 4 {
            if let Some(ev) = pipe.next_event() {
                got += 1;
                match ev {
                    Event::Hashed { tag: 0, hash, .. } => assert_eq!(hash, convenience::hash(&data)),
                    Event::Hashed { tag: 1, hash, .. } => assert_eq!(hash, convenience::hash(&data[..5000])),
                    Event::Hashed { tag: 2, hash, .. } => assert_eq!(hash, convenience::hash(&data[..2 * ROOM_LEN])),
                    Event::Batch { tag: 3, digests, .. } => assert_eq!(digests[3], *convenience::hash(&[0x42; 64]).as_bytes()),
                    _ => panic!("unexpected event"),
                }
            } else {
                pipe.ready().await;
            }
        }
        drop(guard);
    });
}
