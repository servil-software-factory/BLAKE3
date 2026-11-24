#![cfg_attr(not(all(target_arch = "aarch64", target_vendor = "apple")), allow(dead_code, unused_imports))]

use blake3_servil_core::Mode;
use blake3_servil_core::throughput::{Batch, Packed, TreeHasher};
use std::hint::black_box;
use std::time::Instant;

#[cfg(all(target_arch = "aarch64", target_vendor = "apple"))]
core::arch::global_asm!(include_str!("sme2_packed_apple.S"), options(raw));

#[cfg(all(target_arch = "aarch64", target_vendor = "apple"))]
unsafe extern "C" {
    fn probe_sme2_packed16(input: *const u8, key: *const u32, flags: u32, out: *mut u8, groups: u64) -> u64;
    fn probe_sme2_packed32(input: *const u8, key: *const u32, flags: u32, out: *mut u8, groups: u64) -> u64;
    fn probe_sme2_packed48(input: *const u8, key: *const u32, flags: u32, out: *mut u8, groups: u64) -> u64;
    fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
}

const IV: [u32; 8] = [
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A,
    0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
];
const ONE_BLOCK_FLAGS: u32 = 1 | 2 | 8;

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

#[cfg(all(target_arch = "aarch64", target_vendor = "apple"))]
fn run_suite(tag: &str, scale: usize) {
    println!("\n=== {tag} ===");
    let th_sme = TreeHasher::new(Mode::Plain);
    let th_neon = TreeHasher::new(Mode::Plain).on_this_core();
    let warm = Batch::new(64, 512);
    let mut warm_o = vec![[0u8; 32]; 512];
    let t0 = Instant::now();
    while t0.elapsed().as_millis() < 120 {
        th_sme.hash_many(black_box(&warm), black_box(&mut warm_o));
    }

    println!("  count  | Batch(64) | next P16 -> direct SME2 | next P32 -> direct SME2 | next P48 -> direct SME2 | NEON P32");
    for &(n, base_reps) in &[(16usize, 1024usize), (64, 256), (256, 64), (1024, 16), (6144, 4)] {
        let reps = (base_reps / scale).max(1);
        let b64 = Batch::new(64, n);
        let p16 = Packed::<16>::new(n);
        let p32 = Packed::<32>::new(n);
        let p48 = Packed::<48>::new(n);
        let mut out = vec![[0u8; 32]; n];
        let groups = (n / 16) as u64;

        let t_b64 = time_ns(n, reps, 13, || th_sme.hash_many(black_box(&b64), black_box(&mut out)));
        let t_p16_old = time_ns(n, reps, 13, || th_sme.hash_many(black_box(&p16), black_box(&mut out)));
        let t_p16_new = time_ns(n, reps, 13, || unsafe {
            probe_sme2_packed16(black_box(p16.as_flattened().as_ptr()), IV.as_ptr(), ONE_BLOCK_FLAGS, black_box(out.as_mut_ptr().cast()), groups);
        });
        let t_p32_old = time_ns(n, reps, 13, || th_sme.hash_many(black_box(&p32), black_box(&mut out)));
        let t_p32_new = time_ns(n, reps, 13, || unsafe {
            probe_sme2_packed32(black_box(p32.as_flattened().as_ptr()), IV.as_ptr(), ONE_BLOCK_FLAGS, black_box(out.as_mut_ptr().cast()), groups);
        });
        let t_p48_old = time_ns(n, reps, 13, || th_sme.hash_many(black_box(&p48), black_box(&mut out)));
        let t_p48_new = time_ns(n, reps, 13, || unsafe {
            probe_sme2_packed48(black_box(p48.as_flattened().as_ptr()), IV.as_ptr(), ONE_BLOCK_FLAGS, black_box(out.as_mut_ptr().cast()), groups);
        });
        let t_neon = time_ns(n, reps, 13, || th_neon.hash_many(black_box(&p32), black_box(&mut out)));
        println!(
            "  {:5}  |  {:6.2}   | {:5.2} -> {:5.2} ({:.2}x)     | {:5.2} -> {:5.2} ({:.2}x)     | {:5.2} -> {:5.2} ({:.2}x)     | {:6.2}",
            n, t_b64,
            t_p16_old, t_p16_new, t_p16_old / t_p16_new,
            t_p32_old, t_p32_new, t_p32_old / t_p32_new,
            t_p48_old, t_p48_new, t_p48_old / t_p48_new,
            t_neon,
        );
    }
}

#[cfg(all(target_arch = "aarch64", target_vendor = "apple"))]
fn main() {
    blake3_servil_core::initialize();
    let mut p16 = Packed::<16>::new(32);
    let mut p32 = Packed::<32>::new(32);
    let mut p48 = Packed::<48>::new(32);
    for i in 0..32 {
        for j in 0..16 { p16[i][j] = ((i * 29 + j * 11 + 3) & 0xff) as u8; }
        for j in 0..32 { p32[i][j] = ((i * 31 + j * 13 + 7) & 0xff) as u8; }
        for j in 0..48 { p48[i][j] = ((i * 37 + j * 17 + 9) & 0xff) as u8; }
    }
    let (mut o16, mut o32, mut o48) = ([[0u8; 32]; 32], [[0u8; 32]; 32], [[0u8; 32]; 32]);
    let (r16, r32, r48) = unsafe {
        (
            probe_sme2_packed16(p16.as_flattened().as_ptr(), IV.as_ptr(), ONE_BLOCK_FLAGS, o16.as_mut_ptr().cast(), 2),
            probe_sme2_packed32(p32.as_flattened().as_ptr(), IV.as_ptr(), ONE_BLOCK_FLAGS, o32.as_mut_ptr().cast(), 2),
            probe_sme2_packed48(p48.as_flattened().as_ptr(), IV.as_ptr(), ONE_BLOCK_FLAGS, o48.as_mut_ptr().cast(), 2),
        )
    };
    assert_eq!((r16, r32, r48), (16, 16, 16));
    for i in 0..32 {
        assert_eq!(o16[i], *blake3_servil_core::convenience::hash(&p16[i]).as_bytes(), "p16[{i}]");
        assert_eq!(o32[i], *blake3_servil_core::convenience::hash(&p32[i]).as_bytes(), "p32[{i}]");
        assert_eq!(o48[i], *blake3_servil_core::convenience::hash(&p48[i]).as_bytes(), "p48[{i}]");
    }
    println!("verify_all: ALL 3 Apple SME2 direct-packed kernels (16/32/48 B) bit-for-bit identical!");

    run_suite("P-core / default QoS", 1);
    let rc = unsafe { pthread_set_qos_class_self_np(0x09 /* QOS_CLASS_BACKGROUND */, 0) };
    println!("\npthread_set_qos_class_self_np(BACKGROUND) rc={rc}");
    run_suite("E-core (QOS_CLASS_BACKGROUND)", 4);
}

#[cfg(not(all(target_arch = "aarch64", target_vendor = "apple")))]
fn main() {
    println!("Apple AArch64 SME2 only");
}
