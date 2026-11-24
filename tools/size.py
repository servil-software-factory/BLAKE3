#!/usr/bin/env python3
"""What the library holds, counted five ways, at a commit or as a change:
its crates together (the core, and the crate with the pool where the tree
has one).

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
# A module's declaration with the attributes before it: `mod name;`, or
# an inline module's opening.
MODULE = re.compile(r'((?:#\[[^\]]*\]\s*)+)(?:pub(?:\([a-z]+\))? )?mod (\w+)\s*([;{])')


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


def crates(rev):
    """The library's crates at `rev`, each by its folder ("" the root's):
    the core, and the crate with the pool where the tree has it (#335)."""
    return [""] + (["threads/"] if files(rev, "threads/Cargo.toml") else [])


def for_tests_alone(attributes):
    """Whether a `cfg` among `attributes` compiles its item for tests
    alone: `cfg(test)`, or an `all(...)` that names `test` and neither
    `any` nor `not`."""
    for condition in re.findall(r"#\[cfg\((.*?)\)\]", attributes, re.S):
        if condition == "test" or (condition.startswith("all(") and re.search(r"\btest\b", condition) and "any(" not in condition and "not(" not in condition):
            return True
    return False


def test_only(texts):
    """The files and folders, of `texts` (path: Rust source), compiled for
    tests alone, each found where another file names it under such a
    `cfg`: a declared module's file (its `#[path]`, or its name) and the
    folder of its children; an inline module's folder, and the files it
    includes."""
    found = set()
    for path, text in texts.items():
        beside = os.path.dirname(path)
        # A module's children sit beside lib.rs or mod.rs, or in a folder
        # named for any other file.
        below = beside if os.path.basename(path) in ("lib.rs", "mod.rs", "main.rs") else path[:-3]
        for m in MODULE.finditer(text):
            attributes, name, opening = m.groups()
            if not for_tests_alone(attributes):
                continue
            given = re.search(r'#\[path = "([^"]+)"\]', attributes)
            if opening == ";":
                found.add(os.path.normpath(os.path.join(beside, given.group(1))) if given else os.path.join(below, name + ".rs"))
            else:
                depth, end = 1, m.end()
                while depth and end < len(text):
                    depth += {"{": 1, "}": -1}.get(text[end], 0)
                    end += 1
                found |= {os.path.normpath(os.path.join(beside, f)) for f in re.findall(r'include!\("([^"]+)"\)', text[m.end():end])}
            found.add(os.path.join(below, name) + "/")
    return found


def lines_of(rev):
    out = {"library code": 0, "library comments": 0, "unit tests": 0, "build conditions": 0}
    sources = [f for c in crates(rev) for f in files(rev, c + "src", c + "build.rs") if f.endswith(".rs")]
    sources += [f for f in files(rev, "version") if f.endswith(".rs")]
    texts = {f: git("show", f"{rev}:{f}") for f in sources}
    tests = test_only(texts)
    for f, text in texts.items():
        if f in tests or any(f.startswith(t) for t in tests if t.endswith("/")):
            out["unit tests"] += len(text.split("\n"))
            continue
        code, comment, test, cfg = split_rust(text)
        out["library code"] += code
        out["library comments"] += comment
        out["unit tests"] += test
        out["build conditions"] += cfg
    count = lambda fs: sum(len(git("show", f"{rev}:{f}").split("\n")) for f in fs)
    out["integration tests"] = count(f for f in files(rev, "tests", "threads/tests", "test_vectors/src") if f.endswith(".rs"))
    out["generated assembly"] = count(f for f in files(rev, "c") if f.endswith(".S"))
    out["generators and tools"] = count(f for f in files(rev, "tools") if f.endswith((".py", ".sh", ".rs", ".lean")))
    return out


def checkout(rev, into):
    subprocess.run(["git", "-C", ROOT, "worktree", "add", "--detach", "--quiet", into, rev], check=True)


def crate_dirs(tree):
    """(the crate's folder, its mutation lists' folder) for each crate of a checkout."""
    dirs = [(tree, os.path.join(tree, "tools", "mutants"))]
    if os.path.exists(os.path.join(tree, "threads", "Cargo.toml")):
        dirs.append((os.path.join(tree, "threads"), os.path.join(tree, "tools", "mutants", "threads")))
    return dirs


def lib_name(crate):
    """The crate's library, as rustdoc and the build name their files."""
    meta = json.loads(subprocess.run(["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=crate, capture_output=True, text=True, check=True).stdout)
    package = next(p for p in meta["packages"] if os.path.dirname(p["manifest_path"]) == os.path.realpath(crate))
    return next(t["name"] for t in package["targets"] if "lib" in t["kind"]).replace("-", "_")


def mutants(tree):
    """Every crate's mutants, in the files its lists name."""
    total = 0
    for crate, lists in crate_dirs(tree):
        listed = set()
        for name in sorted(os.listdir(lists)) if os.path.isdir(lists) else []:
            if name.startswith("known-") and name.endswith(".txt"):
                for line in open(os.path.join(lists, name)):
                    if line.startswith("files:"):
                        listed |= set(line.split()[1:])
        if not listed:
            return None
        args = ["cargo", "mutants", "--list"] + [a for f in sorted(listed) if os.path.exists(os.path.join(crate, f)) for a in ("--file", f)]
        try:
            total += len(subprocess.run(args, cwd=crate, capture_output=True, text=True, check=True).stdout.splitlines())
        except (OSError, subprocess.CalledProcessError):
            return None
    return total


def public_items(tree, target):
    """Every crate's public items: the core's, and what the crate with the
    pool adds (its own items, and one for each re-export of the core's)."""
    env = dict(os.environ, RUSTC_BOOTSTRAP="1", CARGO_TARGET_DIR=target)
    total = 0
    for crate, _ in crate_dirs(tree):
        try:
            subprocess.run(["cargo", "rustdoc", "--lib", "--quiet", "--", "-Z", "unstable-options", "--output-format", "json"],
                           cwd=crate, env=env, capture_output=True, text=True, check=True)
            doc = json.load(open(os.path.join(target, "doc", lib_name(crate) + ".json")))
        except (OSError, subprocess.CalledProcessError, ValueError, StopIteration):
            return None
        total += count_items(doc)
    return total


def count_items(doc):
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


def is_kernel_object(name):
    """Whether an object of the library's archive is an assembly kernel's:
    the C compiler's objects are `HASH-blake3_NAME.o`, NAME as the file in
    c/ has it (`blake3_sse41_x86-64_unix`, hyphen included); the Rust
    compiler's end in `.rcgu.o`."""
    return not name.endswith(".rcgu.o") and re.search(r"(?:^|-)blake3_[\w-]+\.o$", name) is not None


def machine_code(tree, target):
    """(the assembly kernels' bytes, the compiled Rust's): the text of
    each object in each crate's release library, the kernels' by their
    file names (legacy cc builds) or `blake3_servil_*` symbols (`global_asm!`)."""
    env = dict(os.environ, CARGO_TARGET_DIR=target)
    kernels = rust = 0
    for crate, _ in crate_dirs(tree):
        rlib = os.path.join(target, "release", f"lib{lib_name(crate)}.rlib")
        try:
            subprocess.run(["cargo", "build", "--release", "--lib", "--quiet"], cwd=crate, env=env, capture_output=True, text=True, check=True)
            out = subprocess.run(["size", rlib], capture_output=True, text=True, check=True).stdout
        except (OSError, subprocess.CalledProcessError, StopIteration):
            return None, None
        crate_kernels = crate_rust = 0
        for line in out.splitlines()[1:]:
            fields = line.split()
            if len(fields) >= 6 and fields[0].isdigit():
                if is_kernel_object(fields[5]):
                    crate_kernels += int(fields[0])
                else:
                    crate_rust += int(fields[0])
        if crate_kernels == 0 and lib_name(crate) == "blake3_servil_core":
            # global_asm! places kernels in `.text.blake3_servil_asm` inside the
            # Rust CGU object on ELF; sum that section directly via `size -A`.
            try:
                out_a = subprocess.run(["size", "-A", rlib], capture_output=True, text=True, check=True).stdout
                for line in out_a.splitlines():
                    parts = line.split()
                    if len(parts) >= 2 and parts[0] == ".text.blake3_servil_asm" and parts[1].isdigit():
                        crate_kernels += int(parts[1])
                crate_rust = max(0, crate_rust - crate_kernels)
            except (OSError, subprocess.CalledProcessError, ValueError):
                pass
        kernels += crate_kernels
        rust += crate_rust
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
