#!/usr/bin/env python3
"""Benchmark runner for BLAKE3-Servil, for the Mac's `benchrunner` account.

    sudo -u benchrunner -H python3 runner.py EXCHANGE

EXCHANGE holds `jobs/` (JSON job files, read here) and `results/` (written
here, one folder per job). Code comes from GitHub alone, at the commits a
job names. Done jobs are listed in ~/.benchrunner_done (the runner's home).
Ctrl-C stops the runner after the current job.

Jobs (commits as 7-40 hex digits; time_limit_seconds optional, default
1800, up to 7200; mutants-whole defaults to 7200):

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
    {"type": "b3sum", "fork_commit": "...", "bench_commit": "...",
     "against": "...", "quick": true}

(A b3sum job builds the contenders, bench-hashes' tools/b3sum-contenders.py
at the BLAKE3-Servil commit: official BLAKE3's b3sum, BLAKE3-Servil's as b3sum and
b3sum-1-thread, the system's sum programs; and runs bench-hashes b3sum on
the files in ~/b3sum-bench-files, made there once. With optional "against"
naming an earlier BLAKE3-Servil commit (such as the last release), both
BLAKE3-Servil builds run in the same rounds with the earlier build first
as the judged base, beside official b3sum. The report, samples
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
and the other crate's 23 (November 12, 2026).)
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
progress.json (the current step and completed samples), verdict.json,
stacks.txt (when inspected, cancelled, or stopped by the job's time
limit), and the job's own files: the benchmark's report, graph, and
samples; host_lab's report.

The checkouts persist between jobs (CHECKOUTS: BLAKE3-Servil, and bench-hashes
beside it), moved to each job's commits with a forced checkout that
rewrites only the files that differ, so Cargo rebuilds only what changed:
BLAKE3-Servil's target directory serves test and example jobs, check's
sides (target/check/ in BLAKE3-Servil) its jobs, and bench-hashes' own build
against BLAKE3-Servil as it is (`bench-hashes build-against`) benchmark jobs.
"""
import ctypes
import json
import os
import platform
import re
import select
import selectors
import shutil
import signal
import subprocess
import sys
import tempfile
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

stopping = False
inspect_requested = False
cancel_requested = False
_wake_pipe = None


def _ensure_wake_pipe():
    global _wake_pipe
    if _wake_pipe is None:
        r_fd, w_fd = os.pipe()
        os.set_blocking(r_fd, False)
        os.set_blocking(w_fd, False)
        _wake_pipe = (r_fd, w_fd)
    return _wake_pipe


def _wake_supervisor():
    if _wake_pipe is not None:
        try:
            os.write(_wake_pipe[1], b"1")
        except OSError:
            pass


def _drain_wake_pipe():
    if _wake_pipe is None:
        return
    r_fd = _wake_pipe[0]
    while True:
        try:
            if not os.read(r_fd, 64):
                break
        except OSError:
            break

# The session of the command a job is running: its PID and start time. A
# runner stopped some other way than Ctrl-C (its terminal closed, a kill)
# leaves that session running, and the next runner would measure beside
# it: two checks of job 1939 ran at once, each the other's load
# (bench-hashes #135, November 6, 2026). The next runner ends it first.
SESSION_FILE = Path.home() / "runner-session.txt"


def started(pid):
    """A process's start time as ps prints it, or None when it is gone."""
    out = subprocess.run(["ps", "-o", "lstart=", "-p", str(pid)], capture_output=True, text=True)
    return out.stdout.strip() or None


def end_leftover_session():
    """End the session an earlier runner left running, if it is the one it
    recorded (the same PID and start time; this account's processes only)."""
    if not SESSION_FILE.exists():
        return
    pid, recorded = SESSION_FILE.read_text().split("\t", 1)
    if started(int(pid)) == recorded.strip():
        print(f"runner: ending the session an earlier runner left running (PID {pid})", file=sys.stderr, flush=True)
        os.killpg(int(pid), signal.SIGKILL)
    SESSION_FILE.unlink()


def on_signal(signum, _frame):
    global stopping
    stopping = True
    print("runner: stopping after the current job", file=sys.stderr, flush=True)


def on_inspect(signum, _frame):
    global inspect_requested
    inspect_requested = True
    _wake_supervisor()


def on_cancel(signum, _frame):
    global cancel_requested
    cancel_requested = True
    _wake_supervisor()


def hex_commit(job, key):
    value = job[key]
    assert isinstance(value, str) and 7 <= len(value) <= 40 and all(c in "0123456789abcdef" for c in value), \
        f"{key} must be 7-40 lowercase hex digits, got {value!r}"
    return value


def environment():
    env = {"PATH": f"{CARGO_BIN}:/usr/bin:/bin:/usr/sbin:/sbin", "HOME": str(HOME),
           "LANG": "en_US.UTF-8", "TMPDIR": str(HOME / "tmp") + "/", "TZ": "UTC"}
    return env


def session_processes(pgid):
    """Live processes in `pgid`'s process group as (pid, line) pairs."""
    out = subprocess.run(
        ["ps", "-eo", "pid=,pgid=,ppid=,pcpu=,etime=,state=,command="],
        capture_output=True, text=True, check=False,
    )
    rows = []
    for line in out.stdout.splitlines():
        parts = line.split(None, 6)
        if len(parts) < 7:
            continue
        try:
            pid, pgrp = int(parts[0]), int(parts[1])
        except ValueError:
            continue
        if pgrp == pgid:
            rows.append((pid, line.strip()))
    return rows


def capture_stacks(pgid, out, reason):
    """Record every live process and its thread stacks in `pgid`'s session
    into `out/stacks.txt`, without terminating the process."""
    out = Path(out)
    procs = session_processes(pgid)
    chunks = [f"=== {utc('%Y-%m-%dT%H:%M:%SZ')} ({reason}, session pgid={pgid}) ===\n"]
    if not procs:
        chunks.append("no live processes in session\n")
    else:
        chunks.append("PID PGID PPID %CPU ELAPSED STATE COMMAND\n")
        for _, line in procs:
            chunks.append(line + "\n")
        for pid, _ in procs:
            chunks.append(f"\n--- pid {pid} ---\n")
            if sys.platform == "darwin" and Path("/usr/bin/sample").exists():
                with tempfile.NamedTemporaryFile(prefix="sample-", suffix=".txt", delete=False) as tf:
                    sample_path = Path(tf.name)
                try:
                    r = subprocess.run(
                        ["/usr/bin/sample", str(pid), "1", "-file", str(sample_path)],
                        capture_output=True, text=True, check=False, timeout=15,
                    )
                    text = sample_path.read_text(errors="replace") if sample_path.exists() else ""
                    chunks.append(text if text.strip() else f"sample exited {r.returncode}: {r.stderr.strip()}\n")
                except Exception as e:
                    chunks.append(f"sample failed: {e}\n")
                finally:
                    sample_path.unlink(missing_ok=True)
            elif Path(f"/proc/{pid}/task").is_dir():
                def read_proc(path):
                    try:
                        return path.read_text(errors="replace").strip()
                    except OSError:
                        return ""
                for task in sorted(Path(f"/proc/{pid}/task").iterdir(), key=lambda p: p.name):
                    comm = read_proc(task / "comm") or "?"
                    wchan = read_proc(task / "wchan") or "?"
                    stack = read_proc(task / "stack")
                    chunks.append(f"tid {task.name} ({comm}) wchan={wchan}\n")
                    if stack:
                        chunks.append(stack + "\n")
            else:
                chunks.append("no stack sampler available on this platform\n")
    chunks.append("\n")
    with open(out / "stacks.txt", "a") as f:
        f.write("".join(chunks))
    return out / "stacks.txt"


def _open_dir_watch(folder):
    """An event-driven directory watch fd for `folder` (kqueue vnode on
    macOS, inotify on Linux), or (None, None) when unavailable."""
    if folder is None:
        return None, None
    folder = Path(folder)
    if not folder.is_dir():
        return None, None
    if hasattr(select, "kqueue"):
        flags = os.O_RDONLY | getattr(os, "O_EVTONLY", 0)
        return "vnode", os.open(folder, flags)
    if sys.platform == "linux":
        try:
            libc = ctypes.CDLL("libc.so.6", use_errno=True)
            IN_NONBLOCK = 0o4000
            IN_CLOEXEC = 0o2000000
            IN_MASK = 0x00000100 | 0x00000080 | 0x00000008  # CREATE | MOVED_TO | CLOSE_WRITE
            ifd = libc.inotify_init1(IN_NONBLOCK | IN_CLOEXEC)
            if ifd >= 0:
                wd = libc.inotify_add_watch(ifd, os.fsencode(folder), IN_MASK)
                if wd >= 0:
                    return "inotify", ifd
                os.close(ifd)
        except OSError:
            pass
    return None, None


class Cancelled(Exception):
    """A cancel request ended the job: its own verdict, "cancelled", never
    "error" or "timed out"."""


class TimedOut(Exception):
    """The job's time limit ended it: its own verdict, "timed out", never
    "error" (a crash) or a handler's answer."""


class Job:
    """One job's log, progress record, and event-driven process supervisor.
    Every command runs through `run`. A job's time limit is honored on the
    clock, capturing thread stacks before terminating the session."""

    def __init__(self, log, limit=DEFAULT_LIMIT, out=None, control_dir=None, job_stem=None):
        self.log = log
        self.limit = limit
        self.deadline = (time.monotonic() + limit) if limit is not None else None
        self.out = Path(out) if out is not None else None
        self.control_dir = Path(control_dir) if control_dir is not None else None
        self.job_stem = job_stem
        self.started_utc = utc("%Y-%m-%dT%H:%M:%SZ")
        self.step = 0
        self.inspections = []

    def _control_path(self, kind):
        if self.control_dir is None or not self.job_stem:
            return None
        return self.control_dir / f"{self.job_stem}.{kind}"

    def _consume_control(self, kind):
        path = self._control_path(kind)
        if path is not None and path.exists():
            try:
                path.unlink()
            except OSError:
                pass
            return True
        return False

    def write_progress(self, state, cmd=None, pid=None):
        if self.out is None:
            return
        samples_dir = self.out / "samples"
        samples = sorted(p.name for p in samples_dir.glob("run-*.tsv")) if samples_dir.is_dir() else []
        tail = []
        log_name = getattr(self.log, "name", None)
        if log_name and Path(log_name).is_file():
            self.log.flush()
            lines = [ln for ln in Path(log_name).read_text(errors="replace").splitlines() if ln.strip()]
            tail = lines[-5:]
        payload = {
            "state": state,
            "step": self.step,
            "started": self.started_utc,
            "updated": utc("%Y-%m-%dT%H:%M:%SZ"),
            "command": " ".join(cmd) if cmd else None,
            "pid": pid,
            "samples": samples,
            "log_tail": tail,
        }
        tmp = self.out / "progress.json.tmp"
        tmp.write_text(json.dumps(payload, indent=2) + "\n")
        tmp.replace(self.out / "progress.json")

    def _check_controls(self, proc, cmd):
        global inspect_requested, cancel_requested
        if inspect_requested or self._consume_control("inspect"):
            inspect_requested = False
            if self.out is not None:
                at = utc("%Y-%m-%dT%H:%M:%SZ")
                t0 = time.monotonic()
                capture_stacks(proc.pid, self.out, "inspect")
                self.inspections.append({"at": at, "duration_seconds": round(time.monotonic() - t0, 3)})
                self.write_progress("running", cmd, proc.pid)
        if cancel_requested or self._consume_control("cancel"):
            cancel_requested = False
            try:
                if self.out is not None:
                    capture_stacks(proc.pid, self.out, "cancelled")
            finally:
                os.killpg(proc.pid, signal.SIGKILL)
                proc.wait()
            if self.out is not None:
                self.write_progress("cancelled", cmd, None)
            raise Cancelled(f"cancelled during: {' '.join(cmd)}")

    def _stop_on_timeout(self, proc, cmd):
        try:
            if self.out is not None:
                capture_stacks(proc.pid, self.out, "timed out")
        finally:
            os.killpg(proc.pid, signal.SIGKILL)
            proc.wait()
        if self.out is not None:
            self.write_progress("timed_out", cmd, None)
        raise TimedOut(f"the job's {self.limit} s limit ended: {' '.join(cmd)}")

    def _wait_child(self, proc, cmd, capture):
        """Wait for `proc` to exit using OS process-exit events (kqueue
        EVFILT_PROC on macOS, pidfd_open on Linux) while draining captured
        stdout and reacting immediately to inspect/cancel events."""
        wake_r, _ = _ensure_wake_pipe()
        watch_kind, watch_fd = _open_dir_watch(self.control_dir)
        out_chunks = []
        stdout_fd = proc.stdout.fileno() if (capture and proc.stdout is not None) else None
        if stdout_fd is not None:
            os.set_blocking(stdout_fd, False)

        def read_stdout():
            nonlocal stdout_fd
            if stdout_fd is None:
                return
            while True:
                try:
                    data = os.read(stdout_fd, 65536)
                except BlockingIOError:
                    return
                except OSError:
                    data = b""
                if not data:
                    stdout_fd = None
                    return
                out_chunks.append(data)

        def remaining():
            if self.deadline is None:
                return None
            return max(0.0, self.deadline - time.monotonic())

        try:
            self._check_controls(proc, cmd)
            if hasattr(select, "kqueue"):
                kq = select.kqueue()
                try:
                    exited = proc.poll() is not None
                    if not exited:
                        try:
                            kq.control([select.kevent(proc.pid, filter=select.KQ_FILTER_PROC,
                                                      flags=select.KQ_EV_ADD | select.KQ_EV_ONESHOT,
                                                      fflags=select.KQ_NOTE_EXIT)], 0)
                        except ProcessLookupError:
                            exited = True
                    kq.control([select.kevent(wake_r, filter=select.KQ_FILTER_READ, flags=select.KQ_EV_ADD)], 0)
                    if watch_fd is not None and watch_kind == "vnode":
                        kq.control([select.kevent(watch_fd, filter=select.KQ_FILTER_VNODE,
                                                  flags=select.KQ_EV_ADD | select.KQ_EV_CLEAR,
                                                  fflags=select.KQ_NOTE_WRITE)], 0)
                    if stdout_fd is not None:
                        kq.control([select.kevent(stdout_fd, filter=select.KQ_FILTER_READ, flags=select.KQ_EV_ADD)], 0)
                    while not exited or stdout_fd is not None:
                        rem = remaining()
                        if rem is not None and rem <= 0:
                            self._stop_on_timeout(proc, cmd)
                        events = kq.control(None, 8, rem)
                        if not events and not exited:
                            continue
                        for ev in events:
                            if ev.filter == select.KQ_FILTER_PROC:
                                exited = True
                            elif ev.ident == wake_r:
                                _drain_wake_pipe()
                            elif stdout_fd is not None and ev.ident == stdout_fd:
                                read_stdout()
                                if stdout_fd is None:
                                    try:
                                        kq.control([select.kevent(ev.ident, filter=select.KQ_FILTER_READ, flags=select.KQ_EV_DELETE)], 0)
                                    except OSError:
                                        pass
                        if not exited or stdout_fd is not None:
                            self._check_controls(proc, cmd)
                finally:
                    kq.close()
            else:
                sel = selectors.DefaultSelector()
                pidfd = None
                try:
                    exited = proc.poll() is not None
                    if not exited and hasattr(os, "pidfd_open"):
                        try:
                            pidfd = os.pidfd_open(proc.pid, 0)
                            sel.register(pidfd, selectors.EVENT_READ, "exit")
                        except ProcessLookupError:
                            exited = True
                    sel.register(wake_r, selectors.EVENT_READ, "wake")
                    if watch_fd is not None and watch_kind == "inotify":
                        sel.register(watch_fd, selectors.EVENT_READ, "watch")
                    if stdout_fd is not None:
                        sel.register(stdout_fd, selectors.EVENT_READ, "stdout")
                    while not exited or stdout_fd is not None:
                        rem = remaining()
                        if rem is not None and rem <= 0:
                            self._stop_on_timeout(proc, cmd)
                        poll_timeout = rem if (pidfd is not None or exited) else (min(rem, 0.2) if rem is not None else 0.2)
                        ready = sel.select(timeout=poll_timeout)
                        for key, _ in ready:
                            tag = key.data
                            if tag == "exit":
                                exited = True
                                sel.unregister(key.fileobj)
                            elif tag == "wake":
                                _drain_wake_pipe()
                            elif tag == "watch":
                                while True:
                                    try:
                                        if not os.read(watch_fd, 4096):
                                            break
                                    except OSError:
                                        break
                            elif tag == "stdout":
                                prev_fd = stdout_fd
                                read_stdout()
                                if stdout_fd is None and prev_fd is not None:
                                    sel.unregister(prev_fd)
                        if not exited and pidfd is None and proc.poll() is not None:
                            exited = True
                        if not exited or stdout_fd is not None:
                            self._check_controls(proc, cmd)
                finally:
                    sel.close()
                    if pidfd is not None:
                        os.close(pidfd)
            proc.wait()
            return b"".join(out_chunks).decode("utf-8", errors="replace") if capture else None
        finally:
            if proc.stdout is not None:
                proc.stdout.close()
            if watch_fd is not None:
                os.close(watch_fd)

    def run(self, cmd, cwd, capture=False):
        if self.deadline is not None and self.deadline - time.monotonic() <= 0:
            raise TimedOut(f"the job's {self.limit} s limit passed before: {' '.join(cmd)}")
        self.step += 1
        print(f"$ (cd {cwd} && {' '.join(cmd)})", file=self.log, flush=True)
        # A new session, so Ctrl-C in the runner's terminal reaches the
        # runner alone and the current job finishes.
        proc = subprocess.Popen(cmd, cwd=cwd, env=environment(), start_new_session=True,
                                stdout=subprocess.PIPE if capture else self.log, stderr=self.log)
        SESSION_FILE.write_text(f"{proc.pid}\t{started(proc.pid)}\n")
        self.write_progress("running", cmd, proc.pid)
        stdout = self._wait_child(proc, cmd, capture)
        if proc.returncode != 0:
            self.write_progress("failed", cmd, None)
            raise subprocess.CalledProcessError(proc.returncode, cmd)
        self.write_progress("step_done", cmd, None)
        return stdout

    def sync(self, url, dest, commit):
        """`dest`, a clone of `url` kept between jobs, at `commit`: a commit
        on a branch or at a pull request's head (fetched as
        refs/remotes/origin/pr/N), so a job names a pull request's head as
        it is, from a fork too."""
        if not (dest / ".git").exists():
            self.run(["git", "clone", "--quiet", url, str(dest)], cwd=dest.parent)
        else:
            self.run(["git", "fetch", "--quiet", "--prune", "--tags", "origin"], cwd=dest)
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
    revs = [old_commit, fork_commit] if old_commit else [fork_commit]
    # One contender a line, each one argument (a command may hold flags).
    contenders = [line for line in run.run([sys.executable, "tools/b3sum-contenders.py", str(fork), str(HOME / "b3sum-contenders"), *revs],
                                           cwd=bench, capture=True).splitlines() if "=" in line]
    if old_commit:
        def matches(line, commit):
            key = line.split("=", 1)[0]
            return key.startswith("b3sum-") and commit.startswith(key.removeprefix("b3sum-"))
        old_lines = [c for c in contenders if matches(c, old_commit)]
        new_lines = [c for c in contenders if matches(c, fork_commit)]
        off_lines = [c for c in contenders if c.split("=", 1)[0] == "official"]
        assert old_lines and new_lines, f"b3sum-contenders.py did not return builds for both {old_commit[:7]} and {fork_commit[:7]}"
        contenders = old_lines + new_lines + off_lines
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
        try:
            return subprocess.run(["pmset", *args], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True).stdout
        except FileNotFoundError:
            return ""
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


def process(path, out, control_dir=None):
    global inspect_requested, cancel_requested
    inspect_requested = False
    cancel_requested = False
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
            assert isinstance(limit, int) and not isinstance(limit, bool) and 0 < limit <= MAX_LIMIT, \
                f"time_limit_seconds must be 1-{MAX_LIMIT}"
            meters = {}
            for name, command in [("powermetrics", POWERMETRICS), ("top", TOP)]:
                assert isinstance(job.get(name, False), bool), f"{name}: true or false"
                if job.get(name):
                    meters[name] = subprocess.Popen(command, stdout=open(out / f"{name}.txt", "w"), stderr=log)
            try:
                (HOME / "tmp").mkdir(exist_ok=True)
                runner_job = None
                with tempfile.TemporaryDirectory(dir=HOME / "tmp") as work:
                    runner_job = Job(log, limit=limit, out=out, control_dir=control_dir, job_stem=path.stem)
                    verdict["status"] = HANDLERS[job["type"]](job, runner_job, Path(work), out)
            finally:
                if runner_job is not None and runner_job.inspections:
                    verdict["inspections"] = runner_job.inspections
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
        except Cancelled as e:
            verdict["status"] = "cancelled"
            verdict["cancelled"] = str(e)
            print(f"runner: cancelled: {e}", file=log, flush=True)
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
    if verdict.get("type") in TIMED and verdict.get("status") not in (None, "error") and not verdict["power at end"].startswith("AC Power"):
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
    _ensure_wake_pipe()
    signal.signal(signal.SIGINT, on_signal)
    signal.signal(signal.SIGTERM, on_signal)
    signal.signal(signal.SIGUSR1, on_inspect)
    signal.signal(signal.SIGUSR2, on_cancel)
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
        control_dir = None if exchange.shared else exchange.jobs
        status = process(pending[0], stage, control_dir=control_dir)
        shutil.copytree(stage, out, dirs_exist_ok=True)
        shutil.rmtree(stage)
        exchange.publish(out, f"{pending[0].stem}: {status} ({machine})")


if __name__ == "__main__":
    main()
