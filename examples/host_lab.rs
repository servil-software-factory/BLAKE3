#![cfg_attr(not(target_arch = "aarch64"), allow(dead_code, unused_imports))]

use blake3_servil_core::Mode;
use blake3_servil_core::throughput::{Batch, TreeHasher};
use std::hint::black_box;
use std::time::Instant;

#[cfg(all(target_arch = "aarch64", target_vendor = "apple"))]
core::arch::global_asm!(include_str!("k11_prod_apple.S"), options(raw));
#[cfg(all(target_arch = "aarch64", not(target_vendor = "apple")))]
core::arch::global_asm!(include_str!("k11_prod_elf.S"), options(raw));

#[cfg(target_arch = "aarch64")]
unsafe extern "C" {
    fn blake3_servil_hybrid_k10(inputs: *const *const u8, blocks: u64, key: *const u32, counter: u64, packed_flags: u64, out: *mut u8);
    fn probe_k10_prod(inputs: *const *const u8, blocks: u64, key: *const u32, counter: u64, packed_flags: u64, out: *mut u8);
    fn probe_k11_prod(inputs: *const *const u8, blocks: u64, key: *const u32, counter: u64, packed_flags: u64, out: *mut u8);
    fn probe_p10_prod(inputs: *const *const u8, blocks: u64, key: *const u32, counter: u64, packed_flags: u64, out: *mut u8);
    fn probe_p11_prod(inputs: *const *const u8, blocks: u64, key: *const u32, counter: u64, packed_flags: u64, out: *mut u8);
}

#[cfg(target_vendor = "apple")]
unsafe extern "C" {
    fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
}

const IV: [u32; 8] = [
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A,
    0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
];
const PACKED_CHUNK: u64 = (1u64 << 8) | (2u64 << 16) | (64u64 << 24);
const PACKED_MSG_64: u64 = (1u64 << 8) | (10u64 << 16) | (64u64 << 24);
const PACKED_MSG_37: u64 = (1u64 << 8) | (10u64 << 16) | (37u64 << 24);

#[cfg(target_arch = "aarch64")]
fn median(mut xs: Vec<f64>) -> f64 {
    xs.sort_by(|a, b| a.total_cmp(b));
    xs[xs.len() / 2]
}

#[cfg(target_arch = "aarch64")]
fn verify_all() {
    let mut chunks = vec![[0u8; 1024]; 11];
    for i in 0..11 {
        for j in 0..1024 {
            chunks[i][j] = ((i * 41 + j * 17 + 9) & 0xff) as u8;
        }
    }
    let ptrs: [*const u8; 11] = std::array::from_fn(|i| chunks[i].as_ptr());

    // Verify 1 KiB chunks (blocks=16) against blake3_servil_hybrid_k10.
    let (mut ra, mut rb) = ([[0u8; 32]; 10], [[0u8; 32]; 10]);
    unsafe {
        blake3_servil_hybrid_k10(ptrs.as_ptr(), 16, IV.as_ptr(), 11, PACKED_CHUNK, ra.as_mut_ptr().cast());
        blake3_servil_hybrid_k10(ptrs.as_ptr().add(1), 16, IV.as_ptr(), 12, PACKED_CHUNK, rb.as_mut_ptr().cast());
    }
    let mut kref = [[0u8; 32]; 11];
    kref[..10].copy_from_slice(&ra);
    kref[1..11].copy_from_slice(&rb);
    let (mut o10, mut o11) = ([[0u8; 32]; 10], [[0u8; 32]; 11]);
    unsafe {
        probe_k10_prod(ptrs.as_ptr(), 16, IV.as_ptr(), 11, PACKED_CHUNK, o10.as_mut_ptr().cast());
        probe_k11_prod(ptrs.as_ptr(), 16, IV.as_ptr(), 11, PACKED_CHUNK, o11.as_mut_ptr().cast());
    }
    assert_eq!(o10, kref[..10], "k10_prod mismatch");
    assert_eq!(o11, kref, "k11_prod mismatch");

    // Verify p10_prod / p11_prod at blocks = 1 (37 B partial!), 1 (64 B), 4 (256 B), 16 (1024 B).
    for &(blocks, last_len, packed) in &[(1usize, 37usize, PACKED_MSG_37), (1, 64, PACKED_MSG_64), (4, 64, PACKED_MSG_64), (16, 64, PACKED_MSG_64)] {
        let mut z = vec![[0u8; 1024]; 11];
        let msg_len = (blocks - 1) * 64 + last_len;
        let mut pref = [[0u8; 32]; 11];
        for i in 0..11 {
            z[i][..msg_len].copy_from_slice(&chunks[i][..msg_len]);
            pref[i] = *blake3_servil_core::convenience::hash(&z[i][..msg_len]).as_bytes();
        }
        let zptrs: [*const u8; 11] = std::array::from_fn(|i| z[i].as_ptr());
        unsafe {
            probe_p10_prod(zptrs.as_ptr(), blocks as u64, IV.as_ptr(), 0, packed, o10.as_mut_ptr().cast());
            probe_p11_prod(zptrs.as_ptr(), blocks as u64, IV.as_ptr(), 0, packed, o11.as_mut_ptr().cast());
        }
        assert_eq!(o10, pref[..10], "p10_prod blocks={blocks} last={last_len}");
        assert_eq!(o11, pref, "p11_prod blocks={blocks} last={last_len}");
    }
    println!("verify_all: ALL production-ABI runtime-blocks kernels (blocks=1/4/16, partial last_len=37) bit-for-bit identical!");
}

#[cfg(target_arch = "aarch64")]
fn time_ns(units: usize, reps: usize, rounds: usize, mut f: impl FnMut()) -> f64 {
    for _ in 0..3 { f(); }
    let mut xs = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        let t0 = Instant::now();
        for _ in 0..reps { f(); }
        xs.push(t0.elapsed().as_nanos() as f64 / (reps * units) as f64);
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
    while t0.elapsed().as_millis() < 120 {
        th.hash_many(black_box(&warm_b), black_box(&mut warm_o));
    }

    let n = 660usize;
    let buf = vec![0x5Au8; n * 1024];
    let ptrs: Vec<*const u8> = (0..n).map(|i| unsafe { buf.as_ptr().add(i * 1024) }).collect();
    let mut out = vec![[0u8; 32]; n];

    let tk_base = time_ns(n * 1024, reps, 13, || {
        for off in (0..n).step_by(10) {
            unsafe { blake3_servil_hybrid_k10(black_box(ptrs.as_ptr().add(off)), 16, IV.as_ptr(), off as u64, PACKED_CHUNK, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let tk10 = time_ns(n * 1024, reps, 13, || {
        for off in (0..n).step_by(10) {
            unsafe { probe_k10_prod(black_box(ptrs.as_ptr().add(off)), 16, IV.as_ptr(), off as u64, PACKED_CHUNK, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let tk11 = time_ns(n * 1024, reps, 13, || {
        for off in (0..n).step_by(11) {
            unsafe { probe_k11_prod(black_box(ptrs.as_ptr().add(off)), 16, IV.as_ptr(), off as u64, PACKED_CHUNK, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    println!("  1 KiB chunks (GB/s): next_k10={:.2} | k10_prod={:.2} ({:.2}x) | k11_prod={:.2} ({:.2}x)",
        1.0/tk_base, 1.0/tk10, tk_base/tk10, 1.0/tk11, tk_base/tk11);

    // 256 B Merkle leaves (blocks=4) via runtime-blocks p10_prod / p11_prod vs next NEON
    let b256 = Batch::new(256, n);
    let t256_next = time_ns(n, reps * 2, 13, || th.hash_many(black_box(&b256), black_box(&mut out)));
    let tp10_4 = time_ns(n, reps * 2, 13, || {
        for off in (0..n).step_by(10) {
            unsafe { probe_p10_prod(black_box(ptrs.as_ptr().add(off)), 4, IV.as_ptr(), 0, PACKED_MSG_64, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let tp11_4 = time_ns(n, reps * 2, 13, || {
        for off in (0..n).step_by(11) {
            unsafe { probe_p11_prod(black_box(ptrs.as_ptr().add(off)), 4, IV.as_ptr(), 0, PACKED_MSG_64, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    println!("  256 B leaves (ns/h): next_neon={t256_next:.2} | p10_prod={tp10_4:.2} ({:.2}x) | p11_prod={tp11_4:.2} ({:.2}x)",
        t256_next/tp10_4, t256_next/tp11_4);

    // 64 B messages (blocks=1) via runtime-blocks p10_prod / p11_prod vs next NEON
    let b64 = Batch::new(64, n);
    let t64_next = time_ns(n, reps * 8, 13, || th.hash_many(black_box(&b64), black_box(&mut out)));
    let tp10_1 = time_ns(n, reps * 8, 13, || {
        for off in (0..n).step_by(10) {
            unsafe { probe_p10_prod(black_box(ptrs.as_ptr().add(off)), 1, IV.as_ptr(), 0, PACKED_MSG_64, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let tp11_1 = time_ns(n, reps * 8, 13, || {
        for off in (0..n).step_by(11) {
            unsafe { probe_p11_prod(black_box(ptrs.as_ptr().add(off)), 1, IV.as_ptr(), 0, PACKED_MSG_64, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    println!("  64 B msgs    (ns/h): next_neon={t64_next:.2} | p10_prod={tp10_1:.2} ({:.2}x) | p11_prod={tp11_1:.2} ({:.2}x)",
        t64_next/tp10_1, t64_next/tp11_1);
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
