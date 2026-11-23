#!/usr/bin/env python3
"""Non-rising safety counts for shipped Rust code (Approved Default 9).

    python3 tools/counts.py                   print current counts for the working tree
    python3 tools/counts.py REV               print counts at git revision REV
    python3 tools/counts.py --check BASE [REV]
                                              fail if any ratcheted count at REV
                                              (default working tree) exceeds BASE

Ratcheted counts (must not rise above BASE; comments, strings, and test-only
modules/files are excluded via `unsafe_reasons.non_test_lines`):
  undocumented_unsafe   `unsafe` blocks / `unsafe impl` without `// Sound:`
  b3sum_print_macros    `print!` / `println!` / `eprint!` / `eprintln!` in `b3sum/src/`
  truncation_allows     `#[allow(...clippy::cast_possible_truncation...)]` attributes
  debug_asserts         `debug_assert*!` invocations in shipped code

Also reported alongside for visibility (not failed here):
  shipped_code_lines    non-blank non-comment code lines in shipped Rust files
  cfg_conditions        `cfg(` / `cfg_attr(` occurrences in shipped Rust code
"""

import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import unsafe_reasons as ur

PRINT_RE = re.compile(r"\b(?:e?println|e?print)!")
TRUNC_ALLOW_RE = re.compile(r"#!?\[[^\]]*\ballow\s*\([^)]*\bclippy::cast_possible_truncation\b")
DEBUG_ASSERT_RE = re.compile(r"\bdebug_assert(?:_eq|_ne)?!")
CFG_RE = re.compile(r"\bcfg(?:_attr)?!?\s*\(")

RATCHETED = ("undocumented_unsafe", "b3sum_print_macros", "truncation_allows", "debug_asserts")


def read_tree(rev=None):
    files = {}
    if rev is None:
        for rel in ur.shipped_files():
            with open(os.path.join(ROOT, rel)) as f:
                files[rel] = f.read()
        return files
    out = subprocess.run(["git", "-C", ROOT, "ls-tree", "-r", "--name-only", rev], capture_output=True, text=True, check=True).stdout
    for rel in sorted(out.splitlines()):
        if ur.is_shipped_path(rel):
            files[rel] = subprocess.run(["git", "-C", ROOT, "show", f"{rev}:{rel}"], capture_output=True, text=True, check=True).stdout
    return files


def measure_files(files):
    counts = {
        "undocumented_unsafe": 0,
        "b3sum_print_macros": 0,
        "truncation_allows": 0,
        "debug_asserts": 0,
        "shipped_code_lines": 0,
        "cfg_conditions": 0,
    }
    for rel, text in files.items():
        counts["undocumented_unsafe"] += len(ur.without_reason(text))
        code_lines = []
        for _lineno, _raw, code in ur.non_test_lines(text):
            code_lines.append(code)
            if code.strip():
                counts["shipped_code_lines"] += 1
        body = "\n".join(code_lines)
        if rel.startswith("b3sum/src/"):
            counts["b3sum_print_macros"] += len(PRINT_RE.findall(body))
        counts["truncation_allows"] += len(TRUNC_ALLOW_RE.findall(body))
        counts["debug_asserts"] += len(DEBUG_ASSERT_RE.findall(body))
        counts["cfg_conditions"] += len(CFG_RE.findall(body))
    return counts


def measure(rev=None):
    return measure_files(read_tree(rev))


def fmt(counts):
    return ", ".join(f"{k}={v}" for k, v in counts.items())


def check(base, rev=None):
    base_c = measure(base)
    rev_c = measure(rev)
    rose = [f"{k}: {base_c[k]} -> {rev_c[k]}" for k in RATCHETED if rev_c[k] > base_c[k]]
    return base_c, rev_c, rose


def main(argv):
    if len(argv) >= 2 and argv[0] == "--check":
        base = argv[1]
        rev = argv[2] if len(argv) >= 3 else None
        _base_c, rev_c, rose = check(base, rev)
        label = rev[:12] if rev else "worktree"
        if rose:
            sys.exit(f"counts: rose above {base} at {label}: " + "; ".join(rose))
        print(f"counts: non-rising vs {base} ({fmt(rev_c)})")
        return
    rev = argv[0] if argv else None
    print(f"counts: {fmt(measure(rev))}")


if __name__ == "__main__":
    main(sys.argv[1:])
