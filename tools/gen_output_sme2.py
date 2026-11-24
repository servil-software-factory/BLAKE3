#!/usr/bin/env python3
"""Generate AArch64 SME2 (SVL=512-bit, 16 chunks/group) multi-group keyed chunk
hashing kernel (#336):

blake3_servil_sme2_chunks_keyed(
    chunks:   *const *const u8,  // x0: 16 * groups pointers to 1024-byte chunks
    counters: *const u64,        // x1: groups starting chunk counters (+0..+15 per group)
    keys:     *const [u32; 8],   // x2: groups 8-word keys (one key per 16-chunk group)
    flags:    *const u8,         // x3: groups mode flag bytes (CHUNK_START/CHUNK_END added)
    out:      *mut u8,           // x4: 512 * groups output bytes (16 * groups CVs)
    groups:   u64,               // x5: number of 16-chunk groups (0 returns SVL early)
) -> u64

Hashes all `groups` keyed/mixed 16-chunk stages in one smstart/smstop entry
with pipelined block loading and zero key comparisons. Returns 16 when work
ran (or groups == 0 with SVL == 16), live SVL if != 16, or 0 if TPIDR2_EL0
reserved bytes [10..15] are nonzero.
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


def emit_half_round(quads, sched, base, preload_fn=None, preload_rows=()):
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
    if preload_fn and preload_rows:
        for row in preload_rows[:len(preload_rows) // 2]:
            L.extend(preload_fn(row))
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
    if preload_fn and preload_rows:
        for row in preload_rows[len(preload_rows) // 2:]:
            L.extend(preload_fn(row))
    return L


def emit_chunks_keyed(name):
    # ABI: x0=chunks, x1=counters, x2=keys, x3=flags, x4=out, x5=groups -> x0=svl
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

    # Respect AAPCS64 TPIDR2_EL0 reserved bytes [10..15]: decline with 0.
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

    e("ptrue p0.s")
    e("mov x8, #8")
    e("whilelo p2.s, xzr, x8")
    e("mov w12, #0")
    e("mov w13, #4")
    e("mov w14, #8")
    e("mov w15, #12")

    # Persistent IV[0..3] in ZA2 rows 0..3.
    for i in range(4):
        e(f"mov w8, #{IV[i] & 0xffff}")
        e(f"movk w8, #{IV[i] >> 16}, lsl #16")
        e(f"dup z{16 + i}.s, w8")
    e("mov za2h.s[w12, 0:3], { z16.s - z19.s }")

    group_lbl = f"L_{name}_group"
    block_lbl = f"L_{name}_block"
    not_first_lbl = f"L_{name}_not_first"
    L.append(f"{group_lbl}:")

    # Load this group's key[0..7] into z0..z7.
    for i in range(8):
        e(f"ldr w8, [x2, #{4 * i}]")
        e(f"dup z{i}.s, w8")

    # Load this group's base u64 counter -> z12 = lo + 0..15, z13 = hi + carry -> ZA3 rows 0..1.
    e("ldr x8, [x1], #8")
    e("index z12.s, w8, #1")
    e("dup z16.s, w8")
    e("cmplo p1.s, p0/z, z12.s, z16.s")
    e("lsr x8, x8, #32")
    e("dup z13.s, w8")
    e("mov z17.s, #1")
    e("add z13.s, p1/m, z13.s, z17.s")
    e("mov za3h.s[w12, 0:1], { z12.s - z13.s }")

    # Load this group's mode flags byte: w10=base, w11=base|CHUNK_START(1), w16=base|CHUNK_END(2).
    e("ldrb w10, [x3], #1")
    e("orr w11, w10, #1")
    e("orr w16, w10, #2")

    def preload_row(row):
        sel = ZA_SELS[row // 4]
        sl = row % 4
        return [
            f"    ldr x8, [x0, #{8 * row}]",
            f"    add x8, x8, x9",
            f"    ld1w {{ za0h.s[{sel}, {sl}] }}, p0/z, [x8]",
        ]

    # Load block 0 (offset x9 = 0) into ZA0.
    e("mov x9, #0")
    for row in range(16):
        L.extend(preload_row(row))

    # Blocks 0..14 with pipelined preload of next block (x9 = 64, 128, ..., 960).
    e("mov x7, #0")
    e("mov x9, #64")
    L.append(f"{block_lbl}:")
    e("mov w6, w10")
    e(f"cbnz x7, {not_first_lbl}")
    e("mov w6, w11")
    L.append(f"{not_first_lbl}:")
    e("mov { z8.s - z11.s }, za2h.s[w12, 0:3]")
    e("mov { z12.s - z13.s }, za3h.s[w12, 0:1]")
    e("mov z14.s, #64")
    e("dup z15.s, w6")
    for q in range(4):
        lo = 16 + 4 * q
        e(f"mov {{ z{lo}.s - z{lo + 3}.s }}, za0v.s[{ZA_SELS[q]}, 0:3]")

    for r in range(7):
        sched = SCHEDULE[r]
        pr_cols = range(4 * r, 4 * r + 2) if r < 4 else ()
        pr_diags = range(4 * r + 2, 4 * r + 4) if r < 4 else ()
        L.extend(emit_half_round(COLS, sched, 0, preload_row, list(pr_cols)))
        L.extend(emit_half_round(DIAGS, sched, 8, preload_row, list(pr_diags)))

    for i in range(8):
        e(f"eor z{i}.d, z{i}.d, z{8 + i}.d")

    e("add x9, x9, #64")
    e("add x7, x7, #1")
    e("cmp x7, #15")
    e(f"b.lo {block_lbl}")

    # Last block (block 15, already loaded in ZA0): flags = base | CHUNK_END.
    e("mov { z8.s - z11.s }, za2h.s[w12, 0:3]")
    e("mov { z12.s - z13.s }, za3h.s[w12, 0:1]")
    e("mov z14.s, #64")
    e("dup z15.s, w16")
    for q in range(4):
        lo = 16 + 4 * q
        e(f"mov {{ z{lo}.s - z{lo + 3}.s }}, za0v.s[{ZA_SELS[q]}, 0:3]")

    for r in range(7):
        sched = SCHEDULE[r]
        L.extend(emit_half_round(COLS, sched, 0))
        L.extend(emit_half_round(DIAGS, sched, 8))

    for i in range(8):
        e(f"eor z{i}.d, z{i}.d, z{8 + i}.d")

    e("mov za1v.s[w12, 0:3], { z0.s - z3.s }")
    e("mov za1v.s[w13, 0:3], { z4.s - z7.s }")
    for i in range(16):
        sel = ZA_SELS[i // 4]
        sl = i % 4
        off = 32 * i
        if off == 0:
            e(f"st1w {{ za1h.s[{sel}, {sl}] }}, p2, [x4]")
        else:
            e(f"add x8, x4, #{off}")
            e(f"st1w {{ za1h.s[{sel}, {sl}] }}, p2, [x8]")

    e("add x0, x0, #128")
    e("add x2, x2, #32")
    e("add x4, x4, #512")
    e("subs x5, x5, #1")
    e(f"b.ne {group_lbl}")

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
        "// Generated by tools/gen_output_sme2.py; multi-group keyed SME2 contract in its header.",
        ".arch armv9-a+sme2\n.text",
        emit_chunks_keyed("blake3_servil_sme2_chunks_keyed"),
        '#if defined(__ELF__) && defined(__linux__)\n.section .note.GNU-stack,"",@progbits\n#endif',
    ]
    print("\n\n".join(parts))


if __name__ == "__main__":
    main()
