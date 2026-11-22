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
//!
//!     host_lab rest [VISITS]
//!
//! The calling thread's own speed after a rest (the Macs' question: one
//! helper kept awake took a call after 5 ms asleep to a quarter of its
//! time; does a QoS class on the calling thread do as much with nothing
//! spinning?). `convenience::hash` after 200 microseconds, 1, 5 and 20
//! milliseconds asleep, under each way of staying fast: as launched; the
//! calling thread in QoS class default, user-initiated, user-interactive;
//! the real-time policy audio threads use; one pool worker kept awake; a
//! spinning thread of the program's own in the background QoS class
//! (macOS runs that on efficiency cores). Writes `host_lab.rest.tsv` and
//! the same table.
//!
//!     host_lab cold [VISITS]
//!
//! What a now-and-then call would gain from narrower kernels (x86-64: the
//! Xeon's call after other work takes four times its back-to-back time at
//! 16 KiB and 64 KiB, which reads as the 512-bit unit starting). One
//! message on the calling thread, back to back, after other work and
//! after 5 milliseconds asleep, through `convenience::hash` as shipped and
//! through the one-thread tree on each kernel family the CPU has: the
//! widest, AVX2, SSE4.1. Writes `host_lab.cold.tsv` and the same table.
//!
//!     host_lab warn [VISITS]
//!
//! Helpers woken by a warning in place of helpers kept awake: how far
//! ahead of the call must the warning come? Everything rests 5
//! milliseconds; then three workers are woken (`__probe_warn`), the
//! caller waits out the lead (0, 50 and 200 microseconds, 1 and 5
//! milliseconds), and the call is timed: on one thread at 1 KiB and
//! 64 KiB, over the pool (split from 32 KiB) at 256 KiB and 1 MiB. Beside
//! them the call with no warning, as the library ships, and with the
//! three kept awake through the rest. Writes `host_lab.warn.tsv` and the
//! same table (its "setting" column names the way). `host_lab warn VISITS
//! long [HELPERS]` takes leads of 5 milliseconds to half a second in
//! their place, and wakes HELPERS workers where the plain form wakes three.
//! `host_lab warn VISITS rested [HELPERS]` rests 300 milliseconds before
//! each warning in place of 5, with leads of 1 to 200 milliseconds, and
//! times the calling thread alone.
//!
//!     host_lab early [VISITS]
//!
//! A hasher made early, with nothing kept running (Zooko: a caller may
//! learn that bytes will come, a connection opening, and make its hasher
//! then; what could making it do ahead?). After the program's gap (5
//! milliseconds asleep, or other work) comes the hello, then a lead in
//! which nothing runs (50 and 200 microseconds, 1 and 5 milliseconds),
//! then the bytes: the write and the call, timed. At the hello one of:
//! nothing, as the library ships; the kernels the call will run, run once
//! on the calling thread over other bytes (the most that fetching code and
//! starting the wide unit ahead could give); that and three workers
//! woken, who find nothing and park again. One thread at 1 KiB, 16 KiB
//! and 64 KiB, the pool at 1 MiB, the split as shipped. Writes
//! `host_lab.early.tsv` and the same table.
//!
//!     host_lab price [SECONDS]
//!
//! What helpers kept awake cost while no call comes: for 0, 1, 3, 7 and
//! all workers awake (there and back, a rest between), the process's CPU
//! time and its energy over SECONDS of waiting: on macOS the system's own
//! estimate for the process (`ri_energy_nj`, a model: within about a
//! fifth of the chip's meter in earlier probes), on Linux the package's
//! counter where the process may read it. Each line is led by its start
//! in Unix seconds.

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
            Pause::Idle(200_000) => "idle 200 us",
            Pause::Idle(1_000_000) => "idle 1 ms",
            Pause::Idle(5_000_000) => "idle 5 ms",
            Pause::Idle(20_000_000) => "idle 20 ms",
            Pause::Idle(300_000_000) => "idle 300 ms",
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
    let count: Option<usize> = args.get(2).map(|v| v.parse().expect("the second argument is a number"));
    match args.get(1).map(String::as_str) {
        Some("latency") => latency(count.unwrap_or(200)),
        Some("rest") => rest(count.unwrap_or(200)),
        Some("cold") => cold(count.unwrap_or(200)),
        Some("warn") => warn(count.unwrap_or(200), args.get(3).map_or("", String::as_str), args.get(4).map_or(3, |h| h.parse().expect("HELPERS is a number"))),
        Some("early") => early(count.unwrap_or(200)),
        Some("price") => price(count.unwrap_or(10)),
        _ => panic!(
            "usage: host_lab latency [VISITS] | rest [VISITS] | cold [VISITS] | warn [VISITS] [long|rested [HELPERS]] | early [VISITS] | price [SECONDS]"
        ),
    }
}

/// One visit of `cell`: the same call first, untimed (twice back to back
/// for a nonstop cell); the gap; then, as the bench's helper does, the
/// timing path warmed and the write and the call each timed alone, the
/// thread's counts read outside both. Returns the write's and the call's
/// nanoseconds and the counts over both.
fn visit(cell: Cell, source: &[u8], message: &mut [Block], work: &[u8]) -> (u64, u64, Option<clocks::Counts>) {
    let len = cell.len;
    visit_with(cell.pause, len, source, message, work, |message: &[Block]| {
        if cell.multithreaded {
            black_box(throughput::hash_multithreaded(Mode::Plain, black_box(message), len));
        } else {
            black_box(convenience::hash(black_box(&Block::bytes(message)[..len])));
        }
    })
}

/// [`visit`] with the call given: `call` hashes the message's first `len` bytes.
fn visit_with(pause: Pause, len: usize, source: &[u8], message: &mut [Block], work: &[u8], call: impl Fn(&[Block])) -> (u64, u64, Option<clocks::Counts>) {
    let write = |message: &mut [Block]| Block::bytes_mut(message)[..len].copy_from_slice(black_box(&source[..len]));
    let before = if pause == Pause::Nonstop { 2 } else { 1 };
    for _ in 0..before {
        write(message);
        call(message);
    }
    match pause {
        Pause::Nonstop => {}
        Pause::OtherWork => clocks::Gap::Busy(work).spend(OTHER_WORK_NS),
        Pause::Idle(ns) => clocks::Gap::Idle.spend(ns),
    }
    black_box(clocks::Counts::read());
    black_box(clocks::since_ns(clocks::now()));
    let counts_before = clocks::Counts::read();
    let t = clocks::now();
    write(message);
    let write_ns = clocks::since_ns(t);
    let t = clocks::now();
    call(message);
    let hash_ns = clocks::since_ns(t);
    let counts = counts_before.zip(clocks::Counts::read()).map(|(before, after)| after.since(before));
    (write_ns, hash_ns, counts)
}

fn keep(kept: &mut Samples, (write_ns, hash_ns, counts): (u64, u64, Option<clocks::Counts>)) {
    kept.hash_ns.push(hash_ns.min(u32::MAX as u64) as u32);
    kept.write_ns.push(write_ns.min(u32::MAX as u64) as u32);
    kept.counts = match (kept.counts, counts) {
        (Some(sum), Some(one)) => Some(sum.plus(one)),
        (None, one) if kept.hash_ns.len() == 1 => one,
        _ => None,
    };
}

/// The run's facts, for the head of its file and its table.
fn facts(what: &str, run_ns: u64, workers: usize) -> String {
    let mut head = String::new();
    head += &format!("# host_lab {what}, {:.1} s\n", run_ns as f64 / 1e9);
    head += &format!("# blake3-servil {}\n", blake3_servil::BUILD_VERSION);
    head += &format!("# target: {} {}\n", std::env::consts::ARCH, std::env::consts::OS);
    head += &format!("# cpus: {}; pool workers: {workers}\n", std::thread::available_parallelism().map_or(0, |n| n.get()));
    head += &format!("# wall clock: {}\n", clocks::WALL_CLOCK);
    let windows = clocks::load::windows();
    head += &format!(
        "# other programs' load: {} windows, {} busy; {}\n",
        windows.len(),
        windows.iter().filter(|w| w.busy()).count(),
        clocks::load::describe(&windows).replace('\n', " | ")
    );
    head
}

/// Every sample to `file_name`, and the table to standard output.
fn report(file_name: &str, head: &str, settings: &[Setting], cells: &[Cell], samples: &[Vec<Samples>]) {
    let mut file = std::fs::File::create(file_name).expect("write the samples file");
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

const KIB: usize = 1024;

/// The message's source (the same bytes on every run) and the place it
/// is written to before each call: whole blocks, as the throughput calls
/// take them.
fn buffers(rng: &mut Rng) -> (Vec<u8>, Vec<Block>) {
    let max = 4 << 20;
    let mut source = vec![0u8; max];
    for chunk in source.chunks_mut(8) {
        chunk.copy_from_slice(&rng.next().to_le_bytes()[..chunk.len()]);
    }
    (source, vec![Block::ZERO; max / 64])
}

fn latency(visits: usize) {
    assert!(visits >= 1);

    blake3_servil::initialize_multithreaded(None);
    // The pauses land when asked, as the bench's do.
    clocks::tight_timers();
    let (_, workers) = blake3_servil::__probe_parked();
    assert!(workers >= 1, "the probe needs a pool: more than one CPU");

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

    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let (source, mut message) = buffers(&mut rng);
    // The other work's memory, larger than the caches.
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
                clocks::load::tick();
                keep(&mut samples[s][c], visit(cells[c], &source, &mut message, &work));
            }
        }
    }
    let run_ns = clocks::since_ns(started);
    settle(&ships, workers, &mut unsettled);
    let mut head = facts(&format!("latency: {visits} visits a cell"), run_ns, workers);
    head += &format!("# settles that did not complete in 200 ms: {unsettled}\n");
    report("host_lab.latency.tsv", &head, &settings, &cells, &samples);
}

/// macOS's QoS classes (<sys/qos.h>) beside the two `clocks` names.
const QOS_DEFAULT: u32 = 0x15;
const QOS_USER_INITIATED: u32 = 0x19;

/// The calling thread's QoS class as the system reports it (macOS), or
/// None elsewhere.
fn qos_now() -> Option<u32> {
    #[cfg(target_vendor = "apple")]
    {
        unsafe extern "C" {
            fn pthread_self() -> usize;
            fn pthread_get_qos_class_np(thread: usize, class: *mut u32, relative_priority: *mut i32) -> i32;
        }
        let (mut class, mut priority) = (0u32, 0i32);
        // Sound: both places are writable, and the thread is this one.
        let rc = unsafe { pthread_get_qos_class_np(pthread_self(), &mut class, &mut priority) };
        return (rc == 0).then_some(class);
    }
    #[allow(unreachable_code)]
    None
}

/// Put the calling thread under macOS's real-time policy
/// (THREAD_TIME_CONSTRAINT_POLICY, what audio threads use): no period, 200
/// microseconds of computation within 400. Whether the system took it
/// (false elsewhere).
fn real_time() -> bool {
    #[cfg(target_vendor = "apple")]
    {
        #[repr(C)]
        struct Policy {
            period: u32,
            computation: u32,
            constraint: u32,
            preemptible: u32,
        }
        unsafe extern "C" {
            fn mach_thread_self() -> u32;
            fn thread_policy_set(thread: u32, flavor: u32, policy: *const Policy, count: u32) -> i32;
            fn mach_timebase_info(info: *mut [u32; 2]) -> i32;
        }
        let mut timebase = [0u32; 2];
        // Sound: the two words are writable (numerator, denominator).
        if unsafe { mach_timebase_info(&mut timebase) } != 0 || timebase[0] == 0 {
            return false;
        }
        let ticks = |ns: u64| (ns * timebase[1] as u64 / timebase[0] as u64) as u32;
        let policy = Policy { period: 0, computation: ticks(200_000), constraint: ticks(400_000), preemptible: 1 };
        // Sound: the policy is four words, as the count says; the thread is this one.
        return unsafe { thread_policy_set(mach_thread_self(), 2, &policy, 4) } == 0;
    }
    #[allow(unreachable_code)]
    false
}

/// The blocks whose thread was refused the real-time policy.
static REFUSED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// How the program tries to keep the calling thread fast through a rest.
#[derive(Clone, Copy, PartialEq)]
enum Fast {
    /// Nothing: the thread as the program was launched.
    AsLaunched,
    /// The calling thread in this QoS class.
    Qos(u32),
    /// The calling thread under the real-time policy (macOS).
    RealTime,
    /// One pool worker kept awake (the latency mode's "1 awake").
    Worker,
    /// A spinning thread of the program's own in the background QoS class.
    BackgroundSpinner,
}

fn rest(visits: usize) {
    assert!(visits >= 1);
    blake3_servil::initialize_multithreaded(None);
    clocks::tight_timers();
    let (_, workers) = blake3_servil::__probe_parked();
    let launched = qos_now();
    let ways = [
        ("as launched", Fast::AsLaunched),
        ("QoS default", Fast::Qos(QOS_DEFAULT)),
        ("QoS user-initiated", Fast::Qos(QOS_USER_INITIATED)),
        ("QoS user-interactive", Fast::Qos(clocks::USER_INTERACTIVE)),
        ("real-time policy", Fast::RealTime),
        ("one worker awake", Fast::Worker),
        ("a background spinner", Fast::BackgroundSpinner),
    ];
    // The table's rows carry each way's name where the latency mode's carry a setting's.
    let settings: Vec<Setting> = ways
        .iter()
        .map(|(name, way)| Setting { name: (*name).to_owned(), awake: usize::from(*way == Fast::Worker), split: 512 * KIB, per: 128 * KIB })
        .collect();
    let mut cells = Vec::new();
    for pause in [Pause::Idle(200_000), Pause::Idle(1_000_000), Pause::Idle(5_000_000), Pause::Idle(20_000_000)] {
        for len in [KIB, 16 * KIB, 64 * KIB, 256 * KIB] {
            cells.push(Cell { pause, multithreaded: false, len });
        }
    }
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let (source, mut message) = buffers(&mut rng);

    // The program's own spinner: it spins while `spin` is set and sleeps
    // on its token otherwise.
    static SPIN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let spinner = std::thread::spawn(|| {
        clocks::set_qos(clocks::BACKGROUND);
        loop {
            while SPIN.load(std::sync::atomic::Ordering::Relaxed) {
                std::hint::spin_loop();
            }
            std::thread::park();
        }
    });

    let mut samples = vec![vec![Samples::default(); cells.len()]; settings.len()];
    let mut way_order: Vec<usize> = (0..ways.len()).collect();
    let mut cell_order: Vec<usize> = (0..cells.len()).collect();
    let mut unsettled = 0;
    let started = clocks::now();
    for _ in 0..visits {
        rng.shuffle(&mut way_order);
        for &w in &way_order {
            let way = ways[w].1;
            settle(&settings[w], workers, &mut unsettled);
            SPIN.store(way == Fast::BackgroundSpinner, std::sync::atomic::Ordering::Relaxed);
            spinner.thread().unpark();
            rng.shuffle(&mut cell_order);
            let block = |samples: &mut Vec<Samples>, message: &mut Vec<Block>| {
                for &c in &cell_order {
                    clocks::load::tick();
                    keep(&mut samples[c], visit(cells[c], &source, message, &[]));
                }
            };
            match way {
                // A thread of its own for each block, in the class from its start: the main thread keeps the class it was launched in.
                Fast::Qos(class) => std::thread::scope(|scope| {
                    scope.spawn(|| {
                        clocks::set_qos(class);
                        block(&mut samples[w], &mut message);
                    });
                }),
                Fast::RealTime => std::thread::scope(|scope| {
                    scope.spawn(|| {
                        if !real_time() {
                            REFUSED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                        block(&mut samples[w], &mut message);
                    });
                }),
                _ => block(&mut samples[w], &mut message),
            }
            SPIN.store(false, std::sync::atomic::Ordering::Relaxed);
        }
    }
    let run_ns = clocks::since_ns(started);
    blake3_servil::__probe_latency(0, 512 * KIB, 128 * KIB);
    let mut head = facts(&format!("rest: {visits} visits a cell"), run_ns, workers);
    head += &format!("# the main thread's QoS class as launched: {}\n", launched.map_or("not reported here".to_owned(), |class| format!("{class:#x}")));
    head += &format!("# blocks whose thread the system refused the real-time policy: {} of {visits}\n", REFUSED.load(std::sync::atomic::Ordering::Relaxed));
    head += &format!("# settles that did not complete in 200 ms: {unsettled}\n");
    report("host_lab.rest.tsv", &head, &settings, &cells, &samples);
}

fn cold(visits: usize) {
    assert!(visits >= 1);
    blake3_servil::initialize_multithreaded(None);
    clocks::tight_timers();
    let (_, workers) = blake3_servil::__probe_parked();
    use blake3_servil_core::thread_support::Platform;
    let tree = blake3_servil_core::throughput::TreeHasher::new(Mode::Plain);
    // The kernel families this CPU has, the widest first; None is convenience::hash as shipped.
    let mut kernels: Vec<(String, Option<Platform>)> = vec![("convenience::hash".to_owned(), None)];
    let widest = Platform::detect();
    kernels.push((format!("widest ({})", widest.name()), Some(widest)));
    #[cfg(target_arch = "x86_64")]
    for name in ["avx2", "sse41"] {
        if let Some(platform) = Platform::probe_named(name).filter(|&p| p != widest) {
            kernels.push((platform.name().to_owned(), Some(platform)));
        }
    }
    let settings: Vec<Setting> = kernels.iter().map(|(name, _)| Setting { name: name.clone(), awake: 0, split: 512 * KIB, per: 128 * KIB }).collect();
    let mut cells = Vec::new();
    for pause in [Pause::Nonstop, Pause::OtherWork, Pause::Idle(5_000_000)] {
        for len in [KIB, 4 * KIB, 16 * KIB, 64 * KIB, 256 * KIB, 1 << 20] {
            cells.push(Cell { pause, multithreaded: false, len });
        }
    }
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let (source, mut message) = buffers(&mut rng);
    let work = vec![1u8; OTHER_WORK_BYTES];
    // Every kernel hashes right before anything is timed.
    for (name, platform) in &kernels {
        for len in [KIB, 4 * KIB, 16 * KIB + 1, 64 * KIB, (1 << 20) + 1] {
            if let Some(platform) = platform {
                assert_eq!(tree.hash_on(*platform, &source[..len]), convenience::hash(&source[..len]), "{name} at {len}");
            }
        }
    }
    let mut samples = vec![vec![Samples::default(); cells.len()]; settings.len()];
    // Kernels and cells shuffled together: every visit is one (kernel, cell).
    let mut order: Vec<(usize, usize)> = (0..kernels.len()).flat_map(|k| (0..cells.len()).map(move |c| (k, c))).collect();
    let started = clocks::now();
    for _ in 0..visits {
        rng.shuffle(&mut order);
        for &(k, c) in &order {
            let cell = cells[c];
            let len = cell.len;
            clocks::load::tick();
            let sample = match kernels[k].1 {
                None => visit(cell, &source, &mut message, &work),
                Some(platform) => visit_with(cell.pause, len, &source, &mut message, &work, |message: &[Block]| {
                    black_box(tree.hash_on(platform, black_box(&Block::bytes(message)[..len])));
                }),
            };
            keep(&mut samples[k][c], sample);
        }
    }
    let run_ns = clocks::since_ns(started);
    let head = facts(&format!("cold: {visits} visits a cell"), run_ns, workers);
    report("host_lab.cold.tsv", &head, &settings, &cells, &samples);
}

/// How a `warn` cell readies the pool for its call.
#[derive(Clone, Copy, PartialEq)]
enum Readied {
    /// Nothing: the library as it ships.
    Ships,
    /// Three workers woken this many nanoseconds before the write.
    Warned(u64),
    /// Three workers kept awake through the rest.
    Kept,
}

fn warn(visits: usize, variant: &str, helpers: usize) {
    let (long, rested) = (variant == "long", variant == "rested");
    assert!(long || rested || variant.is_empty(), "the variants are long and rested");
    assert!(visits >= 1);
    blake3_servil::initialize_multithreaded(None);
    clocks::tight_timers();
    let (_, workers) = blake3_servil::__probe_parked();
    assert!((1..=workers).contains(&helpers), "the probe wakes HELPERS of the pool's workers");
    // Everything rests this long before each warning: 5 ms, or with
    // "rested" 300 ms, so that each visit starts from a machine that has
    // been quiet (the Macs' clock remembers the last few hundred ms).
    let rest_ns: u64 = if rested { 300_000_000 } else { 5_000_000 };
    // The leads: up to 5 ms, or with "long" (the Macs, whose clock rises
    // slowly) from 5 ms to half a second.
    let ways = if rested {
        [
            ("no warning", Readied::Ships),
            ("warned 1 ms", Readied::Warned(1_000_000)),
            ("warned 5 ms", Readied::Warned(5_000_000)),
            ("warned 20 ms", Readied::Warned(20_000_000)),
            ("warned 50 ms", Readied::Warned(50_000_000)),
            ("warned 100 ms", Readied::Warned(100_000_000)),
            ("warned 200 ms", Readied::Warned(200_000_000)),
        ]
    } else if long {
        [
            ("no warning", Readied::Ships),
            ("warned 5 ms", Readied::Warned(5_000_000)),
            ("warned 20 ms", Readied::Warned(20_000_000)),
            ("warned 50 ms", Readied::Warned(50_000_000)),
            ("warned 100 ms", Readied::Warned(100_000_000)),
            ("warned 200 ms", Readied::Warned(200_000_000)),
            ("warned 500 ms", Readied::Warned(500_000_000)),
        ]
    } else {
        [
            ("no warning", Readied::Ships),
            ("warned 0 us", Readied::Warned(0)),
            ("warned 50 us", Readied::Warned(50_000)),
            ("warned 200 us", Readied::Warned(200_000)),
            ("warned 1 ms", Readied::Warned(1_000_000)),
            ("warned 5 ms", Readied::Warned(5_000_000)),
            ("kept awake", Readied::Kept),
        ]
    };
    // The rested variant times the calling thread alone: its visits are long.
    let calls: &[(bool, usize)] =
        if rested { &[(false, KIB), (false, 64 * KIB)] } else { &[(false, KIB), (false, 64 * KIB), (true, 256 * KIB), (true, 1 << 20)] };
    // The table's shape, reused: a row for each way (its "gap"), a column
    // of samples for each call (its "setting").
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let (source, mut message) = buffers(&mut rng);
    let mut samples = vec![vec![Samples::default(); calls.len()]; ways.len()];
    let mut order: Vec<(usize, usize)> = (0..ways.len()).flat_map(|w| (0..calls.len()).map(move |c| (w, c))).collect();
    let mut unsettled = 0;
    let asleep = |split: usize| blake3_servil::__probe_latency(0, split, split.min(128 * KIB));
    let started = clocks::now();
    for _ in 0..visits {
        rng.shuffle(&mut order);
        for &(w, c) in &order {
            let (multithreaded, len) = calls[c];
            let way = ways[w].1;
            let call = |message: &[Block]| {
                if multithreaded {
                    black_box(throughput::hash_multithreaded(Mode::Plain, black_box(message), len));
                } else {
                    black_box(convenience::hash(black_box(&Block::bytes(message)[..len])));
                }
            };
            let write = |message: &mut [Block]| Block::bytes_mut(message)[..len].copy_from_slice(black_box(&source[..len]));
            clocks::load::tick();
            // The first call, untimed, as the timed one will run: with the helpers awake for every way but "no warning".
            let ready = Setting { name: String::new(), awake: helpers, split: 32 * KIB, per: 32 * KIB };
            let ships = Setting { name: String::new(), awake: 0, split: 512 * KIB, per: 128 * KIB };
            settle(if way == Readied::Ships { &ships } else { &ready }, workers, &mut unsettled);
            write(&mut message);
            call(&message);
            // The rest: every worker parked unless this way keeps them awake.
            match way {
                Readied::Ships | Readied::Kept => {}
                Readied::Warned(_) => asleep(32 * KIB),
            }
            clocks::Gap::Idle.spend(rest_ns);
            if let Readied::Warned(lead) = way {
                blake3_servil::__probe_warn(helpers, 32 * KIB, 32 * KIB);
                if lead > 0 {
                    clocks::Gap::Idle.spend(lead);
                }
            }
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
            keep(&mut samples[w][c], (write_ns, hash_ns, counts));
        }
    }
    let run_ns = clocks::since_ns(started);
    blake3_servil::__probe_latency(0, 512 * KIB, 128 * KIB);
    let mut head = facts(&format!("warn: {visits} visits a cell, {helpers} workers woken or kept, after {} ms of rest", rest_ns / 1_000_000), run_ns, workers);
    head += &format!("# settles that did not complete in 200 ms: {unsettled}\n");
    // Reported through the shared table: a "setting" for each way, a cell for each call.
    let settings: Vec<Setting> = ways
        .iter()
        .map(|(name, way)| Setting { name: (*name).to_owned(), awake: if *way == Readied::Ships { 0 } else { helpers }, split: 32 * KIB, per: 32 * KIB })
        .collect();
    let cells: Vec<Cell> = calls.iter().map(|&(multithreaded, len)| Cell { pause: Pause::Idle(rest_ns), multithreaded, len }).collect();
    report("host_lab.warn.tsv", &head, &settings, &cells, &samples);
}

/// What an `early` cell does at the hello, a lead before the bytes come:
/// what making the hasher then could do, with nothing left running.
#[derive(Clone, Copy, PartialEq)]
enum Early {
    /// Nothing: the library as it ships.
    Nothing,
    /// The kernels the call will run, run once on the calling thread.
    Run,
    /// The kernels run once, and three workers woken, who find nothing
    /// and park again.
    RunAndWake,
}

fn early(visits: usize) {
    assert!(visits >= 1);
    blake3_servil::initialize_multithreaded(None);
    clocks::tight_timers();
    let (_, workers) = blake3_servil::__probe_parked();
    let helpers = workers.min(3);
    let earlies = [("nothing early", Early::Nothing), ("kernels run", Early::Run), ("run and wake", Early::RunAndWake)];
    let leads: [(&str, u64); 4] = [("50 us", 50_000), ("200 us", 200_000), ("1 ms", 1_000_000), ("5 ms", 5_000_000)];
    // A way is what the hello does and how long before the bytes it comes.
    let ways: Vec<(String, Early, u64)> =
        leads.iter().flat_map(|&(lead_name, lead)| earlies.iter().map(move |&(name, early)| (format!("{name}, {lead_name}"), early, lead))).collect();
    let calls = [(false, KIB), (false, 16 * KIB), (false, 64 * KIB), (true, 1 << 20)];
    let cells: Vec<Cell> = [Pause::Idle(5_000_000), Pause::OtherWork]
        .iter()
        .flat_map(|&pause| calls.iter().map(move |&(multithreaded, len)| Cell { pause, multithreaded, len }))
        .collect();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let (source, mut message) = buffers(&mut rng);
    // What the hello's run hashes: other bytes than the message's.
    let (scratch, _) = buffers(&mut rng);
    let work = vec![1u8; OTHER_WORK_BYTES];
    let mut samples = vec![vec![Samples::default(); cells.len()]; ways.len()];
    let mut order: Vec<(usize, usize)> = (0..ways.len()).flat_map(|w| (0..cells.len()).map(move |c| (w, c))).collect();
    let started = clocks::now();
    for _ in 0..visits {
        rng.shuffle(&mut order);
        for &(w, c) in &order {
            let (early, lead) = (ways[w].1, ways[w].2);
            let Cell { pause, multithreaded, len } = cells[c];
            let call = |message: &[Block]| {
                if multithreaded {
                    black_box(throughput::hash_multithreaded(Mode::Plain, black_box(message), len));
                } else {
                    black_box(convenience::hash(black_box(&Block::bytes(message)[..len])));
                }
            };
            let write = |message: &mut [Block]| Block::bytes_mut(message)[..len].copy_from_slice(black_box(&source[..len]));
            clocks::load::tick();
            // The same call first, untimed; then the program's gap.
            write(&mut message);
            call(&message);
            match pause {
                Pause::Nonstop => {}
                Pause::OtherWork => clocks::Gap::Busy(&work).spend(OTHER_WORK_NS),
                Pause::Idle(ns) => clocks::Gap::Idle.spend(ns),
            }
            // The hello.
            if early != Early::Nothing {
                black_box(convenience::hash(black_box(&scratch[..len.min(64 * KIB)])));
            }
            if early == Early::RunAndWake {
                // Woken, and the knob back at once: they find nothing and park.
                blake3_servil::__probe_warn(helpers, 512 * KIB, 128 * KIB);
                blake3_servil::__probe_warn(0, 512 * KIB, 128 * KIB);
            }
            // The wait for the bytes: nothing runs.
            clocks::Gap::Idle.spend(lead);
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
            keep(&mut samples[w][c], (write_ns, hash_ns, counts));
        }
    }
    let run_ns = clocks::since_ns(started);
    let what = format!("early: {visits} visits a cell; the hello's run hashes other bytes, as many as the call's up to 64 KiB; a wake wakes {helpers} workers");
    let head = facts(&what, run_ns, workers);
    let settings: Vec<Setting> = ways.iter().map(|(name, ..)| Setting { name: name.clone(), awake: 0, split: 512 * KIB, per: 128 * KIB }).collect();
    report("host_lab.early.tsv", &head, &settings, &cells, &samples);
}

/// This process's energy so far by the system's own estimate, and the
/// performance cores' share of it, in nanojoules (macOS: rusage_info_v6's
/// ri_energy_nj and ri_penergy_nj); elsewhere the package's counter where
/// the process may read it, with no share.
fn energy_nj() -> Option<(u64, Option<u64>)> {
    #[cfg(target_vendor = "apple")]
    {
        unsafe extern "C" {
            fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut u64) -> i32;
            fn getpid() -> i32;
        }
        let mut words = [0u64; 64];
        // Sound: `words` is writable and longer than rusage_info_v6 (flavor 6).
        let rc = unsafe { proc_pid_rusage(getpid(), 6, words.as_mut_ptr()) };
        return (rc == 0).then(|| (words[42], Some(words[43])));
    }
    #[allow(unreachable_code)]
    clocks::energy::package_nj().map(|nj| (nj, None))
}

fn price(seconds: usize) {
    assert!(seconds >= 1);
    blake3_servil::initialize_multithreaded(None);
    let (_, workers) = blake3_servil::__probe_parked();
    let mut awakes: Vec<usize> = [0, 1, 3, 7, workers].into_iter().filter(|&k| k <= workers).collect();
    awakes.dedup();
    let there_and_back: Vec<usize> = awakes.iter().chain(awakes.iter().rev()).copied().collect();
    let hold = std::time::Duration::from_secs(seconds as u64);
    let rest = std::time::Duration::from_secs(5);
    let mut unsettled = 0;
    println!("# host_lab price: {seconds} s a hold, 5 s of rest before each, the calling thread asleep throughout");
    println!("# blake3-servil {}", blake3_servil::BUILD_VERSION);
    println!(
        "# target: {} {}; cpus: {}; pool workers: {workers}",
        std::env::consts::ARCH,
        std::env::consts::OS,
        std::thread::available_parallelism().map_or(0, |n| n.get())
    );
    println!(
        "# energy: {}",
        if cfg!(target_vendor = "apple") {
            "the system's estimate for this process (ri_energy_nj), a model"
        } else {
            "the package's counter (energy-pkg), every program's"
        }
    );
    println!("{:<12} {:>6} {:>8} {:>10} {:>8} {:>10}", "start", "awake", "wall s", "CPUs busy", "watts", "P share %");
    for awake in there_and_back {
        blake3_servil::__probe_latency(0, 512 * KIB, 128 * KIB);
        std::thread::sleep(rest);
        settle(&Setting { name: String::new(), awake, split: 512 * KIB, per: 128 * KIB }, workers, &mut unsettled);
        clocks::load::tick();
        let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let (energy, cpu, started) = (energy_nj(), clocks::process_cpu_ns(), clocks::now());
        std::thread::sleep(hold);
        let (wall_ns, cpu_ns, energy_after) = (clocks::since_ns(started), clocks::process_cpu_ns() - cpu, energy_nj());
        let watts = energy.zip(energy_after).map(|((before, _), (after, _))| (after - before) as f64 / wall_ns as f64);
        let p_share = energy
            .zip(energy_after)
            .and_then(|((before, p_before), (after, p_after))| Some((p_after? - p_before?) as f64 * 100.0 / (after - before).max(1) as f64));
        println!(
            "{:<12} {:>6} {:>8.2} {:>10.2} {:>8} {:>10}",
            at,
            awake,
            wall_ns as f64 / 1e9,
            cpu_ns as f64 / wall_ns as f64,
            watts.map_or("-".to_owned(), |w| format!("{w:.2}")),
            p_share.map_or("-".to_owned(), |p| format!("{p:.0}"))
        );
    }
    blake3_servil::__probe_latency(0, 512 * KIB, 128 * KIB);
    let windows = clocks::load::windows();
    println!("# settles that did not complete in 200 ms: {unsettled}");
    println!(
        "# other programs' load: {} windows, {} busy; {}",
        windows.len(),
        windows.iter().filter(|w| w.busy()).count(),
        clocks::load::describe(&windows).replace('\n', " | ")
    );
}
