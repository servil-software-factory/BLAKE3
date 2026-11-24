#!/usr/bin/env python3
"""Formally prove c/blake3_output_sme2_aarch64.S (blake3_servil_sme2_chunks_keyed)
and c/blake3_output_neon_aarch64.S (blake3_servil_neon_output8) (#336) equal to
BLAKE3's Lean specification for ALL symbolic blocks, keys/CVs, lengths, flags,
and independent 64-bit counters:
- blake3_servil_sme2_chunks_keyed: concrete groups 0, 1, 2, 3, 4 PLUS symbolic
  group-loop induction covering EVERY group count from 2 to 2^40.
- blake3_servil_neon_output8: every callable group count 0..=16 (up to 128
  symbolic lanes, matching FINALIZE_BATCH/8 = MAX_PARENTS/8 = 16).
- 3 rejecting fault-injection controls (upper-counter truncation and
  group-count masking in both kernels).

Explicit proof assumptions (matching tools/verify/prove_sme2.py):
- Calling thread SVL is 512 bits (16 32-bit lanes) for the SME2 kernel;
  non-16 SVL early-return behavior is verified separately in runtime tests.
- Calling thread enters with TPIDR2_EL0 == 0 (no dormant caller ZA); lazy ZA
  saving and nonzero reserved-byte refusal are verified on real SME2 silicon
  in `src/ffi_sme2.rs::dormant_caller_za_is_lazily_saved_by_every_sme2_kernel`.
- Input/descriptor/key/counter/flag/output memory regions do not alias each
  other, and AAPCS64 callee-saved registers, SP, and SME exit state (smstop
  za / smstop sm) are checked on return.
"""
import os
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)

import aarch64
import induction
import lean_spec as spec
import prove_hybrid
import prove_sme2
import workers
from aarch64 import Ptr, Region, Unproved
from prove_hybrid import Setup, byte_symbols, same
from z3 import Array, BitVec, BitVecSort, BitVecVal, Extract, Select, UGE, ULE, ULT, ZeroExt, simplify

CLANG = next(c for c in ("clang-19", "clang-18", "clang-17", "clang") if shutil.which(c))
MAX_NEON_GROUPS = 16


def assemble_text(asm_text, tag, march=None):
    tmp = tempfile.mkdtemp()
    src = os.path.join(tmp, tag + ".S")
    obj = os.path.join(tmp, tag + ".o")
    with open(src, "w", encoding="utf-8") as f:
        f.write(asm_text)
    aarch64.OBJDUMP = aarch64.LLVM_OBJDUMP or "llvm-objdump"
    cmd = [CLANG, "-c", "--target=aarch64-linux-gnu"]
    if march:
        cmd.append(f"-march={march}")
    cmd += [src, "-o", obj]
    subprocess.run(cmd, check=True)
    return obj


def assemble(src_name, march=None):
    obj = os.path.join(tempfile.mkdtemp(), src_name + ".o")
    src = os.path.join(ROOT, "c", src_name)
    aarch64.OBJDUMP = aarch64.LLVM_OBJDUMP or "llvm-objdump"
    cmd = [CLANG, "-c", "--target=aarch64-linux-gnu"]
    if march:
        cmd.append(f"-march={march}")
    cmd += [src, "-o", obj]
    subprocess.run(cmd, check=True)
    return obj


def prove_output_kernel(obj, name, lpg, groups):
    n = lpg * groups
    cvs = [byte_symbols(f"cv{i}_", 32) for i in range(n)]
    blocks = [byte_symbols(f"blk{i}_", 64) for i in range(n)]
    lens = [BitVec(f"last_len_{i}", 8) for i in range(n)]
    flags = [BitVec(f"flags_{i}", 8) for i in range(n)]

    regions = {"out": Region("out", max(32 * n, 1), True, None)}
    table = Region("descriptors", max(24 * n, 1), False, None, alignment=8)
    for i in range(n):
        regions[f"blk{i}"] = Region(f"blk{i}", 64, False, lambda o, i=i: blocks[i][o])
        regions[f"cv{i}"] = Region(f"cv{i}", 32, False, lambda o, i=i: cvs[i][o])
        for j in range(8):
            table.bytes[24 * i + j] = (Ptr(f"blk{i}", 0), j)
            table.bytes[24 * i + 8 + j] = (Ptr(f"cv{i}", 0), j)
        table.bytes[24 * i + 16] = lens[i]
        table.bytes[24 * i + 17] = flags[i]
    regions["descriptors"] = table

    real = aarch64.disassemble
    aarch64.disassemble = lambda o, a, b: real(o, a, b, llvm=True)
    try:
        s = Setup(obj, name, name + "_end", regions)
    finally:
        aarch64.disassemble = real

    m = s.m
    m.x[0] = Ptr("descriptors", 0)
    m.x[1] = Ptr("out", 0)
    m.x[2] = BitVecVal(groups, 64)
    s.run_and_check_abi()

    out = m.regions["out"]
    if groups == 0:
        if out.bytes:
            raise Unproved(f"{name}: groups=0 wrote output bytes")
        return m.steps

    if any(i not in out.bytes for i in range(32 * n)):
        raise Unproved(f"{name}: output not wholly written")

    ctr0 = BitVecVal(0, 64)
    for i in range(n):
        want = spec.compress(
            [simplify(w) for w in spec.words(cvs[i])],
            [simplify(w) for w in spec.words(blocks[i])],
            ctr0,
            ZeroExt(24, lens[i]),
            ZeroExt(24, flags[i]),
        )[:8]
        got = spec.words([out.bytes[32 * i + j] for j in range(32)])
        for wi in range(8):
            if not same(got[wi], want[wi]):
                raise Unproved(f"{name} groups={groups}: lane {i} word {wi} differs!")
    return m.steps


def prove_chunks_keyed(obj, name, groups, check=True):
    n = 16 * groups
    # Fully symbolic 1024-byte chunks, independent symbolic keys, mode flags,
    # and independent symbolic 64-bit starting counters per group.
    chunks = [byte_symbols(f"ch{i}_", 1024) for i in range(n)]
    keys = [byte_symbols(f"k{g}_", 32) for g in range(groups)]
    g_flags = [BitVec(f"flags_{g}", 8) for g in range(groups)]
    g_ctrs = [BitVec(f"counters_{g}", 64) for g in range(groups)]

    regions = {"out": Region("out", max(32 * n, 1), True, None)}
    ptrs = Region("ptrs", max(8 * n, 1), False, None, alignment=8)
    for i in range(n):
        regions[f"ch{i}"] = Region(f"ch{i}", 1024, False, lambda o, i=i: chunks[i][o])
        for j in range(8):
            ptrs.bytes[8 * i + j] = (Ptr(f"ch{i}", 0), j)
    regions["ptrs"] = ptrs

    ctrs_bytes = [Extract(8 * b + 7, 8 * b, g_ctrs[g]) for g in range(groups) for b in range(8)]
    regions["ctrs"] = Region("ctrs", max(8 * groups, 1), False, lambda o: ctrs_bytes[o] if groups else BitVecVal(0, 8), alignment=8)

    keys_flat = [b for k in keys for b in k]
    regions["keys"] = Region("keys", max(32 * groups, 1), False, lambda o: keys_flat[o] if groups else BitVecVal(0, 8), alignment=4)
    regions["flags"] = Region("flags", max(groups, 1), False, lambda o: g_flags[o] if groups else BitVecVal(0, 8))

    real = aarch64.disassemble
    aarch64.disassemble = lambda o, a, b: real(o, a, b, llvm=True)
    try:
        s = Setup(obj, name, name + "_end", regions)
    finally:
        aarch64.disassemble = real

    m = s.m
    m.x[0] = Ptr("ptrs", 0)
    m.x[1] = Ptr("ctrs", 0)
    m.x[2] = Ptr("keys", 0)
    m.x[3] = Ptr("flags", 0)
    m.x[4] = Ptr("out", 0)
    m.x[5] = BitVecVal(groups, 64)
    s.run_and_check_abi()

    if not same(m.x[0], BitVecVal(16, 64)):
        raise Unproved(f"{name}: expected return 16, got {m.x[0]}")
    if getattr(m, "streaming", False) or getattr(m, "za", None) is not None:
        raise Unproved(f"{name}: SME streaming/ZA left enabled")

    if not check:
        return m.steps

    out = m.regions["out"]
    if groups == 0:
        if out.bytes:
            raise Unproved(f"{name}: groups=0 wrote output bytes")
        return m.steps

    if any(i not in out.bytes for i in range(32 * n)):
        raise Unproved(f"{name}: output not wholly written")

    for g in range(groups):
        cs = [g_ctrs[g] + lane for lane in range(16)]
        want_cvs = prove_sme2.chunk_cvs(
            chunks[16 * g : 16 * (g + 1)],
            keys[g],
            cs,
            g_flags[g],
            BitVecVal(1, 8),
            BitVecVal(2, 8),
        )
        for lane, cv in enumerate(want_cvs):
            idx = 16 * g + lane
            got = spec.words([out.bytes[32 * idx + j] for j in range(32)])
            for wi in range(8):
                if not same(got[wi], cv[wi]):
                    raise Unproved(f"{name} groups={groups}: group {g} lane {lane} word {wi} differs!")
    return m.steps


def prove_chunks_keyed_every(obj):
    """blake3_servil_sme2_chunks_keyed for every group count g in 2..2^40,
    by symbolic induction over its outer group loop (as prove_sme2.py):
    group K hashes 16 independent 1024-byte chunks at counters_K + 0..15
    under independent key_K and mode flags_K into out[512 K .. 512 K + 512],
    writing no other bytes and restoring AAPCS64/SME exit state at K = g - 1."""
    name = "blake3_servil_sme2_chunks_keyed"
    g, k = BitVec("len_groups", 64), BitVec("len_k", 64)
    lanes = [Array(f"ch{l}", BitVecSort(64), BitVecSort(8)) for l in range(16)]
    key0 = byte_symbols("key0_", 32)
    keyk = byte_symbols("keyk_", 32)
    ctr0, ctrk = BitVec("counters_0", 64), BitVec("counters_k", 64)
    flg0, flgk = BitVec("flags_0", 8), BitVec("flags_k", 8)

    regions = {}
    for l in range(16):
        r = Region(f"lane{l}", g * 1024, False, (lambda a: lambda o: Select(a, aarch64.as_bv(o)))(lanes[l]))
        r.symbolic_initial = True
        regions[f"lane{l}"] = r

    def ptr_entry(offset):
        if isinstance(offset, int) and 0 <= offset < 128:
            return (Ptr(f"lane{offset // 8}", 0), offset % 8)
        within = aarch64.concrete(aarch64.as_bv(offset) - 128 * k)
        if within is not None and 0 <= within < 128:
            return (Ptr(f"lane{within // 8}", simplify(k * 1024, som=True, bv_sort_ac=True)), within % 8)
        raise Unproved(f"ptrs read outside current group: {offset}")

    def ctr_entry(offset):
        if isinstance(offset, int) and 0 <= offset < 8:
            return Extract(8 * offset + 7, 8 * offset, ctr0)
        within = aarch64.concrete(aarch64.as_bv(offset) - 8 * k)
        if within is not None and 0 <= within < 8:
            return Extract(8 * within + 7, 8 * within, ctrk)
        raise Unproved(f"ctrs read outside current group: {offset}")

    def key_entry(offset):
        if isinstance(offset, int) and 0 <= offset < 32:
            return key0[offset]
        within = aarch64.concrete(aarch64.as_bv(offset) - 32 * k)
        if within is not None and 0 <= within < 32:
            return keyk[within]
        raise Unproved(f"keys read outside current group: {offset}")

    def flag_entry(offset):
        if isinstance(offset, int) and offset == 0:
            return flg0
        within = aarch64.concrete(aarch64.as_bv(offset) - k)
        if within == 0:
            return flgk
        raise Unproved(f"flags read outside current group: {offset}")

    for rname, size, fn in (
        ("ptrs", g * 128, ptr_entry),
        ("ctrs", g * 8, ctr_entry),
        ("keys", g * 32, key_entry),
        ("flags", g, flag_entry),
    ):
        reg = Region(rname, size, False, fn)
        reg.symbolic_initial = True
        regions[rname] = reg
    regions["out"] = Region("out", g * 512, True, None)

    head = prove_sme2.loop_head(obj, lambda: prove_chunks_keyed(obj, name, 3, check=False), 2)

    real = aarch64.disassemble
    aarch64.disassemble = lambda o, a, b: real(o, a, b, llvm=True)
    try:
        s = Setup(obj, name, name + "_end", regions)
    finally:
        aarch64.disassemble = real

    m = s.m
    m.lengths = {"len_groups", "len_k"}
    m.assumptions = [UGE(g, 2), ULE(g, 1 << 40)]
    m.x[0] = Ptr("ptrs", 0)
    m.x[1] = Ptr("ctrs", 0)
    m.x[2] = Ptr("keys", 0)
    m.x[3] = Ptr("flags", 0)
    m.x[4] = Ptr("out", 0)
    m.x[5] = g

    def check_group(mach, kval, key_b, ctr_v, flg_v):
        out = mach.regions["out"]
        base_in = BitVecVal(kval * 1024, 64) if isinstance(kval, int) else aarch64.as_bv(kval * 1024)
        grp_chunks = [[Select(lanes[l], simplify(base_in + j)) for j in range(1024)] for l in range(16)]
        want_cvs = prove_sme2.chunk_cvs(grp_chunks, key_b, [ctr_v + l for l in range(16)], flg_v, BitVecVal(1, 8), BitVecVal(2, 8))
        base_out = BitVecVal(kval * 512, 64) if isinstance(kval, int) else aarch64.as_bv(kval * 512)
        keys_written = set()
        for l, cv in enumerate(want_cvs):
            offs = [aarch64.key(base_out + 32 * l + j) for j in range(32)]
            keys_written.update(offs)
            if any(o not in out.bytes for o in offs):
                raise Unproved(f"group output {l} is not wholly written")
            got = spec.words([out.bytes[o] for o in offs])
            for w in range(8):
                if not same(got[w], cv[w]):
                    raise Unproved(f"group output {l} word {w} differs from specification")
        if set(out.bytes) != keys_written:
            raise Unproved("group writes output bytes outside its own 512-byte slice")

    s0 = induction.run_to(m, s.entry, head)
    if s0.regions["out"].bytes:
        raise Unproved("output written before the group loop")
    s1 = induction.run_to(induction.snapshot(s0), head, head)
    check_group(s1, 0, key0, ctr0, flg0)

    gen = induction.Generalized(s0, s1, k)
    bad = induction.mismatches(s0, gen, BitVecVal(0, 64))
    if bad:
        raise Unproved(f"initial state differs from group invariant at {bad}")

    def check(endm, how):
        check_group(endm, k, keyk, ctrk, flgk)
        if how == "return":
            if not aarch64.holds(endm, g == k + 1):
                raise Unproved("a path leaves the group loop before the last group")
            if aarch64.concrete(endm.x[0]) != 16:
                raise Unproved(f"returned {endm.x[0]}, not 16")
            if getattr(endm, "streaming", False) or getattr(endm, "za", None) is not None:
                raise Unproved("SME streaming/ZA left enabled on return")
            prove_hybrid.check_abi(endm)

    kinds = induction.loop_step(gen, s0, k, list(s0.assumptions) + [ULT(k, g)], head, check)
    return f"{kinds['head']} path(s) back to loop head, {kinds['return']} path(s) out"


def verify_rejecting_controls():
    """Fault-injection controls: confirm the prover rejects both an upper-counter
    truncation fault in SME2 chunks_keyed and a group-count masking fault in
    both SME2 chunks_keyed and NEON output8."""
    sme2_src = open(os.path.join(ROOT, "c", "blake3_output_sme2_aarch64.S"), encoding="utf-8").read()
    neon_src = open(os.path.join(ROOT, "c", "blake3_output_neon_aarch64.S"), encoding="utf-8").read()

    # 1. Truncate upper counter bits in SME2 chunks_keyed: lsr x8, x8, #32 -> ubfx x8, x8, #32, #3.
    bad_ctr = sme2_src.replace("    lsr x8, x8, #32\n", "    ubfx x8, x8, #32, #3\n", 1)
    assert bad_ctr != sme2_src, "expected lsr x8, x8, #32 in SME2 source"
    bad_ctr_obj = assemble_text(bad_ctr, "bad_ctr_sme2", march="armv9-a+sme2")
    try:
        prove_chunks_keyed(bad_ctr_obj, "blake3_servil_sme2_chunks_keyed", 1)
    except Unproved:
        pass
    else:
        raise AssertionError("control failed: upper-counter truncation fault was NOT rejected!")

    # 2. Mask group count in NEON output8: insert `and x2, x2, #3` at entry.
    bad_neon = neon_src.replace(
        "\nblake3_servil_neon_output8:\n",
        "\nblake3_servil_neon_output8:\n    and x2, x2, #3\n",
        1,
    )
    assert bad_neon != neon_src, "expected blake3_servil_neon_output8 label"
    bad_neon_obj = assemble_text(bad_neon, "bad_groups_neon")
    try:
        prove_output_kernel(bad_neon_obj, "blake3_servil_neon_output8", 8, 4)
    except Unproved:
        pass
    else:
        raise AssertionError("control failed: NEON group-masking fault was NOT rejected!")

    # 3. Mask group count in SME2 chunks_keyed: insert `and x5, x5, #3` at entry.
    bad_sme2_g = sme2_src.replace(
        "\nblake3_servil_sme2_chunks_keyed:\n",
        "\nblake3_servil_sme2_chunks_keyed:\n    and x5, x5, #3\n",
        1,
    )
    assert bad_sme2_g != sme2_src, "expected blake3_servil_sme2_chunks_keyed label"
    bad_sme2_g_obj = assemble_text(bad_sme2_g, "bad_groups_sme2", march="armv9-a+sme2")
    try:
        prove_chunks_keyed(bad_sme2_g_obj, "blake3_servil_sme2_chunks_keyed", 4)
    except Unproved:
        pass
    else:
        raise AssertionError("control failed: SME2 group-masking fault was NOT rejected!")


SME2_OBJ = None
NEON_OBJ = None


def _run_case(case):
    global SME2_OBJ, NEON_OBJ
    kind, groups = case
    tk = time.monotonic()
    if kind == "sme2":
        if SME2_OBJ is None:
            SME2_OBJ = assemble("blake3_output_sme2_aarch64.S", march="armv9-a+sme2")
        steps = prove_chunks_keyed(SME2_OBJ, "blake3_servil_sme2_chunks_keyed", groups)
        return case, steps, time.monotonic() - tk
    if kind == "sme2_every":
        if SME2_OBJ is None:
            SME2_OBJ = assemble("blake3_output_sme2_aarch64.S", march="armv9-a+sme2")
        detail = prove_chunks_keyed_every(SME2_OBJ)
        return case, detail, time.monotonic() - tk
    if kind == "neon":
        if NEON_OBJ is None:
            NEON_OBJ = assemble("blake3_output_neon_aarch64.S")
        steps = prove_output_kernel(NEON_OBJ, "blake3_servil_neon_output8", 8, groups)
        return case, steps, time.monotonic() - tk
    if kind == "controls":
        verify_rejecting_controls()
        return case, 3, time.monotonic() - tk
    raise ValueError(f"unknown case {case}")


def main():
    global SME2_OBJ, NEON_OBJ
    t0 = time.monotonic()
    SME2_OBJ = assemble("blake3_output_sme2_aarch64.S", march="armv9-a+sme2")
    NEON_OBJ = assemble("blake3_output_neon_aarch64.S")

    todo = (
        [("sme2_every", 0)]
        + [("sme2", g) for g in (4, 3, 2, 1, 0)]
        + [("neon", g) for g in range(MAX_NEON_GROUPS, -1, -1)]
        + [("controls", 0)]
    )
    cases = 0
    for (kind, groups), detail, dt in workers.each(_run_case, todo, len(todo)):
        if kind == "sme2_every":
            print(f"PROVED blake3_servil_sme2_chunks_keyed groups=2..2^40 (induction: {detail}) in {dt:.2f}s", flush=True)
            cases += 1
        elif kind == "sme2":
            print(f"PROVED blake3_servil_sme2_chunks_keyed groups={groups:2d} ({16 * groups:3d} symbolic chunks, {detail:6d} steps) in {dt:.2f}s", flush=True)
            cases += 1
        elif kind == "neon":
            print(f"PROVED blake3_servil_neon_output8     groups={groups:2d} ({8 * groups:3d} symbolic lanes,  {detail:6d} steps) in {dt:.2f}s", flush=True)
            cases += 1
        else:
            print(f"CONTROLS verified ({detail} rejecting fault-injection controls) in {dt:.2f}s", flush=True)

    print(
        f"prove_output_aarch64: all {cases} symbolic cases (SME2 groups 0..4 + 2..2^40 induction, NEON groups 0..={MAX_NEON_GROUPS}) + 3 rejecting controls proved in {time.monotonic() - t0:.2f}s"
    )


if __name__ == "__main__":
    main()
