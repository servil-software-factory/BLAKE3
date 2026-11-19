"""Version stamping in disposable repositories: no release or registry upload.

Cargo resolves four tiny path packages and writes their real lockfile. The
fixtures have no registry dependencies and never compile code.
"""
from pathlib import Path
import os
import shutil
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("gen-ver.py")
DEV = "0.8.0+dev"
MANIFESTS = {
    "version/Cargo.toml": ("blake3-servil-version", ""),
    "Cargo.toml": ("blake3-servil-core", 'blake3-servil-version = { version = "=0.8.0+dev", path = "version" }'),
    "threads/Cargo.toml": ("blake3-servil", 'blake3-servil-core = { version = "=0.8.0+dev", path = ".." }'),
    "b3sum/Cargo.toml": ("b3sums", 'blake3 = { version = "=0.8.0+dev", path = "../threads", package = "blake3-servil" }'),
    "capi/Cargo.toml": ("blake3-servil-c", 'blake3-servil = { path = "..", package = "blake3-servil-core" }'),
}


class Versions(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory(dir="/dev/shm" if Path("/dev/shm").is_dir() else None)
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        for path, (name, dependency) in MANIFESTS.items():
            manifest = self.root / path
            manifest.parent.mkdir(parents=True, exist_ok=True)
            manifest.write_text(f'[package]\nname = "{name}"\nversion = "{DEV}"\nedition = "2024"\n[dependencies]\n{dependency}\n')
            src = manifest.parent / "src"
            src.mkdir()
            (src / "lib.rs").write_text("")
        (self.root / "tools").mkdir()
        shutil.copyfile(SCRIPT, self.root / "tools/gen-ver.py")
        # The stamp reads its own commit as the release check will.
        shutil.copyfile(SCRIPT.with_name("release_artifacts.py"), self.root / "tools/release_artifacts.py")
        (self.root / "CHANGELOG.md").write_text("# blake3-servil 0.8.0+" + "a" * 40 + ", against official BLAKE3\n")
        (self.root / "capi/b3.h").write_text(f'#define B3_VERSION "{DEV}"\n')
        self.run_command("cargo", "generate-lockfile", "--offline", "--manifest-path", "b3sum/Cargo.toml")
        (self.root / "tools/release").mkdir()
        shutil.copyfile(self.root / "b3sum/Cargo.lock", self.root / "tools/release/Cargo.lock")
        self.git("init", "-q")
        self.git("config", "user.name", "Version test")
        self.git("config", "user.email", "version-test@example.invalid")
        self.git("config", "core.hooksPath", str(self.root / "no-hooks"))
        self.git("config", "commit.gpgsign", "false")
        self.git("remote", "add", "origin", "https://example.invalid/repo")
        self.git("add", ".")
        self.git("commit", "-qm", "fixture")

    def run_command(self, *args):
        return subprocess.run(args, cwd=self.root, text=True, capture_output=True, check=True,
                              env=dict(os.environ, CARGO_NET_OFFLINE="true")).stdout.strip()

    def git(self, *args):
        return self.run_command("git", *args)

    def stamp(self, arg, succeeds=True, path=None):
        """Run the tool; `path`: a folder searched for programs before the others."""
        env = dict(os.environ, CARGO_NET_OFFLINE="true")
        if path:
            env["PATH"] = f"{path}{os.pathsep}{env['PATH']}"
        result = subprocess.run(["python3", "tools/gen-ver.py", arg], cwd=self.root,
                                text=True, capture_output=True, env=env)
        self.assertEqual(result.returncode == 0, succeeds, result.stdout + result.stderr)
        return result.stdout + result.stderr

    def test_release_and_dev_update_every_copy_and_lock(self):
        self.stamp("--check")
        parent = self.git("rev-parse", "HEAD")
        full = "99.1.2+" + parent
        output = self.stamp("99.1.2")
        version_commit = self.git("rev-parse", "HEAD")
        self.assertEqual(self.git("rev-parse", "HEAD^"), parent)
        self.assertEqual(self.git("rev-parse", "v" + full), version_commit)
        self.assertIn(f'blake3-servil-core = {{ git = "https://example.invalid/repo", rev = "{version_commit}", version = "={full}" }}', output)
        self.assertIn(f'blake3-servil = {{ git = "https://example.invalid/repo", rev = "{version_commit}", version = "={full}" }}', output)
        for path in MANIFESTS:
            self.assertIn(f'version = "{full}"', (self.root / path).read_text())
        self.assertEqual((self.root / "b3sum/Cargo.lock").read_text().count(f'version = "{full}"'), 4)
        self.assertEqual((self.root / "tools/release/Cargo.lock").read_text().count(f'version = "{full}"'), 4)
        notes = (self.root / "CHANGELOG.md").read_text()
        self.assertTrue(notes.startswith(f"# blake3-servil {full},"))
        self.assertEqual(self.git("status", "--porcelain"), "")
        self.stamp("--check")
        self.git("commit", "--allow-empty", "-qm", "after the release")
        self.assertIn("past that release", self.stamp("--check", succeeds=False))
        self.stamp("--dev")
        self.stamp("--check")
        for path in MANIFESTS:
            self.assertIn('version = "99.1.2+dev"', (self.root / path).read_text())
        self.assertEqual((self.root / "CHANGELOG.md").read_text(), notes)
        self.assertEqual(self.git("status", "--porcelain"), "")

    def test_the_release_check_takes_the_version_commit_gen_ver_makes(self):
        # The two tools agree on what a version commit is: the stamp
        # check of the release's artifacts finds nothing in the commit
        # gen-ver makes, from a title in either form it can replace.
        import importlib.util

        spec = importlib.util.spec_from_file_location("release_artifacts", SCRIPT.with_name("release_artifacts.py"))
        release_artifacts = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(release_artifacts)
        for title in ("0.8.0+dev", "0.8.0+" + "a" * 40):
            with self.subTest(title=title):
                (self.root / "CHANGELOG.md").write_text(f"# blake3-servil {title}, against official BLAKE3\n")
                self.git("commit", "-qam", "the notes' title", "--allow-empty")
                code = self.git("rev-parse", "HEAD")
                number = "99.1." + str(len(title))
                self.stamp(number)
                release = release_artifacts.Release(f"v{number}+{code}", repo=str(self.root))
                self.assertEqual(release.stamp_problems(code), [])
                self.stamp("--dev")

    def test_check_rejects_a_title_no_release_can_stamp(self):
        # The 0.10.0 candidate's: "+SHA" where the commit or "dev" goes.
        notes = self.root / "CHANGELOG.md"
        notes.write_text("# blake3-servil 0.8.0+SHA, against official BLAKE3\n")
        self.assertIn("CHANGELOG.md: its title must name a version", self.stamp("--check", succeeds=False))
        # The stamp replaces the whole token up to a space or a comma, so
        # the version must be that whole token: more after it would be lost.
        for more in ("dev-preview", "devx", "dev.1", "a" * 41, "a" * 40 + "-dirty"):
            with self.subTest(more=more):
                notes.write_text(f"# blake3-servil 0.8.0+{more}, against official BLAKE3\n")
                self.assertIn("CHANGELOG.md: its title must name a version", self.stamp("--check", succeeds=False))
        # Digits and spaces of other scripts are no part of a version: the
        # release check reads the title's bytes.
        for title in ("\uff10.\uff18.\uff10+dev, against official BLAKE3", "0.8.0+dev\u00a0against official BLAKE3"):
            with self.subTest(title=title):
                notes.write_text(f"# blake3-servil {title}\n")
                self.assertIn("CHANGELOG.md: its title must name a version", self.stamp("--check", succeeds=False))
        # A stamp refuses such a title too, before it replaces it: after,
        # the title would be the stamp's own, and look right.
        notes.write_text("# blake3-servil 0.8.0+dev-preview, against official BLAKE3\n")
        self.git("commit", "-qam", "a title no stamp may replace")
        before = {p: p.read_bytes() for p in self.root.rglob("*") if p.is_file() and ".git" not in p.parts}
        self.assertIn("before a stamp replaces it", self.stamp("0.9.0", succeeds=False))
        self.assertEqual({p: p.read_bytes() for p in self.root.rglob("*") if p.is_file() and ".git" not in p.parts}, before, "a refused stamp writes nothing")
        for title in ("0.8.0+dev, against official BLAKE3", "0.8.0+dev against official BLAKE3", "0.8.0+dev", "0.8.0+" + "a" * 40 + ", against official BLAKE3"):
            with self.subTest(title=title):
                notes.write_text(f"# blake3-servil {title}\n")
                self.stamp("--check")

    def test_a_stamp_replaces_the_title_alone(self):
        # The title's version runs to a space, a comma or the line's end: a
        # title with nothing after its version once let the stamp run on
        # into the lines below it, up to their first space or comma.
        notes = self.root / "CHANGELOG.md"
        notes.write_text("# blake3-servil 0.8.0+dev\n\nFaster,\nand the same hashes.\n")
        self.git("commit", "-qam", "a title that ends with its version")
        self.stamp("0.9.0")
        title, rest = notes.read_text().split("\n", 1)
        self.assertRegex(title, r"^# blake3-servil 0\.9\.0\+[0-9a-f]{40}$")
        self.assertEqual(rest, "\nFaster,\nand the same hashes.\n")

    def test_the_check_and_the_stamp_end_the_title_where_a_line_feed_does(self):
        # Python's splitlines ends a line at a form feed, a next-line
        # character and others, and the stamp's pattern read through them:
        # the check saw "0.8.0+dev", and the stamp replaced "-preview" too.
        notes = self.root / "CHANGELOG.md"
        for end in ("\x0c", "\x1c", "\x85", "\u2028"):
            with self.subTest(end=repr(end)):
                notes.write_text(f"# blake3-servil 0.8.0+dev{end}-preview, against official BLAKE3\n")
                self.assertIn("CHANGELOG.md: its title must name a version", self.stamp("--check", succeeds=False))
        self.git("commit", "-qam", "a title that ends at a line separator no file ends a line with")
        before = {p: p.read_bytes() for p in self.root.rglob("*") if p.is_file() and ".git" not in p.parts}
        self.assertIn("before a stamp replaces it", self.stamp("0.9.0", succeeds=False))
        self.assertEqual({p: p.read_bytes() for p in self.root.rglob("*") if p.is_file() and ".git" not in p.parts}, before, "a refused stamp writes nothing")

    def test_a_stamp_keeps_the_notes_line_ends(self):
        # A carriage return before each line feed stays: read as text and
        # written back, the notes lost them all, a change to every line.
        notes = self.root / "CHANGELOG.md"
        notes.write_bytes(b"# blake3-servil 0.8.0+dev, against official BLAKE3\r\n\r\nFaster.\r\n")
        self.git("commit", "-qam", "notes with carriage returns")
        self.stamp("0.9.0")
        title, rest = notes.read_bytes().split(b"\n", 1)
        self.assertRegex(title, rb"^# blake3-servil 0\.9\.0\+[0-9a-f]{40}, against official BLAKE3\r$")
        self.assertEqual(rest, b"\r\nFaster.\r\n")
        # With nothing after the version but the carriage return, the title
        # is not one the pattern knows, and nothing is stamped.
        notes.write_bytes(b"# blake3-servil 0.8.0+dev\r\n\r\nFaster.\r\n")
        self.assertIn("CHANGELOG.md: its title must name a version", self.stamp("--check", succeeds=False))

    def files(self):
        return {p: p.read_bytes() for p in self.root.rglob("*") if p.is_file() and ".git" not in p.parts and "__pycache__" not in p.parts}

    def test_a_stamp_the_release_check_would_refuse_is_undone(self):
        # Cargo writes b3sum's lock anew and drops a comment in it: a line
        # removed, which the release check allows no version commit. The
        # check before the stamp passes; the stamp made its commit and its
        # tag, and the release check then refused them.
        lock = self.root / "b3sum/Cargo.lock"
        lock.write_text(lock.read_text() + "# a note someone left\n")
        self.git("commit", "-qam", "a comment in b3sum's lock")
        self.stamp("--check")
        head, before = self.git("rev-parse", "HEAD"), self.files()
        said = self.stamp("0.9.0", succeeds=False)
        self.assertIn("the stamp is undone", said)
        self.assertIn("b3sum/Cargo.lock has lines added or removed", said)
        self.assertEqual(self.git("rev-parse", "HEAD"), head)
        self.assertEqual(self.git("tag", "-l"), "")
        self.assertEqual(self.git("status", "--porcelain", "--untracked-files=no"), "")
        self.assertEqual(self.files(), before)

    def test_a_stamp_that_stops_after_writing_puts_the_files_back(self):
        # Cargo's update of b3sum's lock comes after the manifests are
        # written, and can fail (no network, a registry that is away): the
        # tree was left stamped in part, with no commit.
        with tempfile.TemporaryDirectory() as bin:
            cargo = Path(bin) / "cargo"
            cargo.write_text("#!/bin/sh\necho 'cargo: it does not work today' >&2\nexit 101\n")
            cargo.chmod(0o755)
            for arg in ("0.9.0", "--dev"):
                with self.subTest(arg=arg):
                    if arg == "--dev":
                        self.stamp("0.9.0")
                    head, before = self.git("rev-parse", "HEAD"), self.files()
                    self.stamp(arg, succeeds=False, path=bin)
                    self.assertEqual(self.files(), before)
                    self.assertEqual(self.git("rev-parse", "HEAD"), head)
                    self.assertEqual(self.git("status", "--porcelain", "--untracked-files=no"), "")

    def stand_in(self, bin, program, script):
        """A program of that name in `bin` (a folder to search first):
        `script` is its shell, in which $real is the program it stands for."""
        path = Path(bin) / program
        path.write_text(f"#!/bin/sh\nreal={shutil.which(program)}\n{script}\n")
        path.chmod(0o755)

    def test_a_stamp_that_stops_touches_nothing_but_its_own_files(self):
        # Someone saves a file of theirs while the stamp runs, and the
        # stamp then stops. It put the whole tree back as Git had it, and
        # undid a refused stamp with a hard reset: their work was gone.
        theirs = self.root / "threads/src/lib.rs"
        saves = "echo '// their work' >> ../threads/src/lib.rs"
        lock = self.root / "b3sum/Cargo.lock"
        lock.write_text(lock.read_text() + "# a note someone left\n")
        self.git("commit", "-qam", "a comment in b3sum's lock")
        cases = {
            # Cargo fails after the manifests are written.
            "cargo fails": ("cargo", f"{saves}\nexit 101", "", "cargo"),
            # Cargo drops the lock's comment: the release check refuses the commit.
            "the release check refuses": ("cargo", f'{saves}\nexec $real "$@"', "the stamp is undone", "b3sum/Cargo.lock has lines added or removed"),
        }
        for case, (program, script, *said) in cases.items():
            with self.subTest(case=case), tempfile.TemporaryDirectory() as bin:
                self.stand_in(bin, program, script)
                head, before = self.git("rev-parse", "HEAD"), self.files()
                out = self.stamp("0.9.0", succeeds=False, path=bin)
                for words in said:
                    self.assertIn(words, out)
                self.assertEqual(self.git("rev-parse", "HEAD"), head)
                self.assertEqual(self.git("tag", "-l"), "")
                self.assertEqual(theirs.read_text(), "// their work\n", "their work is as they saved it")
                self.assertEqual(self.git("status", "--porcelain", "--untracked-files=no"), "M threads/src/lib.rs")
                theirs.write_text("")
                self.assertEqual(self.files(), before)

    def test_a_stamp_whose_commit_or_tag_git_refuses_is_undone(self):
        # The commit can be refused (no identity, a lock on the index) and
        # so can the tag (one of that name stands, a lock on the refs): the
        # one left the files stamped with no commit, the other a commit no
        # tag names.
        for refused in ("commit", "tag"):
            with self.subTest(refused=refused), tempfile.TemporaryDirectory() as bin:
                # (`git tag -l` lists the tags, and is let through.)
                self.stand_in(bin, "git", f'[ "$1" = {refused} ] && [ "$2" != -l ] && {{ echo "git: no {refused} today" >&2; exit 1; }}\nexec $real "$@"')
                head, before = self.git("rev-parse", "HEAD"), self.files()
                out = self.stamp("0.9.0", succeeds=False, path=bin)
                self.assertIn(f"no {refused} today", out)
                self.assertEqual(self.git("rev-parse", "HEAD"), head)
                self.assertEqual(self.git("tag", "-l"), "")
                self.assertEqual(self.git("status", "--porcelain", "--untracked-files=no"), "")
                self.assertEqual(self.files(), before)

    def test_a_refused_stamp_is_left_where_the_branch_has_moved_on(self):
        # Another commit lands on the stamp's before its tag is refused:
        # taking the stamp back would take that commit with it. Nothing is
        # moved, and the stamp says what it left.
        with tempfile.TemporaryDirectory() as bin:
            self.stand_in(bin, "git", '[ "$1" = tag ] && [ "$2" != -l ] && { $real commit -q --allow-empty -m "their commit"; echo "git: no tag today" >&2; exit 1; }\nexec $real "$@"')
            head = self.git("rev-parse", "HEAD")
            out = self.stamp("0.9.0", succeeds=False, path=bin)
            self.assertIn("is left where it is: the branch has moved on", out)
            self.assertEqual(self.git("log", "-1", "--format=%s"), "their commit")
            self.assertEqual(self.git("log", "-1", "--format=%s", "HEAD^"), f"Version 0.9.0+{head}")
            self.assertEqual(self.git("rev-parse", "HEAD^^"), head)
            self.assertEqual(self.git("tag", "-l"), "")
            self.assertEqual(self.git("status", "--porcelain", "--untracked-files=no"), "")

    def test_a_stamp_writes_nothing_where_a_line_to_replace_is_missing(self):
        # The header's line is not as the stamp expects (carriage returns):
        # the manifests before it were written by then, and the stamp left
        # them so.
        (self.root / "capi/b3.h").write_bytes(b'#define B3_VERSION "' + DEV.encode() + b'"\r\n')
        self.git("commit", "-qam", "a header with carriage returns")
        before = self.files()
        said = self.stamp("0.9.0", succeeds=False)
        self.assertIn("capi/b3.h: no line matches", said)
        self.assertIn("nothing is written", said)
        self.assertEqual(self.files(), before)
        self.assertEqual(self.git("tag", "-l"), "")

    def test_check_holds_a_version_commits_title_to_its_release(self):
        self.stamp("99.2.0")
        self.stamp("--check")
        notes = self.root / "CHANGELOG.md"
        stamped = notes.read_text()
        for title in ("0.8.0+dev", "0.8.0+" + "a" * 40, "99.2.0+dev"):
            with self.subTest(title=title):
                notes.write_text(f"# blake3-servil {title}, against official BLAKE3\n")
                self.assertIn("CHANGELOG.md: its title names the release 99.2.0+", self.stamp("--check", succeeds=False))
        notes.write_text(stamped)
        self.stamp("--check")

    def test_check_rejects_each_stale_copy(self):
        cases = [(path, f'version = "{DEV}"') for path in MANIFESTS if path != "Cargo.toml"]
        cases += [(path, f'version = "={DEV}"') for path in ("Cargo.toml", "threads/Cargo.toml", "b3sum/Cargo.toml")]
        cases += [("capi/b3.h", DEV), ("b3sum/Cargo.lock", DEV), ("tools/release/Cargo.lock", DEV)]
        for path, old in cases:
            with self.subTest(path=path, old=old):
                file = self.root / path
                original = file.read_text()
                file.write_text(original.replace(old, old.replace(DEV, "0.7.0+dev"), 1))
                self.assertIn(path, self.stamp("--check", succeeds=False))
                file.write_text(original)
        self.stamp("--check")

    def test_check_holds_the_release_lock_to_b3sums(self):
        # The crates ship the release lock's dependencies, and b3sum is
        # tested with its own lock's: a package of b3sum's lock is in the
        # release lock, the same version and the same archive.
        b3sum, release = self.root / "b3sum/Cargo.lock", self.root / "tools/release/Cargo.lock"
        package = '\n[[package]]\nname = "dependency"\nversion = "{}"\nsource = "registry+https://example.invalid"\nchecksum = "{}"\n'
        ours, theirs = b3sum.read_text(), release.read_text()
        b3sum.write_text(ours + package.format("1.2.3", "aa"))
        self.assertIn("b3sum/Cargo.lock has dependency 1.2.3, and the release lock has none", self.stamp("--check", succeeds=False))
        release.write_text(theirs + package.format("1.2.4", "aa"))
        self.assertIn("the release lock has ['1.2.4']", self.stamp("--check", succeeds=False))
        release.write_text(theirs + package.format("1.2.3", "bb"))
        self.stamp("--check", succeeds=False)
        # The same, beside a package of the libraries' tests alone.
        release.write_text(theirs + package.format("1.2.3", "aa") + package.format("9.9.9", "cc").replace("dependency", "for-tests"))
        self.stamp("--check")

    def test_dirty_tree_is_unchanged(self):
        file = self.root / "threads/src/lib.rs"
        file.write_text("// unfinished work\n")
        before = self.git("diff", "HEAD")
        self.assertIn("tree must be clean", self.stamp("99.1.2", succeeds=False))
        self.assertEqual(self.git("diff", "HEAD"), before)
        self.assertEqual(self.git("tag", "-l"), "")

    def test_version_must_advance(self):
        self.git("tag", "v99.1.2+" + "a" * 40)
        before = self.git("rev-parse", "HEAD")
        self.assertIn("not newer", self.stamp("99.1.2", succeeds=False))
        self.assertEqual(self.git("rev-parse", "HEAD"), before)
        self.assertEqual(self.git("status", "--porcelain"), "")


if __name__ == "__main__":
    unittest.main()
