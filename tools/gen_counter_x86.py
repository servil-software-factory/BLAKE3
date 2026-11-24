#!/usr/bin/env python3
"""Generate register-only x86-64 AVX-512 (16 lanes/group) and AVX2 (8 lanes/group)
universal <= 64 B counter-grinding kernels (#511).

System V x86-64 ABI (both kernels):
  fn(
      template: *const u32,  // rdi: 16 x u32 zero-padded 64-byte template
      groups: u64,           // rsi: number of groups (0 returns immediately)
      key: *const u32,       // rdx: 8 x u32 key words
      base_and_word: u64,    // rcx: (base as u32) | ((counter_word as u64) << 32)
      flags_and_len: u64,    // r8:  (flags & 0xff) | ((block_len & 0xff) << 8)
      out: *mut u8,          // r9:  32 * lanes * groups bytes
  )

AVX-512 holds all 16 broadcast message words in zmm16..zmm31 across the entire
group loop (zero message memory reads inside the loop, zero loop stack frame,
incrementing only zmm(16 + counter_word) by +16 per group).
AVX2 broadcasts the 16 template words once into an L1-resident 512 B stack array
and updates only [rsp + counter_word * 32] by +8 per group.
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

F_MSG = 0      # 16 * 32 = 512 B
F_D = 512      # 4 * 32 = 128 B (d row v12..v15)
F_CTR = 640    # 32 B running counter vector
F_EIGHT = 672  # 32 B constant [8, 8, ..., 8]
F_SCR = 704    # 32 B scratch for ROT16/ROT8 mask construction
FRAME = 736

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
        f".globl {name}",
        f".type {name}, @function",
        ".p2align 6",
        f"{name}:",
        ".cfi_startproc",
    ]
    e = lambda s: L.append(f"    {s}")

    done_lbl = f".L_{name}_done"
    vzero_lbl = f".L_{name}_vzero"
    loop_lbl = f".L_{name}_loop"
    init_done_lbl = f".L_{name}_init_done"

    e("endbr64")
    e("test rsi, rsi")
    e(f"jz {done_lbl}")

    # Build zmm0 = [base + perm[i]] once before the loop, where
    # perm[i] = 4*(i%4) + (i//4) matches the 4x4 output transpose so digests
    # land in linear counter order 0..15.
    e("sub rsp, 64")
    L.append(".cfi_adjust_cfa_offset 64")
    for i in range(16):
        e(f"mov dword ptr [rsp + {4 * i}], {4 * (i % 4) + (i // 4)}")
    e("vmovdqu32 zmm0, zmmword ptr [rsp]")
    e("add rsp, 64")
    L.append(".cfi_adjust_cfa_offset -64")
    e("vpbroadcastd zmm1, ecx")
    e("vpaddd zmm0, zmm0, zmm1")

    # Unpack scalar parameters:
    #   r10  = counter_word (0..15)
    #   r11d = flags (bits 0..7)
    #   r8d  = block_len (bits 8..15)
    e("mov r10, rcx")
    e("shr r10, 32")
    e("mov r11d, r8d")
    e("and r11d, 0xff")
    e("shr r8d, 8")
    e("and r8d, 0xff")

    # Broadcast all 16 static template words once into zmm16..zmm31.
    for w in range(16):
        e(f"vpbroadcastd zmm{16 + w}, dword ptr [rdi + {4 * w}]")

    # Place initial [base..base+15] into zmm(16 + counter_word) once.
    for w in range(15):
        e(f"cmp r10, {w}")
        e(f"je .L_{name}_set_{w}")
    L.append(f".L_{name}_set_15:")
    e("vmovdqa32 zmm31, zmm0")
    e(f"jmp {init_done_lbl}")
    for w in range(15):
        L.append(f".L_{name}_set_{w}:")
        e(f"vmovdqa32 zmm{16 + w}, zmm0")
        e(f"jmp {init_done_lbl}")
    L.append(f"{init_done_lbl}:")

    # Hot group loop: zero message memory reads, zero stack traffic.
    L.append(f"{loop_lbl}:")

    for i in range(8):
        e(f"vpbroadcastd zmm{i}, dword ptr [rdx + {4 * i}]")
    for i in range(4):
        e(f"mov eax, {IV[i]}")
        e(f"vpbroadcastd zmm{8 + i}, eax")
    e("vpxord zmm12, zmm12, zmm12")
    e("vpxord zmm13, zmm13, zmm13")
    e("vpbroadcastd zmm14, r8d")
    e("vpbroadcastd zmm15, r11d")

    for r in range(7):
        sched = SCHEDULE[r]
        for quads, base in ((COLS, 0), (DIAGS, 8)):
            cols = [(f"zmm{ai}", f"zmm{bi}", f"zmm{ci}", f"zmm{di}", sched[base + k], sched[base + k + 1])
                    for (ai, bi, ci, di), k in zip(quads, range(0, 8, 2))]
            for a, _, _, _, mx, _ in cols:
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

    # Feed-forward and output transpose using dead zmm8..zmm11 (zmm16..zmm31 intact).
    for i in range(8):
        e(f"vpxord zmm{i}, zmm{i}, zmm{8 + i}")
    L.extend(transpose4x4("zmm0", "zmm1", "zmm2", "zmm3", "zmm8", "zmm9", "zmm10", "zmm11"))
    L.extend(transpose4x4("zmm4", "zmm5", "zmm6", "zmm7", "zmm8", "zmm9", "zmm10", "zmm11"))
    for p in range(4):
        e(f"vshufi32x4 zmm8, zmm{p}, zmm{4 + p}, 0x44")
        e("vshufi32x4 zmm8, zmm8, zmm8, 0xd8")
        e(f"vmovdqu32 zmmword ptr [r9 + {128 * p}], zmm8")
        e(f"vshufi32x4 zmm9, zmm{p}, zmm{4 + p}, 0xee")
        e("vshufi32x4 zmm9, zmm9, zmm9, 0xd8")
        e(f"vmovdqu32 zmmword ptr [r9 + {128 * p + 64}], zmm9")

    e("add r9, 512")
    e("dec rsi")
    e(f"jz {vzero_lbl}")

    # Increment only zmm(16 + counter_word) by +16 (constant predicted branch).
    e("mov eax, 16")
    e("vpbroadcastd zmm0, eax")
    for w in range(15):
        e(f"cmp r10, {w}")
        e(f"je .L_{name}_inc_{w}")
    L.append(f".L_{name}_inc_15:")
    e("vpaddd zmm31, zmm31, zmm0")
    e(f"jmp {loop_lbl}")
    for w in range(15):
        L.append(f".L_{name}_inc_{w}:")
        e(f"vpaddd zmm{16 + w}, zmm{16 + w}, zmm0")
        e(f"jmp {loop_lbl}")

    L.append(f"{vzero_lbl}:")
    e("vzeroupper")
    L.append(f"{done_lbl}:")
    e("ret")
    L.append(".cfi_endproc")
    L.append(f".size {name}, .-{name}")
    return "\n".join(L)


def emit_avx2(name):
    L = [
        f".globl {name}",
        f".type {name}, @function",
        ".p2align 6",
        f"{name}:",
        ".cfi_startproc",
    ]
    e = lambda s: L.append(f"    {s}")

    done_lbl = f".L_{name}_done"
    loop_lbl = f".L_{name}_loop"

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

    # Build ROT16 (ymm13) and ROT8 (ymm14) shuffle masks once before the loop.
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

    # Build constant [8, 8, ..., 8] at F_EIGHT.
    e("mov eax, 8")
    e("vmovd xmm0, eax")
    e("vpbroadcastd ymm0, xmm0")
    e(f"vmovdqa ymmword ptr [rsp + {F_EIGHT}], ymm0")

    # Build initial running counter vector [base + perm[i]] at F_CTR, where
    # perm[i] = 2*(i%4) + (i//4) matches the 4x4 AVX2 output transpose so
    # digests land in linear counter order 0..7.
    for i in range(8):
        e(f"mov dword ptr [rsp + {F_SCR + 4 * i}], {2 * (i % 4) + (i // 4)}")
    e(f"vmovdqa ymm0, ymmword ptr [rsp + {F_SCR}]")
    e("vmovd xmm1, ecx")
    e("vpbroadcastd ymm1, xmm1")
    e("vpaddd ymm0, ymm0, ymm1")
    e(f"vmovdqa ymmword ptr [rsp + {F_CTR}], ymm0")

    # Unpack scalar parameters:
    #   r10  = counter_word * 32 (byte offset into F_MSG)
    #   r11d = flags (bits 0..7)
    #   r8d  = block_len (bits 8..15)
    e("mov r10, rcx")
    e("shr r10, 32")
    e("shl r10, 5")
    e("mov r11d, r8d")
    e("and r11d, 0xff")
    e("shr r8d, 8")
    e("and r8d, 0xff")

    # Broadcast all 16 static template words once into L1 stack array [rsp + F_MSG..].
    for w in range(16):
        e(f"vpbroadcastd ymm0, dword ptr [rdi + {4 * w}]")
        e(f"vmovdqa ymmword ptr [rsp + {F_MSG + 32 * w}], ymm0")

    L.append(f"{loop_lbl}:")

    # Stamp only [rsp + r10] with current counter vector and advance by +8.
    e(f"vmovdqa ymm0, ymmword ptr [rsp + {F_CTR}]")
    e("vmovdqa ymmword ptr [rsp + r10], ymm0")
    e(f"vpaddd ymm0, ymm0, ymmword ptr [rsp + {F_EIGHT}]")
    e(f"vmovdqa ymmword ptr [rsp + {F_CTR}], ymm0")

    for i in range(8):
        e(f"vpbroadcastd ymm{i}, dword ptr [rdx + {4 * i}]")
    for i in range(4):
        e(f"mov eax, {IV[i]}")
        e("vmovd xmm12, eax")
        e(f"vpbroadcastd ymm{8 + i}, xmm12")
    e("vpxor ymm12, ymm12, ymm12")
    e(f"vmovdqa ymmword ptr [rsp + {F_D}], ymm12")
    e(f"vmovdqa ymmword ptr [rsp + {F_D + 32}], ymm12")
    e("vmovd xmm12, r8d")
    e("vpbroadcastd ymm12, xmm12")
    e(f"vmovdqa ymmword ptr [rsp + {F_D + 64}], ymm12")
    e("vmovd xmm12, r11d")
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
                e(f"vpaddd {a}, {a}, ymmword ptr [rsp + {F_MSG + 32 * mx}]")
                e(f"vpaddd {a}, {a}, {b}")
                e(f"vpxor ymm12, ymm12, {a}")
                e("vpshufb ymm12, ymm12, ymm13")
                e(f"vpaddd {c}, {c}, ymm12")
                e(f"vpxor {b}, {b}, {c}")
                e(f"vpsrld ymm15, {b}, 12")
                e(f"vpslld {b}, {b}, 20")
                e(f"vpor {b}, {b}, ymm15")
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
        e(f"vmovdqu ymmword ptr [r9 + {64 * p}], ymm8")
        e(f"vmovdqu ymmword ptr [r9 + {64 * p + 32}], ymm9")

    e("add r9, 256")
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
        "// Generated by tools/gen_counter_x86.py; register-only counter-range contract in its header.",
        ".intel_syntax noprefix\n.text",
        emit_avx512("blake3_servil_avx512_counter"),
        emit_avx2("blake3_servil_avx2_counter"),
        '.section .note.GNU-stack,"",@progbits',
    ]
    print("\n\n".join(parts))


if __name__ == "__main__":
    main()
