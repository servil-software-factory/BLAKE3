# b3sum

A command-line tool that prints [BLAKE3](https://blake3.io) hashes of files or
standard input, like `sha256sum` or `md5sum`. This is `blake3-servil`'s
`b3sum` (see the repository's [README](../README.md)).

<img src="../docs/images/b3sum-epyc-1000-files.svg" alt="Hashing a tree of 1000 files on an AMD EPYC 9255: b3sum on every core and on one thread, beside sha1sum, sha256sum and md5sum, in GB/s" width="720">

# Installation

## Prebuilt Linux binaries

Download `b3sum-VERSION-x86_64-unknown-linux-gnu` or
`b3sum-VERSION-aarch64-unknown-linux-gnu` and its `.sha256` file from the
[releases page](https://github.com/servil-software-factory/BLAKE3/releases).
These binaries are statically linked, so they start faster when invoked many
times (for example, once per file). Check the digest, make the binary executable,
and move it to a directory on your `PATH` as `b3sum`:

```bash
sha256sum -c b3sum-VERSION-TARGET.sha256
chmod +x b3sum-VERSION-TARGET
```

## Verify a Linux binary by rebuilding it

The Linux release binaries are reproducible (for releases after 0.9.0). From a
checkout of a release tag, you can rebuild them in Docker or Podman and verify
that your build matches the published file byte for byte:

```sh
docker run --rm -v "$PWD":/src -w /src ubuntu:26.04 sh tools/b3sum-release-container.sh
sha256sum dist/b3sum-*
```

## Build from source

With [Rust and Cargo](https://doc.rust-lang.org/cargo/getting-started/installation.html):

- On Linux (recommended static build; use `aarch64-unknown-linux-gnu` on 64-bit
  Arm):

  ```bash
  RUSTFLAGS="-C target-feature=+crt-static" cargo install b3sums --target x86_64-unknown-linux-gnu
  ```

- On macOS:

  ```bash
  cargo install b3sums
  ```

The package name on crates.io is **`b3sums`** (the `b3sum` package is official
upstream BLAKE3). Installing `b3sums` puts the `b3sum` binary in `~/.cargo/bin`.

Packagers building a release binary from a tag checkout can run
`sh tools/b3sum-release.sh`. It builds from a fresh clone of the tag's commit
and writes a `.sha256` file beside the binary.

# Example

```bash
b3sum foo.txt
```

To time hashing a 1 GB file on your machine:

```bash
head -c 1000000000 /dev/zero > /tmp/bigfile
time sha256sum /tmp/bigfile
time b3sum /tmp/bigfile
```

# Usage

```
Usage: b3sum [OPTIONS] [FILE]...

Arguments:
  [FILE]...  Files to hash, or checkfiles to check

Options:
      --keyed                 Use the keyed mode, reading the 32-byte key from stdin
      --derive-key <CONTEXT>  Use the key derivation mode, with the given context string
  -l, --length <LEN>          The number of output bytes, before hex or sigil encoding [default: 32]
      --seek <SEEK>           The starting output byte offset, before hex or sigil encoding
                              [default: 0]
      --num-threads <NUM>     The most threads hashing at once [default: as fast as b3sum can]
      --no-mmap               Read every file and map none: a file that another program shortens
                              while b3sum hashes it then cannot stop the run
      --no-names              Omit filenames in the output
      --raw                   Write raw output bytes to stdout, rather than hex
      --sigilcode             Write each hash as a sigil (25 characters for 32 bytes; see
                              johnservil/sigil), rather than hex; with --check, read sigils as well
                              as hex
      --tag                   Output BSD-style checksums: BLAKE3 ([FILE]) = [HASH]
  -c, --check                 Read BLAKE3 sums from the [FILE]s and check them
      --quiet                 Skip printing OK for each checked file
      --outboard <DIR>        Also write each file's outboard to DIR/<HASH>.obao4: its BLAKE3 tree
                              over 16 KiB chunk groups in bao-tree's pre-order form, for verified
                              streaming
  -h, --help                  Print help
  -V, --version               Print version

More information: b3sum/README.md at
https://github.com/servil-software-factory/BLAKE3/blob/main/b3sum/README.md
```

By default, `b3sum` uses all CPU cores: it hashes multiple files in parallel and
splits large files across threads. Pass `--num-threads 1` to use a single
thread. On Linux, `taskset -c LIST b3sum ...` pins `b3sum` to specific CPUs.

See [What `--check` does](what_does_check_do.md) for checkfile rules.

## What b3sum reads

`b3sum` reads each file from byte 0 until the operating system reports
end-of-file (`0` bytes). Initial file metadata helps choose the read path, but
never decides where the file ends—so `/proc` files that report 0 bytes are still
hashed completely.

Once `b3sum` opens a file, it keeps using that open file handle. Moving or
replacing the file at that path does not switch which file is hashed.

**Files modified while `b3sum` is reading them:** `b3sum` does not lock files or
take a filesystem snapshot. If another program modifies a file while `b3sum`
reads it, the output is **not** guaranteed to be the hash of any single version
of the file that existed (for example, `b3sum` can read old bytes from the first
part and new bytes from the rest). Only hash files that are not changing, or hash
a snapshot.

Specifically, if a file grows or shrinks mid-read:

- **If the file grows:** Paths that read sequentially (small files, cold files,
  standard input, or `--no-mmap`) read until end-of-file, so they include bytes
  appended before `b3sum` reaches the end. Paths that memory-map the file
  (by default, cached files of 512 KiB or more) only map its size at open, so
  bytes appended afterward are not hashed. Direct I/O reads on cold files
  detect trailing bytes and fall back to reading or mapping.
- **If the file shrinks (is truncated):** When `b3sum` memory-maps the file,
  reading past the new end causes a `SIGBUS` error and `b3sum` aborts without
  printing a hash. When `b3sum` reads sequentially (small files, cold files, or
  `--no-mmap`), hitting the shorter end reports end-of-file early, and `b3sum`
  currently prints the hash of the prefix it had read so far rather than
  detecting that the file shrank.

## Direct I/O for large cold files

On Linux, `b3sum` reads cold files of 4 MiB or more with direct I/O (bypassing
the page cache) when hashing a single file, or when hashing any file with
`--outboard`. Bypassing the cache avoids extra memory copies and leaves the page
cache undisturbed when hashing large sets of files once. On a Ryzen 9700X with
NVMe storage, hashing a cold 1 GiB file drops from 0.22 s through the page cache
to 0.10 s with direct I/O.

## Outboards for verified streaming

`--outboard DIR` writes each file's outboard tree to `DIR/<HASH>.obao4`. This
lets a streaming receiver verify any part of the file against its root hash as
that part arrives.

- The format matches `bao-tree`'s `PreOrderMemOutboard::create` with block size
  4 (16 KiB chunk groups): 64-byte parent nodes in pre-order, with no length
  prefix.
- Files of 16 KiB or less have no parent nodes, so no `.obao4` file is written.
- `DIR` must already exist.
- Works in plain mode and `--keyed` mode.

```bash
$ mkdir outboards && b3sum --outboard outboards big.iso
5da5ad397cd129294c30f415b7d4e091b155a5faa37ba0b154ed10a5e5dbfca9  big.iso
$ ls outboards
5da5ad397cd129294c30f415b7d4e091b155a5faa37ba0b154ed10a5e5dbfca9.obao4
```

## Hashes as sigils

`--sigilcode` prints each 32-byte hash as a 25-character
[sigil](https://github.com/johnservil/sigil) instead of hex. A sigil is a
single word (easy to double-click to select), uses visually distinct characters,
and includes its own error check so typos or swapped adjacent characters fail
before hashing begins. With `--check --sigilcode`, checkfiles may mix sigils and
hex.

```
$ b3sum --sigilcode empty
ᢳᕌᔃጾዿᑂഔጕꔍⶰᣟꕄꕛቴⴲజᐽᙿᕯⶂጁᑃሃᛜጩ  empty
$ b3sum --sigilcode empty > sums.b3 && b3sum --sigilcode --check sums.b3
empty: OK
```

Without `--sigilcode`, `b3sum` expects hex only, and `--check` will tell you if
a line looks like a sigil.
