#!/usr/bin/env python3
"""Unit tests for binary Mach-O ARM64 CFG verifier tools/check-waker-order.py (#571)."""
import importlib.util
from pathlib import Path
import struct
import unittest


def _load_module():
    path = Path(__file__).resolve().parent / "check-waker-order.py"
    spec = importlib.util.spec_from_file_location("check_waker_order", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


cwo = _load_module()
Verdict = cwo.Verdict


# Exact AArch64 32-bit instruction encodings (verified against LLVM apple-m1).
NOP = 0xD503201F
ISB = 0xD5033FDF
BTI_C = 0xD503245F
PACIASP = 0xD503233F
AUTIASP = 0xD50323BF
RET = 0xD65F03C0
RET_X8 = 0xD65F0100
RETAA = 0xD65F0BFF
BR_X8 = 0xD61F0100
BRAAZ_X8 = 0xD61F091F
BLRAAZ_X8 = 0xD63F091F
SVC_0 = 0xD4000001
BRK_1 = 0xD4200020
UDF_0 = 0x00000000

MOV_W8_1 = 0x52800028
ADD_X0_X0_1 = 0x91000400
LDR_X8_X0 = 0xF9400008
LDAR_X8_X0 = 0xC8DFFC08

STLR_W8_X0 = 0x889FFC08
STLR_X8_X0 = 0xC89FFC08
STLRB_W8_X0 = 0x089FFC08
LDSETAL_W8_W9_X0 = 0xB8E83009
LDSETAL_X8_X9_X0 = 0xF8E83009
LDCLRAL_X8_X9_X0 = 0xF8E81009

LDAPR_W8_X0 = 0xB8BFC008
LDAPR_X8_X0 = 0xF8BFC008
LDAPRB_W8_X0 = 0x38BFC008
LDAPRH_W8_X0 = 0x78BFC008
LDAPUR_X8_X0_8 = 0xD9408008
LDAPUR_W8_X0_8 = 0x99408008
LDAPURB_W8_X0_8 = 0x19408008
LDAPURH_W8_X0_8 = 0x59408008
LDAPURSB_X8_X0_8 = 0x19808008
LDAPURSB_W8_X0_8 = 0x19C08008
LDAPURSH_X8_X0_8 = 0x59808008
LDAPURSH_W8_X0_8 = 0x59C08008
LDAPURSW_X8_X0_8 = 0x99808008
STLUR_X8_X0_8 = 0xD9008008
LDAP1_V0_X0 = 0x0D418400
LDIAPP_SAMPLE = 0xD9400808  # FEAT_LRCPC3 0x19000000 space (bits[11:10] != 0)
LRCPC3_SIMD_SAMPLE = 0xDD400808


def dmb(crm):
    return 0xD50330BF | ((crm & 0xF) << 8)


def dsb(crm):
    return 0xD503309F | ((crm & 0xF) << 8)


DMB_ISH = dmb(0xB)


def b_imm(delta_insns):
    return 0x14000000 | (delta_insns & 0x03FFFFFF)


def bl_imm(delta_insns):
    return 0x94000000 | (delta_insns & 0x03FFFFFF)


def b_cond(cond, delta_insns):
    return 0x54000000 | ((delta_insns & 0x7FFFF) << 5) | (cond & 0xF)


def bc_cond(cond, delta_insns):
    return 0x54000010 | ((delta_insns & 0x7FFFF) << 5) | (cond & 0xF)


def cbz(rt, delta_insns, is_64=True):
    return (0xB4000000 if is_64 else 0x34000000) | ((delta_insns & 0x7FFFF) << 5) | (rt & 0x1F)


def cbnz(rt, delta_insns, is_64=True):
    return (0xB5000000 if is_64 else 0x35000000) | ((delta_insns & 0x7FFFF) << 5) | (rt & 0x1F)


def tbz(rt, bit, delta_insns):
    b5 = (bit >> 5) & 1
    b40 = bit & 0x1F
    return (b5 << 31) | 0x36000000 | (b40 << 19) | ((delta_insns & 0x3FFF) << 5) | (rt & 0x1F)


def tbnz(rt, bit, delta_insns):
    b5 = (bit >> 5) & 1
    b40 = bit & 0x1F
    return (b5 << 31) | 0x37000000 | (b40 << 19) | ((delta_insns & 0x3FFF) << 5) | (rt & 0x1F)


def blr(rn):
    return 0xD63F0000 | ((rn & 0x1F) << 5)


def build_macho_arm64(words, symbols, relocs=()):
    """Build a minimal valid 64-bit little-endian ARM64 Mach-O MH_OBJECT.

    `symbols`: list of `(name, insn_index_or_None)` — `None` creates an
      external `N_UNDF` symbol; an int creates an `N_SECT` symbol in
      `(__TEXT,__text)` at `insn_index * 4`.
    `relocs`: list of `(insn_index, sym_index, r_type, r_pcrel, r_length, r_extern)`.
    """
    text_bytes = struct.pack(f"<{len(words)}I", *words)
    text_size = len(text_bytes)

    seg_cmd_size = 72 + 80
    sym_cmd_size = 24
    sizeofcmds = seg_cmd_size + sym_cmd_size
    text_foff = 32 + sizeofcmds
    reloff = text_foff + text_size
    reloc_bytes = b"".join(
        struct.pack(
            "<iI",
            insn_idx * 4,
            (sym_idx & 0x00FFFFFF)
            | ((r_pcrel & 1) << 24)
            | ((r_length & 3) << 25)
            | ((r_extern & 1) << 27)
            | ((r_type & 0xF) << 28),
        )
        for insn_idx, sym_idx, r_type, r_pcrel, r_length, r_extern in relocs
    )
    symoff = reloff + len(reloc_bytes)

    strtab = bytearray(b"\0")
    sym_entries = []
    for name, insn_idx in symbols:
        strx = len(strtab)
        strtab.extend(name.encode("utf-8") + b"\0")
        if insn_idx is None:
            sym_entries.append(struct.pack("<IBBHQ", strx, 0x01, 0, 0, 0))
        else:
            sym_entries.append(struct.pack("<IBBHQ", strx, 0x0F, 1, 0, insn_idx * 4))
    symtab_bytes = b"".join(sym_entries)
    stroff = symoff + len(symtab_bytes)

    header = struct.pack(
        "<IiiIIIII",
        cwo.MH_MAGIC_64,
        cwo.CPU_TYPE_ARM64,
        0,
        cwo.MH_OBJECT,
        2,
        sizeofcmds,
        0,
        0,
    )
    seg_cmd = struct.pack(
        "<II16sQQQQiiII",
        cwo.LC_SEGMENT_64,
        seg_cmd_size,
        b"",
        0,
        text_size,
        text_foff,
        text_size,
        7,
        7,
        1,
        0,
    )
    sect_cmd = struct.pack(
        "<16s16sQQIIIIIIII",
        b"__text",
        b"__TEXT",
        0,
        text_size,
        text_foff,
        2,
        reloff,
        len(relocs),
        0x80000400,
        0,
        0,
        0,
    )
    sym_cmd = struct.pack(
        "<IIIIII",
        cwo.LC_SYMTAB,
        sym_cmd_size,
        symoff,
        len(symbols),
        stroff,
        len(strtab),
    )
    return header + seg_cmd + sect_cmd + sym_cmd + text_bytes + reloc_bytes + symtab_bytes + bytes(strtab)


class CheckWakerOrderBinaryTests(unittest.TestCase):
    def test_linear_ordered_and_unfenced_unordered(self):
        fenced = build_macho_arm64(
            [MOV_W8_1, STLR_X8_X0, DMB_ISH, LDAPR_X8_X0, RET],
            [("_f", 0)],
        )
        unfenced = build_macho_arm64(
            [MOV_W8_1, STLR_X8_X0, LDAPR_X8_X0, RET],
            [("_f", 0)],
        )
        self.assertEqual(cwo.check_cfg(fenced, "f", "stlr").verdict, Verdict.ORDERED)
        res = cwo.check_cfg(unfenced, "f", "stlr")
        self.assertEqual(res.verdict, Verdict.UNORDERED)
        self.assertTrue(res.missing_fence_after_announce)
        cwo.verify_function(fenced, unfenced, "f", "stlr")

    def test_all_three_announce_opcodes(self):
        for name, insn in (
            ("stlr", STLR_W8_X0),
            ("stlr", STLR_X8_X0),
            ("stlrb", STLRB_W8_X0),
            ("ldsetal", LDSETAL_W8_W9_X0),
            ("ldsetal", LDSETAL_X8_X9_X0),
        ):
            fenced = build_macho_arm64([insn, DMB_ISH, LDAPR_X8_X0, RET], [("_f", 0)])
            unfenced = build_macho_arm64([insn, LDAPR_X8_X0, RET], [("_f", 0)])
            cwo.verify_function(fenced, unfenced, "f", name)

        # Unrelated RMW (`ldclral`) must not satisfy `ldsetal`.
        wrong = build_macho_arm64([LDCLRAL_X8_X9_X0, DMB_ISH, LDAPR_X8_X0, RET], [("_f", 0)])
        res = cwo.check_cfg(wrong, "f", "ldsetal")
        self.assertEqual(res.verdict, Verdict.UNORDERED)
        self.assertFalse(res.missing_fence_after_announce)

        # Unsupported announce opcode rejected.
        self.assertEqual(cwo.check_cfg(wrong, "f", "mov").verdict, Verdict.UNSUPPORTED)

    def test_diamond_cfg_both_arms_ordered_vs_one_arm_missing_fence(self):
        # 0: cbz x0, +4 (Lslow=4)
        # 1: ldsetal
        # 2: dmb ish
        # 3: b +3 (Lrecheck=6)
        # 4: ldsetal
        # 5: dmb ish
        # 6: ldapr
        # 7: ret
        ordered = build_macho_arm64(
            [cbz(0, 4), LDSETAL_X8_X9_X0, DMB_ISH, b_imm(3), LDSETAL_X8_X9_X0, DMB_ISH, LDAPR_X8_X0, RET],
            [("_park_then_recheck", 0)],
        )
        self.assertEqual(cwo.check_cfg(ordered, "park_then_recheck", "ldsetal").verdict, Verdict.ORDERED)

        # Replace instruction 5 (slow arm `dmb ish`) with `nop`.
        unordered = build_macho_arm64(
            [cbz(0, 4), LDSETAL_X8_X9_X0, DMB_ISH, b_imm(3), LDSETAL_X8_X9_X0, NOP, LDAPR_X8_X0, RET],
            [("_park_then_recheck", 0)],
        )
        res = cwo.check_cfg(unordered, "park_then_recheck", "ldsetal")
        self.assertEqual(res.verdict, Verdict.UNORDERED)
        self.assertTrue(res.missing_fence_after_announce)

    def test_conditional_branches_and_b_al_vs_b_nv_and_bc_cond(self):
        # Every standard condition code 0x0..0xD (`b.eq`..`b.le`) skipping `dmb ish` -> UNORDERED.
        for cond in range(0xE):
            asm = build_macho_arm64(
                [STLR_X8_X0, b_cond(cond, 2), DMB_ISH, LDAPR_X8_X0, RET],
                [("_f", 0)],
            )
            self.assertEqual(cwo.check_cfg(asm, "f", "stlr").verdict, Verdict.UNORDERED, f"cond={cond}")

        # `b.al` (cond=0xE) is always taken, skipping an unreachable unfenced load.
        b_al_ok = build_macho_arm64(
            [STLR_X8_X0, b_cond(0xE, 2), LDAPR_X8_X0, DMB_ISH, LDAPR_X8_X0, RET],
            [("_f", 0)],
        )
        self.assertEqual(cwo.check_cfg(b_al_ok, "f", "stlr").verdict, Verdict.ORDERED)

        # `b.nv` (cond=0xF) and Armv8.8 `bc.eq` fail closed as UNSUPPORTED.
        for bad in (b_cond(0xF, 1), bc_cond(0, 1)):
            obj = build_macho_arm64([STLR_X8_X0, DMB_ISH, bad, LDAPR_X8_X0, RET], [("_f", 0)])
            self.assertEqual(cwo.check_cfg(obj, "f", "stlr").verdict, Verdict.UNSUPPORTED)

    def test_cbz_cbnz_tbz_tbnz_and_zero_register_rejection(self):
        for branch in (cbz(2, 2), cbnz(2, 2, is_64=False), tbz(2, 63, 2), tbnz(2, 31, 2)):
            obj = build_macho_arm64([STLR_X8_X0, branch, DMB_ISH, LDAPR_X8_X0, RET], [("_f", 0)])
            self.assertEqual(cwo.check_cfg(obj, "f", "stlr").verdict, Verdict.UNORDERED)

        # Zero-register `xzr`/`wzr` (rt=31) deterministic/impossible branches -> UNSUPPORTED.
        for bad in (
            cbz(31, 2), cbnz(31, 2), cbz(31, 2, is_64=False), cbnz(31, 2, is_64=False),
            tbz(31, 0, 2), tbnz(31, 0, 2),
        ):
            obj = build_macho_arm64([STLR_X8_X0, bad, DMB_ISH, LDAPR_X8_X0, RET], [("_f", 0)])
            self.assertEqual(cwo.check_cfg(obj, "f", "stlr").verdict, Verdict.UNSUPPORTED)

    def test_indirect_authenticated_system_and_reserved_instructions_unsupported(self):
        for bad in (
            BR_X8, BRAAZ_X8, BLRAAZ_X8, RETAA, RET_X8, blr(31),
            SVC_0, BRK_1, UDF_0,
            0xD50330DF,  # non-SY `isb #0`
        ):
            obj = build_macho_arm64([STLR_X8_X0, DMB_ISH, bad, LDAPR_X8_X0, RET], [("_f", 0)])
            self.assertEqual(cwo.check_cfg(obj, "f", "stlr").verdict, Verdict.UNSUPPORTED, hex(bad))

        # Architectural hints (`nop`, `bti c`, `paciasp`, `autiasp`), `isb`, `blr x8` are allowed.
        ok = build_macho_arm64(
            [PACIASP, BTI_C, ISB, STLR_X8_X0, DMB_ISH, blr(8), LDAPR_X8_X0, AUTIASP, RET],
            [("_f", 0)],
        )
        self.assertEqual(cwo.check_cfg(ok, "f", "stlr").verdict, Verdict.ORDERED)

    def test_barrier_strength_classification(self):
        # Weak `dmb` options (insufficient for inner-shareable store->load) -> UNORDERED.
        for crm in sorted(cwo.WEAK_DMB_CRMS):
            obj = build_macho_arm64([STLR_X8_X0, dmb(crm), LDAPR_X8_X0, RET], [("_f", 0)])
            self.assertEqual(cwo.check_cfg(obj, "f", "stlr").verdict, Verdict.UNORDERED, hex(crm))

        # Stronger `dmb` (`osh`, `sy`), reserved `dmb` CRms, and ALL `dsb` forms -> UNSUPPORTED.
        for bad in (
            dmb(0x3), dmb(0xF), dmb(0x0), dmb(0x4), dmb(0x8), dmb(0xC),
            dsb(0xB), dsb(0xA), dsb(0xE), dsb(0x9), dsb(0xF), dsb(0x3),
        ):
            obj = build_macho_arm64([STLR_X8_X0, bad, LDAPR_X8_X0, RET], [("_f", 0)])
            self.assertEqual(cwo.check_cfg(obj, "f", "stlr").verdict, Verdict.UNSUPPORTED, hex(bad))

    def test_rcpc_lrcpc2_and_lrcpc3_encodings(self):
        for load in (
            LDAPR_W8_X0, LDAPR_X8_X0, LDAPRB_W8_X0, LDAPRH_W8_X0,
            LDAPUR_X8_X0_8, LDAPUR_W8_X0_8, LDAPURB_W8_X0_8, LDAPURH_W8_X0_8,
            LDAPURSB_X8_X0_8, LDAPURSB_W8_X0_8, LDAPURSH_X8_X0_8, LDAPURSH_W8_X0_8,
            LDAPURSW_X8_X0_8,
        ):
            fenced = build_macho_arm64([STLR_X8_X0, DMB_ISH, load, RET], [("_f", 0)])
            unfenced = build_macho_arm64([STLR_X8_X0, load, RET], [("_f", 0)])
            cwo.verify_function(fenced, unfenced, "f", "stlr")

        # `stlur` is a store-release (NOT an RCpc load); alone with no RCpc load -> UNSUPPORTED.
        stlur_only = build_macho_arm64([STLR_X8_X0, DMB_ISH, STLUR_X8_X0_8, RET], [("_f", 0)])
        self.assertEqual(cwo.check_cfg(stlur_only, "f", "stlr").verdict, Verdict.UNSUPPORTED)

        # Unmodeled LRCPC3 forms fail closed as UNSUPPORTED.
        for lrcpc3 in (LDAP1_V0_X0, LDIAPP_SAMPLE, LRCPC3_SIMD_SAMPLE):
            obj = build_macho_arm64([STLR_X8_X0, DMB_ISH, lrcpc3, RET], [("_f", 0)])
            self.assertEqual(cwo.check_cfg(obj, "f", "stlr").verdict, Verdict.UNSUPPORTED, hex(lrcpc3))

    def test_second_announce_and_reannounce_loop_are_unordered(self):
        straight = build_macho_arm64(
            [STLR_X8_X0, DMB_ISH, STLR_X8_X0, LDAPR_X8_X0, RET],
            [("_f", 0)],
        )
        self.assertEqual(cwo.check_cfg(straight, "f", "stlr").verdict, Verdict.UNORDERED)

        # 0: stlr; 1: dmb ish; 2: ldapr; 3: cbz x0, +3 (ret=6); 4: stlr; 5: b -3 (loop=2); 6: ret
        loop = build_macho_arm64(
            [STLR_X8_X0, DMB_ISH, LDAPR_X8_X0, cbz(0, 3), STLR_X8_X0, b_imm(-3), RET],
            [("_f", 0)],
        )
        self.assertEqual(cwo.check_cfg(loop, "f", "stlr").verdict, Verdict.UNORDERED)

    def test_same_file_callees_direct_and_relocated(self):
        # 1. Direct PC-relative `bl` (no relocation) to unfenced helper before fence -> UNORDERED.
        direct_bad = build_macho_arm64(
            [STLR_X8_X0, bl_imm(4), DMB_ISH, LDAPR_X8_X0, RET, LDAPR_X8_X0, RET],
            [("_f", 0)],
        )
        self.assertEqual(cwo.check_cfg(direct_bad, "f", "stlr").verdict, Verdict.UNORDERED)

        # 2. Relocated `bl _sub` before fence vs after fence (as `park_then_recheck` calls `wake_one_of`).
        reloc_before = build_macho_arm64(
            [STLR_X8_X0, bl_imm(0), DMB_ISH, LDAPR_X8_X0, RET, LDAPR_X8_X0, RET],
            [("_f", 0), ("_sub", 5)],
            [(1, 1, cwo.ARM64_RELOC_BRANCH26, 1, 2, 1)],
        )
        reloc_after = build_macho_arm64(
            [STLR_X8_X0, DMB_ISH, bl_imm(0), LDAPR_X8_X0, RET, LDAPR_X8_X0, RET],
            [("_f", 0), ("_sub", 5)],
            [(2, 1, cwo.ARM64_RELOC_BRANCH26, 1, 2, 1)],
        )
        self.assertEqual(cwo.check_cfg(reloc_before, "f", "stlr").verdict, Verdict.UNORDERED)
        self.assertEqual(cwo.check_cfg(reloc_after, "f", "stlr").verdict, Verdict.ORDERED)

        # 3. Callee that fences before its own load -> ORDERED (callee-only load counted!).
        callee_fences = build_macho_arm64(
            [STLR_X8_X0, bl_imm(0), RET, DMB_ISH, LDAPR_X8_X0, RET],
            [("_f", 0), ("_sub", 3)],
            [(1, 1, cwo.ARM64_RELOC_BRANCH26, 1, 2, 1)],
        )
        self.assertEqual(cwo.check_cfg(callee_fences, "f", "stlr").verdict, Verdict.ORDERED)

        # 4. Callee with unreachable load skipped by `b` -> ORDERED (no false UNORDERED).
        callee_dead_load = build_macho_arm64(
            [STLR_X8_X0, bl_imm(0), DMB_ISH, LDAPR_X8_X0, RET, b_imm(2), LDAPR_X8_X0, RET],
            [("_f", 0), ("_sub", 5)],
            [(1, 1, cwo.ARM64_RELOC_BRANCH26, 1, 2, 1)],
        )
        self.assertEqual(cwo.check_cfg(callee_dead_load, "f", "stlr").verdict, Verdict.ORDERED)

        # 5. Infinite-loop callee never returns -> caller load after `bl` unreachable -> UNSUPPORTED.
        callee_loops = build_macho_arm64(
            [STLR_X8_X0, DMB_ISH, bl_imm(0), LDAPR_X8_X0, RET, b_imm(0)],
            [("_f", 0), ("_spin", 5)],
            [(2, 1, cwo.ARM64_RELOC_BRANCH26, 1, 2, 1)],
        )
        self.assertEqual(cwo.check_cfg(callee_loops, "f", "stlr").verdict, Verdict.UNSUPPORTED)

        # 6. Same-file `_abort: ret` resolves as returning callee, NOT external noreturn!
        local_abort = build_macho_arm64(
            [
                cbz(0, 5), STLR_X8_X0, DMB_ISH, LDAPR_X8_X0, RET,
                STLR_X8_X0, bl_imm(0), LDAPR_X8_X0, RET,
                RET,
            ],
            [("_f", 0), ("_abort", 9)],
            [(6, 1, cwo.ARM64_RELOC_BRANCH26, 1, 2, 1)],
        )
        self.assertEqual(cwo.check_cfg(local_abort, "f", "stlr").verdict, Verdict.UNORDERED)

        # 7. External recognized noreturn panic call prunes dead continuation cleanly.
        ext_panic = build_macho_arm64(
            [cbz(0, 5), LDSETAL_X8_X9_X0, DMB_ISH, LDAPR_X8_X0, RET, bl_imm(0), LDAPR_X8_X0, RET],
            [("_f", 0), ("__RNvNtCs8Mbv00yxnRz_4core9panicking18panic_bounds_check", None)],
            [(5, 1, cwo.ARM64_RELOC_BRANCH26, 1, 2, 1)],
        )
        self.assertEqual(cwo.check_cfg(ext_panic, "f", "ldsetal").verdict, Verdict.ORDERED)

        # 8. Unrecognized external name containing `panic_` is treated as returning.
        fake_panic = build_macho_arm64(
            [
                cbz(0, 5), STLR_X8_X0, DMB_ISH, LDAPR_X8_X0, RET,
                STLR_X8_X0, bl_imm(0), LDAPR_X8_X0, RET,
            ],
            [("_f", 0), ("_my9panicking12panic_return", None)],
            [(6, 1, cwo.ARM64_RELOC_BRANCH26, 1, 2, 1)],
        )
        self.assertEqual(cwo.check_cfg(fake_panic, "f", "stlr").verdict, Verdict.UNORDERED)

    def test_call_depth_limit_holds_even_with_primed_cache_and_deep_chains(self):
        # Primed-cache test: `_f` calls `_s0`, `_s1`, ..., `_s17` in order,
        # where each `_sN` calls `_s(N-1)`. Even though `_s0.._s15` prime the
        # cache first, `_s16`/`_s17` exceed MAX_CALL_DEPTH (16) -> UNSUPPORTED.
        words = [STLR_X8_X0, DMB_ISH]
        symbols = [("_f", 0)]
        relocs = []
        # Reserve 18 `bl _sK` instructions in `_f`, then `ldapr`, `ret`.
        for k in range(18):
            relocs.append((len(words), k + 1, cwo.ARM64_RELOC_BRANCH26, 1, 2, 1))
            words.append(bl_imm(0))
        words.extend([LDAPR_X8_X0, RET])
        for k in range(18):
            symbols.append((f"_s{k}", len(words)))
            if k > 0:
                relocs.append((len(words), k, cwo.ARM64_RELOC_BRANCH26, 1, 2, 1))
                words.append(bl_imm(0))
            words.append(RET)
        primed = build_macho_arm64(words, symbols, relocs)
        self.assertEqual(cwo.check_cfg(primed, "f", "stlr").verdict, Verdict.UNSUPPORTED)

        # 1,050-deep linear chain returns UNSUPPORTED without RecursionError.
        words = [STLR_X8_X0, DMB_ISH, bl_imm(0), LDAPR_X8_X0, RET]
        symbols = [("_f", 0)]
        relocs = [(2, 1, cwo.ARM64_RELOC_BRANCH26, 1, 2, 1)]
        for k in range(1050):
            symbols.append((f"_c{k}", len(words)))
            if k < 1049:
                relocs.append((len(words), k + 2, cwo.ARM64_RELOC_BRANCH26, 1, 2, 1))
                words.append(bl_imm(0))
            words.append(RET)
        deep = build_macho_arm64(words, symbols, relocs)
        self.assertEqual(cwo.check_cfg(deep, "f", "stlr").verdict, Verdict.UNSUPPORTED)

        # Direct recursion returns UNSUPPORTED.
        rec = build_macho_arm64(
            [STLR_X8_X0, DMB_ISH, bl_imm(-2), LDAPR_X8_X0, RET],
            [("_f", 0)],
        )
        self.assertEqual(cwo.check_cfg(rec, "f", "stlr").verdict, Verdict.UNSUPPORTED)

    def test_fallthrough_and_relocation_edge_cases(self):
        # Fallthrough into adjacent code without `ret` executes the adjacent
        # unfenced `stlr` + `ldapr` -> UNORDERED (matches hardware execution).
        fall_into_next = build_macho_arm64(
            [STLR_X8_X0, DMB_ISH, LDAPR_X8_X0, STLR_X8_X0, LDAPR_X8_X0, RET],
            [("_f", 0), ("_other", 3)],
        )
        self.assertEqual(cwo.check_cfg(fall_into_next, "f", "stlr").verdict, Verdict.UNORDERED)

        # Fallthrough past end of `(__TEXT,__text)` -> UNSUPPORTED, even when
        # another branch already saw an UNORDERED load.
        fall_off_end = build_macho_arm64(
            [STLR_X8_X0, cbz(2, 2), RET, LDAPR_X8_X0],
            [("_f", 0)],
        )
        self.assertEqual(cwo.check_cfg(fall_off_end, "f", "stlr").verdict, Verdict.UNSUPPORTED)

        # Relocated `b _ext` tail call, `bl` with non-zero imm26 addend, or
        # `bl` with paired ADDEND relocation -> UNSUPPORTED.
        reloc_tail = build_macho_arm64(
            [STLR_X8_X0, DMB_ISH, b_imm(0), LDAPR_X8_X0, RET],
            [("_f", 0), ("_ext", None)],
            [(2, 1, cwo.ARM64_RELOC_BRANCH26, 1, 2, 1)],
        )
        bl_nonzero_imm = build_macho_arm64(
            [STLR_X8_X0, DMB_ISH, bl_imm(2), LDAPR_X8_X0, RET],
            [("_f", 0), ("_ext", None)],
            [(2, 1, cwo.ARM64_RELOC_BRANCH26, 1, 2, 1)],
        )
        bl_with_addend = build_macho_arm64(
            [STLR_X8_X0, DMB_ISH, bl_imm(0), LDAPR_X8_X0, RET],
            [("_f", 0), ("_ext", None)],
            [
                (2, 1, 10, 0, 2, 0),  # ARM64_RELOC_ADDEND
                (2, 1, cwo.ARM64_RELOC_BRANCH26, 1, 2, 1),
            ],
        )
        for bad in (reloc_tail, bl_nonzero_imm, bl_with_addend):
            self.assertEqual(cwo.check_cfg(bad, "f", "stlr").verdict, Verdict.UNSUPPORTED)

        # Out-of-section branch target -> UNSUPPORTED.
        oob_branch = build_macho_arm64([STLR_X8_X0, b_imm(10), DMB_ISH, LDAPR_X8_X0, RET], [("_f", 0)])
        self.assertEqual(cwo.check_cfg(oob_branch, "f", "stlr").verdict, Verdict.UNSUPPORTED)

    def test_unsupported_takes_precedence_over_unordered_and_malformed_macho(self):
        both = build_macho_arm64(
            [STLR_X8_X0, cbz(2, 2), BRAAZ_X8, LDAPR_X8_X0, RET],
            [("_f", 0)],
        )
        self.assertEqual(cwo.check_cfg(both, "f", "stlr").verdict, Verdict.UNSUPPORTED)

        # Missing/duplicate symbols and corrupt Mach-O objects -> UNSUPPORTED.
        dup_sym = build_macho_arm64([STLR_X8_X0, DMB_ISH, LDAPR_X8_X0, RET], [("_f", 0), ("_f", 0)])
        self.assertEqual(cwo.check_cfg(dup_sym, "f", "stlr").verdict, Verdict.UNSUPPORTED)
        self.assertEqual(cwo.check_cfg(dup_sym, "missing", "stlr").verdict, Verdict.UNSUPPORTED)
        self.assertEqual(cwo.check_cfg(b"not a mach-o", "f", "stlr").verdict, Verdict.UNSUPPORTED)

    def test_verify_function_3way_contract(self):
        ordered = build_macho_arm64([STLR_X8_X0, DMB_ISH, LDAPR_X8_X0, RET], [("_f", 0)])
        unordered_ann = build_macho_arm64([STLR_X8_X0, LDAPR_X8_X0, RET], [("_f", 0)])
        unordered_no_ann = build_macho_arm64([LDAPR_X8_X0, RET], [("_f", 0)])
        unsupported = build_macho_arm64([STLR_X8_X0, BRAAZ_X8, LDAPR_X8_X0, RET], [("_f", 0)])
        stronger = build_macho_arm64([STLR_X8_X0, dmb(0xF), LDAPR_X8_X0, RET], [("_f", 0)])

        cwo.verify_function(ordered, unordered_ann, "f", "stlr")

        # An UNSUPPORTED negative control (`dmb sy`, `braaz`) OR an UNORDERED
        # control with no announcement MUST fail.
        for pos, neg in (
            (ordered, unsupported),
            (ordered, stronger),
            (ordered, unordered_no_ann),
            (ordered, ordered),
            (unordered_ann, unordered_ann),
            (unsupported, unordered_ann),
        ):
            with self.assertRaises(SystemExit):
                cwo.verify_function(pos, neg, "f", "stlr")


if __name__ == "__main__":
    unittest.main()
