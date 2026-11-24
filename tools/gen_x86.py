#!/usr/bin/env python3
"""Generate BLAKE3-Servil's x86-64 kernels: whole chunks and a partial one in one
pass, side by side, as the Arm q kernels do (c/blake3_partial_x86-64_unix.S).

    python3 tools/gen_x86.py > c/blake3_partial_x86-64_unix.S

Five kernels, across widths W = 4, 8, and 16: blake3_servil_partial_sse41
(W = 4, xmm, legacy SSE encodings, for any CPU with SSE4.1),
blake3_servil_partial_avx2 (W = 8, ymm, VEX), and three zero-spill EVEX
AVX-512F+VL kernels: blake3_servil_partial_avx512_4 (W = 4, horizontal zmm),
blake3_servil_partial_avx512_8 (W = 8, vertical ymm), and
blake3_servil_partial_avx512_16 (W = 16, vertical zmm). Each takes one
argument, a pointer to a parameter block that the caller fills
(src/ffi_partial.rs), in the first argument register (rdi, the System V
convention of Linux and macOS).

The kernel hashes W lanes of 16 blocks each, block b of lane j read at
lane j's pointer + 64 b. Every per-lane, per-block value comes from the
block, so the kernel branches only on the block index:

    offset          contents (u32 words unless said; vectors are W words)
    MSG             W lane pointers (u64)
    KEY             8 vectors: the chaining value's words, one per lane
    IV              4 vectors: BLAKE3's IV words 0-3, broadcast
    ROT16, ROT8     the byte shuffles that rotate each word right by 16, 8
    CTR             2 vectors: each lane's counter, low and high words
    LENS            16 vectors: each block's length per lane
    FLAGS           16 vectors: each block's flags per lane
    LAST            u32: 64 x the block after which SNAP is taken
    H               8 vectors, out: each lane's chaining value after block 15
    SNAP            8 vectors, out: each lane's chaining value after block LAST

so a partial chunk takes a lane beside whole ones, its last block's length
and CHUNK_END in its own lane at its own block, its value in SNAP; the
lane's later blocks compute words nobody reads (its pointer must reach 16
blocks: the caller zero-pads it). The registers follow upstream BLAKE3's
x86 kernels: the state's four rows in registers 0-15, row c's first word
kept in a stack slot but while row c is added to, so register 8 is the
temporary the rotations need; the block's sixteen message words
transposed onto the stack first.
"""

import sys

W_OF = {"sse41": 4, "avx2": 8, "avx512_4": 4, "avx512_8": 8, "avx512_16": 16}
MSG_PERMUTATION = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8]
COLS = [(0, 4, 8, 12), (1, 5, 9, 13), (2, 6, 10, 14), (3, 7, 11, 15)]
DIAGS = [(0, 5, 10, 15), (1, 6, 11, 12), (2, 7, 8, 13), (3, 4, 9, 14)]
WORD_16 = [0, 2, 1, 3, 4, 6, 5, 7, 8, 10, 9, 11, 12, 14, 13, 15]


def schedule():
    s = [list(range(16))]
    for _ in range(6):
        s.append([s[-1][MSG_PERMUTATION[i]] for i in range(16)])
    return s


def layout(w):
    """The parameter block's offsets for width w, in bytes."""
    v = 4 * w
    off, o = {}, 0
    for name, size in [("MSG", 8 * w), ("KEY", 8 * v), ("IV", 4 * v), ("ROT16", v), ("ROT8", v), ("CTR", 2 * v),
                       ("LENS", 16 * v), ("FLAGS", 16 * v), ("LAST", 64), ("H", 8 * v), ("SNAP", 8 * v)]:
        off[name] = o
        o += size
    off["SIZE"] = o
    return off


class Kernel:
    def __init__(self, isa):
        self.isa = isa
        self.w = W_OF[isa]
        self.v = 4 * self.w
        self.evex = isa.startswith("avx512_")
        self.avx = isa == "avx2"
        self.reg = {"sse41": "xmm", "avx2": "ymm", "avx512_4": "xmm", "avx512_8": "ymm", "avx512_16": "zmm"}[isa]
        self.word = {"sse41": "xmmword", "avx2": "ymmword", "avx512_4": "xmmword", "avx512_8": "ymmword", "avx512_16": "zmmword"}[isa]
        self.p = "rdi"
        self.off = layout(self.w)
        self.lines = []
        # Stack frame: the sixteen message vectors and the spill slot (unused by EVEX kernels).
        self.msg = 0
        self.spill = 16 * self.v
        self.xsave = self.spill + max(self.v, 16)
        self.frame = self.xsave

    def e(self, s):
        self.lines.append("        " + s)

    def r(self, i):
        return f"{self.reg}{i}"

    def mem(self, base, disp):
        return f"{self.word} ptr [{base}+{disp}]" if disp else f"{self.word} ptr [{base}]"

    def m(self, i):
        return self.mem("rsp", self.msg + self.v * i)

    def pm(self, name, extra=0):
        return self.mem(self.p, self.off[name] + extra)

    # Operations, VEX three-operand or legacy two-operand (dst is a).
    def op(self, vex, legacy, dst, a, b, imm=None):
        tail = f", {imm}" if imm is not None else ""
        if self.avx:
            self.e(f"{vex} {dst}, {a}, {b}{tail}")
        else:
            if dst != a:
                self.e(f"movdqa {dst}, {a}")
            self.e(f"{legacy} {dst}, {b}{tail}")

    def add(self, d, a, b):
        self.op("vpaddd", "paddd", d, a, b)

    def xor(self, d, a, b):
        self.op("vpxor", "pxor", d, a, b)

    def orr(self, d, a, b):
        self.op("vpor", "por", d, a, b)

    def shuf(self, d, a, mask):
        self.op("vpshufb", "pshufb", d, a, mask)

    def shift(self, vex, legacy, d, a, n):
        if self.avx:
            self.e(f"{vex} {d}, {a}, {n}")
        else:
            if d != a:
                self.e(f"movdqa {d}, {a}")
            self.e(f"{legacy} {d}, {n}")

    def load(self, d, src):
        self.e(f"{'vmovdqu' if self.avx else 'movdqu'} {d}, {src}")

    def store(self, dst, s):
        self.e(f"{'vmovdqu' if self.avx else 'movdqu'} {dst}, {s}")

    def store_aligned(self, dst, s):
        self.e(f"{'vmovdqa' if self.avx else 'movdqa'} {dst}, {s}")

    def half(self, quads, words, rd, rb):
        """Half of four Gs side by side: a += m + b; d = (d ^ a) >>> rd;
        c += d; b = (b ^ c) >>> rb. Row c's register 8 is in the spill
        slot outside its addition, register 8 the temporary."""
        r, t = self.r, self.r(8)
        for (a, b, c, d), m in zip(quads, words):
            self.add(r(a), r(a), self.m(m))
        for a, b, c, d in quads:
            self.add(r(a), r(a), r(b))
        for a, b, c, d in quads:
            self.xor(r(d), r(d), r(a))
        self.load(t, self.pm("ROT16" if rd == 16 else "ROT8"))
        for a, b, c, d in quads:
            self.shuf(r(d), r(d), t)
        for a, b, c, d in quads:
            if c == 8:
                self.add(t, r(d), self.mem("rsp", self.spill))
            else:
                self.add(r(c), r(c), r(d))
        for a, b, c, d in quads:
            self.xor(r(b), r(b), r(c))
        self.store_aligned(self.mem("rsp", self.spill), t)
        for a, b, c, d in quads:
            self.shift("vpsrld", "psrld", t, r(b), rb)
            self.shift("vpslld", "pslld", r(b), r(b), 32 - rb)
            self.orr(r(b), r(b), t)

    def transpose(self):
        """Block rdx/64's sixteen words of every lane onto the stack, word i
        of lane j at element j of message vector i."""
        r = self.r
        lanes = [f"r{8 + j}" for j in range(self.w)]
        for q in range(4):
            d = f"+{16 * q}" if q else ""
            if self.avx:
                for k in range(4):
                    self.e(f"vmovdqu xmm{8 + k}, xmmword ptr [{lanes[k]}+rdx{d}]")
                    self.e(f"vinsertf128 ymm{8 + k}, ymm{8 + k}, xmmword ptr [{lanes[k + 4]}+rdx{d}], 1")
                self.e("vunpcklpd ymm12, ymm8, ymm9")
                self.e("vunpckhpd ymm13, ymm8, ymm9")
                self.e("vunpcklpd ymm14, ymm10, ymm11")
                self.e("vunpckhpd ymm15, ymm10, ymm11")
                for k, (lo, hi, imm) in enumerate([(12, 14, 136), (12, 14, 221), (13, 15, 136), (13, 15, 221)]):
                    self.e(f"vshufps ymm{8 + k}, ymm{lo}, ymm{hi}, {imm}")
                    self.store_aligned(self.m(4 * q + k), r(8 + k))
            else:
                for k in range(4):
                    self.e(f"movdqu xmm{8 + k}, xmmword ptr [{lanes[k]}+rdx{d}]")
                for dst, a, b, op in [(12, 8, 9, "unpcklpd"), (13, 8, 9, "unpckhpd"), (14, 10, 11, "unpcklpd"), (15, 10, 11, "unpckhpd")]:
                    self.e(f"movaps xmm{dst}, xmm{a}")
                    self.e(f"{op} xmm{dst}, xmm{b}")
                for k, (lo, hi, imm) in enumerate([(12, 14, 136), (12, 14, 221), (13, 15, 136), (13, 15, 221)]):
                    self.e(f"movaps xmm{8 + k}, xmm{lo}")
                    self.e(f"shufps xmm{8 + k}, xmm{hi}, {imm}")
                    self.store_aligned(self.m(4 * q + k), r(8 + k))

    def half_evex(self, quads, words, rd, rb, regs, msgs):
        """Zero-spill EVEX half-round: all 16 state and 16 message vectors stay
        in registers, and vprord performs every rotation in one instruction."""
        r = self.r
        for (a, b, c, d), m in zip(quads, words):
            self.e(f"vpaddd {r(regs[a])}, {r(regs[a])}, {r(msgs[m])}")
        for a, b, c, d in quads:
            self.e(f"vpaddd {r(regs[a])}, {r(regs[a])}, {r(regs[b])}")
        for a, b, c, d in quads:
            self.e(f"vpxord {r(regs[d])}, {r(regs[d])}, {r(regs[a])}")
        for a, b, c, d in quads:
            self.e(f"vprord {r(regs[d])}, {r(regs[d])}, {rd}")
        for a, b, c, d in quads:
            self.e(f"vpaddd {r(regs[c])}, {r(regs[c])}, {r(regs[d])}")
        for a, b, c, d in quads:
            self.e(f"vpxord {r(regs[b])}, {r(regs[b])}, {r(regs[c])}")
        for a, b, c, d in quads:
            self.e(f"vprord {r(regs[b])}, {r(regs[b])}, {rb}")

    def transpose_evex(self):
        """Transpose block rdx/64 of every lane directly into 16 registers
        above v0..v7, leaving 8 distinct registers for upper state (zero stack)."""
        r = self.r
        if self.w == 8:
            lanes = [f"r{8 + j}" for j in range(8)]
            for q in range(4):
                d = f"+{16 * q}" if q else ""
                for k in range(4):
                    self.e(f"vmovdqu32 xmm{8 + k}, xmmword ptr [{lanes[k]}+rdx{d}]")
                    self.e(f"vinserti32x4 ymm{8 + k}, ymm{8 + k}, xmmword ptr [{lanes[k + 4]}+rdx{d}], 1")
                self.e(f"vpunpckldq {r(12)}, {r(8)}, {r(9)}")
                self.e(f"vpunpckhdq {r(13)}, {r(8)}, {r(9)}")
                self.e(f"vpunpckldq {r(14)}, {r(10)}, {r(11)}")
                self.e(f"vpunpckhdq {r(15)}, {r(10)}, {r(11)}")
                self.e(f"vpunpcklqdq {r(16 + 4 * q + 0)}, {r(12)}, {r(14)}")
                self.e(f"vpunpckhqdq {r(16 + 4 * q + 1)}, {r(12)}, {r(14)}")
                self.e(f"vpunpcklqdq {r(16 + 4 * q + 2)}, {r(13)}, {r(15)}")
                self.e(f"vpunpckhqdq {r(16 + 4 * q + 3)}, {r(13)}, {r(15)}")
            return list(range(16, 32)), list(range(8, 16))
        # W = 16: load 16 blocks into zmm16..31 and transpose in-place using zmm8 as rotating scratch.
        for j in range(16):
            self.e(f"mov rax, qword ptr [{self.p}+{self.off['MSG'] + 8 * j}]")
            self.e(f"vmovdqu32 zmm{16 + j}, zmmword ptr [rax+rdx]")
        rows = list(range(16, 32))
        scratch = 8
        for distance, lo, hi, imm_lo, imm_hi in [
            (1, "vpunpckldq", "vpunpckhdq", "", ""),
            (2, "vpunpcklqdq", "vpunpckhqdq", "", ""),
            (4, "vshufi32x4", "vshufi32x4", ", 0x88", ", 0xdd"),
            (8, "vshufi32x4", "vshufi32x4", ", 0x88", ", 0xdd"),
        ]:
            for base in range(0, 16, 2 * distance):
                for i in range(base, base + distance):
                    j = i + distance
                    a, b = rows[i], rows[j]
                    self.e(f"{lo} zmm{scratch}, zmm{a}, zmm{b}{imm_lo}")
                    self.e(f"{hi} zmm{a}, zmm{a}, zmm{b}{imm_hi}")
                    rows[i], rows[j], scratch = scratch, a, b
        msgs = [rows[WORD_16[i]] for i in range(16)]
        upper = [reg for reg in range(8, 32) if reg not in msgs]
        assert len(upper) == 8 and len(set(msgs + upper)) == 24
        return msgs, upper

    def pack4_to_zmm(self, dst, x0, x1, x2, x3):
        """Pack 4 xmm vectors [l0..l3] of words 0..3 into zmm{dst} as
        [l0.w0..3 | l1.w0..3 | l2.w0..3 | l3.w0..3] using xmm12..15 scratch."""
        self.e(f"vpunpckldq xmm12, {x0}, {x1}")
        self.e(f"vpunpckhdq xmm13, {x0}, {x1}")
        self.e(f"vpunpckldq xmm14, {x2}, {x3}")
        self.e(f"vpunpckhdq xmm15, {x2}, {x3}")
        self.e(f"vpunpcklqdq xmm{dst}, xmm12, xmm14")
        self.e("vpunpckhqdq xmm12, xmm12, xmm14")
        self.e(f"vinserti32x4 zmm{dst}, zmm{dst}, xmm12, 1")
        self.e("vpunpcklqdq xmm14, xmm13, xmm15")
        self.e(f"vinserti32x4 zmm{dst}, zmm{dst}, xmm14, 2")
        self.e("vpunpckhqdq xmm13, xmm13, xmm15")
        self.e(f"vinserti32x4 zmm{dst}, zmm{dst}, xmm13, 3")

    def unpack4_from_zmm(self, src, field, base_word):
        """Unpack horizontal zmm{src} ([l0.w0..3 | ... | l3.w0..3]) to 4 xmm
        words in Params<4>.{field}[base_word..base_word+4]."""
        self.e(f"vmovdqa32 xmm12, xmm{src}")
        self.e(f"vshufi32x4 zmm13, zmm{src}, zmm{src}, 0x55")
        self.e(f"vshufi32x4 zmm14, zmm{src}, zmm{src}, 0xaa")
        self.e(f"vshufi32x4 zmm15, zmm{src}, zmm{src}, 0xff")
        self.e("vpunpckldq xmm8, xmm12, xmm13")
        self.e("vpunpckhdq xmm9, xmm12, xmm13")
        self.e("vpunpckldq xmm10, xmm14, xmm15")
        self.e("vpunpckhdq xmm11, xmm14, xmm15")
        self.e("vpunpcklqdq xmm12, xmm8, xmm10")
        self.e("vpunpckhqdq xmm13, xmm8, xmm10")
        self.e("vpunpcklqdq xmm14, xmm9, xmm11")
        self.e("vpunpckhqdq xmm15, xmm9, xmm11")
        for k in range(4):
            self.e(f"vmovdqu32 xmmword ptr [{self.p}+{self.off[field] + 16 * (base_word + k)}], xmm{12 + k}")

    def body_evex_h4(self, name):
        """Horizontal 4-lane AVX-512F kernel: packs all 4 inputs horizontally
        into zmm0..zmm3 (matching upstream blake3_hash4_avx512), needing 4x
        fewer vprord rotations and ~180 instructions total with zero stack frame."""
        L = self.lines
        self.e("push rbx")
        for j in range(4):
            self.e(f"mov r{8 + j}, qword ptr [{self.p}+{self.off['MSG'] + 8 * j}]")
        # Pack KEY[0..3] -> zmm0, KEY[4..7] -> zmm1, IV[0..3] -> zmm16, CTR -> xmm17/18.
        for base, dst in ((0, 0), (4, 1)):
            for k in range(4):
                self.e(f"vmovdqu32 xmm{8 + k}, xmmword ptr [{self.p}+{self.off['KEY'] + 16 * (base + k)}]")
            self.pack4_to_zmm(dst, "xmm8", "xmm9", "xmm10", "xmm11")
        for k in range(4):
            self.e(f"vmovdqu32 xmm{8 + k}, xmmword ptr [{self.p}+{self.off['IV'] + 16 * k}]")
        self.pack4_to_zmm(16, "xmm8", "xmm9", "xmm10", "xmm11")
        self.e(f"vmovdqu32 xmm17, xmmword ptr [{self.p}+{self.off['CTR']}]")
        self.e(f"vmovdqu32 xmm18, xmmword ptr [{self.p}+{self.off['CTR'] + 16}]")
        self.e("mov eax, 0xaaaa")
        self.e("kmovw k3, eax")
        self.e("mov eax, 0x8888")
        self.e("kmovw k4, eax")
        self.e("xor edx, edx")
        self.e("xor ebx, ebx")
        L.append(f"{name}_block:")
        # Row 2 = IV, Row 3 = [ctr_lo, ctr_hi, len, flags] per lane.
        self.e("vmovdqa32 zmm2, zmm16")
        self.e(f"vmovdqu32 xmm19, xmmword ptr [{self.p}+rbx+{self.off['LENS']}]")
        self.e(f"vmovdqu32 xmm20, xmmword ptr [{self.p}+rbx+{self.off['FLAGS']}]")
        self.pack4_to_zmm(3, "xmm17", "xmm18", "xmm19", "xmm20")
        # Load 64-byte blocks horizontally into zmm4..zmm7 (exact blake3_hash4_avx512 schedule).
        for q, (za, zb, rot) in enumerate(((4, 5, False), (6, 7, True))):
            d0 = f"+{32 * q}" if q else ""
            d1 = f"+{32 * q + 16}"
            self.e(f"vmovdqu32 xmm8, xmmword ptr [r8+rdx{d0}]")
            self.e(f"vinserti32x4 zmm8, zmm8, xmmword ptr [r9+rdx{d0}], 1")
            self.e(f"vinserti32x4 zmm8, zmm8, xmmword ptr [r10+rdx{d0}], 2")
            self.e(f"vinserti32x4 zmm8, zmm8, xmmword ptr [r11+rdx{d0}], 3")
            self.e(f"vmovdqu32 xmm9, xmmword ptr [r8+rdx{d1}]")
            self.e(f"vinserti32x4 zmm9, zmm9, xmmword ptr [r9+rdx{d1}], 1")
            self.e(f"vinserti32x4 zmm9, zmm9, xmmword ptr [r10+rdx{d1}], 2")
            self.e(f"vinserti32x4 zmm9, zmm9, xmmword ptr [r11+rdx{d1}], 3")
            self.e(f"vshufps zmm{za}, zmm8, zmm9, 136")
            self.e(f"vshufps zmm{zb}, zmm8, zmm9, 221")
            if rot:
                self.e(f"vpshufd zmm{za}, zmm{za}, 0x93")
                self.e(f"vpshufd zmm{zb}, zmm{zb}, 0x93")
        self.e("mov al, 7")
        L.append(f"{name}_round:")
        for zm0, zm1, diag in ((4, 5, False), (6, 7, True)):
            if diag:
                self.e("vpshufd zmm0, zmm0, 0x93")
                self.e("vpshufd zmm3, zmm3, 0x4E")
                self.e("vpshufd zmm2, zmm2, 0x39")
            self.e(f"vpaddd zmm0, zmm0, zmm{zm0}")
            self.e("vpaddd zmm0, zmm0, zmm1")
            self.e("vpxord zmm3, zmm3, zmm0")
            self.e("vprord zmm3, zmm3, 16")
            self.e("vpaddd zmm2, zmm2, zmm3")
            self.e("vpxord zmm1, zmm1, zmm2")
            self.e("vprord zmm1, zmm1, 12")
            self.e(f"vpaddd zmm0, zmm0, zmm{zm1}")
            self.e("vpaddd zmm0, zmm0, zmm1")
            self.e("vpxord zmm3, zmm3, zmm0")
            self.e("vprord zmm3, zmm3, 8")
            self.e("vpaddd zmm2, zmm2, zmm3")
            self.e("vpxord zmm1, zmm1, zmm2")
            self.e("vprord zmm1, zmm1, 7")
            if diag:
                self.e("vpshufd zmm0, zmm0, 0x39")
                self.e("vpshufd zmm3, zmm3, 0x4E")
                self.e("vpshufd zmm2, zmm2, 0x93")
        self.e("dec al")
        self.e(f"jz {name}_rdone")
        self.e("vshufps zmm8, zmm4, zmm5, 214")
        self.e("vpshufd zmm9, zmm4, 0x0F")
        self.e("vpshufd zmm4, zmm8, 0x39")
        self.e("vshufps zmm8, zmm6, zmm7, 250")
        self.e("vpblendmd zmm9 {k3}, zmm9, zmm8")
        self.e("vpunpcklqdq zmm8, zmm7, zmm5")
        self.e("vpblendmd zmm8 {k4}, zmm8, zmm6")
        self.e("vpshufd zmm8, zmm8, 0x78")
        self.e("vpunpckhdq zmm5, zmm5, zmm7")
        self.e("vpunpckldq zmm6, zmm6, zmm5")
        self.e("vpshufd zmm7, zmm6, 0x1E")
        self.e("vmovdqa32 zmm5, zmm9")
        self.e("vmovdqa32 zmm6, zmm8")
        self.e(f"jmp {name}_round")
        L.append(f"{name}_rdone:")
        self.e("vpxord zmm0, zmm0, zmm2")
        self.e("vpxord zmm1, zmm1, zmm3")
        self.e(f"cmp edx, dword ptr [{self.p}+{self.off['LAST']}]")
        self.e(f"jne {name}_next")
        self.unpack4_from_zmm(0, "SNAP", 0)
        self.unpack4_from_zmm(1, "SNAP", 4)
        L.append(f"{name}_next:")
        self.e("add rdx, 64")
        self.e("add rbx, 16")
        self.e("cmp rdx, 1024")
        self.e(f"jne {name}_block")
        self.unpack4_from_zmm(0, "H", 0)
        self.unpack4_from_zmm(1, "H", 4)
        self.e("vzeroupper")
        self.e("pop rbx")
        self.e("ret")

    def body_evex(self, name):
        if self.w == 4:
            self.body_evex_h4(name)
            return
        r, w, v = self.r, self.w, self.v
        L = self.lines
        saved = ["rbx", "r12", "r13", "r14", "r15"] if w == 8 else ["rbx"]
        for g in saved:
            self.e(f"push {g}")
        if w == 8:
            for j in range(8):
                self.e(f"mov r{8 + j}, qword ptr [{self.p}+{self.off['MSG'] + 8 * j}]")
        for i in range(8):
            self.e(f"vmovdqu32 {r(i)}, {self.pm('KEY', v * i)}")
        self.e("xor edx, edx")
        self.e("xor ebx, ebx")
        L.append(f"{name}_block:")
        msgs, upper = self.transpose_evex()
        regs = list(range(8)) + upper
        for i in range(4):
            self.e(f"vmovdqu32 {r(regs[8 + i])}, {self.pm('IV', v * i)}")
        self.e(f"vmovdqu32 {r(regs[12])}, {self.pm('CTR')}")
        self.e(f"vmovdqu32 {r(regs[13])}, {self.pm('CTR', v)}")
        self.e(f"vmovdqu32 {r(regs[14])}, {self.mem(f'{self.p}+rbx', self.off['LENS'])}")
        self.e(f"vmovdqu32 {r(regs[15])}, {self.mem(f'{self.p}+rbx', self.off['FLAGS'])}")
        for s in schedule():
            for quads, base in ((COLS, 0), (DIAGS, 8)):
                self.half_evex(quads, [s[base + 2 * k] for k in range(4)], 16, 12, regs, msgs)
                self.half_evex(quads, [s[base + 2 * k + 1] for k in range(4)], 8, 7, regs, msgs)
        for i in range(8):
            self.e(f"vpxord {r(i)}, {r(i)}, {r(regs[8 + i])}")
        self.e(f"cmp edx, dword ptr [{self.p}+{self.off['LAST']}]")
        self.e(f"jne {name}_next")
        for i in range(8):
            self.e(f"vmovdqu32 {self.pm('SNAP', v * i)}, {r(i)}")
        L.append(f"{name}_next:")
        self.e("add rdx, 64")
        self.e(f"add rbx, {v}")
        self.e("cmp rdx, 1024")
        self.e(f"jne {name}_block")
        for i in range(8):
            self.e(f"vmovdqu32 {self.pm('H', v * i)}, {r(i)}")
        self.e("vzeroupper")
        for g in reversed(saved):
            self.e(f"pop {g}")
        self.e("ret")

    def body(self, name):
        if self.evex:
            self.body_evex(name)
            return
        r, w, v = self.r, self.w, self.v
        L = self.lines
        saved = ["rbp", "rbx", "r12", "r13", "r14", "r15"]
        for g in saved:
            self.e(f"push {g}")
        self.e("mov rbp, rsp")
        self.e(f"sub rsp, {self.frame + 64}")
        self.e("and rsp, -64")
        for j in range(w):
            self.e(f"mov r{8 + j}, qword ptr [{self.p}+{self.off['MSG'] + 8 * j}]")
        for i in range(8):
            self.load(r(i), self.pm("KEY", v * i))
        self.e("xor edx, edx")
        self.e("xor ebx, ebx")
        L.append(f"{name}_block:")
        self.transpose()
        self.load(r(8), self.pm("IV"))
        self.store_aligned(self.mem("rsp", self.spill), r(8))
        for i in range(1, 4):
            self.load(r(8 + i), self.pm("IV", v * i))
        self.load(r(12), self.pm("CTR"))
        self.load(r(13), self.pm("CTR", v))
        self.load(r(14), self.mem(f"{self.p}+rbx", self.off["LENS"]))
        self.load(r(15), self.mem(f"{self.p}+rbx", self.off["FLAGS"]))
        for s in schedule():
            for quads, base in ((COLS, 0), (DIAGS, 8)):
                self.half(quads, [s[base + 2 * k] for k in range(4)], 16, 12)
                self.half(quads, [s[base + 2 * k + 1] for k in range(4)], 8, 7)
        self.xor(r(0), r(0), self.mem("rsp", self.spill))
        for i in range(1, 8):
            self.xor(r(i), r(i), r(i + 8))
        self.e(f"cmp edx, dword ptr [{self.p}+{self.off['LAST']}]")
        self.e(f"jne {name}_next")
        for i in range(8):
            self.store(self.pm("SNAP", v * i), r(i))
        L.append(f"{name}_next:")
        self.e("add rdx, 64")
        self.e(f"add rbx, {v}")
        self.e("cmp rdx, 1024")
        self.e(f"jne {name}_block")
        for i in range(8):
            self.store(self.pm("H", v * i), r(i))
        if self.avx:
            self.e("vzeroupper")
        self.e("mov rsp, rbp")
        for g in reversed(saved):
            self.e(f"pop {g}")
        self.e("ret")


def unix():
    out = ["// Generated by tools/gen_x86.py; do not edit.",
           "#if defined(__ELF__) && defined(__linux__)",
           '.section .note.GNU-stack,"",%progbits',
           "#endif",
           "#if defined(__ELF__) && defined(__CET__) && defined(__has_include)",
           "#if __has_include(<cet.h>)",
           "#include <cet.h>",
           "#endif",
           "#endif",
           "#if !defined(_CET_ENDBR)",
           "#define _CET_ENDBR",
           "#endif",
           ".intel_syntax noprefix"]
    isas = ("sse41", "avx2", "avx512_4", "avx512_8", "avx512_16")
    for isa in isas:
        name = f"blake3_servil_partial_{isa}"
        out += [f".global _{name}", f".global {name}"]
    out += ["#ifdef __APPLE__", ".text", "#else", ".section .text", "#endif"]
    for isa in isas:
        name = f"blake3_servil_partial_{isa}"
        k = Kernel(isa)
        k.body(name)
        out += ["        .p2align 6", f"_{name}:", f"{name}:", "        _CET_ENDBR"] + k.lines
    return out


if __name__ == "__main__":
    assert len(sys.argv) == 1, "usage: python3 tools/gen_x86.py > c/blake3_partial_x86-64_unix.S"
    print("\n".join(unix()))
