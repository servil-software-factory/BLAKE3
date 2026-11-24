#!/usr/bin/env python3
"""Formally prove exact production-ABI runtime-`blocks` zero-spill sequential-scalar
kernels (probe_k10_prod, probe_k11_prod, probe_p10_prod, probe_p11_prod) against
BLAKE3's Lean spec for ALL inputs, keys, counters, flags, flags_start, flags_end,
and last_len across blocks = 1, 2, 4, and 16!"""
import os
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "tools", "verify"))

import aarch64
from aarch64 import Region, Ptr, Unproved
import lean_spec as spec
from prove_hybrid import Setup, byte_symbols, same
from z3 import BitVec, BitVecVal, Concat, ZeroExt, simplify

CASES = [
    ("probe_k10_prod", 10, True, 1),
    ("probe_k10_prod", 10, True, 2),
    ("probe_k10_prod", 10, True, 16),
    ("probe_k11_prod", 11, True, 1),
    ("probe_k11_prod", 11, True, 2),
    ("probe_k11_prod", 11, True, 16),
    ("probe_p10_prod", 10, False, 1),
    ("probe_p10_prod", 10, False, 4),
    ("probe_p11_prod", 11, False, 1),
    ("probe_p11_prod", 11, False, 2),
    ("probe_p11_prod", 11, False, 4),
    ("probe_p11_prod", 11, False, 16),
]


def assemble():
    obj = os.path.join(tempfile.mkdtemp(), "k11_prod.o")
    src = os.path.join(ROOT, "examples", "k11_prod_elf.S")
    cmd = ["clang", "--target=aarch64-linux-gnu", "-c", src, "-o", obj]
    aarch64.OBJDUMP = aarch64.LLVM_OBJDUMP or "llvm-objdump"
    subprocess.run(cmd, check=True)
    return obj


def prove_case(obj, name, n, per_ctr, blocks):
    msg_len = 64 * blocks
    raw_inputs = [byte_symbols(f"in{i}_", msg_len) for i in range(n)]
    key = byte_symbols("key", 32)
    regions = {f"input{i}": Region(f"input{i}", msg_len, False, (lambda b: lambda o: b[o])(raw_inputs[i])) for i in range(n)}
    regions["key"] = Region("key", 32, False, lambda o: key[o])
    regions["out"] = Region("out", 32 * n, True, None)
    table = Region("table", 8 * n, False, None)
    for i in range(n):
        for j in range(8):
            table.bytes[8 * i + j] = (Ptr(f"input{i}", 0), j)
    regions["table"] = table

    s = Setup(obj, name, name + "_end", regions)
    counter = BitVec("counter", 64)
    flags = BitVec("flags", 8)
    f_start = BitVec("flags_start", 8)
    f_end = BitVec("flags_end", 8)
    last_len = BitVec("last_len", 8)
    packed_flags = Concat(BitVecVal(0, 32), last_len, f_end, f_start, flags)

    m = s.m
    m.x[0] = Ptr("table", 0)
    m.x[1] = BitVecVal(blocks, 64)
    m.x[2] = Ptr("key", 0)
    m.x[3] = counter
    m.x[4] = packed_flags
    m.x[5] = Ptr("out", 0)
    s.run_and_check_abi()

    key_words = [simplify(w) for w in spec.words(key)]
    zero8 = BitVecVal(0, 8)
    bl64 = BitVecVal(64, 32)
    bl_last = ZeroExt(24, last_len)
    out = m.regions["out"]
    if any(i not in out.bytes for i in range(32 * n)):
        raise Unproved(f"{name} (blocks={blocks}): output not wholly written")

    for lane in range(n):
        c = simplify(counter + BitVecVal(lane, 64)) if per_ctr else counter
        cv = key_words
        for b in range(blocks):
            block = spec.words(raw_inputs[lane][64 * b:64 * b + 64])
            f = simplify(ZeroExt(24, flags | (f_start if b == 0 else zero8) | (f_end if b == blocks - 1 else zero8)))
            bl = bl_last if b == blocks - 1 else bl64
            cv = spec.compress(cv, block, c, bl, f)[:8]
        got = spec.words([out.bytes[32 * lane + j] for j in range(32)])
        for wi in range(8):
            if not same(got[wi], cv[wi]):
                raise Unproved(f"{name} (blocks={blocks}): lane {lane} word {wi} differs!")
    return m.steps


def main():
    t0 = time.monotonic()
    obj = assemble()
    for name, n, per_ctr, blocks in CASES:
        tk = time.monotonic()
        steps = prove_case(obj, name, n, per_ctr, blocks)
        print(f"PROVED {name:15s} blocks={blocks:2d} ({n:2d} lanes, {steps:6d} steps, {time.monotonic() - tk:.2f}s)")
    print(f"\nALL {len(CASES)}/{len(CASES)} PRODUCTION-ABI KERNEL CASES FORMALLY PROVED in {time.monotonic() - t0:.2f}s!")


if __name__ == "__main__":
    main()
