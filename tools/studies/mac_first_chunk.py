#!/usr/bin/env python3
"""Run Sam's frozen complete-cycle instrument through the Mac runner.

The sampler/judge come from first-chunk-cycle.sh beside this file, frozen at
samshackleton-tech/bench-hashes 5b2e7261fd9c547333f2eff6de185044df55c0d2.
This adapter changes orchestration, adds load validation and retains separate
P/E counts on stderr. The timed operation and statistical judgment stay shared.
Run with `aa` for an identical-library control, or `ab` for the ARM candidate.
Every job owns a fresh result directory and private targets. Its runner owns
the process group, deadline, mains-power checks and exclusive host access.
"""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

OLD = "85c4b2d92b71c87a42261509fa6258f02903a0bc"
NEW = "fffe8e13c7a192f019e3a5f24a6e6b09a6067e83"
BENCH = "9994de190259648ede82f7c5e4dbac4f1534a41d"
INSTRUMENT = "5b2e7261fd9c547333f2eff6de185044df55c0d2"
LIBRARY = Path(__file__).resolve().parents[2]
ROOT = Path.cwd()
OUT = ROOT / "out"
TARGET = (Path(os.environ["CARGO_TARGET_DIR"]) / ROOT.name
          if "CARGO_TARGET_DIR" in os.environ else ROOT / "targets")


def run(args, *, cwd=ROOT, env=None, log=None):
    print("+", " ".join(map(str, args)), flush=True)
    if log is None:
        return subprocess.run(list(map(str, args)), cwd=cwd, env=env, check=True)
    with log.open("w") as stream:
        return subprocess.run(list(map(str, args)), cwd=cwd, env=env, stdout=stream, stderr=subprocess.STDOUT, check=True)


def git(directory, *args):
    return subprocess.check_output(["git", "-C", str(directory), *args], text=True).strip()


def extract(source, marker):
    matches = re.findall(r"<<'" + marker + r"'\n(.*?)\n" + marker + r"\n", source, re.S)
    assert len(matches) == 1, marker
    return matches[0] + "\n"


def sampler(source):
    # Supplemental diagnostics preserve the frozen TSV/judge contract.
    old = "            clocks::load::tick();\n            println!("
    new = """            clocks::load::tick();
            if let Some(c) = c {
                eprintln!("core_counts shape={shape} items={n} algorithm={algorithm} calls={calls} p_ns={} p_cycles={} p_instructions={} e_ns={} e_cycles={} e_instructions={}", c.p.time_ns, c.p.cycles, c.p.instructions, c.e.time_ns, c.e.cycles, c.e.instructions);
            } else {
                eprintln!("core_counts unavailable shape={shape} items={n} algorithm={algorithm}");
            }
            println!("""
    assert source.count(old) == 1
    source = source.replace(old, new)
    old = "    blake3_servil::initialize();\n"
    new = old + """    let preflight = std::env::var_os("PINNED_RUN_PREFLIGHT").is_some_and(|v| v == "1");
    if !preflight {
        clocks::load::wait_until_quiet();
    }
"""
    assert source.count(old) == 1
    source = source.replace(old, new)
    # Close the final load window before judging the run's validity.
    closing = """
    if !preflight {
        let windows = clocks::load::windows();
        eprintln!("load: {}", clocks::load::describe(&windows));
        if windows.iter().any(|w| w.busy()) {
            eprintln!("NO VERDICT: other programs loaded this measurement");
            std::process::exit(2);
        }
    }
"""
    end = source.rfind("\n}")
    assert end > 0
    return source[:end] + closing + source[end:]


def build_project(name, code, manifest, pair):
    project = ROOT / ("project-" + name)
    (project / "src").mkdir(parents=True)
    (project / "src/main.rs").write_text(code)
    (project / "Cargo.toml").write_text(manifest)
    target = TARGET / ("target-" + name)
    env = dict(os.environ, CARGO_TARGET_DIR=str(target), RUSTFLAGS=f"-C metadata=first_chunk_pair_{pair}")
    run(["cargo", "build", "--release"], cwd=project, env=env, log=OUT / ("build-" + name + ".log"))
    exe = TARGET / ("exe-" + name)
    shutil.copy2(target / "release/incrementalroots", exe)
    shutil.copy2(project / "Cargo.lock", OUT / ("lock-" + name + ".txt"))
    return exe, project, env


def execute(exe, name, preflight=False, reverse=False, expected_failure=False):
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


def main():
    assert len(sys.argv) == 2 and sys.argv[1] in ("aa", "ab"), __doc__
    kind = sys.argv[1]
    assert not OUT.exists(), "a study gets a fresh result directory"
    OUT.mkdir()
    TARGET.mkdir(parents=True, exist_ok=True)
    source = (Path(__file__).parent / "first-chunk-cycle.sh").read_text()
    probe = sampler(extract(source, "PROBE_EOF"))
    judge = extract(source, "JUDGE_EOF")
    (OUT / "sampler.rs").write_text(probe)
    (OUT / "judge.rs").write_text(judge)
    revisions = {"old": OLD, "new": OLD if kind == "aa" else NEW}
    for side, rev in revisions.items():
        run(["git", "worktree", "add", "--detach", ROOT / side, rev], cwd=LIBRARY)
        assert git(ROOT / side, "rev-parse", "HEAD") == rev
    run(["git", "clone", "--no-checkout", "--shared", LIBRARY.parent / "bench-hashes", ROOT / "bench"])
    run(["git", "fetch", "https://github.com/servil-software-factory/bench-hashes", BENCH], cwd=ROOT / "bench")
    run(["git", "checkout", "--detach", BENCH], cwd=ROOT / "bench")
    identity = {"adapter": git(LIBRARY, "rev-parse", "HEAD"), "adapter_working_tree": git(LIBRARY, "status", "--porcelain", "--untracked-files=all"), "instrument": INSTRUMENT, "instrument_git_blob": git(LIBRARY, "hash-object", str(Path(__file__).parent / "first-chunk-cycle.sh")), "bench": BENCH, "libraries": revisions, "kind": kind, "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip()}
    (OUT / "identity.json").write_text(json.dumps(identity, indent=2) + "\n")
    shutil.copy2(__file__, OUT / "adapter.py")
    shutil.copy2(Path(__file__).parent / "first-chunk-cycle.sh", OUT / "instrument.sh")
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
    executables = {}
    for pair in range(1, 5):
        for side in ("old", "new"):
            manifest = f'''[package]
name="incrementalroots"
version="0.1.0"
edition="2021"
[dependencies]
blake3-servil={{path="{ROOT / side}"}}
blake3="=1.8.7"
clocks={{path="{ROOT / 'bench/clocks'}"}}
[workspace]
'''
            name = f"{side}-{pair}"
            exe, project, env = build_project(name, probe, manifest, pair)
            executables[side, pair] = exe
            if pair == 1:
                execute(exe, "preflight-" + side, preflight=True)
                if side == "new":
                    marker = '        assert_eq!(out, wanted, "{algorithm} {shape} {n}");'
                    assert probe.count(marker) == 1
                    fault = probe.replace(marker, "        out[0][0] ^= 1;\n" + marker)
                    (project / "src/main.rs").write_text(fault)
                    run(["cargo", "build", "--release", "--locked"], cwd=project, env=env, log=OUT / "fault-build.log")
                    execute(TARGET / "target-new-1/release/incrementalroots", "fault-wrong-digest", preflight=True, expected_failure=True)
                    (project / "src/main.rs").write_text(probe)
                    # The pristine executable was copied before the control.
                    execute(exe, "restored-new", preflight=True)
        if pair == 1 and os.environ.get("PINNED_RUN_PREFLIGHT") == "1":
            (OUT / "status.json").write_text(json.dumps({"status": "preflight passed", "kind": kind}) + "\n")
            return
    for pair in range(1, 5):
        for side in (("old", "new") if pair % 2 else ("new", "old")):
            execute(executables[side, pair], f"{side}-{pair}", reverse=pair % 2 == 0)
    result = OUT / "judgments-cycle.tsv"
    run([TARGET / "target-judge/release/incrementalrootjudge", OUT, "cycle"], log=result)
    rows = result.read_text().splitlines()
    assert rows[0].startswith("shape\titems\tmetric\tversus\t")
    verdicts = [r.split("\t")[-1] for r in rows[1:]]
    counts = {name: verdicts.count(name) for name in ("Faster", "Level", "Slower")}
    print(json.dumps(counts, sort_keys=True), flush=True)
    if kind == "aa":
        assert counts["Faster"] == counts["Slower"] == 0, "the identical-library comparison needs investigation"
    (OUT / "status.json").write_text(json.dumps({"status": "complete", "kind": kind, "judgments": counts}, indent=2) + "\n")


if __name__ == "__main__":
    main()
