#!/usr/bin/env python3
"""Formally prove c/blake3_packed_x86-64_unix.S (all six direct-load
Packed<16/32/48> kernels: AVX-512 and AVX2) equal to BLAKE3's Lean spec
for ALL inputs, keys, and flags across groups in {0, 1, 2} under the
System V x86-64 ABI.
"""
import os
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)

import lean_spec as spec
import prove_x86
import x86
from aarch64 import Ptr, Region, Unproved
from prove_hybrid import byte_symbols, same
from z3 import BitVec, BitVecVal, ZeroExt, simplify


def assemble_so():
    obj = os.path.join(tempfile.mkdtemp(), "blake3_packed_x86.o")
    src = os.path.join(ROOT, "c", "blake3_packed_x86-64_unix.S")
    clang = x86.tool("clang")
    subprocess.run([clang, "--target=x86_64-linux-gnu", "-c", src, "-o", obj], check=True)
    return x86.link(obj)


def prove_kernel(so, name, lanes_per_group, nbytes, groups):
    n = lanes_per_group * groups
    raw = byte_symbols("in_", max(n * nbytes, 1))
    key = byte_symbols("key", 32)
    regions = {
        "input": Region("input", max(n * nbytes, 1), False, lambda o: raw[o]),
        "key": Region("key", 32, False, lambda o: key[o]),
        "out": Region("out", max(32 * n, 1), True, None),
    }
    flags = BitVec("flags", 8)
    args = [
        Ptr("input", 0),
        BitVecVal(groups, 64),
        Ptr("key", 0),
        ZeroExt(56, flags),
        Ptr("out", 0),
    ]
    s = prove_x86.Setup(so, name, regions, args)
    m = s.run()

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
    so = assemble_so()
    specs = [
        ("blake3_servil_avx512_packed", 16),
        ("blake3_servil_avx2_packed", 8),
    ]
    cases = 0
    for prefix, lpg in specs:
        for nbytes in (16, 32, 48):
            name = f"{prefix}{nbytes}"
            for groups in (0, 1, 2):
                tk = time.monotonic()
                steps = prove_kernel(so, name, lpg, nbytes, groups)
                cases += 1
                print(
                    f"PROVED {name:30s} groups={groups} ({lpg * groups:2d} symbolic lanes, "
                    f"{steps:4d} steps) in {time.monotonic() - tk:.2f}s"
                )
    print(f"prove_packed_x86: all {cases} cases proved in {time.monotonic() - t0:.2f}s")


if __name__ == "__main__":
    main()
