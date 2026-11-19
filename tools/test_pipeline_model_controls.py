#!/usr/bin/env python3
"""tools/pipeline_model_controls.py plants its faults in a copy, and
leaves the tree alone.

    python3 tools/test_pipeline_model_controls.py

These tests run a copy of the script in a small tree of their own (a Git
checkout with a source that holds each fault's place), with a stand-in
for Cargo that passes on the source as it was and, for a planted fault,
fails, passes, or never ends, as each test asks: every fault caught, a
fault missed, a model that fails with no fault, a build Cargo did not
make anew, a run with no test in it, a fault's place gone, a
file saved to the tree while a run goes on and while the copy is made,
a run stopped by each signal it handles, a run killed outright and what
it left running, two runs at once, and what the copy holds from one run
to the next. They build nothing and take a second or two.
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

# Run in the copy's threads/ as `cargo test ... --test shuttle_pipeline`.
CARGO = """#!/bin/sh
echo "$(pwd) $CARGO_TARGET_DIR" >> "$CONTROLS_RAN_IN"
passes() { echo "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out"; exit 0; }
compiles() { echo "   Compiling blake3-servil v0.0.0 ($(pwd))" >&2; }
if cmp -s src/pipeline.rs "$CONTROLS_ORIGINAL"; then
    compiles
    echo "running 1 test"
    [ "${CONTROLS_AS_IT_IS:-0}" = 0 ] && passes
    exit "$CONTROLS_AS_IT_IS"
fi
# Cargo takes the fault's source for unchanged: the build before it runs.
[ "$CONTROLS_FAULT" = stale ] || compiles
[ "$CONTROLS_FAULT" = runs-no-test ] && { echo "running 0 tests"; exit 0; }
echo "running 1 test"
case "$CONTROLS_FAULT" in
caught|stale) exit 101 ;;
missed) passes ;;
allocator)
    echo "shuttle_pipeline: messages_through_rooms_complete_with_their_own_digests" >&2
    echo "free(): invalid pointer" >&2
    echo "error: test failed (signal: 6, SIGABRT: process abort signal)" >&2
    exit 101
    ;;
saves)
    # Someone saves the tree's source while a fault's model runs.
    echo "// saved during the run" >> "$CONTROLS_TREE/threads/src/pipeline.rs"
    exit 101
    ;;
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


def script(path=SCRIPT):
    """The script at `path`, as a module."""
    spec = importlib.util.spec_from_file_location("pipeline_model_controls", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def faults():
    return script().FAULTS


def alive(pid):
    """Whether process `pid` still runs: one that has ended and waits for
    its parent to read its status (where process 1 reads none) has not."""
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    try:
        with open(f"/proc/{pid}/stat") as f:
            return f.read().rsplit(")", 1)[1].split()[0] != "Z"
    except OSError:
        return True


def files_under(folder):
    """Every file under the folder: its path from there, bytes and time;
    for a symbolic link, where it points."""
    found = {}
    for where, _, names in os.walk(folder):
        for name in names:
            path = os.path.join(where, name)
            if os.path.islink(path):
                found[os.path.relpath(path, folder)] = (b"a link to " + os.fsencode(os.readlink(path)), os.lstat(path).st_mtime_ns)
                continue
            with open(path, "rb") as f:
                found[os.path.relpath(path, folder)] = (f.read(), os.stat(path).st_mtime_ns)
    return found


class Controls(unittest.TestCase):
    def setUp(self):
        self.base = tempfile.mkdtemp(prefix="model-controls-")
        self.addCleanup(shutil.rmtree, self.base, True)
        # The tree: a checkout, with a build directory that Git ignores.
        self.root = os.path.join(self.base, "tree")
        for d in ("tools", "threads/src"):
            os.makedirs(os.path.join(self.root, d))
        os.makedirs(os.path.join(self.base, "bin"))
        subprocess.run(["git", "init", "-q", self.root], check=True)
        self.script = os.path.join(self.root, "tools", "pipeline_model_controls.py")
        shutil.copy(SCRIPT, self.script)
        self.source = os.path.join(self.root, "threads", "src", "pipeline.rs")
        # A source that holds each fault's place once, as the Pipeline's does.
        self.original = "".join(f"// The place of fault {i}.\n{find}\n" for i, (_, find, _) in enumerate(faults()))
        self.write("threads/src/pipeline.rs", self.original)
        self.write(".gitignore", "target\nCargo.lock\n")
        self.write("Cargo.lock", "# the core's lock\n")
        self.write("threads/Cargo.lock", "# the pool's lock\n")
        self.write("threads/Cargo.toml", "# a manifest\n")
        self.as_it_was = os.path.join(self.base, "as-it-was.rs")
        with open(self.as_it_was, "w", newline="") as f:
            f.write(self.original)
        cargo = os.path.join(self.base, "bin", "cargo")
        with open(cargo, "w") as f:
            f.write(CARGO)
        os.chmod(cargo, 0o755)
        self.running = os.path.join(self.base, "running")
        self.ran_in = os.path.join(self.base, "ran-in")
        self.copy = os.path.join(self.root, "target", "pipeline-model-controls", "tree")
        self.before = files_under(self.root)

    def write(self, name, text):
        with open(os.path.join(self.root, name), "w", newline="") as f:
            f.write(text)

    def env(self, fault, **more):
        env = {**os.environ, "PATH": os.path.join(self.base, "bin") + os.pathsep + os.environ["PATH"], "CONTROLS_ORIGINAL": self.as_it_was, "CONTROLS_FAULT": fault, "CONTROLS_RUNNING": self.running, "CONTROLS_RAN_IN": self.ran_in, "CONTROLS_TREE": self.root}
        # The tree's own build directory, unless the test names another.
        env.pop("CARGO_TARGET_DIR", None)
        return {**env, **more}

    def run_script(self, fault, *args, **more):
        return subprocess.run([sys.executable, self.script, *args], env=self.env(fault, **more), capture_output=True, text=True, timeout=120)

    def tree_now(self):
        """The tree's files but the build directory's and Git's own."""
        return {name: file for name, file in files_under(self.root).items() if not name.startswith(("target" + os.sep, ".git" + os.sep))}

    def assert_untouched(self):
        """No file of the tree was written, added or removed: the same bytes, at the same times."""
        before = {name: file for name, file in self.before.items() if not name.startswith(".git" + os.sep)}
        self.assertEqual(self.tree_now(), before, "the run changed the tree")

    def copy_source(self):
        with open(os.path.join(self.copy, "threads", "src", "pipeline.rs"), newline="") as f:
            return f.read()

    def wait_for_a_fault(self, run):
        """Wait until a planted fault's model is running under `run`; the model's process."""
        deadline = time.time() + 60
        while not os.path.exists(self.running):
            self.assertIsNone(run.poll(), "the run ended before a fault's model ran")
            self.assertLess(time.time(), deadline, "no fault's model ran within a minute")
            time.sleep(0.01)
        with open(self.running) as f:
            model = int(f.read())
        os.remove(self.running)
        # The stand-in's model, if a test leaves it running: by its number only while it still runs.
        self.addCleanup(lambda: alive(model) and subprocess.run(["kill", "-KILL", str(model)], capture_output=True))
        self.assertNotEqual(self.copy_source(), self.original, "the model ran, and no fault is planted in the copy")
        return model

    def end_during_a_fault(self, signum):
        """Start a run, wait until a planted fault's model is running, send the signal; the run's exit status and what it wrote."""
        run = subprocess.Popen([sys.executable, self.script], env=self.env("never-ends"), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        model = self.wait_for_a_fault(run)
        run.send_signal(signum)
        out, err = run.communicate(timeout=60)
        return run.returncode, out + err, model

    def test_every_fault_is_planted_in_the_copy_and_the_tree_is_untouched(self):
        run = self.run_script("caught")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertEqual(run.stdout.count("caught  "), len(faults()))
        self.assertIn(f"fails for each of {len(faults())} planted faults", run.stdout)
        self.assert_untouched()
        # Every build, the one with no fault included, ran in the copy,
        # into a build directory that is the copy's own.
        with open(self.ran_in) as f:
            ran_in = f.read().split("\n")[:-1]
        home = os.path.realpath(os.path.dirname(self.copy))
        self.assertEqual(ran_in, [f"{os.path.join(home, 'tree', 'threads')} {os.path.join(home, 'target')}"] * (1 + len(faults())))

    def test_a_run_that_cargo_built_nothing_for_is_no_verdict(self):
        # A fault's source that Cargo takes for unchanged runs the build
        # before it: the fault before it, which failed. Each such run
        # counted as its own fault caught.
        run = self.run_script("stale")
        self.assertNotEqual(run.returncode, 0)
        self.assertIn("Cargo compiled nothing of blake3-servil for this run", run.stderr)
        self.assertNotIn("caught  ", run.stdout)
        self.assert_untouched()

    def test_a_run_that_ends_well_without_the_models_test_is_no_verdict(self):
        run = self.run_script("runs-no-test")
        self.assertNotEqual(run.returncode, 0)
        self.assertIn("the model's one test did not run", run.stderr)
        self.assertNotIn("MISSED  ", run.stdout)
        self.assert_untouched()

    def test_each_faults_run_is_kept_whole_and_the_allocators_stop_is_named(self):
        run = self.run_script("allocator")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        lines = [line for line in run.stdout.splitlines() if line.startswith("caught  ")]
        self.assertEqual(len(lines), len(faults()))
        for (name, _, _), line in zip(faults(), lines):
            self.assertEqual(line, f"caught  {name} (in messages_through_rooms_complete_with_their_own_digests: the allocator stopped the process: free(): invalid pointer)")
        logs = os.path.join(os.path.dirname(self.copy), "logs")
        self.assertEqual(sorted(os.listdir(logs)), [f"{n:02}.log" for n in range(len(faults()))])
        for n, (name, _, _) in enumerate(faults()):
            with open(os.path.join(logs, f"{n:02}.log")) as f:
                said = f.read()
            self.assertTrue(said.startswith(name + "\n"), said[:200])
            self.assertIn("free(): invalid pointer", said)
        # The next run's logs are its own.
        run = self.run_script("caught")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        with open(os.path.join(logs, "00.log")) as f:
            self.assertNotIn("free(): invalid pointer", f.read())
        self.assert_untouched()

    def test_a_fault_the_model_passes_fails_the_run(self):
        run = self.run_script("missed")
        self.assertNotEqual(run.returncode, 0)
        self.assertEqual(run.stdout.count("MISSED  "), len(faults()))
        self.assertIn(f"passes {len(faults())} planted faults", run.stderr)
        self.assert_untouched()

    def test_a_model_that_fails_with_no_fault_fails_the_run(self):
        run = self.run_script("caught", CONTROLS_AS_IT_IS="101")
        self.assertNotEqual(run.returncode, 0)
        self.assertIn("the model fails on the Pipeline as it is", run.stderr)
        self.assertNotIn("caught  ", run.stdout)
        self.assert_untouched()

    def test_a_fault_whose_place_is_gone_stops_the_run_before_any_build(self):
        _, find, _ = faults()[3]
        self.write("threads/src/pipeline.rs", self.original.replace(find, "// rewritten\n"))
        self.before = files_under(self.root)
        run = self.run_script("caught")
        self.assertNotEqual(run.returncode, 0)
        self.assertIn("is not there once: update this script", run.stderr)
        self.assertFalse(os.path.exists(self.ran_in), "a build ran")
        self.assert_untouched()

    def test_what_is_saved_to_the_tree_during_a_run_stays(self):
        # Under each fault's model the stand-in adds a line to the tree's
        # source, as an editor or another agent would. The run neither
        # tests those lines nor writes over them.
        run = self.run_script("saves")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        with open(self.source) as f:
            self.assertEqual(f.read(), self.original + "// saved during the run\n" * len(faults()))

    def test_a_stopped_run_stops_the_model(self):
        for signum in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
            with self.subTest(signal=signum.name):
                status, said, model = self.end_during_a_fault(signum)
                self.assertEqual(status, 1, said)
                self.assertIn(f"stopped by signal {int(signum)}", said)
                self.assert_untouched()
                # The model's process, a child of Cargo's, is gone too.
                deadline = time.time() + 10
                while alive(model) and time.time() < deadline:
                    time.sleep(0.01)
                self.assertFalse(alive(model), "the model's process outlived the stopped run")

    def test_a_killed_run_leaves_the_tree_alone_and_the_next_run_is_whole(self):
        status, said, self.model = self.end_during_a_fault(signal.SIGKILL)
        self.assertEqual(status, -signal.SIGKILL, said)
        self.visitor = subprocess.Popen(["sleep", "600"], cwd=os.path.join(self.copy, "threads"))
        self.addCleanup(lambda: (self.visitor.kill(), self.visitor.wait()))
        # Nothing could clean up: the fault is still in the copy. The tree never held it.
        self.assertNotEqual(self.copy_source(), self.original)
        self.assert_untouched()
        # Nor could it stop its model, which runs on in the copy (the
        # stand-in's, as a build or a model of Cargo's would).
        self.assertTrue(alive(self.model), "the killed run's model is gone already: this test shows nothing")
        planted_at = os.stat(os.path.join(self.copy, "threads", "src", "pipeline.rs")).st_mtime_ns
        os.remove(self.ran_in)
        run = self.run_script("caught")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertIn(f"fails for each of {len(faults())} planted faults", run.stdout)
        self.assert_untouched()
        if os.path.isdir("/proc"):
            self.assertFalse(alive(self.model), "the next run left the killed run's model at work in the copy")
            # A process of someone else's whose working directory is the
            # copy (a shell there, an editor) is none of the run's.
            self.assertTrue(alive(self.visitor.pid), "the run ended a process that was not its own")
        # Its first build was of the source as the tree has it, written after the fault was:
        # Cargo would take a file no newer than the fault's build for the fault still.
        with open(self.ran_in) as f:
            self.assertEqual(len(f.read().split("\n")) - 1, 1 + len(faults()))
        self.assertGreater(os.stat(os.path.join(self.copy, "threads", "src", "pipeline.rs")).st_mtime_ns, planted_at)

    def test_a_second_run_waits_for_the_first(self):
        first = subprocess.Popen([sys.executable, self.script], env=self.env("never-ends"), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.wait_for_a_fault(first)
        second = subprocess.Popen([sys.executable, self.script], env=self.env("caught"), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        # It says that it waits, and builds nothing while the first run's fault is planted.
        self.assertIn("waiting for it", second.stdout.readline())
        with open(self.ran_in) as f:
            builds = f.read()
        self.assertIsNone(second.poll())
        with open(self.ran_in) as f:
            self.assertEqual(f.read(), builds, "the second run built while the first held the copy")
        first.send_signal(signal.SIGTERM)
        first.communicate(timeout=60)
        out, err = second.communicate(timeout=60)
        self.assertEqual(second.returncode, 0, out + err)
        self.assertIn(f"fails for each of {len(faults())} planted faults", out)
        self.assert_untouched()

    def test_the_copy_holds_what_git_knows_or_would_add_and_the_lock_files(self):
        self.write("threads/src/new.rs", "// not yet added to Git\n")
        os.makedirs(os.path.join(self.root, "target", "release"))
        self.write("target/release/built", "ignored: a build's output\n")
        self.write("threads/src/crlf.rs", "// a line that ends in a carriage return and a line feed\r\n")
        os.chmod(os.path.join(self.root, "threads", "Cargo.toml"), 0o755)
        os.symlink("threads/Cargo.toml", os.path.join(self.root, "a-link"))
        os.symlink("nowhere", os.path.join(self.root, "a-link-to-nothing"))
        self.write("extra", "a file, for now\n")
        subprocess.run(["git", "-C", self.root, "add", "threads/Cargo.toml", "a-link"], check=True)
        run = self.run_script("caught")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        source = os.path.join("threads", "src", "pipeline.rs")
        tree = {name: file[0] for name, file in self.tree_now().items() if name != source}
        copy = {name: file[0] for name, file in files_under(self.copy).items() if name != source}
        self.assertEqual(copy, tree)
        self.assertIn("Cargo.lock", copy)
        self.assertIn(os.path.join("threads", "Cargo.lock"), copy)
        self.assertEqual(os.stat(os.path.join(self.copy, "threads", "Cargo.toml")).st_mode & 0o777, 0o755)
        # A link is a link in the copy too, whatever it points at.
        for link, to in (("a-link", "threads/Cargo.toml"), ("a-link-to-nothing", "nowhere")):
            self.assertEqual(os.readlink(os.path.join(self.copy, link)), to)
        # The next run makes the copy anew: a file that left the tree is
        # gone, a file that became a folder is a folder (and the reverse),
        # what was left in the copy is gone, and every file is written
        # again, newer than any build of the run before.
        times = {name: file[1] for name, file in files_under(self.copy).items()}
        os.remove(os.path.join(self.root, "threads", "src", "new.rs"))
        os.remove(os.path.join(self.root, "extra"))
        os.mkdir(os.path.join(self.root, "extra"))
        self.write("extra/file", "a folder now\n")
        os.remove(os.path.join(self.root, "threads", "src", "crlf.rs"))
        with open(os.path.join(self.copy, "left-by-someone"), "w") as f:
            f.write("not the tree's\n")
        run = self.run_script("caught")
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        after = files_under(self.copy)
        self.assertEqual({name: file[0] for name, file in after.items() if name != source}, {name: file[0] for name, file in self.tree_now().items() if name != source})
        self.assertIn(os.path.join("extra", "file"), after)
        for name in (".gitignore", "Cargo.lock", os.path.join("threads", "Cargo.toml")):
            self.assertGreater(after[name][1], times[name], f"{name} was not written again")

    def test_a_file_saved_while_the_copy_is_made_stops_the_run(self):
        # The tree is not held still while it is read: a file saved then
        # may be taken half-written, or beside files of an earlier moment.
        controls = script(self.script)
        copy = os.path.join(self.base, "a-copy")

        def save(name):
            if name == os.path.join("threads", "Cargo.toml"):
                self.write("threads/Cargo.toml", "# a manifest, saved while the copy was made\n")

        with self.assertRaises(SystemExit) as stop:
            controls.make_copy(copy, read=save)
        self.assertIn("the tree changed while it was copied (threads/Cargo.toml)", str(stop.exception))
        # And a file added then.
        with self.assertRaises(SystemExit) as stop:
            controls.make_copy(copy, read=lambda name: self.write("added-meanwhile", "new\n"))
        self.assertIn("the tree changed while it was copied", str(stop.exception))
        # With the tree at rest, the copy is made.
        controls.make_copy(copy)
        self.assertTrue(os.path.isfile(os.path.join(copy, "added-meanwhile")))

    def test_the_copy_lies_in_the_build_directory_in_use(self):
        builds = os.path.join(self.base, "builds")
        run = self.run_script("caught", CARGO_TARGET_DIR=builds)
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertTrue(os.path.isfile(os.path.join(builds, "pipeline-model-controls", "tree", "threads", "src", "pipeline.rs")))
        self.assertFalse(os.path.exists(os.path.join(self.root, "target")))
        self.assert_untouched()
        # A build directory in the tree that Git does not ignore: the copy holds no copy of itself.
        inside = os.path.join(self.root, "builds-here")
        for _ in range(2):
            run = self.run_script("caught", CARGO_TARGET_DIR=inside)
            self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertFalse(os.path.lexists(os.path.join(inside, "pipeline-model-controls", "tree", "builds-here")))

    def test_an_argument_is_refused(self):
        run = self.run_script("caught", "--put-back")
        self.assertNotEqual(run.returncode, 0)
        self.assertIn("takes no argument", run.stderr)
        self.assertFalse(os.path.exists(self.ran_in))


if __name__ == "__main__":
    unittest.main()
