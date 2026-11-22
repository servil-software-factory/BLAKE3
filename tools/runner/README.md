# Mac benchmark runner

`runner.py` runs jobs on a Mac as the hidden standard account
`benchrunner`, with code from GitHub at the commits each job names.
Jobs and results pass through an *exchange*, a folder holding `jobs/`
and `results/`. The team's exchange is the Git repository
[servil-software-factory/mac-runner](https://github.com/servil-software-factory/mac-runner):
anyone with write access pushes a job file into `jobs/`, a runner pulls
it, claims it by pushing `results/<job>.<time>-<machine>/claimed.json`,
runs it, and pushes the results' text (`verdict.json`, `runner.log`,
samples, reports). Several runners may share one exchange; the claim
makes sure a job runs once, and the folder's name says which machine ran
it (results from different machines never compare). The team runs two
Mac Mini M6s: one takes timing jobs and nothing else, the other the
suites and the mutants. A private exchange is any plain folder. A job's files are written on the runner's own disk
and copied into the exchange when the job ends, so a folder served by
another process (a mount) costs that process nothing while a job times. Operator notes are in the private repository
servil-software-factory/ops. The runner keeps its clones of BLAKE3-Servil and of
bench-hashes, side by side, in `~benchrunner/checkouts/`, and moves
them to each job's commits, so Cargo rebuilds only what changed: a
benchmark job builds bench-hashes against BLAKE3-Servil as it is, through
`bench-hashes build-against` (in a directory with its own `Cargo.lock`;
the committed lock is never written), and check jobs build their two
sides the same way under BLAKE3-Servil's `target/check/`. Before each job it
removes anything untracked from both clones, so each reads as the commit
the job names. The job format is the docstring at the top of
`runner.py`.

## The account (once)

    pw="$(openssl rand -base64 32)"
    sudo sysadminctl -addUser benchrunner -fullName "Bench Runner" -password "$pw"
    sudo dscl . create /Users/benchrunner IsHidden 1
    sudo dscl . -create /Users/benchrunner UserShell /usr/bin/false
    sudo dscl . -passwd /Users/benchrunner "$(openssl rand -base64 32)"

Nobody can log in to it; `sudo -u benchrunner` runs commands as it.

## Setting up and starting

    sh tools/runner/setup-mac.sh

Run it as yourself; it asks for sudo when it needs it. It makes the
account's home private, installs Python 3 when absent, and gives the account
the same rustc as yours (matched by commit hash). It then makes the
exchange folders (`jobs/` yours and readable, `results/` the runner's and
readable) and lets the account pass through the folders above them. Last,
it copies `runner.py` to `/Users/Shared/bench-runner/` (yours, read-only to
the runner) and starts it. Steps already done are skipped, so the same
command starts the runner every time, including after a change to
`runner.py`. Ctrl-C stops the runner after the current job.

## Running a job

Write `jobs/NNN-name.json` naming pushed commits (a branch's, or a pull
request's head, from a fork too) and push it to the exchange repository (take the next free number); wait with
`python3 tools/runner/wait_for.py NNN-name PATH-OF-YOUR-CLONE`, which
pulls before each look; results land in `results/` with the machine's
name in the folder. A job runs once per file name, so a changed job takes
a new number. The job format is the docstring at the top of `runner.py`.
A machine that serves the shared exchange runs nothing else, so a
timing job there needs no arrangement with anyone.

## A machine for the shared exchange (once)

After the account above, with the command-line tools installed
(`xcode-select --install`), run from a checkout of BLAKE3-Servil, with a
label of your choosing for the machine:

    EXCHANGE_REPO=git@github.com:servil-software-factory/mac-runner.git \
    RUNNER_TOOLCHAIN=nightly-2026-09-24 MACHINE=LABEL \
    sh tools/runner/setup-mac.sh

The first run prints the runner's public key and stops: add it to the
exchange repository as a deploy key with write access (Settings, Deploy
keys), then run the command again. It installs the named nightly for
`benchrunner` (the one the other runners use, so results read the same
compiler), cargo-nextest and cargo-mutants, clones the exchange, and
starts the runner; it prints which of the two sudoers lines below are
missing. Run it again after a reviewed change to `runner.py`.

While a job that times runs (`benchmark`, `b3sum`, `crowd`, `example`, `check`),
nothing else runs on the machine: a machine on the shared exchange runs
nothing else by design, and a private runner's operator keeps the
machine idle and its monitoring tools closed (their load reads as busy
windows). To name the programs behind a run's busy windows, add
`"top": true` to the job and read its `top.txt` at the windows' times.

A timing job runs on mains power: on battery the machine runs another
machine's speeds (more calls after a pause on its efficiency cores). So
the runner refuses a job that times when the machine is on battery (its
status says so: plug in and queue it again), and a job whose power leaves
mains on the way ends with a status that says its results are no
evidence; every job's `verdict.json`
records the power state at its start and end. A `test` job (BLAKE3-Servil's suites, natively) needs
no quiet machine.

A direct A/B is four `benchmark` jobs, old new new old, back to back,
read with `bench-hashes compare`; a calibration is one job with
`"repeat": N`. Mac-only measurements (cycles by core kind, QoS) go in a
`probe/<topic>` branch with its own `examples/host_lab.rs`; the branch
becomes a tag of the same name once its finding is written down.

## Tested

Every job type runs on the Mac. A thorough `--all` benchmark run under the
runner matches the same run from the user's Terminal (704 cells, median
ratio 1.001).

The runner's own tests (`python3 -m unittest discover -s tools/runner`)
run in CI on Linux and macOS, under the system's CPython and under PyPy.

## The chip's power (`"powermetrics": true`)

Energy is a lab instrument (Zooko): measured in experiments here, and kept
out of the benchmark and the gate, where a number would become a goal.
Heat confounds it: back-to-back full-load runs throttle within minutes, so
each run starts after a minute's rest, in an order that cancels drift (0,
1, 2, 3, 3, 2, 1, 0). macOS's per-process estimate (`ri_energy_nj`) is a
model, -16% to +30% against the chip's power, unfit to compare builds.

A job may record the chip's power beside it. The runner's user runs one
fixed command line as root, which only prints and stops by itself after
two hours. Allow it once, with `sudo visudo -f
/etc/sudoers.d/benchrunner-powermetrics`, holding exactly:

    benchrunner ALL=(root) NOPASSWD: /usr/bin/powermetrics --samplers cpu_power -i 500 -n 14400

Check it with `sudo -u benchrunner sudo -n /usr/bin/powermetrics --samplers
cpu_power -i 500 -n 14400` (Ctrl-C after a sample). Any other arguments
stay refused, so the runner cannot use `powermetrics -o` to write files as
root.

## Root for a probe

Two lines in `/etc/sudoers.d/` let the runner's account run two things as
root: `powermetrics` (above) and one probe path. Apple's per-thread
counter interface (`kpc`, in the private kperf framework) configures
counters only as root: the cache-miss, TLB-miss and stall events beside
cycles and instructions, and its own read call (bench-hashes'
`process/fast-feedback/mac-clocks/README.md` has what it buys). A probe
that needs it is built by the job into
`/Users/Shared/bench-runner/root-probes/kpc-probe` and run through
`sudo -n` at that one path. Allow it once, with
`sudo visudo -f /etc/sudoers.d/benchrunner-kpc-probe`, holding exactly:

    benchrunner ALL=(root) NOPASSWD: /Users/Shared/bench-runner/root-probes/kpc-probe

The runner's user writes that path, so this rule lets a job run code of
its choosing as root: it belongs on a machine that holds nothing else.
Without the rule the job records sudo's refusal and the rest of its
results stand.
