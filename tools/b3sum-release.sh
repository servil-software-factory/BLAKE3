#!/bin/sh
# b3sum's release binary for this machine, built from a fresh clone of the
# release's version commit: statically linked on Linux (the recommended
# build, b3sum/README.md), as the platform links elsewhere (macOS). Writes
# dist/b3sum-VERSION-TARGET with both its SHA-256 (.sha256) and independent
# pure-Python BLAKE3 (.b3) digest files beside it, for a GitHub release
# (CONTRIBUTING.md, "Releases"; BLAKE3 #446).
#
#     sh tools/b3sum-release.sh
#
# HEAD is the version commit (its tag need not be made yet), or with
# B3SUM_RELEASE_ANY_COMMIT=1 any commit, its binary named by the commit:
# CI builds every candidate so, which keeps this script working.
#
# The same bytes on every machine. On Linux the binary is a function of
# the commit and of tools/b3sum-release.toolchain, whoever builds it and
# wherever, so a release's binary can be checked by building it again:
# our machines and GitHub's must agree before a release, and anyone can
# rebuild what we published. Three things make it so:
# - A fresh clone: a binary is built from every file of its checkout that
#   Git does not ignore, and a working checkout holds more than the
#   commit. 0.9.0's two binaries were built where logs and worktrees lay
#   untracked, and their versions say "dirty" (BLAKE3 #435).
# - One build directory, /tmp/b3sum-release-build: Cargo names each
#   crate's symbols by a hash of its directory. Cargo's and Rust's own
#   directories, which the binary's messages name, are written as /cargo
#   and /rust.
# - The toolchain file's compiler and C library, and Clang as the linker's
#   driver (GCC's passes the linker other flags on AArch64).
set -eu
cd "$(git rev-parse --show-toplevel)"
root=$PWD
commit=$(git rev-parse HEAD)
target=$(rustc -vV | sed -n 's/^host: //p')
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
if [ "${version#*+}" = "$(git rev-parse HEAD^)" ]; then
	name="b3sum-v$version-$target"
elif [ "${B3SUM_RELEASE_ANY_COMMIT:-}" = 1 ]; then
	version=""
	name="b3sum-$commit-$target"
else
	echo "b3sum-release: HEAD is no version commit (python3 tools/gen-ver.py X.Y.Z makes one)" >&2
	exit 1
fi
flags="--remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=$(rustc --print sysroot)=/rust"
case "$target" in
*-linux-gnu)
	export CC="${CC:-clang}"
	have="rustc $(rustc -V | cut -d' ' -f2)
libc6-dev $(dpkg-query -W -f='${Version}' libc6-dev)"
	want=$(grep '^rustc \|^libc6-dev ' tools/b3sum-release.toolchain)
	if [ "$have" != "$want" ]; then
		printf 'b3sum-release: this machine has\n%s\nand tools/b3sum-release.toolchain names\n%s\nA release binary is built with exactly those.\n' "$have" "$want" >&2
		exit 1
	fi
	flags="$flags -C target-feature=+crt-static -C linker=$CC"
	;;
esac
build=/tmp/b3sum-release-build
rm -rf "$build"
mkdir -m 700 "$build"
git clone --quiet --no-checkout . "$build/source"
git -C "$build/source" checkout --quiet --detach "$commit"
RUSTFLAGS="$flags" cargo build --release --locked --manifest-path "$build/source/b3sum/Cargo.toml" --target "$target" --target-dir "$build/target"
mkdir -p dist
out="dist/$name"
cp "$build/target/$target/release/b3sum" "$out"
rm -rf "$build"
# The binary's own build version: the release's alone, or for another
# commit never a dirty tree's.
if [ -n "$version" ]; then
	python3 tools/release_artifacts.py "v$version" "$out" --commit "$commit" || { rm -f "$out"; exit 1; }
elif grep -a -q "$commit\.dirty" "$out"; then
	echo "b3sum-release: the binary says its tree was dirty" >&2
	rm -f "$out"
	exit 1
fi
# The binary checks itself against BLAKE3's published digest of nothing,
# then both .sha256 and .b3 are written independently of the binary under
# check (tools/blake3_ref.py) and cross-checked against it.
test "$(printf '' | "$out" --no-names)" = af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262
(cd dist && { sha256sum "$name" 2>/dev/null || shasum -a 256 "$name"; } > "$name.sha256")
(cd dist && python3 "$root/tools/blake3_ref.py" "$name" > "$name.b3")
test "$(cd dist && "$root/$out" "$name")" = "$(cat "dist/$name.b3")"
file "$out"
cat "dist/$name.sha256" "dist/$name.b3"
echo "b3sum-release: $root/$out, with .sha256 and .b3 beside it"
