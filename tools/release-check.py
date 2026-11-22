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
- Mac check: the timing Mac's latest check of this commit against the
  previous release, in a clone of the Mac runners' exchange
  (servil-software-factory/mac-runner; $MAC_RUNNER, or mac-runner beside
  the main checkout; pull it first). `mac_check` has the rules. In
  short: a results folder is NAME.TIME-MACHINE, its job is
  jobs/NAME.json, and the folders that count are the timing Mac's whose
  job is a "check" with "cells": "all" naming this commit as new_commit
  and the previous release as old_commit. The latest of them decides,
  and only if it is whole: run on AC power from start to end, not
  cancelled, not inspected (the runner sampled its processes' stacks,
  which suspends what it times). Anything else about the latest one is
  NO VERDICT, with the reason: a record that cannot be read or
  contradicts itself, one still running, two that cannot be put in
  order. An older pass never stands in for a later check.
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
import glob
import io
import json
import socket
import urllib.error
from pathlib import Path
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
RESULTS_FOLDER = re.compile(r"(.+)\.([0-9]{8}T[0-9]{6}Z)-([A-Za-z0-9_.-]+)")


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


def read_object(path):
    """The JSON object in the file at `path`, or a text that says why
    there is none to read (for a sentence that begins with the file)."""
    try:
        value = json.loads(Path(path).read_text())
    except OSError as e:
        return f"cannot be read ({e.strerror or e})"
    except ValueError:  # not JSON, or not text
        return "is not JSON"
    return value if isinstance(value, dict) else "is not a JSON object"


def names_commit(spelling, full):
    """Does a job's `spelling` name the commit `full`? True or False; or
    None where it is a beginning of the commit's name that does not
    resolve to it in this checkout (another object begins so too, and
    what the Mac built is then not known). The runner takes a commit as
    7 to 40 lowercase hex digits: a check that names this commit by
    twelve digits is a check of this commit."""
    if not isinstance(spelling, str) or not re.fullmatch("[0-9a-f]{7,40}", spelling) or not full.startswith(spelling):
        return False
    if spelling == full:
        return True
    try:
        return True if git("rev-parse", "--verify", "--quiet", spelling + "^{commit}") == full else None
    except subprocess.CalledProcessError:
        return None


def mac_check(exchange, commit, old):
    """The Mac step, from a clone of the exchange: (outcome, note), the
    outcome as `step` takes it and the note for its line.

    Which records count. A results folder is NAME.TIME-MACHINE: TIME is
    when the job was handed to MACHINE, and jobs/NAME.json is its job.
    The folders that count are the timing Mac's whose job is a check of
    every cell with `commit` as new_commit and `old` as old_commit (any
    old_commit where there is no previous release). The job is read
    first, by the folder's name, so another job's result, however
    damaged, is never opened; and a verdict that names another job than
    its folder's is not that job's verdict.

    Which one decides: the latest, by TIME and then by the verdict's own
    "started" (several jobs handed over together share a TIME, and the
    Mac runs them one after another). A folder without a verdict yet is
    later than those with one: its check is queued or running, and the
    answer is to wait for it. Never an earlier record in place of the
    latest: where the latest cannot be used, or two cannot be put in
    order, or a later folder's job cannot be read (so that nobody can
    say it is not a check of these commits), the step has no verdict
    and says which folder is why.

    What the one that decides must be: its status a text; both power
    readings "AC Power" (a check on battery measures the efficiency
    cores' clock, not the machine's: job 351; jobs 1696 and 1697 passed
    so, #202); no inspection (the runner sampled the stacks of the job's
    processes, tools/runner, which suspends them while they are timed).
    "inspections" is absent or a list; any other value is a record this
    reader does not understand, and passes nothing. Then "no regression"
    passes and any other status fails, apart from the runner's three
    that decide nothing: "timed out" (its limit ended the job; TIMED
    OUT, whatever else is true of it), "no verdict" (the machine was
    busy) and "cancelled" (someone ended the job)."""
    records, unreadable = [], []
    for folder in sorted(glob.glob(os.path.join(exchange, "results", "*"))):
        ran = RESULTS_FOLDER.fullmatch(os.path.basename(folder))
        if not ran or ran.group(3) != TIMING_MAC:
            continue
        name, time, label = ran.group(1), ran.group(2), os.path.basename(folder)
        job = read_object(os.path.join(exchange, "jobs", name + ".json"))
        if isinstance(job, str):
            unreadable.append((time, f"{label}: its job, jobs/{name}.json, {job}"))
            continue
        if job.get("type") != "check" or job.get("cells") != "all":
            continue
        named = [names_commit(job.get("new_commit"), commit)] + ([names_commit(job.get("old_commit"), old)] if old else [])
        if False in named:
            continue
        unusable = "its job names a commit by a beginning that other objects here share" if None in named else None
        verdict = None
        if os.path.exists(os.path.join(folder, "verdict.json")):
            verdict = read_object(os.path.join(folder, "verdict.json"))
            if isinstance(verdict, str):
                verdict, unusable = None, unusable or f"its verdict.json {verdict}"
        elif unusable is None:
            unusable = "it has no verdict yet (queued or running: wait for it, or queue the check again if it was abandoned)"
        if verdict is not None and unusable is None:
            claim = read_object(os.path.join(folder, "claimed.json")) if os.path.exists(os.path.join(folder, "claimed.json")) else {}
            if verdict.get("job") != name + ".json":
                unusable = f"its verdict names the job {verdict.get('job')!r}, not {name}.json"
            elif isinstance(claim, str):
                unusable = f"its claimed.json {claim}"
            elif claim.get("job", name + ".json") != name + ".json" or claim.get("machine", TIMING_MAC) != TIMING_MAC:
                unusable = f"its claimed.json names {claim.get('job')!r} on {claim.get('machine')!r}"
            elif not isinstance(verdict.get("status"), str) or not verdict["status"]:
                unusable = f"its status is {verdict.get('status')!r}, not a text"
        started = verdict.get("started") if verdict is not None and isinstance(verdict.get("started"), str) else ""
        # In order: by TIME; a folder without a usable verdict after the
        # others of its TIME; then by when the Mac started the job.
        records.append(((time, unusable is not None, started), label, unusable, verdict))
    if not records:
        return None, ""
    records.sort(key=lambda record: record[0])
    key, label, unusable, verdict = records[-1]
    later = [why for time, why in unreadable if time >= key[0]]
    if later:
        return NO_VERDICT, f" (a folder as late as the last check cannot be told from one: {later[-1]})"
    if unusable:
        return NO_VERDICT, f" (the last one, {label}, decides nothing: {unusable})"
    if len(records) > 1 and records[-2][0] == key:
        return NO_VERDICT, f" (the last two cannot be put in order: {records[-2][1]} and {label} were handed over together and neither verdict says when it started; run the check again)"
    status = verdict["status"]
    note = f" (the last one: {status})" if status != "no regression" else ""
    undecided = status in ("no verdict", "cancelled")
    for reading in (verdict.get("power at start"), verdict.get("power at end")):
        if not isinstance(reading, str) or reading.split(",")[0].strip() != "AC Power":
            note += f" (the last one ran off AC power: {reading if isinstance(reading, str) and reading else 'unrecorded'}; run it again plugged in)"
            undecided = True
            break
    inspections = verdict.get("inspections", [])
    if not isinstance(inspections, list):
        note += f" (the last one's \"inspections\" is {type(inspections).__name__}, not a list: a record this check does not understand; run it again)"
        undecided = True
    elif inspections:
        note += f" (the last one was inspected while it ran, {len(inspections)} time{'s' if len(inspections) != 1 else ''}: sampling suspends what it times; run it again)"
        undecided = True
    if status == "timed out":
        return TIMED_OUT, note
    return (NO_VERDICT if undecided else status == "no regression"), note


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
         f"in {EXCHANGE} (pulled), a check job (cells all) on {TIMING_MAC} with old_commit {previous or '<the main line before this release>'} and new_commit {commit[:10]}, and old_bench_commit when the old side predates the bench's calls" + mac_note)

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
