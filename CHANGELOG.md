# blake3-servil 0.9.1+b01b322ad8c411133e9abc49605e65e1a9684924, against blake3-servil 0.8.0

What changed since 0.8.0, each item with where it is described. The
figures come from checks of this code against 0.8.0, on the machines
each names.

## Two crates, and no streaming API in this release

- **`blake3-servil-core`** is BLAKE3-Servil on one thread: every call
  hashes on the thread that makes it, and the crate starts no thread. Its
  assembly kernels are each proved against the BLAKE3 specification.
  Without its `std` feature it needs no allocator.
- **`blake3-servil`** is that crate, whole, with a pool of threads and a
  multithreaded form of each call: `hash_multithreaded`,
  `hash_many_multithreaded`, `hash_each_multithreaded`,
  `update_multithreaded`, `update_each_multithreaded`.
- **0.9 has no streaming API.** 0.8's stream (`performance::stream`) is
  gone, and one streaming API, the Pipeline, is being built to replace it
  after 0.9 (#356). A program that feeds 0.8's stream waits for the
  Pipeline's release and stays on 0.8 until then, knowing the stream's
  two faults there (below, "Safer"). Where a program can do without a
  stream, 0.9's calls take whole inputs: `hash_multithreaded` (one
  message), `hash_many_multithreaded` (a batch), `hash_each_multithreaded`
  (messages of different lengths), `update_multithreaded` (one message
  fed in pieces). Used in a stream's place they are slower ("Slower",
  below).

## What a program on 0.8 changes

- **`performance` is `throughput`**: the module's name; its calls keep
  theirs.
- **The `Hasher` is `convenience::Hasher`** (it was `performance::Hasher`):
  - `Hasher::new(mode)` is `Hasher::new(mode, length_hint)`, the hint an
    `Option<u64>`; `None` is always right;
  - `finalize` and `finalize_xof` take the hasher (`self`, where they
    took `&self`);
  - `count` and `reset` are gone;
  - the method `hasher.update_multithreaded(input)` is the call
    `throughput::update_multithreaded(&mut hasher, input)`.
- **`finalize_each` takes `&mut [Hasher]`** (it took `&[Hasher]`): each
  hasher it finishes starts again.
- **The stream is gone**: `performance::stream`, `Stream` and its seven
  methods.

Unchanged for a caller, though the signature reads differently:
`hash_many` and `hash_many_multithreaded` take any `Prepared` input, and
`&Batch` is one. New: `Packed<16>` and `Packed<32>` (the other `Prepared`
input) and the constant `BUILD_VERSION`. `Hash`, `Mode`, `Block`,
`OutputReader`, the three `convenience` calls, `context_key` and the
other `throughput` calls read the same in both. A program that depends
on `blake3-servil` keeps the crate's name: it re-exports everything the
core documents and adds the multithreaded calls and
`initialize_multithreaded`.

## The API: one call family for a hash now and then, one for call after call

- **`convenience`** is for a hash amid other work: `hash`, `hash_with`,
  `hash_xof`, and now the incremental `Hasher`. Its paths are tuned for
  code that has left the caches (on AArch64, its kernels' code is
  prefetched). The length hint is a scheduling hint and never part of the
  hash.
- **`throughput`** (the `performance` module of 0.8, renamed) is for call
  after call: `hash`, `hash_many`, `hash_each` and their multithreaded
  forms over prepared inputs (`Block`, `Batch`, `Packed`), and grouped
  incremental updates (`update_each`, `finalize_each`). A program chooses
  the pattern by the call it makes; no clock guesses it.
- **The C API is the convenience API**: `b3_hash`, `b3_hasher_*` (with
  the length hint) and the modes.
- **A build names its code** (#328): `BUILD_VERSION`, `b3_version()` and
  `b3sum --version`. A release reads its version; the code after one reads
  `X.Y.Z+dev` with its commit, and a dirty tree adds its Git tree id.

## Slower

**Without a stream: messages hashed one after another on many threads.**
This is what a program would see if it used 0.9's calls where it used
0.8's stream; a program that needs a stream waits for the Pipeline.
0.8's stream hashed on the workers while the program read the next
message, gathered short messages into lanes and spread them over the
cores. A call a message does none of that. Against 0.8's stream, on four
x86-64 machines (a Ryzen 9700X, an EPYC 9275F, an EPYC 9255, two Xeon
Gold 6330), messages one after another through `hash_multithreaded` take
from 1.2 to 25 times the time: the most for messages of 512 B and 1 KiB
on the 16-thread Ryzen, 2 to 3 times for messages of 16 KiB and more
there. Gathering short messages into batches for
`hash_many_multithreaded` recovers most of it: batches one after another
take 1.2 to 2.9 times the stream's time. No other call is slower on
those machines: none of the single-threaded calls, none of the
multithreaded calls made for whole inputs. (Four pairs of runs a cell.)

**On purpose, on AArch64: a `throughput` call made now and then.** 0.8
guessed with a clock whether a call came after a pause, and prefetched
its kernels' code when it did. 0.9 has no such clock (#253): the call
decides. So on AArch64 a `throughput` call made amid other work, where
0.8's clock would have prefetched, is slower than 0.8:

- `hash_multithreaded` of one message of 1.5 to 16 KiB;
- `hash_many` over a batch of 2 to 8 short messages, on one thread or many.

The remedy for one message is the call made for that pattern,
`convenience::hash`, which prefetches on every call of 1 to 64 KiB. A
small batch amid other work has no such call yet. Call after call, both
are as fast as before or faster. On an Apple M6, made amid other work,
the batch takes 37 to 180% more time than 0.8's and the message 28 to
115% more (#364).

**Cause not yet known, on AMD Zen 5: `convenience::hash` of 4 KiB made
after other work** takes 5 to 6% more time than 0.8 on a Ryzen 9700X, an
EPYC 9275F and an EPYC 9255 (#376). The same call on the lengths beside
it is faster than 0.8 there (3 KiB by 28 to 30%).

## Faster

- **On x86-64, whole chunks and parents round up to the lanes** (#248,
  #345): 3 inputs run on 4 lanes, 5 to 7 on 8, 9 to 15 on 16 and 17 to 31
  on 32 where those kernels run, each in one pass where the kernels took
  two or three; the spare lanes hash a copy whose result is dropped. One
  message of 3 KiB to 7935 B takes 24 to 45% less time on a Ryzen 9700X.
- **On x86-64, packed leaves and a batch's messages of 1 to 63 bytes
  run eight at a time on AVX2 and sixteen on AVX-512** (#297), on the
  kernels `finalize_each` uses: a single-threaded Merkle tree over 16- or
  32-byte leaves takes 74 to 78% less time on a Ryzen 9700X and an EPYC
  9275F (sixteen lanes), and 45 to 57% less on two EPYC 7443 (eight). One
  cell pays, on that last machine alone: its multithreaded tree of
  1,048,576 leaves of 32 B, on 96 threads, takes 6% more (#372).
- **`Packed<16>` and `Packed<32>`**: a batch of 16- or 32-byte messages
  (Merkle leaves, digests) hashed without the program writing each into a
  block of its own first (#289).
- **`finalize_each` finishes sixteen hashers at once on AVX-512**, each
  with its own chaining value and flags, on a proved kernel; its indices
  are distinct, a contract a debug build checks (#295).
- **On AArch64, many short messages fed in small pieces finish side by
  side** (#298): 1,024 open messages of 1 KiB arriving in 256-byte pieces
  take 68 to 71% less time a cycle on an M4 Max. Plain messages alone: a
  keyed hasher's key is secret, so it finishes by itself as before.
- **A Hasher's in-place checkpoint copies only the bytes it has staged**
  (`clone_from`), not its whole 16 KiB stage (#283).

## Steadier

- **After preparation, the pool's calls allocate nothing** (#325): a
  whole message, a batch, a collection of chunk-sized items, and repeated
  update-and-finish cycles of 4 MiB messages in every mode, on the calling
  thread and the workers, the first cycle included.
  `initialize_multithreaded` prepares the calling thread's scratch (48 KiB,
  for inputs to 64 MiB).
- **A repeated collection allocates nothing** (#325): `hash_each` keeps
  its messages' places and chunk values on the calling thread between
  calls, where each call took them and freed them.

## Safer

Where an entry says 0.8 has the fault, the remedy for a program on 0.8 is
this release: no 0.8.x follows.

- **0.8's stream has two faults, and 0.9 has no stream.** In 0.8 a worker
  claimed a task in the stream's ring and then copied it, and a worker
  delayed between the two could copy a later task written into the same
  slot (#302). And `performance::Stream::open` is public, safe and hidden
  from the docs: a safe program could open a stream, hand it borrowed
  input, forget it (`mem::forget`) and free the input while the workers
  still read it (#308). The remedy for a program on 0.8's stream is the
  Pipeline's release, which follows this one; 0.9's one-shot calls are a
  remedy today only where the program can do without a stream.
- **The C header says what a finished hasher takes** (#311): after
  `b3_hasher_finalize`, only `b3_hasher_new`. A finished hasher is not
  reset, and an update on it silently continued the old message. Official
  BLAKE3's hasher may be finalized again and updated further; code ported
  from it calls `b3_hasher_new` between messages.
- **`Hash`'s hex is computed, with the same time and memory accesses
  for every hash** (#370). A hash can be a secret (a keyed hash's tag, a
  derived key). 0.8's `to_hex` looked each character up in a table
  indexed by the hash, and its `from_hex` stopped at the first bad
  character. Both now work by arithmetic over all 64 characters. Checked
  on the compiled code: `to_hex` has one path and no leak on x86-64 and
  AArch64; `from_hex`'s accepted path is clean after its first branch on
  AArch64, and on x86-64 the tool's exploration is incomplete, so that
  half rests on a reading of the compiled code.
- **`Batch::new` stops on a count whose blocks overflow** (#320). In 0.8,
  in a release build, a count of 128-byte messages past half of
  `usize::MAX` wrapped to zero blocks and returned an empty batch.
- **`update_each` groups held hashers by public facts alone**, the mode
  and the lengths, never by a key (#312). 0.8 grouped them by key.

## Checked more

- **Mutation testing on a budget**: a change's check covers its own lines
  in about a minute; the whole run, every mutant, gates each release.
- **Every speed-only mutant earns a test that counts its work**, with no
  clock, or its code goes; the pool's clock-bound knobs read a test clock.
- **The tests split in two**: the code's, on fixed inputs and the test
  clock, under two seconds; the machine's (clocks, counters, sleeps) by
  their own profile.

# blake3-servil 0.8.0+b3c5efc7c975d903762dea809066f7c0a0603861, against official BLAKE3 1.8.7

The same digests. Faster than official BLAKE3 1.8.7 on 64-bit Arm and
x86-64. Every kernel is assembly, and every one is proved correct
within its stated contract. Everything else is tested, and the tests
are tested ([QUALITY.md](QUALITY.md)).

## Faster

Each figure is blake3-servil's speed over official BLAKE3 1.8.7's, both
measured in the same run by [bench-hashes](https://github.com/johnservil/bench-hashes)
0.18.1 at this release's commit (one run per machine, October 27).
On the every-core rows each uses its multithreaded function:

| | Intel i7-12700K | AMD EPYC 9255 | Apple M4 Max | Linux VM on the M4 Max |
|---|---|---|---|---|
| One message of 64 B | 0.8× | 1.1× | 1.5× | 1.1× |
| One message of 1 KiB | 1.1× | 1.1× | 1.2× | 1.2× |
| One message of 1 MiB, one thread | 1.0× | 1.2× | 2.5× | 2.5× |
| One message of 128 MiB, every core | 1.1× | 1.0× | 1.3× | 1.3× |
| A stream of 64 KiB messages, every core | 8.0× | 7.2× | 33× | 156× |
| A batch of 1024 messages, 64 B each | 1.1× | 1.1× | 1.9× | 1.8× |
| b3sum over 1000 files of mixed sizes | 3.4× | 5.8× | 4.6× | 26× |

On the M4 Max and in the VM, official's multithreaded call on each 64 KiB
message takes longer than its one-thread call (1.08 and 7.13 ns per byte,
against 0.42 and 0.43), so the stream row's figures there are large.

On 64-bit Arm, new kernels make every size faster: SME2 on the M4 and
later, and kernels that run the integer and NEON units side by side.
On x86-64 with SSE4.1 or AVX2, a single message hashes with official's
own assembly (each proved below), so it runs at official's speed (one
message of 64 B varies from run to run: 0.8× to 1.9× on the i7). On Linux on AMD Zen 5 and later,
new kernels hash 32 whole chunks or 16 parent nodes at once with AVX-512
(proved too): on the EPYC 9255, one thread hashing 32 KiB or more takes
about 20% less time than with upstream's 16-input AVX-512 kernel. Other
AVX-512 CPUs keep the 16-input kernel, which measured faster there (Zen 4:
the wide kernels 4-5% slower from memory; Intel Ice Lake: 3-4.5% slower
throughout). The other gains on x86-64 come
from many messages at once: a stream that hashes on other cores while
the program reads the next input, batches of short messages, and b3sum
over many files.

## Checked more

Beyond official's tests (published test vectors and a reference
implementation), this crate adds:

- **Formal proofs of every assembly kernel**, official's own x86-64
  assembly among them, at every shape this crate calls it: symbolic
  execution against a specification of BLAKE3, with memory accesses,
  the calling convention and secret-dependent branches checked too.
- **Mutation testing**: one small mistake planted at a time, more than
  2,000 in all, and a test must fail for each; every one that survives
  is listed with its reason.
- **A model checker (Shuttle)** for the thread pool's waits, under
  thousands of thread schedules.
- **Guard pages**: tests put an inaccessible page beside every input,
  output, key and context of every public call, in Rust and in C, so
  a call that touches one byte past the caller's memory crashes the
  test.
- **A startup self-test** of the build and the CPU before the first
  hash.

These found eight bugs before release, among them the AVX2 and AVX-512
batch kernels reading past an input's end at shapes this crate's batch
calls use (fixed here; see below). [QUALITY.md](QUALITY.md) says what
each check covers and what remains unproved.

## Before you switch

1. Targets: 64-bit Arm on Linux and macOS, x86-64 on Linux. Latest
   stable Rust and a C compiler, GCC 12 or Clang. Official BLAKE3 keeps
   Windows, 32-bit CPUs, big-endian CPUs, WebAssembly and older compilers;
   a build for any other target stops and names it.
2. Call `blake3_servil::initialize()` once before any other call. A
   program that makes multithreaded calls or uses a stream calls
   `initialize_multithreaded(None)` instead. A call before it panics.
3. Two kinds of call, in two modules. `convenience`: one call per hash,
   every feature, any slice, the result returned: `hash(input)` is now
   `convenience::hash(input)`, `keyed_hash(key, input)` is
   `convenience::hash_with(Mode::Keyed(key), input)`, `derive_key(context,
   material)` is `convenience::hash_with(Mode::DeriveKey(context),
   material)`, and `convenience::hash_xof(mode, input)` gives extended
   output. `performance`, for power users, has every way to hash fast.
   There the program takes on more of the work: inputs as `Block`s (64
   bytes on a 64-byte boundary: a message is blocks and a length, a
   batch messages of one length), digests into its places, and a `Mode`
   in every call. The `Hasher` is there too: `Hasher::new(mode)` replaces
   `Hasher::new()`, `Hasher::new_keyed` and `Hasher::new_derive_key`, the
   plain hash is `Mode::Plain`, and `Hasher` has no `Default`.
4. Threads are the crate's own. `Hasher::update_rayon` is now
   `update_multithreaded`. `performance::hash_multithreaded(mode, input)`
   hashes one message over the threads. There is no `rayon` feature.
5. Removed: `Hasher::update_mmap` and the `mmap` feature (map the file
   yourself, or use `update_reader`), the `traits-preview` feature, the
   `guts`, `hazmat`, and `platform` modules, and the `no_avx2`,
   `no_avx512`, `no_neon`, and `no_sme2` features, and `pure`: this crate
   runs its assembly or not at all. For portable Rust, or an x86-64 CPU
   without SSE4.1 (which stops at `initialize`), use official BLAKE3.
6. b3sum installs from the package `b3sums` (`cargo install b3sums`);
   crates.io's `b3sum` is official BLAKE3's. The program is still `b3sum`.
   `--num-threads N` means at most N threads in all, and 1 means one
   thread. `--no-mmap` does nothing. Every file is read to its end. A
   broken pipe ends b3sum quietly.

## New

- `performance::hash_many` and `hash_many_multithreaded`: a `Batch` of
  messages of one length, each in `Block`s (64 bytes on a 64-byte
  boundary). A `Batch` lets the program write only its messages' bytes,
  so the bytes past each message, which the batch kernels read in place,
  stay zero; a Merkle layer writes its digests straight into the next
  layer's `Batch`.
- `performance::hash_each` and `hash_each_multithreaded`: a collection of
  any lengths.
- `performance::update_each` and `finalize_each`: many messages in
  progress at once, each with its own `Hasher`.
- `performance::stream()`: inputs one after another, hashed behind the
  caller.
- A C API in `capi/b3.h`. The library is `libb3_servil`. C11 and C++11.
- `Mode::DeriveKeyWith` with `performance::context_key`: one context
  hashed once for many derivations.

## Different

- A startup self-test checks the build and the CPU. A wrong digest stops
  the process, uncatchably: with `std` it names the code path and aborts;
  without it, the CPU's trap instruction.
- A panic on one of the crate's threads, or in a caller during a
  multithreaded call, ends the process.
- The threads are the process's: in a child forked after they started,
  every multithreaded call and stream panics, saying so. A program that
  forks calls `initialize_multithreaded` in the process that hashes,
  after any fork; single-threaded calls work anywhere.
- A `Hasher` is about 18 KiB. It gathers short updates and hashes them
  side by side, so a message arriving in small pieces hashes as fast as
  one in a buffer.
- The AVX2 and AVX-512 batch kernels read nothing past an input's end.
- The crate leaves Arm's data-independent timing (DIT) as the calling
  thread has it. A program that wants Arm's promise turns DIT on in its
  own threads before it hashes (the quality manual, "Timing and secrets":
  [docs/quality-manual.md](docs/quality-manual.md)).
- Building the Arm kernels needs an assembler that knows armv8.2-a+sha3,
  GNU as 2.30 or LLVM 7; building the x86-64 kernels needs a C compiler
  that takes the AVX-512 flags, GCC 12 or Clang. Without one, the build
  stops and says so; official BLAKE3 builds without assembly. The SME2 kernels need Clang 17
  or GCC 14 with binutils 2.41; without those the build warns and leaves
  them out.
