#!/usr/bin/env python3
"""Formally prove c/blake3_chain_x86-64_unix.S (AVX-512 16-lane and AVX2 8-lane
fused multi-step 32-byte hash-chain kernels, #584) equal to iterated BLAKE3's
Lean spec for ALL 32-byte chain inputs, keys, wrapping u32 start steps, and
flags under the System V x86-64 ABI.
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
from z3 import BitVec, BitVecVal, Concat, ZeroExt, simplify


def assemble_so():
    obj = os.path.join(tempfile.mkdtemp(), "blake3_chain_x86.o")
    src = os.path.join(ROOT, "c", "blake3_chain_x86-64_unix.S")
    clang = x86.tool("clang")
    subprocess.run([clang, "--target=x86_64-linux-gnu", "-c", src, "-o", obj], check=True)
    return x86.link(obj)


def prove_kernel(so, name, lanes_per_group, steps, groups):
    n = lanes_per_group * groups
    raw_init = byte_symbols("i_", max(32 * n, 1))
    key = byte_symbols("key", 32)
    regions = {
        "init": Region("init", max(32 * n, 1), False, lambda o: raw_init[o]),
        "key": Region("key", 32, False, lambda o: key[o]),
        "out": Region("out", max(16 * n, 1), True, None),
    }
    flags = BitVec("flags", 8)
    junk_flags = BitVec("junk_flags", 32)
    junk_steps = BitVec("junk_steps", 32)
    args = [
        Ptr("init", 0),
        BitVecVal(groups, 64),
        Ptr("key", 0),
        Concat(junk_flags, ZeroExt(24, flags)),
        Ptr("out", 0),
        Concat(junk_steps, BitVecVal(steps, 32)),
    ]
    s = prove_x86.Setup(so, name, regions, args)
    m = s.run()

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
    so = assemble_so()
    cases = 0
    for name, lpg in [
        ("blake3_servil_avx512_chain32_trunc16", 16),
        ("blake3_servil_avx2_chain32_trunc16", 8),
    ]:
        for steps, groups in [(0, 1), (1, 0), (1, 1), (2, 1), (3, 1), (2, 2)]:
            tk = time.monotonic()
            nsteps = prove_kernel(so, name, lpg, steps, groups)
            cases += 1
            print(
                f"PROVED {name:38s} steps={steps} groups={groups} "
                f"({lpg * groups:2d} symbolic lanes, {nsteps:5d} steps) in {time.monotonic() - tk:.2f}s"
            )
    print(f"prove_chain_x86: all {cases} cases proved in {time.monotonic() - t0:.2f}s")


if __name__ == "__main__":
    main()
