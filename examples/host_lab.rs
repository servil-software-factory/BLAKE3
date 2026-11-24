#![cfg_attr(not(target_arch = "aarch64"), allow(dead_code, unused_imports))]

use blake3_servil_core::Mode;
use blake3_servil_core::throughput::{Batch, TreeHasher};
use std::hint::black_box;
use std::time::Instant;

#[cfg(all(target_arch = "aarch64", target_vendor = "apple"))]
core::arch::global_asm!(include_str!("k468_seq_apple.S"), options(raw));

#[cfg(all(target_arch = "aarch64", not(target_vendor = "apple")))]
core::arch::global_asm!(include_str!("k468_seq_elf.S"), options(raw));

#[cfg(target_arch = "aarch64")]
unsafe extern "C" {
    fn blake3_servil_hybrid_k4(inputs: *const *const u8, blocks: u64, key: *const u32, counter: u64, packed_flags: u64, out: *mut u8);
    fn blake3_servil_hybrid_k6(inputs: *const *const u8, blocks: u64, key: *const u32, counter: u64, packed_flags: u64, out: *mut u8);
    fn blake3_servil_hybrid_k8(inputs: *const *const u8, blocks: u64, key: *const u32, counter: u64, packed_flags: u64, out: *mut u8);
    fn probe_k4_seq(inputs: *const *const u8, key: *const u32, counter: u64, flags: u32, out: *mut u8);
    fn probe_k6_seq(inputs: *const *const u8, key: *const u32, counter: u64, flags: u32, out: *mut u8);
    fn probe_k8_seq(inputs: *const *const u8, key: *const u32, counter: u64, flags: u32, out: *mut u8);
}

#[cfg(target_vendor = "apple")]
unsafe extern "C" {
    fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
}

const IV: [u32; 8] = [
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A,
    0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
];
const PACKED_CHUNK_PLAIN: u64 = (1u64 << 8) | (2u64 << 16) | (64u64 << 24);

#[cfg(target_arch = "aarch64")]
fn median(mut xs: Vec<f64>) -> f64 {
    xs.sort_by(|a, b| a.total_cmp(b));
    xs[xs.len() / 2]
}

#[cfg(target_arch = "aarch64")]
fn verify_all() {
    let mut chunks = vec![[0u8; 1024]; 8];
    for i in 0..8 {
        for j in 0..1024 {
            chunks[i][j] = ((i * 37 + j * 13 + 5) & 0xff) as u8;
        }
    }
    let ptrs: [*const u8; 8] = std::array::from_fn(|i| chunks[i].as_ptr());

    let (mut r4, mut p4) = ([[0u8; 32]; 4], [[0u8; 32]; 4]);
    let (mut r6, mut p6) = ([[0u8; 32]; 6], [[0u8; 32]; 6]);
    let (mut r8, mut p8) = ([[0u8; 32]; 8], [[0u8; 32]; 8]);
    unsafe {
        blake3_servil_hybrid_k4(ptrs.as_ptr(), 16, IV.as_ptr(), 3, PACKED_CHUNK_PLAIN, r4.as_mut_ptr().cast());
        probe_k4_seq(ptrs.as_ptr(), IV.as_ptr(), 3, 0, p4.as_mut_ptr().cast());
        blake3_servil_hybrid_k6(ptrs.as_ptr(), 16, IV.as_ptr(), 3, PACKED_CHUNK_PLAIN, r6.as_mut_ptr().cast());
        probe_k6_seq(ptrs.as_ptr(), IV.as_ptr(), 3, 0, p6.as_mut_ptr().cast());
        blake3_servil_hybrid_k8(ptrs.as_ptr(), 16, IV.as_ptr(), 3, PACKED_CHUNK_PLAIN, r8.as_mut_ptr().cast());
        probe_k8_seq(ptrs.as_ptr(), IV.as_ptr(), 3, 0, p8.as_mut_ptr().cast());
    }
    assert_eq!(p4, r4, "k4_seq mismatch");
    assert_eq!(p6, r6, "k6_seq mismatch");
    assert_eq!(p8, r8, "k8_seq mismatch");
    println!("verify_all: ALL 3 k4_seq/k6_seq/k8_seq kernels bit-for-bit identical!");
}

#[cfg(target_arch = "aarch64")]
fn time_ns_per_b(bytes: usize, reps: usize, rounds: usize, mut f: impl FnMut()) -> f64 {
    for _ in 0..4 {
        f();
    }
    let mut xs = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        let t0 = Instant::now();
        for _ in 0..reps {
            f();
        }
        xs.push(t0.elapsed().as_nanos() as f64 / (reps * bytes) as f64);
    }
    median(xs)
}

#[cfg(target_arch = "aarch64")]
fn run_suite(tag: &str, reps: usize) {
    println!("\n=== {tag} ===");
    let th = TreeHasher::new(Mode::Plain).on_this_core();
    let warm_b = Batch::new(256, 256);
    let mut warm_o = vec![[0u8; 32]; 256];
    let t0 = Instant::now();
    while t0.elapsed().as_millis() < 150 {
        th.hash_many(black_box(&warm_b), black_box(&mut warm_o));
    }

    let n = 480usize; // divisible by 4, 6, 8
    let buf = vec![0x5Au8; n * 1024];
    let ptrs: Vec<*const u8> = (0..n).map(|i| unsafe { buf.as_ptr().add(i * 1024) }).collect();
    let mut out = vec![[0u8; 32]; n];

    let tk4 = time_ns_per_b(n * 1024, reps, 15, || {
        for off in (0..n).step_by(4) {
            unsafe { blake3_servil_hybrid_k4(black_box(ptrs.as_ptr().add(off)), 16, IV.as_ptr(), off as u64, PACKED_CHUNK_PLAIN, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let tk4s = time_ns_per_b(n * 1024, reps, 15, || {
        for off in (0..n).step_by(4) {
            unsafe { probe_k4_seq(black_box(ptrs.as_ptr().add(off)), IV.as_ptr(), off as u64, 0, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let tk6 = time_ns_per_b(n * 1024, reps, 15, || {
        for off in (0..n).step_by(6) {
            unsafe { blake3_servil_hybrid_k6(black_box(ptrs.as_ptr().add(off)), 16, IV.as_ptr(), off as u64, PACKED_CHUNK_PLAIN, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let tk6s = time_ns_per_b(n * 1024, reps, 15, || {
        for off in (0..n).step_by(6) {
            unsafe { probe_k6_seq(black_box(ptrs.as_ptr().add(off)), IV.as_ptr(), off as u64, 0, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let tk8 = time_ns_per_b(n * 1024, reps, 15, || {
        for off in (0..n).step_by(8) {
            unsafe { blake3_servil_hybrid_k8(black_box(ptrs.as_ptr().add(off)), 16, IV.as_ptr(), off as u64, PACKED_CHUNK_PLAIN, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let tk8s = time_ns_per_b(n * 1024, reps, 15, || {
        for off in (0..n).step_by(8) {
            unsafe { probe_k8_seq(black_box(ptrs.as_ptr().add(off)), IV.as_ptr(), off as u64, 0, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });

    println!("  k4 (2+2 spilled) vs k4_seq (no-spill): {tk4:.4} ({:.2} GB/s) -> {tk4s:.4} ({:.2} GB/s)  ({:.2}x)", 1.0/tk4, 1.0/tk4s, tk4/tk4s);
    println!("  k6 (4+2 spilled) vs k6_seq (no-spill): {tk6:.4} ({:.2} GB/s) -> {tk6s:.4} ({:.2} GB/s)  ({:.2}x)", 1.0/tk6, 1.0/tk6s, tk6/tk6s);
    println!("  k8 (6+2 spilled) vs k8_seq (no-spill): {tk8:.4} ({:.2} GB/s) -> {tk8s:.4} ({:.2} GB/s)  ({:.2}x)", 1.0/tk8, 1.0/tk8s, tk8/tk8s);
}

fn main() {
    blake3_servil_core::initialize();
    #[cfg(target_arch = "aarch64")]
    {
        verify_all();
        run_suite("P-core / default QoS", 16);
        #[cfg(target_vendor = "apple")]
        {
            let rc = unsafe { pthread_set_qos_class_self_np(0x09 /* QOS_CLASS_BACKGROUND */, 0) };
            println!("\npthread_set_qos_class_self_np(BACKGROUND) rc={rc}");
            run_suite("E-core (QOS_CLASS_BACKGROUND)", 4);
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    println!("aarch64 only");
}
