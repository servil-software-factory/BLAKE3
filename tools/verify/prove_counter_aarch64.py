#!/usr/bin/env python3
"""Formally prove c/blake3_counter_sme2_aarch64.S (18-lane SME2+2-int hybrid
zero-word-pruned counter & find kernels, #511, #585) and
c/blake3_counter_neon_aarch64.S (NEON 8+3 counter kernels) equal to BLAKE3's
Lean spec for ALL templates, keys, wrapping u32 bases, 8-bit block lengths,
flags, masks, and targets under AAPCS64.
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
import lean_spec as spec
from aarch64 import Ptr, Region, Unproved
from prove_hybrid import Setup, byte_symbols, same
from z3 import BitVec, BitVecVal, Extract, If, Not, Or, ZeroExt, is_eq, simplify, substitute

CLANG = next(c for c in ("clang-19", "clang-18", "clang-17", "clang") if shutil.which(c))


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


def prove_counter_kernel(obj, name, lanes_per_group, n_words, counter_word, groups, is_sme2, trunc16=False):
    n = lanes_per_group * groups
    out_stride = 16 if trunc16 else 32
    raw_tmpl = [byte_symbols(f"t_{w}_", 4) if w < n_words else [BitVecVal(0, 8)] * 4 for w in range(16)]
    flat_tmpl = [b for word in raw_tmpl for b in word]
    key = byte_symbols("key", 32)
    regions = {
        "tmpl": Region("tmpl", 64, False, lambda o: flat_tmpl[o]),
        "key": Region("key", 32, False, lambda o: key[o]),
        "out": Region("out", max(out_stride * n, 1), True, None),
    }
    real = aarch64.disassemble
    aarch64.disassemble = lambda o, a, b: real(o, a, b, llvm=True)
    try:
        s = Setup(obj, name, name + "_end", regions)
    finally:
        aarch64.disassemble = real

    base = BitVec("base", 32)
    flags = BitVec("flags", 8)
    block_len = BitVec("blen", 8)
    base_and_word = ZeroExt(32, base) | BitVecVal(counter_word << 32, 64)
    flags_and_len = ZeroExt(56, flags) | (ZeroExt(56, block_len) << 8) | BitVecVal((1 << 16) if trunc16 else 0, 64)

    m = s.m
    m.x[0] = Ptr("tmpl", 0)
    m.x[1] = Ptr("key", 0)
    m.x[2] = base_and_word
    m.x[3] = flags_and_len
    m.x[4] = Ptr("out", 0)
    m.x[5] = BitVecVal(groups, 64)
    s.run_and_check_abi()

    if is_sme2:
        if not same(m.x[0], BitVecVal(16, 64)):
            raise Unproved(f"{name}: expected return 16, got {m.x[0]}")
        if getattr(m, "streaming", False) or getattr(m, "za", None) is not None:
            raise Unproved(f"{name}: SME streaming/ZA left enabled")

    out = m.regions["out"]
    if groups == 0:
        if out.bytes:
            raise Unproved(f"{name}: groups=0 wrote output bytes")
        return m.steps

    if any(i not in out.bytes for i in range(out_stride * n)):
        raise Unproved(f"{name}: output not wholly written")

    key_words = [simplify(w) for w in spec.words(key)]
    tmpl_words = [simplify(w) for w in spec.words(flat_tmpl)]
    ctr0 = BitVecVal(0, 64)
    bl = ZeroExt(24, block_len)
    f32 = ZeroExt(24, flags)
    out_words = 4 if trunc16 else 8

    for lane in range(n):
        msg = list(tmpl_words)
        msg[counter_word] = base + BitVecVal(lane, 32)
        want = spec.compress(key_words, msg, ctr0, bl, f32)[:out_words]
        got = spec.words([out.bytes[out_stride * lane + j] for j in range(out_stride)])
        for wi in range(out_words):
            if not same(got[wi], want[wi]):
                raise Unproved(f"{name} w={n_words} cw={counter_word} groups={groups} t16={trunc16}: lane {lane} word {wi} differs!")
    return m.steps


def prove_sme2_find_kernel(obj, name, n_words, counter_word, groups, hit_schedule):
    """Compositionally prove an 18-lane SME2+int hybrid early-exit `find` kernel.

    `hit_schedule` is a list of tuples `(sme_hit, int16_hit, int17_hit)` for
    each group until the first group that hits (or all `groups` miss).
    At every data-dependent branch (`b.ne` after `cmpeq` and `cbz` on int
    lanes 16 & 17), we structurally prove (`same()`) that the tested condition
    equals the Lean BLAKE3 spec's `(word0 & mask) == target` for every lane in
    that group. On a hit path, we also abstract the 18 proved diff words on the
    stack into fresh 32-bit symbols `D_0..D_17` and let Z3 prove the branchless
    bit-packing epilogue returns the exact `(hit_group << 18) | bitmask_18`.
    """
    raw_tmpl = [byte_symbols(f"t_{w}_", 4) if w < n_words else [BitVecVal(0, 8)] * 4 for w in range(16)]
    flat_tmpl = [b for word in raw_tmpl for b in word]
    key = byte_symbols("key", 32)
    regions = {
        "tmpl": Region("tmpl", 64, False, lambda o: flat_tmpl[o]),
        "key": Region("key", 32, False, lambda o: key[o]),
    }
    real = aarch64.disassemble
    aarch64.disassemble = lambda o, a, b: real(o, a, b, llvm=True)
    try:
        s = Setup(obj, name, name + "_end", regions)
    finally:
        aarch64.disassemble = real

    base = BitVec("base", 32)
    flags = BitVec("flags", 8)
    block_len = BitVec("blen", 8)
    mask = BitVec("mask", 32)
    target = BitVec("target", 32)

    key_words = [simplify(w) for w in spec.words(key)]
    tmpl_words = [simplify(w) for w in spec.words(flat_tmpl)]
    ctr0 = BitVecVal(0, 64)
    bl = ZeroExt(24, block_len)
    f32 = ZeroExt(24, flags)

    def spec_w0(lane):
        msg = list(tmpl_words)
        msg[counter_word] = base + BitVecVal(lane, 32)
        return spec.compress(key_words, msg, ctr0, bl, f32)[0]

    branch_state = {"group": 0, "stage": 0, "hit_group": None, "abstracted": False}
    fresh_d = [BitVec(f"D_{i}", 32) for i in range(18)]

    def decide_hook(m, c):
        g = branch_state["group"]
        st = branch_state["stage"]
        if g >= len(hit_schedule):
            raise Unproved(f"{name}: unexpected extra data branch at group {g} stage {st}")
        sme_hit, i16_hit, i17_hit = hit_schedule[g]
        gbase = 18 * g

        def abstract_hit():
            m.v[0] = [fresh_d[i] ^ target for i in range(16)]
            m.store(m.x["sp"] + (64 + 4 * 16), fresh_d[16], 4)
            m.store(m.x["sp"] + (64 + 4 * 17), fresh_d[17], 4)
            branch_state["hit_group"] = g
            branch_state["abstracted"] = True

        if st == 0:
            # First data branch of group g: b.ne after cmpeq on 16 SME2 lanes.
            # Verify m.v[0][i] == (spec_w0(gbase + i) & mask) and m.v[2][i] == target.
            for i in range(16):
                want_masked = spec_w0(gbase + i) & mask
                if not same(m.v[0][i], want_masked):
                    raise Unproved(f"{name} g={g}: SME2 lane {i} masked word0 != spec")
                if not same(m.v[2][i], target):
                    raise Unproved(f"{name} g={g}: SME2 lane {i} target reg != target")
            # Verify `c` is `Not(Not(Or(m.v[0][i] == m.v[2][i] for i in 0..16)))` without invoking SAT.
            inner = c
            while inner.decl().name() == "not" and inner.arg(0).decl().name() == "not":
                inner = inner.arg(0).arg(0)
            if inner.decl().name() != "or" or inner.num_args() != 16:
                raise Unproved(f"{name} g={g}: cmpeq branch condition not 16-way Or: {inner.decl().name()}")
            for i in range(16):
                arg = inner.arg(i)
                if arg.decl().name() != "=" or not (arg.arg(0).eq(m.v[0][i]) and arg.arg(1).eq(m.v[2][i])):
                    raise Unproved(f"{name} g={g}: cmpeq branch lane {i} mismatch")
            if sme_hit:
                for k in (16, 17):
                    got_d = m.load(m.x["sp"] + (64 + 4 * k), 4)
                    want_d = (spec_w0(gbase + k) & mask) ^ target
                    if not same(got_d, want_d):
                        raise Unproved(f"{name} g={g}: int lane {k} diff != spec")
                abstract_hit()
                return True
            branch_state["stage"] = 1
            return False

        if st == 1:
            # Second data branch of group g: cbz w0 (int lane 16 diff == 0).
            want_d16 = (spec_w0(gbase + 16) & mask) ^ target
            got_w0 = Extract(31, 0, m.x[0])
            if not same(got_w0, want_d16):
                raise Unproved(f"{name} g={g}: int lane 16 diff != spec")
            if c.decl().name() != "=" or not (c.arg(0).eq(got_w0) and c.arg(1).eq(BitVecVal(0, 32))):
                raise Unproved(f"{name} g={g}: int lane 16 cbz condition mismatch")
            if i16_hit:
                got_d17 = m.load(m.x["sp"] + (64 + 4 * 17), 4)
                want_d17 = (spec_w0(gbase + 17) & mask) ^ target
                if not same(got_d17, want_d17):
                    raise Unproved(f"{name} g={g}: int lane 17 diff != spec")
                abstract_hit()
                return True
            branch_state["stage"] = 2
            return False

        if st == 2:
            # Third data branch of group g: cbz w1 (int lane 17 diff == 0).
            want_d17 = (spec_w0(gbase + 17) & mask) ^ target
            got_w1 = Extract(31, 0, m.x[1])
            if not same(got_w1, want_d17):
                raise Unproved(f"{name} g={g}: int lane 17 diff != spec")
            if c.decl().name() != "=" or not (c.arg(0).eq(got_w1) and c.arg(1).eq(BitVecVal(0, 32))):
                raise Unproved(f"{name} g={g}: int lane 17 cbz condition mismatch")
            if i17_hit:
                abstract_hit()
                return True
            branch_state["group"] = g + 1
            branch_state["stage"] = 0
            return False

        raise Unproved(f"{name}: invalid stage {st}")

    m = s.m
    m.decide_hook = decide_hook
    m.x[0] = Ptr("tmpl", 0)
    m.x[1] = Ptr("key", 0)
    m.x[2] = ZeroExt(32, base) | BitVecVal(counter_word << 32, 64)
    m.x[3] = ZeroExt(56, flags) | (ZeroExt(56, block_len) << 8)
    m.x[4] = ZeroExt(32, mask) | (ZeroExt(32, target) << 32)
    m.x[5] = BitVecVal(groups, 64)
    s.run_and_check_abi()

    if getattr(m, "streaming", False) or getattr(m, "za", None) is not None:
        raise Unproved(f"{name}: SME streaming/ZA left enabled")

    if branch_state["hit_group"] is None:
        want_ret = BitVecVal(groups << 18, 64)
        if not same(m.x[0], want_ret):
            raise Unproved(f"{name}: miss return mismatch: got {m.x[0]}, want {want_ret}")
    else:
        hg = branch_state["hit_group"]
        want_mask = BitVecVal(hg << 18, 64)
        for i in range(18):
            want_mask = want_mask | If(fresh_d[i] == BitVecVal(0, 32), BitVecVal(1 << i, 64), BitVecVal(0, 64))
        if not aarch64.unsatisfiable([], m.x[0] != want_mask):
            raise Unproved(f"{name}: hit return bitmask mismatch at group {hg}")
    return m.steps


def prove_neon_find_kernel(obj, name, n_words, counter_word, groups, hit_group):
    """Compositionally prove an 11-lane NEON 8+3 early-exit `find` kernel."""
    raw_tmpl = [byte_symbols(f"t_{w}_", 4) if w < n_words else [BitVecVal(0, 8)] * 4 for w in range(16)]
    flat_tmpl = [b for word in raw_tmpl for b in word]
    key = byte_symbols("key", 32)
    regions = {
        "tmpl": Region("tmpl", 64, False, lambda o: flat_tmpl[o]),
        "key": Region("key", 32, False, lambda o: key[o]),
    }
    real = aarch64.disassemble
    aarch64.disassemble = lambda o, a, b: real(o, a, b, llvm=True)
    try:
        s = Setup(obj, name, name + "_end", regions)
    finally:
        aarch64.disassemble = real

    base = BitVec("base", 32)
    flags = BitVec("flags", 8)
    block_len = BitVec("blen", 8)
    mask = BitVec("mask", 32)
    target = BitVec("target", 32)

    key_words = [simplify(w) for w in spec.words(key)]
    tmpl_words = [simplify(w) for w in spec.words(flat_tmpl)]
    ctr0 = BitVecVal(0, 64)
    bl = ZeroExt(24, block_len)
    f32 = ZeroExt(24, flags)

    def spec_w0(lane):
        msg = list(tmpl_words)
        msg[counter_word] = base + BitVecVal(lane, 32)
        return spec.compress(key_words, msg, ctr0, bl, f32)[0]

    fresh_d = [BitVec(f"ND_{i}", 32) for i in range(11)]
    fresh_mask = BitVecVal(0, 64)
    for i in range(11):
        fresh_mask = fresh_mask | If(fresh_d[i] == BitVecVal(0, 32), BitVecVal(1 << i, 64), BitVecVal(0, 64))

    state = {"group": 0}

    def decide_hook(m, c):
        g = state["group"]
        gbase = 11 * g
        # Verify all 11 diff words at [sp + 0 + 4*lane] match Lean spec, then
        # abstract those exact subterms inside m.x[10] into fresh_d[0..11] and
        # prove with Z3 that m.x[10] equals the exact 11-bit hit mask.
        got_ds = [m.load(m.x["sp"] + (4 * lane), 4) for lane in range(11)]
        for lane in range(11):
            want_d = (spec_w0(gbase + lane) & mask) ^ target
            if not same(got_ds[lane], want_d):
                raise Unproved(f"{name} g={g}: lane {lane} diff != spec")
        x10_abs = substitute(m.x[10], *[(got_ds[i], fresh_d[i]) for i in range(11)])
        if not aarch64.unsatisfiable([], x10_abs != fresh_mask):
            raise Unproved(f"{name} g={g}: x10 bitmask formula mismatch")
        if c.decl().name() != "=" or not (c.arg(0).eq(m.x[10]) and c.arg(1).eq(BitVecVal(0, 64))):
            raise Unproved(f"{name} g={g}: cbnz x10 condition mismatch")
        if hit_group is not None and g == hit_group:
            m.x[10] = fresh_mask
            return False  # cbnz jumps when (x10 == 0) is False!
        state["group"] = g + 1
        return True  # (x10 == 0) is True -> no hit, fall through!

    m = s.m
    m.decide_hook = decide_hook
    m.x[0] = Ptr("tmpl", 0)
    m.x[1] = Ptr("key", 0)
    m.x[2] = ZeroExt(32, base) | BitVecVal(counter_word << 32, 64)
    m.x[3] = ZeroExt(56, flags) | (ZeroExt(56, block_len) << 8)
    m.x[4] = ZeroExt(32, mask) | (ZeroExt(32, target) << 32)
    m.x[5] = BitVecVal(groups, 64)
    s.run_and_check_abi()

    if hit_group is None:
        want = BitVecVal(groups << 11, 64)
        if not same(m.x[0], want):
            raise Unproved(f"{name}: miss return mismatch: got {m.x[0]}, want {want}")
    else:
        want = BitVecVal(hit_group << 11, 64) | fresh_mask
        if not aarch64.unsatisfiable([], m.x[0] != want):
            raise Unproved(f"{name}: hit return mismatch at group {hit_group}")
    return m.steps


def main():
    t0 = time.monotonic()
    sme2_obj = assemble("blake3_counter_sme2_aarch64.S", march="armv9-a+sme2")
    neon_obj = assemble("blake3_counter_neon_aarch64.S")
    cases = 0

    # 1. Prove all 4 zero-word-pruned 18-lane SME2+int hybrid counter kernels (32 B & 16 B outputs).
    for nw, cws in [(4, [0, 3]), (9, [0, 8]), (13, [0, 12]), (16, [0, 12, 15])]:
        kname = f"blake3_servil_sme2_counter_w{nw}"
        for cw in cws:
            for groups, trunc16 in [(0, False), (1, False), (1, True), (2, False), (2, True)]:
                if cw != cws[-1] and (groups != 1 or trunc16):
                    continue
                tk = time.monotonic()
                steps = prove_counter_kernel(sme2_obj, kname, 18, nw, cw, groups, True, trunc16=trunc16)
                cases += 1
                print(
                    f"PROVED {kname:<32} cw={cw:2d} g={groups} t16={int(trunc16)} "
                    f"({18 * groups:2d} symbolic lanes, {steps:5d} steps) in {time.monotonic() - tk:.2f}s"
                )

    # 2. Prove all 4 zero-word-pruned 18-lane SME2+int hybrid early-exit find kernels.
    for nw, cw in [(4, 3), (9, 8), (13, 12), (16, 15)]:
        kname = f"blake3_servil_sme2_find_w{nw}"
        schedules = [
            ("zero", 0, []),
            ("miss1", 1, [(False, False, False)]),
            ("sme_hit", 1, [(True, False, False)]),
            ("int16_hit", 1, [(False, True, False)]),
            ("int17_hit", 1, [(False, False, True)]),
            ("miss2", 2, [(False, False, False), (False, False, False)]),
            ("g1_hit", 2, [(False, False, False), (True, False, False)]),
        ]
        for label, groups, sched in schedules:
            tk = time.monotonic()
            steps = prove_sme2_find_kernel(sme2_obj, kname, nw, cw, groups, sched)
            cases += 1
            print(
                f"PROVED {kname:<32} cw={cw:2d} {label:<9} "
                f"({steps:5d} steps) in {time.monotonic() - tk:.2f}s"
            )

    # 3. Prove all 3 zero-word-pruned 11-lane NEON 8+3 counter kernels (32 B & 16 B).
    for nw in (4, 9, 16):
        kname = f"blake3_servil_neon_p11_counter_w{nw}"
        for cw, groups, t16 in [(0, 0, False), (0, 1, False), (nw - 1, 1, True), (nw - 1, 2, False), (nw - 1, 2, True)]:
            tk = time.monotonic()
            steps = prove_counter_kernel(neon_obj, kname, 11, nw, cw, groups, False, trunc16=t16)
            cases += 1
            print(
                f"PROVED {kname:<32} cw={cw:2d} g={groups} t16={int(t16)} "
                f"({11 * groups:2d} symbolic lanes, {steps:5d} steps) in {time.monotonic() - tk:.2f}s"
            )

    # 4. Prove all 3 zero-word-pruned 11-lane NEON 8+3 early-exit find kernels.
    for nw in (4, 9, 16):
        kname = f"blake3_servil_neon_p11_find_w{nw}"
        for label, groups, hg in [("zero", 0, None), ("miss1", 1, None), ("hit0", 1, 0), ("miss2", 2, None), ("hit1", 2, 1)]:
            tk = time.monotonic()
            steps = prove_neon_find_kernel(neon_obj, kname, nw, nw - 1, groups, hg)
            cases += 1
            print(
                f"PROVED {kname:<32} cw={nw - 1:2d} {label:<9} "
                f"({steps:5d} steps) in {time.monotonic() - tk:.2f}s"
            )

    print(f"prove_counter_aarch64: all {cases} cases proved in {time.monotonic() - t0:.2f}s")


if __name__ == "__main__":
    main()
