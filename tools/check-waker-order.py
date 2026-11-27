#!/usr/bin/env python3
"""Check the sleep announcements on Apple arm64 (#463, #571).

Shuttle checks schedules under SC, not order against an RCpc completion
re-check. Compile the production wait paths to real `aarch64-apple-darwin`
Mach-O object code (`rustc --emit=obj`), then remove just the fences in
scratch copies to verify that the probe detects the loss. Decoding the
assembled 32-bit AArch64 machine words and Mach-O relocations directly avoids
assembler-syntax ambiguities while requiring no linker, Apple SDK, or
external disassembler.
"""
from bisect import bisect_right
from enum import Enum
from pathlib import Path
import re
import struct
import subprocess
import tempfile
from typing import NamedTuple


PROBE = """
#![allow(dead_code, unused_imports)]
mod waker_sync {
    pub use core::cell::UnsafeCell;
    pub use core::sync::atomic::{AtomicUsize, Ordering, fence};
    #[inline(always)]
    pub fn spin() { core::hint::spin_loop(); }
    #[inline(always)]
    pub unsafe fn with_mut<T, R>(cell: &UnsafeCell<T>, f: impl FnOnce(&mut T) -> R) -> R {
        f(unsafe { &mut *cell.get() })
    }
}
mod waker;
mod park_sync {
    pub use std::sync::OnceLock;
    pub use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering, fence};
    pub use std::thread;
}
mod park;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[unsafe(no_mangle)]
pub(crate) fn register_then_recheck(
    slot: &waker::WakerSlot,
    cx: &core::task::Context<'_>,
    completed: &core::sync::atomic::AtomicUsize,
) -> usize {
    slot.register(cx);
    completed.load(Ordering::Acquire)
}

#[unsafe(no_mangle)]
pub(crate) fn park_then_recheck(park: &park::Park, worker: usize, published: &AtomicUsize) -> bool {
    park.park_unless(worker, || published.load(Ordering::Acquire) != 0)
}

#[unsafe(no_mangle)]
pub(crate) fn sme2_then_recheck(flag: &AtomicBool, published: &AtomicUsize) -> bool {
    // SME2_ADVERTISEMENT
    published.load(Ordering::Acquire) != 0
}
"""


class Verdict(Enum):
    ORDERED = "ordered"
    UNORDERED = "unordered"
    UNSUPPORTED = "unsupported"


class CheckResult(NamedTuple):
    verdict: Verdict
    detail: str
    missing_fence_after_announce: bool = False


class Symbol(NamedTuple):
    name: str
    n_type: int
    n_sect: int
    text_pc: int | None  # Byte offset inside (__TEXT,__text), or None


class Reloc(NamedTuple):
    r_address: int
    r_symbolnum: int
    r_pcrel: int
    r_length: int
    r_extern: int
    r_type: int


class MachOObject(NamedTuple):
    words: tuple[int, ...]
    relocs: dict[int, list[Reloc]]
    symtab: tuple[Symbol, ...]
    defined_text: dict[str, list[int]]
    func_starts: tuple[int, ...]


# Mach-O 64-bit ARM64 constants (mach-o/loader.h, mach-o/nlist.h, mach-o/arm64/reloc.h)
MH_MAGIC_64 = 0xFEEDFACF
CPU_TYPE_ARM64 = 0x0100000C
MH_OBJECT = 1
LC_SYMTAB = 0x02
LC_SEGMENT_64 = 0x19
N_STAB = 0xE0
N_TYPE = 0x0E
N_UNDF = 0x00
N_SECT = 0x0E

ARM64_RELOC_BRANCH26 = 2
ARM64_RELOC_DATA_ADDR_TYPES = frozenset({
    3,   # ARM64_RELOC_PAGE21
    4,   # ARM64_RELOC_PAGEOFF12
    5,   # ARM64_RELOC_GOT_LOAD_PAGE21
    6,   # ARM64_RELOC_GOT_LOAD_PAGEOFF12
    8,   # ARM64_RELOC_TLVP_LOAD_PAGE21
    9,   # ARM64_RELOC_TLVP_LOAD_PAGEOFF12
    10,  # ARM64_RELOC_ADDEND
})

MAX_CALL_DEPTH = 16

# Recognized external nonreturning panic/abort callees in compiled Rust probes.
# Unrecognized external calls that reach a function boundary fail closed as
# UNSUPPORTED rather than falling through into an adjacent symbol.
NORETURN_CALL_RE = re.compile(
    r"^(?:_abort|_rust_begin_unwind|"
    r"__RNvNtCs[A-Za-z0-9]+_3std7process5abort|"
    r"__RNvNtCs[A-Za-z0-9]+_4core9panicking(?:"
    r"18panic_bounds_check|5panic|9panic_fmt|14panic_explicit|"
    r"14panic_nounwind|18panic_nounwind_fmt|19panic_cannot_unwind)|"
    r"__RNvNtCs[A-Za-z0-9]+_4core6option(?:13expect_failed|13unwrap_failed)|"
    r"__RNvNtCs[A-Za-z0-9]+_4core6result13unwrap_failed)$"
)

ANNOUNCE_OPS = frozenset({"stlr", "stlrb", "ldsetal"})

# `dmb` CRm encodings (ARMv8-A C6.2.79):
# - 0xB = `dmb ish` (required inner-shareable full barrier)
# - Weak options strictly insufficient for inner-shareable store->load ordering
#   (load-only, store-only, or non-shareable): modeled as no-op -> UNORDERED.
# - Stronger sufficient options (`0x3` `osh`, `0xF` `sy`), reserved encodings,
#   and all `dsb` instructions fail closed as UNSUPPORTED.
DMB_CRM_ISH = 0xB
WEAK_DMB_CRMS = frozenset({
    0x1,  # oshld
    0x2,  # oshst
    0x5,  # nshld
    0x6,  # nshst
    0x7,  # nsh
    0x9,  # ishld
    0xA,  # ishst
    0xD,  # ld
    0xE,  # st
})

# Valid `(size, opc)` pairs for ARMv8.4 `FEAT_LRCPC2` `ldapur*` loads
# (ARMv8-A C4.1.4, `bits[29:24]=011001`, `bit21=0`, `bits[11:10]=00`, `opc!=00`).
LDAPUR_SIZE_OPC = frozenset({
    (0, 1), (0, 2), (0, 3),  # ldapurb, ldapursb (64/32)
    (1, 1), (1, 2), (1, 3),  # ldapurh, ldapursh (64/32)
    (2, 1), (2, 2),          # ldapur (32), ldapursw
    (3, 1),                  # ldapur (64)
})


def compile_probe(sources, directory):
    for name, source in sources.items():
        (directory / name).write_text(source)
    obj = directory / "probe.o"
    obj.unlink(missing_ok=True)
    subprocess.run([
        "rustc", "--edition", "2024", "--crate-type", "lib", "-O",
        "-C", "panic=abort", "-C", "codegen-units=1",
        "--target", "aarch64-apple-darwin", "--emit", "obj", "-o", str(obj),
        str(directory / "probe.rs"),
    ], check=True)
    return obj.read_bytes()


def _sign_extend(val, bits):
    sign = 1 << (bits - 1)
    return (val ^ sign) - sign


def parse_macho_arm64(data):
    """Parse a 64-bit little-endian ARM64 Mach-O relocatable object file."""
    if len(data) < 32:
        return None, "Mach-O object shorter than 64-bit header"
    magic, cputype, _cpusubtype, filetype, ncmds, sizeofcmds, _flags, _reserved = (
        struct.unpack_from("<IiiIIIII", data, 0)
    )
    if magic != MH_MAGIC_64:
        return None, f"unexpected Mach-O magic {magic:#x}"
    if cputype != CPU_TYPE_ARM64:
        return None, f"unexpected Mach-O cputype {cputype:#x}"
    if filetype != MH_OBJECT:
        return None, f"unexpected Mach-O filetype {filetype}"
    if 32 + sizeofcmds > len(data):
        return None, "Mach-O load commands exceed file size"

    off = 32
    cmds_end = 32 + sizeofcmds
    sect_idx = 0
    text_info = None
    symtab_info = None

    for _ in range(ncmds):
        if off + 8 > cmds_end:
            return None, "truncated Mach-O load command header"
        cmd, cmdsize = struct.unpack_from("<II", data, off)
        if cmdsize < 8 or off + cmdsize > cmds_end:
            return None, f"invalid Mach-O load command size {cmdsize}"
        if cmd == LC_SEGMENT_64:
            if cmdsize < 72:
                return None, "truncated LC_SEGMENT_64 command"
            nsects = struct.unpack_from("<I", data, off + 64)[0]
            if 72 + nsects * 80 > cmdsize:
                return None, "LC_SEGMENT_64 sections exceed command size"
            soff = off + 72
            for _ in range(nsects):
                sect_idx += 1
                sn, sgn, addr, size, foff, _align, reloff, nreloc, _fl, _r1, _r2, _r3 = (
                    struct.unpack_from("<16s16sQQIIIIIIII", data, soff)
                )
                soff += 80
                if sgn.rstrip(b"\0") == b"__TEXT" and sn.rstrip(b"\0") == b"__text":
                    if text_info is not None:
                        return None, "duplicate (__TEXT,__text) section"
                    text_info = (sect_idx, addr, size, foff, reloff, nreloc)
        elif cmd == LC_SYMTAB:
            if cmdsize < 24:
                return None, "truncated LC_SYMTAB command"
            if symtab_info is not None:
                return None, "duplicate LC_SYMTAB command"
            symtab_info = struct.unpack_from("<IIII", data, off + 8)
        off += cmdsize

    if text_info is None:
        return None, "missing (__TEXT,__text) section"
    if symtab_info is None:
        return None, "missing LC_SYMTAB command"

    text_sect_idx, text_addr, text_size, text_foff, text_reloff, text_nreloc = text_info
    if text_size % 4 != 0 or text_foff + text_size > len(data):
        return None, "invalid (__TEXT,__text) bounds or alignment"
    if text_reloff + text_nreloc * 8 > len(data):
        return None, "invalid (__TEXT,__text) relocation table bounds"

    words = struct.unpack_from(f"<{text_size // 4}I", data, text_foff)

    relocs = {}
    for i in range(text_nreloc):
        r_address, r_info = struct.unpack_from("<iI", data, text_reloff + i * 8)
        if r_address < 0 or r_address >= text_size or r_address % 4 != 0:
            return None, f"unsupported relocation address {r_address:#x}"
        rel = Reloc(
            r_address=r_address,
            r_symbolnum=r_info & 0x00FFFFFF,
            r_pcrel=(r_info >> 24) & 1,
            r_length=(r_info >> 25) & 3,
            r_extern=(r_info >> 27) & 1,
            r_type=(r_info >> 28) & 0xF,
        )
        relocs.setdefault(r_address, []).append(rel)

    symoff, nsyms, stroff, strsize = symtab_info
    if symoff + nsyms * 16 > len(data) or stroff + strsize > len(data):
        return None, "invalid LC_SYMTAB bounds"

    symtab = []
    defined_text = {}
    for i in range(nsyms):
        n_strx, n_type, n_sect, _n_desc, n_value = struct.unpack_from("<IBBHQ", data, symoff + i * 16)
        if n_strx >= strsize:
            return None, f"symbol {i} string offset out of bounds"
        nul = data.find(b"\0", stroff + n_strx, stroff + strsize)
        if nul == -1:
            return None, f"symbol {i} name is not NUL-terminated"
        name = data[stroff + n_strx:nul].decode("utf-8", errors="replace")
        text_pc = None
        if (n_type & N_STAB) == 0 and (n_type & N_TYPE) == N_SECT and n_sect == text_sect_idx:
            rel_pc = n_value - text_addr
            if rel_pc < 0 or rel_pc > text_size or rel_pc % 4 != 0:
                return None, f"symbol {name} offset {rel_pc:#x} out of (__TEXT,__text) bounds"
            if rel_pc < text_size:
                text_pc = rel_pc
                defined_text.setdefault(name, []).append(rel_pc)
        symtab.append(Symbol(name=name, n_type=n_type, n_sect=n_sect, text_pc=text_pc))

    func_starts = tuple(
        sorted({
            s.text_pc
            for s in symtab
            if s.text_pc is not None and not s.name.startswith(("L", "ltmp"))
        })
    )
    return MachOObject(
        words=words,
        relocs=relocs,
        symtab=tuple(symtab),
        defined_text=defined_text,
        func_starts=func_starts,
    ), None


def _func_end(obj, entry_pc):
    """End byte offset of the symbol extent containing `entry_pc`."""
    idx = bisect_right(obj.func_starts, entry_pc)
    return obj.func_starts[idx] if idx < len(obj.func_starts) else len(obj.words) * 4


def _decode_branch26_reloc(obj, pc, imm26, pc_relocs, opname):
    if len(pc_relocs) != 1:
        return None, f"multiple relocations on {opname} at {pc:#x}"
    rel = pc_relocs[0]
    if (
        rel.r_type != ARM64_RELOC_BRANCH26
        or rel.r_extern != 1
        or rel.r_pcrel != 1
        or rel.r_length != 2
        or imm26 != 0
    ):
        return None, f"unsupported {opname} relocation at {pc:#x}"
    if rel.r_symbolnum >= len(obj.symtab):
        return None, f"out-of-range relocation symbol index at {pc:#x}"
    return obj.symtab[rel.r_symbolnum], None


def _is_announce_word(w, announce):
    if announce == "stlr":
        return (w & 0xBFFFFC00) == 0x889FFC00
    if announce == "stlrb":
        return (w & 0xFFFFFC00) == 0x089FFC00
    if announce == "ldsetal":
        return (w & 0xBFE0FC00) == 0xB8E03000
    return False


def _decode_word(obj, pc, announce):
    """Decode one 32-bit AArch64 instruction word at byte offset `pc` inside
    `(__TEXT,__text)` together with any Mach-O relocations at `pc`.

    Follows the top-level ARMv8-A C4.1 encoding partition on `op0 = bits[28:25]`:
    - `101x` (Branches, Exception Generating, System): exhaustively decoded;
      anything unmodeled fails closed as UNSUPPORTED.
    - `x1x0` (Loads & Stores): decodes `announce` and all RCpc Acquire loads
      (`ldapr*`, `ldapur*`), fails closed on unmodeled LRCPC3 forms, and treats
      other plain/RCsc loads and stores as non-control-flow no-ops.
    - `100x` (DP-Imm), `x101` (DP-Reg), `x111` (Scalar FP / AdvSIMD): pure ALU
      with no control flow, barriers, or memory ops.
    - `00xx` (Reserved / UDF / SVE / SME): fails closed as UNSUPPORTED.
    """
    text_len = len(obj.words) * 4
    w = obj.words[pc // 4]
    pc_relocs = obj.relocs.get(pc, ())
    op0 = (w >> 25) & 0xF

    # Group A: Branches, Exception Generating, and System (`op0 = 101x`).
    if op0 in (0b1010, 0b1011):
        # Unconditional branch immediate `b` (including same-file sibling/tail calls).
        if (w & 0xFC000000) == 0x14000000:
            imm26 = _sign_extend(w & 0x03FFFFFF, 26)
            if pc_relocs:
                sym, err = _decode_branch26_reloc(obj, pc, imm26, pc_relocs, "b")
                if err is not None:
                    return None, err
                if sym.text_pc is not None:
                    return ("local_tail", sym.text_pc, sym.name), None
                if (sym.n_type & N_STAB) == 0 and (sym.n_type & N_TYPE) == N_UNDF:
                    if NORETURN_CALL_RE.fullmatch(sym.name):
                        return ("noreturn",), None
                    return None, f"relocated unconditional branch to external symbol {sym.name} at {pc:#x}"
                return None, f"unsupported b target symbol {sym.name} at {pc:#x}"
            dst = pc + imm26 * 4
            if not (0 <= dst < text_len):
                return None, f"out-of-section branch target {dst:#x} at {pc:#x}"
            return ("branch", dst), None

        # Branch with link `bl`.
        if (w & 0xFC000000) == 0x94000000:
            imm26 = _sign_extend(w & 0x03FFFFFF, 26)
            if pc_relocs:
                sym, err = _decode_branch26_reloc(obj, pc, imm26, pc_relocs, "bl")
                if err is not None:
                    return None, err
                # Resolve same-file definitions BEFORE applying the external
                # noreturn allowlist so a locally defined `_abort: ret` is
                # analyzed rather than pruned.
                if sym.text_pc is not None:
                    return ("local_call", sym.text_pc, sym.name), None
                if (sym.n_type & N_STAB) == 0 and (sym.n_type & N_TYPE) == N_UNDF:
                    if NORETURN_CALL_RE.fullmatch(sym.name):
                        return ("noreturn",), None
                    return ("ext_call", sym.name), None
                return None, f"unsupported bl target symbol {sym.name} at {pc:#x}"
            dst = pc + imm26 * 4
            if not (0 <= dst < text_len):
                return None, f"out-of-section local bl target {dst:#x} at {pc:#x}"
            return ("local_call", dst, f"{dst:#x}"), None

        # Any relocation on any other control-flow/barrier/system op is unsupported.
        if pc_relocs:
            return None, f"unexpected relocation on system/branch instruction at {pc:#x}"

        # Conditional branch immediate `b.cond` (bit 4 == 0; excludes Armv8.8 `bc.cond`).
        if (w & 0xFF000010) == 0x54000000:
            cond = w & 0xF
            if cond == 0xF:  # `b.nv`
                return None, f"unsupported b.nv at {pc:#x}"
            dst = pc + _sign_extend((w >> 5) & 0x7FFFF, 19) * 4
            if not (0 <= dst < text_len):
                return None, f"out-of-section b.cond target {dst:#x} at {pc:#x}"
            if cond == 0xE:  # `b.al`
                return ("branch", dst), None
            return ("cond_branch", dst), None

        # Compare and branch `cbz` / `cbnz`.
        if (w & 0x7E000000) == 0x34000000:
            rt = w & 0x1F
            if rt == 31:  # xzr/wzr deterministic/impossible branch
                return None, f"unsupported zero-register cbz/cbnz at {pc:#x}"
            dst = pc + _sign_extend((w >> 5) & 0x7FFFF, 19) * 4
            if not (0 <= dst < text_len):
                return None, f"out-of-section cbz/cbnz target {dst:#x} at {pc:#x}"
            return ("cond_branch", dst), None

        # Test and branch `tbz` / `tbnz`.
        if (w & 0x7E000000) == 0x36000000:
            rt = w & 0x1F
            if rt == 31:  # xzr/wzr deterministic/impossible branch
                return None, f"unsupported zero-register tbz/tbnz at {pc:#x}"
            dst = pc + _sign_extend((w >> 5) & 0x3FFF, 14) * 4
            if not (0 <= dst < text_len):
                return None, f"out-of-section tbz/tbnz target {dst:#x} at {pc:#x}"
            return ("cond_branch", dst), None

        # Default return `ret` (`x30` only; `ret Xn` and `retaa`/`retab` rejected).
        if w == 0xD65F03C0:
            return ("ret",), None

        # Indirect call `blr Xn` (`x0..x30`, excluding `xzr` and authenticated `blraa*`).
        if (w & 0xFFFFFC1F) == 0xD63F0000:
            rn = (w >> 5) & 0x1F
            if rn == 31:
                return None, f"unsupported blr xzr at {pc:#x}"
            return ("ext_call", f"blr x{rn}"), None

        # Barriers: `dmb <CRm>`, `isb`, `clrex`. (`dsb` fails closed below.)
        if (w & 0xFFFFF0FF) == 0xD50330BF:
            crm = (w >> 8) & 0xF
            if crm == DMB_CRM_ISH:
                return ("fence",), None
            if crm in WEAK_DMB_CRMS:
                return ("noop",), None
            return None, f"unsupported dmb CRm={crm:#x} at {pc:#x}"
        if w == 0xD5033FDF or (w & 0xFFFFF0FF) == 0xD503305F:  # `isb`, `clrex`
            return ("noop",), None

        # Architectural hint space (`nop`, `yield`, `wfe`, `wfi`, `bti`, `paciasp`, `autiasp`, ...).
        if (w & 0xFFFFF01F) == 0xD503201F:
            return ("noop",), None

        return None, f"unsupported branch/system instruction {w:#010x} at {pc:#x}"

    # Group D: Reserved / UDF / SVE / SME (`op0 = 00xx`).
    if op0 in (0b0000, 0b0001, 0b0010, 0b0011):
        return None, f"unsupported reserved/SVE/SME instruction {w:#010x} at {pc:#x}"

    # Group B: Loads and Stores (`op0 = x1x0`).
    if (op0 & 0b0101) == 0b0100:
        if _is_announce_word(w, announce):
            if pc_relocs:
                return None, f"unexpected relocation on {announce} at {pc:#x}"
            return ("announce",), None

        # FEAT_LRCPC: `ldapr`, `ldaprb`, `ldaprh`.
        if (w & 0x3FFFFC00) == 0x38BFC000:
            if pc_relocs:
                return None, f"unexpected relocation on ldapr* at {pc:#x}"
            return ("rcpc_load",), None

        # FEAT_LRCPC2 (`ldapur*` / `stlur*`) and FEAT_LRCPC3 (`0x19000000` space).
        if (w & 0x3F000000) == 0x19000000:
            if pc_relocs:
                return None, f"unexpected relocation on LRCPC2/3 instruction at {pc:#x}"
            if (w & 0x00200C00) == 0:
                size = (w >> 30) & 3
                opc = (w >> 22) & 3
                if opc == 0:  # `stlur*` store-release
                    return ("noop",), None
                if (size, opc) in LDAPUR_SIZE_OPC:
                    return ("rcpc_load",), None
            return None, f"unsupported LRCPC2/3 instruction {w:#010x} at {pc:#x}"

        # FEAT_LRCPC3 SIMD/FP RCpc loads (`ldap1` and `0x1D000000` space) fail closed.
        if (w & 0xBFFFFC00) == 0x0D418400 or (w & 0x3F000000) == 0x1D000000:
            return None, f"unsupported LRCPC3 FP/SIMD instruction {w:#010x} at {pc:#x}"

        if pc_relocs and not all(r.r_type in ARM64_RELOC_DATA_ADDR_TYPES for r in pc_relocs):
            return None, f"unsupported relocation on load/store at {pc:#x}"
        return ("noop",), None

    # Group C: Data Processing — Immediate (`100x`), Register (`x101`), FP/SIMD (`x111`).
    if pc_relocs and not all(r.r_type in ARM64_RELOC_DATA_ADDR_TYPES for r in pc_relocs):
        return None, f"unsupported relocation on ALU instruction at {pc:#x}"
    return ("noop",), None


def _analyze_from(obj, entry_pc, entry_state, announce, cache, active, depth):
    """Explore the syntactic CFG starting at `entry_pc` in `entry_state`.

    Returns `(err, seen_rcpc_pcs, unord_ann, unord_unann, exit_states, max_sub_depth)`:
    - Every reachable instruction is decoded by exact 32-bit hardware encoding.
    - The walk stays within the symbol extent `[entry_pc, func_end)` so an
      unrecognized nonreturning external call or missing `ret` fails closed as
      UNSUPPORTED instead of walking into an adjacent function.
    - Same-file `bl` callees and relocated `b` sibling/tail calls are analyzed
      from the caller's ordering state inside their own extents and memoized
      with their exact `max_sub_depth` so a primed cache still enforces
      `MAX_CALL_DEPTH` on every call path.
    - Only states that reach `ret` continue past a same-file `bl` call, so an
      infinite-loop or nonreturning callee cannot make unreachable caller code
      look ordered or unordered.
    """
    empty_pcs = frozenset()
    if entry_pc in active:
        return f"recursive same-file call to {entry_pc:#x}", empty_pcs, None, None, frozenset(), 0
    if depth > MAX_CALL_DEPTH:
        return f"same-file call depth exceeds limit ({MAX_CALL_DEPTH}) at {entry_pc:#x}", empty_pcs, None, None, frozenset(), 0

    key = (entry_pc, entry_state)
    if key in cache:
        cached = cache[key]
        if depth + cached[5] > MAX_CALL_DEPTH:
            return f"same-file call depth exceeds limit ({MAX_CALL_DEPTH}) at {entry_pc:#x}", empty_pcs, None, None, frozenset(), 0
        return cached

    text_len = len(obj.words) * 4
    func_end = _func_end(obj, entry_pc)
    active.add(entry_pc)
    try:
        seen_rcpc = set()
        unord_ann = None
        unord_unann = None
        exit_states = set()
        max_sub_depth = 0
        visited = set()
        work = [(entry_pc, entry_state)]
        while work:
            pc, state = work.pop()
            if (pc, state) in visited:
                continue
            visited.add((pc, state))
            ir, err = _decode_word(obj, pc, announce)
            if err is not None:
                return err, empty_pcs, None, None, frozenset(), 0
            kind = ir[0]
            s = state
            if kind == "announce":
                s = 1
            elif kind == "fence":
                if s == 1:
                    s = 2
            elif kind == "rcpc_load":
                seen_rcpc.add(pc)
                if s == 1 and unord_ann is None:
                    unord_ann = (
                        f"syntactic path to RCpc load at {pc:#x} "
                        f"after {announce} without dmb ish"
                    )
                elif s == 0 and unord_unann is None:
                    unord_unann = (
                        f"syntactic path to RCpc load at {pc:#x} "
                        f"without {announce}"
                    )
            elif kind in ("local_call", "local_tail"):
                callee_pc = ir[1]
                cerr, c_rcpc, c_ann, c_unann, c_exits, c_depth = _analyze_from(
                    obj, callee_pc, s, announce, cache, active, depth + 1
                )
                if cerr is not None:
                    return cerr, empty_pcs, None, None, frozenset(), 0
                if 1 + c_depth > max_sub_depth:
                    max_sub_depth = 1 + c_depth
                seen_rcpc.update(c_rcpc)
                if c_ann is not None and unord_ann is None:
                    unord_ann = c_ann
                if c_unann is not None and unord_unann is None:
                    unord_unann = c_unann
                if kind == "local_tail":
                    exit_states.update(c_exits)
                elif c_exits:
                    next_pc = pc + 4
                    if next_pc >= func_end:
                        where = "function boundary" if func_end < text_len else "end of __text"
                        return f"reachable fallthrough past {where} after bl at {pc:#x}", empty_pcs, None, None, frozenset(), 0
                    for s_out in c_exits:
                        work.append((next_pc, s_out))
                continue
            elif kind == "ret":
                exit_states.add(s)
                continue
            elif kind == "noreturn":
                continue
            elif kind == "branch":
                dst = ir[1]
                if not (entry_pc <= dst < func_end):
                    return (
                        f"branch target {dst:#x} crosses function boundary "
                        f"[{entry_pc:#x}, {func_end:#x}) at {pc:#x}",
                        empty_pcs, None, None, frozenset(), 0,
                    )
                work.append((dst, s))
                continue
            elif kind == "cond_branch":
                dst = ir[1]
                if not (entry_pc <= dst < func_end):
                    return (
                        f"branch target {dst:#x} crosses function boundary "
                        f"[{entry_pc:#x}, {func_end:#x}) at {pc:#x}",
                        empty_pcs, None, None, frozenset(), 0,
                    )
                work.append((dst, s))

            next_pc = pc + 4
            if next_pc >= func_end:
                where = "function boundary" if func_end < text_len else "end of __text"
                if kind == "ext_call":
                    return (
                        f"unrecognized external call {ir[1]} reaches {where} at {pc:#x}",
                        empty_pcs, None, None, frozenset(), 0,
                    )
                return f"reachable fallthrough past {where} at {pc:#x}", empty_pcs, None, None, frozenset(), 0
            work.append((next_pc, s))

        result = (None, frozenset(seen_rcpc), unord_ann, unord_unann, frozenset(exit_states), max_sub_depth)
        cache[key] = result
        return result
    finally:
        active.remove(entry_pc)


def check_cfg(obj_bytes, function, announce):
    """Verify ordering for every reachable RCpc Acquire load from `_{function}`
    in an assembled 64-bit ARM64 Mach-O object file.

    Scope: checks the syntactic CFG of the assembled machine code for `PROBE`,
    bounded to each symbol's extent in `(__TEXT,__text)` and following
    same-file `bl` callees and relocated `b` sibling calls interprocedurally
    up to `MAX_CALL_DEPTH`, while treating out-of-object returning calls
    (`bl _ext`, `blr`) as opaque fallthroughs and recognized external
    panic/abort symbols as nonreturning.

    Returns `CheckResult(verdict, detail, missing_fence_after_announce)`:
    - `ORDERED`: every reachable instruction/relocation decoded cleanly, no
      reachable path crosses a function boundary or falls off `(__TEXT,__text)`,
      at least one RCpc load is reachable, and every syntactic CFG path to
      every reachable RCpc load executes `announce` then `dmb ish`.
    - `UNORDERED`: every reachable instruction/relocation decoded cleanly, no
      reachable path crosses a function boundary or falls off `(__TEXT,__text)`,
      and at least one syntactic CFG path reaches an RCpc load without
      `announce` followed by `dmb ish`. `missing_fence_after_announce` is
      `True` iff at least one such syntactic path executed `announce` and then
      reached an RCpc load without `dmb ish`.
    - `UNSUPPORTED`: malformed Mach-O, missing/duplicate target symbol,
      unsupported instruction (`br`/`braa*`/`reta*`/`dsb`/`dmb sy`/zero-reg
      branch/reserved/SVE/SME/LRCPC3) or relocation, recursive or over-deep
      same-file call, reachable fallthrough or branch across a function
      boundary or past `(__TEXT,__text)`, or zero reachable RCpc loads.
    """
    announce = announce.lower()
    if announce not in ANNOUNCE_OPS:
        return CheckResult(Verdict.UNSUPPORTED, f"unsupported announce opcode: {announce}")

    obj, perr = parse_macho_arm64(obj_bytes)
    if perr is not None:
        return CheckResult(Verdict.UNSUPPORTED, perr)

    sym_name = f"_{function}"
    entries = obj.defined_text.get(sym_name, [])
    all_named = [s for s in obj.symtab if (s.n_type & N_STAB) == 0 and s.name == sym_name]
    if len(entries) != 1 or len(all_named) != 1:
        return CheckResult(
            Verdict.UNSUPPORTED,
            f"expected unique {sym_name} definition in (__TEXT,__text), "
            f"found {len(entries)} in __text ({len(all_named)} total)",
        )

    err, seen_rcpc, unord_ann, unord_unann, _, _ = _analyze_from(
        obj, entries[0], 0, announce, {}, set(), 0
    )
    if err is not None:
        return CheckResult(Verdict.UNSUPPORTED, err)
    if not seen_rcpc:
        return CheckResult(Verdict.UNSUPPORTED, f"no reachable RCpc load in {function}")
    if unord_ann is not None:
        return CheckResult(Verdict.UNORDERED, unord_ann, missing_fence_after_announce=True)
    if unord_unann is not None:
        return CheckResult(Verdict.UNORDERED, unord_unann, missing_fence_after_announce=False)
    return CheckResult(Verdict.ORDERED, f"{len(seen_rcpc)} reachable RCpc load(s) ordered")


def verify_function(fenced_obj, unfenced_obj, function, announce):
    pos = check_cfg(fenced_obj, function, announce)
    if pos.verdict is not Verdict.ORDERED:
        raise SystemExit(
            f"sleep ordering: expected ORDERED in {function}, got {pos.verdict.value} ({pos.detail})"
        )
    neg = check_cfg(unfenced_obj, function, announce)
    if neg.verdict is not Verdict.UNORDERED or not neg.missing_fence_after_announce:
        raise SystemExit(
            f"sleep ordering: expected UNORDERED (syntactic path after {announce} without dmb ish) "
            f"negative control in {function}, got {neg.verdict.value} ({neg.detail})"
        )
    return pos, neg


def main():
    src = Path(__file__).resolve().parents[1] / "threads/src"
    sources = {name: (src / name).read_text() for name in ("waker.rs", "park.rs")}
    # Reuse the actual SME2 announcement; compile only this prelude, not
    # the hardware-dependent worker loop (which needs the Apple SDK).
    lanes = (src / "lanes.rs").read_text()
    advertisement = re.search(
        r"(SME2_PARKED\.store\(true, Ordering::SeqCst\);\s+"
        r"core::sync::atomic::fence\(Ordering::SeqCst\);)\s+if sources::visible\(true\)",
        lanes,
    )
    if advertisement is None:
        raise SystemExit("sleep ordering: SME2 must order its announcement before the work re-check")
    probe = PROBE.replace("// SME2_ADVERTISEMENT", advertisement[1].replace("SME2_PARKED", "flag"))
    paths = {
        "waker.rs": [("register_then_recheck", "stlr")],
        "park.rs": [("park_then_recheck", "ldsetal")],
        "probe.rs": [("sme2_then_recheck", "stlrb")],
    }
    with tempfile.TemporaryDirectory(prefix="sleep-order-") as tmp:
        directory = Path(tmp)
        sources["probe.rs"] = probe
        obj = compile_probe(sources, directory)
        for name, checks in paths.items():
            prefix = "core::sync::atomic::" if name == "probe.rs" else ""
            fence = prefix + "fence(Ordering::SeqCst);"
            if sources[name].count(fence) != 1:
                raise SystemExit(f"sleep ordering: expected one SeqCst fence in {name}")
            unfenced = compile_probe({**sources, name: sources[name].replace(fence, "")}, directory)
            for function, announce in checks:
                verify_function(obj, unfenced, function, announce)
                print(
                    f"sleep ordering: {function} binary CFG-ordered ({announce}/dmb ish/RCpc); "
                    f"unfenced control has syntactic path missing dmb ish"
                )


if __name__ == "__main__":
    main()
