"""Event-driven child supervision: process-exit wake, stack capture on
inspect/cancel/timeout, progress milestones, and no arbitrary time ceiling."""
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

    def test_optional_time_limit_captures_stacks_and_kills_whole_session(self):
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
            job = runner.Job(log, limit=1, out=out)
            with self.assertRaises(runner.TimedOut):
                job.run([sys.executable, "-c", script], cwd=self.root)
        self.assertTrue((out / "stacks.txt").is_file())
        self.assertIn("timed out", (out / "stacks.txt").read_text())
        if pid_file.exists():
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
        self.assertIn("(inspect,", (out / "stacks.txt").read_text())
        if sys.platform == "darwin":
            self.assertIn("Analysis of sampling", (out / "stacks.txt").read_text())
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
        self.assertIn("(cancelled,", (out / "stacks.txt").read_text())
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
        self.assertIn("(cancelled,", (out / "stacks.txt").read_text())

    def test_process_accepts_job_with_no_time_limit_and_with_large_limit(self):
        out = self.root / "out"
        out.mkdir()
        job_file = self.root / "0003-demo.json"
        job_file.write_text(json.dumps({"type": "test", "fork_commit": "0" * 40, "time_limit_seconds": 14400}))
        orig = runner.HANDLERS["test"]
        seen = {}
        try:
            def fake_test(job, run, work, out_dir):
                seen["limit"] = run.limit
                return "passed"
            runner.HANDLERS["test"] = fake_test
            status = runner.process(job_file, out)
            self.assertEqual(status, "passed")
            self.assertEqual(seen["limit"], 14400)

            job_file.write_text(json.dumps({"type": "test", "fork_commit": "0" * 40}))
            status = runner.process(job_file, out)
            self.assertEqual(status, "passed")
            self.assertIsNone(seen["limit"])
        finally:
            runner.HANDLERS["test"] = orig

    def test_wait_for_exit_codes_include_cancelled_and_no_default_limits(self):
        self.assertFalse(hasattr(wait_for, "MAX_LIMIT"))
        self.assertFalse(hasattr(wait_for, "DEFAULT_LIMIT"))
        self.assertEqual(wait_for.exit_code_for("passed"), 0)
        self.assertEqual(wait_for.exit_code_for("no regression"), 0)
        self.assertEqual(wait_for.exit_code_for("cancelled"), 2)
        self.assertEqual(wait_for.exit_code_for("timed out"), 2)
        self.assertEqual(wait_for.exit_code_for("no verdict"), 2)
        self.assertEqual(wait_for.exit_code_for("regression"), 1)
        self.assertEqual(wait_for.exit_code_for("error"), 1)

    @unittest.skipUnless(sys.platform == "darwin", "macOS top -l 0 check")
    def test_macos_top_runs_without_count_cap_and_stops_on_sigint(self):
        out_path = self.root / "top.txt"
        with open(out_path, "w") as out_f:
            proc = runner.subprocess.Popen(runner.TOP, stdout=out_f, stderr=runner.subprocess.PIPE)
            time.sleep(1.5)
            proc.send_signal(signal.SIGINT)
            proc.wait(timeout=10)
        self.assertGreater(out_path.stat().st_size, 0)


if __name__ == "__main__":
    unittest.main()
