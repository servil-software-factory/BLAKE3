#![cfg_attr(not(target_arch = "aarch64"), allow(dead_code, unused_imports))]

use blake3_servil_core::Mode;
use blake3_servil_core::convenience::hash_with;
use blake3_servil_core::throughput::{Batch, TreeHasher};
use std::hint::black_box;
use std::time::Instant;

#[cfg(all(target_arch = "aarch64", target_vendor = "apple"))]
core::arch::global_asm!(include_str!("k11_seq_apple.S"), options(raw));

#[cfg(all(target_arch = "aarch64", not(target_vendor = "apple")))]
core::arch::global_asm!(include_str!("k11_seq_elf.S"), options(raw));

#[cfg(target_arch = "aarch64")]
unsafe extern "C" {
    fn blake3_servil_hybrid_k9(inputs: *const *const u8, blocks: u64, key: *const u32, counter: u64, packed_flags: u64, out: *mut u8);
    fn blake3_servil_hybrid_k10(inputs: *const *const u8, blocks: u64, key: *const u32, counter: u64, packed_flags: u64, out: *mut u8);
    fn probe_k10_seq_16(inputs: *const *const u8, key: *const u32, counter: u64, flags: u32, out: *mut u8);
    fn probe_k11_seq_16(inputs: *const *const u8, key: *const u32, counter: u64, flags: u32, out: *mut u8);
    fn probe_m10_seq_4(inputs: *const *const u8, key: *const u32, counter: u64, flags: u32, out: *mut u8);
    fn probe_m11_seq_4(inputs: *const *const u8, key: *const u32, counter: u64, flags: u32, out: *mut u8);
}

#[cfg(target_vendor = "apple")]
unsafe extern "C" {
    fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
}

const IV: [u32; 8] = [
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A,
    0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
];
// packed_flags = flags | (CHUNK_START << 8) | (CHUNK_END << 16) | (64 << 24)
const PACKED_CHUNK_PLAIN: u64 = (1u64 << 8) | (2u64 << 16) | (64u64 << 24);

#[cfg(target_arch = "aarch64")]
fn median(mut xs: Vec<f64>) -> f64 {
    xs.sort_by(|a, b| a.total_cmp(b));
    xs[xs.len() / 2]
}

#[cfg(target_arch = "aarch64")]
fn verify_all() {
    // 1. Verify 1 KiB chunks against blake3_servil_hybrid_k10 (with non-zero counter = 7!)
    let mut chunks = vec![[0u8; 1024]; 11];
    for i in 0..11 {
        for j in 0..1024 {
            chunks[i][j] = ((i * 41 + j * 13 + 7) & 0xff) as u8;
        }
    }
    let ptrs: [*const u8; 11] = std::array::from_fn(|i| chunks[i].as_ptr());
    let mut ref_k10 = [[0u8; 32]; 10];
    let mut ref_k10_shift = [[0u8; 32]; 10];
    unsafe {
        blake3_servil_hybrid_k10(ptrs.as_ptr(), 16, IV.as_ptr(), 7, PACKED_CHUNK_PLAIN, ref_k10.as_mut_ptr().cast());
        blake3_servil_hybrid_k10(ptrs[1..].as_ptr(), 16, IV.as_ptr(), 8, PACKED_CHUNK_PLAIN, ref_k10_shift.as_mut_ptr().cast());
    }
    let mut out10 = [[0u8; 32]; 10];
    let mut out11 = [[0u8; 32]; 11];
    unsafe {
        probe_k10_seq_16(ptrs.as_ptr(), IV.as_ptr(), 7, 0, out10.as_mut_ptr().cast());
        probe_k11_seq_16(ptrs.as_ptr(), IV.as_ptr(), 7, 0, out11.as_mut_ptr().cast());
    }
    assert_eq!(out10, ref_k10, "k10_seq_16 mismatch");
    assert_eq!(out11[..10], ref_k10[..10], "k11_seq_16[0..10] mismatch");
    assert_eq!(out11[10], ref_k10_shift[9], "k11_seq_16[10] mismatch");

    // 2. Verify 256 B Merkle leaves against convenience::hash_with
    let mut leaves = vec![[0u8; 256]; 11];
    for i in 0..11 {
        for j in 0..256 {
            leaves[i][j] = ((i * 53 + j * 19 + 3) & 0xff) as u8;
        }
    }
    let lptrs: [*const u8; 11] = std::array::from_fn(|i| leaves[i].as_ptr());
    let mut m10 = [[0u8; 32]; 10];
    let mut m11 = [[0u8; 32]; 11];
    unsafe {
        probe_m10_seq_4(lptrs.as_ptr(), IV.as_ptr(), 0, 0, m10.as_mut_ptr().cast());
        probe_m11_seq_4(lptrs.as_ptr(), IV.as_ptr(), 0, 0, m11.as_mut_ptr().cast());
    }
    for i in 0..10 {
        assert_eq!(m10[i], *hash_with(Mode::Plain, &leaves[i]).as_bytes(), "m10_seq_4 lane {i}");
    }
    for i in 0..11 {
        assert_eq!(m11[i], *hash_with(Mode::Plain, &leaves[i]).as_bytes(), "m11_seq_4 lane {i}");
    }
    println!("verify_all: ALL 4 multi-block sequential-scalar kernels bit-for-bit identical!");
}

#[cfg(target_arch = "aarch64")]
fn time_ns_per_unit(units: usize, reps: usize, rounds: usize, mut f: impl FnMut()) -> f64 {
    for _ in 0..4 {
        f();
    }
    let mut xs = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        let t0 = Instant::now();
        for _ in 0..reps {
            f();
        }
        xs.push(t0.elapsed().as_nanos() as f64 / (reps * units) as f64);
    }
    median(xs)
}

#[cfg(target_arch = "aarch64")]
fn run_suite(tag: &str, reps_chunks: usize) {
    println!("\n=== {tag} ===");
    let th_neon = TreeHasher::new(Mode::Plain).on_this_core();
    let warm_b = Batch::new(256, 256);
    let mut warm_o = vec![[0u8; 32]; 256];
    let warm_t0 = Instant::now();
    while warm_t0.elapsed().as_millis() < 150 {
        th_neon.hash_many(black_box(&warm_b), black_box(&mut warm_o));
    }

    // 1. 1 KiB chunks (n = 990 chunks -> ~1 MiB, divisible by 9, 10, 11)
    let n_chunks = 990usize;
    let buf = vec![0x5Au8; n_chunks * 1024];
    let ptrs: Vec<*const u8> = (0..n_chunks).map(|i| unsafe { buf.as_ptr().add(i * 1024) }).collect();
    let mut out = vec![[0u8; 32]; n_chunks];

    let t_k9 = time_ns_per_unit(n_chunks * 1024, reps_chunks, 15, || {
        for off in (0..n_chunks).step_by(9) {
            unsafe { blake3_servil_hybrid_k9(black_box(ptrs.as_ptr().add(off)), 16, IV.as_ptr(), off as u64, PACKED_CHUNK_PLAIN, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let t_k10 = time_ns_per_unit(n_chunks * 1024, reps_chunks, 15, || {
        for off in (0..n_chunks).step_by(10) {
            unsafe { blake3_servil_hybrid_k10(black_box(ptrs.as_ptr().add(off)), 16, IV.as_ptr(), off as u64, PACKED_CHUNK_PLAIN, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let t_k10_seq = time_ns_per_unit(n_chunks * 1024, reps_chunks, 15, || {
        for off in (0..n_chunks).step_by(10) {
            unsafe { probe_k10_seq_16(black_box(ptrs.as_ptr().add(off)), IV.as_ptr(), off as u64, 0, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let t_k11_seq = time_ns_per_unit(n_chunks * 1024, reps_chunks, 15, || {
        for off in (0..n_chunks).step_by(11) {
            unsafe { probe_k11_seq_16(black_box(ptrs.as_ptr().add(off)), IV.as_ptr(), off as u64, 0, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    println!("1 KiB chunks (990 chunks = 990 KiB, ns/B & GB/s):");
    println!("  next k9  (8+1 zero-spill):   {t_k9:.4} ns/B  ({:.2} GB/s)  ({:.2}x vs k10)", 1.0 / t_k9, t_k10 / t_k9);
    println!("  next k10 (8+2 spilled):      {t_k10:.4} ns/B  ({:.2} GB/s)  (1.00x)", 1.0 / t_k10);
    println!("  probe k10_seq (8+2 no-spil): {t_k10_seq:.4} ns/B  ({:.2} GB/s)  ({:.2}x vs k10)", 1.0 / t_k10_seq, t_k10 / t_k10_seq);
    println!("  probe k11_seq (8+3 no-spil): {t_k11_seq:.4} ns/B  ({:.2} GB/s)  ({:.2}x vs k10)", 1.0 / t_k11_seq, t_k10 / t_k11_seq);

    // 2. 256 B Merkle leaves (n = 990 leaves)
    let mut b256 = Batch::new(256, n_chunks);
    for i in 0..n_chunks {
        b256.message_mut(i).fill((i & 0xff) as u8);
    }
    let lptrs: Vec<*const u8> = (0..n_chunks).map(|i| b256.message_mut(i).as_ptr()).collect();
    let t_next_256 = time_ns_per_unit(n_chunks, reps_chunks * 4, 15, || {
        th_neon.hash_many(black_box(&b256), black_box(&mut out));
    });
    let t_m10 = time_ns_per_unit(n_chunks, reps_chunks * 4, 15, || {
        for off in (0..n_chunks).step_by(10) {
            unsafe { probe_m10_seq_4(black_box(lptrs.as_ptr().add(off)), IV.as_ptr(), 0, 0, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let t_m11 = time_ns_per_unit(n_chunks, reps_chunks * 4, 15, || {
        for off in (0..n_chunks).step_by(11) {
            unsafe { probe_m11_seq_4(black_box(lptrs.as_ptr().add(off)), IV.as_ptr(), 0, 0, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    println!("256 B Merkle leaves (990 leaves, ns/leaf):");
    println!("  next NEON Batch(256):        {t_next_256:6.2} ns/leaf  (1.00x)");
    println!("  probe m10_seq (8+2 no-spil): {t_m10:6.2} ns/leaf  ({:.2}x vs next NEON)", t_next_256 / t_m10);
    println!("  probe m11_seq (8+3 no-spil): {t_m11:6.2} ns/leaf  ({:.2}x vs next NEON)", t_next_256 / t_m11);
}

fn main() {
    blake3_servil_core::initialize();
    #[cfg(target_arch = "aarch64")]
    {
        verify_all();
        run_suite("P-core / default QoS", 8);
        #[cfg(target_vendor = "apple")]
        {
            let rc = unsafe { pthread_set_qos_class_self_np(0x09 /* QOS_CLASS_BACKGROUND */, 0) };
            println!("\npthread_set_qos_class_self_np(BACKGROUND) rc={rc}");
            run_suite("E-core (QOS_CLASS_BACKGROUND)", 2);
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    println!("aarch64 only");
}
