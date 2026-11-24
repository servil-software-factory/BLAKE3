#!/usr/bin/env python3
"""Generate probe-only zero-spill sequential-scalar k4_seq (2+2), k6_seq (4+2),
and k8_seq (6+2) 1 KiB chunk kernels using AArch64 SHA-3 `xar` Pairs + Quad
(matching k4/k6/k8 in tools/gen_neon_hybrid.py, but with ZERO scalar spills!).
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

F_X19 = 0
F_D8 = 96
F_IVT = 160
F_ROT8 = 176
F_DSPILL = 192
F_CTR0 = 256
F_CTR1 = 288
F_MSG0 = 320
F_MSG1 = 576
F_NPTR = 832
FRAME = 896

SC_REGS = [f"w{i}" for i in list(range(6, 18)) + list(range(19, 23))]


def interleave(a, b):
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


def pair_g(base_reg, spill_d, msg_regs, ai, bi, ci, di, msg_base, mx, my):
    a = f"v{base_reg + ai}"
    b = f"v{base_reg + bi}"
    c = f"v{base_reg + ci}"
    L = []
    e = L.append
    if msg_regs:
        d = f"v{base_reg + di}"
        e(f"add {a}.4s, {a}.4s, v{16 + mx}.4s")
        e(f"add {a}.4s, {a}.4s, {b}.4s")
        e(f"xar {d}.2d, {d}.2d, {a}.2d, #16")
        e(f"add {c}.4s, {c}.4s, {d}.4s")
        e(f"xar {b}.2d, {b}.2d, {c}.2d, #12")
        e(f"add {a}.4s, {a}.4s, v{16 + my}.4s")
        e(f"add {a}.4s, {a}.4s, {b}.4s")
        e(f"xar {d}.2d, {d}.2d, {a}.2d, #8")
        e(f"add {c}.4s, {c}.4s, {d}.4s")
        e(f"xar {b}.2d, {b}.2d, {c}.2d, #7")
    elif spill_d:
        doff = F_DSPILL + 16 * (di - 12)
        e(f"ldr q29, [sp, #{doff}]")
        e(f"ldr q28, [sp, #{msg_base + 16 * mx}]")
        e(f"add {a}.4s, {a}.4s, v28.4s")
        e(f"add {a}.4s, {a}.4s, {b}.4s")
        e(f"xar v29.2d, v29.2d, {a}.2d, #16")
        e(f"add {c}.4s, {c}.4s, v29.4s")
        e(f"xar {b}.2d, {b}.2d, {c}.2d, #12")
        e(f"ldr q28, [sp, #{msg_base + 16 * my}]")
        e(f"add {a}.4s, {a}.4s, v28.4s")
        e(f"add {a}.4s, {a}.4s, {b}.4s")
        e(f"xar v29.2d, v29.2d, {a}.2d, #8")
        e(f"add {c}.4s, {c}.4s, v29.4s")
        e(f"str q29, [sp, #{doff}]")
        e(f"xar {b}.2d, {b}.2d, {c}.2d, #7")
    else:
        d = f"v{base_reg + di}"
        e(f"ldr q28, [sp, #{msg_base + 16 * mx}]")
        e(f"add {a}.4s, {a}.4s, v28.4s")
        e(f"add {a}.4s, {a}.4s, {b}.4s")
        e(f"xar {d}.2d, {d}.2d, {a}.2d, #16")
        e(f"add {c}.4s, {c}.4s, {d}.4s")
        e(f"xar {b}.2d, {b}.2d, {c}.2d, #12")
        e(f"ldr q28, [sp, #{msg_base + 16 * my}]")
        e(f"add {a}.4s, {a}.4s, v28.4s")
        e(f"add {a}.4s, {a}.4s, {b}.4s")
        e(f"xar {d}.2d, {d}.2d, {a}.2d, #8")
        e(f"add {c}.4s, {c}.4s, {d}.4s")
        e(f"xar {b}.2d, {b}.2d, {c}.2d, #7")
    return L


def quad_g_steps(base_reg, quads, sched, base, msg_base):
    cols = [(f"v{base_reg + ai}", f"v{base_reg + bi}", f"v{base_reg + ci}", f"v{base_reg + di}", sched[base + k], sched[base + k + 1])
            for (ai, bi, ci, di), k in zip(quads, range(0, 8, 2))]
    home = [b for _, b, _, _, _, _ in cols]
    at, spare = list(home), "v30"
    out = []
    e = out.append
    for a, _, _, _, mx, _ in cols:
        e(f"ldr q28, [sp, #{msg_base + 16 * mx}]")
        e(f"add {a}.4s, {a}.4s, v28.4s")
    for a, b, _, _, _, _ in cols:
        e(f"add {a}.4s, {a}.4s, {b}.4s")
    for a, _, _, d, _, _ in cols:
        e(f"eor {d}.16b, {d}.16b, {a}.16b")
    for _, _, _, d, _, _ in cols:
        e(f"rev32 {d}.8h, {d}.8h")
    for _, _, c, d, _, _ in cols:
        e(f"add {c}.4s, {c}.4s, {d}.4s")
    for _, b, c, _, _, _ in cols:
        e(f"eor {b}.16b, {b}.16b, {c}.16b")
    for j in range(4):
        e(f"shl {spare}.4s, {at[j]}.4s, #20")
        e(f"sri {spare}.4s, {at[j]}.4s, #12")
        at[j], spare = spare, at[j]
    for a, _, _, _, _, my in cols:
        e(f"ldr q28, [sp, #{msg_base + 16 * my}]")
        e(f"add {a}.4s, {a}.4s, v28.4s")
    for j in range(4):
        e(f"add {cols[j][0]}.4s, {cols[j][0]}.4s, {at[j]}.4s")
    for a, _, _, d, _, _ in cols:
        e(f"eor {d}.16b, {d}.16b, {a}.16b")
    for _, _, _, d, _, _ in cols:
        e(f"tbl {d}.16b, {{{d}.16b}}, v31.16b")
    for _, _, c, d, _, _ in cols:
        e(f"add {c}.4s, {c}.4s, {d}.4s")
    for j in range(4):
        e(f"eor {at[j]}.16b, {at[j]}.16b, {cols[j][2]}.16b")
    for j in reversed(range(4)):
        e(f"shl {spare}.4s, {at[j]}.4s, #25")
        e(f"sri {spare}.4s, {at[j]}.4s, #7")
        at[j], spare = spare, at[j]
    assert at == home and spare == "v30"
    return out


def scalar_block_step(label_prefix, step_id, k, blocks=16):
    s = SC_REGS
    L = []
    e = L.append
    e("mov w5, w28")
    e(f"cmp w2, #{blocks}")
    e("orr w4, w5, #1")
    e("csel w5, w4, w5, eq")
    e("cmp w2, #1")
    e("orr w4, w5, #2")
    e("csel w5, w4, w5, eq")
    e(f"ldp {s[8]}, {s[9]}, [sp, #{F_IVT}]")
    e(f"ldp {s[10]}, {s[11]}, [sp, #{F_IVT + 8}]")
    e("add x4, x27, x1")
    e(f"mov {s[12]}, w4")
    e(f"lsr x{s[13][1:]}, x4, #32")
    e(f"mov {s[14]}, w29")
    e(f"mov {s[15]}, w5")
    for r in range(7):
        sched = SCHEDULE[r]
        for quads, base in ((COLS, 0), (DIAGS, 8)):
            for half, (r1, r2) in ((0, (16, 12)), (1, (8, 7))):
                for (ai, bi, ci, di), idx in zip(quads, range(0, 8, 2)):
                    a, b, c, d = s[ai], s[bi], s[ci], s[di]
                    word = sched[base + idx + half]
                    e(f"ldr w23, [x24, #{4 * word}]")
                    e(f"add {a}, {a}, w23")
                    e(f"add {a}, {a}, {b}")
                    e(f"eor {d}, {d}, {a}")
                    e(f"ror {d}, {d}, #{r1}")
                    e(f"add {c}, {c}, {d}")
                    e(f"eor {b}, {b}, {c}")
                    e(f"ror {b}, {b}, #{r2}")
    for i in range(8):
        e(f"eor {s[i]}, {s[i]}, {s[i + 8]}")
    e("add x24, x24, #64")
    done_lbl = f"L_{label_prefix}_s{step_id}_cont"
    sw = [
        "subs w2, w2, #1",
        f"b.ne {done_lbl}",
        "lsl x4, x1, #5",
        "add x4, x25, x4",
    ]
    for i in range(0, 8, 2):
        sw.append(f"stp {s[i]}, {s[i + 1]}, [x4, #{4 * i}]")
    sw += [
        "add x1, x1, #1",
        f"cmp x1, #{k}",
        f"b.eq {done_lbl}",
        "ldr x24, [x30, x1, lsl #3]",
    ]
    for i in range(0, 8, 2):
        sw.append(f"ldp {s[i]}, {s[i + 1]}, [x26, #{4 * i}]")
    sw += [f"mov w2, #{blocks}", f"{done_lbl}:"]
    e("\n".join(sw))
    return L


def emit_kernel(name, units_spec, k, prefix):
    """units_spec: list of ('pair_reg', slots) | ('pair', slots) | ('quad', slots)."""
    sym = f"{prefix}{name}"
    L = [f".globl {sym}", ".p2align 4", f"{sym}:"]
    e = L.append
    e(f"sub sp, sp, #{FRAME}")
    for i, r in enumerate(range(19, 31, 2)):
        e(f"stp x{r}, x{r + 1}, [sp, #{F_X19 + 16 * i}]")
    for i, r in enumerate(range(8, 16, 2)):
        e(f"stp d{r}, d{r + 1}, [sp, #{F_D8 + 16 * i}]")

    e("mov x30, x0")
    e("mov x26, x1")
    e("mov x27, x2")
    e("mov w28, w3")
    e("mov x25, x4")
    e("mov w29, #64")

    for i in range(0, 4, 2):
        e(f"mov w6, #{IV[i] & 0xffff}")
        e(f"movk w6, #{IV[i] >> 16}, lsl #16")
        e(f"mov w7, #{IV[i + 1] & 0xffff}")
        e(f"movk w7, #{IV[i + 1] >> 16}, lsl #16")
        e(f"stp w6, w7, [sp, #{F_IVT + 4 * i}]")

    needs_rot8 = any(kind == "quad" for kind, _ in units_spec)
    if needs_rot8:
        lo = sum(ROT8_TABLE[i] << (8 * i) for i in range(8))
        hi = sum(ROT8_TABLE[8 + i] << (8 * i) for i in range(8))
        for reg, val in (("x6", lo), ("x7", hi)):
            e(f"mov {reg}, #{val & 0xffff}")
            for sh in range(1, 4):
                e(f"movk {reg}, #{(val >> (16 * sh)) & 0xffff}, lsl #{16 * sh}")
        e(f"stp x6, x7, [sp, #{F_ROT8}]")
        e(f"ldr q31, [sp, #{F_ROT8}]")

    # Initialize NEON running pointers and unit counters.
    p_idx = 0
    unit_ptrs = []
    for u_idx, (kind, slots) in enumerate(units_spec):
        ctr_base = F_CTR0 if u_idx == 0 else F_CTR1
        ptrs = []
        for s in slots:
            e(f"ldr x4, [x30, #{8 * s}]")
            e(f"str x4, [sp, #{F_NPTR + 8 * p_idx}]")
            ptrs.append(F_NPTR + 8 * p_idx)
            p_idx += 1
        unit_ptrs.append(ptrs)
        if kind in ("pair_reg", "pair"):
            sa, sb = slots
            e(f"add x4, x27, #{sa}")
            e(f"add x5, x27, #{sb}")
            e("mov w6, w4")
            e("bfi x6, x6, #32, #32")
            e("mov w7, w5")
            e("bfi x7, x7, #32, #32")
            e(f"stp x6, x7, [sp, #{ctr_base}]")
            e("lsr x6, x4, #32")
            e("bfi x6, x6, #32, #32")
            e("lsr x7, x5, #32")
            e("bfi x7, x7, #32, #32")
            e(f"stp x6, x7, [sp, #{ctr_base + 16}]")
        else:
            for j, s in enumerate(slots):
                e(f"add x4, x27, #{s}")
                e(f"str w4, [sp, #{ctr_base + 4 * j}]")
                e("lsr x4, x4, #32")
                e(f"str w4, [sp, #{ctr_base + 16 + 4 * j}]")

    # Load key into NEON a/b rows.
    for i in range(8):
        e(f"ldr s28, [x26, #{4 * i}]")
        e(f"dup v{i}.4s, v28.s[0]")
        if len(units_spec) == 2:
            e(f"dup v{16 + i}.4s, v28.s[0]")

    # Init scalar 0.
    e("mov x1, #0")
    e("mov w2, #16")
    e("ldr x24, [x30, #0]")
    for i in range(0, 8, 2):
        e(f"ldp {SC_REGS[i]}, {SC_REGS[i + 1]}, [x26, #{4 * i}]")

    loop_lbl = f"L_{name}_loop"
    e("mov x0, #16")
    e(f"{loop_lbl}:")

    # Transpose each unit's next 64-byte block (using v8..v15 as dead temps).
    for u_idx, (kind, slots) in enumerate(units_spec):
        ptrs = unit_ptrs[u_idx]
        msg_base = F_MSG0 if u_idx == 0 else F_MSG1
        if kind == "pair_reg":
            e(f"ldr x4, [sp, #{ptrs[0]}]")
            e(f"ldr x5, [sp, #{ptrs[1]}]")
            for q in range(4):
                e(f"ldr q8, [x4, #{16 * q}]")
                e(f"ldr q9, [x5, #{16 * q}]")
                e("zip1 v10.4s, v8.4s, v9.4s")
                e("zip2 v11.4s, v8.4s, v9.4s")
                e(f"zip1 v{16 + 4 * q}.4s, v10.4s, v10.4s")
                e(f"zip2 v{16 + 4 * q + 1}.4s, v10.4s, v10.4s")
                e(f"zip1 v{16 + 4 * q + 2}.4s, v11.4s, v11.4s")
                e(f"zip2 v{16 + 4 * q + 3}.4s, v11.4s, v11.4s")
            e("add x4, x4, #64")
            e("add x5, x5, #64")
            e(f"str x4, [sp, #{ptrs[0]}]")
            e(f"str x5, [sp, #{ptrs[1]}]")
        elif kind == "pair":
            e(f"ldr x4, [sp, #{ptrs[0]}]")
            e(f"ldr x5, [sp, #{ptrs[1]}]")
            for q in range(4):
                e(f"ldr q8, [x4, #{16 * q}]")
                e(f"ldr q9, [x5, #{16 * q}]")
                e("zip1 v10.4s, v8.4s, v9.4s")
                e("zip2 v11.4s, v8.4s, v9.4s")
                e("zip1 v12.4s, v10.4s, v10.4s")
                e("zip2 v13.4s, v10.4s, v10.4s")
                e("zip1 v14.4s, v11.4s, v11.4s")
                e("zip2 v15.4s, v11.4s, v11.4s")
                e(f"stp q12, q13, [sp, #{msg_base + 64 * q}]")
                e(f"stp q14, q15, [sp, #{msg_base + 64 * q + 32}]")
            e("add x4, x4, #64")
            e("add x5, x5, #64")
            e(f"str x4, [sp, #{ptrs[0]}]")
            e(f"str x5, [sp, #{ptrs[1]}]")
        else:
            for q in range(4):
                for j in range(4):
                    e(f"ldr x4, [sp, #{ptrs[j]}]")
                    e(f"ldr q{8 + j}, [x4, #{16 * q}]")
                e("trn1 v12.4s, v8.4s, v9.4s")
                e("trn2 v13.4s, v8.4s, v9.4s")
                e("trn1 v14.4s, v10.4s, v11.4s")
                e("trn2 v15.4s, v10.4s, v11.4s")
                e("trn1 v8.2d, v12.2d, v14.2d")
                e("trn1 v9.2d, v13.2d, v15.2d")
                e("trn2 v10.2d, v12.2d, v14.2d")
                e("trn2 v11.2d, v13.2d, v15.2d")
                e(f"stp q8, q9, [sp, #{msg_base + 64 * q}]")
                e(f"stp q10, q11, [sp, #{msg_base + 64 * q + 32}]")
            for j in range(4):
                e(f"ldr x4, [sp, #{ptrs[j]}]")
                e("add x4, x4, #64")
                e(f"str x4, [sp, #{ptrs[j]}]")

    # NEON block flags in w5.
    e("mov w5, w28")
    e("cmp x0, #16")
    e("orr w4, w5, #1")
    e("csel w5, w4, w5, eq")
    e("cmp x0, #1")
    e("orr w4, w5, #2")
    e("csel w5, w4, w5, eq")

    # Load NEON c/d rows.
    for i in range(4):
        e(f"ldr s{8 + i}, [sp, #{F_IVT + 4 * i}]")
        e(f"dup v{8 + i}.4s, v{8 + i}.s[0]")
        if len(units_spec) == 2:
            e(f"mov v{24 + i}.16b, v{8 + i}.16b")
    e(f"ldr q12, [sp, #{F_CTR0}]")
    e(f"ldr q13, [sp, #{F_CTR0 + 16}]")
    e("dup v14.4s, w29")
    e("dup v15.4s, w5")
    if len(units_spec) == 2:
        e(f"ldr q28, [sp, #{F_CTR1}]")
        e(f"ldr q29, [sp, #{F_CTR1 + 16}]")
        e(f"stp q28, q29, [sp, #{F_DSPILL}]")
        e(f"stp q14, q15, [sp, #{F_DSPILL + 32}]")

    vec_stream = []
    for r in range(7):
        sched = SCHEDULE[r]
        for quads, base in ((COLS, 0), (DIAGS, 8)):
            for u_idx, (kind, _) in enumerate(units_spec):
                msg_base = F_MSG0 if u_idx == 0 else F_MSG1
                if kind == "quad":
                    vec_stream.extend(quad_g_steps(0, quads, sched, base, msg_base))
                elif kind == "pair_reg":
                    for (ai, bi, ci, di), idx in zip(quads, range(0, 8, 2)):
                        vec_stream.extend(pair_g(0, False, True, ai, bi, ci, di, msg_base, sched[base + idx], sched[base + idx + 1]))
                else:
                    base_r = 0 if u_idx == 0 else 16
                    for (ai, bi, ci, di), idx in zip(quads, range(0, 8, 2)):
                        vec_stream.extend(pair_g(base_r, u_idx == 1, False, ai, bi, ci, di, msg_base, sched[base + idx], sched[base + idx + 1]))

    scalar_stream = []
    for s_id in range(k):
        scalar_stream.extend(scalar_block_step(name, s_id, k, 16))

    L.extend(interleave(scalar_stream, vec_stream))

    for i in range(8):
        e(f"eor v{i}.16b, v{i}.16b, v{8 + i}.16b")
    if len(units_spec) == 2:
        for i in range(4):
            e(f"eor v{16 + i}.16b, v{16 + i}.16b, v{24 + i}.16b")
        for i in range(4):
            e(f"ldr q29, [sp, #{F_DSPILL + 16 * i}]")
            e(f"eor v{20 + i}.16b, v{20 + i}.16b, v29.16b")

    e("subs x0, x0, #1")
    e(f"b.ne {loop_lbl}")

    # Store NEON outputs.
    for u_idx, (kind, slots) in enumerate(units_spec):
        base_r = 0 if u_idx == 0 else 16
        t = list(range(8, 16)) if u_idx == 0 else list(range(0, 8))
        if kind in ("pair_reg", "pair"):
            sa, sb = slots
            for half in (0, 4):
                h = [base_r + half + j for j in range(4)]
                e(f"zip1 v{t[0]}.2d, v{h[0]}.2d, v{h[1]}.2d")
                e(f"zip1 v{t[1]}.2d, v{h[2]}.2d, v{h[3]}.2d")
                e(f"zip2 v{t[2]}.2d, v{h[0]}.2d, v{h[1]}.2d")
                e(f"zip2 v{t[3]}.2d, v{h[2]}.2d, v{h[3]}.2d")
                e(f"uzp1 v{t[0]}.4s, v{t[0]}.4s, v{t[1]}.4s")
                e(f"uzp1 v{t[2]}.4s, v{t[2]}.4s, v{t[3]}.4s")
                e(f"str q{t[0]}, [x25, #{32 * sa + 4 * half}]")
                e(f"str q{t[2]}, [x25, #{32 * sb + 4 * half}]")
        else:
            for half in (0, 4):
                h = [base_r + half + j for j in range(4)]
                e(f"trn1 v{t[0]}.4s, v{h[0]}.4s, v{h[1]}.4s")
                e(f"trn2 v{t[1]}.4s, v{h[0]}.4s, v{h[1]}.4s")
                e(f"trn1 v{t[2]}.4s, v{h[2]}.4s, v{h[3]}.4s")
                e(f"trn2 v{t[3]}.4s, v{h[2]}.4s, v{h[3]}.4s")
                e(f"trn1 v{t[4]}.2d, v{t[0]}.2d, v{t[2]}.2d")
                e(f"trn1 v{t[5]}.2d, v{t[1]}.2d, v{t[3]}.2d")
                e(f"trn2 v{t[6]}.2d, v{t[0]}.2d, v{t[2]}.2d")
                e(f"trn2 v{t[7]}.2d, v{t[1]}.2d, v{t[3]}.2d")
                for j in range(4):
                    e(f"str q{t[4 + j]}, [x25, #{32 * slots[j] + 4 * half}]")

    for i, r in enumerate(range(8, 16, 2)):
        e(f"ldp d{r}, d{r + 1}, [sp, #{F_D8 + 16 * i}]")
    for i, r in enumerate(range(19, 31, 2)):
        e(f"ldp x{r}, x{r + 1}, [sp, #{F_X19 + 16 * i}]")
    e(f"add sp, sp, #{FRAME}")
    e("ret")
    e(f"{sym}_end:")
    return "\n".join(L)


KERNELS = [
    ("probe_k4_seq", [("pair_reg", (2, 3))], 2),
    ("probe_k6_seq", [("pair", (2, 3)), ("pair", (4, 5))], 2),
    ("probe_k8_seq", [("quad", (2, 3, 4, 5)), ("pair", (6, 7))], 2),
]


def main():
    out_dir = Path("examples")
    for suffix, prefix in (("elf", ""), ("apple", "_")):
        parts = [".text"]
        for name, spec, k in KERNELS:
            parts.append(emit_kernel(name, spec, k, prefix))
        (out_dir / f"k468_seq_{suffix}.S").write_text("\n\n".join(parts) + "\n")
    print("Generated examples/k468_seq_{elf,apple}.S")


if __name__ == "__main__":
    main()
