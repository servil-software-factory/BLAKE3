#!/usr/bin/env python3
"""Formally prove c/blake3_counter_sme2_aarch64.S and
c/blake3_counter_neon_aarch64.S (universal <= 64 B SME2 and NEON 8+3
counter-grinding kernels, #511) equal to BLAKE3's Lean spec for ALL
64-byte templates, keys, wrapping u32 bases, 8-bit block lengths, and flags
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
from z3 import BitVec, BitVecVal, ZeroExt, simplify

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


def prove_kernel(obj, name, lanes_per_group, counter_word, groups, is_sme2):
    n = lanes_per_group * groups
    raw_tmpl = byte_symbols("t_", 64)
    key = byte_symbols("key", 32)
    regions = {
        "tmpl": Region("tmpl", 64, False, lambda o: raw_tmpl[o]),
        "key": Region("key", 32, False, lambda o: key[o]),
        "out": Region("out", max(32 * n, 1), True, None),
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
    flags_and_len = ZeroExt(56, flags) | (ZeroExt(56, block_len) << 8)

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

    if any(i not in out.bytes for i in range(32 * n)):
        raise Unproved(f"{name}: output not wholly written")

    key_words = [simplify(w) for w in spec.words(key)]
    tmpl_words = [simplify(w) for w in spec.words(raw_tmpl)]
    ctr0 = BitVecVal(0, 64)
    bl = ZeroExt(24, block_len)
    f32 = ZeroExt(24, flags)

    for lane in range(n):
        msg = list(tmpl_words)
        msg[counter_word] = base + BitVecVal(lane, 32)
        want = spec.compress(key_words, msg, ctr0, bl, f32)[:8]
        got = spec.words([out.bytes[32 * lane + j] for j in range(32)])
        for wi in range(8):
            if not same(got[wi], want[wi]):
                raise Unproved(f"{name} cw={counter_word} groups={groups}: lane {lane} word {wi} differs!")
    return m.steps


def main():
    t0 = time.monotonic()
    sme2_obj = assemble("blake3_counter_sme2_aarch64.S", march="armv9-a+sme2")
    neon_obj = assemble("blake3_counter_neon_aarch64.S")
    cases = 0

    for cw, groups in [(0, 0), (0, 1), (8, 1), (12, 1), (15, 1), (0, 2), (12, 2), (15, 2), (12, 3)]:
        tk = time.monotonic()
        steps = prove_kernel(sme2_obj, "blake3_servil_sme2_counter", 16, cw, groups, True)
        cases += 1
        print(
            f"PROVED blake3_servil_sme2_counter     cw={cw:2d} groups={groups} "
            f"({16 * groups:2d} symbolic lanes, {steps:5d} steps) in {time.monotonic() - tk:.2f}s"
        )

    for cw, groups in [(0, 0), (0, 1), (8, 1), (12, 1), (15, 1), (0, 2), (12, 2), (15, 2), (12, 3)]:
        tk = time.monotonic()
        steps = prove_kernel(neon_obj, "blake3_servil_neon_p11_counter", 11, cw, groups, False)
        cases += 1
        print(
            f"PROVED blake3_servil_neon_p11_counter cw={cw:2d} groups={groups} "
            f"({11 * groups:2d} symbolic lanes, {steps:5d} steps) in {time.monotonic() - tk:.2f}s"
        )

    print(f"prove_counter_aarch64: all {cases} cases proved in {time.monotonic() - t0:.2f}s")


if __name__ == "__main__":
    main()
