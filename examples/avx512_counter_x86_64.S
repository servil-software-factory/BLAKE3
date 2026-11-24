.intel_syntax noprefix

.text

.globl probe_avx512_counter_range
.p2align 4
probe_avx512_counter_range:
test rsi, rsi
jz .L_probe_avx512_counter_range_done
sub rsp, 64
mov dword ptr [rsp + 0], 0
mov dword ptr [rsp + 4], 4
mov dword ptr [rsp + 8], 8
mov dword ptr [rsp + 12], 12
mov dword ptr [rsp + 16], 1
mov dword ptr [rsp + 20], 5
mov dword ptr [rsp + 24], 9
mov dword ptr [rsp + 28], 13
mov dword ptr [rsp + 32], 2
mov dword ptr [rsp + 36], 6
mov dword ptr [rsp + 40], 10
mov dword ptr [rsp + 44], 14
mov dword ptr [rsp + 48], 3
mov dword ptr [rsp + 52], 7
mov dword ptr [rsp + 56], 11
mov dword ptr [rsp + 60], 15
vmovdqu32 zmm0, zmmword ptr [rsp]
add rsp, 64
vpbroadcastd zmm1, ecx
vpaddd zmm0, zmm0, zmm1
mov r10, rcx
shr r10, 32
mov r11d, r8d
and r11d, 0xff
shr r8d, 8
and r8d, 0xff
vpbroadcastd zmm16, dword ptr [rdi + 0]
vpbroadcastd zmm17, dword ptr [rdi + 4]
vpbroadcastd zmm18, dword ptr [rdi + 8]
vpbroadcastd zmm19, dword ptr [rdi + 12]
vpbroadcastd zmm20, dword ptr [rdi + 16]
vpbroadcastd zmm21, dword ptr [rdi + 20]
vpbroadcastd zmm22, dword ptr [rdi + 24]
vpbroadcastd zmm23, dword ptr [rdi + 28]
vpbroadcastd zmm24, dword ptr [rdi + 32]
vpbroadcastd zmm25, dword ptr [rdi + 36]
vpbroadcastd zmm26, dword ptr [rdi + 40]
vpbroadcastd zmm27, dword ptr [rdi + 44]
vpbroadcastd zmm28, dword ptr [rdi + 48]
vpbroadcastd zmm29, dword ptr [rdi + 52]
vpbroadcastd zmm30, dword ptr [rdi + 56]
vpbroadcastd zmm31, dword ptr [rdi + 60]
cmp r10, 0
je .L_probe_avx512_counter_range_set_0
cmp r10, 1
je .L_probe_avx512_counter_range_set_1
cmp r10, 2
je .L_probe_avx512_counter_range_set_2
cmp r10, 3
je .L_probe_avx512_counter_range_set_3
cmp r10, 4
je .L_probe_avx512_counter_range_set_4
cmp r10, 5
je .L_probe_avx512_counter_range_set_5
cmp r10, 6
je .L_probe_avx512_counter_range_set_6
cmp r10, 7
je .L_probe_avx512_counter_range_set_7
cmp r10, 8
je .L_probe_avx512_counter_range_set_8
cmp r10, 9
je .L_probe_avx512_counter_range_set_9
cmp r10, 10
je .L_probe_avx512_counter_range_set_10
cmp r10, 11
je .L_probe_avx512_counter_range_set_11
cmp r10, 12
je .L_probe_avx512_counter_range_set_12
cmp r10, 13
je .L_probe_avx512_counter_range_set_13
cmp r10, 14
je .L_probe_avx512_counter_range_set_14
cmp r10, 15
je .L_probe_avx512_counter_range_set_15
.L_probe_avx512_counter_range_set_0:
vmovdqa32 zmm16, zmm0
jmp .L_probe_avx512_counter_range_init_done
.L_probe_avx512_counter_range_set_1:
vmovdqa32 zmm17, zmm0
jmp .L_probe_avx512_counter_range_init_done
.L_probe_avx512_counter_range_set_2:
vmovdqa32 zmm18, zmm0
jmp .L_probe_avx512_counter_range_init_done
.L_probe_avx512_counter_range_set_3:
vmovdqa32 zmm19, zmm0
jmp .L_probe_avx512_counter_range_init_done
.L_probe_avx512_counter_range_set_4:
vmovdqa32 zmm20, zmm0
jmp .L_probe_avx512_counter_range_init_done
.L_probe_avx512_counter_range_set_5:
vmovdqa32 zmm21, zmm0
jmp .L_probe_avx512_counter_range_init_done
.L_probe_avx512_counter_range_set_6:
vmovdqa32 zmm22, zmm0
jmp .L_probe_avx512_counter_range_init_done
.L_probe_avx512_counter_range_set_7:
vmovdqa32 zmm23, zmm0
jmp .L_probe_avx512_counter_range_init_done
.L_probe_avx512_counter_range_set_8:
vmovdqa32 zmm24, zmm0
jmp .L_probe_avx512_counter_range_init_done
.L_probe_avx512_counter_range_set_9:
vmovdqa32 zmm25, zmm0
jmp .L_probe_avx512_counter_range_init_done
.L_probe_avx512_counter_range_set_10:
vmovdqa32 zmm26, zmm0
jmp .L_probe_avx512_counter_range_init_done
.L_probe_avx512_counter_range_set_11:
vmovdqa32 zmm27, zmm0
jmp .L_probe_avx512_counter_range_init_done
.L_probe_avx512_counter_range_set_12:
vmovdqa32 zmm28, zmm0
jmp .L_probe_avx512_counter_range_init_done
.L_probe_avx512_counter_range_set_13:
vmovdqa32 zmm29, zmm0
jmp .L_probe_avx512_counter_range_init_done
.L_probe_avx512_counter_range_set_14:
vmovdqa32 zmm30, zmm0
jmp .L_probe_avx512_counter_range_init_done
.L_probe_avx512_counter_range_set_15:
vmovdqa32 zmm31, zmm0
jmp .L_probe_avx512_counter_range_init_done
.L_probe_avx512_counter_range_init_done:
.L_probe_avx512_counter_range_loop:
vpbroadcastd zmm0, dword ptr [rdx + 0]
vpbroadcastd zmm1, dword ptr [rdx + 4]
vpbroadcastd zmm2, dword ptr [rdx + 8]
vpbroadcastd zmm3, dword ptr [rdx + 12]
vpbroadcastd zmm4, dword ptr [rdx + 16]
vpbroadcastd zmm5, dword ptr [rdx + 20]
vpbroadcastd zmm6, dword ptr [rdx + 24]
vpbroadcastd zmm7, dword ptr [rdx + 28]
mov eax, 1779033703
vpbroadcastd zmm8, eax
mov eax, 3144134277
vpbroadcastd zmm9, eax
mov eax, 1013904242
vpbroadcastd zmm10, eax
mov eax, 2773480762
vpbroadcastd zmm11, eax
vpxord zmm12, zmm12, zmm12
vpxord zmm13, zmm13, zmm13
vpbroadcastd zmm14, r8d
vpbroadcastd zmm15, r11d
vpaddd zmm0, zmm0, zmm16
vpaddd zmm1, zmm1, zmm18
vpaddd zmm2, zmm2, zmm20
vpaddd zmm3, zmm3, zmm22
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vprord zmm15, zmm15, 16
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 12
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vpaddd zmm0, zmm0, zmm17
vpaddd zmm1, zmm1, zmm19
vpaddd zmm2, zmm2, zmm21
vpaddd zmm3, zmm3, zmm23
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vprord zmm15, zmm15, 8
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 7
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vpaddd zmm0, zmm0, zmm24
vpaddd zmm1, zmm1, zmm26
vpaddd zmm2, zmm2, zmm28
vpaddd zmm3, zmm3, zmm30
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 16
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vprord zmm4, zmm4, 12
vpaddd zmm0, zmm0, zmm25
vpaddd zmm1, zmm1, zmm27
vpaddd zmm2, zmm2, zmm29
vpaddd zmm3, zmm3, zmm31
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 8
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vprord zmm4, zmm4, 7
vpaddd zmm0, zmm0, zmm18
vpaddd zmm1, zmm1, zmm19
vpaddd zmm2, zmm2, zmm23
vpaddd zmm3, zmm3, zmm20
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vprord zmm15, zmm15, 16
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 12
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vpaddd zmm0, zmm0, zmm22
vpaddd zmm1, zmm1, zmm26
vpaddd zmm2, zmm2, zmm16
vpaddd zmm3, zmm3, zmm29
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vprord zmm15, zmm15, 8
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 7
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vpaddd zmm0, zmm0, zmm17
vpaddd zmm1, zmm1, zmm28
vpaddd zmm2, zmm2, zmm25
vpaddd zmm3, zmm3, zmm31
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 16
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vprord zmm4, zmm4, 12
vpaddd zmm0, zmm0, zmm27
vpaddd zmm1, zmm1, zmm21
vpaddd zmm2, zmm2, zmm30
vpaddd zmm3, zmm3, zmm24
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 8
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vprord zmm4, zmm4, 7
vpaddd zmm0, zmm0, zmm19
vpaddd zmm1, zmm1, zmm26
vpaddd zmm2, zmm2, zmm29
vpaddd zmm3, zmm3, zmm23
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vprord zmm15, zmm15, 16
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 12
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vpaddd zmm0, zmm0, zmm20
vpaddd zmm1, zmm1, zmm28
vpaddd zmm2, zmm2, zmm18
vpaddd zmm3, zmm3, zmm30
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vprord zmm15, zmm15, 8
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 7
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vpaddd zmm0, zmm0, zmm22
vpaddd zmm1, zmm1, zmm25
vpaddd zmm2, zmm2, zmm27
vpaddd zmm3, zmm3, zmm24
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 16
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vprord zmm4, zmm4, 12
vpaddd zmm0, zmm0, zmm21
vpaddd zmm1, zmm1, zmm16
vpaddd zmm2, zmm2, zmm31
vpaddd zmm3, zmm3, zmm17
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 8
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vprord zmm4, zmm4, 7
vpaddd zmm0, zmm0, zmm26
vpaddd zmm1, zmm1, zmm28
vpaddd zmm2, zmm2, zmm30
vpaddd zmm3, zmm3, zmm29
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vprord zmm15, zmm15, 16
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 12
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vpaddd zmm0, zmm0, zmm23
vpaddd zmm1, zmm1, zmm25
vpaddd zmm2, zmm2, zmm19
vpaddd zmm3, zmm3, zmm31
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vprord zmm15, zmm15, 8
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 7
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vpaddd zmm0, zmm0, zmm20
vpaddd zmm1, zmm1, zmm27
vpaddd zmm2, zmm2, zmm21
vpaddd zmm3, zmm3, zmm17
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 16
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vprord zmm4, zmm4, 12
vpaddd zmm0, zmm0, zmm16
vpaddd zmm1, zmm1, zmm18
vpaddd zmm2, zmm2, zmm24
vpaddd zmm3, zmm3, zmm22
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 8
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vprord zmm4, zmm4, 7
vpaddd zmm0, zmm0, zmm28
vpaddd zmm1, zmm1, zmm25
vpaddd zmm2, zmm2, zmm31
vpaddd zmm3, zmm3, zmm30
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vprord zmm15, zmm15, 16
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 12
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vpaddd zmm0, zmm0, zmm29
vpaddd zmm1, zmm1, zmm27
vpaddd zmm2, zmm2, zmm26
vpaddd zmm3, zmm3, zmm24
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vprord zmm15, zmm15, 8
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 7
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vpaddd zmm0, zmm0, zmm23
vpaddd zmm1, zmm1, zmm21
vpaddd zmm2, zmm2, zmm16
vpaddd zmm3, zmm3, zmm22
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 16
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vprord zmm4, zmm4, 12
vpaddd zmm0, zmm0, zmm18
vpaddd zmm1, zmm1, zmm19
vpaddd zmm2, zmm2, zmm17
vpaddd zmm3, zmm3, zmm20
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 8
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vprord zmm4, zmm4, 7
vpaddd zmm0, zmm0, zmm25
vpaddd zmm1, zmm1, zmm27
vpaddd zmm2, zmm2, zmm24
vpaddd zmm3, zmm3, zmm31
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vprord zmm15, zmm15, 16
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 12
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vpaddd zmm0, zmm0, zmm30
vpaddd zmm1, zmm1, zmm21
vpaddd zmm2, zmm2, zmm28
vpaddd zmm3, zmm3, zmm17
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vprord zmm15, zmm15, 8
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 7
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vpaddd zmm0, zmm0, zmm29
vpaddd zmm1, zmm1, zmm16
vpaddd zmm2, zmm2, zmm18
vpaddd zmm3, zmm3, zmm20
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 16
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vprord zmm4, zmm4, 12
vpaddd zmm0, zmm0, zmm19
vpaddd zmm1, zmm1, zmm26
vpaddd zmm2, zmm2, zmm22
vpaddd zmm3, zmm3, zmm23
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 8
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vprord zmm4, zmm4, 7
vpaddd zmm0, zmm0, zmm27
vpaddd zmm1, zmm1, zmm21
vpaddd zmm2, zmm2, zmm17
vpaddd zmm3, zmm3, zmm24
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vprord zmm15, zmm15, 16
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 12
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vpaddd zmm0, zmm0, zmm31
vpaddd zmm1, zmm1, zmm16
vpaddd zmm2, zmm2, zmm25
vpaddd zmm3, zmm3, zmm22
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vprord zmm15, zmm15, 8
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 7
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vpaddd zmm0, zmm0, zmm30
vpaddd zmm1, zmm1, zmm18
vpaddd zmm2, zmm2, zmm19
vpaddd zmm3, zmm3, zmm23
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 16
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vprord zmm4, zmm4, 12
vpaddd zmm0, zmm0, zmm26
vpaddd zmm1, zmm1, zmm28
vpaddd zmm2, zmm2, zmm20
vpaddd zmm3, zmm3, zmm29
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 8
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vprord zmm4, zmm4, 7
vpxord zmm0, zmm0, zmm8
vpxord zmm1, zmm1, zmm9
vpxord zmm2, zmm2, zmm10
vpxord zmm3, zmm3, zmm11
vpxord zmm4, zmm4, zmm12
vpxord zmm5, zmm5, zmm13
vpxord zmm6, zmm6, zmm14
vpxord zmm7, zmm7, zmm15
vpunpckldq zmm8, zmm0, zmm1
vpunpckhdq zmm9, zmm0, zmm1
vpunpckldq zmm10, zmm2, zmm3
vpunpckhdq zmm11, zmm2, zmm3
vpunpcklqdq zmm0, zmm8, zmm10
vpunpckhqdq zmm1, zmm8, zmm10
vpunpcklqdq zmm2, zmm9, zmm11
vpunpckhqdq zmm3, zmm9, zmm11
vpunpckldq zmm8, zmm4, zmm5
vpunpckhdq zmm9, zmm4, zmm5
vpunpckldq zmm10, zmm6, zmm7
vpunpckhdq zmm11, zmm6, zmm7
vpunpcklqdq zmm4, zmm8, zmm10
vpunpckhqdq zmm5, zmm8, zmm10
vpunpcklqdq zmm6, zmm9, zmm11
vpunpckhqdq zmm7, zmm9, zmm11
vshufi32x4 zmm8, zmm0, zmm4, 0x44
vshufi32x4 zmm8, zmm8, zmm8, 0xd8
vmovdqu32 zmmword ptr [r9 + 0], zmm8
vshufi32x4 zmm9, zmm0, zmm4, 0xee
vshufi32x4 zmm9, zmm9, zmm9, 0xd8
vmovdqu32 zmmword ptr [r9 + 64], zmm9
vshufi32x4 zmm8, zmm1, zmm5, 0x44
vshufi32x4 zmm8, zmm8, zmm8, 0xd8
vmovdqu32 zmmword ptr [r9 + 128], zmm8
vshufi32x4 zmm9, zmm1, zmm5, 0xee
vshufi32x4 zmm9, zmm9, zmm9, 0xd8
vmovdqu32 zmmword ptr [r9 + 192], zmm9
vshufi32x4 zmm8, zmm2, zmm6, 0x44
vshufi32x4 zmm8, zmm8, zmm8, 0xd8
vmovdqu32 zmmword ptr [r9 + 256], zmm8
vshufi32x4 zmm9, zmm2, zmm6, 0xee
vshufi32x4 zmm9, zmm9, zmm9, 0xd8
vmovdqu32 zmmword ptr [r9 + 320], zmm9
vshufi32x4 zmm8, zmm3, zmm7, 0x44
vshufi32x4 zmm8, zmm8, zmm8, 0xd8
vmovdqu32 zmmword ptr [r9 + 384], zmm8
vshufi32x4 zmm9, zmm3, zmm7, 0xee
vshufi32x4 zmm9, zmm9, zmm9, 0xd8
vmovdqu32 zmmword ptr [r9 + 448], zmm9
add r9, 512
dec rsi
jz .L_probe_avx512_counter_range_done
mov eax, 16
vpbroadcastd zmm0, eax
cmp r10, 0
je .L_probe_avx512_counter_range_inc_0
cmp r10, 1
je .L_probe_avx512_counter_range_inc_1
cmp r10, 2
je .L_probe_avx512_counter_range_inc_2
cmp r10, 3
je .L_probe_avx512_counter_range_inc_3
cmp r10, 4
je .L_probe_avx512_counter_range_inc_4
cmp r10, 5
je .L_probe_avx512_counter_range_inc_5
cmp r10, 6
je .L_probe_avx512_counter_range_inc_6
cmp r10, 7
je .L_probe_avx512_counter_range_inc_7
cmp r10, 8
je .L_probe_avx512_counter_range_inc_8
cmp r10, 9
je .L_probe_avx512_counter_range_inc_9
cmp r10, 10
je .L_probe_avx512_counter_range_inc_10
cmp r10, 11
je .L_probe_avx512_counter_range_inc_11
cmp r10, 12
je .L_probe_avx512_counter_range_inc_12
cmp r10, 13
je .L_probe_avx512_counter_range_inc_13
cmp r10, 14
je .L_probe_avx512_counter_range_inc_14
cmp r10, 15
je .L_probe_avx512_counter_range_inc_15
.L_probe_avx512_counter_range_inc_0:
vpaddd zmm16, zmm16, zmm0
jmp .L_probe_avx512_counter_range_loop
.L_probe_avx512_counter_range_inc_1:
vpaddd zmm17, zmm17, zmm0
jmp .L_probe_avx512_counter_range_loop
.L_probe_avx512_counter_range_inc_2:
vpaddd zmm18, zmm18, zmm0
jmp .L_probe_avx512_counter_range_loop
.L_probe_avx512_counter_range_inc_3:
vpaddd zmm19, zmm19, zmm0
jmp .L_probe_avx512_counter_range_loop
.L_probe_avx512_counter_range_inc_4:
vpaddd zmm20, zmm20, zmm0
jmp .L_probe_avx512_counter_range_loop
.L_probe_avx512_counter_range_inc_5:
vpaddd zmm21, zmm21, zmm0
jmp .L_probe_avx512_counter_range_loop
.L_probe_avx512_counter_range_inc_6:
vpaddd zmm22, zmm22, zmm0
jmp .L_probe_avx512_counter_range_loop
.L_probe_avx512_counter_range_inc_7:
vpaddd zmm23, zmm23, zmm0
jmp .L_probe_avx512_counter_range_loop
.L_probe_avx512_counter_range_inc_8:
vpaddd zmm24, zmm24, zmm0
jmp .L_probe_avx512_counter_range_loop
.L_probe_avx512_counter_range_inc_9:
vpaddd zmm25, zmm25, zmm0
jmp .L_probe_avx512_counter_range_loop
.L_probe_avx512_counter_range_inc_10:
vpaddd zmm26, zmm26, zmm0
jmp .L_probe_avx512_counter_range_loop
.L_probe_avx512_counter_range_inc_11:
vpaddd zmm27, zmm27, zmm0
jmp .L_probe_avx512_counter_range_loop
.L_probe_avx512_counter_range_inc_12:
vpaddd zmm28, zmm28, zmm0
jmp .L_probe_avx512_counter_range_loop
.L_probe_avx512_counter_range_inc_13:
vpaddd zmm29, zmm29, zmm0
jmp .L_probe_avx512_counter_range_loop
.L_probe_avx512_counter_range_inc_14:
vpaddd zmm30, zmm30, zmm0
jmp .L_probe_avx512_counter_range_loop
.L_probe_avx512_counter_range_inc_15:
vpaddd zmm31, zmm31, zmm0
jmp .L_probe_avx512_counter_range_loop
.L_probe_avx512_counter_range_done:
vzeroupper
ret
probe_avx512_counter_range_end:

.globl probe_avx512_counter_range_52
.p2align 4
probe_avx512_counter_range_52:
test rsi, rsi
jz .L_probe_avx512_counter_range_52_done
sub rsp, 64
mov dword ptr [rsp + 0], 0
mov dword ptr [rsp + 4], 4
mov dword ptr [rsp + 8], 8
mov dword ptr [rsp + 12], 12
mov dword ptr [rsp + 16], 1
mov dword ptr [rsp + 20], 5
mov dword ptr [rsp + 24], 9
mov dword ptr [rsp + 28], 13
mov dword ptr [rsp + 32], 2
mov dword ptr [rsp + 36], 6
mov dword ptr [rsp + 40], 10
mov dword ptr [rsp + 44], 14
mov dword ptr [rsp + 48], 3
mov dword ptr [rsp + 52], 7
mov dword ptr [rsp + 56], 11
mov dword ptr [rsp + 60], 15
vmovdqu32 zmm0, zmmword ptr [rsp]
add rsp, 64
vpbroadcastd zmm1, ecx
vpaddd zmm0, zmm0, zmm1
mov r10, rcx
shr r10, 32
mov r11d, r8d
and r11d, 0xff
shr r8d, 8
and r8d, 0xff
vpbroadcastd zmm16, dword ptr [rdi + 0]
vpbroadcastd zmm17, dword ptr [rdi + 4]
vpbroadcastd zmm18, dword ptr [rdi + 8]
vpbroadcastd zmm19, dword ptr [rdi + 12]
vpbroadcastd zmm20, dword ptr [rdi + 16]
vpbroadcastd zmm21, dword ptr [rdi + 20]
vpbroadcastd zmm22, dword ptr [rdi + 24]
vpbroadcastd zmm23, dword ptr [rdi + 28]
vpbroadcastd zmm24, dword ptr [rdi + 32]
vpbroadcastd zmm25, dword ptr [rdi + 36]
vpbroadcastd zmm26, dword ptr [rdi + 40]
vpbroadcastd zmm27, dword ptr [rdi + 44]
vpbroadcastd zmm28, dword ptr [rdi + 48]
cmp r10, 0
je .L_probe_avx512_counter_range_52_set_0
cmp r10, 1
je .L_probe_avx512_counter_range_52_set_1
cmp r10, 2
je .L_probe_avx512_counter_range_52_set_2
cmp r10, 3
je .L_probe_avx512_counter_range_52_set_3
cmp r10, 4
je .L_probe_avx512_counter_range_52_set_4
cmp r10, 5
je .L_probe_avx512_counter_range_52_set_5
cmp r10, 6
je .L_probe_avx512_counter_range_52_set_6
cmp r10, 7
je .L_probe_avx512_counter_range_52_set_7
cmp r10, 8
je .L_probe_avx512_counter_range_52_set_8
cmp r10, 9
je .L_probe_avx512_counter_range_52_set_9
cmp r10, 10
je .L_probe_avx512_counter_range_52_set_10
cmp r10, 11
je .L_probe_avx512_counter_range_52_set_11
cmp r10, 12
je .L_probe_avx512_counter_range_52_set_12
.L_probe_avx512_counter_range_52_set_0:
vmovdqa32 zmm16, zmm0
jmp .L_probe_avx512_counter_range_52_init_done
.L_probe_avx512_counter_range_52_set_1:
vmovdqa32 zmm17, zmm0
jmp .L_probe_avx512_counter_range_52_init_done
.L_probe_avx512_counter_range_52_set_2:
vmovdqa32 zmm18, zmm0
jmp .L_probe_avx512_counter_range_52_init_done
.L_probe_avx512_counter_range_52_set_3:
vmovdqa32 zmm19, zmm0
jmp .L_probe_avx512_counter_range_52_init_done
.L_probe_avx512_counter_range_52_set_4:
vmovdqa32 zmm20, zmm0
jmp .L_probe_avx512_counter_range_52_init_done
.L_probe_avx512_counter_range_52_set_5:
vmovdqa32 zmm21, zmm0
jmp .L_probe_avx512_counter_range_52_init_done
.L_probe_avx512_counter_range_52_set_6:
vmovdqa32 zmm22, zmm0
jmp .L_probe_avx512_counter_range_52_init_done
.L_probe_avx512_counter_range_52_set_7:
vmovdqa32 zmm23, zmm0
jmp .L_probe_avx512_counter_range_52_init_done
.L_probe_avx512_counter_range_52_set_8:
vmovdqa32 zmm24, zmm0
jmp .L_probe_avx512_counter_range_52_init_done
.L_probe_avx512_counter_range_52_set_9:
vmovdqa32 zmm25, zmm0
jmp .L_probe_avx512_counter_range_52_init_done
.L_probe_avx512_counter_range_52_set_10:
vmovdqa32 zmm26, zmm0
jmp .L_probe_avx512_counter_range_52_init_done
.L_probe_avx512_counter_range_52_set_11:
vmovdqa32 zmm27, zmm0
jmp .L_probe_avx512_counter_range_52_init_done
.L_probe_avx512_counter_range_52_set_12:
vmovdqa32 zmm28, zmm0
jmp .L_probe_avx512_counter_range_52_init_done
.L_probe_avx512_counter_range_52_init_done:
.L_probe_avx512_counter_range_52_loop:
vpbroadcastd zmm0, dword ptr [rdx + 0]
vpbroadcastd zmm1, dword ptr [rdx + 4]
vpbroadcastd zmm2, dword ptr [rdx + 8]
vpbroadcastd zmm3, dword ptr [rdx + 12]
vpbroadcastd zmm4, dword ptr [rdx + 16]
vpbroadcastd zmm5, dword ptr [rdx + 20]
vpbroadcastd zmm6, dword ptr [rdx + 24]
vpbroadcastd zmm7, dword ptr [rdx + 28]
mov eax, 1779033703
vpbroadcastd zmm8, eax
mov eax, 3144134277
vpbroadcastd zmm9, eax
mov eax, 1013904242
vpbroadcastd zmm10, eax
mov eax, 2773480762
vpbroadcastd zmm11, eax
vpxord zmm12, zmm12, zmm12
vpxord zmm13, zmm13, zmm13
vpbroadcastd zmm14, r8d
vpbroadcastd zmm15, r11d
vpaddd zmm0, zmm0, zmm16
vpaddd zmm1, zmm1, zmm18
vpaddd zmm2, zmm2, zmm20
vpaddd zmm3, zmm3, zmm22
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vprord zmm15, zmm15, 16
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 12
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vpaddd zmm0, zmm0, zmm17
vpaddd zmm1, zmm1, zmm19
vpaddd zmm2, zmm2, zmm21
vpaddd zmm3, zmm3, zmm23
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vprord zmm15, zmm15, 8
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 7
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vpaddd zmm0, zmm0, zmm24
vpaddd zmm1, zmm1, zmm26
vpaddd zmm2, zmm2, zmm28
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 16
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vprord zmm4, zmm4, 12
vpaddd zmm0, zmm0, zmm25
vpaddd zmm1, zmm1, zmm27
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 8
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vprord zmm4, zmm4, 7
vpaddd zmm0, zmm0, zmm18
vpaddd zmm1, zmm1, zmm19
vpaddd zmm2, zmm2, zmm23
vpaddd zmm3, zmm3, zmm20
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vprord zmm15, zmm15, 16
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 12
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vpaddd zmm0, zmm0, zmm22
vpaddd zmm1, zmm1, zmm26
vpaddd zmm2, zmm2, zmm16
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vprord zmm15, zmm15, 8
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 7
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vpaddd zmm0, zmm0, zmm17
vpaddd zmm1, zmm1, zmm28
vpaddd zmm2, zmm2, zmm25
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 16
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vprord zmm4, zmm4, 12
vpaddd zmm0, zmm0, zmm27
vpaddd zmm1, zmm1, zmm21
vpaddd zmm3, zmm3, zmm24
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 8
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vprord zmm4, zmm4, 7
vpaddd zmm0, zmm0, zmm19
vpaddd zmm1, zmm1, zmm26
vpaddd zmm3, zmm3, zmm23
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vprord zmm15, zmm15, 16
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 12
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vpaddd zmm0, zmm0, zmm20
vpaddd zmm1, zmm1, zmm28
vpaddd zmm2, zmm2, zmm18
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vprord zmm15, zmm15, 8
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 7
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vpaddd zmm0, zmm0, zmm22
vpaddd zmm1, zmm1, zmm25
vpaddd zmm2, zmm2, zmm27
vpaddd zmm3, zmm3, zmm24
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 16
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vprord zmm4, zmm4, 12
vpaddd zmm0, zmm0, zmm21
vpaddd zmm1, zmm1, zmm16
vpaddd zmm3, zmm3, zmm17
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 8
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vprord zmm4, zmm4, 7
vpaddd zmm0, zmm0, zmm26
vpaddd zmm1, zmm1, zmm28
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vprord zmm15, zmm15, 16
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 12
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vpaddd zmm0, zmm0, zmm23
vpaddd zmm1, zmm1, zmm25
vpaddd zmm2, zmm2, zmm19
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vprord zmm15, zmm15, 8
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 7
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vpaddd zmm0, zmm0, zmm20
vpaddd zmm1, zmm1, zmm27
vpaddd zmm2, zmm2, zmm21
vpaddd zmm3, zmm3, zmm17
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 16
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vprord zmm4, zmm4, 12
vpaddd zmm0, zmm0, zmm16
vpaddd zmm1, zmm1, zmm18
vpaddd zmm2, zmm2, zmm24
vpaddd zmm3, zmm3, zmm22
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 8
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vprord zmm4, zmm4, 7
vpaddd zmm0, zmm0, zmm28
vpaddd zmm1, zmm1, zmm25
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vprord zmm15, zmm15, 16
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 12
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vpaddd zmm1, zmm1, zmm27
vpaddd zmm2, zmm2, zmm26
vpaddd zmm3, zmm3, zmm24
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vprord zmm15, zmm15, 8
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 7
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vpaddd zmm0, zmm0, zmm23
vpaddd zmm1, zmm1, zmm21
vpaddd zmm2, zmm2, zmm16
vpaddd zmm3, zmm3, zmm22
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 16
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vprord zmm4, zmm4, 12
vpaddd zmm0, zmm0, zmm18
vpaddd zmm1, zmm1, zmm19
vpaddd zmm2, zmm2, zmm17
vpaddd zmm3, zmm3, zmm20
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 8
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vprord zmm4, zmm4, 7
vpaddd zmm0, zmm0, zmm25
vpaddd zmm1, zmm1, zmm27
vpaddd zmm2, zmm2, zmm24
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vprord zmm15, zmm15, 16
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 12
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vpaddd zmm1, zmm1, zmm21
vpaddd zmm2, zmm2, zmm28
vpaddd zmm3, zmm3, zmm17
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vprord zmm15, zmm15, 8
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 7
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vpaddd zmm1, zmm1, zmm16
vpaddd zmm2, zmm2, zmm18
vpaddd zmm3, zmm3, zmm20
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 16
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vprord zmm4, zmm4, 12
vpaddd zmm0, zmm0, zmm19
vpaddd zmm1, zmm1, zmm26
vpaddd zmm2, zmm2, zmm22
vpaddd zmm3, zmm3, zmm23
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 8
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vprord zmm4, zmm4, 7
vpaddd zmm0, zmm0, zmm27
vpaddd zmm1, zmm1, zmm21
vpaddd zmm2, zmm2, zmm17
vpaddd zmm3, zmm3, zmm24
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vprord zmm15, zmm15, 16
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 12
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vpaddd zmm1, zmm1, zmm16
vpaddd zmm2, zmm2, zmm25
vpaddd zmm3, zmm3, zmm22
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vprord zmm15, zmm15, 8
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 7
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vpaddd zmm1, zmm1, zmm18
vpaddd zmm2, zmm2, zmm19
vpaddd zmm3, zmm3, zmm23
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 16
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vprord zmm4, zmm4, 12
vpaddd zmm0, zmm0, zmm26
vpaddd zmm1, zmm1, zmm28
vpaddd zmm2, zmm2, zmm20
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 8
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vprord zmm4, zmm4, 7
vpxord zmm0, zmm0, zmm8
vpxord zmm1, zmm1, zmm9
vpxord zmm2, zmm2, zmm10
vpxord zmm3, zmm3, zmm11
vpxord zmm4, zmm4, zmm12
vpxord zmm5, zmm5, zmm13
vpxord zmm6, zmm6, zmm14
vpxord zmm7, zmm7, zmm15
vpunpckldq zmm8, zmm0, zmm1
vpunpckhdq zmm9, zmm0, zmm1
vpunpckldq zmm10, zmm2, zmm3
vpunpckhdq zmm11, zmm2, zmm3
vpunpcklqdq zmm0, zmm8, zmm10
vpunpckhqdq zmm1, zmm8, zmm10
vpunpcklqdq zmm2, zmm9, zmm11
vpunpckhqdq zmm3, zmm9, zmm11
vpunpckldq zmm8, zmm4, zmm5
vpunpckhdq zmm9, zmm4, zmm5
vpunpckldq zmm10, zmm6, zmm7
vpunpckhdq zmm11, zmm6, zmm7
vpunpcklqdq zmm4, zmm8, zmm10
vpunpckhqdq zmm5, zmm8, zmm10
vpunpcklqdq zmm6, zmm9, zmm11
vpunpckhqdq zmm7, zmm9, zmm11
vshufi32x4 zmm8, zmm0, zmm4, 0x44
vshufi32x4 zmm8, zmm8, zmm8, 0xd8
vmovdqu32 zmmword ptr [r9 + 0], zmm8
vshufi32x4 zmm9, zmm0, zmm4, 0xee
vshufi32x4 zmm9, zmm9, zmm9, 0xd8
vmovdqu32 zmmword ptr [r9 + 64], zmm9
vshufi32x4 zmm8, zmm1, zmm5, 0x44
vshufi32x4 zmm8, zmm8, zmm8, 0xd8
vmovdqu32 zmmword ptr [r9 + 128], zmm8
vshufi32x4 zmm9, zmm1, zmm5, 0xee
vshufi32x4 zmm9, zmm9, zmm9, 0xd8
vmovdqu32 zmmword ptr [r9 + 192], zmm9
vshufi32x4 zmm8, zmm2, zmm6, 0x44
vshufi32x4 zmm8, zmm8, zmm8, 0xd8
vmovdqu32 zmmword ptr [r9 + 256], zmm8
vshufi32x4 zmm9, zmm2, zmm6, 0xee
vshufi32x4 zmm9, zmm9, zmm9, 0xd8
vmovdqu32 zmmword ptr [r9 + 320], zmm9
vshufi32x4 zmm8, zmm3, zmm7, 0x44
vshufi32x4 zmm8, zmm8, zmm8, 0xd8
vmovdqu32 zmmword ptr [r9 + 384], zmm8
vshufi32x4 zmm9, zmm3, zmm7, 0xee
vshufi32x4 zmm9, zmm9, zmm9, 0xd8
vmovdqu32 zmmword ptr [r9 + 448], zmm9
add r9, 512
dec rsi
jz .L_probe_avx512_counter_range_52_done
mov eax, 16
vpbroadcastd zmm0, eax
cmp r10, 0
je .L_probe_avx512_counter_range_52_inc_0
cmp r10, 1
je .L_probe_avx512_counter_range_52_inc_1
cmp r10, 2
je .L_probe_avx512_counter_range_52_inc_2
cmp r10, 3
je .L_probe_avx512_counter_range_52_inc_3
cmp r10, 4
je .L_probe_avx512_counter_range_52_inc_4
cmp r10, 5
je .L_probe_avx512_counter_range_52_inc_5
cmp r10, 6
je .L_probe_avx512_counter_range_52_inc_6
cmp r10, 7
je .L_probe_avx512_counter_range_52_inc_7
cmp r10, 8
je .L_probe_avx512_counter_range_52_inc_8
cmp r10, 9
je .L_probe_avx512_counter_range_52_inc_9
cmp r10, 10
je .L_probe_avx512_counter_range_52_inc_10
cmp r10, 11
je .L_probe_avx512_counter_range_52_inc_11
cmp r10, 12
je .L_probe_avx512_counter_range_52_inc_12
.L_probe_avx512_counter_range_52_inc_0:
vpaddd zmm16, zmm16, zmm0
jmp .L_probe_avx512_counter_range_52_loop
.L_probe_avx512_counter_range_52_inc_1:
vpaddd zmm17, zmm17, zmm0
jmp .L_probe_avx512_counter_range_52_loop
.L_probe_avx512_counter_range_52_inc_2:
vpaddd zmm18, zmm18, zmm0
jmp .L_probe_avx512_counter_range_52_loop
.L_probe_avx512_counter_range_52_inc_3:
vpaddd zmm19, zmm19, zmm0
jmp .L_probe_avx512_counter_range_52_loop
.L_probe_avx512_counter_range_52_inc_4:
vpaddd zmm20, zmm20, zmm0
jmp .L_probe_avx512_counter_range_52_loop
.L_probe_avx512_counter_range_52_inc_5:
vpaddd zmm21, zmm21, zmm0
jmp .L_probe_avx512_counter_range_52_loop
.L_probe_avx512_counter_range_52_inc_6:
vpaddd zmm22, zmm22, zmm0
jmp .L_probe_avx512_counter_range_52_loop
.L_probe_avx512_counter_range_52_inc_7:
vpaddd zmm23, zmm23, zmm0
jmp .L_probe_avx512_counter_range_52_loop
.L_probe_avx512_counter_range_52_inc_8:
vpaddd zmm24, zmm24, zmm0
jmp .L_probe_avx512_counter_range_52_loop
.L_probe_avx512_counter_range_52_inc_9:
vpaddd zmm25, zmm25, zmm0
jmp .L_probe_avx512_counter_range_52_loop
.L_probe_avx512_counter_range_52_inc_10:
vpaddd zmm26, zmm26, zmm0
jmp .L_probe_avx512_counter_range_52_loop
.L_probe_avx512_counter_range_52_inc_11:
vpaddd zmm27, zmm27, zmm0
jmp .L_probe_avx512_counter_range_52_loop
.L_probe_avx512_counter_range_52_inc_12:
vpaddd zmm28, zmm28, zmm0
jmp .L_probe_avx512_counter_range_52_loop
.L_probe_avx512_counter_range_52_done:
vzeroupper
ret
probe_avx512_counter_range_52_end:

.globl probe_avx512_counter_range_36
.p2align 4
probe_avx512_counter_range_36:
test rsi, rsi
jz .L_probe_avx512_counter_range_36_done
sub rsp, 64
mov dword ptr [rsp + 0], 0
mov dword ptr [rsp + 4], 4
mov dword ptr [rsp + 8], 8
mov dword ptr [rsp + 12], 12
mov dword ptr [rsp + 16], 1
mov dword ptr [rsp + 20], 5
mov dword ptr [rsp + 24], 9
mov dword ptr [rsp + 28], 13
mov dword ptr [rsp + 32], 2
mov dword ptr [rsp + 36], 6
mov dword ptr [rsp + 40], 10
mov dword ptr [rsp + 44], 14
mov dword ptr [rsp + 48], 3
mov dword ptr [rsp + 52], 7
mov dword ptr [rsp + 56], 11
mov dword ptr [rsp + 60], 15
vmovdqu32 zmm0, zmmword ptr [rsp]
add rsp, 64
vpbroadcastd zmm1, ecx
vpaddd zmm0, zmm0, zmm1
mov r10, rcx
shr r10, 32
mov r11d, r8d
and r11d, 0xff
shr r8d, 8
and r8d, 0xff
vpbroadcastd zmm16, dword ptr [rdi + 0]
vpbroadcastd zmm17, dword ptr [rdi + 4]
vpbroadcastd zmm18, dword ptr [rdi + 8]
vpbroadcastd zmm19, dword ptr [rdi + 12]
vpbroadcastd zmm20, dword ptr [rdi + 16]
vpbroadcastd zmm21, dword ptr [rdi + 20]
vpbroadcastd zmm22, dword ptr [rdi + 24]
vpbroadcastd zmm23, dword ptr [rdi + 28]
vpbroadcastd zmm24, dword ptr [rdi + 32]
cmp r10, 0
je .L_probe_avx512_counter_range_36_set_0
cmp r10, 1
je .L_probe_avx512_counter_range_36_set_1
cmp r10, 2
je .L_probe_avx512_counter_range_36_set_2
cmp r10, 3
je .L_probe_avx512_counter_range_36_set_3
cmp r10, 4
je .L_probe_avx512_counter_range_36_set_4
cmp r10, 5
je .L_probe_avx512_counter_range_36_set_5
cmp r10, 6
je .L_probe_avx512_counter_range_36_set_6
cmp r10, 7
je .L_probe_avx512_counter_range_36_set_7
cmp r10, 8
je .L_probe_avx512_counter_range_36_set_8
.L_probe_avx512_counter_range_36_set_0:
vmovdqa32 zmm16, zmm0
jmp .L_probe_avx512_counter_range_36_init_done
.L_probe_avx512_counter_range_36_set_1:
vmovdqa32 zmm17, zmm0
jmp .L_probe_avx512_counter_range_36_init_done
.L_probe_avx512_counter_range_36_set_2:
vmovdqa32 zmm18, zmm0
jmp .L_probe_avx512_counter_range_36_init_done
.L_probe_avx512_counter_range_36_set_3:
vmovdqa32 zmm19, zmm0
jmp .L_probe_avx512_counter_range_36_init_done
.L_probe_avx512_counter_range_36_set_4:
vmovdqa32 zmm20, zmm0
jmp .L_probe_avx512_counter_range_36_init_done
.L_probe_avx512_counter_range_36_set_5:
vmovdqa32 zmm21, zmm0
jmp .L_probe_avx512_counter_range_36_init_done
.L_probe_avx512_counter_range_36_set_6:
vmovdqa32 zmm22, zmm0
jmp .L_probe_avx512_counter_range_36_init_done
.L_probe_avx512_counter_range_36_set_7:
vmovdqa32 zmm23, zmm0
jmp .L_probe_avx512_counter_range_36_init_done
.L_probe_avx512_counter_range_36_set_8:
vmovdqa32 zmm24, zmm0
jmp .L_probe_avx512_counter_range_36_init_done
.L_probe_avx512_counter_range_36_init_done:
.L_probe_avx512_counter_range_36_loop:
vpbroadcastd zmm0, dword ptr [rdx + 0]
vpbroadcastd zmm1, dword ptr [rdx + 4]
vpbroadcastd zmm2, dword ptr [rdx + 8]
vpbroadcastd zmm3, dword ptr [rdx + 12]
vpbroadcastd zmm4, dword ptr [rdx + 16]
vpbroadcastd zmm5, dword ptr [rdx + 20]
vpbroadcastd zmm6, dword ptr [rdx + 24]
vpbroadcastd zmm7, dword ptr [rdx + 28]
mov eax, 1779033703
vpbroadcastd zmm8, eax
mov eax, 3144134277
vpbroadcastd zmm9, eax
mov eax, 1013904242
vpbroadcastd zmm10, eax
mov eax, 2773480762
vpbroadcastd zmm11, eax
vpxord zmm12, zmm12, zmm12
vpxord zmm13, zmm13, zmm13
vpbroadcastd zmm14, r8d
vpbroadcastd zmm15, r11d
vpaddd zmm0, zmm0, zmm16
vpaddd zmm1, zmm1, zmm18
vpaddd zmm2, zmm2, zmm20
vpaddd zmm3, zmm3, zmm22
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vprord zmm15, zmm15, 16
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 12
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vpaddd zmm0, zmm0, zmm17
vpaddd zmm1, zmm1, zmm19
vpaddd zmm2, zmm2, zmm21
vpaddd zmm3, zmm3, zmm23
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vprord zmm15, zmm15, 8
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 7
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vpaddd zmm0, zmm0, zmm24
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 16
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vprord zmm4, zmm4, 12
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 8
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vprord zmm4, zmm4, 7
vpaddd zmm0, zmm0, zmm18
vpaddd zmm1, zmm1, zmm19
vpaddd zmm2, zmm2, zmm23
vpaddd zmm3, zmm3, zmm20
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vprord zmm15, zmm15, 16
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 12
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vpaddd zmm0, zmm0, zmm22
vpaddd zmm2, zmm2, zmm16
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vprord zmm15, zmm15, 8
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 7
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vpaddd zmm0, zmm0, zmm17
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 16
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vprord zmm4, zmm4, 12
vpaddd zmm1, zmm1, zmm21
vpaddd zmm3, zmm3, zmm24
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 8
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vprord zmm4, zmm4, 7
vpaddd zmm0, zmm0, zmm19
vpaddd zmm3, zmm3, zmm23
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vprord zmm15, zmm15, 16
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 12
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vpaddd zmm0, zmm0, zmm20
vpaddd zmm2, zmm2, zmm18
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vprord zmm15, zmm15, 8
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 7
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vpaddd zmm0, zmm0, zmm22
vpaddd zmm3, zmm3, zmm24
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 16
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vprord zmm4, zmm4, 12
vpaddd zmm0, zmm0, zmm21
vpaddd zmm1, zmm1, zmm16
vpaddd zmm3, zmm3, zmm17
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 8
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vprord zmm4, zmm4, 7
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vprord zmm15, zmm15, 16
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 12
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vpaddd zmm0, zmm0, zmm23
vpaddd zmm2, zmm2, zmm19
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vprord zmm15, zmm15, 8
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 7
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vpaddd zmm0, zmm0, zmm20
vpaddd zmm2, zmm2, zmm21
vpaddd zmm3, zmm3, zmm17
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 16
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vprord zmm4, zmm4, 12
vpaddd zmm0, zmm0, zmm16
vpaddd zmm1, zmm1, zmm18
vpaddd zmm2, zmm2, zmm24
vpaddd zmm3, zmm3, zmm22
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 8
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vprord zmm4, zmm4, 7
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vprord zmm15, zmm15, 16
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 12
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vpaddd zmm3, zmm3, zmm24
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vprord zmm15, zmm15, 8
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 7
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vpaddd zmm0, zmm0, zmm23
vpaddd zmm1, zmm1, zmm21
vpaddd zmm2, zmm2, zmm16
vpaddd zmm3, zmm3, zmm22
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 16
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vprord zmm4, zmm4, 12
vpaddd zmm0, zmm0, zmm18
vpaddd zmm1, zmm1, zmm19
vpaddd zmm2, zmm2, zmm17
vpaddd zmm3, zmm3, zmm20
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 8
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vprord zmm4, zmm4, 7
vpaddd zmm2, zmm2, zmm24
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vprord zmm15, zmm15, 16
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 12
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vpaddd zmm1, zmm1, zmm21
vpaddd zmm3, zmm3, zmm17
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vprord zmm15, zmm15, 8
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 7
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vpaddd zmm1, zmm1, zmm16
vpaddd zmm2, zmm2, zmm18
vpaddd zmm3, zmm3, zmm20
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 16
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vprord zmm4, zmm4, 12
vpaddd zmm0, zmm0, zmm19
vpaddd zmm2, zmm2, zmm22
vpaddd zmm3, zmm3, zmm23
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 8
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vprord zmm4, zmm4, 7
vpaddd zmm1, zmm1, zmm21
vpaddd zmm2, zmm2, zmm17
vpaddd zmm3, zmm3, zmm24
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vprord zmm15, zmm15, 16
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 12
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vpaddd zmm1, zmm1, zmm16
vpaddd zmm3, zmm3, zmm22
vpaddd zmm0, zmm0, zmm4
vpaddd zmm1, zmm1, zmm5
vpaddd zmm2, zmm2, zmm6
vpaddd zmm3, zmm3, zmm7
vpxord zmm12, zmm12, zmm0
vpxord zmm13, zmm13, zmm1
vpxord zmm14, zmm14, zmm2
vpxord zmm15, zmm15, zmm3
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vprord zmm15, zmm15, 8
vpaddd zmm8, zmm8, zmm12
vpaddd zmm9, zmm9, zmm13
vpaddd zmm10, zmm10, zmm14
vpaddd zmm11, zmm11, zmm15
vpxord zmm4, zmm4, zmm8
vpxord zmm5, zmm5, zmm9
vpxord zmm6, zmm6, zmm10
vpxord zmm7, zmm7, zmm11
vprord zmm4, zmm4, 7
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vpaddd zmm1, zmm1, zmm18
vpaddd zmm2, zmm2, zmm19
vpaddd zmm3, zmm3, zmm23
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 16
vprord zmm12, zmm12, 16
vprord zmm13, zmm13, 16
vprord zmm14, zmm14, 16
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 12
vprord zmm6, zmm6, 12
vprord zmm7, zmm7, 12
vprord zmm4, zmm4, 12
vpaddd zmm2, zmm2, zmm20
vpaddd zmm0, zmm0, zmm5
vpaddd zmm1, zmm1, zmm6
vpaddd zmm2, zmm2, zmm7
vpaddd zmm3, zmm3, zmm4
vpxord zmm15, zmm15, zmm0
vpxord zmm12, zmm12, zmm1
vpxord zmm13, zmm13, zmm2
vpxord zmm14, zmm14, zmm3
vprord zmm15, zmm15, 8
vprord zmm12, zmm12, 8
vprord zmm13, zmm13, 8
vprord zmm14, zmm14, 8
vpaddd zmm10, zmm10, zmm15
vpaddd zmm11, zmm11, zmm12
vpaddd zmm8, zmm8, zmm13
vpaddd zmm9, zmm9, zmm14
vpxord zmm5, zmm5, zmm10
vpxord zmm6, zmm6, zmm11
vpxord zmm7, zmm7, zmm8
vpxord zmm4, zmm4, zmm9
vprord zmm5, zmm5, 7
vprord zmm6, zmm6, 7
vprord zmm7, zmm7, 7
vprord zmm4, zmm4, 7
vpxord zmm0, zmm0, zmm8
vpxord zmm1, zmm1, zmm9
vpxord zmm2, zmm2, zmm10
vpxord zmm3, zmm3, zmm11
vpxord zmm4, zmm4, zmm12
vpxord zmm5, zmm5, zmm13
vpxord zmm6, zmm6, zmm14
vpxord zmm7, zmm7, zmm15
vpunpckldq zmm8, zmm0, zmm1
vpunpckhdq zmm9, zmm0, zmm1
vpunpckldq zmm10, zmm2, zmm3
vpunpckhdq zmm11, zmm2, zmm3
vpunpcklqdq zmm0, zmm8, zmm10
vpunpckhqdq zmm1, zmm8, zmm10
vpunpcklqdq zmm2, zmm9, zmm11
vpunpckhqdq zmm3, zmm9, zmm11
vpunpckldq zmm8, zmm4, zmm5
vpunpckhdq zmm9, zmm4, zmm5
vpunpckldq zmm10, zmm6, zmm7
vpunpckhdq zmm11, zmm6, zmm7
vpunpcklqdq zmm4, zmm8, zmm10
vpunpckhqdq zmm5, zmm8, zmm10
vpunpcklqdq zmm6, zmm9, zmm11
vpunpckhqdq zmm7, zmm9, zmm11
vshufi32x4 zmm8, zmm0, zmm4, 0x44
vshufi32x4 zmm8, zmm8, zmm8, 0xd8
vmovdqu32 zmmword ptr [r9 + 0], zmm8
vshufi32x4 zmm9, zmm0, zmm4, 0xee
vshufi32x4 zmm9, zmm9, zmm9, 0xd8
vmovdqu32 zmmword ptr [r9 + 64], zmm9
vshufi32x4 zmm8, zmm1, zmm5, 0x44
vshufi32x4 zmm8, zmm8, zmm8, 0xd8
vmovdqu32 zmmword ptr [r9 + 128], zmm8
vshufi32x4 zmm9, zmm1, zmm5, 0xee
vshufi32x4 zmm9, zmm9, zmm9, 0xd8
vmovdqu32 zmmword ptr [r9 + 192], zmm9
vshufi32x4 zmm8, zmm2, zmm6, 0x44
vshufi32x4 zmm8, zmm8, zmm8, 0xd8
vmovdqu32 zmmword ptr [r9 + 256], zmm8
vshufi32x4 zmm9, zmm2, zmm6, 0xee
vshufi32x4 zmm9, zmm9, zmm9, 0xd8
vmovdqu32 zmmword ptr [r9 + 320], zmm9
vshufi32x4 zmm8, zmm3, zmm7, 0x44
vshufi32x4 zmm8, zmm8, zmm8, 0xd8
vmovdqu32 zmmword ptr [r9 + 384], zmm8
vshufi32x4 zmm9, zmm3, zmm7, 0xee
vshufi32x4 zmm9, zmm9, zmm9, 0xd8
vmovdqu32 zmmword ptr [r9 + 448], zmm9
add r9, 512
dec rsi
jz .L_probe_avx512_counter_range_36_done
mov eax, 16
vpbroadcastd zmm0, eax
cmp r10, 0
je .L_probe_avx512_counter_range_36_inc_0
cmp r10, 1
je .L_probe_avx512_counter_range_36_inc_1
cmp r10, 2
je .L_probe_avx512_counter_range_36_inc_2
cmp r10, 3
je .L_probe_avx512_counter_range_36_inc_3
cmp r10, 4
je .L_probe_avx512_counter_range_36_inc_4
cmp r10, 5
je .L_probe_avx512_counter_range_36_inc_5
cmp r10, 6
je .L_probe_avx512_counter_range_36_inc_6
cmp r10, 7
je .L_probe_avx512_counter_range_36_inc_7
cmp r10, 8
je .L_probe_avx512_counter_range_36_inc_8
.L_probe_avx512_counter_range_36_inc_0:
vpaddd zmm16, zmm16, zmm0
jmp .L_probe_avx512_counter_range_36_loop
.L_probe_avx512_counter_range_36_inc_1:
vpaddd zmm17, zmm17, zmm0
jmp .L_probe_avx512_counter_range_36_loop
.L_probe_avx512_counter_range_36_inc_2:
vpaddd zmm18, zmm18, zmm0
jmp .L_probe_avx512_counter_range_36_loop
.L_probe_avx512_counter_range_36_inc_3:
vpaddd zmm19, zmm19, zmm0
jmp .L_probe_avx512_counter_range_36_loop
.L_probe_avx512_counter_range_36_inc_4:
vpaddd zmm20, zmm20, zmm0
jmp .L_probe_avx512_counter_range_36_loop
.L_probe_avx512_counter_range_36_inc_5:
vpaddd zmm21, zmm21, zmm0
jmp .L_probe_avx512_counter_range_36_loop
.L_probe_avx512_counter_range_36_inc_6:
vpaddd zmm22, zmm22, zmm0
jmp .L_probe_avx512_counter_range_36_loop
.L_probe_avx512_counter_range_36_inc_7:
vpaddd zmm23, zmm23, zmm0
jmp .L_probe_avx512_counter_range_36_loop
.L_probe_avx512_counter_range_36_inc_8:
vpaddd zmm24, zmm24, zmm0
jmp .L_probe_avx512_counter_range_36_loop
.L_probe_avx512_counter_range_36_done:
vzeroupper
ret
probe_avx512_counter_range_36_end:

.att_syntax prefix
