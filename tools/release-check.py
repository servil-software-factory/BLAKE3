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
- Mac check: results/*/verdict.json, in a clone of the Mac runners'
  exchange (servil-software-factory/mac-runner; $MAC_RUNNER, or
  mac-runner beside the main checkout; pull it first), whose job names
  this commit as new_commit and the previous release as old_commit (a
  "check" job with "cells": "all"), run on AC power from start to end (its verdict's "power
  at start" and "power at end"); on battery it is NO VERDICT.
- The logs, each named for the commit it ran on (C: the commit's first 12
  hex digits), so an earlier commit's log cannot stand in for it:
  - VM check: tmp/night/check-release-vm-C.log, from
    `cd ../bench-hashes && cargo run --release -- check --fork <this checkout> --against <previous tag> --cells all`,
    on one compatible bench where possible, with `--old-bench` only when
    the API change prevents that. Keep both output streams. Finish the
    log with `release-vm: ok` only on exit 0, so a first-stage reading
    cannot pass the gate while the bench repeats its suspects.
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


# A step's two outcomes besides PASS, FAIL and OPEN.
TIMED_OUT, NO_VERDICT = "timed out", "no verdict"
VM_OK = "release-vm: ok"


def git(*args):
    return subprocess.run(["git", "-C", ROOT, *args], capture_output=True, text=True, check=True).stdout.strip()


class GitHubTimedOut(Exception):
    """GitHub's API did not answer within its 60 s: the step timed out, a
    different outcome from a failed or an open one."""


def github(path):
    token = open(os.path.join(MAIN, "ghtokenclassic.txt")).read().strip()
    request = urllib.request.Request("https://api.github.com" + path, headers={"Authorization": f"token {token}"})
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


def log_ends_with(path, text):
    tail = log_tail(path)
    return None if tail is None else text in tail


def vm_result(path, commit, old):
    """Require the bench's own comparison line, and a full check's verdict."""
    try:
        lines = (Path(MAIN) / path).read_text(errors="replace").splitlines()
    except FileNotFoundError:
        return None
    title = re.compile(r"check: BLAKE3-Servil's .+ \(([0-9a-f]{12})\) against .+ \(([0-9a-f]{12})\)")
    comparisons = [m.groups() for line in lines if (m := title.fullmatch(line))]
    if len(comparisons) != 1:
        return False
    candidate, baseline = comparisons[0]
    if candidate != commit[:12] or baseline == candidate or (old is not None and baseline != old[:12]):
        return False
    if any(line.startswith("check: measuring the ") for line in lines):
        return False
    return lines[-1:] == [VM_OK] and "regress: no cell slower" in lines


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


def ci_builds(run, into):
    """Write into the folder `into` the b3sum programs and the crates
    that the CI run `run` made (its `b3sum release build` jobs' artifacts,
    each a zip of that job's dist/)."""
    token = open(os.path.join(MAIN, "ghtokenclassic.txt")).read().strip()
    for artifact in github(f"/repos/servil-software-factory/BLAKE3/actions/runs/{run['id']}/artifacts?per_page=100")["artifacts"]:
        if not artifact["name"].startswith("b3sum-"):
            continue
        # GitHub answers with a redirect to its storage, which refuses the
        # token: gh follows it without.
        archive = subprocess.run(["gh", "api", f"repos/servil-software-factory/BLAKE3/actions/artifacts/{artifact['id']}/zip"],
                                 env={**os.environ, "GH_TOKEN": token}, capture_output=True, check=True, timeout=300).stdout
        with zipfile.ZipFile(io.BytesIO(archive)) as files:
            for name in files.namelist():
                if os.path.basename(name).startswith("b3sum-") or name.endswith(".crate"):
                    if release_artifacts.is_digest(name):
                        continue
                    with open(os.path.join(into, os.path.basename(name)), "wb") as f:
                        f.write(files.read(name))


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
        if run is not None:
            ci_builds(run, second)
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
        runs = github(f"/repos/servil-software-factory/BLAKE3/actions/runs?head_sha={commit}&event=workflow_dispatch&per_page=20").get("workflow_runs", [])
        ci = [r for r in runs if r["name"] == "tests"]
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

    mac, mac_power = None, None
    for verdict in sorted(glob.glob(os.path.join(EXCHANGE, "results/*/verdict.json"))):
        v = json.loads(Path(verdict).read_text())
        job = os.path.join(EXCHANGE, "jobs", v["job"])
        if not os.path.exists(job):
            continue
        j = json.loads(Path(job).read_text())
        if (j.get("type") == "check" and j.get("cells") == "all"
                and j.get("new_commit") == commit
                and (old is None or j.get("old_commit") == old)):
            mac = v["status"]
            # A check on battery measures the efficiency cores' clock, not
            # the machine's (job 351; jobs 1696 and 1697 passed so, #202):
            # it is no evidence. Both readings must say AC power.
            readings = [v.get("power at start", ""), v.get("power at end", "")]
            mac_power = next((r or "unrecorded" for r in readings if not r.startswith("AC Power")), None)
    # The runner's verdict, each status as itself: "timed out" (its limit
    # ended the job) and "no verdict" (the machine was busy) are neither a
    # pass nor a regression.
    step("Mac check against the previous release",
         None if mac is None else TIMED_OUT if mac == "timed out" else NO_VERDICT if mac == "no verdict" or mac_power else mac == "no regression",
         f"in {EXCHANGE} (pulled), a check job (cells all) with old_commit {previous or '<the main line before this release>'} and new_commit {commit[:10]}, and old_bench_commit when the old side predates the bench's calls"
         + (f" (the last one: {mac})" if mac in ("timed out", "no verdict") else "")
         + (f" (the last one ran off AC power: {mac_power}; run it again plugged in)" if mac_power else ""))

    vm_log = f"tmp/night/check-release-vm-{commit[:12]}.log"
    step("VM check against the previous release", vm_result(vm_log, commit, old),
         f"(cd ../bench-hashes && cargo run --locked --release -- check --fork {ROOT} --against {previous or '<the main line before this release>'} --cells all) > {vm_log} 2>&1"
         f" && printf '%s\\n' '{VM_OK}' >> {vm_log}")
    step("full mutation run, core and threads", mutation_result(f"tmp/night/mutants-{commit[:12]}.log", commit),
         f"python3 tools/mutants/check.py --whole > tmp/night/mutants-{commit[:12]}.log, the commit checked out (both crates; --whole, since the default covers the change since origin/next)")
    step("check-builds", log_ends_with(f"tmp/night/check-builds-{commit[:12]}.log", "every build passes"),
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
