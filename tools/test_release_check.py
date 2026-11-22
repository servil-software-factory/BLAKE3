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
import warnings
import zipfile
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("release_check", Path(__file__).with_name("release-check.py"))
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)

HEAD, OLD = "a" * 40, "b" * 40
# A verdict's field left out, where None would write JSON's null.
ABSENT = object()
RUN = {"id": 7, "run_number": 7, "name": "tests", "path": ".github/workflows/ci.yml", "event": "workflow_dispatch", "status": "completed", "conclusion": "success"}
MUTATION_OK = f"mutants: BLAKE3-Servil at {HEAD}\nmutants: the whole run\nmutants: 12 survive, each known and explained\nmutants: 3 time out, each known (timeouts-x86_64.txt)\n"


def job(conclusion="success", status="completed"):
    return {"name": "test job", "status": status, "conclusion": conclusion}


def resolve_fixture(spelling, commits=(HEAD, OLD)):
    """`git rev-parse --verify --quiet SPELLING` over `commits`: the one
    that begins so, or git's failure where none or several do."""
    found = [c for c in commits if c.startswith(spelling.removesuffix("^{commit}"))]
    if len(found) != 1:
        raise subprocess.CalledProcessError(1, ["git", "rev-parse", spelling])
    return found[0]


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
    def run_gate(self, cells="all", kind="check", ci_jobs=None, new=HEAD, old=OLD, verdict="no regression", git_override=None, ci_event="workflow_dispatch", mutation_log=MUTATION_OK, core_log=MUTATION_OK, power=("AC Power, 100%", "AC Power, 100%"), artifacts=(True, ""), previous=None, builds_log="check-builds: every build passes", dry_run_log="publish-dry-run: ok", mac_results=None, inspections=ABSENT):
        jobs = [job(), job()] if ci_jobs is None else ci_jobs
        def git(*args):
            if args[:3] == ("rev-parse", "--verify", "--quiet"):
                return resolve_fixture(args[3])
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
            task = {"type": kind, "old_commit": old, "new_commit": new}
            if cells is not None:
                task["cells"] = cells
            readings = {k: v for k, v in zip(("power at start", "power at end"), power) if v is not None}
            if inspections is not ABSENT:
                readings["inspections"] = inspections
            # A results folder is NAME.TIME-MACHINE, and its job is
            # jobs/NAME.json; `mac_results` gives others than the one
            # check on the timing Mac, each the same task under its name.
            for folder, status in mac_results or [("job.20261120T135633Z-mac-m6-timing", verdict)]:
                name = folder.split(".")[0]
                (root / "mac-runner/jobs" / (name + ".json")).write_text(json.dumps(task))
                (root / "mac-runner/results" / folder).mkdir(parents=True)
                (root / "mac-runner/results" / folder / "verdict.json").write_text(json.dumps({"job": name + ".json", "status": status, **readings}))
            logs = root / "tmp/night"
            logs.mkdir(parents=True)
            candidate = (git_override or git)("rev-parse", "candidate")[:12]
            for name, text in [("check-builds", builds_log), ("publish-dry-run", dry_run_log)]:
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

    def test_a_log_passes_by_its_last_line_alone(self):
        """A success line with a later run's failure under it is no pass
        (the tail was searched for the line, and held it), nor is the
        line as part of another."""
        marker = "check-builds: every build passes"
        for log in ("check-builds: every build passes\nerror: a later run failed", "the note says: check-builds: every build passes", "every build passes", "", marker + "\t", *("Note: " + marker + "\n" * n for n in range(3996 - len(marker), 4003 - len(marker)))):
            code, output = self.run_gate(builds_log=log)
            self.assertEqual(code, 1, log)
            self.assertIn("FAIL  check-builds", output, log)
        for log in ("publish-dry-run: ok\nerror: failed to verify package tarball", "publish-dry-run: okay", "not publish-dry-run: ok"):
            code, output = self.run_gate(dry_run_log=log)
            self.assertEqual(code, 1, log)
            self.assertIn("FAIL  cargo publish --dry-run", output, log)
        # Lines above the last, and blank lines after it, change nothing.
        self.assertEqual(self.run_gate(builds_log="error: an earlier run failed\ncheck-builds: every build passes\n\n")[0], 0)

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

    def test_another_macs_check_is_not_the_timing_macs(self):
        # The exchange serves other Macs. 0.10.0's step read an Apple M4's
        # check of the same commits, the last by name, and failed where
        # the timing Mac's had passed. Alone, another Mac's check leaves
        # the step open, whatever it says.
        m6, m4 = "3547-check.20261120T135633Z-mac-m6-timing", "3564-check.20261121T032909Z-mac-m4-timing"
        code, output = self.run_gate(mac_results=[(m6, "no regression"), (m4, "regression")])
        self.assertEqual(code, 0, output)
        code, output = self.run_gate(mac_results=[(m6, "regression"), (m4, "no regression")])
        self.assertEqual(code, 1)
        self.assertIn("FAIL  Mac check", output)
        for status in ["no regression", "regression"]:
            code, output = self.run_gate(mac_results=[(m4, status), ("9999-check.20261121T040000Z-mac-m6-build", status), ("1", status)])
            self.assertEqual(code, 1, status)
            self.assertIn("OPEN  Mac check", output)
            self.assertIn("on mac-m6-timing", output)

    def test_the_timing_macs_latest_check_counts(self):
        # By the time in the folder's name: a job's name may begin with a
        # number or with a date, so the names' order is not the jobs'.
        early, late = "3547-check.20261120T135633Z-mac-m6-timing", "20261122T020016Z-check.20261122T020027Z-mac-m6-timing"
        self.assertLess(late, early, "the later job's name sorts first")
        for results in ([(early, "regression"), (late, "no regression")], [(late, "no regression"), (early, "regression")]):
            code, output = self.run_gate(mac_results=results)
            self.assertEqual(code, 0, output)
        for results in ([(early, "no regression"), (late, "regression")], [(late, "regression"), (early, "no regression")]):
            code, output = self.run_gate(mac_results=results)
            self.assertEqual(code, 1)
            self.assertIn("FAIL  Mac check", output)

    def test_partial_mac_not_pass(self):
        code, output = self.run_gate(cells=None)
        self.assertEqual(code, 1)
        self.assertIn("OPEN  Mac check", output)

    def test_non_check_mac_not_pass(self):
        self.assertEqual(self.run_gate(kind="benchmark")[0], 1)

    def test_wrong_commits_not_pass(self):
        self.assertEqual(self.run_gate(new="c" * 40)[0], 1)
        self.assertEqual(self.run_gate(old="c" * 40)[0], 1)

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
        for verdict, label in [("timed out", "TIMED OUT"), ("no verdict", "NO VERDICT"), ("cancelled", "NO VERDICT")]:
            with self.subTest(verdict=verdict):
                code, output = self.run_gate(verdict=verdict)
                self.assertEqual(code, 2)
                self.assertIn(label + "  Mac check", output)
                self.assertIn(f"(the last one: {verdict})", output)

    def test_an_inspected_mac_check_is_no_verdict(self):
        # The runner's inspection samples the job's processes, which
        # suspends them while they are timed (BLAKE3 #464): the check's
        # "no regression" is then no evidence, and neither is its
        # "regression". An empty list is a check nobody inspected.
        once = [{"at": "2026-11-22T09:00:00Z", "duration_seconds": 1.25}]
        for verdict in ["no regression", "regression"]:
            for inspections, times in [(once, "1 time:"), (once * 3, "3 times:"), ([None], "1 time:")]:
                with self.subTest(verdict=verdict, inspections=inspections):
                    code, output = self.run_gate(verdict=verdict, inspections=inspections)
                    self.assertEqual(code, 2, output)
                    self.assertIn("NO VERDICT  Mac check", output)
                    self.assertIn("inspected while it ran, " + times, output)
        code, output = self.run_gate(inspections=[])
        self.assertEqual(code, 0, output)
        self.assertEqual(self.run_gate(verdict="regression", inspections=[])[0], 1)

    def test_inspections_that_are_no_list_pass_nothing(self):
        # Absent is a check nobody inspected. Any value that is no list,
        # the empty ones too (null, false, 0, "", {}), is a record this
        # reader does not understand: a passing check with
        # "inspections": null once passed the step.
        for inspections in [None, False, 0, 0.0, "", {}, "yes", 3, {"at": "09:00"}, True]:
            for verdict in ["no regression", "regression"]:
                with self.subTest(inspections=inspections, verdict=verdict):
                    code, output = self.run_gate(verdict=verdict, inspections=inspections)
                    self.assertEqual(code, 2, output)
                    self.assertIn("NO VERDICT  Mac check", output)
                    self.assertIn(f'"inspections" is {type(inspections).__name__}, not a list', output)

    def test_a_timed_out_mac_check_is_timed_out_whatever_else(self):
        # The runner's limit ended the job: that is the step's outcome,
        # inspected or off AC power or both, and the line says each.
        once = [{"at": "2026-11-22T09:00:00Z", "duration_seconds": 1.25}]
        code, output = self.run_gate(verdict="timed out", inspections=once, power=("AC Power, 100%", "Battery Power, 98%"))
        self.assertEqual(code, 2, output)
        self.assertIn("TIMED OUT  Mac check", output)
        for text in ["(the last one: timed out)", "inspected while it ran, 1 time:", "run it again plugged in"]:
            self.assertIn(text, output)
        # The runner's other statuses decide nothing once inspected, an
        # error among them; uninspected, an error fails.
        for verdict in ["error", "a side stopped", "no verdict", "cancelled"]:
            code, output = self.run_gate(verdict=verdict, inspections=once)
            self.assertEqual(code, 2, output)
            self.assertIn("NO VERDICT  Mac check", output)
            self.assertIn(f"(the last one: {verdict})", output)
        code, output = self.run_gate(verdict="error")
        self.assertEqual(code, 1, output)
        self.assertIn("FAIL  Mac check", output)
        self.assertIn("(the last one: error)", output)


class MacCheck(unittest.TestCase):
    """`mac_check` on an exchange of several records, each with its own
    job: which record decides, and what an unusable one does. (A review
    of this reader found each of these; the step had passed on an older
    record where the latest was spelled otherwise, misattributed, tied,
    or damaged.)"""

    T1, T2, T3 = "20261120T135633Z", "20261121T020027Z", "20261122T020016Z"

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / "jobs").mkdir()
        (self.root / "results").mkdir()
        self.commits = [HEAD, OLD]

    def record(self, name, time, status="no regression", machine="mac-m6-timing", new=HEAD, old=OLD, job_file=ABSENT, verdict_file=ABSENT, claim_file=ABSENT, **fields):
        """One job and its results folder. `job_file`, `verdict_file` and
        `claim_file` are a file's whole text (None: no such file);
        `fields` go into the verdict, and one that is ABSENT is left out."""
        task = json.dumps({"type": "check", "cells": "all", "old_commit": old, "new_commit": new}) if job_file is ABSENT else job_file
        if task is not None:
            (self.root / "jobs" / (name + ".json")).write_text(task)
        folder = self.root / "results" / f"{name}.{time}-{machine}"
        folder.mkdir()
        if verdict_file is ABSENT:
            v = {"job": name + ".json", "status": status, "power at start": "AC Power, lowpowermode 0", "power at end": "AC Power, lowpowermode 0", **fields}
            verdict_file = json.dumps({k: value for k, value in v.items() if value is not ABSENT})
        if verdict_file is not None:
            (folder / "verdict.json").write_text(verdict_file)
        if claim_file is not ABSENT:
            (folder / "claimed.json").write_text(claim_file)
        return folder.name

    def check(self, old=OLD):
        def git(*args):
            assert args[:3] == ("rev-parse", "--verify", "--quiet"), args
            return resolve_fixture(args[3], self.commits)
        with patch.object(check, "git", git):
            return check.mac_check(str(self.root), HEAD, old)

    def test_no_record_is_open_and_one_pass_passes(self):
        self.assertEqual(self.check(), (None, ""))
        self.record("3547-check", self.T1)
        self.assertEqual(self.check(), (True, ""))

    def test_a_commit_named_by_its_beginning_is_that_commit(self):
        # The runner takes 7 to 40 hex digits. A later check that spelled
        # the candidate with twelve was passed over for an older pass.
        self.record("first", self.T1)
        for field in ["new", "old"]:
            for digits in [7, 12, 39]:
                with self.subTest(field=field, digits=digits):
                    name = f"later-{field}-{digits}"
                    spelled = {field: (HEAD if field == "new" else OLD)[:digits]}
                    self.record(name, self.T2, "cancelled", **spelled)
                    outcome, note = self.check()
                    self.assertEqual(outcome, check.NO_VERDICT, note)
                    self.assertIn("(the last one: cancelled)", note)
                    self.record(name + "-again", self.T3, "regression", **spelled)
                    self.assertIs(self.check()[0], False)
                    for made in list((self.root / "results").glob("later-*")):
                        for file in made.iterdir():
                            file.unlink()
                        made.rmdir()
        # Not this commit: six digits or capitals (the runner refuses
        # both), another commit's beginning, a value that is no text.
        for i, spelling in enumerate([HEAD[:6], HEAD[:12].upper(), "c" * 12, "a" * 11 + "c", None, 7, ""]):
            self.record(f"other-{i}", self.T3, "regression", new=spelling)
        self.assertEqual(self.check(), (True, ""))

    def test_a_beginning_that_other_objects_share_decides_nothing(self):
        self.record("first", self.T1)
        self.commits.append("a" * 12 + "c" * 28)
        self.record("later", self.T2, "no regression", new="a" * 12)
        outcome, note = self.check()
        self.assertEqual(outcome, check.NO_VERDICT, note)
        self.assertIn("later." + self.T2, note)
        self.assertIn("a beginning that other objects here share", note)

    def test_a_folders_job_is_the_one_its_name_gives(self):
        # The later folder is another commit's check, and its verdict
        # names the first job by mistake: the candidate's own check, the
        # inspected one, still decides. (The step took the job from the
        # verdict, and passed.)
        self.record("first", self.T1, inspections=[{"at": "09:00"}])
        self.record("later", self.T2, new="c" * 40, job="first.json")
        outcome, note = self.check()
        self.assertEqual(outcome, check.NO_VERDICT, note)
        self.assertIn("inspected while it ran, 1 time", note)
        # And a later check of these commits whose verdict names another
        # job is not a verdict on it.
        named = self.record("named-otherwise", self.T3, job="first.json")
        outcome, note = self.check()
        self.assertEqual(outcome, check.NO_VERDICT, note)
        self.assertIn(f"the last one, {named}, decides nothing: its verdict names the job 'first.json', not named-otherwise.json", note)

    def test_a_later_folder_whose_job_cannot_be_read_holds_the_step(self):
        self.record("early-and-jobless", self.T1, job_file=None)
        self.record("the-check", self.T2)
        self.assertEqual(self.check(), (True, ""), "an earlier folder without a job decides nothing")
        for i, (text, why) in enumerate([(None, "cannot be read"), ("{", "is not JSON"), ("[]", "is not a JSON object"), ("not text", "is not JSON")]):
            with self.subTest(job_file=text):
                later = self.record(f"jobless-{i}", self.T3, job_file=text)
                if text == "not text":
                    (self.root / "jobs" / f"jobless-{i}.json").write_bytes(b"\xff\xfe")
                outcome, note = self.check()
                self.assertEqual(outcome, check.NO_VERDICT, note)
                self.assertIn(later, note)
                self.assertIn(why, note)
                (self.root / "results" / later / "verdict.json").unlink()
                (self.root / "results" / later).rmdir()
        # In the same hand-over as the check: also held.
        self.record("jobless-with-it", self.T2, job_file=None)
        self.assertEqual(self.check()[0], check.NO_VERDICT)

    def test_the_latest_decides_and_a_tie_decides_nothing(self):
        # Jobs handed over together share the folder's time; the Mac
        # runs them in turn, and the verdict's "started" orders them.
        for first, second in [("regression", "no regression"), ("no regression", "regression")]:
            with self.subTest(first=first), tempfile.TemporaryDirectory() as tmp:
                self.root = Path(tmp)
                (self.root / "jobs").mkdir()
                (self.root / "results").mkdir()
                self.record("zz-ran-first", self.T2, first, started="2026-11-21T02:00:30Z")
                self.record("aa-ran-second", self.T2, second, started="2026-11-21T02:31:00Z")
                self.assertIs(self.check()[0], second == "no regression")
                # Without the times: no order, whatever the two say.
                self.record("mm-third", self.T3, first)
                self.record("nn-fourth", self.T3, second)
                outcome, note = self.check()
                self.assertEqual(outcome, check.NO_VERDICT, note)
                self.assertIn("cannot be put in order", note)
                self.assertIn("mm-third", note)
                self.assertIn("nn-fourth", note)

    def test_a_check_without_a_verdict_yet_is_waited_for(self):
        self.record("the-check", self.T2, started="2026-11-21T02:00:30Z")
        self.record("abandoned-before", self.T1, verdict_file=None, claim_file='{"job": "abandoned-before.json", "machine": "mac-m6-timing"}')
        self.assertEqual(self.check(), (True, ""))
        running = self.record("running", self.T3, verdict_file=None, claim_file='{"job": "running.json", "machine": "mac-m6-timing"}')
        outcome, note = self.check()
        self.assertEqual(outcome, check.NO_VERDICT, note)
        self.assertIn(f"the last one, {running}, decides nothing: it has no verdict yet", note)
        # Handed over with a check that has finished: it runs after it.
        (self.root / "results" / running).rename(self.root / "results" / f"running.{self.T2}-mac-m6-timing")
        outcome, note = self.check()
        self.assertEqual(outcome, check.NO_VERDICT, note)
        self.assertIn("it has no verdict yet", note)

    def test_a_damaged_latest_record_decides_nothing_and_an_older_pass_does_not_stand_in(self):
        self.record("first", self.T1)
        cases = [
            (dict(verdict_file="{\"job\": \"x.json\", \"status\": \"no regr"), "its verdict.json is not JSON"),
            (dict(verdict_file="[]"), "its verdict.json is not a JSON object"),
            (dict(status=None), "its status is None, not a text"),
            (dict(status=ABSENT), "its status is None, not a text"),
            (dict(status=""), "its status is '', not a text"),
            (dict(status=["no regression"]), "not a text"),
            (dict(claim_file="{"), "its claimed.json is not JSON"),
            (dict(claim_file='{"job": "first.json", "machine": "mac-m6-timing"}'), "its claimed.json names 'first.json'"),
            (dict(claim_file='{"job": "damaged-8.json", "machine": "mac-m4-timing"}'), "on 'mac-m4-timing'"),
        ]
        for i, (fields, why) in enumerate(cases):
            with self.subTest(fields=fields):
                later = self.record(f"damaged-{i}", self.T2, **fields)
                outcome, note = self.check()
                self.assertEqual(outcome, check.NO_VERDICT, note)
                self.assertIn(f"the last one, {later}, decides nothing", note)
                self.assertIn(why, note)
                for file in (self.root / "results" / later).iterdir():
                    file.unlink()
                (self.root / "results" / later).rmdir()
        # Power readings of the wrong type or the wrong word are off AC.
        for i, reading in enumerate([None, 5, ["AC Power"], "AC Powerless", "", "ac power", ABSENT]):
            with self.subTest(reading=reading):
                later = self.record(f"power-{i}", self.T2, **{"power at end": reading})
                outcome, note = self.check()
                self.assertEqual(outcome, check.NO_VERDICT, note)
                self.assertIn("ran off AC power: " + (reading if isinstance(reading, str) and reading else "unrecorded"), note)
                for file in (self.root / "results" / later).iterdir():
                    file.unlink()
                (self.root / "results" / later).rmdir()
        self.assertEqual(self.check(), (True, ""))

    def test_damage_elsewhere_is_never_read(self):
        # Older than the check, on another Mac, or another job's: such a
        # record is not opened, so its damage cannot stop the checklist
        # (a truncated verdict of an unrelated job once raised from it).
        self.record("older-damaged", self.T1, verdict_file="{")
        self.record("m4-damaged", self.T3, verdict_file="{", machine="mac-m4-timing")
        self.record("benchmark-damaged", self.T3, verdict_file="{", job_file='{"type": "benchmark"}')
        self.record("partial-damaged", self.T3, verdict_file="{", job_file=json.dumps({"type": "check", "cells": "small", "old_commit": OLD, "new_commit": HEAD}))
        self.record("another-commit-damaged", self.T3, verdict_file="{", new="c" * 40)
        self.record("the-check", self.T2)
        self.assertEqual(self.check(), (True, ""))
        (self.root / "results" / "not-a-results-folder").mkdir()
        (self.root / "results" / "a-file").write_text("x")
        self.assertEqual(self.check(), (True, ""))

    def test_without_a_previous_release_any_old_side_counts(self):
        self.record("first-release", self.T1, old="c" * 40)
        self.assertEqual(self.check(old=None), (True, ""))
        self.assertEqual(self.check(), (None, ""))


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

    def test_ci_builders_that_disagree_fail_in_either_order(self):
        # Both Linux jobs package the crates. The later zip's copy replaced
        # the earlier one's unread, so the verdict went by download order.
        crate = f"top-crate-0.3.0+{self.code}.crate"

        def zipped(files):
            archive = io.BytesIO()
            with zipfile.ZipFile(archive, "w") as z:
                for name, data in files.items():
                    z.writestr(name, data)
            return archive.getvalue()

        one = zipped({f"dist/{crate}": b"the commit's crate", "dist/b3sum-x86": b"a program", "dist/b3sum-x86.sha256": b"a digest", "dist/notes.txt": b"aside"})
        same = zipped({f"dist/{crate}": b"the commit's crate", "dist/b3sum-arm": b"another program"})
        other = zipped({f"dist/{crate}": b"another crate", "dist/b3sum-arm": b"another program"})
        for archives in ([("b3sum-x86", one), ("b3sum-arm", other)], [("b3sum-arm", other), ("b3sum-x86", one)]):
            with self.subTest(first=archives[0][0]), tempfile.TemporaryDirectory() as into:
                differ = check.unpack_builds(archives, into)
                self.assertEqual(len(differ), 1, differ)
                self.assertTrue(differ[0].startswith(crate + " (") and "b3sum-x86" in differ[0] and "b3sum-arm" in differ[0], differ)
        with tempfile.TemporaryDirectory() as into:
            self.assertEqual(check.unpack_builds([("b3sum-x86", one), ("b3sum-arm", same)], into), [])
            self.assertEqual(sorted(os.listdir(into)), sorted(["b3sum-arm", "b3sum-x86", crate]), "programs and crates, no digest file, nothing else")
            self.assertEqual((Path(into) / crate).read_bytes(), b"the commit's crate")

    def test_each_ci_builder_of_crates_holds_every_crate(self):
        # The union of two builders' files held every crate even where one
        # builder had left a crate out: that crate was then seen once, and
        # passed as if both had made it (the 0.10 fixes' own audit, #461).
        crate, second = f"top-crate-0.3.0+{self.code}.crate", f"side-crate-0.3.0+{self.code}.crate"

        def zipped(files):
            archive = io.BytesIO()
            with zipfile.ZipFile(archive, "w") as z:
                for name, data in files:
                    z.writestr(name, data)
            return archive.getvalue()

        both = zipped([(f"dist/{crate}", b"one"), (f"dist/{second}", b"two"), ("dist/b3sum-x86", b"a program")])
        lacks = zipped([(f"dist/{crate}", b"one"), ("dist/b3sum-arm", b"another program")])
        program = zipped([("dist/b3sum-mac", b"a third program")])
        twice = zipped([(f"dist/{crate}", b"one"), (f"dist/{crate}", b"one"), (f"dist/{second}", b"two"), ("dist/b3sum-arm", b"another program")])
        for archives in ([("b3sum-x86", both), ("b3sum-arm", lacks)], [("b3sum-arm", lacks), ("b3sum-x86", both)]):
            with self.subTest(first=archives[0][0]), tempfile.TemporaryDirectory() as into:
                self.assertEqual(check.unpack_builds(archives, into), [f"{second} (b3sum-x86 made it and b3sum-arm did not)"])
        with tempfile.TemporaryDirectory() as into:
            # A builder of programs alone (the Mac's) is no builder of crates.
            self.assertEqual(check.unpack_builds([("b3sum-x86", both), ("b3sum-mac", program)], into),
                             ["the crates (b3sum-x86 alone made them: no second builder bears them out)"])
        with tempfile.TemporaryDirectory() as into:
            self.assertEqual(check.unpack_builds([("b3sum-x86", both), ("b3sum-arm", both.replace(b"b3sum-x86", b"b3sum-arm")), ("b3sum-mac", program)], into), [])
        with tempfile.TemporaryDirectory() as into, warnings.catch_warnings():
            warnings.simplefilter("ignore")  # zipfile's own word about the name written twice
            self.assertEqual(check.unpack_builds([("b3sum-x86", both), ("b3sum-arm", twice)], into), [f"{crate} (b3sum-arm holds it 2 times)"])

    def test_a_disagreement_among_ci_builders_fails_the_artifacts(self):
        with patch.object(check, "ROOT", str(self.repo)), patch.object(check, "git", self.git), \
             patch.object(check, "ci_builds", lambda run, into: ["top-crate.crate (b3sum-x86 and b3sum-arm)"]), \
             patch.object(check.release_artifacts, "CRATE_DIRS", ("",)), patch.object(check.release_artifacts, "B3SUM_TARGETS", ("x86_64-unknown-linux-gnu",)):
            ok, why = check.artifacts_result(self.commit, {"id": 7})
        self.assertIs(ok, False)
        self.assertIn("CI's own builders do not bear each other out on top-crate.crate (b3sum-x86 and b3sum-arm)", why)

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
