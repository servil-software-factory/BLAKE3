#!/usr/bin/env python3
"""Generate register-only x86-64 AVX2 (8 lanes/group) counter-grinding kernels (#511):
  - ZERO message transpose, ZERO caller RAM stamping, ZERO input RAM reads inside
    the group loop!
  - All 16 template words broadcast ONCE into L1-resident stack array [rsp + 0..512]
  - Each group updates ONLY [rsp + counter_word * 32] with running counter vector
    (1 L1 store + 1 vpaddd by +8 per 8 hashes!)
  - One universal kernel (`probe_avx2_counter_range`) handles ANY len 4..=64,
    ANY counter_word 0..=15, ANY wrapping base: u32
  - Also emits 52 B (13-word) and 36 B (9-word) zero-add-pruned variants

System V x86-64 ABI:
  fn(
      template: *const u32,  // rdi: 16 x u32 zero-padded 64-byte template
      groups: u64,           // rsi: number of 8-try groups (>= 1)
      key: *const u32,       // rdx: 8 x u32 key words
      base_and_word: u64,    // rcx: (base as u32) | ((counter_word as u64) << 32)
      flags_and_len: u64,    // r8:  (flags & 0xff) | ((block_len & 0xff) << 8)
      out: *mut u8,          // r9:  256 * groups bytes
  )
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
        f"vpunpckldq {t0}, {a}, {b}",
        f"vpunpckhdq {t1}, {a}, {b}",
        f"vpunpckldq {t2}, {c}, {d}",
        f"vpunpckhdq {t3}, {c}, {d}",
        f"vpunpcklqdq {a}, {t0}, {t2}",
        f"vpunpckhqdq {b}, {t0}, {t2}",
        f"vpunpcklqdq {c}, {t1}, {t3}",
        f"vpunpckhqdq {d}, {t1}, {t3}",
    ]


def emit_kernel(name, nwords):
    L = [f".globl {name}", ".p2align 5", f"{name}:"]
    e = L.append

    done_lbl = f".L_{name}_done"
    loop_lbl = f".L_{name}_loop"

    e("push rbp")
    e("mov rbp, rsp")
    e("and rsp, -32")
    e(f"sub rsp, {FRAME}")

    e("test rsi, rsi")
    e(f"jz {done_lbl}")

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
    # digests land in exact linear order 0..7.
    for i in range(8):
        e(f"mov dword ptr [rsp + {F_SCR + 4 * i}], {2 * (i % 4) + (i // 4)}")
    e(f"vmovdqa ymm0, ymmword ptr [rsp + {F_SCR}]")
    e("vmovd xmm1, ecx")
    e("vpbroadcastd ymm1, xmm1")
    e("vpaddd ymm0, ymm0, ymm1")
    e(f"vmovdqa ymmword ptr [rsp + {F_CTR}], ymm0")

    # Unpack scalar parameters:
    #   r10  = counter_word * 32 (byte offset into F_MSG!)
    #   r11d = flags (bits 0..7)
    #   r8d  = block_len (bits 8..15)
    e("mov r10, rcx")
    e("shr r10, 32")
    e("shl r10, 5")
    e("mov r11d, r8d")
    e("and r11d, 0xff")
    e("shr r8d, 8")
    e("and r8d, 0xff")

    # Broadcast all static template words ONCE into L1 stack array [rsp + F_MSG..].
    for w in range(nwords):
        e(f"vpbroadcastd ymm0, dword ptr [rdi + {4 * w}]")
        e(f"vmovdqa ymmword ptr [rsp + {F_MSG + 32 * w}], ymm0")

    e(f"{loop_lbl}:")

    # 1. Stamp ONLY [rsp + r10] with current counter vector & advance by +8!
    e(f"vmovdqa ymm0, ymmword ptr [rsp + {F_CTR}]")
    e("vmovdqa ymmword ptr [rsp + r10], ymm0")
    e(f"vpaddd ymm0, ymm0, ymmword ptr [rsp + {F_EIGHT}]")
    e(f"vmovdqa ymmword ptr [rsp + {F_CTR}], ymm0")

    # 2. Load state ymm0..ymm11 + d-row [rsp + F_D..].
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

    # 3. 7 unrolled rounds: zero message loads from input RAM, all L1 hits!
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

    # 4. Feed-forward & output transpose -> 8 contiguous 32-byte stores (256 B).
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

    e(f"{done_lbl}:")
    e("vzeroupper")
    e("mov rsp, rbp")
    e("pop rbp")
    e("ret")
    e(f"{name}_end:")
    return "\n".join(L)


def main():
    parts = [".intel_syntax noprefix", ".text"]
    parts.append(emit_kernel("probe_avx2_counter_range", 16))
    parts.append(emit_kernel("probe_avx2_counter_range_52", 13))
    parts.append(emit_kernel("probe_avx2_counter_range_36", 9))
    parts.append(".att_syntax prefix")
    Path("examples/avx2_counter_x86_64.S").write_text("\n\n".join(parts) + "\n")
    print("Generated examples/avx2_counter_x86_64.S")


if __name__ == "__main__":
    main()
