#!/usr/bin/env python3
"""Enforce single-frame release stack budgets via ELF `.stack_sizes`.

Builds `b3sum` (which links `blake3_servil_core`, `blake3_servil`, and `b3sums`)
with `RUSTC_BOOTSTRAP=1 RUSTFLAGS="-Z emit-stack-sizes"` into a dedicated target
subdirectory so it never invalidates the main Cargo cache, parses
`llvm-readobj --stack-sizes`, demangles every symbol via `llvm-cxxfilt` (or
`c++filt`), and enforces the caps approved for #499:

- `blake3_servil_core` / `b3sums`: <= 128 KiB (131,072 bytes)
- `blake3_servil_core` `Hasher` non-update methods: <= 32 KiB (32,768 bytes)
- `blake3_servil` (`threads/`): <= 16 KiB (16,384 bytes)
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

CAP_CORE_B3SUM = 128 * 1024  # 131,072 B
CAP_HASHER = 32 * 1024       #  32,768 B
CAP_THREADS = 16 * 1024      #  16,384 B

SHIPPED_CRATES = frozenset({"blake3_servil_core", "blake3_servil", "b3sum", "b3sums"})
EXPECTED_BUCKETS = frozenset({"b3sum", "core", "core::Hasher", "threads"})

# Match Entry blocks and anchored Size/Functions lines inside `llvm-readobj --stack-sizes`
# so headers like `AddressSize: 64bit` outside `Entry { ... }` are never parsed as stack sizes.
_ENTRY_OPEN_RE = re.compile(r"^\s*Entry\s*\{\s*$")
_ENTRY_CLOSE_RE = re.compile(r"^\s*\}\s*$")
_SIZE_RE = re.compile(r"^\s*Size:\s*(0x[0-9a-fA-F]+|\d+)\s*$")
_FUNCS_RE = re.compile(r"^\s*Functions:\s*\[([^\]]*)\]\s*$")

_LLVM_SUFFIX_RE = re.compile(r"\s*\(\.llvm\.\d+\)$")
_HASH_SUFFIX_RE = re.compile(r"::h[0-9a-f]{16}$")
_IDENT_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*")


def _strip_suffixes(sym: str) -> str:
    s = sym.strip()
    s = _LLVM_SUFFIX_RE.sub("", s)
    s = _HASH_SUFFIX_RE.sub("", s)
    return s


def _find_matching_angle(s: str, start: int = 0) -> int | None:
    """Return index of '>' closing the '<' at `start`, or None."""
    depth = 0
    for i in range(start, len(s)):
        ch = s[i]
        if ch == "<":
            depth += 1
        elif ch == ">":
            depth -= 1
            if depth == 0:
                return i
    return None


def _split_top_level_as(inner: str) -> tuple[str, str | None]:
    """Split `<TYPE as TRAIT>` content at top-level ` as `, ignoring nested delimiters."""
    angle = paren = brace = bracket = 0
    i = 0
    n = len(inner)
    while i < n:
        ch = inner[i]
        if ch == "<":
            angle += 1
        elif ch == ">":
            angle = max(0, angle - 1)
        elif ch == "(":
            paren += 1
        elif ch == ")":
            paren = max(0, paren - 1)
        elif ch == "{":
            brace += 1
        elif ch == "}":
            brace = max(0, brace - 1)
        elif ch == "[":
            bracket += 1
        elif ch == "]":
            bracket = max(0, bracket - 1)
        elif angle == paren == brace == bracket == 0 and inner.startswith(" as ", i):
            return inner[:i].strip(), inner[i + 4 :].strip()
        i += 1
    return inner.strip(), None


def _strip_generics(path: str) -> str:
    """Remove angle-bracket generic argument lists `<...>` from a demangled Rust path."""
    out: list[str] = []
    depth = 0
    for ch in path:
        if ch == "<":
            depth += 1
        elif ch == ">":
            depth = max(0, depth - 1)
        elif depth == 0:
            out.append(ch)
    return "".join(out)


def _leading_crate(path: str) -> str | None:
    """Return the leading crate identifier of a demangled Rust path, if any."""
    s = path.strip()
    while s.startswith("&"):
        s = s[1:].lstrip()
        if s.startswith("mut "):
            s = s[4:].lstrip()
    if s.startswith("<"):
        close = _find_matching_angle(s)
        if close is None:
            return None
        recv, trait = _split_top_level_as(s[1:close])
        c = _leading_crate(recv)
        if c in SHIPPED_CRATES:
            return c
        if trait is not None:
            ct = _leading_crate(trait)
            if ct in SHIPPED_CRATES:
                return ct
        return c
    m = _IDENT_RE.match(s)
    if not m:
        return None
    name = m.group(0)
    rest = s[m.end() :]
    if rest.startswith("::") or rest == "":
        return name
    return None


def _first_segment(tail: str) -> str:
    s = tail.lstrip(":").strip()
    s = _strip_generics(s)
    return s.split("::", 1)[0] if s else ""


def classify_symbol(demangled_sym: str) -> tuple[str, int] | None:
    """Return `(bucket_name, cap_bytes)` for a demangled symbol of our shipped crates, or None."""
    s = _strip_suffixes(demangled_sym)
    if not s:
        return None

    receiver = ""
    method = ""
    owner_crate: str | None = None

    if s.startswith("<"):
        close = _find_matching_angle(s)
        if close is None:
            return None
        inner = s[1:close]
        tail = s[close + 1 :]
        recv, trait = _split_top_level_as(inner)
        recv_crate = _leading_crate(recv)
        if recv_crate in SHIPPED_CRATES:
            owner_crate = recv_crate
            recv_clean = _strip_generics(recv).strip()
            receiver = recv_clean.split("::")[-1] if recv_clean else ""
        elif trait is not None:
            trait_crate = _leading_crate(trait)
            if trait_crate in SHIPPED_CRATES:
                owner_crate = trait_crate
        method = _first_segment(tail)
    else:
        owner_crate = _leading_crate(s)
        if owner_crate not in SHIPPED_CRATES:
            return None
        segs = [seg for seg in _strip_generics(s).split("::") if seg]
        if len(segs) >= 3 and segs[-2] == "Hasher":
            receiver = "Hasher"
            method = segs[-1]
        elif segs:
            method = segs[-1]

    if owner_crate == "blake3_servil_core":
        if receiver == "Hasher" and not (method == "update" or method.startswith("update_")):
            return ("core::Hasher", CAP_HASHER)
        return ("core", CAP_CORE_B3SUM)
    if owner_crate == "blake3_servil":
        return ("threads", CAP_THREADS)
    if owner_crate in ("b3sum", "b3sums"):
        return ("b3sum", CAP_CORE_B3SUM)
    return None


def find_demangler() -> list[str] | None:
    """Locate `llvm-cxxfilt` (preferred) or GNU/Apple `c++filt`."""
    cxxfilt = shutil.which("llvm-cxxfilt")
    if cxxfilt:
        return [cxxfilt]
    cppfilt = shutil.which("c++filt")
    if cppfilt:
        return [cppfilt]
    return None


def demangle_symbols(raw_syms: list[str], cmd: list[str] | None = None) -> list[str]:
    """Batch-demangle symbol names via `llvm-cxxfilt` / `c++filt`."""
    if not raw_syms:
        return []
    tool = cmd if cmd is not None else find_demangler()
    if not tool:
        raise RuntimeError("neither llvm-cxxfilt nor c++filt found in PATH")
    proc = subprocess.run(
        tool,
        input="\n".join(raw_syms) + "\n",
        capture_output=True,
        text=True,
        check=True,
    )
    out = proc.stdout.splitlines()
    if len(out) != len(raw_syms):
        raise RuntimeError(
            f"demangler line count mismatch: expected {len(raw_syms)}, got {len(out)}"
        )
    return out


def parse_llvm_readobj_stack_sizes(text: str) -> list[tuple[int, str]]:
    """Parse `llvm-readobj --stack-sizes` output into `(size_bytes, symbol)` pairs."""
    entries: list[tuple[int, str]] = []
    in_entry = False
    entry_size: int | None = None
    entry_funcs: list[str] = []
    for line in text.splitlines():
        if _ENTRY_OPEN_RE.match(line):
            in_entry = True
            entry_size = None
            entry_funcs = []
            continue
        if not in_entry:
            continue
        if _ENTRY_CLOSE_RE.match(line):
            if entry_size is not None:
                for fn in entry_funcs:
                    entries.append((entry_size, fn))
            in_entry = False
            entry_size = None
            entry_funcs = []
            continue
        m_fn = _FUNCS_RE.match(line)
        if m_fn:
            entry_funcs = [f.strip() for f in m_fn.group(1).split(",") if f.strip()]
            continue
        m_sz = _SIZE_RE.match(line)
        if m_sz:
            raw = m_sz.group(1)
            entry_size = int(raw, 16) if raw.startswith("0x") else int(raw)
    return entries


def check_entries(
    entries: list[tuple[int, str]],
) -> tuple[dict[str, tuple[int, str]], list[str], int]:
    """Return `(max_by_bucket, violation_messages, budgeted_count)` for demangled entries."""
    max_by_bucket: dict[str, tuple[int, str]] = {}
    violations: list[str] = []
    budgeted = 0
    for size, sym in entries:
        cls = classify_symbol(sym)
        if cls is None:
            continue
        budgeted += 1
        bucket, cap = cls
        prev = max_by_bucket.get(bucket)
        if prev is None or size > prev[0]:
            max_by_bucket[bucket] = (size, sym)
        if size > cap:
            violations.append(f"{bucket}: {size} B > {cap} B cap in {sym}")
    return max_by_bucket, violations, budgeted


def main() -> int:
    readobj = shutil.which("llvm-readobj")
    if not readobj:
        print("stack_budget: llvm-readobj not found in PATH", file=sys.stderr)
        return 1
    demangler = find_demangler()
    if not demangler:
        print("stack_budget: llvm-cxxfilt / c++filt not found in PATH", file=sys.stderr)
        return 1

    repo = Path(__file__).resolve().parent.parent
    base_target = Path(os.environ.get("CARGO_TARGET_DIR", str(repo / "target")))
    stack_target = base_target / "stack-budget"

    env = os.environ.copy()
    env["RUSTC_BOOTSTRAP"] = "1"
    existing_flags = env.get("RUSTFLAGS", "").strip()
    env["RUSTFLAGS"] = f"{existing_flags} -Z emit-stack-sizes".strip()
    env["CARGO_TARGET_DIR"] = str(stack_target)

    subprocess.run(
        [
            "cargo",
            "build",
            "--quiet",
            "--release",
            "--locked",
            "--manifest-path",
            str(repo / "b3sum" / "Cargo.toml"),
            "--bin",
            "b3sum",
        ],
        cwd=repo,
        env=env,
        check=True,
    )

    binary = stack_target / "release" / "b3sum"
    proc = subprocess.run(
        [readobj, "--stack-sizes", str(binary)],
        capture_output=True,
        text=True,
        check=True,
    )
    raw_entries = parse_llvm_readobj_stack_sizes(proc.stdout)
    if not raw_entries:
        print("stack_budget: no .stack_sizes entries found in binary", file=sys.stderr)
        return 1

    demangled = demangle_symbols([sym for _, sym in raw_entries], cmd=demangler)
    entries = [(size, dsym) for (size, _), dsym in zip(raw_entries, demangled)]

    max_by_bucket, violations, budgeted = check_entries(entries)
    missing = sorted(EXPECTED_BUCKETS - set(max_by_bucket))
    if missing:
        print(
            f"stack_budget: expected shipped buckets missing from .stack_sizes: {', '.join(missing)}",
            file=sys.stderr,
        )
        return 1

    summary = ", ".join(
        f"{bucket}={sz}B" for bucket, (sz, _) in sorted(max_by_bucket.items())
    )
    if violations:
        print(
            f"stack_budget: FAILED ({len(violations)} violation(s); {budgeted}/{len(entries)} symbols budgeted; max: {summary})",
            file=sys.stderr,
        )
        for v in violations:
            print(f"  {v}", file=sys.stderr)
        return 1

    print(f"stack_budget: OK ({budgeted}/{len(entries)} symbols budgeted; max: {summary})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
