#!/usr/bin/env python3
"""Start, watch, and stop long jobs, by name, safely.

    python3 tools/jobs.py start NAME -- COMMAND...   run COMMAND as job NAME
    python3 tools/jobs.py status [NAME]              running or ended, and the log's tail
    python3 tools/jobs.py stop NAME|PID              stop it and every process it started
    python3 tools/jobs.py find TEXT                  the processes whose command line holds TEXT

A job runs in a session of its own (its own process group), with its
output in tmp/jobs/NAME.log and its process ID and start time in
tmp/jobs/NAME.pid, in the checkout this tool sits in; `start` refuses a
NAME still running. A process is the job's only while both match: a
record outlives a reboot, and its number may by then name another
process (one did, and `stop` would have ended it).
`stop` sends TERM to the job's process group and to every descendant of
its leader, waits up to 10 s, then sends KILL to what remains. It never
matches processes by name (as pkill -f does, which can match the shell
that runs it), and it refuses to stop itself or any process it descends
from. A raw PID stops that process's tree the same way. An ended
process whose parent is gone can linger as a zombie (ps state Z: the
VM's PID 1 reaps none); it holds nothing, and this tool counts it as
ended.

A record names its process by when it started, as `started` reads it.

`find` lists processes by their command line, for the PID to stop: it
never lists itself or a process it descends from, the shell that runs it
among them (pgrep -f lists that shell, whose command holds the pattern
too). The VM refuses pkill, pgrep and killall (vm/setup.sh), at Zooko's
request: matching by name has ended the shell running it many times.

Why and when to use it: CONTRIBUTING.md, "Maintainers' conventions".
"""

import os
import signal
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
JOBS = os.path.join(ROOT, "tmp", "jobs")
GRACE_S = 10


def processes():
    """{pid: (ppid, pgid)} for every process, from ps (the same on Linux and macOS)."""
    out = subprocess.run(["ps", "-A", "-o", "pid=,ppid=,pgid="], capture_output=True, text=True, check=True).stdout
    table = {}
    for line in out.split("\n"):
        fields = line.split()
        if len(fields) == 3:
            pid, ppid, pgid = map(int, fields)
            table[pid] = (ppid, pgid)
    return table


def ancestors(table):
    """This process and every process it descends from."""
    seen, pid = set(), os.getpid()
    while pid in table and pid not in seen and pid > 1:
        seen.add(pid)
        pid = table[pid][0]
    return seen


def tree(table, root):
    """`root`, the processes of its group when it leads one, and their descendants."""
    found = {root} if root in table else set()
    found |= {pid for pid, (_, pgid) in table.items() if pgid == root}
    grew = True
    while grew:
        children = {pid for pid, (ppid, _) in table.items() if ppid in found} - found
        found |= children
        grew = bool(children)
    return found


def started(pid):
    """When process `pid` started, as a string that names it alone; None if
    there is no such process. On Linux, the kernel's own count of clock
    ticks from boot to its start (/proc/PID/stat, field 22): ps's lstart
    there is that count added to a boot time ps infers from the wall clock,
    so it moves when the clock is set, and the VM's moved 4.5 minutes under
    a running job, which status then called ended (November 4). On macOS,
    ps's lstart, which the kernel keeps as a time."""
    stat = f"/proc/{pid}/stat"
    if os.path.exists("/proc/self/stat"):
        try:
            text = open(stat).read()
        except FileNotFoundError:
            return None
        # The command sits in parentheses and may hold spaces; the fields follow it.
        return "ticks " + text[text.rindex(")") + 2:].split()[19]
    out = subprocess.run(["ps", "-o", "lstart=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    return out or None


def alive(pid, since=None):
    """Whether `pid` runs, and, given `since`, is the process that started then."""
    if since is not None and started(pid) != since:
        return False
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    # A zombie has ended; only its reaping is left.
    state = subprocess.run(["ps", "-o", "stat=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    return bool(state) and not state.startswith("Z")


def pid_of(name):
    """Job `name`'s process: (its ID, when it started)."""
    path = os.path.join(JOBS, f"{name}.pid")
    if not os.path.exists(path):
        sys.exit(f"jobs: no job named {name} (no {path})")
    pid, since = open(path).read().rstrip("\n").split("\t")
    return int(pid), since


def start(name, command):
    assert name.replace("-", "").replace("_", "").isalnum(), "a job's NAME: letters, digits, - and _"
    assert command, "start NAME -- COMMAND..."
    os.makedirs(JOBS, exist_ok=True)
    pid_path, log_path = os.path.join(JOBS, f"{name}.pid"), os.path.join(JOBS, f"{name}.log")
    if os.path.exists(pid_path) and alive(*pid_of(name)):
        sys.exit(f"jobs: {name} is still running (PID {pid_of(name)[0]}); stop it first")
    log = open(log_path, "w")
    proc = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, start_new_session=True)
    since = started(proc.pid)
    assert since, f"jobs: {name}'s process {proc.pid} ended before its start time was read"
    with open(pid_path, "w") as f:
        f.write(f"{proc.pid}\t{since}\n")
    print(f"jobs: {name} started, PID {proc.pid}, log {log_path}")


def status(name=None):
    names = [name] if name else sorted(f[:-4] for f in os.listdir(JOBS) if f.endswith(".pid")) if os.path.isdir(JOBS) else []
    if not names:
        print("jobs: none")
    for n in names:
        pid, since = pid_of(n)
        log = os.path.join(JOBS, f"{n}.log")
        print(f"{n}: PID {pid}, {'running' if alive(pid, since) else 'ended'}, log {log}")
        if name and os.path.exists(log):
            print("".join(open(log, errors="replace").readlines()[-5:]), end="")


def stop(target):
    if target.isdigit():
        root = int(target)
    else:
        root, since = pid_of(target)
        if not alive(root, since):
            print(f"jobs: nothing of {target} is running")
            return
    table = processes()
    victims = tree(table, root)
    mine = ancestors(table)
    assert not (victims & mine), f"jobs: PID {root}'s tree holds this tool or a process it descends from; refusing"
    victims = {p for p in victims if alive(p)}
    if not victims:
        print(f"jobs: nothing of {target} is running")
        return
    for sig in (signal.SIGTERM, signal.SIGKILL):
        for pid in victims:
            try:
                os.kill(pid, sig)
            except ProcessLookupError:
                pass
        deadline = time.monotonic() + GRACE_S
        while time.monotonic() < deadline and any(alive(p) for p in victims):
            time.sleep(0.1)
        left = {p for p in victims if alive(p)}
        if not left:
            print(f"jobs: stopped {target}: {len(victims)} processes ({'TERM' if sig == signal.SIGTERM else 'KILL'})")
            return
        victims = left
    sys.exit(f"jobs: still alive after KILL: {sorted(victims)}")


def find(text):
    """Each process whose command line holds `text` (a literal string),
    but this tool and the processes it descends from: its PID, when it
    started, and its command line."""
    table = processes()
    mine = ancestors(table)
    out = subprocess.run(["ps", "-A", "-o", "pid=,args="], capture_output=True, text=True, check=True).stdout
    found = 0
    for line in out.split("\n"):
        pid, _, args = line.strip().partition(" ")
        if pid.isdigit() and int(pid) not in mine and text in args and alive(int(pid)):
            print(f"{pid}\t{started(int(pid))}\t{args.strip()}")
            found += 1
    if not found:
        print(f"jobs: no process's command line holds {text!r}")


def main():
    args = sys.argv[1:]
    if not args or args[0] in ("-h", "--help"):
        print(__doc__)
        return
    command, rest = args[0], args[1:]
    if command == "start":
        assert len(rest) >= 2 and "--" in rest, "start NAME -- COMMAND..."
        split = rest.index("--")
        assert split == 1, "start NAME -- COMMAND..."
        start(rest[0], rest[2:])
    elif command == "status":
        status(*rest[:1])
    elif command == "stop":
        assert len(rest) == 1, "stop NAME|PID"
        stop(rest[0])
    elif command == "find":
        assert len(rest) == 1 and rest[0], "find TEXT"
        find(rest[0])
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
