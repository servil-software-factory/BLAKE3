# How BLAKE3-Servil protects your results

BLAKE3-Servil computes official BLAKE3 hashes and stays within your buffers.
When a call breaks a checkable rule (such as bad shape, unaligned or short
buffers, or out-of-range tree coordinates), the call panics before returning a
wrong hash (and startup self-test failure or early drop of an active `Pipeline`
loan aborts the process); properties the library cannot inspect, such as
supplying `Cv`s produced under a mismatched key to `TreeHasher`, remain the
caller's responsibility as documented on `TreeHasher`.

The [quality manual](docs/quality-manual.md) holds BLAKE3-Servil's full
verification contracts and reproduction commands. Organization-wide
engineering and supply-chain standards live in the
[Servil Software Factory Manual](https://github.com/servil-software-factory).

## What we check

| Layer | How we check it | What remains outside it |
|---|---|---|
| Assembly kernels | Symbolic proofs check the assembled instructions against a BLAKE3 specification. Arm kernels are proved at every supported block count; x86-64 kernels are proved for the input shapes the library calls. The proofs check outputs, memory bounds, the calling convention, and secret-independent branches and addresses. | The Rust code around the kernels. Your compiler, assembler, and CPU. |
| Rust types, lints, and calls | Stateful types hold invariants behind private fields, checked constructors, and RAII guards (`Domain`, `Stage`, `CvStack`, `Block`, `Batch`, `TreeHasher`/`Cv`/`LastCv`, `Loan`), and internal callers go through the same checked APIs as external callers (`hazmat` is gone). Crate `[lints]` tables deny `unsafe_op_in_unsafe_fn`, `unused_must_use`, `unreachable_patterns`, `clippy::correctness`, and narrowing `as` casts (`clippy::cast_possible_truncation = "deny"`, zero shipped suppressions). Cheap, critical invariants run in every build; `Pipeline` keeps its worker-facing `PipeSlot` alive for the process, frees per-pipeline storage once the `Pipeline` and last surviving `Room` drop (`Lent` observes the retained slot), and aborts if a loan guard drops early; `tools/counts.py` blocks rising counts of undocumented `unsafe`, undocumented `debug_assert!`, `debug_assert!`, silent narrowings, and truncation allows. | Keeping loaned memory valid until its hash finishes in the `unsafe` lending call. |
| Thread schedules and memory order | [Shuttle](https://github.com/awslabs/shuttle) runs the thread pool's wake-up code and the `Pipeline` across thousands of schedules, and seventeen planted concurrency bugs each fail the `Pipeline` model. For Apple Arm64 weak ordering, `tools/check-waker-order.py` compiles the production wait paths (`WakerSlot`, `Park`, `sme2_serve`) and verifies their fences against `RCpc` loads, rejecting unfenced controls. | Schedules Shuttle has not tried, and weak-memory paths outside those wait handshakes. |
| Memory bounds | Guard-page tests and custom guard-page allocators (`tests/guard_alloc.rs`, `threads/tests/guard_alloc.rs`) place unreadable guard pages beside input and output buffers, so reading or writing one byte too far crashes at once. | Paths no test runs. |
| `unsafe` Rust and `debug_assert!` | Every `unsafe` block across all four shipped crates (`version`, `core`, `threads`, `capi`) and build scripts carries a `// Sound:` comment, and every shipped `debug_assert!` carries a `// Debug-only:` comment explaining why it cannot run in release builds; both are verified by `tools/unsafe_reasons.py` (0 undocumented blocks or `debug_assert!`s). | Whether each stated reason holds: people and audits check that. |
| Tests of the tests | Every path is checked against official BLAKE3 test vectors and outputs from the official `blake3` crate, plus arithmetic work-count lifecycle tests (`src/lifecycle_counts.rs`). Fast native mutation testing (x86-64 Linux and Apple Silicon arm64+SME2) plants small bugs in the library crates one by one and requires a failing test or a written reason. | Bugs outside those mutations and controls. |
| Supply chain | Shipped library crates (`blake3-servil-core`, `blake3-servil`, `blake3-servil-c`) have **zero third-party runtime or build dependencies** on every supported platform (`src/ct.rs` replaces `constant_time_eq`, `libc` is dev-only in `6366776f`, and `core::arch::global_asm!` replaces `cc` in `e36ef4d1` / `#487`). CI actions, containers, and bootstrap tools are pinned by full SHA/digest; builds run `--locked`; `tools/dependency_policy.py` enforces `cargo-deny` and `cargo-vet` with ten planted-fault controls. | Bugs or compromise in the Rust compiler, OS C library, or remaining dev/test dependencies. |
| Startup (`initialize()`) | Runs the kernels chosen for your CPU against known answers and aborts on a mismatch. | Tampered binaries, later hardware faults, and one Arm short-message family skipped for startup speed. |
| Landing & release reviews | Every change to shipped code gets a cross-family AI landing review (Rule 2) in a sandbox on our build host (`review-landing`). Before each stable release, multiple separate AI agents using leading-edge models with measured recall on a real defect tree audit the source, stateful types' contracts, and the final version diff. *(After `#499`'s stateful-type rollout, 12 blinded before/after GPT Astra audits dropped findings 82.7%, `168 → 29`, with `0` memory-safety findings.)* | Shared AI blind spots: of fourteen audits of the 0.10 candidate, one model reported the wrong hash it held (six of nine found it when asked to write the type's contract). No independent human security audit has been done. |

We also test the proof tools by planting bugs in kernels (such as a wrong
rotation or an out-of-bounds write) and checking that the tools reject them.

## Verify a release

The published Rust library crates build reproducibly from the release tag,
byte for byte (`sh tools/crates-release.sh` and `tools/release_artifacts.py`).
The `b3sum` CLI lives in [`servil-software-factory/b3sum`](https://github.com/servil-software-factory/b3sum).
GitHub Actions rebuilds them on each release and compares each builder's output
independently against the published release artifacts and their digest files.
Release tags are locked against changes.

Releases are not cryptographically signed. Check the SHA-256 digest from a
channel you trust before running a downloaded binary. Build the C library from
source.

## Run the tests

Building the shipped library crates requires only Rust (no C compiler).
Running the full test suite also uses [cargo-nextest](https://nexte.st)
and a C compiler (for the official `blake3` test oracle and `capi/` tests):

```sh
cargo nextest run --release --lib --tests
cargo nextest run --release --manifest-path threads/Cargo.toml --lib --tests
```

The [quality manual](docs/quality-manual.md) gives commands for the assembly
proofs and the fault controls.
