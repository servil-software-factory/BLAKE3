# Security

## Reporting a vulnerability

Report it privately, through GitHub's
[private vulnerability report](https://github.com/servil-software-factory/BLAKE3/security/advisories/new)
for this repository. Please do not open a public issue for something an
attacker could use before a fix is released. A report is useful with the
version (the full one, `b3sum --version` or the crate's, which carries
the source commit), the machine, and the smallest input or program that
shows the problem.

We answer in the report itself. A confirmed problem in a released
version gets a fixed release, a GitHub security advisory, and, where
the released version should no longer be used, a yank on crates.io.
0.8.0 was withdrawn that way:
[GHSA-mf3f-vh3v-f7v6](https://github.com/servil-software-factory/BLAKE3/security/advisories/GHSA-mf3f-vh3v-f7v6).

A bug that is not a vulnerability goes in an ordinary
[issue](https://github.com/servil-software-factory/BLAKE3/issues).

## Supported versions

The latest release. Fixes are not carried back to earlier ones.

## What is in scope

The two libraries (`blake3-servil-core`, `blake3-servil`), the C
library, and the release tools that decide what users download
(`tools/release_artifacts.py`, `tools/crates-release.sh`, and the workflows
that run them). The `b3sum` CLI lives in
[`servil-software-factory/b3sum`](https://github.com/servil-software-factory/b3sum).
The BLAKE3 algorithm itself is the BLAKE3 team's.

[QUALITY.md](QUALITY.md) says what the library checks cover and what
remains the caller's responsibility. Usage guidance for the `b3sum` CLI
lives in [`servil-software-factory/b3sum`](https://github.com/servil-software-factory/b3sum).

## Checking what you downloaded

A release's tag is `vX.Y.Z+<commit>`, and those 40 hex digits are the
hash of the source it was made from. From a clone of this repository:

```sh
git checkout <those 40 hex digits>
python3 tools/release_artifacts.py vX.Y.Z+<commit> --published
```

checks that the tag still names that source, and that every published
crate is made from it. (Releases after 0.9.0.)
