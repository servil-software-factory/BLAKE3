#!/usr/bin/env python3
"""The dependency policy's checks, as CI runs them (CONTRIBUTING.md, "The dependency policy").

    python3 tools/dependency_policy.py              the checks
    python3 tools/dependency_policy.py controls     planted faults, each of which the checks must fail
    python3 tools/dependency_policy.py vet ARGS...  cargo vet ARGS on the release workspace and the tree's
                                                    audits (diff, certify, add-exemption, prune ...)

What is checked is both locked resolutions the repository builds and ships from, under every feature:
- `release`: all six crates in the repository workspace (tools/release/workspace.toml and
  tools/release/Cargo.lock), covering the four published crates (tools/crates-release.sh) plus `capi/`
  and `test_vectors/` (tools/ci-locked.sh).
- `cli`: the standalone CLI (b3sum/Cargo.toml and b3sum/Cargo.lock), as tools/b3sum-release.sh builds it.

Each invocation stages its own private temporary directory under target/ (cleaned up on exit), holding a
copy of the tracked files as the working tree has them, so concurrent runs never collide and a planted
fault never touches the tree. On each of the two graphs:
- `by path`: every package without a source must match one of the graph's repository crates (`OURS` for
  `release`, `CLI_OURS` for `cli`) by both name and manifest path, and Cargo's workspace_members must be
  exactly the expected roots (`strangers`).
- cargo-deny against deny.toml (`--color never`, `-D index-failure`): `advisories`, `bans`, `licenses`,
  `sources`.
- `audits`: cargo-vet against tools/release/supply-chain/ (`--locked`).
"""
from contextlib import contextmanager
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
AUDITS = Path("tools") / "release" / "supply-chain"
# The four crates reachable from standalone b3sum/Cargo.toml, by package name and folder.
CLI_OURS = {
    "blake3-servil-version": "version",
    "blake3-servil-core": ".",
    "blake3-servil": "threads",
    "b3sums": "b3sum",
}
# All six crates in the repository workspace (tools/release/workspace.toml).
OURS = {
    **CLI_OURS,
    "blake3-servil-c": "capi",
    "test_vectors": "test_vectors",
}


def _copy_tracked(dest, tracked_files):
    for name in tracked_files:
        src = ROOT / name
        if src.is_file() or src.is_symlink():                          # a tracked file deleted in the working tree stays out
            target = dest / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(src, target, follow_symlinks=False)


@contextmanager
def staged():
    """A private staging directory under target/ with two separate checkouts (`release/checkout` under
    the virtual release workspace, and standalone `cli/` with no parent workspace above it), plus copies
    of deny.toml and the audit store."""
    target_dir = ROOT / "target"
    target_dir.mkdir(exist_ok=True)
    tracked = [n for n in subprocess.run(["git", "ls-files", "-z"], cwd=ROOT, check=True,
                                         capture_output=True, text=True).stdout.split("\0") if n]
    with tempfile.TemporaryDirectory(prefix="dependency-policy-", dir=target_dir) as tmp:
        stage = Path(tmp)
        _copy_tracked(stage / "release" / "checkout", tracked)
        shutil.copy(stage / "release" / "checkout" / "tools" / "release" / "workspace.toml", stage / "release" / "Cargo.toml")
        shutil.copy(stage / "release" / "checkout" / "tools" / "release" / "Cargo.lock", stage / "release" / "Cargo.lock")
        _copy_tracked(stage / "cli", tracked)
        shutil.copy(ROOT / "deny.toml", stage / "deny.toml")
        shutil.copytree(ROOT / AUDITS, stage / "supply-chain")
        yield stage


def strangers(metadata, checkout, expected_members, workspace_members_list=None, ours=OURS):
    """What is wrong with the packages that have no source, in `cargo metadata`'s account of a graph:
    - If `workspace_members_list` is given (from tools/release/workspace.toml), it must name the four
      checkout folders alone.
    - Cargo's own `workspace_members` must resolve to exactly `expected_members` (catching implicit
      members that Cargo pulls into a workspace).
    - Every package with `source is None` must match one of `ours` by both name and manifest path.
    - Every crate in `ours` must be present in the graph."""
    checkout = Path(checkout).resolve()
    wrong = []
    if workspace_members_list is not None:
        expected_folders = {f"checkout/{folder}" if folder != "." else "checkout" for folder in ours.values()}
        if set(workspace_members_list) != expected_folders:
            extra = sorted(set(workspace_members_list) - expected_folders)
            missing = sorted(expected_folders - set(workspace_members_list))
            wrong.append(f"tools/release/workspace.toml lists members {sorted(workspace_members_list)}; expected {sorted(expected_folders)}"
                         + (f" (unexpected: {extra})" if extra else "") + (f" (missing: {missing})" if missing else ""))
    by_id = {p["id"]: p for p in metadata["packages"]}
    actual_members = {by_id[mid]["name"] if mid in by_id else mid for mid in metadata.get("workspace_members", [])}
    if actual_members != set(expected_members):
        wrong.append(f"Cargo workspace_members are {sorted(actual_members)}; expected {sorted(expected_members)}")
    expected_paths = {name: (checkout / folder / "Cargo.toml").resolve() for name, folder in ours.items()}
    seen = set()
    for package in sorted((p for p in metadata["packages"] if p["source"] is None), key=lambda p: (p["name"], p["manifest_path"])):
        name = package["name"]
        path = Path(package["manifest_path"]).resolve()
        if expected_paths.get(name) == path:
            seen.add(name)
            continue
        try:
            where = f"{path.parent.relative_to(checkout)}/"
        except ValueError:
            where = str(path.parent)
        wrong.append(f"{name} {package['version']} is taken by path, from {where}, and matches none of the repository crates "
                     "by name and path: a dependency comes from crates.io, where the policy's checks see it")
    for name in sorted(set(ours) - seen):
        wrong.append(f"expected crate {name} at {ours[name]}/Cargo.toml is absent from the graph")
    return wrong


def check_graph(manifest, checkout, expected_members, deny_config, audit_store, workspace_members_list=None, ours=OURS, say=print, environment=None):
    """Run all checks on one locked graph; return the list of failed check-group names."""
    def run(*command):
        done = subprocess.run(command, cwd=manifest.parent, env=environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        say(done.stderr + (done.stdout if command[1] != "metadata" else ""), end="")
        return done

    metadata = run("cargo", "metadata", "--locked", "--all-features", "--format-version", "1", "--manifest-path", manifest)
    if metadata.returncode:
        return ["the lock"]                                                # Cargo could not read the graph with this lock
    wrong = strangers(json.loads(metadata.stdout), checkout, expected_members, workspace_members_list=workspace_members_list, ours=ours)
    for line in wrong:
        say(f"dependency policy: {line}")
    names = ["by path"] if wrong else []
    deny = run("cargo", "deny", "--color", "never", "--manifest-path", manifest, "--locked", "--config", deny_config,
               "check", "-D", "index-failure")
    if deny.returncode:
        names += re.findall(r"(\w+) FAILED", deny.stdout + deny.stderr) or ["cargo-deny"]
    vet_store = audit_store
    unused_policies = sorted(set(OURS) - set(ours))
    with tempfile.TemporaryDirectory(prefix="vet-store-", dir=audit_store.parent) as tmp_store:
        if unused_policies:
            vet_store = Path(tmp_store) / "supply-chain"
            shutil.copytree(audit_store, vet_store)
            cfg = vet_store / "config.toml"
            text = cfg.read_text()
            for crate in unused_policies:
                text = strip_toml_crate_sections(text, crate, kinds=("policy",))
            cfg.write_text(text)
            subprocess.run(["cargo", "vet", "fmt", "--manifest-path", manifest, "--store-path", vet_store],
                           cwd=manifest.parent, env=environment, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        vet = run("cargo", "vet", "--manifest-path", manifest, "--store-path", vet_store, "--locked")
    return names + (["audits"] if vet.returncode else [])


def failed(stage, say=print, environment=None):
    """Check both locked resolutions (`release` workspace and standalone `cli`), returning `{graph: [failed]}`."""
    release_manifest = stage / "release" / "Cargo.toml"
    cli_manifest = stage / "cli" / "b3sum" / "Cargo.toml"
    members_list = tomllib.loads(release_manifest.read_text())["workspace"]["members"]
    deny_config = stage / "deny.toml"
    audit_store = stage / "supply-chain"
    return {
        "release": check_graph(release_manifest, stage / "release" / "checkout", set(OURS), deny_config, audit_store,
                               workspace_members_list=members_list, ours=OURS, say=say, environment=environment),
        "cli": check_graph(cli_manifest, stage / "cli", {"b3sums"}, deny_config, audit_store, ours=CLI_OURS, say=say, environment=environment),
    }


def _header_matches_crate(header_line, crate, kinds=("audits", "exemptions", "trusted", "wildcard-audits")):
    """Does a TOML section header (`[...]` or `[[...]]`) belong to one of `kinds` for `crate`?"""
    inner = header_line.strip()
    if inner.startswith("[[") and inner.endswith("]]"):
        inner = inner[2:-2].strip()
    elif inner.startswith("[") and inner.endswith("]"):
        inner = inner[1:-1].strip()
    else:
        return False
    parts = [p.strip().strip('"').strip("'") for p in re.findall(r'"[^"]*"|\'[^\']*\'|[^.\s]+', inner)]
    return len(parts) >= 2 and parts[-1] == crate and parts[-2] in kinds


def strip_toml_crate_sections(text, crate, kinds=("audits", "exemptions", "trusted", "wildcard-audits")):
    """Remove every TOML section whose header matches `kinds` for `crate`."""
    out = []
    skip = False
    in_triple = None
    for line in text.splitlines(keepends=True):
        if in_triple is not None:
            if not skip:
                out.append(line)
            if in_triple in line:
                in_triple = None
            continue
        stripped = line.lstrip()
        if stripped.startswith("["):
            skip = _header_matches_crate(stripped, crate, kinds=kinds)
        if not skip:
            out.append(line)
        for quote in ('"""', "'''"):
            if line.count(quote) % 2 == 1:
                in_triple = quote
                break
    return "".join(out)


def strip_audit_evidence(stage, crate, replace_with=()):
    """Strip all audit, exemption, trust and wildcard records for `crate` across the staged cargo-vet store,
    regardless of how `crate` is currently certified. Append `(version, criteria)` exemptions if given, and
    reformat with `cargo vet fmt` so `--locked` sees canonical formatting."""
    store = stage / "supply-chain"
    for name in ("config.toml", "audits.toml", "imports.lock"):
        path = store / name
        if path.exists():
            cleaned = strip_toml_crate_sections(path.read_text(), crate)
            if name == "audits.toml" and "audits" not in tomllib.loads(cleaned):
                cleaned = cleaned.rstrip() + "\n\n[audits]\n"
            path.write_text(cleaned)
    if replace_with:
        key = f'"{crate}"' if not re.fullmatch(r"[A-Za-z0-9_-]+", crate) else crate
        extra = "".join(f'\n[[exemptions.{key}]]\nversion = "{version}"\ncriteria = "{criteria}"\n'
                        for version, criteria in replace_with)
        config = store / "config.toml"
        config.write_text(config.read_text() + extra)
    subprocess.run(["cargo", "vet", "fmt", "--manifest-path", stage / "release" / "Cargo.toml", "--store-path", store],
                   cwd=stage / "release", check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def edit(path, pattern, replacement):
    """One place in a staged file, by a pattern that a version's change leaves matching."""
    text, count = re.subn(pattern, replacement, path.read_text(), count=1, flags=re.M)
    assert count == 1, f"{path}: nothing matches {pattern}; this control needs another fault"
    path.write_text(text)


def locked_version(lock_path, crate):
    versions = [p["version"] for p in tomllib.loads(lock_path.read_text())["package"] if p["name"] == crate]
    assert len(versions) == 1, f"{lock_path}: expected one {crate}, found {versions}"
    return versions[0]


def _write_path_crate(crate_dir, name, licence):
    (crate_dir / "src").mkdir(parents=True, exist_ok=True)
    (crate_dir / "src" / "lib.rs").write_text("")
    (crate_dir / "Cargo.toml").write_text(f'[package]\nname = "{name}"\nversion = "0.1.0"\nedition = "2021"\nlicense = "{licence}"\n')


def a_vulnerable_version(stage):
    # RUSTSEC-2026-0007 (vulnerability advisory). Both locks reach bytes through bao-tree (tests only).
    # Replace all audit evidence for bytes with a safe-to-run exemption for 1.11.0 so the advisory is
    # all that fails, even if bytes is later audited.
    for lock in (stage / "release" / "Cargo.lock", stage / "cli" / "b3sum" / "Cargo.lock"):
        edit(lock, r'(name = "bytes"\nversion = ")[^"]*("\nsource = "[^"]*"\nchecksum = ")[0-9a-f]*',
             r"\g<1>1.11.0\g<2>b35204fbdc0b3f4446b89fc1ac2cf84a8a68971995d0bf2e925ec7cd960f9cb3")
    strip_audit_evidence(stage, "bytes", replace_with=[("1.11.0", "agent-safe-to-run")])


def no_licence_exception(stage):
    edit(stage / "deny.toml", r'^    \{ allow = \["Unicode-3\.0"\], crate = "unicode-ident" \},\n', "")


def a_crate_by_path_inside(stage):
    """A fifth MIT crate copied inside each checkout and taken by path as a CLI test dependency."""
    for checkout, lock in ((stage / "release" / "checkout", stage / "release" / "Cargo.lock"),
                           (stage / "cli", stage / "cli" / "b3sum" / "Cargo.lock")):
        _write_path_crate(checkout / "copied", "copied", "MIT")
        edit(checkout / "b3sum" / "Cargo.toml", r"^\[dev-dependencies\]\n", '[dev-dependencies]\ncopied = { path = "../copied" }\n')
        edit(lock, r'(name = "b3sums"\nversion = "[^"]*"\ndependencies = \[\n)', r'\g<1> "copied",\n')
        with open(lock, "a") as f:
            f.write('\n[[package]]\nname = "copied"\nversion = "0.1.0"\n')


def a_dev_only_gpl_crate_outside(stage):
    """A GPL-3.0-only crate staged OUTSIDE both workspace roots and referenced solely from [dev-dependencies]:
    Cargo keeps workspace_members at 4 (release) and 1 (cli), so only include-dev = true rejects its licence."""
    outside = stage / "outside" / "dev_gpl"
    _write_path_crate(outside, "dev_gpl", "GPL-3.0-only")
    for manifest, lock, rel in ((stage / "release" / "checkout" / "b3sum" / "Cargo.toml", stage / "release" / "Cargo.lock", "../../../outside/dev_gpl"),
                                (stage / "cli" / "b3sum" / "Cargo.toml", stage / "cli" / "b3sum" / "Cargo.lock", "../../outside/dev_gpl")):
        edit(manifest, r"^\[dev-dependencies\]\n", f'[dev-dependencies]\ndev_gpl = {{ path = "{rel}" }}\n')
        edit(lock, r'(name = "b3sums"\nversion = "[^"]*"\ndependencies = \[\n)', r'\g<1> "dev_gpl",\n')
        with open(lock, "a") as f:
            f.write('\n[[package]]\nname = "dev_gpl"\nversion = "0.1.0"\n')


def an_optional_feature_gpl_crate(stage):
    """A GPL-3.0-only crate outside the workspace, reachable ONLY through a new non-default feature of
    blake3-servil-core: tests --all-features for both cargo metadata (by path) and cargo-deny (licenses)."""
    outside = stage / "outside" / "opt_gpl"
    _write_path_crate(outside, "opt_gpl", "GPL-3.0-only")
    manifest = stage / "release" / "checkout" / "Cargo.toml"
    lock = stage / "release" / "Cargo.lock"
    edit(manifest, r"^\[features\]\n", '[features]\npolicy_control_feature = ["dep:opt_gpl"]\n')
    edit(manifest, r"^\[dependencies\]\n", '[dependencies]\nopt_gpl = { path = "../../outside/opt_gpl", optional = true }\n')
    edit(lock, r'(name = "blake3-servil-core"\nversion = "[^"]*"\ndependencies = \[\n)', r'\g<1> "opt_gpl",\n')
    with open(lock, "a") as f:
        f.write('\n[[package]]\nname = "opt_gpl"\nversion = "0.1.0"\n')


def no_audit_record_libc(stage):
    """Drop all evidence (exemption, local audit, imported audit, trust) for libc."""
    strip_audit_evidence(stage, "libc")


def optional_feature_only_safe_to_run(stage):
    """Plant an optional feature depending on safe-to-run-only arrayvec: default builds ignore it, so ONLY
    deploy-strength checking under --all-features catches it."""
    ver = locked_version(stage / "release" / "Cargo.lock", "arrayvec")
    strip_audit_evidence(stage, "arrayvec", replace_with=[(ver, "agent-safe-to-run")])
    edit(stage / "release" / "checkout" / "Cargo.toml", r"^\[dependencies\]\n",
         f'[dependencies]\narrayvec = {{ version = "={ver}", default-features = false, optional = true }}\n')
    edit(stage / "release" / "Cargo.lock", r'(name = "blake3-servil-core"\nversion = "[^"]*"\ndependencies = \[\n)',
         r'\g<1> "arrayvec",\n')


def arrayvec_in_what_ships(stage):
    """arrayvec is safe-to-run only (tests); adding it to b3sum's normal dependencies must fail audits."""
    ver = locked_version(stage / "release" / "Cargo.lock", "arrayvec")
    strip_audit_evidence(stage, "arrayvec", replace_with=[(ver, "agent-safe-to-run")])
    for checkout, lock in ((stage / "release" / "checkout", stage / "release" / "Cargo.lock"),
                           (stage / "cli", stage / "cli" / "b3sum" / "Cargo.lock")):
        edit(checkout / "b3sum" / "Cargo.toml", r"^\[dependencies\]\n", f'[dependencies]\narrayvec = "{ver}"\n')
        edit(lock, r'(name = "b3sums"\nversion = "[^"]*"\ndependencies = \[\n)', r'\g<1> "arrayvec",\n')


def cli_selects_unapproved_version(stage):
    """Two valid locks whose package sets agree with gen-ver.py --check: release selects syn 3.x
    (safe-to-deploy) for a core build-dependency, while standalone cli selects syn 2.x (safe-to-run
    only). Release must pass and standalone cli must fail audits."""
    release_lock = stage / "release" / "Cargo.lock"
    syn_versions = [p["version"] for p in tomllib.loads(release_lock.read_text())["package"] if p["name"] == "syn"]
    syn2_ver = next((v for v in syn_versions if v.startswith("2.")), None)
    syn3_ver = next((v for v in syn_versions if v.startswith("3.")), None)
    assert syn2_ver and syn3_ver, f"release lock needs both syn 2.x and 3.x, found {syn_versions}"
    # Resolve each lock canonically with Cargo (offline, from the already-cached crates), then broaden
    # both manifests to the same requirement (>=2, <4) so both locks remain valid under --locked.
    for manifest, workdir, pinned in (
        (stage / "release" / "checkout" / "Cargo.toml", stage / "release", syn3_ver),
        (stage / "cli" / "Cargo.toml", stage / "cli" / "b3sum", syn2_ver),
    ):
        text = manifest.read_text()
        if re.search(r"^\[build-dependencies\]\n", text, re.MULTILINE):
            edit(manifest, r"^\[build-dependencies\]\n", f'[build-dependencies]\nsyn = "={pinned}"\n')
        else:
            manifest.write_text(text + f'\n[build-dependencies]\nsyn = "={pinned}"\n')
        subprocess.run(["cargo", "metadata", "--offline", "--format-version", "1"], cwd=workdir,
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        edit(manifest, rf'^syn = "={re.escape(pinned)}"\n', 'syn = ">=2, <4"\n')
    strip_audit_evidence(stage, "syn", replace_with=[(syn3_ver, "agent-safe-to-deploy"), (syn2_ver, "agent-safe-to-run")])
    # Promoting syn from test-only to a core build-dependency also promotes its three proc-macro
    # support crates from dev-criteria to deploy-criteria; exempt them at deploy strength in this
    # synthetic stage so only syn 2.x vs 3.x decides the outcome.
    for dep in ("proc-macro2", "quote", "unicode-ident"):
        dep_ver = locked_version(release_lock, dep)
        strip_audit_evidence(stage, dep, replace_with=[(dep_ver, "agent-safe-to-deploy")])


def an_unread_index(stage):
    """Both resolutions vendored beside their manifests and a Cargo home with no crates.io index: no
    version's yank status can be read. Copy any cached advisory DB so cargo-deny need not re-clone it."""
    for folder in (stage / "release", stage / "cli" / "b3sum"):
        vendored = subprocess.run(["cargo", "vendor", "--locked", "vendor"], cwd=folder, check=True, capture_output=True, text=True)
        (folder / ".cargo").mkdir()
        (folder / ".cargo" / "config.toml").write_text(vendored.stdout)
    cargo_home = stage / "cargo-home"
    cargo_home.mkdir()
    home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
    if (home / "advisory-dbs").is_dir():
        shutil.copytree(home / "advisory-dbs", cargo_home / "advisory-dbs")
    return dict(os.environ, CARGO_HOME=str(cargo_home), PATH=f"{home / 'bin'}{os.pathsep}{os.environ['PATH']}",
                RUSTUP_HOME=os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")))


CONTROLS = [
    ("a version with a vulnerability advisory, reached through a test dependency alone",
     a_vulnerable_version, {"release": ["advisories"], "cli": ["advisories"]}),
    ("a licence outside the list, with no exception recorded",
     no_licence_exception, {"release": ["licenses"], "cli": ["licenses"]}),
    ("a crate copied into the checkout and taken by path",
     a_crate_by_path_inside, {"release": ["by path"], "cli": ["by path"]}),
    ("a dev-only GPL crate outside the workspace (tests include-dev without implicit membership)",
     a_dev_only_gpl_crate_outside, {"release": ["by path", "licenses"], "cli": ["by path", "licenses"]}),
    ("a GPL crate behind a non-default feature (tests all-features in metadata and deny)",
     an_optional_feature_gpl_crate, {"release": ["by path", "licenses"], "cli": []}),
    ("a shipped crate with all audit evidence stripped",
     no_audit_record_libc, {"release": ["audits"], "cli": ["audits"]}),
    ("optional feature depending on safe-to-run-only arrayvec (tests deploy check under all-features)",
     optional_feature_only_safe_to_run, {"release": ["audits"], "cli": []}),
    ("arrayvec (safe-to-run only) reintroduced into what ships",
     arrayvec_in_what_ships, {"release": ["audits"], "cli": ["audits"]}),
    ("two valid locks where standalone cli selects an unapproved build-dep version",
     cli_selects_unapproved_version, {"release": [], "cli": ["audits"]}),
    ("versions whose yank status cannot be read",
     an_unread_index, {"release": ["advisories"], "cli": ["advisories"]}),
]


def _summary(outcomes):
    parts = [f"{graph}: {', '.join(names)}" for graph, names in outcomes.items() if names]
    return "; ".join(parts) + " failed" if parts else "every check passed"


def controls():
    """Each planted fault must fail the exact checks named beside it on each graph; untouched must pass."""
    holes = 0
    for what, plant, expected in [*CONTROLS, ("nothing planted", lambda stage: None, {"release": [], "cli": []})]:
        with staged() as stage:
            buf = []
            env = plant(stage)
            found = failed(stage, say=lambda *said, **how: buf.append(" ".join(map(str, said)) + how.get("end", "\n")), environment=env)
        ok = found == expected
        holes += not ok
        print(f"{'as it should' if ok else 'A HOLE'}: {what}: {_summary(found)}"
              + ("" if ok else f"; expected {_summary(expected)}"))
        if not ok:
            for line in "".join(buf).splitlines():
                if line.strip() and not line.startswith(("warning: profiles", "package:", "workspace:")):
                    print(f"    | {line}", file=sys.stderr)
    return 1 if holes else 0


def main(arguments):
    if arguments == ["controls"]:
        return controls()
    with staged() as stage:
        if arguments[:1] == ["vet"]:
            # cargo-vet 0.10.4 takes these two options after a subcommand, and refuses them before it.
            return subprocess.run(["cargo", "vet", *arguments[1:], "--manifest-path", stage / "release" / "Cargo.toml",
                                   "--store-path", ROOT / AUDITS]).returncode
        if arguments:
            sys.exit(__doc__)
        outcomes = failed(stage)
        bad = {graph: names for graph, names in outcomes.items() if names}
        if bad:
            print(f"dependency policy: failed: {_summary(outcomes)}", file=sys.stderr)
            return 1
        return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
