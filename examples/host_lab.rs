use blake3_servil::throughput::{self, Batch, Packed, Prepared};
use blake3_servil::Mode;
use std::hint::black_box;
use std::time::Instant;

fn median_ns_per_msg(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(|a, b| a.total_cmp(b));
    samples[samples.len() / 2]
}

fn measure<P: Prepared>(label: &str, count: usize, batch: &P) {
    let mut out_st = vec![[0u8; 32]; count];
    let mut out_mt = vec![[0u8; 32]; count];

    // Verify correctness once and warm caches + pool.
    throughput::hash_many(Mode::Plain, batch, &mut out_st);
    throughput::probe_hash_many_mt(Mode::Plain, batch, &mut out_mt);
    assert_eq!(out_st, out_mt, "{label} count={count} mismatch");

    let reps = (16384 / count).clamp(4, 128);
    let rounds = 15usize;

    let mut st_samples = Vec::with_capacity(rounds);
    let mut mt_samples = Vec::with_capacity(rounds);

    // Interleave ST and MT rounds so thermal / frequency drift affects both equally.
    for _ in 0..rounds {
        let t0 = Instant::now();
        for _ in 0..reps {
            throughput::hash_many(Mode::Plain, black_box(batch), black_box(&mut out_st));
        }
        let dt = t0.elapsed().as_nanos() as f64 / (reps * count) as f64;
        st_samples.push(dt);

        let t0 = Instant::now();
        for _ in 0..reps {
            throughput::probe_hash_many_mt(Mode::Plain, black_box(batch), black_box(&mut out_mt));
        }
        let dt = t0.elapsed().as_nanos() as f64 / (reps * count) as f64;
        mt_samples.push(dt);
    }

    let st = median_ns_per_msg(st_samples);
    let mt = median_ns_per_msg(mt_samples);
    let ratio = st / mt;
    let bytes = batch.bytes().len();
    println!("{label:12} count={count:6} bytes={bytes:8}  st={st:7.2} ns/msg  mt={mt:7.2} ns/msg  speedup={ratio:5.2}x");
}

fn main() {
    blake3_servil::initialize_multithreaded(None);
    let counts = [256usize, 512, 1024, 2048, 4096, 8192, 16384, 32768];

    // Warm the pool before timing.
    let warm = Batch::new(64, 16384);
    let mut warm_out = vec![[0u8; 32]; 16384];
    for _ in 0..5 {
        throughput::probe_hash_many_mt(Mode::Plain, &warm, &mut warm_out);
    }

    for &count in &counts {
        let mut p16 = Packed::<16>::new(count);
        for i in 0..count {
            p16[i] = [(i & 0xff) as u8; 16];
        }
        measure("Packed<16>", count, &p16);
    }
    println!();
    for &count in &counts {
        let mut p32 = Packed::<32>::new(count);
        for i in 0..count {
            p32[i] = [(i & 0xff) as u8; 32];
        }
        measure("Packed<32>", count, &p32);
    }
    println!();
    for &count in &counts {
        let mut b64 = Batch::new(64, count);
        for i in 0..count {
            b64.message_mut(i).fill((i & 0xff) as u8);
        }
        measure("Batch(64)", count, &b64);
    }
    println!();
    for &count in &counts {
        let mut b256 = Batch::new(256, count);
        for i in 0..count {
            b256.message_mut(i).fill((i & 0xff) as u8);
        }
        measure("Batch(256)", count, &b256);
    }
}
