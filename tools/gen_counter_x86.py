#!/usr/bin/env python3
"""Generate zero-word-pruned x86-64 AVX-512 (16 lanes/group) and AVX2
(8 lanes/group) counter-range (`32 B` / `16 B`) and early-exit `find` kernels
(#511, #585).

System V x86-64 ABI (`counter`):
  fn(
      template: *const u32,  // rdi: 16 x u32 zero-padded 64-byte template
      groups: u64,           // rsi: number of groups (0 returns immediately)
      key: *const u32,       // rdx: 8 x u32 key words
      base_and_word: u64,    // rcx: (base as u32) | ((counter_word as u64) << 32)
      flags_and_len: u64,    // r8:  flags_u8 | (block_len_u8 << 8) | (trunc16_bit << 16)
      out: *mut u8,          // r9:  (32 or 16) * lanes * groups bytes
  )

System V x86-64 ABI (`find`):
  fn(
      template: *const u32,  // rdi: 16 x u32 zero-padded 64-byte template
      groups: u64,           // rsi: max groups (0 returns 0)
      key: *const u32,       // rdx: 8 x u32 key words
      base_and_word: u64,    // rcx: (base as u32) | ((counter_word as u64) << 32)
      flags_and_len: u64,    // r8:  flags_u8 | (block_len_u8 << 8)
      mask_and_target: u64,  // r9:  (mask as u32) | ((target as u64) << 32)
  ) -> u64                   // (groups_done << lanes) | hit_mask
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


def emit_avx512(name, n_words, mode):
    assert mode in ("counter", "find")
    L = [
        f".globl {name}",
        f".hidden {name}",
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
    if mode == "find":
        e("xor eax, eax")
    e("test rsi, rsi")
    e(f"jz {done_lbl}")

    # Build zmm0 = [base + lane_offset[i]]:
    # - counter mode uses 4*(i%4)+(i//4) to match the 4x4 output transpose.
    # - find mode uses linear 0..15 so vpcmpeqd k1 gives the linear 16-bit mask.
    e("sub rsp, 64")
    L.append(".cfi_adjust_cfa_offset 64")
    for i in range(16):
        off = (4 * (i % 4) + (i // 4)) if mode == "counter" else i
        e(f"mov dword ptr [rsp + {4 * i}], {off}")
    e("vmovdqu32 zmm0, zmmword ptr [rsp]")
    e("add rsp, 64")
    L.append(".cfi_adjust_cfa_offset -64")
    e("vpbroadcastd zmm1, ecx")
    e("vpaddd zmm0, zmm0, zmm1")

    # Unpack scalar parameters:
    #   r10  = counter_word (0..n_words-1)
    #   r11d = flags (bits 0..7)
    #   r8d  = block_len (bits 8..15)
    #   rcx  = trunc16 bit (counter mode) OR groups_done (find mode)
    e("mov r10, rcx")
    e("shr r10, 32")
    if mode == "counter":
        e("mov ecx, r8d")
        e("shr ecx, 16")
        e("and ecx, 1")
    else:
        e("xor ecx, ecx")
    e("mov r11d, r8d")
    e("and r11d, 0xff")
    e("shr r8d, 8")
    e("and r8d, 0xff")

    # Broadcast active static template words 0..n_words-1 once into zmm16..zmm(16+n_words-1).
    for w in range(n_words):
        e(f"vpbroadcastd zmm{16 + w}, dword ptr [rdi + {4 * w}]")

    # Place initial counters into zmm(16 + counter_word) once.
    for w in range(n_words - 1):
        e(f"cmp r10, {w}")
        e(f"je .L_{name}_set_{w}")
    L.append(f".L_{name}_set_{n_words - 1}:")
    e(f"vmovdqa32 zmm{16 + n_words - 1}, zmm0")
    e(f"jmp {init_done_lbl}")
    for w in range(n_words - 1):
        L.append(f".L_{name}_set_{w}:")
        e(f"vmovdqa32 zmm{16 + w}, zmm0")
        e(f"jmp {init_done_lbl}")
    L.append(f"{init_done_lbl}:")

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
                if mx < n_words:
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
                if my < n_words:
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

    if mode == "counter":
        for i in range(4):
            e(f"vpxord zmm{i}, zmm{i}, zmm{8 + i}")
        e("test ecx, ecx")
        e(f"jnz .L_{name}_store16")
        for i in range(4, 8):
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
        e(f"jmp .L_{name}_next")

        L.append(f".L_{name}_store16:")
        L.extend(transpose4x4("zmm0", "zmm1", "zmm2", "zmm3", "zmm8", "zmm9", "zmm10", "zmm11"))
        for p in range(4):
            e(f"vmovdqu32 zmmword ptr [r9 + {64 * p}], zmm{p}")
        e("add r9, 256")

        L.append(f".L_{name}_next:")
        e("dec rsi")
        e(f"jz {vzero_lbl}")
    else:
        # find mode: in-register word0 pre-filter, zero RAM writes!
        e("vpxord zmm0, zmm0, zmm8")
        e("vpbroadcastd zmm1, r9d")
        e("mov rax, r9")
        e("shr rax, 32")
        e("vpbroadcastd zmm2, eax")
        e("vpandd zmm0, zmm0, zmm1")
        e("vpcmpeqd k1, zmm0, zmm2")
        e("kmovw eax, k1")
        e("test eax, eax")
        e(f"jnz .L_{name}_hit")

        e("inc rcx")
        e("cmp rcx, rsi")
        e(f"je .L_{name}_miss_done")

    # Increment only zmm(16 + counter_word) by +16.
    e("mov eax, 16")
    e("vpbroadcastd zmm0, eax")
    for w in range(n_words - 1):
        e(f"cmp r10, {w}")
        e(f"je .L_{name}_inc_{w}")
    L.append(f".L_{name}_inc_{n_words - 1}:")
    e(f"vpaddd zmm{16 + n_words - 1}, zmm{16 + n_words - 1}, zmm0")
    e(f"jmp {loop_lbl}")
    for w in range(n_words - 1):
        L.append(f".L_{name}_inc_{w}:")
        e(f"vpaddd zmm{16 + w}, zmm{16 + w}, zmm0")
        e(f"jmp {loop_lbl}")

    if mode == "find":
        L.append(f".L_{name}_miss_done:")
        e("mov rax, rcx")
        e("shl rax, 16")
        e(f"jmp {vzero_lbl}")
        L.append(f".L_{name}_hit:")
        e("shl rcx, 16")
        e("or rax, rcx")

    L.append(f"{vzero_lbl}:")
    e("vzeroupper")
    L.append(f"{done_lbl}:")
    e("ret")
    L.append(".cfi_endproc")
    L.append(f".size {name}, .-{name}")
    return "\n".join(L)


def emit_avx2(name, n_words, mode):
    assert mode in ("counter", "find")
    L = [
        f".globl {name}",
        f".hidden {name}",
        f".type {name}, @function",
        ".p2align 6",
        f"{name}:",
        ".cfi_startproc",
    ]
    e = lambda s: L.append(f"    {s}")

    done_lbl = f".L_{name}_done"
    loop_lbl = f".L_{name}_loop"

    e("endbr64")
    if mode == "find":
        e("xor eax, eax")
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

    e("mov eax, 8")
    e("vmovd xmm0, eax")
    e("vpbroadcastd ymm0, xmm0")
    e(f"vmovdqa ymmword ptr [rsp + {F_EIGHT}], ymm0")

    for i in range(8):
        off = (2 * (i % 4) + (i // 4)) if mode == "counter" else i
        e(f"mov dword ptr [rsp + {F_SCR + 4 * i}], {off}")
    e(f"vmovdqa ymm0, ymmword ptr [rsp + {F_SCR}]")
    e("vmovd xmm1, ecx")
    e("vpbroadcastd ymm1, xmm1")
    e("vpaddd ymm0, ymm0, ymm1")
    e(f"vmovdqa ymmword ptr [rsp + {F_CTR}], ymm0")

    e("mov r10, rcx")
    e("shr r10, 32")
    e("shl r10, 5")
    if mode == "counter":
        e("mov ecx, r8d")
        e("shr ecx, 16")
        e("and ecx, 1")
    else:
        e("xor ecx, ecx")
    e("mov r11d, r8d")
    e("and r11d, 0xff")
    e("shr r8d, 8")
    e("and r8d, 0xff")

    for w in range(n_words):
        e(f"vpbroadcastd ymm0, dword ptr [rdi + {4 * w}]")
        e(f"vmovdqa ymmword ptr [rsp + {F_MSG + 32 * w}], ymm0")

    L.append(f"{loop_lbl}:")

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
                if mx < n_words:
                    e(f"vpaddd {a}, {a}, ymmword ptr [rsp + {F_MSG + 32 * mx}]")
                e(f"vpaddd {a}, {a}, {b}")
                e(f"vpxor ymm12, ymm12, {a}")
                e("vpshufb ymm12, ymm12, ymm13")
                e(f"vpaddd {c}, {c}, ymm12")
                e(f"vpxor {b}, {b}, {c}")
                e(f"vpsrld ymm15, {b}, 12")
                e(f"vpslld {b}, {b}, 20")
                e(f"vpor {b}, {b}, ymm15")
                if my < n_words:
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

    if mode == "counter":
        for i in range(4):
            e(f"vpxor ymm{i}, ymm{i}, ymm{8 + i}")
        e("test ecx, ecx")
        e(f"jnz .L_{name}_store16")
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
        e(f"jmp .L_{name}_next")

        L.append(f".L_{name}_store16:")
        L.extend(transpose4x4("ymm0", "ymm1", "ymm2", "ymm3", "ymm8", "ymm9", "ymm10", "ymm11"))
        for p in range(4):
            e(f"vmovdqu ymmword ptr [r9 + {32 * p}], ymm{p}")
        e("add r9, 128")

        L.append(f".L_{name}_next:")
        e("dec rsi")
        e(f"jnz {loop_lbl}")
    else:
        e("vpxor ymm0, ymm0, ymm8")
        e("vmovd xmm1, r9d")
        e("vpbroadcastd ymm1, xmm1")
        e("mov rax, r9")
        e("shr rax, 32")
        e("vmovd xmm2, eax")
        e("vpbroadcastd ymm2, xmm2")
        e("vpand ymm0, ymm0, ymm1")
        e("vpcmpeqd ymm0, ymm0, ymm2")
        e("vmovmskps eax, ymm0")
        e("test eax, eax")
        e(f"jnz .L_{name}_hit")

        e("inc rcx")
        e("cmp rcx, rsi")
        e(f"jb {loop_lbl}")

        e("mov rax, rcx")
        e("shl rax, 8")
        e(f"jmp .L_{name}_epilogue")

        L.append(f".L_{name}_hit:")
        e("shl rcx, 8")
        e("or rax, rcx")
        L.append(f".L_{name}_epilogue:")

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
        "// Generated by tools/gen_counter_x86.py; zero-word-pruned AVX-512 & AVX2 counter & find kernels (#511, #585).",
        ".intel_syntax noprefix\n.text",
    ]
    for w in (4, 9, 13, 16):
        parts.append(emit_avx512(f"blake3_servil_avx512_counter_w{w}", w, "counter"))
    for w in (4, 9, 13, 16):
        parts.append(emit_avx512(f"blake3_servil_avx512_find_w{w}", w, "find"))
    for w in (4, 9, 16):
        parts.append(emit_avx2(f"blake3_servil_avx2_counter_w{w}", w, "counter"))
    for w in (4, 9, 16):
        parts.append(emit_avx2(f"blake3_servil_avx2_find_w{w}", w, "find"))
    parts.append('.section .note.GNU-stack,"",@progbits')
    print("\n\n".join(parts))


if __name__ == "__main__":
    main()
