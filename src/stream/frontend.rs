//! The pipeline over the receiver: messages and batches submitted by one
//! producer, completions delivered to its handler on its own thread from
//! `collect` and `finish`, and `ready` to wait for any of them. Public as
//! [`crate::throughput::pipeline`].

pub use super::receiver::Input;
use super::receiver::{self, BatchCompletion, Completion, Receiver};
use crate::{Hash, Mode, OUT_LEN, throughput::Batch};
use core::cell::RefCell;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use std::sync::{Arc, Mutex};

/// What a pipeline holds at most, and so its memory: `buffers` input rooms
/// of 16 KiB each (one holds a message's bytes from `input` until a worker
/// has hashed them) and `messages` begun and not yet collected. A request
/// beyond either waits, in the order asked, until a completion is
/// collected or a worker frees a room.
pub struct Limits {
    /// Input rooms of 16 KiB: the bytes in flight.
    pub buffers: usize,
    /// Messages begun and not yet collected.
    pub messages: usize,
}

/// A completion, delivered to the pipeline's handler with the tag its work
/// was given.
pub enum Event<'a, T> {
    /// The message ended; its digest.
    Hashed {
        /// The message's tag.
        tag: T,
        /// Its digest.
        hash: Hash,
    },
    /// The message was aborted: no digest.
    Aborted {
        /// The message's tag.
        tag: T,
    },
    /// Every digest of the batch is in its places, which come back with
    /// the batch for the producer to refill and submit again.
    Batch {
        /// The batch's tag.
        tag: T,
        /// The batch submitted, the producer's again to refill.
        batch: Batch<'a>,
        /// Its digests, one per message of the batch.
        places: &'a mut [[u8; OUT_LEN]],
    },
}

/// A completion the producer has yet to collect, kept in place in its
/// entry of the stream's `pending` (see there) and polled there. Polled once
/// as it arrives, so its waker is registered before `ready` can wait on it.
enum Pending<'a, T> {
    Message(T, Completion),
    /// The batch and places, held until the completion returns them.
    Batch(T, BatchCompletion, Option<Batch<'a>>, &'a mut [[u8; OUT_LEN]]),
}

/// Completions wake the producer through this; `ready` waits on it.
#[derive(Default)]
struct Notify {
    flagged: bool,
    waker: Option<Waker>,
}
struct NotifyWake(Arc<Mutex<Notify>>);
impl std::task::Wake for NotifyWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        let waker = {
            let mut notify = self.0.lock().unwrap();
            notify.flagged = true;
            notify.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

/// One producer's messages and batches, hashed by the pool's workers (and
/// the GPU where it is efficient) while the producer goes on; completions
/// reach its handler on the producer's own thread, from [`collect`],
/// [`finish`], and a wait for admission.
///
/// Crash-only: a [`Message`] is ended or aborted, never dropped; a
/// pipeline is finished, never dropped, once it has accepted work. A
/// pending `message` or `submit_batch` future holds nothing and drops as
/// any future does.
///
/// [`collect`]: Stream::collect
/// [`finish`]: Stream::finish
pub struct Stream<'a, T, F: FnMut(Event<'a, T>)> {
    receiver: Receiver,
    on_completion: RefCell<F>,
    /// The uncollected completions, one entry per message slot: each holds
    /// its slot until collected, so `limits.messages` entries hold them all,
    /// made with the stream. A completion stays pinned in its entry: the
    /// entries never move (the slice is never resized), an occupied entry
    /// is never moved, and a completion is dropped where it lies. So a
    /// message's or batch's cycle allocates nothing (#325).
    pending: RefCell<Box<[Option<Pending<'a, T>>]>>,
    notify: Arc<Mutex<Notify>>,
    completion_waker: Waker,
}

/// A message begun on a pipeline: fed through [`input`](Message::input),
/// then [`end`](Message::end)ed or [`abort`](Message::abort)ed.
pub struct Message<'s, 'a, T, F: FnMut(Event<'a, T>)> {
    stream: &'s Stream<'a, T, F>,
    pub(super) inner: receiver::Message,
    tag: T,
}

impl<'a, T: Copy, F: FnMut(Event<'a, T>)> Stream<'a, T, F> {
    /// A pipeline hashing in `mode`, within `limits`, delivering each
    /// completion to `on_completion`.
    pub fn new(mode: Mode<'_>, limits: Limits, on_completion: F) -> Self {
        let notify = Arc::new(Mutex::new(Notify::default()));
        Self {
            receiver: Receiver::new(mode, limits.buffers, limits.messages),
            on_completion: RefCell::new(on_completion),
            pending: RefCell::new((0..limits.messages).map(|_| None).collect()),
            completion_waker: Waker::from(Arc::new(NotifyWake(notify.clone()))),
            notify,
        }
    }

    /// Begin a message once a message slot is reserved, delivering
    /// completions that are ready while it waits (a slot returns through
    /// them). `length_hint`, where the producer knows the message's
    /// length, helps scheduling and never affects the digest.
    pub async fn message(&self, tag: T, length_hint: Option<u64>) -> Message<'_, 'a, T, F> {
        Message { stream: self, inner: self.admit(self.receiver.message(length_hint)).await, tag }
    }

    /// Submit a prepared batch once admitted (delivering completions as
    /// `message` does while it waits); its digests land in `places`, one
    /// per message of the batch, and both come back with its
    /// [`Event::Batch`].
    pub async fn submit_batch(&self, batch: Batch<'a>, places: &'a mut [[u8; OUT_LEN]], tag: T) {
        assert_eq!(batch.len(), places.len(), "one place per batch message");
        // Sound: the batch and `places` are held here, untouched, until the
        // completion resolves, then handed back with the event.
        let completion = self.admit(unsafe { self.receiver.submit_batch(&batch, places.as_mut_ptr()) }).await;
        self.push(Pending::Batch(tag, completion, Some(batch), places));
    }

    /// Await an admission, collecting while it is pending: a slot held by an
    /// uncollected completion comes back through `collect`, so a wait that
    /// collected nothing would otherwise never end; and hashing a waiting
    /// task itself where no worker takes it (one CPU). Wakes on the
    /// admission itself and on any completion.
    async fn admit<R>(&self, admission: impl Future<Output = R>) -> R {
        let mut admission = core::pin::pin!(admission);
        core::future::poll_fn(|cx| {
            loop {
                if let Poll::Ready(r) = admission.as_mut().poll(cx) {
                    return Poll::Ready(r);
                }
                if self.collect() == 0 && !self.receiver.help() {
                    break;
                }
                // Something was delivered or hashed: poll again.
            }
            let mut notify = self.notify.lock().unwrap();
            notify.flagged = false;
            notify.waker = Some(cx.waker().clone());
            Poll::Pending
        })
        .await
    }

    /// Deliver every completion that is ready, without waiting: how many.
    pub fn collect(&self) -> usize {
        let mut delivered = 0;
        let entries = self.pending.borrow().len();
        for i in 0..entries {
            // The handler runs with no borrow held: it may submit.
            if let Some(event) = self.poll_entry(i) {
                (self.on_completion.borrow_mut())(event);
                delivered += 1;
            }
        }
        delivered
    }

    /// One poll of entry `i`'s completion, where it lies: its event once
    /// ready, the completion then dropped in place and the entry free.
    fn poll_entry(&self, i: usize) -> Option<Event<'a, T>> {
        let mut pending = self.pending.borrow_mut();
        let entry = &mut pending[i];
        let event = self.poll_pending(entry.as_mut()?)?;
        *entry = None; // Drops the finished completion in place.
        Some(event)
    }

    /// One poll of a pending completion with the stream's waker: its event
    /// once ready, its slot then retired.
    fn poll_pending(&self, item: &mut Pending<'a, T>) -> Option<Event<'a, T>> {
        let mut cx = Context::from_waker(&self.completion_waker);
        match item {
            // Sound: the completion lies in its entry of `pending`, which
            // never moves it (see there).
            Pending::Message(tag, completion) => match unsafe { Pin::new_unchecked(completion) }.poll(&mut cx) {
                Poll::Ready(finished) => {
                    let outcome = finished.outcome().copied();
                    finished.retire();
                    Some(match outcome {
                        Some(hash) => Event::Hashed { tag: *tag, hash },
                        None => Event::Aborted { tag: *tag },
                    })
                }
                Poll::Pending => None,
            },
            Pending::Batch(tag, completion, batch, places) => match unsafe { Pin::new_unchecked(completion) }.poll(&mut cx) {
                Poll::Ready(finished) => {
                    finished.retire();
                    Some(Event::Batch { tag: *tag, batch: batch.take().expect("a batch completes once"), places: core::mem::take(places) })
                }
                Poll::Pending => None,
            },
        }
    }

    /// Keep a completion for `collect` in a free entry, polled once there
    /// so that its waker is registered; one already ready is delivered at
    /// once. Its message slot guarantees a free entry.
    fn push(&self, item: Pending<'a, T>) {
        let i = {
            let mut pending = self.pending.borrow_mut();
            let i = pending.iter().position(Option::is_none).expect("a completion holds a message slot, and there is an entry for each");
            pending[i] = Some(item);
            i
        };
        if let Some(event) = self.poll_entry(i) {
            (self.on_completion.borrow_mut())(event);
        }
    }

    /// Wait until some completion may be ready (then [`collect`](Stream::collect)).
    pub async fn ready(&self) {
        core::future::poll_fn(|cx| {
            loop {
                {
                    let mut notify = self.notify.lock().unwrap();
                    if core::mem::take(&mut notify.flagged) {
                        return Poll::Ready(());
                    }
                    notify.waker = Some(cx.waker().clone());
                }
                // The producer's own help where no worker takes a task (one
                // CPU, or every worker parked): hash one, then look again.
                if !self.receiver.help() {
                    return Poll::Pending;
                }
            }
        })
        .await
    }

    /// Wait for every accepted piece of work, deliver every completion, and
    /// close. Every message has been ended or aborted.
    pub fn finish(self) {
        self.receiver.drain();
        while self.pending.borrow().iter().any(Option::is_some) {
            assert!(self.collect() > 0, "after the drain, every pending completion is ready");
        }
        self.receiver.close();
    }
}

impl<'a, T: Copy, F: FnMut(Event<'a, T>)> Message<'_, 'a, T, F> {
    /// Room for the message's next bytes, once a buffer is reserved: write
    /// into [`Input::bytes_mut`], then [`Input::commit`] how many.
    pub async fn input(&mut self) -> Input<'_> {
        let receiver = &self.stream.receiver;
        let mut input = core::pin::pin!(self.inner.input());
        core::future::poll_fn(|cx| {
            loop {
                if let Poll::Ready(input) = input.as_mut().poll(cx) {
                    return Poll::Ready(input);
                }
                // A room comes back when a worker has hashed it; with none to
                // (one CPU), the producer hashes one itself.
                if !receiver.help() {
                    return Poll::Pending;
                }
            }
        })
        .await
    }
    /// The message is complete, its last `written` bytes at the start of
    /// the current room's free part (0 when none are new): the digest comes
    /// as [`Event::Hashed`]. Where the producer learns the bytes and the
    /// end together, `end(n)` publishes them as the last part at once.
    pub fn end(self, written: usize) {
        self.stream.push(Pending::Message(self.tag, self.inner.end(written)));
    }
    /// Give the message up: no digest, [`Event::Aborted`] instead.
    pub fn abort(self) {
        self.stream.push(Pending::Message(self.tag, self.inner.abort()));
    }
}

#[cfg(test)]
mod tests {
    use super::super::receiver::tests::{expected, run};
    use super::*;
    use crate::throughput::Block;
    use std::cell::RefCell;

    fn stream<'a, T: Copy>(buffers: usize, messages: usize, events: &'a RefCell<Vec<Event<'a, T>>>) -> Stream<'a, T, impl FnMut(Event<'a, T>) + 'a> {
        crate::initialize_multithreaded(core::num::NonZeroUsize::new(3));
        Stream::new(Mode::Plain, Limits { buffers, messages }, move |event| events.borrow_mut().push(event))
    }

    /// A reader standing in for a socket, as the sketch's `read_fragment`.
    struct Reader<'a> {
        input: &'a [u8],
        at: usize,
        turn: usize,
        fail_at: Option<usize>,
    }
    impl Reader<'_> {
        fn read_fragment(&mut self, room: &mut [u8]) -> Result<(usize, bool), &'static str> {
            if self.fail_at == Some(self.turn) {
                return Err("connection reset");
            }
            let wanted = [1448, 4096, 16 * 1024, 64 * 1024][self.turn % 4];
            self.turn += 1;
            let n = wanted.min(room.len()).min(self.input.len() - self.at);
            room[..n].copy_from_slice(&self.input[self.at..self.at + n]);
            self.at += n;
            Ok((n, self.at == self.input.len()))
        }
    }

    /// The sketch's loop, verbatim but for `run` in place of `.await`.
    fn receive<'s, 'a, T: Copy, F: FnMut(Event<'a, T>)>(stream: &'s Stream<'a, T, F>, tag: T, reader: &mut Reader<'_>) -> Result<(), &'static str> {
        let mut message = run(stream.message(tag, Some(reader.input.len() as u64)));
        loop {
            let mut input = run(message.input());
            match reader.read_fragment(input.bytes_mut()) {
                Ok((written, false)) => input.commit(written),
                Ok((written, true)) => {
                    // The read learned the bytes and the end together: the
                    // last part is published as the last at once.
                    drop(input);
                    message.end(written);
                    return Ok(());
                }
                Err(error) => {
                    message.abort();
                    return Err(error);
                }
            }
        }
    }

    #[test]
    fn messages_batches_and_an_abort_reach_the_handler() {
        // A batch and its places outlive the stream that borrows them: the
        // borrow checker holds the producer to it (declared first).
        let mut batch = Batch::new(100, 20);
        for i in 0..batch.len() {
            batch.set(i, &[i as u8; 100]);
        }
        let mut places = vec![[0u8; OUT_LEN]; batch.len()];
        let events = RefCell::new(Vec::new());
        let stream = stream(4, 4, &events);
        let mut input = vec![0u8; (1 << 20) + 777];
        crate::test::paint_test_input(&mut input);
        assert_eq!(receive(&stream, "long", &mut Reader { input: &input, at: 0, turn: 0, fail_at: None }), Ok(()));
        run(stream.submit_batch(batch, &mut places, "batch"));
        assert_eq!(receive(&stream, "reset", &mut Reader { input: &input, at: 0, turn: 0, fail_at: Some(5) }), Err("connection reset"));
        assert_eq!(receive(&stream, "short", &mut Reader { input: &input[..1000], at: 0, turn: 0, fail_at: None }), Ok(()));
        // The producer waits for completions as an event loop would.
        while events.borrow().len() < 4 {
            run(stream.ready());
            stream.collect();
        }
        stream.finish();
        let mut got: Vec<String> = events
            .borrow()
            .iter()
            .map(|e| match e {
                Event::Hashed { tag, hash } => format!("{tag}: {}", hex::encode(hash.as_bytes())),
                Event::Aborted { tag } => format!("{tag}: aborted"),
                Event::Batch { tag, .. } => format!("{tag}: batch"),
            })
            .collect();
        got.sort();
        let mut want = vec![
            format!("long: {}", hex::encode(expected(Mode::Plain, &input))),
            format!("short: {}", hex::encode(expected(Mode::Plain, &input[..1000]))),
            "reset: aborted".to_string(),
            "batch: batch".to_string(),
        ];
        want.sort();
        assert_eq!(got, want);
        for (i, place) in places.iter().enumerate() {
            assert_eq!(place, &expected(Mode::Plain, &[i as u8; 100]), "batch message {i}");
        }
    }

    /// A later short message is delivered while an earlier one is held.
    #[test]
    fn a_later_message_is_delivered_before_an_earlier_held_one() {
        let events = RefCell::new(Vec::new());
        let stream = stream(3, 2, &events);
        let mut long = vec![0u8; crate::incremental::STAGE_LEN];
        crate::test::paint_test_input(&mut long);
        let gate = super::super::receiver::Gate::new();
        let mut first = run(stream.message(1u8, None));
        first.inner.hold_next(gate.clone());
        {
            let mut room = run(first.input());
            room.bytes_mut().copy_from_slice(&long);
            room.commit(long.len());
        }
        gate.started.wait();
        let mut second = run(stream.message(2u8, Some(3)));
        {
            let mut room = run(second.input());
            room.bytes_mut()[..3].copy_from_slice(b"abc");
            room.commit(3);
        }
        second.end(0);
        // The completion may be delivered by end() itself when the worker
        // is quick; otherwise ready() waits for it.
        while events.borrow().is_empty() {
            run(stream.ready());
            stream.collect();
        }
        assert!(matches!(events.borrow()[0], Event::Hashed { tag: 2, .. }), "message 2 delivered while 1 is held");
        gate.release.wait();
        first.end(0);
        stream.finish();
        assert!(matches!(events.borrow()[1], Event::Hashed { tag: 1, hash } if hash.as_bytes() == &expected(Mode::Plain, &long)));
    }

    /// Many messages interleaved at random, in fragments of random lengths,
    /// over a tight budget: every digest matches, every slot and buffer
    /// comes back, with a fixed seed so a failure repeats.
    #[test]
    fn interleaved_messages_in_random_fragments_match_official() {
        use rand::{RngExt, SeedableRng};
        let events = RefCell::new(Vec::new());
        let stream = stream(3, 2, &events);
        let mut rng = rand::rngs::StdRng::seed_from_u64(0x5eed_b3);
        let mut inputs: Vec<Vec<u8>> = (0..24)
            .map(|i| {
                let len = [0, 1, 63, 64, 65, 1000, 16384, 16385, 40000, 100_000][i % 10] + (i / 10) * 7;
                let mut v = vec![0u8; len];
                crate::test::paint_test_input(&mut v);
                v
            })
            .collect();
        let mut open: Vec<(usize, Message<'_, '_, usize, _>, usize)> = Vec::new();
        let mut next = 0;
        while next < inputs.len() || !open.is_empty() {
            if open.len() < 2 && next < inputs.len() {
                open.push((next, run(stream.message(next, Some(inputs[next].len() as u64))), 0));
                next += 1;
                continue;
            }
            let k = rng.random_range(0..open.len());
            let (i, message, at) = &mut open[k];
            let input = &inputs[*i];
            if *at == input.len() {
                let (i, message, _) = open.swap_remove(k);
                let _ = i;
                message.end(0);
                stream.collect();
                continue;
            }
            let mut room = run(message.input());
            let n = rng.random_range(1..=room.bytes_mut().len().min(input.len() - *at).max(1));
            room.bytes_mut()[..n].copy_from_slice(&input[*at..*at + n]);
            room.commit(n);
            *at += n;
        }
        stream.finish();
        let mut got: Vec<(usize, [u8; 32])> = events
            .borrow()
            .iter()
            .map(|e| match e {
                Event::Hashed { tag, hash } => (*tag, *hash.as_bytes()),
                _ => panic!("every message hashes"),
            })
            .collect();
        got.sort();
        assert_eq!(got.len(), inputs.len());
        for (tag, hash) in got {
            assert_eq!(hash, expected(Mode::Plain, &inputs[tag]), "message {tag}");
        }
        inputs.clear();
    }

    /// Diagnostic (#272): which threads hash a long message's parts, and
    /// how fast the pipeline takes 64 MiB in 16 KiB rooms.
    #[test]
    fn parts_of_a_long_message_spread_over_the_workers() {
        // The whole machine's pool (the other tests' three threads hide
        // the cost of idle workers).
        crate::initialize_multithreaded(None);
        let events = RefCell::new(Vec::new());
        let stream = Stream::new(Mode::Plain, Limits { buffers: 64, messages: 2 }, |event| events.borrow_mut().push(event));
        let piece = vec![7u8; 16384];
        super::super::receiver::EXECUTED_BY.lock().unwrap().clear();
        let started = std::time::Instant::now();
        let mut message = run(stream.message(0usize, None));
        for _ in 0..4096 {
            let mut room = run(message.input());
            room.bytes_mut().copy_from_slice(&piece);
            room.commit(16384);
        }
        message.end(0);
        while events.borrow().is_empty() {
            run(stream.ready());
            stream.collect();
        }
        events.borrow_mut().clear();
        let took = started.elapsed();
        // The benchmark's receive trace: a 16 MiB source in memory, fragments
        // of 1448, 4096, 16384 and 65536 bytes in a cycle, each written room
        // by room.
        let source = vec![3u8; 16 << 20];
        for (label, fragments) in [("receive trace", &[1448usize, 4096, 16384, 65536][..]), ("64 KiB fragments", &[65536][..])] {
            let t = std::time::Instant::now();
            let mut message = run(stream.message(1usize, Some(source.len() as u64)));
            let mut at = 0;
            let mut k = 0;
            while at < source.len() {
                let len = fragments[k % fragments.len()].min(source.len() - at);
                k += 1;
                let mut bytes = &source[at..at + len];
                while !bytes.is_empty() {
                    let mut room = run(message.input());
                    let n = room.bytes_mut().len().min(bytes.len());
                    room.bytes_mut()[..n].copy_from_slice(&bytes[..n]);
                    room.commit(n);
                    bytes = &bytes[n..];
                }
                at += len;
            }
            message.end(0);
            while events.borrow().is_empty() {
                run(stream.ready());
                stream.collect();
            }
            events.borrow_mut().clear();
            eprintln!("{label}: 16 MiB in {:?} ({:.3} ns/B)", t.elapsed(), t.elapsed().as_nanos() as f64 / (16.0 * 1048576.0));
        }
        let t = std::time::Instant::now();
        let mut hasher = crate::Hasher::new(Mode::Plain, None);
        for chunk in source.chunks(65536) {
            hasher.update(chunk);
        }
        core::hint::black_box(hasher.finalize());
        eprintln!("Hasher, 64 KiB updates: 16 MiB in {:?} ({:.3} ns/B)", t.elapsed(), t.elapsed().as_nanos() as f64 / (16.0 * 1048576.0));
        stream.finish();
        let by = super::super::receiver::EXECUTED_BY.lock().unwrap().clone();
        let me = std::thread::current().id();
        eprintln!(
            "64 MiB in {took:?} ({:.3} ns/B); parts by thread: {:?}; producer among them: {}",
            took.as_nanos() as f64 / (64.0 * 1048576.0),
            by.iter().map(|(_, n)| *n).collect::<Vec<_>>(),
            by.iter().any(|(t, _)| *t == me)
        );
        // On one CPU the pool has no workers: the producer hashes every
        // part itself, its own help while it waits.
        let workers = crate::lanes::worker_count();
        assert!(
            by.len() > 1 || (workers == 0 && by.iter().any(|(t, _)| *t == me)),
            "more than one thread hashed parts (or the producer alone, with {workers} workers): {by:?}"
        );
    }

    /// `end(n)` carries a message's last bytes: at a room boundary, where
    /// the room would otherwise go out as "more follows", in a partial
    /// room, and with none (`end(0)` after a read that met the end).
    #[test]
    fn end_carries_the_last_bytes() {
        let events = RefCell::new(Vec::new());
        let stream = stream(2, 1, &events);
        for (len, tail) in [(16384, 16384), (32768, 16384), (100, 100), (16384 + 5, 5), (16384, 0), (0, 0)] {
            let mut input = vec![0u8; len];
            crate::test::paint_test_input(&mut input);
            let mut message = run(stream.message(len, None));
            let (head, last) = input.split_at(len - tail);
            for piece in head.chunks(16384) {
                let mut room = run(message.input());
                room.bytes_mut()[..piece.len()].copy_from_slice(piece);
                room.commit(piece.len());
            }
            if tail > 0 {
                let mut room = run(message.input());
                room.bytes_mut()[..tail].copy_from_slice(last);
            }
            message.end(tail);
            stream.collect();
        }
        stream.finish();
        let events = events.borrow();
        assert_eq!(events.len(), 6);
        for event in events.iter() {
            let Event::Hashed { tag: len, hash } = event else { panic!("hashed") };
            let mut input = vec![0u8; *len];
            crate::test::paint_test_input(&mut input);
            assert_eq!(*hash.as_bytes(), expected(Mode::Plain, &input), "{len} bytes");
        }
    }

    /// Sam's acceptance example (#275): two batches and their places cycle
    /// through one pipeline (filled, submitted, completed, refilled,
    /// submitted again), the producer regaining each through its event,
    /// with no drain between cycles.
    #[test]
    fn batches_and_places_come_back_to_be_refilled() {
        let mut places = [vec![[0u8; OUT_LEN]; 32], vec![[0u8; OUT_LEN]; 32]];
        let returned: RefCell<Vec<(u8, Batch<'_>, &mut [[u8; OUT_LEN]])>> = RefCell::new(Vec::new());
        let stream = Stream::new(Mode::Plain, Limits { buffers: 2, messages: 2 }, |event| match event {
            Event::Batch { tag, batch, places } => returned.borrow_mut().push((tag, batch, places)),
            _ => unreachable!(),
        });
        let fill = |batch: &mut Batch<'_>, round: u8| {
            for i in 0..batch.len() {
                batch.set(i, &[round.wrapping_mul(7).wrapping_add(i as u8); 64]);
            }
        };
        let [pa, pb] = &mut places;
        let (mut a, mut b) = (Batch::new(64, 32), Batch::new(64, 32));
        fill(&mut a, 0);
        fill(&mut b, 1);
        run(stream.submit_batch(a, pa, 0));
        run(stream.submit_batch(b, pb, 1));
        // Each batch comes back with its places and goes round again, six
        // times; the places hold the digests of the round just hashed.
        let mut rounds = [0u8; 2];
        for _ in 0..12 {
            while returned.borrow().is_empty() {
                run(stream.ready());
                stream.collect();
            }
            let (tag, mut batch, places) = returned.borrow_mut().pop().unwrap();
            let round = rounds[tag as usize] * 2 + tag;
            for (i, place) in places.iter().enumerate() {
                assert_eq!(*place, expected(Mode::Plain, &[round.wrapping_mul(7).wrapping_add(i as u8); 64]), "round {round}, message {i}");
            }
            rounds[tag as usize] += 1;
            fill(&mut batch, rounds[tag as usize] * 2 + tag);
            run(stream.submit_batch(batch, places, tag));
        }
        stream.finish();
        assert_eq!(returned.borrow().len(), 2, "the last round of each came back at the finish");
    }

    /// `collect` says how many completions it delivered: each ready one
    /// once, then none. A pool of one thread has no worker, so no part is
    /// hashed until the producer helps: neither completion is ready as it
    /// is pushed, and the drain hashes both.
    #[test]
    fn collect_counts_the_completions_it_delivers() {
        crate::initialize_multithreaded(core::num::NonZeroUsize::new(1));
        let events = RefCell::new(Vec::new());
        let stream = Stream::new(Mode::Plain, Limits { buffers: 2, messages: 2 }, |event| events.borrow_mut().push(event));
        for tag in 0..2u8 {
            let mut message = run(stream.message(tag, None));
            let mut room = run(message.input());
            room.bytes_mut()[..64].fill(tag);
            room.commit(64);
            message.end(0);
        }
        assert!(events.borrow().is_empty(), "no worker has hashed either");
        stream.receiver.drain();
        assert_eq!(stream.collect(), 2, "the ready completions, each once");
        assert_eq!(stream.collect(), 0, "none twice");
        assert_eq!(events.borrow().len(), 2);
        stream.finish();
    }

    /// Whole-block storage is borrowed as a batch without a copy.
    #[test]
    fn a_borrowed_batch_fills_the_producers_places() {
        let mut blocks = vec![Block::ZERO; 8];
        crate::test::paint_test_input(Block::bytes_mut(&mut blocks));
        let borrowed = Batch::from_blocks(&blocks, 128);
        let mut places = [[0u8; OUT_LEN]; 4];
        let events = RefCell::new(Vec::new());
        let stream = stream(1, 1, &events);
        run(stream.submit_batch(borrowed, &mut places, ()));
        stream.finish();
        assert_eq!(events.borrow().len(), 1);
        for (i, place) in places.iter().enumerate() {
            assert_eq!(place, &expected(Mode::Plain, &Block::bytes(&blocks)[i * 128..][..128]));
        }
    }
}
