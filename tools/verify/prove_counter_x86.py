#!/usr/bin/env python3
"""Formally prove c/blake3_counter_x86-64_unix.S (zero-word-pruned AVX-512
and AVX2 counter & early-exit find kernels, #511, #585) equal to BLAKE3's
Lean spec for ALL templates, keys, wrapping u32 bases, 8-bit block lengths,
flags, masks, and targets under the System V x86-64 ABI.
"""
import os
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)

import aarch64
import lean_spec as spec
import prove_x86
import x86
from aarch64 import Ptr, Region, Unproved
from prove_hybrid import byte_symbols, same
from z3 import BitVec, BitVecVal, Concat, Extract, If, ZeroExt, simplify


def assemble_so():
    obj = os.path.join(tempfile.mkdtemp(), "blake3_counter_x86.o")
    src = os.path.join(ROOT, "c", "blake3_counter_x86-64_unix.S")
    clang = x86.tool("clang")
    subprocess.run([clang, "--target=x86_64-linux-gnu", "-c", src, "-o", obj], check=True)
    return x86.link(obj)


def prove_counter_kernel(so, name, lanes_per_group, n_words, counter_word, groups, trunc16=False):
    n = lanes_per_group * groups
    out_stride = 16 if trunc16 else 32
    raw_tmpl = [byte_symbols(f"t_{w}_", 4) if w < n_words else [BitVecVal(0, 8)] * 4 for w in range(16)]
    flat_tmpl = [b for word in raw_tmpl for b in word]
    key = byte_symbols("key", 32)
    regions = {
        "tmpl": Region("tmpl", 64, False, lambda o: flat_tmpl[o]),
        "key": Region("key", 32, False, lambda o: key[o]),
        "out": Region("out", max(out_stride * n, 1), True, None),
    }
    base = BitVec("base", 32)
    flags = BitVec("flags", 8)
    block_len = BitVec("blen", 8)
    base_and_word = ZeroExt(32, base) | BitVecVal(counter_word << 32, 64)
    flags_and_len = ZeroExt(56, flags) | (ZeroExt(56, block_len) << 8) | BitVecVal((1 << 16) if trunc16 else 0, 64)
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

    if any(i not in out.bytes for i in range(out_stride * n)):
        raise Unproved(f"{name}: output not wholly written")

    key_words = [simplify(w) for w in spec.words(key)]
    tmpl_words = [simplify(w) for w in spec.words(flat_tmpl)]
    ctr0 = BitVecVal(0, 64)
    bl = ZeroExt(24, block_len)
    f32 = ZeroExt(24, flags)
    out_words = 4 if trunc16 else 8

    for lane in range(n):
        msg = list(tmpl_words)
        msg[counter_word] = base + BitVecVal(lane, 32)
        want = spec.compress(key_words, msg, ctr0, bl, f32)[:out_words]
        got = spec.words([out.bytes[out_stride * lane + j] for j in range(out_stride)])
        for wi in range(out_words):
            if not same(got[wi], want[wi]):
                raise Unproved(f"{name} w={n_words} cw={counter_word} g={groups} t16={trunc16}: lane {lane} word {wi} differs!")
    return m.steps


def prove_find_kernel(so, name, lanes, n_words, counter_word, groups, hit_group):
    """Compositionally prove an AVX-512 (16-lane) or AVX2 (8-lane) early-exit
    `find` kernel: verifies every lane's `(word0 & mask) == target` structurally
    against `lean_spec.compress`, then proves the returned `(g << lanes) | mask`
    bit-for-bit over fresh symbolic hit bits."""
    raw_tmpl = [byte_symbols(f"t_{w}_", 4) if w < n_words else [BitVecVal(0, 8)] * 4 for w in range(16)]
    flat_tmpl = [b for word in raw_tmpl for b in word]
    key = byte_symbols("key", 32)
    regions = {
        "tmpl": Region("tmpl", 64, False, lambda o: flat_tmpl[o]),
        "key": Region("key", 32, False, lambda o: key[o]),
    }
    base = BitVec("base", 32)
    flags = BitVec("flags", 8)
    block_len = BitVec("blen", 8)
    mask = BitVec("mask", 32)
    target = BitVec("target", 32)
    args = [
        Ptr("tmpl", 0),
        BitVecVal(groups, 64),
        Ptr("key", 0),
        ZeroExt(32, base) | BitVecVal(counter_word << 32, 64),
        ZeroExt(56, flags) | (ZeroExt(56, block_len) << 8),
        ZeroExt(32, mask) | (ZeroExt(32, target) << 32),
    ]
    s = prove_x86.Setup(so, name, regions, args)
    m = s.m

    key_words = [simplify(w) for w in spec.words(key)]
    tmpl_words = [simplify(w) for w in spec.words(flat_tmpl)]
    ctr0 = BitVecVal(0, 64)
    bl = ZeroExt(24, block_len)
    f32 = ZeroExt(24, flags)

    def spec_w0(lane):
        msg = list(tmpl_words)
        msg[counter_word] = base + BitVecVal(lane, 32)
        return spec.compress(key_words, msg, ctr0, bl, f32)[0]

    fresh_bits = [BitVec(f"hit_{i}", 1) for i in range(lanes)]
    fresh_mask = ZeroExt(64 - lanes, Concat(*reversed(fresh_bits)))
    state = {"group": 0}

    def decide_hook(m, c):
        g = state["group"]
        gbase = lanes * g
        # At `test eax, eax; jnz .L_hit`, verify `eax` holds the exact `lanes`-bit mask!
        got_eax = Extract(lanes - 1, 0, m.x["rax"])
        # Reconstruct expected bits from `m.v[0]` / `m.k[1]` against Lean spec:
        if lanes == 16:
            # AVX-512: m.v[0][i] == (spec_w0 & mask), m.v[2][i] == target, k1[i] == (m.v[0][i] == m.v[2][i])
            for i in range(16):
                if not same(m.v[0][i], spec_w0(gbase + i) & mask):
                    raise Unproved(f"{name} g={g}: lane {i} masked word0 != spec")
                if not same(m.v[2][i], target):
                    raise Unproved(f"{name} g={g}: lane {i} target != target")
            want_k = simplify(Concat(*reversed([If(m.v[0][i] == m.v[2][i], BitVecVal(1, 1), BitVecVal(0, 1)) for i in range(16)])))
            if not m.k[1].eq(want_k):
                raise Unproved(f"{name} g={g}: k1 mask mismatch")
            if not simplify(got_eax).eq(want_k):
                raise Unproved(f"{name} g={g}: eax != k1")
        else:
            # AVX2: m.v[0][i] == If((spec_w0 & mask) == target, 0xFFFFFFFF, 0)
            for i in range(8):
                lane_val = m.v[0][i]
                if lane_val.decl().name() != "if":
                    raise Unproved(f"{name} g={g}: AVX2 lane {i} not pcmpeqd If")
                cond_i = lane_val.arg(0)
                if cond_i.decl().name() != "=" or not (same(cond_i.arg(0), spec_w0(gbase + i) & mask) and same(cond_i.arg(1), target)):
                    raise Unproved(f"{name} g={g}: AVX2 lane {i} cmp != spec")
                if not (lane_val.arg(1).eq(BitVecVal(0xFFFFFFFF, 32)) and lane_val.arg(2).eq(BitVecVal(0, 32))):
                    raise Unproved(f"{name} g={g}: AVX2 lane {i} pcmpeqd values wrong")
            want_m = simplify(Concat(*reversed([Extract(31, 31, m.v[0][i]) for i in range(8)])))
            if not simplify(got_eax).eq(want_m):
                raise Unproved(f"{name} g={g}: eax != vmovmskps")
        # Verify the branch condition `c` tests `eax != 0` (rejecting inverted
        # `jz`, wrong register, or constant branch).
        expected_cond = got_eax != BitVecVal(0, lanes)
        if not aarch64.unsatisfiable([], c != expected_cond):
            raise Unproved(f"{name} g={g}: branch condition {c} != (eax != 0)")
        if hit_group is not None and g == hit_group:
            m.x["rax"] = fresh_mask
            return True
        state["group"] = g + 1
        return False

    m.decide_hook = decide_hook
    s.run()

    if hit_group is None:
        want = BitVecVal(groups << lanes, 64)
        if not same(m.x["rax"], want):
            raise Unproved(f"{name}: miss return mismatch: got {m.x['rax']}, want {want}")
    else:
        want = BitVecVal(hit_group << lanes, 64) | fresh_mask
        if not aarch64.unsatisfiable([], m.x["rax"] != want):
            raise Unproved(f"{name}: hit return mismatch at group {hit_group}")
    return m.steps


def main():
    t0 = time.monotonic()
    so = assemble_so()
    cases = 0

    # 1. AVX-512 zero-word-pruned counter kernels (32 B & 16 B, every cw branch in 0..n_words).
    for nw in (4, 9, 13, 16):
        kname = f"blake3_servil_avx512_counter_w{nw}"
        # Test groups=0, EVERY cw in 0..nw (groups=2, 32 B) for 100% branch coverage, + trunc16=True (groups=2).
        for cw in range(nw):
            tk = time.monotonic()
            steps = prove_counter_kernel(so, kname, 16, nw, cw, 2, trunc16=False)
            cases += 1
            if cw in (0, nw - 1):
                print(f"PROVED {kname:<32} cw={cw:2d} g=2 t16=0 (32 symbolic lanes, {steps:4d} steps) in {time.monotonic() - tk:.2f}s")
        for groups, t16 in [(0, False), (1, True), (2, True)]:
            tk = time.monotonic()
            steps = prove_counter_kernel(so, kname, 16, nw, nw - 1, groups, trunc16=t16)
            cases += 1
            print(f"PROVED {kname:<32} cw={nw - 1:2d} g={groups} t16={int(t16)} ({16 * groups:2d} symbolic lanes, {steps:4d} steps) in {time.monotonic() - tk:.2f}s")

    # 2. AVX-512 early-exit find kernels.
    for nw in (4, 9, 13, 16):
        kname = f"blake3_servil_avx512_find_w{nw}"
        for label, groups, hg in [("zero", 0, None), ("miss1", 1, None), ("hit0", 1, 0), ("miss2", 2, None), ("hit1", 2, 1)]:
            tk = time.monotonic()
            steps = prove_find_kernel(so, kname, 16, nw, nw - 1, groups, hg)
            cases += 1
            print(f"PROVED {kname:<32} cw={nw - 1:2d} {label:<6} ({steps:4d} steps) in {time.monotonic() - tk:.2f}s")

    # 3. AVX2 zero-word-pruned counter kernels (32 B & 16 B).
    for nw in (4, 9, 16):
        kname = f"blake3_servil_avx2_counter_w{nw}"
        for cw, groups, t16 in [(0, 0, False), (0, 1, False), (nw - 1, 1, True), (nw - 1, 2, False), (nw - 1, 2, True)]:
            tk = time.monotonic()
            steps = prove_counter_kernel(so, kname, 8, nw, cw, groups, trunc16=t16)
            cases += 1
            print(f"PROVED {kname:<32} cw={cw:2d} g={groups} t16={int(t16)} ({8 * groups:2d} symbolic lanes, {steps:4d} steps) in {time.monotonic() - tk:.2f}s")

    # 4. AVX2 early-exit find kernels.
    for nw in (4, 9, 16):
        kname = f"blake3_servil_avx2_find_w{nw}"
        for label, groups, hg in [("zero", 0, None), ("miss1", 1, None), ("hit0", 1, 0), ("miss2", 2, None), ("hit1", 2, 1)]:
            tk = time.monotonic()
            steps = prove_find_kernel(so, kname, 8, nw, nw - 1, groups, hg)
            cases += 1
            print(f"PROVED {kname:<32} cw={nw - 1:2d} {label:<6} ({steps:4d} steps) in {time.monotonic() - tk:.2f}s")

    print(f"prove_counter_x86: all {cases} cases proved in {time.monotonic() - t0:.2f}s")


if __name__ == "__main__":
    main()
