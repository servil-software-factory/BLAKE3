#![cfg_attr(not(target_arch = "aarch64"), allow(dead_code, unused_imports))]

use blake3_servil_core::Mode;
use blake3_servil_core::convenience::hash_with;
use blake3_servil_core::throughput::{Batch, Packed, TreeHasher};
use std::hint::black_box;
use std::time::Instant;

#[cfg(all(target_arch = "aarch64", target_vendor = "apple"))]
core::arch::global_asm!(include_str!("p10_p11_apple.S"), options(raw));

#[cfg(all(target_arch = "aarch64", not(target_vendor = "apple")))]
core::arch::global_asm!(include_str!("p10_p11_elf.S"), options(raw));

#[cfg(target_arch = "aarch64")]
unsafe extern "C" {
    fn probe_p10_64(inputs: *const *const u8, key: *const u32, counter: u64, flags: u32, block_len: u32, out: *mut u8);
    fn probe_p11_64(inputs: *const *const u8, key: *const u32, counter: u64, flags: u32, block_len: u32, out: *mut u8);
    fn probe_p10_packed32(base: *const u8, key: *const u32, counter: u64, flags: u32, block_len: u32, out: *mut u8);
    fn probe_p11_packed32(base: *const u8, key: *const u32, counter: u64, flags: u32, block_len: u32, out: *mut u8);
    fn probe_p10_packed48(base: *const u8, key: *const u32, counter: u64, flags: u32, block_len: u32, out: *mut u8);
    fn probe_p11_packed48(base: *const u8, key: *const u32, counter: u64, flags: u32, block_len: u32, out: *mut u8);
    fn probe_p11_packed16(base: *const u8, key: *const u32, counter: u64, flags: u32, block_len: u32, out: *mut u8);
    fn probe_p11_packed64(base: *const u8, key: *const u32, counter: u64, flags: u32, block_len: u32, out: *mut u8);
}

#[cfg(target_vendor = "apple")]
unsafe extern "C" {
    fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
}

const IV: [u32; 8] = [
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A,
    0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
];
const FLAGS_PLAIN: u32 = 1 | 2 | 8; // CHUNK_START | CHUNK_END | ROOT

#[cfg(target_arch = "aarch64")]
fn median(mut xs: Vec<f64>) -> f64 {
    xs.sort_by(|a, b| a.total_cmp(b));
    xs[xs.len() / 2]
}

#[cfg(target_arch = "aarch64")]
fn verify_all() {
    let key_bytes = [0x5Au8; 32];
    let mut key_words = [0u32; 8];
    for i in 0..8 {
        key_words[i] = u32::from_le_bytes(key_bytes[4 * i..4 * i + 4].try_into().unwrap());
    }
    let modes: &[(&str, Mode, [u32; 8], u32)] = &[
        ("plain", Mode::Plain, IV, FLAGS_PLAIN),
        ("keyed", Mode::Keyed(&key_bytes), key_words, FLAGS_PLAIN | 16),
    ];

    for &(mname, mode, kw, flags) in modes {
        // 64-byte inputs (11 messages)
        let mut msgs64 = [[0u8; 64]; 11];
        for i in 0..11 {
            for j in 0..64 {
                msgs64[i][j] = ((i * 37 + j * 13 + 7) & 0xff) as u8;
            }
        }
        let ptrs: [*const u8; 11] = std::array::from_fn(|i| msgs64[i].as_ptr());

        let mut out10 = [[0u8; 32]; 10];
        unsafe { probe_p10_64(ptrs.as_ptr(), kw.as_ptr(), 0, flags, 64, out10.as_mut_ptr().cast()) };
        for i in 0..10 {
            assert_eq!(out10[i], *hash_with(mode, &msgs64[i]).as_bytes(), "p10_64 {mname} lane {i}");
        }

        let mut out11 = [[0u8; 32]; 11];
        unsafe { probe_p11_64(ptrs.as_ptr(), kw.as_ptr(), 0, flags, 64, out11.as_mut_ptr().cast()) };
        for i in 0..11 {
            assert_eq!(out11[i], *hash_with(mode, &msgs64[i]).as_bytes(), "p11_64 {mname} lane {i}");
        }

        let mut out11c = [[0u8; 32]; 11];
        unsafe { probe_p11_packed64(msgs64.as_ptr().cast(), kw.as_ptr(), 0, flags, 64, out11c.as_mut_ptr().cast()) };
        for i in 0..11 {
            assert_eq!(out11c[i], *hash_with(mode, &msgs64[i]).as_bytes(), "p11_packed64 {mname} lane {i}");
        }

        // 32-byte packed inputs (11 messages)
        let mut msgs32 = [[0u8; 32]; 11];
        for i in 0..11 {
            for j in 0..32 {
                msgs32[i][j] = ((i * 41 + j * 17 + 3) & 0xff) as u8;
            }
        }
        let mut out10_32 = [[0u8; 32]; 10];
        unsafe { probe_p10_packed32(msgs32.as_ptr().cast(), kw.as_ptr(), 0, flags, 32, out10_32.as_mut_ptr().cast()) };
        for i in 0..10 {
            assert_eq!(out10_32[i], *hash_with(mode, &msgs32[i]).as_bytes(), "p10_packed32 {mname} lane {i}");
        }

        let mut out11_32 = [[0u8; 32]; 11];
        unsafe { probe_p11_packed32(msgs32.as_ptr().cast(), kw.as_ptr(), 0, flags, 32, out11_32.as_mut_ptr().cast()) };
        for i in 0..11 {
            assert_eq!(out11_32[i], *hash_with(mode, &msgs32[i]).as_bytes(), "p11_packed32 {mname} lane {i}");
        }

        // 16-byte packed inputs (11 messages)
        let mut msgs16 = [[0u8; 16]; 11];
        for i in 0..11 {
            for j in 0..16 {
                msgs16[i][j] = ((i * 53 + j * 19 + 11) & 0xff) as u8;
            }
        }
        let mut out11_16 = [[0u8; 32]; 11];
        unsafe { probe_p11_packed16(msgs16.as_ptr().cast(), kw.as_ptr(), 0, flags, 16, out11_16.as_mut_ptr().cast()) };
        for i in 0..11 {
            assert_eq!(out11_16[i], *hash_with(mode, &msgs16[i]).as_bytes(), "p11_packed16 {mname} lane {i}");
        }

        // 48-byte packed inputs (11 messages)
        let mut msgs48 = [[0u8; 48]; 11];
        for i in 0..11 {
            for j in 0..48 {
                msgs48[i][j] = ((i * 59 + j * 23 + 13) & 0xff) as u8;
            }
        }
        let mut out10_48 = [[0u8; 32]; 10];
        unsafe { probe_p10_packed48(msgs48.as_ptr().cast(), kw.as_ptr(), 0, flags, 48, out10_48.as_mut_ptr().cast()) };
        for i in 0..10 {
            assert_eq!(out10_48[i], *hash_with(mode, &msgs48[i]).as_bytes(), "p10_packed48 {mname} lane {i}");
        }
        let mut out11_48 = [[0u8; 32]; 11];
        unsafe { probe_p11_packed48(msgs48.as_ptr().cast(), kw.as_ptr(), 0, flags, 48, out11_48.as_mut_ptr().cast()) };
        for i in 0..11 {
            assert_eq!(out11_48[i], *hash_with(mode, &msgs48[i]).as_bytes(), "p11_packed48 {mname} lane {i}");
        }
    }
    println!("verify_all: ALL 8 probe kernels bit-for-bit identical (Plain + Keyed)!");
}

#[cfg(target_arch = "aarch64")]
fn time_ns_per_hash(count: usize, reps: usize, rounds: usize, mut f: impl FnMut()) -> f64 {
    for _ in 0..4 {
        f();
    }
    let mut xs = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        let t0 = Instant::now();
        for _ in 0..reps {
            f();
        }
        xs.push(t0.elapsed().as_nanos() as f64 / (reps * count) as f64);
    }
    median(xs)
}

#[cfg(target_arch = "aarch64")]
fn run_suite(tag: &str, reps_1k: usize) {
    println!("\n=== {tag} ===");
    let th_neon = TreeHasher::new(Mode::Plain).on_this_core();
    let th_sme2 = TreeHasher::new(Mode::Plain);

    // Warm up the core clock for ~150 ms so the first cell never sees a cold-clock ramp.
    let warm_b = Batch::new(64, 1024);
    let mut warm_o = vec![[0u8; 32]; 1024];
    let warm_t0 = Instant::now();
    while warm_t0.elapsed().as_millis() < 150 {
        th_neon.hash_many(black_box(&warm_b), black_box(&mut warm_o));
    }

    // 1. Sustained n = 1,020 (divisible by 10) and n = 1,023 (divisible by 11) vs n = 1,024
    let n10 = 1020usize;
    let n11 = 1023usize;
    let n1k = 1024usize;
    let mut b64 = Batch::new(64, n1k);
    let mut p32 = Packed::<32>::new(n1k);
    let mut p16 = Packed::<16>::new(n1k);
    for i in 0..n1k {
        b64.message_mut(i).fill((i & 0xff) as u8);
        p32[i] = [(i & 0xff) as u8; 32];
        p16[i] = [(i & 0xff) as u8; 16];
    }
    let ptrs64: Vec<*const u8> = (0..n1k).map(|i| b64.message_mut(i).as_ptr()).collect();
    let mut out = vec![[0u8; 32]; n1k];

    let next_neon_64 = time_ns_per_hash(n1k, reps_1k, 15, || {
        th_neon.hash_many(black_box(&b64), black_box(&mut out));
    });
    let next_sme2_64 = time_ns_per_hash(n1k, reps_1k, 15, || {
        th_sme2.hash_many(black_box(&b64), black_box(&mut out));
    });
    let p10_64 = time_ns_per_hash(n10, reps_1k, 15, || {
        for off in (0..n10).step_by(10) {
            unsafe { probe_p10_64(black_box(ptrs64.as_ptr().add(off)), IV.as_ptr(), 0, FLAGS_PLAIN, 64, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let p11_64 = time_ns_per_hash(n11, reps_1k, 15, || {
        for off in (0..n11).step_by(11) {
            unsafe { probe_p11_64(black_box(ptrs64.as_ptr().add(off)), IV.as_ptr(), 0, FLAGS_PLAIN, 64, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let raw64_ptr = b64.blocks().as_ptr() as *const u8;
    let p11_p64 = time_ns_per_hash(n11, reps_1k, 15, || {
        for off in (0..n11).step_by(11) {
            unsafe { probe_p11_packed64(black_box(raw64_ptr.add(off * 64)), IV.as_ptr(), 0, FLAGS_PLAIN, 64, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });

    println!("64 B (n~1024):");
    println!("  next NEON Batch(64):  {next_neon_64:6.2} ns/h  (1.00x)");
    println!("  probe p10 (8+2 ptrs): {p10_64:6.2} ns/h  ({:.2}x vs next NEON)", next_neon_64 / p10_64);
    println!("  probe p11 (8+3 ptrs): {p11_64:6.2} ns/h  ({:.2}x vs next NEON)", next_neon_64 / p11_64);
    println!("  probe p11 (8+3 raw):  {p11_p64:6.2} ns/h  ({:.2}x vs next NEON)", next_neon_64 / p11_p64);
    println!("  next SME2 Batch(64):  {next_sme2_64:6.2} ns/h  ({:.2}x vs next NEON)", next_neon_64 / next_sme2_64);

    let next_neon_32 = time_ns_per_hash(n1k, reps_1k, 15, || {
        th_neon.hash_many(black_box(&p32), black_box(&mut out));
    });
    let next_sme2_32 = time_ns_per_hash(n1k, reps_1k, 15, || {
        th_sme2.hash_many(black_box(&p32), black_box(&mut out));
    });
    let raw32_ptr = p32.as_ptr() as *const u8;
    let p10_32 = time_ns_per_hash(n10, reps_1k, 15, || {
        for off in (0..n10).step_by(10) {
            unsafe { probe_p10_packed32(black_box(raw32_ptr.add(off * 32)), IV.as_ptr(), 0, FLAGS_PLAIN, 32, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let p11_32 = time_ns_per_hash(n11, reps_1k, 15, || {
        for off in (0..n11).step_by(11) {
            unsafe { probe_p11_packed32(black_box(raw32_ptr.add(off * 32)), IV.as_ptr(), 0, FLAGS_PLAIN, 32, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    println!("Packed<32> (n~1024):");
    println!("  next NEON Packed<32>: {next_neon_32:6.2} ns/h  (1.00x)");
    println!("  probe p10 Packed<32>: {p10_32:6.2} ns/h  ({:.2}x vs next NEON)", next_neon_32 / p10_32);
    println!("  probe p11 Packed<32>: {p11_32:6.2} ns/h  ({:.2}x vs next NEON)", next_neon_32 / p11_32);
    println!("  next SME2 Packed<32>: {next_sme2_32:6.2} ns/h  ({:.2}x vs next NEON)", next_neon_32 / next_sme2_32);

    let next_neon_16 = time_ns_per_hash(n1k, reps_1k, 15, || {
        th_neon.hash_many(black_box(&p16), black_box(&mut out));
    });
    let next_sme2_16 = time_ns_per_hash(n1k, reps_1k, 15, || {
        th_sme2.hash_many(black_box(&p16), black_box(&mut out));
    });
    let raw16_ptr = p16.as_ptr() as *const u8;
    let p11_16 = time_ns_per_hash(n11, reps_1k, 15, || {
        for off in (0..n11).step_by(11) {
            unsafe { probe_p11_packed16(black_box(raw16_ptr.add(off * 16)), IV.as_ptr(), 0, FLAGS_PLAIN, 16, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    println!("Packed<16> (n~1024):");
    println!("  next NEON Packed<16>: {next_neon_16:6.2} ns/h  (1.00x)");
    println!("  probe p11 Packed<16>: {p11_16:6.2} ns/h  ({:.2}x vs next NEON)", next_neon_16 / p11_16);
    println!("  next SME2 Packed<16>: {next_sme2_16:6.2} ns/h  ({:.2}x vs next NEON)", next_neon_16 / next_sme2_16);

    let mut b48 = Batch::new(48, n1k);
    let mut raw48 = vec![[0u8; 48]; n1k];
    for i in 0..n1k {
        b48.message_mut(i).fill((i & 0xff) as u8);
        raw48[i].fill((i & 0xff) as u8);
    }
    let next_neon_48 = time_ns_per_hash(n1k, reps_1k, 15, || {
        th_neon.hash_many(black_box(&b48), black_box(&mut out));
    });
    let next_sme2_48 = time_ns_per_hash(n1k, reps_1k, 15, || {
        th_sme2.hash_many(black_box(&b48), black_box(&mut out));
    });
    let raw48_ptr = raw48.as_ptr() as *const u8;
    let p10_48 = time_ns_per_hash(n10, reps_1k, 15, || {
        for off in (0..n10).step_by(10) {
            unsafe { probe_p10_packed48(black_box(raw48_ptr.add(off * 48)), IV.as_ptr(), 0, FLAGS_PLAIN, 48, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let p11_48 = time_ns_per_hash(n11, reps_1k, 15, || {
        for off in (0..n11).step_by(11) {
            unsafe { probe_p11_packed48(black_box(raw48_ptr.add(off * 48)), IV.as_ptr(), 0, FLAGS_PLAIN, 48, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    println!("48 B (n~1024):");
    println!("  next NEON Batch(48):  {next_neon_48:6.2} ns/h  (1.00x)");
    println!("  probe p10 Packed<48>: {p10_48:6.2} ns/h  ({:.2}x vs next NEON)", next_neon_48 / p10_48);
    println!("  probe p11 Packed<48>: {p11_48:6.2} ns/h  ({:.2}x vs next NEON)", next_neon_48 / p11_48);
    println!("  next SME2 Batch(48):  {next_sme2_48:6.2} ns/h  ({:.2}x vs next NEON)", next_neon_48 / next_sme2_48);

    // 2. Small bursts (n = 11, 22, 33) interleaved with scalar work (BLINCS #502 pattern)
    println!("Small bursts Packed<32> (n = 11, 22, 33):");
    for &cnt in &[11usize, 22, 33] {
        let p_sm = Packed::<32>::new(cnt);
        let mut o_sm = vec![[0u8; 32]; cnt];
        let reps_sm = reps_1k * 20;
        let t_neon = time_ns_per_hash(cnt, reps_sm, 15, || {
            th_neon.hash_many(black_box(&p_sm), black_box(&mut o_sm));
        });
        let t_sme2 = time_ns_per_hash(cnt, reps_sm, 15, || {
            th_sme2.hash_many(black_box(&p_sm), black_box(&mut o_sm));
        });
        let rptr = p_sm.as_ptr() as *const u8;
        let t_p11 = time_ns_per_hash(cnt, reps_sm, 15, || {
            for off in (0..cnt).step_by(11) {
                unsafe { probe_p11_packed32(black_box(rptr.add(off * 32)), IV.as_ptr(), 0, FLAGS_PLAIN, 32, black_box(o_sm.as_mut_ptr().add(off).cast())) };
            }
        });
        println!("  n={cnt:2}: next_NEON={t_neon:6.2}  probe_p11={t_p11:6.2} ({:.2}x)  next_SME2={t_sme2:6.2} ns/h", t_neon / t_p11);
    }
}

fn main() {
    blake3_servil_core::initialize();
    #[cfg(target_arch = "aarch64")]
    {
        verify_all();
        run_suite("P-core / default QoS", 64);
        #[cfg(target_vendor = "apple")]
        {
            let rc = unsafe { pthread_set_qos_class_self_np(0x09 /* QOS_CLASS_BACKGROUND */, 0) };
            println!("\npthread_set_qos_class_self_np(BACKGROUND) rc={rc}");
            run_suite("E-core (QOS_CLASS_BACKGROUND)", 16);
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    println!("aarch64 only");
}
