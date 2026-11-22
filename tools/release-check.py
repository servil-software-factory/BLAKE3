#!/usr/bin/env python3
"""The release checklist's mechanical steps, read from where each gate
left its result, for one commit (CONTRIBUTING.md, "Releases").

    python3 tools/release-check.py [COMMIT] [--previous TAG]  # default: main's tip

It runs nothing heavy. For each step it reads the evidence the gate
wrote and prints one line: PASS, FAIL, OPEN (no evidence yet, with the
command that makes it), or, each its own outcome, TIMED OUT (the gate, or
GitHub's answer here, ran out of time) and NO VERDICT (the gate could not
judge: a check whose machine was busy). Exit status 0 only when every step
passes; 1 when a step failed or is open; 2 when every other step passes
and the rest timed out or have no verdict.

- CI: a workflow_dispatch run on the commit (every job succeeded).
  Pull-request runs check a synthetic merge by default, not the head SHA
  GitHub associates with the run, so they are not release evidence.
- Long proofs: the commit's GitHub status "long proofs" (long-proofs.sh).
- Mac check: the timing Mac's check of every cell of this commit against
  the previous release, in a clone of the Mac runners' exchange
  (servil-software-factory/mac-runner; $MAC_RUNNER, or mac-runner beside
  the main checkout; pull it first). `mac_check` has the rules. In
  short: a results folder is NAME.TIME-MACHINE, its job is
  jobs/NAME.json, and the records that count are the timing Mac's whose
  job is a "check" with "cells": "all" naming this commit as new_commit
  and the previous release as old_commit. The step passes when every
  such record is whole (a claim and a finished verdict that agree with
  the folder), none is still queued, and the one the Mac started last
  names both commits in full and says "no regression" of a run on AC
  power that nobody inspected. A record that is not whole, a last
  record whose job cannot be told from such a check, two that started
  last in one second: NO VERDICT, with the folder named. An earlier
  pass never stands in.
- The logs, each named for the commit it ran on (C: the commit's first 12
  hex digits), so an earlier commit's log cannot stand in for it:
  - Full mutation run: tmp/night/mutants-C.log, with one complete section
    per crate (core and threads), each naming this commit and matching its
    survivor and timeout lists, from
    `python3 tools/mutants/check.py --whole` with the commit checked out.
  - check-builds: tmp/night/check-builds-C.log ending "every build passes".
  - Publish dry run: tmp/night/publish-dry-run-C.log, from the
    packages' dry-run in CONTRIBUTING.md, followed by "publish-dry-run: ok"
    only on success.
- The release's artifacts, gathered in this checkout's dist/ before any
  is published: b3sum for each machine (tools/b3sum-release.sh writes it
  there) and the four crates' archives (the packages' dry-run's, copied
  there), each a clean build of this commit by tools/release_artifacts.py:
  the programs by the build version they hold and, byte for byte, by the
  builds GitHub's machines made of the commit in the CI run above (the
  build is reproducible, so two honest builders agree, and neither's
  word is taken alone); the crates file by file against the commit. 0.9.0's two b3sum programs were built in checkouts
  holding untracked files and say "dirty" (BLAKE3 #435); no step looked.
- The pages: README.md, QUALITY.md, CHANGELOG.md changed since the
  previous release (a release rewrites them for its readers).

By default the previous release is the nearest reachable v* tag other
than the candidate. If a patch release is on a separate line, pass its
published tag with --previous. The tag must name an older version commit.
The first release has no prior tag. Its checks run against the main line
before the release's changes, and its pages are read fresh.
"""

import argparse
import datetime
import io
import json
import socket
import stat
import urllib.error
from pathlib import Path, PurePath
import os
import re
import subprocess
import sys
import tempfile
import urllib.request
import zipfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import release_artifacts  # noqa: E402 (beside this file)

ROOT = subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True, check=True).stdout.strip()
# The main checkout holds the token, the runner's jobs and results, and the
# night's logs, for a worktree too.
MAIN = subprocess.run(["git", "-C", ROOT, "rev-parse", "--path-format=absolute", "--git-common-dir"], capture_output=True, text=True, check=True).stdout.strip()
MAIN = os.path.dirname(MAIN) if MAIN.endswith("/.git") else MAIN
# A clone of the Mac runners' exchange: its jobs/ and results/.
EXCHANGE = os.environ.get("MAC_RUNNER") or os.path.join(os.path.dirname(MAIN), "mac-runner")
# The timing Mac of CONTRIBUTING's step 3, by its label in the exchange.
# Other Macs run check jobs there too (an Apple M4 since November 21,
# 2026), and a job of theirs is not this step's evidence: at 0.10.0's
# publication the step read the M4's check of the same two commits, the
# last by its folder's name, and said FAIL where the M6's had passed.
TIMING_MAC = "mac-m6-timing"
# A results folder: the job's name, when a Mac claimed it, and which Mac.
RESULTS_FOLDER = re.compile(r"(?s)(.+)\.([0-9]{8}T[0-9]{6}Z)-(.+)")


# A step's two outcomes besides PASS, FAIL and OPEN.
TIMED_OUT, NO_VERDICT = "timed out", "no verdict"


def git(*args):
    return subprocess.run(["git", "-C", ROOT, *args], capture_output=True, text=True, check=True).stdout.strip()


class GitHubTimedOut(Exception):
    """GitHub's API did not answer within its 60 s: the step timed out, a
    different outcome from a failed or an open one."""


def github(path):
    token = open(os.path.join(MAIN, "ghtokenclassic.txt")).read().strip()
    request = urllib.request.Request("https://api.github.com" + path)
    # Not sent on to where a redirect leads.
    request.add_unredirected_header("Authorization", f"token {token}")
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            return json.load(response)
    except (TimeoutError, socket.timeout) as e:
        raise GitHubTimedOut(f"GitHub's API did not answer {path} within 60 s") from e
    except urllib.error.URLError as e:
        if isinstance(e.reason, (TimeoutError, socket.timeout)):
            raise GitHubTimedOut(f"GitHub's API did not answer {path} within 60 s") from e
        raise


def log_tail(path):
    full = os.path.join(MAIN, path)
    if not os.path.exists(full):
        return None
    return Path(full).read_text(errors="replace")[-4000:]


def log_ends_with(path, line):
    """Whether the log's last line is `line`: None where there is no log.
    A line above the last is an earlier run's, with whatever was written
    after it: a rerun that failed, appended to the same log, left the
    first run's line in the tail, and a search of the tail found it."""
    full = os.path.join(MAIN, path)
    if not os.path.exists(full):
        return None
    # Whole lines of the whole log (the last 4,000 characters could begin
    # inside a line, and "Note: " before the words then passed); empty
    # lines after the last are skipped, and the line is compared as it is
    # but for its end.
    lines = Path(full).read_text(errors="replace").split("\n")
    while lines and lines[-1].strip("\r") == "":
        lines.pop()
    return bool(lines) and lines[-1].rstrip("\r") == line


def mutation_crate(lines, commit):
    """One crate's whole run, from its commit identity through both summaries."""
    if not lines:
        return None
    if lines[0] != f"mutants: BLAKE3-Servil at {commit}":
        return False
    if "mutants: the whole run" not in lines:
        return False
    if len(lines) < 2:
        return None
    survivors, timeouts = lines[-2:]
    if survivors == "mutants: the list and this run differ, or a listed mutant has no reason":
        return False
    if not re.fullmatch(r"mutants: \d+ survive, each known and explained", survivors):
        return None
    if re.fullmatch(r"mutants: \d+ time out, each known \([^)]+\)", timeouts):
        return True
    if re.fullmatch(r"mutants: the time-outs differ from .+ \(no verdict on those mutants\)", timeouts):
        return NO_VERDICT
    return None


def mutation_result(path, commit):
    """Both crates must pass. The last crate's result cannot stand for the first."""
    full = Path(MAIN) / path
    if not full.exists():
        return None
    lines = full.read_text(errors="replace").splitlines()
    headers = ["mutants: the crate at the root", "mutants: the crate in threads/"]
    found = [(i, line) for i, line in enumerate(lines) if line.startswith("mutants: the crate ")]
    if [line for _, line in found] != headers:
        return None
    if found[0][0] != 0 or not re.fullmatch(r"mutants: 2 crates took \d+ s in all", lines[-1]):
        return None
    middle = found[1][0]
    results = [mutation_crate(lines[1:middle], commit), mutation_crate(lines[middle + 1:-1], commit)]
    # A failure stays visible beside an incomplete or undecided run.
    return next((r for r in (False, None, NO_VERDICT) if r in results), True)


def unpack_builds(archives, into):
    """Write into the folder `into` each b3sum program and crate of the
    zips `archives` ((artifact name, bytes) pairs, each one job's dist/),
    and name where the builders do not bear each other out. Both Linux
    builders package the crates, and each crate is to be the same bytes
    from both:
    - a crate two builders made with different bytes (the later copy once
      replaced the earlier unread, so a builder that disagreed went unseen
      whenever the other's crate was the local one's);
    - a crate one builder made and another builder of crates did not (the
      union of their files would still hold every crate, each seen once);
    - crates from one builder alone;
    - a zip that holds a name twice (reading by name reads one of them)."""
    made_by, crates, differ = {}, {}, []
    for artifact, archive in archives:
        with zipfile.ZipFile(io.BytesIO(archive)) as files:
            names = files.namelist()
            for name in sorted({n for n in names if names.count(n) > 1}):
                differ.append(f"{os.path.basename(name)} ({artifact} holds it {names.count(name)} times)")
            for name in names:
                base = os.path.basename(name)
                if not (base.startswith("b3sum-") or name.endswith(".crate")) or release_artifacts.is_digest(name):
                    continue
                if name.endswith(".crate"):
                    crates.setdefault(artifact, set()).add(base)
                data, path = files.read(name), os.path.join(into, base)
                if base in made_by:
                    if release_artifacts.read(path) != data:
                        differ.append(f"{base} ({made_by[base]} and {artifact})")
                    continue
                made_by[base] = artifact
                with open(path, "wb") as f:
                    f.write(data)
    every = set().union(*crates.values())
    for artifact in sorted(crates):
        for base in sorted(every - crates[artifact]):
            differ.append(f"{base} ({made_by[base]} made it and {artifact} did not)")
    if len(crates) == 1:
        differ.append(f"the crates ({next(iter(crates))} alone made them: no second builder bears them out)")
    return differ


def ci_builds(run, into):
    """Write into the folder `into` the b3sum programs and the crates
    that the CI run `run` made (its `b3sum release build` jobs' artifacts,
    each a zip of that job's dist/); return what its jobs disagree on
    (unpack_builds)."""
    token = open(os.path.join(MAIN, "ghtokenclassic.txt")).read().strip()

    def archives():
        for artifact in github(f"/repos/servil-software-factory/BLAKE3/actions/runs/{run['id']}/artifacts?per_page=100")["artifacts"]:
            if not artifact["name"].startswith("b3sum-"):
                continue
            # GitHub answers with a redirect to its storage, which refuses the
            # token: gh follows it without.
            yield artifact["name"], subprocess.run(["gh", "api", f"repos/servil-software-factory/BLAKE3/actions/artifacts/{artifact['id']}/zip"],
                                                   env={**os.environ, "GH_TOKEN": token}, capture_output=True, check=True, timeout=300).stdout

    return unpack_builds(archives(), into)


def version_at(commit):
    return re.search(r'(?m)^version = "([^"]+)"', git("show", f"{commit}:Cargo.toml")).group(1)


def artifacts_result(commit, run=None):
    """The release's artifacts in this checkout's dist/, by
    release_artifacts.directory: True where every one is there and a clean
    build of `commit`, False where one is not (or `commit` is no version
    commit), None where some are not there yet; and what to say of it.
    `run`: the commit's dispatched CI run, where it has completed. Each
    b3sum program and each crate must be, byte for byte, the one that run
    made: two builders, ours and GitHub's, and neither's word alone."""
    try:
        release = release_artifacts.Release("v" + version_at(commit), ROOT, commit)
    except release_artifacts.NotARelease as e:
        return False, f"{e} (python3 tools/gen-ver.py X.Y.Z makes a version commit)"
    dist = os.path.join(ROOT, "dist")
    with tempfile.TemporaryDirectory() as second:
        differ = (ci_builds(run, second) or []) if run is not None else []
        if differ:
            return False, "CI's own builders do not bear each other out on " + ", ".join(differ) + ": their files are no witness of the version commit's"
        ok, lines = release_artifacts.directory(dist, release, rebuilt=None if run is None else second)
    if ok is None:
        return None, (f"not in {dist} yet: {', '.join(lines)}. On each machine `sh tools/b3sum-release.sh` writes b3sum to its dist/, "
                      "and `sh tools/crates-release.sh` writes the crates' archives there. Gather them here")
    if ok and run is None:
        return None, "each is clean by its own word; CI's builds, which each b3sum program and crate must equal byte for byte, are not there yet (dispatch the tests workflow on the commit)"
    return ok, "; ".join(line.replace(second, "CI's run") for line in lines)


def ci_result(run):
    """Release CI tests the actual candidate, with every job executed."""
    if run["event"] != "workflow_dispatch":
        return None
    if run["status"] != "completed":
        return None
    if run["conclusion"] != "success":
        return TIMED_OUT if run["conclusion"] == "timed_out" else False
    jobs, page = [], 1
    while True:
        reply = github(f"/repos/servil-software-factory/BLAKE3/actions/runs/{run['id']}/jobs?per_page=100&page={page}")
        jobs.extend(reply["jobs"])
        if len(jobs) == reply["total_count"]:
            break
        assert reply["jobs"] and len(jobs) < reply["total_count"], "CI jobs response is incomplete or inconsistent"
        page += 1
    if not jobs:
        return None
    if any(job["conclusion"] == "timed_out" for job in jobs):
        return TIMED_OUT
    if any(job["status"] != "completed" or job["conclusion"] in (None, "skipped") for job in jobs):
        return None
    return all(job["conclusion"] == "success" for job in jobs)


def previous_release(commit):
    """Nearest reachable release tag, excluding tags on the candidate itself."""
    tags = git("tag", "--list", "v*", "--merged", commit).split()
    tags = [tag for tag in tags if git("rev-parse", tag + "^{commit}") != commit]
    if not tags:
        return None
    matches = [arg for tag in tags for arg in ("--match", tag)]
    return git("describe", "--tags", "--abbrev=0", *matches, commit)


def named_previous(commit, tag):
    """An explicit baseline must be a tag of a real, older stamp."""
    try:
        old = git("rev-parse", "--verify", f"refs/tags/{tag}^{{commit}}")
    except subprocess.CalledProcessError as e:
        raise release_artifacts.NotARelease(f"no local tag {tag}; fetch the published release tag") from e
    release = release_artifacts.Release(tag, ROOT, old)
    if release.version != version_at(old):
        raise release_artifacts.NotARelease(f"{tag} and its Cargo.toml version disagree")
    triplet = lambda version: tuple(int(n) for n in version.split("+", 1)[0].split("."))
    if old == commit or triplet(release.version) >= triplet(version_at(commit)):
        raise release_artifacts.NotARelease(f"{tag} is not older than the candidate")
    return tag


def plain(value, most=160):
    """`value` for a step's one line: a text as JSON writes it, without
    the quotes (no line break survives), anything else as Python does;
    `most` characters of it, and then three dots."""
    text = json.dumps(value)[1:-1] if isinstance(value, str) else repr(value)
    return text if len(text) <= most else text[:most] + "..."


def shown(value):
    """`plain`, in quotes where `value` is a text: null is not "null"."""
    return f'"{plain(value)}"' if isinstance(value, str) else plain(value)


# What the exchange publishes of a job's results (tools/runner/exchange.py):
# no record is larger.
RECORD_LIMIT = 20 << 20


def read_object(path):
    """The JSON object in the regular file at `path`, or a text that says
    why there is none to take (for a sentence that begins with the
    file). A link is not the file; a field written twice, or NaN, is not
    JSON's; and the exchange publishes no file above its limit."""

    def once(pairs):
        if len({key for key, _ in pairs}) != len(pairs):
            raise ValueError("a field twice")
        return dict(pairs)

    def no_constant(name):
        raise ValueError(name)

    try:
        found = os.lstat(path)
        if not stat.S_ISREG(found.st_mode):
            return "is not a regular file"
        if found.st_size > RECORD_LIMIT:
            return "is too large for a record"
        value = json.loads(Path(path).read_bytes().decode(), object_pairs_hook=once, parse_constant=no_constant)
    except OSError as e:
        return f"cannot be read ({e.strerror or e})"
    except (ValueError, RecursionError):  # not text, not JSON, a field twice, or nested past reading
        return "is not a JSON object with each field once"
    return value if isinstance(value, dict) else "is not a JSON object with each field once"


def utc(text, form):
    """The moment `text` names in the writers' own form, digit for digit
    (`form` is "%Y-%m-%dT%H:%M:%SZ" or the folders' "%Y%m%dT%H%M%SZ"), or
    None: strptime alone takes "T3:00:00Z" for three o'clock."""
    shape = re.sub("%[YmdHMS]", lambda m: r"\d{4}" if m.group() == "%Y" else r"\d{2}", form)
    if not isinstance(text, str) or not re.fullmatch(shape, text):
        return None
    try:
        return datetime.datetime.strptime(text, form)
    except ValueError:
        return None


def named(spelling, full):
    """Does a job's `spelling` name the commit `full`: "in full", "by its
    beginning" (the runner takes 7 to 40 lowercase hex digits), "another"
    (a well-formed name of another commit), or None for what is no name.
    Where `full` is None any commit will do, and the answer is how
    `spelling` names its own."""
    if not isinstance(spelling, str) or not re.fullmatch("[0-9a-f]{7,40}", spelling):
        return None
    if full is None:
        return "in full" if len(spelling) == 40 else "by its beginning"
    return "in full" if spelling == full else "by its beginning" if full.startswith(spelling) else "another"


def check_of(job, commit, old):
    """What a job file's content says of this step: None, it is not a
    check of every cell of these commits (another kind of job, a check
    of some cells, another commit's); "in full" or "by its beginning",
    it is one, and how it names them; or a text that begins "?", it
    cannot be told, with why. A job is another's only where it says so:
    `{}` says no more than `{` does, and the runner refuses a check whose
    "cells" is anything but "all" or that names two scopes. A commit that
    is well formed and another's settles it, whatever else the job says."""
    if isinstance(job, str):
        return "? its job file " + job
    kind = job.get("type")
    if not isinstance(kind, str) or not kind:
        return f"? its job's type is {shown(kind)}"
    if kind != "check":
        return None
    sides = [named(job.get("new_commit"), commit), named(job.get("old_commit"), old)]
    scope = [key for key in ("cells", "points", "probe") if key in job]
    if "another" in sides or "cells" not in scope:
        return None
    if scope != ["cells"] or job["cells"] != "all":
        return f"? its job's scope is {', '.join(f'{key} {shown(job[key])}' for key in scope)}, which the runner refuses"
    if None in sides:
        return f"? its job is a check of every cell whose commits are {shown(job.get('old_commit'))} and {shown(job.get('new_commit'))}"
    return "by its beginning" if "by its beginning" in sides else "in full"


WITHDRAW = "move its job and its folder out of jobs/ and results/, in a commit that says why"


def mac_check(exchange, commit, old):
    """The Mac step, from a clone of the exchange: (outcome, note), the
    outcome as `step` takes it and the note for its line.

    The step says PASS for one thing only: every record in the exchange
    that is the timing Mac's check of every cell of `commit` against
    `old` is whole, and the one the Mac started last says "no regression"
    of a run on AC power that nobody inspected. It asks what a good
    record looks like and holds the step (NO VERDICT) on the rest.

    What it guards against. The exchange is a git repository that the
    runners and their bridge write (tools/runner), and people add jobs
    to. So: a check queued again, running, or abandoned; a result on
    another Mac; a commit named by an abbreviation; jobs handed over
    together; a record cut short or missing a file; a job written wrong.
    It checks that a record agrees with itself and with its job. It is
    no guard against a record edited so that it still agrees with
    itself: whoever can write the exchange can write a pass, and a
    verdict says only what the runner put there.

    The records. A job is a file of jobs/ whose name ends ".json", and
    NAME is its name without that end (pathlib's stem, as the runner
    takes it). A results folder is NAME.TIME-MACHINE: TIME is when the
    job was handed to MACHINE. The job says whether a folder of the
    timing Mac's counts: a "check" whose only scope is "cells": "all"
    and whose new_commit and old_commit name `commit` and `old` (any
    commit for old where there is no previous release), in full or by a
    beginning. Another kind of job, a check of points or of a probe, and
    a check that names another commit do not count, and their results
    are never opened. A job that cannot be told (unreadable, without a
    type, a scope the runner refuses, a commit that is no name) is not
    taken for another commit's: its record is put in order with the
    others, and it holds the step where it is the last, or not whole.
    A job that no results folder names yet, as the runner sees it (no
    entry of results/ begins "NAME."), is queued: a check of these
    commits, or a job that cannot be told, holds the step until a Mac
    has taken it.

    Whole. A record that counts is a real folder with a claim
    (claimed.json: this job, this Mac, and when) and a verdict
    (verdict.json: this job, the type "check", a status, when it started
    and when it finished), each a regular file with one JSON object.
    The times are the writers' own, digit for digit, and in order: the
    hand-over (TIME), the claim, the start, the finish, with a minute's
    grace between the two clocks (the bridge's and the Mac's; no record
    so far needed any: each of the first three may follow the next by
    that much). A folder without a verdict is a check queued or
    running. One record that is not whole holds the step, however old:
    nothing says when it ran.

    Last. By the verdicts' "started": several jobs handed over together
    share a TIME, and a busy Mac takes pending jobs by name. Two that
    started in the same second cannot be put in order.

    What the last one must be to pass: its commits named in full (an
    abbreviation is resolved on the Mac, in a checkout this one cannot
    see; such a check can fail the step and cannot pass it); both power
    readings from the source "AC Power" (a check on battery measures the
    efficiency cores' clock, not the machine's: job 351; jobs 1696 and
    1697 passed so, #202; the readings are the two ends', which is all
    the runner records); no inspection (the runner sampled the stacks of
    the job's processes, tools/runner, which suspends them while they
    are timed), where "inspections" is absent or a list, and any other
    value passes nothing; and none of the fields the runner writes
    beside a failure ("error", "timed out", "cancelled"). Then "no
    regression" passes and any other status fails, apart from the
    runner's three that decide nothing: the status "timed out" (its
    limit ended the job: TIMED OUT, whatever else is true of the
    record), "no verdict" (the machine was busy) and "cancelled"
    (someone ended the job)."""

    def hold(why):
        return NO_VERDICT, f" ({why})"

    try:
        folders = sorted(os.scandir(os.path.join(exchange, "results")), key=lambda entry: entry.name)
        jobs = sorted(entry.name for entry in os.scandir(os.path.join(exchange, "jobs")) if entry.name.endswith(".json"))
    except OSError as e:
        return hold(f"the exchange cannot be listed: {e.strerror or e}")
    files = {}
    for name in jobs:
        files.setdefault(PurePath(name).stem, []).append(name)
    minute = datetime.timedelta(minutes=1)
    records = []
    for entry in folders:
        ran = RESULTS_FOLDER.fullmatch(entry.name)
        if not ran or ran.group(3) != TIMING_MAC:
            continue
        name, label = ran.group(1), shown(entry.name)
        if len(files.get(name, [])) > 1:
            return hold(f"{label}: two files of jobs/ have its name, {' and '.join(shown(file) for file in files[name])}")
        file = files[name][0] if name in files else name + ".json"
        job = read_object(os.path.join(exchange, "jobs", file))
        how = check_of(job, commit, old)
        if how is None:
            continue
        unknown = how.startswith("?")
        folder = os.path.join(exchange, "results", entry.name)
        if entry.is_symlink() or not entry.is_dir(follow_symlinks=False):
            why = "it is not a folder"
        else:
            claim, verdict = read_object(os.path.join(folder, "claimed.json")), read_object(os.path.join(folder, "verdict.json"))
            handed = utc(ran.group(2), "%Y%m%dT%H%M%SZ")
            claimed, started, finished = (utc(record.get(key), "%Y-%m-%dT%H:%M:%SZ") if isinstance(record, dict) else None
                                          for record, key in ((claim, "claimed"), (verdict, "started"), (verdict, "finished")))
            why = (
                "its folder's time is no date" if handed is None
                else f"its claimed.json {claim}" if isinstance(claim, str)
                else f"its claimed.json names {shown(claim.get('job'))} on {shown(claim.get('machine'))}" if claim.get("job") != file or claim.get("machine") != TIMING_MAC
                else "it has no verdict yet (queued or running: wait for it; if it was abandoned, " + WITHDRAW + ")" if not os.path.lexists(os.path.join(folder, "verdict.json"))
                else f"its verdict.json {verdict}" if isinstance(verdict, str)
                else f"its verdict names the job {shown(verdict.get('job'))}" if verdict.get("job") != file
                else f"its verdict is of the type {shown(verdict.get('type'))}, not of a check" if verdict.get("type") != "check" and not unknown
                else f"its status is {shown(verdict.get('status'))}, not a text" if not isinstance(verdict.get("status"), str) or not verdict["status"]
                else f"its times are not four in order: handed over {ran.group(2)}, claimed {shown(claim.get('claimed'))}, started {shown(verdict.get('started'))}, finished {shown(verdict.get('finished'))}"
                if None in (claimed, started, finished) or handed - claimed > minute or claimed - started > minute or finished < started
                else None
            )
        if why:
            return hold(f"{label} is no whole record, and nothing says when it ran: {why}" + ("" if "wait for it" in why else f"; repair it, or {WITHDRAW}, and run the check again")
                        + (f" (and {how[2:]})" if unknown else ""))
        records.append((started, label, how, verdict))
    taken = [entry.name for entry in folders]
    for name, names in files.items():
        if any(entry.startswith(name + ".") for entry in taken):
            continue
        for file in names:
            how = check_of(read_object(os.path.join(exchange, "jobs", file)), commit, old)
            if how is not None:
                return hold(f"jobs/{plain(file)} is queued and no Mac has taken it: " + (how[2:] if how.startswith("?") else "a check of these commits")
                            + f"; wait for it, or {WITHDRAW.replace(' and its folder', '').replace(' and results/', '')}")
    if not records:
        return None, ""
    records.sort(key=lambda record: record[0])
    started, label, how, verdict = records[-1]
    if len(records) > 1 and records[-2][0] == started:
        return hold(f"the last two started in the same second and cannot be put in order: {records[-2][1]} and {label}; run the check again")
    if how.startswith("?"):
        return hold(f"the last one, {label}, cannot be told from a check of these commits: {how[2:]}; run the check again, with a job the runner takes")
    status = verdict["status"]
    notes = []
    for reading in (verdict.get("power at start"), verdict.get("power at end")):
        if not isinstance(reading, str) or reading.split(",")[0].strip() != "AC Power":
            notes.append(f"it ran off AC power: {plain(reading) if isinstance(reading, str) and reading else 'unrecorded'}; run it again plugged in")
            break
    inspections = verdict.get("inspections", [])
    if not isinstance(inspections, list):
        notes.append(f"its \"inspections\" is {type(inspections).__name__}, not a list: a record this check does not understand; run it again")
    elif inspections:
        notes.append(f"it was inspected while it ran, {len(inspections)} time{'s' if len(inspections) != 1 else ''}: sampling suspends what it times; run it again")
    if status == "no regression":
        failed = [key for key in ("error", "timed out", "cancelled") if key in verdict]
        if failed:
            notes.append(f"it says no regression and also has the runner's {' and '.join(shown(key) for key in failed)}; run it again")
        elif not notes and how != "in full":
            notes.append("it names a commit by its beginning, which the Mac resolved in a checkout of its own: that cannot pass a release; queue the check with both commits in full")
        if not notes:
            return True, ""
    note = f" (the last one, {label}: {plain(status)})" + "".join(f" ({each})" for each in notes)
    if status == "timed out":
        return TIMED_OUT, note
    return (NO_VERDICT if notes or status in ("no verdict", "cancelled") else False), note


def main():
    parser = argparse.ArgumentParser(description="Check the evidence for a BLAKE3-Servil release.")
    parser.add_argument("commit", nargs="?", default="main")
    parser.add_argument("--previous", metavar="TAG", help="the published baseline tag, when it is not on the candidate's line")
    args = parser.parse_args()
    commit = git("rev-parse", args.commit)
    try:
        previous = named_previous(commit, args.previous) if args.previous else previous_release(commit)
    except release_artifacts.NotARelease as e:
        parser.error(str(e))
    old = git("rev-parse", previous + "^{commit}") if previous else None
    results = []
    built = None

    def step(name, ok, how):
        """ok: True (PASS), False (FAIL), None (OPEN), or TIMED_OUT or
        NO_VERDICT, each printed and counted as itself."""
        results.append(ok)
        label = {True: "PASS", False: "FAIL", None: "OPEN", TIMED_OUT: "TIMED OUT", NO_VERDICT: "NO VERDICT"}[ok]
        print(f"{label}  {name}" + ("" if ok is True else f": {how}"))

    try:
        runs = github(f"/repos/servil-software-factory/BLAKE3/actions/runs?head_sha={commit}&event=workflow_dispatch&per_page=100").get("workflow_runs", [])
        # The tests' workflow by its file, newest run first: another
        # workflow may bear the name.
        ci = sorted((r for r in runs if r.get("path") == ".github/workflows/ci.yml"), key=lambda r: r["run_number"], reverse=True)
        result = ci_result(ci[0]) if ci else None
        built = ci[0] if result is True else None
        step("CI, every job", result,
             "dispatch the tests workflow on the commit; every job must run and succeed" if result != TIMED_OUT else "a job reached its timeout-minutes")
    except GitHubTimedOut as e:
        step("CI, every job", TIMED_OUT, str(e))

    try:
        statuses = github(f"/repos/servil-software-factory/BLAKE3/commits/{commit}/statuses")
        proofs = [s for s in statuses if s["context"] == "long proofs"]
        step("long proofs", (proofs[0]["state"] == "success") if proofs else None, "sh tools/verify/long-proofs.sh <commit> (overnight)")
    except GitHubTimedOut as e:
        step("long proofs", TIMED_OUT, str(e))

    mac, mac_note = mac_check(EXCHANGE, commit, old)
    step("Mac check against the previous release", mac,
         f"in {EXCHANGE} (pulled), a check job (cells all) on {TIMING_MAC} with old_commit {old or '<the commit of the main line before this release>'} and new_commit {commit}, both in full, "
         f"and old_bench_commit when the old side predates the bench's calls" + mac_note)

    step("full mutation run, core and threads", mutation_result(f"tmp/night/mutants-{commit[:12]}.log", commit),
         f"python3 tools/mutants/check.py --whole > tmp/night/mutants-{commit[:12]}.log, the commit checked out (both crates; --whole, since the default covers the change since origin/next)")
    step("check-builds", log_ends_with(f"tmp/night/check-builds-{commit[:12]}.log", "check-builds: every build passes"),
         f"sh tools/check-builds.sh > tmp/night/check-builds-{commit[:12]}.log")
    step("cargo publish --dry-run", log_ends_with(f"tmp/night/publish-dry-run-{commit[:12]}.log", "publish-dry-run: ok"),
         f"the packages' dry-run in CONTRIBUTING.md; save its output to tmp/night/publish-dry-run-{commit[:12]}.log and append 'publish-dry-run: ok' only on success")

    try:
        step("release artifacts, each a clean build of this commit", *artifacts_result(commit, built))
    except GitHubTimedOut as e:
        step("release artifacts, each a clean build of this commit", TIMED_OUT, str(e))

    if previous:
        changed = set(git("diff", "--name-only", previous, commit).split())
        for page in ["README.md", "QUALITY.md", "CHANGELOG.md"]:
            step(f"{page} rewritten since {previous}", page in changed, "a release rewrites it for its readers")
    else:
        step("pages read fresh for the first release", None, "no previous release: read README.md, QUALITY.md and CHANGELOG.md as a newcomer would, and record it in the release issue")

    failing = sum(1 for r in results if r is False or r is None)
    undecided = sum(1 for r in results if r in (TIMED_OUT, NO_VERDICT))
    if not failing and not undecided:
        print("release-check: every step passes")
    else:
        print(f"release-check: {failing} steps open or failing, {undecided} timed out or without a verdict")
    sys.exit(0 if not failing and not undecided else 1 if failing else 2)


if __name__ == "__main__":
    main()
