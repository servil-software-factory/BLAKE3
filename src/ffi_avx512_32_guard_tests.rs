// Included inside ffi_avx512_32::tests (Linux x86-64 only).
// Reuses the common `crate::test::guard::Guarded` / `ReadOnlyGuarded` owners
// so over/under-reads and writes fault rather than hide in allocator slack.

#[test]
fn guarded_input_key_pointer_table_and_output_bounds() {
    if !crate::platform::avx512_detected() {
        return;
    }
    for width in [16, 32] {
        guarded_kernel_bounds(width);
    }
}

fn guarded_kernel_bounds(width: usize) {
    use crate::test::guard::Guarded;
    assert!(crate::platform::avx512_detected() && matches!(width, 16 | 32));
    for at_end in [false, true] {
        for blocks in [1, 16] {
            for last_len in [0, 63, 64] {
                let n = blocks * BLOCK_LEN;
                let inputs: Vec<_> = (0..width)
                    .map(|i| {
                        let mut input = Guarded::new(n, at_end);
                        let bytes = input.bytes();
                        for (j, b) in bytes.iter_mut().enumerate() {
                            *b = (13 * j + 17 * i) as u8;
                        }
                        bytes[(blocks - 1) * BLOCK_LEN + last_len..].fill(0);
                        input.read_only()
                    })
                    .collect();
                let mut table = Guarded::new(width * core::mem::size_of::<*const u8>(), at_end);
                for (i, input) in inputs.iter().enumerate() {
                    let ptr_bytes = (input.as_bytes().as_ptr() as usize).to_ne_bytes();
                    table.bytes()[i * core::mem::size_of::<*const u8>()..][..core::mem::size_of::<*const u8>()].copy_from_slice(&ptr_bytes);
                }
                let table = table.read_only();
                let mut key = Guarded::new(32, at_end);
                key.bytes().copy_from_slice(&crate::hasher::le_bytes(crate::IV));
                let key = key.read_only();
                let mut out = Guarded::new(width * OUT_LEN, at_end);
                let counter = u64::MAX - 15;
                let packed = 16 | (1 << 8) | (2 << 16) | ((last_len as u64) << 24);
                for inc in [false, true] {
                    if width == 16 && inc {
                        continue;
                    }
                    let mut want = vec![0u8; width * OUT_LEN];
                    for (i, input) in inputs.iter().enumerate() {
                        let mut cv = *crate::IV;
                        let c = counter.wrapping_add(if inc { i as u64 } else { 0 });
                        for (b, block) in input.as_bytes().chunks_exact(BLOCK_LEN).enumerate() {
                            crate::platform::Platform::SSE41.compress_in_place(
                                &mut cv,
                                block.try_into().unwrap(),
                                if b + 1 == blocks { last_len as u8 } else { BLOCK_LEN as u8 },
                                c,
                                16 | if b == 0 { 1 } else { 0 } | if b + 1 == blocks { 2 } else { 0 },
                            );
                        }
                        for w in 0..8 {
                            want[i * OUT_LEN + 4 * w..][..4].copy_from_slice(&cv[w].to_le_bytes());
                        }
                    }
                    out.bytes().fill(0xa5);
                    unsafe {
                        let kernel = if width == 16 {
                            blake3_servil_avx512_p16
                        } else if inc {
                            blake3_servil_avx512_k32
                        } else {
                            blake3_servil_avx512_p32
                        };
                        kernel(
                            table.as_bytes().as_ptr().cast(),
                            blocks as u64,
                            key.as_bytes().as_ptr().cast(),
                            counter,
                            packed,
                            out.bytes().as_mut_ptr(),
                        );
                    }
                    assert_eq!(out.as_bytes(), &want, "width={width} at_end={at_end} blocks={blocks} last_len={last_len} increment={inc}");
                }
            }
        }
    }
}
