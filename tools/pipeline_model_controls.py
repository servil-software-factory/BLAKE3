#!/usr/bin/env python3
"""The Pipeline's Shuttle model fails for each of these planted faults.

    python3 tools/pipeline_model_controls.py

Each fault is one edit of threads/src/pipeline.rs that breaks a handing-
over between the caller and the workers. The model
(threads/tests/shuttle_pipeline.rs) is run on the edited file and must
fail; then the file is put back. A fault the model passes is a hole in
the model, and this exits 1 naming it. The model must also pass on the
file as it is.

Takes a build of the model per fault (about half a minute each).

The file is edited where it lies, so a run that ends early must not
leave its fault there (one did, on November 19, 2026: the next check of
that tree failed in the model, and looked like a fault of the Pipeline).
- Stopped by a signal (SIGTERM, SIGINT, SIGHUP), the run stops the
  model's build and puts the file back before it exits.
- While a fault is planted, the file as it was lies beside it
  (pipeline.rs.before-fault). A run that is killed outright leaves both;
  the next run puts the file back first, and so does

      python3 tools/pipeline_model_controls.py --put-back

  tools/check-builds.sh refuses a tree where that file lies.
"""

import os
import signal
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SOURCE = os.path.join(ROOT, "threads", "src", "pipeline.rs")
# The source as it was, while a fault is planted in it.
KEPT = SOURCE + ".before-fault"

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


def model_passes():
    env = {**os.environ, "RUSTFLAGS": "--cfg shuttle"}
    # In a process group of its own: a stop of this script stops the build
    # and the model with it, and nothing goes on compiling a planted fault.
    run = subprocess.Popen(["cargo", "test", "--locked", "-q", "--release", "--test", "shuttle_pipeline"], cwd=os.path.join(ROOT, "threads"), env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
    try:
        stdout, stderr = run.communicate()
    finally:
        if run.poll() is None:
            os.killpg(run.pid, signal.SIGKILL)
            run.wait()
    # A fault may show as a failed comparison, a deadlock Shuttle reports,
    # or the Pipeline's own stop of the process: each is a panic or a
    # signal after the test began. Anything else is no verdict.
    began = "running 1 test" in stdout
    if run.returncode != 0 and not began:
        sys.exit(f"pipeline_model_controls: the model did not build or run:\n{stdout[-2000:]}\n{stderr[-2000:]}")
    return run.returncode == 0, stdout[-3000:] + stderr[-3000:]


def plant(original, faulted):
    """Write the faulted source, the source as it was kept beside it first."""
    # Under another name until it is whole: a file named KEPT is the whole source.
    with open(KEPT + ".part", "w") as f:
        f.write(original)
    os.replace(KEPT + ".part", KEPT)
    with open(SOURCE, "w") as f:
        f.write(faulted)


def put_back():
    """Put the source back as it was, if a fault is planted; say whether one was."""
    if os.path.exists(KEPT + ".part"):
        os.remove(KEPT + ".part")
    if not os.path.exists(KEPT):
        return False
    with open(KEPT) as f:
        original = f.read()
    # Written anew, not renamed into place: Cargo builds again what is newer than its last build.
    with open(SOURCE, "w") as f:
        f.write(original)
    os.remove(KEPT)
    return True


def stopped(signum, _frame):
    # As an exception: the `finally` clauses stop the model and put the source back.
    sys.exit(f"pipeline_model_controls: stopped by signal {signum}; the source is put back")


def main():
    if put_back():
        print("pipeline_model_controls: a run that was killed had left its fault planted; the source is put back", flush=True)
    if sys.argv[1:] == ["--put-back"]:
        return
    if sys.argv[1:]:
        sys.exit("pipeline_model_controls: no such option (--put-back, or none)")
    for signum in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        signal.signal(signum, stopped)
    with open(SOURCE) as f:
        original = f.read()
    passes, said = model_passes()
    if not passes:
        sys.exit(f"pipeline_model_controls: the model fails on the Pipeline as it is:\n{said}")
    missed = []
    try:
        for name, find, replace in FAULTS:
            if original.count(find) != 1:
                sys.exit(f"pipeline_model_controls: the place of the fault '{name}' is not there once: update this script")
            plant(original, original.replace(find, replace))
            caught = not model_passes()[0]
            put_back()
            print(f"{'caught' if caught else 'MISSED'}  {name}", flush=True)
            if not caught:
                missed.append(name)
    finally:
        put_back()
    if missed:
        sys.exit(f"pipeline_model_controls: the model passes {len(missed)} planted faults")
    print(f"pipeline_model_controls: the model fails for each of {len(FAULTS)} planted faults")


if __name__ == "__main__":
    main()
