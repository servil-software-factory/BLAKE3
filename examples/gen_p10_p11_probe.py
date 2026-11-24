#!/usr/bin/env python3
"""Generate probe-only AArch64 NEON dual-quad + sequential zero-spill scalar
1-block kernels (p10 = 8+2, p11 = 8+3) for both pointer-table 64 B blocks
and contiguous Packed<16/32/64> inputs (#509).

Usage:
    python3 examples/gen_p10_p11_probe.py
"""
from pathlib import Path

IV = [0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A]
SCHEDULE = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8],
    [3, 4, 10, 12, 13, 2, 7, 14, 6, 5, 9, 0, 11, 15, 8, 1],
    [10, 7, 12, 9, 14, 3, 13, 15, 4, 0, 11, 2, 5, 8, 1, 6],
    [12, 13, 9, 11, 15, 10, 14, 8, 7, 2, 5, 3, 0, 1, 6, 4],
    [9, 14, 11, 5, 8, 12, 15, 1, 13, 3, 0, 10, 2, 6, 4, 7],
    [11, 15, 5, 0, 1, 9, 8, 6, 14, 10, 2, 12, 3, 4, 7, 13],
]
COLS = [(0, 4, 8, 12), (1, 5, 9, 13), (2, 6, 10, 14), (3, 7, 11, 15)]
DIAGS = [(0, 5, 10, 15), (1, 6, 11, 12), (2, 7, 8, 13), (3, 4, 9, 14)]
ROT8_TABLE = [1, 2, 3, 0, 5, 6, 7, 4, 9, 10, 11, 8, 13, 14, 15, 12]

F_X19 = 0       # x19..x30 (96 B)
F_D8 = 96       # d8..d15 (64 B)
F_IVT = 160     # IV[0..4] (16 B)
F_ROT8 = 176    # ROT8_TABLE (16 B)
F_DSPILL = 192  # unit 1 spilled d row (64 B)
F_MSG0 = 256    # unit 0 transposed msg words (256 B)
F_MSG1 = 512    # unit 1 transposed msg words (256 B)
FRAME = 768

SC_REGS = [f"w{i}" for i in list(range(6, 18)) + list(range(19, 23))]


def interleave(a, b):
    """Proportionally interleave two instruction streams."""
    if not a:
        return list(b)
    if not b:
        return list(a)
    out = []
    na, nb = len(a), len(b)
    ia = ib = 0
    while ia < na or ib < nb:
        if ia < na and (ib == nb or ia * nb <= ib * na):
            out.append(a[ia])
            ia += 1
        else:
            out.append(b[ib])
            ib += 1
    return out


def scalar_one(slot, stride, nwords):
    """One complete 1-block compression on the zero-spill scalar register file
    (w6..w17, w19..w22; temp w23; input ptr x24), storing 32 B to [x25 + 32*slot]."""
    s = SC_REGS
    L = []
    e = L.append
    if stride is None:
        e(f"ldr x24, [x30, #{8 * slot}]")
    elif slot == 0:
        e("mov x24, x30")
    else:
        e(f"add x24, x30, #{stride * slot}")
    # Load key into a/b rows (s[0..8]), IV into c row (s[8..12]), d row from x27/w29/w28.
    for i in range(0, 8, 2):
        e(f"ldp {s[i]}, {s[i + 1]}, [x26, #{4 * i}]")
    e(f"ldp {s[8]}, {s[9]}, [sp, #{F_IVT}]")
    e(f"ldp {s[10]}, {s[11]}, [sp, #{F_IVT + 8}]")
    e(f"mov {s[12]}, w27")
    e(f"lsr x{s[13][1:]}, x27, #32")
    e(f"mov {s[14]}, w29")
    e(f"mov {s[15]}, w28")

    # 7 rounds, zero stack spills, skipping zero message words (>= nwords).
    for r in range(7):
        sched = SCHEDULE[r]
        for quads, base in ((COLS, 0), (DIAGS, 8)):
            for half, (r1, r2) in ((0, (16, 12)), (1, (8, 7))):
                for (ai, bi, ci, di), k in zip(quads, range(0, 8, 2)):
                    a, b, c, d = s[ai], s[bi], s[ci], s[di]
                    word = sched[base + k + half]
                    if word < nwords:
                        e(f"ldr w23, [x24, #{4 * word}]")
                        e(f"add {a}, {a}, w23")
                    e(f"add {a}, {a}, {b}")
                    e(f"eor {d}, {d}, {a}")
                    e(f"ror {d}, {d}, #{r1}")
                    e(f"add {c}, {c}, {d}")
                    e(f"eor {b}, {b}, {c}")
                    e(f"ror {b}, {b}, #{r2}")

    # Finalize CV and store directly to [x25 + 32*slot].
    for i in range(8):
        e(f"eor {s[i]}, {s[i]}, {s[i + 8]}")
    off = 32 * slot
    for i in range(0, 8, 2):
        e(f"stp {s[i]}, {s[i + 1]}, [x25, #{off + 4 * i}]")
    return L


def neon_quad_g(base_reg, spill_d, ai, bi, ci, di, msg_base, mx, my, nwords):
    a = f"v{base_reg + ai}"
    b = f"v{base_reg + bi}"
    c = f"v{base_reg + ci}"
    t = "v30"
    L = []
    e = L.append
    if spill_d:
        doff = F_DSPILL + 16 * (di - 12)
        e(f"ldr q29, [sp, #{doff}]")
        if mx < nwords:
            e(f"ldr q28, [sp, #{msg_base + 16 * mx}]")
            e(f"add {a}.4s, {a}.4s, v28.4s")
        e(f"add {a}.4s, {a}.4s, {b}.4s")
        e(f"eor v29.16b, v29.16b, {a}.16b")
        e("rev32 v29.8h, v29.8h")
        e(f"add {c}.4s, {c}.4s, v29.4s")
        e(f"eor {b}.16b, {b}.16b, {c}.16b")
        e(f"shl {t}.4s, {b}.4s, #20")
        e(f"sri {t}.4s, {b}.4s, #12")
        if my < nwords:
            e(f"ldr q28, [sp, #{msg_base + 16 * my}]")
            e(f"add {a}.4s, {a}.4s, v28.4s")
        e(f"add {a}.4s, {a}.4s, {t}.4s")
        e(f"eor v29.16b, v29.16b, {a}.16b")
        e("tbl v29.16b, {v29.16b}, v31.16b")
        e(f"add {c}.4s, {c}.4s, v29.4s")
        e(f"str q29, [sp, #{doff}]")
        e(f"eor {t}.16b, {t}.16b, {c}.16b")
        e(f"shl {b}.4s, {t}.4s, #25")
        e(f"sri {b}.4s, {t}.4s, #7")
    else:
        d = f"v{base_reg + di}"
        if mx < nwords:
            e(f"ldr q28, [sp, #{msg_base + 16 * mx}]")
            e(f"add {a}.4s, {a}.4s, v28.4s")
        e(f"add {a}.4s, {a}.4s, {b}.4s")
        e(f"eor {d}.16b, {d}.16b, {a}.16b")
        e(f"rev32 {d}.8h, {d}.8h")
        e(f"add {c}.4s, {c}.4s, {d}.4s")
        e(f"eor {b}.16b, {b}.16b, {c}.16b")
        e(f"shl {t}.4s, {b}.4s, #20")
        e(f"sri {t}.4s, {b}.4s, #12")
        if my < nwords:
            e(f"ldr q28, [sp, #{msg_base + 16 * my}]")
            e(f"add {a}.4s, {a}.4s, v28.4s")
        e(f"add {a}.4s, {a}.4s, {t}.4s")
        e(f"eor {d}.16b, {d}.16b, {a}.16b")
        e(f"tbl {d}.16b, {{{d}.16b}}, v31.16b")
        e(f"add {c}.4s, {c}.4s, {d}.4s")
        e(f"eor {t}.16b, {t}.16b, {c}.16b")
        e(f"shl {b}.4s, {t}.4s, #25")
        e(f"sri {b}.4s, {t}.4s, #7")
    return L


def neon_transpose_quad(slots, stride, msg_base, nwords):
    """Transpose 4 lanes (`slots`) of `nwords` words (4, 8, 12, or 16) into [sp, #msg_base]."""
    assert nwords in (4, 8, 12, 16)
    L = []
    e = L.append
    # Use v0..v7 as temps (key is loaded after transpose).
    for q in range(nwords // 4):
        if stride is None:
            for j in range(4):
                e(f"ldr x6, [x30, #{8 * slots[j]}]")
                e(f"ldr q{j}, [x6, #{16 * q}]")
        else:
            for j in range(4):
                off = stride * slots[j] + 16 * q
                e(f"ldr q{j}, [x30, #{off}]")
        e("trn1 v4.4s, v0.4s, v1.4s")
        e("trn2 v5.4s, v0.4s, v1.4s")
        e("trn1 v6.4s, v2.4s, v3.4s")
        e("trn2 v7.4s, v2.4s, v3.4s")
        e("trn1 v0.2d, v4.2d, v6.2d")
        e("trn1 v1.2d, v5.2d, v7.2d")
        e("trn2 v2.2d, v4.2d, v6.2d")
        e("trn2 v3.2d, v5.2d, v7.2d")
        e(f"stp q0, q1, [sp, #{msg_base + 64 * q}]")
        e(f"stp q2, q3, [sp, #{msg_base + 64 * q + 32}]")
    return L


def neon_store_quad(base_reg, slots):
    """Store 4 lanes of CV (rows 0..8) from v{base_reg..base_reg+8} to [x25 + 32*slot]."""
    L = []
    e = L.append
    # Free temps: v8..v15 (for u0) or v24..v31 (for u1); use v24..v31 for u0 after u1's
    # block_end? Wait: u0 stores first using v8..v15 (u0's c/d are dead after block_end!),
    # then u1 stores using v0..v7 (dead after u0 stored!).
    temps = list(range(8, 16)) if base_reg == 0 else list(range(0, 8))
    t = temps
    for half in (0, 4):
        h = [base_reg + half + j for j in range(4)]
        e(f"trn1 v{t[0]}.4s, v{h[0]}.4s, v{h[1]}.4s")
        e(f"trn2 v{t[1]}.4s, v{h[0]}.4s, v{h[1]}.4s")
        e(f"trn1 v{t[2]}.4s, v{h[2]}.4s, v{h[3]}.4s")
        e(f"trn2 v{t[3]}.4s, v{h[2]}.4s, v{h[3]}.4s")
        e(f"trn1 v{t[4]}.2d, v{t[0]}.2d, v{t[2]}.2d")
        e(f"trn1 v{t[5]}.2d, v{t[1]}.2d, v{t[3]}.2d")
        e(f"trn2 v{t[6]}.2d, v{t[0]}.2d, v{t[2]}.2d")
        e(f"trn2 v{t[7]}.2d, v{t[1]}.2d, v{t[3]}.2d")
        for j in range(4):
            off = 32 * slots[j] + 4 * half
            e(f"str q{t[4 + j]}, [x25, #{off}]")
    return L


def emit_kernel(name, k, stride, nwords, prefix):
    """Emit one 1-block (8 + k)-lane kernel: 2 NEON quads + k sequential zero-spill scalars."""
    sym = f"{prefix}{name}"
    L = [f".globl {sym}", ".p2align 4", f"{sym}:"]
    e = L.append
    e(f"sub sp, sp, #{FRAME}")
    for i, r in enumerate(range(19, 31, 2)):
        e(f"stp x{r}, x{r + 1}, [sp, #{F_X19 + 16 * i}]")
    for i, r in enumerate(range(8, 16, 2)):
        e(f"stp d{r}, d{r + 1}, [sp, #{F_D8 + 16 * i}]")

    # Save arguments into x25..x30.
    e("mov x30, x0")  # inputs
    e("mov x26, x1")  # key
    e("mov x27, x2")  # counter
    e("mov w28, w3")  # flags
    e("mov w29, w4")  # block_len
    e("mov x25, x5")  # out

    # Store IV[0..4] to F_IVT and ROT8_TABLE to F_ROT8/v31.
    for i in range(0, 4, 2):
        e(f"mov w6, #{IV[i] & 0xffff}")
        e(f"movk w6, #{IV[i] >> 16}, lsl #16")
        e(f"mov w7, #{IV[i + 1] & 0xffff}")
        e(f"movk w7, #{IV[i + 1] >> 16}, lsl #16")
        e(f"stp w6, w7, [sp, #{F_IVT + 4 * i}]")
    lo = sum(ROT8_TABLE[i] << (8 * i) for i in range(8))
    hi = sum(ROT8_TABLE[8 + i] << (8 * i) for i in range(8))
    for reg, val in (("x6", lo), ("x7", hi)):
        e(f"mov {reg}, #{val & 0xffff}")
        for sh in range(1, 4):
            e(f"movk {reg}, #{(val >> (16 * sh)) & 0xffff}, lsl #{16 * sh}")
    e(f"stp x6, x7, [sp, #{F_ROT8}]")
    e(f"ldr q31, [sp, #{F_ROT8}]")

    u0_slots = [k + j for j in range(4)]
    u1_slots = [k + 4 + j for j in range(4)]

    # Transpose the two NEON quads' message blocks into F_MSG0 / F_MSG1.
    L.extend(neon_transpose_quad(u0_slots, stride, F_MSG0, nwords))
    L.extend(neon_transpose_quad(u1_slots, stride, F_MSG1, nwords))

    # Load key into v0..v7 (u0) and v16..v23 (u1).
    for i in range(8):
        e(f"ldr s28, [x26, #{4 * i}]")
        e(f"dup v{i}.4s, v28.s[0]")
        e(f"dup v{16 + i}.4s, v28.s[0]")

    # Load IV into v8..v11 (u0) and v24..v27 (u1).
    for i in range(4):
        e(f"ldr s28, [sp, #{F_IVT + 4 * i}]")
        e(f"dup v{8 + i}.4s, v28.s[0]")
        e(f"dup v{24 + i}.4s, v28.s[0]")

    # Load d row [ctr_lo, ctr_hi, block_len, flags] into v12..v15 (u0) and F_DSPILL (u1).
    e("lsr x6, x27, #32")
    e("dup v12.4s, w27")
    e("dup v13.4s, w6")
    e("dup v14.4s, w29")
    e("dup v15.4s, w28")
    e(f"stp q12, q13, [sp, #{F_DSPILL}]")
    e(f"stp q14, q15, [sp, #{F_DSPILL + 32}]")

    # Build the two independent instruction streams and interleave them!
    vec_stream = []
    for r in range(7):
        sched = SCHEDULE[r]
        for quads, base in ((COLS, 0), (DIAGS, 8)):
            for (ai, bi, ci, di), idx in zip(quads, range(0, 8, 2)):
                mx, my = sched[base + idx], sched[base + idx + 1]
                vec_stream.extend(neon_quad_g(0, False, ai, bi, ci, di, F_MSG0, mx, my, nwords))
            for (ai, bi, ci, di), idx in zip(quads, range(0, 8, 2)):
                mx, my = sched[base + idx], sched[base + idx + 1]
                vec_stream.extend(neon_quad_g(16, True, ai, bi, ci, di, F_MSG1, mx, my, nwords))

    scalar_stream = []
    for s in range(k):
        scalar_stream.extend(scalar_one(s, stride, nwords))

    L.extend(interleave(scalar_stream, vec_stream))

    # NEON block_end for u0 (v0..v7 ^= v8..v15) and u1 (v16..v23 ^= v24..v27, F_DSPILL).
    for i in range(8):
        e(f"eor v{i}.16b, v{i}.16b, v{8 + i}.16b")
    for i in range(4):
        e(f"eor v{16 + i}.16b, v{16 + i}.16b, v{24 + i}.16b")
    for i in range(4):
        e(f"ldr q29, [sp, #{F_DSPILL + 16 * i}]")
        e(f"eor v{20 + i}.16b, v{20 + i}.16b, v29.16b")

    # Store u0 (using v8..v15 as temps) then u1 (using v0..v7 as temps).
    L.extend(neon_store_quad(0, u0_slots))
    L.extend(neon_store_quad(16, u1_slots))

    for i, r in enumerate(range(8, 16, 2)):
        e(f"ldp d{r}, d{r + 1}, [sp, #{F_D8 + 16 * i}]")
    for i, r in enumerate(range(19, 31, 2)):
        e(f"ldp x{r}, x{r + 1}, [sp, #{F_X19 + 16 * i}]")
    e(f"add sp, sp, #{FRAME}")
    e("ret")
    return "\n".join(L)


KERNELS = [
    ("probe_p10_64", 2, None, 16),
    ("probe_p11_64", 3, None, 16),
    ("probe_p10_packed32", 2, 32, 8),
    ("probe_p11_packed32", 3, 32, 8),
    ("probe_p10_packed48", 2, 48, 12),
    ("probe_p11_packed48", 3, 48, 12),
    ("probe_p11_packed16", 3, 16, 4),
    ("probe_p11_packed64", 3, 64, 16),
]


def main():
    out_dir = Path("examples")
    for suffix, prefix in (("elf", ""), ("apple", "_")):
        parts = [".text"]
        for name, k, stride, nwords in KERNELS:
            parts.append(emit_kernel(name, k, stride, nwords, prefix))
        (out_dir / f"p10_p11_{suffix}.S").write_text("\n\n".join(parts) + "\n")
    print("Generated examples/p10_p11_{elf,apple}.S")


if __name__ == "__main__":
    main()
