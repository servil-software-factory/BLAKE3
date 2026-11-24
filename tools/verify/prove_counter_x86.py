#!/usr/bin/env python3
"""Formally prove c/blake3_counter_x86-64_unix.S (universal <= 64 B AVX-512
and AVX2 counter-grinding kernels, #511) equal to BLAKE3's Lean spec for ALL
64-byte templates, keys, wrapping u32 bases, 8-bit block lengths, and flags
under the System V x86-64 ABI.
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
    obj = os.path.join(tempfile.mkdtemp(), "blake3_counter_x86.o")
    src = os.path.join(ROOT, "c", "blake3_counter_x86-64_unix.S")
    clang = x86.tool("clang")
    subprocess.run([clang, "--target=x86_64-linux-gnu", "-c", src, "-o", obj], check=True)
    return x86.link(obj)


def prove_kernel(so, name, lanes_per_group, counter_word, groups):
    n = lanes_per_group * groups
    raw_tmpl = byte_symbols("t_", 64)
    key = byte_symbols("key", 32)
    regions = {
        "tmpl": Region("tmpl", 64, False, lambda o: raw_tmpl[o]),
        "key": Region("key", 32, False, lambda o: key[o]),
        "out": Region("out", max(32 * n, 1), True, None),
    }
    base = BitVec("base", 32)
    flags = BitVec("flags", 8)
    block_len = BitVec("blen", 8)
    base_and_word = ZeroExt(32, base) | BitVecVal(counter_word << 32, 64)
    flags_and_len = ZeroExt(56, flags) | (ZeroExt(56, block_len) << 8)
    args = [
        Ptr("tmpl", 0),
        BitVecVal(groups, 64),
        Ptr("key", 0),
        base_and_word,
        flags_and_len,
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
    so = assemble_so()
    cases = 0

    # AVX-512 branches on counter_word in both init and loop increment:
    # prove groups=0 plus EVERY counter_word 0..15 across groups=2 (100% branch coverage).
    for cw, groups in [(0, 0)] + [(w, 2) for w in range(16)] + [(12, 3)]:
        tk = time.monotonic()
        steps = prove_kernel(so, "blake3_servil_avx512_counter", 16, cw, groups)
        cases += 1
        print(
            f"PROVED blake3_servil_avx512_counter cw={cw:2d} groups={groups} "
            f"({16 * groups:2d} symbolic lanes, {steps:4d} steps) in {time.monotonic() - tk:.2f}s"
        )

    # AVX2 indexes [rsp + counter_word * 32] uniformly without per-word branches:
    # prove groups=0, groups=1, groups=2, and groups=3 across boundary/interior words.
    for cw, groups in [(0, 0), (0, 1), (7, 1), (12, 1), (15, 1), (0, 2), (12, 2), (15, 2), (12, 3)]:
        tk = time.monotonic()
        steps = prove_kernel(so, "blake3_servil_avx2_counter", 8, cw, groups)
        cases += 1
        print(
            f"PROVED blake3_servil_avx2_counter   cw={cw:2d} groups={groups} "
            f"({8 * groups:2d} symbolic lanes, {steps:4d} steps) in {time.monotonic() - tk:.2f}s"
        )

    print(f"prove_counter_x86: all {cases} cases proved in {time.monotonic() - t0:.2f}s")


if __name__ == "__main__":
    main()
