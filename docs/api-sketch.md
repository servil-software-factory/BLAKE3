# Prepared bulk and incremental background hashing: Rust API sketch

**Status, November 8, 2026.** Everything below is on `next`: the `Block`/`Batch`/`Packed` types and direct bulk calls, `convenience::Hasher` with `finalize(self)`, and the incremental pipeline as `throughput::pipeline` (its names may still change before 1.0; bench-hashes measures it in the receive cells). This page stays as the design's record; the crate's documentation is the contract.

BLAKE3-Servil has two intended interfaces:

- **Convenience:** ordinary bytes, synchronous one-shot or incremental hashing.
- **Throughput:** prepared storage, including a pipeline that hashes earlier bytes while the application receives or produces later bytes of the same message.

The [contract proposal](https://github.com/servil-software-factory/BLAKE3/issues/267) specifies behavior. The [implementation prototype](https://github.com/servil-software-factory/BLAKE3/pull/272) exercises reservations and worker handoff.

## API combinations

Convenience is tuned for occasional hashing amid other work. Throughput is tuned for repeated hashing. Its direct calls offer caller-thread and multithread-capable forms independently of the background pipeline.

| Your work / call family | Tuned for | Caller-thread entry | Workers / background entry |
|---|---|---|---|
| Ordinary bytes, one message | Occasional / cold | `convenience::hash`, `hash_with`, `hash_xof` | — |
| Ordinary bytes arriving in pieces | Occasional / convenient incremental state | `convenience::Hasher` | — |
| Repeated aligned messages | Repeated / warm | `throughput::hash` | `throughput::hash_multithreaded` |
| Equal-length messages in a `Batch` | Repeated / warm, side-by-side hashing | `throughput::hash_many` | `throughput::hash_many_multithreaded` |
| Mixed-length collection in `Block`s | Repeated / warm | `throughput::hash_each` | `throughput::hash_each_multithreaded` |
| Read/generate complete messages, batches, or incremental pieces | Ongoing production overlapped with hashing | — | `throughput::pipeline::Stream`: `submit_batch`, or a message's `input` / `commit` / `end(n)` (beside today's `throughput::Stream`, until the measurements decide) |

- The first row's three functions are in `convenience`. Batching and library-managed parallelism belong to throughput. Its direct calls and prepared types already exist and are retained.
- Multithread-capable means the call may use workers where that helps. It still completes synchronously. Initialize those entry points with `initialize_multithreaded`.
- The background stream lets the producer continue while hashing runs. It uses the same initialized worker pool, within its configured budget.
- The current shared `throughput::Hasher` has hybrid tuning and parallel-update methods. The proposal splits that role into a convenience Hasher and throughput message handles. Grouped incremental Hasher operations need a settled migration path, as discussed below.

## Prepared inputs stay part of throughput

The throughput interface carries layout guarantees in types:

| Type | Guarantee |
|---|---|
| `Block` | 64 accessible bytes on a 64-byte boundary. Its contents can be any bytes. |
| `Batch` | Messages of one logical length, each starting on a block boundary, with protected zero padding after each message. |

Constructing a value establishes the guarantee. Hashing callers pass that value, so the implementation can trust the alignment and layout. `Block::from_bytes` checks existing storage once and returns a borrowed view when it is aligned and whole-block sized. It performs no copy.

The single-message and collection calls still ignore bytes beyond each logical end. `Block` alone does **not** promise a zero tail. `Batch` carries that stronger invariant because batch kernels read those bytes in place.

### Direct prepared-bulk calls: retained existing API

These are synchronous operations on work already ready to hash. They retain direct kernel paths, without forcing inputs through pipeline setup or completion callbacks.

| Input | Caller-thread call | Multithread-capable call |
|---|---|---|
| One aligned message: `&[Block]` and logical length | `throughput::hash` | `throughput::hash_multithreaded` |
| Equal-length messages: `&Batch` | `throughput::hash_many` | `throughput::hash_many_multithreaded` |
| Mixed lengths: `&[(&[Block], usize)]` | `throughput::hash_each` | `throughput::hash_each_multithreaded` |

Each call takes `Mode`. The multi-result calls write into the caller's output array.

#### One million 32-byte messages

A direct prepared-batch operation on `Packed<32>`, the messages end to end as the program holds them, on a 128-byte boundary the type guarantees (Sam's audit on #275; Zooko: 16- and 32-byte leaves will be common):

```rust
use blake3_servil::{Mode, throughput::{self, Packed}};

blake3_servil::initialize_multithreaded(None); // Once per program.
const COUNT: usize = 1_000_000;
let mut leaves = Packed::<32>::new(COUNT);
for i in 0..COUNT {
    produce_message_into(&mut leaves[i]); // Writes the 32 bytes in place.
}
let mut digests = Packed::<32>::new(COUNT); // The next Merkle layer's input.
throughput::hash_many_multithreaded(Mode::Plain, &leaves, &mut digests);
```

`produce_message_into` is application code. The batch occupies 32,000,000 bytes and the digests the same; a layer's digests, two by two, are the next layer's parents (`digests.as_blocks()` as a `Batch` of 64-byte messages). Measured (bench-hashes' Merkle cells, M4 Max): 3.9 ns a leaf over the cores, 26 ns on one thread, against 75 for one `keyed_hash` call each; a kernel that reads the packed form itself is #294. A `Batch::new(32, COUNT)` of blocks remains for messages of any length, at 64 bytes a message.

For caller-thread hashing, replace the final call with `throughput::hash_many(Mode::Plain, &batch, &mut digests)`. Convenience exposes ordinary byte-oriented hashing. These batch operations belong to throughput.

Already-aligned whole storage can also be borrowed without copying:

```rust
// aligned_whole_storage holds complete blocks, unlike packed 32-byte records.
let blocks = Block::from_bytes(aligned_whole_storage)
    .expect("the producer supplied aligned, whole-block storage");
let digest = throughput::hash(Mode::Plain, blocks, logical_message_bytes);
let nodes = Batch::from_blocks(blocks, 64); // Complete 64-byte messages.
```

`Batch::from_blocks` is a zero-copy read-only borrow when messages fill whole blocks. Throughput hashing retains prepared typed inputs.

### Pipelined prepared batches: the admission form

The pipeline also accepts ready work in its prepared representation. The async form waits for admission, then returns without waiting for hashing:

```rust
stream.submit_batch(batch, batch_places, batch_tag).await;
```

Here `batch` (a `Batch<'static>`: owned, or borrowing blocks that live for the program) and `batch_places` (a `Box<[[u8; 32]]>`, one place per batch message) go in by value: the pipeline owns both while its workers write the digests. Both come back in the batch's completion, `Event::Batch { tag, batch, places }`, the producer's again to refill and submit. A pipeline that is forgotten (`mem::forget`) leaks them; a borrow would free them under the workers (#309). A producer can choose bounded groups independently of the million-message direct-call example. A whole batch retains its compact layout and group completion.

The exact accounting of batch admission against stream limits remains provisional. Preserving the efficient batch representation is a requirement. Input ownership and output lifetimes are part of the review below.

## Convenience

```rust
let mut hasher = convenience::Hasher::new(Mode::Plain, None);
hasher.update(first_piece);
hasher.update(second_piece);
let digest = hasher.finalize(); // takes the Hasher: a new one for the next message
```

The optional hint estimates the whole message's length. Actual bytes determine the digest. `update` returns with its input incorporated. The caller can immediately reuse that input. `finalize` consumes the Hasher and returns the digest synchronously, the state finished in place with no copy; a digest of a prefix with hashing going on after it is `hasher.clone().finalize()`, the copy paid where it shows. There is no parallel-update or background-submit variant here.

One-shot hashing, keyed hashing, key derivation and extended output remain convenience operations. `Mode::Plain` means ordinary BLAKE3. A keyed or key-derivation mode uses the same incremental interface.

## Receive into the throughput pipeline

Program setup calls `initialize_multithreaded` once. Its configured thread count includes the producer. Background execution requires at least one worker besides it. The library chooses kernels and GPU use within the execution budget.

Inside an application's async receive routine, `reader.read_fragment(room)` writes into `room` and returns `(bytes_written, message_ended)`. This is an **application-supplied helper**. The application's protocol defines that message boundary.

```rust
use blake3_servil::{Hash, Mode, throughput};
use blake3_servil::throughput::{Completion, Limits};

let stream = throughput::Stream::new(
    Mode::Plain,
    Limits { buffered_bytes: 2 << 20, messages: 32 },
    |completion| match completion {
        Completion::Hashed { tag, hash } => println!("{tag}: {hash}"),
        Completion::Aborted { tag } => eprintln!("{tag}: aborted"),
    },
);

let mut message = stream.message(7u64, None).await;
let read_result = loop {
    let mut input = message.input().await;
    let ended = match reader.read_fragment(input.bytes_mut()).await {
        Ok((written, false)) => input.commit(written),
        Ok((written, true)) => {
            // The read learned the bytes and the end together: the last
            // part goes to a worker tagged last, at once.
            drop(input);
            message.end(written);
            break Ok(());
        }
        Err(error) => {
            // The borrow ends here; abort returns the room.
            message.abort();
            break Err(error);
        }
    }
};

stream.finish().await;
read_result?;
```

The tag is caller data, returned unchanged with its completion. An incremental message's digest comes with its completion, by value: the producer lends no output place. `buffered_bytes` bounds retained and reserved input, including writable grants. `messages` bounds messages whose output places have yet to return. These limits are stream-wide.

`message.input().await` grants writable library-owned storage **and its submission capacity**. Available capacity completes immediately. Otherwise the operation registers once and the application handles other events. Space belongs to that request before its waker is notified.

`input.commit(written)` consumes the writable guard. It records the written bytes, submits complete prepared portions and preserves a partial portion for the next callback. It returns without waiting for hashing. A zero-byte commit records no bytes. `end(written)` records the message's last `written` bytes (at the start of the current room's free part) and declares it complete, so that part reaches a worker tagged last; `end(0)` serves a read that meets the end on its own. The `Input` guard owns nothing: dropped unused, its room stays with the message for the next callback.

`Input::bytes_mut` is a controlled writer into library-owned storage, not a hashing call accepting arbitrary byte slices. Its preparation helpers establish alignment, whole-unit boundaries and the final zero tail. Workers receive immutable prepared input. They hash it in place. A filesystem read or socket receive can fill that storage directly, with no intermediate input buffer.

`message.end(n)` consumes the write handle and seals its final portion. `abort()` also consumes the handle, produces no digest and returns storage when active jobs release it. `finish().await` consumes the stream and requires all messages ended or aborted. It waits for accepted work and invokes remaining completion handlers on the caller's thread.

### Crash-only lifecycle

Every lifecycle step is an explicit call: `commit`, `end`, `abort`, `finish`. No handle does that work in `Drop`. Dropping a `Message` or granted `Input` with work accepted, or unwinding through a stream scope, is a broken contract: the process ends at once, since workers may still be reading or writing that memory. This is the rule the current `stream` already follows. A prototype that cleaned up in `Drop` let a panicking producer free a frame a worker was writing; the explicit form removes that path.

Async cancellation of accepted work means calling `abort()` (or letting `finish` drain); dropping a future that holds accepted work ends the process. A pending `message()` or `input()` request holds nothing yet: dropping it, as a timeout or `select!` does, is an ordinary cancellation, and the next waiter is served.

## Several unfinished messages

One producer owns each stream. Its methods use shared access, so one message's write handle leaves the stream available to other messages.

```rust
let mut a = stream.message(1u64, None).await;
let mut b = stream.message(2u64, Some(3)).await;

let mut input = b.input().await;
input.bytes_mut()[..3].copy_from_slice(b"abc");
drop(input);
b.end(3);

stream.ready().await;
stream.collect(); // Can deliver message 2 while message 1 is unfinished.

a.abort();
stream.finish().await;
```

`ready` notifies the producer about completed work. `collect` invokes handlers for ready completions without waiting. Workers signal readiness and compute hashes. Completion handlers run on the producer's thread: when it collects or finishes, and while a `message()` or `submit_batch()` call waits for admission (a message slot returns through its completion, so the wait delivers what is ready, as today's stream hands places back while it waits for room).

## Provisional surface

These are Rust-style operation signatures. Private fields, exact lifetime parameters and trait bounds are omitted. This block is notation for review, not a compilable API declaration.

```text
// Existing direct prepared-input calls remain as listed above.
Stream::new(mode, limits, on_completion) -> Stream
Stream::submit_batch(&self, batch: Batch<'static>, places: Box<[[u8; 32]]>, tag)
    -> Future<Output = ()>             // Admission, not hash completion; both
                                       // come back in Event::Batch.
Stream::message(&self, tag, length_hint)
    -> Future<Output = Message>      // Its digest arrives with its completion.

Message::input(&mut self) -> Future<Output = Input>
Input::bytes_mut(&mut self) -> &mut [u8]
Input::commit(self, written: usize)
Message::end(self, written: usize)
Message::abort(self)

Stream::ready(&self) -> Future<Output = ()>
Stream::collect(&self) -> usize
Stream::finish(self) -> Future<Output = ()>
```

The futures use Rust's standard polling/waker interface. BLAKE3-Servil chooses no async runtime. A callback adapter can arrange notification through an event loop. The blocking reader adapter waits internally on the same resource requests.

## Existing prepared inputs and blocking readers

The current `Stream::submit_reader`, `submit_with` and `submit_batch_with` already read or generate directly into library-owned storage. Direct-fill message and batch adapters stay in the design. The new capability is preserving one unfinished message across separate producer turns, with capacity waiting supplied by the library.

The grouped incremental Hasher operations (`update_each`, `update_each_multithreaded`, `finalize_each`) already have this split's shape, throughput functions over convenience Hashers, and stay as they are; `finalize_each` finishes and resets the ended hashers.

The public C API is convenience-only in this proposal. Prepared bulk inputs, the background pipeline and library-managed multithreading remain Rust interfaces.

## Feedback requested

1. Do the retained typed bulk calls and the `submit_batch` form cover your ready-work flow? Is `input().await` → write → `commit(n)` clear for incremental production?
2. **Output ownership:** incremental messages hand their digest over by value with the completion (the pipeline owns the result; one 32-byte copy), while batches move their digest places in and get them back with the batch (#309). Does that split fit your programs?
3. Are the byte/message budgets and advisory total-length hint enough to predict and control resource use? An incorrect hint changes neither bytes consumed nor end-of-message behavior.
4. Which concrete receive/generate workflow would make you choose this pipeline over the convenience Hasher or direct prepared-bulk calls?

The public Rust signatures will be settled after this review and implementation checks. The sketch makes no speed claims.
