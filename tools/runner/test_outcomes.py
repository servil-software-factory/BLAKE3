"""The runner's completed jobs use statuses wait_for already understands."""
import json
import subprocess
import tempfile
import unittest
from pathlib import Path

import exchange
import runner


class FakeRun:
    def __init__(self, code=0):
        self.code = code
        self.command = None

    def checkouts(self, *_):
        return Path("/code/library"), Path("/code/benchmark")

    def run(self, command, **_):
        self.command = command
        if self.code:
            raise subprocess.CalledProcessError(self.code, command)


class CheckOutcomes(unittest.TestCase):
    def job(self, probe=False):
        job = {"old_commit": "1" * 40, "new_commit": "2" * 40, "bench_commit": "3" * 40}
        job["probe" if probe else "points"] = "lent 64 KiB"
        return job

    def test_probe_completes_without_claiming_a_speed_verdict(self):
        run = FakeRun()
        self.assertEqual(runner.check(self.job(True), run, None, Path("/results")), "ok")
        self.assertIn("--probe", run.command)

    def test_gate_and_failure_outcomes_keep_their_meaning(self):
        for code, outcome in [(0, "no regression"), (1, "regression"), (2, "no verdict"), (3, "a side stopped")]:
            self.assertEqual(runner.check(self.job(), FakeRun(code), None, Path("/results")), outcome)


class WholeRun(FakeRun):
    """A checkout on disk where the tool's run would leave its lists and outcomes; `git diff` answers with a
    line, the tool exits as told."""

    def __init__(self, root, code=0):
        super().__init__(code)
        self.root, self.commands = Path(root), []
        self.log = open(self.root / "runner.log", "w")

    def checkouts(self, *_):
        return self.root, None

    def run(self, command, capture=False, **_):
        self.commands.append(command)
        if command[0] == "git":
            return "+a row\n"
        print("$ (cd checkout && tools/mutants/check.py)", file=self.log, flush=True)
        if self.code:
            raise subprocess.CalledProcessError(self.code, command)


class WholeMutationRun(unittest.TestCase):
    def checkout(self, root, crate):
        lists = Path(root) / "tools" / "mutants" / ("" if crate == "." else crate)
        lists.mkdir(parents=True)
        for kind in ("known", "timeouts"):
            (lists / f"{kind}-{runner.platform.machine()}.txt").write_text(f"# {kind} of {crate}\nsrc/x.rs\tdelete ! in f\t1\ta reason\n")
            (lists / f"{kind}-another.txt").write_text("another machine's\n")
        shard = Path(root) / "tmp" / "mutants-last" / ("" if crate == "." else crate) / "run" / "shard0"
        shard.mkdir(parents=True)
        (shard / "outcomes.json").write_text(json.dumps({"outcomes": [
            {"scenario": "Baseline", "summary": "Success", "phase_results": [{"duration": 9.0}]},
            {"scenario": {"Mutant": {"name": "src/x.rs:1:1: replace f -> u8 with 0"}}, "summary": "CaughtMutant",
             "phase_results": [{"duration": 1.25}, {"duration": 0.5}]}]}))

    def test_a_crate_s_lists_and_outcomes_come_back_and_nothing_else(self):
        for crate in (".", "threads"):
            with tempfile.TemporaryDirectory() as root, tempfile.TemporaryDirectory() as out:
                self.checkout(root, crate)
                run = WholeRun(root)
                status = runner.mutants_whole({"fork_commit": "1" * 40, "crate": crate}, run, None, Path(out))
                run.log.close()
                self.assertEqual(status, "no new survivor")
                self.assertEqual(run.commands[0][1:], ["tools/mutants/check.py", "--whole", "--write", "--crate", crate])
                machine = runner.platform.machine()
                self.assertEqual(sorted(p.name for p in (Path(out) / "lists").iterdir()), [f"known-{machine}.txt", f"timeouts-{machine}.txt"])
                self.assertEqual((Path(out) / "lists" / f"known-{machine}.txt").read_text(), f"# known of {crate}\nsrc/x.rs\tdelete ! in f\t1\ta reason\n")
                # the lists as the run alone wrote them, beside the lists to land (test_union.py: where they differ)
                self.assertEqual(sorted(p.name for p in (Path(out) / "lists-written").iterdir()), [f"known-{machine}.txt", f"timeouts-{machine}.txt"])
                self.assertEqual((Path(out) / "lists-put-back.txt").read_text(), "")
                self.assertEqual((Path(out) / "lists-diff.txt").read_text(), "+a row\n")
                # The exchange publishes a file by its kind: one of another kind stays on the machine, as the
                # first run's lists.diff did.
                for written in Path(out).rglob("*"):
                    self.assertTrue(written.is_dir() or written.suffix in exchange.PUBLISHED_KINDS, written)
                self.assertEqual((Path(out) / "outcomes.tsv").read_text(), "run\tsrc/x.rs:1:1: replace f -> u8 with 0\tCaughtMutant\t1.75\n")
                self.assertFalse((Path(out) / "mutants-last").exists())

    def test_its_exit_is_the_verdict_and_the_lists_come_back_either_way(self):
        for code, outcome in [(1, "a survivor differs from its list"), (2, "only time-outs differ")]:
            with tempfile.TemporaryDirectory() as root, tempfile.TemporaryDirectory() as out:
                self.checkout(root, ".")
                run = WholeRun(root, code)
                self.assertEqual(runner.mutants_whole({"fork_commit": "1" * 40, "crate": "."}, run, None, Path(out)), outcome)
                run.log.close()
                self.assertTrue((Path(out) / "outcomes.tsv").exists())

    def test_a_job_names_one_crate(self):
        for crate in (None, "b3sum", "../x"):
            with self.assertRaises(AssertionError):
                runner.mutants_whole({"fork_commit": "1" * 40, "crate": crate}, FakeRun(), None, Path("/results"))

    def test_a_whole_run_has_no_default_time_ceiling(self):
        self.assertFalse(hasattr(runner, "MAX_LIMIT"))
        self.assertFalse(hasattr(runner, "DEFAULT_LIMIT"))
        self.assertIs(runner.HANDLERS["mutants-whole"], runner.mutants_whole)


class B3sumAgainst(unittest.TestCase):
    def test_b3sum_with_against_runs_old_as_base_with_new_and_official(self):
        old_c = "2c60420c2bccbf6073e35e668691d98520f9b096"
        new_c = "abcdef0123456789abcdef0123456789abcdef01"
        bench_c = "3" * 40
        commands = []

        class StubRun:
            def checkouts(self, *_):
                return Path("/code/fork"), Path("/code/bench")

            def run(self, command, capture=False, **_):
                commands.append(command)
                if "b3sum-contenders.py" in command[1]:
                    return "\n".join([
                        "official=/bins/official",
                        "official-1-thread=/bins/official --num-threads 1",
                        f"b3sum-{old_c[:7]}=/bins/old",
                        f"b3sum-{new_c[:7]}=/bins/new",
                        "sha256sum=/usr/bin/sha256sum",
                    ]) + "\n"
                if "build-against" in command:
                    return "/bins/bench-hashes\n"
                return ""

        status = runner.b3sum(
            {"fork_commit": new_c, "against": old_c, "bench_commit": bench_c},
            StubRun(),
            None,
            Path("/results"),
        )
        self.assertEqual(status, "ok")
        self.assertEqual(commands[0][-2:], [old_c, new_c])
        b3sum_cmd = commands[-1]
        self.assertEqual(
            b3sum_cmd[-3:],
            [f"b3sum-{old_c[:7]}=/bins/old", f"b3sum-{new_c[:7]}=/bins/new", "official=/bins/official"],
        )


if __name__ == "__main__":
    unittest.main()
