"""Tests for runner.b3sum standalone `b3sum_commit` and legacy `fork_commit` routing."""
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import runner


class FakeRun:
    def __init__(self, work: Path):
        self.work = work
        self.synced = []
        self.commands = []

    def checkouts(self, fork_commit, bench_commit):
        fork = self.work / "fork"
        bench = self.work / "bench"
        fork.mkdir(parents=True, exist_ok=True)
        bench.mkdir(parents=True, exist_ok=True)
        self.synced.append(("checkouts", fork_commit, bench_commit))
        return fork, bench

    def sync(self, url, dest, commit):
        dest.mkdir(parents=True, exist_ok=True)
        self.synced.append((url, dest, commit))

    def run(self, cmd, cwd=None, capture=False):
        self.commands.append((list(cmd), Path(cwd) if cwd else None))
        if len(cmd) >= 2 and str(cmd[1]).endswith("b3sum-contenders.py"):
            commits = cmd[4:]
            lines = [f"b3sum-{c[:7]}=/bin/b3sum-{c[:7]}" for c in commits] + ["official=/bin/official"]
            return "\n".join(lines) + "\n"
        if "build-against" in cmd:
            return "/tmp/bench-exe\n"
        return "" if capture else None


class B3sumJobTests(unittest.TestCase):
    def test_standalone_b3sum_commit_uses_b3sum_repo_for_contenders_and_fork_for_bench(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            run = FakeRun(root / "work")
            out = root / "out"
            out.mkdir()
            job = {
                "type": "b3sum",
                "fork_commit": "a" * 40,
                "b3sum_commit": "b" * 40,
                "against": "c" * 40,
                "bench_commit": "d" * 40,
                "quick": True,
            }
            with patch.object(runner, "CHECKOUTS", root / "checkouts"), patch.object(runner, "HOME", root / "home"):
                verdict = runner.b3sum(job, run, root / "work", out)
            self.assertEqual(verdict, "ok")
            # Separate sync to standalone b3sum repo.
            b3sum_syncs = [s for s in run.synced if s[0] == runner.B3SUM_URL]
            self.assertEqual(len(b3sum_syncs), 1)
            self.assertEqual(b3sum_syncs[0][2], "b" * 40)
            b3sum_dir = b3sum_syncs[0][1]
            # Contender builder receives standalone b3sum checkout + (against, b3sum_commit).
            contender_cmd = next(cmd for cmd, _ in run.commands if len(cmd) > 1 and str(cmd[1]).endswith("b3sum-contenders.py"))
            self.assertEqual(Path(contender_cmd[2]), b3sum_dir)
            self.assertEqual(contender_cmd[4:], ["c" * 40, "b" * 40])
            # bench-hashes build-against still uses the BLAKE3 library fork checkout.
            build_cmd = next(cmd for cmd, _ in run.commands if "build-against" in cmd)
            self.assertEqual(Path(build_cmd[-1]), root / "work" / "fork")
            # Benchmark invocation receives contenders in old, new, official order.
            bench_cmd = next(cmd for cmd, _ in run.commands if cmd[0] == "/tmp/bench-exe")
            self.assertEqual(
                bench_cmd[-3:],
                [f"b3sum-{'c'*7}=/bin/b3sum-{'c'*7}", f"b3sum-{'b'*7}=/bin/b3sum-{'b'*7}", "official=/bin/official"],
            )

    def test_legacy_job_without_b3sum_commit_uses_fork_repo(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            run = FakeRun(root / "work")
            out = root / "out"
            out.mkdir()
            job = {"type": "b3sum", "fork_commit": "e" * 40, "bench_commit": "f" * 40}
            with patch.object(runner, "CHECKOUTS", root / "checkouts"), patch.object(runner, "HOME", root / "home"):
                verdict = runner.b3sum(job, run, root / "work", out)
            self.assertEqual(verdict, "ok")
            self.assertFalse(any(s[0] == runner.B3SUM_URL for s in run.synced))
            contender_cmd = next(cmd for cmd, _ in run.commands if len(cmd) > 1 and str(cmd[1]).endswith("b3sum-contenders.py"))
            self.assertEqual(Path(contender_cmd[2]), root / "work" / "fork")
            self.assertEqual(contender_cmd[4:], ["e" * 40])


if __name__ == "__main__":
    unittest.main()
