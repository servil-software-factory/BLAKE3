"""No process of a job's command outlives the command (#473).

cargo-nextest and cargo-mutants give each process they start a process group of its own. Such a process
leaves the command's group and stays in its session, so ending the group, which the runner did at a job's
time limit, left it running after the job was `timed out`. Each shape here is first shown to outlive that
old way (`os.killpg` of the command's group), so that a test would fail if `end_job` did no more."""
import os
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest.mock import patch

import runner

SLEEP = "import time; time.sleep(30)"
# A worker that leaves the command's process group, as a test started by cargo-nextest does, and then says
# its PID: so the PID is known only once the worker is in a group of its own.
OWN_GROUP = "import os, sys, time; os.setpgid(0, 0); open(sys.argv[1], 'w').write(str(os.getpid())); time.sleep(30)"
# A worker that leaves the command's session as well.
OWN_SESSION = "import os, sys, time; os.setsid(); open(sys.argv[1], 'w').write(str(os.getpid())); time.sleep(30)"
# A process with a zombie under it (a child that has ended and that it does not collect), and the same as
# a worker in a group of its own.
ZOMBIE_UNDER = ("import os, sys, time\n"
                "if os.fork() == 0: os._exit(0)\n"
                "time.sleep(0.3); open(sys.argv[1], 'w').write(str(os.getpid())); time.sleep(30)\n")
WITH_ZOMBIE = "import os\nos.setpgid(0, 0)\n" + ZOMBIE_UNDER
# The command: it starts the worker and waits.
PARENT = "import subprocess, sys, time; subprocess.Popen([sys.executable, '-c', sys.argv[1], sys.argv[2]]); time.sleep(30)"
# The command starts a process that starts the worker and ends at once: the worker's parent is gone.
GRANDPARENT = ("import subprocess, sys, time; "
               "subprocess.Popen([sys.executable, '-c', 'import subprocess, sys; subprocess.Popen([sys.executable, \"-c\", sys.argv[1], sys.argv[2]])', sys.argv[1], sys.argv[2]]); "
               "time.sleep(30)")
# The command starts the worker, waits until the worker has said its PID, and ends by itself with the exit
# code it is given.
LEAVES = ("import os, subprocess, sys, time\n"
          "subprocess.Popen([sys.executable, '-c', sys.argv[1], sys.argv[2]])\n"
          "while not (os.path.exists(sys.argv[2]) and open(sys.argv[2]).read()): time.sleep(0.01)\n"
          "sys.exit(int(sys.argv[3]))\n")
# The command keeps starting short-lived sleepers, each in a session of its own, for at most 4 s (bounded
# so a killed test run can never leak an unbounded fork loop). `sys.argv[1]` names this test's sleepers.
SPAWNER = ("import subprocess, sys, time\n"
           "end = time.monotonic() + 4.0\n"
           "while time.monotonic() < end:\n"
           "    subprocess.Popen(['sleep', sys.argv[1]], start_new_session=True)\n"
           "    time.sleep(0.02)\n")
SHAPES = [("a child in a group of its own", PARENT, OWN_GROUP),
          ("a grandchild in a group of its own, whose parent has ended", GRANDPARENT, OWN_GROUP),
          ("a child that left the session", PARENT, OWN_SESSION)]


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


def sleepers(argument):
    """The PIDs of the processes `sleep ARGUMENT` that are there and no zombies."""
    listing = subprocess.run(["ps", "-A", "-o", "pid=", "-o", "stat=", "-o", "args="], capture_output=True, text=True).stdout
    rows = [line.split() for line in listing.splitlines()]
    return {int(row[0]) for row in rows if len(row) == 4 and not row[1].startswith("Z") and row[2].endswith("sleep") and row[3] == argument}


class Limit(unittest.TestCase):
    def setUp(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        self.dir = Path(folder.name)
        for name, value in (("SESSION_FILE", self.dir / "runner-session.txt"), ("environment", lambda: dict(os.environ))):
            patched = patch.object(runner, name, value)
            patched.start()
            self.addCleanup(patched.stop)
        self.pid_file = self.dir / "worker.pid"
        # 300 s and this test's own number: no other process sleeps for exactly that long
        self.sleep = f"20.{os.getpid():07d}"
        self.started = []

    def tearDown(self):
        for pid in self.started + sorted(sleepers(self.sleep)):
            try:
                os.kill(pid, signal.SIGKILL)
            except OSError:
                pass

    def command(self, *args):
        """Start a command as `Job.run` does: in a session of its own. Its process and its start time."""
        proc = subprocess.Popen([sys.executable, "-c", *args], start_new_session=True)
        self.started.append(proc.pid)
        return proc, runner.started(proc.pid)

    def worker(self, within=10.0):
        """The worker's PID, once it has said it."""
        end = time.monotonic() + within
        while not (self.pid_file.exists() and self.pid_file.read_text()):
            self.assertLess(time.monotonic(), end, "the worker did not start")
            time.sleep(0.02)
        pid = int(self.pid_file.read_text())
        self.started.append(pid)
        return pid

    def test_killing_the_commands_group_leaves_each_shape_running(self):
        """The control: what the runner did before. Then `end_job`, from the command's PID and start time
        alone, ends the worker, also where the command's first process is gone by then."""
        for what, command, worker in SHAPES:
            with self.subTest(what=what):
                self.pid_file.unlink(missing_ok=True)
                proc, began = self.command(command, worker, str(self.pid_file))
                pid = self.worker()
                os.killpg(proc.pid, signal.SIGKILL)
                proc.wait()
                time.sleep(0.3)
                self.assertTrue(running(pid), "the worker ended with the group: this shape tests nothing")
                found, left, err = runner.end_job(proc.pid, began, leader_reaped=True)
                self.assertIsNone(err)
                if worker is OWN_SESSION:
                    # Its parent is gone and it is in no session of the job's: nothing leads to it any more.
                    # (At a job's limit the runner looks before it signals, while the parent is there: the
                    # next test.)
                    self.assertEqual((found, left), ({}, set()))
                    self.assertTrue(running(pid))
                else:
                    self.assertEqual((set(found), left), ({pid}, set()))
                    self.assertTrue(gone(pid), what)

    def test_the_limit_ends_each_shape(self):
        for what, command, worker in SHAPES:
            with self.subTest(what=what):
                self.pid_file.unlink(missing_ok=True)
                with open(self.dir / "runner.log", "w") as log:
                    job = runner.Job(log, 3)
                    began = time.monotonic()
                    with self.assertRaises(runner.TimedOut) as e:
                        job.run([sys.executable, "-c", command, worker, str(self.pid_file)], cwd=self.dir)
                self.assertLess(time.monotonic() - began, 20)
                self.assertTrue(gone(self.worker(within=0.1)), what)        # it had said its PID before the limit
                self.assertNotIn("could not be ended", str(e.exception))
                self.assertEqual(job.left_running, [])

    def test_the_limit_ends_what_a_command_keeps_starting(self):
        """A command that starts a process in a session of its own every few milliseconds. Each process of
        the job is stopped before any is killed, so none is started between the runner's look and its
        signal. (Killed without that, the command leaves the ones it started in between: their parent is
        gone and they are in sessions of their own.)"""
        proc, began = self.command(SPAWNER, self.sleep)
        time.sleep(1.0)
        os.killpg(proc.pid, signal.SIGKILL)                                 # the control: the old way
        proc.wait()
        left = sleepers(self.sleep)
        self.assertGreater(len(left), 10, "the command started too few sleepers: this tests nothing")
        self.assertEqual(runner.end_job(proc.pid, began, leader_reaped=True), ({}, set(), None))      # nothing leads to them any more
        for pid in left:
            os.kill(pid, signal.SIGKILL)
        for _ in range(3):
            with self.subTest("the limit"):
                with open(self.dir / "runner.log", "w") as log:
                    with self.assertRaises(runner.TimedOut) as e:
                        runner.Job(log, 1).run([sys.executable, "-c", SPAWNER, self.sleep], cwd=self.dir)
                self.assertNotIn("could not be ended", str(e.exception))
                time.sleep(0.2)
                self.assertEqual(sleepers(self.sleep), set())

    def test_the_limit_ends_a_command_whose_output_is_captured(self):
        """With its output captured, a worker in its own group that holds stdout is ended at the limit promptly."""
        began = time.monotonic()
        with open(self.dir / "runner.log", "w") as log:
            with self.assertRaises(runner.TimedOut):
                runner.Job(log, 2).run([sys.executable, "-c", PARENT, OWN_GROUP, str(self.pid_file)], cwd=self.dir, capture=True)
        self.assertLess(time.monotonic() - began, 10.0)
        self.assertTrue(gone(self.worker(within=0.1)))

    def test_captured_command_that_exits_and_leaves_worker_holding_stdout_is_cleaned_up_at_exit(self):
        """Finding #8: a captured command that exits cleanly while a worker holds stdout does not wait for the limit."""
        cmd = [sys.executable, "-c", LEAVES, OWN_GROUP, str(self.pid_file), "0"]
        began = time.monotonic()
        with open(self.dir / "runner.log", "w") as log:
            job = runner.Job(log, 15)
            out = job.run(cmd, cwd=self.dir, capture=True)
        self.assertLess(time.monotonic() - began, 8.0)
        self.assertEqual(out, "")
        self.assertTrue(gone(self.worker(within=0.1)))
        self.assertEqual(len(job.left_running), 1)

    def test_what_could_not_be_ended_is_said(self):
        """A process that no signal ends (another account's, say) is named in the verdict's text."""
        real, spared = os.kill, []

        def kill(pid, signum):
            if pid not in spared:                                           # as if no signal reached the spared one
                real(pid, signum)

        with open(self.dir / "runner.log", "w") as log, patch.object(runner.os, "kill", kill):
            # the worker's PID is known a moment after the command starts: noted from another thread
            threading.Thread(target=lambda: spared.append(self.worker()), daemon=True).start()
            with self.assertRaises(runner.TimedOut) as e:
                runner.Job(log, 2).run([sys.executable, "-c", PARENT, OWN_GROUP, str(self.pid_file)], cwd=self.dir)
        self.assertEqual(len(spared), 1)
        self.assertIn(f"; 1 of its processes could not be ended and may still be running: PID {spared[0]}", str(e.exception))
        self.assertTrue(running(spared[0]))

    def test_a_command_that_ends_and_leaves_a_process_running(self):
        """The command ends by itself, with success or with failure, and a process it started is still
        there: the runner ends it and says so in the log and for the verdict. A zombie is not counted."""
        for code, worker in ((0, OWN_GROUP), (3, OWN_GROUP), (0, WITH_ZOMBIE)):
            with self.subTest(code=code, zombie=worker is WITH_ZOMBIE):
                self.pid_file.unlink(missing_ok=True)
                cmd = [sys.executable, "-c", LEAVES, worker, str(self.pid_file), str(code)]
                with open(self.dir / "runner.log", "w") as log:
                    job = runner.Job(log, 30)
                    if code:
                        with self.assertRaises(subprocess.CalledProcessError):
                            job.run(cmd, cwd=self.dir)
                    else:
                        job.run(cmd, cwd=self.dir)
                    job.run([sys.executable, "-c", "pass"], cwd=self.dir)   # a command that leaves nothing says nothing
                self.assertTrue(gone(self.worker(within=0.1)))
                said = f"{' '.join(cmd)} ended and left 1 of its processes running; the runner ended them"
                self.assertEqual(job.left_running, [said])
                self.assertEqual((self.dir / "runner.log").read_text().count(f"runner: {said}\n"), 1)

    def test_a_zombie_is_not_counted(self):
        """`end_job` says what it found running: the command, and not the zombie under it."""
        proc, began = self.command(ZOMBIE_UNDER, str(self.pid_file))
        self.assertEqual(self.worker(), proc.pid)
        found, left, err = runner.end_job(proc.pid, began)
        self.assertIsNone(err)
        self.assertEqual((found, left), ({proc.pid: began}, set()))
        proc.wait(timeout=10)

    def test_a_later_runner_ends_what_an_earlier_one_left(self):
        """`end_leftover_session`: ends a recorded live session whose leader matches its start time, and
        refuses to signal when the leader is gone, mismatched, or invalid (Findings #1, #9)."""
        proc, began = self.command(PARENT, OWN_GROUP, str(self.pid_file))
        # as the runner before this change wrote it: ps's own spacing
        runner.SESSION_FILE.write_text(f"{proc.pid}\t{began.replace(' ', '  ')}   \n")
        pid = self.worker()
        runner.end_leftover_session()
        proc.wait(timeout=10)
        self.assertTrue(gone(pid))
        self.assertFalse(runner.SESSION_FILE.exists())

        # Stale record whose leader is already gone (or PID reused by an unrelated leaderless session):
        # must NOT signal the unrelated session's member (Finding #1).
        self.pid_file.unlink(missing_ok=True)
        unrelated, _ = self.command(PARENT, OWN_GROUP, str(self.pid_file))
        member = self.worker()
        os.kill(unrelated.pid, signal.SIGKILL)
        unrelated.wait()
        self.assertTrue(running(member))
        runner.SESSION_FILE.write_text(f"{unrelated.pid}\tThu Jan 1 00:00:00 1970\n")
        runner.end_leftover_session()
        time.sleep(0.2)
        self.assertTrue(running(member), "stale leaderless record must not kill unrelated session")
        self.assertFalse(runner.SESSION_FILE.exists())
        os.kill(member, signal.SIGKILL)

        # Mismatched start time for a live leader: must not signal it.
        other, _ = self.command(SLEEP)
        runner.SESSION_FILE.write_text(f"{other.pid}\tThu Jan 1 00:00:00 1970\n")
        runner.end_leftover_session()
        time.sleep(0.2)
        self.assertTrue(running(other.pid))
        self.assertFalse(runner.SESSION_FILE.exists())
        other.kill()
        other.wait()

        # Damaged record (PID 0 or malformed): safely ignored.
        for bad in ("0\tThu Jan 1 00:00:00 1970\n", "not-a-record\n", "-5\tNone\n"):
            runner.SESSION_FILE.write_text(bad)
            runner.end_leftover_session()
            self.assertFalse(runner.SESSION_FILE.exists())


    def test_clean_run_clears_session_file_and_process_records_left_running_in_verdict(self):
        """Findings #1 and #10: clean run removes SESSION_FILE; process() writes left running to verdict.json."""
        import json
        out = self.dir / "out"
        out.mkdir()
        job_file = self.dir / "0001-demo.json"
        job_file.write_text(json.dumps({"type": "test", "fork_commit": "0" * 40}))
        cmd = [sys.executable, "-c", LEAVES, OWN_GROUP, str(self.pid_file), "0"]
        orig_home = runner.HOME
        orig_handler = runner.HANDLERS["test"]
        try:
            runner.HOME = self.dir
            (self.dir / "tmp").mkdir(exist_ok=True)
            def fake_test(job, run, work, out_dir):
                run.run(cmd, cwd=self.dir)
                return "passed"
            runner.HANDLERS["test"] = fake_test
            with patch.object(runner, "power", lambda: "AC Power, lowpowermode 0"):
                status = runner.process(job_file, out)
            verdict = json.loads((out / "verdict.json").read_text())
            self.assertEqual(status, "passed", verdict)
            self.assertFalse(runner.SESSION_FILE.exists())
            self.assertIn("left running", verdict)
            self.assertEqual(len(verdict["left running"]), 1)
            self.assertTrue(gone(self.worker(within=0.1)))
        finally:
            runner.HOME = orig_home
            runner.HANDLERS["test"] = orig_handler

    def test_ps_failure_falls_back_to_killpg_and_never_passes_silently(self):
        """Finding #3 & Gemini #1/#2: if `ps` fails at timeout, killpg still kills the job and TimedOut
        is still raised; if `ps` fails after a clean exit, the job errors rather than passing silently."""
        with open(self.dir / "runner.log", "w") as log,              patch.object(runner, "process_table", side_effect=RuntimeError("ps failed: EAGAIN")):
            with self.assertRaises(runner.TimedOut) as e:
                runner.Job(log, 1).run([sys.executable, "-c", SLEEP], cwd=self.dir)
            self.assertIn("cleanup error: ps failed: EAGAIN", str(e.exception))

        with open(self.dir / "runner.log", "w") as log, patch.object(runner, "process_table", side_effect=RuntimeError("ps failed")):
            with self.assertRaises(RuntimeError) as e:
                runner.Job(log, 10).run([sys.executable, "-c", "pass"], cwd=self.dir)
            self.assertIn("cleanup could not verify process table", str(e.exception))

        # Gemini #2: ps succeeds during stop pass then fails during kill pass -> fallback kills stopped worker.
        self.pid_file.unlink(missing_ok=True)
        proc, began = self.command(PARENT, OWN_GROUP, str(self.pid_file))
        wpid = self.worker()
        real_pt = runner.process_table
        calls = {"n": 0}
        def flaky_pt():
            calls["n"] += 1
            if calls["n"] >= 3:
                raise RuntimeError("ps interrupted")
            return real_pt()
        with patch.object(runner, "process_table", side_effect=flaky_pt):
            _found, _left, err = runner.end_job(proc.pid, began, leader_reaped=False)
        self.assertIn("ps interrupted", err)
        proc.wait(timeout=10)
        self.assertTrue(gone(wpid), "worker must not be left stopped (T) when ps fails mid-cleanup")

    def test_known_survivor_after_clean_exit_stops_job_with_error(self):
        """Finding #6: a survivor after a command exits must raise an error (not let the next command run)
        and must not falsely claim 'the runner ended them'."""
        real, spared = os.kill, []

        def kill(pid, signum):
            if pid not in spared:
                real(pid, signum)

        cmd = [sys.executable, "-c", LEAVES, OWN_GROUP, str(self.pid_file), "0"]
        with open(self.dir / "runner.log", "w") as log, patch.object(runner.os, "kill", kill):
            threading.Thread(target=lambda: spared.append(self.worker()), daemon=True).start()
            job = runner.Job(log, 15)
            with self.assertRaises(RuntimeError) as e:
                job.run(cmd, cwd=self.dir)
            self.assertIn("could not be ended", str(e.exception))
            self.assertEqual(len(job.left_running), 1)
            self.assertNotIn("the runner ended them", job.left_running[0])
            self.assertIn("ended 0", job.left_running[0])
        self.assertTrue(running(spared[0]))

    def test_child_in_new_session_of_in_session_parent_is_killed_when_leader_exits(self):
        """Finding #4: a worker in a new session whose parent is in the job's session is still found and
        killed after the command's leader has exited."""
        # Leader spawns mid (stays in session, sleeps 30s) which spawns leaf (own session, writes pid).
        # Leader exits once leaf's pid is written.
        leaf = OWN_SESSION
        mid = (
            "import subprocess, sys, time\n"
            "subprocess.Popen([sys.executable, '-c', sys.argv[1], sys.argv[2]])\n"
            "time.sleep(30)\n"
        )
        leader_script = (
            "import os, subprocess, sys, time\n"
            "subprocess.Popen([sys.executable, '-c', sys.argv[1], sys.argv[2], sys.argv[3]])\n"
            "while not (os.path.exists(sys.argv[3]) and open(sys.argv[3]).read()): time.sleep(0.01)\n"
        )
        with open(self.dir / "runner.log", "w") as log:
            job = runner.Job(log, 15)
            job.run([sys.executable, "-c", leader_script, mid, leaf, str(self.pid_file)], cwd=self.dir)
        leaf_pid = self.worker(within=0.1)
        self.assertTrue(gone(leaf_pid), "new-session child of in-session parent must be ended")


class Table(unittest.TestCase):
    """`job_processes` and `end_job` on tables written here. The PIDs are above any that a system gives out,
    so no real process answers for their sessions."""

    def test_descendants_zombies_reaped_leader_and_invalid_pids(self):
        base = 10 ** 8
        leader, child, grandchild, zombie, other, loop_a, loop_b = range(base, base + 7)
        table = {
            leader: (1, "S", "t0"),
            child: (leader, "S", "t1"),
            grandchild: (child, "S", "t2"),
            zombie: (child, "Z", "t3"),
            other: (1, "S", "t4"),
            loop_a: (loop_b, "S", "t5"),
            loop_b: (loop_a, "S", "t6"),
        }
        under = {child: ("t1", False), grandchild: ("t2", False), zombie: ("t3", True)}
        self.assertEqual(runner.job_processes(leader, "t0", table), {leader: ("t0", False), **under})
        self.assertEqual(runner.job_processes(leader, None, table), {leader: ("t0", False), **under})
        # Another process has the leader PID with a different start time: nothing is the job's.
        self.assertEqual(runner.job_processes(leader, "another time", table), {})
        # Reaped leader (Gemini #3): any row at `leader` in the table is a reused PID and must be ignored.
        self.assertEqual(runner.job_processes(leader, "t0", table, leader_reaped=True), {})
        # Leader missing (not reaped) or zombie: parent links still find descendants.
        self.assertEqual(runner.job_processes(leader, "t0", {pid: row for pid, row in table.items() if pid != leader}), under)
        self.assertEqual(runner.job_processes(leader, "t0", {**table, leader: (1, "Z", "t0")}), {leader: ("t0", True), **under})
        # Invalid leader PID <= 0 (Finding #9): returns empty.
        self.assertEqual(runner.job_processes(0, "t0", {0: (0, "S", "t0"), 1: (0, "S", "t1")}), {})
        self.assertEqual(runner.job_processes(-1, "t0", table), {})
        # This very process is never the job's.
        self.assertNotIn(os.getpid(), runner.job_processes(leader, "t0", {**table, os.getpid(): (child, "S", "t7")}))

    def test_stop_pass_waits_for_T_state_and_kill_checks_start_time(self):
        """Findings #2 and #5: stop pass keeps signalling until state is T; kill pass skips a reused PID
        whose start time changed."""
        base = 10 ** 8
        leader, worker = base, base + 1
        # Sequence of tables:
        # 1. stop round 1: leader & worker in 'S' -> sends SIGSTOP to both
        # 2. stop round 2: leader in 'T', worker still 'S' -> sends SIGSTOP to worker again
        # 3. stop round 3: both in 'T' -> stop pass finishes
        # 4. kill round 1: leader in 'T', worker PID reused with new start time 't_new' -> SIGKILL leader only
        # 5. kill round 2: leader gone, worker is 't_new' -> kill pass finishes with left=set()
        tables = iter([
            {leader: (1, "S", "t0"), worker: (leader, "S", "t1")},
            {leader: (1, "T", "t0"), worker: (leader, "S", "t1")},
            {leader: (1, "T", "t0"), worker: (leader, "T", "t1")},
            {leader: (1, "T", "t0"), worker: (1, "S", "t_new")},
            {worker: (1, "S", "t_new")},
        ])
        signals = []
        with patch.object(runner, "process_table", side_effect=lambda: next(tables)),              patch.object(runner.os, "kill", side_effect=lambda pid, sig: signals.append((pid, sig))),              patch.object(runner.time, "sleep"):
            found, left, err = runner.end_job(leader, "t0")
        self.assertIsNone(err)
        self.assertEqual(left, set())
        self.assertEqual(found, {leader: "t0", worker: "t1"})
        # Worker got SIGSTOP twice (rounds 1 and 2) and NEVER got SIGKILL after its start time changed.
        self.assertEqual(signals.count((worker, signal.SIGSTOP)), 2)
        self.assertNotIn((worker, signal.SIGKILL), signals)
        self.assertIn((leader, signal.SIGKILL), signals)


if __name__ == "__main__":
    unittest.main()
