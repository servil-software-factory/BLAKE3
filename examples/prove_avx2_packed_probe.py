#!/usr/bin/env python3
"""Formally prove probe_avx2_packed{16,32,48} equal to BLAKE3's Lean spec
for ALL inputs, keys, and flags under System V x86-64 ABI using tools/verify/prove_x86.py."""
import os
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "tools", "verify"))

import x86
import prove_x86
from aarch64 import Ptr, Region, Unproved
from prove_hybrid import byte_symbols, same
import lean_spec as spec
from z3 import BitVec, BitVecVal, ZeroExt, simplify


def assemble_so():
    obj = os.path.join(tempfile.mkdtemp(), "avx2_packed.o")
    src = os.path.join(ROOT, "examples", "avx2_packed_x86_64.S")
    clang = x86.tool("clang")
    subprocess.run([clang, "--target=x86_64-linux-gnu", "-c", src, "-o", obj], check=True)
    return x86.link(obj)


def prove_kernel(so, name, nbytes, groups=1):
    n = 8 * groups
    raw = byte_symbols("in_", n * nbytes)
    key = byte_symbols("key", 32)
    regions = {
        "input": Region("input", n * nbytes, False, lambda o: raw[o]),
        "key": Region("key", 32, False, lambda o: key[o]),
        "out": Region("out", 32 * n, True, None),
    }
    flags = BitVec("flags", 8)
    args = [Ptr("input", 0), BitVecVal(groups, 64), Ptr("key", 0), ZeroExt(56, flags), Ptr("out", 0)]
    s = prove_x86.Setup(so, name, regions, args)
    m = s.run()

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
    so = assemble_so()
    for nbytes in (16, 32, 48):
        name = f"probe_avx2_packed{nbytes}"
        for groups in (1, 2):
            tk = time.monotonic()
            steps = prove_kernel(so, name, nbytes, groups=groups)
            print(f"PROVED {name:20s} groups={groups} ({8 * groups:2d} lanes, {steps:5d} steps, {time.monotonic() - tk:.2f}s)")
    print(f"\nALL 6/6 AVX2 DIRECT-PACKED KERNEL CASES FORMALLY PROVED in {time.monotonic() - t0:.2f}s!")


if __name__ == "__main__":
    main()
