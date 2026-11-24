#!/usr/bin/env python3
"""Generate 0-stack-frame, all-in-register, direct-load AVX-512 kernels for
Packed<16>, Packed<32>, and Packed<48> (16 lanes per group, contiguous loads &
stores, zero 8 KiB stack expansion, zero Input structs, 25-75% of message adds
eliminated at generation time).

SysV x86-64 ABI:
  fn(input: *const u8, groups: u64, key: *const u32, flags: u32, out: *mut u8)
     rdi               rsi          rdx              ecx         r8
Requires AVX-512F and groups >= 1.
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
    loop_lbl = f".L_{name}_loop"

    e(f"{loop_lbl}:")

    # 1. Load & transpose 16 contiguous messages into zmm16..zmm(16+nwords-1)
    #    in lane order [0,4,8,12 | 1,5,9,13 | 2,6,10,14 | 3,7,11,15].
    if nbytes == 16:
        for p in range(4):
            e(f"vmovdqu32 zmm{16 + p}, zmmword ptr [rdi + {64 * p}]")
        L.extend(transpose4x4("zmm16", "zmm17", "zmm18", "zmm19", "zmm28", "zmm29", "zmm30", "zmm31"))
    elif nbytes == 32:
        for k in range(8):
            e(f"vmovdqu32 zmm{k}, zmmword ptr [rdi + {64 * k}]")
        for p in range(4):
            e(f"vshufi32x4 zmm{16 + p}, zmm{2 * p}, zmm{2 * p + 1}, 0x88")
            e(f"vshufi32x4 zmm{20 + p}, zmm{2 * p}, zmm{2 * p + 1}, 0xdd")
        L.extend(transpose4x4("zmm16", "zmm17", "zmm18", "zmm19", "zmm28", "zmm29", "zmm30", "zmm31"))
        L.extend(transpose4x4("zmm20", "zmm21", "zmm22", "zmm23", "zmm28", "zmm29", "zmm30", "zmm31"))
    elif nbytes == 48:
        for p in range(4):
            base = 192 * p
            e(f"vmovdqu32 zmm0, zmmword ptr [rdi + {base}]")
            e(f"vmovdqu32 zmm1, zmmword ptr [rdi + {base + 64}]")
            e(f"vmovdqu32 zmm2, zmmword ptr [rdi + {base + 128}]")
            # L_p = [z0[0], z0[3], z1[2], z2[1]]
            e("vshufi32x4 zmm28, zmm1, zmm2, 0x12")
            e(f"vshufi32x4 zmm{16 + p}, zmm0, zmm28, 0x8c")
            # M_p = [z0[1], z1[0], z1[3], z2[2]]
            e("vshufi32x4 zmm28, zmm0, zmm1, 0x01")
            e("vshufi32x4 zmm29, zmm1, zmm2, 0x23")
            e(f"vshufi32x4 zmm{20 + p}, zmm28, zmm29, 0x88")
            # H_p = [z0[2], z1[1], z2[0], z2[3]]
            e("vshufi32x4 zmm28, zmm0, zmm1, 0x12")
            e(f"vshufi32x4 zmm{24 + p}, zmm28, zmm2, 0xc8")
        L.extend(transpose4x4("zmm16", "zmm17", "zmm18", "zmm19", "zmm28", "zmm29", "zmm30", "zmm31"))
        L.extend(transpose4x4("zmm20", "zmm21", "zmm22", "zmm23", "zmm28", "zmm29", "zmm30", "zmm31"))
        L.extend(transpose4x4("zmm24", "zmm25", "zmm26", "zmm27", "zmm28", "zmm29", "zmm30", "zmm31"))
    else:
        raise ValueError(nbytes)

    # 2. Load state zmm0..zmm15 (key, IV, counter=0, block_len=nbytes, flags).
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

    # 3. 7 unrolled rounds, step-at-a-time across the 4 parallel G columns/diags,
    #    omitting every vpaddd whose message word is guaranteed zero (>= nwords).
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

    # 4. Feed-forward & output transpose -> 8 contiguous 64-byte stores (512 B).
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
    e("ret")
    e(f"{name}_end:")
    return "\n".join(L)


def main():
    parts = [".intel_syntax noprefix", ".text"]
    for n in (16, 32, 48):
        parts.append(emit_kernel(f"probe_avx512_packed{n}", n))
    parts.append(".att_syntax prefix")
    Path("examples/avx512_packed_x86_64.S").write_text("\n\n".join(parts) + "\n")
    print("Generated examples/avx512_packed_x86_64.S")


if __name__ == "__main__":
    main()
