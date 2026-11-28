#!/usr/bin/env python3
"""Generate c/blake3_chain_sme2_aarch64.S: 18-lane AArch64 SME2 + 2-integer
hybrid fused multi-step 32-byte hash-chain kernel (#584).

Each 32-byte chain slot holds `[prefix12 || start_step_u32_le || state16]`.
For each step `s in 0..steps`, every chain advances in registers without
exiting SME2 or touching memory:

    state_{s+1} = BLAKE3_compress(key, [prefix12, start_step + s, state_s],
                                  len=32, counter=0, flags)[..16]

16 chains run in SME2 streaming vectors (`z16..z18` = prefix, `z19` = step,
`z20..z23` = state, `z28` = 1; words 8..15 omitted at generation time) while
2 additional chains run on the P-core integer units interleaved 1-to-1 inside
the SME2 dependency shadow (18 chains per group, matching `gen_sme2_hybrid.py`).

AArch64 ABI:
  fn(
      init: *const u8,   // x0: 18 * groups * 32 bytes
      key: *const u32,   // x1: 8 x u32 key words
      flags: u32,        // w2: root block flags (domain | CHUNK_START | CHUNK_END | ROOT)
      steps: u32,        // w3: number of chain iterations (0 returns early without writing out)
      out: *mut u8,      // x4: 18 * groups * 16 bytes
      groups: u64,       // x5: number of 18-chain groups (0 returns early without writing out)
  ) -> u64               // SVL in 32-bit lanes (16 when work ran; 0 if TPIDR2 reserved != 0)

x18 untouched; d8-d15 and x19-x30 preserved.

    python3 tools/gen_chain_sme2.py > c/blake3_chain_sme2_aarch64.S
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

# Integer-lane registers (all disjoint from ZA selectors w12..w15, x18, x25..x28):
# 16 BLAKE3 state words + 1 message temp.
INT_STATE = ["w0", "w1", "w2", "w3", "w4", "w5", "w6", "w7", "w8", "w9", "w10", "w11", "w16", "w17", "w19", "w20"]
INT_M = "w21"
KEY_REG = "x25"
FLAGS_REG = "w26"
STEP_REG = "w27"

# Stack frame (256 B, 16-byte aligned).
F_D8 = 0         # d8..d15 (64 B)
F_X19 = 64       # x19..x30 (96 B)
F_INIT = 160     # running init pointer (8 B)
F_OUT = 168      # running out pointer (8 B)
F_GROUPS = 176   # remaining groups (8 B)
F_STEPS = 184    # steps per group (4 B)
F_INT0 = 192     # lane 16 32-byte working buffer [prefix12 || step || state16]
F_INT1 = 224     # lane 17 32-byte working buffer [prefix12 || step || state16]
FRAME = 256


def interleave(a, b):
    """Merge two instruction lists keeping each one's order, b spread evenly through a."""
    if not b:
        return list(a)
    if not a:
        return list(b)
    out, j = [], 0
    for i, ins in enumerate(a):
        out.append(ins)
        target = (i + 1) * len(b) // len(a)
        while j < target:
            out.append(b[j])
            j += 1
    out += b[j:]
    return out


def sme2_step_insns():
    """One full 7-round SME2 compression + in-register 16-byte feedback + step+=1
    for 16 chains (`z16..z18` = prefix, `z19` = step, `z20..z23` = state, `z28` = 1)."""
    L = [
        "mov { z0.s - z3.s }, za3h.s[w12, 0:3]",
        "mov { z4.s - z7.s }, za3h.s[w13, 0:3]",
        "mov { z8.s - z11.s }, za2h.s[w12, 0:3]",
        "mov z12.s, #0",
        "mov z13.s, #0",
        "mov z14.s, #32",
        f"dup z15.s, {FLAGS_REG}",
    ]
    for r in range(7):
        sched = SCHEDULE[r]
        for quads, base in ((COLS, 0), (DIAGS, 8)):
            cols = [(ai, bi, ci, di, sched[base + k], sched[base + k + 1])
                    for (ai, bi, ci, di), k in zip(quads, range(0, 8, 2))]
            for a, b, _, _, _, _ in cols:
                L.append(f"add z{a}.s, z{a}.s, z{b}.s")
            for a, _, _, _, mx, _ in cols:
                if mx < 8:
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
                if my < 8:
                    L.append(f"add z{a}.s, z{a}.s, z{16 + my}.s")
            for a, _, _, d, _, _ in cols:
                L.append(f"xar z{d}.s, z{d}.s, z{a}.s, #8")
            for _, _, c, d, _, _ in cols:
                L.append(f"add z{c}.s, z{c}.s, z{d}.s")
            for _, b, c, _, _, _ in cols:
                L.append(f"xar z{b}.s, z{b}.s, z{c}.s, #7")
    # 16-byte truncated feedback directly into z20..z23 and increment step in z19.
    for i in range(4):
        L.append(f"eor z{20 + i}.d, z{i}.d, z{8 + i}.d")
    L.append("add z19.s, z19.s, z28.s")
    return L


def int_lane_step_insns(buf_off):
    """One full 7-round zero-word-pruned (8-word) scalar compression on the
    32-byte stack buffer at `[sp, #buf_off]`, updating `state[0..3]` at
    `buf_off + 16..32` and incrementing `step` at `buf_off + 12`."""
    s = INT_STATE
    L = []
    # Load key[0..7] into s[0..7].
    for i in range(0, 8, 2):
        L.append(f"ldp {s[i]}, {s[i + 1]}, [{KEY_REG}, #{4 * i}]")
    # Load IV[0..3] into s[8..11].
    for i in range(4):
        L.append(f"mov {s[8 + i]}, #{IV[i] & 0xffff}")
        L.append(f"movk {s[8 + i]}, #{IV[i] >> 16}, lsl #16")
    # counter = 0, block_len = 32, flags = FLAGS_REG.
    L.append(f"mov {s[12]}, #0")
    L.append(f"mov {s[13]}, #0")
    L.append(f"mov {s[14]}, #32")
    L.append(f"mov {s[15]}, {FLAGS_REG}")
    # 7 zero-word-pruned rounds (words 8..15 are 0 and omitted).
    for r in range(7):
        for quads, base in ((COLS, 0), (DIAGS, 8)):
            for half in (0, 1):
                rot1, rot2 = (16, 12) if half == 0 else (8, 7)
                for (ai, bi, ci, di), q in zip(quads, range(4)):
                    a, b, c, d = s[ai], s[bi], s[ci], s[di]
                    word = SCHEDULE[r][base + 2 * q + half]
                    if word < 8:
                        L.append(f"ldr {INT_M}, [sp, #{buf_off + 4 * word}]")
                        L.append(f"add {a}, {a}, {INT_M}")
                    L.append(f"add {a}, {a}, {b}")
                    L.append(f"eor {d}, {d}, {a}")
                    L.append(f"ror {d}, {d}, #{rot1}")
                    L.append(f"add {c}, {c}, {d}")
                    L.append(f"eor {b}, {b}, {c}")
                    L.append(f"ror {b}, {b}, #{rot2}")
    # 16-byte truncated feedback + wrapping u32 step increment.
    for i in range(4):
        L.append(f"eor {s[i]}, {s[i]}, {s[8 + i]}")
    L.append(f"stp {s[0]}, {s[1]}, [sp, #{buf_off + 16}]")
    L.append(f"stp {s[2]}, {s[3]}, [sp, #{buf_off + 24}]")
    L.append(f"ldr {INT_M}, [sp, #{buf_off + 12}]")
    L.append(f"add {INT_M}, {INT_M}, #1")
    L.append(f"str {INT_M}, [sp, #{buf_off + 12}]")
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

    # Check SVL == 16 lanes (64 B), groups > 0, and steps > 0 before touching SME state.
    e("rdsvl x8, #1")
    e("lsr x8, x8, #2")
    e("cmp x8, #16")
    e(f"b.ne L_{name}_early")
    e(f"cbz x5, L_{name}_early")
    e(f"cbz w3, L_{name}_early")

    # Respect AAPCS64 TPIDR2_EL0 reserved bytes [10..15]: if nonzero, decline
    # with 0 without touching stack, TPIDR2_EL0, or ZA so Rust falls back to NEON.
    e("mrs x16, tpidr2_el0")
    e(f"cbz x16, L_{name}_tpidr2_ok")
    e("ldrh w15, [x16, #10]")
    e("ldr w14, [x16, #12]")
    e("orr w15, w15, w14")
    e(f"cbz w15, L_{name}_tpidr2_ok")
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

    e(f"str x0, [sp, #{F_INIT}]")
    e(f"mov {KEY_REG}, x1")
    e(f"mov {FLAGS_REG}, w2")
    e(f"str w3, [sp, #{F_STEPS}]")
    e(f"str x4, [sp, #{F_OUT}]")
    e(f"str x5, [sp, #{F_GROUPS}]")

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
    e("mov x8, #8")
    e("whilelo p1.s, xzr, x8")

    e("mov w12, #0")
    e("mov w13, #4")
    e("mov w14, #8")
    e("mov w15, #12")
    e("mov z28.s, #1")

    # Persistent IV[0..3] in ZA2 rows 0..3.
    for i in range(4):
        e(f"mov w8, #{IV[i] & 0xffff}")
        e(f"movk w8, #{IV[i] >> 16}, lsl #16")
        e(f"dup z{16 + i}.s, w8")
    e("mov za2h.s[w12, 0:3], { z16.s - z19.s }")

    # Persistent key[0..7] in ZA3 rows 0..7.
    for i in range(8):
        e(f"ldr w8, [{KEY_REG}, #{4 * i}]")
        e(f"dup z{16 + i}.s, w8")
    e("mov za3h.s[w12, 0:3], { z16.s - z19.s }")
    e("mov za3h.s[w13, 0:3], { z20.s - z23.s }")

    group_lbl = f"L_{name}_group"
    step_lbl = f"L_{name}_step"
    L.append(f"{group_lbl}:")

    # Load 16 SME2 chains (16 x 32 B = 512 B) into ZA0 rows 0..15 and transpose.
    e(f"ldr x0, [sp, #{F_INIT}]")
    for r_idx in range(16):
        sel = ZA_SELS[r_idx // 4]
        sl = r_idx % 4
        off = 32 * r_idx
        if off == 0:
            e(f"ld1w {{ za0h.s[{sel}, {sl}] }}, p1/z, [x0]")
        else:
            e(f"add x8, x0, #{off}")
            e(f"ld1w {{ za0h.s[{sel}, {sl}] }}, p1/z, [x8]")
    e("mov { z16.s - z19.s }, za0v.s[w12, 0:3]")
    e("mov { z20.s - z23.s }, za0v.s[w13, 0:3]")

    # Copy integer lanes 16 & 17 (bytes 512..576) into stack scratch [F_INT0], [F_INT1].
    e("add x8, x0, #512")
    e(f"ldp x1, x2, [x8, #0]")
    e(f"stp x1, x2, [sp, #{F_INT0}]")
    e(f"ldp x1, x2, [x8, #16]")
    e(f"stp x1, x2, [sp, #{F_INT0 + 16}]")
    e(f"ldp x1, x2, [x8, #32]")
    e(f"stp x1, x2, [sp, #{F_INT1}]")
    e(f"ldp x1, x2, [x8, #48]")
    e(f"stp x1, x2, [sp, #{F_INT1 + 16}]")
    e("add x0, x0, #576")
    e(f"str x0, [sp, #{F_INIT}]")

    e(f"ldr {STEP_REG}, [sp, #{F_STEPS}]")

    # Fused step loop: 16 SME2 chains interleaved 1-to-1 with 2 integer chains.
    L.append(f"{step_lbl}:")
    sme_insns = sme2_step_insns()
    int_insns = int_lane_step_insns(F_INT0) + int_lane_step_insns(F_INT1)
    for ins in interleave(sme_insns, int_insns):
        e(ins)
    e(f"subs {STEP_REG}, {STEP_REG}, #1")
    e(f"b.ne {step_lbl}")

    # Transpose and store 16-byte final states for all 18 chains (288 B):
    # transpose z20..z23 in registers into 4 full 64-byte vectors (4 lanes x
    # 16 B each) and emit 4 full-line unmasked `st1w p0` stores (eliminating
    # 16 partial 16-byte ZA-tile `st1w p2` stores).
    e(f"ldr x4, [sp, #{F_OUT}]")
    e("zip1 z4.s, z20.s, z21.s")
    e("zip2 z5.s, z20.s, z21.s")
    e("zip1 z6.s, z22.s, z23.s")
    e("zip2 z7.s, z22.s, z23.s")
    e("zip1 z0.d, z4.d, z6.d")
    e("zip2 z1.d, z4.d, z6.d")
    e("zip1 z2.d, z5.d, z7.d")
    e("zip2 z3.d, z5.d, z7.d")
    e("st1w { z0.s }, p0, [x4]")
    e("add x8, x4, #64")
    e("st1w { z1.s }, p0, [x8]")
    e("add x8, x4, #128")
    e("st1w { z2.s }, p0, [x8]")
    e("add x8, x4, #192")
    e("st1w { z3.s }, p0, [x8]")
    e(f"ldp x1, x2, [sp, #{F_INT0 + 16}]")
    e("stp x1, x2, [x4, #256]")
    e(f"ldp x1, x2, [sp, #{F_INT1 + 16}]")
    e("stp x1, x2, [x4, #272]")
    e("add x4, x4, #288")
    e(f"str x4, [sp, #{F_OUT}]")

    e(f"ldr x5, [sp, #{F_GROUPS}]")
    e("subs x5, x5, #1")
    e(f"str x5, [sp, #{F_GROUPS}]")
    e(f"b.ne {group_lbl}")

    e("mov x0, #16")
    e("smstop za")
    e("smstop sm")
    for n in range(19, 31, 2):
        e(f"ldp x{n}, x{n + 1}, [sp, #{F_X19 + 8 * (n - 19)}]")
    for n in range(8, 16, 2):
        e(f"ldp d{n}, d{n + 1}, [sp, #{F_D8 + 8 * (n - 8)}]")
    e(f"add sp, sp, #{FRAME}")
    e("ret")

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
        "// Generated by tools/gen_chain_sme2.py; 18-lane SME2+int hybrid fused hash-chain contract in its header.",
        ".arch armv9-a+sme2\n.text",
        emit_kernel("blake3_servil_sme2_chain32_trunc16"),
        '#if defined(__ELF__) && defined(__linux__)\n.section .note.GNU-stack,"",@progbits\n#endif',
    ]
    print("\n\n".join(parts))


if __name__ == "__main__":
    main()
