"""A job that reaches its time limit leaves where its processes stood (stacks.txt).

A hang found by the limit used to leave "timed out" and the command's name. Now the folder says where each
process was. The runner reads the processes while it holds them stopped, between `end_job`'s stop and its
kill, so these tests also hold the reading to its budget: a sampler or a `ps` that hangs must not keep a
job's processes there."""
import json
import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch

import runner

# A worker in a process group of its own, as a test started by cargo-nextest is, which says its PID.
# (One line: `ps` prints a command's arguments as they are, and the listing is read by line.)
WORKER = "import os, sys, time; os.setpgid(0, 0); said = open(sys.argv[1], 'w'); said.write(str(os.getpid())); said.close(); time.sleep(30)"
# The command: it starts the worker and waits.
COMMAND = "import subprocess, sys, time; subprocess.Popen([sys.executable, '-c', sys.argv[1], sys.argv[2]]); time.sleep(30)"
# A sampler called as macOS's is (PID, seconds, -file, path): it says its own PID, then "samples": the
# process's state as ps gives it then.
SAMPLER = '#!/bin/sh\necho $$ >> "$(dirname "$0")/sampler.pids"\necho "Call graph of $1, in state $(ps -o stat= -p "$1")" > "$4"\n'
# One that hangs, and a `ps` that hangs.
HANGS = '#!/bin/sh\necho $$ >> "$(dirname "$0")/sampler.pids"\nexec sleep 30\n'
# What a child interpreter under the C locale does: a command whose one thread has a name outside ASCII
# runs into a 2 s limit. Its source is ASCII; the name is written by the command itself.
C_LOCALE = r"""
import os, sys
from pathlib import Path
from unittest.mock import patch
import runner
folder = Path(sys.argv[1])
named = "import time\nwith open('/proc/self/comm', 'wb') as comm: comm.write('h\\u00e9h\\u00e9'.encode('utf-8'))\ntime.sleep(30)\n"
with patch.object(runner, "SESSION_FILE", folder / "runner-session.txt"), patch.object(runner, "environment", lambda: dict(os.environ)):
    with open(folder / "runner.log", "w") as log:
        job = runner.Job(log, 2)
        try:
            job.run([sys.executable, "-c", named], cwd=folder)
        except runner.TimedOut:
            print("timed out")
        print(job.stacks)
"""


def running(pid):
    """Is the process there and no zombie?"""
    out = subprocess.run(["ps", "-o", "stat=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    return bool(out) and not out.startswith("Z")


def gone(pid, within=5.0):
    end = time.monotonic() + within
    while running(pid):
        if time.monotonic() > end:
            return False
        time.sleep(0.05)
    return True


class Stacks(unittest.TestCase):
    def setUp(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        self.dir = Path(folder.name)
        for name, value in (("SESSION_FILE", self.dir / "runner-session.txt"), ("environment", lambda: dict(os.environ))):
            patched = patch.object(runner, name, value)
            patched.start()
            self.addCleanup(patched.stop)
        self.pid_file = self.dir / "worker.pid"
        self.stacks = self.dir / "stacks.txt"       # beside the log
        self.started = []

    def tearDown(self):
        for pid in self.started + self.samplers():
            try:
                os.kill(pid, signal.SIGKILL)
            except OSError:
                pass

    def script(self, name, text):
        """An executable script in this test's folder; its path."""
        path = self.dir / name
        path.write_text(text)
        path.chmod(0o755)
        return str(path)

    def samplers(self):
        """The PIDs the fake samplers said."""
        said = self.dir / "sampler.pids"
        return [int(line) for line in said.read_text().split()] if said.exists() else []

    def worker(self, within=10.0):
        """The worker's PID, once it has said it."""
        end = time.monotonic() + within
        while not (self.pid_file.exists() and self.pid_file.read_text()):
            self.assertLess(time.monotonic(), end, "the worker did not start")
            time.sleep(0.02)
        pid = int(self.pid_file.read_text())
        self.started.append(pid)
        return pid

    def command(self):
        """The command and its worker, started as `Job.run` starts a command: the command's PID and start
        time, and the worker's PID."""
        proc = subprocess.Popen([sys.executable, "-c", COMMAND, WORKER, str(self.pid_file)], start_new_session=True)
        self.started.append(proc.pid)
        self.addCleanup(proc.wait)
        return proc.pid, runner.started(proc.pid), self.worker()

    def at_its_limit(self, limit):
        """Run the command with its worker into a limit of `limit` seconds. The job, the seconds it took,
        and the worker's PID."""
        with open(self.dir / "runner.log", "w") as log:
            job = runner.Job(log, limit)
            began = time.monotonic()
            with self.assertRaises(runner.TimedOut):
                job.run([sys.executable, "-c", COMMAND, WORKER, str(self.pid_file)], cwd=self.dir)
            took = time.monotonic() - began
        return job, took, self.worker(within=0.1)       # it had said its PID before the limit

    def section(self, pid):
        """What stacks.txt holds for one process."""
        text = self.stacks.read_text(encoding="utf-8")
        self.assertIn(f"\n--- process {pid} ---\n", text)
        return text.split(f"\n--- process {pid} ---\n")[1].split("\n--- process ")[0]

    def test_a_job_at_its_limit_leaves_what_its_processes_were_doing(self):
        """With what the platform has: macOS's sample, or Linux's /proc."""
        job, _, worker = self.at_its_limit(2)
        self.assertEqual(job.stacks, "stacks.txt: 2 of the command's 2 processes at the time limit")
        text = self.stacks.read_text(encoding="utf-8")
        self.assertIn("the job's 2 s limit ended: ", text.splitlines()[0])
        self.assertIn("2 processes: PID, parent, state before the stop, % of a CPU, time running, command", text)
        # The worker's row: it slept (S, or I on macOS) when the runner found it, and its command.
        self.assertRegex(text, rf"(?m)^\s*{worker}\s+\d+\s+[SI]\S*\s.*time\.sleep\(30\)")
        if runner.SAMPLER:
            self.assertIn("Call graph:", self.section(worker))
        else:
            self.assertRegex(self.section(worker), r"^thread \d+ \([^)]+\): \d+\.\d\d s of CPU")
        self.assertIn("runner: stacks.txt: 2 of", (self.dir / "runner.log").read_text())
        self.assertTrue(gone(worker), "the worker outlived the limit")

    def test_with_a_sampler_each_process_has_its_sample(self):
        with patch.object(runner, "SAMPLER", self.script("sampler", SAMPLER)):
            job, _, worker = self.at_its_limit(2)
        self.assertEqual(job.stacks, "stacks.txt: 2 of the command's 2 processes at the time limit")
        # Sampled while stopped: nothing of the job runs between the runner's stop and its kill.
        self.assertRegex(self.section(worker).strip(), rf"^Call graph of {worker}, in state T\S*$")
        # The worker is the deeper of the two: its sample comes first.
        text = self.stacks.read_text(encoding="utf-8")
        self.assertEqual(text.split("\n--- process ")[1].split(" ---")[0], str(worker))
        self.assertEqual(text.count("\n--- process "), 2)

    def test_a_sampler_that_hangs_is_cut_at_the_budget(self):
        """John's finding 4 on #464: the budget was checked between processes only, and each step kept a
        timeout of its own."""
        leader, started, worker = self.command()
        with patch.object(runner, "SAMPLER", self.script("sampler", HANGS)):
            began = time.monotonic()
            text, said = runner.stacks({leader: started, worker: runner.started(worker)}, {}, 1.0)
            took = time.monotonic() - began
        self.assertLess(took, 2.5, "the capture outlasted its budget of 1 s")
        self.assertGreater(took, 0.9, "the samplers were not given the budget")
        self.assertEqual(said, "stacks.txt: 0 of the command's 2 processes at the time limit; why the others are missing is in the file")
        self.assertEqual(text.count("no sample: the sampler ran out of time"), 2)
        self.assertEqual(len(self.samplers()), 2)
        for pid in self.samplers():
            self.assertTrue(gone(pid), "a sampler outlived the capture")
        self.assertTrue(running(worker), "the capture ended the worker: that is end_job's to do")

    def test_a_ps_that_hangs_is_cut_at_the_budget(self):
        """It takes the whole budget, so no process is sampled, and the file says both."""
        leader, started, worker = self.command()
        processes = {leader: started, worker: runner.started(worker)}
        bin = self.dir / "bin"
        bin.mkdir()
        (bin / "ps").write_text(HANGS)
        (bin / "ps").chmod(0o755)
        with patch.object(runner, "SAMPLER", self.script("sampler", SAMPLER)), patch.dict(os.environ, {"PATH": f"{bin}{os.pathsep}{os.environ['PATH']}"}):
            began = time.monotonic()
            text, said = runner.stacks(processes, {}, 1.0)
            took = time.monotonic() - began
        self.assertLess(took, 2.5, "the capture outlasted its budget of 1 s")
        self.assertEqual(said, "stacks.txt: 0 of the command's 2 processes at the time limit; why the others are missing is in the file")
        self.assertIn("ps could not list them: ", text)
        self.assertEqual(text.count("no sample: the capture's 1 s were over"), 2)

    def test_what_reads_the_stopped_processes_cannot_keep_them(self):
        """`end_job` hands its reader every process, stopped, with the state each had before; and the kill
        follows whatever the reader does."""
        leader, started, worker = self.command()
        read = {}

        def reads(processes, states):
            read.update(processes=processes, states=states, now={pid: subprocess.run(["ps", "-o", "stat=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
                                                                 for pid in processes})
            raise KeyError("the reader failed")
        with self.assertRaisesRegex(KeyError, "the reader failed"):
            runner.end_job(leader, started, while_stopped=reads)
        self.assertEqual(read["processes"], {leader: started, worker: runner.started(worker) or read["processes"].get(worker)})
        self.assertEqual({pid: state[:1] for pid, state in read["now"].items()}, {leader: "T", worker: "T"})
        for pid in (leader, worker):
            self.assertRegex(read["states"][pid], r"^[SI]", "a sleeping process, before the stop")
            self.assertTrue(gone(pid), "the reader's failure kept a process")

    def test_a_listing_that_fails_is_said_in_the_verdict(self):
        """`end_job` then kills what it can without a listing, and there is nothing to read."""
        def fails():
            raise RuntimeError("ps exited 1")
        with patch.object(runner, "process_table", fails), open(self.dir / "runner.log", "w") as log:
            job = runner.Job(log, 1)
            with self.assertRaisesRegex(runner.TimedOut, "cleanup error: ps exited 1"):
                job.run([sys.executable, "-c", "import time; time.sleep(30)"], cwd=self.dir)
        self.assertEqual(job.stacks, "no stacks: the processes could not be listed")
        self.assertFalse(self.stacks.exists())

    def test_the_job_is_ended_right_after_a_capture_that_ran_out_of_time(self):
        with patch.object(runner, "SAMPLER", self.script("sampler", HANGS)), patch.object(runner, "STACKS_BUDGET", 1.0):
            job, took, worker = self.at_its_limit(1)
        self.assertLess(took, 8, "the limit of 1 s and the capture's 1 s, then the end")
        self.assertIn("0 of the command's 2 processes", job.stacks)
        self.assertTrue(gone(worker), "the worker outlived the limit")

    def test_a_capture_that_fails_leaves_the_verdict_its_own(self):
        """Whatever goes wrong in it, the job is `timed out`, and its processes are ended."""
        def fails(*_):
            raise UnicodeEncodeError("ascii", "h\u00e9", 1, 2, "ordinal not in range(128)")
        with patch.object(runner, "stacks", fails):
            job, _, worker = self.at_its_limit(1)
        self.assertEqual(job.stacks, "no stacks: UnicodeEncodeError: 'ascii' codec can't encode character '\\xe9' in position 1: ordinal not in range(128)")
        self.assertFalse(self.stacks.exists())
        self.assertTrue(gone(worker), "the worker outlived the limit")

    @unittest.skipUnless(Path("/proc/self/comm").exists(), "a thread is named through /proc here: Linux alone")
    def test_a_thread_name_outside_ascii_under_the_C_locale(self):
        """John's finding 7 on #464: the write raised UnicodeEncodeError there, and the job read `error`."""
        environment = dict(os.environ, LC_ALL="C", PYTHONUTF8="0", PYTHONCOERCECLOCALE="0", PYTHONPATH=str(Path(runner.__file__).parent))
        out = subprocess.run([sys.executable, "-c", C_LOCALE, str(self.dir)], capture_output=True, text=True, env=environment, timeout=60)
        self.assertEqual(out.stdout, "timed out\nstacks.txt: 1 of the command's 1 processes at the time limit\n", out.stderr)
        self.assertRegex(self.stacks.read_text(encoding="utf-8"), "\\(h\u00e9h\u00e9\\): \\d+\\.\\d\\d s of CPU")

    def test_a_command_that_ends_in_time_leaves_no_stacks(self):
        with open(self.dir / "runner.log", "w") as log:
            job = runner.Job(log, 30)
            job.run([sys.executable, "-c", "pass"], cwd=self.dir)
        self.assertIsNone(job.stacks)
        self.assertFalse(self.stacks.exists())

    def test_the_verdict_says_what_the_file_holds(self):
        out = self.dir / "out"
        out.mkdir()
        (self.dir / "tmp").mkdir()
        job_file = self.dir / "0001-demo.json"
        job_file.write_text(json.dumps({"type": "test", "fork_commit": "0" * 40, "time_limit_seconds": 2}))

        def sleeps(job, run, work, out_dir):
            run.run([sys.executable, "-c", "import time; time.sleep(30)"], cwd=self.dir)
            return "passed"
        with patch.object(runner, "HOME", self.dir), patch.dict(runner.HANDLERS, {"test": sleeps}), \
                patch.object(runner, "power", lambda: "AC Power, lowpowermode 0"):
            status = runner.process(job_file, out)
        verdict = json.loads((out / "verdict.json").read_text())
        self.assertEqual((status, verdict["status"]), ("timed out", "timed out"), verdict)
        self.assertEqual(verdict["stacks"], "stacks.txt: 1 of the command's 1 processes at the time limit")
        self.assertTrue((out / "stacks.txt").exists())


if __name__ == "__main__":
    unittest.main()
