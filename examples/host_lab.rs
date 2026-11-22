//! PROBE (probe/latency-ready, never to land). The question (Zooko,
//! November 22, 2026): what would a caller who cares for latency, and will
//! pay for it, gain from helpers kept awake between calls and a lower
//! split, beside the two APIs as they ship?
//!
//!     host_lab latency [VISITS]
//!
//! One call after each kind of gap a program leaves (none; other work; 20
//! microseconds asleep; 5 milliseconds asleep), on the calling thread
//! (`convenience::hash`) and over the pool (`throughput::hash_multithreaded`),
//! under each setting of the probe's knobs (`__probe_latency`): the library
//! as it ships, the split lowered with the workers asleep as they are, and
//! the split lowered with 1, 3, 7 and all workers kept polling. Each visit
//! is the bench's after-a-gap sample (`clocks::measure_after_gaps_prepared`):
//! the same call first, untimed; the gap; the input written; the call,
//! timed alone. Settings and cells are shuffled within each round, so
//! every cell sees the run's whole length.
//!
//! Timed with the servil projects' clocks (clocks/ here is a copy of
//! bench-hashes 7f2a0b21's). Writes every sample to
//! `host_lab.latency.tsv` in the working directory and a table to
//! standard output: for each cell the mean and the 50th, 90th and 99th
//! percentiles and the longest call, in microseconds; the calling thread's
//! clock rate over the write and the call, and its share of time on
//! efficiency cores, where the platform counts them.

use blake3_servil::throughput::{self, Block};
use blake3_servil::{Mode, convenience};
use std::hint::black_box;
use std::io::Write;

/// The gaps: what the caller does between the first call and the timed one.
#[derive(Clone, Copy, PartialEq)]
enum Pause {
    Nonstop,
    OtherWork,
    Idle(u64),
}

impl Pause {
    fn name(self) -> &'static str {
        match self {
            Pause::Nonstop => "nonstop",
            Pause::OtherWork => "other work",
            Pause::Idle(20_000) => "idle 20 us",
            Pause::Idle(5_000_000) => "idle 5 ms",
            Pause::Idle(_) => "idle",
        }
    }
}

/// The bench's gap of other work (bench-hashes' GAP_NS and its 128 MiB walk).
const OTHER_WORK_NS: u64 = 1_000_000;
const OTHER_WORK_BYTES: usize = 128 << 20;

#[derive(Clone)]
struct Setting {
    name: String,
    awake: usize,
    split: usize,
    per: usize,
}

#[derive(Clone, Copy)]
struct Cell {
    pause: Pause,
    multithreaded: bool,
    len: usize,
}

#[derive(Default, Clone)]
struct Samples {
    hash_ns: Vec<u32>,
    write_ns: Vec<u32>,
    counts: Option<clocks::Counts>,
}

fn size_name(len: usize) -> String {
    if len >= 1 << 20 { format!("{} MiB", len >> 20) } else { format!("{} KiB", len >> 10) }
}

/// xorshift64*: the shuffles' order, the same on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, (self.next() % (i as u64 + 1)) as usize);
        }
    }
}

/// Set the knobs and wait for the pool to settle: the workers kept awake
/// polling, every other one parked.
fn settle(setting: &Setting, workers: usize, unsettled: &mut usize) {
    blake3_servil::__probe_latency(setting.awake, setting.split, setting.per);
    let want = workers - setting.awake.min(workers);
    let started = clocks::now();
    while blake3_servil::__probe_parked().0 != want {
        if clocks::since_ns(started) > 200_000_000 {
            *unsettled += 1;
            break;
        }
        std::thread::sleep(std::time::Duration::from_micros(50));
    }
}

fn percentile(sorted: &[u32], percent: usize) -> u32 {
    sorted[(sorted.len() * percent / 100).min(sorted.len() - 1)]
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert!(args.get(1).map(String::as_str) == Some("latency"), "usage: host_lab latency [VISITS]");
    let visits: usize = args.get(2).map_or(200, |v| v.parse().expect("VISITS is a number"));
    assert!(visits >= 1);

    blake3_servil::initialize_multithreaded(None);
    // The pauses land when asked, as the bench's do.
    clocks::tight_timers();
    let (_, workers) = blake3_servil::__probe_parked();
    assert!(workers >= 1, "the probe needs a pool: more than one CPU");

    const KIB: usize = 1024;
    let ships = Setting { name: "ships".to_owned(), awake: 0, split: 512 * KIB, per: 128 * KIB };
    let mut settings = vec![ships.clone(), Setting { name: "asleep, split 32 KiB".to_owned(), awake: 0, split: 32 * KIB, per: 32 * KIB }];
    let mut awakes: Vec<usize> = [1, 3, 7, workers].into_iter().filter(|&k| k <= workers).collect();
    awakes.dedup();
    for awake in awakes {
        let name = if awake == workers { format!("all {awake} awake, split 32 KiB") } else { format!("{awake} awake, split 32 KiB") };
        settings.push(Setting { name, awake, split: 32 * KIB, per: 32 * KIB });
    }

    let pauses = [Pause::Nonstop, Pause::OtherWork, Pause::Idle(20_000), Pause::Idle(5_000_000)];
    let mut cells = Vec::new();
    for pause in pauses {
        for len in [KIB, 16 * KIB, 64 * KIB, 256 * KIB, 1 << 20, 4 << 20] {
            cells.push(Cell { pause, multithreaded: false, len });
        }
        for len in [64 * KIB, 256 * KIB, 1 << 20, 4 << 20] {
            cells.push(Cell { pause, multithreaded: true, len });
        }
    }

    // The message's source, the place it is written to before each call
    // (whole blocks, as the throughput calls take them), and the other
    // work's memory, larger than the caches.
    let max = 4 << 20;
    let mut source = vec![0u8; max];
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for chunk in source.chunks_mut(8) {
        chunk.copy_from_slice(&rng.next().to_le_bytes()[..chunk.len()]);
    }
    let mut message = vec![Block::ZERO; max / 64];
    let work = vec![1u8; OTHER_WORK_BYTES];

    // Every setting hashes right, on both calls, before anything is timed.
    let mut unsettled = 0;
    for setting in &settings {
        settle(setting, workers, &mut unsettled);
        for len in [16 * KIB, 64 * KIB, 256 * KIB, (1 << 20) + 1, 4 << 20] {
            Block::bytes_mut(&mut message)[..len].copy_from_slice(&source[..len]);
            let want = convenience::hash(&source[..len]);
            assert_eq!(throughput::hash_multithreaded(Mode::Plain, &message, len), want, "{} at {len}", setting.name);
        }
    }

    let mut samples = vec![vec![Samples::default(); cells.len()]; settings.len()];
    let mut setting_order: Vec<usize> = (0..settings.len()).collect();
    let mut cell_order: Vec<usize> = (0..cells.len()).collect();
    let started = clocks::now();
    for _ in 0..visits {
        rng.shuffle(&mut setting_order);
        for &s in &setting_order {
            settle(&settings[s], workers, &mut unsettled);
            rng.shuffle(&mut cell_order);
            for &c in &cell_order {
                let cell = cells[c];
                let len = cell.len;
                let call = |message: &[Block]| {
                    if cell.multithreaded {
                        black_box(throughput::hash_multithreaded(Mode::Plain, black_box(message), len));
                    } else {
                        black_box(convenience::hash(black_box(&Block::bytes(message)[..len])));
                    }
                };
                let write = |message: &mut [Block]| Block::bytes_mut(message)[..len].copy_from_slice(black_box(&source[..len]));
                clocks::load::tick();
                // The first call, untimed: the timed one then follows the same call.
                let before = if cell.pause == Pause::Nonstop { 2 } else { 1 };
                for _ in 0..before {
                    write(&mut message);
                    call(&message);
                }
                match cell.pause {
                    Pause::Nonstop => {}
                    Pause::OtherWork => clocks::Gap::Busy(&work).spend(OTHER_WORK_NS),
                    Pause::Idle(ns) => clocks::Gap::Idle.spend(ns),
                }
                // As the bench's helper does: the timing path warmed, then
                // the write and the call each timed alone, the counts read
                // outside both.
                black_box(clocks::Counts::read());
                black_box(clocks::since_ns(clocks::now()));
                let counts_before = clocks::Counts::read();
                let t = clocks::now();
                write(&mut message);
                let write_ns = clocks::since_ns(t);
                let t = clocks::now();
                call(&message);
                let hash_ns = clocks::since_ns(t);
                let counts = counts_before.zip(clocks::Counts::read()).map(|(before, after)| after.since(before));
                let kept = &mut samples[s][c];
                kept.hash_ns.push(hash_ns.min(u32::MAX as u64) as u32);
                kept.write_ns.push(write_ns.min(u32::MAX as u64) as u32);
                kept.counts = match (kept.counts, counts) {
                    (Some(sum), Some(one)) => Some(sum.plus(one)),
                    (None, one) if kept.hash_ns.len() == 1 => one,
                    _ => None,
                };
            }
        }
    }
    let run_ns = clocks::since_ns(started);
    settle(&ships, workers, &mut unsettled);

    let mut file = std::fs::File::create("host_lab.latency.tsv").expect("write host_lab.latency.tsv");
    let mut head = String::new();
    head += &format!("# host_lab latency: {visits} visits a cell, {:.1} s\n", run_ns as f64 / 1e9);
    head += &format!("# blake3-servil {}\n", blake3_servil::BUILD_VERSION);
    head += &format!("# target: {} {}\n", std::env::consts::ARCH, std::env::consts::OS);
    head += &format!("# cpus: {}; pool workers: {workers}\n", std::thread::available_parallelism().map_or(0, |n| n.get()));
    head += &format!("# wall clock: {}\n", clocks::WALL_CLOCK);
    head += &format!("# settles that did not complete in 200 ms: {unsettled}\n");
    let windows = clocks::load::windows();
    head += &format!(
        "# other programs' load: {} windows, {} busy; {}\n",
        windows.len(),
        windows.iter().filter(|w| w.busy()).count(),
        clocks::load::describe(&windows).replace('\n', " | ")
    );
    file.write_all(head.as_bytes()).unwrap();
    print!("{head}");
    writeln!(file, "setting\tawake\tsplit\tper\tgap\tcall\tsize\thash ns\twrite ns\tp cycles\tp ns\te cycles\te ns").unwrap();
    println!(
        "{:<11} {:<4} {:>8}  {:<28} {:>9} {:>9} {:>9} {:>9} {:>10} {:>9} {:>6} {:>4}",
        "gap", "call", "size", "setting", "mean", "p50", "p90", "p99", "max", "write p50", "MHz", "E%"
    );
    for (c, cell) in cells.iter().enumerate() {
        for (s, setting) in settings.iter().enumerate() {
            let kept = &samples[s][c];
            let call = if cell.multithreaded { "mt" } else { "st" };
            let list = |v: &[u32]| v.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(",");
            let (p_cycles, p_ns, e_cycles, e_ns) = kept.counts.map_or((0, 0, 0, 0), |k| (k.p.cycles, k.p.time_ns, k.e.cycles, k.e.time_ns));
            writeln!(
                file,
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                setting.name,
                setting.awake,
                setting.split,
                setting.per,
                cell.pause.name(),
                call,
                cell.len,
                list(&kept.hash_ns),
                list(&kept.write_ns),
                p_cycles,
                p_ns,
                e_cycles,
                e_ns
            )
            .unwrap();
            let mut sorted = kept.hash_ns.clone();
            sorted.sort_unstable();
            let mut writes = kept.write_ns.clone();
            writes.sort_unstable();
            let mean = sorted.iter().map(|&n| n as u64).sum::<u64>() as f64 / sorted.len() as f64 / 1000.0;
            let us = |ns: u32| ns as f64 / 1000.0;
            let (mhz, e_share) = match kept.counts {
                Some(k) if k.p.time_ns + k.e.time_ns > 0 => (k.mhz().to_string(), k.e_percent().to_string()),
                _ => ("-".to_owned(), "-".to_owned()),
            };
            println!(
                "{:<11} {:<4} {:>8}  {:<28} {:>9.2} {:>9.2} {:>9.2} {:>9.2} {:>10.2} {:>9.2} {:>6} {:>4}",
                cell.pause.name(),
                call,
                size_name(cell.len),
                setting.name,
                mean,
                us(percentile(&sorted, 50)),
                us(percentile(&sorted, 90)),
                us(percentile(&sorted, 99)),
                us(*sorted.last().unwrap()),
                us(percentile(&writes, 50)),
                mhz,
                e_share
            );
        }
    }
}
