#!/usr/bin/env python3
"""The Pipeline's Shuttle model fails for each of these planted faults.

    python3 tools/pipeline_model_controls.py

Each fault is one edit of threads/src/pipeline.rs that breaks a handing-
over between the caller and the workers. The model
(threads/tests/shuttle_pipeline.rs) is run on the edited file and must
fail. A fault the model passes is a hole in the model, and this exits 1
naming it. The model must also pass on the file as it is.

Takes a build of the model per fault (about half a minute each).

The faults are planted in a copy of the tree, never in the tree. (Until
November 19, 2026 they were planted in the file where it lies. A run
that was stopped half-way left its fault there, and the next check of
that tree failed in the model and looked like a fault of the Pipeline;
a run that put the file back wrote over what had been saved to it
meanwhile; and any build of the tree during a run could compile a
fault.)

The copy holds the files Git knows or would add, and the lock files, as
they are in the working tree when the run begins: what is saved to the
tree during a run is neither tested nor touched. It lies in the build
directory (pipeline-model-controls/ in CARGO_TARGET_DIR, or in target/
here), which Git ignores, and each run brings it up to date. Between
runs it may hold the last fault planted; nothing but this script builds
from it. One run at a time uses a copy: a second run waits for the first.
"""

import fcntl
import os
import signal
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SOURCE = os.path.join("threads", "src", "pipeline.rs")
# Git ignores them, and a build with --locked needs them.
LOCK_FILES = ("Cargo.lock", os.path.join("threads", "Cargo.lock"))

# (name, the text to find once, what replaces it)
FAULTS = [
    (
        "a group is published done before its members are hashed",
        "        task.run_members(self.tree(), platform);\n",
        "        group.completed_epoch.swap(epoch, Ordering::SeqCst);\n        task.run_members(self.tree(), platform);\n",
    ),
    (
        "a finished task wakes no one",
        "    publish();\n    storage.waker.notify_after_seqcst();\n",
        "    publish();\n",
    ),
    (
        "a room piece is published done before it is hashed",
        "        unsafe { *pnode.out.0.get() = value };",
        "        pnode.done.swap(true, Ordering::SeqCst);\n        unsafe { *pnode.out.0.get() = value };",

    ),
    (
        "a room piece gives its room back before it has read it",
        "        let ptr = unsafe { rooms.room_ptr(room_idx as usize).add(offset) };",
        "        self.storage.rooms.release_n(room_idx, 1);\n        let ptr = unsafe { rooms.room_ptr(room_idx as usize).add(offset) };",

    ),
    (
        "a message's pieces are published done before the last is hashed",
        "        if self.tasks_left[slot].fetch_sub(1, Ordering::AcqRel) == 1 {\n            // Sound: this is the slot's last task",
        "        if self.tasks_left[slot].fetch_sub(1, Ordering::AcqRel) != 0 {\n            // Sound: this is the slot's last task",
    ),
    (
        "a storage goes back on the free list while a Room still holds it",
        "        if st.holders.fetch_sub(1, Ordering::AcqRel) == 1 {",
        "        if st.holders.fetch_sub(1, Ordering::AcqRel) >= 1 {",
    ),
    (
        "a Room's drop gives back one credit more than it has",
        "        let unused = ROOM_CREDITS - (self.commits as usize);",
        "        let unused = ROOM_CREDITS - (self.commits as usize) + (self.commits != 0) as usize;",
    ),
    (
        "a loan's event comes before the loan is marked complete",
        "            if desc.loan_epoch != 0 {\n                self.storage.loan_completed[slot].0.store(desc.loan_epoch, Ordering::Release);\n            }\n            finished(storage, || {\n                self.slot_done[slot].swap(true, Ordering::SeqCst);\n            });",
        "            finished(storage, || {\n                self.slot_done[slot].swap(true, Ordering::SeqCst);\n            });\n            if desc.loan_epoch != 0 {\n                self.storage.loan_completed[slot].0.store(desc.loan_epoch, Ordering::Release);\n            }",
    ),
    (
        "a batch's tasks are published done before the last has hashed",
        "            if self.tasks_left[slot].fetch_sub(1, Ordering::AcqRel) == 1 {\n                finished(storage, || {",
        "            if self.tasks_left[slot].fetch_sub(1, Ordering::AcqRel) != 0 {\n                finished(storage, || {",
    ),
]


def build_directory():
    """Where the copy and its builds go: the build directory in use, so
    that the crates of others built there for the model serve the copy too."""
    return os.path.abspath(os.environ.get("CARGO_TARGET_DIR") or os.path.join(ROOT, "target"))


def model_passes(copy):
    env = {**os.environ, "RUSTFLAGS": "--cfg shuttle", "CARGO_TARGET_DIR": build_directory()}
    run = None
    try:
        # In a process group of its own: a stop of this script stops the
        # build and the model with it.
        run = subprocess.Popen(
            ["cargo", "test", "--locked", "-q", "--release", "--test", "shuttle_pipeline"],
            cwd=os.path.join(copy, "threads"), env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
        stdout, stderr = run.communicate()
    except BaseException:
        # Stopped (a signal's exception) while the model ran: the whole
        # group, whether or not Cargo itself has ended by now.
        if run is not None:
            try:
                os.killpg(run.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            run.wait()
        raise
    # A fault may show as a failed comparison, a deadlock Shuttle reports,
    # or the Pipeline's own stop of the process: each is a panic or a
    # signal after the test began. Anything else is no verdict.
    began = "running 1 test" in stdout
    if run.returncode != 0 and not began:
        sys.exit(f"pipeline_model_controls: the model did not build or run:\n{stdout[-2000:]}\n{stderr[-2000:]}")
    return run.returncode == 0, stdout[-3000:] + stderr[-3000:]


def tree_files():
    """The tree's files, as paths from its root: what Git knows or would
    add, and the lock files; those that are there now. Nothing of the
    build directory's, should it lie in the tree under a name Git does
    not ignore: the copy would hold a copy of itself."""
    listed = subprocess.run(["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"], cwd=ROOT, check=True, stdout=subprocess.PIPE).stdout
    names = {os.fsdecode(name) for name in listed.split(b"\0") if name} | set(LOCK_FILES)
    builds = build_directory() + os.sep
    return sorted(name for name in names if os.path.isfile(os.path.join(ROOT, name)) and not os.path.join(ROOT, name).startswith(builds))


def write(path, data):
    """Write the file at this moment. Cargo takes a source no newer than
    its last build for unchanged: a file whose bytes change is written
    anew, and never given an older file's time."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data)


def bring_up_to_date(copy):
    """Make the copy hold the tree's files, byte for byte, and no other
    file. A file whose bytes are the tree's already is left as it is, so
    that Cargo builds again only what changed."""
    names = tree_files()
    for name in names:
        with open(os.path.join(ROOT, name), "rb") as f:
            data = f.read()
        path = os.path.join(copy, name)
        try:
            with open(path, "rb") as f:
                same = f.read() == data
        except OSError:
            same = False
        if not same:
            write(path, data)
        os.chmod(path, os.stat(os.path.join(ROOT, name)).st_mode & 0o777)
    wanted = set(names)
    for folder, _, files in os.walk(copy):
        for file in files:
            path = os.path.join(folder, file)
            if os.path.relpath(path, copy) not in wanted:
                os.remove(path)


def stopped(signum, _frame):
    # As an exception: the model's build and the model stop with the run.
    sys.exit(f"pipeline_model_controls: stopped by signal {signum}")


def main():
    if sys.argv[1:]:
        sys.exit("pipeline_model_controls: takes no argument")
    for signum in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        signal.signal(signum, stopped)
    home = os.path.join(build_directory(), "pipeline-model-controls")
    copy = os.path.join(home, "tree")
    os.makedirs(copy, exist_ok=True)
    # One run at a time plants faults in a copy. The lock ends with the
    # process, however it ends.
    with open(os.path.join(home, "lock"), "w") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            print(f"pipeline_model_controls: another run is using {copy}: waiting for it", flush=True)
            fcntl.flock(lock, fcntl.LOCK_EX)
        controls(copy)


def controls(copy):
    bring_up_to_date(copy)
    source = os.path.join(copy, SOURCE)
    with open(source, "rb") as f:
        original = f.read().decode()
    for name, find, _ in FAULTS:
        if original.count(find) != 1:
            sys.exit(f"pipeline_model_controls: the place of the fault '{name}' is not there once: update this script")
    passes, said = model_passes(copy)
    if not passes:
        sys.exit(f"pipeline_model_controls: the model fails on the Pipeline as it is:\n{said}")
    missed = []
    for name, find, replace in FAULTS:
        write(source, original.replace(find, replace).encode())
        caught = not model_passes(copy)[0]
        print(f"{'caught' if caught else 'MISSED'}  {name}", flush=True)
        if not caught:
            missed.append(name)
    if missed:
        sys.exit(f"pipeline_model_controls: the model passes {len(missed)} planted faults")
    print(f"pipeline_model_controls: the model fails for each of {len(FAULTS)} planted faults")


if __name__ == "__main__":
    main()
