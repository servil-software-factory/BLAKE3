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

    def stamp(self, arg, succeeds=True):
        result = subprocess.run(["python3", "tools/gen-ver.py", arg], cwd=self.root,
                                text=True, capture_output=True,
                                env=dict(os.environ, CARGO_NET_OFFLINE="true"))
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
        notes.write_text("# blake3-servil 0.8.0+dev, against official BLAKE3\n")
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
