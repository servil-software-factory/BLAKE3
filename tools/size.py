#!/usr/bin/env python3
"""What the library holds, counted five ways, at a commit or as a change.

    python3 tools/size.py [REV]            the measures at REV (default HEAD)
    python3 tools/size.py BASE REV         both, and what the change adds

For a change's record, beside its speed (AGENTS, "Size beside speed").
The numbers prompt a question (what does the growth buy?); none is a gate
or a goal, and a simplification is credited by the thing it removes, which
its author names. Five measures, since each alone can be moved by making
the code worse to read, and together they cannot:

  what a user learns   public items (functions, methods, types, constants)
                       that the crate's documentation lists
  decisions            the mutants cargo-mutants finds in the library's
                       source: every operator, branch and return it can
                       alter; layout does not move it
  build conditions     the `cfg` conditions in the library's code
  machine code         the bytes of instructions in a release build of the
                       library for this machine: the assembly kernels', and
                       the compiled Rust's (Zooko, November 10, 2026). What
                       ships, and what a cold call's caches hold; a
                       program links the part it calls
  lines                of code, comments and tests apart (so that deleting
                       a comment or a test moves nothing here), by kind:
                       the library, its generated assembly, the generators
                       and tools, the tests

Reads committed code (`git show REV:path`), as the mutation tools do. The
first two and the machine code need their tools: `cargo mutants`, a
toolchain whose rustdoc writes JSON (RUSTC_BOOTSTRAP=1 on a stable one),
and binutils' `size` on the library a release build makes; where one is
missing its line says so, and the others stand.
"""
import json
import os
import re
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TEST_FILES = re.compile(r"src/(test|.*_tests)\.rs$")


def git(*args):
    return subprocess.run(["git", "-C", ROOT, *args], capture_output=True, text=True, check=True).stdout


def files(rev, *patterns):
    return [f for f in git("ls-tree", "-r", "--name-only", rev, "--", *patterns).splitlines()]


def split_rust(text):
    """(code, comment, test, cfg) lines of a Rust file: a `#[cfg(test)]` or
    `#[cfg(all(test, ...))]` module, to its closing brace, is test; `cfg`
    counts the conditions on code lines outside it."""
    code = comment = test = cfg = 0
    lines = text.split("\n")
    i = 0
    while i < len(lines):
        line = lines[i].strip()
        if re.match(r"#\[cfg\((all\()?test\b", line) and i + 1 < len(lines) and re.match(r"\s*(pub(\([a-z]+\))? )?mod \w+ \{", lines[i + 1]):
            depth, j = 0, i + 1
            while j < len(lines):
                depth += lines[j].count("{") - lines[j].count("}")
                j += 1
                if depth <= 0:
                    break
            test += j - i
            i = j
            continue
        if not line:
            pass
        elif line.startswith("//"):
            comment += 1
        else:
            code += 1
            cfg += len(re.findall(r"\bcfg(?:_attr)?!?\(", line))
        i += 1
    return code, comment, test, cfg


def lines_of(rev):
    out = {"library code": 0, "library comments": 0, "unit tests": 0, "build conditions": 0}
    for f in files(rev, "src", "build.rs", "version"):
        if not f.endswith(".rs"):
            continue
        text = git("show", f"{rev}:{f}")
        if TEST_FILES.search(f):
            out["unit tests"] += len(text.split("\n"))
            continue
        code, comment, test, cfg = split_rust(text)
        out["library code"] += code
        out["library comments"] += comment
        out["unit tests"] += test
        out["build conditions"] += cfg
    count = lambda fs: sum(len(git("show", f"{rev}:{f}").split("\n")) for f in fs)
    out["integration tests"] = count(f for f in files(rev, "tests", "test_vectors/src") if f.endswith(".rs"))
    out["generated assembly"] = count(f for f in files(rev, "c") if f.endswith(".S"))
    out["generators and tools"] = count(f for f in files(rev, "tools") if f.endswith((".py", ".sh", ".rs", ".lean")))
    return out


def checkout(rev, into):
    subprocess.run(["git", "-C", ROOT, "worktree", "add", "--detach", "--quiet", into, rev], check=True)


def mutants(tree):
    listed = set()
    for name in sorted(os.listdir(os.path.join(tree, "tools", "mutants"))):
        if name.startswith("known-") and name.endswith(".txt"):
            for line in open(os.path.join(tree, "tools", "mutants", name)):
                if line.startswith("files:"):
                    listed |= set(line.split()[1:])
    if not listed:
        return None
    args = ["cargo", "mutants", "--list"] + [a for f in sorted(listed) if os.path.exists(os.path.join(tree, f)) for a in ("--file", f)]
    try:
        return len(subprocess.run(args, cwd=tree, capture_output=True, text=True, check=True).stdout.splitlines())
    except (OSError, subprocess.CalledProcessError):
        return None


def public_items(tree, target):
    env = dict(os.environ, RUSTC_BOOTSTRAP="1", CARGO_TARGET_DIR=target)
    try:
        subprocess.run(["cargo", "rustdoc", "--lib", "--quiet", "--", "-Z", "unstable-options", "--output-format", "json"],
                       cwd=tree, env=env, capture_output=True, text=True, check=True)
        doc = json.load(open(os.path.join(target, "doc", "blake3_servil.json")))
    except (OSError, subprocess.CalledProcessError, ValueError):
        return None
    index, seen, count = doc["index"], set(), 0

    def item(i):
        nonlocal count
        it = index.get(str(i))
        if it is None:
            count += 1  # a re-export of another crate's item
            return
        kind = next(iter(it["inner"]))
        inner = it["inner"][kind]
        if kind == "use":
            target_id = inner.get("id")
            if target_id is None or str(target_id) not in index:
                count += 1
            else:
                item(target_id)
            return
        if kind == "module":
            for child in inner["items"]:
                if index[str(child)]["visibility"] == "public":
                    item(child)
            return
        count += 1
        if it["id"] in seen:
            return
        seen.add(it["id"])
        if kind == "enum":
            count += len(inner["variants"])
        for impl in inner.get("impls", []) if isinstance(inner, dict) else []:
            body = index[str(impl)]["inner"]["impl"]
            if body.get("trait") is None:
                count += sum(1 for m in body["items"] if index[str(m)]["visibility"] == "public")

    item(doc["root"])
    return count


def machine_code(tree, target):
    """(the assembly kernels' bytes, the compiled Rust's): the text of
    each object in the release build's library, the kernels' by their
    file names."""
    env = dict(os.environ, CARGO_TARGET_DIR=target)
    try:
        subprocess.run(["cargo", "build", "--release", "--lib", "--quiet"], cwd=tree, env=env, capture_output=True, text=True, check=True)
        out = subprocess.run(["size", os.path.join(target, "release", "libblake3_servil.rlib")], capture_output=True, text=True, check=True).stdout
    except (OSError, subprocess.CalledProcessError):
        return None, None
    kernels = rust = 0
    for line in out.splitlines()[1:]:
        fields = line.split()
        if len(fields) >= 6 and fields[0].isdigit():
            if re.search(r"blake3_\w+\.o", fields[5]):
                kernels += int(fields[0])
            else:
                rust += int(fields[0])
    return (kernels, rust) if kernels and rust else (None, None)


def resolve(rev):
    """The commit `rev` names where the tool is run: a name such as HEAD
    is the caller's, each worktree having its own."""
    return subprocess.run(["git", "rev-parse", "--verify", f"{rev}^{{commit}}"], capture_output=True, text=True, check=True).stdout.strip()


def measures(rev):
    out = lines_of(rev)
    with tempfile.TemporaryDirectory(prefix="size-") as scratch:
        tree = os.path.join(scratch, "tree")
        checkout(rev, tree)
        try:
            out["decisions (mutants)"] = mutants(tree)
            out["public items"] = public_items(tree, os.path.join(scratch, "target"))
            out["machine code: kernels"], out["machine code: Rust"] = machine_code(tree, os.path.join(scratch, "target"))
        finally:
            subprocess.run(["git", "-C", ROOT, "worktree", "remove", "--force", tree], check=True)
    return rev, out


ORDER = ["public items", "decisions (mutants)", "build conditions", "machine code: kernels", "machine code: Rust", "library code", "library comments",
         "unit tests", "integration tests", "generated assembly", "generators and tools"]


def main():
    revs = sys.argv[1:] or ["HEAD"]
    assert len(revs) <= 2, __doc__
    commits = [resolve(r) for r in revs]
    assert len(set(commits)) == len(commits), f"size: {revs[0]} and {revs[-1]} are one commit ({commits[0][:12]}): nothing to compare"
    results = [measures(c) for c in commits]
    show = lambda v: "not measured here" if v is None else f"{v:,}"
    if len(results) == 1:
        rev, m = results[0]
        print(f"size: BLAKE3-Servil at {rev[:12]}")
        for k in ORDER:
            print(f"  {k:22} {show(m[k]):>18}")
    else:
        (base, a), (rev, b) = results
        print(f"size: {rev[:12]} against {base[:12]}")
        for k in ORDER:
            delta = "" if a[k] is None or b[k] is None else f"{b[k] - a[k]:+,}"
            print(f"  {k:22} {show(a[k]):>18} {show(b[k]):>18} {delta:>10}")


if __name__ == "__main__":
    main()
