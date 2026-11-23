#!/bin/sh
# Install hash-verified CI binaries (cargo-nextest 0.9.152, cargo-c 0.10.26)
# into ${DEST:-${CARGO_HOME:-$HOME/.cargo}/bin}, dropping third-party
# installer actions.
#
#     sh tools/ci-tools.sh nextest [cargo-c]
set -eu
DEST=${DEST:-"${CARGO_HOME:-$HOME/.cargo}/bin"}
if [ "$#" -eq 0 ]; then
  echo "usage: sh tools/ci-tools.sh nextest|cargo-c ..." >&2
  exit 2
fi

verify_sha256() {
  sha=$1
  file=$2
  sum="$tmp/sum"
  printf '%s  %s\n' "$sha" "$file" > "$sum"
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 -c "$sum"
  else
    sha256sum -c "$sum"
  fi
}

install_nextest() {
  ver=0.9.152
  case "$(uname -sm)" in
    "Linux x86_64")
      target=x86_64-unknown-linux-gnu
      sha=d039b43c3e0958441b6ad0a39c8884d018b3d2e8fb9928f209074b1100b5e17a
      # BLAKE3: 31690208bdb8da793c0480fc470e82662f69c18edf845465b65099f8027a767c
      ;;
    "Linux aarch64")
      target=aarch64-unknown-linux-gnu
      sha=ca1f81a5b86533eb45ab97e0064aa88e925e53aab9a698ca6331a7cad1f85f07
      # BLAKE3: 5364c97094b42f458661d2b1e0626de7b0bb3beeb2e32ab82129d5d1c39af3ab
      ;;
    "Darwin arm64"|"Darwin x86_64")
      target=universal-apple-darwin
      sha=2fadf57df4243e691f088ee3bff4f4ab9af16da0c95a1db84b5cb1685e098a76
      # BLAKE3: 5a6fafdb3f1e4c18116a600ea264eaae7a615a0723fb9dc9756ff06ceab8cdf0
      ;;
    *) echo "ci-tools: no cargo-nextest binary named for $(uname -sm)" >&2; exit 1 ;;
  esac
  archive="$tmp/cargo-nextest.tar.gz"
  curl --proto '=https' --tlsv1.2 -sSfL -o "$archive" \
    "https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-${ver}/cargo-nextest-${ver}-${target}.tar.gz"
  verify_sha256 "$sha" "$archive"
  mkdir -p "$DEST"
  tar -xzf "$archive" -C "$DEST" cargo-nextest
  "$DEST/cargo-nextest" --version
}

install_cargo_c() {
  ver=0.10.26
  case "$(uname -sm)" in
    "Linux x86_64")
      asset=cargo-c-x86_64-unknown-linux-musl.tar.gz
      sha=685fab5a2e1539c5075cea21c42469ee5a489d877dbdbf7adccf4bb631d77545
      # BLAKE3: 73e8bc49237e44e8160d594b6e0e9cb2e659c41499cfed66c36db95719f2dbcb
      ;;
    "Linux aarch64")
      asset=cargo-c-aarch64-unknown-linux-musl.tar.gz
      sha=68a45c967438240b64f2806639098c89df676c0152e855f5122cf5173204ce6c
      # BLAKE3: 6c80df600f3e4e5aa548d8f85d2a44b20add795b05522d431cbf86bffbd26c58
      ;;
    "Darwin arm64"|"Darwin x86_64")
      asset=cargo-c-macos.zip
      sha=891ec1f27eb48f24ed615fd52180528c71295a2c9049b306c204e15e53130354
      # BLAKE3: 72ee5199219b8a47316f9fae23debb7c83c9bc647708798c8a3c4bd317b97488
      ;;
    *) echo "ci-tools: no cargo-c binary named for $(uname -sm)" >&2; exit 1 ;;
  esac
  archive="$tmp/$asset"
  curl --proto '=https' --tlsv1.2 -sSfL -o "$archive" \
    "https://github.com/lu-zero/cargo-c/releases/download/v${ver}/${asset}"
  verify_sha256 "$sha" "$archive"
  mkdir -p "$DEST"
  case "$asset" in
    *.tar.gz) tar -xzf "$archive" -C "$DEST" cargo-capi cargo-cbuild cargo-cinstall cargo-ctest ;;
    *.zip)
      unzip -oq "$archive" -d "$DEST" cargo-capi cargo-cbuild cargo-cinstall cargo-ctest
      chmod +x "$DEST/cargo-capi" "$DEST/cargo-cbuild" "$DEST/cargo-cinstall" "$DEST/cargo-ctest"
      ;;
  esac
  "$DEST/cargo-cinstall" --version
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

for tool in "$@"; do
  case "$tool" in
    nextest) install_nextest ;;
    cargo-c) install_cargo_c ;;
    *) echo "ci-tools: unknown tool $tool (nextest, cargo-c)" >&2; exit 2 ;;
  esac
done
