# How BLAKE3-Servil protects your results

BLAKE3-Servil computes official BLAKE3 hashes and stays within your buffers.
When a call breaks a rule, the process stops before returning a wrong hash.

The [quality manual](docs/quality-manual.md) holds the full contracts and
reproduction commands.

## What we check

| Layer | How we check it | What remains outside it |
|---|---|---|
| Assembly kernels | Symbolic proofs check the assembled instructions against a BLAKE3 specification. Arm kernels are proved at every supported block count; x86-64 kernels are proved for the input shapes the library calls. The proofs check outputs, memory bounds, the calling convention, and secret-independent branches and addresses. | The Rust code around the kernels. Your compiler, assembler, and CPU. |
| Rust types and calls | `Block` and `Batch` enforce alignment and padding, and internal callers go through the same checked APIs as external callers (`hazmat` is gone). `TreeHasher` rejects wrong part sizes at compile time and checks sibling order and total length in every build. Cheap, critical invariants (buffer shapes, initialization, prefix-cut bounds) run in every build. `Pipeline` keeps worker state alive for the whole process and aborts if a loan guard drops early. | Keeping loaned memory valid until its hash finishes in the `unsafe` lending call. |
| Thread schedules and memory order | [Shuttle](https://github.com/awslabs/shuttle) runs the thread pool's wake-up code and the `Pipeline` across thousands of schedules, and ten planted concurrency bugs each fail the `Pipeline` model. For Apple Arm64 weak ordering, `tools/check-waker-order.py` compiles the production wait paths (`WakerSlot`, `Park`, `sme2_serve`) and verifies their fences against `RCpc` loads, rejecting unfenced controls. | Schedules Shuttle has not tried, and weak-memory paths outside those wait handshakes. |
| Memory bounds | Tests place unreadable guard pages beside input and output buffers, so reading or writing one byte too far crashes at once. | Paths no test runs. |
| `unsafe` Rust | Every `unsafe` block in the `Pipeline` and in `b3sum` has a comment stating why it is sound. A script checks that each block has one. | Whether each stated reason holds: people and audits check that. |
| `b3sum` I/O | `b3sum` hashes the open file handle, checks output writes, rejects empty checkfiles, uses 64-bit ring read lengths, and handles short cache-only reads and repeated standard input. On a panic or failed `io_uring` call, it exits at once so the kernel cannot write into freed buffers. | Files modified while `b3sum` reads them (see below). |
| Tests of the tests | Every path is checked against official BLAKE3 test vectors and outputs from the official `blake3` crate. Mutation testing plants over 2,000 small bugs in the library crates one by one and requires a failing test or a written reason. A script removes eleven `b3sum` safety checks one by one and requires a failing test for each. | Bugs outside those mutations and controls. |
| Startup (`initialize()`) | Runs the kernels chosen for your CPU against known answers and aborts on a mismatch (`b3sum` calls this at startup). | Tampered binaries, later hardware faults, and one Arm short-message family skipped for startup speed. |
| Release reviews | Before each stable release, multiple separate AI agents using leading-edge models audit the source, its changes, and stateful types' contracts (invariants and every operation, including trait implementations). An audit counts toward the release bar when its model has measured recall on a tree of ours with known defects, and the diff to the final version commit is reviewed too. | Shared AI blind spots: of fourteen audits of the 0.10 candidate, one model reported the wrong hash it held (six of nine found it when asked to write the type's contract). No independent human security audit has been done. |

We also test the proof tools by planting bugs in kernels (such as a wrong
rotation or an out-of-bounds write) and checking that the tools reject them.

## Verify a release

Linux `b3sum` binaries and the published Rust crates build reproducibly from
the release tag, byte for byte ([how to rebuild and compare](b3sum/README.md)).
GitHub Actions rebuilds them on each release and checks each builder's output
independently against the tag and its digest files. Release tags are locked
against changes.

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
