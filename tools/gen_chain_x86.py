#!/usr/bin/env python3
"""Generate c/blake3_chain_x86-64_unix.S: x86-64 AVX-512 (16-lane, 0-byte stack)
and AVX2 (8-lane) fused multi-step 32-byte hash-chain kernels (#584).

Each 32-byte chain slot holds `[prefix12 || start_step_u32_le || state16]`.
For each step `s in 0..steps`:

    state_{s+1} = BLAKE3_compress(key, [prefix12, start_step + s, state_s],
                                  len=32, counter=0, flags)[..16]

System V x86-64 ABI (both kernels):
  fn(
      init: *const u8,   // rdi: lanes * groups * 32 bytes
      groups: u64,       // rsi: number of groups (0 returns early)
      key: *const u32,   // rdx: 8 x u32 key words
      flags: u32,        // ecx: root block flags
      out: *mut u8,      // r8:  lanes * groups * 16 bytes
      steps: u32,        // r9d: number of iterations (0 returns early)
  )

    python3 tools/gen_chain_x86.py > c/blake3_chain_x86-64_unix.S
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

F_MSG = 0      # 8 x 32 B = 256 B
F_D = 256      # 4 x 32 B = 128 B
F_ONE = 384    # 32 B ([1u32; 8])
F_SCR = 416    # 32 B
FRAME = 448

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


def emit_avx512(name):
    L = [
        ".p2align 6",
        f".global {name}",
        f".hidden {name}",
        f".type {name}, @function",
        f"{name}:",
        ".cfi_startproc",
    ]
    e = lambda s: L.append(f"    {s}")

    group_lbl = f".L_{name}_group"
    step_lbl = f".L_{name}_step"
    done_lbl = f".L_{name}_done"

    e("endbr64")
    e("test rsi, rsi")
    e(f"jz {done_lbl}")
    e("test r9d, r9d")
    e(f"jz {done_lbl}")

    e("mov eax, 1")
    e("vpbroadcastd zmm24, eax")

    L.append(".p2align 5")
    L.append(f"{group_lbl}:")

    for p in range(4):
        e(f"vmovdqu32 zmm0, zmmword ptr [rdi + {128 * p}]")
        e(f"vmovdqu32 zmm1, zmmword ptr [rdi + {128 * p + 64}]")
        e(f"vshufi32x4 zmm{16 + p}, zmm0, zmm1, 0x88")
        e(f"vshufi32x4 zmm{20 + p}, zmm0, zmm1, 0xdd")
    L.extend(transpose4x4("zmm16", "zmm17", "zmm18", "zmm19", "zmm28", "zmm29", "zmm30", "zmm31"))
    L.extend(transpose4x4("zmm20", "zmm21", "zmm22", "zmm23", "zmm28", "zmm29", "zmm30", "zmm31"))

    e("mov r10d, r9d")

    L.append(".p2align 5")
    L.append(f"{step_lbl}:")

    for i in range(8):
        e(f"vpbroadcastd zmm{i}, dword ptr [rdx + {4 * i}]")
    for i in range(4):
        e(f"mov eax, {IV[i]}")
        e(f"vpbroadcastd zmm{8 + i}, eax")
    e("vpxord zmm12, zmm12, zmm12")
    e("vpxord zmm13, zmm13, zmm13")
    e("mov eax, 32")
    e("vpbroadcastd zmm14, eax")
    e("vpbroadcastd zmm15, ecx")

    for r in range(7):
        sched = SCHEDULE[r]
        for quads, base in ((COLS, 0), (DIAGS, 8)):
            cols = [(f"zmm{ai}", f"zmm{bi}", f"zmm{ci}", f"zmm{di}", sched[base + k], sched[base + k + 1])
                    for (ai, bi, ci, di), k in zip(quads, range(0, 8, 2))]
            for a, _, _, _, mx, _ in cols:
                if mx < 8:
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
                if my < 8:
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

    for i in range(4):
        e(f"vpxord zmm{20 + i}, zmm{i}, zmm{8 + i}")
    e("vpaddd zmm19, zmm19, zmm24")
    e("dec r10d")
    e(f"jnz {step_lbl}")

    # Transpose zmm20..zmm23 -> 4 x 64 B contiguous 16-byte digests (256 B total).
    L.extend(transpose4x4("zmm20", "zmm21", "zmm22", "zmm23", "zmm28", "zmm29", "zmm30", "zmm31"))
    for p in range(4):
        e(f"vmovdqu32 zmmword ptr [r8 + {64 * p}], zmm{20 + p}")

    e("add rdi, 512")
    e("add r8, 256")
    e("dec rsi")
    e(f"jnz {group_lbl}")
    e("vzeroupper")
    L.append(f"{done_lbl}:")
    e("ret")
    L.append(".cfi_endproc")
    L.append(f".size {name}, .-{name}")
    return "\n".join(L)


def emit_avx2(name):
    L = [
        ".p2align 6",
        f".global {name}",
        f".hidden {name}",
        f".type {name}, @function",
        f"{name}:",
        ".cfi_startproc",
    ]
    e = lambda s: L.append(f"    {s}")

    group_lbl = f".L_{name}_group"
    step_lbl = f".L_{name}_step"
    done_lbl = f".L_{name}_done"

    e("endbr64")
    e("test rsi, rsi")
    e(f"jz {done_lbl}")
    e("test r9d, r9d")
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

    e("mov eax, 1")
    e("vmovd xmm12, eax")
    e("vpbroadcastd ymm12, xmm12")
    e(f"vmovdqa ymmword ptr [rsp + {F_ONE}], ymm12")

    L.append(".p2align 5")
    L.append(f"{group_lbl}:")

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

    e("mov r10d, r9d")

    L.append(".p2align 5")
    L.append(f"{step_lbl}:")

    for i in range(8):
        e(f"vpbroadcastd ymm{i}, dword ptr [rdx + {4 * i}]")
    for i in range(4):
        e(f"mov eax, {IV[i]}")
        e("vmovd xmm12, eax")
        e(f"vpbroadcastd ymm{8 + i}, xmm12")
    e("vpxor ymm12, ymm12, ymm12")
    e(f"vmovdqa ymmword ptr [rsp + {F_D}], ymm12")
    e(f"vmovdqa ymmword ptr [rsp + {F_D + 32}], ymm12")
    e("mov eax, 32")
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
                if mx < 8:
                    e(f"vpaddd {a}, {a}, ymmword ptr [rsp + {F_MSG + 32 * mx}]")
                e(f"vpaddd {a}, {a}, {b}")
                e(f"vpxor ymm12, ymm12, {a}")
                e("vpshufb ymm12, ymm12, ymm13")
                e(f"vpaddd {c}, {c}, ymm12")
                e(f"vpxor {b}, {b}, {c}")
                e(f"vpsrld ymm15, {b}, 12")
                e(f"vpslld {b}, {b}, 20")
                e(f"vpor {b}, {b}, ymm15")
                if my < 8:
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
        e(f"vmovdqa ymmword ptr [rsp + {F_MSG + 32 * (4 + i)}], ymm{i}")
    e(f"vmovdqa ymm12, ymmword ptr [rsp + {F_MSG + 96}]")
    e(f"vpaddd ymm12, ymm12, ymmword ptr [rsp + {F_ONE}]")
    e(f"vmovdqa ymmword ptr [rsp + {F_MSG + 96}], ymm12")
    e("dec r10d")
    e(f"jnz {step_lbl}")

    # ymm0..ymm3 ALREADY hold the final 4 state words after the last step!
    L.extend(transpose4x4("ymm0", "ymm1", "ymm2", "ymm3", "ymm8", "ymm9", "ymm10", "ymm11"))
    for p in range(4):
        e(f"vmovdqu ymmword ptr [r8 + {32 * p}], ymm{p}")

    e("add rdi, 256")
    e("add r8, 128")
    e("dec rsi")
    e(f"jnz {group_lbl}")

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
        "// Generated by tools/gen_chain_x86.py; AVX-512/AVX2 fused hash-chain contract in its header.",
        ".intel_syntax noprefix\n.text",
        emit_avx512("blake3_servil_avx512_chain32_trunc16"),
        emit_avx2("blake3_servil_avx2_chain32_trunc16"),
        '.section .note.GNU-stack,"",@progbits',
    ]
    print("\n\n".join(parts))


if __name__ == "__main__":
    main()
