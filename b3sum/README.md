# b3sum

A command-line tool that prints [BLAKE3](https://blake3.io) hashes of files or standard
input, like `sha256sum` or `md5sum`. This is `blake3-servil`'s `b3sum` (see the repository's [README](../README.md)).

<img src="../docs/images/b3sum-epyc-1000-files.svg" alt="Hashing a tree of 1000 files on an AMD EPYC 9255: b3sum on every core and on one thread, beside sha1sum, sha256sum and md5sum, in GB/s" width="720">

# Installation

On Linux, download the binary for your machine from the
[releases page](https://github.com/servil-software-factory/BLAKE3/releases):
`b3sum-VERSION-x86_64-unknown-linux-gnu` or
`b3sum-VERSION-aarch64-unknown-linux-gnu`, each beside its SHA-256 and
BLAKE3 digests. They are statically linked, so each run starts in about
two thirds of the time a dynamically linked build takes, which matters
where `b3sum` runs once per file. Check both digests, then put it on
your `PATH` as `b3sum`:

```
sha256sum -c b3sum-VERSION-TARGET.sha256
chmod +x b3sum-VERSION-TARGET && ./b3sum-VERSION-TARGET -c b3sum-VERSION-TARGET.b3
```

To build it yourself on Linux, with [Rust and Cargo](https://doc.rust-lang.org/cargo/getting-started/installation.html),
link it statically too, giving your machine's target
(`aarch64-unknown-linux-gnu` on 64-bit Arm):

```
RUSTFLAGS="-C target-feature=+crt-static" cargo install b3sums --target x86_64-unknown-linux-gnu
```

On macOS, `cargo install b3sums`. The package is `b3sums` (`b3sum` on
crates.io is official BLAKE3's); the program it installs is `b3sum`, in
`~/.cargo/bin`. (`cargo install b3sum` and the prebuilt binaries on
[official BLAKE3's releases page](https://github.com/BLAKE3-team/BLAKE3/releases)
give official BLAKE3's `b3sum`.)

**For packagers:** build the release binary with `sh tools/b3sum-release.sh`
from a clean checkout of a release tag. On Linux it links statically, as
above, and it writes the binary's SHA-256 and BLAKE3 digests beside it.

# Example

```bash
b3sum foo.txt
```

Time hashing a gigabyte, to see how fast it is on your machine:

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
      --no-mmap               No effect: b3sum chooses between reading and mapping each file
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
https://github.com/servil-software-factory/BLAKE3/blob/servil/b3sum/README.md
```

By default b3sum uses every CPU: several files at once, and a large file over several
threads. `--num-threads 1` hashes on one thread, file by file, leaving the other cores to
other work (or to other b3sums). On Linux, `taskset -c LIST b3sum ...` runs it on chosen
CPUs.

[What `--check` does](what_does_check_do.md) explains checkfiles.

## Outboards for verified streaming

`--outboard DIR` also writes each file's outboard to `DIR/<HASH>.obao4`: the file's
BLAKE3 tree above its chunk groups of 16 KiB, which lets a reader verify any part of
the file against its hash as the part arrives. The bytes are those bao-tree's
`PreOrderMemOutboard::create` makes for block size 4 (b3sum's tests compare the two):
each parent node's two children, 64 bytes a node, the nodes in pre-order, with no length
before them. A file of 16 KiB or less has an empty outboard. `DIR` must exist.

```bash
$ mkdir outboards && b3sum --outboard outboards big.iso
5da5ad397cd129294c30f415b7d4e091b155a5faa37ba0b154ed10a5e5dbfca9  big.iso
$ ls outboards
5da5ad397cd129294c30f415b7d4e091b155a5faa37ba0b154ed10a5e5dbfca9.obao4
```

It works in the plain and keyed modes, the two bao-tree reads.

On Linux, a file of 4 MiB or more that the page cache does not hold is read past the
cache (direct I/O), several reads at once, which is the fast way to read a large set of
files once. The cache does not hold such a file afterwards.

## Hashes as sigils

`--sigilcode` writes each hash as a [sigil](https://github.com/johnservil/sigil),
one word of 26 characters for a 32-byte hash. A double-click selects it
whole, and no two of its characters look alike. With `--check`, the same
flag reads sigils as well as hex, so a checkfile may mix the two:

```
$ b3sum --sigilcode empty
ᢳᕌᔃጾዿᑂഔጕꔍⶰᣟꕄꕛቴⴲజᐽᙿᕯⶂጁᑃሃᛜጩ  empty
$ b3sum --sigilcode empty > sums.b3 && b3sum --sigilcode --check sums.b3
empty: OK
```

Without the flag, `b3sum` writes and reads hex alone, as every other
BLAKE3 tool does. A sigil carries its own check: a changed character, or
two neighbours swapped, makes its line fail to read before the file is
hashed. Without the flag, `--check` says when a line's digest looks like a sigil.
