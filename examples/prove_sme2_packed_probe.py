#!/usr/bin/env python3
"""Formally prove probe_sme2_packed{16,32,48} equal to BLAKE3's Lean spec
for ALL inputs, keys, and flags (groups = 1 and 2, testing both last-group
and pipelined next-group paths!) using tools/verify/prove_sme2.py + Z3."""
import os
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "tools", "verify"))

import aarch64
from aarch64 import Ptr, Region, Unproved
from prove_hybrid import Setup, byte_symbols, same
import lean_spec as spec
from z3 import BitVec, BitVecVal, ZeroExt, simplify

CLANG = next(c for c in ("clang-19", "clang-18", "clang-17", "clang") if shutil.which(c))


def assemble():
    obj = os.path.join(tempfile.mkdtemp(), "sme2_packed.o")
    src = os.path.join(ROOT, "examples", "sme2_packed_elf.S")
    aarch64.OBJDUMP = aarch64.LLVM_OBJDUMP or "llvm-objdump"
    subprocess.run([CLANG, "-c", "--target=aarch64-linux-gnu", "-march=armv9-a+sme2", src, "-o", obj], check=True)
    return obj


def prove_kernel(obj, name, nbytes, groups):
    n = 16 * groups
    raw = byte_symbols("in_", n * nbytes)
    key = byte_symbols("key", 32)
    regions = {
        "input": Region("input", n * nbytes, False, lambda o: raw[o]),
        "key": Region("key", 32, False, lambda o: key[o]),
        "out": Region("out", 32 * n, True, None),
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

    key_words = [simplify(w) for w in spec.words(key)]
    zero = BitVecVal(0, 8)
    ctr0 = BitVecVal(0, 64)
    bl = BitVecVal(nbytes, 32)
    f32 = ZeroExt(24, flags)
    out = m.regions["out"]
    if any(i not in out.bytes for i in range(32 * n)):
        raise Unproved(f"{name}: output not wholly written")

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
    obj = assemble()
    for nbytes in (16, 32, 48):
        name = f"probe_sme2_packed{nbytes}"
        for groups in (1, 2):
            tk = time.monotonic()
            steps = prove_kernel(obj, name, nbytes, groups)
            print(f"PROVED {name:20s} groups={groups} ({16 * groups:2d} lanes, {steps:5d} steps, {time.monotonic() - tk:.2f}s)")
    print(f"\nALL 6/6 APPLE SME2 DIRECT-PACKED KERNEL CASES FORMALLY PROVED in {time.monotonic() - t0:.2f}s!")


if __name__ == "__main__":
    main()
