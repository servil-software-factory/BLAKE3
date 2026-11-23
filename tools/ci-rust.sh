#!/bin/sh
# CI's Rust compiler, by name:
#
#     sh tools/ci-rust.sh stable|nightly [COMPONENT...]
#
# "stable" is the compiler releases are built with (the rustc line of
# tools/b3sum-release.toolchain); "nightly" is the one NIGHTLY names here.
# rustup, which GitHub's machines carry, installs it and makes it the
# default: a named toolchain's files do not change, and rustup checks
# each against the hash its manifest gives. A commit moves a name; no
# job takes the compiler of the day.
set -eu
NIGHTLY=nightly-2026-11-22
case "${1:-}" in
	stable) toolchain=$(sed -n 's/^rustc //p' "$(dirname "$0")/b3sum-release.toolchain") ;;
	nightly) toolchain=$NIGHTLY ;;
	*) echo "usage: sh tools/ci-rust.sh stable|nightly [COMPONENT...]" >&2; exit 2 ;;
esac
shift
components=
for component in "$@"; do
	components="$components --component $component"
done
# shellcheck disable=SC2086
rustup toolchain install "$toolchain" --profile minimal --no-self-update $components
rustup default "$toolchain"
rustc -vV
