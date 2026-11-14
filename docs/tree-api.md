# A tree API, and the end of `hazmat` (design draft)

*A draft for BLAKE3 #378, November 14, 2026. Layer 1 is built
(`throughput::TreeHasher`), with one client, `b3sum --outboard`; the threads
crate is not yet moved onto it, and layers 2 and 3 are not built. The
draft proposes a small public API for programs that build or verify a
BLAKE3 tree themselves, and uses the library's own threads crate as a
client, so that the hidden `hazmat` module can be deleted.*

## Who needs it

**TreeHasher builders.** bao-tree (verified streaming, used by iroh) calls two
things from official BLAKE3's `hazmat`, in the plain and keyed modes:

- the hash of a subtree: `start_chunk`, its bytes, and whether it is the
  root (`set_input_offset`, `update`, `finalize_non_root`; or `hash` for
  the root);
- the hash of a parent: two children, and whether it is the root
  (`merge_subtrees_non_root`, `merge_subtrees_root`).

That is all of it (bao-tree `src/lib.rs`, `HashMode::hash_subtree` and
`HashMode::parent_cv`).

**Our own threads crate.** `blake3-servil` (the pool, the multithreaded
calls, the Pipeline) reaches the core through `hazmat`, a public module
hidden from the documentation. Counting its uses in `threads/src` at
`c5d8331c`, they fall into three groups:

| Group | Items | What they are |
|---|---|---|
| TreeHasher pieces | `hash_all_at_once(..).chaining_value()`, `compress_subtree_to_parent_node`, `parent_output`, `compress_parents_parallel`, `tree::left_len`, `CVBytes`, `CVWords`, `IV`, the flag constants | A subtree's chaining value, a subtree's two halves, a parent, a layer of parents, where a message splits. |
| The key and the kernels | `Mode::key` (key words and flags), `Platform` (about 40 uses), `Sme2Turn`, `pool_platform` | The mode, prepared once, and which kernels a call runs on. |
| Hooks into the core's own calls | `incremental::Pool` and `Hasher::update_on`, `each::update_turn`, `each::hash_each_on`, `many::hash_many_on`, `MEMBERS`, `prepare_thread`, `slot_len`, `assert_slots` | The core's Hasher, batch and collection calls with their large pieces handed to the pool. |

The first group is what bao-tree needs. So one API can serve both, and
the threads crate tests it before any outside program depends on it.

## Layer 1: the tree's pieces

In `blake3-servil-core`, in `throughput`:

```rust
/// A subtree's chaining value: 32 bytes that are not a hash of anything a
/// program can check alone. Its own type, so that it is never taken for one.
pub struct Cv([u8; 32]);
impl Cv {
    pub const fn as_bytes(&self) -> &[u8; 32];
    pub const fn from_bytes(bytes: [u8; 32]) -> Self;
}

/// A mode prepared for tree work: its key and flags derived once.
pub struct TreeHasher { /* key words, flags, kernel choice */ }
impl TreeHasher {
    pub fn new(mode: Mode) -> Self;

    /// The chaining value of the subtree whose first chunk is `first_chunk`.
    pub fn subtree_cv(&self, first_chunk: u64, input: &[u8]) -> Cv;
    /// The same subtree as its two halves' chaining values (more than one chunk).
    pub fn subtree_halves(&self, first_chunk: u64, input: &[u8]) -> [Cv; 2];

    /// The parent of two subtrees.
    pub fn parent_cv(&self, left: &Cv, right: &Cv) -> Cv;
    /// A layer of parents: `out[i]` from `children[2 * i]` and `children[2 * i + 1]`.
    pub fn parent_cvs(&self, children: &[Cv], out: &mut [Cv]);

    /// The root, from its two children: the message's hash, or its extended output.
    pub fn root_hash(&self, left: &Cv, right: &Cv) -> Hash;
    pub fn root_output(&self, left: &Cv, right: &Cv) -> OutputReader;
}

/// The bytes of the left subtree of a message, or a subtree, of `len` bytes
/// (more than one chunk): the largest power of two of chunks below its count.
pub const fn left_len(len: u64) -> u64;
```

A message of one chunk or less has no parent: its root is
`convenience::hash_with`.

**Why `TreeHasher`, not free functions taking `Mode`.** `Mode::DeriveKey` hashes
its context string to make the key. A tree builder calls these thousands
of times a message, and the threads crate once per piece, so the mode is
prepared once. It also keeps key words, flag constants and kernel names
out of the API: those stay the library's.

**Contracts**, in the library's style: a broken one stops the program.

- `subtree_cv` and `subtree_halves` take a whole subtree: `input` is not
  empty, starts at chunk `first_chunk`, and holds no more bytes than the
  subtree aligned there can (`first_chunk` zero: any length).
- `subtree_halves` takes more than one chunk.
- `parent_cvs` takes an even number of children and half as many outputs.
- Nothing secret chooses a branch, a group or a buffer size: the work
  depends on lengths and on the mode's kind alone.

**bao-tree's two calls on it:**

| bao-tree | Here |
|---|---|
| `hash_subtree(start, data, is_root = false)` | `tree.subtree_cv(start, data)` |
| `hash_subtree(0, data, is_root = true)` | `convenience::hash_with(mode, data)` |
| `parent_cv(l, r, is_root = false)` | `tree.parent_cv(l, r)` |
| `parent_cv(l, r, is_root = true)` | `tree.root_hash(l, r)` |

**The threads crate's uses on it:**

| Today, through `hazmat` | With layer 1 |
|---|---|
| `hash_all_at_once(bytes, key, counter, flags, platform).chaining_value()` | `tree.subtree_cv(counter, bytes)` |
| `compress_subtree_to_parent_node(bytes, 0, key, counter, flags, platform)` | `tree.subtree_halves(counter, bytes)` |
| `parent_output(l, r, key, flags, platform)` then `.chaining_value()` or `.root_hash()` | `tree.parent_cv(l, r)`, `tree.root_hash(l, r)` |
| `compress_parents_parallel(children, key, flags, platform, out)` | `tree.parent_cvs(children, out)` |
| `tree::left_len(len)` | `left_len(len)` |

### The one kernel choice a caller must be able to make

On Apple M4 and later one accelerator (SME2) serves the whole chip. A
call that hashes 16 KiB or more takes it when it is free. The pool
depends on choosing: its workers run on each core's own kernels (the NEON
hybrids), and one thread takes the accelerator for a share of the work
sized to its speed. Today it says so by passing a `Platform`.

A tree builder with workers of its own faces the same choice. So `TreeHasher`
carries it, named by what it means and not by a kernel:

```rust
impl TreeHasher {
    /// This TreeHasher's calls keep to the calling core's own kernels: for a
    /// program that runs many of them at once on different cores, where
    /// the chip's one shared accelerator would serve one and slow the rest.
    pub fn on_this_core(self) -> Self;
}
```

Without it, a call takes the accelerator when it is free, as
`convenience::hash` does. On every other CPU the two are the same. This
replaces every `Platform` the threads crate passes.

## Layer 2: a Hasher that takes its large subtrees from the caller

`update_multithreaded` and `update_each_multithreaded` are the core's
Hasher with one change: a subtree of 512 KiB or more is hashed by the
pool. Today the core takes a hidden `Pool` (a length and a function
pointer) through the hidden `Hasher::update_on`.

The same thing, public:

```rust
impl Hasher {
    /// `update`, with each whole subtree of `min_len` bytes or more
    /// hashed by `halves`: the call a pool makes on other threads.
    pub fn update_with(&mut self, input: &[u8], min_len: usize,
        halves: fn(&TreeHasher, first_chunk: u64, input: &[u8]) -> [Cv; 2]) -> &mut Self;
}
```

and its twin for a turn of many hashers. A GPU path (#170) is a second
caller of this.

## Layer 3: batches and collections

`hash_many_multithreaded` and `hash_each_multithreaded` cut a batch or a
collection into ranges and run the core's kernels on each. They need the
core's `hash_many` and `hash_each` over a range, on the calling core's
kernels. Whether that is `TreeHasher::on_this_core` reused, or the two calls
taking a range, is not designed yet. `MEMBERS`, `slot_len`,
`assert_slots` and `prepare_thread` go with whatever that becomes.

## What this deletes

With the three layers, `threads/` names nothing in `hazmat`, and the
module goes. Layer 1 alone removes the tree group and every `Platform`.
`hasher::subtree_len` is exported there and unused today: it can go now.

## Order of work

1. Layer 1 in the core, tested against official BLAKE3's `hazmat` at
   every length around a chunk, a group and a power of two, in each mode.
2. The threads crate moved onto it (`lanes.rs`, `pipeline.rs`): no cell
   slower on any machine. This is the test that the API is enough.
3. A second client: `b3sums --outboard`, which writes bao-tree's
   outboard for 16 KiB chunk groups (`<hash>.obao4`, the files Rüdiger
   Klaehn describes) beside each file's hash, through the Pipeline and
   io_uring path. Its layout is read from bao-tree's source first, and
   its bytes are checked against bao-tree's own.
4. Layers 2 and 3, then the deletion.

## Open

- **Names.** `TreeHasher` and `Cv` are settled (Zooko, November 14); `subtree_halves` and `on_this_core` are open.
- **`&[u8]` or `&[Block]`.** `throughput`'s other calls take 64-byte
  aligned `Block`s. A tree builder's subtrees sit at arbitrary places in
  a file or a network buffer, so this draft takes `&[u8]`.
- **Many subtrees in one call.** bao-tree hashes one chunk group (16 KiB)
  at a time. A call over many groups at once would let short groups
  share lanes. Not proposed until a measurement asks for it.
- **What the core's structure should make provable** (#371): the tree's
  pieces are where a proof of the Rust above the kernels would start.
  This layer should be cut where that proof wants it.
- **A prototype port** from bao-tree's side, before the API freezes at 1.0.

— John Servil
