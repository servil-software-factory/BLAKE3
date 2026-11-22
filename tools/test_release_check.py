"""Release-gate controls. All GitHub answers and release logs are fixtures."""
import contextlib
import datetime
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
    # The note that names the fixture's one record, and what it said.
    LAST = '(the last one, "job.20261120T135633Z-mac-m6-timing": {})'

    def run_gate(self, cells="all", kind="check", ci_jobs=None, new=HEAD, old=OLD, verdict="no regression", git_override=None, ci_event="workflow_dispatch", mutation_log=MUTATION_OK, core_log=MUTATION_OK, power=("AC Power, 100%", "AC Power, 100%"), artifacts=(True, ""), previous=None, builds_log="check-builds: every build passes", dry_run_log="publish-dry-run: ok", mac_results=None, inspections=ABSENT):
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
            task = {"type": kind, "old_commit": old, "new_commit": new, "bench_commit": "c" * 40, "time_limit_seconds": 7200}
            if cells is not None:
                task["cells"] = cells
            readings = {k: v for k, v in zip(("power at start", "power at end"), power) if v is not None}
            if inspections is not ABSENT:
                readings["inspections"] = inspections
            # A results folder is NAME.TIME-MACHINE, its job is
            # jobs/NAME.json, and it holds the bridge's claim and the
            # runner's verdict; `mac_results` gives others than the one
            # check on the timing Mac, each the same task under its name,
            # started when it was handed over.
            for folder, status in mac_results or [("job.20261120T135633Z-mac-m6-timing", verdict)]:
                (root / "mac-runner/results" / folder).mkdir(parents=True)
                ran = check.RESULTS_FOLDER.fullmatch(folder)
                if not ran:
                    continue
                name, t, machine = ran.groups()
                began = f"{t[:4]}-{t[4:6]}-{t[6:8]}T{t[9:11]}:{t[11:13]}:{t[13:15]}Z"
                (root / "mac-runner/jobs" / (name + ".json")).write_text(json.dumps(task))
                (root / "mac-runner/results" / folder / "claimed.json").write_text(json.dumps({"job": name + ".json", "machine": machine, "claimed": began}))
                (root / "mac-runner/results" / folder / "verdict.json").write_text(
                    json.dumps({"job": name + ".json", "started": began, "type": kind, "status": status, "finished": began, **readings}))
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
        # By its start (this fixture starts each check when it was handed
        # over): a job's name may begin with a number or with a date, so
        # the names' order is not the jobs'.
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
                self.assertIn(self.LAST.format(verdict), output)

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
        for text in [self.LAST.format("timed out"), "inspected while it ran, 1 time:", "run it again plugged in"]:
            self.assertIn(text, output)
        # The runner's other statuses decide nothing once inspected, an
        # error among them; uninspected, an error fails.
        for verdict in ["error", "a side stopped", "no verdict", "cancelled"]:
            code, output = self.run_gate(verdict=verdict, inspections=once)
            self.assertEqual(code, 2, output)
            self.assertIn("NO VERDICT  Mac check", output)
            self.assertIn(self.LAST.format(verdict), output)
        code, output = self.run_gate(verdict="error")
        self.assertEqual(code, 1, output)
        self.assertIn("FAIL  Mac check", output)
        self.assertIn(self.LAST.format("error"), output)

    def test_the_mac_steps_line_names_both_commits_in_full(self):
        # What to queue: a check that names a commit by its beginning
        # cannot pass, so the line gives both names whole.
        code, output = self.run_gate(verdict="regression")
        self.assertEqual(code, 1, output)
        self.assertIn(f"old_commit {OLD} and new_commit {HEAD}, both in full", output)


class MacCheck(unittest.TestCase):
    """`mac_check` on exchanges of several records, each as the runner and
    the bridge write it (a job, a claim, a verdict), and then with one
    thing wrong. The step passes on whole records only, by the one the
    Mac started last; three reviews found the ways it had passed, or had
    held, otherwise."""

    # When a job was handed to the Mac, the folder's time: two before the
    # day the fixtures' checks start on, one after it.
    T1, T2, AFTER = "20261120T135633Z", "20261121T020027Z", "20261122T020016Z"

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name) / "mac-runner"
        (self.root / "jobs").mkdir(parents=True)
        (self.root / "results").mkdir()

    def write(self, path, default, change):
        """`default` changed by `change`: a dict of fields (ABSENT takes
        one out), a text (the file's whole content), a function of the
        default's text, or False (no file)."""
        if change is False:
            return
        if isinstance(change, (str, bytes)):
            path.write_bytes(change.encode() if isinstance(change, str) else change)
            return
        if callable(change):
            path.write_text(change(json.dumps(default)))
            return
        fields = {**default, **(change or {})}
        path.write_text(json.dumps({key: value for key, value in fields.items() if value is not ABSENT}))

    def record(self, name, started, status="no regression", handed=None, machine="mac-m6-timing", job=None, claim=None, verdict=None, folder=True, minutes=20, file=None):
        """One job and its results folder, as written for a check that ran
        at `started` ("10:00" or "10:00:30", on November 21) for twenty
        minutes. `file` is the job's file where it is not NAME.json."""
        begun = f"2026-11-21T{started if len(started) == 8 else started + ':00'}Z"
        ended = (datetime.datetime.strptime(begun, "%Y-%m-%dT%H:%M:%SZ") + datetime.timedelta(minutes=minutes)).strftime("%Y-%m-%dT%H:%M:%SZ")
        file = file or name + ".json"
        self.write(self.root / "jobs" / file, {"type": "check", "old_commit": OLD, "new_commit": HEAD, "bench_commit": "c" * 40, "cells": "all", "time_limit_seconds": 7200}, job)
        label = f"{name}.{handed or self.T2}-{machine}"
        if folder:
            (self.root / "results" / label).mkdir()
            self.write(self.root / "results" / label / "claimed.json", {"job": file, "machine": machine, "claimed": begun, "note": "queued on the Mac by the bridge"}, claim)
            self.write(self.root / "results" / label / "verdict.json",
                       {"job": file, "started": begun, "power at start": "AC Power, lowpowermode 0", "type": "check", "status": status, "finished": ended, "power at end": "AC Power, lowpowermode 0"}, verdict)
        return label

    def remove(self, label, file=None):
        folder = self.root / "results" / label
        if folder.is_dir() and not folder.is_symlink():
            for inside in folder.iterdir():
                inside.unlink()
            folder.rmdir()
        else:
            folder.unlink()
        (self.root / "jobs" / (file or check.RESULTS_FOLDER.fullmatch(label).group(1) + ".json")).unlink(missing_ok=True)

    def check(self, old=OLD):
        return check.mac_check(str(self.root), HEAD, old)

    def last(self, label, status):
        """The note that names the record that decided and what it said."""
        return f" (the last one, {json.dumps(label)}: {status})"

    def held(self, *texts, old=OLD):
        outcome, note = self.check(old)
        self.assertEqual(outcome, check.NO_VERDICT, note)
        for text in texts:
            self.assertIn(text, note)
        self.assertNotIn("\n", note)

    def test_no_record_is_open_and_each_status_reads_as_itself(self):
        self.assertEqual(self.check(), (None, ""))
        self.record("3547-check-release-0.10.0", "10:00")
        self.assertEqual(self.check(), (True, ""), "a whole record, as the bridge and the runner write it")
        for hour, (status, outcome) in enumerate([("regression", False), ("error: the bench did not build", False), ("timed out", check.TIMED_OUT),
                                                  ("no verdict", check.NO_VERDICT), ("cancelled", check.NO_VERDICT), ("no regression", True)], 11):
            label = self.record(f"again-{hour}", f"{hour}:00", status)
            self.assertEqual(self.check(), (outcome, "" if outcome is True else self.last(label, status)))
        # A job is read whatever space surrounds it, to the size the
        # exchange publishes: one with a megabyte of spaces before it ran
        # on the Mac and was refused here.
        label = self.record("spaced", "17:00", "regression", job=lambda text: " " * (1 << 20) + text)
        self.assertEqual(self.check(), (False, self.last(label, "regression")))

    def test_the_one_started_last_decides_whatever_the_folders_say(self):
        # Handed over first and run last: a busy Mac takes the pending
        # jobs by name. And a name may begin with a number or a date.
        for early, late in [("regression", "no regression"), ("no regression", "regression")]:
            with self.subTest(late=late):
                first = self.record("20261122T020016Z-check", "10:00", early, handed=self.T2)
                second = self.record("3547-check", "11:00", late, handed=self.T1)
                self.assertIs(self.check()[0], late == "no regression")
                self.remove(first)
                self.remove(second)
        # By the start, not the finish: a long check that began first and
        # ended last does not decide.
        for long_one, short_one in [("regression", "no regression"), ("no regression", "regression")]:
            with self.subTest(short_one=short_one):
                first = self.record("long", "10:00", long_one, minutes=120)
                second = self.record("short", "10:30", short_one, minutes=10)
                self.assertIs(self.check()[0], short_one == "no regression")
                self.remove(first)
                self.remove(second)
        # Not by the claim either: a job claimed first and started last.
        first = self.record("claimed-first", "11:00", "regression", claim={"claimed": "2026-11-21T09:00:00Z"})
        second = self.record("claimed-second", "10:30", claim={"claimed": "2026-11-21T10:00:00Z"})
        self.assertIs(self.check()[0], False)
        self.remove(first)
        self.remove(second)
        # And by the second: two starts in one minute, the later one
        # under the name that sorts first, then the earlier one.
        self.record("zz", "10:00:10", "regression")
        later = self.record("aa", "10:00:50")
        self.assertEqual(self.check(), (True, ""))
        self.remove(later)
        self.record("aa", "10:00:05")
        self.assertIs(self.check()[0], False)

    def test_two_that_started_in_one_second_cannot_be_put_in_order(self):
        self.record("aa", "10:00", "regression")
        tied = self.record("zz", "10:00")
        self.held("started in the same second", "aa.", "zz.")
        # Their finishes do not order them.
        self.remove(tied)
        self.record("zz", "10:00", minutes=45)
        self.held("started in the same second", "aa.", "zz.")
        self.record("later", "11:00")
        self.assertEqual(self.check(), (True, ""), "a later start settles it")

    def test_a_record_that_is_not_whole_holds_the_step_however_old(self):
        # Nothing says when such a record ran: it may be the last. (A
        # verdict that lost its start once sorted before a dated pass.)
        self.record("the-pass", "12:00", handed=self.T2)
        ended = "2026-11-21T10:20:00Z"
        times = "its times are not four in order"
        whole = "a check may finish in the second it started, and the two clocks may differ by a minute"
        cases = [
            (dict(verdict={"started": ABSENT}), times, "started None"),
            (dict(verdict={"started": None}), times, "started None"),
            (dict(verdict={"started": "not a date"}), times, 'started "not a date"'),
            (dict(verdict={"started": "2026-13-41T10:00:00Z"}), times, ""),
            (dict(verdict={"started": 1795000000}), times, "started 1795000000"),
            # No other time stands in for a start: not the hand-over's, in the claim's own second.
            (dict(verdict={"started": ABSENT}, handed="20261121T100000Z"), times, "started None"),
            # Digit for digit: strptime alone takes each of these.
            (dict(verdict={"started": "2026-11-21T3:00:00Z"}), times, 'started "2026-11-21T3:00:00Z"'),
            (dict(verdict={"started": "2026-11-21T10:0:0Z"}), times, ""),
            (dict(verdict={"started": " 2026-11-21T10:00:00Z"}), times, ""),
            (dict(verdict={"started": "2026-11-21T10:00:00Z\n"}), times, ""),
            (dict(verdict={"finished": ABSENT}), times, "finished None"),
            (dict(verdict={"finished": "2026-11-21T09:59:59Z"}), times, 'finished "2026-11-21T09:59:59Z"'),
            (dict(verdict={"started": ended, "finished": ended}, claim={"claimed": ended}), None, whole),
            # The claim's time too: the Mac claims a job, then starts it.
            (dict(claim={"claimed": ABSENT}), times, "claimed None"),
            (dict(claim={"claimed": "yesterday"}), times, 'claimed "yesterday"'),
            (dict(claim={"claimed": "2026-11-21T10:01:00Z"}), None, whole),
            (dict(claim={"claimed": "2026-11-21T10:01:01Z"}), times, 'claimed "2026-11-21T10:01:01Z"'),
            (dict(claim={"claimed": "2026-11-21T10:30:00Z"}), times, ""),
            (dict(claim={"claimed": "2026-11-21T09:00:00Z"}), None, "claimed an hour before it started: a busy Mac"),
            # And the hand-over's, the folder's: before the claim.
            (dict(handed="20261341T996633Z"), "its folder's time is no date", ""),
            (dict(handed=self.AFTER), times, "handed over " + self.AFTER),
            (dict(handed="20261121T100100Z"), None, whole),
            (dict(handed="20261121T100101Z"), times, ""),
            (dict(handed="00010101T000000Z"), None, "a hand-over long ago is in order"),
            (dict(handed="99991231T235959Z"), times, ""),
            (dict(verdict=False), "it has no verdict yet", "wait for it"),
            (dict(verdict="{\"job\": \"x.json\", \"status\": \"no regr"), "its verdict.json is not a JSON object with each field once", ""),
            (dict(verdict="[]"), "its verdict.json is not a JSON object", ""),
            (dict(verdict='{"job": "damaged.json", "status": "regression", "status": "no regression"}'), "each field once", ""),
            # A field twice, or a value that is not JSON's, in a record that is whole otherwise.
            (dict(verdict=lambda text: text[:-1] + ', "status": "no regression"}'), "its verdict.json is not a JSON object with each field once", ""),
            (dict(verdict=lambda text: text[:-1] + ', "inspections": [{"at": 1, "at": 2}]}'), "its verdict.json is not a JSON object with each field once", ""),
            (dict(verdict=lambda text: text[:-1] + ', "extra": NaN}'), "its verdict.json is not a JSON object with each field once", ""),
            (dict(verdict=lambda text: text[:-1] + ', "extra": -Infinity}'), "its verdict.json is not a JSON object with each field once", ""),
            (dict(claim=lambda text: text[:-1] + ', "machine": "mac-m6-timing"}'), "its claimed.json is not a JSON object with each field once", ""),
            (dict(verdict="[" * 100000), "its verdict.json is not a JSON object", ""),
            (dict(verdict=b"\xff\xfe"), "its verdict.json is not a JSON object", ""),
            # The exchange publishes no file above 20 MiB; at it, a record is read.
            (dict(verdict=lambda text: text + " " * (check.RECORD_LIMIT + 1 - len(text))), "its verdict.json is too large for a record", ""),
            (dict(verdict=lambda text: text + " " * (check.RECORD_LIMIT - len(text))), None, "a record of exactly the limit"),
            (dict(verdict=lambda text: text + " " * (2 << 20)), None, "two megabytes of a record are read"),
            (dict(verdict={"job": "the-pass.json"}), 'its verdict names the job "the-pass.json"', ""),
            (dict(verdict={"job": ABSENT}), "its verdict names the job None", ""),
            (dict(verdict={"type": "benchmark"}), 'its verdict is of the type "benchmark", not of a check', ""),
            (dict(verdict={"type": ABSENT}), "its verdict is of the type None", ""),
            (dict(verdict={"status": None}), "its status is None, not a text", ""),
            (dict(verdict={"status": ABSENT}), "its status is None, not a text", ""),
            (dict(verdict={"status": ""}), 'its status is "", not a text', ""),
            (dict(verdict={"status": ["no regression"]}), "not a text", ""),
            (dict(claim=False), "its claimed.json cannot be read", ""),
            (dict(claim="{"), "its claimed.json is not a JSON object", ""),
            (dict(claim="{}"), "its claimed.json names None on None", ""),
            (dict(claim={"job": "the-pass.json"}), 'its claimed.json names "the-pass.json"', ""),
            (dict(claim={"job": ABSENT}), "its claimed.json names None", ""),
            (dict(claim={"machine": "mac-m4-timing"}), 'on "mac-m4-timing"', ""),
            (dict(claim={"machine": ABSENT}), "on None", ""),
        ]
        for changes, why, more in cases:
            with self.subTest(changes={key: value for key, value in changes.items() if not callable(value)}, why=why, more=more):
                damaged = self.record("damaged", "10:00", **{"status": "regression", "handed": self.T1, **changes})
                if why is None:
                    self.assertEqual(self.check(), (True, ""), more)
                else:
                    self.held("is no whole record", "damaged.", why, more)
                self.remove(damaged)
        self.assertEqual(self.check(), (True, ""))

    def test_a_link_or_a_file_in_a_records_place_is_no_record(self):
        # A link reads as the file it points to: a verdict linked to an
        # earlier run's would be this run's pass.
        earlier = self.record("earlier", "09:00")
        for what in ["verdict.json", "claimed.json", "the folder", "the job", "a file for the folder"]:
            with self.subTest(what=what):
                later = self.record("later", "10:00", "regression")
                folder = self.root / "results" / later
                if what in ("verdict.json", "claimed.json"):
                    (folder / what).unlink()
                    (folder / what).symlink_to(self.root / "results" / earlier / "verdict.json" if what == "verdict.json" else folder / "nowhere.json")
                    why = "its " + what + " is not a regular file"
                elif what == "the folder":
                    kept = folder.rename(self.root / "kept-elsewhere")
                    folder.symlink_to(kept)
                    why = "it is not a folder"
                elif what == "the job":
                    job = self.root / "jobs" / "later.json"
                    job.unlink()
                    job.symlink_to(self.root / "jobs" / "earlier.json")
                    why = "its job file is not a regular file"
                else:
                    for file in folder.iterdir():
                        file.unlink()
                    folder.rmdir()
                    folder.write_text("notes")
                    why = "it is not a folder"
                self.held(why, "later.")
                self.remove(later)
        self.assertEqual(self.check(), (True, ""))

    def test_another_job_is_never_opened(self):
        self.record("the-pass", "12:00")
        self.assertEqual(self.check(), (True, ""))
        # Not a check of every cell of these commits: by what the job
        # says. One commit that is well formed and another's settles it,
        # whatever the other side is (a job written wrong for another
        # release must not hold this one).
        other = [
            dict(job={"type": "benchmark"}),
            dict(job={"type": "mutants-whole", "cells": ABSENT, "old_commit": ABSENT, "new_commit": ABSENT}),
            dict(job={"cells": ABSENT, "points": ["st|solo|Hash|1024"]}),
            dict(job={"cells": ABSENT, "probe": "st|solo|Hash|1024"}),
            dict(job={"cells": ABSENT}),
            dict(job={"new_commit": "c" * 40}),
            dict(job={"old_commit": "c" * 40}),
            dict(job={"new_commit": "a" * 11 + "c"}),
            dict(job={"new_commit": "c" * 40, "old_commit": None}),
            dict(job={"old_commit": "c" * 40, "new_commit": 7}),
            dict(job={"new_commit": "c" * 40, "cells": "small"}),
            dict(job={"old_commit": "c" * 40, "points": ["st|solo|Hash|1024"]}),
            dict(machine="mac-m4-timing"),
            dict(machine="other-mac-m6-timing"),
            dict(machine="mac-m6-timing-2"),
            dict(machine="mac-m6-timing.bak"),
            dict(machine="mac-\u00e9"),
        ]
        opened = []
        read = check.read_object
        for i, changes in enumerate(other):
            self.record(f"other-{i}", "13:00", "regression", verdict="{", claim="{", **changes)
        with patch.object(check, "read_object", lambda path: opened.append(str(path)) or read(path)):
            self.assertEqual(self.check(), (True, ""))
        self.assertEqual(sorted(path for path in opened if "/results/" in path and "other-" in path), [], "another job's claim and verdict are not opened")
        self.assertEqual(len([path for path in opened if "/results/" in path]), 2, "the one record's claim and verdict")

    def test_a_job_that_cannot_be_told_holds_the_step_where_it_is_the_last_or_not_whole(self):
        self.record("the-pass", "12:00")
        # Cannot be told: no such job says it is another commit's, and
        # none is a check the runner would have run.
        unknown = [
            (dict(job="{"), "its job file is not a JSON object"),
            (dict(job="{}"), "its job's type is None"),
            (dict(job=False), "its job file cannot be read"),
            (dict(job="[]"), "its job file is not a JSON object"),
            (dict(job=lambda text: text[:-1] + ', "cells": "all"}'), "its job file is not a JSON object with each field once"),
            (dict(job=lambda text: text[:-1] + ', "extra": NaN}'), "its job file is not a JSON object with each field once"),
            (dict(job={"type": ABSENT}), "its job's type is None"),
            (dict(job={"type": ["check"]}), "its job's type is ['check']"),
            (dict(job={"type": ""}), 'its job\'s type is ""'),
            (dict(job={"cells": None}), "its job's scope is cells None, which the runner refuses"),
            (dict(job={"cells": ["all"]}), "its job's scope is cells ['all'], which the runner refuses"),
            (dict(job={"cells": "small"}), 'its job\'s scope is cells "small", which the runner refuses'),
            (dict(job={"points": ["st|solo|Hash|1024"]}), "its job's scope is cells \"all\", points ['st|solo|Hash|1024'], which the runner refuses"),
            (dict(job={"probe": "st|solo|Hash|1024"}), 'its job\'s scope is cells "all", probe "st|solo|Hash|1024", which the runner refuses'),
            (dict(job={"new_commit": ABSENT}), "whose commits are"),
            (dict(job={"new_commit": None}), "whose commits are"),
            (dict(job={"new_commit": HEAD[:6]}), "whose commits are"),
            (dict(job={"new_commit": HEAD.upper()}), "whose commits are"),
            (dict(job={"new_commit": HEAD + "0"}), "whose commits are"),
            (dict(job={"old_commit": ABSENT}), "whose commits are"),
            (dict(job={"old_commit": 7}), "whose commits are"),
        ]
        for changes, why in unknown:
            with self.subTest(changes={key: value for key, value in changes.items() if not callable(value)}, why=why):
                # Whole, and started before the pass: the pass stands. (A
                # job written wrong once held every later release.)
                label = self.record("unknown", "09:00", "regression", handed=self.T1, **changes)
                self.assertEqual(self.check(), (True, ""))
                self.remove(label)
                # Started last: it may be the check that decides. Its
                # verdict need not say what kind of job it was.
                for kind in ["check", "benchmark", ABSENT]:
                    label = self.record("unknown", "13:00", "regression", verdict={"type": kind}, **changes)
                    self.held("the last one", "unknown.", "cannot be told from a check of these commits", why)
                    self.remove(label)
                # Not whole: nothing says when it ran.
                label = self.record("unknown", "09:00", "regression", handed=self.T1, verdict={"started": ABSENT}, **changes)
                self.held("is no whole record", "unknown.", why)
                self.remove(label)
        self.assertEqual(self.check(), (True, ""))

    def test_a_commit_named_by_its_beginning_can_fail_the_step_and_cannot_pass_it(self):
        # The runner takes 7 to 40 hex digits and resolves them in its own
        # checkout. A later check that spelled the candidate with twelve
        # was once passed over for an older pass.
        self.record("first", "09:00")
        hour = 10
        for side in ["new_commit", "old_commit"]:
            for digits in [7, 12, 39]:
                with self.subTest(side=side, digits=digits):
                    short = {side: (HEAD if side == "new_commit" else OLD)[:digits]}
                    label = self.record(f"regression-{hour}", f"{hour}:00", "regression", job=short)
                    self.assertEqual(self.check(), (False, self.last(label, "regression")))
                    label = self.record(f"cancelled-{hour}", f"{hour}:20", "cancelled", job=short)
                    self.assertEqual(self.check(), (check.NO_VERDICT, self.last(label, "cancelled")))
                    self.record(f"pass-{hour}", f"{hour}:40", job=short)
                    self.held("names a commit by its beginning", f"pass-{hour}.", "cannot pass a release", "both commits in full")
                    hour += 1
        self.record("in-full", "23:00")
        self.assertEqual(self.check(), (True, ""))

    def test_a_check_still_queued_holds_the_step(self):
        self.record("the-pass", "12:00")
        # A job no Mac has taken: no results folder under any Mac's name.
        self.record("queued", "13:00", folder=False)
        self.held("jobs/queued.json is queued and no Mac has taken it", "a check of these commits", "wait for it, or move its job out of jobs/")
        (self.root / "jobs" / "queued.json").unlink()
        # Whatever Mac the job names: the runners take pending jobs by
        # their files' names, and one written for an M4 was claimed by the M6.
        for i, (changes, why) in enumerate([(dict(job="{"), "its job file is not a JSON object"), (dict(job="{}"), "its job's type is None"),
                                            (dict(job={"new_commit": HEAD[:9]}), "a check of these commits"), (dict(job={"cells": None}), "its job's scope is"),
                                            (dict(job={"machine": "mac-m6-timing"}), "a check of these commits"), (dict(job={"machine": "mac-m4-timing"}), "a check of these commits"),
                                            (dict(job={"machine": None}), "a check of these commits"), (dict(job={"machine": []}), "a check of these commits")]):
            with self.subTest(changes=changes):
                self.record(f"queued-{i}", "13:00", folder=False, **changes)
                self.held(f"jobs/queued-{i}.json is queued", why)
                (self.root / "jobs" / f"queued-{i}.json").unlink()
        # Not this step's: another commit's, another kind, a file that is
        # no job, and a job that a Mac has taken, as the runner sees it
        # (an entry of results/ that begins with the job's name and a dot).
        self.record("another-commit", "13:00", folder=False, job={"new_commit": "c" * 40})
        self.record("a-benchmark", "13:00", folder=False, job={"type": "benchmark"})
        (self.root / "jobs" / ".keep").write_text("")
        (self.root / "jobs" / "README.md").write_text("{")
        self.record("taken-by-the-m4", "13:00", "regression", machine="mac-m4-timing")
        self.record("taken-by-a-mac-named-otherwise", "13:00", "regression", machine="mac-\u00e9")
        self.record("taken-in-name", "13:00", folder=False)
        (self.root / "results" / f"taken-in-name.{self.T2}-mac-m4-timing").write_text("")
        self.assertEqual(self.check(), (True, ""))
        # A job whose name begins a taken job's name is not that job.
        self.record("the-pas", "13:00", folder=False)
        self.held("jobs/the-pas.json is queued")

    def test_a_jobs_name_is_whatever_it_is(self):
        # Dots, a time and a Mac's name inside, a leading dot, a line
        # break: the runner takes each, and a results folder is read from
        # its end. (Split at the first dot, 0.10.0's checks had no job.)
        names = ["3547-check-release-0.10.0-2fd4ff6-margin5", f"odd.{self.T1}-mac-m4-timing.again", ".retry", "two\nlines", "[1]", "*"]
        for hour, name in enumerate(names, 10):
            with self.subTest(name=name):
                self.record(name, f"{hour}:00", "regression")
                self.assertIs(self.check()[0], False)
                self.record(name + "-rerun", f"{hour}:30")
                self.assertEqual(self.check(), (True, ""))
        # The job is the file of jobs/ with the folder's name, as the
        # runner takes it: jobs/.json is the job named ".json".
        label = self.record(".json", "20:00", "regression", file=".json")
        self.assertEqual(self.check(), (False, self.last(label, "regression")))
        (self.root / "jobs" / ".json.json").write_text((self.root / "jobs" / ".json").read_text())
        self.held("two files of jobs/ have its name", '".json"', '".json.json"')
        (self.root / "jobs" / ".json.json").unlink()
        self.remove(label, file=".json")
        self.record("held\nhere", "23:00", verdict={"status": None})
        self.held("held\\nhere")

    def test_the_exchanges_path_is_a_path(self):
        # Not a pattern: a clone at "mac-runner[1]" read as no exchange.
        odd = self.root.with_name("mac-runner[1]*")
        self.root.rename(odd)
        self.root = odd
        self.record("the-check", "10:00", "regression")
        self.assertIs(self.check()[0], False)
        self.assertEqual(check.mac_check(str(odd) + "-gone", HEAD, OLD)[0], check.NO_VERDICT, "an exchange that is not there decides nothing")

    def test_both_ends_power_must_be_ac(self):
        for hour, end in enumerate(["power at start", "power at end"], 10):
            for i, reading in enumerate([None, 5, ["AC Power"], "AC Powerless", "", "ac power", "Battery Power, 98%", ABSENT]):
                with self.subTest(end=end, reading=reading):
                    label = self.record("power", f"{hour}:00", verdict={end: reading})
                    outcome, note = self.check()
                    self.assertEqual(outcome, check.NO_VERDICT, note)
                    self.assertIn(self.last(label, "no regression") + " (it ran off AC power: " + (reading if isinstance(reading, str) and reading else "unrecorded"), note)
                    self.remove(label)
        self.record("power", "12:00", verdict={"power at start": "AC Power", "power at end": "AC Power , 100%; charging"})
        self.assertEqual(self.check(), (True, ""), "the source alone, whatever follows it")

    def test_inspections_and_the_runners_statuses(self):
        once = [{"at": "2026-11-21T10:05:00Z", "duration_seconds": 1.25}]
        hour = 10
        for inspections, why in [(once, "it was inspected while it ran, 1 time:"), (once * 3, "it was inspected while it ran, 3 times:"), ([None], "1 time:")] + [
                (value, f'its "inspections" is {type(value).__name__}, not a list') for value in [None, False, 0, 0.0, "", {}, "yes", 3, {"at": "09:00"}, True]]:
            for status in ["no regression", "regression", "error", "no verdict", "cancelled"]:
                with self.subTest(inspections=inspections, status=status):
                    label = self.record("inspected", f"{hour}:00", status, verdict={"inspections": inspections})
                    self.held(self.last(label, status), why)
                    self.remove(label)
        label = self.record("timed-out", "11:00", "timed out", verdict={"inspections": once, "power at end": "Battery Power, 98%"})
        outcome, note = self.check()
        self.assertEqual(outcome, check.TIMED_OUT)
        for text in [self.last(label, "timed out"), "inspected while it ran, 1 time:", "run it again plugged in"]:
            self.assertIn(text, note)
        self.remove(label)
        for status, outcome in [("no regression", True), ("regression", False)]:
            label = self.record("not-inspected", "12:00", status, verdict={"inspections": []})
            self.assertIs(self.check()[0], outcome)
            self.remove(label)

    def test_a_pass_that_carries_the_runners_word_of_a_failure_passes_nothing(self):
        # Beside a failing status the runner writes why, as a field of
        # that name ("error", "timed out"; "cancelled" with BLAKE3 #464).
        # A verdict that says "no regression" and holds one contradicts
        # itself: 0.10.0's job 3551, its status alone changed, passed.
        fields = [("error", "CalledProcessError: Command '['cargo', 'run']' died with <Signals.SIGABRT: 6>.", "error", False),
                  ("timed out", "the job's 7200 s limit ended: cargo run", "timed out", check.TIMED_OUT), ("cancelled", "by a person", "cancelled", check.NO_VERDICT)]
        for field, value, status, outcome in fields:
            with self.subTest(field=field):
                label = self.record("contradicts", "10:00", verdict={field: value})
                self.held(self.last(label, "no regression"), "it says no regression and also has the runner's", json.dumps(field))
                self.remove(label)
                # As the runner writes it, the status and the field agree.
                label = self.record("agrees", "10:00", status, verdict={field: value})
                self.assertEqual(self.check(), (outcome, self.last(label, status)))
                self.remove(label)
        label = self.record("contradicts", "10:00", verdict={"error": None, "cancelled": False})
        self.held('"error" and "cancelled"')

    def test_without_a_previous_release_any_commit_is_the_old_side_and_it_is_named_in_full(self):
        self.record("first-release", "10:00", job={"old_commit": "c" * 40})
        self.assertEqual(self.check(old=None), (True, ""))
        self.assertEqual(self.check(), (None, ""), "with a previous release it is another commit's check")
        self.record("short", "11:00", job={"old_commit": "c" * 12})
        self.held("names a commit by its beginning", old=None)
        self.record("no-old-side", "12:00", "regression", job={"old_commit": ABSENT})
        self.held("cannot be told from a check of these commits", old=None)

    def test_a_status_and_a_name_are_shown_on_one_line_and_cut(self):
        label = self.record("odd-status", "10:00", "regression\nPASS  Mac check" + "!" * 400)
        outcome, note = self.check()
        self.assertIs(outcome, False)
        self.assertNotIn("\n", note)
        self.assertIn(json.dumps(label) + ": regression\\nPASS  Mac check!!!", note)
        self.assertTrue(note.endswith("!...)"), note[-20:])
        self.assertLess(len(note), 60 + len(label) + 163)
        self.remove(label)
        label = self.record("n" * 200, "10:00", "regression")
        outcome, note = self.check()
        self.assertIn('"' + "n" * 160 + "...", note)
        self.assertLess(len(note), 240)


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
