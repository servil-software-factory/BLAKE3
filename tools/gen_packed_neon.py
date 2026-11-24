#!/usr/bin/env python3
"""Generate direct-load AArch64 NEON dual-quad + 3 zero-spill sequential scalar
(8+3 = 11 lanes/group) kernels for Packed<16>, Packed<32>, and Packed<48> (#501).

AArch64 ABI:
  fn(input: *const u8, groups: u64, key: *const u32, flags: u32, out: *mut u8)
     x0                x1           x2               w3          x4
Hashes `11 * groups` contiguous packed messages in one call with zero 8 KiB
stack expansion, zero pointer table, zero scalar stack spills, and all
zero-word message loads/adds omitted at generation time.
"""
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
F_MSG0 = 256    # unit 0 transposed msg words (up to 12*16 = 192 B; keep 256 B stride)
F_MSG1 = 512    # unit 1 transposed msg words (256 B)
F_GROUPS = 768  # remaining groups counter (u64)
FRAME = 784

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


def scalar_one(slot, nbytes, nwords):
    s = SC_REGS
    L = []
    e = lambda inst: L.append(f"    {inst}")
    if slot == 0:
        e("mov x24, x30")
    else:
        e(f"add x24, x30, #{nbytes * slot}")
    for i in range(0, 8, 2):
        e(f"ldp {s[i]}, {s[i + 1]}, [x26, #{4 * i}]")
    e(f"ldp {s[8]}, {s[9]}, [sp, #{F_IVT}]")
    e(f"ldp {s[10]}, {s[11]}, [sp, #{F_IVT + 8}]")
    e(f"mov {s[12]}, wzr")
    e(f"mov {s[13]}, wzr")
    e(f"mov {s[14]}, #{nbytes}")
    e(f"mov {s[15]}, w28")

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

    for i in range(8):
        e(f"eor {s[i]}, {s[i]}, {s[i + 8]}")
    off = 32 * slot
    for i in range(0, 8, 2):
        e(f"stp {s[i]}, {s[i + 1]}, [x25, #{off + 4 * i}]")
    return L


def neon_transpose_quad(slots, nbytes, msg_base, nwords):
    L = []
    e = lambda inst: L.append(f"    {inst}")
    for q in range(nwords // 4):
        for j in range(4):
            off = nbytes * slots[j] + 16 * q
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


def neon_quad_g(base_reg, ai, bi, ci, di, msg_base, mx, my, nwords):
    a = f"v{base_reg + ai}"
    b = f"v{base_reg + bi}"
    c = f"v{base_reg + ci}"
    t = "v30"
    L = []
    e = lambda inst: L.append(f"    {inst}")
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
    return L


def neon_quad_half_step(base_reg, spill_d, quads, sched, base, msg_base, nwords):
    if spill_d:
        out = []
        for (ai, bi, ci, di), k in zip(quads, range(0, 8, 2)):
            out.extend(neon_quad_g(base_reg, ai, bi, ci, di, msg_base, sched[base + k], sched[base + k + 1], nwords))
        return out
    cols = [(f"v{base_reg + ai}", f"v{base_reg + bi}", f"v{base_reg + ci}", f"v{base_reg + di}", sched[base + k], sched[base + k + 1])
            for (ai, bi, ci, di), k in zip(quads, range(0, 8, 2))]
    home = [b for _, b, _, _, _, _ in cols]
    at, spare = list(home), "v30"
    out = []
    e = lambda inst: out.append(f"    {inst}")
    for a, _, _, _, mx, _ in cols:
        if mx < nwords:
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
        if my < nwords:
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


def neon_store_quad(base_reg, slots):
    L = []
    e = lambda inst: L.append(f"    {inst}")
    t = list(range(8, 16)) if base_reg == 0 else list(range(0, 8))
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


def emit_kernel(name, nbytes):
    nwords = nbytes // 4
    L = [
        f".globl _{name}",
        f".globl {name}",
        ".p2align 4",
        f"_{name}:",
        f"{name}:",
    ]
    e = lambda inst: L.append(f"    {inst}")

    done_lbl = f"L_{name}_done"
    loop_lbl = f"L_{name}_loop"

    e(f"cbz x1, {done_lbl}")

    e(f"sub sp, sp, #{FRAME}")
    for i, r in enumerate(range(19, 31, 2)):
        e(f"stp x{r}, x{r + 1}, [sp, #{F_X19 + 16 * i}]")
    for i, r in enumerate(range(8, 16, 2)):
        e(f"stp d{r}, d{r + 1}, [sp, #{F_D8 + 16 * i}]")

    # x30 = input, x26 = key, w28 = flags, x25 = out, [sp + F_GROUPS] = groups
    e("mov x30, x0")
    e(f"str x1, [sp, #{F_GROUPS}]")
    e("mov x26, x2")
    e("mov w28, w3")
    e("mov x25, x4")

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

    # Slots 0, 1, 2 -> 3 sequential scalars (keeps stp offsets <= 96 < 252!);
    # Slots 3..6 -> NEON unit 0; Slots 7..10 -> NEON unit 1.
    u0_slots = [3, 4, 5, 6]
    u1_slots = [7, 8, 9, 10]

    L.append(f"{loop_lbl}:")

    L.extend(neon_transpose_quad(u0_slots, nbytes, F_MSG0, nwords))
    L.extend(neon_transpose_quad(u1_slots, nbytes, F_MSG1, nwords))

    for i in range(8):
        e(f"ldr w6, [x26, #{4 * i}]")
        e(f"dup v{i}.4s, w6")
        e(f"mov v{16 + i}.16b, v{i}.16b")
    for i in range(4):
        e(f"ldr w6, [sp, #{F_IVT + 4 * i}]")
        e(f"dup v{8 + i}.4s, w6")
        e(f"mov v{24 + i}.16b, v{8 + i}.16b")
    e("movi v12.4s, #0")
    e("movi v13.4s, #0")
    e(f"mov w6, #{nbytes}")
    e("dup v14.4s, w6")
    e("dup v15.4s, w28")
    e(f"stp q12, q13, [sp, #{F_DSPILL}]")
    e(f"stp q14, q15, [sp, #{F_DSPILL + 32}]")

    neon_stream = []
    for r in range(7):
        sched = SCHEDULE[r]
        for quads, base in ((COLS, 0), (DIAGS, 8)):
            neon_stream.extend(neon_quad_half_step(0, False, quads, sched, base, F_MSG0, nwords))
            neon_stream.extend(neon_quad_half_step(16, True, quads, sched, base, F_MSG1, nwords))

    scalar_stream = []
    for s in range(3):
        scalar_stream.extend(scalar_one(s, nbytes, nwords))

    L.extend(interleave(neon_stream, scalar_stream))

    for i in range(8):
        e(f"eor v{i}.16b, v{i}.16b, v{8 + i}.16b")
    L.extend(neon_store_quad(0, u0_slots))
    for i in range(4):
        e(f"eor v{16 + i}.16b, v{16 + i}.16b, v{24 + i}.16b")
    for i in range(4):
        e(f"ldr q28, [sp, #{F_DSPILL + 16 * i}]")
        e(f"eor v{20 + i}.16b, v{20 + i}.16b, v28.16b")
    L.extend(neon_store_quad(16, u1_slots))

    e(f"add x30, x30, #{11 * nbytes}")
    e("add x25, x25, #352")
    e(f"ldr x6, [sp, #{F_GROUPS}]")
    e("subs x6, x6, #1")
    e(f"str x6, [sp, #{F_GROUPS}]")
    e(f"b.ne {loop_lbl}")

    for i, r in enumerate(range(8, 16, 2)):
        e(f"ldp d{r}, d{r + 1}, [sp, #{F_D8 + 16 * i}]")
    for i, r in enumerate(range(19, 31, 2)):
        e(f"ldp x{r}, x{r + 1}, [sp, #{F_X19 + 16 * i}]")
    e(f"add sp, sp, #{FRAME}")
    L.append(f"{done_lbl}:")
    e("ret")
    L.append(f".globl _{name}_end")
    L.append(f".globl {name}_end")
    L.append(f"_{name}_end:")
    L.append(f"{name}_end:")
    return "\n".join(L)


def main():
    parts = [
        "// Generated by tools/gen_packed_neon.py; direct-load NEON 8+3 Packed<16/32/48> contract in its header.",
        ".text",
    ]
    for n in (16, 32, 48):
        parts.append(emit_kernel(f"blake3_servil_neon_p11_packed{n}", n))
    parts.append('#if defined(__ELF__) && defined(__linux__)\n.section .note.GNU-stack,"",@progbits\n#endif')
    print("\n\n".join(parts))


if __name__ == "__main__":
    main()
