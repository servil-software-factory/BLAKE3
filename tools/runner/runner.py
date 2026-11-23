#!/usr/bin/env python3
"""Benchmark runner for BLAKE3-Servil, for the Mac's `benchrunner` account.

    sudo -u benchrunner -H python3 runner.py EXCHANGE

EXCHANGE holds `jobs/` (JSON job files, read here) and `results/` (written
here, one folder per job). Code comes from GitHub alone, at the commits a
job names. Done jobs are listed in ~/.benchrunner_done (the runner's home).
Ctrl-C stops the runner after the current job.

Jobs (commits as 7-40 hex digits; time_limit_seconds optional, default 1800
(a whole mutation run: 7200), at most 7200):

    {"type": "benchmark", "fork_commit": "...", "bench_commit": "...",
     "flags": ["--all", "--quick"], "contenders": [...], "points": [...],
     "rounds": N, "repeat": N}

(bench-hashes before its git dependency on BLAKE3-Servil took `--thorough` for
a full run; later ones run in full by default and take `--quick`.)
    {"type": "check", "old_commit": "...", "new_commit": "...",
     "bench_commit": "...", "old_bench_commit": "...", "points": "3 KiB,idle 3 KiB",
     "layouts": N}

(The check's scope: "points" names the cells, as bench-hashes check --points
takes them; "probe" the same for its three-pair survey, with no verdict;
"cells": "all" every cell. From bench-hashes 0.21 the check needs one; an
older bench, given none, measures the cells whose code the change touches.
"layouts" is the links a side, a pair
each, its default check's. "old_bench_commit" builds the old side with the bench at that
commit (check --old-bench), for an old side that predates the bench's
calls: 0.8 against the main line before the throughput API. Every run's
samples file lands in the job's samples/.)

(Any job may add "powermetrics": true: the chip's CPU, GPU, and ANE power
every 500 ms beside it, in its results' powermetrics.txt. And "top": true:
the twelve busiest processes every second beside it, each sample with its
time, in its results' top.txt, to name the other programs a timing job's
busy windows count.)
    {"type": "b3sum", "fork_commit": "...", "bench_commit": "...", "against": "...", "quick": true}

(A b3sum job builds the contenders, bench-hashes' tools/b3sum-contenders.py
at the BLAKE3-Servil commit: official BLAKE3's b3sum, BLAKE3-Servil's as b3sum and
b3sum-1-thread, the system's sum programs; and runs bench-hashes b3sum on
the files in ~/b3sum-bench-files, made there once. The report, samples
and map land in the job's benchmark-results/. "quick" runs the small set.)
    {"type": "crowd", "fork_commit": "...", "bench_commit": "..."}

(A crowd job runs `bench-hashes crowd` with its defaults, built against the
fork commit: many programs at once, one a CPU, each with its own pool. Its
samples and report land in the job's benchmark-results/.)
    {"type": "mutants", "fork_commit": "...", "since": "..."}

(A mutants job runs BLAKE3-Servil's tools/mutants/check.py --since SINCE at the
fork commit: the mutants in the lines the change since SINCE makes, judged
against the list for this machine. Its exit is the verdict, and each run's
outcomes land in the job's mutants-last/. No timing: other programs on the
Mac can only make some mutants time out, an outcome of their own.)
    {"type": "mutants-whole", "fork_commit": "...", "crate": "." | "threads"}

(A whole mutation run of one crate, the core at "." or the crate in
threads/: tools/mutants/check.py --whole --write --crate CRATE at the fork
commit, which writes this machine's lists anew. The job's lists/ holds the
lists to land: the survivors as the run wrote them, and the time-outs with
the commit's rows the run did not see put back (lists-put-back.txt names
them: a time-out can be a crash in another run, #380). lists-diff.txt is
what they change in the commit's, lists-written/ the lists as this run
alone wrote them, and outcomes.tsv every mutant's outcome and seconds. Its exit is the verdict, as
a mutants job's. One crate a job: the core's run took 39 minutes on an M6
and the other crate's 23 (November 12, 2026), so its time limit is two
hours unless the job names another.)
    {"type": "example", "example": "scaling" | "host_lab" | "energy_probe" | "job_trace",
     "fork_commit": "...", "args": ["message", "1048576"]}
    {"type": "test", "fork_commit": "..."}

(A test job runs BLAKE3-Servil's suites natively, under cargo-nextest: the
library tests, the doc tests, the
integration tests in tests/ (the planned API's contract), and the
official vectors.)

(A benchmark job may add "train": [W_MS, S] and "nonstop_visits": N, the
continuous-load proposal's tests: bench-hashes' --train and
--nonstop-visits.)

(A benchmark job with "repeat": N runs the benchmark N times back to
back, run i's files in run-i/, for calibrations.)

Each result folder holds runner.log (every command and its output),
verdict.json, and the job's own files: the benchmark's report, graph,
and samples; host_lab's report.

No process of a job's command is left when the command is over. At the
job's time limit, and when a command ends by itself, the runner ends
every process in the command's session and every descendant of any
member of that session (whichever process group each is in;
cargo-nextest gives each test a group of its own). A command that ended
and left processes running has a sentence in the verdict's "left running".
A process that could not be ended, or a failed process listing, stops the
job (status "error" or "timed out") rather than starting the next command
beside an unknown survivor.

A job that reaches its time limit leaves stacks.txt beside its log: its
processes, and where each stood when the limit came. The runner reads
them while it holds them stopped, between stopping every one and
killing every one, so nothing of the job runs or starts meanwhile. The
file lists the processes (each one's state before the stop, its share
of a CPU, its command). Then, on macOS, /usr/bin/sample of each process
(every thread's call graph), sixteen processes at a time, the deepest
descendants first; on Linux each thread's name and the CPU time it has
used, which is what an account without root can read of a stopped
process there. The capture takes STACKS_BUDGET seconds at most,
whatever any step of it does. It never changes the verdict: what it
could not read it says in the file, and the verdict's "stacks" says
what the file holds.

Known limits of this PID-based cleanup (a cgroup per job on Linux belongs
to ops #25):
- A process that left the session and whose parent had already ended
  before the runner looked is not found.
- `lstart` has 1-second resolution and `os.kill` takes a PID, so rapid
  same-second PID reuse is a theoretical window.
- On startup (`end_leftover_session`), a leftover record whose leader is
  already gone is not trusted to signal an arbitrary session (to avoid
  signalling an unrelated session after PID reuse); only a live leader
  matching the recorded start time is ended on restart.
- A Linux zombie thread-group leader whose non-main thread is still alive
  appears as a zombie in `ps` and is signalled, though not counted as live.

The checkouts persist between jobs (CHECKOUTS: BLAKE3-Servil, and bench-hashes
beside it), moved to each job's commits with a forced checkout that
rewrites only the files that differ, so Cargo rebuilds only what changed:
BLAKE3-Servil's target directory serves test and example jobs, check's
sides (target/check/ in BLAKE3-Servil) its jobs, and bench-hashes' own build
against BLAKE3-Servil as it is (`bench-hashes build-against`) benchmark jobs.
"""
import json
import os
import platform
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

from exchange import Exchange, utc

FORK_URL = "https://github.com/servil-software-factory/BLAKE3.git"
BENCH_URL = "https://github.com/servil-software-factory/bench-hashes.git"
HOME = Path.home()
CHECKOUTS = HOME / "checkouts"
CARGO_BIN = HOME / ".cargo" / "bin"

BENCHMARK_FLAGS = {"--all", "--quick", "--thorough"}
# blake3, blake3-mt, blake3-servil: bench-hashes' keys before September 25, 2026.
CONTENDERS = {"blake3-official", "blake3-official-mt", "blake3", "blake3-servil-st", "blake3-servil", "blake3-servil-mt", "blake3-mt",
              "sha256", "sha256-ring", "sha256-cc", "sha1dc", "sha3-256", "blake3-commonware", "blake3-servil-pipeline"}
EXAMPLES = {"host_lab", "energy_probe", "job_trace", "sme_units"}
FEATURES = {"job-trace"}
DEFAULT_LIMIT = 1800
MAX_LIMIT = 7200
# What a job at its limit leaves in stacks.txt (the docstring's paragraph):
# the seconds the capture may take all told, the program that samples a
# process's threads (macOS has one; None reads /proc), and how many of
# them run at once.
STACKS_BUDGET = 10.0
SAMPLER = "/usr/bin/sample" if sys.platform == "darwin" else None
SAMPLED_AT_ONCE = 16

stopping = False

# The session of the command a job is running: its PID and start time. A
# runner stopped some other way than Ctrl-C (its terminal closed, a kill)
# leaves that session running, and the next runner would measure beside
# it: two checks of job 1939 ran at once, each the other's load
# (bench-hashes #135, November 6, 2026). The next runner ends it first.
SESSION_FILE = Path.home() / "runner-session.txt"


def started(pid):
    """A process's start time as ps prints it (its words a space apart), or
    None when it is gone or ps fails."""
    try:
        out = subprocess.run(
            ["ps", "-o", "lstart=", "-p", str(pid)],
            capture_output=True, text=True, timeout=10, start_new_session=True,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    if out.returncode != 0:
        return None
    return " ".join(out.stdout.split()) or None


def process_table():
    """Every process there is, by PID: (ppid, state_char, start_time).
    Raises RuntimeError if `ps` cannot start, times out, exits non-zero,
    or returns an empty table."""
    try:
        out = subprocess.run(
            ["ps", "-A", "-o", "pid=", "-o", "ppid=", "-o", "stat=", "-o", "lstart="],
            capture_output=True, text=True, timeout=15, start_new_session=True,
        )
    except (OSError, subprocess.SubprocessError) as e:
        raise RuntimeError(f"process listing failed: {e}") from e
    if out.returncode != 0:
        raise RuntimeError(f"ps exited {out.returncode}: {out.stderr.strip()}")
    table = {}
    for line in out.stdout.splitlines():
        fields = line.split()
        if len(fields) >= 4:
            try:
                pid, ppid = int(fields[0]), int(fields[1])
            except ValueError:
                continue
            stat = fields[2][0] if fields[2] else "?"
            table[pid] = (ppid, stat, " ".join(fields[3:]))
    if not table:
        raise RuntimeError("ps returned an empty process table")
    return table


def _is_zombie(row):
    return row[1] == "Z"


def _is_stopped(row):
    return row[1] == "T"


def job_processes(leader, began, table, leader_reaped=False):
    """The processes of a job's command among those of `table`, by PID:
    {pid: (start_time, is_zombie)}. Includes every process in `leader`'s
    session and every descendant of `leader` or of any session member.

    Validates `leader > 0`. When `leader_reaped` is True, `leader` itself
    has already been waited on, so any row at `leader` in `table` belongs
    to a reused PID (never ours) and is ignored; only session members and
    their descendants are returned. When `leader_reaped` is False and a
    row at `leader` has a start time other than `began`, nothing is ours.
    """
    if not isinstance(leader, int) or leader <= 0:
        return {}
    me = os.getpid()
    first = table.get(leader)
    if not leader_reaped and first is not None and began is not None and first[2] != began:
        return {}

    def in_session(pid):
        try:
            return os.getsid(pid) == leader
        except OSError:
            return False

    # Seed set: the live leader (if not reaped) plus every process in `leader`'s session.
    roots = set()
    if not leader_reaped and leader in table and leader != me:
        roots.add(leader)
    for pid in table:
        if pid == me:
            continue
        if leader_reaped and pid == leader:
            continue
        if in_session(pid):
            roots.add(pid)

    # Walk parent links from every process to see if it reaches any root (or `leader`
    # when not reaped, e.g. a leader that just exited between calls).
    found = set(roots)
    children = {}
    for pid, row in table.items():
        if pid == me or (leader_reaped and pid == leader):
            continue
        children.setdefault(row[0], []).append(pid)

    queue = list(roots)
    if not leader_reaped and leader not in found:
        queue.append(leader)
    while queue:
        cur = queue.pop()
        for child in children.get(cur, ()):
            if child not in found:
                found.add(child)
                queue.append(child)

    found.discard(me)
    if leader_reaped:
        found.discard(leader)
    return {pid: (table[pid][2], _is_zombie(table[pid])) for pid in found if pid in table}


def signal_verified(targets, signum, table):
    """Signal each (pid, start_time) in `targets` only if `table` still shows
    that PID with the same start time, avoiding signalling a reused PID."""
    for pid, at in targets.items():
        if pid <= 0 or pid == os.getpid():
            continue
        row = table.get(pid)
        if row is None or (at is not None and row[2] != at):
            continue
        try:
            os.kill(pid, signum)
        except OSError:
            pass


def _fallback_killpg(leader):
    if isinstance(leader, int) and leader > 0:
        try:
            os.killpg(leader, signal.SIGKILL)
        except OSError:
            pass


def end_job(leader, began, leader_reaped=False, while_stopped=None):
    """End every process of a job's command (job_processes). Returns
    `(found, left, error)`:
    - `found`: {pid: start_time} of non-zombie processes seen;
    - `left`: set of non-zombie PIDs still alive at the end;
    - `error`: None if process listings succeeded, else an error message.

    Stop pass: sends SIGSTOP to every non-stopped, non-zombie job process
    and re-reads until every live job process is stopped (`T`) and no new
    ones appear (up to 15 rounds).
    Between the passes, `while_stopped(processes, states)` is called if
    given: `processes` is {pid: start_time} of the stopped ones, `states`
    {pid: the state letter ps gave when the pid was first seen, before this
    function signalled it}. The kill pass follows whatever it does; what it
    raises is raised again once the kill pass is over.
    Kill pass: verifies each process's start time against the current table
    before SIGKILL, and re-reads until none remain.
    If `ps` fails at any point (e.g. EAGAIN at the process limit), falls
    back to `os.killpg(leader, SIGKILL)` and kills every PID already seen
    so nothing is left stopped or running, and reports the failure in `error`.
    """
    if not isinstance(leader, int) or leader <= 0:
        return {}, set(), f"invalid session leader PID {leader!r}"
    seen = {}       # pid -> (start_time, was_zombie_first_seen)
    states = {}     # pid -> ps's state letter when first seen
    error = None

    def kill_known_and_group():
        if not leader_reaped:
            _fallback_killpg(leader)
        for pid in list(seen):
            if pid > 0 and pid != os.getpid():
                try:
                    os.kill(pid, signal.SIGKILL)
                except OSError:
                    pass

    # Stop pass: stop every running job process until all are stopped (T) and none are new.
    try:
        for attempt in range(15):
            table = process_table()
            current = job_processes(leader, began, table, leader_reaped=leader_reaped)
            for pid, info in current.items():
                if pid not in seen:
                    seen[pid] = info
                    states[pid] = table[pid][1]
            to_stop = {
                pid: at
                for pid, (at, z) in current.items()
                if not z and not _is_stopped(table[pid])
            }
            if not to_stop:
                break
            signal_verified(to_stop, signal.SIGSTOP, table)
            time.sleep(0.01 if attempt < 5 else 0.03)
    except RuntimeError as e:
        error = str(e)
        kill_known_and_group()
        return {pid: at for pid, (at, z) in seen.items() if not z}, set(), error

    raised = None
    if while_stopped is not None:
        try:
            while_stopped({pid: at for pid, (at, zombie) in current.items() if not zombie}, states)
        except BaseException as e:
            raised = e

    # Kill pass: keep discovering and killing until no live job processes remain.
    left = set()
    try:
        for attempt in range(7 if seen else 0):
            table = process_table()
            current = job_processes(leader, began, table, leader_reaped=leader_reaped)
            for pid, info in current.items():
                if pid not in seen or seen[pid][0] != info[0]:
                    # Newly discovered (or replaced within the job): stop before killing if live.
                    if not info[1] and not _is_stopped(table[pid]):
                        signal_verified({pid: info[0]}, signal.SIGSTOP, table)
                    seen[pid] = info
            there = {
                pid: at
                for pid, (at, _) in seen.items()
                if pid in table and table[pid][2] == at
            }
            left = {pid for pid in there if not _is_zombie(table[pid])}
            if attempt == 6 or (attempt > 0 and not left):
                break
            signal_verified(there, signal.SIGKILL, table)
            time.sleep(0.05 * 2 ** attempt)
    except RuntimeError as e:
        error = str(e)
        kill_known_and_group()

    if raised is not None:
        raise raised
    return {pid: at for pid, (at, z) in seen.items() if not z}, left, error


def threads(pid):
    """Each thread of a Linux process, a line: its name and the CPU time
    it has used, from /proc. (Of a stopped process an account without root
    reads no more: the kernel symbol a thread waited in is gone with the
    stop.)"""
    def read(path):
        try:
            return path.read_bytes().decode("utf-8", "replace").strip()
        except OSError:
            return ""
    try:
        tasks = sorted(Path(f"/proc/{pid}/task").iterdir(), key=lambda task: int(task.name))
    except (OSError, ValueError):
        return "its threads could not be read\n"
    lines = []
    for task in tasks:
        # The times follow the name's closing parenthesis, the 12th and 13th fields after it; the name may hold any byte.
        fields = read(task / "stat").rpartition(")")[2].split()
        used = f"{(int(fields[11]) + int(fields[12])) / os.sysconf('SC_CLK_TCK'):.2f} s of CPU" if len(fields) > 12 else "its times could not be read"
        lines.append(f"thread {task.name} ({read(task / 'comm') or '?'}): {used}")
    return "\n".join(lines) + "\n"


def stacks(processes, states, budget):
    """Where a job's stopped processes stand: the text for stacks.txt, and
    a sentence for the verdict. `processes` and `states` are end_job's.
    Takes `budget` seconds at most whatever any step does, and what it
    could not read it says in the text.

    First the processes as ps lists them, with each one's state before
    the stop. Then, with a SAMPLER, a second's sample of every process,
    SAMPLED_AT_ONCE at a time, the deepest descendants first: a test that
    hangs is a leaf, and its ancestors wait for it. Without one,
    `threads` reads each."""
    deadline = time.monotonic() + budget
    left = lambda: max(0.0, deadline - time.monotonic())
    live = sorted(processes)
    if not live:
        return "no process of the command was running\n", "no stacks: no process of the command was running"
    parent, rows = {}, []
    try:
        listed = subprocess.run(["ps", "-o", "pid=", "-o", "ppid=", "-o", "%cpu=", "-o", "etime=", "-o", "args=", "-p", ",".join(map(str, live))],
                                capture_output=True, timeout=left(), start_new_session=True).stdout.decode("utf-8", "replace")
        for line in listed.splitlines():
            fields = line.split(None, 4)
            # ps prints a command's arguments as they are: a line break within them makes lines that are no rows.
            if len(fields) >= 4 and fields[0].isdigit() and fields[1].isdigit():
                pid = int(fields[0])
                parent[pid] = int(fields[1])
                rows.append(f"{pid:>7} {fields[1]:>7} {states.get(pid, '?'):<4} {fields[2]:>5} {fields[3]:>11} {fields[4] if len(fields) > 4 else ''}")
            else:
                rows.append(line)
    except (OSError, subprocess.SubprocessError) as e:
        rows.append(f"ps could not list them: {e}")

    def depth(pid):
        steps = 0
        while parent.get(pid) in processes and steps < len(processes):
            pid, steps = parent[pid], steps + 1
        return steps
    live.sort(key=lambda pid: (-depth(pid), pid))
    parts = [f"{len(live)} processes: PID, parent, state before the stop, % of a CPU, time running, command\n" + "\n".join(rows) + "\n"]
    read = 0
    if SAMPLER:
        with tempfile.TemporaryDirectory() as folder:
            waiting = list(live)
            while waiting and left() > 0:
                batch, waiting = waiting[:SAMPLED_AT_ONCE], waiting[SAMPLED_AT_ONCE:]
                samplers = []
                for pid in batch:
                    try:
                        samplers.append((pid, subprocess.Popen([SAMPLER, str(pid), "1", "-file", f"{folder}/{pid}.txt"], stdout=subprocess.DEVNULL,
                                                               stderr=subprocess.DEVNULL, start_new_session=True)))
                    except OSError as e:
                        parts.append(f"\n--- process {pid} ---\nthe sampler did not start: {e}\n")
                for pid, sampler in samplers:
                    try:
                        sampler.wait(timeout=left())
                        ended = f"ended with {sampler.returncode}"
                    except subprocess.TimeoutExpired:
                        sampler.kill()
                        ended = "ran out of time"
                        try:
                            sampler.wait(timeout=2)
                        except subprocess.TimeoutExpired:
                            pass
                    try:
                        sample = Path(folder, f"{pid}.txt").read_bytes().decode("utf-8", "replace")
                    except OSError:
                        sample = ""
                    read += bool(sample.strip())
                    parts.append(f"\n--- process {pid} ---\n{sample if sample.strip() else f'no sample: the sampler {ended}'}\n")
            parts.extend(f"\n--- process {pid} ---\nno sample: the capture's {budget:g} s were over\n" for pid in waiting)
    else:
        for pid in live:
            if left() <= 0:
                parts.append(f"\n--- process {pid} ---\nnot read: the capture's {budget:g} s were over\n")
                continue
            parts.append(f"\n--- process {pid} ---\n{threads(pid)}")
            read += 1
    said = f"stacks.txt: {read} of the command's {len(live)} processes at the time limit"
    return "".join(parts), said if read == len(live) else f"{said}; why the others are missing is in the file"


def not_ended(left):
    return f"{len(left)} of its processes could not be ended and may still be running: PID {', '.join(map(str, sorted(left)))}"


def _clear_session_file():
    try:
        SESSION_FILE.unlink()
    except FileNotFoundError:
        pass


def end_leftover_session():
    """End what an earlier runner left running of its last command, if the
    recorded leader is still alive with the exact recorded start time.
    Refuses to act on a missing or mismatched leader, or an invalid PID,
    so a stale record can never signal an unrelated session."""
    if not SESSION_FILE.exists():
        return
    try:
        raw = SESSION_FILE.read_text().strip()
        pid_s, recorded = raw.split("\t", 1)
        pid = int(pid_s)
        recorded = " ".join(recorded.split())
    except ValueError:
        print("runner: ignoring damaged session record", file=sys.stderr, flush=True)
        _clear_session_file()
        return
    if pid <= 0 or not recorded or recorded == "None":
        print(f"runner: ignoring invalid session record (PID {pid!r}, start {recorded!r})", file=sys.stderr, flush=True)
        _clear_session_file()
        return
    if started(pid) != recorded:
        # Leader is gone or PID was reused; do not guess at an unrelated session.
        _clear_session_file()
        return
    found, left, err = end_job(pid, recorded, leader_reaped=False)
    if found:
        print(f"runner: ended what an earlier runner left running (the session of PID {pid}: {len(found)} of its processes)", file=sys.stderr, flush=True)
    if err:
        print(f"runner: leftover session cleanup error: {err}", file=sys.stderr, flush=True)
    if left:
        print(f"runner: {not_ended(left)}", file=sys.stderr, flush=True)
    if not left and not err:
        _clear_session_file()


def on_signal(signum, _frame):
    global stopping
    stopping = True
    print("runner: stopping after the current job", file=sys.stderr, flush=True)


def hex_commit(job, key):
    value = job[key]
    assert isinstance(value, str) and 7 <= len(value) <= 40 and all(c in "0123456789abcdef" for c in value), \
        f"{key} must be 7-40 lowercase hex digits, got {value!r}"
    return value


def environment():
    env = {"PATH": f"{CARGO_BIN}:/usr/bin:/bin:/usr/sbin:/sbin", "HOME": str(HOME),
           "LANG": "en_US.UTF-8", "TMPDIR": str(HOME / "tmp") + "/", "TZ": "UTC"}
    return env


class TimedOut(Exception):
    """The job's time limit ended it: its own verdict, "timed out", never
    "error" (a crash) or a handler's answer. A limit reached says nothing
    about what the job would have found."""


class Job:
    """One job's log and deadline; every command runs through `run`. The
    deadline is the job's one clock (wait_for.py keeps none over it)."""

    def __init__(self, log, limit):
        self.log = log
        self.limit = limit
        self.deadline = time.monotonic() + limit
        self.left_running = []              # what commands left running when they ended, a sentence each: for the verdict
        self.stacks = None                  # what stacks.txt holds, once the limit has ended a command: for the verdict

    def keep_stacks(self, processes, states, why):
        """Write stacks.txt beside the log: where the command's stopped
        processes stood when `why` came (end_job's `while_stopped`). A
        capture that fails says so in `stacks` and changes nothing else:
        the job's verdict stays its own."""
        try:
            text, self.stacks = stacks(processes, states, STACKS_BUDGET)
            with open(Path(self.log.name).with_name("stacks.txt"), "w", encoding="utf-8", errors="backslashreplace") as file:
                file.write(f"{utc('%Y-%m-%dT%H:%M:%SZ')} {why}\n{text}")
        except Exception as e:
            self.stacks = f"no stacks: {type(e).__name__}: {e}".encode("ascii", "backslashreplace").decode()
        print(f"runner: {self.stacks}", file=self.log, flush=True)

    def run(self, cmd, cwd, capture=False):
        remaining = self.deadline - time.monotonic()
        if remaining <= 0:
            raise TimedOut(f"the job's {self.limit} s limit passed before: {' '.join(cmd)}")
        print(f"$ (cd {cwd} && {' '.join(cmd)})", file=self.log, flush=True)
        # A new session, so Ctrl-C in the runner's terminal reaches the
        # runner alone and the current job finishes.
        proc = subprocess.Popen(cmd, cwd=cwd, env=environment(), start_new_session=True,
                                stdout=subprocess.PIPE if capture else self.log, stderr=self.log, text=True)
        began = started(proc.pid)
        SESSION_FILE.write_text(f"{proc.pid}\t{began or ''}\n")
        # When capturing stdout, drain it in a daemon thread while waiting on the
        # leader itself: a worker that outlives the leader and keeps stdout open
        # is cleaned up at the leader's exit instead of hanging until timeout (#475).
        chunks = []
        reader = None
        if capture and proc.stdout is not None:
            def _drain():
                try:
                    while True:
                        part = proc.stdout.read(65536)
                        if not part:
                            break
                        chunks.append(part)
                except OSError:
                    pass
            reader = threading.Thread(target=_drain, daemon=True)
            reader.start()
        try:
            proc.wait(timeout=remaining)
        except subprocess.TimeoutExpired:
            said = f"the job's {self.limit} s limit ended: {' '.join(cmd)}"
            # Every process of the command, not the command alone nor its
            # process group alone: cargo's children (a benchmark it runs, a
            # test that cargo-nextest put in a group of its own) would
            # otherwise go on into the next job (#473). While they are
            # stopped, where each one stood is kept for whoever reads
            # "timed out".
            _, left, err = end_job(proc.pid, began, leader_reaped=False,
                                   while_stopped=lambda processes, states: self.keep_stacks(processes, states, said))
            if self.stacks is None:
                self.stacks = "no stacks: the processes could not be listed"
            if proc.stdout is not None:
                try:
                    proc.stdout.close()
                except OSError:
                    pass
            try:
                proc.wait(timeout=30)
            except subprocess.TimeoutExpired:
                pass                        # it could not be ended: it is among `left`, and said below
            if not left and not err:
                _clear_session_file()
            detail = ""
            if err:
                detail += f"; cleanup error: {err}"
            if left:
                detail += f"; {not_ended(left)}"
            raise TimedOut(f"{said}{detail}")
        # The leader has exited and been reaped (`leader_reaped=True`). End any
        # descendant/session process it left running (which also closes any held
        # stdout pipe so `reader` finishes promptly).
        found, left, err = end_job(proc.pid, began, leader_reaped=True)
        if reader is not None:
            reader.join(timeout=5.0)
        if proc.stdout is not None:
            try:
                proc.stdout.close()
            except OSError:
                pass
        if not left and not err:
            _clear_session_file()
        if found:
            if left:
                ended = len(found) - len(left)
                said = f"{' '.join(cmd)} ended and left {len(found)} of its processes running; ended {ended}, {not_ended(left)}"
            else:
                said = f"{' '.join(cmd)} ended and left {len(found)} of its processes running; the runner ended them"
            self.left_running.append(said)
            print(f"runner: {said}", file=self.log, flush=True)
        if err or left:
            parts = []
            if err:
                parts.append(f"cleanup could not verify process table: {err}")
            if left:
                parts.append(not_ended(left))
            raise RuntimeError(f"after {' '.join(cmd)}: {'; '.join(parts)}")
        if proc.returncode != 0:
            raise subprocess.CalledProcessError(proc.returncode, cmd)
        return "".join(chunks) if capture else None

    def sync(self, url, dest, commit):
        """`dest`, a clone of `url` kept between jobs, at `commit`: a commit
        on a branch or at a pull request's head (fetched as
        refs/remotes/origin/pr/N), so a job names a pull request's head as
        it is, from a fork too."""
        if not (dest / ".git").exists():
            self.run(["git", "clone", "--quiet", url, str(dest)], cwd=dest.parent)
        else:
            self.run(["git", "fetch", "--quiet", "--prune", "origin"], cwd=dest)
        self.run(["git", "fetch", "--quiet", "origin", "+refs/pull/*/head:refs/remotes/origin/pr/*"], cwd=dest)
        self.run(["git", "checkout", "--quiet", "--force", "--detach", commit], cwd=dest)
        full = self.run(["git", "rev-parse", "HEAD"], cwd=dest, capture=True).strip()
        assert full.startswith(commit), f"{url}: asked for {commit}, got {full}"

    def checkouts(self, fork_commit, bench_commit):
        """BLAKE3-Servil's checkout and bench-hashes', each a clone of its own
        beside the other (neither inside the other: bench-hashes names the
        fork's checkout, `build-against DIR`, `check --fork DIR`), kept
        between jobs: (fork, bench). Anything untracked goes before each
        job, so each reads as the commit the job names (nested bench clones
        that probe examples left in BLAKE3-Servil made every build's provenance
        read dirty, job 1778); ignored paths (target/) stay."""
        CHECKOUTS.mkdir(exist_ok=True)
        fork, bench = CHECKOUTS / "fork", CHECKOUTS / "bench"
        self.sync(FORK_URL, fork, fork_commit)
        (fork / ".git/info").mkdir(exist_ok=True)
        (fork / ".git/info/exclude").write_text("/tmp/\n")
        self.run(["git", "clean", "-ffd", "--quiet"], cwd=fork)
        if bench_commit is not None:
            self.sync(BENCH_URL, bench, bench_commit)
            self.run(["git", "clean", "-ffd", "--quiet"], cwd=bench)
        return fork, bench


def benchmark(job, run, work, out):
    flags = job.get("flags", [])
    assert set(flags) <= BENCHMARK_FLAGS, f"flags must come from {sorted(BENCHMARK_FLAGS)}"
    args = list(flags)
    if "contenders" in job:
        assert job["contenders"] and set(job["contenders"]) <= CONTENDERS, \
            f"contenders must come from {sorted(CONTENDERS)}"
        args += ["--contenders", ",".join(job["contenders"])]
    if "points" in job:
        points = job["points"]
        assert points and all(isinstance(p, str) and p.replace(" ", "").isalnum() for p in points), \
            "points are labels such as \"64 B\" or \"1024\""
        args += ["--points", ",".join(points)]
    if "rounds" in job:
        assert isinstance(job["rounds"], int) and job["rounds"] > 0, "rounds must be a positive integer"
        args += ["--rounds", str(job["rounds"])]
    # The continuous-load proposal's tests (bench-hashes FROZEN.md, October
    # 31, 2026): a nonstop visit's warm-up and samples, and a cell's visits.
    if "train" in job:
        train = job["train"]
        assert isinstance(train, list) and len(train) == 2 and all(isinstance(x, int) and x >= 0 for x in train) and train[1] > 0, \
            "train is [warm-up ms, samples a visit]"
        args += ["--train", f"{train[0]},{train[1]}"]
    if "nonstop_visits" in job:
        assert isinstance(job["nonstop_visits"], int) and job["nonstop_visits"] > 0, "nonstop_visits must be a positive integer"
        args += ["--nonstop-visits", str(job["nonstop_visits"])]
    repeat = job.get("repeat", 1)
    assert isinstance(repeat, int) and 1 <= repeat <= 100, "repeat must be 1-100"
    fork_commit = hex_commit(job, "fork_commit")
    fork, bench = run.checkouts(fork_commit, hex_commit(job, "bench_commit"))
    # bench-hashes built against the BLAKE3-Servil checkout as it is, in a side
    # directory that owns its lock and target directory (the committed lock
    # is never written); Cargo rebuilds only what changed. The one build
    # mechanism, check's (bench-hashes #54).
    exe = run.run(["cargo", "run", "--release", "--quiet", "--", "build-against", str(fork)],
                  cwd=bench, capture=True).strip().splitlines()[-1]
    # Each run from its result folder: the benchmark writes
    # benchmark-results/ relative to its working directory.
    for folder in [out] if repeat == 1 else [out / f"run-{i}" for i in range(1, repeat + 1)]:
        folder.mkdir(exist_ok=True)
        run.run([exe, *args], cwd=folder)
    return "ok"


def b3sum(job, run, work, out):
    fork_commit = hex_commit(job, "fork_commit")
    old_commit = hex_commit(job, "against") if "against" in job else None
    fork, bench = run.checkouts(fork_commit, hex_commit(job, "bench_commit"))
    commits = [old_commit, fork_commit] if old_commit is not None else [fork_commit]
    # One contender a line, each one argument (a command may hold flags).
    contenders = [line for line in run.run([sys.executable, "tools/b3sum-contenders.py", str(fork), str(HOME / "b3sum-contenders"), *commits],
                                           cwd=bench, capture=True).splitlines() if "=" in line]
    if old_commit is not None:
        # Head-to-head comparison: old_commit is the base (first), followed by fork_commit and official.
        old_tag, new_tag = f"b3sum-{old_commit[:7]}=", f"b3sum-{fork_commit[:7]}="
        contenders = (
            [c for c in contenders if c.startswith(old_tag)]
            + [c for c in contenders if c.startswith(new_tag)]
            + [c for c in contenders if c.startswith("official=")]
        )
    exe = run.run(["cargo", "run", "--release", "--quiet", "--", "build-against", str(fork)],
                  cwd=bench, capture=True).strip().splitlines()[-1]
    quick = ["--quick"] if job.get("quick") else []
    run.run([exe, "b3sum", "--files", str(HOME / "b3sum-bench-files"), *quick, *contenders], cwd=out)
    return "ok"


def check(job, run, work, out):
    """bench-hashes check: BLAKE3-Servil at new_commit, with bench-hashes at
    bench_commit beside it (a bench with `check --fork`, October 31, 2026),
    against old_commit. check builds both
    sides itself (under BLAKE3-Servil's target/check/), relinks each per pair,
    and measures the job's scope (the module docstring). Its exit is the
    outcome: 0 completed (a probe gives no speed verdict), 1 a regression,
    2 no verdict (other programs kept the machine busy, or no scope given
    to a bench that needs one), 3 a side's run stopped (a panic, a crash:
    bench-hashes 0.21 names the side and quotes its last words; the job's
    log has them, and no speed verdict follows)."""
    old, new = hex_commit(job, "old_commit"), hex_commit(job, "new_commit")
    fork, bench = run.checkouts(new, hex_commit(job, "bench_commit"))
    # --when-quiet: a Mac busy for a few seconds (a scan after a build, a
    # runner restart) waits instead of abstaining (jobs 1986, 1989, 1990).
    cmd = ["cargo", "run", "--release", "--quiet", "--", "check", "--fork", str(fork), "--against", old, "--when-quiet", "--keep", str(out / "samples")]
    scope = [k for k in ("cells", "points", "probe") if k in job]
    assert len(scope) <= 1, "one scope: cells, points, or probe"
    assert job.get("cells", "all") == "all", "cells: \"all\""
    for k in scope:
        cmd += [f"--{k}", job[k]]
    if "layouts" in job:
        assert isinstance(job["layouts"], int) and job["layouts"] >= 3, "layouts: 3 or more (regress's rule needs three pairs)"
        cmd += ["--layouts", str(job["layouts"])]
    if "old_bench_commit" in job:
        cmd += ["--old-bench", hex_commit(job, "old_bench_commit")]
    try:
        run.run(cmd, cwd=bench)
        return "ok" if "probe" in job else "no regression"
    except subprocess.CalledProcessError as e:
        if e.returncode in (1, 2, 3):
            return {1: "regression", 2: "no verdict", 3: "a side stopped"}[e.returncode]
        raise


def example(job, run, work, out):
    assert job["example"] in EXAMPLES, f"example must be one of {sorted(EXAMPLES)}"
    features = job.get("features", [])
    assert set(features) <= FEATURES, f"features must come from {sorted(FEATURES)}"
    fork, _ = run.checkouts(hex_commit(job, "fork_commit"), None)
    cmd = ["cargo", "build", "--release", "--example", job["example"]]
    if features:
        cmd += ["--features", ",".join(features)]
    run.run(cmd, cwd=fork)
    # An example's arguments: a few short words or numbers, nothing a shell
    # would read.
    args = job.get("args", [])
    assert isinstance(args, list) and len(args) <= 8 and all(isinstance(a, str) and re.fullmatch(r"[A-Za-z0-9_.-]{1,32}", a) for a in args), \
        "args: at most 8 words of letters, digits, _ . -"
    # host_lab writes its report to the working directory.
    run.run([str(fork / "target" / "release" / "examples" / job["example"]), *args], cwd=out)
    return "ok"


def test(job, run, work, out):
    fork, _ = run.checkouts(hex_commit(job, "fork_commit"), None)
    # Under cargo-nextest, each test in its own process; the doc examples
    # under cargo test --doc.
    run.run(["cargo", "nextest", "run", "--release", "--lib"], cwd=fork)
    run.run(["cargo", "test", "--release", "--doc"], cwd=fork)
    run.run(["cargo", "nextest", "run", "--release", "--tests"], cwd=fork)
    run.run(["cargo", "nextest", "run", "--release", "--manifest-path", "test_vectors/Cargo.toml"], cwd=fork)
    # A checkout of two crates (BLAKE3 #335): the crate with the pool, the
    # stream and the GPU beside the one-thread crate at the root.
    threads = Path(fork) / "threads"
    if (threads / "Cargo.toml").exists():
        run.run(["cargo", "nextest", "run", "--release"], cwd=threads)
        run.run(["cargo", "test", "--release", "--doc"], cwd=threads)
    return "passed"


def mutation_check(run, fork, args):
    """tools/mutants/check.py ARGS in the checkout. Its exit is the verdict:
    0 no new survivor, 1 a survivor differs from its list, 2 only the
    time-outs differ (no verdict on those)."""
    try:
        run.run([sys.executable, "tools/mutants/check.py", *args], cwd=fork)
        return "no new survivor"
    except subprocess.CalledProcessError as e:
        if e.returncode not in (1, 2):
            raise
        # A traceback exits 1 too: a crash, never a verdict (job 1865's
        # preexec_fn failure read as "a survivor differs from its list").
        run.log.flush()
        text = Path(run.log.name).read_text()
        if "Traceback (most recent call last):" in text[text.rfind("$ (cd"):]:
            raise RuntimeError("tools/mutants/check.py stopped with a traceback (its log)") from e
        return {1: "a survivor differs from its list", 2: "only time-outs differ"}[e.returncode]


def mutants(job, run, work, out):
    """tools/mutants/check.py --since SINCE at fork_commit: the change's
    mutants."""
    fork, _ = run.checkouts(hex_commit(job, "fork_commit"), None)
    verdict = mutation_check(run, fork, ["--since", hex_commit(job, "since")])
    last = Path(fork) / "tmp" / "mutants-last"
    if last.is_dir():
        shutil.copytree(last, out / "mutants-last", dirs_exist_ok=True)
    return verdict


def outcomes_of(last):
    """Every mutant of a run's shards (tmp/mutants-last/...), one a line:
    the run, the mutant, its outcome, its seconds. The run's own folder is
    logs by the hundred megabytes; this is what a reader of the lists
    wants of it."""
    lines = []
    for label in ("run", "confirm", "loom"):
        for path in sorted(Path(last).glob(f"{label}/shard*/outcomes.json")):
            for x in json.loads(path.read_text())["outcomes"]:
                scenario = x["scenario"]
                if scenario != "Baseline":
                    seconds = sum(phase.get("duration", 0) for phase in x.get("phase_results", []))
                    lines.append(f"{label}\t{scenario['Mutant']['name']}\t{x['summary']}\t{seconds:.2f}\n")
    return "".join(lines)


def list_rows(text):
    """A list's rows by what names each: file, mutant, and the CPU
    condition its reason opens with (check.py's CONDITION: a name has one
    row a condition)."""
    found = {}
    for line in text.splitlines():
        if line and not line.startswith(("#", "files:", "model files:")):
            file, mutant, _, reason = line.split("\t")
            condition = reason[:reason.index("]") + 1] if reason.startswith("[cpu ") else ""
            found[(file, mutant, condition)] = line
    return found


def list_union(before, written, survived=()):
    """The time-outs as one whole run wrote them, with the rows of the
    commit's list that the run did not see put back (BLAKE3 #380). Where a
    mutant's outcome varies from run to run (on the Apple M6 a changed Ring
    crashes some tests and stops the clock in others, and a run that ends
    at the first failure sees either), --write drops the time-outs one run
    happened not to see, and the next run's list drops others: the list to
    land is what the runs have shown together. A row put back with its
    reason and every place of its name is a known hang (check.py), which
    whole runs skip; any other can read NO LONGER TIMES OUT in a run that
    does not see it, as it could before. A name both lists hold keeps the
    run's row. A name the run saw survive
    (`survived`: file and mutant) is not put back: it would stand in both
    lists, and that is for a person to settle. Returns the text, the rows
    put back, and the rows left out for that reason."""
    old, new = list_rows(before), list_rows(written)
    missing = {key: line for key, line in old.items() if key not in new}
    back = sorted(line for key, line in missing.items() if key[:2] not in survived)
    left = sorted(line for key, line in missing.items() if key[:2] in survived)
    head = [line for line in written.splitlines() if line.startswith(("#", "files:", "model files:"))]
    return "\n".join(head + sorted(list(new.values()) + back)) + "\n", back, left


def mutants_whole(job, run, work, out):
    """One crate's whole mutation run with --write at fork_commit (BLAKE3
    #374): the lists to land (the survivors as this run wrote them; the
    time-outs with the commit's rows the run did not see put back:
    list_union), what they change in the commit's lists, the lists as
    this run alone wrote them, and every mutant's outcome."""
    crate = job.get("crate")
    assert crate in (".", "threads"), 'crate must be "." (the core) or "threads" (the crate in threads/)'
    fork, _ = run.checkouts(hex_commit(job, "fork_commit"), None)
    verdict = mutation_check(run, fork, ["--whole", "--write", "--crate", crate])
    fork = Path(fork)
    lists = fork / "tools" / "mutants" / ("" if crate == "." else crate)
    (out / "lists").mkdir(exist_ok=True)
    (out / "lists-written").mkdir(exist_ok=True)
    known, timeouts = (lists / f"{kind}-{platform.machine()}.txt" for kind in ("known", "timeouts"))
    for written in (known, timeouts):
        if written.exists():
            shutil.copy(written, out / "lists-written" / written.name)
    notes = []
    if timeouts.exists():
        listed = subprocess.run(["git", "show", f"HEAD:{timeouts.relative_to(fork)}"], cwd=fork, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        survived = {key[:2] for key in list_rows(known.read_text())} if known.exists() else set()
        # a list the commit does not hold yet (a new machine's) has no rows to put back
        text, back, left = list_union(listed.stdout if listed.returncode == 0 else "", timeouts.read_text(), survived)
        timeouts.write_text(text)
        notes = [f"put back\t{row}\n" for row in back] + [f"NOT put back: this run saw it survive\t{row}\n" for row in left]
    (out / "lists-put-back.txt").write_text("".join(notes))
    for written in (known, timeouts):
        if written.exists():
            shutil.copy(written, out / "lists" / written.name)
    (out / "lists-diff.txt").write_text(run.run(["git", "diff", "--", str(lists.relative_to(fork))], cwd=fork, capture=True))
    last = fork / "tmp" / "mutants-last" / ("" if crate == "." else crate)
    if last.is_dir():
        (out / "outcomes.tsv").write_text(outcomes_of(last))
    return verdict


def crowd(job, run, work, out):
    fork, bench = run.checkouts(hex_commit(job, "fork_commit"), hex_commit(job, "bench_commit"))
    exe = run.run(["cargo", "run", "--release", "--quiet", "--", "build-against", str(fork)],
                  cwd=bench, capture=True).strip().splitlines()[-1]
    run.run([exe, "crowd"], cwd=out)
    return "ok"


# The jobs that time (README, "Running a job"): on mains power alone. On
# battery the Mac runs another machine's speeds (more calls on the
# efficiency cores, job 351), so such a job is refused before it runs, and
# one whose power leaves mains on the way says so in its status.
TIMED = {"benchmark", "b3sum", "crowd", "example", "check"}

HANDLERS = {"benchmark": benchmark, "b3sum": b3sum, "check": check, "crowd": crowd, "mutants": mutants, "mutants-whole": mutants_whole,
            "example": example, "test": test}
# A job's time limit where it names none: a whole mutation run is longer than the others.
LIMITS = {"mutants-whole": MAX_LIMIT}


def power():
    """The Mac's power state in pmset's terms: the source ("AC Power",
    "Battery Power") and its charge, then the power mode setting in use
    (lowpowermode, or powermode 0 automatic, 1 low, 2 high). On battery a
    thread that sleeps between calls moved to the efficiency cores at
    about 1 GHz (job 351), so every job records it."""
    def pmset(*args):
        return subprocess.run(["pmset", *args], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True).stdout
    batt = pmset("-g", "batt").splitlines()
    source = batt[0].removeprefix("Now drawing from ").strip("'") if batt else "unreported"
    charge = next((word.rstrip(";") for line in batt[1:] for word in line.split() if word.rstrip(";").endswith("%")), None)
    modes = [" ".join(line.split()) for line in pmset("-g").splitlines() if line.split()[:1] in (["powermode"], ["lowpowermode"])]
    return ", ".join([source] + ([charge] if charge else []) + modes)


# The chip's power beside a job ("powermetrics": true), from the one command
# line /etc/sudoers.d/benchrunner-powermetrics lets this user run as root:
# CPU, GPU, and ANE power every 500 ms, stopping by itself after MAX_LIMIT,
# so a runner that dies leaves no sampler running.
POWERMETRICS = ["sudo", "-n", "/usr/bin/powermetrics", "--samplers", "cpu_power", "-i", "500", "-n", "14400"]
assert 14400 * 0.5 >= MAX_LIMIT

# The busiest processes beside a job ("top": true), every second, stopping
# by itself after MAX_LIMIT; top's own share shows in its samples.
TOP = ["/usr/bin/top", "-l", "7200", "-s", "1", "-o", "cpu", "-n", "12", "-stats", "pid,command,cpu,time,user"]
assert 7200 * 1 >= MAX_LIMIT


def process(path, out):
    print(f"runner: {path.name}: started, log in {out / 'runner.log'} until it ends", file=sys.stderr, flush=True)
    verdict = {"job": path.name, "started": utc("%Y-%m-%dT%H:%M:%SZ"), "power at start": power()}
    with open(out / "runner.log", "w") as log:
        try:
            job = json.loads(path.read_text())
            verdict["type"] = job.get("type")
            assert job.get("type") in HANDLERS, f"type must be one of {sorted(HANDLERS)}"
            on_mains = lambda state: state.startswith("AC Power")
            assert job["type"] not in TIMED or on_mains(verdict["power at start"]), \
                f"a {job['type']} job times, so it runs on mains power alone; the Mac is on {verdict['power at start']}: plug it in and queue the job again"
            limit = job.get("time_limit_seconds", LIMITS.get(job["type"], DEFAULT_LIMIT))
            assert isinstance(limit, int) and not isinstance(limit, bool) and 0 < limit <= MAX_LIMIT, f"time_limit_seconds must be 1-{MAX_LIMIT}"
            meters = {}
            meter_files = []
            running = Job(log, limit)
            try:
                for name, command in [("powermetrics", POWERMETRICS), ("top", TOP)]:
                    assert isinstance(job.get(name, False), bool), f"{name}: true or false"
                    if job.get(name):
                        mf = open(out / f"{name}.txt", "w")
                        meter_files.append(mf)
                        meters[name] = subprocess.Popen(command, stdout=mf, stderr=log)
                with tempfile.TemporaryDirectory(dir=HOME / "tmp") as work:
                    verdict["status"] = HANDLERS[job["type"]](job, running, Path(work), out)
            finally:
                if running.left_running:
                    verdict["left running"] = running.left_running
                if running.stacks:
                    verdict["stacks"] = running.stacks
                for name, meter in meters.items():
                    # sudo passes the signal on to powermetrics, which ends its
                    # last sample and exits, as top does. A meter that outlasts
                    # 10 s is killed and noted beside the verdict: it is no
                    # reason to replace the job's own status (it once could,
                    # with "error", from wait's TimeoutExpired).
                    meter.send_signal(signal.SIGINT)
                    try:
                        meter.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        meter.kill()
                        meter.wait()
                        verdict[name] = "did not exit within 10 s of SIGINT; killed (its last samples may be missing)"
                for mf in meter_files:
                    mf.close()
        except TimedOut as e:
            verdict["status"] = "timed out"
            verdict["timed out"] = str(e)
            print(f"runner: timed out: {e}", file=log, flush=True)
        except Exception as e:
            verdict["status"] = "error"
            verdict["error"] = f"{type(e).__name__}: {e}"
            print(f"runner: {verdict['error']}", file=log, flush=True)
    verdict["finished"] = utc("%Y-%m-%dT%H:%M:%SZ")
    verdict["power at end"] = power()
    if verdict.get("type") in TIMED and verdict.get("status") not in (None, "error", "timed out") and not verdict["power at end"].startswith("AC Power"):
        verdict["status"] = f"power left mains during the job ({verdict['status']}): its results are no evidence of speed"
    (out / "verdict.json").write_text(json.dumps(verdict, indent=2) + "\n")
    print(f"runner: {path.name}: {verdict['status']} -> {out}", file=sys.stderr, flush=True)
    return verdict["status"]


def main():
    assert len(sys.argv) in (2, 3), "usage: runner.py EXCHANGE [LABEL] (the folder, or Git clone, holding jobs/ and results/; a label for this machine, chosen by its operator)"
    # The label is the operator's choice, never read from the machine: a
    # host's own name can name its owner, and results are published.
    machine = sys.argv[2] if len(sys.argv) == 3 else "local"
    exchange = Exchange(sys.argv[1], machine)
    assert not exchange.shared or len(sys.argv) == 3, "a shared exchange needs a label for this machine: runner.py EXCHANGE LABEL"
    assert (CARGO_BIN / "cargo").exists(), f"no cargo at {CARGO_BIN}: install rustup for this account"
    (HOME / "tmp").mkdir(exist_ok=True)
    # Every time a job or its meters print is UTC: results are published,
    # and a machine's local time and zone say where it is.
    os.environ["TZ"] = "UTC"
    time.tzset()
    signal.signal(signal.SIGINT, on_signal)
    signal.signal(signal.SIGTERM, on_signal)
    end_leftover_session()
    print(f"runner: {machine} watching {exchange.jobs}{' (shared through Git)' if exchange.shared else ''}; Ctrl-C stops after the current job", file=sys.stderr, flush=True)
    while not stopping:
        exchange.refresh()
        pending = exchange.pending()
        if not pending:
            time.sleep(30 if exchange.shared else 2)
            continue
        out = exchange.claim(pending[0])
        if out is None:
            continue
        # The job's files are written on this machine's own disk and copied
        # into the exchange once the job is over: an exchange on a shared
        # folder is served by another process, and every line written while
        # a job times is that process's work beside the measurement.
        stage = HOME / "tmp" / out.name
        stage.mkdir()
        status = process(pending[0], stage)
        shutil.copytree(stage, out, dirs_exist_ok=True)
        shutil.rmtree(stage)
        exchange.publish(out, f"{pending[0].stem}: {status} ({machine})")


if __name__ == "__main__":
    main()
