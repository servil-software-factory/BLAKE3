#!/usr/bin/env python3
"""Formally prove all 10 probe AArch64 NEON dual-quad + sequential zero-spill
scalar kernels in examples/p10_p11_elf.S equal to BLAKE3's Lean specification
for EVERY input, key, counter, and flags, plus ABI/stack/x18 preservation!

Runs tools/verify/{aarch64,canon,lean_spec,prove_hybrid}.py directly on the
assembled ELF object file.
"""
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
    ("probe_p10_64", 10, None, 16),
    ("probe_p11_64", 11, None, 16),
    ("probe_p11_64_steps", 11, None, 16),
    ("probe_p10_packed32", 10, 32, 8),
    ("probe_p11_packed32", 11, 32, 8),
    ("probe_p11_packed32_steps", 11, 32, 8),
    ("probe_p10_packed48", 10, 48, 12),
    ("probe_p11_packed48", 11, 48, 12),
    ("probe_p11_packed16", 11, 16, 4),
    ("probe_p11_packed64", 11, 64, 16),
]


def assemble():
    obj = os.path.join(tempfile.mkdtemp(), "p10_p11_probe.o")
    src = os.path.join(ROOT, "examples", "p10_p11_elf.S")
    if aarch64.ARM_HOST:
        cmd = ["cc", "-c", src, "-o", obj]
    else:
        cmd = ["clang", "--target=aarch64-linux-gnu", "-c", src, "-o", obj]
        aarch64.OBJDUMP = aarch64.LLVM_OBJDUMP or "llvm-objdump"
    subprocess.run(cmd, check=True)
    return obj


def prove_kernel(obj, name, n, stride, nwords):
    msg_bytes = 4 * nwords
    key = byte_symbols("key", 32)
    regions = {
        "key": Region("key", 32, False, lambda o: key[o]),
        "out": Region("out", 32 * n, True, None),
    }
    zero_byte = BitVecVal(0, 8)
    inputs_padded = []
    if stride is None:
        raw_inputs = [byte_symbols(f"in{i}_", 64) for i in range(n)]
        for i in range(n):
            regions[f"input{i}"] = Region(f"input{i}", 64, False, (lambda b: lambda o: b[o])(raw_inputs[i]))
            inputs_padded.append(raw_inputs[i])
        table = Region("table", 8 * n, False, None)
        for i in range(n):
            for j in range(8):
                table.bytes[8 * i + j] = (Ptr(f"input{i}", 0), j)
        regions["table"] = table
        x0_val = Ptr("table", 0)
    else:
        assert stride == msg_bytes
        flat = byte_symbols("in_", stride * n)
        regions["packed"] = Region("packed", stride * n, False, lambda o: flat[o])
        for i in range(n):
            msg = flat[i * stride:(i + 1) * stride]
            inputs_padded.append(msg + [zero_byte] * (64 - stride))
        x0_val = Ptr("packed", 0)

    s = Setup(obj, name, name + "_end", regions)
    counter = BitVec("counter", 64)
    flags = BitVec("flags", 8)
    block_len = BitVec("block_len", 8)
    m = s.m
    m.x[0] = x0_val
    m.x[1] = Ptr("key", 0)
    m.x[2] = counter
    m.x[3] = ZeroExt(56, flags)
    m.x[4] = ZeroExt(56, block_len)
    m.x[5] = Ptr("out", 0)

    s.run_and_check_abi()

    key_words = [simplify(w) for w in spec.words(key)]
    c = simplify(counter)
    f = simplify(ZeroExt(24, flags))
    bl = simplify(ZeroExt(24, block_len))

    out = m.regions["out"]
    if any(i not in out.bytes for i in range(32 * n)):
        raise Unproved(f"{name}: output not wholly written")

    for lane in range(n):
        block = spec.words(inputs_padded[lane])
        want = spec.compress(key_words, block, c, bl, f)[:8]
        got = spec.words([out.bytes[32 * lane + j] for j in range(32)])
        for wi in range(8):
            if not same(got[wi], want[wi]):
                raise Unproved(f"{name}: lane {lane} word {wi} differs from BLAKE3 spec!")
    return m.steps


def main():
    t0 = time.monotonic()
    obj = assemble()
    for name, n, stride, nwords in KERNELS:
        tk = time.monotonic()
        steps = prove_kernel(obj, name, n, stride, nwords)
        print(f"PROVED {name:26s} ({n:2d} lanes, {steps:5d} steps, {time.monotonic() - tk:.2f}s)")
    print(f"\nALL {len(KERNELS)}/{len(KERNELS)} KERNELS FORMALLY PROVED against BLAKE3 Lean spec in {time.monotonic() - t0:.2f}s!")


if __name__ == "__main__":
    main()
