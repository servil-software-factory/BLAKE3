use std::hint::black_box;
use std::time::Instant;

#[cfg(target_arch = "aarch64")]
#[inline(never)]
fn eq_plain(a: &[u8; 32], b: &[u8; 32]) -> bool {
    blake3_servil_core::Hash::from_bytes(*a) == blake3_servil_core::Hash::from_bytes(*b)
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
fn read_dit() -> u64 {
    let v: u64;
    // Sound: reading architectural DIT register (S3_3_C4_C2_5) when FEAT_DIT is present.
    unsafe { core::arch::asm!("mrs {}, S3_3_C4_C2_5", out(reg) v, options(nomem, nostack)) };
    (v >> 24) & 1
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
fn write_dit(bit: u64) {
    let v = (bit & 1) << 24;
    // Sound: writing architectural DIT register (S3_3_C4_C2_5) when FEAT_DIT is present.
    unsafe { core::arch::asm!("msr S3_3_C4_C2_5, {}", in(reg) v, options(nomem, nostack)) };
}

#[cfg(target_arch = "aarch64")]
#[inline(never)]
fn eq_scoped_dit(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let saved = read_dit();
    if saved == 0 {
        write_dit(1);
    }
    let res = blake3_servil_core::Hash::from_bytes(*a) == blake3_servil_core::Hash::from_bytes(*b);
    if saved == 0 {
        write_dit(0);
    }
    res
}

fn bench(label: &str, f: fn(&[u8; 32], &[u8; 32]) -> bool) {
    let a = [0x5au8; 32];
    let mut b = [0x5au8; 32];
    b[31] = 0x5b;
    const ITERS: u64 = 5_000_000;
    let mut ns_list = Vec::new();
    for _ in 0..7 {
        let mut acc = 0u64;
        let t0 = Instant::now();
        for i in 0..ITERS {
            let rhs = if i & 1 == 0 { &a } else { &b };
            acc += f(black_box(&a), black_box(rhs)) as u64;
        }
        let dt = t0.elapsed();
        black_box(acc);
        ns_list.push(dt.as_nanos() as f64 / ITERS as f64);
    }
    ns_list.sort_by(|x, y| x.total_cmp(y));
    println!("{label}: median={:.3} ns/call  min={:.3}  max={:.3}", ns_list[3], ns_list[0], ns_list[6]);
}

fn main() {
    blake3_servil_core::initialize();
    #[cfg(target_arch = "aarch64")]
    {
        let initial_dit = read_dit();
        println!("initial_dit={initial_dit}");
        bench("plain (initial_dit)", eq_plain);
        write_dit(1);
        println!("after write_dit(1): dit={}", read_dit());
        bench("scoped_dit (DIT=1)", eq_scoped_dit);
        write_dit(0);
        println!("after write_dit(0): dit={}", read_dit());
        bench("plain (DIT=0)", eq_plain);
        bench("scoped_dit (DIT=0)", eq_scoped_dit);
        write_dit(initial_dit);
    }
}
