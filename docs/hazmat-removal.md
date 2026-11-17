# Removing `hazmat`: what is left, and what each piece needs

A plan (November 16, 2026); the names at its end are decided. `docs/tree-api.md` was layer 1 and is landed: `throughput::TreeHasher`,
`Cv`, `left_len`. This is the rest.

**November 17 (BLAKE3 #421).** The public tree calls now take and give
values that carry their places (`Cv`, `LastCv`), and refuse a tree that is
not BLAKE3's. The threads crate's pool builds its trees with them: its
cut's rules, which were `debug_assert`s in the merge, are now kept by the
calls in every build. Two hidden items remain of the first form, both for
the Hasher's large updates (C below): `BareCv`, 32 bytes with no place,
and `TreeHasher::bare_subtree_halves`, what the pool's hook returns and
falls back to. `subtree_cv_on` takes a level and an index like
`subtree_cv`.

`blake3_servil_core::hazmat` is hidden (`#[doc(hidden)]`) and has one user,
the threads crate (`threads/`). It exists because that crate carries a
message's mode as a bare `(key, flags)` pair and names the kernels'
`Platform` itself. Nothing outside the two crates may use it, and the two
are released together, so it can change at will; the reason to remove it
is that a second implementation of a pool (Bao, iroh, anyone's) should be
able to do what ours does through the public API, and today cannot.

## What `threads/` takes from it

| From `hazmat` | Uses in `threads/src` | What it is for | Public replacement |
|---|---|---|---|
| `hash_all_at_once(..).chaining_value()` | 6 | a piece's subtree, off the root | `TreeHasher::subtree_cv` (landed) |
| `compress_subtree_to_parent_node` | 5 | a message's first piece, as its two halves | `TreeHasher::subtree_halves` (landed) |
| `compress_parents_parallel` | 2 | a layer of parents | `TreeHasher::parent_cvs` (landed) |
| `hasher::parent_output(..).root_hash()` | 3 | the root from two values | `TreeHasher::root_hash`, `root_output` (landed) |
| `tree::left_len` | 3 | where a message's left subtree ends | `throughput::left_len` (landed) |
| `hash_serial`, `hash_serial_on` | 8 | a whole message on one thread, the mode as `(key, flags)` | **new A** |
| `hash_many_serial`, `many::hash_many_on`, `many::slot_len`, `assert_slots` | 12 | a batch of messages on one thread | **new B** |
| `each::{hash_each_on, Item, MEMBERS, prepare_thread}` | about 8 | a collection of messages of any lengths, side by side | **new B** |
| `each::update_turn`, `incremental::Pool` | about 4 | many Hashers fed in one turn; a Hasher that hands large updates to a pool | **new C** |
| `platform::{Platform, Sme2Turn, MAX_SIMD_DEGREE}` | 18 and more | which kernels a thread runs, and the one SME2 unit's turn (Apple) | **new D** |
| `IV`, the flag constants, `CVWords`, `CVBytes`, `Output` | about 75 | the `(key, flags)` pair itself, and tests | gone with A |

## The four new pieces

**A. A mode, prepared once.** `TreeHasher` already holds what a mode comes
to (the key words and flags; for key derivation, the context's key, which
costs a hash to make). The threads crate would carry a `TreeHasher` where
it carries `(key, flags, platform)` today, and needs from it one more
call: the hash of a whole message.

    impl TreeHasher {
        pub fn hash(&self, input: &[u8]) -> Hash;      // name to decide
    }

It is useful outside the pool too: a program that hashes many messages
under one derive-key context pays for the context once. Question: is this
`TreeHasher`'s to have, or does it want a name of its own (a prepared
`Mode`)? `convenience::hash_with(mode, ..)` takes a `Mode` each call.

**B. Batches and collections on the calling thread.** The public
`throughput::hash_many` and `hash_each` exist; what the pool needs is the
same two for a part of a batch, in a prepared mode, with the output
slots it names. Probably two methods beside A's, taking the same
arguments the public functions take. `MEMBERS` (64, how many messages a
task holds) is the pool's own number and can live there.

**C. A Hasher that lends its large updates.** `incremental::Pool` is a
function pointer the multithreaded `update` passes down so that an update
of several megabytes is cut and hashed by the pool. The public form is the
hook Rüdiger asked about on #378: a `Hasher::update_with`-style call that
takes "hash these subtrees for me" from the caller. `each::update_turn`
(many Hashers, one piece each, side by side) needs the same hook.

**D. Which kernels, and the SME2 unit's turn.** On every machine but
Apple's SME2 ones, a thread runs the detected kernels and there is nothing
to choose. On those, one SME2 unit serves all cores: pool pieces run the
NEON kernels (`TreeHasher::on_this_core`, landed), and one thread at a time
takes the unit (`Sme2Turn`). The turn is the one item with no public shape
yet. Two ways: (1) keep it inside the core crate entirely, so that a call
"on the unit if it is free" is something `TreeHasher` decides and no
caller names; (2) publish the turn as a guard type. (1) is less surface
and I would try it first; it needs measurements on the Mac, which has
returned no verdict since November 14 (ops #14).

## Order I would take

1. The five landed replacements, in `threads/` (mechanical once A exists,
   since the call sites hold `(key, flags)`): no new surface beyond A.
2. A, then B: each removes a group of `hazmat` names and can be checked on
   the x86 machines and the Arm VM alone (the code Apple runs for them is
   the same NEON path).
3. C, with Rüdiger's use as the test of its shape.
4. D last, when the Mac measures again.

After 1 and 2, `hazmat` holds C's and D's names only.

## Decided

Zooko left the remaining names to me (November 16, 2026). They are:

- **A is a method, `TreeHasher::hash(&self, input: &[u8]) -> Hash`.** A
  `TreeHasher` is already "a mode, prepared": a second type that held the
  same three fields would be one mechanism too many. The whole-message
  hash is the tree's root, so it belongs with the tree's other pieces.
- **B's calls are methods beside it, `TreeHasher::hash_many` and
  `TreeHasher::hash_each`,** with the arguments of `throughput::hash_many`
  and `throughput::hash_each` less the mode. The two free functions stay,
  as the one-call forms.
- **C is `Hasher::update_with`:** an update that hands its whole subtrees
  to a function the caller gives, which returns their chaining values. Its
  exact signature waits for the code, with Rüdiger's use (#378) as the
  test of it; the name is settled.
- **D: the SME2 unit's turn stays inside the core crate** and gets no
  public type. A call says only `on_this_core` or not, as now. If the
  Mac's measurements show the pool needs more than that, D comes back
  with the numbers.

`first_chunk`, `Cv` and `on_this_core` keep the names layer 1 gave them.

## State

**Landed with this page's last change: A, B, and the pool's calls on them.**

- Core: `TreeHasher::hash`, `hash_many`, `hash_each`; `Prepared::part`,
  which gives some of a batch's messages as a batch (a `Part`), so a pool
  can hand each thread its share; a `TreeHasher` kept `on_this_core` runs
  its parents and root on the core's own kernels too.
- `threads/src/lanes.rs` and the multithreaded calls (`hash_multithreaded`,
  `hash_many_multithreaded`, `hash_each_multithreaded`,
  `update_multithreaded`) carry a `TreeHasher` and call only its public
  methods, with one exception below. Their short paths are the core's
  public one-thread calls.
- The free functions `throughput::hash`, `hash_many` and `hash_each` did
  not become "make a `TreeHasher` and call it" as the list above says:
  a `TreeHasher` holds a copy of its key words, and for the plain mode that
  copy is 32 bytes of stores a short call would pay (measured October 24).
  They keep their own way to the same kernels.
- C's hook has its shape inside the crates: the pool a Hasher's large
  updates go to is `fn(&TreeHasher, first_chunk, input) -> [Cv; 2]`
  (`incremental::Pool::halves`), which is `subtree_halves` over threads.
  `Hasher::update_with` would publish it.

**Left in `hazmat`:**

- D: `platform::{Platform, Sme2Turn, MAX_SIMD_DEGREE}`, and the hidden
  `TreeHasher::subtree_cv_on(platform, ..)`, for the caller that holds the
  SME2 unit's turn over a whole job and hashes a prefix on it.
- The `Pipeline` (`threads/src/pipeline.rs`) and its tasks of short
  messages (`lanes::Task`): they still carry `(key, flags)` and take a
  `Platform` from the thread that runs them, the SME2 thread among them,
  so they wait for D. With them: `hash_all_at_once`,
  `compress_subtree_to_parent_node`, `hash_serial`, `hash_serial_on`,
  `many::{assert_slots, hash_many_on, slot_len}`, `hasher::parent_output`,
  `tree::left_len`, `each::*`, `incremental::Pool`, and the constants.

Next: C's public call with Rüdiger's use (#378); then D and the
`Pipeline`, when the Mac measures again (ops #14).
