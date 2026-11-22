#!/usr/bin/env python3
"""Move `next` to a commit only when every gate is green on that commit.

    python3 tools/land.py status COMMIT     the gates COMMIT needs, and which it has
    python3 tools/land.py gate COMMIT       run this machine's gates on COMMIT and post them
    python3 tools/land.py record GATE COMMIT success|failure DESCRIPTION
                                            post a gate that ran elsewhere, naming its log
    python3 tools/land.py land COMMIT       fast-forward next to COMMIT, if every gate is green

A gate is a GitHub commit status on the exact commit, so a rebase or an
amend starts again from none, and anyone can read what a landing stood
on. `land` posts the status `landable` and pushes; branch protection on
`next` requires `landable`, so no other push moves it. (November 15,
2026: two commits reached next by a hand push without check-builds, and
next stayed red until someone noticed.)

The gates of a commit, from what it changes against next:
  nothing but *.md files           none (the fast-forward alone)
  anything else                    the jobs of the tests' workflow on the
                                   commit (ci.yml), all green; check-builds;
                                   mutants on x86_64, aarch64 and arm64;
                                   tests on aarch64 and arm64; cells
  shipped code (src/, c/, build.rs, threads/src/, threads/build.rs,
                b3sum/src/, capi/src/, capi/b3.h, version/src/,
                the crates' Cargo.toml)
                                   review as well
  proof inputs (c/, c2sp/, tools/verify/, the generators)
                                   long proofs as well

`gate` runs check-builds and the x86-64 mutants here, in a worktree of
their own at COMMIT. The others run where they run (the Arm VM, the Mac,
the fleet's bench-hashes check, tools/verify/long-proofs.sh) and are
posted with `record`, whose description says where the log is. A gate
that does not apply is recorded as success with a description that
begins "waived: " and gives the reason; the waiver is then on the commit
for every reader, which a skipped step never was.

`review` is one review of a change to shipped code by an AI model of
another family than its author's (CONTRIBUTING, "Branches and changes";
tools/review/BRIEF.md is the brief). It holds a landing only for a
wrong result, a crash or panic, a hang, or a slowdown; everything else
lands and becomes an issue. Its description names the reviewer and says
what it found and what was done about it; its report is public.
(November 22, 2026: Zooko's trial of Rule 2, after a single review of
shipped code caught the running-digest slowdown of #472 while
multi-round reviews of tools and lists slowed down landings.)
"""

import json
import os
import subprocess
import sys

REPO = "servil-software-factory/BLAKE3"
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LOCAL = ("check-builds", "mutants x86_64")
ELSEWHERE = ("mutants aarch64", "mutants arm64", "tests aarch64", "tests arm64", "cells")
SHIPPED_CODE = (
    "src/", "c/", "build.rs", "Cargo.toml",
    "threads/src/", "threads/build.rs", "threads/Cargo.toml",
    "b3sum/src/", "b3sum/Cargo.toml",
    "capi/src/", "capi/b3.h", "capi/Cargo.toml",
    "version/src/", "version/Cargo.toml",
)
PROOF_INPUTS = ("c/", "c2sp/", "tools/verify/", "tools/gen_neon_hybrid.py", "tools/gen_sme2_hybrid.py", "tools/gen_x86.py", "tools/gen_x86_chunk_root.py",
                "tools/gen_avx512_32.py", "tools/gen_output8.py", "tools/gen_output16.py")


def git(*args):
    return subprocess.run(["git", "-C", ROOT, *args], check=True, capture_output=True, text=True).stdout.strip()


def api(path, *fields, method="GET"):
    out = subprocess.run(["gh", "api", "-X", method, f"repos/{REPO}/{path}", *fields], capture_output=True, text=True)
    if out.returncode != 0:
        sys.exit(f"land: GitHub refused {method} {path}: {out.stderr.strip()}")
    return json.loads(out.stdout) if out.stdout.strip() else None


def is_shipped(path):
    return any(path.startswith(p) if p.endswith("/") else path == p for p in SHIPPED_CODE)


def required(paths):
    """The gates a change to `paths` needs (see the module's text)."""
    if all(p.endswith(".md") for p in paths):
        return ()
    gates = ("CI",) + LOCAL + ELSEWHERE
    if any(is_shipped(p) for p in paths):
        gates += ("review",)
    if any(p.startswith(PROOF_INPUTS) for p in paths):
        gates += ("long proofs",)
    return gates


# The tests' workflow: its jobs are the gate "CI".
TESTS = ".github/workflows/ci.yml"


def pages(path, key=None, api=None):
    """Every item of a GitHub list, page after page (`key`: the list's
    name where a page is an object that holds it). One page of a hundred
    was read, and the rest taken for absent."""
    api = api or globals()["api"]
    items, page = [], 1
    while True:
        got = api(f"{path}{'&' if '?' in path else '?'}per_page=100&page={page}")
        batch = got[key] if key else got
        items += batch
        if len(batch) < 100:
            # A list that says how many it holds is held to it: a page
            # that changed while the pages were read is no answer.
            if key and isinstance(got, dict) and got.get("total_count", len(items)) != len(items):
                sys.exit(f"land: GitHub's list {path} says {got['total_count']} and gave {len(items)}: ask again")
            return items
        page += 1


def tests_jobs(sha, pages=pages):
    """The jobs of the tests' workflow on this very commit: those of its
    newest run there, at that run's last attempt; none where it has not
    run. Any job on the commit was taken for a test, so a commit on
    which another workflow alone had run, and passed, read as tested. A
    run for a pull request is left out: it tests the merge, another
    tree."""
    runs = [r for r in pages(f"actions/runs?head_sha={sha}", "workflow_runs") if r["path"] == TESTS and r["event"] != "pull_request" and r["head_sha"] == sha]
    if not runs:
        return []
    newest = max(runs, key=lambda run: run["run_number"])
    jobs = pages(f"actions/runs/{newest['id']}/jobs?filter=latest", "jobs")
    # The run itself must have ended in success: its jobs so far may all
    # be green while it still runs (a retry, jobs not yet listed), and a
    # run that failed may show green jobs only. What the run says counts
    # as one job more where it is not a plain success.
    if newest.get("status") != "completed" or newest.get("conclusion") != "success":
        jobs = jobs + [{"name": "(the run)", "conclusion": newest.get("conclusion") if newest.get("status") == "completed" else None}]
    return jobs


def verdicts(statuses, jobs):
    """Each gate's state from a commit's statuses (newest first, as GitHub
    lists them: the first of a context is the one that stands) and the
    tests' jobs on it ([`tests_jobs`]): CI is green when there is a job
    and every job succeeded."""
    seen = {}
    for s in statuses:
        # "CI" is never taken from a status: anything may post one of
        # that name, and it would stand where the tests never ran.
        if s["context"] != "CI":
            seen.setdefault(s["context"], (s["state"], s.get("description") or ""))
    runs = [r["conclusion"] for r in jobs]
    if runs:
        green = all(c == "success" for c in runs)
        seen["CI"] = ("success" if green else "pending" if any(c is None for c in runs) else "failure", f"{runs.count('success')} of {len(runs)} jobs green")
    return seen


def landable(base, gates):
    """The `landable` status's text, within GitHub's 140 characters: the
    gates by name while they fit (a proof input's eight did not, and the
    landing stopped on its last step), else their count."""
    text = f"every gate green on top of {base[:12]}: " + (", ".join(gates) or "*.md only")
    return text if len(text) <= 140 else f"every gate green on top of {base[:12]}: {len(gates)} gates"


def unfit(subject):
    """Why a commit's subject cannot land, or None: next is never
    rewritten, so a working title stays in its history for good (one
    did: 6a9c999f's subject is "x", an amend's stray -m)."""
    words = subject.split()
    if len(subject) < 20 or len(words) < 3:
        return "too short to say what the commit does"
    if words[0].rstrip(":").lower() in ("wip", "fixup!", "squash!", "probe", "lab", "trial", "tmp", "x"):
        return f"a working title ({words[0]})"
    if subject.rstrip().lower().endswith("(draft)"):
        return "marked a draft"
    return None


def resolve(commit):
    git("fetch", "-q", "origin", "next")
    sha, base = git("rev-parse", commit + "^{commit}"), git("rev-parse", "origin/next")
    if git("rev-list", "--count", f"{base}..{sha}") != "1" or git("merge-base", base, sha) != base:
        sys.exit(f"land: {sha[:12]} is not one commit on top of next ({base[:12]}): rebase and squash first")
    subject = git("log", "-1", "--format=%s", sha)
    if unfit(subject):
        sys.exit(f"land: {sha[:12]}'s subject, {subject!r}, is {unfit(subject)}: reword it (its gates start again)")
    return sha, base, git("diff", "--name-only", base, sha).split("\n")


def state(sha, paths):
    seen = verdicts(pages(f"commits/{sha}/statuses"), tests_jobs(sha))
    return [(g, *seen.get(g, ("missing", ""))) for g in required(paths)]


def post(sha, gate, verdict, description):
    assert verdict in ("success", "failure", "pending") and len(description) <= 140, "a state GitHub knows, and 140 characters at most"
    api(f"statuses/{sha}", "-f", f"state={verdict}", "-f", f"context={gate}", "-f", f"description={description}", method="POST")


def run_local(sha):
    tree = os.path.join(ROOT, "tmp", "land", sha)
    os.makedirs(os.path.dirname(tree), exist_ok=True)
    subprocess.run(["git", "-C", ROOT, "worktree", "remove", "--force", tree], capture_output=True)
    git("worktree", "add", "-q", "--detach", tree, sha)
    env = dict(os.environ, CARGO_TARGET_DIR=os.path.join(ROOT, "tmp", "land", "target"))
    steps = {
        "check-builds": [["sh", "tools/check-builds.sh"]],
        "mutants x86_64": [["python3", "tools/mutants/check.py", "--since", "origin/next", "--crate", c, "--every-mutant"] for c in (".", "threads")],
    }
    for gate, commands in steps.items():
        log = os.path.join(ROOT, "tmp", "land", f"{sha}-{gate.replace(' ', '-')}.log")
        with open(log, "w") as out:
            ok = all(subprocess.run(c, cwd=tree, env=env, stdout=out, stderr=subprocess.STDOUT).returncode == 0 for c in commands)
        post(sha, gate, "success" if ok else "failure", f"{'passed' if ok else 'FAILED'} on {os.uname().nodename}; log tmp/land/{os.path.basename(log)}")
        print(f"land: {gate}: {'passed' if ok else 'FAILED'} ({log})")
    subprocess.run(["git", "-C", ROOT, "worktree", "remove", "--force", tree], capture_output=True)


def main(argv):
    if len(argv) < 2 or argv[0] not in ("status", "gate", "record", "land"):
        sys.exit(__doc__)
    if argv[0] == "record":
        if len(argv) != 5 or argv[1] not in ELSEWHERE + LOCAL + ("long proofs",):
            sys.exit("land: record GATE COMMIT success|failure DESCRIPTION, GATE one of: " + ", ".join(ELSEWHERE + LOCAL + ("long proofs",)))
        post(git("rev-parse", argv[2] + "^{commit}"), argv[1], argv[3], argv[4])
        return
    sha, base, paths = resolve(argv[1])
    if argv[0] == "gate":
        if required(paths):
            run_local(sha)
    gates = state(sha, paths)
    for gate, verdict, description in gates:
        print(f"  {verdict:8s} {gate}: {description}")
    missing = [g for g, v, _ in gates if v != "success"]
    if argv[0] != "land":
        print(f"land: {sha[:12]} " + ("has every gate" if not missing else "lacks: " + ", ".join(missing)))
        return
    if missing:
        sys.exit(f"land: {sha[:12]} does not land: " + ", ".join(missing))
    post(sha, "landable", "success", landable(base, [g for g, _, _ in gates]))
    subprocess.run(["git", "-C", ROOT, "push", "origin", f"{sha}:refs/heads/next"], check=True)
    print(f"land: next is {sha}")


if __name__ == "__main__":
    main(sys.argv[1:])
