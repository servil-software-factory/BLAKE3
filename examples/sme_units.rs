use std::hint::black_box;
use std::time::Instant;

use blake3_servil_core::throughput::{Batch, TreeHasher};
use blake3_servil_core::{Mode, initialize};

fn median(mut xs: Vec<f64>) -> f64 {
    xs.sort_by(|a, b| a.total_cmp(b));
    xs[xs.len() / 2]
}

fn bench_grinding(tree: &TreeHasher, label: &str, msg_len: usize, batch_n: usize, total_tries: usize) {
    let mut batch = Batch::new(msg_len, batch_n);
    for i in 0..batch_n {
        let m = batch.message_mut(i);
        for (j, b) in m.iter_mut().enumerate() {
            *b = ((j * 7 + 3) & 0xff) as u8;
        }
    }
    let mut out = vec![[0u8; 32]; batch_n];
    let iters = (total_tries / batch_n).max(1);

    for _ in 0..4 {
        tree.hash_many(&batch, &mut out);
        black_box(&out);
    }

    // 1. Pure hash_many (theoretical floor)
    let mut pure_ns = Vec::with_capacity(11);
    for _ in 0..11 {
        let t0 = Instant::now();
        for _ in 0..iters {
            tree.hash_many(black_box(&batch), &mut out);
            black_box(&out);
        }
        pure_ns.push(t0.elapsed().as_nanos() as f64 / (iters * batch_n) as f64);
    }

    // 2. Stamp-only (message_mut counter write) + hash_many (no output scan)
    let mut stamp_only_ns = Vec::with_capacity(11);
    let mut ctr: u32 = 1;
    for _ in 0..11 {
        let t0 = Instant::now();
        for _ in 0..iters {
            for i in 0..batch_n {
                batch.message_mut(i)[16..20].copy_from_slice(&ctr.to_le_bytes());
                ctr = ctr.wrapping_add(1);
            }
            tree.hash_many(black_box(&batch), &mut out);
            black_box(&out);
        }
        stamp_only_ns.push(t0.elapsed().as_nanos() as f64 / (iters * batch_n) as f64);
    }

    // 3. Stamp + hash_many + scan output digests
    let mut full_ns = Vec::with_capacity(11);
    for _ in 0..11 {
        let t0 = Instant::now();
        let mut hits: u64 = 0;
        for _ in 0..iters {
            for i in 0..batch_n {
                batch.message_mut(i)[16..20].copy_from_slice(&ctr.to_le_bytes());
                ctr = ctr.wrapping_add(1);
            }
            tree.hash_many(black_box(&batch), &mut out);
            for d in &out {
                hits += u64::from(d[0] == 0 && d[1] == 0);
            }
        }
        black_box(hits);
        full_ns.push(t0.elapsed().as_nanos() as f64 / (iters * batch_n) as f64);
    }

    let p = median(pure_ns);
    let so = median(stamp_only_ns);
    let f = median(full_ns);
    println!(
        "GRIND {label:12} len={msg_len:2} batch={batch_n:4}: pure={p:5.2}  +stamp={so:5.2} ({:+5.2}ns/{:+4.1}%)  +scan={f:5.2} ({:+5.2}ns/{:+4.1}%)",
        so - p,
        100.0 * (so - p) / p,
        f - so,
        100.0 * (f - p) / p,
    );
}

fn bench_interleaved(default_tree: &TreeHasher, core_tree: &TreeHasher, msg_len: usize, batch_n: usize) {
    let mut batch = Batch::new(msg_len, batch_n);
    for i in 0..batch_n {
        let m = batch.message_mut(i);
        for (j, b) in m.iter_mut().enumerate() {
            *b = ((i * 13 + j * 5 + 1) & 0xff) as u8;
        }
    }
    let mut out = vec![[0u8; 32]; batch_n];
    let mut scalar_buf = [0x5Au8; 64];
    let rounds = 300usize;

    let mut measure = |tree: &TreeHasher| -> (f64, f64) {
        for _ in 0..15 {
            tree.hash_many(&batch, &mut out);
            black_box(&out);
        }
        let mut inter_us = Vec::with_capacity(11);
        for _ in 0..11 {
            let t0 = Instant::now();
            for _ in 0..rounds {
                tree.hash_many(black_box(&batch), &mut out);
                let mut acc = out[0];
                for step in 0..16u8 {
                    scalar_buf[..32].copy_from_slice(&acc);
                    scalar_buf[32] = step;
                    acc = *tree.hash(black_box(&scalar_buf[..48])).as_bytes();
                }
                black_box(acc);
            }
            inter_us.push(t0.elapsed().as_nanos() as f64 / rounds as f64 / 1000.0);
        }
        let mut b2b_ns = Vec::with_capacity(11);
        for _ in 0..11 {
            let t0 = Instant::now();
            for _ in 0..(rounds * 2) {
                tree.hash_many(black_box(&batch), &mut out);
                black_box(&out);
            }
            b2b_ns.push(t0.elapsed().as_nanos() as f64 / (rounds * 2 * batch_n) as f64);
        }
        (median(inter_us), median(b2b_ns))
    };

    let (def_inter, def_b2b) = measure(default_tree);
    let (core_inter, core_b2b) = measure(core_tree);
    println!(
        "SME2  len={msg_len:2} n={batch_n:3}: inter default={def_inter:6.3}us vs on_core={core_inter:6.3}us ({:.2}x) | b2b def={def_b2b:5.2} vs core={core_b2b:5.2} ns/m",
        def_inter / core_inter
    );
}

fn main() {
    initialize();
    let key = [0x42u8; 32];
    let default_tree = TreeHasher::new(Mode::Keyed(&key));
    let core_tree = default_tree.clone().on_this_core();

    println!("=== #511 Counter-Grinding Probe (pure vs +stamp vs +stamp+scan) ===");
    for &(msg_len, batch_n) in &[(52, 16), (52, 64), (52, 128), (52, 256), (52, 2048), (36, 64), (36, 2048)] {
        bench_grinding(&default_tree, "default", msg_len, batch_n, 16_384);
        bench_grinding(&core_tree, "on_this_core", msg_len, batch_n, 16_384);
    }

    println!("\n=== #502 Small-Batch Interleaved-with-Scalar Probe (default vs on_this_core) ===");
    for &msg_len in &[32usize, 48] {
        for &n in &[11usize, 12, 16, 20, 24, 32, 36, 48, 64, 96, 128] {
            bench_interleaved(&default_tree, &core_tree, msg_len, n);
        }
    }
}
