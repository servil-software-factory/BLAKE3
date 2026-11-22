"""Default build directory for linked worktrees (#453)."""
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest

import check


def _git(where, *args, env=None):
    return subprocess.run(
        ["git", "-C", str(where), *args],
        check=True, capture_output=True, text=True, env=env,
    )


def _clean_env(extra=None):
    env = {k: v for k, v in os.environ.items() if k not in ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR")}
    if extra:
        env.update(extra)
    return env


def _make_crate(path, name="fixture"):
    path.mkdir(parents=True, exist_ok=True)
    with open(path / "Cargo.toml", "w") as f:
        f.write(f'[package]\nname = "{name}"\nversion = "0.1.0"\nedition = "2021"\n')
    src = path / "src"
    src.mkdir(exist_ok=True)
    with open(src / "lib.rs", "w") as f:
        f.write("pub fn f() -> u32 { 1 }\n")


def _init_repo(repo, extra_crates=()):
    _make_crate(repo, "root_crate")
    for sub in extra_crates:
        _make_crate(repo / sub, f"sub_{sub}")
    _git(repo, "init", "-q")
    _git(repo, "config", "user.name", "Mutation fixture")
    _git(repo, "config", "user.email", "mutants@example.invalid")
    _git(repo, "config", "commit.gpgsign", "false")
    _git(repo, "add", ".")
    _git(repo, "-c", "core.hooksPath=/dev/null", "commit", "-qm", "init")


class DefaultTargetDir(unittest.TestCase):
    def test_linked_worktree_uses_main_checkout_target_by_default(self):
        with tempfile.TemporaryDirectory() as d:
            home = Path(d)
            main, wt = home / "main", home / "wt"
            _init_repo(main, extra_crates=("threads",))
            _git(main, "worktree", "add", "-q", "--detach", str(wt), "HEAD")
            env = _clean_env({"CARGO_HOME": str(home / "cargo-home")})
            # Main checkout stays on its own target/.
            self.assertEqual(
                os.path.realpath(check.default_target_dir(str(main), str(main), env=env)),
                os.path.realpath(str(main / "target")),
            )
            # Linked worktree defaults to main checkout's target/.
            self.assertEqual(
                os.path.realpath(check.default_target_dir(str(wt), str(wt), env=env)),
                os.path.realpath(str(main / "target")),
            )
            # Subdirectory crate (--crate threads) maps to main/threads/target.
            self.assertEqual(
                os.path.realpath(check.default_target_dir(str(wt), str(wt / "threads"), env=env)),
                os.path.realpath(str(main / "threads" / "target")),
            )

    def test_explicit_env_or_cargo_config_is_respected(self):
        with tempfile.TemporaryDirectory() as d:
            home = Path(d)
            main, wt = home / "main", home / "wt"
            _init_repo(main)
            _git(main, "worktree", "add", "-q", "--detach", str(wt), "HEAD")
            base_env = _clean_env({"CARGO_HOME": str(home / "cargo-home")})

            # Explicit CARGO_TARGET_DIR pointing at the worktree's own target/ is NOT overridden.
            own_target = str(wt / "target")
            env1 = dict(base_env, CARGO_TARGET_DIR=own_target)
            self.assertEqual(
                os.path.realpath(check.default_target_dir(str(wt), str(wt), env=env1)),
                os.path.realpath(own_target),
            )

            # Explicit CARGO_BUILD_TARGET_DIR is respected too.
            custom = str(home / "explicit-build")
            env2 = dict(base_env, CARGO_BUILD_TARGET_DIR=custom)
            self.assertEqual(
                os.path.realpath(check.default_target_dir(str(wt), str(wt), env=env2)),
                os.path.realpath(custom),
            )

            # Cargo config [build] target-dir is respected.
            cfg_dir = wt / ".cargo"
            cfg_dir.mkdir()
            cfg_target = home / "cfg-target"
            with open(cfg_dir / "config.toml", "w") as f:
                f.write(f'[build]\ntarget-dir = "{cfg_target.as_posix()}"\n')
            self.assertEqual(
                os.path.realpath(check.default_target_dir(str(wt), str(wt), env=base_env)),
                os.path.realpath(str(cfg_target)),
            )

    def test_bare_repo_and_separate_git_dir_keep_worktree_target(self):
        with tempfile.TemporaryDirectory() as d:
            home = Path(d)
            src = home / "src"
            _init_repo(src)
            env = _clean_env({"CARGO_HOME": str(home / "cargo-home")})

            # Bare repo named repo.git + linked worktree
            bare = home / "repo.git"
            subprocess.run(["git", "clone", "-q", "--bare", str(src), str(bare)], check=True)
            bwt = home / "bwt"
            _git(bare, "worktree", "add", "-q", "--detach", str(bwt), "HEAD")
            self.assertIsNone(check.main_checkout(str(bwt)))
            self.assertEqual(
                os.path.realpath(check.default_target_dir(str(bwt), str(bwt), env=env)),
                os.path.realpath(str(bwt / "target")),
            )

            # Bare repo literally named .git + linked worktree
            bare_dot = home / "bare_parent" / ".git"
            bare_dot.parent.mkdir()
            subprocess.run(["git", "clone", "-q", "--bare", str(src), str(bare_dot)], check=True)
            bdwt = home / "bdwt"
            _git(bare_dot, "worktree", "add", "-q", "--detach", str(bdwt), "HEAD")
            self.assertIsNone(check.main_checkout(str(bdwt)))
            self.assertEqual(
                os.path.realpath(check.default_target_dir(str(bdwt), str(bdwt), env=env)),
                os.path.realpath(str(bdwt / "target")),
            )

            # Separate git dir + linked worktree
            sep, sep_git = home / "sep", home / "sep.git"
            _make_crate(sep, "sep_crate")
            subprocess.run(["git", "init", "-q", f"--separate-git-dir={sep_git}", str(sep)], check=True)
            _git(sep, "config", "user.name", "Mutation fixture")
            _git(sep, "config", "user.email", "mutants@example.invalid")
            _git(sep, "config", "commit.gpgsign", "false")
            _git(sep, "add", ".")
            _git(sep, "-c", "core.hooksPath=/dev/null", "commit", "-qm", "init")
            swt = home / "swt"
            _git(sep, "worktree", "add", "-q", "--detach", str(swt), "HEAD")
            self.assertIsNone(check.main_checkout(str(swt)))
            self.assertEqual(
                os.path.realpath(check.default_target_dir(str(swt), str(swt), env=env)),
                os.path.realpath(str(swt / "target")),
            )

    def test_new_crate_only_in_worktree_or_non_dir_target_falls_back(self):
        with tempfile.TemporaryDirectory() as d:
            home = Path(d)
            main, wt = home / "main", home / "wt"
            _init_repo(main)
            _git(main, "worktree", "add", "-q", "--detach", str(wt), "HEAD")
            env = _clean_env({"CARGO_HOME": str(home / "cargo-home")})

            # Branch in worktree adds a new crate subdirectory not present in main:
            # do not create an untracked directory in main.
            _make_crate(wt / "newcrate", "newcrate")
            self.assertEqual(
                os.path.realpath(check.default_target_dir(str(wt), str(wt / "newcrate"), env=env)),
                os.path.realpath(str(wt / "newcrate" / "target")),
            )
            self.assertFalse((main / "newcrate").exists())

            # If main/target exists as a regular file or broken symlink, fall back safely.
            with open(main / "target", "w") as f:
                f.write("not a dir\n")
            self.assertEqual(
                os.path.realpath(check.default_target_dir(str(wt), str(wt), env=env)),
                os.path.realpath(str(wt / "target")),
            )
            (main / "target").unlink()
            os.symlink(str(home / "missing-target"), str(main / "target"))
            self.assertEqual(
                os.path.realpath(check.default_target_dir(str(wt), str(wt), env=env)),
                os.path.realpath(str(wt / "target")),
            )
            (main / "target").unlink()

            # Read-only main/target falls back when not running as root.
            if os.geteuid() != 0:
                (main / "target").mkdir()
                os.chmod(main / "target", stat.S_IRUSR | stat.S_IXUSR)
                try:
                    self.assertEqual(
                        os.path.realpath(check.default_target_dir(str(wt), str(wt), env=env)),
                        os.path.realpath(str(wt / "target")),
                    )
                finally:
                    os.chmod(main / "target", 0o755)


if __name__ == "__main__":
    unittest.main()
