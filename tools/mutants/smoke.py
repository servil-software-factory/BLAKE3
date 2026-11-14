"""A short run of check.py, start to finish, for a change to it (CONTRIBUTING:
a tool that runs for hours lands after one on each kind of machine): two
shards on one small file; the next run's start-of-run cleanup, which keeps
the shards' worktrees and removes any other leftover; and a second run on
the kept worktrees, with the same outcome. About a minute and a half.

    python3 tools/mutants/smoke.py [--crate threads]

With --crate, the same on that crate (BLAKE3 #335): cargo runs in its
folder, in the shards' worktrees too, and the file is one of its own.
"""
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
os.chdir(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
import check

# One small file of the crate, every mutant of it caught or unbuildable.
FILE = (os.path.relpath(next(t["src_path"] for t in check.TARGETS if "lib" in t["kind"] or "bin" in t["kind"]), check.CRATE_DIR)
        if check.CRATE else "src/io.rs")

def trees():
    """The worktrees under check.py's directory, by name."""
    listed = subprocess.run(["git", "worktree", "list", "--porcelain"], capture_output=True, text=True, check=True).stdout
    home = os.path.realpath(check.WORKTREES) + os.sep
    return sorted(os.path.basename(t) for t in (l.removeprefix("worktree ") for l in listed.splitlines() if l.startswith("worktree "))
                  if os.path.realpath(t).startswith(home))


check.remove_leftovers()
found, functions, timed = check.run([FILE], shards=2, label="smoke")
print("found", dict(found), "timed", dict(timed), "functions", len(functions))
assert {"smoke-0", "smoke-1"} <= set(trees()), f"the run keeps its two shards' worktrees for the next: {trees()}"
# A leftover no shard reuses (a crash's, a stop by hand's) goes at the next start; the shards' stay.
subprocess.run(["git", "worktree", "add", "-q", "--detach", os.path.join(check.WORKTREES, "leftover"), "HEAD"], check=True)
check.remove_leftovers()
assert "leftover" not in trees() and {"smoke-0", "smoke-1"} <= set(trees()), f"the next start removes the leftover alone: {trees()}"
# The next run reuses them, with the same outcome.
before = trees()
again, _, _ = check.run([FILE], shards=2, label="smoke")
assert trees() == before, f"the second run reuses its worktrees: {before} -> {trees()}"
# But for a mutant the pool's timing decides run by run: the tests' hook that says
# every worker is parked, answering yes too soon, which a test sees or not
# (the VM's first run of this on the second crate caught it, its second did not).
steady = lambda names: {k: n for k, n in names.items() if "__all_workers_parked" not in k}
assert steady(again) == steady(found), f"the second run finds the same survivors: {dict(found)} -> {dict(again)}"
print(f"smoke: ok ({'the crate in ' + check.CRATE + '/' if check.CRATE else 'the crate at the root'}, {FILE})")
