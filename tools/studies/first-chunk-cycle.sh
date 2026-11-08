#!/bin/bash
set -euo pipefail
R="$1";O="$R/out";mkdir -p "$O";export HOME=/root;. /root/.cargo/env
{ date -u;uname -a;grep -m1 'model name' /proc/cpuinfo;rustc --version;for repo in old new bench;do echo "$repo $(git -C "$R/$repo" rev-parse HEAD)";done; } > "$O/identity.txt"
pids=()
pairs="1 2 3 4"
[ "${PINNED_RUN_PREFLIGHT:-0}" != 1 ] || pairs="1"
for pair in $pairs;do
 for side in old new;do
 (
 P="$R/probe-$side-$pair";mkdir -p "$P/src"
 printf '[package]
name="incrementalroots"
version="0.1.0"
edition="2021"
[dependencies]
blake3-servil={path="%s/%s"}
blake3="=1.8.7"
clocks={path="%s/bench/clocks"}
' "$R" "$side" "$R" > "$P/Cargo.toml"
 cat > "$P/src/main.rs" <<'PROBE_EOF'
use blake3_servil::{convenience::Hasher, throughput, Mode};

fn mode<'a>(i: usize, mixed: bool, keys: &'a [[u8; 32]]) -> Mode<'a> {
    if !mixed {
        return Mode::Plain;
    }
    match i % 3 {
        0 => Mode::Plain,
        1 => Mode::Keyed(&keys[i]),
        _ => Mode::DeriveKey("BLAKE3-Servil incremental bulk benchmark"),
    }
}
fn expected(i: usize, mixed: bool, keys: &[[u8; 32]], data: &[u8]) -> [u8; 32] {
    match mode(i, mixed, keys) {
        Mode::Plain => *blake3::hash(data).as_bytes(),
        Mode::Keyed(key) => *blake3::keyed_hash(key, data).as_bytes(),
        _ => blake3::derive_key("BLAKE3-Servil incremental bulk benchmark", data),
    }
}
fn feed<'a>(hashers: &mut [Hasher], data: &'a [Vec<u8>], pieces: &mut Vec<(usize, &'a [u8])>, width: usize) {
    for from in (0..data.iter().map(Vec::len).max().unwrap_or(0)).step_by(width) {
        pieces.clear();
        for (i, bytes) in data.iter().enumerate() {
            if from < bytes.len() {
                pieces.push((i, &bytes[from..(from + width).min(bytes.len())]));
            }
        }
        throughput::update_each(hashers, std::hint::black_box(pieces.as_slice()));
    }
}
fn run(shape: &str, lens: &[usize], mixed: bool, reverse: bool, width: usize) {
    let n = lens.len();
    let keys: Vec<[u8; 32]> = (0..n)
        .map(|i| core::array::from_fn(|j| (i * 37 + j * 11) as u8))
        .collect();
    let data: Vec<Vec<u8>> = lens
        .iter()
        .enumerate()
        .map(|(i, &l)| (0..l).map(|j| (i * 31 + j * 17) as u8).collect())
        .collect();
    let mut hashers: Vec<Hasher> = (0..n).map(|i| Hasher::new(mode(i, mixed, &keys), None)).collect();
    let which: Vec<usize> = (0..n).collect();
    let wanted: Vec<[u8; 32]> = data
        .iter()
        .enumerate()
        .map(|(i, d)| expected(i, mixed, &keys, d))
        .collect();
    let mut out = vec![[0u8; 32]; n];
    let mut pieces = Vec::with_capacity(n);
    let prefix = if mixed { "mixed_mode" } else { "plain" };
    let shape = format!("{prefix}_{shape}_piece{width}");
    let stride = if n > 1 {
        &hashers[1] as *const Hasher as usize - &hashers[0] as *const Hasher as usize
    } else {
        std::mem::size_of::<Hasher>()
    };
    eprintln!("layout shape={shape} count={n} hasher_size={} hasher_align={} native_stride={stride} hasher_base={:#x} output_base={:#x} digest_stride={}",std::mem::size_of::<Hasher>(),std::mem::align_of::<Hasher>(),hashers.as_ptr() as usize,out.as_ptr() as usize,std::mem::size_of::<[u8;32]>());
    eprintln!("ownership: one caller owns every state, descriptor and output. Construction/key preparation are outside; grouped updates including descriptor refill/library allocations, digest production and resets are inside each cycle.");
    let mut algorithms = ["cycle"];
    if reverse {
        algorithms.reverse();
    }
    for algorithm in algorithms {
        let mut work = || {
            if algorithm == "cycle" {
                feed(&mut hashers, &data, &mut pieces, width);
            }
            throughput::finalize_each(
                std::hint::black_box(&mut hashers),
                std::hint::black_box(&which),
                &mut out,
            );
            std::hint::black_box(&out);
        };
        if std::env::var_os("PINNED_RUN_PREFLIGHT").is_some_and(|v| v == "1") {
            work();
        } else {
            let trial = clocks::measure(1, 10_000_000, &mut work);
            let calls = trial[0].calls * 10;
            let e = clocks::energy::package_nj();
            let cpu = clocks::process_cpu_ns();
            let c = clocks::Counts::read();
            let t = clocks::now();
            for _ in 0..calls {
                work();
            }
            let wall = clocks::since_ns(t);
            let c = c.zip(clocks::Counts::read()).map(|(a, b)| b.since(a));
            let cpu = clocks::process_cpu_ns() - cpu;
            let e = e.zip(clocks::energy::package_nj()).map(|(a, b)| b - a);
            clocks::load::tick();
            println!(
                "{shape}\t{n}\t{}\t{algorithm}\t{calls}\t{wall}\t{cpu}\t{}\t{}\t{}",
                lens.iter().sum::<usize>(),
                c.map_or(0, |c| c.p.cycles + c.e.cycles),
                c.map_or(0, |c| c.p.instructions + c.e.instructions),
                e.map_or_else(|| "NA".into(), |e| e.to_string())
            );
        }
        assert_eq!(out, wanted, "{algorithm} {shape} {n}");
        assert!(hashers
            .iter()
            .all(|h| h.count() == 0), "finalize_each resets each state");
    }
}
fn main() {
    blake3_servil::initialize();
    let reverse = std::env::args().nth(1).as_deref() == Some("reverse");
    println!(
        "shape\titems\tbytes\talgorithm\tcalls\twall_ns\tcpu_ns\tcycles\tinstructions\tpackage_nj"
    );
    for n in [2, 16, 17, 64, 1024] {
        for width in [17, 256] {
            for l in [0, 1, 31, 63, 64, 65, 128, 255, 512, 1023, 1024, 2048] {
                run(&l.to_string(), &vec![l; n], false, reverse, width);
            }
            let mut x = 0x9e3779b9u64;
            let lens: Vec<usize> = (0..n)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    1 + (x % 1024) as usize
                })
                .collect();
            run("varied_lengths", &lens, false, reverse, width);
            for l in [1, 64, 1024] {
                run(&l.to_string(), &vec![l; n], true, reverse, width);
            }
            run("varied_lengths", &lens, true, reverse, width);
        }
    }
}

PROBE_EOF
 (cd "$P" && CARGO_TARGET_DIR="$R/target-$side-$pair" RUSTFLAGS="-C metadata=incrementalroots_$pair" cargo build --release && cargo tree -p blake3-servil --depth 0) > "$O/build-$side-$pair.log" 2>&1
 cp "$R/target-$side-$pair/release/incrementalroots" "$R/exe-$side-$pair";cp "$P/Cargo.lock" "$O/lock-$side-$pair.txt"
 ) &
 pids+=("$!")
 done
done
failed=0;for pid in "${pids[@]}";do if ! wait "$pid";then failed=1;fi;done
[ "$failed" = 0 ]
mkdir -p "$R/judge/src"
printf '[package]
name="incrementalrootjudge"
version="0.1.0"
edition="2021"
[dependencies]
clocks={path="%s/bench/clocks"}
' "$R" > "$R/judge/Cargo.toml"
cat > "$R/judge/src/main.rs" <<'JUDGE_EOF'
use clocks::summary::{discoveries, mean};
use std::{collections::BTreeMap, fs, path::Path};
#[derive(Clone)]
struct Row {
    units: u64,
    metrics: BTreeMap<String, u64>,
}
type Key = (String, u64, String);
fn read(path: &Path) -> BTreeMap<Key, Row> {
    let text = fs::read_to_string(path).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next(),Some("shape\titems\tbytes\talgorithm\tcalls\twall_ns\tcpu_ns\tcycles\tinstructions\tpackage_nj"));
    let mut rows = BTreeMap::new();
    for line in lines {
        let f: Vec<_> = line.split('\t').collect();
        assert_eq!(f.len(), 10);
        let count: u64 = f[1].parse().unwrap();
        let calls: u64 = f[4].parse().unwrap();
        let mut metrics = BTreeMap::new();
        for (i, name) in [
            (5, "wall_ns"),
            (6, "cpu_ns"),
            (7, "cycles"),
            (8, "instructions"),
            (9, "package_nj"),
        ] {
            if f[i] != "NA" {
                metrics.insert(name.to_string(), f[i].parse().unwrap());
            }
        }
        assert!(rows
            .insert(
                (f[0].into(), count, f[3].into()),
                Row {
                    units: count * calls,
                    metrics
                }
            )
            .is_none());
    }
    rows
}
fn main() {
    let dir = std::env::args().nth(1).expect("directory");
    let dir = Path::new(&dir);
    let main_algorithm = std::env::args().nth(2).unwrap_or_else(|| "b3".into());
    // Compare the same algorithm across the two pinned library versions.
    let old_label = format!("old-{main_algorithm}");
    let comparators = [old_label.as_str()];
    let old: Vec<_> = (1..=4)
        .map(|p| read(&dir.join(format!("old-{p}.tsv"))))
        .collect();
    let new: Vec<_> = (1..=4)
        .map(|p| read(&dir.join(format!("new-{p}.tsv"))))
        .collect();
    let mut cells = Vec::new();
    let mut names = Vec::new();
    let mut costs = Vec::new();
    for (shape, count, algorithm) in old[0].keys() {
        if algorithm != &main_algorithm {
            continue;
        }
        for metric in ["wall_ns", "cpu_ns", "cycles", "instructions", "package_nj"] {
            for &versus in &comparators {
                let mut pairs = Vec::new();
                let mut old_tot = (0u128, 0u128);
                let mut new_tot = (0u128, 0u128);
                for p in 0..4 {
                    let a = if versus == old_label {
                        &old[p][&(shape.clone(), *count, main_algorithm.clone())]
                    } else {
                        &new[p][&(shape.clone(), *count, versus.to_string())]
                    };
                    let b = &new[p][&(shape.clone(), *count, main_algorithm.clone())];
                    let (Some(&av), Some(&bv)) = (a.metrics.get(metric), b.metrics.get(metric))
                    else {
                        break;
                    };
                    if av == 0 || bv == 0 {
                        break;
                    }
                    pairs.push((mean([(av, a.units)]), mean([(bv, b.units)])));
                    old_tot.0 += av as u128;
                    old_tot.1 += a.units as u128;
                    new_tot.0 += bv as u128;
                    new_tot.1 += b.units as u128;
                }
                if pairs.len() == 4 {
                    names.push((shape.clone(), *count, metric, versus));
                    cells.push(pairs);
                    costs.push((old_tot, new_tot));
                }
            }
        }
    }
    let judged = discoveries(&cells, 50, 30);
    println!("shape\titems\tmetric\tversus\told_per_item\tnew_per_item\tratio_permille\tverdict");
    for (((shape, count, metric, versus), cost), j) in names.into_iter().zip(costs).zip(judged) {
        println!(
            "{shape}\t{count}\t{metric}\t{versus}\t{:.4}\t{:.4}\t{}\t{:?}",
            cost.0 .0 as f64 / cost.0 .1 as f64,
            cost.1 .0 as f64 / cost.1 .1 as f64,
            j.ratio_permille,
            j.verdict
        );
    }
}

JUDGE_EOF
(cd "$R/judge" && CARGO_TARGET_DIR="$R/target-judge" cargo build --release) > "$O/build-judge.log" 2>&1
if [ "${PINNED_RUN_PREFLIGHT:-0}" = 1 ]; then
 for side in old new;do "$R/exe-$side-1" > "$O/preflight-$side.txt" 2> "$O/preflight-$side.err";done
 echo "pinned-run preflight complete" > "$O/preflight.txt"
 echo "incremental root complete-cycle preflight done" > "$O/status.txt"
 exit 0
fi
for pair in 1 2 3 4;do
 sides='old new';order=forward;[ "$((pair%2))" != 0 ] || { sides='new old';order=reverse; }
 for side in $sides;do taskset -c 2 "$R/exe-$side-$pair" "$order" > "$O/$side-$pair.tsv" 2> "$O/$side-$pair.err";done
done
for algorithm in cycle;do "$R/target-judge/release/incrementalrootjudge" "$O" "$algorithm" > "$O/judgments-$algorithm.tsv";done
echo 'incremental root complete-cycle matrix done' > "$O/status.txt"
