#!/usr/bin/env python3
"""Generate zero-word-pruned AArch64 NEON dual-quad + 3 zero-spill scalar
(11 lanes/group) counter-range (`32 B` / `16 B`) and early-exit `find` kernels
(#511, #585).

AArch64 ABI (`counter`):
  fn(
      template: *const u32,  // x0: 16 x u32 zero-padded 64-byte template
      key: *const u32,       // x1: 8 x u32 key words
      base_and_word: u64,    // x2: (base as u32) | ((counter_word as u64) << 32)
      flags_and_len: u64,    // x3: flags_u8 | (block_len_u8 << 8) | (trunc16_bit << 16)
      out: *mut u8,          // x4: (352 or 176) * groups bytes
      groups: u64,           // x5: number of 11-lane groups (0 returns immediately)
  )

AArch64 ABI (`find`):
  fn(
      template: *const u32,  // x0: 16 x u32 zero-padded 64-byte template
      key: *const u32,       // x1: 8 x u32 key words
      base_and_word: u64,    // x2: (base as u32) | ((counter_word as u64) << 32)
      flags_and_len: u64,    // x3: flags_u8 | (block_len_u8 << 8)
      mask_and_target: u64,  // x4: (mask as u32) | ((target as u64) << 32)
      groups: u64,           // x5: max 11-lane groups (0 returns 0)
  ) -> u64                   // (groups_done << 11) | hit_mask_11
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

F_SCOUT = 0     # 96 B: counter = 3x32 B scalar digests; find = 11x4 B diff words (8 NEON + 3 scalar)
F_IVT = 96      # IV[0..4] (16 B; ldp w offset <= 252)
F_ROT8 = 112    # ROT8_TABLE (16 B)
F_X19 = 128     # x19..x30 (96 B; x29 at 208)
F_D8 = 224      # d8..d15 (64 B)
F_DSPILL = 288  # unit 1 spilled d row (64 B)
F_MSG0 = 352    # unit 0 broadcast template words (256 B)
F_MSG1 = 608    # unit 1 broadcast template words (256 B)
F_TMPL = 864    # scalar 16-word template copy (64 B)
F_GROUPS = 928  # u64 remaining groups (counter) or max_groups (find)
F_BASE = 936    # u32 current base
F_BLEN = 940    # u32 block_len (kept off x29 to preserve Apple arm64 FP ABI)
F_CW = 944      # u64 counter_word (0..n_words-1)
F_AUX = 952     # u64 trunc16 flag (counter) or groups_done (find)
F_OFF0 = 960    # [0, 1, 2, 3] (16 B, 16-byte aligned)
F_OFF1 = 976    # [4, 5, 6, 7] (16 B, 16-byte aligned)
FRAME = 992

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


def scalar_one(s_idx, n_words, mode):
    """One complete zero-word-pruned compression on scalar registers
    (w6..w17, w19..w22; temp w23; x24 = sp+F_TMPL; x30 = 4*cw; w27 = counter)."""
    s = SC_REGS
    L = []
    e = lambda inst: L.append(f"    {inst}")
    e("str w27, [x24, x30]")
    e("add w27, w27, #1")

    for i in range(0, 8, 2):
        e(f"ldp {s[i]}, {s[i + 1]}, [x26, #{4 * i}]")
    e(f"ldp {s[8]}, {s[9]}, [sp, #{F_IVT}]")
    e(f"ldp {s[10]}, {s[11]}, [sp, #{F_IVT + 8}]")
    e(f"mov {s[12]}, wzr")
    e(f"mov {s[13]}, wzr")
    e(f"ldr {s[14]}, [sp, #{F_BLEN}]")
    e(f"mov {s[15]}, w28")

    for r in range(7):
        sched = SCHEDULE[r]
        for quads, base in ((COLS, 0), (DIAGS, 8)):
            for half, (r1, r2) in ((0, (16, 12)), (1, (8, 7))):
                for (ai, bi, ci, di), k in zip(quads, range(0, 8, 2)):
                    a, b, c, d = s[ai], s[bi], s[ci], s[di]
                    word = sched[base + k + half]
                    if word < n_words:
                        e(f"ldr w23, [x24, #{4 * word}]")
                        e(f"add {a}, {a}, w23")
                    e(f"add {a}, {a}, {b}")
                    e(f"eor {d}, {d}, {a}")
                    e(f"ror {d}, {d}, #{r1}")
                    e(f"add {c}, {c}, {d}")
                    e(f"eor {b}, {b}, {c}")
                    e(f"ror {b}, {b}, #{r2}")

    if mode == "counter":
        for i in range(8):
            e(f"eor {s[i]}, {s[i]}, {s[i + 8]}")
        off = F_SCOUT + 32 * s_idx
        for i in range(0, 8, 2):
            e(f"stp {s[i]}, {s[i + 1]}, [sp, #{off + 4 * i}]")
    else:
        # find mode: compute ((word0 & mask) ^ target) and write 4 bytes to [sp, #F_SCOUT + 32 + 4*s_idx].
        e(f"eor {s[0]}, {s[0]}, {s[8]}")
        e(f"and {s[0]}, {s[0]}, w25")
        e("lsr x23, x25, #32")
        e(f"eor {s[0]}, {s[0]}, w23")
        e(f"str {s[0]}, [sp, #{F_SCOUT + 32 + 4 * s_idx}]")
    return L


def neon_quad_g(base_reg, ai, bi, ci, di, msg_base, mx, my, n_words):
    a = f"v{base_reg + ai}"
    b = f"v{base_reg + bi}"
    c = f"v{base_reg + ci}"
    t = "v30"
    L = []
    e = lambda inst: L.append(f"    {inst}")
    doff = F_DSPILL + 16 * (di - 12)
    e(f"ldr q29, [sp, #{doff}]")
    if mx < n_words:
        e(f"ldr q28, [sp, #{msg_base + 16 * mx}]")
        e(f"add {a}.4s, {a}.4s, v28.4s")
    e(f"add {a}.4s, {a}.4s, {b}.4s")
    e(f"eor v29.16b, v29.16b, {a}.16b")
    e("rev32 v29.8h, v29.8h")
    e(f"add {c}.4s, {c}.4s, v29.4s")
    e(f"eor {b}.16b, {b}.16b, {c}.16b")
    e(f"shl {t}.4s, {b}.4s, #20")
    e(f"sri {t}.4s, {b}.4s, #12")
    if my < n_words:
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


def neon_quad_half_step(base_reg, spill_d, quads, sched, base, msg_base, n_words):
    if spill_d:
        out = []
        for (ai, bi, ci, di), k in zip(quads, range(0, 8, 2)):
            out.extend(neon_quad_g(base_reg, ai, bi, ci, di, msg_base, sched[base + k], sched[base + k + 1], n_words))
        return out
    cols = [(f"v{base_reg + ai}", f"v{base_reg + bi}", f"v{base_reg + ci}", f"v{base_reg + di}", sched[base + k], sched[base + k + 1])
            for (ai, bi, ci, di), k in zip(quads, range(0, 8, 2))]
    home = [b for _, b, _, _, _, _ in cols]
    at, spare = list(home), "v30"
    out = []
    e = lambda inst: out.append(f"    {inst}")
    for a, _, _, _, mx, _ in cols:
        if mx < n_words:
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
        if my < n_words:
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


def neon_store_quad(base_reg, slots, stride):
    L = []
    e = lambda inst: L.append(f"    {inst}")
    t = list(range(8, 16)) if base_reg == 0 else list(range(0, 8))
    halves = (0, 4) if stride == 32 else (0,)
    for half in halves:
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
            off = stride * slots[j] + 4 * half
            e(f"str q{t[4 + j]}, [x25, #{off}]")
    return L


def emit_kernel(name, n_words, mode):
    assert mode in ("counter", "find")
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

    if mode == "find":
        e(f"cbz x5, L_{name}_zero")
    else:
        e(f"cbz x5, {done_lbl}")

    e(f"sub sp, sp, #{FRAME}")
    for i, r in enumerate(range(19, 31, 2)):
        e(f"stp x{r}, x{r + 1}, [sp, #{F_X19 + 16 * i}]")
    # Keep x29 pointing at the saved (FP, LR) frame record (Apple arm64 ABI).
    e(f"add x29, sp, #{F_X19 + 16 * 5}")
    for i, r in enumerate(range(8, 16, 2)):
        e(f"stp d{r}, d{r + 1}, [sp, #{F_D8 + 16 * i}]")

    e("mov x26, x1")
    e("mov x25, x4")
    e("and w28, w3, #0xff")
    e("lsr w6, w3, #8")
    e("and w6, w6, #0xff")
    e(f"str w6, [sp, #{F_BLEN}]")
    if mode == "counter":
        e("lsr x6, x3, #16")
        e("and x6, x6, #1")
        e(f"str x6, [sp, #{F_AUX}]")
    else:
        e(f"str xzr, [sp, #{F_AUX}]")
    e(f"str x5, [sp, #{F_GROUPS}]")
    e(f"str w2, [sp, #{F_BASE}]")
    e("lsr x6, x2, #32")
    e(f"str x6, [sp, #{F_CW}]")
    e("lsl x30, x6, #2")
    e(f"add x24, sp, #{F_TMPL}")

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

    for i in range(4):
        e(f"mov w6, #{i}")
        e(f"str w6, [sp, #{F_OFF0 + 4 * i}]")
        e(f"mov w6, #{4 + i}")
        e(f"str w6, [sp, #{F_OFF1 + 4 * i}]")

    for w in range(n_words):
        e(f"ldr w6, [x0, #{4 * w}]")
        e(f"str w6, [sp, #{F_TMPL + 4 * w}]")
        e("dup v0.4s, w6")
        e(f"str q0, [sp, #{F_MSG0 + 16 * w}]")
        e(f"str q0, [sp, #{F_MSG1 + 16 * w}]")

    L.append(f"{loop_lbl}:")

    e(f"ldr w6, [sp, #{F_BASE}]")
    e(f"ldr x7, [sp, #{F_CW}]")
    e("lsl x8, x7, #4")
    e("dup v0.4s, w6")
    e(f"ldr q1, [sp, #{F_OFF0}]")
    e(f"ldr q2, [sp, #{F_OFF1}]")
    e("add v1.4s, v0.4s, v1.4s")
    e("add v2.4s, v0.4s, v2.4s")
    e(f"add x9, sp, #{F_MSG0}")
    e("str q1, [x9, x8]")
    e(f"add x9, sp, #{F_MSG1}")
    e("str q2, [x9, x8]")
    e("add w27, w6, #8")
    e("add w6, w6, #11")
    e(f"str w6, [sp, #{F_BASE}]")

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
    e(f"ldr w6, [sp, #{F_BLEN}]")
    e("dup v14.4s, w6")
    e("dup v15.4s, w28")
    e(f"stp q12, q13, [sp, #{F_DSPILL}]")
    e(f"stp q14, q15, [sp, #{F_DSPILL + 32}]")

    neon_stream = []
    for r in range(7):
        sched = SCHEDULE[r]
        for quads, base in ((COLS, 0), (DIAGS, 8)):
            neon_stream.extend(neon_quad_half_step(0, False, quads, sched, base, F_MSG0, n_words))
            neon_stream.extend(neon_quad_half_step(16, True, quads, sched, base, F_MSG1, n_words))

    scalar_stream = []
    for s_idx in range(3):
        scalar_stream.extend(scalar_one(s_idx, n_words, mode))

    L.extend(interleave(neon_stream, scalar_stream))

    if mode == "counter":
        for i in range(4):
            e(f"eor v{i}.16b, v{i}.16b, v{8 + i}.16b")
        for i in range(4):
            e(f"eor v{16 + i}.16b, v{16 + i}.16b, v{24 + i}.16b")
        e(f"ldr x6, [sp, #{F_AUX}]")
        e(f"cbnz x6, L_{name}_store16")
        # 32-byte full output (11 * 32 = 352 B/group).
        for i in range(4, 8):
            e(f"eor v{i}.16b, v{i}.16b, v{8 + i}.16b")
        L.extend(neon_store_quad(0, [0, 1, 2, 3], 32))
        for i in range(4):
            e(f"ldr q28, [sp, #{F_DSPILL + 16 * i}]")
            e(f"eor v{20 + i}.16b, v{20 + i}.16b, v28.16b")
        L.extend(neon_store_quad(16, [4, 5, 6, 7], 32))
        e("add x23, x25, #256")
        for off in range(0, 96, 16):
            e(f"ldp x6, x7, [sp, #{F_SCOUT + off}]")
            e(f"stp x6, x7, [x23, #{off}]")
        e("add x25, x25, #352")
        e(f"b L_{name}_next")

        # 16-byte truncated output (11 * 16 = 176 B/group).
        L.append(f"L_{name}_store16:")
        L.extend(neon_store_quad(0, [0, 1, 2, 3], 16))
        L.extend(neon_store_quad(16, [4, 5, 6, 7], 16))
        for s_idx in range(3):
            e(f"ldp x6, x7, [sp, #{F_SCOUT + 32 * s_idx}]")
            e(f"stp x6, x7, [x25, #{128 + 16 * s_idx}]")
        e("add x25, x25, #176")

        L.append(f"L_{name}_next:")
        e(f"ldr x6, [sp, #{F_GROUPS}]")
        e("subs x6, x6, #1")
        e(f"str x6, [sp, #{F_GROUPS}]")
        e(f"b.ne {loop_lbl}")
    else:
        # find mode: zero RAM writes, branchless 11-bit hit mask + single cbnz!
        e("eor v0.16b, v0.16b, v8.16b")
        e("eor v16.16b, v16.16b, v24.16b")
        e("dup v1.4s, w25")
        e("lsr x6, x25, #32")
        e("dup v2.4s, w6")
        e("and v0.16b, v0.16b, v1.16b")
        e("and v16.16b, v16.16b, v1.16b")
        e("eor v0.16b, v0.16b, v2.16b")
        e("eor v16.16b, v16.16b, v2.16b")
        e(f"stp q0, q16, [sp, #{F_SCOUT}]")
        e("mov x10, #0")
        for lane in range(11):
            e(f"ldr w8, [sp, #{F_SCOUT + 4 * lane}]")
            e("sub w9, wzr, w8")
            e("orr w8, w8, w9")
            e("lsr w8, w8, #31")
            e("eor w8, w8, #1")
            if lane == 0:
                e("orr x10, x10, x8")
            else:
                e(f"orr x10, x10, x8, lsl #{lane}")
        e(f"cbnz x10, L_{name}_hit")

        e(f"ldr x6, [sp, #{F_AUX}]")
        e("add x6, x6, #1")
        e(f"str x6, [sp, #{F_AUX}]")
        e(f"ldr x7, [sp, #{F_GROUPS}]")
        e("cmp x6, x7")
        e(f"b.lo {loop_lbl}")

        e("lsl x0, x6, #11")
        e(f"b L_{name}_restore")

        L.append(f"L_{name}_hit:")
        e(f"ldr x6, [sp, #{F_AUX}]")
        e("orr x0, x10, x6, lsl #11")

    L.append(f"L_{name}_restore:")
    for i, r in enumerate(range(8, 16, 2)):
        e(f"ldp d{r}, d{r + 1}, [sp, #{F_D8 + 16 * i}]")
    for i, r in enumerate(range(19, 31, 2)):
        e(f"ldp x{r}, x{r + 1}, [sp, #{F_X19 + 16 * i}]")
    e(f"add sp, sp, #{FRAME}")
    L.append(f"{done_lbl}:")
    e("ret")

    if mode == "find":
        L.append(f"L_{name}_zero:")
        e("mov x0, #0")
        e("ret")

    L.append(f".globl _{name}_end")
    L.append(f".globl {name}_end")
    L.append(f"_{name}_end:")
    L.append(f"{name}_end:")
    return "\n".join(L)


def main():
    parts = [
        "// Generated by tools/gen_counter_neon.py; zero-word-pruned NEON 8+3 counter & find kernels (#511, #585).",
        ".text",
    ]
    for w in (4, 9, 16):
        parts.append(emit_kernel(f"blake3_servil_neon_p11_counter_w{w}", w, "counter"))
    for w in (4, 9, 16):
        parts.append(emit_kernel(f"blake3_servil_neon_p11_find_w{w}", w, "find"))
    parts.append('#if defined(__ELF__) && defined(__linux__)\n.section .note.GNU-stack,"",@progbits\n#endif')
    print("\n\n".join(parts))


if __name__ == "__main__":
    main()
