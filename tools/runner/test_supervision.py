"""Event-driven child supervision: process-exit wake, stack capture on
inspect/cancel/timeout, progress milestones, and verdict inspection records."""
import json
import os
import signal
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path

import runner
import wait_for


class Supervision(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.prev_session = runner.SESSION_FILE
        self.prev_home = runner.HOME
        runner.HOME = self.root
        runner.SESSION_FILE = self.root / "runner-session.txt"
        runner.inspect_requested = False
        runner.cancel_requested = False

    def tearDown(self):
        runner.HOME = self.prev_home
        runner.SESSION_FILE = self.prev_session
        runner.inspect_requested = False
        runner.cancel_requested = False
        self.tmp.cleanup()

    def test_fast_exit_and_large_captured_stdout_do_not_deadlock(self):
        out = self.root / "out"
        out.mkdir()
        with open(out / "runner.log", "w") as log:
            job = runner.Job(log, limit=None, out=out)
            t0 = time.monotonic()
            # > 128 KiB exceeds Linux/macOS pipe buffers if stdout isn't drained during wait.
            got = job.run([sys.executable, "-c", "import sys; sys.stdout.write('x' * 200000)"], cwd=self.root, capture=True)
            elapsed = time.monotonic() - t0
        self.assertEqual(len(got), 200000)
        self.assertLess(elapsed, 2.0)
        prog = json.loads((out / "progress.json").read_text())
        self.assertEqual(prog["state"], "step_done")
        self.assertEqual(prog["step"], 1)

    def _assert_stacks_captured(self, text):
        if sys.platform == "darwin":
            self.assertIn("Call graph:", text)
        elif sys.platform == "linux":
            # Verify actual /proc/<pid>/task/<tid>/{comm,wchan} content was read
            # (not placeholders "(?)" or "wchan=?").
            lines = [ln for ln in text.splitlines() if ln.startswith("tid ")]
            self.assertTrue(lines, f"expected tid lines in stacks:\n{text}")
            self.assertTrue(
                any("(?)" not in ln and "wchan=?" not in ln and "wchan=" in ln for ln in lines),
                f"expected real comm and wchan from /proc:\n{text}",
            )

    def test_time_limit_captures_stacks_and_kills_whole_session(self):
        out = self.root / "out"
        out.mkdir()
        pid_file = self.root / "grandchild.pid"
        script = (
            "import subprocess, sys, time, pathlib\n"
            f"p = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'])\n"
            f"pathlib.Path({str(pid_file)!r}).write_text(str(p.pid))\n"
            "time.sleep(30)\n"
        )
        with open(out / "runner.log", "w") as log:
            job = runner.Job(log, limit=2, out=out)
            with self.assertRaises(runner.TimedOut):
                job.run([sys.executable, "-c", script], cwd=self.root)
        self.assertTrue((out / "stacks.txt").is_file())
        stacks = (out / "stacks.txt").read_text()
        self.assertIn("timed out", stacks)
        self._assert_stacks_captured(stacks)
        self.assertTrue(pid_file.exists())
        gc_pid = int(pid_file.read_text().strip())
        time.sleep(0.1)
        with self.assertRaises(ProcessLookupError):
            os.kill(gc_pid, 0)

    def test_deadline_still_applies_when_descendant_keeps_stdout_open(self):
        # The leader exits, but EOF cannot arrive until the descendant closes stdout.
        # Give the descendant a finite lifetime even if supervision regresses.
        script = (
            "import subprocess, sys; "
            "subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(3)'])"
        )
        with open(self.root / "runner.log", "w") as log:
            with self.assertRaises(runner.TimedOut):
                runner.Job(log, limit=1).run([sys.executable, "-c", script], cwd=self.root, capture=True)

    def test_inspect_control_file_records_stacks_without_stopping_child(self):
        out = self.root / "out"
        ctrl = self.root / "jobs"
        out.mkdir()
        ctrl.mkdir()
        marker = self.root / "started"
        done = self.root / "done"

        def trigger():
            deadline = time.monotonic() + 5.0
            while not marker.exists() and time.monotonic() < deadline:
                time.sleep(0.01)
            (ctrl / "0001-demo.inspect").write_text("now\n")
            deadline = time.monotonic() + 5.0
            while not (out / "stacks.txt").exists() and time.monotonic() < deadline:
                time.sleep(0.02)
            done.write_text("ok\n")

        t = threading.Thread(target=trigger, daemon=True)
        t.start()
        script = (
            f"import pathlib, time; pathlib.Path({str(marker)!r}).write_text('1')\n"
            f"end = time.monotonic() + 10\n"
            f"while not pathlib.Path({str(done)!r}).exists() and time.monotonic() < end: time.sleep(0.02)\n"
        )
        with open(out / "runner.log", "w") as log:
            job = runner.Job(log, limit=None, out=out, control_dir=ctrl, job_stem="0001-demo")
            job.run([sys.executable, "-c", script], cwd=self.root)
        t.join(timeout=2)
        stacks = (out / "stacks.txt").read_text()
        self.assertIn("(inspect,", stacks)
        self._assert_stacks_captured(stacks)
        self.assertEqual(len(job.inspections), 1)
        self.assertIn("at", job.inspections[0])
        self.assertGreaterEqual(job.inspections[0]["duration_seconds"], 0.0)
        self.assertFalse((ctrl / "0001-demo.inspect").exists())

    def test_cancel_control_file_wakes_promptly_captures_stacks_and_marks_cancelled(self):
        out = self.root / "out"
        ctrl = self.root / "jobs"
        out.mkdir()
        ctrl.mkdir()
        marker = self.root / "started"

        def trigger():
            deadline = time.monotonic() + 5.0
            while not marker.exists() and time.monotonic() < deadline:
                time.sleep(0.01)
            (ctrl / "0002-demo.cancel").write_text("stop\n")

        t = threading.Thread(target=trigger, daemon=True)
        t.start()
        script = f"import pathlib, time; pathlib.Path({str(marker)!r}).write_text('1'); time.sleep(30)"
        t0 = time.monotonic()
        with open(out / "runner.log", "w") as log:
            job = runner.Job(log, limit=None, out=out, control_dir=ctrl, job_stem="0002-demo")
            with self.assertRaises(runner.Cancelled):
                job.run([sys.executable, "-c", script], cwd=self.root)
        elapsed = time.monotonic() - t0
        t.join(timeout=2)
        self.assertLess(elapsed, 5.0)
        stacks = (out / "stacks.txt").read_text()
        self.assertIn("(cancelled,", stacks)
        self._assert_stacks_captured(stacks)
        self.assertEqual(json.loads((out / "progress.json").read_text())["state"], "cancelled")

    def test_signal_cancel_wakes_supervisor(self):
        out = self.root / "out"
        out.mkdir()
        marker = self.root / "started"

        def trigger():
            deadline = time.monotonic() + 5.0
            while not marker.exists() and time.monotonic() < deadline:
                time.sleep(0.01)
            runner.on_cancel(signal.SIGUSR2, None)

        t = threading.Thread(target=trigger, daemon=True)
        t.start()
        script = f"import pathlib, time; pathlib.Path({str(marker)!r}).write_text('1'); time.sleep(30)"
        t0 = time.monotonic()
        with open(out / "runner.log", "w") as log:
            job = runner.Job(log, limit=None, out=out)
            with self.assertRaises(runner.Cancelled):
                job.run([sys.executable, "-c", script], cwd=self.root)
        elapsed = time.monotonic() - t0
        t.join(timeout=2)
        self.assertLess(elapsed, 5.0)
        stacks = (out / "stacks.txt").read_text()
        self.assertIn("(cancelled,", stacks)
        self._assert_stacks_captured(stacks)

    def test_process_enforces_limits_and_records_inspections_in_verdict(self):
        out = self.root / "out"
        out.mkdir()
        job_file = self.root / "0003-demo.json"
        orig = runner.HANDLERS["test"]
        seen = {}
        try:
            def fake_test(job, run, work, out_dir):
                seen["limit"] = run.limit
                run.inspections.append({"at": "2026-11-22T09:00:00Z", "duration_seconds": 1.25})
                return "passed"
            runner.HANDLERS["test"] = fake_test

            job_file.write_text(json.dumps({"type": "test", "fork_commit": "0" * 40}))
            status = runner.process(job_file, out)
            self.assertEqual(status, "passed")
            self.assertEqual(seen["limit"], runner.DEFAULT_LIMIT)
            verdict = json.loads((out / "verdict.json").read_text())
            self.assertEqual(verdict["inspections"], [{"at": "2026-11-22T09:00:00Z", "duration_seconds": 1.25}])

            job_file.write_text(json.dumps({"type": "test", "fork_commit": "0" * 40, "time_limit_seconds": runner.MAX_LIMIT + 1}))
            status = runner.process(job_file, out)
            self.assertEqual(status, "error")
        finally:
            runner.HANDLERS["test"] = orig

    def test_wait_for_keeps_limits_and_cancelled_exit_code(self):
        self.assertEqual(wait_for.DEFAULT_LIMIT, runner.DEFAULT_LIMIT)
        self.assertEqual(wait_for.MAX_LIMIT, runner.MAX_LIMIT)
        self.assertEqual(wait_for.exit_code_for("passed"), 0)
        self.assertEqual(wait_for.exit_code_for("no regression"), 0)
        self.assertEqual(wait_for.exit_code_for("cancelled"), 2)
        self.assertEqual(wait_for.exit_code_for("timed out"), 2)
        self.assertEqual(wait_for.exit_code_for("no verdict"), 2)
        self.assertEqual(wait_for.exit_code_for("regression"), 1)
        self.assertEqual(wait_for.exit_code_for("error"), 1)

    def test_child_in_own_process_group_is_sampled_and_killed(self):
        # Finding #1: cargo-nextest / cargo-mutants place workers in their own pgrp.
        out = self.root / "out"
        out.mkdir()
        worker_pid_file = self.root / "worker.pid"
        script = (
            "import os, subprocess, sys, time, pathlib\n"
            "p = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'], process_group=0)\n"
            f"pathlib.Path({str(worker_pid_file)!r}).write_text(str(p.pid))\n"
            "time.sleep(30)\n"
        )
        with open(out / "runner.log", "w") as log:
            job = runner.Job(log, limit=2, out=out)
            with self.assertRaises(runner.TimedOut):
                job.run([sys.executable, "-c", script], cwd=self.root)
        self.assertTrue(worker_pid_file.exists())
        worker_pid = int(worker_pid_file.read_text().strip())
        stacks = (out / "stacks.txt").read_text()
        self.assertIn(f"--- pid {worker_pid} ---", stacks)
        time.sleep(0.1)
        with self.assertRaises(ProcessLookupError):
            os.kill(worker_pid, 0)

    def test_deadline_still_enforced_when_inspection_outlasts_limit(self):
        # Finding #2: if an inspection runs past the deadline while the child exits,
        # the step must still end TimedOut (not step_done).
        out = self.root / "out"
        ctrl = self.root / "jobs"
        out.mkdir()
        ctrl.mkdir()
        (ctrl / "0010-demo.inspect").write_text("1\n")
        orig_capture = runner.capture_stacks
        try:
            def slow_capture(sid, out_dir, reason, budget=runner.CAPTURE_BUDGET):
                if reason == "inspect":
                    time.sleep(0.6)
                return orig_capture(sid, out_dir, reason, budget=budget)
            runner.capture_stacks = slow_capture
            with open(out / "runner.log", "w") as log:
                job = runner.Job(log, limit=0.4, out=out, control_dir=ctrl, job_stem="0010-demo")
                with self.assertRaises(runner.TimedOut):
                    job.run([sys.executable, "-c", "import time; time.sleep(0.2)"], cwd=self.root)
        finally:
            runner.capture_stacks = orig_capture

    def test_cancel_detected_when_dir_watch_unavailable(self):
        # Finding #3: fallback polling when inotify/vnode watch cannot be opened.
        out = self.root / "out"
        ctrl = self.root / "jobs"
        out.mkdir()
        ctrl.mkdir()
        marker = self.root / "started"

        def trigger():
            deadline = time.monotonic() + 5.0
            while not marker.exists() and time.monotonic() < deadline:
                time.sleep(0.01)
            (ctrl / "0011-demo.cancel").write_text("stop\n")

        orig_watch = runner._open_dir_watch
        t = threading.Thread(target=trigger, daemon=True)
        t.start()
        try:
            runner._open_dir_watch = lambda _folder: (None, None)
            script = f"import pathlib, time; pathlib.Path({str(marker)!r}).write_text('1'); time.sleep(30)"
            with open(out / "runner.log", "w") as log:
                job = runner.Job(log, limit=10, out=out, control_dir=ctrl, job_stem="0011-demo")
                t0 = time.monotonic()
                with self.assertRaises(runner.Cancelled):
                    job.run([sys.executable, "-c", script], cwd=self.root)
                self.assertLess(time.monotonic() - t0, 3.0)
        finally:
            runner._open_dir_watch = orig_watch
            t.join(timeout=2)

    def test_read_only_inspect_marker_fires_once(self):
        # Finding #6: an undeletable .inspect file must not re-fire on every check.
        ctrl = self.root / "jobs"
        ctrl.mkdir()
        marker = ctrl / "0012-demo.inspect"
        marker.write_text("1\n")
        orig_unlink = Path.unlink
        try:
            def deny_unlink(self_path, *args, **kwargs):
                if self_path == marker:
                    raise PermissionError("read-only jobs dir")
                return orig_unlink(self_path, *args, **kwargs)
            Path.unlink = deny_unlink
            with open(self.root / "runner.log", "w") as log:
                job = runner.Job(log, limit=10, control_dir=ctrl, job_stem="0012-demo")
                self.assertTrue(job._consume_control("inspect"))
                self.assertFalse(job._consume_control("inspect"))
                self.assertTrue(marker.exists())
        finally:
            Path.unlink = orig_unlink

    def test_supervision_errors_still_kill_child_and_preserve_timeout(self):
        # Finding #4: pidfd_open EPERM falls back cleanly; unwritable out/ does not turn TimedOut into error.
        out = self.root / "missing-out"
        orig_pidfd = getattr(os, "pidfd_open", None)
        try:
            if orig_pidfd is not None:
                os.pidfd_open = lambda _pid, _flags=0: (_ for _ in ()).throw(PermissionError("blocked"))
            with open(self.root / "runner.log", "w") as log:
                job = runner.Job(log, limit=1, out=out)
                with self.assertRaises(runner.TimedOut):
                    job.run([sys.executable, "-c", "import time; time.sleep(30)"], cwd=self.root)
        finally:
            if orig_pidfd is not None:
                os.pidfd_open = orig_pidfd

    def test_cancel_during_post_command_collection_cancels_job(self):
        # Finding #8: a cancel arriving after the last command while handler collects results.
        out = self.root / "out"
        ctrl = self.root / "jobs"
        out.mkdir()
        ctrl.mkdir()
        job_file = ctrl / "0013-demo.json"
        job_file.write_text(json.dumps({"type": "test", "fork_commit": "0" * 40}))
        orig = runner.HANDLERS["test"]
        try:
            def fake_test(job, run, work, out_dir):
                (ctrl / "0013-demo.cancel").write_text("stop\n")
                return "passed"
            runner.HANDLERS["test"] = fake_test
            status = runner.process(job_file, out, control_dir=ctrl)
            self.assertEqual(status, "cancelled")
        finally:
            runner.HANDLERS["test"] = orig

    def test_tmp_mkdir_failure_still_stops_meters(self):
        # Finding #9: failing before runner_job is created must not raise UnboundLocalError or leak meters.
        out = self.root / "out"
        out.mkdir()
        # Block (HOME / "tmp").mkdir by placing a regular file at HOME / "tmp".
        (self.root / "tmp").write_text("not a dir\n")
        job_file = self.root / "0014-demo.json"
        job_file.write_text(json.dumps({"type": "test", "fork_commit": "0" * 40, "top": True}))
        orig_top = runner.TOP
        meter_pid_file = self.root / "meter.pid"
        runner.TOP = [
            sys.executable, "-c",
            f"import os, pathlib, time; pathlib.Path({str(meter_pid_file)!r}).write_text(str(os.getpid())); time.sleep(30)",
        ]
        try:
            status = runner.process(job_file, out)
            self.assertEqual(status, "error")
            verdict = json.loads((out / "verdict.json").read_text())
            self.assertNotIn("UnboundLocalError", verdict.get("error", ""))
            if meter_pid_file.exists():
                mpid = int(meter_pid_file.read_text().strip())
                time.sleep(0.1)
                with self.assertRaises(ProcessLookupError):
                    os.kill(mpid, 0)
        finally:
            runner.TOP = orig_top

    def test_cancelled_timed_job_keeps_cancelled_status_if_power_changes(self):
        # Finding #11: cancelled status is preserved (wait_for exit 2) even if power left mains.
        out = self.root / "out"
        out.mkdir()
        job_file = self.root / "0015-demo.json"
        job_file.write_text(json.dumps({"type": "crowd", "fork_commit": "0" * 40}))
        orig_power = runner.power
        orig_handler = runner.HANDLERS["crowd"]
        calls = {"n": 0}
        try:
            def fake_power():
                calls["n"] += 1
                return "AC Power, lowpowermode 0" if calls["n"] == 1 else "Battery Power, 90%"
            def fake_crowd(job, run, work, out_dir):
                raise runner.Cancelled("cancelled by user")
            runner.power = fake_power
            runner.HANDLERS["crowd"] = fake_crowd
            status = runner.process(job_file, out)
            self.assertEqual(status, "cancelled")
            self.assertEqual(wait_for.exit_code_for(status), 2)
        finally:
            runner.power = orig_power
            runner.HANDLERS["crowd"] = orig_handler

    def test_wait_for_limits_match_runner_limits(self):
        self.assertEqual(wait_for.LIMITS, runner.LIMITS)
    def test_sync_tolerates_moved_upstream_tag(self):
        # Finding #12: git fetch must not fail with "would clobber existing tag".
        origin = self.root / "origin.git"
        work = self.root / "work"
        dest = self.root / "dest"
        subprocess_run = runner.subprocess.run
        subprocess_run(["git", "init", "--bare", "--quiet", str(origin)], check=True)
        subprocess_run(["git", "init", "--quiet", str(work)], check=True)
        subprocess_run(["git", "config", "user.email", "t@example.com"], cwd=work, check=True)
        subprocess_run(["git", "config", "user.name", "Test"], cwd=work, check=True)
        (work / "a.txt").write_text("one\n")
        subprocess_run(["git", "add", "a.txt"], cwd=work, check=True)
        subprocess_run(["git", "commit", "--quiet", "-m", "one"], cwd=work, check=True)
        c1 = subprocess_run(["git", "rev-parse", "HEAD"], cwd=work, capture_output=True, text=True, check=True).stdout.strip()
        subprocess_run(["git", "tag", "v1"], cwd=work, check=True)
        subprocess_run(["git", "push", "--quiet", str(origin), "HEAD:refs/heads/main", "refs/tags/v1"], cwd=work, check=True)

        with open(self.root / "runner.log", "w") as log:
            job = runner.Job(log, limit=30)
            job.sync(str(origin), dest, c1)

        # Move tag v1 upstream to a new commit.
        (work / "a.txt").write_text("two\n")
        subprocess_run(["git", "commit", "--quiet", "-am", "two"], cwd=work, check=True)
        c2 = subprocess_run(["git", "rev-parse", "HEAD"], cwd=work, capture_output=True, text=True, check=True).stdout.strip()
        subprocess_run(["git", "tag", "-f", "v1"], cwd=work, check=True)
        subprocess_run(["git", "push", "--quiet", "--force", str(origin), "HEAD:refs/heads/main", "refs/tags/v1"], cwd=work, check=True)

        with open(self.root / "runner.log", "w") as log:
            job = runner.Job(log, limit=30)
            job.sync(str(origin), dest, c2)
        got = subprocess_run(["git", "rev-parse", "HEAD"], cwd=dest, capture_output=True, text=True, check=True).stdout.strip()
        self.assertEqual(got, c2)

if __name__ == "__main__":
    unittest.main()
