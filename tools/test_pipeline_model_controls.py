#!/usr/bin/env python3
"""tools/pipeline_model_controls.py leaves the source as it found it.

    python3 tools/test_pipeline_model_controls.py

The script plants each fault in threads/src/pipeline.rs where the file
lies. These tests run a copy of it in a directory of their own, with a
stand-in for Cargo that passes on the source as it was and, for a planted
fault, fails, passes, or never ends, as each test asks: every fault
caught, a fault missed, a run stopped by each signal it handles, and a
run killed outright, which only the next run can mend. They build
nothing and take a second or two.
"""

import importlib.util
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest

TOOLS = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(TOOLS, "pipeline_model_controls.py")

# Run in threads/ as `cargo test ... --test shuttle_pipeline`.
CARGO = """#!/bin/sh
echo "running 1 test"
cmp -s src/pipeline.rs "$CONTROLS_ORIGINAL" && exit 0
case "$CONTROLS_FAULT" in
caught) exit 101 ;;
missed) exit 0 ;;
never-ends)
    # The model is a child of Cargo's, as the test binary is.
    sleep 600 &
    echo $! > "$CONTROLS_RUNNING.part"
    mv "$CONTROLS_RUNNING.part" "$CONTROLS_RUNNING"
    wait
    ;;
esac
exit 2
"""


def faults():
    spec = importlib.util.spec_from_file_location("pipeline_model_controls", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.FAULTS


def alive(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    return True


class Controls(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="model-controls-")
        self.addCleanup(shutil.rmtree, self.root, True)
        for d in ("tools", "threads/src", "bin"):
            os.makedirs(os.path.join(self.root, d))
        self.script = os.path.join(self.root, "tools", "pipeline_model_controls.py")
        shutil.copy(SCRIPT, self.script)
        self.source = os.path.join(self.root, "threads", "src", "pipeline.rs")
        self.kept = self.source + ".before-fault"
        # A source that holds each fault's place once, as the Pipeline's does.
        self.original = "".join(f"// The place of fault {i}.\n{find}\n" for i, (_, find, _) in enumerate(faults()))
        with open(self.source, "w") as f:
            f.write(self.original)
        self.as_it_was = os.path.join(self.root, "as-it-was.rs")
        with open(self.as_it_was, "w") as f:
            f.write(self.original)
        cargo = os.path.join(self.root, "bin", "cargo")
        with open(cargo, "w") as f:
            f.write(CARGO)
        os.chmod(cargo, 0o755)
        self.running = os.path.join(self.root, "running")

    def env(self, fault):
        return {**os.environ, "PATH": os.path.join(self.root, "bin") + os.pathsep + os.environ["PATH"], "CONTROLS_ORIGINAL": self.as_it_was, "CONTROLS_FAULT": fault, "CONTROLS_RUNNING": self.running}

    def run_script(self, fault, *args):
        return subprocess.run([sys.executable, self.script, *args], env=self.env(fault), capture_output=True, text=True, timeout=120)

    def source_now(self):
        with open(self.source) as f:
            return f.read()

    def assert_as_it_was(self):
        self.assertEqual(self.source_now(), self.original, "the source is not as it was")
        self.assertEqual(os.listdir(os.path.dirname(self.source)), ["pipeline.rs"], "a file is left beside the source")

    def end_during_a_fault(self, signum):
        """Start a run, wait until a planted fault's model is running, send the signal; the run's exit status and what it wrote."""
        run = subprocess.Popen([sys.executable, self.script], env=self.env("never-ends"), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        deadline = time.time() + 60
        while not os.path.exists(self.running):
            self.assertIsNone(run.poll(), "the run ended before a fault's model ran")
            self.assertLess(time.time(), deadline, "no fault's model ran within a minute")
            time.sleep(0.01)
        with open(self.running) as f:
            model = int(f.read())
        os.remove(self.running)
        self.addCleanup(lambda: subprocess.run(["kill", "-KILL", str(model)], capture_output=True))
        self.assertNotEqual(self.source_now(), self.original, "the model ran, and no fault is planted")
        run.send_signal(signum)
        out, err = run.communicate(timeout=60)
        return run.returncode, out + err, model

    def test_every_fault_is_planted_and_the_source_is_put_back(self):
        run = self.run_script("caught")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertEqual(run.stdout.count("caught  "), len(faults()))
        self.assertIn(f"fails for each of {len(faults())} planted faults", run.stdout)
        self.assert_as_it_was()

    def test_a_fault_the_model_passes_fails_the_run(self):
        run = self.run_script("missed")
        self.assertNotEqual(run.returncode, 0)
        self.assertEqual(run.stdout.count("MISSED  "), len(faults()))
        self.assert_as_it_was()

    def test_a_stopped_run_stops_the_model_and_puts_the_source_back(self):
        for signum in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
            with self.subTest(signal=signum.name):
                status, said, model = self.end_during_a_fault(signum)
                self.assertEqual(status, 1, said)
                self.assertIn(f"stopped by signal {int(signum)}", said)
                self.assert_as_it_was()
                # The model's process, a child of Cargo's, is gone too.
                deadline = time.time() + 10
                while alive(model) and time.time() < deadline:
                    time.sleep(0.01)
                self.assertFalse(alive(model), "the model's process outlived the stopped run")

    def test_a_killed_run_is_mended_by_put_back(self):
        status, said, _ = self.end_during_a_fault(signal.SIGKILL)
        self.assertEqual(status, -signal.SIGKILL, said)
        # Nothing could put it back: the fault is there, and the source as it was lies beside it.
        self.assertNotEqual(self.source_now(), self.original)
        with open(self.kept) as f:
            self.assertEqual(f.read(), self.original)
        planted_at = os.stat(self.source).st_mtime_ns
        run = self.run_script("caught", "--put-back")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertIn("had left its fault planted", run.stdout)
        self.assert_as_it_was()
        self.assertGreater(os.stat(self.source).st_mtime_ns, planted_at, "the source put back is no newer than the fault: Cargo would keep the fault's build")
        # With nothing planted, --put-back changes nothing and says nothing.
        run = self.run_script("caught", "--put-back")
        self.assertEqual((run.returncode, run.stdout), (0, ""))
        self.assert_as_it_was()

    def test_a_killed_run_is_mended_by_the_next_run(self):
        self.end_during_a_fault(signal.SIGKILL)
        self.assertNotEqual(self.source_now(), self.original)
        run = self.run_script("caught")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertIn("had left its fault planted", run.stdout)
        self.assertIn(f"fails for each of {len(faults())} planted faults", run.stdout)
        self.assert_as_it_was()

    def test_check_builds_refuses_a_tree_with_a_planted_fault(self):
        subprocess.run(["git", "init", "-q", self.root], check=True)
        shutil.copy(os.path.join(TOOLS, "check-builds.sh"), os.path.join(self.root, "tools", "check-builds.sh"))
        with open(self.kept, "w") as f:
            f.write(self.original)
        run = subprocess.run(["sh", "tools/check-builds.sh"], cwd=self.root, capture_output=True, text=True, timeout=60)
        self.assertEqual(run.returncode, 1, run.stdout + run.stderr)
        self.assertIn("holds a planted fault", run.stderr)
        self.assertIn("--put-back", run.stderr)
        self.assertEqual(run.stdout, "", "check-builds ran a step before it refused")


if __name__ == "__main__":
    unittest.main()
