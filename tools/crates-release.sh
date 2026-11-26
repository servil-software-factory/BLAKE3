#!/bin/sh
# The three crates' archives for a release, packaged from a fresh clone of
# the version commit with the dependency versions the repository names and
# no others. Writes dist/NAME-VERSION.crate for each (CONTRIBUTING.md,
# "Releases"), and prints the directory to publish from.
#
#     sh tools/crates-release.sh [--no-verify]
#     sh tools/crates-release.sh --relock
#
# HEAD is the version commit, or with RELEASE_ANY_COMMIT=1 any
# commit: CI packages every candidate so. --no-verify leaves out Cargo's
# build of each packaged crate (the release's own run keeps it).
#
# --relock packages nothing: it writes tools/release/Cargo.lock anew from
# HEAD's manifests, for a commit that changes a dependency
# (CONTRIBUTING.md, "The release lock").
#
# The same bytes whoever packages. An archive is a function of the commit,
# of tools/release/Cargo.lock and of Cargo's version, so a release's crate
# can be checked by packaging it again (tools/release_artifacts.py
# --rebuilt): ours must equal GitHub's before a release, and anyone can
# repackage what crates.io serves. What makes it so:
# - A fresh clone of the commit alone.
# - tools/release/Cargo.lock, the lock of the workspace together, and
#   --frozen: Cargo resolves nothing and asks the network nothing while it
#   packages.
# - The rustc that tools/release.toolchain names: Cargo rewrites
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
if [ -z "$relock" ] && [ "${version#*+}" != "$(git rev-parse HEAD^)" ] && [ "${RELEASE_ANY_COMMIT:-${B3SUM_RELEASE_ANY_COMMIT:-}}" != 1 ]; then
	echo "crates-release: HEAD is no version commit (python3 tools/gen-ver.py X.Y.Z makes one)" >&2
	exit 1
fi
have="rustc $(rustc -V | cut -d' ' -f2)"
want=$(grep '^rustc ' tools/release.toolchain)
if [ "$have" != "$want" ]; then
	echo "crates-release: this machine has $have, and tools/release.toolchain names $want: a release's crates are packaged with exactly that" >&2
	exit 1
fi
stage="$root/target/crates-release"
rm -rf "$stage"
mkdir -p "$stage"
git clone --quiet --no-checkout . "$stage/checkout"
git -C "$stage/checkout" checkout --quiet --detach "$commit"
# The workspace's crates together, beside the clone.
cp "$stage/checkout/tools/release/workspace.toml" "$stage/Cargo.toml"
if [ -n "$relock" ]; then
	cargo fetch --quiet --manifest-path "$stage/Cargo.toml"
	cp "$stage/Cargo.lock" tools/release/Cargo.lock
	python3 tools/gen-ver.py --check
	echo "crates-release: tools/release/Cargo.lock is written anew: read its change (git diff), then commit it"
	exit 0
fi
cp "$stage/checkout/tools/release/Cargo.lock" "$stage/Cargo.lock"
packages="-p blake3-servil-version -p blake3-servil-core -p blake3-servil"
# The dependencies' archives, exactly the lock's: the one step that asks
# the network, and it may change nothing.
cargo fetch --quiet --locked --manifest-path "$stage/Cargo.toml"
# shellcheck disable=SC2086
cargo package --frozen $verify --manifest-path "$stage/Cargo.toml" --target-dir "$stage/target" $packages
cmp -s "$stage/checkout/tools/release/Cargo.lock" "$stage/Cargo.lock" || { echo "crates-release: packaging changed the lock" >&2; exit 1; }
mkdir -p dist
cp "$stage"/target/package/*.crate dist/
(
	cd dist
	for crate in "$stage"/target/package/*.crate; do
		base=$(basename "$crate")
		{ sha256sum "$base" 2>/dev/null || shasum -a 256 "$base"; } > "$base.sha256"
		python3 "$root/tools/blake3_ref.py" "$base" > "$base.b3"
		cat "$base.sha256" "$base.b3"
	done
)
echo "crates-release: the three archives (with .sha256 and .b3) are in $root/dist; publish from $stage (CONTRIBUTING.md, step 9): cargo publish --locked --no-verify, the same -p names one at a time"
