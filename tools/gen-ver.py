"""The crate's version: a release's, the code after it, and the check.

    python3 tools/gen-ver.py X.Y.Z    # a release, from a clean tree
    python3 tools/gen-ver.py --dev    # the code after a release: X.Y.Z+dev
    python3 tools/gen-ver.py --check  # the version names this commit truly

One commit, on top of whatever HEAD is, setting the crate's version to
X.Y.Z+<HEAD's hash>, and the tag vX.Y.Z+<hash> on it. A stamp is whole or
not at all: where it stops, its own files go back as they were and its
commit is taken back, and no other file or commit is touched. (Run it in a
checkout nothing else writes to for those seconds; a process killed
outright leaves what it had written, which `git status` shows.) The hash inside the
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

# No compiled copy of the module beside it: a stamp needs a clean tree,
# and must not be what dirties it.
sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import release_artifacts


def git(*args):
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout.strip()


def report(kind, error, trace):
    """A command that failed, in its own words: Python would print where
    in this file it was run, and not what it said."""
    if isinstance(error, subprocess.CalledProcessError):
        print(f"{' '.join(map(str, error.cmd))}: exit status {error.returncode}: {(error.stderr or '').strip()}", file=sys.stderr)
    else:
        sys.__excepthook__(kind, error, trace)


sys.excepthook = report


def edit(path, pattern, replacement):
    """Replace the first line matching `pattern`; stop if none does. Every
    other byte stays: the file is read and written with its line ends as
    they are."""
    with open(path, newline="") as f:
        text, n = re.subn(pattern, replacement, f.read(), count=1, flags=re.M)
    if n != 1:
        sys.exit(f"{path}: no line matches {pattern}")
    with open(path, "w", newline="") as f:
        f.write(text)


RELEASE = re.compile(r"([0-9]+\.[0-9]+\.[0-9]+)\+([0-9a-f]{40}|dev)")
# The notes' title as a stamp can replace it: the version is the whole
# token up to a space, a comma or the line's end, in ASCII digits (the
# release check reads bytes; Python's \d and \s also take other scripts'
# digits and spaces). The check and the stamp read the title by this one
# pattern: where they differed, a stamp replaced more than the check saw.
TITLED = r"# blake3-servil [0-9]+\.[0-9]+\.[0-9]+\+(?:[0-9a-f]{40}|dev)"
TITLE = re.compile(TITLED + r"(?:[ \t,].*)?")


def notes_title():
    """The notes' first line: up to the first line feed, and no other
    character ends it (Python's splitlines ends a line at a form feed and
    at a dozen more, which the stamp reads through)."""
    with open("CHANGELOG.md", newline="") as f:
        return f.read().split("\n", 1)[0]


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
    title = notes_title()
    # The whole token set_version replaces (up to a space or a comma): a
    # title "0.10.0+dev-preview," passed a looser test here, and its stamp,
    # which drops "-preview", was then refused by the release check.
    if not TITLE.fullmatch(title):
        sys.exit(f"CHANGELOG.md: its title must name a version as X.Y.Z+dev or X.Y.Z+<commit> (found {title!r})")
    # On a version commit the title names the release, as every other copy
    # does: the stamp writes it, and a title put back by hand to X.Y.Z+dev
    # or to an older release's would go out over the new release's notes.
    if not full.endswith("+dev") and not title.startswith((f"# blake3-servil {full} ", f"# blake3-servil {full},", f"# blake3-servil {full}\t")) and title != f"# blake3-servil {full}":
        sys.exit(f"CHANGELOG.md: its title names the release {full} on its version commit (found {title!r})")
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


def stamp_files(notes=True):
    """The files a stamp writes, and no others: what it commits, and what
    it puts back where it stops."""
    return [*PACKAGES.values(), *(["CHANGELOG.md"] if notes else []), "b3sum/Cargo.lock", RELEASE_LOCK, "capi/b3.h"]


def put_back(files):
    """The stamp's own files as the commit under them has them. No other
    file is touched: what someone saved elsewhere in the tree while the
    stamp ran is theirs."""
    git("checkout", "-q", "HEAD", "--", *files)


def stamped(full, message, notes=True):
    """The version written into the stamp's files and committed, whole or
    not at all: where it stops after it has written (Cargo's update of
    b3sum's lock fails, a copy does not agree, the commit is refused),
    its files go back as they were, and only they. Returns the commit."""
    files = stamp_files(notes)
    try:
        set_version(full, notes)
        git("commit", "-q", "--no-verify", "-m", message, "--", *files)
    except BaseException:
        put_back(files)
        raise
    return git("rev-parse", "HEAD")


def taken_back(stamp, notes=True):
    """Undo the stamp's commit: the branch back at its parent and the
    stamp's files as they are there. Only where the commit is still the
    branch's last: if something else has moved the branch since, nothing
    is moved or written, and the answer is False."""
    if git("rev-parse", "HEAD") != stamp:
        return False
    git("reset", "-q", "--soft", "HEAD^")
    put_back(stamp_files(notes))
    return True


def set_version(full, notes=True):
    """Stamp the packages and their pins; keep a past release's notes with --dev."""
    if notes:
        # The title as it is must be one a stamp may replace, checked
        # before anything is written: the stamp writes the whole token
        # anew, and the check after it would see only the new one.
        title = notes_title()
        if not TITLE.fullmatch(title):
            sys.exit(f"CHANGELOG.md: its title must name a version as X.Y.Z+dev or X.Y.Z+<commit> before a stamp replaces it (found {title!r})")
    lines = [(path, r'^version = ".*"$', f'version = "{full}"') for path in PACKAGES.values()]
    lines += [(path, rf'^({dependency} = \{{ version = ")[^"]*', rf"\g<1>={full}") for path, dependency in PINS.items()]
    if notes:
        lines.append(("CHANGELOG.md", r"\A" + TITLED + r"(?=[ \t,\n]|\Z)", f"# blake3-servil {full}"))
    lines.append(("capi/b3.h", r'^#define B3_VERSION ".*"$', f'#define B3_VERSION "{full}"'))
    # The release lock names the published packages too, as its
    # workspace's members: no workspace of theirs is in the repository for
    # Cargo to update it by.
    lines += [(RELEASE_LOCK, rf'^(name = "{re.escape(name)}"\nversion = ")[^"]*', rf"\g<1>{full}") for name in PACKAGES if name != "blake3-servil-c"]
    # Every line to replace is found before any is written: a stamp that
    # stopped at a file it could not read as it expects (one with carriage
    # returns before its line feeds, say) had by then written the files
    # before it.
    for path, pattern, _ in lines:
        with open(path, newline="") as f:
            if not re.search(pattern, f.read(), flags=re.M):
                sys.exit(f"{path}: no line matches {pattern}; nothing is written")
    for path, pattern, replacement in lines:
        edit(path, pattern, replacement)
    # The CLI's tracked lock contains all four published packages.
    subprocess.run(["cargo", "update", "-q", *[a for name in PACKAGES if name != "blake3-servil-c" for a in ("-p", name)]],
                   cwd="b3sum", check=True)
    check_copies(full)


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
    stamped(full, f"Version {full}: the code after {release}", notes=False)
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
# Asked before anything is written: a stamp with no remote to name still
# stamps, and says so in the lines it prints.
try:
    origin = git("remote", "get-url", "origin")
except subprocess.CalledProcessError:
    origin = "<this repository's URL>"
# The commit is made with --no-verify: the pre-commit hook's speed check
# measures code, and this commit changes version strings alone (AGENTS:
# follow a rule for its purpose).
stamp = stamped(full, f"Version {full}")
# The release check's own reading of a version commit, before the tag
# names this one: its parent with the version written in, and nothing
# else. A stamp can write more than that where a tool it runs rewrites a
# whole file (Cargo's update of b3sum's lock drops a comment there), and
# the release check then refused a tag that already stood. The tag names
# the commit that was read, whatever the branch's last commit is by then.
try:
    release_artifacts.Release(f"v{full}", repo=os.getcwd(), commit=stamp)
    git("tag", f"v{full}", stamp)
except BaseException as refused:
    reason = refused.stderr.strip() if isinstance(refused, subprocess.CalledProcessError) else refused
    if taken_back(stamp):
        sys.exit(f"the stamp is undone: no tag is made, and its files are as they were. {reason}")
    sys.exit(f"no tag is made, and the stamp's commit {stamp} is left where it is: the branch has moved on since. {reason}")
print(f"v{full}")
# The dependency line for the release notes: the release commit by its full
# hash, which Cargo fetches exactly, and the full version.
for crate in ("blake3-servil-core", "blake3-servil"):
    print(f'{crate} = {{ git = "{origin}", rev = "{stamp}", version = "={full}" }}')
