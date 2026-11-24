.intel_syntax noprefix

.text

.globl probe_avx2_packed16
.p2align 5
probe_avx2_packed16:
push rbp
mov rbp, rsp
and rsp, -32
sub rsp, 544
movabs rax, 361421592464458498
mov qword ptr [rsp + 512], rax
mov qword ptr [rsp + 528], rax
movabs rax, 940142975169071882
mov qword ptr [rsp + 520], rax
mov qword ptr [rsp + 536], rax
vmovdqa ymm13, ymmword ptr [rsp + 512]
movabs rax, 290207319533486593
mov qword ptr [rsp + 512], rax
mov qword ptr [rsp + 528], rax
movabs rax, 868928702238099977
mov qword ptr [rsp + 520], rax
mov qword ptr [rsp + 536], rax
vmovdqa ymm14, ymmword ptr [rsp + 512]
.L_probe_avx2_packed16_loop:
vmovdqu ymm0, ymmword ptr [rdi + 0]
vmovdqu ymm1, ymmword ptr [rdi + 32]
vmovdqu ymm2, ymmword ptr [rdi + 64]
vmovdqu ymm3, ymmword ptr [rdi + 96]
vpunpckldq ymm8, ymm0, ymm1
vpunpckhdq ymm9, ymm0, ymm1
vpunpckldq ymm10, ymm2, ymm3
vpunpckhdq ymm11, ymm2, ymm3
vpunpcklqdq ymm0, ymm8, ymm10
vpunpckhqdq ymm1, ymm8, ymm10
vpunpcklqdq ymm2, ymm9, ymm11
vpunpckhqdq ymm3, ymm9, ymm11
vmovdqa ymmword ptr [rsp + 0], ymm0
vmovdqa ymmword ptr [rsp + 32], ymm1
vmovdqa ymmword ptr [rsp + 64], ymm2
vmovdqa ymmword ptr [rsp + 96], ymm3
vpbroadcastd ymm0, dword ptr [rdx + 0]
vpbroadcastd ymm1, dword ptr [rdx + 4]
vpbroadcastd ymm2, dword ptr [rdx + 8]
vpbroadcastd ymm3, dword ptr [rdx + 12]
vpbroadcastd ymm4, dword ptr [rdx + 16]
vpbroadcastd ymm5, dword ptr [rdx + 20]
vpbroadcastd ymm6, dword ptr [rdx + 24]
vpbroadcastd ymm7, dword ptr [rdx + 28]
mov eax, 1779033703
vmovd xmm12, eax
vpbroadcastd ymm8, xmm12
mov eax, 3144134277
vmovd xmm12, eax
vpbroadcastd ymm9, xmm12
mov eax, 1013904242
vmovd xmm12, eax
vpbroadcastd ymm10, xmm12
mov eax, 2773480762
vmovd xmm12, eax
vpbroadcastd ymm11, xmm12
vpxor ymm12, ymm12, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
mov eax, 16
vmovd xmm12, eax
vpbroadcastd ymm12, xmm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vmovd xmm12, ecx
vpbroadcastd ymm12, xmm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 0]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 32]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 64]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 96]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 64]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 96]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 0]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 32]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 96]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 64]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 0]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm3, ymm3, ymmword ptr [rsp + 32]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 96]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 0]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 64]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm3, ymm3, ymmword ptr [rsp + 32]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 64]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 96]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 0]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 32]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm3, ymm3, ymmword ptr [rsp + 32]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 96]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 0]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 64]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 0]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 32]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 64]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 96]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vpxor ymm0, ymm0, ymm8
vpxor ymm1, ymm1, ymm9
vpxor ymm2, ymm2, ymm10
vpxor ymm3, ymm3, ymm11
vpxor ymm4, ymm4, ymmword ptr [rsp + 384]
vpxor ymm5, ymm5, ymmword ptr [rsp + 416]
vpxor ymm6, ymm6, ymmword ptr [rsp + 448]
vpxor ymm7, ymm7, ymmword ptr [rsp + 480]
vpunpckldq ymm8, ymm0, ymm1
vpunpckhdq ymm9, ymm0, ymm1
vpunpckldq ymm10, ymm2, ymm3
vpunpckhdq ymm11, ymm2, ymm3
vpunpcklqdq ymm0, ymm8, ymm10
vpunpckhqdq ymm1, ymm8, ymm10
vpunpcklqdq ymm2, ymm9, ymm11
vpunpckhqdq ymm3, ymm9, ymm11
vpunpckldq ymm8, ymm4, ymm5
vpunpckhdq ymm9, ymm4, ymm5
vpunpckldq ymm10, ymm6, ymm7
vpunpckhdq ymm11, ymm6, ymm7
vpunpcklqdq ymm4, ymm8, ymm10
vpunpckhqdq ymm5, ymm8, ymm10
vpunpcklqdq ymm6, ymm9, ymm11
vpunpckhqdq ymm7, ymm9, ymm11
vperm2i128 ymm8, ymm0, ymm4, 0x20
vperm2i128 ymm9, ymm0, ymm4, 0x31
vmovdqu ymmword ptr [r8 + 0], ymm8
vmovdqu ymmword ptr [r8 + 32], ymm9
vperm2i128 ymm8, ymm1, ymm5, 0x20
vperm2i128 ymm9, ymm1, ymm5, 0x31
vmovdqu ymmword ptr [r8 + 64], ymm8
vmovdqu ymmword ptr [r8 + 96], ymm9
vperm2i128 ymm8, ymm2, ymm6, 0x20
vperm2i128 ymm9, ymm2, ymm6, 0x31
vmovdqu ymmword ptr [r8 + 128], ymm8
vmovdqu ymmword ptr [r8 + 160], ymm9
vperm2i128 ymm8, ymm3, ymm7, 0x20
vperm2i128 ymm9, ymm3, ymm7, 0x31
vmovdqu ymmword ptr [r8 + 192], ymm8
vmovdqu ymmword ptr [r8 + 224], ymm9
add rdi, 128
add r8, 256
dec rsi
jnz .L_probe_avx2_packed16_loop
vzeroupper
mov rsp, rbp
pop rbp
ret
probe_avx2_packed16_end:

.globl probe_avx2_packed32
.p2align 5
probe_avx2_packed32:
push rbp
mov rbp, rsp
and rsp, -32
sub rsp, 544
movabs rax, 361421592464458498
mov qword ptr [rsp + 512], rax
mov qword ptr [rsp + 528], rax
movabs rax, 940142975169071882
mov qword ptr [rsp + 520], rax
mov qword ptr [rsp + 536], rax
vmovdqa ymm13, ymmword ptr [rsp + 512]
movabs rax, 290207319533486593
mov qword ptr [rsp + 512], rax
mov qword ptr [rsp + 528], rax
movabs rax, 868928702238099977
mov qword ptr [rsp + 520], rax
mov qword ptr [rsp + 536], rax
vmovdqa ymm14, ymmword ptr [rsp + 512]
.L_probe_avx2_packed32_loop:
vmovdqu ymm8, ymmword ptr [rdi + 0]
vmovdqu ymm9, ymmword ptr [rdi + 32]
vperm2i128 ymm0, ymm8, ymm9, 0x20
vperm2i128 ymm4, ymm8, ymm9, 0x31
vmovdqu ymm8, ymmword ptr [rdi + 64]
vmovdqu ymm9, ymmword ptr [rdi + 96]
vperm2i128 ymm1, ymm8, ymm9, 0x20
vperm2i128 ymm5, ymm8, ymm9, 0x31
vmovdqu ymm8, ymmword ptr [rdi + 128]
vmovdqu ymm9, ymmword ptr [rdi + 160]
vperm2i128 ymm2, ymm8, ymm9, 0x20
vperm2i128 ymm6, ymm8, ymm9, 0x31
vmovdqu ymm8, ymmword ptr [rdi + 192]
vmovdqu ymm9, ymmword ptr [rdi + 224]
vperm2i128 ymm3, ymm8, ymm9, 0x20
vperm2i128 ymm7, ymm8, ymm9, 0x31
vpunpckldq ymm8, ymm0, ymm1
vpunpckhdq ymm9, ymm0, ymm1
vpunpckldq ymm10, ymm2, ymm3
vpunpckhdq ymm11, ymm2, ymm3
vpunpcklqdq ymm0, ymm8, ymm10
vpunpckhqdq ymm1, ymm8, ymm10
vpunpcklqdq ymm2, ymm9, ymm11
vpunpckhqdq ymm3, ymm9, ymm11
vpunpckldq ymm8, ymm4, ymm5
vpunpckhdq ymm9, ymm4, ymm5
vpunpckldq ymm10, ymm6, ymm7
vpunpckhdq ymm11, ymm6, ymm7
vpunpcklqdq ymm4, ymm8, ymm10
vpunpckhqdq ymm5, ymm8, ymm10
vpunpcklqdq ymm6, ymm9, ymm11
vpunpckhqdq ymm7, ymm9, ymm11
vmovdqa ymmword ptr [rsp + 0], ymm0
vmovdqa ymmword ptr [rsp + 32], ymm1
vmovdqa ymmword ptr [rsp + 64], ymm2
vmovdqa ymmword ptr [rsp + 96], ymm3
vmovdqa ymmword ptr [rsp + 128], ymm4
vmovdqa ymmword ptr [rsp + 160], ymm5
vmovdqa ymmword ptr [rsp + 192], ymm6
vmovdqa ymmword ptr [rsp + 224], ymm7
vpbroadcastd ymm0, dword ptr [rdx + 0]
vpbroadcastd ymm1, dword ptr [rdx + 4]
vpbroadcastd ymm2, dword ptr [rdx + 8]
vpbroadcastd ymm3, dword ptr [rdx + 12]
vpbroadcastd ymm4, dword ptr [rdx + 16]
vpbroadcastd ymm5, dword ptr [rdx + 20]
vpbroadcastd ymm6, dword ptr [rdx + 24]
vpbroadcastd ymm7, dword ptr [rdx + 28]
mov eax, 1779033703
vmovd xmm12, eax
vpbroadcastd ymm8, xmm12
mov eax, 3144134277
vmovd xmm12, eax
vpbroadcastd ymm9, xmm12
mov eax, 1013904242
vmovd xmm12, eax
vpbroadcastd ymm10, xmm12
mov eax, 2773480762
vmovd xmm12, eax
vpbroadcastd ymm11, xmm12
vpxor ymm12, ymm12, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
mov eax, 32
vmovd xmm12, eax
vpbroadcastd ymm12, xmm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vmovd xmm12, ecx
vpbroadcastd ymm12, xmm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 0]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 32]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 64]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 96]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 128]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 160]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm3, ymm3, ymmword ptr [rsp + 192]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm3, ymm3, ymmword ptr [rsp + 224]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 64]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 192]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 96]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 224]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 0]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm3, ymm3, ymmword ptr [rsp + 128]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 32]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 160]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 96]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 128]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 64]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm3, ymm3, ymmword ptr [rsp + 224]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 192]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 160]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 0]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm3, ymm3, ymmword ptr [rsp + 32]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 224]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 96]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 128]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 0]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 64]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 160]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm3, ymm3, ymmword ptr [rsp + 32]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm3, ymm3, ymmword ptr [rsp + 192]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 224]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 64]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 160]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 96]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 0]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 32]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm3, ymm3, ymmword ptr [rsp + 192]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm3, ymm3, ymmword ptr [rsp + 128]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 160]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm3, ymm3, ymmword ptr [rsp + 32]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 96]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 0]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 64]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 192]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm3, ymm3, ymmword ptr [rsp + 128]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm3, ymm3, ymmword ptr [rsp + 224]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 160]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 0]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 32]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm3, ymm3, ymmword ptr [rsp + 192]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 64]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 96]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 128]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm3, ymm3, ymmword ptr [rsp + 224]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vpxor ymm0, ymm0, ymm8
vpxor ymm1, ymm1, ymm9
vpxor ymm2, ymm2, ymm10
vpxor ymm3, ymm3, ymm11
vpxor ymm4, ymm4, ymmword ptr [rsp + 384]
vpxor ymm5, ymm5, ymmword ptr [rsp + 416]
vpxor ymm6, ymm6, ymmword ptr [rsp + 448]
vpxor ymm7, ymm7, ymmword ptr [rsp + 480]
vpunpckldq ymm8, ymm0, ymm1
vpunpckhdq ymm9, ymm0, ymm1
vpunpckldq ymm10, ymm2, ymm3
vpunpckhdq ymm11, ymm2, ymm3
vpunpcklqdq ymm0, ymm8, ymm10
vpunpckhqdq ymm1, ymm8, ymm10
vpunpcklqdq ymm2, ymm9, ymm11
vpunpckhqdq ymm3, ymm9, ymm11
vpunpckldq ymm8, ymm4, ymm5
vpunpckhdq ymm9, ymm4, ymm5
vpunpckldq ymm10, ymm6, ymm7
vpunpckhdq ymm11, ymm6, ymm7
vpunpcklqdq ymm4, ymm8, ymm10
vpunpckhqdq ymm5, ymm8, ymm10
vpunpcklqdq ymm6, ymm9, ymm11
vpunpckhqdq ymm7, ymm9, ymm11
vperm2i128 ymm8, ymm0, ymm4, 0x20
vperm2i128 ymm9, ymm0, ymm4, 0x31
vmovdqu ymmword ptr [r8 + 0], ymm8
vmovdqu ymmword ptr [r8 + 32], ymm9
vperm2i128 ymm8, ymm1, ymm5, 0x20
vperm2i128 ymm9, ymm1, ymm5, 0x31
vmovdqu ymmword ptr [r8 + 64], ymm8
vmovdqu ymmword ptr [r8 + 96], ymm9
vperm2i128 ymm8, ymm2, ymm6, 0x20
vperm2i128 ymm9, ymm2, ymm6, 0x31
vmovdqu ymmword ptr [r8 + 128], ymm8
vmovdqu ymmword ptr [r8 + 160], ymm9
vperm2i128 ymm8, ymm3, ymm7, 0x20
vperm2i128 ymm9, ymm3, ymm7, 0x31
vmovdqu ymmword ptr [r8 + 192], ymm8
vmovdqu ymmword ptr [r8 + 224], ymm9
add rdi, 256
add r8, 256
dec rsi
jnz .L_probe_avx2_packed32_loop
vzeroupper
mov rsp, rbp
pop rbp
ret
probe_avx2_packed32_end:

.globl probe_avx2_packed48
.p2align 5
probe_avx2_packed48:
push rbp
mov rbp, rsp
and rsp, -32
sub rsp, 544
movabs rax, 361421592464458498
mov qword ptr [rsp + 512], rax
mov qword ptr [rsp + 528], rax
movabs rax, 940142975169071882
mov qword ptr [rsp + 520], rax
mov qword ptr [rsp + 536], rax
vmovdqa ymm13, ymmword ptr [rsp + 512]
movabs rax, 290207319533486593
mov qword ptr [rsp + 512], rax
mov qword ptr [rsp + 528], rax
movabs rax, 868928702238099977
mov qword ptr [rsp + 520], rax
mov qword ptr [rsp + 536], rax
vmovdqa ymm14, ymmword ptr [rsp + 512]
.L_probe_avx2_packed48_loop:
vmovdqu ymm0, ymmword ptr [rdi + 0]
vmovdqu ymm1, ymmword ptr [rdi + 32]
vmovdqu ymm2, ymmword ptr [rdi + 64]
vperm2i128 ymm3, ymm0, ymm1, 0x30
vperm2i128 ymm7, ymm0, ymm2, 0x21
vperm2i128 ymm12, ymm1, ymm2, 0x30
vmovdqa ymmword ptr [rsp + 256], ymm12
vmovdqu ymm0, ymmword ptr [rdi + 96]
vmovdqu ymm1, ymmword ptr [rdi + 128]
vmovdqu ymm2, ymmword ptr [rdi + 160]
vperm2i128 ymm4, ymm0, ymm1, 0x30
vperm2i128 ymm8, ymm0, ymm2, 0x21
vperm2i128 ymm12, ymm1, ymm2, 0x30
vmovdqa ymmword ptr [rsp + 288], ymm12
vmovdqu ymm0, ymmword ptr [rdi + 192]
vmovdqu ymm1, ymmword ptr [rdi + 224]
vmovdqu ymm2, ymmword ptr [rdi + 256]
vperm2i128 ymm5, ymm0, ymm1, 0x30
vperm2i128 ymm9, ymm0, ymm2, 0x21
vperm2i128 ymm12, ymm1, ymm2, 0x30
vmovdqa ymmword ptr [rsp + 320], ymm12
vmovdqu ymm0, ymmword ptr [rdi + 288]
vmovdqu ymm1, ymmword ptr [rdi + 320]
vmovdqu ymm2, ymmword ptr [rdi + 352]
vperm2i128 ymm6, ymm0, ymm1, 0x30
vperm2i128 ymm10, ymm0, ymm2, 0x21
vperm2i128 ymm12, ymm1, ymm2, 0x30
vmovdqa ymmword ptr [rsp + 352], ymm12
vpunpckldq ymm0, ymm3, ymm4
vpunpckhdq ymm1, ymm3, ymm4
vpunpckldq ymm2, ymm5, ymm6
vpunpckhdq ymm12, ymm5, ymm6
vpunpcklqdq ymm3, ymm0, ymm2
vpunpckhqdq ymm4, ymm0, ymm2
vpunpcklqdq ymm5, ymm1, ymm12
vpunpckhqdq ymm6, ymm1, ymm12
vmovdqa ymmword ptr [rsp + 0], ymm3
vmovdqa ymmword ptr [rsp + 32], ymm4
vmovdqa ymmword ptr [rsp + 64], ymm5
vmovdqa ymmword ptr [rsp + 96], ymm6
vpunpckldq ymm0, ymm7, ymm8
vpunpckhdq ymm1, ymm7, ymm8
vpunpckldq ymm2, ymm9, ymm10
vpunpckhdq ymm12, ymm9, ymm10
vpunpcklqdq ymm7, ymm0, ymm2
vpunpckhqdq ymm8, ymm0, ymm2
vpunpcklqdq ymm9, ymm1, ymm12
vpunpckhqdq ymm10, ymm1, ymm12
vmovdqa ymmword ptr [rsp + 128], ymm7
vmovdqa ymmword ptr [rsp + 160], ymm8
vmovdqa ymmword ptr [rsp + 192], ymm9
vmovdqa ymmword ptr [rsp + 224], ymm10
vmovdqa ymm0, ymmword ptr [rsp + 256]
vmovdqa ymm1, ymmword ptr [rsp + 288]
vmovdqa ymm2, ymmword ptr [rsp + 320]
vmovdqa ymm3, ymmword ptr [rsp + 352]
vpunpckldq ymm8, ymm0, ymm1
vpunpckhdq ymm9, ymm0, ymm1
vpunpckldq ymm10, ymm2, ymm3
vpunpckhdq ymm11, ymm2, ymm3
vpunpcklqdq ymm0, ymm8, ymm10
vpunpckhqdq ymm1, ymm8, ymm10
vpunpcklqdq ymm2, ymm9, ymm11
vpunpckhqdq ymm3, ymm9, ymm11
vmovdqa ymmword ptr [rsp + 256], ymm0
vmovdqa ymmword ptr [rsp + 288], ymm1
vmovdqa ymmword ptr [rsp + 320], ymm2
vmovdqa ymmword ptr [rsp + 352], ymm3
vpbroadcastd ymm0, dword ptr [rdx + 0]
vpbroadcastd ymm1, dword ptr [rdx + 4]
vpbroadcastd ymm2, dword ptr [rdx + 8]
vpbroadcastd ymm3, dword ptr [rdx + 12]
vpbroadcastd ymm4, dword ptr [rdx + 16]
vpbroadcastd ymm5, dword ptr [rdx + 20]
vpbroadcastd ymm6, dword ptr [rdx + 24]
vpbroadcastd ymm7, dword ptr [rdx + 28]
mov eax, 1779033703
vmovd xmm12, eax
vpbroadcastd ymm8, xmm12
mov eax, 3144134277
vmovd xmm12, eax
vpbroadcastd ymm9, xmm12
mov eax, 1013904242
vmovd xmm12, eax
vpbroadcastd ymm10, xmm12
mov eax, 2773480762
vmovd xmm12, eax
vpbroadcastd ymm11, xmm12
vpxor ymm12, ymm12, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
mov eax, 48
vmovd xmm12, eax
vpbroadcastd ymm12, xmm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vmovd xmm12, ecx
vpbroadcastd ymm12, xmm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 0]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 32]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 64]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 96]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 128]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 160]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm3, ymm3, ymmword ptr [rsp + 192]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm3, ymm3, ymmword ptr [rsp + 224]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 256]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 288]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 320]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 352]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 64]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 192]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 96]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 320]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 224]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 0]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm3, ymm3, ymmword ptr [rsp + 128]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 32]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 352]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 160]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 288]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm3, ymm3, ymmword ptr [rsp + 256]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 96]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 128]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 320]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 64]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm3, ymm3, ymmword ptr [rsp + 224]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 192]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 160]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 288]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 0]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 352]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm3, ymm3, ymmword ptr [rsp + 256]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm3, ymm3, ymmword ptr [rsp + 32]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 320]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 224]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 288]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 96]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 128]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 0]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 352]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 64]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 160]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 256]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm3, ymm3, ymmword ptr [rsp + 32]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm3, ymm3, ymmword ptr [rsp + 192]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 288]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 352]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 320]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm3, ymm3, ymmword ptr [rsp + 256]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 224]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 64]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 160]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 96]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 0]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 32]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm3, ymm3, ymmword ptr [rsp + 192]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm3, ymm3, ymmword ptr [rsp + 128]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 288]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 352]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 160]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 256]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm3, ymm3, ymmword ptr [rsp + 32]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 96]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 0]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 320]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 64]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 192]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm3, ymm3, ymmword ptr [rsp + 128]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm3, ymm3, ymmword ptr [rsp + 224]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 352]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 160]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 0]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 32]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 288]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm3, ymm3, ymmword ptr [rsp + 256]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm3, ymm3, ymmword ptr [rsp + 192]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 480]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 320]
vpaddd ymm0, ymm0, ymm5
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 480], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 384]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 64]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 384], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 416]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 96]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 128]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 416], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 448]
vpaddd ymm3, ymm3, ymmword ptr [rsp + 224]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 448], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vpxor ymm0, ymm0, ymm8
vpxor ymm1, ymm1, ymm9
vpxor ymm2, ymm2, ymm10
vpxor ymm3, ymm3, ymm11
vpxor ymm4, ymm4, ymmword ptr [rsp + 384]
vpxor ymm5, ymm5, ymmword ptr [rsp + 416]
vpxor ymm6, ymm6, ymmword ptr [rsp + 448]
vpxor ymm7, ymm7, ymmword ptr [rsp + 480]
vpunpckldq ymm8, ymm0, ymm1
vpunpckhdq ymm9, ymm0, ymm1
vpunpckldq ymm10, ymm2, ymm3
vpunpckhdq ymm11, ymm2, ymm3
vpunpcklqdq ymm0, ymm8, ymm10
vpunpckhqdq ymm1, ymm8, ymm10
vpunpcklqdq ymm2, ymm9, ymm11
vpunpckhqdq ymm3, ymm9, ymm11
vpunpckldq ymm8, ymm4, ymm5
vpunpckhdq ymm9, ymm4, ymm5
vpunpckldq ymm10, ymm6, ymm7
vpunpckhdq ymm11, ymm6, ymm7
vpunpcklqdq ymm4, ymm8, ymm10
vpunpckhqdq ymm5, ymm8, ymm10
vpunpcklqdq ymm6, ymm9, ymm11
vpunpckhqdq ymm7, ymm9, ymm11
vperm2i128 ymm8, ymm0, ymm4, 0x20
vperm2i128 ymm9, ymm0, ymm4, 0x31
vmovdqu ymmword ptr [r8 + 0], ymm8
vmovdqu ymmword ptr [r8 + 32], ymm9
vperm2i128 ymm8, ymm1, ymm5, 0x20
vperm2i128 ymm9, ymm1, ymm5, 0x31
vmovdqu ymmword ptr [r8 + 64], ymm8
vmovdqu ymmword ptr [r8 + 96], ymm9
vperm2i128 ymm8, ymm2, ymm6, 0x20
vperm2i128 ymm9, ymm2, ymm6, 0x31
vmovdqu ymmword ptr [r8 + 128], ymm8
vmovdqu ymmword ptr [r8 + 160], ymm9
vperm2i128 ymm8, ymm3, ymm7, 0x20
vperm2i128 ymm9, ymm3, ymm7, 0x31
vmovdqu ymmword ptr [r8 + 192], ymm8
vmovdqu ymmword ptr [r8 + 224], ymm9
add rdi, 384
add r8, 256
dec rsi
jnz .L_probe_avx2_packed48_loop
vzeroupper
mov rsp, rbp
pop rbp
ret
probe_avx2_packed48_end:

.att_syntax prefix
