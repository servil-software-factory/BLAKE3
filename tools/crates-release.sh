#!/bin/sh
# The four crates' archives for a release, packaged from a fresh clone of
# the version commit with the dependency versions the repository names and
# no others. Writes dist/NAME-VERSION.crate for each (CONTRIBUTING.md,
# "Releases"), and prints the directory to publish from.
#
#     sh tools/crates-release.sh [--no-verify]
#     sh tools/crates-release.sh --relock
#
# HEAD is the version commit, or with B3SUM_RELEASE_ANY_COMMIT=1 any
# commit: CI packages every candidate so. --no-verify leaves out Cargo's
# build of each packaged crate (the release's own run keeps it).
#
# --relock packages nothing: it writes tools/release/Cargo.lock anew from
# HEAD's b3sum/Cargo.lock, for a commit that changes a dependency
# (CONTRIBUTING.md, "The release lock"). Every package b3sum's lock names
# stays as it is there; Cargo adds the libraries' test dependencies, the
# newest that fit, and this one step is where a version is chosen.
#
# The same bytes whoever packages. An archive is a function of the commit,
# of tools/release/Cargo.lock and of Cargo's version, so a release's crate
# can be checked by packaging it again (tools/release_artifacts.py
# --rebuilt): ours must equal GitHub's before a release, and anyone can
# repackage what crates.io serves. What makes it so:
# - A fresh clone, as tools/b3sum-release.sh takes, of the commit alone.
# - tools/release/Cargo.lock, the lock of the four crates together, and
#   --frozen: Cargo resolves nothing and asks the network nothing while it
#   packages. Without a lock it took each dependency's newest version of
#   that day, and 0.9.0's b3sums crate shipped a lock that no test had
#   run (BLAKE3 #436). A dependency now changes when a commit changes the
#   lock (tools/gen-ver.py --check holds it to b3sum/Cargo.lock).
# - The rustc that tools/b3sum-release.toolchain names: Cargo rewrites
#   each Cargo.toml as it packages, in its version's way.
set -eu
cd "$(git rev-parse --show-toplevel)"
root=$PWD
verify=""
relock=""
case "${1:-}" in
--no-verify) verify="--no-verify" ;;
--relock) relock=1 ;;
"") ;;
*) echo "crates-release: no option $1 (--no-verify, --relock)" >&2; exit 2 ;;
esac
commit=$(git rev-parse HEAD)
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
if [ -z "$relock" ] && [ "${version#*+}" != "$(git rev-parse HEAD^)" ] && [ "${B3SUM_RELEASE_ANY_COMMIT:-}" != 1 ]; then
	echo "crates-release: HEAD is no version commit (python3 tools/gen-ver.py X.Y.Z makes one)" >&2
	exit 1
fi
have="rustc $(rustc -V | cut -d' ' -f2)"
want=$(grep '^rustc ' tools/b3sum-release.toolchain)
if [ "$have" != "$want" ]; then
	echo "crates-release: this machine has $have, and tools/b3sum-release.toolchain names $want: a release's crates are packaged with exactly that" >&2
	exit 1
fi
stage="$root/target/crates-release"
rm -rf "$stage"
mkdir -p "$stage"
git clone --quiet --no-checkout . "$stage/checkout"
git -C "$stage/checkout" checkout --quiet --detach "$commit"
# The four crates as one workspace, beside the clone.
cp "$stage/checkout/tools/release/workspace.toml" "$stage/Cargo.toml"
if [ -n "$relock" ]; then
	cp "$stage/checkout/b3sum/Cargo.lock" "$stage/Cargo.lock"
	cargo fetch --quiet --manifest-path "$stage/Cargo.toml"
	cp "$stage/Cargo.lock" tools/release/Cargo.lock
	python3 tools/gen-ver.py --check
	echo "crates-release: tools/release/Cargo.lock is written anew from b3sum/Cargo.lock: read its change (git diff), then commit it"
	exit 0
fi
cp "$stage/checkout/tools/release/Cargo.lock" "$stage/Cargo.lock"
packages="-p blake3-servil-version -p blake3-servil-core -p blake3-servil -p b3sums"
# The dependencies' archives, exactly the lock's: the one step that asks
# the network, and it may change nothing.
cargo fetch --quiet --locked --manifest-path "$stage/Cargo.toml"
# shellcheck disable=SC2086
cargo package --frozen $verify --manifest-path "$stage/Cargo.toml" --target-dir "$stage/target" $packages
cmp -s "$stage/checkout/tools/release/Cargo.lock" "$stage/Cargo.lock" || { echo "crates-release: packaging changed the lock" >&2; exit 1; }
mkdir -p dist
cp "$stage"/target/package/*.crate dist/
(cd dist && for crate in "$stage"/target/package/*.crate; do sha256sum "$(basename "$crate")"; done)
echo "crates-release: the four archives are in $root/dist; publish from $stage (CONTRIBUTING.md, step 9): cargo publish --locked --no-verify, the same -p names one at a time"
