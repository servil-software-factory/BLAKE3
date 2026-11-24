#!/usr/bin/env python3
"""Formally prove probe_neon_counter_{p11,p10,p11_52} equal to BLAKE3's Lean spec
for ALL templates, keys, wrapping bases, flags, counter_words {0, 12, 15},
and groups {1, 2} using tools/verify/prove_hybrid.py + Z3.
"""
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
    obj = os.path.join(tempfile.mkdtemp(), "neon_counter.o")
    src = os.path.join(ROOT, "examples", "neon_counter_elf.S")
    aarch64.OBJDUMP = aarch64.LLVM_OBJDUMP or "llvm-objdump"
    subprocess.run([CLANG, "-c", "--target=aarch64-linux-gnu", src, "-o", obj], check=True)
    return obj


def prove_case(obj, name, n_lanes, nwords, block_len, counter_word, groups):
    n = n_lanes * groups
    raw_tmpl = byte_symbols("t_", 4 * nwords) + [BitVecVal(0, 8)] * (64 - 4 * nwords)
    key = byte_symbols("key", 32)
    regions = {
        "tmpl": Region("tmpl", 64, False, lambda o: raw_tmpl[o]),
        "key": Region("key", 32, False, lambda o: key[o]),
        "out": Region("out", 32 * n, True, None),
    }
    real = aarch64.disassemble
    aarch64.disassemble = lambda o, a, b: real(o, a, b, llvm=True)
    try:
        s = Setup(obj, name, name + "_end", regions)
    finally:
        aarch64.disassemble = real

    base = BitVec("base", 32)
    flags = BitVec("flags", 8)
    m = s.m
    m.x[0] = Ptr("tmpl", 0)
    m.x[1] = Ptr("key", 0)
    m.x[2] = ZeroExt(32, base) | BitVecVal(counter_word << 32, 64)
    m.x[3] = ZeroExt(56, flags) | BitVecVal(block_len << 8, 64)
    m.x[4] = Ptr("out", 0)
    m.x[5] = BitVecVal(groups, 64)
    s.run_and_check_abi()

    key_words = [simplify(w) for w in spec.words(key)]
    tmpl_words = [simplify(w) for w in spec.words(raw_tmpl)]
    ctr0 = BitVecVal(0, 64)
    bl = BitVecVal(block_len, 32)
    f32 = ZeroExt(24, flags)
    out = m.regions["out"]
    if any(i not in out.bytes for i in range(32 * n)):
        raise Unproved(f"{name}: output not wholly written")

    for lane in range(n):
        msg = list(tmpl_words)
        msg[counter_word] = base + BitVecVal(lane, 32)
        want = spec.compress(key_words, msg, ctr0, bl, f32)[:8]
        got = spec.words([out.bytes[32 * lane + j] for j in range(32)])
        for wi in range(8):
            if not same(got[wi], want[wi]):
                raise Unproved(f"{name} w={counter_word}: lane {lane} word {wi} differs!")
    return m.steps


def main():
    t0 = time.monotonic()
    obj = assemble()
    cases = [
        ("probe_neon_counter_p11",    11, 16, 64,  0, 1),
        ("probe_neon_counter_p11",    11, 16, 52, 12, 2),
        ("probe_neon_counter_p11",    11, 16, 64, 15, 2),
        ("probe_neon_counter_p10",    10, 16, 52, 12, 2),
        ("probe_neon_counter_p11_52", 11, 13, 52, 12, 2),
    ]
    for name, nl, nw, blen, cw, grp in cases:
        tk = time.monotonic()
        steps = prove_case(obj, name, nl, nw, blen, cw, grp)
        print(f"PROVED {name:26s} len={blen:2d} word={cw:2d} groups={grp} "
              f"({steps} steps) in {time.monotonic() - tk:.2f}s")
    print(f"\nALL {len(cases)} NEON counter-range cases FORMALLY PROVED in {time.monotonic() - t0:.2f}s!")


if __name__ == "__main__":
    main()
