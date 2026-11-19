//! A pool of one thread has no worker. A Pipeline there must still finish
//! under an executor that is not this crate's `block_on` (an audit of the
//! 0.10 delta found it waiting for ever: BLAKE3 #461).
use blake3_servil::Mode;
use blake3_servil::throughput::{Event, Limits, Pipeline, ROOM_LEN};
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Wake};

struct Count(AtomicUsize);
impl Wake for Count {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn an_outside_executor_finishes_on_a_one_thread_pool() {
    blake3_servil::initialize_multithreaded(std::num::NonZeroUsize::new(1));
    let woken = Arc::new(Count(AtomicUsize::new(0)));
    let waker = woken.clone().into();
    let mut cx = Context::from_waker(&waker);
    // A short message, and one of several rooms: the grouped path and the room pieces'.
    for len in [3000, 3 * ROOM_LEN + 111] {
        let data: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
        let mut pipe = Pipeline::new(Mode::Plain, Limits { messages: 2, rooms: 2 });
        let mut run = Box::pin(async {
            let mut m = pipe.message((), None).await;
            for part in data.chunks(ROOM_LEN) {
                let mut r = pipe.room().await;
                r.bytes_mut()[..part.len()].copy_from_slice(part);
                pipe.commit(&mut m, &mut r, part.len());
            }
            pipe.end(m);
            pipe.ready().await;
        });
        // No other thread hashes in a pool of one, so a poll is all the
        // progress there is: a count of polls is this test's clock.
        let mut polls = 0;
        while run.as_mut().poll(&mut cx).is_pending() {
            polls += 1;
            assert!(polls < 1000, "{len} bytes: still pending after {polls} polls and {} wakes", woken.0.load(Ordering::SeqCst));
        }
        drop(run);
        match pipe.next_event() {
            Some(Event::Hashed { hash, .. }) => assert_eq!(hash.as_bytes(), blake3::hash(&data).as_bytes(), "{len} bytes"),
            _ => panic!("{len} bytes: expected its hash"),
        }
    }
}
