#!/usr/bin/env python3
"""Enforce single-frame release stack budgets via ELF `.stack_sizes`.

Builds `b3sum` (which links `blake3_servil_core`, `blake3_servil`, and `b3sums`)
with `RUSTC_BOOTSTRAP=1 RUSTFLAGS="-Z emit-stack-sizes"` into a dedicated target
subdirectory so it never invalidates the main Cargo cache, parses
`llvm-readobj --stack-sizes`, resolves Rust v0 (RFC 2603) and legacy (`_ZN...E`)
symbol definition provenance directly, and enforces the caps approved for #499:

- `blake3_servil_core` / `b3sums`: <= 128 KiB (131,072 bytes)
- `blake3_servil_core` `Hasher` non-update methods: <= 32 KiB (32,768 bytes)
- `blake3_servil` (`threads/`): <= 16 KiB (16,384 bytes)
"""

from __future__ import annotations

from dataclasses import dataclass
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

_LLVM_RAW_SUFFIX_RE = re.compile(r"\.llvm\.\d+$")
_LLVM_DEMANGLED_SUFFIX_RE = re.compile(r"\s*\(\.llvm\.\d+\)$")
_HASH_SUFFIX_RE = re.compile(r"::h[0-9a-f]{16}$")
_IDENT_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*")
_BASE62_ALPHA = "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
_BASE62_VAL = {ch: i for i, ch in enumerate(_BASE62_ALPHA)}

_LEGACY_ESCAPES = {
    "$SP$": "@",
    "$BP$": "*",
    "$RF$": "&",
    "$LT$": "<",
    "$GT$": ">",
    "$LP$": "(",
    "$RP$": ")",
    "$C$": ",",
}
_LEGACY_U_ESC_RE = re.compile(r"\$u([0-9a-fA-F]+)\$")
_LEGACY_HASH_SEG_RE = re.compile(r"^h[0-9a-f]{16}$")
_SHIPPED_TOKEN_RE = re.compile(r"(?:blake3_servil_core|blake3_servil|b3sums|b3sum)")


class SymbolParseError(ValueError):
    """Raised when a mangled Rust symbol cannot be parsed cleanly."""


@dataclass(frozen=True)
class _PathInfo:
    """Resolved defining-crate provenance and receiver/method leaf of a path."""

    crate: str | None
    leaf: str
    receiver: str
    method: str


class _V0Parser:
    """Minimal RFC 2603 Rust v0 mangled-symbol parser for definition provenance."""

    def __init__(self, sym: str) -> None:
        self.s = sym
        self.n = len(sym)
        self.pos = 0
        self._path_cache: dict[int, _PathInfo] = {}
        self._type_cache: dict[int, _PathInfo] = {}

    def peek(self) -> str:
        if self.pos >= self.n:
            raise SymbolParseError(f"unexpected EOF at {self.pos} in {self.s}")
        return self.s[self.pos]

    def take(self) -> str:
        ch = self.peek()
        self.pos += 1
        return ch

    def expect(self, ch: str) -> None:
        got = self.take()
        if got != ch:
            raise SymbolParseError(f"expected {ch!r}, got {got!r} at {self.pos - 1} in {self.s}")

    def parse_base62(self) -> int:
        if self.peek() == "_":
            self.pos += 1
            return 0
        val = 0
        while True:
            ch = self.take()
            if ch == "_":
                return val + 1
            d = _BASE62_VAL.get(ch)
            if d is None:
                raise SymbolParseError(f"bad base62 char {ch!r} at {self.pos - 1} in {self.s}")
            val = val * 62 + d

    def parse_opt_disambiguator(self) -> int:
        if self.pos < self.n and self.s[self.pos] == "s":
            self.pos += 1
            return self.parse_base62()
        return 0

    def parse_decimal(self) -> int:
        start = self.pos
        if self.peek() == "0":
            self.pos += 1
            return 0
        while self.pos < self.n and self.s[self.pos].isdigit():
            self.pos += 1
        if self.pos == start:
            raise SymbolParseError(f"expected decimal at {self.pos} in {self.s}")
        return int(self.s[start : self.pos])

    def parse_ident(self) -> str:
        self.parse_opt_disambiguator()
        if self.pos < self.n and self.s[self.pos] == "u":
            self.pos += 1
        length = self.parse_decimal()
        if self.pos < self.n and self.s[self.pos] == "_":
            self.pos += 1
        end = self.pos + length
        if end > self.n:
            raise SymbolParseError(f"ident length {length} past EOF in {self.s}")
        ident = self.s[self.pos : end]
        self.pos = end
        return ident

    def parse_hex_bytes(self) -> None:
        while self.peek() != "_":
            ch = self.take()
            if ch not in "0123456789abcdef":
                raise SymbolParseError(f"bad hex char {ch!r} in {self.s}")
        self.pos += 1

    def parse_binder(self) -> None:
        if self.pos < self.n and self.s[self.pos] == "G":
            self.pos += 1
            self.parse_base62()

    def parse_const(self) -> None:
        tag = self.take()
        if tag == "B":
            self.parse_base62()
            return
        if tag == "p":
            return
        # Typed constant: `<type> <const-data>`
        self.pos -= 1
        self.parse_type()
        if self.pos < self.n and self.s[self.pos] == "n":
            self.pos += 1
        self.parse_hex_bytes()

    def parse_generic_arg(self) -> None:
        tag = self.peek()
        if tag == "L":
            self.pos += 1
            self.parse_base62()
        elif tag == "K":
            self.pos += 1
            self.parse_const()
        else:
            self.parse_type()

    def parse_dyn_trait(self) -> None:
        self.parse_path()
        while self.pos < self.n and self.s[self.pos] == "p":
            self.pos += 1
            self.parse_ident()
            self.parse_type()

    def parse_type(self) -> _PathInfo:
        start = self.pos
        tag = self.take()
        if tag == "B":
            idx = self.parse_base62()
            target = 2 + idx
            if target in self._type_cache:
                return self._type_cache[target]
            if target in self._path_cache:
                return self._path_cache[target]
            saved = self.pos
            self.pos = target
            info = self.parse_type()
            self.pos = saved
            return info
        # Basic primitive types (single lowercase letter, RFC 2603)
        if tag in "abcdefhijlmnpstuvyz" or tag == "o":
            info = _PathInfo(None, tag, "", "")
        elif tag == "A":
            self.parse_type()
            self.parse_const()
            info = _PathInfo(None, "[]", "", "")
        elif tag == "S":
            self.parse_type()
            info = _PathInfo(None, "[]", "", "")
        elif tag == "T":
            while self.peek() != "E":
                self.parse_type()
            self.pos += 1
            info = _PathInfo(None, "()", "", "")
        elif tag in ("R", "Q"):
            if self.pos < self.n and self.s[self.pos] == "L":
                self.pos += 1
                self.parse_base62()
            inner = self.parse_type()
            info = inner
        elif tag in ("P", "O"):
            inner = self.parse_type()
            info = inner
        elif tag == "F":
            self.parse_binder()
            if self.pos < self.n and self.s[self.pos] == "U":
                self.pos += 1
            if self.pos < self.n and self.s[self.pos] == "K":
                self.pos += 1
                if self.peek() == "C":
                    self.pos += 1
                else:
                    self.parse_ident()
            while self.peek() != "E":
                self.parse_type()
            self.pos += 1
            self.parse_type()
            info = _PathInfo(None, "fn", "", "")
        elif tag == "D":
            self.parse_binder()
            while self.peek() != "E":
                self.parse_dyn_trait()
            self.pos += 1
            self.expect("L")
            self.parse_base62()
            info = _PathInfo(None, "dyn", "", "")
        else:
            # Named type: `<path>`
            self.pos = start
            info = self.parse_path()
        self._type_cache[start] = info
        return info

    def parse_path(self) -> _PathInfo:
        start = self.pos
        tag = self.take()
        if tag == "B":
            idx = self.parse_base62()
            target = 2 + idx
            if target in self._path_cache:
                return self._path_cache[target]
            saved = self.pos
            self.pos = target
            info = self.parse_path()
            self.pos = saved
            return info
        if tag == "C":
            crate = self.parse_ident()
            info = _PathInfo(crate=crate, leaf=crate, receiver="", method="")
        elif tag == "M":
            self.parse_opt_disambiguator()
            impl_path = self.parse_path()
            ty = self.parse_type()
            info = _PathInfo(crate=impl_path.crate, leaf=ty.leaf, receiver=ty.leaf, method="")
        elif tag == "X":
            self.parse_opt_disambiguator()
            impl_path = self.parse_path()
            ty = self.parse_type()
            _trait = self.parse_path()
            # RFC 2603: `impl_path` encodes the crate/module that defines the impl,
            # even when neither `ty` nor `trait` is local (e.g. `impl From<Hash> for [u8; 32]`).
            info = _PathInfo(crate=impl_path.crate, leaf=ty.leaf, receiver=ty.leaf, method="")
        elif tag == "Y":
            ty = self.parse_type()
            trait = self.parse_path()
            # Default trait method or closure/FnOnce shim: prefer shipped type if present,
            # else the trait's defining crate.
            owner = ty.crate if ty.crate in SHIPPED_CRATES else trait.crate
            info = _PathInfo(crate=owner, leaf=ty.leaf, receiver=ty.receiver or ty.leaf, method="")
        elif tag == "N":
            ns = self.take()
            parent = self.parse_path()
            ident = self.parse_ident()
            if ns == "v":
                # Free function or method on an impl/type (`parent.receiver` or `parent.leaf` for Hasher)
                recv = parent.receiver or (parent.leaf if parent.leaf == "Hasher" else "")
                info = _PathInfo(crate=parent.crate, leaf=ident, receiver=recv, method=ident)
            elif ns in ("C", "S"):
                # Closure or shim nested inside a function/method: preserve owning crate & receiver/method
                info = _PathInfo(
                    crate=parent.crate,
                    leaf=parent.leaf,
                    receiver=parent.receiver,
                    method=parent.method,
                )
            else:
                # Module (`t`), type (`t`), etc.
                info = _PathInfo(crate=parent.crate, leaf=ident, receiver=parent.receiver, method="")
        elif tag == "I":
            base = self.parse_path()
            while self.peek() != "E":
                self.parse_generic_arg()
            self.pos += 1
            info = base
        else:
            raise SymbolParseError(f"unknown v0 path tag {tag!r} at {start} in {self.s}")
        self._path_cache[start] = info
        return info

    def parse_symbol(self) -> _PathInfo:
        if not self.s.startswith("_R"):
            raise SymbolParseError(f"not a v0 symbol: {self.s}")
        self.pos = 2
        if self.pos < self.n and self.s[self.pos].isdigit():
            raise SymbolParseError(f"unsupported Rust mangling version in {self.s}")
        info = self.parse_path()
        # Optional trailing instantiating-crate `<path>` (ignored for defining ownership)
        if self.pos < self.n:
            self.parse_path()
        if self.pos != self.n:
            raise SymbolParseError(f"trailing bytes at {self.pos} in {self.s}")
        return info


def _decode_legacy_segment(seg: str) -> str:
    """Decode Rust legacy mangling escapes (`$LT$`, `$u20$`, `..`, etc.) in one segment."""
    if seg.startswith("_$"):
        seg = seg[1:]
    for esc, repl in _LEGACY_ESCAPES.items():
        seg = seg.replace(esc, repl)
    seg = _LEGACY_U_ESC_RE.sub(lambda m: chr(int(m.group(1), 16)), seg)
    return seg.replace("..", "::")


def _parse_legacy_symbol(sym: str) -> str:
    """Decode a Rust legacy `_ZN...E` symbol into a demangled path string."""
    if not (sym.startswith("_ZN") and sym.endswith("E")):
        raise SymbolParseError(f"not a legacy _ZN...E symbol: {sym}")
    pos = 3
    end = len(sym) - 1
    segs: list[str] = []
    while pos < end:
        num_start = pos
        while pos < end and sym[pos].isdigit():
            pos += 1
        if pos == num_start:
            raise SymbolParseError(f"expected segment length at {pos} in {sym}")
        length = int(sym[num_start:pos])
        seg_end = pos + length
        if seg_end > end:
            raise SymbolParseError(f"segment length {length} past E in {sym}")
        segs.append(_decode_legacy_segment(sym[pos:seg_end]))
        pos = seg_end
    if not segs:
        raise SymbolParseError(f"empty legacy symbol: {sym}")
    if _LEGACY_HASH_SEG_RE.match(segs[-1]):
        segs.pop()
    return "::".join(segs)


def _strip_demangled_suffixes(sym: str) -> str:
    s = sym.strip()
    s = _LLVM_DEMANGLED_SUFFIX_RE.sub("", s)
    s = _HASH_SUFFIX_RE.sub("", s)
    return s


def _find_matching_angle(s: str, start: int = 0) -> int | None:
    depth = 0
    for i in range(start, len(s)):
        ch = s[i]
        if ch == "<":
            depth += 1
        elif ch == ">":
            if i > 0 and s[i - 1] == "-":
                continue
            depth -= 1
            if depth == 0:
                return i
    return None


def _split_top_level_as(inner: str) -> tuple[str, str | None]:
    angle = paren = brace = bracket = 0
    i = 0
    n = len(inner)
    while i < n:
        ch = inner[i]
        if ch == "<":
            angle += 1
        elif ch == ">":
            if not (i > 0 and inner[i - 1] == "-"):
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
    out: list[str] = []
    depth = 0
    for i, ch in enumerate(path):
        if ch == "<":
            depth += 1
        elif ch == ">":
            if i > 0 and path[i - 1] == "-":
                if depth == 0:
                    out.append(ch)
                continue
            depth = max(0, depth - 1)
        elif depth == 0:
            out.append(ch)
    return "".join(out)


def _strip_crate_disambiguator(name: str) -> str:
    """Strip GNU `c++filt` crate hash suffixes like `blake3_servil_core[4edb22372a9d6ceb]`."""
    return re.sub(r"\[[0-9a-f]+\]$", "", name)


def _leading_crate(path: str) -> str | None:
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
    if rest.startswith("["):
        end_br = rest.find("]")
        if end_br != -1:
            rest = rest[end_br + 1 :]
    if rest.startswith("::") or rest == "":
        return name
    return None


def _first_segment(tail: str) -> str:
    s = tail.lstrip(":").strip()
    s = _strip_generics(s)
    return s.split("::", 1)[0] if s else ""


def _bucket_for(owner_crate: str | None, receiver: str, method: str) -> tuple[str, int] | None:
    if owner_crate == "blake3_servil_core":
        if receiver == "Hasher" and not (method == "update" or method.startswith("update_")):
            return ("core::Hasher", CAP_HASHER)
        return ("core", CAP_CORE_B3SUM)
    if owner_crate == "blake3_servil":
        return ("threads", CAP_THREADS)
    if owner_crate in ("b3sum", "b3sums"):
        return ("b3sum", CAP_CORE_B3SUM)
    return None


def _classify_demangled(demangled_sym: str) -> tuple[str, int] | None:
    s = _strip_demangled_suffixes(demangled_sym)
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
            receiver = _strip_crate_disambiguator(recv_clean.split("::")[-1]) if recv_clean else ""
        elif trait is not None:
            trait_crate = _leading_crate(trait)
            if trait_crate in SHIPPED_CRATES:
                owner_crate = trait_crate
            else:
                # Legacy/demangled impls like `<[u8; 32] as core::convert::From<blake3_servil_core::Hash>>::from`
                # lose v0 impl-path provenance; only local crate type args in foreign-receiver/foreign-trait
                # impls can exist in Rust (orphan rules).
                for crate in SHIPPED_CRATES:
                    if re.search(rf"\b{re.escape(crate)}(?:\[[0-9a-f]+\])?::", trait):
                        owner_crate = crate
                        break
        method = _first_segment(tail)
    else:
        owner_crate = _leading_crate(s)
        if owner_crate not in SHIPPED_CRATES:
            return None
        segs = [_strip_crate_disambiguator(seg) for seg in _strip_generics(s).split("::") if seg]
        if len(segs) >= 3 and segs[-2] == "Hasher":
            receiver = "Hasher"
            method = segs[-1]
        elif segs:
            method = segs[-1]

    return _bucket_for(owner_crate, receiver, method)


def classify_symbol(sym: str) -> tuple[str, int] | None:
    """Return `(bucket_name, cap_bytes)` for a raw v0, raw legacy, or demangled symbol, or None.

    Raises `SymbolParseError` if a raw Rust mangled symbol (`_R...` or `_ZN...E`)
    mentions one of our shipped crates yet fails to parse (fail-closed).
    """
    raw = sym.strip()
    if not raw:
        return None
    if raw.startswith("_R"):
        cleaned = _LLVM_RAW_SUFFIX_RE.sub("", raw)
        try:
            info = _V0Parser(cleaned).parse_symbol()
        except SymbolParseError:
            if _SHIPPED_TOKEN_RE.search(cleaned):
                raise
            return None
        return _bucket_for(info.crate, info.receiver, info.method)
    if raw.startswith("_ZN"):
        cleaned = _LLVM_RAW_SUFFIX_RE.sub("", raw)
        try:
            demangled = _parse_legacy_symbol(cleaned)
        except SymbolParseError:
            if _SHIPPED_TOKEN_RE.search(cleaned):
                raise
            return None
        return _classify_demangled(demangled)
    return _classify_demangled(raw)


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
    """Return `(max_by_bucket, violation_messages, budgeted_count)`."""
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


def evaluate_stack_sizes(text: str) -> tuple[int, str, list[str]]:
    """Parse and evaluate `llvm-readobj --stack-sizes` output.

    Returns `(exit_code, headline, detail_lines)`.
    """
    entries = parse_llvm_readobj_stack_sizes(text)
    if not entries:
        return 1, "stack_budget: no .stack_sizes entries found in binary", []

    try:
        max_by_bucket, violations, budgeted = check_entries(entries)
    except SymbolParseError as exc:
        return 1, f"stack_budget: failed to parse shipped symbol: {exc}", []

    missing = sorted(EXPECTED_BUCKETS - set(max_by_bucket))
    if missing:
        return (
            1,
            f"stack_budget: expected shipped buckets missing from .stack_sizes: {', '.join(missing)}",
            [],
        )

    summary = ", ".join(
        f"{bucket}={sz}B" for bucket, (sz, _) in sorted(max_by_bucket.items())
    )
    if violations:
        return (
            1,
            f"stack_budget: FAILED ({len(violations)} violation(s); {budgeted}/{len(entries)} symbols budgeted; max: {summary})",
            violations,
        )

    return (
        0,
        f"stack_budget: OK ({budgeted}/{len(entries)} symbols budgeted; max: {summary})",
        [],
    )


def main() -> int:
    readobj = shutil.which("llvm-readobj")
    if not readobj:
        print("stack_budget: llvm-readobj not found in PATH", file=sys.stderr)
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
    code, headline, details = evaluate_stack_sizes(proc.stdout)
    stream = sys.stdout if code == 0 else sys.stderr
    print(headline, file=stream)
    for line in details:
        print(f"  {line}", file=stream)
    return code


if __name__ == "__main__":
    raise SystemExit(main())
