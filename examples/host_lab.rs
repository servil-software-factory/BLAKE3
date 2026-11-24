#![cfg_attr(not(target_arch = "aarch64"), allow(dead_code, unused_imports))]

use blake3_servil_core::Mode;
use blake3_servil_core::throughput::{Batch, TreeHasher};
use std::hint::black_box;
use std::time::Instant;

#[cfg(all(target_arch = "aarch64", target_vendor = "apple"))]
core::arch::global_asm!(include_str!("k11_seq_apple.S"), options(raw));
#[cfg(all(target_arch = "aarch64", target_vendor = "apple"))]
core::arch::global_asm!(include_str!("p10_p11_apple.S"), options(raw));

#[cfg(all(target_arch = "aarch64", not(target_vendor = "apple")))]
core::arch::global_asm!(include_str!("k11_seq_elf.S"), options(raw));
#[cfg(all(target_arch = "aarch64", not(target_vendor = "apple")))]
core::arch::global_asm!(include_str!("p10_p11_elf.S"), options(raw));

#[cfg(target_arch = "aarch64")]
unsafe extern "C" {
    fn blake3_servil_hybrid_k10(inputs: *const *const u8, blocks: u64, key: *const u32, counter: u64, packed_flags: u64, out: *mut u8);
    fn probe_k10_seq_16(inputs: *const *const u8, key: *const u32, counter: u64, flags: u32, out: *mut u8);
    fn probe_k11_seq_16(inputs: *const *const u8, key: *const u32, counter: u64, flags: u32, out: *mut u8);
    fn probe_k12_seq_16(inputs: *const *const u8, key: *const u32, counter: u64, flags: u32, out: *mut u8);
    fn probe_k13_seq_16(inputs: *const *const u8, key: *const u32, counter: u64, flags: u32, out: *mut u8);
    fn probe_m10_seq_4(inputs: *const *const u8, key: *const u32, counter: u64, flags: u32, out: *mut u8);
    fn probe_m11_seq_4(inputs: *const *const u8, key: *const u32, counter: u64, flags: u32, out: *mut u8);
    fn probe_m12_seq_4(inputs: *const *const u8, key: *const u32, counter: u64, flags: u32, out: *mut u8);
    fn probe_m13_seq_4(inputs: *const *const u8, key: *const u32, counter: u64, flags: u32, out: *mut u8);
    fn probe_p10_64_steps(inputs: *const *const u8, key: *const u32, counter: u64, flags: u32, block_len: u32, out: *mut u8);
    fn probe_p11_64_steps(inputs: *const *const u8, key: *const u32, counter: u64, flags: u32, block_len: u32, out: *mut u8);
    fn probe_p12_64_steps(inputs: *const *const u8, key: *const u32, counter: u64, flags: u32, block_len: u32, out: *mut u8);
    fn probe_p13_64_steps(inputs: *const *const u8, key: *const u32, counter: u64, flags: u32, block_len: u32, out: *mut u8);
    fn probe_p10_packed32_steps(base: *const u8, key: *const u32, counter: u64, flags: u32, block_len: u32, out: *mut u8);
    fn probe_p11_packed32_steps(base: *const u8, key: *const u32, counter: u64, flags: u32, block_len: u32, out: *mut u8);
    fn probe_p12_packed32_steps(base: *const u8, key: *const u32, counter: u64, flags: u32, block_len: u32, out: *mut u8);
    fn probe_p13_packed32_steps(base: *const u8, key: *const u32, counter: u64, flags: u32, block_len: u32, out: *mut u8);
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
const ONE_BLOCK_FLAGS: u32 = 1 | 2 | 8;

#[cfg(target_arch = "aarch64")]
fn median(mut xs: Vec<f64>) -> f64 {
    xs.sort_by(|a, b| a.total_cmp(b));
    xs[xs.len() / 2]
}

#[cfg(target_arch = "aarch64")]
fn verify_all() {
    let mut chunks = vec![[0u8; 1024]; 13];
    for i in 0..13 {
        for j in 0..1024 {
            chunks[i][j] = ((i * 37 + j * 13 + 5) & 0xff) as u8;
        }
    }
    let ptrs: [*const u8; 13] = std::array::from_fn(|i| chunks[i].as_ptr());

    // Reference 1 KiB chunks via k10 on 0..10 and 3..13
    let (mut ref_a, mut ref_b) = ([[0u8; 32]; 10], [[0u8; 32]; 10]);
    unsafe {
        blake3_servil_hybrid_k10(ptrs.as_ptr(), 16, IV.as_ptr(), 7, PACKED_CHUNK_PLAIN, ref_a.as_mut_ptr().cast());
        blake3_servil_hybrid_k10(ptrs.as_ptr().add(3), 16, IV.as_ptr(), 10, PACKED_CHUNK_PLAIN, ref_b.as_mut_ptr().cast());
    }
    let mut kref = [[0u8; 32]; 13];
    kref[..10].copy_from_slice(&ref_a);
    kref[3..13].copy_from_slice(&ref_b);

    let (mut o10, mut o11, mut o12, mut o13) = ([[0u8; 32]; 10], [[0u8; 32]; 11], [[0u8; 32]; 12], [[0u8; 32]; 13]);
    unsafe {
        probe_k10_seq_16(ptrs.as_ptr(), IV.as_ptr(), 7, 0, o10.as_mut_ptr().cast());
        probe_k11_seq_16(ptrs.as_ptr(), IV.as_ptr(), 7, 0, o11.as_mut_ptr().cast());
        probe_k12_seq_16(ptrs.as_ptr(), IV.as_ptr(), 7, 0, o12.as_mut_ptr().cast());
        probe_k13_seq_16(ptrs.as_ptr(), IV.as_ptr(), 7, 0, o13.as_mut_ptr().cast());
    }
    assert_eq!(o10, kref[..10], "k10 mismatch");
    assert_eq!(o11, kref[..11], "k11 mismatch");
    assert_eq!(o12, kref[..12], "k12 mismatch");
    assert_eq!(o13, kref[..13], "k13 mismatch");

    let mut mref = [[0u8; 32]; 13];
    let mut p64ref = [[0u8; 32]; 13];
    let mut p32ref = [[0u8; 32]; 13];
    let mut p32buf = [0u8; 13 * 32];
    for i in 0..13 {
        mref[i] = *blake3_servil_core::convenience::hash(&chunks[i][..256]).as_bytes();
        p64ref[i] = *blake3_servil_core::convenience::hash(&chunks[i][..64]).as_bytes();
        p32buf[i * 32..(i + 1) * 32].copy_from_slice(&chunks[i][..32]);
        p32ref[i] = *blake3_servil_core::convenience::hash(&chunks[i][..32]).as_bytes();
    }
    unsafe {
        probe_m10_seq_4(ptrs.as_ptr(), IV.as_ptr(), 0, 0, o10.as_mut_ptr().cast());
        probe_m11_seq_4(ptrs.as_ptr(), IV.as_ptr(), 0, 0, o11.as_mut_ptr().cast());
        probe_m12_seq_4(ptrs.as_ptr(), IV.as_ptr(), 0, 0, o12.as_mut_ptr().cast());
        probe_m13_seq_4(ptrs.as_ptr(), IV.as_ptr(), 0, 0, o13.as_mut_ptr().cast());
    }
    assert_eq!(o10, mref[..10], "m10 mismatch");
    assert_eq!(o11, mref[..11], "m11 mismatch");
    assert_eq!(o12, mref[..12], "m12 mismatch");
    assert_eq!(o13, mref[..13], "m13 mismatch");

    unsafe {
        probe_p10_64_steps(ptrs.as_ptr(), IV.as_ptr(), 0, ONE_BLOCK_FLAGS, 64, o10.as_mut_ptr().cast());
        probe_p11_64_steps(ptrs.as_ptr(), IV.as_ptr(), 0, ONE_BLOCK_FLAGS, 64, o11.as_mut_ptr().cast());
        probe_p12_64_steps(ptrs.as_ptr(), IV.as_ptr(), 0, ONE_BLOCK_FLAGS, 64, o12.as_mut_ptr().cast());
        probe_p13_64_steps(ptrs.as_ptr(), IV.as_ptr(), 0, ONE_BLOCK_FLAGS, 64, o13.as_mut_ptr().cast());
    }
    assert_eq!(o10, p64ref[..10], "p10_64 mismatch");
    assert_eq!(o11, p64ref[..11], "p11_64 mismatch");
    assert_eq!(o12, p64ref[..12], "p12_64 mismatch");
    assert_eq!(o13, p64ref[..13], "p13_64 mismatch");

    unsafe {
        probe_p10_packed32_steps(p32buf.as_ptr(), IV.as_ptr(), 0, ONE_BLOCK_FLAGS, 32, o10.as_mut_ptr().cast());
        probe_p11_packed32_steps(p32buf.as_ptr(), IV.as_ptr(), 0, ONE_BLOCK_FLAGS, 32, o11.as_mut_ptr().cast());
        probe_p12_packed32_steps(p32buf.as_ptr(), IV.as_ptr(), 0, ONE_BLOCK_FLAGS, 32, o12.as_mut_ptr().cast());
        probe_p13_packed32_steps(p32buf.as_ptr(), IV.as_ptr(), 0, ONE_BLOCK_FLAGS, 32, o13.as_mut_ptr().cast());
    }
    assert_eq!(o10, p32ref[..10], "p10_32 mismatch");
    assert_eq!(o11, p32ref[..11], "p11_32 mismatch");
    assert_eq!(o12, p32ref[..12], "p12_32 mismatch");
    assert_eq!(o13, p32ref[..13], "p13_32 mismatch");
    println!("verify_all: ALL 16 kernels (k/m/p64/p32 x 10..13 lanes) bit-for-bit identical!");
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
fn run_suite(tag: &str, reps_k: usize, reps_p: usize) {
    println!("\n=== {tag} ===");
    let th = TreeHasher::new(Mode::Plain).on_this_core();
    let warm_b = Batch::new(256, 256);
    let mut warm_o = vec![[0u8; 32]; 256];
    let t0 = Instant::now();
    while t0.elapsed().as_millis() < 120 {
        th.hash_many(black_box(&warm_b), black_box(&mut warm_o));
    }

    // 1) 1 KiB chunks (count = 8580 / 260 — use 660 for 10/11/12 and 650 for 13)
    let buf = vec![0x5Au8; 660 * 1024];
    let ptrs: Vec<*const u8> = (0..660).map(|i| unsafe { buf.as_ptr().add(i * 1024) }).collect();
    let mut out = vec![[0u8; 32]; 660];

    let tk_base = time_ns(660 * 1024, reps_k, 13, || {
        for off in (0..660).step_by(10) {
            unsafe { blake3_servil_hybrid_k10(black_box(ptrs.as_ptr().add(off)), 16, IV.as_ptr(), off as u64, PACKED_CHUNK_PLAIN, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let tk10 = time_ns(660 * 1024, reps_k, 13, || {
        for off in (0..660).step_by(10) {
            unsafe { probe_k10_seq_16(black_box(ptrs.as_ptr().add(off)), IV.as_ptr(), off as u64, 0, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let tk11 = time_ns(660 * 1024, reps_k, 13, || {
        for off in (0..660).step_by(11) {
            unsafe { probe_k11_seq_16(black_box(ptrs.as_ptr().add(off)), IV.as_ptr(), off as u64, 0, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let tk12 = time_ns(660 * 1024, reps_k, 13, || {
        for off in (0..660).step_by(12) {
            unsafe { probe_k12_seq_16(black_box(ptrs.as_ptr().add(off)), IV.as_ptr(), off as u64, 0, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let tk13 = time_ns(650 * 1024, reps_k, 13, || {
        for off in (0..650).step_by(13) {
            unsafe { probe_k13_seq_16(black_box(ptrs.as_ptr().add(off)), IV.as_ptr(), off as u64, 0, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    println!("  1 KiB chunks (GB/s): next_k10={:.2} | k10={:.2} | k11={:.2} | k12={:.2} | k13={:.2}",
        1.0/tk_base, 1.0/tk10, 1.0/tk11, 1.0/tk12, 1.0/tk13);

    // 2) 256 B leaves (ns/leaf)
    let tm10 = time_ns(660, reps_k * 2, 13, || {
        for off in (0..660).step_by(10) {
            unsafe { probe_m10_seq_4(black_box(ptrs.as_ptr().add(off)), IV.as_ptr(), 0, 0, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let tm11 = time_ns(660, reps_k * 2, 13, || {
        for off in (0..660).step_by(11) {
            unsafe { probe_m11_seq_4(black_box(ptrs.as_ptr().add(off)), IV.as_ptr(), 0, 0, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let tm12 = time_ns(660, reps_k * 2, 13, || {
        for off in (0..660).step_by(12) {
            unsafe { probe_m12_seq_4(black_box(ptrs.as_ptr().add(off)), IV.as_ptr(), 0, 0, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let tm13 = time_ns(650, reps_k * 2, 13, || {
        for off in (0..650).step_by(13) {
            unsafe { probe_m13_seq_4(black_box(ptrs.as_ptr().add(off)), IV.as_ptr(), 0, 0, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    println!("  256 B leaves (ns/h): m10={tm10:.2} | m11={tm11:.2} | m12={tm12:.2} | m13={tm13:.2}");

    // 3) 64 B messages (ns/h)
    let tp64_10 = time_ns(660, reps_p, 13, || {
        for off in (0..660).step_by(10) {
            unsafe { probe_p10_64_steps(black_box(ptrs.as_ptr().add(off)), IV.as_ptr(), 0, ONE_BLOCK_FLAGS, 64, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let tp64_11 = time_ns(660, reps_p, 13, || {
        for off in (0..660).step_by(11) {
            unsafe { probe_p11_64_steps(black_box(ptrs.as_ptr().add(off)), IV.as_ptr(), 0, ONE_BLOCK_FLAGS, 64, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let tp64_12 = time_ns(660, reps_p, 13, || {
        for off in (0..660).step_by(12) {
            unsafe { probe_p12_64_steps(black_box(ptrs.as_ptr().add(off)), IV.as_ptr(), 0, ONE_BLOCK_FLAGS, 64, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let tp64_13 = time_ns(650, reps_p, 13, || {
        for off in (0..650).step_by(13) {
            unsafe { probe_p13_64_steps(black_box(ptrs.as_ptr().add(off)), IV.as_ptr(), 0, ONE_BLOCK_FLAGS, 64, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    println!("  64 B msgs    (ns/h): p10={tp64_10:.2} | p11={tp64_11:.2} | p12={tp64_12:.2} | p13={tp64_13:.2}");

    // 4) Packed<32> (ns/h)
    let p32 = vec![0x33u8; 660 * 32];
    let tp32_10 = time_ns(660, reps_p, 13, || {
        for off in (0..660).step_by(10) {
            unsafe { probe_p10_packed32_steps(black_box(p32.as_ptr().add(off * 32)), IV.as_ptr(), 0, ONE_BLOCK_FLAGS, 32, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let tp32_11 = time_ns(660, reps_p, 13, || {
        for off in (0..660).step_by(11) {
            unsafe { probe_p11_packed32_steps(black_box(p32.as_ptr().add(off * 32)), IV.as_ptr(), 0, ONE_BLOCK_FLAGS, 32, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let tp32_12 = time_ns(660, reps_p, 13, || {
        for off in (0..660).step_by(12) {
            unsafe { probe_p12_packed32_steps(black_box(p32.as_ptr().add(off * 32)), IV.as_ptr(), 0, ONE_BLOCK_FLAGS, 32, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    let tp32_13 = time_ns(650, reps_p, 13, || {
        for off in (0..650).step_by(13) {
            unsafe { probe_p13_packed32_steps(black_box(p32.as_ptr().add(off * 32)), IV.as_ptr(), 0, ONE_BLOCK_FLAGS, 32, black_box(out.as_mut_ptr().add(off).cast())) };
        }
    });
    println!("  Packed<32>   (ns/h): p10={tp32_10:.2} | p11={tp32_11:.2} | p12={tp32_12:.2} | p13={tp32_13:.2}");
}

fn main() {
    blake3_servil_core::initialize();
    #[cfg(target_arch = "aarch64")]
    {
        verify_all();
        run_suite("P-core / default QoS", 8, 64);
        #[cfg(target_vendor = "apple")]
        {
            let rc = unsafe { pthread_set_qos_class_self_np(0x09 /* QOS_CLASS_BACKGROUND */, 0) };
            println!("\npthread_set_qos_class_self_np(BACKGROUND) rc={rc}");
            run_suite("E-core (QOS_CLASS_BACKGROUND)", 2, 16);
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    println!("aarch64 only");
}
