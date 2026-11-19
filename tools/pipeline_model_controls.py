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
tree during a run is neither tested nor touched. (The tree is read
while others may write it. A run that finds a file changed under its
reading stops and says so.) It lies in the build directory
(pipeline-model-controls/ in CARGO_TARGET_DIR, or in target/ here),
which Git ignores, with a build directory of its own beside it, so that
no build of the tree can take a build of the copy for its own. Each run
makes the copy anew. Between runs it holds the last fault planted;
nothing but this script builds from it. Each fault's run is kept whole
beside the copy (logs/NN.log): the line this prints for a fault is a
reading of it. A run counts only where Cargo compiled the Pipeline's
package for it and the model's test ran. One run at a time uses it: a
second run waits for the first, and a run ends what an earlier run that
was killed outright left running (its build, its model).
"""

import fcntl
import os
import re
import shutil
import signal
import subprocess
import sys
import time

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
        "a Room's drop wakes no one",
        "        if self.storage.rooms.release_n(self.index, unused) {\n            self.storage.waker.notify_after_seqcst();\n        }\n",
        "        self.storage.rooms.release_n(self.index, unused);\n",
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


def home():
    """Where the copy and its builds go: in the build directory in use."""
    return os.path.join(os.path.abspath(os.environ.get("CARGO_TARGET_DIR") or os.path.join(ROOT, "target")), "pipeline-model-controls")


# The Pipeline's package, as Cargo names what it compiles.
PACKAGE = "blake3-servil"


def model_passes(copy):
    """Whether the model passes on the copy as it is now, and what it
    said. The copy's source was written since its last build (the copy
    made anew, or a fault planted), so Cargo must compile the Pipeline's
    package for this run, and say so: a run for which it compiled
    nothing ran an earlier build (Cargo takes a source no newer than its
    last build for unchanged, and a clock or a file system can make a
    new file no newer), and is no verdict on this source. A run that
    passes ran the model's one test."""
    env = {**os.environ, "RUSTFLAGS": "--cfg shuttle", "CARGO_TARGET_DIR": os.path.join(home(), "target")}
    run = None
    try:
        # In a process group of its own: a stop of this script stops the
        # build and the model with it.
        # --nocapture: the model names each scenario as it begins, and a
        # process that the Pipeline stops takes what was captured with it.
        run = subprocess.Popen(
            ["cargo", "test", "--locked", "--release", "--test", "shuttle_pipeline", "--", "--nocapture"],
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
    if not re.search(rf"^\s*Compiling {re.escape(PACKAGE)} v", stderr, re.M):
        sys.exit(f"pipeline_model_controls: Cargo compiled nothing of {PACKAGE} for this run, whose source was just written: "
                 f"it ran an earlier build, which says nothing of this source\n{stderr[-2000:]}")
    if run.returncode == 0 and not re.search(r"^test result: ok\. 1 passed", stdout, re.M):
        sys.exit(f"pipeline_model_controls: the run ended well and the model's one test did not run:\n{stdout[-2000:]}\n{stderr[-2000:]}")
    return run.returncode == 0, stdout + stderr


def how_it_failed(said):
    """Where and how a run of the model failed, for the reader of this
    script's lines: the scenario it was in (the model names each as it
    begins) and the failure's kind. The verdict is the model's exit
    alone; this says whether the fault was caught for the reason its
    name gives."""
    scenarios = [line.split(": ", 1)[1].strip() for line in said.splitlines() if line.startswith("shuttle_pipeline: ")]
    lines = said.splitlines()
    at = [n for n, line in enumerate(lines) if "panicked at" in line]
    stops = [line.strip() for line in lines if line.startswith("blake3-servil: ")]
    # The C library's allocator, stopping a process whose memory it finds
    # broken (a storage freed twice, or given out while held).
    allocator = [line.strip() for line in lines if re.match(r"(free|malloc|realloc|munmap_chunk|double free|corrupted)\b.*(\(\)|:)", line.strip())]
    if "deadlock!" in said:
        how = "a deadlock"
    elif at and at[0] + 1 < len(lines):
        how = lines[at[0] + 1].strip()[:100]
    elif stops:
        how = "the Pipeline stopped the process: " + stops[-1][len("blake3-servil: "):][:100]
    elif allocator:
        how = "the allocator stopped the process: " + allocator[0][:100]
    else:
        # Cargo's line for a test process that a signal ended.
        signalled = re.search(r"\(signal: [^)]*\)", said)
        how = "the process ended with no word of the model's" + (f" {signalled[0]}" if signalled else "")
    return f"in {scenarios[-1] if scenarios else 'no scenario'}: {how}"


def tree_files():
    """The tree's files and symbolic links, as paths from its root: what
    Git knows or would add, and the lock files; those that are there now.
    Nothing of the build directory's, should it lie in the tree under a
    name Git does not ignore: the copy would hold a copy of itself."""
    listed = subprocess.run(["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"], cwd=ROOT, check=True, stdout=subprocess.PIPE).stdout
    names = {os.fsdecode(name) for name in listed.split(b"\0") if name} | set(LOCK_FILES)
    builds = os.path.dirname(home()) + os.sep
    there = lambda path: os.path.islink(path) or os.path.isfile(path)
    return sorted(name for name in names if there(os.path.join(ROOT, name)) and not os.path.join(ROOT, name).startswith(builds))


def end_earlier_runs():
    """End what a run that was killed outright left running (its build,
    its model): they would read the copy while this run makes it anew,
    and their builds would land among this run's. They are known by the
    build directory in their environment, which is the copy's own and
    which this script alone gives a process (a person's shell in the
    copy, or an editor, has another or none, and is left alone). Where
    the system has no /proc to ask, nothing is looked for."""
    if not os.path.isdir("/proc"):
        return
    mark = os.fsencode("CARGO_TARGET_DIR=" + os.path.join(home(), "target"))
    for _ in range(100):
        found = False
        for entry in os.listdir("/proc"):
            if not entry.isdigit() or int(entry) == os.getpid():
                continue
            try:
                with open(f"/proc/{entry}/environ", "rb") as f:
                    theirs = f.read().split(b"\0")
            except OSError:
                continue
            if mark in theirs:
                found = True
                try:
                    os.kill(int(entry), signal.SIGKILL)
                except OSError:
                    pass
        if not found:
            return
        time.sleep(0.05)
    sys.exit(f"pipeline_model_controls: processes of an earlier run are still at work for {home()}")


def make_copy(copy, read=None):
    """Make the copy anew: the tree's files, byte for byte, with their
    modes, and its symbolic links as links. Anew, not brought up to date:
    whatever an earlier run or an earlier tree left there is gone, and
    every file is newer than every build made from the copy before
    (Cargo takes a source no newer than its last build for unchanged).
    `read` is called with each file's name after it is read (the tests'
    way of saving a file while the copy is made)."""
    if os.path.lexists(copy):
        shutil.rmtree(copy)
    names = tree_files()
    seen = {}
    for name in names:
        source, path = os.path.join(ROOT, name), os.path.join(copy, name)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        if os.path.islink(source):
            os.symlink(os.readlink(source), path)
            continue
        was = os.stat(source)
        with open(source, "rb") as f:
            data = f.read()
        if read:
            read(name)
        with open(path, "wb") as f:
            f.write(data)
        os.chmod(path, was.st_mode & 0o777)
        seen[name] = (was.st_mtime_ns, was.st_size)
    # The tree is not held still while it is read. A file written
    # meanwhile may have been taken half-written, or beside files of an
    # earlier moment: no tree that ever was.
    now = {}
    for name in seen:
        try:
            is_now = os.stat(os.path.join(ROOT, name))
            now[name] = (is_now.st_mtime_ns, is_now.st_size)
        except OSError:
            now[name] = None
    changed = sorted(name for name in seen if now[name] != seen[name])
    if changed or tree_files() != names:
        sys.exit("pipeline_model_controls: the tree changed while it was copied" + (f" ({', '.join(changed[:3])})" if changed else "") + ": nothing was built; run this again")


def stopped(signum, _frame):
    # As an exception: the model's build and the model stop with the run.
    sys.exit(f"pipeline_model_controls: stopped by signal {signum}")


def main():
    if sys.argv[1:]:
        sys.exit("pipeline_model_controls: takes no argument")
    for signum in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        signal.signal(signum, stopped)
    copy = os.path.join(home(), "tree")
    os.makedirs(home(), exist_ok=True)
    # One run at a time plants faults in the copy. The lock ends with the
    # process, however it ends.
    with open(os.path.join(home(), "lock"), "w") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            print(f"pipeline_model_controls: another run is using {copy}: waiting for it", flush=True)
            fcntl.flock(lock, fcntl.LOCK_EX)
        controls(copy)


def controls(copy):
    end_earlier_runs()
    make_copy(copy)
    source = os.path.join(copy, SOURCE)
    with open(source, "rb") as f:
        original = f.read().decode()
    for name, find, _ in FAULTS:
        if original.count(find) != 1:
            sys.exit(f"pipeline_model_controls: the place of the fault '{name}' is not there once: update this script")
    passes, said = model_passes(copy)
    if not passes:
        sys.exit(f"pipeline_model_controls: the model fails on the Pipeline as it is:\n{said[-6000:]}")
    # Each fault's run, as it said it: this run's alone.
    shutil.rmtree(os.path.join(home(), "logs"), ignore_errors=True)
    os.makedirs(os.path.join(home(), "logs"))
    missed = []
    for name, find, replace in FAULTS:
        with open(source, "wb") as f:
            f.write(original.replace(find, replace).encode())
        passes, said = model_passes(copy)
        # What the run said, whole, beside the copy: the line below is a
        # reading of it.
        with open(os.path.join(home(), "logs", f"{len(os.listdir(os.path.join(home(), 'logs'))):02}.log"), "w") as f:
            f.write(f"{name}\n\n{said}")
        print(f"MISSED  {name}" if passes else f"caught  {name} ({how_it_failed(said)})", flush=True)
        if passes:
            missed.append(name)
    if missed:
        sys.exit(f"pipeline_model_controls: the model passes {len(missed)} planted faults")
    print(f"pipeline_model_controls: the model fails for each of {len(FAULTS)} planted faults")


if __name__ == "__main__":
    main()
