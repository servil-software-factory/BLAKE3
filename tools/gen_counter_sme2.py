#!/usr/bin/env python3
"""Generate register-only AArch64 SME2 (SVL=512-bit, 16 lanes/group) universal
<= 64 B counter-grinding kernel (#511).

AArch64 ABI:
  fn(
      template: *const u32,  // x0: 16 x u32 zero-padded 64-byte template
      key: *const u32,       // x1: 8 x u32 key words
      base_and_word: u64,    // x2: (base as u32) | ((counter_word as u64) << 32)
      flags_and_len: u64,    // x3: (flags & 0xff) | ((block_len & 0xff) << 8)
      out: *mut u8,          // x4: 512 * groups bytes
      groups: u64,           // x5: number of 16-try groups (0 returns early)
  ) -> u64                   // SVL in 32-bit lanes (16 when work ran; 0 if TPIDR2 reserved != 0)

Broadcasts all 16 template words once into ZA0.S before the loop (key in ZA3.S,
IV in ZA2.S). Each group overwrites only row counter_word of ZA0 with
`index z28.s, w6, #1`, eliminating input RAM reads and P-core->SME2 coherency
stalls inside the group loop.
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


def emit_half_round(quads, sched, base):
    cols = [(ai, bi, ci, di, sched[base + k], sched[base + k + 1])
            for (ai, bi, ci, di), k in zip(quads, range(0, 8, 2))]
    L = []
    e = lambda s: L.append(f"    {s}")
    for a, b, _, _, _, _ in cols:
        e(f"add z{a}.s, z{a}.s, z{b}.s")
    for a, _, _, _, mx, _ in cols:
        e(f"add z{a}.s, z{a}.s, z{16 + mx}.s")
    for a, _, _, d, _, _ in cols:
        e(f"xar z{d}.s, z{d}.s, z{a}.s, #16")
    for _, _, c, d, _, _ in cols:
        e(f"add z{c}.s, z{c}.s, z{d}.s")
    for _, b, c, _, _, _ in cols:
        e(f"xar z{b}.s, z{b}.s, z{c}.s, #12")
    for a, b, _, _, _, _ in cols:
        e(f"add z{a}.s, z{a}.s, z{b}.s")
    for a, _, _, _, _, my in cols:
        e(f"add z{a}.s, z{a}.s, z{16 + my}.s")
    for a, _, _, d, _, _ in cols:
        e(f"xar z{d}.s, z{d}.s, z{a}.s, #8")
    for _, _, c, d, _, _ in cols:
        e(f"add z{c}.s, z{c}.s, z{d}.s")
    for _, b, c, _, _, _ in cols:
        e(f"xar z{b}.s, z{b}.s, z{c}.s, #7")
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

    # Check SVL == 16 lanes (64 B) and groups > 0 before touching SME state.
    e("rdsvl x8, #1")
    e("lsr x8, x8, #2")
    e("cmp x8, #16")
    e(f"b.ne L_{name}_early")
    e(f"cbz x5, L_{name}_early")

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

    e("sub sp, sp, #64")
    e("stp d8, d9, [sp, #0]")
    e("stp d10, d11, [sp, #16]")
    e("stp d12, d13, [sp, #32]")
    e("stp d14, d15, [sp, #48]")

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

    # Unpack scalar parameters:
    #   w6  = base (wrapping u32 counter)
    #   w11 = counter_word (0..15)
    #   w10 = flags (bits 0..7)
    #   w9  = block_len (bits 8..15)
    e("mov w6, w2")
    e("lsr x11, x2, #32")
    e("and w10, w3, #0xff")
    e("lsr w9, w3, #8")
    e("and w9, w9, #0xff")

    e("ptrue p0.s")
    e("mov x8, #8")
    e("whilelo p2.s, xzr, x8")

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
        e(f"ldr w8, [x1, #{4 * i}]")
        e(f"dup z{16 + i}.s, w8")
    e("mov za3h.s[w12, 0:3], { z16.s - z19.s }")
    e("mov za3h.s[w13, 0:3], { z20.s - z23.s }")

    # Broadcast all 16 template words once into ZA0 rows 0..15.
    for q in range(4):
        for j in range(4):
            e(f"ldr w8, [x0, #{4 * (4 * q + j)}]")
            e(f"dup z{16 + j}.s, w8")
        e(f"mov za0h.s[{ZA_SELS[q]}, 0:3], {{ z16.s - z19.s }}")

    loop_lbl = f"L_{name}_loop"
    L.append(f"{loop_lbl}:")

    # Generate [base, base+1, ..., base+15] (wrapping u32) and overwrite only
    # row counter_word of ZA0.
    e("index z28.s, w6, #1")
    e("mov w15, w11")
    e("mov za0h.s[w15, 0], p0/m, z28.s")
    e("mov w15, #12")
    e("add w6, w6, #16")

    e("mov { z0.s - z3.s }, za3h.s[w12, 0:3]")
    e("mov { z4.s - z7.s }, za3h.s[w13, 0:3]")
    e("mov { z8.s - z11.s }, za2h.s[w12, 0:3]")
    e("mov z12.s, #0")
    e("mov z13.s, #0")
    e("dup z14.s, w9")
    e("dup z15.s, w10")

    for q in range(4):
        lo = 16 + 4 * q
        e(f"mov {{ z{lo}.s - z{lo + 3}.s }}, za0h.s[{ZA_SELS[q]}, 0:3]")

    for r in range(7):
        sched = SCHEDULE[r]
        L.extend(emit_half_round(COLS, sched, 0))
        L.extend(emit_half_round(DIAGS, sched, 8))

    for i in range(8):
        e(f"eor z{i}.d, z{i}.d, z{8 + i}.d")

    e("mov za1v.s[w12, 0:3], { z0.s - z3.s }")
    e("mov za1v.s[w13, 0:3], { z4.s - z7.s }")
    for r_idx in range(16):
        sel = ZA_SELS[r_idx // 4]
        sl = r_idx % 4
        off = 32 * r_idx
        if off == 0:
            e(f"st1w {{ za1h.s[{sel}, {sl}] }}, p2, [x4]")
        else:
            e(f"add x8, x4, #{off}")
            e(f"st1w {{ za1h.s[{sel}, {sl}] }}, p2, [x8]")

    e("add x4, x4, #512")
    e("subs x5, x5, #1")
    e(f"b.ne {loop_lbl}")

    e("mov x0, #16")
    e("smstop za")
    e("smstop sm")
    e("ldp d8, d9, [sp, #0]")
    e("ldp d10, d11, [sp, #16]")
    e("ldp d12, d13, [sp, #32]")
    e("ldp d14, d15, [sp, #48]")
    e("add sp, sp, #64")
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
        "// Generated by tools/gen_counter_sme2.py; register-only SME2 counter-range contract in its header.",
        ".arch armv9-a+sme2\n.text",
        emit_kernel("blake3_servil_sme2_counter"),
        '#if defined(__ELF__) && defined(__linux__)\n.section .note.GNU-stack,"",@progbits\n#endif',
    ]
    print("\n\n".join(parts))


if __name__ == "__main__":
    main()
