#![cfg_attr(not(target_arch = "aarch64"), allow(dead_code, unused_imports))]
//! Measure register-only AArch64 NEON zero-spill `8+3`/`8+2` counter grinding (#511)
//! vs `next` NEON `Batch` (pure & +stamp) on AmpereOne (`arm-vm`) and Apple M4/M6.
use blake3_servil_core::Mode;
use blake3_servil_core::throughput::{Batch, TreeHasher};
use std::hint::black_box;
use std::time::Instant;

#[cfg(all(target_arch = "aarch64", target_vendor = "apple"))]
core::arch::global_asm!(include_str!("neon_counter_apple.S"), options(raw));
#[cfg(all(target_arch = "aarch64", not(target_vendor = "apple")))]
core::arch::global_asm!(include_str!("neon_counter_elf.S"), options(raw));

#[cfg(target_arch = "aarch64")]
unsafe extern "C" {
    fn probe_neon_counter_p11(
        template: *const u32,
        key: *const u32,
        base_and_word: u64,
        flags_and_len: u64,
        out: *mut u8,
        groups: u64,
    );
    fn probe_neon_counter_p10(
        template: *const u32,
        key: *const u32,
        base_and_word: u64,
        flags_and_len: u64,
        out: *mut u8,
        groups: u64,
    );
    fn probe_neon_counter_p11_52(
        template: *const u32,
        key: *const u32,
        base_and_word: u64,
        flags_and_len: u64,
        out: *mut u8,
        groups: u64,
    );
}

const KEY_BYTES: [u8; 32] = [
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
    0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e, 0x2f,
];
const KEYED_FLAGS: u64 = 1 | 2 | 8 | 16;

fn key_words() -> [u32; 8] {
    let mut kw = [0u32; 8];
    for i in 0..8 {
        kw[i] = u32::from_le_bytes(KEY_BYTES[4 * i..4 * i + 4].try_into().unwrap());
    }
    kw
}

fn template_bytes(len: usize) -> [u8; 64] {
    let mut b = [0u8; 64];
    for i in 0..len {
        b[i] = ((i * 73 + 19) & 0xff) as u8;
    }
    b
}

fn template_words(b: &[u8; 64]) -> [u32; 16] {
    let mut w = [0u32; 16];
    for i in 0..16 {
        w[i] = u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap());
    }
    w
}

#[cfg(target_arch = "aarch64")]
fn verify_all() {
    let th = TreeHasher::new(Mode::Keyed(&KEY_BYTES)).on_this_core();
    let kw = key_words();
    let mut out = vec![[0u8; 32]; 64];
    for (len, word, base, groups) in [
        (52usize, 12usize, 0xffff_fff7u32, 3usize),
        (36, 8, 42, 3),
        (64, 0, 0x1234_5678, 2),
        (64, 15, 0xffff_fff2, 2),
    ] {
        let tb = template_bytes(len);
        let tw = template_words(&tb);
        unsafe {
            probe_neon_counter_p11(
                tw.as_ptr(),
                kw.as_ptr(),
                (base as u64) | ((word as u64) << 32),
                KEYED_FLAGS | ((len as u64) << 8),
                out.as_mut_ptr().cast(),
                groups as u64,
            );
        }
        for i in 0..11 * groups {
            let mut msg = tb;
            msg[4 * word..4 * word + 4].copy_from_slice(&base.wrapping_add(i as u32).to_le_bytes());
            let expected = *th.hash(&msg[..len]).as_bytes();
            assert_eq!(out[i], expected, "mismatch p11 len={len} word={word} i={i}");
        }
        unsafe {
            probe_neon_counter_p10(
                tw.as_ptr(),
                kw.as_ptr(),
                (base as u64) | ((word as u64) << 32),
                KEYED_FLAGS | ((len as u64) << 8),
                out.as_mut_ptr().cast(),
                groups as u64,
            );
        }
        for i in 0..10 * groups {
            let mut msg = tb;
            msg[4 * word..4 * word + 4].copy_from_slice(&base.wrapping_add(i as u32).to_le_bytes());
            let expected = *th.hash(&msg[..len]).as_bytes();
            assert_eq!(out[i], expected, "mismatch p10 len={len} word={word} i={i}");
        }
    }
    let tb = template_bytes(52);
    let tw = template_words(&tb);
    unsafe {
        probe_neon_counter_p11_52(
            tw.as_ptr(),
            kw.as_ptr(),
            (0xffff_fff7u32 as u64) | (12u64 << 32),
            KEYED_FLAGS | (52u64 << 8),
            out.as_mut_ptr().cast(),
            3,
        );
    }
    for i in 0..33 {
        let mut msg = tb;
        msg[48..52].copy_from_slice(&0xffff_fff7u32.wrapping_add(i as u32).to_le_bytes());
        let expected = *th.hash(&msg[..52]).as_bytes();
        assert_eq!(out[i], expected, "mismatch p11_52 i={i}");
    }
    println!("verify_all: ALL 3 NEON counter-range kernels bit-for-bit identical (incl. u32 wrap)!");
}

fn bench_ns_per_try<F: FnMut()>(n_tries: usize, mut f: F) -> f64 {
    for _ in 0..3 {
        f();
    }
    let mut reps = 1usize;
    loop {
        let t0 = Instant::now();
        for _ in 0..reps {
            f();
        }
        if t0.elapsed().as_millis() >= 15 || reps >= 1_000_000 {
            break;
        }
        reps *= 2;
    }
    let mut samples = [0.0f64; 9];
    for s in &mut samples {
        let t0 = Instant::now();
        for _ in 0..reps {
            f();
        }
        *s = t0.elapsed().as_nanos() as f64 / (reps * n_tries) as f64;
    }
    samples.sort_by(|a, b| a.total_cmp(b));
    samples[samples.len() / 2]
}

fn main() {
    blake3_servil_core::initialize();
    #[cfg(target_arch = "aarch64")]
    {
        verify_all();
        let kw = key_words();
        let th = TreeHasher::new(Mode::Keyed(&KEY_BYTES)).on_this_core();
        for (len, word) in [(52usize, 12usize), (64, 15)] {
            let tb = template_bytes(len);
            let tw = template_words(&tb);
            println!("\n--- NEON len={len} B, counter_word={word} (offset {} B) ---", 4 * word);
            println!("  count(11x) | Batch pure | Batch +stamp | NEON p11 | NEON p10 | p11_52 | speedup vs +stamp");
            for &groups in &[1usize, 6, 24, 186] {
                let n11 = 11 * groups;
                let n10 = 10 * groups;
                let mut batch = Batch::new(len, n11);
                for i in 0..n11 {
                    let dst = batch.message_mut(i);
                    dst.copy_from_slice(&tb[..len]);
                    dst[4 * word..4 * word + 4].copy_from_slice(&(i as u32).to_le_bytes());
                }
                let mut out = vec![[0u8; 32]; n11];
                let pure = bench_ns_per_try(n11, || {
                    th.hash_many(black_box(&batch), black_box(&mut out));
                    black_box(&out);
                });
                let mut ctr = 0u32;
                let stamped = bench_ns_per_try(n11, || {
                    let b = ctr;
                    ctr = ctr.wrapping_add(n11 as u32);
                    for i in 0..n11 {
                        let dst = batch.message_mut(i);
                        dst[4 * word..4 * word + 4]
                            .copy_from_slice(&b.wrapping_add(i as u32).to_le_bytes());
                    }
                    th.hash_many(black_box(&batch), black_box(&mut out));
                    black_box(&out);
                });
                let p11 = bench_ns_per_try(n11, || {
                    let b = ctr;
                    ctr = ctr.wrapping_add(n11 as u32);
                    unsafe {
                        probe_neon_counter_p11(
                            black_box(tw.as_ptr()),
                            black_box(kw.as_ptr()),
                            (b as u64) | ((word as u64) << 32),
                            KEYED_FLAGS | ((len as u64) << 8),
                            black_box(out.as_mut_ptr().cast()),
                            groups as u64,
                        );
                    }
                    black_box(&out);
                });
                let p10 = bench_ns_per_try(n10, || {
                    let b = ctr;
                    ctr = ctr.wrapping_add(n10 as u32);
                    unsafe {
                        probe_neon_counter_p10(
                            black_box(tw.as_ptr()),
                            black_box(kw.as_ptr()),
                            (b as u64) | ((word as u64) << 32),
                            KEYED_FLAGS | ((len as u64) << 8),
                            black_box(out.as_mut_ptr().cast()),
                            groups as u64,
                        );
                    }
                    black_box(&out);
                });
                let p11_52 = if len == 52 {
                    bench_ns_per_try(n11, || {
                        let b = ctr;
                        ctr = ctr.wrapping_add(n11 as u32);
                        unsafe {
                            probe_neon_counter_p11_52(
                                black_box(tw.as_ptr()),
                                black_box(kw.as_ptr()),
                                (b as u64) | ((word as u64) << 32),
                                KEYED_FLAGS | ((len as u64) << 8),
                                black_box(out.as_mut_ptr().cast()),
                                groups as u64,
                            );
                        }
                        black_box(&out);
                    })
                } else {
                    p11
                };
                println!(
                    "  {:10} |   {pure:7.2}  |    {stamped:7.2}   |  {p11:6.2}  |  {p10:6.2}  | {p11_52:6.2} |  {:.2}x (p11_52 {:.2}x)",
                    n11,
                    stamped / p11,
                    stamped / p11_52
                );
            }
        }
    }
}
