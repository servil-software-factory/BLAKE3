#!/bin/sh
# Run a Cargo command against the repository's audited lockfile
# (tools/release/Cargo.lock) and six-crate workspace
# (tools/release/workspace.toml), without adding a second lockfile.
#
#     sh tools/ci-locked.sh check -p blake3-servil-core --lib --no-default-features
#     sh tools/ci-locked.sh nextest run -p blake3-servil-core
#     sh tools/ci-locked.sh test --doc -p blake3-servil
#
# The virtual workspace is staged outside the repository tree at a
# deterministic path per checkout so Cargo's directory walks never see a
# recursive symlink and consecutive steps reuse compiled artifacts.
# [profile.test] is taken directly from root Cargo.toml (one source of truth).
set -eu
if [ "$#" -lt 1 ]; then
  echo "usage: sh tools/ci-locked.sh <cargo-subcommand> [ARGS...]" >&2
  exit 2
fi

CARGO_BIN=${CARGO:-cargo}
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
target_dir=${CARGO_TARGET_DIR:-"$root/target"}
key=$(printf %s "$root" | cksum | awk '{print $1}')
ws="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/blake3-ci-locked-${key}"
mkdir -p "$ws"
ln -sfn "$root" "$ws/checkout"
ln -sfn "$root/.config" "$ws/.config"
{
  cat "$root/tools/release/workspace.toml"
  printf '\n'
  sed -n '/^\[profile\.test\]/,/^\[/{ /^\[profile\.test\]/p; /^\[/!p; }' "$root/Cargo.toml"
} > "$ws/Cargo.toml"
cp "$root/tools/release/Cargo.lock" "$ws/Cargo.lock"

if [ "$1" = "nextest" ] && [ "$#" -ge 2 ]; then
  sub1=$1
  sub2=$2
  shift 2
  "$CARGO_BIN" "$sub1" "$sub2" --locked --manifest-path "$ws/Cargo.toml" --target-dir "$target_dir" "$@"
else
  sub1=$1
  shift 1
  "$CARGO_BIN" "$sub1" --locked --manifest-path "$ws/Cargo.toml" --target-dir "$target_dir" "$@"
fi
cmp -s "$root/tools/release/Cargo.lock" "$ws/Cargo.lock" || {
  echo "ci-locked: Cargo modified the staged lockfile" >&2
  exit 1
}
