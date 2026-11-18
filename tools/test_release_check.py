"""Release-gate controls. All GitHub answers and release logs are fixtures."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("release_check", Path(__file__).with_name("release-check.py"))
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)

HEAD, OLD = "a" * 40, "b" * 40
RUN = {"id": 7, "name": "tests", "event": "workflow_dispatch", "status": "completed", "conclusion": "success"}
MUTATION_OK = f"mutants: BLAKE3-Servil at {HEAD}\nmutants: the whole run\nmutants: 12 survive, each known and explained\nmutants: 3 time out, each known (timeouts-x86_64.txt)\n"


def job(conclusion="success", status="completed"):
    return {"name": "test job", "status": status, "conclusion": conclusion}


class CI(unittest.TestCase):
    def result(self, jobs):
        with patch.object(check, "github", return_value={"total_count": len(jobs), "jobs": jobs}):
            return check.ci_result(RUN)

    def test_pr_merge_run_is_not_release_evidence(self):
        with patch.object(check, "github") as github:
            self.assertIsNone(check.ci_result(dict(RUN, event="pull_request")))
            github.assert_not_called()

    def test_success_needs_successful_jobs(self):
        self.assertIs(self.result([job(), job()]), True)
        self.assertIsNone(self.result([]))

    def test_skipped_or_pending_is_open(self):
        for other in [job("skipped"), job(None, "in_progress")]:
            with self.subTest(other=other):
                self.assertIsNone(self.result([job(), other]))

    def test_failed_or_cancelled_does_not_pass(self):
        for conclusion in ["failure", "cancelled", "action_required"]:
            with self.subTest(conclusion=conclusion):
                self.assertIs(self.result([job(conclusion)]), False)

    def test_job_timeout_stays_distinct(self):
        self.assertEqual(self.result([job("timed_out")]), check.TIMED_OUT)

    def test_unfinished_or_failed_workflow_does_not_pass(self):
        with patch.object(check, "github") as github:
            self.assertIsNone(check.ci_result(dict(RUN, status="in_progress", conclusion=None)))
            self.assertIs(check.ci_result(dict(RUN, conclusion="failure")), False)
            self.assertEqual(check.ci_result(dict(RUN, conclusion="timed_out")), check.TIMED_OUT)
            github.assert_not_called()

    def test_all_pages_are_checked(self):
        replies = [{"total_count": 101, "jobs": [job()] * 100}, {"total_count": 101, "jobs": [job("skipped")]}]
        with patch.object(check, "github", side_effect=replies) as github:
            self.assertIsNone(check.ci_result(RUN))
            self.assertIn("page=2", github.call_args.args[0])

    def test_missing_page_stops(self):
        with patch.object(check, "github", return_value={"total_count": 1, "jobs": []}):
            with self.assertRaises(AssertionError):
                check.ci_result(RUN)


class ReleaseEvidence(unittest.TestCase):
    def run_gate(self, cells="all", kind="check", ci_jobs=None, new=HEAD, old=OLD, verdict="no regression", git_override=None, ci_event="workflow_dispatch", mutation_log=MUTATION_OK, core_log=MUTATION_OK, power=("AC Power, 100%", "AC Power, 100%"), artifacts=(True, ""), vm_log=None, previous=None):
        jobs = [job(), job()] if ci_jobs is None else ci_jobs
        def git(*args):
            if args[0] == "rev-parse":
                return HEAD if args[1] == "candidate" else OLD
            if args[0] == "tag":
                return "v0.3.0+fixture"
            if args[0] == "describe":
                return "v0.3.0+fixture"
            if args[0] == "diff":
                return "README.md\nQUALITY.md\nCHANGELOG.md"
            raise AssertionError(args)

        def github(path):
            if "/actions/runs?" in path:
                return {"workflow_runs": [dict(RUN, event=ci_event)]}
            if "/actions/runs/7/jobs?" in path:
                return {"total_count": len(jobs), "jobs": jobs}
            if path.endswith("/statuses"):
                return [{"context": "long proofs", "state": "success"}]
            raise AssertionError(path)

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "mac-runner/jobs").mkdir(parents=True)
            (root / "mac-runner/results/1").mkdir(parents=True)
            task = {"type": kind, "old_commit": old, "new_commit": new}
            if cells is not None:
                task["cells"] = cells
            (root / "mac-runner/jobs/job.json").write_text(json.dumps(task))
            readings = {k: v for k, v in zip(("power at start", "power at end"), power) if v is not None}
            (root / "mac-runner/results/1/verdict.json").write_text(json.dumps({"job": "job.json", "status": verdict, **readings}))
            logs = root / "tmp/night"
            logs.mkdir(parents=True)
            candidate = (git_override or git)("rev-parse", "candidate")[:12]
            if vm_log is None:
                vm_log = f"check: BLAKE3-Servil's working tree ({candidate}) against {old} ({old[:12]})\nregress: no cell slower\n{check.VM_OK}"
            for name, text in [("check-release-vm", vm_log), ("check-builds", "every build passes"), ("publish-dry-run", "publish-dry-run: ok")]:
                (logs / f"{name}-{candidate}.log").write_text(text + "\n")
            if mutation_log is not None:
                # A real run prints the full SHA for each crate, even when this
                # fixture's history is created by PreviousRelease below.
                identity = (git_override or git)("rev-parse", "candidate")
                text = ("mutants: the crate at the root\n" + core_log
                        + "mutants: the crate in threads/\n" + mutation_log
                        + "mutants: 2 crates took 99 s in all\n")
                (logs / f"mutants-{candidate}.log").write_text(text.replace(HEAD, identity))
            output = io.StringIO()
            args = ["release-check.py", "candidate"] + (["--previous", previous] if previous else [])
            with patch.object(check, "MAIN", tmp), patch.object(check, "EXCHANGE", str(root / "mac-runner")), patch.object(check, "git", git_override or git), \
                 patch.object(check, "github", github), patch.object(check, "artifacts_result", lambda commit, run=None: artifacts), \
                 patch("sys.argv", args), contextlib.redirect_stdout(output):
                with self.assertRaises(SystemExit) as caught:
                    check.main()
            return caught.exception.code, output.getvalue()

    def test_complete_fixture_passes(self):
        self.assertEqual(self.run_gate()[0], 0)

    def test_artifacts_not_gathered_yet_are_open(self):
        code, output = self.run_gate(artifacts=(None, "not in dist/ yet: b3sum-v0.3.1+c-x86_64-unknown-linux-gnu"))
        self.assertEqual(code, 1)
        self.assertIn("OPEN  release artifacts, each a clean build of this commit: not in dist/ yet: b3sum-v0.3.1+c-x86_64-unknown-linux-gnu", output)

    def test_an_artifact_that_is_not_clean_fails(self):
        # 0.9.0's b3sum programs said so, and no step looked (BLAKE3 #435).
        code, output = self.run_gate(artifacts=(False, "NOT CLEAN  b3sum-v0.9.0+c-x86_64-unknown-linux-gnu: its build version is 0.9.0+c.d.dirty.e"))
        self.assertEqual(code, 1)
        self.assertIn("FAIL  release artifacts, each a clean build of this commit: NOT CLEAN  b3sum-v0.9.0+c-x86_64-unknown-linux-gnu", output)

    def test_pr_merge_ci_not_pass(self):
        code, output = self.run_gate(ci_event="pull_request")
        self.assertEqual(code, 1)
        self.assertIn("OPEN  CI, every job", output)

    def test_skipped_ci_not_pass(self):
        code, output = self.run_gate(ci_jobs=[job(), job("skipped")])
        self.assertEqual(code, 1)
        self.assertIn("OPEN  CI, every job", output)

    def test_mac_off_ac_power_is_no_verdict(self):
        # A check on battery measures the efficiency cores (#202): no
        # evidence, whether it started, ended or went unrecorded so.
        for power in [("Battery Power, 20%", "Battery Power, 12%"), ("AC Power, 100%", "Battery Power, 98%"), ("AC Power, 100%", None)]:
            code, output = self.run_gate(power=power)
            self.assertEqual(code, 2, power)
            self.assertIn("NO VERDICT  Mac check", output)
            self.assertIn("run it again plugged in", output)

    def test_partial_mac_not_pass(self):
        code, output = self.run_gate(cells=None)
        self.assertEqual(code, 1)
        self.assertIn("OPEN  Mac check", output)

    def test_non_check_mac_not_pass(self):
        self.assertEqual(self.run_gate(kind="benchmark")[0], 1)

    def test_wrong_commits_not_pass(self):
        self.assertEqual(self.run_gate(new="c" * 40)[0], 1)
        self.assertEqual(self.run_gate(old="c" * 40)[0], 1)

    def test_vm_verdict_needs_the_right_comparison_and_full_scope(self):
        good = f"check: BLAKE3-Servil's working tree ({HEAD[:12]}) against {OLD} ({OLD[:12]})\n"
        for header in ("", good.replace(OLD[:12], "c" * 12), good.replace(HEAD[:12], "d" * 12), good + "check: measuring the 7 named points\n"):
            with self.subTest(header=header):
                code, output = self.run_gate(vm_log=header + f"regress: no cell slower\n{check.VM_OK}")
                self.assertEqual(code, 1, output)
                self.assertIn("FAIL  VM check against the previous release", output)

    def test_a_first_stage_vm_reading_is_not_a_verdict(self):
        start = f"check: BLAKE3-Servil's working tree ({HEAD[:12]}) against {OLD} ({OLD[:12]})\n"
        text = start + "regress: no cell slower\nregress: suspects: a cell to repeat"
        code, output = self.run_gate(vm_log=text)
        self.assertEqual(code, 1, output)
        self.assertIn("FAIL  VM check against the previous release", output)

    def test_mutation_timeout_mismatch_is_no_verdict(self):
        text = f"mutants: BLAKE3-Servil at {HEAD}\nmutants: the whole run\nmutants: 12 survive, each known and explained\nmutants: the time-outs differ from timeouts-x86_64.txt (no verdict on those mutants)\n"
        code, output = self.run_gate(mutation_log=text)
        self.assertEqual(code, 2, output)
        self.assertIn("NO VERDICT  full mutation run", output)

    def test_mutation_survivor_failure_is_failed(self):
        text = f"mutants: BLAKE3-Servil at {HEAD}\nmutants: the whole run\nmutants: the list and this run differ, or a listed mutant has no reason\nmutants: 3 time out, each known (timeouts-x86_64.txt)\n"
        code, output = self.run_gate(mutation_log=text)
        self.assertEqual(code, 1, output)
        self.assertIn("FAIL  full mutation run", output)

    def test_a_mutation_log_must_name_the_whole_run(self):
        # check.py's first line names the run's scope; its default covers the change since
        # origin/next, and the gate wants the whole run (--whole).
        tail = MUTATION_OK.split("\n", 2)[2]
        for text in ["mutants: the change since 0123456789ab\n" + tail, tail]:
            with self.subTest(text=text):
                code, output = self.run_gate(mutation_log=text)
                self.assertEqual(code, 1, output)
                self.assertIn("FAIL  full mutation run", output)

    def test_incomplete_mutation_log_is_open(self):
        for text in [None, "", f"mutants: BLAKE3-Servil at {HEAD}\nmutants: the whole run\n", MUTATION_OK + "interrupted\n"]:
            with self.subTest(text=text):
                code, output = self.run_gate(mutation_log=text)
                self.assertEqual(code, 1, output)
                self.assertIn("OPEN  full mutation run", output)

    def test_core_failure_cannot_hide_behind_thread_success(self):
        text = (f"mutants: BLAKE3-Servil at {HEAD}\nmutants: the whole run\n"
                "mutants: the list and this run differ, or a listed mutant has no reason\n"
                "mutants: 0 time out, each known (timeouts-x86_64.txt)\n")
        code, output = self.run_gate(core_log=text)
        self.assertEqual(code, 1, output)
        self.assertIn("FAIL  full mutation run, core and threads", output)

    def test_both_crates_name_the_clean_candidate(self):
        for bad in (MUTATION_OK.replace(HEAD, OLD), MUTATION_OK.replace(HEAD, HEAD + ", with uncommitted changes")):
            for crate in ("core_log", "mutation_log"):
                with self.subTest(crate=crate, log=bad):
                    code, output = self.run_gate(**{crate: bad})
                    self.assertEqual(code, 1, output)
                    self.assertIn("FAIL  full mutation run", output)

    def test_single_crate_and_truncated_combined_logs_stay_open(self):
        combined = ("mutants: the crate at the root\n" + MUTATION_OK
                    + "mutants: the crate in threads/\n" + MUTATION_OK
                    + "mutants: 2 crates took 99 s in all\n")
        with tempfile.TemporaryDirectory() as tmp, patch.object(check, "MAIN", tmp):
            path = Path(tmp) / "mutants.log"
            path.write_text(combined)
            self.assertIs(check.mutation_result("mutants.log", HEAD), True)
            for text in (MUTATION_OK, combined.rsplit("\n", 2)[0], combined + "interrupted\n",
                         combined.replace("mutants: the crate in threads/", "mutants: the crate at the root")):
                path.write_text(text)
                self.assertIsNone(check.mutation_result("mutants.log", HEAD), text)

    def test_mac_undecided_outcomes_remain_distinct(self):
        for verdict, label in [("timed out", "TIMED OUT"), ("no verdict", "NO VERDICT")]:
            with self.subTest(verdict=verdict):
                code, output = self.run_gate(verdict=verdict)
                self.assertEqual(code, 2)
                self.assertIn(label + "  Mac check", output)


class Artifacts(unittest.TestCase):
    """The artifacts step on a real history: what it asks of the commit
    and of this checkout's dist/ (the artifacts' own checks have
    tools/test_release_artifacts.py)."""

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.repo = Path(temporary.name)
        self.git("init", "-q", "--initial-branch=main")
        for key, value in [("user.name", "A Test"), ("user.email", "test@example.invalid"), ("commit.gpgsign", "false"), ("core.hooksPath", str(self.repo / "no-hooks"))]:
            self.git("config", key, value)
        (self.repo / "Cargo.toml").write_text('[package]\nname = "top-crate"\nversion = "0.3.0+dev"\n')
        self.git("add", ".")
        self.git("commit", "-qm", "the code")
        self.code = self.git("rev-parse", "HEAD")
        (self.repo / "Cargo.toml").write_text(f'[package]\nname = "top-crate"\nversion = "0.3.0+{self.code}"\n')
        self.git("commit", "-qam", "the version")
        self.commit = self.git("rev-parse", "HEAD")

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.repo), *args], text=True, stderr=subprocess.PIPE).strip()

    def result(self, commit, ci_built=None, ci_crate=None):
        """`ci_built`: the bytes of the program the commit's CI run built,
        where that run has completed; `ci_crate`: of the crate it made."""
        def ci_builds(run, into):
            (Path(into) / f"b3sum-v0.3.0+{self.code}-x86_64-unknown-linux-gnu").write_bytes(ci_built)
            if ci_crate is not None:
                (Path(into) / f"top-crate-0.3.0+{self.code}.crate").write_bytes(ci_crate)

        with patch.object(check, "ROOT", str(self.repo)), patch.object(check, "git", self.git), patch.object(check, "ci_builds", ci_builds), \
             patch.object(check.release_artifacts, "CRATE_DIRS", ("",)), patch.object(check.release_artifacts, "B3SUM_TARGETS", ("x86_64-unknown-linux-gnu",)):
            return check.artifacts_result(commit, None if ci_built is None else {"id": 7})

    def test_a_commit_that_is_no_version_commit_fails(self):
        ok, why = self.result(self.code)
        self.assertIs(ok, False)
        self.assertIn("v0.3.0+dev is no release tag", why)

    def test_nothing_gathered_is_open_and_names_what_and_where(self):
        ok, why = self.result(self.commit)
        self.assertIsNone(ok)
        self.assertIn(f"not in {self.repo / 'dist'} yet: b3sum-v0.3.0+{self.code}-x86_64-unknown-linux-gnu, top-crate-0.3.0+{self.code}.crate", why)

    def test_a_dirty_program_in_dist_fails_and_a_clean_one_is_not_enough_alone(self):
        (self.repo / "dist").mkdir()
        program = self.repo / "dist" / f"b3sum-v0.3.0+{self.code}-x86_64-unknown-linux-gnu"
        program.write_bytes(f"b3sums 0.3.0+{self.code}.{self.commit}.dirty".encode())
        ok, why = self.result(self.commit)
        self.assertIs(ok, False)
        self.assertIn("built at the version commit from a dirty tree", why)
        program.write_bytes(f"b3sums 0.3.0+{self.code}\0".encode())
        ok, why = self.result(self.commit)
        self.assertIsNone(ok, "the crate's archive is not there yet")
        self.assertIn(f"top-crate-0.3.0+{self.code}.crate", why)


    def test_a_clean_program_must_also_be_the_one_ci_built(self):
        (self.repo / "dist").mkdir()
        clean = f"b3sums 0.3.0+{self.code}\0".encode()
        (self.repo / "dist" / f"b3sum-v0.3.0+{self.code}-x86_64-unknown-linux-gnu").write_bytes(clean)
        # Other code under the release's version: its own word is clean.
        ok, why = self.result(self.commit, ci_built=clean + b"other code")
        self.assertIs(ok, False)
        self.assertIn("are not the second build's in CI's run", why)
        ok, why = self.result(self.commit, ci_built=clean)
        self.assertIsNone(ok, "the same bytes: only the crate's archive is not there yet")
        self.assertIn("not in", why)

    def test_without_cis_builds_clean_artifacts_stay_open(self):
        (self.repo / "dist").mkdir()
        (self.repo / "dist" / f"b3sum-v0.3.0+{self.code}-x86_64-unknown-linux-gnu").write_bytes(f"b3sums 0.3.0+{self.code}\0".encode())
        crate = io.BytesIO()
        with tarfile.open(fileobj=crate, mode="w:gz") as archive:
            for name, data in ((".cargo_vcs_info.json", json.dumps({"git": {"sha1": self.commit}, "path_in_vcs": ""}).encode()),
                               ("Cargo.toml.orig", (self.repo / "Cargo.toml").read_bytes()), ("Cargo.toml", b"cargo's")):
                info = tarfile.TarInfo(f"top-crate-0.3.0+{self.code}/{name}")
                info.size = len(data)
                archive.addfile(info, io.BytesIO(data))
        (self.repo / "dist" / f"top-crate-0.3.0+{self.code}.crate").write_bytes(crate.getvalue())
        ok, why = self.result(self.commit)
        self.assertIsNone(ok)
        self.assertIn("CI's builds, which each b3sum program and crate must equal byte for byte, are not there yet", why)
        clean = f"b3sums 0.3.0+{self.code}\0".encode()
        ok, why = self.result(self.commit, ci_built=clean, ci_crate=crate.getvalue())
        self.assertIs(ok, True, why)
        # A crate whose files are all the commit's, and whose archive is
        # not the one CI packaged: Cargo's own files in it may differ.
        ok, why = self.result(self.commit, ci_built=clean, ci_crate=crate.getvalue() + b"\0")
        self.assertIs(ok, False)
        self.assertIn(f"top-crate-0.3.0+{self.code}.crate: its bytes", why)
        ok, why = self.result(self.commit, ci_built=clean)
        self.assertIs(ok, False)
        self.assertIn("no second build of it in CI's run to compare with (sh tools/crates-release.sh", why)


class PreviousRelease(unittest.TestCase):
    """Real Git histories, with synthetic CI/Mac/log evidence for main()."""
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.repo = Path(temporary.name)
        self.date = "2020-01-01T00:00:00+00:00"
        self.git("init", "-q", "--initial-branch=main")
        self.git("config", "user.name", "Sam Shackleton")
        self.git("config", "user.email", "samshackleton-tech@users.noreply.github.com")
        self.git("config", "commit.gpgsign", "false")
        self.git("config", "core.hooksPath", str(self.repo / "no-hooks"))
        for page in ["README.md", "QUALITY.md", "CHANGELOG.md"]:
            (self.repo / page).write_text("old\n")
        self.git("add", ".")
        self.git("commit", "-qm", "old release")
        self.old = self.git("rev-parse", "HEAD")
        self.git("tag", "v0.3.0+fixture")
        self.date = "2020-01-02T00:00:00+00:00"
        for page in ["README.md", "QUALITY.md", "CHANGELOG.md"]:
            (self.repo / page).write_text("new\n")
        self.git("commit", "-qam", "candidate")
        self.new = self.git("rev-parse", "HEAD")
        self.git("branch", "candidate")

    def git(self, *args):
        env = dict(os.environ, GIT_AUTHOR_DATE=self.date, GIT_COMMITTER_DATE=self.date)
        return subprocess.check_output(["git", "-C", str(self.repo), *args], text=True, env=env, stderr=subprocess.PIPE).strip()

    def test_tagged_candidate_uses_previous_release(self):
        self.git("tag", "-a", "v0.8.0+fixture", "-m", "rehearsal tag")
        code, output = ReleaseEvidence.run_gate(self, new=self.new, old=self.old, git_override=self.git)
        self.assertEqual(code, 0, output)
        self.assertIn("rewritten since v0.3.0+fixture", output)

    def test_unrelated_recent_tag_cannot_set_the_baseline(self):
        self.git("checkout", "-qb", "unrelated", self.old)
        self.date = "2020-01-03T00:00:00+00:00"
        (self.repo / "side.txt").write_text("another branch\n")
        self.git("add", ".")
        self.git("commit", "-qm", "unrelated work")
        self.git("tag", "v99.0.0+fixture")
        self.git("checkout", "-q", "candidate")
        code, output = ReleaseEvidence.run_gate(self, new=self.new, old=self.old, git_override=self.git)
        self.assertEqual(code, 0, output)

    def test_no_reachable_release_is_open(self):
        self.git("tag", "-d", "v0.3.0+fixture")
        self.git("tag", "v0.8.0+fixture")
        code, output = ReleaseEvidence.run_gate(self, new=self.new, old=self.old, git_override=self.git)
        self.assertEqual(code, 1)
        self.assertIn("no previous release: read README.md", output)

    def test_an_explicit_patch_tag_can_be_on_a_separate_line(self):
        self.git("checkout", "-qb", "patch-line", self.old)
        manifest = self.repo / "Cargo.toml"
        manifest.write_text('[package]\nversion = "0.3.1+dev"\n')
        self.git("add", "Cargo.toml")
        self.git("commit", "-qm", "patch code")
        version = "0.3.1+" + self.git("rev-parse", "HEAD")
        manifest.write_text(f'[package]\nversion = "{version}"\n')
        self.git("commit", "-qam", "stamp patch")
        tag = "v" + version
        self.git("tag", "-a", tag, "-m", "patch release")
        patch_commit = self.git("rev-parse", "HEAD")
        self.git("checkout", "-q", "candidate")
        manifest.write_text('[package]\nversion = "0.4.0+dev"\n')
        self.git("add", "Cargo.toml")
        self.git("commit", "-qm", "candidate version")
        head = self.git("rev-parse", "HEAD")

        with patch.object(check, "ROOT", str(self.repo)):
            code, output = ReleaseEvidence.run_gate(self, new=head, old=patch_commit, git_override=self.git, previous=tag)
            self.assertEqual(code, 0, output)
            self.assertIn(f"rewritten since {tag}", output)
            with patch.object(check, "git", self.git):
                self.assertEqual(check.previous_release(head), "v0.3.0+fixture")
                with self.assertRaisesRegex(check.release_artifacts.NotARelease, "no local tag candidate"):
                    check.named_previous(head, "candidate")
                with self.assertRaisesRegex(check.release_artifacts.NotARelease, "not older"):
                    check.named_previous(patch_commit, tag)

    def test_release_on_second_parent_is_reachable(self):
        self.git("checkout", "-qb", "release-side", self.old)
        self.date = "2020-01-03T00:00:00+00:00"
        (self.repo / "side.txt").write_text("previous release\n")
        self.git("add", ".")
        self.git("commit", "-qm", "previous release on another parent")
        self.git("tag", "v0.4.0+fixture")
        self.git("checkout", "-q", "candidate")
        self.git("merge", "--no-ff", "-qm", "merge release", "release-side")
        head = self.git("rev-parse", "HEAD")
        with patch.object(check, "git", self.git):
            self.assertEqual(check.previous_release(head), "v0.4.0+fixture")


if __name__ == "__main__":
    unittest.main()
