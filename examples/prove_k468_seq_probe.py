#!/usr/bin/env python3
"""Formally prove probe_k4_seq (2+2), probe_k6_seq (4+2), and probe_k8_seq (6+2)
equal to BLAKE3's Lean spec for ALL inputs, keys, counters, flags, and ABI."""
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
from z3 import BitVec, BitVecVal, ZeroExt, simplify

KERNELS = [
    ("probe_k4_seq", 4),
    ("probe_k6_seq", 6),
    ("probe_k8_seq", 8),
]


def assemble():
    obj = os.path.join(tempfile.mkdtemp(), "k468_seq.o")
    src = os.path.join(ROOT, "examples", "k468_seq_elf.S")
    cmd = ["clang", "--target=aarch64-linux-gnu", "-march=armv8.2-a+sha3", "-c", src, "-o", obj]
    aarch64.OBJDUMP = aarch64.LLVM_OBJDUMP or "llvm-objdump"
    subprocess.run(cmd, check=True)
    return obj


def prove_kernel(obj, name, n):
    blocks = 16
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
    m = s.m
    m.x[0], m.x[1], m.x[2], m.x[3], m.x[4] = Ptr("table", 0), Ptr("key", 0), counter, ZeroExt(56, flags), Ptr("out", 0)
    s.run_and_check_abi()

    key_words = [simplify(w) for w in spec.words(key)]
    zero8, start8, end8, bl64 = BitVecVal(0, 8), BitVecVal(1, 8), BitVecVal(2, 8), BitVecVal(64, 32)
    out = m.regions["out"]
    if any(i not in out.bytes for i in range(32 * n)):
        raise Unproved(f"{name}: output not wholly written")
    for lane in range(n):
        c = simplify(counter + BitVecVal(lane, 64))
        cv = key_words
        for b in range(blocks):
            block = spec.words(raw_inputs[lane][64 * b:64 * b + 64])
            f = simplify(ZeroExt(24, flags | (start8 if b == 0 else zero8) | (end8 if b == blocks - 1 else zero8)))
            cv = spec.compress(cv, block, c, bl64, f)[:8]
        got = spec.words([out.bytes[32 * lane + j] for j in range(32)])
        for wi in range(8):
            if not same(got[wi], cv[wi]):
                raise Unproved(f"{name}: lane {lane} word {wi} differs!")
    return m.steps


def main():
    t0 = time.monotonic()
    obj = assemble()
    for name, n in KERNELS:
        tk = time.monotonic()
        steps = prove_kernel(obj, name, n)
        print(f"PROVED {name:16s} ({n} lanes x 16 blocks, {steps:6d} steps, {time.monotonic() - tk:.2f}s)")
    print(f"\nALL 3/3 k4/k6/k8 ZERO-SPILL KERNELS FORMALLY PROVED in {time.monotonic() - t0:.2f}s!")


if __name__ == "__main__":
    main()
