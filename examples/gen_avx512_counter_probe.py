#!/usr/bin/env python3
"""Generate register-only x86-64 AVX-512 (16 lanes/group) counter-grinding kernels (#511):
  - ZERO message memory reads inside the group loop (all 16 message words live
    in zmm16..zmm31 across the entire `groups` loop!)
  - Dead zmm8..zmm11 (after feed-forward) used as output-transpose scratch so
    zmm16..zmm31 are never clobbered
  - 0-byte stack frame during the loop; 64 B stack scratch used ONCE before
    the loop to build [0, 1, ..., 15]
  - One universal kernel (`probe_avx512_counter_range`) handles ANY len 4..=64,
    ANY counter_word 0..=15, ANY wrapping base: u32
  - Also emits 52 B (13-word) and 36 B (9-word) zero-add-pruned variants

System V x86-64 ABI:
  fn(
      template: *const u32,  // rdi: 16 x u32 zero-padded 64-byte template
      groups: u64,           // rsi: number of 16-try groups (>= 1)
      key: *const u32,       // rdx: 8 x u32 key words
      base_and_word: u64,    // rcx: (base as u32) | ((counter_word as u64) << 32)
      flags_and_len: u64,    // r8:  (flags & 0xff) | ((block_len & 0xff) << 8)
      out: *mut u8,          // r9:  512 * groups bytes
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
    L = [f".globl {name}", ".p2align 4", f"{name}:"]
    e = L.append

    done_lbl = f".L_{name}_done"
    loop_lbl = f".L_{name}_loop"
    init_done_lbl = f".L_{name}_init_done"

    e("test rsi, rsi")
    e(f"jz {done_lbl}")

    # Build zmm0 = [base + perm[i]] ONCE before the loop, where perm[i] = 4*(i%4) + (i//4)
    # matches the 4x4 output transpose so digests land in exact linear order 0..15.
    e("sub rsp, 64")
    for i in range(16):
        e(f"mov dword ptr [rsp + {4 * i}], {4 * (i % 4) + (i // 4)}")
    e("vmovdqu32 zmm0, zmmword ptr [rsp]")
    e("add rsp, 64")
    e("vpbroadcastd zmm1, ecx")
    e("vpaddd zmm0, zmm0, zmm1")

    # Unpack scalar parameters:
    #   r10  = counter_word (0..nwords-1)
    #   r11d = flags (bits 0..7)
    #   r8d  = block_len (bits 8..15)
    e("mov r10, rcx")
    e("shr r10, 32")
    e("mov r11d, r8d")
    e("and r11d, 0xff")
    e("shr r8d, 8")
    e("and r8d, 0xff")

    # Broadcast all static template words ONCE into zmm16..zmm(16+nwords-1).
    for w in range(nwords):
        e(f"vpbroadcastd zmm{16 + w}, dword ptr [rdi + {4 * w}]")

    # Place initial [base..base+15] into zmm(16 + counter_word) ONCE before the loop.
    for w in range(nwords):
        e(f"cmp r10, {w}")
        e(f"je .L_{name}_set_{w}")
    for w in range(nwords):
        e(f".L_{name}_set_{w}:")
        e(f"vmovdqa32 zmm{16 + w}, zmm0")
        e(f"jmp {init_done_lbl}")
    e(f"{init_done_lbl}:")

    # === Hot group loop: ZERO message memory reads, ZERO stack traffic! ===
    e(f"{loop_lbl}:")

    # 1. Load state zmm0..zmm15 (key, IV, counter=0, block_len, flags).
    for i in range(8):
        e(f"vpbroadcastd zmm{i}, dword ptr [rdx + {4 * i}]")
    for i in range(4):
        e(f"mov eax, {IV[i]}")
        e(f"vpbroadcastd zmm{8 + i}, eax")
    e("vpxord zmm12, zmm12, zmm12")
    e("vpxord zmm13, zmm13, zmm13")
    e("vpbroadcastd zmm14, r8d")
    e("vpbroadcastd zmm15, r11d")

    # 2. 7 unrolled rounds (zmm16..zmm31 read-only!).
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

    # 3. Feed-forward & output transpose using DEAD zmm8..zmm11 (zmm16..zmm31 untouched!).
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
    e(f"jz {done_lbl}")

    # 4. Increment ONLY zmm(16 + counter_word) by +16 (100% predicted across all groups!).
    e("mov eax, 16")
    e("vpbroadcastd zmm0, eax")
    for w in range(nwords):
        e(f"cmp r10, {w}")
        e(f"je .L_{name}_inc_{w}")
    for w in range(nwords):
        e(f".L_{name}_inc_{w}:")
        e(f"vpaddd zmm{16 + w}, zmm{16 + w}, zmm0")
        e(f"jmp {loop_lbl}")

    e(f"{done_lbl}:")
    e("vzeroupper")
    e("ret")
    e(f"{name}_end:")
    return "\n".join(L)


def main():
    parts = [".intel_syntax noprefix", ".text"]
    parts.append(emit_kernel("probe_avx512_counter_range", 16))
    parts.append(emit_kernel("probe_avx512_counter_range_52", 13))
    parts.append(emit_kernel("probe_avx512_counter_range_36", 9))
    parts.append(".att_syntax prefix")
    Path("examples/avx512_counter_x86_64.S").write_text("\n\n".join(parts) + "\n")
    print("Generated examples/avx512_counter_x86_64.S")


if __name__ == "__main__":
    main()
