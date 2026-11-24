#!/usr/bin/env python3
"""Generate direct-load x86-64 AVX-512 and AVX2 kernels for Packed<16>,
Packed<32>, and Packed<48> (#501).

Every kernel hashes contiguous packed one-block messages directly from input
memory with zero stack expansion, zero pointer table, and zero-word message
adds omitted at generation time.

System V x86-64 ABI (all six kernels):
  fn(input: *const u8, groups: u64, key: *const u32, flags: u32, out: *mut u8)
     rdi               rsi          rdx              ecx         r8
  - blake3_servil_avx512_packed{16,32,48}: 16 messages/group, 0-byte stack frame
  - blake3_servil_avx2_packed{16,32,48}:    8 messages/group, 544-byte frame
  Requires groups >= 1 (returns immediately if groups == 0).
  `flags` already carries CHUNK_START | CHUNK_END | ROOT | mode_flags.
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

F_MSG = 0
F_D = 384
F_SCR = 512
FRAME = 544

ROT16_LO = 0x0504070601000302
ROT16_HI = 0x0D0C0F0E09080B0A
ROT8_LO = 0x0407060500030201
ROT8_HI = 0x0C0F0E0D080B0A09


def transpose4x4(a, b, c, d, t0, t1, t2, t3):
    return [
        f"    vpunpckldq {t0}, {a}, {b}",
        f"    vpunpckhdq {t1}, {a}, {b}",
        f"    vpunpckldq {t2}, {c}, {d}",
        f"    vpunpckhdq {t3}, {c}, {d}",
        f"    vpunpcklqdq {a}, {t0}, {t2}",
        f"    vpunpckhqdq {b}, {t0}, {t2}",
        f"    vpunpcklqdq {c}, {t1}, {t3}",
        f"    vpunpckhqdq {d}, {t1}, {t3}",
    ]


def emit_avx512(name, nbytes):
    nwords = nbytes // 4
    L = [
        ".p2align 6",
        f".global {name}",
        f".hidden {name}",
        f".type {name}, @function",
        f"{name}:",
        ".cfi_startproc",
    ]
    e = lambda s: L.append(f"    {s}")

    loop_lbl = f".L_{name}_loop"
    done_lbl = f".L_{name}_done"

    e("endbr64")
    e("test rsi, rsi")
    e(f"jz {done_lbl}")

    L.append(".p2align 5")
    L.append(f"{loop_lbl}:")

    if nbytes == 16:
        for p in range(4):
            e(f"vmovdqu32 zmm{16 + p}, zmmword ptr [rdi + {64 * p}]")
        L.extend(transpose4x4("zmm16", "zmm17", "zmm18", "zmm19", "zmm28", "zmm29", "zmm30", "zmm31"))
    elif nbytes == 32:
        for p in range(4):
            e(f"vmovdqu32 zmm0, zmmword ptr [rdi + {128 * p}]")
            e(f"vmovdqu32 zmm1, zmmword ptr [rdi + {128 * p + 64}]")
            e(f"vshufi32x4 zmm{16 + p}, zmm0, zmm1, 0x88")
            e(f"vshufi32x4 zmm{20 + p}, zmm0, zmm1, 0xdd")
        L.extend(transpose4x4("zmm16", "zmm17", "zmm18", "zmm19", "zmm28", "zmm29", "zmm30", "zmm31"))
        L.extend(transpose4x4("zmm20", "zmm21", "zmm22", "zmm23", "zmm28", "zmm29", "zmm30", "zmm31"))
    elif nbytes == 48:
        for p in range(4):
            base = 192 * p
            e(f"vmovdqu32 zmm0, zmmword ptr [rdi + {base}]")
            e(f"vmovdqu32 zmm1, zmmword ptr [rdi + {base + 64}]")
            e(f"vmovdqu32 zmm2, zmmword ptr [rdi + {base + 128}]")
            e("vshufi32x4 zmm28, zmm1, zmm2, 0x12")
            e(f"vshufi32x4 zmm{16 + p}, zmm0, zmm28, 0x8c")
            e("vshufi32x4 zmm28, zmm0, zmm1, 0x01")
            e("vshufi32x4 zmm29, zmm1, zmm2, 0x23")
            e(f"vshufi32x4 zmm{20 + p}, zmm28, zmm29, 0x88")
            e("vshufi32x4 zmm28, zmm0, zmm1, 0x12")
            e(f"vshufi32x4 zmm{24 + p}, zmm28, zmm2, 0xc8")
        L.extend(transpose4x4("zmm16", "zmm17", "zmm18", "zmm19", "zmm28", "zmm29", "zmm30", "zmm31"))
        L.extend(transpose4x4("zmm20", "zmm21", "zmm22", "zmm23", "zmm28", "zmm29", "zmm30", "zmm31"))
        L.extend(transpose4x4("zmm24", "zmm25", "zmm26", "zmm27", "zmm28", "zmm29", "zmm30", "zmm31"))
    else:
        raise ValueError(nbytes)

    for i in range(8):
        e(f"vpbroadcastd zmm{i}, dword ptr [rdx + {4 * i}]")
    for i in range(4):
        e(f"mov eax, {IV[i]}")
        e(f"vpbroadcastd zmm{8 + i}, eax")
    e("vpxord zmm12, zmm12, zmm12")
    e("vpxord zmm13, zmm13, zmm13")
    e(f"mov eax, {nbytes}")
    e("vpbroadcastd zmm14, eax")
    e("vpbroadcastd zmm15, ecx")

    for r in range(7):
        sched = SCHEDULE[r]
        for quads, base in ((COLS, 0), (DIAGS, 8)):
            cols = [(f"zmm{ai}", f"zmm{bi}", f"zmm{ci}", f"zmm{di}", sched[base + k], sched[base + k + 1])
                    for (ai, bi, ci, di), k in zip(quads, range(0, 8, 2))]
            for a, _, _, _, mx, _ in cols:
                if mx < nwords:
                    e(f"vpaddd {a}, {a}, zmm{16 + mx}")
            for a, b, _, _, _, _ in cols:
                e(f"vpaddd {a}, {a}, {b}")
            for a, _, _, d, _, _ in cols:
                e(f"vpxord {d}, {d}, {a}")
            for _, _, _, d, _, _ in cols:
                e(f"vprord {d}, {d}, 16")
            for _, _, c, d, _, _ in cols:
                e(f"vpaddd {c}, {c}, {d}")
            for _, b, c, _, _, _ in cols:
                e(f"vpxord {b}, {b}, {c}")
            for _, b, _, _, _, _ in cols:
                e(f"vprord {b}, {b}, 12")

            for a, _, _, _, _, my in cols:
                if my < nwords:
                    e(f"vpaddd {a}, {a}, zmm{16 + my}")
            for a, b, _, _, _, _ in cols:
                e(f"vpaddd {a}, {a}, {b}")
            for a, _, _, d, _, _ in cols:
                e(f"vpxord {d}, {d}, {a}")
            for _, _, _, d, _, _ in cols:
                e(f"vprord {d}, {d}, 8")
            for _, _, c, d, _, _ in cols:
                e(f"vpaddd {c}, {c}, {d}")
            for _, b, c, _, _, _ in cols:
                e(f"vpxord {b}, {b}, {c}")
            for _, b, _, _, _, _ in cols:
                e(f"vprord {b}, {b}, 7")

    for i in range(8):
        e(f"vpxord zmm{i}, zmm{i}, zmm{8 + i}")
    L.extend(transpose4x4("zmm0", "zmm1", "zmm2", "zmm3", "zmm28", "zmm29", "zmm30", "zmm31"))
    L.extend(transpose4x4("zmm4", "zmm5", "zmm6", "zmm7", "zmm28", "zmm29", "zmm30", "zmm31"))
    for p in range(4):
        e(f"vshufi32x4 zmm28, zmm{p}, zmm{4 + p}, 0x44")
        e("vshufi32x4 zmm28, zmm28, zmm28, 0xd8")
        e(f"vmovdqu32 zmmword ptr [r8 + {128 * p}], zmm28")
        e(f"vshufi32x4 zmm29, zmm{p}, zmm{4 + p}, 0xee")
        e("vshufi32x4 zmm29, zmm29, zmm29, 0xd8")
        e(f"vmovdqu32 zmmword ptr [r8 + {128 * p + 64}], zmm29")

    e(f"add rdi, {16 * nbytes}")
    e("add r8, 512")
    e("dec rsi")
    e(f"jnz {loop_lbl}")
    e("vzeroupper")
    L.append(f"{done_lbl}:")
    e("ret")
    L.append(".cfi_endproc")
    L.append(f".size {name}, .-{name}")
    return "\n".join(L)


def emit_avx2(name, nbytes):
    nwords = nbytes // 4
    L = [
        ".p2align 6",
        f".global {name}",
        f".hidden {name}",
        f".type {name}, @function",
        f"{name}:",
        ".cfi_startproc",
    ]
    e = lambda s: L.append(f"    {s}")

    loop_lbl = f".L_{name}_loop"
    done_lbl = f".L_{name}_done"

    e("endbr64")
    e("test rsi, rsi")
    e(f"jz {done_lbl}")

    e("push rbp")
    L.append(".cfi_def_cfa_offset 16")
    L.append(".cfi_offset rbp, -16")
    e("mov rbp, rsp")
    L.append(".cfi_def_cfa_register rbp")
    e("and rsp, -32")
    e(f"sub rsp, {FRAME}")

    e(f"movabs rax, {ROT16_LO}")
    e(f"mov qword ptr [rsp + {F_SCR}], rax")
    e(f"mov qword ptr [rsp + {F_SCR + 16}], rax")
    e(f"movabs rax, {ROT16_HI}")
    e(f"mov qword ptr [rsp + {F_SCR + 8}], rax")
    e(f"mov qword ptr [rsp + {F_SCR + 24}], rax")
    e(f"vmovdqa ymm13, ymmword ptr [rsp + {F_SCR}]")

    e(f"movabs rax, {ROT8_LO}")
    e(f"mov qword ptr [rsp + {F_SCR}], rax")
    e(f"mov qword ptr [rsp + {F_SCR + 16}], rax")
    e(f"movabs rax, {ROT8_HI}")
    e(f"mov qword ptr [rsp + {F_SCR + 8}], rax")
    e(f"mov qword ptr [rsp + {F_SCR + 24}], rax")
    e(f"vmovdqa ymm14, ymmword ptr [rsp + {F_SCR}]")

    L.append(".p2align 5")
    L.append(f"{loop_lbl}:")

    if nbytes == 16:
        for p in range(4):
            e(f"vmovdqu ymm{p}, ymmword ptr [rdi + {32 * p}]")
        L.extend(transpose4x4("ymm0", "ymm1", "ymm2", "ymm3", "ymm8", "ymm9", "ymm10", "ymm11"))
        for i in range(4):
            e(f"vmovdqa ymmword ptr [rsp + {F_MSG + 32 * i}], ymm{i}")
    elif nbytes == 32:
        for p in range(4):
            e(f"vmovdqu ymm8, ymmword ptr [rdi + {64 * p}]")
            e(f"vmovdqu ymm9, ymmword ptr [rdi + {64 * p + 32}]")
            e(f"vperm2i128 ymm{p}, ymm8, ymm9, 0x20")
            e(f"vperm2i128 ymm{4 + p}, ymm8, ymm9, 0x31")
        L.extend(transpose4x4("ymm0", "ymm1", "ymm2", "ymm3", "ymm8", "ymm9", "ymm10", "ymm11"))
        for i in range(4):
            e(f"vmovdqa ymmword ptr [rsp + {F_MSG + 32 * i}], ymm{i}")
        L.extend(transpose4x4("ymm4", "ymm5", "ymm6", "ymm7", "ymm8", "ymm9", "ymm10", "ymm11"))
        for i in range(4):
            e(f"vmovdqa ymmword ptr [rsp + {F_MSG + 32 * (4 + i)}], ymm{4 + i}")
    elif nbytes == 48:
        for p in range(4):
            base = 96 * p
            e(f"vmovdqu xmm{p}, xmmword ptr [rdi + {base}]")
            e(f"vinserti128 ymm{p}, ymm{p}, xmmword ptr [rdi + {base + 48}], 1")
            e(f"vmovdqu xmm{4 + p}, xmmword ptr [rdi + {base + 16}]")
            e(f"vinserti128 ymm{4 + p}, ymm{4 + p}, xmmword ptr [rdi + {base + 64}], 1")
        L.extend(transpose4x4("ymm0", "ymm1", "ymm2", "ymm3", "ymm8", "ymm9", "ymm10", "ymm11"))
        for i in range(4):
            e(f"vmovdqa ymmword ptr [rsp + {F_MSG + 32 * i}], ymm{i}")
        L.extend(transpose4x4("ymm4", "ymm5", "ymm6", "ymm7", "ymm8", "ymm9", "ymm10", "ymm11"))
        for i in range(4):
            e(f"vmovdqa ymmword ptr [rsp + {F_MSG + 32 * (4 + i)}], ymm{4 + i}")
        for p in range(4):
            base = 96 * p
            e(f"vmovdqu xmm{p}, xmmword ptr [rdi + {base + 32}]")
            e(f"vinserti128 ymm{p}, ymm{p}, xmmword ptr [rdi + {base + 80}], 1")
        L.extend(transpose4x4("ymm0", "ymm1", "ymm2", "ymm3", "ymm8", "ymm9", "ymm10", "ymm11"))
        for i in range(4):
            e(f"vmovdqa ymmword ptr [rsp + {F_MSG + 32 * (8 + i)}], ymm{i}")
    else:
        raise ValueError(nbytes)

    for i in range(8):
        e(f"vpbroadcastd ymm{i}, dword ptr [rdx + {4 * i}]")
    for i in range(4):
        e(f"mov eax, {IV[i]}")
        e("vmovd xmm12, eax")
        e(f"vpbroadcastd ymm{8 + i}, xmm12")
    e("vpxor ymm12, ymm12, ymm12")
    e(f"vmovdqa ymmword ptr [rsp + {F_D}], ymm12")
    e(f"vmovdqa ymmword ptr [rsp + {F_D + 32}], ymm12")
    e(f"mov eax, {nbytes}")
    e("vmovd xmm12, eax")
    e("vpbroadcastd ymm12, xmm12")
    e(f"vmovdqa ymmword ptr [rsp + {F_D + 64}], ymm12")
    e("vmovd xmm12, ecx")
    e("vpbroadcastd ymm12, xmm12")
    e(f"vmovdqa ymmword ptr [rsp + {F_D + 96}], ymm12")

    for r in range(7):
        sched = SCHEDULE[r]
        for quads, base in ((COLS, 0), (DIAGS, 8)):
            for (ai, bi, ci, di), k in zip(quads, range(0, 8, 2)):
                a, b, c = f"ymm{ai}", f"ymm{bi}", f"ymm{ci}"
                doff = F_D + 32 * (di - 12)
                mx, my = sched[base + k], sched[base + k + 1]
                e(f"vmovdqa ymm12, ymmword ptr [rsp + {doff}]")
                if mx < nwords:
                    e(f"vpaddd {a}, {a}, ymmword ptr [rsp + {F_MSG + 32 * mx}]")
                e(f"vpaddd {a}, {a}, {b}")
                e(f"vpxor ymm12, ymm12, {a}")
                e("vpshufb ymm12, ymm12, ymm13")
                e(f"vpaddd {c}, {c}, ymm12")
                e(f"vpxor {b}, {b}, {c}")
                e(f"vpsrld ymm15, {b}, 12")
                e(f"vpslld {b}, {b}, 20")
                e(f"vpor {b}, {b}, ymm15")
                if my < nwords:
                    e(f"vpaddd {a}, {a}, ymmword ptr [rsp + {F_MSG + 32 * my}]")
                e(f"vpaddd {a}, {a}, {b}")
                e(f"vpxor ymm12, ymm12, {a}")
                e("vpshufb ymm12, ymm12, ymm14")
                e(f"vpaddd {c}, {c}, ymm12")
                e(f"vmovdqa ymmword ptr [rsp + {doff}], ymm12")
                e(f"vpxor {b}, {b}, {c}")
                e(f"vpsrld ymm15, {b}, 7")
                e(f"vpslld {b}, {b}, 25")
                e(f"vpor {b}, {b}, ymm15")

    for i in range(4):
        e(f"vpxor ymm{i}, ymm{i}, ymm{8 + i}")
    for i in range(4):
        e(f"vpxor ymm{4 + i}, ymm{4 + i}, ymmword ptr [rsp + {F_D + 32 * i}]")
    L.extend(transpose4x4("ymm0", "ymm1", "ymm2", "ymm3", "ymm8", "ymm9", "ymm10", "ymm11"))
    L.extend(transpose4x4("ymm4", "ymm5", "ymm6", "ymm7", "ymm8", "ymm9", "ymm10", "ymm11"))
    for p in range(4):
        e(f"vperm2i128 ymm8, ymm{p}, ymm{4 + p}, 0x20")
        e(f"vperm2i128 ymm9, ymm{p}, ymm{4 + p}, 0x31")
        e(f"vmovdqu ymmword ptr [r8 + {64 * p}], ymm8")
        e(f"vmovdqu ymmword ptr [r8 + {64 * p + 32}], ymm9")

    e(f"add rdi, {8 * nbytes}")
    e("add r8, 256")
    e("dec rsi")
    e(f"jnz {loop_lbl}")

    e("vzeroupper")
    e("mov rsp, rbp")
    e("pop rbp")
    L.append(".cfi_def_cfa rsp, 8")
    L.append(".cfi_restore rbp")
    L.append(f"{done_lbl}:")
    e("ret")
    L.append(".cfi_endproc")
    L.append(f".size {name}, .-{name}")
    return "\n".join(L)


def main():
    parts = [
        "// Generated by tools/gen_packed_x86.py; direct-load Packed<16/32/48> contract in its header.",
        ".intel_syntax noprefix\n.text",
    ]
    for n in (16, 32, 48):
        parts.append(emit_avx512(f"blake3_servil_avx512_packed{n}", n))
    for n in (16, 32, 48):
        parts.append(emit_avx2(f"blake3_servil_avx2_packed{n}", n))
    parts.append('.section .note.GNU-stack,"",@progbits')
    print("\n\n".join(parts))


if __name__ == "__main__":
    main()
