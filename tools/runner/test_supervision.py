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
            self.assertIn("wchan=", text)

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


if __name__ == "__main__":
    unittest.main()
