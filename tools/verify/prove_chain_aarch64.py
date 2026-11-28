#!/usr/bin/env python3
"""Formally prove c/blake3_chain_sme2_aarch64.S (18-lane AArch64 SME2 + 2-integer
hybrid fused 32-byte hash-chain kernel, #584) equal to iterated BLAKE3's Lean
spec for ALL 32-byte chain inputs, keys, wrapping u32 start steps, and flags
under AAPCS64.
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
from z3 import BitVec, BitVecVal, Concat, ZeroExt, simplify

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


def prove_kernel(obj, name, lanes_per_group, steps, groups, is_sme2):
    n = lanes_per_group * groups
    raw_init = byte_symbols("i_", max(32 * n, 1))
    key = byte_symbols("key", 32)
    regions = {
        "init": Region("init", max(32 * n, 1), False, lambda o: raw_init[o]),
        "key": Region("key", 32, False, lambda o: key[o]),
        "out": Region("out", max(16 * n, 1), True, None),
    }
    real = aarch64.disassemble
    aarch64.disassemble = lambda o, a, b: real(o, a, b, llvm=True)
    try:
        s = Setup(obj, name, name + "_end", regions)
    finally:
        aarch64.disassemble = real

    flags = BitVec("flags", 8)
    junk_flags = BitVec("junk_flags", 32)
    junk_steps = BitVec("junk_steps", 32)
    flags_arg = Concat(junk_flags, ZeroExt(24, flags))
    steps_arg = Concat(junk_steps, BitVecVal(steps, 32))
    m = s.m
    if is_sme2:
        m.x[0] = Ptr("init", 0)
        m.x[1] = Ptr("key", 0)
        m.x[2] = flags_arg
        m.x[3] = steps_arg
        m.x[4] = Ptr("out", 0)
        m.x[5] = BitVecVal(groups, 64)
    else:
        m.x[0] = Ptr("init", 0)
        m.x[1] = BitVecVal(groups, 64)
        m.x[2] = Ptr("key", 0)
        m.x[3] = flags_arg
        m.x[4] = Ptr("out", 0)
        m.x[5] = steps_arg
    s.run_and_check_abi()

    if is_sme2:
        if not same(m.x[0], BitVecVal(16, 64)):
            raise Unproved(f"{name}: expected return 16, got {m.x[0]}")
        if getattr(m, "streaming", False) or getattr(m, "za", None) is not None:
            raise Unproved(f"{name}: SME streaming/ZA left enabled")

    out = m.regions["out"]
    if groups == 0 or steps == 0:
        if out.bytes:
            raise Unproved(f"{name}: groups={groups} steps={steps} wrote output bytes")
        return m.steps

    if any(i not in out.bytes for i in range(16 * n)):
        raise Unproved(f"{name}: output not wholly written")

    key_words = [simplify(w) for w in spec.words(key)]
    ctr0 = BitVecVal(0, 64)
    bl32 = BitVecVal(32, 32)
    f32 = ZeroExt(24, flags)
    zeros8 = [BitVecVal(0, 32)] * 8

    for lane in range(n):
        lane_words = [simplify(w) for w in spec.words(raw_init[32 * lane : 32 * (lane + 1)])]
        prefix = lane_words[0:3]
        step_w = lane_words[3]
        state = lane_words[4:8]
        for st in range(steps):
            msg = prefix + [step_w + BitVecVal(st, 32)] + state + zeros8
            state = spec.compress(key_words, msg, ctr0, bl32, f32)[:4]
        got = spec.words([out.bytes[16 * lane + j] for j in range(16)])
        for wi in range(4):
            if not same(got[wi], state[wi]):
                raise Unproved(f"{name} steps={steps} groups={groups}: lane {lane} word {wi} differs!")
    return m.steps


def main():
    t0 = time.monotonic()
    sme2_obj = assemble("blake3_chain_sme2_aarch64.S", march="armv9-a+sme2")
    neon_obj = assemble("blake3_chain_neon_aarch64.S")
    cases = 0
    for steps, groups in [(0, 1), (1, 0), (1, 1), (2, 1), (3, 1), (2, 2)]:
        tk = time.monotonic()
        nsteps = prove_kernel(sme2_obj, "blake3_servil_sme2_chain32_trunc16", 18, steps, groups, True)
        cases += 1
        print(
            f"PROVED blake3_servil_sme2_chain32_trunc16     steps={steps} groups={groups} "
            f"({18 * groups:2d} symbolic lanes, {nsteps:5d} steps) in {time.monotonic() - tk:.2f}s"
        )
    for steps, groups in [(0, 1), (1, 0), (1, 1), (2, 1), (3, 1), (2, 2)]:
        tk = time.monotonic()
        nsteps = prove_kernel(neon_obj, "blake3_servil_neon_p11_chain32_trunc16", 11, steps, groups, False)
        cases += 1
        print(
            f"PROVED blake3_servil_neon_p11_chain32_trunc16 steps={steps} groups={groups} "
            f"({11 * groups:2d} symbolic lanes, {nsteps:5d} steps) in {time.monotonic() - tk:.2f}s"
        )
    print(f"prove_chain_aarch64: all {cases} cases proved in {time.monotonic() - t0:.2f}s")


if __name__ == "__main__":
    main()
