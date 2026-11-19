#!/usr/bin/env python3
"""Check that a release's artifacts are clean builds of its version commit.

    python3 tools/release_artifacts.py TAG FILE...       artifacts on disk
    python3 tools/release_artifacts.py TAG --published   what users download: the GitHub
                                                         release's assets, crates.io's packages
    ... --rebuilt DIR    each b3sum program and each crate is also, byte for byte, the one in DIR
                         (and the one in each folder inside DIR: a builder's each)
    ... --builders A,B   with --rebuilt: the builders are the folders A and B inside DIR, and no
                         others; each must be there, and must hold every crate and a program;
                         DIR holds nothing else to compare with
    ... --commit COMMIT  the version commit, where its tag is not made yet

TAG is a release's tag, vX.Y.Z+<code commit>: it names the version commit
(tools/gen-ver.py), whose parent is the code commit. Run this from a clone
that has the tag; anyone can. It prints one line an artifact. Exit status
0: every artifact is a clean build of the version commit; 1: one is not,
or one a release has is missing.

The tag's name is the release's source. The 40 hex digits in it (which a
program's --version prints too) are the code commit's hash, and this
check holds the version commit to that commit: the same files, but for
the release's version written where a version stood. So a tag that was
moved, or a version commit with anything else in it, is no release here,
and every check below is then of the source the name commits to. With
--published the version commit must also be in the history of the
remote's main branch, which only a release moves.

Take this checker from a commit you trust, not from the tag's own: the
code commit the name gives (git checkout <those 40 digits>) or main. A
forged version commit could carry a checker that passes it.

What makes an artifact clean, by its kind:

  A program or a library (b3sum-*, *.so, *.a, *.dylib, and the libraries
  in a .tar.gz): it holds its build version (version/build_version.rs),
  which is the release's version alone when it was built in a Git
  checkout at the version commit with no file changed or added (or from
  the release's own package, where there is no Git to ask). Every other
  build says more: its commit, and from a dirty tree `.dirty` and the
  tree's id. The file's bytes are searched, so a program for another
  machine is checked without running it. This is the build's own word:
  it catches a mistake, as 0.9.0's was, and not a forgery.

  A b3sum program or a .crate, with --rebuilt DIR: it is also the same
  bytes as a second one made from the version commit on another machine
  (tools/b3sum-release.sh, tools/crates-release.sh) and put in DIR under
  the same name. Each script's output is a function of the commit and of
  the toolchain and lock the repository names, so two honest builders
  agree; an artifact that differs from yours was not made from the
  commit, whatever it says of itself. For a crate this covers the files
  Cargo writes too. Before a release the two builders are our machine
  and GitHub's (tools/release-check.py); after it, the `release
  artifacts` workflow makes again what was published, and so can anyone.
  DIR may hold a folder for each of several builders (the workflow's
  jobs each put theirs in one): the artifact is then compared with every
  builder's copy, and a builder that made crates made this one. Copied
  into one folder instead, a later builder's file would replace an
  earlier one's unread, and a builder that disagreed would go unseen.

  A .crate: Cargo's record in it (.cargo_vcs_info.json) names the version
  commit and no dirty tree, and every file in it is that commit's file,
  byte for byte. Three files are Cargo's own and are not compared with
  the commit: Cargo.toml (its rewriting of the manifest, whose original,
  Cargo.toml.orig, is compared), Cargo.lock, and the record itself.
  --rebuilt covers those: the whole archive against a second packaging.

Why (BLAKE3 #435): 0.9.0's two b3sum programs were built in checkouts that
held untracked files. They say `dirty`, nothing checked, and nobody saw
until a user's question. No source file differed, as the trees their
versions name showed, but only on our machines. Those two programs are
this check's control: `release_artifacts.py v0.9.0+73ee859e... --published`
fails on them and passes the release's library and its four crates.
"""

import hashlib
import io
import json
import os
import re
import subprocess
import sys
import tarfile
import urllib.error
import urllib.parse
import urllib.request

REPO = "servil-software-factory/BLAKE3"
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# The crates a release publishes, by their folders here (CONTRIBUTING.md,
# "Releases"; the C library's and the test vectors' are not published).
CRATE_DIRS = ("version", "", "threads", "b3sum")
# The machines a release has a b3sum program for (b3sum/README.md,
# "Installation").
B3SUM_TARGETS = ("x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu")
# A version as tools/gen-ver.py writes one: a release's, or the
# development line's.
STAMP = re.compile(rb"\d+\.\d+\.\d+\+(?:[0-9a-f]{40}|dev)")
LIBRARY = re.compile(r"\.(so|a|dylib)(\.[0-9.]+)?$")
# Cargo's own files in a .crate: not the commit's.
CARGOS = (".cargo_vcs_info.json", "Cargo.toml", "Cargo.lock")


class NotARelease(Exception):
    """The tag is not a release's, as tools/gen-ver.py makes them."""


class Release:
    """A release by its tag: its version, and the version commit whose
    tree its artifacts are built from. Raises NotARelease for any other
    tag."""

    def __init__(self, tag, repo=None, commit=None):
        """`commit`: the version commit where the caller has it (a
        candidate whose tag is not made yet); else the tag's, in `repo`
        (this checkout's repository unless another is named)."""
        self.tag, self.repo, self.version = tag, repo or ROOT, tag[1:]
        named = re.fullmatch(r"v\d+\.\d+\.\d+\+([0-9a-f]{40})", tag)
        if not named:
            raise NotARelease(f"{tag} is no release tag (vX.Y.Z+<the code commit's 40 hex digits>)")
        if commit is None:
            commit = self.found(tag + "^{commit}")
            if commit is None:
                raise NotARelease(f"this clone has no tag {tag}: fetch it (git fetch origin tag {tag})")
        self.commit = commit
        if self.found(commit + "^") != named.group(1):
            raise NotARelease(f"{tag} names commit {commit}, whose parent is not {named.group(1)}: not a version commit")
        more = self.stamp_problems(named.group(1))
        if more:
            raise NotARelease(f"{tag} names commit {commit}, which is not its parent with the version written in: {more[0]}"
                              + (f" (and {len(more) - 1} more)" if len(more) > 1 else "") + ": not a version commit")

    def stamp_problems(self, code):
        """Why the version commit is not the code commit `code` with the
        release's version written in: nothing where each file is the
        same but for lines in which a version (X.Y.Z+<commit>, or
        X.Y.Z+dev) became this release's. tools/gen-ver.py makes it so,
        and nothing else belongs in a version commit: its hash is not in
        the tag's name, and the code commit's is."""
        problems = []
        raw = subprocess.run(["git", "-C", self.repo, "diff-tree", "-r", "--raw", "--no-renames", "-z", code, self.commit], capture_output=True, check=True).stdout
        fields = raw.split(b"\0")
        for entry, path in zip(fields[0::2], fields[1::2]):
            old_mode, new_mode, old, new, status = entry.decode().lstrip(":").split()
            path = path.decode(errors="replace")
            if status != "M" or old_mode != new_mode:
                problems.append(f"{path} is added, removed, or of another kind")
                continue
            before, after = (subprocess.run(["git", "-C", self.repo, "cat-file", "blob", blob], capture_output=True, check=True).stdout.split(b"\n") for blob in (old, new))
            if len(before) != len(after):
                problems.append(f"{path} has lines added or removed")
                continue
            for number, (was, now) in enumerate(zip(before, after), 1):
                if was == now:
                    continue
                stamps_were, stamps_are = STAMP.findall(was), STAMP.findall(now)
                written = [n for w, n in zip(stamps_were, stamps_are) if w != n]
                # The same count of stamps: with one removed or added, what
                # is left around them can still read the same.
                if len(stamps_were) != len(stamps_are) or STAMP.split(was) != STAMP.split(now) or any(n != self.version.encode() for n in written):
                    problems.append(f"{path} line {number} changes more than a version to this release's")
                    break
        return problems

    def on_main(self):
        """Whether the version commit is in the history of the remote's
        main branch: True or False, or None where this clone has no
        origin/main to ask."""
        if self.found("refs/remotes/origin/main") is None:
            return None
        return subprocess.run(["git", "-C", self.repo, "merge-base", "--is-ancestor", self.commit, "refs/remotes/origin/main"]).returncode == 0

    def found(self, revision):
        """The commit `revision` names here, or None."""
        out = subprocess.run(["git", "-C", self.repo, "rev-parse", "--verify", "--quiet", revision], capture_output=True, text=True)
        return out.stdout.strip() if out.returncode == 0 else None

    def git(self, *args):
        return subprocess.run(["git", "-C", self.repo, *args], capture_output=True, check=True).stdout.decode().strip()

    def file(self, path):
        """The bytes of `path` in the version commit, through any symbolic
        links (Cargo packages what a link points at); None where the
        commit has no such file."""
        for _ in range(8):
            path = os.path.normpath(path)
            if path.startswith("..") or os.path.isabs(path):
                return None
            entry = self.git("ls-tree", self.commit, "--", path)
            if not entry:
                return None
            mode, kind, blob = entry.split("\t")[0].split()
            if kind != "blob":
                return None
            data = subprocess.run(["git", "-C", self.repo, "cat-file", "blob", blob], capture_output=True, check=True).stdout
            if mode != "120000":
                return data
            path = os.path.join(os.path.dirname(path), data.decode())
        return None

    def crate(self, folder):
        """The name of the crate in `folder`, from the commit's manifest."""
        manifest = self.file(os.path.join(folder, "Cargo.toml"))
        if manifest is None:
            raise NotARelease(f"{self.tag}'s commit has no {os.path.join(folder, 'Cargo.toml')}: a release from before the four crates (0.9.0), which this check does not know")
        return re.search(r'(?m)^name\s*=\s*"([^"]+)"', manifest.decode()).group(1)


def said_beyond(data, version):
    """What the build versions in `data` say beyond `version`, each once:
    nothing for a clean build of the version commit. None where `data`
    holds no build version of this release at all."""
    pattern = re.compile(re.escape(version.encode()) + rb"((?:\.[0-9a-f]{40})?(?:\.dirty(?:\.[0-9a-f]{40})?)?)")
    found = [m.group(1).decode() for m in pattern.finditer(data)]
    return sorted({more for more in found if more}) if found else None


def program_problems(data, release):
    """Why the program or library `data` is no clean build of the release."""
    said = said_beyond(data, release.version)
    if said is None:
        return [f"holds no build version of {release.version}: not a build of this release"]
    problems = []
    for more in said:
        commit, dirty, tree = more.partition(".dirty")
        commit, tree = commit.lstrip("."), tree.lstrip(".")
        where = "the version commit" if commit in ("", release.commit) else f"another commit ({commit[:12]})"
        what = f"built at {where}" + (" from a dirty tree, files changed or added" + (f" (its tree: {tree})" if tree else "") if dirty else "")
        problems.append(f"its build version is {release.version}{more}: {what}")
    return problems


def archive_problems(data, release):
    """Why the libraries in the .tar.gz `data` are no clean builds."""
    problems, libraries = [], 0
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
        for member in archive.getmembers():
            if member.isfile() and LIBRARY.search(member.name):
                libraries += 1
                problems += [f"{member.name}: {problem}" for problem in program_problems(archive.extractfile(member).read(), release)]
    return problems if libraries else ["holds no library (*.so, *.a, *.dylib)"]


def crate_problems(data, release):
    """Why the .crate `data` is not the version commit's files."""
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
        members = archive.getmembers()
        odd = [m.name for m in members if not (m.isfile() or m.isdir())]
        names = [m.name for m in members if m.isfile()]
        files = {m.name.split("/", 1)[1]: archive.extractfile(m).read() for m in members if m.isfile() and "/" in m.name}
    if odd:
        return [f"holds what is no plain file ({odd[0]})"]
    # Every file once, and in the one folder Cargo packs a crate into: a
    # second entry of a name, or one outside the folder, is a file some
    # reader of the archive sees and this check would not.
    twice = sorted(n for n in set(names) if names.count(n) > 1)
    if twice:
        return [f"holds {twice[0]} more than once"]
    folders = sorted({n.split("/", 1)[0] if "/" in n else "" for n in names} | {m.name.split("/", 1)[0] for m in members if m.isdir()})
    if len(folders) != 1 or not folders[0]:
        return [f"holds files outside one folder ({', '.join(f or 'the top' for f in folders)})"]
    if any(os.path.normpath(n) != n or n.startswith("/") for n in names):
        return ["holds a file whose path is not plain"]
    for cargos in (".cargo_vcs_info.json", "Cargo.toml.orig"):
        if cargos not in files:
            return [f"has no {cargos}: not as Cargo packages a crate from a Git checkout"]
    record = json.loads(files[".cargo_vcs_info.json"])
    problems = []
    if record.get("git", {}).get("sha1") != release.commit:
        problems.append(f"packaged at commit {record.get('git', {}).get('sha1')}, not the version commit")
    if record.get("git", {}).get("dirty"):
        problems.append("Cargo packaged it from a dirty tree")
    folder = record.get("path_in_vcs", "")
    # A readme outside the crate's folder (`readme = "../README.md"`): Cargo
    # copies it to the package's top under its own name.
    readme = re.search(r'(?m)^readme\s*=\s*"([^"]+)"', files["Cargo.toml.orig"].decode(errors="replace"))
    outside = os.path.normpath(os.path.join(folder, readme.group(1))) if readme and readme.group(1).startswith("..") else None
    for path, got in sorted(files.items()):
        if path in CARGOS:
            continue
        in_commit = os.path.join(folder, "Cargo.toml" if path == "Cargo.toml.orig" else path)
        want = release.file(in_commit)
        if want is None and outside and path == os.path.basename(outside):
            in_commit, want = outside, release.file(outside)
        if want is None:
            problems.append(f"{path} is no file of the version commit")
        elif want != got:
            problems.append(f"{path} differs from the version commit's {in_commit}")
    return problems


def problems(name, data, release):
    """Why the artifact `name`, with bytes `data`, is no clean build of the
    release's version commit: nothing for one that is. A kind this check
    does not know is a problem, never skipped."""
    if name.endswith(".crate"):
        return crate_problems(data, release)
    if name.endswith((".tar.gz", ".tgz")):
        return archive_problems(data, release)
    # b3sum as tools/b3sum-release.sh names it, for some machine: a file of
    # another name beside it (b3sum-...-x86_64-unknown-linux-gnu.b3) is
    # not taken for the program.
    if re.fullmatch(re.escape(f"b3sum-{release.tag}-") + r"[A-Za-z0-9_-]+", name) or LIBRARY.search(name):
        return program_problems(data, release)
    return ["an artifact of a kind this check does not know (a program, a library, a .tar.gz of libraries, a .crate)"]


def expected(release):
    """The file names a release has, as tools/b3sum-release.sh and Cargo
    name them: b3sum for each machine, and the four crates."""
    programs = [f"b3sum-{release.tag}-{target}" for target in B3SUM_TARGETS]
    return programs + [f"{release.crate(folder)}-{release.version}.crate" for folder in CRATE_DIRS]


def read(path):
    with open(path, "rb") as f:
        return f.read()


def is_digest(name):
    """A digest file beside an artifact (tools/b3sum-release.sh writes one)."""
    return name.endswith(".sha256")


def digest_problems(name, data, seen):
    """Why the digest file `name` is not the SHA-256 line of the artifact
    it is named for, among those checked before it (`seen`, by name)."""
    of = name[: -len(".sha256")]
    if of not in seen:
        return [f"is the digest of {of}, which is no artifact here"]
    want = f"{hashlib.sha256(seen[of]).hexdigest()}  {of}\n".encode()
    return [] if data == want else [f"is not the SHA-256 line of {of}"]


def fetch(url):
    headers = {"User-Agent": f"{REPO} tools/release_artifacts.py"}
    if "api.github.com" in url and os.environ.get("GITHUB_TOKEN"):
        headers["Authorization"] = "Bearer " + os.environ["GITHUB_TOKEN"]
    with urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=300) as response:
        return response.read()


def published(release):
    """What users download for the release, each as (name, bytes or None
    where it is not there): the four crates from crates.io, and every
    asset of the GitHub release but the digest files."""
    for folder in CRATE_DIRS:
        crate = release.crate(folder)
        name = f"{crate}-{release.version}.crate"
        try:
            yield f"crates.io: {name}", name, fetch(f"https://static.crates.io/crates/{crate}/{urllib.parse.quote(name)}")
        except urllib.error.HTTPError as e:
            if e.code not in (403, 404):
                raise
            e.close()
            yield f"crates.io: {name}", name, None
    try:
        listing = json.loads(fetch(f"https://api.github.com/repos/{REPO}/releases/tags/{urllib.parse.quote(release.tag)}"))
    except urllib.error.HTTPError as e:
        if e.code != 404:
            raise
        e.close()
        yield f"the GitHub release {release.tag}", release.tag, None
        return
    names = [asset["name"] for asset in listing["assets"]]
    # Each artifact before its digest file, which is checked against it.
    for asset in sorted(listing["assets"], key=lambda asset: (is_digest(asset["name"]), asset["name"])):
        yield f"the GitHub release: {asset['name']}", asset["name"], fetch(asset["browser_download_url"])
    for missing in [n for n in expected(release) if not n.endswith(".crate") and n not in names]:
        yield f"the GitHub release: {missing}", missing, None


def rebuilt_problems(name, data, rebuilt, named=None):
    """Why the b3sum program or crate `name` is not the second one in the
    folder `rebuilt`: nothing where the two are the same bytes. A folder
    inside `rebuilt` is one builder's files: `name` is then compared with
    every builder's copy, and a builder that made any crate made this
    one. With `named`, the builders are those folders of `rebuilt` and no
    others: each is there, and each holds every crate (which builders
    there should be is then the caller's word, not the folder's: a
    builder that left no crate at all, or one folder holding everyone's
    files, was not told from two builders that agree). The names are
    folders' own, each once, and the folders real ones; `rebuilt` holds
    no other folder and no copy of `name` beside them; and each builder
    holds a b3sum program (a third folder that disagreed, one builder
    named twice, and one builder holding both programs each passed)."""
    if named is None:
        builders = [rebuilt] + sorted(os.path.join(rebuilt, sub) for sub in os.listdir(rebuilt) if os.path.isdir(os.path.join(rebuilt, sub)))
    else:
        if len(set(named)) != len(named) or any(sub in ("", ".", "..") or os.path.basename(sub) != sub for sub in named):
            return [f"the builders named are not folders' names, each once: {', '.join(named)}"]
        builders = [os.path.join(rebuilt, sub) for sub in named]
        absent = [builder for builder in builders if not os.path.isdir(builder) or os.path.islink(builder)]
        if absent:
            return [f"no files of the builder {os.path.basename(builder)} in {rebuilt}: each builder named has a folder of its own there" for builder in absent]
        others = sorted(sub for sub in os.listdir(rebuilt) if sub not in named and os.path.isdir(os.path.join(rebuilt, sub)))
        if others:
            return [f"{rebuilt} holds a folder that is no builder named ({', '.join(others)}): its files would go uncompared"]
        if os.path.lexists(os.path.join(rebuilt, name)):
            return [f"a copy of it lies in {rebuilt} outside the builders' folders: with builders named, every copy is one builder's"]
    copies = [builder for builder in builders if os.path.isfile(os.path.join(builder, name))]
    if not copies:
        script = "crates-release" if name.endswith(".crate") else "b3sum-release"
        return [f"no second build of it in {rebuilt} to compare with (sh tools/{script}.sh at the version commit, on another machine)"]
    found = []
    for builder in copies:
        other = read(os.path.join(builder, name))
        if other != data:
            digests = [hashlib.sha256(d).hexdigest() for d in (data, other)]
            found.append(f"its bytes (SHA-256 {digests[0]}) are not the second build's in {builder} (SHA-256 {digests[1]}): one of the two was not made from the version commit by the release's script")
    if name.endswith(".crate"):
        for builder in builders:
            if builder in copies:
                continue
            if named is not None:
                found.append(f"{builder} does not hold this crate: every builder named packages every crate of the release")
            elif any(n.endswith(".crate") for n in os.listdir(builder)):
                found.append(f"{builder} holds crates and not this one: its builder packaged fewer than the release has")
    elif named is not None:
        # The release's programs are named alike but for their machine.
        release = next((name[: -len(target)] for target in B3SUM_TARGETS if name.endswith("-" + target)), None)
        for builder in builders:
            if release is not None and not any(os.path.isfile(os.path.join(builder, release + target)) for target in B3SUM_TARGETS):
                found.append(f"{builder} holds no b3sum program of the release: every builder named builds one")
    return found


def check(items, release, out=None, rebuilt=None, builders=None):
    """Print a line for each artifact (more for one with several
    problems); how many are not clean or not there. With `rebuilt`, a
    folder, each b3sum program and crate is also the second one there;
    `builders`: rebuilt_problems' `named`."""
    bad = 0
    seen = {}
    for label, name, data in items:
        if data is None:
            lines = [f"MISSING    {label}"]
        elif is_digest(name):
            lines = [f"NOT CLEAN  {label}: {problem}" for problem in digest_problems(name, data, seen)]
        else:
            seen[name] = data
            found = problems(name, data, release)
            if rebuilt is not None and (name.startswith("b3sum-") or name.endswith(".crate")):
                found = found + rebuilt_problems(name, data, rebuilt, builders)
            lines = [f"NOT CLEAN  {label}: {problem}" for problem in found]
        bad += bool(lines)
        print("\n".join(lines or [f"clean      {label}"]), file=out or sys.stdout)
    return bad


def directory(path, release, rebuilt=None, builders=None):
    """A release's artifacts gathered in the folder `path` before anything
    is published (the files there that carry its version): (True, [])
    where every one a release has is there and each is clean; (False, the
    lines of those that are not clean); (None, the names not there yet).
    `rebuilt`, `builders`: as check's."""
    names = sorted((n for n in os.listdir(path) if release.version in n), key=lambda n: (is_digest(n), n)) if os.path.isdir(path) else []
    report = io.StringIO()
    if check(((n, n, read(os.path.join(path, n))) for n in names), release, out=report, rebuilt=rebuilt, builders=builders):
        return False, [line for line in report.getvalue().splitlines() if not line.startswith("clean")]
    missing = [n for n in expected(release) if n not in names]
    return (None, missing) if missing else (True, [])


def main(argv):
    argv, options = list(argv), {}
    for option in ("--rebuilt", "--commit", "--builders"):
        if option in argv[:-1]:
            at = argv.index(option)
            options[option] = argv[at + 1]
            del argv[at : at + 2]
    paths = sorted(argv[1:], key=is_digest)
    builders = options["--builders"].split(",") if "--builders" in options else None
    if not paths or argv[0].startswith("-") or any(path.startswith("--") for path in paths if path != "--published") or (builders is not None and "--rebuilt" not in options):
        print(__doc__.split("\n\n")[1], file=sys.stderr)
        sys.exit(2)
    try:
        release = Release(argv[0], commit=options.get("--commit"))
        bad = 0
        if paths == ["--published"]:
            items = published(release)
            # Published from main: the branch only a release moves.
            on_main = release.on_main()
            if on_main is not True:
                bad = 1
                print(f"NOT CLEAN  the tag: its commit {release.commit} is not in origin/main's history" if on_main is False
                      else "MISSING    origin/main in this clone, whose history a release's commit is in (git fetch origin main)")
        else:
            items = ((path, os.path.basename(path), read(path)) for path in paths)
        bad += check(items, release, rebuilt=options.get("--rebuilt"), builders=builders)
    except NotARelease as e:
        sys.exit(f"release_artifacts: {e}")
    print(f"release_artifacts: {release.tag} is commit {release.commit}: " + (f"{bad} artifacts are not clean builds of it, or are missing" if bad else "every artifact is a clean build of it"))
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main(sys.argv[1:])
