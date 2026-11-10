//! The one streaming API, as signatures: every body is `todo!()`. Its
//! purpose is to let the compiler confirm what the design claims (the
//! examples below compile; the `compile_fail` ones are refused).
#![allow(unused_variables, dead_code)]
use core::marker::PhantomData;

pub struct Hash(pub [u8; 32]);
#[derive(Clone, Copy)]
pub enum Mode<'a> {
    Plain,
    Keyed(&'a [u8; 32]),
    DeriveKey(&'a str),
}
/// The crate's `Batch<'a>`: owned (`'static`) or over borrowed blocks.
pub struct Batch<'a> {
    _blocks: PhantomData<&'a [u8]>,
}
impl Batch<'static> {
    pub fn new(message_len: usize, count: usize) -> Self {
        todo!()
    }
}
impl<'a> Batch<'a> {
    pub fn from_blocks(blocks: &'a [u8], message_len: usize) -> Self {
        todo!()
    }
}

/// The pipeline's fixed memory.
pub struct Limits {
    /// Messages open or in flight; as many finished events again may
    /// wait to be pulled.
    pub messages: usize,
    /// Rooms of `ROOM_LEN` bytes: the arriving bytes in flight.
    pub rooms: usize,
}
pub const ROOM_LEN: usize = 16 * 1024;

/// `T`: the program's tag, handed back with each event. `B`: the type
/// that owns a message hashed where it lies (`owned`); `()` for none.
pub struct Pipeline<T, B = ()> {
    _own: PhantomData<(T, B)>,
}

/// An open message: a token. It holds no reference to its pipeline.
#[must_use = "end, abort, or hand the message's bytes over; a dropped Message is aborted"]
pub struct Message {
    _slot: u32,
}

/// Room for a message's next bytes: a token. Its memory never moves.
#[must_use = "commit the room; a dropped Room goes back unused"]
pub struct Room {
    _index: u32,
}
impl Room {
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        todo!()
    }
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        todo!()
    }
    pub fn len(&self) -> usize {
        ROOM_LEN
    }
}

/// The loan of a lent message. While it lives, the compiler keeps the
/// bytes borrowed; dropped before the message's event, it stops the
/// hashing and waits for the readers, then the bytes are free.
#[must_use = "hold the guard until the message's event; dropping it aborts the message"]
pub struct Lent<'a> {
    _bytes: PhantomData<&'a [u8]>,
}
impl Drop for Lent<'_> {
    fn drop(&mut self) {}
}

pub enum Event<T, B = ()> {
    Hashed { tag: T, hash: Hash, owner: Option<B> },
    Aborted { tag: T, owner: Option<B> },
    Batch { tag: T, batch: Batch<'static>, digests: Box<[[u8; 32]]> },
}

impl<T> Pipeline<T, ()> {
    pub fn new(mode: Mode<'_>, limits: Limits) -> Self {
        todo!()
    }
}

impl<T, B: AsRef<[u8]>> Pipeline<T, B> {
    pub fn with_owners(mode: Mode<'_>, limits: Limits) -> Self {
        todo!()
    }
    /// The message is `owner`'s bytes, hashed where they lie; the owner
    /// comes back in the message's event.
    pub fn owned(&mut self, message: Message, owner: B) {
        todo!()
    }
}

impl<T, B> Pipeline<T, B> {
    /// Wait for a message slot.
    pub async fn message(&mut self, tag: T, length_hint: Option<u64>) -> Message {
        todo!()
    }
    /// Wait for a room: memory for some message's next bytes. A room
    /// belongs to no message until it is committed to one.
    pub async fn room(&mut self) -> Room {
        todo!()
    }
    /// The room's first `written` bytes are `message`'s next bytes.
    pub fn commit(&mut self, message: &mut Message, room: Room, written: usize) {
        todo!()
    }
    pub fn end(&mut self, message: Message) {
        todo!()
    }
    pub fn abort(&mut self, message: Message) {
        todo!()
    }
    /// The message is `bytes`, hashed where they lie, on loan.
    ///
    /// # Safety
    /// The returned guard is dropped, never leaked (`mem::forget`,
    /// `Box::leak`, a reference cycle, a leaked task that holds it).
    pub unsafe fn lent<'a>(&mut self, message: Message, bytes: &'a [u8]) -> Lent<'a> {
        todo!()
    }
    /// The message is a batch; it and its digests come back in its event.
    pub fn batch(&mut self, message: Message, batch: Batch<'static>, digests: Box<[[u8; 32]]>) {
        todo!()
    }
    /// The next finished message, if any.
    pub fn next_event(&mut self) -> Option<Event<T, B>> {
        todo!()
    }
    /// Wait until `next_event` has one.
    pub async fn ready(&mut self) {
        todo!()
    }
}

/// Run a future on this thread, parked between polls: for a program with
/// no async runtime.
pub fn block_on<F: Future>(future: F) -> F::Output {
    todo!()
}

/// What the design claims, for the compiler to confirm.
///
/// Two messages open at once, fed in turns; events pulled, and the pipeline
/// called while handling one:
/// ```
/// use pipeline_api_skeleton::*;
/// async fn read(_into: &mut [u8]) -> usize { 0 }
/// async fn two_uploads(pipeline: &mut Pipeline<&'static str>) {
///     let mut a = pipeline.message("A", None).await;
///     let mut b = pipeline.message("B", None).await;
///     let mut room = pipeline.room().await;
///     let n = read(room.bytes_mut()).await;      // the pipeline is free during this wait
///     pipeline.commit(&mut a, room, n);
///     let mut room = pipeline.room().await;
///     let n = read(room.bytes_mut()).await;
///     pipeline.commit(&mut b, room, n);
///     pipeline.end(a);
///     pipeline.end(b);
///     pipeline.ready().await;
///     while let Some(event) = pipeline.next_event() {
///         if let Event::Hashed { tag, .. } = event {
///             let follow_up = pipeline.message(tag, None).await;   // no re-entrancy: the call returned
///             pipeline.end(follow_up);
///         }
///     }
/// }
/// ```
///
/// A task that holds the pipeline, messages and a room across awaits may
/// move between threads (what `tokio::spawn` requires):
/// ```
/// use pipeline_api_skeleton::*;
/// fn spawnable<F: Future + Send + 'static>(_task: F) {}
/// async fn read(_into: &mut [u8]) -> usize { 0 }
/// spawnable(async {
///     let mut pipeline = Pipeline::<u64, Vec<u8>>::with_owners(Mode::Plain, Limits { messages: 2, rooms: 4 });
///     let mut m = pipeline.message(7, None).await;
///     let mut room = pipeline.room().await;
///     let n = read(room.bytes_mut()).await;
///     pipeline.commit(&mut m, room, n);
///     pipeline.end(m);
///     let whole = pipeline.message(8, None).await;
///     pipeline.owned(whole, vec![1, 2, 3]);
///     pipeline.ready().await;
/// });
/// ```
///
/// A message used after it ended is refused:
/// ```compile_fail
/// use pipeline_api_skeleton::*;
/// async fn f(pipeline: &mut Pipeline<u64>) {
///     let mut m = pipeline.message(1, None).await;
///     let room = pipeline.room().await;
///     pipeline.end(m);
///     pipeline.commit(&mut m, room, 1);
/// }
/// ```
///
/// A lent buffer cannot be freed while its guard lives:
/// ```compile_fail
/// use pipeline_api_skeleton::*;
/// async fn f(pipeline: &mut Pipeline<u64>) {
///     let buffer = vec![0u8; 1 << 20];
///     let m = pipeline.message(1, None).await;
///     let guard = unsafe { pipeline.lent(m, &buffer) };
///     drop(buffer);
///     pipeline.ready().await;
///     drop(guard);
/// }
/// ```
///
/// nor written:
/// ```compile_fail
/// use pipeline_api_skeleton::*;
/// async fn f(pipeline: &mut Pipeline<u64>) {
///     let mut buffer = vec![0u8; 1 << 20];
///     let m = pipeline.message(1, None).await;
///     let guard = unsafe { pipeline.lent(m, &buffer) };
///     buffer[0] = 1;
///     pipeline.ready().await;
///     drop(guard);
/// }
/// ```
///
/// and held properly, it compiles, the guard dropped before the buffer:
/// ```
/// use pipeline_api_skeleton::*;
/// async fn f(pipeline: &mut Pipeline<u64>) {
///     let buffer = vec![0u8; 1 << 20];
///     let m = pipeline.message(1, None).await;
///     let _guard = unsafe { pipeline.lent(m, &buffer) };
///     pipeline.ready().await;
/// }
/// ```
///
/// A pipeline with no owner type has no `owned`:
/// ```compile_fail
/// use pipeline_api_skeleton::*;
/// async fn f(pipeline: &mut Pipeline<u64>) {
///     let m = pipeline.message(1, None).await;
///     pipeline.owned(m, ());
/// }
/// ```
///
/// A room is taken before the program knows whose bytes it will hold (a
/// multiplexed connection: read, then learn the message), and committed
/// to that message:
/// ```
/// use pipeline_api_skeleton::*;
/// async fn read(_into: &mut [u8]) -> (usize, bool) { (0, true) }
/// async fn f(pipeline: &mut Pipeline<u64>, a: &mut Message, b: &mut Message) {
///     let mut room = pipeline.room().await;
///     let (n, for_a) = read(room.bytes_mut()).await;
///     pipeline.commit(if for_a { a } else { b }, room, n);
/// }
/// ```
///
/// A batch the pipeline may keep is handed over and comes back, with its
/// digests' storage, in its event:
/// ```
/// use pipeline_api_skeleton::*;
/// async fn f(pipeline: &mut Pipeline<u64>) -> Option<(Batch<'static>, Box<[[u8; 32]]>)> {
///     let batch = Batch::new(64, 1024);
///     let digests = vec![[0u8; 32]; 1024].into_boxed_slice();
///     let m = pipeline.message(1, None).await;
///     pipeline.batch(m, batch, digests);
///     pipeline.ready().await;
///     match pipeline.next_event() {
///         Some(Event::Batch { batch, digests, .. }) => Some((batch, digests)),
///         _ => None,
///     }
/// }
/// ```
///
/// A batch over borrowed blocks is refused: the pipeline would read them
/// after the loan:
/// ```compile_fail
/// use pipeline_api_skeleton::*;
/// async fn f(pipeline: &mut Pipeline<u64>, blocks: &[u8]) {
///     let batch = Batch::from_blocks(blocks, 64);
///     let m = pipeline.message(1, None).await;
///     pipeline.batch(m, batch, Box::new([]));
/// }
/// ```
///
/// One driver at a time: two waits on one pipeline at once are refused:
/// ```compile_fail
/// use pipeline_api_skeleton::*;
/// async fn f(pipeline: &mut Pipeline<u64>) {
///     let slot = pipeline.message(1, None);
///     let event = pipeline.ready();
///     let _ = (slot.await, event.await);
/// }
/// ```
///
/// What the compiler cannot refuse, and the pipeline stops at run time
/// (the page's "Contract checks"): a wait that only its caller could end.
/// With one message slot, this compiles, and `message` stops the program:
/// ```
/// use pipeline_api_skeleton::*;
/// async fn f(pipeline: &mut Pipeline<u64>) {
///     let a = pipeline.message(1, None).await;
///     let b = pipeline.message(2, None).await; // every slot is an open message of this caller's
///     pipeline.end(a);
///     pipeline.end(b);
/// }
/// ```
pub mod claims {}
