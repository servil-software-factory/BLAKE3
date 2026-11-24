#!/usr/bin/env python3
"""Formally prove probe_avx512_counter_range{,_52,_36} equal to BLAKE3's Lean spec
for ALL templates, keys, bases (wrapping u32), flags, counter_word in {0,8,12,15},
and groups in {1, 2} under System V x86-64 ABI using tools/verify/prove_x86.py.
"""
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
    obj = os.path.join(tempfile.mkdtemp(), "avx512_counter.o")
    src = os.path.join(ROOT, "examples", "avx512_counter_x86_64.S")
    clang = x86.tool("clang")
    subprocess.run([clang, "--target=x86_64-linux-gnu", "-c", src, "-o", obj], check=True)
    return x86.link(obj)


def prove_case(so, name, nwords, block_len, counter_word, groups):
    n = 16 * groups
    raw_tmpl = byte_symbols("t_", 4 * nwords) + [BitVecVal(0, 8)] * (64 - 4 * nwords)
    key = byte_symbols("key", 32)
    regions = {
        "tmpl": Region("tmpl", 64, False, lambda o: raw_tmpl[o]),
        "key": Region("key", 32, False, lambda o: key[o]),
        "out": Region("out", 32 * n, True, None),
    }
    base = BitVec("base", 32)
    flags = BitVec("flags", 8)
    base_and_word = ZeroExt(32, base) | BitVecVal(counter_word << 32, 64)
    flags_and_len = ZeroExt(56, flags) | BitVecVal(block_len << 8, 64)
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
    so = assemble_so()
    cases = [
        ("probe_avx512_counter_range",    16, 64,  0, 1),
        ("probe_avx512_counter_range",    16, 52, 12, 2),
        ("probe_avx512_counter_range",    16, 64, 15, 2),
        ("probe_avx512_counter_range_52", 13, 52, 12, 2),
        ("probe_avx512_counter_range_36",  9, 36,  8, 2),
    ]
    for name, nw, blen, cw, grp in cases:
        tk = time.monotonic()
        steps = prove_case(so, name, nw, blen, cw, grp)
        print(f"PROVED {name:30s} len={blen:2d} word={cw:2d} groups={grp} "
              f"({steps} steps) in {time.monotonic() - tk:.2f}s")
    print(f"\nALL {len(cases)} AVX-512 counter-range cases FORMALLY PROVED in {time.monotonic() - t0:.2f}s!")


if __name__ == "__main__":
    main()
