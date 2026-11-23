#!/bin/sh
# tools/b3sum-release.sh in a new Ubuntu 26.04 container, as root: installs
# what tools/b3sum-release.toolchain names and runs it. How GitHub's
# machines build b3sum's release binary (.github/workflows), and how
# anyone can, with Docker or Podman, from a clone at a release's tag:
#
#     docker run --rm -v "$PWD":/src -w /src "$(sed -n 's/^image //p' tools/b3sum-release.toolchain)" sh tools/b3sum-release-container.sh
#
# The binary and its digests are then in dist/: the same bytes as the
# release's, or the release's was not built from this source.
set -eu
want() { sed -n "s/^$1 //p" tools/b3sum-release.toolchain; }
export DEBIAN_FRONTEND=noninteractive
apt="apt-get -q -o Acquire::Retries=3 -o Acquire::http::Timeout=60 -o Acquire::https::Timeout=60"
$apt update
$apt install -y --no-install-recommends ca-certificates git curl clang file python3
# The C library the toolchain file names: from Ubuntu's archive while it
# is the current one there, and after that from the archive's snapshot of
# the day the file names.
libc=$(want libc6-dev)
libc="libc6-dev=$libc libc-dev-bin=$libc libc6=$libc libc-bin=$libc"
# shellcheck disable=SC2086
$apt install -y --no-install-recommends --allow-downgrades $libc || {
	$apt update --snapshot "$(want apt-snapshot)"
	# shellcheck disable=SC2086
	$apt install -y --no-install-recommends --allow-downgrades --snapshot "$(want apt-snapshot)" $libc
}
case "$(uname -m)" in
	x86_64)  rustup_arch=x86_64-unknown-linux-gnu;  rustup_sha=b1c8cb14e30889e7e2999e6b69e62d9e9b9ffc0f4df8180564d306d9680e6c55 ;;
	aarch64) rustup_arch=aarch64-unknown-linux-gnu; rustup_sha=9f608c142e0e38a78957d0661ba1dfaf0b6fb37184cfaa4b9a0aa57af6184282 ;;
	*) echo "b3sum-release-container: unsupported arch $(uname -m)" >&2; exit 1 ;;
esac
curl --proto '=https' --tlsv1.2 -sSfL -o /tmp/rustup-init "https://static.rust-lang.org/rustup/archive/1.29.3/$rustup_arch/rustup-init"
printf '%s  /tmp/rustup-init\n' "$rustup_sha" | sha256sum -c
chmod +x /tmp/rustup-init
/tmp/rustup-init -y --profile minimal --default-toolchain "$(want rustc)"
rm /tmp/rustup-init
. "$HOME/.cargo/env"
git config --global --add safe.directory '*'
sh tools/b3sum-release.sh
# The crates too, packaged as a release packages them: Cargo's build of
# each is the release's own run's to make.
sh tools/crates-release.sh --no-verify
