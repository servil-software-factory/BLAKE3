.intel_syntax noprefix

.text

.globl probe_avx2_counter_range
.p2align 5
probe_avx2_counter_range:
push rbp
mov rbp, rsp
and rsp, -32
sub rsp, 736
test rsi, rsi
jz .L_probe_avx2_counter_range_done
movabs rax, 361421592464458498
mov qword ptr [rsp + 704], rax
mov qword ptr [rsp + 720], rax
movabs rax, 940142975169071882
mov qword ptr [rsp + 712], rax
mov qword ptr [rsp + 728], rax
vmovdqa ymm13, ymmword ptr [rsp + 704]
movabs rax, 290207319533486593
mov qword ptr [rsp + 704], rax
mov qword ptr [rsp + 720], rax
movabs rax, 868928702238099977
mov qword ptr [rsp + 712], rax
mov qword ptr [rsp + 728], rax
vmovdqa ymm14, ymmword ptr [rsp + 704]
mov eax, 8
vmovd xmm0, eax
vpbroadcastd ymm0, xmm0
vmovdqa ymmword ptr [rsp + 672], ymm0
mov dword ptr [rsp + 704], 0
mov dword ptr [rsp + 708], 2
mov dword ptr [rsp + 712], 4
mov dword ptr [rsp + 716], 6
mov dword ptr [rsp + 720], 1
mov dword ptr [rsp + 724], 3
mov dword ptr [rsp + 728], 5
mov dword ptr [rsp + 732], 7
vmovdqa ymm0, ymmword ptr [rsp + 704]
vmovd xmm1, ecx
vpbroadcastd ymm1, xmm1
vpaddd ymm0, ymm0, ymm1
vmovdqa ymmword ptr [rsp + 640], ymm0
mov r10, rcx
shr r10, 32
shl r10, 5
mov r11d, r8d
and r11d, 0xff
shr r8d, 8
and r8d, 0xff
vpbroadcastd ymm0, dword ptr [rdi + 0]
vmovdqa ymmword ptr [rsp + 0], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 4]
vmovdqa ymmword ptr [rsp + 32], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 8]
vmovdqa ymmword ptr [rsp + 64], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 12]
vmovdqa ymmword ptr [rsp + 96], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 16]
vmovdqa ymmword ptr [rsp + 128], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 20]
vmovdqa ymmword ptr [rsp + 160], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 24]
vmovdqa ymmword ptr [rsp + 192], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 28]
vmovdqa ymmword ptr [rsp + 224], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 32]
vmovdqa ymmword ptr [rsp + 256], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 36]
vmovdqa ymmword ptr [rsp + 288], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 40]
vmovdqa ymmword ptr [rsp + 320], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 44]
vmovdqa ymmword ptr [rsp + 352], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 48]
vmovdqa ymmword ptr [rsp + 384], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 52]
vmovdqa ymmword ptr [rsp + 416], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 56]
vmovdqa ymmword ptr [rsp + 448], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 60]
vmovdqa ymmword ptr [rsp + 480], ymm0
.L_probe_avx2_counter_range_loop:
vmovdqa ymm0, ymmword ptr [rsp + 640]
vmovdqa ymmword ptr [rsp + r10], ymm0
vpaddd ymm0, ymm0, ymmword ptr [rsp + 672]
vmovdqa ymmword ptr [rsp + 640], ymm0
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vmovdqa ymmword ptr [rsp + 544], ymm12
vmovd xmm12, r8d
vpbroadcastd ymm12, xmm12
vmovdqa ymmword ptr [rsp + 576], ymm12
vmovd xmm12, r11d
vpbroadcastd ymm12, xmm12
vmovdqa ymmword ptr [rsp + 608], ymm12
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 384]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 416]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
vpaddd ymm3, ymm3, ymmword ptr [rsp + 448]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm3, ymm3, ymmword ptr [rsp + 480]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
vpaddd ymm3, ymm3, ymmword ptr [rsp + 128]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm3, ymm3, ymmword ptr [rsp + 416]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 384]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 288]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 448]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
vpaddd ymm3, ymm3, ymmword ptr [rsp + 480]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 320]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 384]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 416]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
vpaddd ymm3, ymm3, ymmword ptr [rsp + 224]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm3, ymm3, ymmword ptr [rsp + 448]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 352]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 480]
vpaddd ymm2, ymm2, ymm7
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 384]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 448]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
vpaddd ymm3, ymm3, ymmword ptr [rsp + 416]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 12
vpslld ymm7, ymm7, 20
vpor ymm7, ymm7, ymm15
vpaddd ymm3, ymm3, ymmword ptr [rsp + 480]
vpaddd ymm3, ymm3, ymm7
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 384]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 416]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 480]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
vpaddd ymm3, ymm3, ymmword ptr [rsp + 448]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 288]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 448]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 256]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 384]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
vpaddd ymm3, ymm3, ymmword ptr [rsp + 480]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 416]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 352]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm13
vpaddd ymm8, ymm8, ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm0, ymm0, ymmword ptr [rsp + 480]
vpaddd ymm0, ymm0, ymm4
vpxor ymm12, ymm12, ymm0
vpshufb ymm12, ymm12, ymm14
vpaddd ymm8, ymm8, ymm12
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 448]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 64]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 384]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
vpaddd ymm3, ymm3, ymmword ptr [rsp + 224]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 12
vpslld ymm4, ymm4, 20
vpor ymm4, ymm4, ymm15
vpaddd ymm3, ymm3, ymmword ptr [rsp + 416]
vpaddd ymm3, ymm3, ymm4
vpxor ymm12, ymm12, ymm3
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vpxor ymm0, ymm0, ymm8
vpxor ymm1, ymm1, ymm9
vpxor ymm2, ymm2, ymm10
vpxor ymm3, ymm3, ymm11
vpxor ymm4, ymm4, ymmword ptr [rsp + 512]
vpxor ymm5, ymm5, ymmword ptr [rsp + 544]
vpxor ymm6, ymm6, ymmword ptr [rsp + 576]
vpxor ymm7, ymm7, ymmword ptr [rsp + 608]
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
vmovdqu ymmword ptr [r9 + 0], ymm8
vmovdqu ymmword ptr [r9 + 32], ymm9
vperm2i128 ymm8, ymm1, ymm5, 0x20
vperm2i128 ymm9, ymm1, ymm5, 0x31
vmovdqu ymmword ptr [r9 + 64], ymm8
vmovdqu ymmword ptr [r9 + 96], ymm9
vperm2i128 ymm8, ymm2, ymm6, 0x20
vperm2i128 ymm9, ymm2, ymm6, 0x31
vmovdqu ymmword ptr [r9 + 128], ymm8
vmovdqu ymmword ptr [r9 + 160], ymm9
vperm2i128 ymm8, ymm3, ymm7, 0x20
vperm2i128 ymm9, ymm3, ymm7, 0x31
vmovdqu ymmword ptr [r9 + 192], ymm8
vmovdqu ymmword ptr [r9 + 224], ymm9
add r9, 256
dec rsi
jnz .L_probe_avx2_counter_range_loop
.L_probe_avx2_counter_range_done:
vzeroupper
mov rsp, rbp
pop rbp
ret
probe_avx2_counter_range_end:

.globl probe_avx2_counter_range_52
.p2align 5
probe_avx2_counter_range_52:
push rbp
mov rbp, rsp
and rsp, -32
sub rsp, 736
test rsi, rsi
jz .L_probe_avx2_counter_range_52_done
movabs rax, 361421592464458498
mov qword ptr [rsp + 704], rax
mov qword ptr [rsp + 720], rax
movabs rax, 940142975169071882
mov qword ptr [rsp + 712], rax
mov qword ptr [rsp + 728], rax
vmovdqa ymm13, ymmword ptr [rsp + 704]
movabs rax, 290207319533486593
mov qword ptr [rsp + 704], rax
mov qword ptr [rsp + 720], rax
movabs rax, 868928702238099977
mov qword ptr [rsp + 712], rax
mov qword ptr [rsp + 728], rax
vmovdqa ymm14, ymmword ptr [rsp + 704]
mov eax, 8
vmovd xmm0, eax
vpbroadcastd ymm0, xmm0
vmovdqa ymmword ptr [rsp + 672], ymm0
mov dword ptr [rsp + 704], 0
mov dword ptr [rsp + 708], 2
mov dword ptr [rsp + 712], 4
mov dword ptr [rsp + 716], 6
mov dword ptr [rsp + 720], 1
mov dword ptr [rsp + 724], 3
mov dword ptr [rsp + 728], 5
mov dword ptr [rsp + 732], 7
vmovdqa ymm0, ymmword ptr [rsp + 704]
vmovd xmm1, ecx
vpbroadcastd ymm1, xmm1
vpaddd ymm0, ymm0, ymm1
vmovdqa ymmword ptr [rsp + 640], ymm0
mov r10, rcx
shr r10, 32
shl r10, 5
mov r11d, r8d
and r11d, 0xff
shr r8d, 8
and r8d, 0xff
vpbroadcastd ymm0, dword ptr [rdi + 0]
vmovdqa ymmword ptr [rsp + 0], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 4]
vmovdqa ymmword ptr [rsp + 32], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 8]
vmovdqa ymmword ptr [rsp + 64], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 12]
vmovdqa ymmword ptr [rsp + 96], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 16]
vmovdqa ymmword ptr [rsp + 128], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 20]
vmovdqa ymmword ptr [rsp + 160], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 24]
vmovdqa ymmword ptr [rsp + 192], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 28]
vmovdqa ymmword ptr [rsp + 224], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 32]
vmovdqa ymmword ptr [rsp + 256], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 36]
vmovdqa ymmword ptr [rsp + 288], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 40]
vmovdqa ymmword ptr [rsp + 320], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 44]
vmovdqa ymmword ptr [rsp + 352], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 48]
vmovdqa ymmword ptr [rsp + 384], ymm0
.L_probe_avx2_counter_range_52_loop:
vmovdqa ymm0, ymmword ptr [rsp + 640]
vmovdqa ymmword ptr [rsp + r10], ymm0
vpaddd ymm0, ymm0, ymmword ptr [rsp + 672]
vmovdqa ymmword ptr [rsp + 640], ymm0
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vmovdqa ymmword ptr [rsp + 544], ymm12
vmovd xmm12, r8d
vpbroadcastd ymm12, xmm12
vmovdqa ymmword ptr [rsp + 576], ymm12
vmovd xmm12, r11d
vpbroadcastd ymm12, xmm12
vmovdqa ymmword ptr [rsp + 608], ymm12
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 384]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 384]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 320]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm9, ymm9, ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 12
vpslld ymm5, ymm5, 20
vpor ymm5, ymm5, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 384]
vpaddd ymm1, ymm1, ymm5
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm9, ymm9, ymm12
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 384]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 384]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
vpaddd ymm2, ymm2, ymmword ptr [rsp + 256]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm13
vpaddd ymm10, ymm10, ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm2, ymm2, ymmword ptr [rsp + 384]
vpaddd ymm2, ymm2, ymm6
vpxor ymm12, ymm12, ymm2
vpshufb ymm12, ymm12, ymm14
vpaddd ymm10, ymm10, ymm12
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
vpaddd ymm1, ymm1, ymmword ptr [rsp + 64]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm13
vpaddd ymm11, ymm11, ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 12
vpslld ymm6, ymm6, 20
vpor ymm6, ymm6, ymm15
vpaddd ymm1, ymm1, ymmword ptr [rsp + 384]
vpaddd ymm1, ymm1, ymm6
vpxor ymm12, ymm12, ymm1
vpshufb ymm12, ymm12, ymm14
vpaddd ymm11, ymm11, ymm12
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vpxor ymm0, ymm0, ymm8
vpxor ymm1, ymm1, ymm9
vpxor ymm2, ymm2, ymm10
vpxor ymm3, ymm3, ymm11
vpxor ymm4, ymm4, ymmword ptr [rsp + 512]
vpxor ymm5, ymm5, ymmword ptr [rsp + 544]
vpxor ymm6, ymm6, ymmword ptr [rsp + 576]
vpxor ymm7, ymm7, ymmword ptr [rsp + 608]
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
vmovdqu ymmword ptr [r9 + 0], ymm8
vmovdqu ymmword ptr [r9 + 32], ymm9
vperm2i128 ymm8, ymm1, ymm5, 0x20
vperm2i128 ymm9, ymm1, ymm5, 0x31
vmovdqu ymmword ptr [r9 + 64], ymm8
vmovdqu ymmword ptr [r9 + 96], ymm9
vperm2i128 ymm8, ymm2, ymm6, 0x20
vperm2i128 ymm9, ymm2, ymm6, 0x31
vmovdqu ymmword ptr [r9 + 128], ymm8
vmovdqu ymmword ptr [r9 + 160], ymm9
vperm2i128 ymm8, ymm3, ymm7, 0x20
vperm2i128 ymm9, ymm3, ymm7, 0x31
vmovdqu ymmword ptr [r9 + 192], ymm8
vmovdqu ymmword ptr [r9 + 224], ymm9
add r9, 256
dec rsi
jnz .L_probe_avx2_counter_range_52_loop
.L_probe_avx2_counter_range_52_done:
vzeroupper
mov rsp, rbp
pop rbp
ret
probe_avx2_counter_range_52_end:

.globl probe_avx2_counter_range_36
.p2align 5
probe_avx2_counter_range_36:
push rbp
mov rbp, rsp
and rsp, -32
sub rsp, 736
test rsi, rsi
jz .L_probe_avx2_counter_range_36_done
movabs rax, 361421592464458498
mov qword ptr [rsp + 704], rax
mov qword ptr [rsp + 720], rax
movabs rax, 940142975169071882
mov qword ptr [rsp + 712], rax
mov qword ptr [rsp + 728], rax
vmovdqa ymm13, ymmword ptr [rsp + 704]
movabs rax, 290207319533486593
mov qword ptr [rsp + 704], rax
mov qword ptr [rsp + 720], rax
movabs rax, 868928702238099977
mov qword ptr [rsp + 712], rax
mov qword ptr [rsp + 728], rax
vmovdqa ymm14, ymmword ptr [rsp + 704]
mov eax, 8
vmovd xmm0, eax
vpbroadcastd ymm0, xmm0
vmovdqa ymmword ptr [rsp + 672], ymm0
mov dword ptr [rsp + 704], 0
mov dword ptr [rsp + 708], 2
mov dword ptr [rsp + 712], 4
mov dword ptr [rsp + 716], 6
mov dword ptr [rsp + 720], 1
mov dword ptr [rsp + 724], 3
mov dword ptr [rsp + 728], 5
mov dword ptr [rsp + 732], 7
vmovdqa ymm0, ymmword ptr [rsp + 704]
vmovd xmm1, ecx
vpbroadcastd ymm1, xmm1
vpaddd ymm0, ymm0, ymm1
vmovdqa ymmword ptr [rsp + 640], ymm0
mov r10, rcx
shr r10, 32
shl r10, 5
mov r11d, r8d
and r11d, 0xff
shr r8d, 8
and r8d, 0xff
vpbroadcastd ymm0, dword ptr [rdi + 0]
vmovdqa ymmword ptr [rsp + 0], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 4]
vmovdqa ymmword ptr [rsp + 32], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 8]
vmovdqa ymmword ptr [rsp + 64], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 12]
vmovdqa ymmword ptr [rsp + 96], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 16]
vmovdqa ymmword ptr [rsp + 128], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 20]
vmovdqa ymmword ptr [rsp + 160], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 24]
vmovdqa ymmword ptr [rsp + 192], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 28]
vmovdqa ymmword ptr [rsp + 224], ymm0
vpbroadcastd ymm0, dword ptr [rdi + 32]
vmovdqa ymmword ptr [rsp + 256], ymm0
.L_probe_avx2_counter_range_36_loop:
vmovdqa ymm0, ymmword ptr [rsp + 640]
vmovdqa ymmword ptr [rsp + r10], ymm0
vpaddd ymm0, ymm0, ymmword ptr [rsp + 672]
vmovdqa ymmword ptr [rsp + 640], ymm0
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vmovdqa ymmword ptr [rsp + 544], ymm12
vmovd xmm12, r8d
vpbroadcastd ymm12, xmm12
vmovdqa ymmword ptr [rsp + 576], ymm12
vmovd xmm12, r11d
vpbroadcastd ymm12, xmm12
vmovdqa ymmword ptr [rsp + 608], ymm12
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
vpaddd ymm0, ymm0, ymmword ptr [rsp + 256]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm4, ymm4, ymm8
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm5, ymm5, ymm9
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm6, ymm6, ymm10
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm7, ymm7, ymm11
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 608]
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
vmovdqa ymmword ptr [rsp + 608], ymm12
vpxor ymm5, ymm5, ymm10
vpsrld ymm15, ymm5, 7
vpslld ymm5, ymm5, 25
vpor ymm5, ymm5, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 512]
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
vmovdqa ymmword ptr [rsp + 512], ymm12
vpxor ymm6, ymm6, ymm11
vpsrld ymm15, ymm6, 7
vpslld ymm6, ymm6, 25
vpor ymm6, ymm6, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 544]
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
vmovdqa ymmword ptr [rsp + 544], ymm12
vpxor ymm7, ymm7, ymm8
vpsrld ymm15, ymm7, 7
vpslld ymm7, ymm7, 25
vpor ymm7, ymm7, ymm15
vmovdqa ymm12, ymmword ptr [rsp + 576]
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
vmovdqa ymmword ptr [rsp + 576], ymm12
vpxor ymm4, ymm4, ymm9
vpsrld ymm15, ymm4, 7
vpslld ymm4, ymm4, 25
vpor ymm4, ymm4, ymm15
vpxor ymm0, ymm0, ymm8
vpxor ymm1, ymm1, ymm9
vpxor ymm2, ymm2, ymm10
vpxor ymm3, ymm3, ymm11
vpxor ymm4, ymm4, ymmword ptr [rsp + 512]
vpxor ymm5, ymm5, ymmword ptr [rsp + 544]
vpxor ymm6, ymm6, ymmword ptr [rsp + 576]
vpxor ymm7, ymm7, ymmword ptr [rsp + 608]
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
vmovdqu ymmword ptr [r9 + 0], ymm8
vmovdqu ymmword ptr [r9 + 32], ymm9
vperm2i128 ymm8, ymm1, ymm5, 0x20
vperm2i128 ymm9, ymm1, ymm5, 0x31
vmovdqu ymmword ptr [r9 + 64], ymm8
vmovdqu ymmword ptr [r9 + 96], ymm9
vperm2i128 ymm8, ymm2, ymm6, 0x20
vperm2i128 ymm9, ymm2, ymm6, 0x31
vmovdqu ymmword ptr [r9 + 128], ymm8
vmovdqu ymmword ptr [r9 + 160], ymm9
vperm2i128 ymm8, ymm3, ymm7, 0x20
vperm2i128 ymm9, ymm3, ymm7, 0x31
vmovdqu ymmword ptr [r9 + 192], ymm8
vmovdqu ymmword ptr [r9 + 224], ymm9
add r9, 256
dec rsi
jnz .L_probe_avx2_counter_range_36_loop
.L_probe_avx2_counter_range_36_done:
vzeroupper
mov rsp, rbp
pop rbp
ret
probe_avx2_counter_range_36_end:

.att_syntax prefix
