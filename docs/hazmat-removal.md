# The threads crate builds its hashes with core

`blake3-servil-core::hazmat` is gone. The two crates use checked tree and
batch operations rather than sharing keys, flag words, raw chaining
values, or raw compression calls.

- `TreeHasher` prepares a mode once. The pool, Pipeline, and `b3sum`
  carry `Cv` and `LastCv`: each whole value carries its level and index,
  and the right edge carries its position and length. The joins check
  these places. A root checks the submitted message length separately.
- `TreeHasher::subtree_halves` gives two checked whole values. The
  Hasher's `update_with` takes a
  borrowed callback for large subtrees. The Hasher checks both places
  before its stack takes their bytes. The threads crate uses these calls
  for its own multithreaded updates.
- The Pipeline hashes a room or loan on the kernels its worker was
  given. Whole rooms and slice pieces share the checked fold. Its short
  groups build typed lanes for equal whole-block messages and use the
  same kernel call as a batch. The rest hash one at a time. A group
  carries its Pipeline's prepared mode by reference.

## Choosing workers and kernels

`thread_support` is the small, hidden interface for the worker scheduler.
It supplies CPU detection, each core's kernels, the accelerator turn,
and batch and thread-storage setup. Only core can construct a platform
value, after it asks the CPU. It gives no access to compression or to a
mode's key words and flags.

The scheduler uses the accelerator turn to choose a pool's SME2 prefix
and to keep a Pipeline job on the kernels it was assigned. `TreeHasher`'s
worker calls use that kernel choice without taking a second turn, while
keeping the tree checks and all input checks save `hash_slots_on`'s
trust that its caller zero-padded the blocks (#458). Normal callers use the documented methods; a
caller that already distributes its own work can use `on_this_core`.

The remaining question is whether the scheduler can leave more of the
accelerator choice inside core without slowing down the pool or the
Pipeline on Apple. Compare that separately. On other CPUs there is no
shared accelerator to acquire.
