#!/usr/bin/env python3
"""Generate exact production-ABI zero-spill sequential-scalar AArch64 NEON
kernels (k10_prod, k11_prod, p10_prod, p11_prod):

    fn(inputs: *const *const u8, blocks: u64, key: *const u32,
       counter: u64, packed_flags: u64, out: *mut u8)

where packed_flags = flags | (flags_start << 8) | (flags_end << 16) | (last_len << 24),
matching c/blake3_neon_hybrid_aarch64.S and src/ffi_neon_hybrid.rs byte-for-byte.
Works for ANY runtime blocks >= 1 (1..=16) and ANY last_len (0..=64) with ZERO
scalar stack spills!
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
F_CTR0 = 256    # unit 0 ctr_lo[4], ctr_hi[4] (32 B)
F_CTR1 = 288    # unit 1 ctr_lo[4], ctr_hi[4] (32 B)
F_MSG0 = 320    # unit 0 transposed msg words (256 B)
F_MSG1 = 576    # unit 1 transposed msg words (256 B)
F_NPTR = 832    # 8 NEON running pointers (64 B)
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


def neon_quad_g(base_reg, spill_d, ai, bi, ci, di, msg_base, mx, my):
    a = f"v{base_reg + ai}"
    b = f"v{base_reg + bi}"
    c = f"v{base_reg + ci}"
    L = []
    e = L.append
    if spill_d:
        doff = F_DSPILL + 16 * (di - 12)
        e(f"ldr q29, [sp, #{doff}]")
        d = "v29"
    else:
        d = f"v{base_reg + di}"
    e(f"ldr q28, [sp, #{msg_base + 16 * mx}]")
    e(f"add {a}.4s, {a}.4s, v28.4s")
    e(f"add {a}.4s, {a}.4s, {b}.4s")
    e(f"eor {d}.16b, {d}.16b, {a}.16b")
    e(f"rev32 {d}.8h, {d}.8h")
    e(f"add {c}.4s, {c}.4s, {d}.4s")
    e(f"eor {b}.16b, {b}.16b, {c}.16b")
    e(f"shl v30.4s, {b}.4s, #20")
    e(f"sri v30.4s, {b}.4s, #12")
    e(f"mov {b}.16b, v30.16b")
    e(f"ldr q28, [sp, #{msg_base + 16 * my}]")
    e(f"add {a}.4s, {a}.4s, v28.4s")
    e(f"add {a}.4s, {a}.4s, {b}.4s")
    e(f"eor {d}.16b, {d}.16b, {a}.16b")
    e(f"tbl {d}.16b, {{{d}.16b}}, v31.16b")
    e(f"add {c}.4s, {c}.4s, {d}.4s")
    if spill_d:
        e(f"str q29, [sp, #{doff}]")
    e(f"eor {b}.16b, {b}.16b, {c}.16b")
    e(f"shl v30.4s, {b}.4s, #25")
    e(f"sri v30.4s, {b}.4s, #7")
    e(f"mov {b}.16b, v30.16b")
    return L


def neon_quad_half_step_steps(base_reg, quads, sched, base, msg_base):
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


def scalar_block_step(label_prefix, step_id, k, per_input_counter):
    """Run 1 zero-spill scalar compression block using runtime blocks (w29)
    and runtime packed_flags (x28). Atomic chunk-switch block at the end."""
    s = SC_REGS
    L = []
    e = L.append
    # Compute runtime block flags in w5 and block_len in w4 from x28 & w2 vs w29/1.
    e("and w5, w28, #0xff")
    e("ubfx w4, w28, #8, #8")
    e("orr w3, w5, w4")
    e("cmp w2, w29")
    e("csel w5, w3, w5, eq")
    e("ubfx w4, w28, #16, #8")
    e("orr w3, w5, w4")
    e("cmp w2, #1")
    e("csel w5, w3, w5, eq")
    e("ubfx w4, w28, #24, #8")
    e("mov w3, #64")
    e("csel w4, w4, w3, eq")
    # Load c row (IV) and d row ([ctr_lo, ctr_hi, block_len, flags]).
    e(f"ldp {s[8]}, {s[9]}, [sp, #{F_IVT}]")
    e(f"ldp {s[10]}, {s[11]}, [sp, #{F_IVT + 8}]")
    if per_input_counter:
        e("add x3, x27, x1")
        e(f"mov {s[12]}, w3")
        e(f"lsr x{s[13][1:]}, x3, #32")
    else:
        e(f"mov {s[12]}, w27")
        e(f"lsr x{s[13][1:]}, x27, #32")
    e(f"mov {s[14]}, w4")
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

    # Atomic chunk-switch block (never split by interleave!).
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
    sw += [
        "mov w2, w29",
        f"{done_lbl}:",
    ]
    e("\n".join(sw))
    return L


def emit_prod_kernel(name, k, per_input_counter, prefix):
    sym = f"{prefix}{name}"
    L = [f".globl {sym}", ".p2align 4", f"{sym}:"]
    e = L.append
    e(f"sub sp, sp, #{FRAME}")
    for i, r in enumerate(range(19, 31, 2)):
        e(f"stp x{r}, x{r + 1}, [sp, #{F_X19 + 16 * i}]")
    for i, r in enumerate(range(8, 16, 2)):
        e(f"stp d{r}, d{r + 1}, [sp, #{F_D8 + 16 * i}]")

    # Exact production ABI: (x0=inputs, x1=blocks, x2=key, x3=counter, x4=packed_flags, x5=out)
    e("mov x30, x0")  # inputs
    e("mov x29, x1")  # blocks (1..16)
    e("mov x26, x2")  # key
    e("mov x27, x3")  # counter
    e("mov x28, x4")  # packed_flags
    e("mov x25, x5")  # out

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

    for j in range(8):
        slot = k + j
        e(f"ldr x4, [x30, #{8 * slot}]")
        e(f"str x4, [sp, #{F_NPTR + 8 * j}]")
        ctr_base = F_CTR0 if j < 4 else F_CTR1
        lane = j % 4
        if per_input_counter:
            e(f"add x4, x27, #{slot}")
        else:
            e("mov x4, x27")
        e(f"str w4, [sp, #{ctr_base + 4 * lane}]")
        e("lsr x4, x4, #32")
        e(f"str w4, [sp, #{ctr_base + 16 + 4 * lane}]")

    for i in range(8):
        e(f"ldr s28, [x26, #{4 * i}]")
        e(f"dup v{i}.4s, v28.s[0]")
        e(f"dup v{16 + i}.4s, v28.s[0]")

    # Init scalar lane 0 with w2 = blocks (w29).
    e("mov x1, #0")
    e("mov w2, w29")
    e("ldr x24, [x30, #0]")
    for i in range(0, 8, 2):
        e(f"ldp {SC_REGS[i]}, {SC_REGS[i + 1]}, [x26, #{4 * i}]")

    loop_lbl = f"L_{name}_loop"
    e("mov x0, x29")
    e(f"{loop_lbl}:")

    for unit_idx, msg_base in ((0, F_MSG0), (1, F_MSG1)):
        for q in range(4):
            for j in range(4):
                p_off = F_NPTR + 8 * (4 * unit_idx + j)
                e(f"ldr x4, [sp, #{p_off}]")
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
            p_off = F_NPTR + 8 * (4 * unit_idx + j)
            e(f"ldr x4, [sp, #{p_off}]")
            e("add x4, x4, #64")
            e(f"str x4, [sp, #{p_off}]")

    # NEON block flags in w5 and block_len in w4 from x28 & x0 vs x29/1.
    e("and w5, w28, #0xff")
    e("ubfx w4, w28, #8, #8")
    e("orr w3, w5, w4")
    e("cmp x0, x29")
    e("csel w5, w3, w5, eq")
    e("ubfx w4, w28, #16, #8")
    e("orr w3, w5, w4")
    e("cmp x0, #1")
    e("csel w5, w3, w5, eq")
    e("ubfx w4, w28, #24, #8")
    e("mov w3, #64")
    e("csel w4, w4, w3, eq")

    for i in range(4):
        e(f"ldr s28, [sp, #{F_IVT + 4 * i}]")
        e(f"dup v{8 + i}.4s, v28.s[0]")
        e(f"mov v{24 + i}.16b, v{8 + i}.16b")
    e(f"ldr q12, [sp, #{F_CTR0}]")
    e(f"ldr q13, [sp, #{F_CTR0 + 16}]")
    e("dup v14.4s, w4")
    e("dup v15.4s, w5")
    e(f"ldr q28, [sp, #{F_CTR1}]")
    e(f"ldr q29, [sp, #{F_CTR1 + 16}]")
    e(f"stp q28, q29, [sp, #{F_DSPILL}]")
    e(f"stp q14, q15, [sp, #{F_DSPILL + 32}]")

    vec_stream = []
    for r in range(7):
        sched = SCHEDULE[r]
        for quads, base in ((COLS, 0), (DIAGS, 8)):
            vec_stream.extend(neon_quad_half_step_steps(0, quads, sched, base, F_MSG0))
            for (ai, bi, ci, di), idx in zip(quads, range(0, 8, 2)):
                mx, my = sched[base + idx], sched[base + idx + 1]
                vec_stream.extend(neon_quad_g(16, True, ai, bi, ci, di, F_MSG1, mx, my))

    scalar_stream = []
    for s_id in range(k):
        scalar_stream.extend(scalar_block_step(name, s_id, k, per_input_counter))

    L.extend(interleave(scalar_stream, vec_stream))

    for i in range(8):
        e(f"eor v{i}.16b, v{i}.16b, v{8 + i}.16b")
    for i in range(4):
        e(f"eor v{16 + i}.16b, v{16 + i}.16b, v{24 + i}.16b")
    for i in range(4):
        e(f"ldr q29, [sp, #{F_DSPILL + 16 * i}]")
        e(f"eor v{20 + i}.16b, v{20 + i}.16b, v29.16b")

    e("subs x0, x0, #1")
    e(f"b.ne {loop_lbl}")

    for base_reg, slots, temps in ((0, [k + j for j in range(4)], list(range(8, 16))),
                                   (16, [k + 4 + j for j in range(4)], list(range(0, 8)))):
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

    for i, r in enumerate(range(8, 16, 2)):
        e(f"ldp d{r}, d{r + 1}, [sp, #{F_D8 + 16 * i}]")
    for i, r in enumerate(range(19, 31, 2)):
        e(f"ldp x{r}, x{r + 1}, [sp, #{F_X19 + 16 * i}]")
    e(f"add sp, sp, #{FRAME}")
    e("ret")
    e(f"{sym}_end:")
    return "\n".join(L)


KERNELS = [
    ("probe_k10_prod", 2, True),
    ("probe_k11_prod", 3, True),
    ("probe_p10_prod", 2, False),
    ("probe_p11_prod", 3, False),
]


def main():
    out_dir = Path("examples")
    for suffix, prefix in (("elf", ""), ("apple", "_")):
        parts = [".text"]
        for name, k, per_ctr in KERNELS:
            parts.append(emit_prod_kernel(name, k, per_ctr, prefix))
        (out_dir / f"k11_prod_{suffix}.S").write_text("\n\n".join(parts) + "\n")
    print("Generated examples/k11_prod_{elf,apple}.S")


if __name__ == "__main__":
    main()
