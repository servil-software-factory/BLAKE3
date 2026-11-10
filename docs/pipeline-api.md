# The Pipeline: the one streaming API, defined

**Status, November 10, 2026: a design, not yet built.** Zooko and John
worked it out in conversation; this page is what they arrived at. It is to
replace both `throughput::stream` and `throughput::pipeline` before 0.9.
Its signatures compile, with every body empty, in
`docs/experiments/pipeline-api-skeleton`, whose examples are the claims
below as the compiler checks them (`cargo test --doc` there): seven that
compile and six it refuses. No speed
claim on this page is measured.

## What it is for

A program hashes messages on other cores while it goes on reading or
producing the next bytes. The pipeline holds a fixed amount of memory, makes
the program wait when that is full, and allocates nothing once made.

## One rule

**Every call takes exclusive access to the pipeline, and everything the
program holds between calls is a plain value.** A message is a `Message`,
room for bytes is a `Room`, a finished message is an `Event` the program
pulls. None of them refers back to the pipeline.

From that rule:

- One thread drives a pipeline at a time, and the compiler checks it. The
  pipeline's bookkeeping needs no lock.
- The pipeline, its messages and its rooms may move between threads, so a
  task that holds them runs under a work-stealing runtime.
- The pipeline is free between calls: during a network wait, while another
  message is fed, while an event is handled.
- Handling an event may call the pipeline, since the call that returned the
  event is over.

## The calls

```rust
pub struct Limits { pub messages: usize, pub rooms: usize }
pub const ROOM_LEN: usize = 16 * 1024;

pub struct Pipeline<T, B = ()>;        // T: the program's tag. B: the owner of a message hashed in place.
pub struct Message;                  // an open message
pub struct Room;                     // room for messages' next bytes
pub struct Lent<'a>;                 // the loan of a lent message

pub enum Event<T, B = ()> {
    Hashed  { tag: T, hash: Hash, owner: Option<B> },
    Aborted { tag: T, owner: Option<B> },
    Batch   { tag: T, batch: Batch<'static>, digests: Box<[[u8; 32]]> },
}

impl<T> Pipeline<T> {
    pub fn new(mode: Mode, limits: Limits) -> Self;
}
impl<T, B: AsRef<[u8]>> Pipeline<T, B> {
    pub fn with_owners(mode: Mode, limits: Limits) -> Self;
    pub fn owned(&mut self, message: Message, owner: B);
}
impl<T, B> Pipeline<T, B> {
    pub async fn message(&mut self, tag: T, length_hint: Option<u64>) -> Message;

    pub async fn room(&mut self) -> Room;
    pub fn commit(&mut self, message: &mut Message, room: &mut Room, written: usize);
    pub fn end(&mut self, message: Message);
    pub fn abort(&mut self, message: Message);

    pub unsafe fn lent<'a>(&mut self, message: Message, bytes: &'a [u8]) -> Lent<'a>;
    pub fn batch(&mut self, message: Message, batch: Batch<'static>, digests: Box<[[u8; 32]]>);

    pub fn next_event(&mut self) -> Option<Event<T, B>>;
    pub async fn ready(&mut self);
}
impl Room {
    pub fn bytes_mut(&mut self) -> &mut [u8];
    pub fn as_mut_ptr(&mut self) -> *mut u8;
    pub fn len(&self) -> usize;
}

pub fn block_on<F: Future>(future: F) -> F::Output;
```

## A message, and the four ways to give it bytes

`message(tag, length_hint).await` waits for a message slot and returns the
message. The program then gives it bytes in exactly one of four ways.

| The bytes | Calls | Copies | Checked by |
|---|---|---|---|
| arrive over time (a socket, a file, a generator), or are short and many | `room`, write, `commit`, repeated; then `end` | the one that brings them into the process; none when produced in the room | the compiler |
| are in memory, owned by a value the program can hand over | `owned(message, owner)` | none | the compiler |
| are in memory, on loan | `unsafe lent(message, bytes)` | none | the compiler, given one promise |
| are a batch of equal-length messages | `batch(message, batch, digests)` | none | the compiler |

Each ends in one event carrying the tag: `Hashed`, `Batch`, or `Aborted`.
What was handed over comes back in it: the owner, the batch and its
digests' storage.

**Rooms.** A room is `ROOM_LEN` bytes of the pipeline's memory, at an
address that never changes, and it belongs to no message.
`commit(message, room, written)` makes the first `written` bytes of what
is left of the room that message's next bytes; the room goes on from the
next 64-byte boundary, for the next commit, of any message. So:

- many short messages share a room, back to back, and one `room` call
  serves them all (256 messages of 64 bytes to a room);
- a program may take a room before it knows whose bytes it will hold (a
  multiplexed connection: read, then learn the message);
- a program may hold several rooms at once.

**Every commit to a message but its last is a whole room.** A short
message is one commit, from what is left of a room; a long one is whole
rooms and a last part. Each part then hashes where it lies, as a whole
subtree of the message, with nothing copied and no memory kept per
message. A program that cannot tell whether a message fits what is left
of a room takes a fresh room for it.

**In place, owned.** The pipeline keeps `owner` where it puts it and reads
`owner.as_ref()` there, so any type that gives its bytes works: a `Vec`,
a `Bytes`, a file mapping, an array. Bytes on a 64-byte boundary hash
fastest.

**In place, lent.** `lent` returns a guard. While the guard lives the
compiler keeps `bytes` borrowed: the program can neither free nor write
them. Dropped before the message's event, the guard withdraws the pieces
not yet started, waits the microseconds for those being read, and the
message ends `Aborted`; so a cancelled task and an early return are safe.

- **The caller's promise** (why the call is `unsafe`): the guard is
  dropped, never leaked (`mem::forget`, `Box::leak`, a reference cycle, a
  leaked task that holds it). `#![deny(clippy::mem_forget)]` rejects the
  first of these in a program's own code.
- **A broken promise** has the pipeline's workers read memory that was freed
  or is being rewritten. They only read, and treat the bytes as data. The
  likely result is a wrong digest with no warning; a crash if the memory
  was unmapped; where the memory was reused for a secret, a digest of that
  secret. Rust defines none of this.

The places digests are written are never lent: there the pipeline would
write into freed memory.

## Memory, waiting, and events

- **Fixed memory.** `limits.rooms` rooms, and records for
  `limits.messages` messages open or in flight and as many finished
  events, made with the pipeline. Nothing is allocated after.
- **Two waits.** `message` waits for a message slot; `room` waits for a
  room. A room frees when every byte committed from it is hashed and the
  program has let it go; the room freed most recently is handed out first.
- **Events are pulled.** `next_event()` returns the next finished message;
  `ready().await` waits until there is one. Events come in the order
  messages were ended.
- **Pull to make progress.** Finished events wait to be pulled, up to
  `limits.messages` of them, beside the messages still open or in flight.
  A program pulls events between opening messages. When `message` would
  have to wait and that many events are waiting, nothing could ever free
  a slot: the call stops the program, saying so.
- **Blocking programs** run the waits with `block_on`, which parks the
  thread between polls.

## Dropped values

- A `Message` dropped without `end`, `abort` or a hand-over is aborted:
  the pipeline notices at its next call and its `Aborted` event follows. So
  `?` and a cancelled task release what they held.
- A `Room` dropped gives back what is left of it.
- A wait that is dropped took nothing, except that `message` drops its
  tag.
- A `Pipeline` dropped with work in flight abandons it. Its memory serves a
  later pipeline once that work ends; owners and batches it still held are
  dropped then.

## Contract checks

What the compiler cannot refuse. Each stops the program, in every build,
with a plain message; each is a compare on counts and records the call
already reads.

- a `Message` or `Room` used with a pipeline that did not make it;
- `commit` with `written > room.len()`;
- a commit after one that was less than a whole room: a message's parts
  but its last are whole rooms;
- bytes given two ways: `owned`, `lent` or `batch` on a message that has
  committed bytes;
- **a wait that only its caller could end**:
  - `message` when every slot is an open message of the caller's, or
    `limits.messages` events wait to be pulled;
  - `room` when every room is out, uncommitted, in the caller's hands.

  Each is checked when the wait begins and each time it is woken: a slot
  or a room that a worker will free is waited for; one that only the
  waiting caller could free never comes. (With `limits.messages = 1`:
  open A, then wait for B before ending A.)

## io_uring

A program that drives io_uring hands the kernel `room.as_mut_ptr()` and
commits the room when the read completes; a `Room` is a value, so a
runtime that insists on owning the buffer during a read can hold it. For
later, and unmeasured: registering the pipeline's rooms with the kernel
once, so that reads land in them by io_uring's fixed-buffer path.

## What this replaces

- `throughput::stream`, `Stream::submit*`, `collect`, `finish`: gone.
- `throughput::pipeline` and its callback: this is its successor, under
  the name `throughput::Pipeline`.
- The direct calls stay as they are (`hash_multithreaded`, `hash_many*`,
  `hash_each*`): a program that only has a loan and nothing else to do
  meanwhile calls them, with no copy.

## Open questions

1. **Whether the last commit and `end` become one call**, and whether
   events are pulled several at a call: each saves a call a short
   message. Left for the measurements of the build (the stream spends
   17 ns of the producer's time a 64 B message, i7-12700K).
2. **A count of unpulled messages** (`pending()`), so that "wait for
   everything" is a three-line loop without the program counting. Left out
   as the program can count; cheap to add.
3. **Lending a batch.** Left out: its digests would be written to lent
   memory.
4. **Names** still open: `with_owners`, `block_on`. Decided (Zooko,
   November 10): `Pipeline`, `owned`, `lent`.
5. **Many tasks, one memory budget.** A pipeline per task is the design
   point; a shared budget that several pipelines draw rooms from would be an
   addition.
