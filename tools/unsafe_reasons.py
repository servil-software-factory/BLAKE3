#!/usr/bin/env python3
"""Every `unsafe` block and `unsafe impl` in shipped Rust code says why it is sound.

    python3 tools/unsafe_reasons.py

A reason is a comment with "Sound:" in the eight lines above the block
(blocks that stand together share one). Exit status 1 names each block
without one. Covers every shipped `.rs` file across all five crates (`src/`,
`threads/src/`, `b3sum/src/`, `capi/src/`, `version/src/`, plus all build
scripts and helpers: `build.rs`, `threads/build.rs`, `version/build.rs`,
`version/build_version.rs`), excluding only the explicit list of test-only
source files (`TEST_ONLY_FILES`) and inline `mod ... { ... }` blocks whose
attached `#[cfg(...)]` requires `test`.

Why: the 0.10.0 audit's memory-safety findings were in files whose unsafe
blocks stated no reason (threads/src/pipeline.rs had 79 and none), while
the files that state theirs had none (BLAKE3 #411, #417, #418, #427;
Approved Default 2).
"""

import glob
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SHIPPED_GLOBS = (
    "src/**/*.rs",
    "threads/src/**/*.rs",
    "b3sum/src/**/*.rs",
    "capi/src/**/*.rs",
    "version/src/**/*.rs",
)
SHIPPED_SINGLE = (
    "build.rs",
    "threads/build.rs",
    "version/build.rs",
    "version/build_version.rs",
)
# Explicit allowlist of source files inside `*/src/` that are compiled ONLY
# under `#[cfg(test)]`. Never exclude by filename suffix (e.g. `src/self_test.rs`
# is shipped startup code!).
TEST_ONLY_FILES = frozenset({
    "src/test.rs",
    "src/test_support.rs",
    "src/lifecycle_counts.rs",
    "src/ffi_avx512_32_guard_tests.rs",
    "threads/src/forms_test.rs",
    "b3sum/src/unit_tests.rs",
})
REACH = 8
UNSAFE = re.compile(r"\bunsafe\s*\{|\bunsafe\s+impl\b")


def is_test_file(rel):
    return rel.replace(os.sep, "/") in TEST_ONLY_FILES


def is_shipped_path(rel):
    rel = rel.replace(os.sep, "/")
    if rel in SHIPPED_SINGLE:
        return True
    if not rel.endswith(".rs"):
        return False
    if not any(rel.startswith(p) for p in ("src/", "threads/src/", "b3sum/src/", "capi/src/", "version/src/")):
        return False
    return not is_test_file(rel)


def shipped_files():
    found = set()
    for pat in SHIPPED_GLOBS:
        for full in glob.glob(os.path.join(ROOT, pat), recursive=True):
            rel = os.path.relpath(full, ROOT).replace(os.sep, "/")
            if is_shipped_path(rel):
                found.add(rel)
    for rel in SHIPPED_SINGLE:
        if os.path.isfile(os.path.join(ROOT, rel)):
            found.add(rel)
    return sorted(found)


def _split_top_level(expr):
    parts, buf, depth = [], [], 0
    for ch in expr:
        if ch == "(":
            depth += 1
            buf.append(ch)
        elif ch == ")":
            depth -= 1
            buf.append(ch)
        elif ch == "," and depth == 0:
            parts.append("".join(buf).strip())
            buf = []
        else:
            buf.append(ch)
    if buf:
        parts.append("".join(buf).strip())
    return parts


def cfg_requires_test(expr):
    """True iff a `cfg(...)` predicate `expr` can only hold when `test` is set.
    Accepts `test` and `all(..., test, ...)`; rejects `not(test)` and `any(test, ...)`."""
    s = expr.strip()
    if s == "test":
        return True
    if s.startswith("all(") and s.endswith(")"):
        return any(cfg_requires_test(part) for part in _split_top_level(s[4:-1]))
    return False


def _extract_cfg_exprs(attr_line):
    exprs = []
    idx = 0
    while True:
        pos = attr_line.find("cfg(", idx)
        if pos < 0:
            break
        if pos > 0 and (attr_line[pos - 1].isalnum() or attr_line[pos - 1] == "_"):
            idx = pos + 4
            continue
        depth = 0
        start = pos + 4
        j = start
        while j < len(attr_line):
            if attr_line[j] == "(":
                depth += 1
            elif attr_line[j] == ")":
                if depth == 0:
                    exprs.append(attr_line[start:j])
                    break
                depth -= 1
            j += 1
        idx = j + 1
    return exprs


def is_test_mod_header(lines, i):
    """True iff `lines[i]` opens an inline `mod <name> {` whose directly
    attached attributes include a `#[cfg(...)]` requiring `test`."""
    if not re.match(r"\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{", lines[i]):
        return False
    # Walk immediately attached attributes / comments / blank lines above `lines[i]`.
    k = i - 1
    while k >= 0:
        s = lines[k].strip()
        if not s or s.startswith("//"):
            k -= 1
            continue
        if s.startswith("#["):
            for expr in _extract_cfg_exprs(s):
                if cfg_requires_test(expr):
                    return True
            k -= 1
            continue
        break
    return False


def strip_code_preserving_lines(text):
    """Return a parallel string with comments and string/raw-string/char
    literals blanked out (preserving newlines), so token & brace scanning
    never matches text inside strings or comments."""
    out = []
    n = len(text)
    i = 0
    block_comment_depth = 0
    while i < n:
        ch = text[i]
        if block_comment_depth > 0:
            if ch == "/" and i + 1 < n and text[i + 1] == "*":
                block_comment_depth += 1
                out.append("  ")
                i += 2
            elif ch == "*" and i + 1 < n and text[i + 1] == "/":
                block_comment_depth -= 1
                out.append("  ")
                i += 2
            elif ch == "\n":
                out.append("\n")
                i += 1
            else:
                out.append(" ")
                i += 1
            continue
        if ch == "/" and i + 1 < n and text[i + 1] == "/":
            while i < n and text[i] != "\n":
                out.append(" ")
                i += 1
            continue
        if ch == "/" and i + 1 < n and text[i + 1] == "*":
            block_comment_depth = 1
            out.append("  ")
            i += 2
            continue
        # Raw strings: r"...", r#"..."#, br#"..."#, cr#"..."#
        m_raw = re.match(r"(?:[bc]?r)(#*)\"", text[i:])
        if m_raw and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
            hashes = m_raw.group(1)
            prefix_len = len(m_raw.group(0))
            out.append(" " * prefix_len)
            i += prefix_len
            end_pat = '"' + hashes
            while i < n:
                if text.startswith(end_pat, i):
                    out.append(" " * len(end_pat))
                    i += len(end_pat)
                    break
                out.append("\n" if text[i] == "\n" else " ")
                i += 1
            continue
        # Regular / byte / C strings
        if ch == '"' or (ch in ("b", "c") and i + 1 < n and text[i + 1] == '"' and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_"))):
            if ch != '"':
                out.append(" ")
                i += 1
            out.append(" ")
            i += 1
            while i < n:
                if text[i] == "\\" and i + 1 < n:
                    out.append(" ")
                    out.append("\n" if text[i + 1] == "\n" else " ")
                    i += 2
                    continue
                if text[i] == '"':
                    out.append(" ")
                    i += 1
                    break
                out.append("\n" if text[i] == "\n" else " ")
                i += 1
            continue
        # Char / byte-char literals (avoid colliding with lifetimes like `'a`)
        if ch == "'":
            m_ch = re.match(r"'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F_]+\}|.)|[^'\\\n])'", text[i:])
            if m_ch:
                out.append(" " * len(m_ch.group(0)))
                i += len(m_ch.group(0))
                continue
        out.append(ch)
        i += 1
    return "".join(out)


def non_test_lines(text):
    """Yield `(line_number_1based, raw_line, code_line)` for every line outside
    test-only inline `mod ... { ... }` blocks, using string/comment-stripped
    brace counting."""
    raw_lines = text.split("\n")
    code_lines = strip_code_preserving_lines(text).split("\n")
    i = 0
    n = len(raw_lines)
    while i < n:
        if is_test_mod_header(raw_lines, i):
            depth = 0
            while i < n:
                depth += code_lines[i].count("{") - code_lines[i].count("}")
                i += 1
                if depth <= 0:
                    break
            continue
        yield i + 1, raw_lines[i], code_lines[i]
        i += 1


def without_reason(text):
    """The lines (numbers from 1, and the line) of `text`'s unsafe blocks
    that have no "Sound:" comment within REACH lines above, skipping
    test-only `#[cfg(...)] mod ... { ... }` modules."""
    raw_lines = text.split("\n")
    found = []
    for lineno, raw, code in non_test_lines(text):
        if not UNSAFE.search(code):
            continue
        idx = lineno - 1
        above = raw_lines[max(0, idx - REACH):idx + 1]
        if not any("Sound:" in a and a.lstrip().startswith("//") for a in above[:-1]) and "// Sound:" not in raw:
            found.append((lineno, raw.strip()))
    return found


def main():
    files = shipped_files()
    bad = 0
    for path in files:
        with open(os.path.join(ROOT, path)) as f:
            for number, line in without_reason(f.read()):
                bad += 1
                print(f"{path}:{number}: no reason (a `// Sound: ...` comment above): {line[:100]}")
    if bad:
        sys.exit(f"unsafe_reasons: {bad} unsafe blocks state no reason")
    print(f"unsafe_reasons: every unsafe block across all {len(files)} shipped Rust files states its reason")


if __name__ == "__main__":
    main()
