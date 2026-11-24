"""Controls for tools/release_artifacts.py: every kind of artifact, clean
and not, against a Git history made here; what users download, with the
network's answers as fixtures."""
import contextlib
import hashlib
import gzip
import io
import json
import os
import shutil
import subprocess
import tarfile
import tempfile
import unittest
import urllib.error
from unittest.mock import patch

import release_artifacts as ra

# 0.9.0's two released b3sum programs said this (BLAKE3 #435).
V090 = "0.9.0+73ee859efce085d28f1c2b22c35a5703403476f7"
DIRTY_090 = [
    V090 + ".bf2da8b957f5e0c80babb57952de3e53f95c921a.dirty.347ca7b26fa57c6937e310dd509edd994500c83e",
    V090 + ".bf2da8b957f5e0c80babb57952de3e53f95c921a.dirty.f42581ee9d779607f10121fa4cf0f6e0f9a95143",
]


def not_found(url):
    """crates.io's and GitHub's answer for what is not there."""
    return urllib.error.HTTPError(url, 404, "Not Found", {}, io.BytesIO(b""))


def tar_gz(files):
    """A .tar.gz of `files` (name: bytes), in memory."""
    out = io.BytesIO()
    # The gzip header's time is fixed: with the clock's, two archives of
    # the same files differed when a second turned between them, and the
    # test of two equal builds failed once in CI (November 17).
    with gzip.GzipFile(fileobj=out, mode="wb", mtime=0) as packed:
        with tarfile.open(fileobj=packed, mode="w") as archive:
            for name, data in files.items():
                member = tarfile.TarInfo(name)
                member.size = len(data)
                archive.addfile(member, io.BytesIO(data))
    return out.getvalue()


class Repository(unittest.TestCase):
    """A repository with one release: a crate at its top, and one in sub/
    whose licence is a link and whose readme is the top's, as threads/ has
    them here."""

    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.repo = scratch.name
        self.git("init", "-q", "--initial-branch=main")
        for key, value in [("user.name", "A Test"), ("user.email", "test@example.invalid"), ("commit.gpgsign", "false"), ("core.hooksPath", os.path.join(self.repo, "no-hooks"))]:
            self.git("config", key, value)
        self.write("Cargo.toml", '[package]\nname = "top-crate"\nversion = "0.3.0+dev"\n')
        self.write("src/lib.rs", "pub fn top() {}\n")
        self.write("LICENSE", "a licence\n")
        self.write("README.md", "the readme\n")
        self.write("sub/Cargo.toml", '[package]\nname = "sub-crate"\nversion = "0.3.0+dev"\nreadme = "../README.md"\n')
        self.write("sub/src/lib.rs", "pub fn sub() {}\n")
        os.symlink("../LICENSE", os.path.join(self.repo, "sub/LICENSE"))
        self.git("add", ".")
        self.git("commit", "-qm", "the code")
        self.code = self.git("rev-parse", "HEAD")
        self.version = "0.3.0+" + self.code
        for manifest in ("Cargo.toml", "sub/Cargo.toml"):
            self.write(manifest, self.read(manifest).replace("0.3.0+dev", self.version))
        self.git("commit", "-qam", "Version " + self.version)
        self.commit = self.git("rev-parse", "HEAD")
        self.tag = "v" + self.version
        self.git("tag", self.tag)
        self.release = ra.Release(self.tag, self.repo)

    def git(self, *args):
        return subprocess.run(["git", "-C", self.repo, *args], check=True, capture_output=True, text=True).stdout.strip()

    def write(self, path, text):
        full = os.path.join(self.repo, path)
        os.makedirs(os.path.dirname(full), exist_ok=True)
        with open(full, "w") as f:
            f.write(text)

    def read(self, path):
        with open(os.path.join(self.repo, path)) as f:
            return f.read()

    def crate(self, folder, change=lambda files, record: None):
        """The .crate Cargo makes of the crate in `folder` at the version
        commit: its record, its rewriting of the manifest beside the
        original, a lock, and the files, links followed and the outside
        readme copied in. `change` edits the files and the record first."""
        name = "sub-crate" if folder else "top-crate"
        files = {"Cargo.toml": b"# Cargo's rewriting\n", "Cargo.lock": b"# Cargo's lock\n", "Cargo.toml.orig": self.read(os.path.join(folder, "Cargo.toml")).encode()}
        files["src/lib.rs"] = self.read(os.path.join(folder, "src/lib.rs")).encode()
        files["LICENSE"] = b"a licence\n"
        files["README.md"] = b"the readme\n"
        record = {"git": {"sha1": self.commit}, "path_in_vcs": folder}
        change(files, record)
        if record is not None:
            files[".cargo_vcs_info.json"] = json.dumps(record).encode()
        prefix = f"{name}-{self.version}/"
        return tar_gz({prefix + path: data for path, data in files.items()})

    def program(self, said=""):
        """A program's bytes, its build version `said` past the release's."""
        return b"\x7fELF\0\0" + b"b3sums " + self.version.encode() + said.encode() + b"\0more bytes"


class Crates(Repository):
    def test_the_commits_files_are_clean_links_and_an_outside_readme_too(self):
        for folder in ("", "sub"):
            self.assertEqual(ra.crate_problems(self.crate(folder), self.release), [], folder)

    def test_a_changed_file_is_named(self):
        def change(files, record):
            files["src/lib.rs"] += b"// one more line\n"
        self.assertEqual(ra.crate_problems(self.crate("sub", change), self.release), ["src/lib.rs differs from the version commit's sub/src/lib.rs"])

    def test_an_added_file_is_named(self):
        def change(files, record):
            files["src/extra.rs"] = b"pub fn extra() {}\n"
        self.assertEqual(ra.crate_problems(self.crate("", change), self.release), ["src/extra.rs is no file of the version commit"])

    def test_a_changed_manifest_is_named_by_its_original(self):
        def change(files, record):
            files["Cargo.toml.orig"] += b'[dependencies]\nother = "1"\n'
        self.assertEqual(ra.crate_problems(self.crate("", change), self.release), ["Cargo.toml.orig differs from the version commit's Cargo.toml"])

    def test_a_changed_licence_behind_the_link_and_a_changed_readme_are_named(self):
        def change(files, record):
            files["LICENSE"] = b"another licence\n"
            files["README.md"] = b"another readme\n"
        self.assertEqual(
            ra.crate_problems(self.crate("sub", change), self.release),
            ["LICENSE differs from the version commit's sub/LICENSE", "README.md differs from the version commit's README.md"],
        )

    def test_cargos_dirty_mark_and_another_commit_are_named(self):
        def dirty(files, record):
            record["git"]["dirty"] = True
        self.assertEqual(ra.crate_problems(self.crate("", dirty), self.release), ["Cargo packaged it from a dirty tree"])

        def elsewhere(files, record):
            record["git"]["sha1"] = self.code
        self.assertEqual(ra.crate_problems(self.crate("", elsewhere), self.release), [f"packaged at commit {self.code}, not the version commit"])

    def test_a_path_out_of_the_repository_is_no_file_of_the_commit(self):
        def change(files, record):
            files["../outside.rs"] = b""
        self.assertEqual(ra.crate_problems(self.crate("", change), self.release), ["holds a file whose path is not plain"])
        self.assertIsNone(self.release.file("/etc/hostname"))

    def test_a_file_twice_or_outside_the_crates_folder_is_not_clean(self):
        # BLAKE3 #439: a second entry of a name replaced the first in this
        # check, and an entry outside the folder was not looked at.
        clean = self.crate("")
        with tarfile.open(fileobj=io.BytesIO(clean), mode="r:gz") as archive:
            files = {m.name: archive.extractfile(m).read() for m in archive.getmembers()}
        source = f"top-crate-{self.version}/src/lib.rs"

        def packed(entries):
            out = io.BytesIO()
            with tarfile.open(fileobj=out, mode="w:gz") as archive:
                for name, data in entries:
                    member = tarfile.TarInfo(name)
                    member.size = len(data)
                    archive.addfile(member, io.BytesIO(data))
            return out.getvalue()

        twice = packed([(source, b"pub fn back_door() {}\n")] + list(files.items()))
        self.assertEqual(ra.crate_problems(twice, self.release), [f"holds {source} more than once"])
        self.assertEqual(ra.crate_problems(packed(list(files.items()) + [("build.rs", b"")]), self.release), [f"holds files outside one folder (the top, top-crate-{self.version})"])
        self.assertEqual(ra.crate_problems(packed(list(files.items()) + [("other/build.rs", b"")]), self.release), [f"holds files outside one folder (other, top-crate-{self.version})"])

    def test_a_crate_without_cargos_record_or_original_is_not_clean(self):
        def no_original(files, record):
            del files["Cargo.toml.orig"]
        self.assertEqual(ra.crate_problems(self.crate("", no_original), self.release), ["has no Cargo.toml.orig: not as Cargo packages a crate from a Git checkout"])
        archive = tar_gz({f"top-crate-{self.version}/src/lib.rs": b"pub fn top() {}\n"})
        self.assertEqual(ra.crate_problems(archive, self.release), ["has no .cargo_vcs_info.json: not as Cargo packages a crate from a Git checkout"])


class Programs(Repository):
    def test_the_versions_0_9_0s_programs_carried_are_not_clean(self):
        for said in DIRTY_090:
            more = ra.said_beyond(b"\0b3sums " + said.encode() + b"\0", V090)
            self.assertEqual(more, [said[len(V090):]])
        self.assertEqual(ra.said_beyond(b"b3sums " + V090.encode() + b"\0", V090), [], "the release's version alone")
        self.assertIsNone(ra.said_beyond(b"b3sums 0.9.0+dev.0123", V090), "no build version of this release")

    def test_a_clean_program(self):
        self.assertEqual(ra.program_problems(self.program(), self.release), [])

    def test_a_dirty_tree_is_named_with_its_tree(self):
        tree = "347ca7b26fa57c6937e310dd509edd994500c83e"
        said = f".{self.commit}.dirty.{tree}"
        self.assertEqual(
            ra.program_problems(self.program(said), self.release),
            [f"its build version is {self.version}{said}: built at the version commit from a dirty tree, files changed or added (its tree: {tree})"],
        )

    def test_a_dirty_package_and_another_commit_are_named(self):
        self.assertEqual(
            ra.program_problems(self.program(f".{self.commit}.dirty"), self.release),
            [f"its build version is {self.version}.{self.commit}.dirty: built at the version commit from a dirty tree, files changed or added"],
        )
        self.assertEqual(
            ra.program_problems(self.program("." + self.code), self.release),
            [f"its build version is {self.version}.{self.code}: built at another commit ({self.code[:12]})"],
        )

    def test_one_dirty_version_among_clean_ones_is_not_clean(self):
        # A program holds its packages' plain versions beside its build version.
        data = self.program() + self.program(f".{self.commit}.dirty") + self.program()
        self.assertEqual(len(ra.program_problems(data, self.release)), 1)

    def test_a_program_of_no_such_release_is_not_clean(self):
        self.assertEqual(ra.program_problems(b"\x7fELF b3sums 0.2.0", self.release), [f"holds no build version of {self.version}: not a build of this release"])

    def test_an_archives_libraries_are_each_checked(self):
        clean = tar_gz({"usr/lib/libb3_servil.so.0.3.0": self.program(), "usr/lib/libb3_servil.a": self.program(), "usr/include/b3.h": b"/* no version */"})
        self.assertEqual(ra.archive_problems(clean, self.release), [])
        dirty = tar_gz({"usr/lib/libb3_servil.so.0.3.0": self.program(), "usr/lib/libb3_servil.a": self.program(f".{self.commit}.dirty")})
        self.assertEqual(len(ra.archive_problems(dirty, self.release)), 1)
        self.assertTrue(ra.archive_problems(dirty, self.release)[0].startswith("usr/lib/libb3_servil.a: its build version is "))
        self.assertEqual(ra.archive_problems(tar_gz({"usr/include/b3.h": b""}), self.release), ["holds no library (*.so, *.a, *.dylib)"])

    def test_each_kind_goes_to_its_check_and_an_unknown_kind_is_a_problem(self):
        self.assertEqual(ra.problems(f"b3sum-{self.tag}-x86_64-unknown-linux-gnu", self.program(), self.release), [])
        self.assertEqual(ra.problems("libb3_servil.0.3.0.dylib", self.program(), self.release), [])
        self.assertEqual(ra.problems(f"top-crate-{self.version}.crate", self.crate(""), self.release), [])
        self.assertEqual(ra.problems("lib.tar.gz", tar_gz({"libb3_servil.a": self.program()}), self.release), [])
        self.assertEqual(len(ra.problems(f"b3sum-{self.tag}-x86_64-unknown-linux-gnu", self.program(".dirty"), self.release)), 1)
        self.assertEqual(ra.problems("notes.txt", b"", self.release), ["an artifact of a kind this check does not know (a program, a library, a .tar.gz of libraries, a .crate)"])


class Releases(Repository):
    def test_a_tag_that_is_no_version_commits_is_refused(self):
        self.git("tag", "v0.3.1+" + self.code, self.code)
        for tag in ("v0.3.1+" + self.code, "v0.3.0", "0.3.0+" + self.code, "v0.4.0+" + "0" * 40):
            with self.assertRaises(ra.NotARelease, msg=tag):
                ra.Release(tag, self.repo)

    def test_a_candidate_is_named_by_its_commit_before_its_tag_is_made(self):
        self.git("tag", "-d", self.tag)
        self.assertEqual(ra.Release(self.tag, self.repo, self.commit).commit, self.commit)
        with self.assertRaises(ra.NotARelease):
            ra.Release(self.tag, self.repo, self.code)


class TheNameIsTheSource(Repository):
    """The tag's name holds the code commit's hash, and the version commit
    is that commit with the version written in: nothing else. Each test
    makes a version commit of its own on the fixture's code commit."""

    def version_commit(self, change):
        """A tag on a version commit made as the fixture's, then changed
        by `change`: whether it is a release, and why not."""
        self.git("checkout", "-q", "--detach", self.code)
        for manifest in ("Cargo.toml", "sub/Cargo.toml"):
            self.write(manifest, self.read(manifest).replace("0.3.0+dev", self.version))
        change()
        self.git("add", "-A")
        self.git("commit", "-qm", "another version commit")
        try:
            ra.Release(self.tag, self.repo, self.git("rev-parse", "HEAD"))
        except ra.NotARelease as e:
            return str(e)
        return None

    def test_the_version_written_in_and_nothing_else_is_a_release(self):
        self.assertIsNone(self.version_commit(lambda: None))

    def test_a_changed_line_of_code_is_refused(self):
        why = self.version_commit(lambda: self.write("src/lib.rs", "pub fn top() { /* and more */ }\n"))
        self.assertIn("which is not its parent with the version written in: src/lib.rs line 1 changes more than a version to this release's", why)

    def test_an_added_file_and_a_removed_one_are_refused(self):
        self.assertIn("build.rs is added, removed, or of another kind", self.version_commit(lambda: self.write("build.rs", "fn main() {}\n")))
        self.assertIn("LICENSE is added, removed, or of another kind", self.version_commit(lambda: os.remove(os.path.join(self.repo, "LICENSE"))))

    def test_an_added_line_is_refused(self):
        why = self.version_commit(lambda: self.write("Cargo.toml", self.read("Cargo.toml") + 'build = "other.rs"\n'))
        self.assertIn("Cargo.toml has lines added or removed", why)

    def test_more_than_the_version_on_a_versions_line_is_refused(self):
        why = self.version_commit(lambda: self.write("Cargo.toml", self.read("Cargo.toml").replace(f'version = "{self.version}"', f'version = "{self.version}" # more')))
        self.assertIn("Cargo.toml line 3 changes more than a version to this release's", why)

    def test_another_version_than_the_releases_is_refused(self):
        other = "0.3.0+" + "5" * 40
        why = self.version_commit(lambda: self.write("sub/Cargo.toml", self.read("sub/Cargo.toml").replace(self.version, other)))
        self.assertIn("sub/Cargo.toml line 3 changes more than a version to this release's", why)

    def test_a_version_written_twice_where_one_stood_is_refused(self):
        # BLAKE3 #439: with the stamps taken out, both lines read the same.
        twice = self.version + self.version
        why = self.version_commit(lambda: self.write("Cargo.toml", self.read("Cargo.toml").replace(f'version = "{self.version}"', f'version = "{twice}"')))
        self.assertIn("Cargo.toml line 3 changes more than a version to this release's", why)

    def test_a_file_made_executable_is_refused(self):
        why = self.version_commit(lambda: os.chmod(os.path.join(self.repo, "src/lib.rs"), 0o755))
        self.assertIn("src/lib.rs is added, removed, or of another kind", why)

    def test_an_empty_version_commit_is_refused(self):
        self.git("checkout", "-q", "--detach", self.code)
        self.git("commit", "--allow-empty", "-qm", "empty version commit")
        with self.assertRaisesRegex(ra.NotARelease, "changes no version from its parent"):
            ra.Release(self.tag, self.repo, self.git("rev-parse", "HEAD"))

    def test_main_holds_a_published_releases_commit(self):
        self.assertIsNone(self.release.on_main(), "no origin/main in this clone")
        self.git("update-ref", "refs/remotes/origin/main", self.code)
        self.assertIs(self.release.on_main(), False, "main is still at the code commit")
        self.git("update-ref", "refs/remotes/origin/main", self.commit)
        self.assertIs(self.release.on_main(), True)
        self.git("commit", "--allow-empty", "-qm", "the development line after the release")
        self.git("update-ref", "refs/remotes/origin/main", "HEAD")
        self.assertIs(self.release.on_main(), True, "and in its history later")


@patch.object(ra, "CRATE_DIRS", ("", "sub"))
@patch.object(ra, "B3SUM_TARGETS", ("x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"))
class Gathered(Repository):
    """A release's artifacts in one folder, before anything is published."""

    def fill(self, folder, programs=("x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"), crates=("", "sub"), said="", digests=True):
        for target in programs:
            name = f"b3sum-{self.tag}-{target}"
            raw = self.program(said)
            with open(os.path.join(folder, name), "wb") as f:
                f.write(raw)
            if digests:
                with open(os.path.join(folder, name + ".sha256"), "w") as f:
                    f.write(f"{hashlib.sha256(raw).hexdigest()}  {name}\n")
                with open(os.path.join(folder, name + ".b3"), "w") as f:
                    f.write(f"{ra.blake3_ref.blake3_hex(raw)}  {name}\n")
        for crate in crates:
            with open(os.path.join(folder, f"{'sub-crate' if crate else 'top-crate'}-{self.version}.crate"), "wb") as f:
                f.write(self.crate(crate))

    def test_every_artifact_there_and_clean_passes_digest_files_and_other_releases_aside(self):
        with tempfile.TemporaryDirectory() as folder:
            self.fill(folder)
            program = f"b3sum-{self.tag}-x86_64-unknown-linux-gnu"
            open(os.path.join(folder, "b3sum-v0.2.0+old-x86_64-unknown-linux-gnu"), "wb").close()
            with open(os.path.join(folder, program), "rb") as f:
                raw = f.read()
            sha_line = f"{hashlib.sha256(raw).hexdigest()}  {program}\n"
            b3_line = f"{ra.blake3_ref.blake3_hex(raw)}  {program}\n"
            self.assertEqual(ra.directory(folder, self.release), (True, []))
            with open(os.path.join(folder, program + ".sha256"), "w") as f:
                f.write(sha_line.replace(sha_line[0], "0" if sha_line[0] != "0" else "1", 1))
            self.assertEqual(ra.directory(folder, self.release), (False, [f"NOT CLEAN  {program}.sha256: is not the SHA-256 line of {program}"]))
            with open(os.path.join(folder, program + ".sha256"), "w") as f:
                f.write(sha_line)
            with open(os.path.join(folder, program + ".b3"), "w") as f:
                f.write(b3_line.replace(b3_line[0], "0" if b3_line[0] != "0" else "1", 1))
            self.assertEqual(ra.directory(folder, self.release), (False, [f"NOT CLEAN  {program}.b3: is not the BLAKE3 line of {program}"]))
            # Missing either sidecar in dist/ keeps the release open (#465).
            os.remove(os.path.join(folder, program + ".b3"))
            self.assertEqual(ra.directory(folder, self.release), (None, [program + ".b3"]))
            os.remove(os.path.join(folder, program + ".sha256"))
            self.assertEqual(ra.directory(folder, self.release), (None, [program + ".sha256", program + ".b3"]))
            # A symbolic link inside dist/ is refused.
            os.symlink(program, os.path.join(folder, program + ".sha256"))
            ok, lines = ra.directory(folder, self.release)
            self.assertIs(ok, False)
            self.assertIn(f"NOT CLEAN  {program}.sha256: is no plain file", lines)

    def test_an_artifact_not_there_yet_is_named(self):
        with tempfile.TemporaryDirectory() as folder:
            self.assertEqual(ra.directory(os.path.join(folder, "none"), self.release)[0], None)
            self.fill(folder, programs=("x86_64-unknown-linux-gnu",), crates=("",))
            arm = f"b3sum-{self.tag}-aarch64-unknown-linux-gnu"
            self.assertEqual(
                ra.directory(folder, self.release),
                (None, [arm, f"sub-crate-{self.version}.crate", arm + ".sha256", arm + ".b3"]),
            )

    def test_one_that_is_not_clean_fails_whatever_else_is_missing(self):
        with tempfile.TemporaryDirectory() as folder:
            self.fill(folder, programs=("x86_64-unknown-linux-gnu",), crates=(), said=f".{self.commit}.dirty")
            ok, lines = ra.directory(folder, self.release)
            self.assertIs(ok, False)
            self.assertEqual(len(lines), 1)
            self.assertTrue(lines[0].startswith(f"NOT CLEAN  b3sum-{self.tag}-x86_64-unknown-linux-gnu: its build version is "))


@patch.object(ra, "CRATE_DIRS", ("", "sub"))
@patch.object(ra, "B3SUM_TARGETS", ("x86_64-unknown-linux-gnu",))
class Published(Repository):
    """What users download, the network's answers given here."""

    def answers(self, assets, crates=("top-crate", "sub-crate")):
        quoted = self.version.replace("+", "%2B")
        served = {f"https://static.crates.io/crates/{name}/{name}-{quoted}.crate": self.crate("sub" if name == "sub-crate" else "") for name in crates}
        listing = {"assets": [{"name": name, "browser_download_url": "https://github.invalid/" + name} for name in assets]}
        served[f"https://api.github.com/repos/{ra.REPO}/releases/tags/v{quoted}"] = json.dumps(listing).encode()
        served.update({"https://github.invalid/" + name: data for name, data in assets.items() if data is not None})

        def fetch(url):
            if url not in served:
                raise not_found(url)
            return served[url]

        return fetch

    def digest(self, program, of=None):
        """The SHA-256 digest file's line for `program`, whose bytes are `of`."""
        return f"{hashlib.sha256(self.program() if of is None else of).hexdigest()}  {program}\n".encode()

    def b3_digest(self, program, of=None):
        """The BLAKE3 digest file's line for `program`, whose bytes are `of`."""
        return f"{ra.blake3_ref.blake3_hex(self.program() if of is None else of)}  {program}\n".encode()

    def run_check(self, fetch):
        out = io.StringIO()
        with patch.object(ra, "fetch", fetch):
            bad = ra.check(ra.published(self.release), self.release, out=out)
        return bad, out.getvalue().splitlines()

    def test_a_clean_release_passes_with_its_digest_files(self):
        program = f"b3sum-{self.tag}-x86_64-unknown-linux-gnu"
        bad, lines = self.run_check(
            self.answers({
                program + ".sha256": self.digest(program),
                program + ".b3": self.b3_digest(program),
                program: self.program(),
                "lib.tar.gz": tar_gz({"libb3_servil.a": self.program()}),
            })
        )
        self.assertEqual(bad, 0, lines)
        self.assertEqual(len(lines), 6)
        self.assertTrue(all(line.startswith("clean      ") for line in lines), lines)

    def test_a_digest_file_of_other_bytes_or_of_nothing_here_or_of_another_kind_fails(self):
        # BLAKE3 #439, #446: both .sha256 and .b3 digest files are held to their artifacts.
        program = f"b3sum-{self.tag}-x86_64-unknown-linux-gnu"
        bad_sha = f"{hashlib.sha256(b'another program').hexdigest()}  {program}\n".encode()
        bad_b3 = f"{ra.blake3_ref.blake3_hex(b'another program')}  {program}\n".encode()
        bad, lines = self.run_check(
            self.answers({
                program: self.program(),
                program + ".sha256": bad_sha,
                program + ".b3": bad_b3,
                "gone.sha256": bad_sha,
                "gone.b3": bad_b3,
                program + ".md5": bad_sha,
            })
        )
        self.assertEqual(bad, 5, lines)
        self.assertIn(f"NOT CLEAN  the GitHub release: {program}.sha256: is not the SHA-256 line of {program}", lines)
        self.assertIn(f"NOT CLEAN  the GitHub release: {program}.b3: is not the BLAKE3 line of {program}", lines)
        self.assertIn("NOT CLEAN  the GitHub release: gone.sha256: is the digest of gone, which is no artifact here", lines)
        self.assertIn("NOT CLEAN  the GitHub release: gone.b3: is the digest of gone, which is no artifact here", lines)
        self.assertTrue(any(l.startswith(f"NOT CLEAN  the GitHub release: {program}.md5: ") for l in lines), lines)

    def test_a_dirty_program_fails_and_the_rest_still_pass(self):
        program = f"b3sum-{self.tag}-x86_64-unknown-linux-gnu"
        dirty = self.program(f".{self.commit}.dirty.{'1' * 40}")
        bad, lines = self.run_check(
            self.answers({program: dirty, program + ".sha256": self.digest(program, dirty), program + ".b3": self.b3_digest(program, dirty)})
        )
        self.assertEqual(bad, 1)
        self.assertEqual([line[:9] for line in lines], ["clean    ", "clean    ", "NOT CLEAN", "clean    ", "clean    "])
        self.assertIn(f"the GitHub release: {program}: its build version is", lines[2])

    def test_a_crate_or_a_program_not_published_is_missing(self):
        bad, lines = self.run_check(self.answers({}, crates=("top-crate",)))
        self.assertEqual(bad, 4)
        program = f"b3sum-{self.tag}-x86_64-unknown-linux-gnu"
        self.assertEqual(
            lines[1:],
            [
                f"MISSING    crates.io: sub-crate-{self.version}.crate",
                f"MISSING    the GitHub release: {program}",
                f"MISSING    the GitHub release: {program}.sha256",
                f"MISSING    the GitHub release: {program}.b3",
            ],
        )

    def test_a_program_published_without_either_digest_file_is_missing_it(self):
        """A release's programs come with both .sha256 and .b3 files (BLAKE3 #446).
        Missing either digest file fails."""
        program = f"b3sum-{self.tag}-x86_64-unknown-linux-gnu"
        bad, lines = self.run_check(self.answers({program: self.program()}))
        self.assertEqual(
            (bad, lines[-2:]),
            (2, [f"MISSING    the GitHub release: {program}.sha256", f"MISSING    the GitHub release: {program}.b3"]),
        )
        self.assertTrue(all(line.startswith("clean      ") for line in lines[:-2]), lines)
        bad, lines = self.run_check(self.answers({program: self.program(), program + ".sha256": self.digest(program)}))
        self.assertEqual((bad, lines[-1]), (1, f"MISSING    the GitHub release: {program}.b3"))
        bad, lines = self.run_check(self.answers({program: self.program(), program + ".b3": self.b3_digest(program)}))
        self.assertEqual((bad, lines[-1]), (1, f"MISSING    the GitHub release: {program}.sha256"))

    def test_no_github_release_is_missing(self):
        def fetch(url):
            if "api.github.com" in url:
                raise not_found(url)
            return self.answers({})(url)
        bad, lines = self.run_check(fetch)
        self.assertEqual((bad, lines[-1]), (1, f"MISSING    the GitHub release {self.tag}"))

    def test_an_asset_of_an_unknown_kind_fails(self):
        program = f"b3sum-{self.tag}-x86_64-unknown-linux-gnu"
        bad, lines = self.run_check(
            self.answers({
                program: self.program(),
                program + ".sha256": self.digest(program),
                program + ".b3": self.b3_digest(program),
                "installer.sh": b"#!/bin/sh\n",
            })
        )
        self.assertEqual(bad, 1, lines)
        self.assertTrue(any("installer.sh: an artifact of a kind this check does not know" in line for line in lines), lines)


@patch.object(ra, "CRATE_DIRS", ("", "sub"))
class Rebuilt(Gathered):
    """A b3sum program against a second build of the same commit."""

    def test_the_same_bytes_as_the_second_build_pass(self):
        with tempfile.TemporaryDirectory() as folder, tempfile.TemporaryDirectory() as second:
            self.fill(folder)
            self.fill(second)
            self.assertEqual(ra.directory(folder, self.release, rebuilt=second), (True, []))

    def test_a_program_that_says_it_is_clean_and_differs_from_the_second_build_fails(self):
        # The forgery the build's own word cannot show: the right version
        # in it, and other code.
        with tempfile.TemporaryDirectory() as folder, tempfile.TemporaryDirectory() as second:
            self.fill(folder)
            self.fill(second)
            name = f"b3sum-{self.tag}-aarch64-unknown-linux-gnu"
            with open(os.path.join(folder, name), "ab") as f:
                f.write(b"and one byte more")
            raw = open(os.path.join(folder, name), "rb").read()
            with open(os.path.join(folder, name + ".sha256"), "w") as f:
                f.write(f"{hashlib.sha256(raw).hexdigest()}  {name}\n")
            with open(os.path.join(folder, name + ".b3"), "w") as f:
                f.write(f"{ra.blake3_ref.blake3_hex(raw)}  {name}\n")
            self.assertEqual(ra.directory(folder, self.release), (True, []), "its own word is clean")
            ok, lines = ra.directory(folder, self.release, rebuilt=second)
            self.assertIs(ok, False)
            self.assertEqual(len(lines), 1)
            self.assertTrue(lines[0].startswith(f"NOT CLEAN  {name}: its bytes (SHA-256 "), lines[0])
            self.assertIn(f"are not the second build's in {second} (SHA-256 ", lines[0])

    def test_every_builders_copy_is_compared(self):
        # The release workflow's jobs each leave a folder of their own. Both
        # package the crates; merged into one folder, the later one's crate
        # would stand for both.
        with tempfile.TemporaryDirectory() as folder, tempfile.TemporaryDirectory() as second:
            self.fill(folder)
            for builder in ("b3sum-ARM64", "b3sum-X64"):
                os.mkdir(os.path.join(second, builder))
                self.fill(os.path.join(second, builder))
            self.assertEqual(ra.directory(folder, self.release, rebuilt=second), (True, []))
            crate = f"sub-crate-{self.version}.crate"
            with open(os.path.join(second, "b3sum-ARM64", crate), "ab") as f:
                f.write(b"and one byte more")
            ok, lines = ra.directory(folder, self.release, rebuilt=second)
            self.assertIs(ok, False)
            self.assertEqual(len(lines), 1, lines)
            self.assertIn(f"are not the second build's in {os.path.join(second, 'b3sum-ARM64')} (SHA-256 ", lines[0])
            # A builder that packaged fewer crates than the release has.
            os.remove(os.path.join(second, "b3sum-ARM64", crate))
            ok, lines = ra.directory(folder, self.release, rebuilt=second)
            self.assertIs(ok, False)
            self.assertEqual(lines, [f"NOT CLEAN  {crate}: {os.path.join(second, 'b3sum-ARM64')} holds crates and not this one: its builder packaged fewer than the release has"])
            # A builder of programs alone (no crates at all) owes none.
            for name in os.listdir(os.path.join(second, "b3sum-ARM64")):
                if name.endswith(".crate"):
                    os.remove(os.path.join(second, "b3sum-ARM64", name))
            self.assertEqual(ra.directory(folder, self.release, rebuilt=second), (True, []))

    def test_builders_that_are_named_must_each_be_there_with_every_crate(self):
        # Told who the builders are (NAME=TARGET), the check holds each
        # builder to its own target's program and every release crate.
        crate = f"sub-crate-{self.version}.crate"
        named = ["b3sum-ARM64=aarch64-unknown-linux-gnu", "b3sum-X64=x86_64-unknown-linux-gnu"]
        with tempfile.TemporaryDirectory() as folder, tempfile.TemporaryDirectory() as second:
            self.fill(folder)
            os.mkdir(os.path.join(second, "b3sum-ARM64"))
            self.fill(os.path.join(second, "b3sum-ARM64"), programs=("aarch64-unknown-linux-gnu",))
            os.mkdir(os.path.join(second, "b3sum-X64"))
            self.fill(os.path.join(second, "b3sum-X64"), programs=("x86_64-unknown-linux-gnu",))
            self.assertEqual(ra.directory(folder, self.release, rebuilt=second, builders=named), (True, []))
            # Its program, and no crate at all.
            arm = os.path.join(second, "b3sum-ARM64")
            for name in os.listdir(arm):
                if name.endswith(".crate"):
                    os.remove(os.path.join(arm, name))
            self.assertEqual(ra.directory(folder, self.release, rebuilt=second), (True, []), "unnamed, a builder of programs alone owes no crate")
            ok, lines = ra.directory(folder, self.release, rebuilt=second, builders=named)
            self.assertIs(ok, False)
            self.assertIn(f"NOT CLEAN  {crate}: {arm} does not hold this crate: every builder named packages every crate of the release", lines)
            # No folder of its own.
            shutil.rmtree(arm)
            ok, lines = ra.directory(folder, self.release, rebuilt=second, builders=named)
            self.assertIs(ok, False)
            self.assertIn(f"NOT CLEAN  {crate}: no files of the builder b3sum-ARM64 in {second}: each builder named has a folder of its own there", lines)
        # One folder with everyone's files, and no builder's folder in it.
        with tempfile.TemporaryDirectory() as folder, tempfile.TemporaryDirectory() as second:
            self.fill(folder)
            self.fill(second)
            self.assertEqual(ra.directory(folder, self.release, rebuilt=second), (True, []))
            ok, lines = ra.directory(folder, self.release, rebuilt=second, builders=named)
            self.assertIs(ok, False)
            self.assertTrue(all("no files of the builder" in line for line in lines), lines)

    def test_builders_that_are_named_are_all_there_is_to_compare_with(self):
        # Named builders (NAME=TARGET) reject extra folders/files, symlinks,
        # swapped target binaries, and malformed builder specs (#465).
        crate = f"sub-crate-{self.version}.crate"
        named = ["b3sum-ARM64=aarch64-unknown-linux-gnu", "b3sum-X64=x86_64-unknown-linux-gnu"]

        def builders(second):
            os.mkdir(os.path.join(second, "b3sum-ARM64"))
            self.fill(os.path.join(second, "b3sum-ARM64"), programs=("aarch64-unknown-linux-gnu",))
            os.mkdir(os.path.join(second, "b3sum-X64"))
            self.fill(os.path.join(second, "b3sum-X64"), programs=("x86_64-unknown-linux-gnu",))

        def refused(folder, second, names, words):
            ok, lines = ra.directory(folder, self.release, rebuilt=second, builders=names)
            self.assertIs(ok, False, words)
            self.assertTrue(lines and all(words in line for line in lines), (words, lines))

        with tempfile.TemporaryDirectory() as folder, tempfile.TemporaryDirectory() as second:
            self.fill(folder)
            builders(second)
            self.assertEqual(ra.directory(folder, self.release, rebuilt=second, builders=named), (True, []))
            # A third folder, whose crate is another.
            third = os.path.join(second, "b3sum-RISCV")
            os.mkdir(third)
            self.fill(third)
            refused(folder, second, named, "holds entries outside the builders named (b3sum-RISCV)")
            shutil.rmtree(third)
            # A copy outside every builder's folder.
            shutil.copyfile(os.path.join(second, "b3sum-X64", crate), os.path.join(second, crate))
            refused(folder, second, named, f"holds entries outside the builders named ({crate})")
            os.remove(os.path.join(second, crate))
            # An unexpected extra file inside a builder's folder.
            stray = os.path.join(second, "b3sum-X64", "extra.bin")
            open(stray, "wb").close()
            refused(folder, second, named, "holds an unexpected file (extra.bin)")
            os.remove(stray)
            # A symbolic link inside a builder's folder (refused both named and unnamed).
            arm_crate = os.path.join(second, "b3sum-ARM64", crate)
            os.remove(arm_crate)
            os.symlink(os.path.join(second, "b3sum-X64", crate), arm_crate)
            refused(folder, second, named, f"holds what is no plain file ({crate})")
            ok_un, lines_un = ra.directory(folder, self.release, rebuilt=second)
            self.assertIs(ok_un, False)
            self.assertTrue(any("holds a symbolic link" in l for l in lines_un), lines_un)
            os.remove(arm_crate)
            shutil.copyfile(os.path.join(second, "b3sum-X64", crate), arm_crate)
            # Malformed specs: duplicate name, duplicate target, missing target, no target.
            refused(folder, second, ["b3sum-X64=x86_64-unknown-linux-gnu", "b3sum-X64=aarch64-unknown-linux-gnu"], "NAME=TARGET")
            refused(folder, second, ["b3sum-X64=x86_64-unknown-linux-gnu", "b3sum-ARM64=x86_64-unknown-linux-gnu"], "each release target once")
            refused(folder, second, ["b3sum-X64=x86_64-unknown-linux-gnu"], "cover no builder for aarch64-unknown-linux-gnu")
            refused(folder, second, ["b3sum-X64", "b3sum-ARM64"], "NAME=TARGET")
            refused(folder, second, [".=x86_64-unknown-linux-gnu", "b3sum-ARM64=aarch64-unknown-linux-gnu"], "NAME=TARGET")
            # Swapped target binaries between builders fail.
            refused(folder, second, ["b3sum-ARM64=x86_64-unknown-linux-gnu", "b3sum-X64=aarch64-unknown-linux-gnu"], "holds an unexpected file")
            # A builder folder replaced by a symlink.
            shutil.rmtree(os.path.join(second, "b3sum-ARM64"))
            os.symlink("b3sum-X64", os.path.join(second, "b3sum-ARM64"))
            refused(folder, second, named, "no files of the builder b3sum-ARM64")
        # Both programs in one builder's folder, and none in the other's.
        with tempfile.TemporaryDirectory() as folder, tempfile.TemporaryDirectory() as second:
            self.fill(folder)
            builders(second)
            arm = os.path.join(second, "b3sum-ARM64")
            for name in os.listdir(arm):
                if name.startswith("b3sum-"):
                    os.remove(os.path.join(arm, name))
            refused(folder, second, named, f"holds no b3sum-{self.tag}-aarch64-unknown-linux-gnu")

    def test_a_program_or_a_crate_with_no_second_build_fails(self):
        with tempfile.TemporaryDirectory() as folder, tempfile.TemporaryDirectory() as second:
            self.fill(folder)
            self.fill(second, programs=("x86_64-unknown-linux-gnu",), crates=("",))
            ok, lines = ra.directory(folder, self.release, rebuilt=second)
            self.assertIs(ok, False)
            self.assertEqual(lines, [
                f"NOT CLEAN  b3sum-{self.tag}-aarch64-unknown-linux-gnu: no second build of it in {second} to compare with (sh tools/b3sum-release.sh at the version commit, on another machine)",
                f"NOT CLEAN  sub-crate-{self.version}.crate: no second build of it in {second} to compare with (sh tools/crates-release.sh at the version commit, on another machine)",
            ])

    def test_a_crate_of_the_commits_files_that_is_not_the_second_packaging_fails(self):
        # What the file-by-file check passes over: Cargo's own files in
        # the archive (its Cargo.toml, its lock).
        with tempfile.TemporaryDirectory() as folder, tempfile.TemporaryDirectory() as second:
            self.fill(folder)
            self.fill(second)
            def cargos(files, record):
                files["Cargo.toml"] = b'[dependencies]\nsomething-else = "1"\n'
            name = f"top-crate-{self.version}.crate"
            with open(os.path.join(folder, name), "wb") as f:
                f.write(self.crate("", cargos))
            self.assertEqual(ra.directory(folder, self.release), (True, []), "file by file, it is the commit's")
            ok, lines = ra.directory(folder, self.release, rebuilt=second)
            self.assertIs(ok, False)
            self.assertEqual(len(lines), 1)
            self.assertTrue(lines[0].startswith(f"NOT CLEAN  {name}: its bytes (SHA-256 "), lines[0])


class CommandLine(Repository):
    def run_main(self, *argv):
        out = io.StringIO()
        with patch.object(ra, "ROOT", self.repo), contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
            with self.assertRaises(SystemExit) as stopped:
                ra.main(list(argv))
        return stopped.exception.code, out.getvalue()

    def test_files_clean_exit_0_not_clean_exit_1_and_a_digest_file_is_checked(self):
        with tempfile.TemporaryDirectory() as folder:
            clean, dirty = os.path.join(folder, f"b3sum-{self.tag}-a"), os.path.join(folder, f"b3sum-{self.tag}-b")
            for path, said in ((clean, ""), (dirty, f".{self.commit}.dirty")):
                with open(path, "wb") as f:
                    f.write(self.program(said))
            with open(clean + ".sha256", "w") as f:
                f.write(f"{hashlib.sha256(self.program()).hexdigest()}  {os.path.basename(clean)}\n")
            with open(clean + ".b3", "w") as f:
                f.write(f"{ra.blake3_ref.blake3_hex(self.program())}  {os.path.basename(clean)}\n")
            self.assertEqual(self.run_main(self.tag, clean + ".sha256", clean + ".b3", clean)[0], 0)
            self.assertEqual(self.run_main(self.tag, clean + ".sha256")[0], 1, "a .sha256 file with no artifact")
            self.assertEqual(self.run_main(self.tag, clean + ".b3")[0], 1, "a .b3 file with no artifact")
            code, said = self.run_main(self.tag, clean, dirty)
            self.assertEqual(code, 1)
            self.assertIn("1 artifacts are not clean builds of it, or are missing", said)

    def test_rebuilt_and_commit_are_options_wherever_they_stand(self):
        with tempfile.TemporaryDirectory() as folder, tempfile.TemporaryDirectory() as second:
            name = f"b3sum-{self.tag}-a"
            for where, more in ((folder, b""), (second, b"!")):
                with open(os.path.join(where, name), "wb") as f:
                    f.write(self.program() + more)
            path = os.path.join(folder, name)
            self.assertEqual(self.run_main(self.tag, path, "--commit", self.commit)[0], 0)
            self.assertEqual(self.run_main(self.tag, "--rebuilt", folder, path)[0], 0)
            self.assertEqual(self.run_main(self.tag, path, "--rebuilt", second)[0], 1)
            self.assertEqual(self.run_main(self.tag, path, "--commit", self.code)[0], f"release_artifacts: {self.tag} names commit {self.code}, whose parent is not {self.code}: not a version commit")
            self.assertEqual(self.run_main(self.tag, path, "--rebuilt")[0], 2, "an option without its value")
            self.assertEqual(self.run_main(self.tag, path, "--rebuild", second)[0], 2, "an option this does not know")

    def test_published_needs_the_commit_in_mains_history(self):
        def nothing_published(release):
            return iter(())

        with patch.object(ra, "published", nothing_published):
            code, said = self.run_main(self.tag, "--published")
            self.assertEqual(code, 1)
            self.assertIn("MISSING    origin/main in this clone", said)
            self.git("update-ref", "refs/remotes/origin/main", self.code)
            code, said = self.run_main(self.tag, "--published")
            self.assertEqual(code, 1)
            self.assertIn(f"NOT CLEAN  the tag: its commit {self.commit} is not in origin/main's history", said)
            self.git("update-ref", "refs/remotes/origin/main", self.commit)
            self.assertEqual(self.run_main(self.tag, "--published")[0], 0)

    def test_no_artifact_named_is_a_usage_error(self):
        self.assertEqual(self.run_main(self.tag)[0], 2)
        self.assertEqual(self.run_main()[0], 2)

    def test_a_tag_that_is_no_releases_stops_saying_so(self):
        code, _ = self.run_main("v0.3.0", "some-file")
        self.assertEqual(code, "release_artifacts: v0.3.0 is no release tag (vX.Y.Z+<the code commit's 40 hex digits>)")

    def test_the_token_goes_to_githubs_api_and_nowhere_else(self):
        """The address was searched for the API's name: a path or a longer
        host that held it got the token too."""
        sent = {}

        class Answer:
            def __enter__(self):
                return self

            def __exit__(self, *_):
                return False

            def read(self):
                return b"bytes"

        def urlopen(request, timeout):
            sent[request.full_url] = request.get_header("Authorization")
            return Answer()

        addresses = {
            "https://api.github.com/repos/o/r/releases/tags/v1": "Bearer a-token",
            "https://github.com/o/r/releases/download/v1/b3sum": None,
            "https://static.crates.io/crates/c/c-1.crate": None,
            "https://elsewhere.example/api.github.com/x": None,
            "https://api.github.com.elsewhere.example/x": None,
            "https://elsewhere.example/?next=https://api.github.com/": None,
            "http://api.github.com/repos/o/r": None,
        }
        with patch.dict(os.environ, {"GITHUB_TOKEN": "a-token"}), patch.object(ra.urllib.request, "urlopen", urlopen):
            for address in addresses:
                self.assertEqual(ra.fetch(address), b"bytes")
        self.assertEqual(sent, addresses)

    def test_the_token_is_not_sent_on_where_a_redirect_leads(self):
        """An ordinary header is copied to the request a redirect makes,
        whatever its host; the token is not such a header."""
        with patch.dict(os.environ, {"GITHUB_TOKEN": "a-token"}):
            request = ra.request_for("https://api.github.com/repos/o/r/releases/assets/1")
        self.assertEqual(request.get_header("Authorization"), "Bearer a-token")
        followed = ra.urllib.request.HTTPRedirectHandler().redirect_request(request, None, 302, "Found", {}, "https://storage.example/asset")
        self.assertIsNone(followed.get_header("Authorization"))
        self.assertIsNotNone(followed.get_header("User-agent"))

    def test_a_tag_with_a_zero_before_a_digit_is_no_release_tag(self):
        tag = "v01.0.0+" + "a" * 40
        code, _ = self.run_main(tag, "some-file")
        self.assertEqual(code, f"release_artifacts: {tag} is no release tag (vX.Y.Z+<the code commit's 40 hex digits>)")

    def test_a_tag_whose_version_is_in_another_scripts_digits_is_no_release_tag(self):
        tag = "v\u0660.\u0663.\u0660+" + "a" * 40
        code, _ = self.run_main(tag, "some-file")
        self.assertEqual(code, f"release_artifacts: {tag} is no release tag (vX.Y.Z+<the code commit's 40 hex digits>)")


class Image(unittest.TestCase):
    """The container a release binary is built in is named once, by its
    digest: the image line of tools/b3sum-release.toolchain. Every command
    that runs the build, in the pages and in the workflows, reads it from
    there, so the command a user copies starts from the bytes CI starts
    from."""

    def test_the_image_is_named_once_and_every_command_reads_it(self):
        root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
        with open(os.path.join(root, "tools", "b3sum-release.toolchain")) as file:
            images = [line.split(" ", 1)[1].strip() for line in file if line.startswith("image ")]
        self.assertEqual(len(images), 1)
        self.assertRegex(images[0], r"^ubuntu:[0-9.]+@sha256:[0-9a-f]{64}$")
        tag = images[0].split("@")[0]
        reads = "\"$(sed -n 's/^image //p' tools/b3sum-release.toolchain)\" sh tools/b3sum-release-container.sh"
        tracked = subprocess.run(["git", "ls-files", "-z"], cwd=root, capture_output=True, text=True, check=True).stdout.split("\0")
        named, commands, other = [], 0, []
        # Where commands are written: pages, workflows, scripts.
        for name in tracked:
            if not name.endswith((".md", ".yml", ".sh", ".toolchain")):
                continue
            with open(os.path.join(root, name), encoding="utf-8") as file:
                lines = file.read().splitlines()
            if any(tag in line for line in lines):
                named.append(name)
            runs = [line for line in lines if "docker run" in line and "b3sum-release-container.sh" in line]
            commands += len(runs)
            other += [f"{name}: {line.strip()}" for line in runs if reads not in line]
        self.assertEqual(named, ["tools/b3sum-release.toolchain"])
        self.assertEqual(other, [])
        self.assertEqual(commands, 5, "two pages, the script's own header, two workflows")


if __name__ == "__main__":
    unittest.main()
