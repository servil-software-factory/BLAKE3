//! The Pipeline's lifecycles: every legal sequence of its calls to a small
//! depth, and a long seeded one, under the crate's `block_on` and under
//! an executor that polls again only when its waker has woken, in a pool
//! of one thread and in a pool with workers. Every event is checked
//! against official BLAKE3 and against the order the messages ended in.
//! (The core's tests/lifecycle.rs says why; `tools/api.py --check` holds
//! the lists below to the crate's public operations.)
//!
//! The two tests start pools of different sizes, and the pool is the
//! process's: they need a process each, which nextest gives them.

use blake3_servil::{Mode, throughput::*};

#[path = "../../tools/require_nextest.rs"]
mod require_nextest;

struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn range(&mut self, len: usize) -> usize {
        self.next_u64() as usize % len
    }
}

use std::{
    collections::VecDeque,
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Wake},
    thread,
};

// WALKED
// Pipeline::new Pipeline::with_owners Pipeline::with_batches Pipeline::with_owners_and_batches Pipeline::message Pipeline::message_with Pipeline::room Pipeline::commit Pipeline::end Pipeline::abort Pipeline::owned
// Pipeline::lent Pipeline::batch Pipeline::next_event Pipeline::ready Pipeline::Drop Room::len Room::bytes_mut Room::as_mut_ptr Room::Drop Lent::Drop
// Pipeline::Send Room::Send
// NOT_WALKED
// Room::Sync: A shared reference to a Room gives its length alone.
// Message::Drop: Its panic has a unit test (contract_dropped_unfinished_message_panics, threads/src/pipeline.rs); legal steps drop no message.
// END
//
// The drops walked here are the quiet ones: a Pipeline with nothing in
// flight, a guard after its event. The stops (a Lent guard or a Pipeline
// dropped while a worker reads) end the process, and have their test in
// threads/src/pipeline.rs
// (active_loan_drop_and_active_pipeline_drop_abort_the_process), as each
// refusal of a call has there (contract_*).

/// The modes a walk's Pipeline takes in turn, and official BLAKE3's hash in
/// each: a Pipeline that ignored its key or its context fails here.
const KEY: [u8; 32] = [7; 32];
const MODES: [Mode<'static>; 3] = [Mode::Plain, Mode::Keyed(&KEY), Mode::DeriveKey("BLAKE3-Servil Pipeline lifecycle test")];
fn official(mode: Mode<'static>, bytes: &[u8]) -> [u8; 32] {
    match mode {
        Mode::Plain => *blake3::hash(bytes).as_bytes(),
        Mode::Keyed(key) => *blake3::keyed_hash(key, bytes).as_bytes(),
        Mode::DeriveKey(context) => blake3::derive_key(context, bytes),
        _ => unreachable!(),
    }
}

struct Counter(AtomicUsize, thread::Thread);
impl Wake for Counter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::Release);
        self.1.unpark();
    }
}
fn outside<F: Future>(future: F) -> F::Output {
    let counter = Arc::new(Counter(AtomicUsize::new(0), thread::current()));
    let waker = counter.clone().into();
    let mut f = std::pin::pin!(future);
    loop {
        let before = counter.0.load(Ordering::Acquire);
        if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut Context::from_waker(&waker)) {
            return v;
        }
        while counter.0.load(Ordering::Acquire) == before {
            thread::park();
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Step {
    Open,
    Take,
    Release(usize),
    Commit(usize, usize, usize),
    End(usize),
    Abort(usize),
    Owned(usize),
    Loan(usize),
    Batch(usize),
    Next,
    Ready,
}
struct Open {
    tag: usize,
    m: Message,
    bytes: Vec<u8>,
    sealed: bool,
}
enum Owed<'a> {
    Hash(Vec<u8>, Option<Lent<'a>>, bool),
    Abort,
    Batch(Vec<[u8; 32]>),
}
struct Walk<'a> {
    p: Pipeline<usize, Vec<u8>>,
    mode: Mode<'static>,
    limits: Limits,
    open: Vec<Open>,
    rooms: Vec<(Room, usize)>,
    owed: VecDeque<(usize, Owed<'a>)>,
    data: &'a [u8],
    next: usize,
    ready: bool,
}
impl<'a> Walk<'a> {
    fn new(mode: Mode<'static>, limits: Limits, data: &'a [u8]) -> Self {
        Self { p: Pipeline::with_owners(mode, limits), mode, limits, open: vec![], rooms: vec![], owed: VecDeque::new(), data, next: 0, ready: false }
    }
    fn choices(&self) -> Vec<Step> {
        use Step::*;
        let mut v = vec![Next];
        if !self.owed.is_empty() {
            v.push(Ready);
        }
        if self.open.len() < self.limits.messages && self.open.len() + self.owed.len() < 2 * self.limits.messages {
            v.push(Step::Open);
        }
        if self.rooms.len() < self.limits.rooms {
            v.push(Take);
        }
        for r in 0..self.rooms.len() {
            v.push(Release(r));
        }
        for (i, msg) in self.open.iter().enumerate() {
            v.extend([End(i), Abort(i)]);
            if !msg.sealed && msg.bytes.is_empty() {
                v.extend([Owned(i), Loan(i), Batch(i)]);
            }
            if !msg.sealed {
                for (r, (_, remaining)) in self.rooms.iter().enumerate() {
                    for n in [0, 37, ROOM_LEN] {
                        if n <= *remaining {
                            v.push(Commit(i, r, n));
                        }
                    }
                }
            }
        }
        v
    }
    fn receive(&mut self, event: Event<usize, Vec<u8>>) {
        self.ready = false;
        let (tag, owed) = self.owed.pop_front().expect("unexpected event");
        match (event, owed) {
            (Event::Hashed { tag: t, hash, owner }, Owed::Hash(bytes, loan, owned)) => {
                assert_eq!(t, tag);
                assert_eq!(*hash.as_bytes(), official(self.mode, &bytes));
                assert_eq!(owner.as_deref(), owned.then_some(bytes.as_slice()));
                drop(loan);
            }
            (Event::Aborted { tag: t, owner }, Owed::Abort) => {
                assert_eq!(t, tag);
                assert!(owner.is_none());
            }
            (Event::Batch { tag: t, batch, digests }, Owed::Batch(expect)) => {
                assert_eq!(t, tag);
                assert_eq!(batch.len(), expect.len());
                assert_eq!(&*digests, &expect);
            }
            _ => panic!("wrong event kind for tag {tag}"),
        }
    }
    async fn step(&mut self, s: Step) {
        match s {
            Step::Open => {
                let tag = self.next;
                self.next += 1;
                let m = self.p.message(tag, (tag % 2 == 0).then_some(0)).await;
                self.open.push(Open { tag, m, bytes: vec![], sealed: false });
            }
            Step::Take => {
                let r = self.p.room().await;
                assert_eq!(r.len(), ROOM_LEN);
                self.rooms.push((r, ROOM_LEN));
            }
            Step::Release(r) => {
                drop(self.rooms.remove(r));
            }
            Step::Commit(i, r, n) => {
                let m = &mut self.open[i];
                let (room, left) = &mut self.rooms[r];
                assert_eq!(room.as_mut_ptr(), room.bytes_mut().as_mut_ptr());
                assert_eq!(room.bytes_mut().len(), *left, "a Room lends what is left of it, and no more");
                let offset = (m.tag + m.bytes.len() / ROOM_LEN) % 251;
                let bytes = &self.data[offset..offset + n];
                room.bytes_mut()[..n].copy_from_slice(bytes);
                m.bytes.extend_from_slice(bytes);
                self.p.commit(&mut m.m, room, n);
                *left -= n.next_multiple_of(64);
                assert_eq!(room.len(), *left);
                m.sealed = n != ROOM_LEN;
            }
            Step::Next => {
                if let Some(ev) = self.p.next_event() {
                    self.receive(ev);
                } else if !self.owed.is_empty() {
                    assert!(!self.ready, "ready promised an event");
                    // Normalize worker timing so replay has the same branches on either executor.
                    self.p.ready().await;
                    let ev = self.p.next_event().expect("ready promised an event");
                    self.receive(ev);
                }
            }
            Step::Ready => {
                self.p.ready().await;
                self.ready = true;
            }
            Step::End(i) | Step::Abort(i) | Step::Owned(i) | Step::Loan(i) | Step::Batch(i) => {
                let Open { tag, m, bytes, .. } = self.open.remove(i);
                let data = &self.data[..[65, self.data.len(), 1025, 0][tag % 4]];
                let owed = match s {
                    Step::End(_) => {
                        self.p.end(m);
                        Owed::Hash(bytes, None, false)
                    }
                    Step::Abort(_) => {
                        self.p.abort(m);
                        Owed::Abort
                    }
                    Step::Owned(_) => {
                        self.p.owned(m, data.to_vec());
                        Owed::Hash(data.to_vec(), None, true)
                    }
                    Step::Loan(_) => {
                        // Sound: data outlives the walk and the guard stays queued until its event.
                        let guard = unsafe { self.p.lent(m, data) };
                        Owed::Hash(data.to_vec(), Some(guard), false)
                    }
                    Step::Batch(_) => {
                        let mut batch = Batch::new(73, 3);
                        let mut expected = vec![];
                        for j in 0..3 {
                            let b = &self.data[tag % 251 + j..][..73];
                            batch.set(j, b);
                            expected.push(official(self.mode, b));
                        }
                        self.p.batch(m, batch, vec![[0; 32]; 3].into());
                        Owed::Batch(expected)
                    }
                    _ => unreachable!(),
                };
                self.owed.push_back((tag, owed));
            }
        }
    }
    async fn finish(mut self) {
        self.rooms.clear();
        while !self.open.is_empty() {
            self.step(Step::End(0)).await;
        }
        while !self.owed.is_empty() {
            self.step(Step::Next).await;
        }
        assert!(self.p.next_event().is_none());
    }
}
/// `Pipeline::new`, the Pipeline that takes no owners: a message on loan,
/// and one through a Room where it has one.
async fn without_owners(mode: Mode<'static>, limits: Limits, data: &[u8]) {
    let mut p = Pipeline::<usize>::new(mode, limits);
    let m = p.message(1, None).await;
    // Sound: `data` outlives the Pipeline, and the guard is held until its event is taken.
    let guard = unsafe { p.lent(m, &data[..1025]) };
    p.ready().await;
    let Some(Event::Hashed { tag: 1, hash, owner: None }) = p.next_event() else { panic!("the loan's event") };
    assert_eq!(*hash.as_bytes(), official(mode, &data[..1025]));
    drop(guard);
    {
        let mut pb = Pipeline::<usize, (), blake3_servil::throughput::Packed<32>>::with_batches(mode, Limits { messages: 1, rooms: 0 });
        let mb = pb.message(0, None).await;
        let mut packed = blake3_servil::throughput::Packed::<32>::new(4);
        for (i, item) in packed.iter_mut().enumerate() {
            item.fill(i as u8);
        }
        pb.batch(mb, packed, vec![[0u8; 32]; 4].into_boxed_slice());
        pb.ready().await;
        let Some(Event::Batch { batch, digests, .. }) = pb.next_event() else {
            panic!("expected packed Event::Batch");
        };
        for i in 0..4 {
            assert_eq!(digests[i], official(mode, &batch[i]));
        }
    }
    if limits.rooms > 0 {
        let mut m = p.message_with(mode, 2, None).await;
        let mut room = p.room().await;
        room.bytes_mut()[..37].copy_from_slice(&data[..37]);
        p.commit(&mut m, &mut room, 37);
        drop(room);
        p.end(m);
        p.ready().await;
        let Some(Event::Hashed { tag: 2, hash, owner: None }) = p.next_event() else { panic!("the Room's event") };
        assert_eq!(*hash.as_bytes(), official(mode, &data[..37]));
    }
    assert!(p.next_event().is_none());
}
async fn exercise() {
    let data: Vec<_> = (0..3 * 1024 * 1024 + 1).map(|j| (j % 251) as u8).collect();
    let mut paths = 0;
    for messages in 1..=3 {
        for rooms in 0..=2 {
            let limits = Limits { messages, rooms };
            without_owners(MODES[(messages + rooms) % 3], limits, &data).await;
            // Replay rather than clone a Pipeline with live workers. Each digit chooses one legal step in the current state.
            const DEPTH: usize = 5;
            let mut digits = [0; DEPTH];
            loop {
                let mut w = Walk::new(MODES[paths % 3], limits, &data[..ROOM_LEN + 251]);
                let mut widths = [0; DEPTH];
                for d in 0..DEPTH {
                    let choices = w.choices();
                    widths[d] = choices.len();
                    w.step(choices[digits[d]]).await;
                }
                w.finish().await;
                paths += 1;
                let Some(d) = (0..DEPTH).rev().find(|&d| digits[d] + 1 < widths[d]) else { break };
                digits[d] += 1;
                digits[d + 1..].fill(0);
            }
        }
    }
    eprintln!("{paths} depth-five paths");
    const SEED: u64 = 0x774aa90;
    eprintln!("seed {SEED:#x}");
    let mut rng = SplitMix64(SEED);
    let mut w = Walk::new(MODES[1], Limits { messages: 3, rooms: 2 }, &data);
    for _ in 0..2300 {
        let choices = w.choices();
        w.step(choices[rng.range(choices.len())]).await;
    }
    w.finish().await;
}
fn run(n: usize) {
    blake3_servil::initialize_multithreaded(Some(n.try_into().unwrap()));
    block_on(exercise());
    outside(exercise());
}
/// A Pipeline, its open message and a Room of it are `Send`: another
/// thread finishes the message, and the hash is the whole message's.
#[test]
fn a_pipeline_and_its_tokens_move_to_another_thread_mid_message() {
    blake3_servil::initialize_multithreaded(Some(3.try_into().unwrap()));
    let data: Vec<u8> = (0..ROOM_LEN + 37).map(|j| (j % 251) as u8).collect();
    let mode = MODES[1];
    let mut p = Pipeline::<usize, Vec<u8>>::with_owners(mode, Limits { messages: 2, rooms: 2 });
    let (mut m, mut first, mut second) = block_on(async { (p.message(9, None).await, p.room().await, p.room().await) });
    first.bytes_mut().copy_from_slice(&data[..ROOM_LEN]);
    p.commit(&mut m, &mut first, ROOM_LEN);
    second.bytes_mut()[..37].copy_from_slice(&data[ROOM_LEN..]);
    let there = thread::spawn(move || {
        p.commit(&mut m, &mut second, 37);
        drop((first, second));
        p.end(m);
        block_on(p.ready());
        let Some(Event::Hashed { tag: 9, hash, owner: None }) = p.next_event() else { panic!("the message's event") };
        *hash.as_bytes()
    });
    assert_eq!(there.join().unwrap(), official(mode, &data));
}
#[test]
fn keyed_pipeline_with_owners_and_batches_mixes_owned_and_packed() {
    blake3_servil::initialize_multithreaded(Some(3.try_into().unwrap()));
    let mode = MODES[1];
    let mut p = Pipeline::<usize, Vec<u8>, blake3_servil::throughput::Packed<32>>::with_owners_and_batches(mode, Limits { messages: 4, rooms: 0 });
    let owned_msg: Vec<u8> = (0..3000).map(|i| ((i * 19 + 5) & 0xff) as u8).collect();
    let mut packed = blake3_servil::throughput::Packed::<32>::new(1030);
    for i in 0..1030 {
        let idx = (i as u32).to_le_bytes();
        for j in 0..32 {
            packed[i][j] = if j < 4 { idx[j] } else { ((i * 7 + j * 11) & 0xff) as u8 };
        }
    }
    let (m0, m1) = block_on(async { (p.message(0, None).await, p.message(1, None).await) });
    p.owned(m0, owned_msg.clone());
    p.batch(m1, packed, vec![[0u8; 32]; 1030].into_boxed_slice());
    let mut got = 0;
    while got < 2 {
        match p.next_event() {
            Some(Event::Hashed { tag: 0, hash, owner: Some(buf) }) => {
                assert_eq!(buf, owned_msg);
                assert_eq!(*hash.as_bytes(), official(mode, &owned_msg));
                got += 1;
            }
            Some(Event::Batch { tag: 1, batch, digests }) => {
                assert_eq!(batch.len(), 1030);
                for i in 0..1030 {
                    assert_eq!(digests[i], official(mode, &batch[i]));
                }
                got += 1;
            }
            Some(_) => panic!("unexpected event"),
            None => block_on(p.ready()),
        }
    }
}
#[test]
fn both_executors_deliver_the_promised_events_without_workers() {
    run(1);
}
#[test]
fn both_executors_deliver_the_promised_events_with_workers() {
    run(3);
}
