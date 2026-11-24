#![cfg_attr(not(all(target_arch = "x86_64", target_os = "linux")), allow(dead_code, unused_imports))]

use blake3_servil_core::Mode;
use blake3_servil_core::throughput::{Batch, Packed, TreeHasher};
use std::hint::black_box;
use std::time::Instant;

#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
core::arch::global_asm!(include_str!("avx512_packed_x86_64.S"), options(raw));

#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
unsafe extern "C" {
    fn probe_avx512_packed16(input: *const u8, groups: u64, key: *const u32, flags: u32, out: *mut u8);
    fn probe_avx512_packed32(input: *const u8, groups: u64, key: *const u32, flags: u32, out: *mut u8);
    fn probe_avx512_packed48(input: *const u8, groups: u64, key: *const u32, flags: u32, out: *mut u8);
}

const IV: [u32; 8] = [
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A,
    0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
];
const ONE_BLOCK_FLAGS: u32 = 1 | 2 | 8; // CHUNK_START | CHUNK_END | ROOT

fn median(mut xs: Vec<f64>) -> f64 {
    xs.sort_by(|a, b| a.total_cmp(b));
    xs[xs.len() / 2]
}

fn time_ns(count: usize, reps: usize, rounds: usize, mut f: impl FnMut()) -> f64 {
    for _ in 0..4 { f(); }
    let mut xs = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        let t0 = Instant::now();
        for _ in 0..reps { f(); }
        xs.push(t0.elapsed().as_nanos() as f64 / (reps * count) as f64);
    }
    median(xs)
}

#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
fn main() {
    blake3_servil_core::initialize();
    if !std::arch::is_x86_feature_detected!("avx512f") {
        println!("no AVX-512F on this CPU");
        return;
    }

    // 1. Verify bit-for-bit against blake3_servil_core::convenience::hash for 32 messages each.
    let mut p16 = Packed::<16>::new(32);
    let mut p32 = Packed::<32>::new(32);
    let mut p48 = Packed::<48>::new(32);
    for i in 0..32 {
        for j in 0..16 { p16[i][j] = ((i * 29 + j * 11 + 3) & 0xff) as u8; }
        for j in 0..32 { p32[i][j] = ((i * 31 + j * 13 + 7) & 0xff) as u8; }
        for j in 0..48 { p48[i][j] = ((i * 37 + j * 17 + 9) & 0xff) as u8; }
    }
    let (mut o16, mut o32, mut o48) = ([[0u8; 32]; 32], [[0u8; 32]; 32], [[0u8; 32]; 32]);
    unsafe {
        probe_avx512_packed16(p16.as_flattened().as_ptr(), 2, IV.as_ptr(), ONE_BLOCK_FLAGS, o16.as_mut_ptr().cast());
        probe_avx512_packed32(p32.as_flattened().as_ptr(), 2, IV.as_ptr(), ONE_BLOCK_FLAGS, o32.as_mut_ptr().cast());
        probe_avx512_packed48(p48.as_flattened().as_ptr(), 2, IV.as_ptr(), ONE_BLOCK_FLAGS, o48.as_mut_ptr().cast());
    }
    for i in 0..32 {
        assert_eq!(o16[i], *blake3_servil_core::convenience::hash(&p16[i]).as_bytes(), "p16[{i}]");
        assert_eq!(o32[i], *blake3_servil_core::convenience::hash(&p32[i]).as_bytes(), "p32[{i}]");
        assert_eq!(o48[i], *blake3_servil_core::convenience::hash(&p48[i]).as_bytes(), "p48[{i}]");
    }
    println!("verify_all: ALL 3 AVX-512 direct-packed kernels (16/32/48 B) bit-for-bit identical!");

    // 2. Benchmark across counts (16, 64, 256, 1024, 6144) on 1 core (ST).
    let th = TreeHasher::new(Mode::Plain).on_this_core();
    let warm = Batch::new(64, 1024);
    let mut warm_o = vec![[0u8; 32]; 1024];
    let t0 = Instant::now();
    while t0.elapsed().as_millis() < 150 {
        th.hash_many(black_box(&warm), black_box(&mut warm_o));
    }

    println!("\nSingle-core ns/hash (lower is better):");
    println!("  count  | Batch(64) | next P16 -> direct | next P32 -> direct | next P48 -> direct");
    for &(n, reps) in &[(16usize, 4096usize), (64, 1024), (256, 256), (1024, 64), (6144, 16)] {
        let b64 = Batch::new(64, n);
        let p16 = Packed::<16>::new(n);
        let p32 = Packed::<32>::new(n);
        let p48 = Packed::<48>::new(n);
        let mut out = vec![[0u8; 32]; n];
        let groups = (n / 16) as u64;

        let t_b64 = time_ns(n, reps, 15, || th.hash_many(black_box(&b64), black_box(&mut out)));
        let t_p16_old = time_ns(n, reps, 15, || th.hash_many(black_box(&p16), black_box(&mut out)));
        let t_p16_new = time_ns(n, reps, 15, || unsafe {
            probe_avx512_packed16(black_box(p16.as_flattened().as_ptr()), groups, IV.as_ptr(), ONE_BLOCK_FLAGS, black_box(out.as_mut_ptr().cast()));
        });
        let t_p32_old = time_ns(n, reps, 15, || th.hash_many(black_box(&p32), black_box(&mut out)));
        let t_p32_new = time_ns(n, reps, 15, || unsafe {
            probe_avx512_packed32(black_box(p32.as_flattened().as_ptr()), groups, IV.as_ptr(), ONE_BLOCK_FLAGS, black_box(out.as_mut_ptr().cast()));
        });
        let t_p48_old = time_ns(n, reps, 15, || th.hash_many(black_box(&p48), black_box(&mut out)));
        let t_p48_new = time_ns(n, reps, 15, || unsafe {
            probe_avx512_packed48(black_box(p48.as_flattened().as_ptr()), groups, IV.as_ptr(), ONE_BLOCK_FLAGS, black_box(out.as_mut_ptr().cast()));
        });
        println!(
            "  {:5}  |  {:6.2}   | {:5.2} -> {:5.2} ({:.2}x) | {:5.2} -> {:5.2} ({:.2}x) | {:5.2} -> {:5.2} ({:.2}x)",
            n, t_b64,
            t_p16_old, t_p16_new, t_p16_old / t_p16_new,
            t_p32_old, t_p32_new, t_p32_old / t_p32_new,
            t_p48_old, t_p48_new, t_p48_old / t_p48_new,
        );
    }
}

#[cfg(not(all(target_arch = "x86_64", target_os = "linux")))]
fn main() {
    println!("Linux x86-64 only");
}
