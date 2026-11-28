#!/usr/bin/env python3
"""Generate c/blake3_chain_neon_aarch64.S: 11-lane AArch64 NEON dual-quad +
3 sequential scalar (8+3) fused multi-step 32-byte hash-chain kernel (#584).

Each 32-byte chain slot holds `[prefix12 || start_step_u32_le || state16]`.
For each step `s in 0..steps`:

    state_{s+1} = BLAKE3_compress(key, [prefix12, start_step + s, state_s],
                                  len=32, counter=0, flags)[..16]

All 11 chains stay in L1 stack slots across all `steps` iterations with zero
caller RAM traffic, zero zero-word (8..15) adds, and only 16 bytes written per
chain at the end.

AArch64 ABI:
  fn(
      init: *const u8,   // x0: 11 * groups * 32 bytes
      groups: u64,       // x1: number of 11-chain groups (0 returns early)
      key: *const u32,   // x2: 8 x u32 key words
      flags: u32,        // w3: root block flags
      out: *mut u8,      // x4: 11 * groups * 16 bytes
      steps: u32,        // w5: number of chain iterations (0 returns early)
  )

x18 untouched; d8-d15 and x19-x30 preserved.

    python3 tools/gen_chain_neon.py > c/blake3_chain_neon_aarch64.S
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

# Stack frame (640 B, 16-byte aligned; F_SC and F_IVT sit below 252 for `stp w`).
F_SC = 0         # 3 scalar chain 32-byte buffers (96 B: slots 0, 1, 2)
F_IVT = 96       # IV[0..4] (16 B)
F_ROT8 = 112     # ROT8_TABLE (16 B)
F_ONE = 128      # [1u32; 4] (16 B)
F_X19 = 144      # x19..x30 (96 B)
F_D8 = 240       # d8..d15 (64 B)
F_DSPILL = 304   # unit 1 spilled d row (64 B)
F_MSG0 = 368     # unit 0 transposed 8 x 16 B msg words (128 B)
F_MSG1 = 496     # unit 1 transposed 8 x 16 B msg words (128 B)
F_GROUPS = 624   # remaining groups (8 B)
F_STEPS = 632    # steps per group (4 B)
FRAME = 640

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


def scalar_one_step(slot):
    """One 7-round zero-word-pruned step for scalar slot in {0, 1, 2}, reading
    from `[sp, #F_SC + 32*slot]`, updating 16-byte state at `+16..+32`, and
    incrementing `step` at `+12`."""
    s = SC_REGS
    base_off = F_SC + 32 * slot
    L = []
    e = lambda inst: L.append(f"    {inst}")
    for i in range(0, 8, 2):
        e(f"ldp {s[i]}, {s[i + 1]}, [x26, #{4 * i}]")
    e(f"ldp {s[8]}, {s[9]}, [sp, #{F_IVT}]")
    e(f"ldp {s[10]}, {s[11]}, [sp, #{F_IVT + 8}]")
    e(f"mov {s[12]}, wzr")
    e(f"mov {s[13]}, wzr")
    e(f"mov {s[14]}, #32")
    e(f"mov {s[15]}, w28")

    for r in range(7):
        sched = SCHEDULE[r]
        for quads, base in ((COLS, 0), (DIAGS, 8)):
            for half, (r1, r2) in ((0, (16, 12)), (1, (8, 7))):
                for (ai, bi, ci, di), k in zip(quads, range(0, 8, 2)):
                    a, b, c, d = s[ai], s[bi], s[ci], s[di]
                    word = sched[base + k + half]
                    if word < 8:
                        e(f"ldr w23, [sp, #{base_off + 4 * word}]")
                        e(f"add {a}, {a}, w23")
                    e(f"add {a}, {a}, {b}")
                    e(f"eor {d}, {d}, {a}")
                    e(f"ror {d}, {d}, #{r1}")
                    e(f"add {c}, {c}, {d}")
                    e(f"eor {b}, {b}, {c}")
                    e(f"ror {b}, {b}, #{r2}")

    for i in range(4):
        e(f"eor {s[i]}, {s[i]}, {s[i + 8]}")
    e(f"stp {s[0]}, {s[1]}, [sp, #{base_off + 16}]")
    e(f"stp {s[2]}, {s[3]}, [sp, #{base_off + 24}]")
    e(f"ldr w23, [sp, #{base_off + 12}]")
    e("add w23, w23, #1")
    e(f"str w23, [sp, #{base_off + 12}]")
    return L


def neon_transpose_quad(slots, msg_base):
    L = []
    e = lambda inst: L.append(f"    {inst}")
    for q in range(2):
        for j in range(4):
            off = 32 * slots[j] + 16 * q
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


def neon_quad_g(base_reg, ai, bi, ci, di, msg_base, mx, my):
    a = f"v{base_reg + ai}"
    b = f"v{base_reg + bi}"
    c = f"v{base_reg + ci}"
    t = "v30"
    L = []
    e = lambda inst: L.append(f"    {inst}")
    doff = F_DSPILL + 16 * (di - 12)
    e(f"ldr q29, [sp, #{doff}]")
    if mx < 8:
        e(f"ldr q28, [sp, #{msg_base + 16 * mx}]")
        e(f"add {a}.4s, {a}.4s, v28.4s")
    e(f"add {a}.4s, {a}.4s, {b}.4s")
    e(f"eor v29.16b, v29.16b, {a}.16b")
    e("rev32 v29.8h, v29.8h")
    e(f"add {c}.4s, {c}.4s, v29.4s")
    e(f"eor {b}.16b, {b}.16b, {c}.16b")
    e(f"shl {t}.4s, {b}.4s, #20")
    e(f"sri {t}.4s, {b}.4s, #12")
    if my < 8:
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


def neon_quad_half_step(base_reg, spill_d, quads, sched, base, msg_base):
    if spill_d:
        out = []
        for (ai, bi, ci, di), k in zip(quads, range(0, 8, 2)):
            out.extend(neon_quad_g(base_reg, ai, bi, ci, di, msg_base, sched[base + k], sched[base + k + 1]))
        return out
    cols = [(f"v{base_reg + ai}", f"v{base_reg + bi}", f"v{base_reg + ci}", f"v{base_reg + di}", sched[base + k], sched[base + k + 1])
            for (ai, bi, ci, di), k in zip(quads, range(0, 8, 2))]
    home = [b for _, b, _, _, _, _ in cols]
    at, spare = list(home), "v30"
    out = []
    e = lambda inst: out.append(f"    {inst}")
    for a, _, _, _, mx, _ in cols:
        if mx < 8:
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
        if my < 8:
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


def neon_store_quad16(msg_base, slots):
    """Load the 4 transposed final state vectors from `[sp, #msg_base + 64]`,
    transpose 4x4, and store 16 bytes per lane to `[x25, #16 * slot]`."""
    L = []
    e = lambda inst: L.append(f"    {inst}")
    e(f"ldp q0, q1, [sp, #{msg_base + 64}]")
    e(f"ldp q2, q3, [sp, #{msg_base + 96}]")
    e("trn1 v4.4s, v0.4s, v1.4s")
    e("trn2 v5.4s, v0.4s, v1.4s")
    e("trn1 v6.4s, v2.4s, v3.4s")
    e("trn2 v7.4s, v2.4s, v3.4s")
    e("trn1 v8.2d, v4.2d, v6.2d")
    e("trn1 v9.2d, v5.2d, v7.2d")
    e("trn2 v10.2d, v4.2d, v6.2d")
    e("trn2 v11.2d, v5.2d, v7.2d")
    for j in range(4):
        e(f"str q{8 + j}, [x25, #{16 * slots[j]}]")
    return L


def emit_kernel(name):
    L = [
        f".globl _{name}",
        f".globl {name}",
        ".p2align 4",
        f"_{name}:",
        f"{name}:",
    ]
    e = lambda inst: L.append(f"    {inst}")

    done_lbl = f"L_{name}_done"
    group_lbl = f"L_{name}_group"
    step_lbl = f"L_{name}_step"

    e(f"cbz x1, {done_lbl}")
    e(f"cbz w5, {done_lbl}")

    e(f"sub sp, sp, #{FRAME}")
    for i, r in enumerate(range(19, 31, 2)):
        e(f"stp x{r}, x{r + 1}, [sp, #{F_X19 + 16 * i}]")
    # Keep x29 pointing at the saved (FP, LR) frame record (Apple arm64 ABI).
    e(f"add x29, sp, #{F_X19 + 16 * 5}")
    for i, r in enumerate(range(8, 16, 2)):
        e(f"stp d{r}, d{r + 1}, [sp, #{F_D8 + 16 * i}]")

    # x30 = init, [sp + F_GROUPS] = groups, x26 = key, w28 = flags, x25 = out, [sp + F_STEPS] = steps
    e("mov x30, x0")
    e(f"str x1, [sp, #{F_GROUPS}]")
    e("mov x26, x2")
    e("mov w28, w3")
    e("mov x25, x4")
    e(f"str w5, [sp, #{F_STEPS}]")

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
    e("mov w6, #1")
    e("dup v0.4s, w6")
    e(f"str q0, [sp, #{F_ONE}]")

    u0_slots = [3, 4, 5, 6]
    u1_slots = [7, 8, 9, 10]

    L.append(f"{group_lbl}:")

    # Copy scalar slots 0, 1, 2 (96 B) into [sp, #F_SC].
    for slot in range(3):
        e(f"ldp x6, x7, [x30, #{32 * slot}]")
        e(f"stp x6, x7, [sp, #{F_SC + 32 * slot}]")
        e(f"ldp x6, x7, [x30, #{32 * slot + 16}]")
        e(f"stp x6, x7, [sp, #{F_SC + 32 * slot + 16}]")

    # Transpose NEON units 0 & 1 once per group.
    L.extend(neon_transpose_quad(u0_slots, F_MSG0))
    L.extend(neon_transpose_quad(u1_slots, F_MSG1))

    # w27 = remaining steps (untouched by scalar_one_step, which uses w6..w17, w19..w23).
    e(f"ldr w27, [sp, #{F_STEPS}]")

    L.append(f"{step_lbl}:")

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
    e("mov w6, #32")
    e("dup v14.4s, w6")
    e("dup v15.4s, w28")
    e(f"stp q12, q13, [sp, #{F_DSPILL}]")
    e(f"stp q14, q15, [sp, #{F_DSPILL + 32}]")

    neon_stream = []
    for r in range(7):
        sched = SCHEDULE[r]
        for quads, base in ((COLS, 0), (DIAGS, 8)):
            neon_stream.extend(neon_quad_half_step(0, False, quads, sched, base, F_MSG0))
            neon_stream.extend(neon_quad_half_step(16, True, quads, sched, base, F_MSG1))

    scalar_stream = []
    for s in range(3):
        scalar_stream.extend(scalar_one_step(s))

    L.extend(interleave(neon_stream, scalar_stream))

    # Update unit 0 state[0..3] in [sp, #F_MSG0 + 64..128] and step += 1 at +48.
    for i in range(4):
        e(f"eor v{i}.16b, v{i}.16b, v{8 + i}.16b")
    e(f"stp q0, q1, [sp, #{F_MSG0 + 64}]")
    e(f"stp q2, q3, [sp, #{F_MSG0 + 96}]")
    e(f"ldr q28, [sp, #{F_MSG0 + 48}]")
    e(f"ldr q29, [sp, #{F_ONE}]")
    e("add v28.4s, v28.4s, v29.4s")
    e(f"str q28, [sp, #{F_MSG0 + 48}]")

    # Update unit 1 state[0..3] in [sp, #F_MSG1 + 64..128] and step += 1 at +48.
    for i in range(4):
        e(f"eor v{16 + i}.16b, v{16 + i}.16b, v{24 + i}.16b")
    e(f"stp q16, q17, [sp, #{F_MSG1 + 64}]")
    e(f"stp q18, q19, [sp, #{F_MSG1 + 96}]")
    e(f"ldr q28, [sp, #{F_MSG1 + 48}]")
    e("add v28.4s, v28.4s, v29.4s")
    e(f"str q28, [sp, #{F_MSG1 + 48}]")

    e("subs w27, w27, #1")
    e(f"b.ne {step_lbl}")

    # Store 16-byte final states for scalar slots 0..2 and NEON slots 3..10 (176 B).
    for slot in range(3):
        e(f"ldp x6, x7, [sp, #{F_SC + 32 * slot + 16}]")
        e(f"stp x6, x7, [x25, #{16 * slot}]")
    L.extend(neon_store_quad16(F_MSG0, u0_slots))
    L.extend(neon_store_quad16(F_MSG1, u1_slots))

    e("add x30, x30, #352")
    e("add x25, x25, #176")
    e(f"ldr x6, [sp, #{F_GROUPS}]")
    e("subs x6, x6, #1")
    e(f"str x6, [sp, #{F_GROUPS}]")
    e(f"b.ne {group_lbl}")

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
        "// Generated by tools/gen_chain_neon.py; 11-lane NEON 8+3 fused hash-chain contract in its header.",
        ".text",
        emit_kernel("blake3_servil_neon_p11_chain32_trunc16"),
        '#if defined(__ELF__) && defined(__linux__)\n.section .note.GNU-stack,"",@progbits\n#endif',
    ]
    print("\n\n".join(parts))


if __name__ == "__main__":
    main()
