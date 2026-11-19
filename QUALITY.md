# How BLAKE3-Servil protects your results

BLAKE3-Servil computes official BLAKE3 hashes and stays within your buffers.
When a call breaks a rule, the process stops before returning a wrong hash.

The [quality manual](docs/quality-manual.md) holds the full contracts and
reproduction commands.

## What we check

| Layer | How we check it | What remains outside it |
|---|---|---|
| Assembly kernels | Symbolic proofs check the assembled instructions against a BLAKE3 specification. Arm kernels are proved at every supported block count; x86-64 kernels are proved for the input shapes the library calls. The proofs check outputs, memory bounds, the calling convention, and secret-independent branches and addresses. | The Rust code around the kernels. Your compiler, assembler, and CPU. |
| Rust types and calls | `Block` and `Batch` enforce alignment and padding. `TreeHasher` rejects wrong part sizes at compile time and checks sibling order and total length in every build. `Pipeline` keeps worker state alive for the whole process and aborts if a loan guard drops early. | Keeping loaned memory valid until its hash finishes in the `unsafe` lending call. |
| Thread schedules | [Shuttle](https://github.com/awslabs/shuttle) runs the thread pool's wake-up code and the `Pipeline` across thousands of schedules. Nine planted concurrency bugs each fail the `Pipeline` model. | Schedules Shuttle has not tried. Weak hardware memory ordering. |
| Memory bounds | Tests place unreadable guard pages beside input and output buffers, so reading or writing one byte too far crashes at once. | Paths no test runs. |
| `unsafe` Rust | Every `unsafe` block in the `Pipeline` and in `b3sum` has a comment stating why it is sound. A script checks that each block has one. | Whether each stated reason holds: people and audits check that. |
| `b3sum` I/O | `b3sum` hashes the open file handle, checks output writes, and rejects empty checkfiles. On a panic or failed `io_uring` call, it exits at once so the kernel cannot write into freed buffers. | Files modified while `b3sum` reads them (see below). |
| Tests of the tests | Every path is checked against official BLAKE3 test vectors and outputs from the official `blake3` crate. Mutation testing plants over 2,000 small bugs in the library crates one by one and requires a failing test or a written reason. A script removes eleven `b3sum` safety checks one by one and requires a failing test for each. | Bugs outside those mutations and controls. |
| Startup (`initialize()`) | Runs the kernels chosen for your CPU against known answers and aborts on a mismatch (`b3sum` calls this at startup). | Tampered binaries, later hardware faults, and one Arm short-message family skipped for startup speed. |
| Release reviews | Before each stable release, multiple separate AI agents using different leading-edge models from different model families do deep security audits of the source and its changes. An audit counts when its model has been measured on a tree of ours with known defects. | Shared AI blind spots: of fourteen audits of the 0.10 candidate, one model reported the wrong hash it held. No independent human security audit has been done. |

We also test the proof tools by planting bugs in kernels (such as a wrong
rotation or an out-of-bounds write) and checking that the tools reject them.

## Verify a release

Linux `b3sum` binaries and the published Rust crates build reproducibly from
the release tag, byte for byte ([how to rebuild and compare](b3sum/README.md)).
GitHub Actions rebuilds and compares them on each release. Release tags are
locked against changes.

Releases are not cryptographically signed. Check the SHA-256 digest from a
channel you trust before running a downloaded binary. Build the C library from
source.

## Use `b3sum` safely

Get your checkfile from a trusted source. `b3sum --check` verifies the files
listed in the checkfile. Extra files in a directory or deleted lines in a
checkfile go unnoticed.

Hash stable files or a filesystem snapshot. Modifying a file while `b3sum`
reads it can produce the hash of mixed bytes without an error
([what each path reads](b3sum/README.md)).

## Run the tests

Install Rust, a C compiler, and [cargo-nextest](https://nexte.st):

```sh
cargo nextest run --release --lib --tests
cargo nextest run --release --manifest-path threads/Cargo.toml --lib --tests
cargo nextest run --release --manifest-path b3sum/Cargo.toml --tests
```

The [quality manual](docs/quality-manual.md) gives commands for the assembly
proofs and the fault controls.
