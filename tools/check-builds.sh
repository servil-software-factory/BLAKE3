#!/bin/sh
# The builds the Mac's and the VM's suites leave out, checked in minutes
# on this machine (part of a candidate's gate, CONTRIBUTING.md): the library
# without std, tested; the library on one CPU without SME2, tested;
# warnings as errors for the library and b3sum;
# the C API, tested from Rust and from C;
# compile/codegen checks for x86-64 and macOS AArch64 (via rustc's
# global_asm!, with no external C compiler); and the stop for every other
# target. Each failure stops the script.
#
#     sh tools/check-builds.sh
set -eu
cd "$(git rev-parse --show-toplevel)"
[ "${CARGO_TARGET_DIR:-}" != /tmp/target ] || unset CARGO_TARGET_DIR
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-"$PWD/target"}
rustup target add x86_64-unknown-linux-gnu aarch64-apple-darwin 2>/dev/null
step() { echo "== $*"; "$@"; }
# The version names this commit truly (tools/gen-ver.py: a release's on
# the release commit alone, X.Y.Z+dev after it).
step python3 tools/gen-ver.py --check
step python3 tools/unsafe_reasons.py
step sh -c 'base=$(git rev-parse --verify -q origin/next || git rev-parse --verify -q HEAD~1); [ -n "$base" ] || { echo "counts: cannot resolve base revision" >&2; exit 1; }; python3 tools/counts.py --check "$base"'
RUSTFLAGS="-D warnings" step cargo nextest run --cargo-quiet --release --no-default-features
# nextest skips doc tests: the examples without std too, where Batch is absent.
step cargo test -q --release --doc --no-default-features
# An example that must not compile names the compiler's reason
# (compile_fail,E0308) where the compiler gives one a code. Stable
# rustdoc takes any failure for it; with
# this switch it holds each such example to its reason, in both crates.
step sh -c 'RUSTC_BOOTSTRAP=1 cargo test -q --doc && cd threads && RUSTC_BOOTSTRAP=1 cargo test -q --doc'
RUSTFLAGS="-D warnings" step cargo check -q --lib --no-default-features
RUSTFLAGS="-D warnings" step cargo check -q --release --all-features --tests --examples
# The crate with the pool (threads/). One CPU, as in a one-CPU container:
# the pool has no worker thread, and the caller hashes everything (the
# suites run on many CPUs and never reach that path). The CPU is the
# first of those this shell may use: CPU 0 need not be one of them.
one_cpu=$(taskset -cp $$ | sed 's/.*: *//; s/[,-].*//')
step sh -c "cd threads && taskset -c $one_cpu cargo nextest run --cargo-quiet --release --lib"
# The pool's handshakes (the finish line, the park, the waker slot) under
# the model checker, every schedule Shuttle explores: seconds. Until
# November 15, 2026 they ran only inside the mutation check, so a change
# that broke one could pass every gate that was not about its own lines.
step sh -c 'cd threads && RUSTFLAGS="--cfg shuttle" cargo test -q --release --test shuttle_finish --test shuttle_park --test shuttle_waker --test shuttle_pipeline'
# The Pipeline's model fails for each planted fault (a build of it each,
# in a copy of this tree: no fault is ever planted here).
step python3 tools/pipeline_model_controls.py
# Shuttle's SC model cannot check the RCpc wake-publication barrier.
step python3 tools/check-waker-order.py
RUSTFLAGS="-D warnings" step sh -c 'cd threads && cargo nextest run --cargo-quiet --release --no-default-features'
RUSTFLAGS="-D warnings" step sh -c 'cd threads && cargo check -q --release --all-features --tests'
RUSTFLAGS="-D warnings" step sh -c 'cd threads && cargo check -q --tests --target x86_64-unknown-linux-gnu'
RUSTFLAGS="-D warnings" step cargo check -q --release --manifest-path b3sum/Cargo.toml
RUSTFLAGS="-D warnings" step cargo check -q --release --tests --manifest-path b3sum/Cargo.toml
RUSTFLAGS="-D warnings" step cargo check -q --release --manifest-path version/Cargo.toml
# Cross-check macOS b3sum compilation under -D warnings on Linux build hosts.
CC_aarch64_apple_darwin=clang RUSTFLAGS="-D warnings" step cargo check -q --release --manifest-path b3sum/Cargo.toml --target aarch64-apple-darwin
# Crate [lints.clippy] enforcement (no silent narrowing `as`, no b3sum stdout/stderr print macros).
for manifest in Cargo.toml threads/Cargo.toml b3sum/Cargo.toml capi/Cargo.toml version/Cargo.toml; do
    step cargo clippy -q --manifest-path "$manifest" --no-deps -- -D warnings
done
# The C API (capi/): its Rust tests, and b3.h compiled as C11 and run.
RUSTFLAGS="-D warnings" step cargo nextest run --cargo-quiet --release --manifest-path capi/Cargo.toml
RUSTFLAGS="-D warnings" step cargo check -q --tests --examples --target x86_64-unknown-linux-gnu
RUSTFLAGS="-D warnings" step cargo check -q --tests --target x86_64-unknown-linux-gnu --no-default-features
# A build for any other target stops in build.rs, naming official BLAKE3
# (#160, #180): Windows, WebAssembly for the rest, and x86-64 Linux with
# pointers of 32 bits (the x32 ABI: the right architecture and system,
# and sizes this library does not check for). The build script
# runs even where the target's standard library is absent; the Linux builds
# above are the control.
for target in x86_64-pc-windows-msvc wasm32-unknown-unknown x86_64-unknown-linux-gnux32; do
	step sh -c "cargo check -q --keep-going --target $target 2>&1 | grep -q 'for $target, use official BLAKE3'"
done
# Assembly is built directly by rustc via core::arch::global_asm! (#487),
# so building both shipped library crates (default, no_std, and macOS
# AArch64 Mach-O codegen) requires no external C compiler at all.
step sh -c 'no_cc="$CARGO_TARGET_DIR/no-cc"; rm -rf "$no_cc"; CC=/nonexistent/cc CARGO_TARGET_DIR="$no_cc" cargo build -q --release --lib && CC=/nonexistent/cc CARGO_TARGET_DIR="$no_cc" cargo build -q --release --lib --no-default-features && CC=/nonexistent/cc CARGO_TARGET_DIR="$no_cc" cargo build -q --release --manifest-path threads/Cargo.toml --lib && CC=/nonexistent/cc CARGO_TARGET_DIR="$no_cc" cargo build -q --release --lib --target aarch64-apple-darwin && CC=/nonexistent/cc CARGO_TARGET_DIR="$no_cc" cargo build -q --release --lib --no-default-features --target aarch64-apple-darwin && rm -rf "$no_cc"'
# Every generated kernel file is its generator's output.
step sh -c 'python3 tools/gen_neon_hybrid.py | cmp - c/blake3_neon_hybrid_aarch64.S && python3 tools/gen_sme2_hybrid.py | cmp - c/blake3_sme2_hybrid_aarch64.S'
step sh -c 'python3 tools/gen_x86.py | cmp - c/blake3_partial_x86-64_unix.S'
step sh -c 'python3 tools/gen_x86_chunk_root.py | cmp - c/blake3_chunk_root_x86-64_unix.S'
step sh -c 'python3 tools/gen_avx512_32.py | cmp - c/blake3_avx512_32.S'
step sh -c 'python3 tools/gen_output16.py | cmp - c/blake3_output16.S'
step sh -c 'python3 tools/gen_output8.py | cmp - c/blake3_output8.S'
step sh -c 'python3 tools/gen_packed_x86.py | cmp - c/blake3_packed_x86-64_unix.S'
step sh -c 'python3 tools/gen_packed_sme2.py | cmp - c/blake3_packed_sme2_aarch64.S'
step sh -c 'python3 tools/gen_packed_neon.py | cmp - c/blake3_packed_neon_aarch64.S'
step sh -c 'python3 tools/gen_counter_x86.py | cmp - c/blake3_counter_x86-64_unix.S'
step sh -c 'python3 tools/gen_counter_sme2.py | cmp - c/blake3_counter_sme2_aarch64.S'
step sh -c 'python3 tools/gen_counter_neon.py | cmp - c/blake3_counter_neon_aarch64.S'
step sh -c 'python3 tools/gen_output_sme2.py | cmp - c/blake3_output_sme2_aarch64.S'
step sh -c 'python3 tools/gen_output_neon.py | cmp - c/blake3_output_neon_aarch64.S'
# Both crates document their traits and account for their lifecycle
# operations: with every feature on, so that a trait implementation
# behind a feature (as 0.9's Zeroize was) is one of those accounted for.
step python3 tools/test_api.py
step python3 tools/test_stack_budget.py
step python3 tools/stack_budget.py
for manifest in Cargo.toml threads/Cargo.toml; do
    step sh -c 'RUSTC_BOOTSTRAP=1 cargo rustdoc -q --manifest-path "$0" --lib --all-features -- -Z unstable-options --output-format json' "$manifest"
done
step python3 tools/api.py "$CARGO_TARGET_DIR/doc/blake3_servil_core.json" --check docs/core-api.md
step python3 tools/api.py "$CARGO_TARGET_DIR/doc/blake3_servil.json" --check
# Each of b3sum's protections has a test that fails without it.
step python3 tools/b3sum_controls.py
echo "check-builds: every build passes"
