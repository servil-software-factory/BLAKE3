"""The crate's version: a release's, the code after it, and the check.

    python3 tools/gen-ver.py X.Y.Z    # a release, from a clean tree
    python3 tools/gen-ver.py --dev    # the code after a release: X.Y.Z+dev
    python3 tools/gen-ver.py --check  # the version names this commit truly

One commit, on top of whatever HEAD is, setting the crate's version to
X.Y.Z+<HEAD's hash>, and the tag vX.Y.Z+<hash> on it. The hash inside the
version string names the code commit, so whoever holds the string and
nothing else can later check they have the right code (a string cannot
hold the hash of the commit it sits in, so it holds its parent's; the
release commit changes version strings alone). Zooko's scheme, shared
with his other projects.

Every copy of the version carries it in full, hash included, whether or
not today's tools check it, since future tools may (Zooko, October 20):
both libraries (crates.io, their docs) and their build version's crate
(version/, its exact dependency), the tag, and, where the repository has
them, the CHANGELOG's title, b3sum's own version (`b3sum --version`) and
its exact requirement on the crate (`=X.Y.Z+<hash>`: Cargo reads it up to
the `+` today, and warns), the C library's crate (`b3_version()`), header
(`B3_VERSION`); cargo-c generates the pkg-config version from the crate.
The soname alone takes X.Y: its format holds digits and dots.

A release's version names its code, so the commits after it must not keep
it: the first change after a release, on any branch, sets X.Y.Z+dev
(`--dev`, in every copy but the CHANGELOG's title, which names the
release its notes describe). A reader who knows the releases then reads "newer
than X.Y.Z" in the version alone (bench-hashes' map shows it so). `--check`
holds the rule, run by tools/check-builds.sh: the version is X.Y.Z+dev, or
X.Y.Z+<hash> on the commit whose parent is <hash> (the release commit).
"""
import os
import re
import subprocess
import sys
import tomllib
from pathlib import Path


def git(*args):
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout.strip()


def edit(path, pattern, replacement):
    """Replace the first line matching `pattern`; stop if none does."""
    with open(path) as f:
        text, n = re.subn(pattern, replacement, f.read(), count=1, flags=re.M)
    if n != 1:
        sys.exit(f"{path}: no line matches {pattern}")
    with open(path, "w") as f:
        f.write(text)


RELEASE = re.compile(r"(\d+\.\d+\.\d+)\+([0-9a-f]{40}|dev)")


def current():
    """The crate's version, split: its release number and its +part."""
    version = re.search(r'^version = "(.*)"$', open("Cargo.toml").read(), re.M).group(1)
    m = RELEASE.fullmatch(version)
    if not m:
        sys.exit(f"Cargo.toml's version {version!r} is neither X.Y.Z+<commit> nor X.Y.Z+dev")
    return m.group(1), m.group(2)


# Dependency order for the published crates; the C package ships as libraries.
PACKAGES = {
    "blake3-servil-version": "version/Cargo.toml",
    "blake3-servil-core": "Cargo.toml",
    "blake3-servil": "threads/Cargo.toml",
    "b3sums": "b3sum/Cargo.toml",
    "blake3-servil-c": "capi/Cargo.toml",
}
PINS = {
    "Cargo.toml": "blake3-servil-version",
    "threads/Cargo.toml": "blake3-servil-core",
    "b3sum/Cargo.toml": "blake3",
}


RELEASE_LOCK = "tools/release/Cargo.lock"


def check_copies(full):
    """Each package, dependency pin, header and CLI lock names the same code."""
    for name, path in PACKAGES.items():
        manifest = tomllib.loads(Path(path).read_text())
        if manifest["package"]["name"] != name or manifest["package"]["version"] != full:
            sys.exit(f"{path}: expected {name} {full}")
        if path in PINS and manifest["dependencies"][PINS[path]]["version"] != "=" + full:
            sys.exit(f"{path}: {PINS[path]} must require ={full}")
    # The release's notes are titled with a version a release's stamp can
    # replace: the release check (tools/release_artifacts.py) allows a
    # version commit to change a line only from X.Y.Z+dev, or
    # X.Y.Z+<commit>, to the release's. A title such as "0.10.0+SHA"
    # made the 0.10.0 candidate's version commit one that check refused.
    title = Path("CHANGELOG.md").read_text().splitlines()[0]
    if not re.fullmatch(r"# blake3-servil \d+\.\d+\.\d+\+(?:[0-9a-f]{40}|dev)\b.*", title):
        sys.exit(f"CHANGELOG.md: its title must name a version as X.Y.Z+dev or X.Y.Z+<commit> (found {title!r})")
    if f'#define B3_VERSION "{full}"' not in Path("capi/b3.h").read_text().splitlines():
        sys.exit(f"capi/b3.h: B3_VERSION must be {full}")
    lock = tomllib.loads(Path("b3sum/Cargo.lock").read_text())
    release = tomllib.loads(Path(RELEASE_LOCK).read_text())
    for name in PACKAGES.keys() - {"blake3-servil-c"}:
        for path, packages in (("b3sum/Cargo.lock", lock), (RELEASE_LOCK, release)):
            versions = [p["version"] for p in packages["package"] if p["name"] == name]
            if versions != [full]:
                sys.exit(f"{path}: expected one {name} at {full}, found {versions}")
    # The crates are packaged with the release lock (tools/crates-release.sh)
    # and b3sum is built and tested with its own: one set of dependencies,
    # so every package of b3sum's lock is in the release lock as it is there.
    pinned = {(p["name"], p["version"]): p.get("checksum") for p in release["package"]}
    for p in lock["package"]:
        key = (p["name"], p["version"])
        if key not in pinned or pinned[key] != p.get("checksum"):
            sys.exit(f"{RELEASE_LOCK}: b3sum/Cargo.lock has {p['name']} {p['version']}, and the release lock has "
                     f"{sorted(v for n, v in pinned if n == p['name']) or 'none'}: the crates would ship another dependency than b3sum is "
                     f"tested with (CONTRIBUTING.md, \"The release lock\")")


def set_version(full, notes=True):
    """Stamp the packages and their pins; keep a past release's notes with --dev."""
    files = list(PACKAGES.values())
    for path in files:
        edit(path, r'^version = ".*"$', f'version = "{full}"')
    for path, dependency in PINS.items():
        edit(path, rf'^({dependency} = \{{ version = ")[^"]*', rf"\g<1>={full}")
    if notes:
        edit("CHANGELOG.md", r"^# blake3-servil [^\s,]+", f"# blake3-servil {full}")
        files.append("CHANGELOG.md")
    edit("capi/b3.h", r'^#define B3_VERSION ".*"$', f'#define B3_VERSION "{full}"')
    # The CLI's tracked lock contains all four published packages.
    subprocess.run(["cargo", "update", "-q", *[a for name in PACKAGES if name != "blake3-servil-c" for a in ("-p", name)]],
                   cwd="b3sum", check=True)
    # The release lock names them too, as its workspace's members: no
    # workspace of theirs is in the repository for Cargo to update it by.
    for name in PACKAGES:
        if name != "blake3-servil-c":
            edit(RELEASE_LOCK, rf'^(name = "{re.escape(name)}"\nversion = ")[^"]*', rf"\g<1>{full}")
    files += ["b3sum/Cargo.lock", RELEASE_LOCK, "capi/b3.h"]
    check_copies(full)
    return files


os.chdir(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
if sys.argv[1:] == ["--check"]:
    release, named = current()
    check_copies(f"{release}+{named}")
    if named != "dev" and git("rev-parse", "HEAD^") != named:
        sys.exit(f"the version {release}+{named[:12]} names a release's code, and this commit is past that release's: "
                 f"set {release}+dev (python3 tools/gen-ver.py --dev)")
    sys.exit(0)
if sys.argv[1:] == ["--dev"]:
    if git("status", "--porcelain"):
        sys.exit("the tree must be clean")
    release, named = current()
    if named == "dev":
        sys.exit(f"the version is {release}+dev already")
    full = f"{release}+dev"
    files = set_version(full, notes=False)
    git("commit", "-q", "--no-verify", "-m", f"Version {full}: the code after {release}", "--", *files)
    print(full)
    sys.exit(0)
if len(sys.argv) != 2 or not re.fullmatch(r"\d+\.\d+\.\d+", sys.argv[1]):
    sys.exit("usage: python3 tools/gen-ver.py X.Y.Z | --dev | --check")
version = sys.argv[1]
if git("status", "--porcelain"):
    sys.exit("the tree must be clean")
number = lambda v: tuple(map(int, v.split("+")[0].split(".")))
tags = [number(t[1:]) for t in git("tag", "-l", "v*").split()]
if tags and number(version) <= max(tags):
    sys.exit(f"{version} is not newer than the latest tag, v{'.'.join(map(str, max(tags)))}")

full = f"{version}+{git('rev-parse', 'HEAD')}"
files = set_version(full)
# --no-verify: the pre-commit hook's speed check measures code, and this
# commit changes version strings alone (AGENTS: follow a rule for its purpose).
git("commit", "-q", "--no-verify", "-m", f"Version {full}", "--", *files)
git("tag", f"v{full}")
print(f"v{full}")
# The dependency line for the release notes: the release commit by its full
# hash, which Cargo fetches exactly, and the full version.
for crate in ("blake3-servil-core", "blake3-servil"):
    print(f'{crate} = {{ git = "{git("remote", "get-url", "origin")}", rev = "{git("rev-parse", "HEAD")}", version = "={full}" }}')
