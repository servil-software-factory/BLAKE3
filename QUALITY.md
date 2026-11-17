# How BLAKE3-Servil protects your results

BLAKE3-Servil aims to produce exactly the official BLAKE3 digests, without
reading or writing outside your buffers. A wrong hash reported as right
can be worse than a crash. Our first choice is to make a dangerous state
impossible; the next is to stop before returning a result from it.

This page is for users. The [quality manual](docs/quality-manual.md) gives
experts and maintainers the contracts, commands, evidence and limitations.

## Prevent mistakes, then check the remaining cases

Rust's safe interfaces enforce ownership and borrowing. The throughput
API's `Block` type supplies alignment; a `Batch` lets you write only the
message bytes, keeping the padding the kernels require intact. These
restrictions apply in release builds as well as development builds.

Some of the code is `unsafe` Rust or assembly. Its safety depends on
contracts the compiler cannot check. We reduce that responsibility where
we can:

- The Pipeline's shared state lives as long as the process. Workers
  cannot read it after its owner has freed it. It is reused between
  Pipelines, which still requires correct synchronization.
- Dropping a Pipeline loan's guard before its work completes stops the
  process. The unsafe lending call still requires the caller to keep
  the loaned memory valid.
- In `b3sum`, direct reads use the already-open file, rather than looking
  up its pathname again. Replacing the name after open cannot switch
  that read to another file.
- An `io_uring` submission/wait failure stops `b3sum` without unwinding:
  the kernel might still be writing into its buffers. An individual
  read's completed error is reported as a file error instead.
- `b3sum` reports failure if it cannot write its output, or if a checkfile
  is empty. Success cannot mean that no checks were supplied.

The last three protections are added for 0.10.0. They address specific
failure paths, not every possible I/O or concurrency bug.

## Proofs of the assembly

The fastest routines, called kernels, are written in assembly. Symbolic
execution compares assembled instructions with a written BLAKE3
specification. A completed proof covers every input value within its
stated contract, rather than a sample of messages. The proofs also check
memory accesses, the calling convention, and whether secret bytes affect
branches or addresses.

Arm kernels are proved at every supported block count; induction covers
the SME2 kernels' group loops. Intel and AMD kernels have proofs over the
input shapes the library calls. The manual lists the exact coverage.

The proof tools are checked too. Instruction models are compared with
processors and, where supported, Arm's specification. Planted mistakes
in kernels must be rejected: for example, a wrong rotation, an incorrect
stride, or a write outside the allowed memory.

**These are kernel proofs, not a proof of the whole library.** The Rust
that chooses work, builds the tree, and shares it among threads is tested
and reviewed. A user's assembler and compiler also remain part of the
trusted build chain.

## Small checks with a known reason to fail

Tests use published BLAKE3 vectors and fixed answers generated with
official BLAKE3. Cases are chosen from the code's boundaries and distinct
paths. Mutation testing asks whether the core and thread-library tests
reject deliberately changed code. Each surviving mutation has an
explanation; a survivor is not automatically harmless. The mutation
gate does not cover `b3sum`. In its place a program takes eleven of
`b3sum`'s protections out one at a time and requires the test for each
to fail.

For failures that ordinary successful hashing would miss, we arrange the
failure directly:

- **Memory bounds:** inaccessible guard pages sit immediately beside
  buffers. A stray access in an exercised path faults, assembly included.
- **Thread handshakes:** Shuttle runs the production finish, park and
  wake-up code, and the whole Pipeline with stand-in worker threads,
  under thousands of chosen schedules. A wait that could hang shows as
  a deadlock with the schedule that caused it, and a result handed
  over too early as a wrong digest. Nine faults planted in the
  Pipeline's code one at a time (a result published before it is
  written, a wake left out, a buffer given back while a worker reads
  it, and others) each make its model fail. These checks run before a
  change lands. They cover the schedules tried, not every one, and
  Shuttle treats every atomic operation as the strictest kind, so a
  fault that needs a weaker ordering to show is outside them.
- **Unsafe code states its reason:** every `unsafe` block of the
  Pipeline and of `b3sum` has a comment saying why it is sound, and a
  check refuses a block there without one. Writing them found three
  bugs in `b3sum`.
- **I/O failures:** bounded tests use a full output device, an empty
  checkfile, a pathname replaced after open, a file longer than its
  size said, a link where an output file goes, and a failed `io_uring`
  syscall. They check the error or stop, not just the digest.
- **Wrong calls:** a call that breaks a rule whose breach would corrupt
  memory or give a wrong hash stops in every build, not only in debug
  builds. Where the compiler can refuse the call instead, it does.

The manual gives commands to repeat both the checks and their controls.

## Your build checks itself

Call `initialize()` before using the library (`b3sum` does this for you).
It hashes fixed inputs with known answers on your machine, and stops the
process on a mismatch. The cases cover the selected kernels, apart from
the Arm kernels for short messages of unequal lengths; those are proved
and tested, but deliberately excluded from startup for its cost.

This can detect some broken builds and faulty processors. It cannot
certify a hostile binary: an attacker can change the check as well as
the hashing code. It also cannot detect every hardware fault or a fault
that occurs later.

## Security reviews before each stable release

Our release practice includes deep security audits by multiple separate
AI agents using different leading-edge models. Each examines the source
and its trust boundaries; we retain reports naming the audited revision
and models, and resolve findings before publication. The manual specifies
what a report records and how findings are checked.

Different models can still share blind spots. Their agreement is not a
proof, and AI review is not a substitute for an independent human
security audit. We claim no such human audit. A release's reports, not
this statement of practice, are the evidence that its reviews occurred.

## What you can verify, and what remains your responsibility

A full release version includes a source commit hash. Release binaries
come with SHA-256 checksums. Obtain the expected checksum from a source
you trust, and check it before running a downloaded binary. A checksum
detects changed bytes; it does not identify a publisher if an attacker
can replace the file and its checksum together.

You need not trust a Linux `b3sum` binary or a crate we publish: their
builds are reproducible, so you can build one again from the release's
tag and compare the bytes ([how](b3sum/README.md)). We do so ourselves
on GitHub's machines before and after each release, and a program
compares each published crate with the tagged source file by file. A
release's tag names its source by hash and cannot be moved. The C
library's archive is not reproducible: build it from source. Releases
are not signed. The manual states what else this leaves out. Releases
through 0.9.0 predate it.

For file verification, obtain the expected hash through a trusted
channel. `b3sum --check` checks the named files, not a directory's complete
contents: an added file can go unnoticed. An empty checkfile now fails,
but deleting some entries can still hide those missing checks.

Hash stable files or a filesystem snapshot. `b3sum` hashes the file it
opened, whatever its name points to later, but it takes no lock and no
snapshot: a file that grows, is cut short or is overwritten while it is
hashed can give the hash of bytes the file never held, with no error.
[b3sum's README](b3sum/README.md), "What b3sum reads", says what
happens in each case. Detecting it is not in this release (#443).

The proofs trust their specification, instruction models and proof
tools. Nothing establishes memory safety for all Rust thread
interleavings. The manual also explains hardware timing channels beyond
the code's control. These protections reduce risk; none certifies the
absence of bugs or establishes the security of the BLAKE3 algorithm
itself.

## Repeat the checks

From a checkout of the release you intend to use, with Rust, a supported
C assembler and [cargo-nextest](https://nexte.st):

```sh
cargo nextest run --release --lib --tests
cargo nextest run --release --manifest-path threads/Cargo.toml --lib --tests
cargo nextest run --release --manifest-path b3sum/Cargo.toml --tests
```

Some checks are platform-specific. The [manual](docs/quality-manual.md)
names their prerequisites and expected failures, and explains how to run
the proofs. Development commits land through a protected gate with checks
recorded on the exact commit; exceptions are recorded as explicit waivers.
