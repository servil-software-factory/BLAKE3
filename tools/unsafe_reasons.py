#!/usr/bin/env python3
"""Every `unsafe` block and `unsafe impl` in shipped Rust code says why it is sound.

    python3 tools/unsafe_reasons.py

A reason is a comment with "Sound:" in the eight lines above the block
(blocks that stand together share one). Exit status 1 names each block
without one. Covers every shipped `.rs` file across all four crates (`src/`,
`threads/src/`, `capi/src/`, `version/src/`, plus all build
scripts and helpers: `build.rs`, `threads/build.rs` (if present),
`version/build.rs`, and `version/build_version.rs`), excluding only the explicit list of test-only
source files (`TEST_ONLY_FILES`) and inline `mod ... { ... }` blocks whose
attached `#[cfg(...)]` requires `test`.

Why: the 0.10.0 audit's memory-safety findings were in files whose unsafe
blocks stated no reason (threads/src/pipeline.rs had 79 and none), while
the files that state theirs had none (BLAKE3 #411, #417, #418, #427;
Approved Default 2).
"""

import bisect
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
SOUND_RE = re.compile(r"Sound:(?:(?!/\*|\*/)[\W_])*[^\W_]")
DEBUG_ONLY_RE = re.compile(r"Debug-only:(?:(?!/\*|\*/)[\W_])*[^\W_]")
_OUTER_CFG_RE = re.compile(r"^#\[\s*cfg\s*\(")
_CFG_CALL_RE = re.compile(r"\bcfg\s*\(")
_PUB_PREFIX_RE = re.compile(r"pub(?:\s*\([^)]*\))?\s+")
_ITEM_OR_STMT_KW_RE = re.compile(r"(?:fn|impl|struct|enum|trait|type|use|static|const|let|mod|unsafe|async|extern|if|match|for|while|loop)\b")
_ELSE_RE = re.compile(r"\s*else\b")
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
        m = _CFG_CALL_RE.search(attr_line, idx)
        if not m:
            break
        depth = 0
        start = m.end()
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


def _attr_requires_test(attr_text):
    """True iff outer attribute `attr_text` (`#[...]`) is a `#[cfg(...)]`
    whose predicate requires `test`. Rejects `#[cfg_attr(..., cfg(test))]`
    because a `cfg_attr` target is shipped whenever its condition is false."""
    if not _OUTER_CFG_RE.match(attr_text):
        return False
    return any(cfg_requires_test(expr) for expr in _extract_cfg_exprs(attr_text))


def line_has_test_cfg(code_line):
    """True iff `code_line` begins with an outer `#[cfg(...)]` requiring `test`."""
    return _attr_requires_test(code_line.strip())


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


def _iter_comment_tokens(comm_text):
    """Yield `(start, end, body)` for every `//` line comment and nested
    `/* ... */` block comment in `comm_text` (already stripped of non-comments)."""
    n = len(comm_text)
    i = 0
    while i < n:
        if comm_text[i] == "/" and i + 1 < n and comm_text[i + 1] == "/":
            start = i
            nl = comm_text.find("\n", i + 2)
            end = n if nl == -1 else nl
            yield start, end, comm_text[start + 2:end]
            i = end
        elif comm_text[i] == "/" and i + 1 < n and comm_text[i + 1] == "*":
            start = i
            depth = 1
            i += 2
            while i < n and depth > 0:
                if comm_text[i] == "/" and i + 1 < n and comm_text[i + 1] == "*":
                    depth += 1
                    i += 2
                elif comm_text[i] == "*" and i + 1 < n and comm_text[i + 1] == "/":
                    depth -= 1
                    i += 2
                else:
                    i += 1
            end = i
            body = comm_text[start + 2:end - 2] if depth == 0 else comm_text[start + 2:end]
            yield start, end, body
        else:
            i += 1


def _comment_has_reason(comm_slice, pattern):
    """True iff at least one complete comment token in `comm_slice` matches
    `pattern` after stripping its delimiters."""
    return any(pattern.search(body) for _s, _e, body in _iter_comment_tokens(comm_slice))


def _lines_with_reason(comm_text, starts, pattern):
    """Return the set of 0-based line numbers spanned by any complete comment
    token in `comm_text` whose delimiter-stripped body matches `pattern`
    (supports both `//` and multiline nested `/* ... */` rationales)."""
    covered = set()
    for start, end, body in _iter_comment_tokens(comm_text):
        if not pattern.search(body):
            continue
        s_ln = bisect.bisect_right(starts, start) - 1
        e_ln = bisect.bisect_right(starts, max(start, end - 1)) - 1
        for ln in range(s_ln, e_ln + 1):
            covered.add(ln)
    return covered


def _line_starts(text):
    starts = [0]
    pos = 0
    while True:
        nl = text.find("\n", pos)
        if nl == -1:
            break
        starts.append(nl + 1)
        pos = nl + 1
    return starts


def _find_attr_end(code, start):
    """Given `code[start:start+2] == '#['`, return index immediately after its
    matching `]`, or `len(code)` if unterminated."""
    n = len(code)
    j = start + 1
    depth = 0
    while j < n:
        ch = code[j]
        if ch == "[":
            depth += 1
        elif ch == "]":
            depth -= 1
            if depth == 0:
                return j + 1
        j += 1
    return n


def _preceding_attr_stack_start(code, pos):
    """Walk backward from outer attribute at `pos` across any immediately
    preceding outer `#[...]` attributes (separated only by whitespace) so
    stacked attributes such as `#[allow(...)] #[cfg(test)] fn f()` are masked
    together."""
    cur = pos
    while True:
        k = cur - 1
        while k >= 0 and code[k].isspace():
            k -= 1
        if k < 0 or code[k] != "]":
            return cur
        depth = 0
        j = k
        while j >= 0:
            if code[j] == "]":
                depth += 1
            elif code[j] == "[":
                depth -= 1
                if depth == 0:
                    break
            j -= 1
        if j >= 1 and code[j - 1] == "#" and (j < 2 or code[j - 2] != "!"):
            cur = j - 1
            continue
        return cur


_GENERIC_DECL_KW = frozenset({"fn", "struct", "enum", "trait", "type", "union"})
_GENERIC_DIRECT_KW = frozenset({"impl", "for"})


def _prev_ident_token(code, pos):
    """Return `(word, start_idx)` for the identifier/keyword immediately before
    `pos` (skipping whitespace), or `("", pos)` if preceded by punctuation."""
    end = pos
    while end > 0 and code[end - 1].isspace():
        end -= 1
    start = end
    while start > 0 and (code[start - 1].isalnum() or code[start - 1] == "_"):
        start -= 1
    return code[start:end], start


def _in_generic_param_list(code, pos):
    """Return True iff `pos` (in blanked code) sits directly inside a syntactic
    generic parameter list `<...>` (following `impl`/`for` or `fn`/`struct`/
    `enum`/`trait`/`type`/`union` `<ident>`)."""
    p = b = c = a = 0
    k = pos - 1
    while k >= 0:
        ch = code[k]
        if ch == ")":
            p += 1
        elif ch == "(":
            if p == 0:
                return False
            p -= 1
        elif ch == "]":
            b += 1
        elif ch == "[":
            if b == 0:
                return False
            b -= 1
        elif ch == "}":
            c += 1
        elif ch == "{":
            if c == 0:
                return False
            c -= 1
        elif p == 0 and b == 0 and c == 0:
            if ch == ";":
                return False
            if ch == ">":
                prev = code[k - 1] if k > 0 else ""
                if prev not in ("-", "="):
                    a += 1
            elif ch == "<":
                prev = code[k - 1] if k > 0 else ""
                nxt = code[k + 1] if k + 1 < len(code) else ""
                if nxt != "=":
                    if a > 0:
                        a -= 1
                    elif prev != "<" and nxt != "<":
                        tok1, t1_start = _prev_ident_token(code, k)
                        if tok1 in _GENERIC_DIRECT_KW:
                            return True
                        if tok1 and (tok1[0].isalpha() or tok1[0] == "_"):
                            tok2, _ = _prev_ident_token(code, t1_start)
                            return tok2 in _GENERIC_DECL_KW
                        return False
        k -= 1
    return False


def _is_rust_ws(ch):
    # Rust Pattern_White_Space includes Unicode whitespace plus U+200E / U+200F.
    return ch.isspace() or ch in ("\u200e", "\u200f")


def _test_item_spans(code):
    """Return sorted `(start, end)` spans of every outer `#[cfg(...)]` requiring
    `test` together with its stacked attributes and attached item, block,
    statement, field, or parameter in comment/string-stripped `code`."""
    n = len(code)
    spans = []
    i = 0
    while i < n:
        pos = code.find("#[", i)
        if pos == -1:
            break
        attr_end = _find_attr_end(code, pos)
        attr_text = code[pos:attr_end]
        if not _attr_requires_test(attr_text):
            i = attr_end
            continue
        span_start = _preceding_attr_stack_start(code, pos)
        # Advance past Rust whitespace and any stacked outer `#[...]` attributes.
        j = attr_end
        while j < n:
            while j < n and _is_rust_ws(code[j]):
                j += 1
            if j + 1 < n and code[j] == "#" and code[j + 1] == "[":
                j = _find_attr_end(code, j)
                continue
            break
        # Strip optional `pub` / `pub(...)` before checking whether the
        # attributed construct starts with an item/statement keyword (so
        # `#[cfg(test)] pub field: u32,` stops at `,`, whereas
        # `#[cfg(test)] pub fn f<A, B>() {}` stops at `}`).
        in_generic = _in_generic_param_list(code, span_start)
        kw_pos = j
        m_pub = _PUB_PREFIX_RE.match(code, kw_pos)
        if m_pub:
            kw_pos = m_pub.end()
        m_kw = _ITEM_OR_STMT_KW_RE.match(code, kw_pos)
        kw = m_kw.group(0) if m_kw else ""
        starts_item_kw = (not in_generic) and bool(m_kw)
        is_const_block = False
        if kw == "const":
            rest_pos = m_kw.end()
            while rest_pos < n and _is_rust_ws(code[rest_pos]):
                rest_pos += 1
            # Any `const` item (`const NAME:`, `const r#match:`, `const Δ:`,
            # `const _:`) ends at `;`; `const fn` / `const unsafe` / `const
            # extern` are brace-terminated items; inline `const { ... }` is an
            # expression (so inside arrays/tuples/calls it stops at top-level `,`).
            is_const_block = rest_pos < n and code[rest_pos] == "{"
            if is_const_block:
                starts_item_kw = False
            is_const_fn = bool(re.match(r"(?:unsafe|extern|fn)\b", code[rest_pos:]))
            must_end_semi = starts_item_kw and not is_const_block and not is_const_fn
        else:
            must_end_semi = starts_item_kw and kw in ("let", "static", "type", "use")
        # Consume the annotated item/statement: track balanced delimiters and
        # only mark `seen_top_brace` for a `{` at outer level (`paren == 0`
        # and `bracket == 0`) when the construct is brace-terminated (not a
        # semicolon-terminated `let`/`const`/`static`/`type`/`use`). Never cross
        # an enclosing `)`, `]`, `}`, or (for generic params) `>`.
        brace = 0
        paren = 0
        bracket = 0
        angle = 0
        just_closed_angle = False
        seen_top_brace = False
        seen_top_eq = False
        end = n
        while j < n:
            ch = code[j]
            if ch != ">":
                just_closed_angle = False
            if ch == "(":
                paren += 1
            elif ch == ")":
                if paren == 0:
                    end = j
                    break
                paren -= 1
            elif ch == "[":
                bracket += 1
            elif ch == "]":
                if bracket == 0:
                    end = j
                    break
                bracket -= 1
            elif ch == "=" and brace == 0 and paren == 0 and bracket == 0 and angle == 0:
                prev = code[j - 1] if j > 0 else ""
                nxt = code[j + 1] if j + 1 < n else ""
                if prev not in ("=", "!", "<", ">") and nxt not in ("=", ">"):
                    seen_top_eq = True
            elif ch == "<" and not starts_item_kw and not seen_top_brace and brace == 0 and paren == 0 and bracket == 0:
                prev = code[j - 1] if j > 0 else ""
                nxt = code[j + 1] if j + 1 < n else ""
                if in_generic and not seen_top_eq:
                    if nxt != "=":
                        angle += 1
                elif prev != "<" and nxt not in ("<", "="):
                    angle += 1
            elif ch == ">" and not starts_item_kw and not seen_top_brace and brace == 0 and paren == 0 and bracket == 0:
                prev = code[j - 1] if j > 0 else ""
                nxt = code[j + 1] if j + 1 < n else ""
                if angle > 0:
                    # Inside `<...>` generics, every `>` (including `>>` in
                    # `Vec<Vec<u8>>`) closes one nesting level unless `->`/`=>`.
                    if prev not in ("-", "="):
                        angle -= 1
                        just_closed_angle = True
                elif prev not in ("-", "=") and (in_generic or just_closed_angle or (prev != ">" and nxt not in (">", "="))):
                    # Stop before the closing `>` of an enclosing generic list
                    # (including `const N: usize>=[...]` and `T: Into<Vec<u8>>>`).
                    end = j
                    break
            elif ch == "{":
                if not must_end_semi and brace == 0 and paren == 0 and bracket == 0:
                    seen_top_brace = True
                brace += 1
            elif ch == "}":
                if not seen_top_brace and brace == 0 and paren == 0 and bracket == 0:
                    end = j
                    break
                if brace > 0:
                    brace -= 1
                if seen_top_brace and brace == 0 and paren == 0 and bracket == 0:
                    j += 1
                    m_else = _ELSE_RE.match(code, j)
                    if m_else:
                        seen_top_brace = False
                        j = m_else.end()
                        continue
                    # For inline `const { ... }` block expressions, keep
                    # scanning if followed by a postfix continuation (`.foo`,
                    # `?`, NOT a `..` range expression starting the next
                    # statement) or consume a trailing `;` or `,` (even on the
                    # next line) so its trailing comment is masked too.
                    k = j
                    while k < n and _is_rust_ws(code[k]):
                        k += 1
                    if is_const_block and k < n and (
                        code[k] == "?"
                        or (code[k] == "." and not code.startswith("..", k))
                    ):
                        must_end_semi = True
                        seen_top_brace = False
                        j = k + 1
                        continue
                    if k < n and (code[k] == ";" or (is_const_block and code[k] == ",")):
                        end = k + 1
                    else:
                        end = j
                    break
            elif ch == ";" and not seen_top_brace and brace == 0 and paren == 0 and bracket == 0:
                end = j + 1
                break
            elif ch == "," and not starts_item_kw and not seen_top_brace and brace == 0 and paren == 0 and bracket == 0 and angle == 0:
                end = j + 1
                break
            j += 1
        spans.append((span_start, end))
        i = end
    return spans


def _blank_spans(text, spans):
    if not spans:
        return text
    out = list(text)
    for start, end in spans:
        for k in range(start, end):
            if out[k] != "\n":
                out[k] = " "
    return "".join(out)


def mask_test_items(code):
    """Mask every outer `#[cfg(...)]` requiring `test` together with its exact
    attached item, block, statement, field, or parameter in comment/string-
    stripped `code`, replacing masked characters with spaces while preserving
    every `\\n`."""
    return _blank_spans(code, _test_item_spans(code))


def _extend_trailing_comment_spans(code_text, comm_text, spans):
    """Extend each masked `#[cfg(test)]` span across any immediately following
    trailing comment tokens on its ending line (including nested or multi-line
    `/* ... */` tokens starting on that line), bridging ONLY horizontal
    whitespace (`' '`, `'\\t'`) and stopping at newlines or code. Prevents
    trailing comments on masked test statements from donating rationales to
    later shipped lines without ever touching a next-line shipped comment."""
    n = len(code_text)
    comment_by_start = {s: e for s, e, _ in _iter_comment_tokens(comm_text)}
    out = []
    for start, end in spans:
        c_end = end
        j = end
        while j < n:
            while j < n and code_text[j] in (" ", "\t", "\u200e", "\u200f") and comm_text[j] == " ":
                j += 1
            if j in comment_by_start:
                tok_end = comment_by_start[j]
                c_end = tok_end
                if "\n" in comm_text[j:tok_end]:
                    break
                j = tok_end
            else:
                break
        out.append((start, c_end))
    return out


def _masked_code_and_spans(text):
    code_text, comm_text = split_code_and_comments(text)
    spans = _test_item_spans(code_text)
    comm_spans = _extend_trailing_comment_spans(code_text, comm_text, spans)
    return _blank_spans(code_text, spans), _blank_spans(comm_text, comm_spans), spans


def _span_lines(spans, starts):
    lines = set()
    for s, e in spans:
        if e <= s:
            continue
        s_ln = bisect.bisect_right(starts, s) - 1
        e_ln = bisect.bisect_right(starts, e - 1) - 1
        for ln in range(s_ln, e_ln + 1):
            lines.add(ln)
    return lines


def masked_code(text):
    """Return `(masked_code_text, masked_comment_text)` with comments/strings
    stripped from `masked_code_text` and all `#[cfg(...)]` test-only spans
    blanked out in BOTH code and comments (preserving every `\\n`) so comments
    inside `#[cfg(test)]` blocks can never satisfy shipped-code rationales."""
    masked, comm_text, _spans = _masked_code_and_spans(text)
    return masked, comm_text


def non_test_lines(text):
    """Yield `(line_number_1based, raw_line, code_line)` for every line of
    `text`, with test-only `#[cfg(...)]` modules, items, and statements masked
    out of `code_line` at exact token boundaries."""
    raw_lines = text.split("\n")
    masked_lines = masked_code(text)[0].split("\n")
    for idx, (raw, code) in enumerate(zip(raw_lines, masked_lines), 1):
        yield idx, raw, code


def _macro_call_end(code, bang_end):
    """Given index right after `!` of a macro call, return index right after
    its balanced delimiter group (and optional trailing `;`)."""
    n = len(code)
    j = bang_end
    while j < n and code[j].isspace():
        j += 1
    if j >= n or code[j] not in "([{":
        return bang_end
    open_ch = code[j]
    close_ch = {"(": ")", "[": "]", "{": "}"}[open_ch]
    depth = 0
    while j < n:
        ch = code[j]
        if ch == open_ch:
            depth += 1
        elif ch == close_ch:
            depth -= 1
            if depth == 0:
                j += 1
                break
        j += 1
    k = j
    while k < n and code[k] in " \t":
        k += 1
    if k < n and code[k] == ";":
        return k + 1
    return j


_ATTR_PREFIX = r"(?:#\[[^\]]*\]\s*)*"
_ITEM_QUAL = r"(?:pub(?:\s*\([^)]*\))?\s+)?(?:(?:default|const|async|unsafe)\s+|extern\s+(?:\"[^\"]*\"\s+)?)*"
_ITEM_BOUNDARY_RE = re.compile(
    rf"^\s*{_ATTR_PREFIX}{_ITEM_QUAL}(?:(?:fn|impl|mod|struct|enum|trait|type|static)\b|const\s+[A-Za-z_]\w*\s*:)"
)
_UNSAFE_IMPL_FOR_RE = re.compile(
    rf"^\s*{_ATTR_PREFIX}unsafe\s+impl\b.*?\bfor\s+([A-Za-z_][A-Za-z0-9_:<>,\s'\"]*?)(?:\s+where\b|\s*\{{|$)"
)


def without_reason(text):
    """The lines (numbers from 1, and the line) of `text`'s unsafe blocks
    that have no non-empty `Sound:` comment within REACH lines above (stopping
    at function/item boundaries so a rationale in one function cannot clear an
    unrelated unsafe site in the next), skipping test-only `#[cfg(test)]`
    modules/items."""
    raw_lines = text.split("\n")
    masked, comm_text = masked_code(text)
    masked_lines = masked.split("\n")
    starts = _line_starts(masked)
    sound_lines = _lines_with_reason(comm_text, starts, SOUND_RE)
    found = []
    seen_lines = set()
    for m in UNSAFE.finditer(masked):
        idx = bisect.bisect_right(starts, m.start()) - 1
        if idx in seen_lines:
            continue
        seen_lines.add(idx)
        cur_line = masked_lines[idx]
        cur_impl = _UNSAFE_IMPL_FOR_RE.match(cur_line)
        cur_opens_non_impl_item = bool(_ITEM_BOUNDARY_RE.match(cur_line)) and not cur_impl
        ok = False
        lo = max(0, idx - REACH)
        for ln in range(idx, lo - 1, -1):
            if ln in sound_lines:
                ok = True
                break
            ml = masked_lines[ln]
            if ln == idx:
                # If the unsafe site sits directly on a one-line fn/const/etc.
                # item line (e.g. `fn g() { unsafe { *q } }`), only a `Sound:`
                # comment immediately above that item (skipping blank lines and
                # attributes only) may clear it — never cross preceding code.
                if cur_opens_non_impl_item:
                    for above in range(idx - 1, lo - 1, -1):
                        if above in sound_lines:
                            ok = True
                            break
                        aml = masked_lines[above].strip()
                        if aml and not aml.startswith("#["):
                            break
                    break
                continue
            if ml.startswith("}"):
                break
            if _ITEM_BOUNDARY_RE.match(ml):
                # Adjacent `unsafe impl Send` / `unsafe impl Sync` lines for the
                # SAME target type share one `// Sound:` comment right above;
                # every other item/function boundary stops rationale attachment.
                if cur_impl:
                    prev_impl = _UNSAFE_IMPL_FOR_RE.match(ml)
                    if prev_impl and prev_impl.group(1).strip() == cur_impl.group(1).strip():
                        continue
                break
        if not ok:
            found.append((idx + 1, raw_lines[idx].strip()))
    return found


def debug_assert_without_reason(text):
    """The lines (numbers from 1, and the line) of `text`'s `debug_assert*!`
    invocations that have no non-empty `// Debug-only:` comment directly
    attached above (or inline/trailing on that invocation alone), skipping
    test-only modules/items (QUALITY.md). Consecutive standalone
    `debug_assert*!` statements share one directly attached comment above;
    any other code or branch boundary breaks attachment."""
    raw_lines = text.split("\n")
    masked, comm_text, test_spans = _masked_code_and_spans(text)
    masked_lines = masked.split("\n")
    starts = _line_starts(masked)
    masked_test_lines = _span_lines(test_spans, starts)
    debug_lines = _lines_with_reason(comm_text, starts, DEBUG_ONLY_RE)
    matches = list(DEBUG_ASSERT.finditer(masked))
    # Track lines occupied by standalone `debug_assert*!` statements so
    # consecutive multi-line assertions can share one header comment above.
    da_stmt_lines = set()
    spans = []
    for m in matches:
        s_idx = bisect.bisect_right(starts, m.start()) - 1
        call_end = _macro_call_end(masked, m.end())
        e_idx = bisect.bisect_right(starts, max(m.start(), call_end - 1)) - 1
        line_start = starts[s_idx]
        line_end = starts[e_idx + 1] - 1 if e_idx + 1 < len(starts) else len(masked)
        prefix_on_line = masked[line_start:m.start()].strip()
        suffix_on_line = masked[call_end:line_end].strip()
        standalone = (prefix_on_line == "" and suffix_on_line == "")
        if standalone:
            for ln in range(s_idx, e_idx + 1):
                da_stmt_lines.add(ln)
        spans.append((m, s_idx, e_idx, call_end, line_start, line_end, standalone))

    found = []
    for m, s_idx, e_idx, call_end, line_start, line_end, standalone in spans:
        # 1. Check inline/trailing comment belonging strictly to THIS invocation:
        # bounded on the left by any `;`, `{`, or `}` before `m.start()` on its line,
        # and on the right by the next `;`, `}`, or macro call after `call_end` on its end line.
        left_bound = line_start
        for sep in (";", "{", "}"):
            p = masked.rfind(sep, line_start, m.start())
            if p != -1:
                left_bound = max(left_bound, p + 1)
        same_line_masked = False
        for ts_start, ts_end in test_spans:
            if ts_start < m.start() and ts_end > line_start:
                same_line_masked = True
                if ts_end <= m.start():
                    left_bound = max(left_bound, ts_end)
        right_bound = call_end
        if not masked[call_end:line_end].strip():
            # Nothing follows this invocation on `e_idx`, unless a masked
            # `#[cfg(test)]` statement sits between `call_end` and `line_end`.
            right_bound = line_end
            for ts_start, ts_end in test_spans:
                if ts_start >= call_end and ts_start < line_end and ts_end > ts_start:
                    right_bound = min(right_bound, ts_start)
        has_reason = _comment_has_reason(comm_text[left_bound:right_bound], DEBUG_ONLY_RE)

        # 2. Check directly attached comment lines above ONLY if no other code
        # (including a masked `#[cfg(test)]` statement) precedes this invocation on `s_idx`.
        if not has_reason and not same_line_masked and masked[line_start:m.start()].strip() == "":
            k = s_idx - 1
            while k >= 0:
                if k in masked_test_lines:
                    break
                c_strip = masked_lines[k].strip()
                if c_strip and k not in da_stmt_lines:
                    break
                if k in debug_lines:
                    has_reason = True
                    break
                k -= 1
        if not has_reason:
            found.append((s_idx + 1, raw_lines[s_idx].strip()))
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
    print(f"unsafe_reasons: all {len(files)} shipped Rust files pass Sound:/Debug-only: rationale checks")


if __name__ == "__main__":
    main()
