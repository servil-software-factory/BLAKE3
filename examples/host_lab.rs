//! Measure register-only Apple SME2 counter grinding (#511) vs `next` `Batch`
//! (+stamp & pure) across len=52 (BLINCS WOTS+C), len=36, len=64 on Apple M4/M6
//! P-cores and E-cores.
#![cfg_attr(not(target_arch = "aarch64"), allow(dead_code, unused_imports))]
use blake3_servil_core::Mode;
use blake3_servil_core::throughput::{Batch, TreeHasher};
use std::hint::black_box;
use std::time::Instant;

#[cfg(all(target_arch = "aarch64", target_vendor = "apple"))]
core::arch::global_asm!(include_str!("sme2_counter_apple.S"), options(raw));
#[cfg(all(target_arch = "aarch64", not(target_vendor = "apple")))]
core::arch::global_asm!(include_str!("sme2_counter_elf.S"), options(raw));

#[cfg(target_arch = "aarch64")]
unsafe extern "C" {
    fn probe_sme2_counter_range(
        template: *const u32,
        key: *const u32,
        base_and_word: u64,
        flags_and_len: u64,
        out: *mut u8,
        groups: u64,
    ) -> u64;
    fn probe_sme2_counter_range_52(
        template: *const u32,
        key: *const u32,
        base_and_word: u64,
        flags_and_len: u64,
        out: *mut u8,
        groups: u64,
    ) -> u64;
    fn probe_sme2_counter_range_36(
        template: *const u32,
        key: *const u32,
        base_and_word: u64,
        flags_and_len: u64,
        out: *mut u8,
        groups: u64,
    ) -> u64;
}

const KEY_BYTES: [u8; 32] = [
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
    0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e, 0x2f,
];
// CHUNK_START (1) | CHUNK_END (2) | ROOT (8) | KEYED_HASH (16) = 27
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
    let th = TreeHasher::new(Mode::Keyed(&KEY_BYTES));
    let kw = key_words();
    let mut out = vec![[0u8; 32]; 64];

    // Check universal kernel across varied (len, counter_word, base including wraparound)
    let cases = [
        (52usize, 12usize, 0xffff_fff5u32, 3usize),
        (36, 8, 42, 4),
        (64, 0, 0x1234_5678, 2),
        (64, 15, 0xffff_fff0, 2),
        (4, 0, 0, 1),
    ];
    for (len, word, base, groups) in cases {
        let tb = template_bytes(len);
        let tw = template_words(&tb);
        let n = 16 * groups;
        let ret = unsafe {
            probe_sme2_counter_range(
                tw.as_ptr(),
                kw.as_ptr(),
                (base as u64) | ((word as u64) << 32),
                KEYED_FLAGS | ((len as u64) << 8),
                out.as_mut_ptr().cast(),
                groups as u64,
            )
        };
        assert_eq!(ret, 16, "SVL must be 16 on Apple SME2");
        for i in 0..n {
            let mut msg = tb;
            msg[4 * word..4 * word + 4].copy_from_slice(&base.wrapping_add(i as u32).to_le_bytes());
            let expected = *th.hash(&msg[..len]).as_bytes();
            assert_eq!(out[i], expected, "mismatch gen len={len} word={word} i={i}");
        }
    }

    // Check 52 B and 36 B zero-add-pruned variants
    for (len, word, base, is_52) in [(52usize, 12usize, 0xffff_fff7u32, true), (36, 8, 99, false)] {
        let tb = template_bytes(len);
        let tw = template_words(&tb);
        let groups = 3usize;
        let ret = unsafe {
            if is_52 {
                probe_sme2_counter_range_52(
                    tw.as_ptr(),
                    kw.as_ptr(),
                    (base as u64) | ((word as u64) << 32),
                    KEYED_FLAGS | ((len as u64) << 8),
                    out.as_mut_ptr().cast(),
                    groups as u64,
                )
            } else {
                probe_sme2_counter_range_36(
                    tw.as_ptr(),
                    kw.as_ptr(),
                    (base as u64) | ((word as u64) << 32),
                    KEYED_FLAGS | ((len as u64) << 8),
                    out.as_mut_ptr().cast(),
                    groups as u64,
                )
            }
        };
        assert_eq!(ret, 16);
        for i in 0..16 * groups {
            let mut msg = tb;
            msg[4 * word..4 * word + 4].copy_from_slice(&base.wrapping_add(i as u32).to_le_bytes());
            let expected = *th.hash(&msg[..len]).as_bytes();
            assert_eq!(out[i], expected, "mismatch pruned len={len} i={i}");
        }
    }
    println!("verify_all: ALL 3 Apple SME2 counter-range kernels bit-for-bit identical (incl. u32 wrap)!");
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

#[cfg(target_arch = "aarch64")]
fn run_suite(label: &str) {
    println!("\n=== {label} (Mode::Keyed, ns/try) ===");
    let kw = key_words();
    let th = TreeHasher::new(Mode::Keyed(&KEY_BYTES));

    for (len, word) in [(52usize, 12usize), (36, 8), (64, 15)] {
        let tb = template_bytes(len);
        let tw = template_words(&tb);
        println!("--- len={len} B, counter_word={word} (offset {} B) ---", 4 * word);
        println!("  count | Batch pure | Batch +stamp | SME2 gen(<=64) | SME2 pruned | speedup vs +stamp");
        for &count in &[16usize, 64, 256, 2048, 8192] {
            let groups = (count / 16) as u64;
            let mut batch = Batch::new(len, count);
            for i in 0..count {
                let dst = batch.message_mut(i);
                dst.copy_from_slice(&tb[..len]);
                dst[4 * word..4 * word + 4].copy_from_slice(&(i as u32).to_le_bytes());
            }
            let mut out = vec![[0u8; 32]; count];

            let pure = bench_ns_per_try(count, || {
                th.hash_many(black_box(&batch), black_box(&mut out));
                black_box(&out);
            });
            let mut ctr = 0u32;
            let stamped = bench_ns_per_try(count, || {
                let b = ctr;
                ctr = ctr.wrapping_add(count as u32);
                for i in 0..count {
                    let dst = batch.message_mut(i);
                    dst[4 * word..4 * word + 4]
                        .copy_from_slice(&b.wrapping_add(i as u32).to_le_bytes());
                }
                th.hash_many(black_box(&batch), black_box(&mut out));
                black_box(&out);
            });
            let sme_gen = bench_ns_per_try(count, || {
                let b = ctr;
                ctr = ctr.wrapping_add(count as u32);
                unsafe {
                    probe_sme2_counter_range(
                        black_box(tw.as_ptr()),
                        black_box(kw.as_ptr()),
                        (b as u64) | ((word as u64) << 32),
                        KEYED_FLAGS | ((len as u64) << 8),
                        black_box(out.as_mut_ptr().cast()),
                        groups,
                    );
                }
                black_box(&out);
            });
            let sme_pruned = if len == 52 || len == 36 {
                bench_ns_per_try(count, || {
                    let b = ctr;
                    ctr = ctr.wrapping_add(count as u32);
                    unsafe {
                        if len == 52 {
                            probe_sme2_counter_range_52(
                                black_box(tw.as_ptr()),
                                black_box(kw.as_ptr()),
                                (b as u64) | ((word as u64) << 32),
                                KEYED_FLAGS | ((len as u64) << 8),
                                black_box(out.as_mut_ptr().cast()),
                                groups,
                            );
                        } else {
                            probe_sme2_counter_range_36(
                                black_box(tw.as_ptr()),
                                black_box(kw.as_ptr()),
                                (b as u64) | ((word as u64) << 32),
                                KEYED_FLAGS | ((len as u64) << 8),
                                black_box(out.as_mut_ptr().cast()),
                                groups,
                            );
                        }
                    }
                    black_box(&out);
                })
            } else {
                sme_gen
            };
            println!(
                "  {count:5} |   {pure:7.2}  |    {stamped:7.2}   |     {sme_gen:7.2}    |   {sme_pruned:7.2}   |  {:.2}x (pruned {:.2}x)",
                stamped / sme_gen,
                stamped / sme_pruned
            );
        }
    }
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
}

fn main() {
    blake3_servil_core::initialize();
    #[cfg(target_arch = "aarch64")]
    {
        verify_all();
        run_suite("P-core / default QoS");
        #[cfg(target_os = "macos")]
        {
            let rc = unsafe { pthread_set_qos_class_self_np(0x09, 0) };
            println!("\npthread_set_qos_class_self_np(BACKGROUND) rc={rc}");
            run_suite("E-core (QOS_CLASS_BACKGROUND)");
        }
    }
}
