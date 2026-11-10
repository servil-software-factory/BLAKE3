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
pub struct Batch;

/// The stream's fixed memory.
pub struct Limits {
    /// Messages open or in flight; as many finished events again may
    /// wait to be pulled.
    pub messages: usize,
    /// Rooms of `ROOM_LEN` bytes: the arriving bytes in flight.
    pub rooms: usize,
}
pub const ROOM_LEN: usize = 16 * 1024;

/// `T`: the program's tag, handed back with each event. `B`: the type
/// that owns a message hashed where it lies (`whole`); `()` for none.
pub struct Stream<T, B = ()> {
    _own: PhantomData<(T, B)>,
}

/// An open message: a token. It holds no reference to its stream.
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
    Batch { tag: T, batch: Batch, digests: Box<[[u8; 32]]> },
}

impl<T> Stream<T, ()> {
    pub fn new(mode: Mode<'_>, limits: Limits) -> Self {
        todo!()
    }
}

impl<T, B: AsRef<[u8]>> Stream<T, B> {
    pub fn with_owners(mode: Mode<'_>, limits: Limits) -> Self {
        todo!()
    }
    /// The message is `owner`'s bytes, hashed where they lie; the owner
    /// comes back in the message's event.
    pub fn whole(&mut self, message: Message, owner: B) {
        todo!()
    }
}

impl<T, B> Stream<T, B> {
    /// Wait for a message slot.
    pub async fn message(&mut self, tag: T, length_hint: Option<u64>) -> Message {
        todo!()
    }
    /// Wait for a room for `message`'s next bytes.
    pub async fn room(&mut self, message: &mut Message) -> Room {
        todo!()
    }
    /// The room's first `written` bytes are the message's next bytes.
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
    pub unsafe fn lend<'a>(&mut self, message: Message, bytes: &'a [u8]) -> Lent<'a> {
        todo!()
    }
    /// The message is a batch; it and its digests come back in its event.
    pub fn batch(&mut self, message: Message, batch: Batch, digests: Box<[[u8; 32]]>) {
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
/// Two messages open at once, fed in turns; events pulled, and the stream
/// called while handling one:
/// ```
/// use stream_api_skeleton::*;
/// async fn read(_into: &mut [u8]) -> usize { 0 }
/// async fn two_uploads(stream: &mut Stream<&'static str>) {
///     let mut a = stream.message("A", None).await;
///     let mut b = stream.message("B", None).await;
///     let mut room = stream.room(&mut a).await;
///     let n = read(room.bytes_mut()).await;      // the stream is free during this wait
///     stream.commit(&mut a, room, n);
///     let mut room = stream.room(&mut b).await;
///     let n = read(room.bytes_mut()).await;
///     stream.commit(&mut b, room, n);
///     stream.end(a);
///     stream.end(b);
///     stream.ready().await;
///     while let Some(event) = stream.next_event() {
///         if let Event::Hashed { tag, .. } = event {
///             let follow_up = stream.message(tag, None).await;   // no re-entrancy: the call returned
///             stream.end(follow_up);
///         }
///     }
/// }
/// ```
///
/// A task that holds the stream, messages and a room across awaits may
/// move between threads (what `tokio::spawn` requires):
/// ```
/// use stream_api_skeleton::*;
/// fn spawnable<F: Future + Send + 'static>(_task: F) {}
/// async fn read(_into: &mut [u8]) -> usize { 0 }
/// spawnable(async {
///     let mut stream = Stream::<u64, Vec<u8>>::with_owners(Mode::Plain, Limits { messages: 2, rooms: 4 });
///     let mut m = stream.message(7, None).await;
///     let mut room = stream.room(&mut m).await;
///     let n = read(room.bytes_mut()).await;
///     stream.commit(&mut m, room, n);
///     stream.end(m);
///     let whole = stream.message(8, None).await;
///     stream.whole(whole, vec![1, 2, 3]);
///     stream.ready().await;
/// });
/// ```
///
/// A message used after it ended is refused:
/// ```compile_fail
/// use stream_api_skeleton::*;
/// async fn f(stream: &mut Stream<u64>) {
///     let mut m = stream.message(1, None).await;
///     stream.end(m);
///     let _ = stream.room(&mut m).await;
/// }
/// ```
///
/// A lent buffer cannot be freed while its guard lives:
/// ```compile_fail
/// use stream_api_skeleton::*;
/// async fn f(stream: &mut Stream<u64>) {
///     let buffer = vec![0u8; 1 << 20];
///     let m = stream.message(1, None).await;
///     let guard = unsafe { stream.lend(m, &buffer) };
///     drop(buffer);
///     stream.ready().await;
///     drop(guard);
/// }
/// ```
///
/// nor written:
/// ```compile_fail
/// use stream_api_skeleton::*;
/// async fn f(stream: &mut Stream<u64>) {
///     let mut buffer = vec![0u8; 1 << 20];
///     let m = stream.message(1, None).await;
///     let guard = unsafe { stream.lend(m, &buffer) };
///     buffer[0] = 1;
///     stream.ready().await;
///     drop(guard);
/// }
/// ```
///
/// and held properly, it compiles, the guard dropped before the buffer:
/// ```
/// use stream_api_skeleton::*;
/// async fn f(stream: &mut Stream<u64>) {
///     let buffer = vec![0u8; 1 << 20];
///     let m = stream.message(1, None).await;
///     let _guard = unsafe { stream.lend(m, &buffer) };
///     stream.ready().await;
/// }
/// ```
///
/// A stream with no owner type has no `whole`:
/// ```compile_fail
/// use stream_api_skeleton::*;
/// async fn f(stream: &mut Stream<u64>) {
///     let m = stream.message(1, None).await;
///     stream.whole(m, ());
/// }
/// ```
///
/// One driver at a time: two waits on one stream at once are refused:
/// ```compile_fail
/// use stream_api_skeleton::*;
/// async fn f(stream: &mut Stream<u64>) {
///     let slot = stream.message(1, None);
///     let event = stream.ready();
///     let _ = (slot.await, event.await);
/// }
/// ```
pub mod claims {}
