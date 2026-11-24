#!/usr/bin/env python3
"""Generate direct-load Apple SME2 (SVL=512-bit, 16 lanes/group) kernels for
Packed<16>, Packed<32>, and Packed<48>:
  - Zero 8 KiB stack buffer, zero pointer table, zero store-buffer stalls
  - Contiguous predicated ld1w directly into ZA0.S (p1 = 4/8/12 words)
  - 1-3 VG4 vertical movs transpose all 16 messages into z16..z(16+nwords-1)
  - Persistent key in ZA3.S and IV in ZA2.S (loaded ONCE per call)
  - Next group's 16 ld1w loads pipelined inside rounds 0..6
  - All zero-word message adds (25-75%) omitted at generation time

AArch64 ABI:
  fn(input: *const u8, key: *const u32, flags: u32, out: *mut u8, groups: u64) -> u64
     x0                x1               w2          x3            x4
Returns SVL in 32-bit lanes (16 on Apple M4/M6; does work only when == 16 and groups > 0).
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
ZA_SELS = ["w12", "w13", "w14", "w15"]


def emit_load_row(row_idx, nbytes):
    sel = ZA_SELS[row_idx // 4]
    sl = row_idx % 4
    off = row_idx * nbytes
    if off == 0:
        return [f"ld1w {{ za0h.s[{sel}, {sl}] }}, p1/z, [x0]"]
    return [
        f"add x8, x0, #{off}",
        f"ld1w {{ za0h.s[{sel}, {sl}] }}, p1/z, [x8]",
    ]


def emit_half_round(quads, sched, base, nwords, prefetch_rows=()):
    cols = [(ai, bi, ci, di, sched[base + k], sched[base + k + 1])
            for (ai, bi, ci, di), k in zip(quads, range(0, 8, 2))]
    L = []
    e = L.append
    for a, b, _, _, _, _ in cols:
        e(f"add z{a}.s, z{a}.s, z{b}.s")
    for a, _, _, _, mx, _ in cols:
        if mx < nwords:
            e(f"add z{a}.s, z{a}.s, z{16 + mx}.s")
    for a, _, _, d, _, _ in cols:
        e(f"xar z{d}.s, z{d}.s, z{a}.s, #16")
    for _, _, c, d, _, _ in cols:
        e(f"add z{c}.s, z{c}.s, z{d}.s")
    for _, b, c, _, _, _ in cols:
        e(f"xar z{b}.s, z{b}.s, z{c}.s, #12")
    if prefetch_rows:
        for r_idx, nb in prefetch_rows[:len(prefetch_rows) // 2]:
            L.extend(emit_load_row(r_idx, nb))
    for a, b, _, _, _, _ in cols:
        e(f"add z{a}.s, z{a}.s, z{b}.s")
    for a, _, _, _, _, my in cols:
        if my < nwords:
            e(f"add z{a}.s, z{a}.s, z{16 + my}.s")
    for a, _, _, d, _, _ in cols:
        e(f"xar z{d}.s, z{d}.s, z{a}.s, #8")
    for _, _, c, d, _, _ in cols:
        e(f"add z{c}.s, z{c}.s, z{d}.s")
    for _, b, c, _, _, _ in cols:
        e(f"xar z{b}.s, z{b}.s, z{c}.s, #7")
    if prefetch_rows:
        for r_idx, nb in prefetch_rows[len(prefetch_rows) // 2:]:
            L.extend(emit_load_row(r_idx, nb))
    return L


def emit_kernel(name, nbytes, prefix):
    nwords = nbytes // 4
    sym = f"{prefix}{name}"
    L = [f".globl {sym}", ".p2align 4", f"{sym}:"]
    e = L.append

    # Check SVL == 16 lanes (64 B) and groups > 0 before touching SME state.
    e("rdsvl x8, #1")
    e("lsr x8, x8, #2")
    e("cmp x8, #16")
    e(f"b.ne L_{name}_early")
    e(f"cbz x4, L_{name}_early")

    e("sub sp, sp, #64")
    e("stp d8, d9, [sp, #0]")
    e("stp d10, d11, [sp, #16]")
    e("stp d12, d13, [sp, #32]")
    e("stp d14, d15, [sp, #48]")

    # AAPCS64 SME private-ZA lazy-saving protocol (SAVE_DORMANT_ZA).
    e("mrs x16, tpidr2_el0")
    e(f"cbz x16, L_{name}_za_ok")
    e("ldr x14, [x16]")
    e(f"cbz x14, L_{name}_za_clr")
    e("ldrh w15, [x16, #8]")
    e(f"cbz w15, L_{name}_za_clr")
    e("mov w12, #0")
    e(f"L_{name}_za_save:")
    e("str za[w12, 0], [x14]")
    e("addsvl x14, x14, #1")
    e("add w12, w12, #1")
    e("cmp w12, w15")
    e(f"b.lo L_{name}_za_save")
    e(f"L_{name}_za_clr:")
    e("msr tpidr2_el0, xzr")
    e(f"L_{name}_za_ok:")

    e("smstart sm")
    e("smstart za")

    # Predicates: p1 = nwords (4/8/12) for input ld1w, p2 = 8 for 32 B output st1w.
    e(f"mov x8, #{nwords}")
    e("whilelo p1.s, xzr, x8")
    e("mov x8, #8")
    e("whilelo p2.s, xzr, x8")

    e("mov w12, #0")
    e("mov w13, #4")
    e("mov w14, #8")
    e("mov w15, #12")

    # Load IV[0..3] ONCE into ZA2 rows 0..3.
    for i in range(4):
        e(f"mov w8, #{IV[i] & 0xffff}")
        e(f"movk w8, #{IV[i] >> 16}, lsl #16")
        e(f"dup z{16 + i}.s, w8")
    e("mov za2h.s[w12, 0:3], { z16.s - z19.s }")

    # Load key[0..7] ONCE into ZA3 rows 0..7.
    for i in range(8):
        e(f"ldr w8, [x1, #{4 * i}]")
        e(f"dup z{16 + i}.s, w8")
    e("mov za3h.s[w12, 0:3], { z16.s - z19.s }")
    e("mov za3h.s[w13, 0:3], { z20.s - z23.s }")

    # Load first group's 16 messages directly into ZA0.S.
    for r_idx in range(16):
        L.extend(emit_load_row(r_idx, nbytes))

    loop_lbl = f"L_{name}_loop"
    last_lbl = f"L_{name}_last"
    ff_lbl = f"L_{name}_ff"
    e(f"{loop_lbl}:")

    # Restore key, IV, counter=0, block_len=nbytes, flags (6 instructions total!).
    e("mov { z0.s - z3.s }, za3h.s[w12, 0:3]")
    e("mov { z4.s - z7.s }, za3h.s[w13, 0:3]")
    e("mov { z8.s - z11.s }, za2h.s[w12, 0:3]")
    e("mov z12.s, #0")
    e("mov z13.s, #0")
    e(f"mov z14.s, #{nbytes}")
    e("dup z15.s, w2")

    # Read transposed message words vertically from ZA0.S (1, 2, or 3 instructions!).
    e("mov { z16.s - z19.s }, za0v.s[w12, 0:3]")
    if nwords >= 8:
        e("mov { z20.s - z23.s }, za0v.s[w13, 0:3]")
    if nwords >= 12:
        e("mov { z24.s - z27.s }, za0v.s[w14, 0:3]")

    e(f"add x0, x0, #{16 * nbytes}")
    e("cmp x4, #1")
    e(f"b.eq {last_lbl}")

    # Pipelined 7 rounds: load next group's 16 rows (2 rows per half-round across rounds 0..3).
    row_cursor = 0
    for r in range(7):
        sched = SCHEDULE[r]
        for quads, base in ((COLS, 0), (DIAGS, 8)):
            pf = []
            if row_cursor < 16:
                pf = [(row_cursor, nbytes), (row_cursor + 1, nbytes)]
                row_cursor += 2
            L.extend(emit_half_round(quads, sched, base, nwords, pf))
    e(f"b {ff_lbl}")

    # Last group: 7 rounds without prefetching past end of input.
    e(f"{last_lbl}:")
    for r in range(7):
        sched = SCHEDULE[r]
        for quads, base in ((COLS, 0), (DIAGS, 8)):
            L.extend(emit_half_round(quads, sched, base, nwords))

    e(f"{ff_lbl}:")
    for i in range(8):
        e(f"eor z{i}.d, z{i}.d, z{8 + i}.d")

    e("mov za1v.s[w12, 0:3], { z0.s - z3.s }")
    e("mov za1v.s[w13, 0:3], { z4.s - z7.s }")
    for r_idx in range(16):
        sel = ZA_SELS[r_idx // 4]
        sl = r_idx % 4
        off = 32 * r_idx
        if off == 0:
            e(f"st1w {{ za1h.s[{sel}, {sl}] }}, p2, [x3]")
        else:
            e(f"add x8, x3, #{off}")
            e(f"st1w {{ za1h.s[{sel}, {sl}] }}, p2, [x8]")

    e("add x3, x3, #512")
    e("subs x4, x4, #1")
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

    e(f"L_{name}_early:")
    e("mov x0, x8")
    e("ret")
    e(f"{sym}_end:")
    return "\n".join(L)


def main():
    out_dir = Path("examples")
    for suffix, prefix in (("elf", ""), ("apple", "_")):
        parts = [".arch armv9-a+sme2", ".text"]
        for n in (16, 32, 48):
            parts.append(emit_kernel(f"probe_sme2_packed{n}", n, prefix))
        parts.append(f".globl {prefix}probe_sme2_text_end\n{prefix}probe_sme2_text_end:")
        (out_dir / f"sme2_packed_{suffix}.S").write_text("\n\n".join(parts) + "\n")
    print("Generated examples/sme2_packed_{elf,apple}.S")


if __name__ == "__main__":
    main()
