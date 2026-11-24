#!/usr/bin/env python3
"""Generate AArch64 NEON dual-quad (8 lanes/group) per-lane-key independent
counter-zero compression kernel (#336).

AArch64 ABI:
  fn blake3_servil_neon_output8(
      inputs: *const Input,  // x0: 8 * groups repr(C) 24-byte descriptors
      out:    *mut u8,       // x1: 256 * groups output bytes
      groups: u64,           // x2: number of 8-lane groups (0 returns immediately)
  )

Each Input descriptor has stride 24:
  [0..8]  block: *const [u8; 64]
  [8..16] cv:    *const [u32; 8]
  [16]    len:   u8 (0..=64)
  [17]    flags: u8
All 8 lanes execute independently with their own CV, 64-byte block, block
length, and flags (counter = 0). No key sharing or key comparisons.
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

F_D8 = 0        # d8..d15 (64 B)
F_IVT = 64      # IV[0..4] (16 B)
F_ROT8 = 80     # ROT8_TABLE (16 B)
F_DSPILL = 96   # unit 1 spilled d row (64 B)
F_MSG0 = 160    # unit 0 transposed 16 message words (256 B)
F_MSG1 = 416    # unit 1 transposed 16 message words (256 B)
F_SCR = 672     # 32 B scratch for loading lens/flags vectors
FRAME = 704


def transpose_4x4(a, b, c, d, t0, t1, t2, t3):
    """Transpose four 4x32-bit vectors (in a,b,c,d) using t0..t3, leaving result in a,b,c,d."""
    return [
        f"    trn1 v{t0}.4s, v{a}.4s, v{b}.4s",
        f"    trn2 v{t1}.4s, v{a}.4s, v{b}.4s",
        f"    trn1 v{t2}.4s, v{c}.4s, v{d}.4s",
        f"    trn2 v{t3}.4s, v{c}.4s, v{d}.4s",
        f"    trn1 v{a}.2d, v{t0}.2d, v{t2}.2d",
        f"    trn1 v{b}.2d, v{t1}.2d, v{t3}.2d",
        f"    trn2 v{c}.2d, v{t0}.2d, v{t2}.2d",
        f"    trn2 v{d}.2d, v{t1}.2d, v{t3}.2d",
    ]


def emit_load_msg_quad(lane_base, msg_off):
    """Load and transpose 4 x 64 B blocks for lanes [lane_base..lane_base+4] into [sp + msg_off]."""
    L = []
    e = lambda s: L.append(f"    {s}")
    for q in range(4):
        for i in range(4):
            e(f"ldr x8, [x0, #{24 * (lane_base + i)}]")
            e(f"ldr q{i}, [x8, #{16 * q}]")
        L.extend(transpose_4x4(0, 1, 2, 3, 4, 5, 6, 7))
        for j in range(4):
            e(f"str q{j}, [sp, #{msg_off + 16 * (4 * q + j)}]")
    return L


def emit_load_cv_quad(lane_base, dst_base):
    """Load and transpose 4 x 32 B CVs for lanes [lane_base..lane_base+4] into v[dst_base..dst_base+8]."""
    L = []
    e = lambda s: L.append(f"    {s}")
    # Temporaries in v24..v27 (free during setup!).
    for half in (0, 4):
        r = [dst_base + half + i for i in range(4)]
        for i in range(4):
            e(f"ldr x8, [x0, #{24 * (lane_base + i) + 8}]")
            e(f"ldr q{r[i]}, [x8, #{4 * half}]")
        L.extend(transpose_4x4(r[0], r[1], r[2], r[3], 24, 25, 26, 27))
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
    e(f"ldr q28, [sp, #{msg_base + 16 * mx}]")
    e(f"add {a}.4s, {a}.4s, v28.4s")
    e(f"add {a}.4s, {a}.4s, {b}.4s")
    e(f"eor v29.16b, v29.16b, {a}.16b")
    e("rev32 v29.8h, v29.8h")
    e(f"add {c}.4s, {c}.4s, v29.4s")
    e(f"eor {b}.16b, {b}.16b, {c}.16b")
    e(f"shl {t}.4s, {b}.4s, #20")
    e(f"sri {t}.4s, {b}.4s, #12")
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
            e(f"str q{t[4 + j]}, [x1, #{off}]")
    return L


def emit_kernel(name):
    L = [
        f".globl _{name}",
        f".globl {name}",
        ".p2align 4",
        f"_{name}:",
        f"{name}:",
    ]
    e = lambda s: L.append(f"    {s}")

    done_lbl = f"L_{name}_done"
    loop_lbl = f"L_{name}_loop"

    e(f"cbz x2, {done_lbl}")
    e(f"sub sp, sp, #{FRAME}")
    for i, r in enumerate(range(8, 16, 2)):
        e(f"stp d{r}, d{r + 1}, [sp, #{F_D8 + 16 * i}]")

    for i, val in enumerate(IV):
        e(f"mov w6, #{val & 0xffff}")
        e(f"movk w6, #{val >> 16}, lsl #16")
        e(f"str w6, [sp, #{F_IVT + 4 * i}]")
    for i in range(0, 16, 4):
        w = (ROT8_TABLE[i] | (ROT8_TABLE[i + 1] << 8)
             | (ROT8_TABLE[i + 2] << 16) | (ROT8_TABLE[i + 3] << 24))
        e(f"mov w6, #{w & 0xffff}")
        e(f"movk w6, #{w >> 16}, lsl #16")
        e(f"str w6, [sp, #{F_ROT8 + i}]")
    e(f"ldr q31, [sp, #{F_ROT8}]")

    L.append(f"{loop_lbl}:")

    # 1. Load & transpose 8 blocks into F_MSG0 (lanes 0..3) and F_MSG1 (lanes 4..7).
    L.extend(emit_load_msg_quad(0, F_MSG0))
    L.extend(emit_load_msg_quad(4, F_MSG1))

    # 2. Load & transpose 8 CVs into v0..v7 (lanes 0..3) and v16..v23 (lanes 4..7).
    L.extend(emit_load_cv_quad(0, 0))
    L.extend(emit_load_cv_quad(4, 16))

    # 3. Load IV into v8..v11 and v24..v27; build counter=0, lens, flags for both units.
    for i in range(4):
        e(f"ldr w6, [sp, #{F_IVT + 4 * i}]")
        e(f"dup v{8 + i}.4s, w6")
        e(f"mov v{24 + i}.16b, v{8 + i}.16b")

    e("movi v12.4s, #0")
    e("movi v13.4s, #0")
    for i in range(4):
        e(f"ldrb w6, [x0, #{24 * i + 16}]")
        e(f"str w6, [sp, #{F_SCR + 4 * i}]")
        e(f"ldrb w6, [x0, #{24 * i + 17}]")
        e(f"str w6, [sp, #{F_SCR + 16 + 4 * i}]")
    e(f"ldr q14, [sp, #{F_SCR}]")
    e(f"ldr q15, [sp, #{F_SCR + 16}]")

    e(f"stp q12, q13, [sp, #{F_DSPILL}]")
    for i in range(4):
        e(f"ldrb w6, [x0, #{24 * (4 + i) + 16}]")
        e(f"str w6, [sp, #{F_DSPILL + 32 + 4 * i}]")
        e(f"ldrb w6, [x0, #{24 * (4 + i) + 17}]")
        e(f"str w6, [sp, #{F_DSPILL + 48 + 4 * i}]")

    # 4. 7 dual-quad NEON rounds.
    for r in range(7):
        sched = SCHEDULE[r]
        for quads, base in ((COLS, 0), (DIAGS, 8)):
            L.extend(neon_quad_half_step(0, False, quads, sched, base, F_MSG0))
            L.extend(neon_quad_half_step(16, True, quads, sched, base, F_MSG1))

    # 5. Feed-forward & store 8 contiguous 32-byte digests.
    for i in range(8):
        e(f"eor v{i}.16b, v{i}.16b, v{8 + i}.16b")
    L.extend(neon_store_quad(0, [0, 1, 2, 3]))
    for i in range(4):
        e(f"eor v{16 + i}.16b, v{16 + i}.16b, v{24 + i}.16b")
    for i in range(4):
        e(f"ldr q28, [sp, #{F_DSPILL + 16 * i}]")
        e(f"eor v{20 + i}.16b, v{20 + i}.16b, v28.16b")
    L.extend(neon_store_quad(16, [4, 5, 6, 7]))

    e("add x0, x0, #192")
    e("add x1, x1, #256")
    e("subs x2, x2, #1")
    e(f"b.ne {loop_lbl}")

    for i, r in enumerate(range(8, 16, 2)):
        e(f"ldp d{r}, d{r + 1}, [sp, #{F_D8 + 16 * i}]")
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
        "// Generated by tools/gen_output_neon.py; per-lane-key NEON 8-lane contract in its header.",
        ".text",
        emit_kernel("blake3_servil_neon_output8"),
        '#if defined(__ELF__) && defined(__linux__)\n.section .note.GNU-stack,"",@progbits\n#endif',
    ]
    print("\n\n".join(parts))


if __name__ == "__main__":
    main()
