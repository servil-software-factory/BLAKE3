#!/usr/bin/env python3
"""Generate direct-load AVX2 kernels for Packed<16>, Packed<32>, and Packed<48>
(8 lanes per group, contiguous 32-byte loads & stores, tiny 128-384 B transposed
message frame instead of 8 KiB stack expansion, zero Input structs, 25-75% of
message loads & adds eliminated at generation time).

SysV x86-64 ABI:
  fn(input: *const u8, groups: u64, key: *const u32, flags: u32, out: *mut u8)
     rdi               rsi          rdx              ecx         r8
Requires AVX2 and groups >= 1.
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

F_MSG = 0      # up to 12 * 32 = 384 B
F_D = 384      # 4 * 32 = 128 B (d row v12..v15)
F_SCR = 512    # 32 B scratch for ROT16/ROT8 mask construction
FRAME = 544

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


def emit_kernel(name, nbytes):
    nwords = nbytes // 4
    L = [f".globl {name}", ".p2align 5", f"{name}:"]
    e = L.append

    # 32-byte aligned frame, preserving rbp.
    e("push rbp")
    e("mov rbp, rsp")
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

    loop_lbl = f".L_{name}_loop"
    e(f"{loop_lbl}:")

    # 1. Load & transpose 8 contiguous messages into [rsp + F_MSG + 32*w],
    #    lane order [0, 2, 4, 6 | 1, 3, 5, 7].
    if nbytes == 16:
        for p in range(4):
            e(f"vmovdqu ymm{p}, ymmword ptr [rdi + {32 * p}]")
        L.extend(transpose4x4("ymm0", "ymm1", "ymm2", "ymm3", "ymm8", "ymm9", "ymm10", "ymm11"))
        for w in range(4):
            e(f"vmovdqa ymmword ptr [rsp + {F_MSG + 32 * w}], ymm{w}")
    elif nbytes == 32:
        for p in range(4):
            e(f"vmovdqu ymm8, ymmword ptr [rdi + {64 * p}]")
            e(f"vmovdqu ymm9, ymmword ptr [rdi + {64 * p + 32}]")
            e(f"vperm2i128 ymm{p}, ymm8, ymm9, 0x20")
            e(f"vperm2i128 ymm{4 + p}, ymm8, ymm9, 0x31")
        L.extend(transpose4x4("ymm0", "ymm1", "ymm2", "ymm3", "ymm8", "ymm9", "ymm10", "ymm11"))
        L.extend(transpose4x4("ymm4", "ymm5", "ymm6", "ymm7", "ymm8", "ymm9", "ymm10", "ymm11"))
        for w in range(8):
            e(f"vmovdqa ymmword ptr [rsp + {F_MSG + 32 * w}], ymm{w}")
    elif nbytes == 48:
        for p in range(4):
            base = 96 * p
            e(f"vmovdqu ymm0, ymmword ptr [rdi + {base}]")
            e(f"vmovdqu ymm1, ymmword ptr [rdi + {base + 32}]")
            e(f"vmovdqu ymm2, ymmword ptr [rdi + {base + 64}]")
            e(f"vperm2i128 ymm{3 + p}, ymm0, ymm1, 0x30")   # L_p = [M0.lo | M1.lo]
            e(f"vperm2i128 ymm{7 + p}, ymm0, ymm2, 0x21")   # M_p = [M0.mid | M1.mid]
            e("vperm2i128 ymm12, ymm1, ymm2, 0x30")         # H_p = [M0.hi | M1.hi]
            e(f"vmovdqa ymmword ptr [rsp + {F_MSG + 256 + 32 * p}], ymm12")
        L.extend(transpose4x4("ymm3", "ymm4", "ymm5", "ymm6", "ymm0", "ymm1", "ymm2", "ymm12"))
        for w in range(4):
            e(f"vmovdqa ymmword ptr [rsp + {F_MSG + 32 * w}], ymm{3 + w}")
        L.extend(transpose4x4("ymm7", "ymm8", "ymm9", "ymm10", "ymm0", "ymm1", "ymm2", "ymm12"))
        for w in range(4):
            e(f"vmovdqa ymmword ptr [rsp + {F_MSG + 128 + 32 * w}], ymm{7 + w}")
        for p in range(4):
            e(f"vmovdqa ymm{p}, ymmword ptr [rsp + {F_MSG + 256 + 32 * p}]")
        L.extend(transpose4x4("ymm0", "ymm1", "ymm2", "ymm3", "ymm8", "ymm9", "ymm10", "ymm11"))
        for w in range(4):
            e(f"vmovdqa ymmword ptr [rsp + {F_MSG + 256 + 32 * w}], ymm{w}")
    else:
        raise ValueError(nbytes)

    # 2. Initialize state: v0..v11 in ymm0..ymm11, v12..v15 in [rsp + F_D + ...].
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

    # 3. 7 unrolled rounds: v0..v11 in registers, d word loaded/stored once per G,
    #    omitting every message add where word >= nwords!
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
        e(f"vmovdqu ymmword ptr [r8 + {64 * p}], ymm8")
        e(f"vmovdqu ymmword ptr [r8 + {64 * p + 32}], ymm9")

    e(f"add rdi, {8 * nbytes}")
    e("add r8, 256")
    e("dec rsi")
    e(f"jnz {loop_lbl}")

    e("vzeroupper")
    e("mov rsp, rbp")
    e("pop rbp")
    e("ret")
    e(f"{name}_end:")
    return "\n".join(L)


def main():
    parts = [".intel_syntax noprefix", ".text"]
    for n in (16, 32, 48):
        parts.append(emit_kernel(f"probe_avx2_packed{n}", n))
    parts.append(".att_syntax prefix")
    Path("examples/avx2_packed_x86_64.S").write_text("\n\n".join(parts) + "\n")
    print("Generated examples/avx2_packed_x86_64.S")


if __name__ == "__main__":
    main()
