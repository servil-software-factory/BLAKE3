#!/usr/bin/env python3
"""Formally prove c/blake3_packed_sme2_aarch64.S and c/blake3_packed_neon_aarch64.S
(all six direct-load AArch64 Packed<16/32/48> kernels: SME2 and NEON 8+3)
equal to BLAKE3's Lean spec for ALL inputs, keys, and flags across
groups in {0, 1, 2} under AAPCS64.
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


def prove_sme2_kernel(obj, name, nbytes, groups):
    n = 16 * groups
    raw = byte_symbols("in_", max(n * nbytes, 1))
    key = byte_symbols("key", 32)
    regions = {
        "input": Region("input", max(n * nbytes, 1), False, lambda o: raw[o]),
        "key": Region("key", 32, False, lambda o: key[o]),
        "out": Region("out", max(32 * n, 1), True, None),
    }
    real = aarch64.disassemble
    aarch64.disassemble = lambda o, a, b: real(o, a, b, llvm=True)
    try:
        s = Setup(obj, name, name + "_end", regions)
    finally:
        aarch64.disassemble = real

    flags = BitVec("flags", 8)
    m = s.m
    m.x[0] = Ptr("input", 0)
    m.x[1] = Ptr("key", 0)
    m.x[2] = ZeroExt(56, flags)
    m.x[3] = Ptr("out", 0)
    m.x[4] = BitVecVal(groups, 64)
    s.run_and_check_abi()

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
    zero = BitVecVal(0, 8)
    ctr0 = BitVecVal(0, 64)
    bl = BitVecVal(nbytes, 32)
    f32 = ZeroExt(24, flags)

    for lane in range(n):
        padded = raw[lane * nbytes:(lane + 1) * nbytes] + [zero] * (64 - nbytes)
        want = spec.compress(key_words, spec.words(padded), ctr0, bl, f32)[:8]
        got = spec.words([out.bytes[32 * lane + j] for j in range(32)])
        for wi in range(8):
            if not same(got[wi], want[wi]):
                raise Unproved(f"{name}: lane {lane} word {wi} differs!")
    return m.steps


def prove_neon_kernel(obj, name, nbytes, groups):
    n = 11 * groups
    raw = byte_symbols("in_", max(n * nbytes, 1))
    key = byte_symbols("key", 32)
    regions = {
        "input": Region("input", max(n * nbytes, 1), False, lambda o: raw[o]),
        "key": Region("key", 32, False, lambda o: key[o]),
        "out": Region("out", max(32 * n, 1), True, None),
    }
    real = aarch64.disassemble
    aarch64.disassemble = lambda o, a, b: real(o, a, b, llvm=True)
    try:
        s = Setup(obj, name, name + "_end", regions)
    finally:
        aarch64.disassemble = real

    flags = BitVec("flags", 8)
    m = s.m
    m.x[0] = Ptr("input", 0)
    m.x[1] = BitVecVal(groups, 64)
    m.x[2] = Ptr("key", 0)
    m.x[3] = ZeroExt(56, flags)
    m.x[4] = Ptr("out", 0)
    s.run_and_check_abi()

    out = m.regions["out"]
    if groups == 0:
        if out.bytes:
            raise Unproved(f"{name}: groups=0 wrote output bytes")
        return m.steps

    if any(i not in out.bytes for i in range(32 * n)):
        raise Unproved(f"{name}: output not wholly written")

    key_words = [simplify(w) for w in spec.words(key)]
    zero = BitVecVal(0, 8)
    ctr0 = BitVecVal(0, 64)
    bl = BitVecVal(nbytes, 32)
    f32 = ZeroExt(24, flags)

    for lane in range(n):
        padded = raw[lane * nbytes:(lane + 1) * nbytes] + [zero] * (64 - nbytes)
        want = spec.compress(key_words, spec.words(padded), ctr0, bl, f32)[:8]
        got = spec.words([out.bytes[32 * lane + j] for j in range(32)])
        for wi in range(8):
            if not same(got[wi], want[wi]):
                raise Unproved(f"{name}: lane {lane} word {wi} differs!")
    return m.steps


def main():
    t0 = time.monotonic()
    sme2_obj = assemble("blake3_packed_sme2_aarch64.S", march="armv9-a+sme2")
    neon_obj = assemble("blake3_packed_neon_aarch64.S")
    cases = 0
    for nbytes in (16, 32, 48):
        name = f"blake3_servil_sme2_packed{nbytes}"
        for groups in (0, 1, 2):
            tk = time.monotonic()
            steps = prove_sme2_kernel(sme2_obj, name, nbytes, groups)
            cases += 1
            print(
                f"PROVED {name:34s} groups={groups} ({16 * groups:2d} symbolic lanes, "
                f"{steps:5d} steps) in {time.monotonic() - tk:.2f}s"
            )
    for nbytes in (16, 32, 48):
        name = f"blake3_servil_neon_p11_packed{nbytes}"
        for groups in (0, 1, 2):
            tk = time.monotonic()
            steps = prove_neon_kernel(neon_obj, name, nbytes, groups)
            cases += 1
            print(
                f"PROVED {name:34s} groups={groups} ({11 * groups:2d} symbolic lanes, "
                f"{steps:5d} steps) in {time.monotonic() - tk:.2f}s"
            )
    print(f"prove_packed_aarch64: all {cases} cases proved in {time.monotonic() - t0:.2f}s")


if __name__ == "__main__":
    main()
