#!/usr/bin/env python3
"""Run the shared mixed-stage sampler/judge through the Mac runner.

Run from a fresh directory: python3 mac_cycles.py main aa (or next/ab).
examples/host_lab.rs runs aa before ab for the selected family.
The two Rust bodies are extracted unchanged from mixed-stage-costs.sh,
from samshackleton-tech/bench-hashes 0b2e9892f6cb30e38ddce5ba82ad7b8d85f060d0.
The shared sampler owns clocks, load validation, API contracts and diagnostics.
This adapter owns pinned builds, fresh paired layouts and rejecting controls.
PINNED_RUN_PREFLIGHT=1 compiles and checks digests without timing or waiting.
In the VM, set CARGO_TARGET_DIR to private disk-backed executable storage.
The Mac runner owns exclusivity, power checks and the process deadline.
"""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

PAIRS = {
    "main": ("6e0dffd37e04b01b80dc56eb833b00fc337b6873", "7b1ba7b9da67e61dd53010de2054edfa32035170"),
    "next": ("a930b4c1a7ac94a77b6decd4a5bfd710391286b6", "6eefdc657effe6002f9a001fccac9306699b3911"),
}
BENCH = "9994de190259648ede82f7c5e4dbac4f1534a41d"
INSTRUMENT = "0b2e9892f6cb30e38ddce5ba82ad7b8d85f060d0"
LIBRARY = Path(__file__).resolve().parents[2]
ROOT = Path.cwd()
OUT = ROOT / "out"
TARGET = (Path(os.environ["CARGO_TARGET_DIR"]) / ROOT.name
          if "CARGO_TARGET_DIR" in os.environ else ROOT / "targets")


def run(args, *, cwd=ROOT, env=None, log=None, check=True):
    print("+", " ".join(map(str, args)), flush=True)
    if log is None:
        return subprocess.run(list(map(str, args)), cwd=cwd, env=env, check=check)
    with log.open("w") as stream:
        return subprocess.run(list(map(str, args)), cwd=cwd, env=env, stdout=stream, stderr=subprocess.STDOUT, check=check)


def git(directory, *args):
    return subprocess.check_output(["git", "-C", str(directory), *args], text=True).strip()


def extract(source, marker):
    matches = re.findall(r"<<'" + marker + r"'\n(.*?)\n" + marker + r"\n", source, re.S)
    assert len(matches) == 1, marker
    return matches[0] + "\n"


def build_project(name, code, manifest, pair, family):
    project = ROOT / ("project-" + name)
    (project / "src").mkdir(parents=True)
    (project / "src/main.rs").write_text(code)
    (project / "Cargo.toml").write_text(manifest)
    target = TARGET / ("target-" + name)
    env = dict(os.environ, CARGO_TARGET_DIR=str(target), RUSTFLAGS=f"-C metadata=mixed_stage_pair_{pair}")
    run(["cargo", "build", "--release", "--features", family + "-api"], cwd=project, env=env, log=OUT / ("build-" + name + ".log"))
    exe = TARGET / ("exe-" + name)
    shutil.copy2(target / "release/incrementalroots", exe)
    shutil.copy2(project / "Cargo.lock", OUT / ("lock-" + name + ".txt"))
    return exe, project, env


def execute(exe, name, family, preflight=False, reverse=False, expected_failure=False):
    env = dict(os.environ)
    env.pop("PINNED_RUN_PREFLIGHT", None)
    if preflight:
        env["PINNED_RUN_PREFLIGHT"] = "1"
    with (OUT / (name + ".tsv")).open("w") as out, (OUT / (name + ".err")).open("w") as err:
        p = subprocess.run([str(exe), "reverse" if reverse else "forward"], env=env, stdout=out, stderr=err)
    if expected_failure:
        assert p.returncode != 0, "the planted wrong digest must fail the reference check"
        assert "assertion `left == right` failed" in (OUT / (name + ".err")).read_text()
        return
    assert p.returncode == 0, f"{name}: exit {p.returncode}; inspect its retained stderr (2 means busy, no speed verdict)"
    if preflight:
        assert len((OUT / (name + ".tsv")).read_text().splitlines()) == 1, "preflight emits no speed rows"
        text = (OUT / (name + ".err")).read_text()
        assert f"finalization_contract={family}:" in text
        counts = [line for line in text.splitlines() if line.startswith("core_counts ")]
        assert len(counts) == 192, "64 cases, three phases"
        assert all("p_time_ns=NA" in line and "e_time_ns=NA" in line for line in counts), "preflight reports counts as NA"


def main():
    assert len(sys.argv) == 3 and sys.argv[1] in PAIRS and sys.argv[2] in ("aa", "ab"), __doc__
    family, kind = sys.argv[1:]
    old, new = PAIRS[family]
    assert not OUT.exists(), "a study gets a fresh result directory"
    OUT.mkdir()
    TARGET.mkdir(parents=True, exist_ok=True)
    source = (Path(__file__).parent / "mixed-stage-costs.sh").read_text()
    probe = extract(source, "PROBE_EOF")
    judge = extract(source, "JUDGE_EOF")
    (OUT / "sampler.rs").write_text(probe)
    (OUT / "judge.rs").write_text(judge)
    revisions = {"old": old, "new": old if kind == "aa" else new}
    for rev in dict.fromkeys(revisions.values()):
        run(["git", "fetch", "https://github.com/servil-software-factory/BLAKE3", rev], cwd=LIBRARY)
    for side, rev in revisions.items():
        run(["git", "worktree", "add", "--detach", ROOT / side, rev], cwd=LIBRARY)
        assert git(ROOT / side, "rev-parse", "HEAD") == rev
    run(["git", "clone", "--no-checkout", "https://github.com/servil-software-factory/bench-hashes", ROOT / "bench"])
    run(["git", "fetch", "https://github.com/servil-software-factory/bench-hashes", BENCH], cwd=ROOT / "bench")
    run(["git", "checkout", "--detach", BENCH], cwd=ROOT / "bench")
    identity = {"adapter": git(LIBRARY, "rev-parse", "HEAD"), "adapter_working_tree": git(LIBRARY, "status", "--porcelain", "--untracked-files=all"), "instrument": INSTRUMENT, "instrument_git_blob": git(LIBRARY, "hash-object", str(Path(__file__).parent / "mixed-stage-costs.sh")), "bench": BENCH, "libraries": revisions, "family": family, "kind": kind, "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip()}
    (OUT / "identity.json").write_text(json.dumps(identity, indent=2) + "\n")
    shutil.copy2(__file__, OUT / "adapter.py")
    shutil.copy2(Path(__file__).parent / "mixed-stage-costs.sh", OUT / "instrument.sh")
    project = ROOT / "judge"
    (project / "src").mkdir(parents=True)
    (project / "src/main.rs").write_text(judge)
    (project / "Cargo.toml").write_text(f'''[package]
name="incrementalrootjudge"
version="0.1.0"
edition="2021"
[dependencies]
clocks={{path="{ROOT / 'bench/clocks'}"}}
[workspace]
''')
    run(["cargo", "build", "--release"], cwd=project, env=dict(os.environ, CARGO_TARGET_DIR=str(TARGET / "target-judge")), log=OUT / "build-judge.log")
    run(["cargo", "nextest", "run", "--release", "--locked"], cwd=project, env=dict(os.environ, CARGO_TARGET_DIR=str(TARGET / "target-judge")), log=OUT / "judge-tests.log")
    executables = {}
    for pair in range(1, 5):
        for side in ("old", "new"):
            manifest = f'''[package]
name="incrementalroots"
version="0.1.0"
edition="2021"
[features]
main-api=[]
next-api=[]
[dependencies]
blake3-servil={{path="{ROOT / side}"}}
blake3="=1.8.7"
clocks={{path="{ROOT / 'bench/clocks'}"}}
[workspace]
'''
            name = f"{side}-{pair}"
            exe, project, env = build_project(name, probe, manifest, pair, family)
            executables[side, pair] = exe
            if pair == 1:
                execute(exe, "preflight-" + side, family, preflight=True)
                if side == "new":
                    for label, flags in [("missing", []), ("both", ["--features", "main-api,next-api"])]:
                        log = OUT / ("feature-" + label + ".log")
                        p = run(["cargo", "build", "--release", "--locked", *flags], cwd=project, env=env, log=log, check=False)
                        assert p.returncode != 0 and "select exactly one finalization contract" in log.read_text(), label
                    marker = "    let mut out = vec![[0u8; 32]; active];"
                    assert probe.count(marker) == 1 and probe.count("    let wanted: Vec<_>") == 1
                    fault = probe.replace("    let wanted: Vec<_>", "    let mut wanted: Vec<_>").replace(marker, "    wanted[0][0] ^= 1;\n" + marker)
                    (OUT / "fault-sampler.rs").write_text(fault)
                    (project / "src/main.rs").write_text(fault)
                    run(["cargo", "build", "--release", "--locked", "--features", family + "-api"], cwd=project, env=env, log=OUT / "fault-build.log")
                    for reverse in (False, True):
                        execute(TARGET / "target-new-1/release/incrementalroots", "fault-" + ("reverse" if reverse else "forward"), family, preflight=True, reverse=reverse, expected_failure=True)
                    (project / "src/main.rs").write_text(probe)
                    assert (project / "src/main.rs").read_text() == probe
                    # The pristine executable was copied before the control.
                    execute(exe, "restored-new", family, preflight=True, reverse=True)
        if pair == 1 and os.environ.get("PINNED_RUN_PREFLIGHT") == "1":
            (OUT / "status.json").write_text(json.dumps({"status": "preflight passed", "family": family, "kind": kind}) + "\n")
            return
    for pair in range(1, 5):
        for side in (("old", "new") if pair % 2 else ("new", "old")):
            execute(executables[side, pair], f"{side}-{pair}", family, reverse=pair % 2 == 0)
    counts = {}
    for algorithm in ("update", "cycle", "finish"):
        result = OUT / f"judgments-{algorithm}.tsv"
        run([TARGET / "target-judge/release/incrementalrootjudge", OUT, algorithm], log=result)
        rows = result.read_text().splitlines()
        assert rows[0].startswith("shape\titems\tmetric\tversus\t")
        verdicts = [r.split("\t")[-1] for r in rows[1:]]
        assert verdicts and all(v in ("Faster", "Level", "Slower") for v in verdicts)
        counts[algorithm] = {name: verdicts.count(name) for name in ("Faster", "Level", "Slower")}
        if kind == "aa":
            assert counts[algorithm]["Faster"] == counts[algorithm]["Slower"] == 0, "the identical-library comparison needs investigation"
    print(json.dumps(counts, sort_keys=True), flush=True)
    (OUT / "status.json").write_text(json.dumps({"status": "complete", "family": family, "kind": kind, "judgments": counts}, indent=2) + "\n")


if __name__ == "__main__":
    main()
