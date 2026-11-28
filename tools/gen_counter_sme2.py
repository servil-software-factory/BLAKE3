#!/usr/bin/env python3
"""Generate 18-lane AArch64 SME2 + 2-integer hybrid zero-word-pruned
counter-range and early-exit grinding kernels (#511, #585).

Emits 8 kernels in `c/blake3_counter_sme2_aarch64.S`:
  - `blake3_servil_sme2_counter_w{4,9,13,16}`:
      18-lane hybrid (16 SME2 + 2 P-core integer lanes interleaved 1-to-1),
      pruning zero message words `>= n_words` (`<= 16/36/52/64 B` templates).
      Supports both 32-byte (`trunc16 = 0`, 576 B/group) and 16-byte
      (`trunc16 = 1`, 288 B/group) outputs via bit 16 of `flags_and_len`.
  - `blake3_servil_sme2_find_w{4,9,13,16}`:
      18-lane hybrid early-exit bitmask search inside one `smstart`/`smstop`
      session with zero output RAM writes on non-matching groups (testing
      `(word0 & mask) == target` in registers).

AArch64 ABI (`counter`):
  fn(
      template: *const u32,  // x0: 16 x u32 zero-padded 64-byte template
      key: *const u32,       // x1: 8 x u32 key words
      base_and_word: u64,    // x2: (base as u32) | ((counter_word as u64) << 32)
      flags_and_len: u64,    // x3: flags_u8 | (block_len_u8 << 8) | (trunc16_bit << 16)
      out: *mut u8,          // x4: (576 or 288) * groups bytes
      groups: u64,           // x5: number of 18-lane groups (0 returns 16)
  ) -> u64                   // 16 when work ran; != 16 if declined (SVL != 16 / TPIDR2)

AArch64 ABI (`find`):
  fn(
      template: *const u32,  // x0: 16 x u32 zero-padded 64-byte template
      key: *const u32,       // x1: 8 x u32 key words
      base_and_word: u64,    // x2: (base as u32) | ((counter_word as u64) << 32)
      flags_and_len: u64,    // x3: flags_u8 | (block_len_u8 << 8)
      mask_and_target: u64,  // x4: (mask as u32) | ((target as u64) << 32)
      groups: u64,           // x5: max 18-lane groups (< 2^45; 0 returns 0)
  ) -> u64                   // u64::MAX if declined; else (groups_done << 18) | hit_mask_18
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
ZA_SELS = ["w12", "w13", "w14", "w15"]

# Integer-lane registers (disjoint from ZA selectors w12..w15, macOS x18, persistent x22..x30):
INT_STATE = ["w0", "w1", "w2", "w3", "w4", "w5", "w6", "w7", "w8", "w9", "w10", "w11", "w16", "w17", "w19", "w20"]
INT_M = "w21"
KEY_REG = "x22"
OUT_OR_MT = "x23"
FL_REG = "w24"
BASE_REG = "w25"
CW_PTR = "x26"
CW_REG = "w27"
GRP_REG = "x28"

# Stack frame layout (336 B, 16-byte aligned):
F_TMPL = 0     # 64 B: scalar template words 0..15 (slot CW_PTR updated per int lane)
F_W0 = 64      # 72 B: counter mode = lane 16+17 digests (64 B); find mode = 18 diff words (72 B)
F_MAXG = 136   #  8 B: find mode max_groups
F_C16 = 144    #  4 B: lane 16 counter (base + 16)
F_C17 = 148    #  4 B: lane 17 counter (base + 17)
F_D8 = 160     # 64 B: saved d8..d15
F_X19 = 224    # 96 B: saved x19..x30 (x29 at 224 + 80 = 304)
FRAME = 336


def sme2_rounds_insns(n_words):
    """7 zero-word-pruned SME2 rounds over z0..z15 using message words z16..z(16+n_words-1)."""
    L = []
    for r in range(7):
        sched = SCHEDULE[r]
        for quads, base in ((COLS, 0), (DIAGS, 8)):
            cols = [(ai, bi, ci, di, sched[base + k], sched[base + k + 1])
                    for (ai, bi, ci, di), k in zip(quads, range(0, 8, 2))]
            for a, b, _, _, _, _ in cols:
                L.append(f"add z{a}.s, z{a}.s, z{b}.s")
            for a, _, _, _, mx, _ in cols:
                if mx < n_words:
                    L.append(f"add z{a}.s, z{a}.s, z{16 + mx}.s")
            for a, _, _, d, _, _ in cols:
                L.append(f"xar z{d}.s, z{d}.s, z{a}.s, #16")
            for _, _, c, d, _, _ in cols:
                L.append(f"add z{c}.s, z{c}.s, z{d}.s")
            for _, b, c, _, _, _ in cols:
                L.append(f"xar z{b}.s, z{b}.s, z{c}.s, #12")
            for a, b, _, _, _, _ in cols:
                L.append(f"add z{a}.s, z{a}.s, z{b}.s")
            for a, _, _, _, _, my in cols:
                if my < n_words:
                    L.append(f"add z{a}.s, z{a}.s, z{16 + my}.s")
            for a, _, _, d, _, _ in cols:
                L.append(f"xar z{d}.s, z{d}.s, z{a}.s, #8")
            for _, _, c, d, _, _ in cols:
                L.append(f"add z{c}.s, z{c}.s, z{d}.s")
            for _, b, c, _, _, _ in cols:
                L.append(f"xar z{b}.s, z{b}.s, z{c}.s, #7")
    return L


def int_lane_insns(n_words, ctr_off, mode, out_off):
    """One zero-word-pruned scalar compression for integer lane 16 or 17.
    Loads its u32 counter from `[sp, #ctr_off]`, stores it at `[CW_PTR]`,
    runs 7 rounds, and writes either 32 B digest (`mode == 'counter'`) or
    masked-xor-target word0 (`mode == 'find'`) to `[sp, #out_off]`."""
    s = INT_STATE
    L = [
        f"ldr {INT_M}, [sp, #{ctr_off}]",
        f"str {INT_M}, [{CW_PTR}]",
    ]
    for i in range(0, 8, 2):
        L.append(f"ldp {s[i]}, {s[i + 1]}, [{KEY_REG}, #{4 * i}]")
    for i in range(4):
        L.append(f"mov {s[8 + i]}, #{IV[i] & 0xffff}")
        L.append(f"movk {s[8 + i]}, #{IV[i] >> 16}, lsl #16")
    L.append(f"mov {s[12]}, #0")
    L.append(f"mov {s[13]}, #0")
    L.append(f"lsr {s[14]}, {FL_REG}, #8")
    L.append(f"and {s[14]}, {s[14]}, #0xff")
    L.append(f"and {s[15]}, {FL_REG}, #0xff")
    for r in range(7):
        for quads, base in ((COLS, 0), (DIAGS, 8)):
            for half in (0, 1):
                rot1, rot2 = (16, 12) if half == 0 else (8, 7)
                for (ai, bi, ci, di), q in zip(quads, range(4)):
                    a, b, c, d = s[ai], s[bi], s[ci], s[di]
                    word = SCHEDULE[r][base + 2 * q + half]
                    if word < n_words:
                        L.append(f"ldr {INT_M}, [sp, #{F_TMPL + 4 * word}]")
                        L.append(f"add {a}, {a}, {INT_M}")
                    L.append(f"add {a}, {a}, {b}")
                    L.append(f"eor {d}, {d}, {a}")
                    L.append(f"ror {d}, {d}, #{rot1}")
                    L.append(f"add {c}, {c}, {d}")
                    L.append(f"eor {b}, {b}, {c}")
                    L.append(f"ror {b}, {b}, #{rot2}")
    if mode == "counter":
        for i in range(8):
            L.append(f"eor {s[i]}, {s[i]}, {s[8 + i]}")
        for i in range(0, 8, 2):
            L.append(f"stp {s[i]}, {s[i + 1]}, [sp, #{out_off + 4 * i}]")
    else:
        # find mode: compute (word0 & mask) ^ target (0 iff hit).
        L.append(f"eor {s[0]}, {s[0]}, {s[8]}")
        L.append(f"and {s[0]}, {s[0]}, w23")
        L.append(f"lsr x1, {OUT_OR_MT}, #32")
        L.append(f"eor {s[0]}, {s[0]}, w1")
        L.append(f"str {s[0]}, [sp, #{out_off}]")
    return L


def interleave(sme_insns, int_insns):
    """Interleave SME2 and integer instructions evenly."""
    out = []
    ns, ni = len(sme_insns), len(int_insns)
    si = ii = 0
    while si < ns or ii < ni:
        if si < ns and (ii >= ni or si * ni <= ii * ns):
            out.append(sme_insns[si])
            si += 1
        else:
            out.append(int_insns[ii])
            ii += 1
    return out


def emit_kernel(name, n_words, mode):
    assert mode in ("counter", "find")
    n_quads = (n_words + 3) // 4
    L = [
        f".globl _{name}",
        f".globl {name}",
        ".p2align 4",
        f"_{name}:",
        f"{name}:",
    ]
    e = lambda s: L.append(f"    {s}")

    # Check SVL == 16 lanes (64 B) before touching SME state or stack.
    e("rdsvl x8, #1")
    e("lsr x8, x8, #2")
    e("cmp x8, #16")
    if mode == "find":
        e(f"b.ne L_{name}_decline")
        e(f"cbz x5, L_{name}_zero")
    else:
        e(f"b.ne L_{name}_early")
        e(f"cbz x5, L_{name}_early")

    # Respect AAPCS64 TPIDR2_EL0 reserved bytes [10..15]: decline cleanly if nonzero.
    e("mrs x16, tpidr2_el0")
    e(f"cbz x16, L_{name}_tpidr2_ok")
    e("ldrh w15, [x16, #10]")
    e("ldr w14, [x16, #12]")
    e("orr w15, w15, w14")
    e(f"cbz w15, L_{name}_tpidr2_ok")
    if mode == "find":
        e(f"b L_{name}_decline")
    else:
        e("mov x8, #0")
        e(f"b L_{name}_early")
    L.append(f"L_{name}_tpidr2_ok:")

    e(f"sub sp, sp, #{FRAME}")
    for n in range(8, 16, 2):
        e(f"stp d{n}, d{n + 1}, [sp, #{F_D8 + 8 * (n - 8)}]")
    for n in range(19, 31, 2):
        e(f"stp x{n}, x{n + 1}, [sp, #{F_X19 + 8 * (n - 19)}]")
    # Keep x29 pointing at the saved (FP, LR) frame record (Apple arm64 ABI).
    e(f"add x29, sp, #{F_X19 + 80}")

    # Copy active template words into [sp, #F_TMPL] for the 2 integer lanes.
    for i in range(0, n_words, 2):
        if i + 1 < n_words:
            e(f"ldp w8, w9, [x0, #{4 * i}]")
            e(f"stp w8, w9, [sp, #{F_TMPL + 4 * i}]")
        else:
            e(f"ldr w8, [x0, #{4 * i}]")
            e(f"str w8, [sp, #{F_TMPL + 4 * i}]")

    e(f"mov {KEY_REG}, x1")
    e(f"mov {BASE_REG}, w2")
    e("lsr x8, x2, #32")
    e(f"mov {CW_REG}, w8")
    e(f"add {CW_PTR}, sp, x8, lsl #2")
    e(f"mov {FL_REG}, w3")
    e(f"mov {OUT_OR_MT}, x4")
    if mode == "counter":
        e(f"mov {GRP_REG}, x5")
    else:
        e(f"str x5, [sp, #{F_MAXG}]")
        e(f"mov {GRP_REG}, #0")

    # AAPCS64 SME private-ZA lazy-saving protocol.
    e("mrs x16, tpidr2_el0")
    e(f"cbz x16, L_{name}_za_ok")
    e("ldr x14, [x16]")
    e(f"cbz x14, L_{name}_za_clr")
    e("ldrh w15, [x16, #8]")
    e(f"cbz w15, L_{name}_za_clr")
    e("mov w12, #0")
    L.append(f"L_{name}_za_save:")
    e("str za[w12, 0], [x14]")
    e("addsvl x14, x14, #1")
    e("add w12, w12, #1")
    e("cmp w12, w15")
    e(f"b.lo L_{name}_za_save")
    L.append(f"L_{name}_za_clr:")
    e("msr tpidr2_el0, xzr")
    L.append(f"L_{name}_za_ok:")

    e("smstart sm")
    e("smstart za")

    e("ptrue p0.s")

    e("mov w12, #0")
    e("mov w13, #4")
    e("mov w14, #8")
    e("mov w15, #12")

    # Load IV[0..3] once into ZA2 rows 0..3.
    for i in range(4):
        e(f"mov w8, #{IV[i] & 0xffff}")
        e(f"movk w8, #{IV[i] >> 16}, lsl #16")
        e(f"dup z{16 + i}.s, w8")
    e("mov za2h.s[w12, 0:3], { z16.s - z19.s }")

    # Load key[0..7] once into ZA3 rows 0..7.
    for i in range(8):
        e(f"ldr w8, [{KEY_REG}, #{4 * i}]")
        e(f"dup z{16 + i}.s, w8")
    e("mov za3h.s[w12, 0:3], { z16.s - z19.s }")
    e("mov za3h.s[w13, 0:3], { z20.s - z23.s }")

    # Broadcast active template words 0..n_words-1 once into ZA0.
    for q in range(n_quads):
        for j in range(4):
            w = 4 * q + j
            if w < n_words:
                e(f"ldr w8, [sp, #{F_TMPL + 4 * w}]")
                e(f"dup z{16 + j}.s, w8")
            else:
                e(f"mov z{16 + j}.s, #0")
        e(f"mov za0h.s[{ZA_SELS[q]}, 0:3], {{ z16.s - z19.s }}")

    loop_lbl = f"L_{name}_loop"
    L.append(f"{loop_lbl}:")

    # Generate [base..base+15] into ZA0 row counter_word, and save base+16, base+17 for int lanes.
    e(f"index z28.s, {BASE_REG}, #1")
    e(f"mov w15, {CW_REG}")
    e("mov za0h.s[w15, 0], p0/m, z28.s")
    e("mov w15, #12")
    e(f"add w8, {BASE_REG}, #16")
    e(f"add w9, {BASE_REG}, #17")
    e(f"stp w8, w9, [sp, #{F_C16}]")
    e(f"add {BASE_REG}, {BASE_REG}, #18")

    # Initialize SME2 working state z0..z15 and load active message quads.
    e("mov { z0.s - z3.s }, za3h.s[w12, 0:3]")
    e("mov { z4.s - z7.s }, za3h.s[w13, 0:3]")
    e("mov { z8.s - z11.s }, za2h.s[w12, 0:3]")
    e("mov z12.s, #0")
    e("mov z13.s, #0")
    e(f"lsr w8, {FL_REG}, #8")
    e("and w8, w8, #0xff")
    e("dup z14.s, w8")
    e(f"and w8, {FL_REG}, #0xff")
    e("dup z15.s, w8")
    for q in range(n_quads):
        lo = 16 + 4 * q
        e(f"mov {{ z{lo}.s - z{lo + 3}.s }}, za0h.s[{ZA_SELS[q]}, 0:3]")

    # Interleave 16-lane SME2 rounds with 2 P-core integer lanes (16 & 17).
    sme_insns = sme2_rounds_insns(n_words)
    if mode == "counter":
        int_insns = int_lane_insns(n_words, F_C16, "counter", F_W0) + int_lane_insns(n_words, F_C17, "counter", F_W0 + 32)
    else:
        int_insns = int_lane_insns(n_words, F_C16, "find", F_W0 + 64) + int_lane_insns(n_words, F_C17, "find", F_W0 + 68)
    for insn in interleave(sme_insns, int_insns):
        e(insn)

    if mode == "counter":
        for i in range(4):
            e(f"eor z{i}.d, z{i}.d, z{8 + i}.d")
        e(f"tbnz {FL_REG}, #16, L_{name}_store16")
        # 32-byte full output (18 * 32 = 576 B/group): mirror z0..z7 into both
        # halves of ZA1 (cols 0..7 and 8..15) so each row r holds [L_r || L_r],
        # pack adjacent lanes with `ext #32`, and emit 8 full-line 64-byte
        # unmasked `st1w p0` stores (eliminating 16 partial `st1w za1h, p2` stores).
        for i in range(4, 8):
            e(f"eor z{i}.d, z{i}.d, z{8 + i}.d")
        e("mov za1v.s[w12, 0:3], { z0.s - z3.s }")
        e("mov za1v.s[w13, 0:3], { z4.s - z7.s }")
        e("mov za1v.s[w14, 0:3], { z0.s - z3.s }")
        e("mov za1v.s[w15, 0:3], { z4.s - z7.s }")
        for q, sel in enumerate(ZA_SELS):
            e(f"mov {{ z0.s - z3.s }}, za1h.s[{sel}, 0:3]")
            e("ext z0.b, z0.b, z1.b, #32")
            e("ext z2.b, z2.b, z3.b, #32")
            off0 = 128 * q
            if off0 == 0:
                e(f"st1w {{ z0.s }}, p0, [{OUT_OR_MT}]")
            else:
                e(f"add x8, {OUT_OR_MT}, #{off0}")
                e("st1w { z0.s }, p0, [x8]")
            e(f"add x8, {OUT_OR_MT}, #{off0 + 64}")
            e("st1w { z2.s }, p0, [x8]")
        # Copy lanes 16 & 17 (64 B) from [sp, #F_W0] to [out + 512].
        e(f"add x8, {OUT_OR_MT}, #512")
        for off in range(0, 64, 16):
            e(f"ldp x0, x1, [sp, #{F_W0 + off}]")
            e(f"stp x0, x1, [x8, #{off}]")
        e(f"add {OUT_OR_MT}, {OUT_OR_MT}, #576")
        e(f"b L_{name}_next")

        # 16-byte truncated output (18 * 16 = 288 B/group): transpose z0..z3
        # in registers into 4 full 64-byte vectors (4 lanes x 16 B each) and
        # emit 4 full-line unmasked `st1w p0` stores (eliminating 16 partial
        # 16-byte ZA-tile `st1w p1` stores and Apple M6 sub-line store stalls).
        L.append(f"L_{name}_store16:")
        e("zip1 z4.s, z0.s, z1.s")
        e("zip2 z5.s, z0.s, z1.s")
        e("zip1 z6.s, z2.s, z3.s")
        e("zip2 z7.s, z2.s, z3.s")
        e("zip1 z0.d, z4.d, z6.d")
        e("zip2 z1.d, z4.d, z6.d")
        e("zip1 z2.d, z5.d, z7.d")
        e("zip2 z3.d, z5.d, z7.d")
        e(f"st1w {{ z0.s }}, p0, [{OUT_OR_MT}]")
        e(f"add x8, {OUT_OR_MT}, #64")
        e("st1w { z1.s }, p0, [x8]")
        e(f"add x8, {OUT_OR_MT}, #128")
        e("st1w { z2.s }, p0, [x8]")
        e(f"add x8, {OUT_OR_MT}, #192")
        e("st1w { z3.s }, p0, [x8]")
        # Copy lanes 16 & 17 first 16 B each to [out + 256], [out + 272].
        e(f"ldp x0, x1, [sp, #{F_W0}]")
        e(f"stp x0, x1, [{OUT_OR_MT}, #256]")
        e(f"ldp x0, x1, [sp, #{F_W0 + 32}]")
        e(f"stp x0, x1, [{OUT_OR_MT}, #272]")
        e(f"add {OUT_OR_MT}, {OUT_OR_MT}, #288")

        L.append(f"L_{name}_next:")
        e(f"subs {GRP_REG}, {GRP_REG}, #1")
        e(f"b.ne {loop_lbl}")

        e("smstop za")
        e("smstop sm")
        e("mov x0, #16")
    else:
        # find mode: in-register word0 pre-filter (ZERO RAM writes on miss!).
        e("eor z0.d, z0.d, z8.d")
        e("dup z1.s, w23")
        e(f"lsr x8, {OUT_OR_MT}, #32")
        e("dup z2.s, w8")
        e("and z0.d, z0.d, z1.d")
        e("cmpeq p1.s, p0/z, z0.s, z2.s")
        e(f"b.ne L_{name}_hit")
        e(f"ldp w0, w1, [sp, #{F_W0 + 64}]")
        e(f"cbz w0, L_{name}_hit")
        e(f"cbz w1, L_{name}_hit")

        e(f"add {GRP_REG}, {GRP_REG}, #1")
        e(f"ldr x8, [sp, #{F_MAXG}]")
        e(f"cmp {GRP_REG}, x8")
        e(f"b.lo {loop_lbl}")

        # No hit across all groups.
        e("smstop za")
        e("smstop sm")
        e(f"lsl x0, {GRP_REG}, #18")
        e(f"b L_{name}_restore")

        # Cold hit path: store 16 SME2 diff words (`0` iff hit) to [sp, #F_W0],
        # leave streaming mode, and build exact 18-bit hit mask branchlessly.
        L.append(f"L_{name}_hit:")
        e("eor z0.d, z0.d, z2.d")
        e("mov za1h.s[w12, 0], p0/m, z0.s")
        e(f"add x8, sp, #{F_W0}")
        e("st1w { za1h.s[w12, 0] }, p0, [x8]")
        e("smstop za")
        e("smstop sm")
        e("mov x10, #0")
        for lane in range(18):
            e(f"ldr w8, [sp, #{F_W0 + 4 * lane}]")
            e("sub w9, wzr, w8")
            e("orr w8, w8, w9")
            e("lsr w8, w8, #31")
            e("eor w8, w8, #1")
            if lane == 0:
                e("orr x10, x10, x8")
            else:
                e(f"orr x10, x10, x8, lsl #{lane}")
        e(f"orr x0, x10, {GRP_REG}, lsl #18")

    L.append(f"L_{name}_restore:")
    for n in range(19, 31, 2):
        e(f"ldp x{n}, x{n + 1}, [sp, #{F_X19 + 8 * (n - 19)}]")
    for n in range(8, 16, 2):
        e(f"ldp d{n}, d{n + 1}, [sp, #{F_D8 + 8 * (n - 8)}]")
    e(f"add sp, sp, #{FRAME}")
    e("ret")

    if mode == "find":
        L.append(f"L_{name}_zero:")
        e("mov x0, #0")
        e("ret")
        L.append(f"L_{name}_decline:")
        e("mov x0, #-1")
        e("ret")
    else:
        L.append(f"L_{name}_early:")
        e("mov x0, x8")
        e("ret")

    L.append(f".globl _{name}_end")
    L.append(f".globl {name}_end")
    L.append(f"_{name}_end:")
    L.append(f"{name}_end:")
    return "\n".join(L)


def main():
    parts = [
        "// Generated by tools/gen_counter_sme2.py; 18-lane SME2+int hybrid zero-word-pruned counter & find kernels (#511, #585).",
        ".arch armv9-a+sme2\n.text",
    ]
    for w in (4, 9, 13, 16):
        parts.append(emit_kernel(f"blake3_servil_sme2_counter_w{w}", w, "counter"))
    for w in (4, 9, 13, 16):
        parts.append(emit_kernel(f"blake3_servil_sme2_find_w{w}", w, "find"))
    parts.append('#if defined(__ELF__) && defined(__linux__)\n.section .note.GNU-stack,"",@progbits\n#endif')
    print("\n\n".join(parts))


if __name__ == "__main__":
    main()
