#!/usr/bin/env python3
"""Every `unsafe` block and `unsafe impl` in shipped Rust code says why it is sound.

    python3 tools/unsafe_reasons.py

A reason is a comment with "Sound:" in the eight lines above the block
(blocks that stand together share one). Exit status 1 names each block
without one. Covers every shipped `.rs` file across all four crates (`src/`,
`threads/src/`, `capi/src/`, `version/src/`, plus all build
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
})
REACH = 8
UNSAFE = re.compile(r"\bunsafe\s*\{|\bunsafe\s+impl\b")
DEBUG_ASSERT = re.compile(r"\bdebug_assert(?:_eq|_ne)?\s*!")
SOUND_RE = re.compile(r"Sound:\s*\S")
DEBUG_ONLY_RE = re.compile(r"Debug-only:\s*\S")
_RAW_STR_RE = re.compile(r"(?:[bc]?r)(#*)\"")
_CHAR_RE = re.compile(r"'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F_]+\}|.)|[^'\\\n])'")


def is_test_file(rel):
    return rel.replace(os.sep, "/") in TEST_ONLY_FILES


def is_shipped_path(rel):
    rel = rel.replace(os.sep, "/")
    if rel in SHIPPED_SINGLE:
        return True
    if not rel.endswith(".rs"):
        return False
    if not any(rel.startswith(p) for p in ("src/", "threads/src/", "capi/src/", "version/src/")):
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


def line_has_test_cfg(code_line):
    """True iff `code_line` is a `#[cfg(...)]` attribute line requiring `test`."""
    s = code_line.strip()
    if not s.startswith("#["):
        return False
    return any(cfg_requires_test(expr) for expr in _extract_cfg_exprs(s))


def split_code_and_comments(text):
    """Return `(code_text, comment_text)` preserving line numbers:
    - `code_text` blanks out comments and string/raw-string/char literals;
    - `comment_text` keeps only real `//` and `/* ... */` comments (strings and
      code are blanked out so string literals can never spoof a comment marker)."""
    code_out = []
    comm_out = []
    n = len(text)
    i = 0
    block_comment_depth = 0
    while i < n:
        ch = text[i]
        if block_comment_depth > 0:
            if ch == "/" and i + 1 < n and text[i + 1] == "*":
                block_comment_depth += 1
                code_out.append("  ")
                comm_out.append("/*")
                i += 2
            elif ch == "*" and i + 1 < n and text[i + 1] == "/":
                block_comment_depth -= 1
                code_out.append("  ")
                comm_out.append("*/")
                i += 2
            elif ch == "\n":
                code_out.append("\n")
                comm_out.append("\n")
                i += 1
            else:
                code_out.append(" ")
                comm_out.append(ch)
                i += 1
            continue
        if ch == "/" and i + 1 < n and text[i + 1] == "/":
            while i < n and text[i] != "\n":
                code_out.append(" ")
                comm_out.append(text[i])
                i += 1
            continue
        if ch == "/" and i + 1 < n and text[i + 1] == "*":
            block_comment_depth = 1
            code_out.append("  ")
            comm_out.append("/*")
            i += 2
            continue
        # Raw strings: r"...", r#"..."#, br#"..."#, cr#"..."#
        if ch in ("r", "b", "c") and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
            m_raw = _RAW_STR_RE.match(text, i)
        else:
            m_raw = None
        if m_raw:
            hashes = m_raw.group(1)
            prefix_len = len(m_raw.group(0))
            code_out.append(" " * prefix_len)
            comm_out.append(" " * prefix_len)
            i += prefix_len
            end_pat = '"' + hashes
            while i < n:
                if text.startswith(end_pat, i):
                    code_out.append(" " * len(end_pat))
                    comm_out.append(" " * len(end_pat))
                    i += len(end_pat)
                    break
                ch2 = "\n" if text[i] == "\n" else " "
                code_out.append(ch2)
                comm_out.append(ch2)
                i += 1
            continue
        # Regular / byte / C strings
        if ch == '"' or (ch in ("b", "c") and i + 1 < n and text[i + 1] == '"' and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_"))):
            if ch != '"':
                code_out.append(" ")
                comm_out.append(" ")
                i += 1
            code_out.append(" ")
            comm_out.append(" ")
            i += 1
            while i < n:
                if text[i] == "\\" and i + 1 < n:
                    ch2 = "\n" if text[i + 1] == "\n" else " "
                    code_out.append(" " + ch2)
                    comm_out.append(" " + ch2)
                    i += 2
                    continue
                if text[i] == '"':
                    code_out.append(" ")
                    comm_out.append(" ")
                    i += 1
                    break
                ch2 = "\n" if text[i] == "\n" else " "
                code_out.append(ch2)
                comm_out.append(ch2)
                i += 1
            continue
        # Char / byte-char literals (avoid colliding with lifetimes like `'a`)
        if ch == "'":
            m_ch = _CHAR_RE.match(text, i)
            if m_ch:
                pad = " " * len(m_ch.group(0))
                code_out.append(pad)
                comm_out.append(pad)
                i += len(m_ch.group(0))
                continue
        code_out.append(ch)
        comm_out.append("\n" if ch == "\n" else " ")
        i += 1
    return "".join(code_out), "".join(comm_out)


def strip_code_preserving_lines(text):
    return split_code_and_comments(text)[0]


def _skip_test_annotated_item(code_lines, i):
    """Given `code_lines[i]` has a `#[cfg(...)]` requiring `test`, return the
    next line index after the annotated item or statement (skipping stacked
    attributes, doc comments, and the item's braced body or `;` terminator)."""
    n = len(code_lines)
    i += 1
    # Advance past stacked outer attributes, blank lines, and comments.
    while i < n:
        s = code_lines[i].strip()
        if not s or s.startswith("#["):
            i += 1
            continue
        break
    if i >= n:
        return n
    # Track balanced braces/parens/brackets of the annotated item or statement.
    brace = 0
    paren = 0
    bracket = 0
    seen_brace = False
    while i < n:
        for ch in code_lines[i]:
            if ch == "{":
                brace += 1
                seen_brace = True
            elif ch == "}":
                brace -= 1
                if seen_brace and brace <= 0 and paren <= 0 and bracket <= 0:
                    return i + 1
            elif ch == "(":
                paren += 1
            elif ch == ")":
                paren -= 1
            elif ch == "[":
                bracket += 1
            elif ch == "]":
                bracket -= 1
            elif ch == ";" and not seen_brace and brace == 0 and paren == 0 and bracket == 0:
                return i + 1
        i += 1
    return n


def non_test_lines(text):
    """Yield `(line_number_1based, raw_line, code_line)` for every line outside
    test-only `#[cfg(...)]` modules, items, and statements, using
    string/comment-stripped token and brace counting."""
    raw_lines = text.split("\n")
    code_text, _ = split_code_and_comments(text)
    code_lines = code_text.split("\n")
    i = 0
    n = len(raw_lines)
    while i < n:
        if line_has_test_cfg(code_lines[i]):
            i = _skip_test_annotated_item(code_lines, i)
            continue
        yield i + 1, raw_lines[i], code_lines[i]
        i += 1


def without_reason(text):
    """The lines (numbers from 1, and the line) of `text`'s unsafe blocks
    that have no non-empty `Sound:` comment within REACH lines above, skipping
    test-only `#[cfg(test)]` modules/items."""
    code_text, comm_text = split_code_and_comments(text)
    comm_lines = comm_text.split("\n")
    found = []
    for lineno, raw, code in non_test_lines(text):
        if not UNSAFE.search(code):
            continue
        idx = lineno - 1
        above = comm_lines[max(0, idx - REACH):idx + 1]
        if not any(SOUND_RE.search(c) for c in above):
            found.append((lineno, raw.strip()))
    return found


def debug_assert_without_reason(text):
    """The lines (numbers from 1, and the line) of `text`'s `debug_assert*!`
    invocations that have no non-empty `// Debug-only:` comment directly
    attached above (or trailing on the same line), skipping test-only
    modules/items (QUALITY.md). Consecutive `debug_assert*!` invocations share
    one directly attached comment; any unrelated code line breaks attachment."""
    code_text, comm_text = split_code_and_comments(text)
    code_lines = code_text.split("\n")
    comm_lines = comm_text.split("\n")
    found = []
    for lineno, raw, code in non_test_lines(text):
        if not DEBUG_ASSERT.search(code):
            continue
        idx = lineno - 1
        has_reason = bool(DEBUG_ONLY_RE.search(comm_lines[idx]))
        if not has_reason:
            k = idx - 1
            while k >= 0:
                if DEBUG_ONLY_RE.search(comm_lines[k]):
                    has_reason = True
                    break
                c_strip = code_lines[k].strip()
                if not c_strip:
                    k -= 1
                    continue
                # Consecutive debug_assert*! lines share an attached comment;
                # any other code statement breaks attachment (no cross-branch borrowing).
                if DEBUG_ASSERT.search(c_strip):
                    k -= 1
                    continue
                break
        if not has_reason:
            found.append((lineno, raw.strip()))
    return found


def main():
    files = shipped_files()
    bad = 0
    bad_da = 0
    for path in files:
        with open(os.path.join(ROOT, path)) as f:
            text = f.read()
        for number, line in without_reason(text):
            bad += 1
            print(f"{path}:{number}: no reason (a `// Sound: ...` comment above): {line[:100]}")
        for number, line in debug_assert_without_reason(text):
            bad_da += 1
            print(f"{path}:{number}: no reason (a `// Debug-only: ...` comment above): {line[:100]}")
    if bad or bad_da:
        sys.exit(f"unsafe_reasons: {bad} unsafe blocks and {bad_da} debug_asserts state no reason")
    print(f"unsafe_reasons: every unsafe block and debug_assert across all {len(files)} shipped Rust files states its reason")


if __name__ == "__main__":
    main()
