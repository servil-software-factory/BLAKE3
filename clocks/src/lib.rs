//! The one place BLAKE3-Servil and bench-hashes read clocks. A
//! measurement reads these through it:
//!
//! - **Wall time**, what a caller waits for: the platform's hardware
//!   counter, read raw ([`now`], [`since_ns`]): `CLOCK_MONOTONIC_RAW` on
//!   Linux, `CLOCK_UPTIME_RAW` on Darwin. Raw means no NTP: Linux's
//!   `CLOCK_MONOTONIC` (what `std::time::Instant` reads there) is slewed
//!   by the time daemon, up to 500 ppm, which could be in error or
//!   another's to control and helps no measurement here (Zooko, November
//!   4, 2026; measure-clocks3 compares the clocks). A counter read can
//!   only over-count, when the thread is interrupted.
//! - **The thread's counts per core kind** ([`Counts`]): cycles,
//!   instructions, and time on performance and efficiency cores, from
//!   Apple's `thread_selfcounts(THSC_TIME_CPI_PER_PERF_LEVEL)`, and on
//!   Linux x86-64 CPUs through `perf_event_open`: hybrid CPUs use the
//!   `cpu_core` and `cpu_atom` PMUs; homogeneous CPUs use generic hardware
//!   events (recorded in `p`, with `e` zero). Linux counts user-space
//!   cycles over running time (see `linux_counts`). Cycles over time is
//!   the clock the thread ran at,
//!   lower where the core waited on the SME unit; the split says which
//!   kind of core ran it. Other platforms give none ([`Counts::read`]
//!   returns `None`), and results say so.
//! - **The process's CPU time** ([`process_cpu_ns`]), all its threads
//!   together: what a call costs the machine beside what its caller waits
//!   for, read outside the timed interval (bench-hashes records it for each
//!   solo sample of its nonstop cells; a sample after a gap has none, the
//!   gap's other work being no part of the call's), and this process's share
//!   of the machine's busy time ([`load`]).
//! - **The CPU package's energy** ([`energy`]), where the platform counts
//!   it.
//!
//! Read wall time alone inside a timed interval and the counts outside it:
//! a counts read is a system call.
//!
//! **What a read costs** (an M4 Max, November 9, 2026; BLAKE3-Servil's
//! `probe/measure-clocks3`, Mac jobs 2068-2072):
//!
//! - A wall read: 12-23 ns, 41 cycles, 81 instructions.
//! - A `thread_selfcounts` read: 110-154 ns, about 400 cycles, 1,613
//!   instructions. The two reads around an interval add about 470 cycles
//!   and 1,792 instructions to its counts, and about 10 ns to its wall
//!   time. The instructions are constant to ±1. The cycles vary with the
//!   cache state the interval leaves behind.
//! - Apple's `kpc` read (root): 578 ns, 2,391 cycles, 12,370
//!   instructions, four to six times more. It buys the configurable
//!   events for diagnosis: cache and TLB misses, dispatch stalls, SME-unit
//!   waits, 104 in all, eight at a time beside cycles and instructions.
//! - A direct counter read (`mrs` of PMC0, PMC1, or `PMCCNTR_EL0`) traps
//!   in user mode, as root too, counting on or off. The kernel leaves
//!   `PMCR0.USEREN` off and exposes no switch.
//!
//! So an interval is made long against 470 cycles: a batch of calls, or
//! several prepared sets finished inside one bracket. zooko/measure-clocks3
//! at 5005425b on the same Mac agrees with its December 2025 results: every
//! wall clock steps 41.67 ns, the thread CPU-time clock has the smallest
//! spread (it stops while the thread is interrupted), and process CPU time
//! steps 1 μs.
//!
//! **Another program's run** ([`child`]): wall time from its start to its
//! exit, with the counts the OS keeps for the process (CPU time, peak
//! memory, reads from storage, and on macOS cycles per core kind).
//!
//! **Other programs' load** ([`load`]): every measurement here also records
//! how many CPUs other programs kept busy, in windows of about a second,
//! read between samples; each [`Batch`] records when it started, so its
//! window is known, and a busy window is reported on stderr as it closes.
//!
//! **Resolution.** The wall clock steps in ticks of the platform's counter:
//! 24 MHz, 41.67 ns, on an Apple M4 Max (every Darwin wall clock and the
//! CPU-time clocks alike; `CLOCK_REALTIME` and `CLOCK_MONOTONIC` in whole
//! microseconds: measure-clocks3, results of December 7, 2025) and in the
//! Linux VM (`arch_timer` at 24 MHz). A reading is a whole number of
//! ticks. So an interval of a few ticks is measured in one of two ways:
//! as a batch of calls long enough that a tick is a small share of it
//! ([`measure`]), or, where every call must be timed alone (a call after
//! a gap), as the sum of many such readings, each starting at a phase
//! against the ticks that nothing correlates with the call
//! ([`measure_after_gaps_prepared`]): the sum's rounding averages out. A median or
//! a minimum of single short readings keeps the rounding; take neither.

// Unix's clocks (Linux and macOS); Windows is not supported (BLAKE3 #160).
#[cfg(not(unix))]
compile_error!("clocks reads Unix's clocks (Linux and macOS); Windows is not supported (https://github.com/servil-software-factory/BLAKE3/issues/160)");

pub mod child;
pub mod energy;
pub mod events;
pub mod load;
pub mod other_code;
pub mod summary;
pub mod threads;

/// The wall clock, as reports name it.
#[cfg(target_vendor = "apple")]
pub const WALL_CLOCK: &str = "CLOCK_UPTIME_RAW (mach_absolute_time; stops during sleep; the raw counter, no NTP)";
#[cfg(all(unix, not(target_vendor = "apple")))]
pub const WALL_CLOCK: &str = "CLOCK_MONOTONIC_RAW (stops during suspend; the raw counter, no NTP slew)";

/// A point on the wall clock: the raw counter's nanoseconds. Only
/// differences are meaningful, and only within one boot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Stamp(u64);

/// The wall clock now.
#[inline(always)]
pub fn now() -> Stamp {
    Stamp(clock_ns())
}

/// Nanoseconds of wall time since `start`.
#[inline(always)]
pub fn since_ns(start: Stamp) -> u64 {
    clock_ns() - start.0
}

/// The clock every stamp reads: the raw counter, or in a test that froze
/// it, the test clock (test_clock), so a window's or an interval's
/// bookkeeping is checked against exact times in microseconds instead of
/// against real time and its noise. A release build has no test clock.
#[inline(always)]
fn clock_ns() -> u64 {
    #[cfg(test)]
    if let Some(ns) = test_clock::reading() {
        return ns;
    }
    raw_ns()
}

/// A clock a test holds (Zooko and John, November 3, 2026, for
/// BLAKE3-Servil's pool: a test-only clock, where the code reads time
/// through one function): `freeze` stops this thread's stamps at the real
/// clock's present value, `advance` moves them; `thaw` gives the real
/// clock back. Other threads keep the real clock.
#[cfg(test)]
pub(crate) mod test_clock {
    use std::cell::Cell;
    thread_local! {
        static HELD: Cell<Option<u64>> = const { Cell::new(None) };
    }
    pub fn reading() -> Option<u64> {
        HELD.with(Cell::get)
    }
    pub fn freeze() {
        HELD.with(|h| h.set(Some(super::raw_ns())));
    }
    pub fn advance(ns: u64) {
        HELD.with(|h| h.set(Some(h.get().expect("a frozen clock advances") + ns)));
    }
    pub fn thaw() {
        HELD.with(|h| h.set(None));
    }
}

/// The raw counter in nanoseconds: `clock_gettime(CLOCK_MONOTONIC_RAW)`
/// on Linux (the vDSO's, 11 ns a read on an i7-12700K, 1 ns steps),
/// `clock_gettime_nsec_np(CLOCK_UPTIME_RAW)` on Darwin (41.67 ns steps on
/// an M4 Max). Neither is adjusted by a time daemon.
#[cfg(all(unix, not(target_vendor = "apple")))]
#[inline(always)]
fn raw_ns() -> u64 {
    #[repr(C)]
    struct Timespec {
        sec: i64,
        nsec: i64,
    }
    unsafe extern "C" {
        fn clock_gettime(clock_id: i32, tp: *mut Timespec) -> i32;
    }
    const CLOCK_MONOTONIC_RAW: i32 = 4;
    let mut ts = Timespec { sec: 0, nsec: 0 };
    // Sound: clock_gettime writes the timespec it is given.
    let r = unsafe { clock_gettime(CLOCK_MONOTONIC_RAW, &mut ts) };
    debug_assert_eq!(r, 0, "clock_gettime(CLOCK_MONOTONIC_RAW)");
    ts.sec as u64 * 1_000_000_000 + ts.nsec as u64
}

#[cfg(target_vendor = "apple")]
#[inline(always)]
fn raw_ns() -> u64 {
    unsafe extern "C" {
        fn clock_gettime_nsec_np(clock_id: u32) -> u64;
    }
    const CLOCK_UPTIME_RAW: u32 = 8;
    // Sound: a plain libc call.
    unsafe { clock_gettime_nsec_np(CLOCK_UPTIME_RAW) }
}

/// One core kind's share of a thread's counts: its cycles and
/// instructions retired in user space, its requests that reached the
/// last-level cache and those that missed it (perf's generic
/// `cache-references` and `cache-misses`: on Intel
/// `LONGEST_LAT_CACHE.REFERENCE` and `.MISS`, demand and prefetch
/// requests to L3 and the lines fetched from beyond it; AMD maps them by
/// generation), and the time its counters ran.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Level {
    pub cycles: u64,
    pub instructions: u64,
    pub cache_references: u64,
    pub cache_misses: u64,
    pub time_ns: u64,
}

/// A thread's counts so far, on performance cores (`p`) and efficiency
/// cores (`e`); homogeneous Linux x86 CPUs use `p`, with `e` zero.
/// The difference of two reads covers what ran between them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub p: Level,
    pub e: Level,
}

/// What the routine counts' own reads cost, as a samples file records it
/// beside its samples: the median, over 1,001 brackets around no work, of
/// two `Counts::read`s' cycles and instructions, on each kind of core
/// (Linux: a thread pinned to one CPU of each kind; elsewhere the calling
/// thread). Every counted sample carries this much that is the reads' own,
/// so a sample's work against it says how diluted the reads are
/// (bench-hashes #160: a quarter of a 0.6 μs sample on the Mac, 0.4% of
/// 36 μs). "not measured" where the platform gives no counts.
pub fn read_overhead() -> String {
    /// Pin the calling thread to `cpu` (Linux); whether it held.
    fn pin(cpu: usize) -> bool {
        #[cfg(target_os = "linux")]
        {
            unsafe extern "C" {
                fn sched_setaffinity(pid: i32, size: usize, set: *const u64) -> i32;
            }
            let mut set = [0u64; 16];
            if cpu >= 1024 {
                return false;
            }
            set[cpu / 64] |= 1 << (cpu % 64);
            // Sound: `set` is a cpu_set_t of 1024 bits; pid 0 is this thread.
            unsafe { sched_setaffinity(0, core::mem::size_of_val(&set), set.as_ptr()) == 0 }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = cpu;
            false
        }
    }
    fn median(mut v: Vec<u64>) -> u64 {
        v.sort_unstable();
        v[v.len() / 2]
    }
    /// The brackets on the calling thread: (P cycles, P instructions, E
    /// cycles, E instructions) medians, each kind where it ran.
    fn brackets() -> Option<[Option<(u64, u64)>; 2]> {
        let mut kinds = [(Vec::new(), Vec::new()), (Vec::new(), Vec::new())];
        for _ in 0..1001 {
            let then = Counts::read()?;
            let now = Counts::read()?;
            let d = now.since(then);
            for (k, level) in [d.p, d.e].into_iter().enumerate() {
                if level.cycles > 0 {
                    kinds[k].0.push(level.cycles);
                    kinds[k].1.push(level.instructions);
                }
            }
        }
        Some(kinds.map(|(c, i)| (!c.is_empty()).then(|| (median(c), median(i)))))
    }
    let on = |cpus: Option<&str>| -> Option<[Option<(u64, u64)>; 2]> {
        match cpus {
            None => brackets(),
            Some(cpu) => {
                let cpu: usize = cpu.parse().ok()?;
                std::thread::spawn(move || pin(cpu).then(brackets).flatten()).join().ok()?
            }
        }
    };
    // The first CPU of each kind, where Linux names the kinds.
    let first = |pmu: &str| -> Option<String> {
        let list = std::fs::read_to_string(format!("/sys/bus/event_source/devices/{pmu}/cpus")).ok()?;
        Some(list.trim().split(|c| c == ',' || c == '-').next()?.to_owned())
    };
    let (p, e) = match (first("cpu_core"), first("cpu_atom")) {
        (Some(p), Some(e)) => (on(Some(&p)).and_then(|k| k[0]), on(Some(&e)).and_then(|k| k[1])),
        _ => match on(None) {
            Some([p, e]) => (p, e),
            None => (None, None),
        },
    };
    let show = |kind: &str, v: Option<(u64, u64)>| v.map(|(c, i)| format!("{kind} {c} cycles, {i} instructions"));
    let kinds: Vec<String> = [show("P", p), show("E", e)].into_iter().flatten().collect();
    if kinds.is_empty() {
        return "not measured".to_owned();
    }
    format!("{} (the median of 1,001 brackets around no work, two counts reads each)", kinds.join("; "))
}

impl Counts {
    /// The calling thread's counts, or `None` where the platform gives none.
    pub fn read() -> Option<Counts> {
        imp::read()
    }

    /// The counts accumulated since `earlier`, a read on the same thread.
    pub fn since(self, earlier: Counts) -> Counts {
        let level = |now: Level, then: Level| Level {
            cycles: now.cycles - then.cycles,
            instructions: now.instructions - then.instructions,
            cache_references: now.cache_references - then.cache_references,
            cache_misses: now.cache_misses - then.cache_misses,
            time_ns: now.time_ns - then.time_ns,
        };
        Counts { p: level(self.p, earlier.p), e: level(self.e, earlier.e) }
    }

    /// These counts and `other`'s together.
    pub fn plus(self, other: Counts) -> Counts {
        let level = |a: Level, b: Level| Level {
            cycles: a.cycles + b.cycles,
            instructions: a.instructions + b.instructions,
            cache_references: a.cache_references + b.cache_references,
            cache_misses: a.cache_misses + b.cache_misses,
            time_ns: a.time_ns + b.time_ns,
        };
        Counts { p: level(self.p, other.p), e: level(self.e, other.e) }
    }

    /// Cycles per microsecond of the thread's time on cores, rounded: the
    /// clock it ran at in MHz (lower where it waited on the SME unit).
    /// Requires some time counted.
    pub fn mhz(&self) -> u64 {
        let ns = self.p.time_ns + self.e.time_ns;
        assert!(ns > 0, "a clock rate needs some time counted");
        ((self.p.cycles + self.e.cycles) * 1000 + ns / 2) / ns
    }

    /// The efficiency cores' share of the thread's time, in percent, rounded.
    /// Requires some time counted.
    pub fn e_percent(&self) -> u64 {
        let ns = self.p.time_ns + self.e.time_ns;
        assert!(ns > 0, "a share needs some time counted");
        (self.e.time_ns * 100 + ns / 2) / ns
    }
}

/// One batch of [`measure`]: `calls` calls in `wall_ns`, with the counts
/// the thread accumulated around it, started at `started_ns` ([`load::now_ns`],
/// the scale of the load windows). Nonstop batches also record the whole
/// process's CPU time. A caller sharing its process with another measured
/// program leaves that CPU figure out of its report.
#[derive(Clone, Copy, Debug)]
pub struct Batch {
    pub calls: u64,
    pub wall_ns: u64,
    /// All threads' CPU time around a nonstop batch; unavailable clocks
    /// and after-gap measurements give None.
    pub cpu_ns: Option<u64>,
    pub counts: Option<Counts>,
    pub started_ns: u64,
}

impl Batch {
    /// "12.3 ns/call, 4390 MHz P" (or "12.3 ns/call, no cycle counts").
    pub fn show(&self) -> String {
        let tenths = (self.wall_ns * 10 + self.calls / 2) / self.calls;
        let rate = match self.counts {
            Some(c) if c.p.time_ns + c.e.time_ns > 0 => {
                let kind = match c.e_percent() {
                    0..=4 => "P".to_owned(),
                    96.. => "E".to_owned(),
                    e => format!("{e}% E"),
                };
                format!("{} MHz {kind}", c.mhz())
            }
            _ => "no cycle counts".to_owned(),
        };
        format!("{}.{} ns/call, {rate}", tenths / 10, tenths % 10)
    }
}

/// `batches` batches of `f`, each about `batch_ns` long (sized by a first,
/// untimed run of that length), in the order taken; wall time inside each
/// batch, the process CPU time and thread counts around it. Calibration
/// completes at least one call, even when the requested budget is tiny.
pub fn measure(batches: usize, batch_ns: u64, mut f: impl FnMut()) -> Vec<Batch> {
    assert!(batches > 0 && batch_ns > 0, "a measurement takes at least one batch of some length");
    let started = now();
    let mut calls = 0u64;
    loop {
        f();
        calls += 1;
        if since_ns(started) >= batch_ns {
            break;
        }
    }
    (0..batches)
        .map(|_| {
            load::tick();
            let started_ns = load::now_ns();
            let before = Counts::read();
            let cpu = process_cpu_ns();
            let t = now();
            for _ in 0..calls {
                f();
            }
            let wall_ns = since_ns(t);
            let cpu_ns = Some(process_cpu_ns() - cpu);
            let counts = before.zip(Counts::read()).map(|(before, after)| after.since(before));
            Batch { calls, wall_ns, cpu_ns, counts, started_ns }
        })
        .collect()
}

/// A measurement with the producer's preparation timed separately.
/// `calls` contains only the hashing calls; `preparation` contains only
/// the writes that produce their input. The gap belongs to neither.
#[derive(Clone, Copy, Debug)]
pub struct PreparedBatch {
    pub calls: Batch,
    pub preparation: Batch,
}

/// What a program does between two calls it makes now and then (Zooko,
/// September 30, 2026: both are measured, and compared).
#[derive(Clone, Copy, Debug)]
pub enum Gap<'a> {
    /// Idling: the thread sleeps, as a network server waits for its next
    /// packet. The core may slow, power down, or pass the thread to
    /// another core; the calls after it run at whatever speeds follow.
    Idle,
    /// Other work: a fixed other program ([`other_code::run`], about
    /// 1 MiB of distinct code), then a walk of all of this buffer at
    /// 64-byte intervals, then integer work for the rest of the gap, as
    /// a program hashes between other tasks, or on a machine busy with
    /// other programs: their code and data fill the caches. The caller keeps the buffer
    /// across samples and chooses it larger than those caches.
    Busy(&'a [u8]),
}

impl Gap<'_> {
    /// Spend at least `gap_ns` as this gap describes: an idle gap sleeps
    /// `gap_ns`; a busy one runs its whole program, even past `gap_ns`.
    pub fn spend(&self, gap_ns: u64) {
        match self {
            Gap::Idle => std::thread::sleep(std::time::Duration::from_nanos(gap_ns)),
            Gap::Busy(work) => {
                let started = now();
                other_code::run();
                let mut sum = 0u64;
                for &byte in std::hint::black_box(*work).iter().step_by(64) {
                    sum = sum.wrapping_add(u64::from(byte));
                }
                std::hint::black_box(sum);
                busy_work(gap_ns.saturating_sub(since_ns(started)));
            }
        }
    }
}

/// `calls` calls of `f`, each after its own `gap` of at least `gap_ns`
/// ([`Gap::spend`]), with a producer that writes the input after the gap,
/// before the call. One more call comes first, gap and all, untimed: each
/// timed call then follows the same call (Zooko, September 30, 2026). A
/// gap leaves some of what ran before it in the caches, so a first call
/// would otherwise carry whatever the caller ran earlier (another
/// function, another size). The
/// gap belongs to neither interval: after it, this helper warms its own
/// timing path (a counts read, a clock read), so the interval measures the
/// call, whose code stays as the gap left it. The
/// preparation and the call each record wall time and thread counts
/// (counts read outside the wall intervals), summed over the calls (see
/// "Resolution" above for why a sum). Requires at least one call, and a
/// nonempty buffer for a busy gap.
pub fn measure_after_gaps_prepared<T: ?Sized>(
    calls: u64,
    gap: Gap,
    gap_ns: u64,
    input: &mut T,
    mut prepare: impl FnMut(&mut T),
    mut f: impl FnMut(&T),
) -> PreparedBatch {
    assert!(calls > 0, "a measurement takes at least one call");
    if let Gap::Busy(work) = gap {
        assert!(!work.is_empty(), "a busy gap needs a nonempty work buffer");
    }
    let started_ns = load::now_ns();
    let empty = || Batch { calls, wall_ns: 0, cpu_ns: None, counts: Some(Counts::default()), started_ns };
    let mut measured = PreparedBatch { calls: empty(), preparation: empty() };
    for call in 0..=calls {
        load::tick();
        gap.spend(gap_ns);
        if call == 0 {
            prepare(input);
            f(input);
            continue;
        }
        std::hint::black_box(Counts::read());
        std::hint::black_box(since_ns(now()));

        let before = Counts::read();
        let t = now();
        prepare(input);
        measured.preparation.wall_ns += since_ns(t);
        let counts = before.zip(Counts::read()).map(|(before, after)| after.since(before));
        measured.preparation.counts = measured.preparation.counts.zip(counts).map(|(sum, call)| sum.plus(call));

        let before = Counts::read();
        let t = now();
        f(input);
        measured.calls.wall_ns += since_ns(t);
        let counts = before.zip(Counts::read()).map(|(before, after)| after.since(before));
        measured.calls.counts = measured.calls.counts.zip(counts).map(|(sum, call)| sum.plus(call));
    }
    measured
}

/// Sleep until `elapsed_ns` after `start`. A late caller proceeds directly.
/// Fixed arrival schedules use this so consumer work leaves later deadlines
/// unchanged. Sleep overshoot is carried into the next call, as queued I/O is.
pub fn sleep_until(start: Stamp, elapsed_ns: u64) {
    sleep_until_with(start, elapsed_ns, |ns| Gap::Idle.spend(ns));
}

fn sleep_until_with(start: Stamp, elapsed_ns: u64, sleep: impl FnOnce(u64)) {
    let left = elapsed_ns.saturating_sub(since_ns(start));
    if left != 0 {
        sleep(left);
    }
}

/// Keep the calling thread busy for `ns` of wall time with integer
/// arithmetic in registers (a multiply-add chain), touching no memory: the
/// program's own work between calls, which leaves the caches as the call
/// last left them.
pub fn busy_work(ns: u64) {
    let started = now();
    let mut x = 1u64;
    while since_ns(started) < ns {
        for _ in 0..256 {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        }
        x = std::hint::black_box(x);
    }
}

/// The CPU time this process has used, all its threads together, with
/// that of its children it has reaped and of the one [`child::run`] is
/// running, in nanoseconds: a sample's CPU time is the difference of two
/// reads around it, and [`load`] tells this process's share of the
/// machine's busy time from other programs' with it (a launcher counts
/// what it runs as its own).
pub fn process_cpu_ns() -> u64 {
    {
        #[repr(C)]
        struct Timespec {
            tv_sec: i64,
            tv_nsec: i64,
        }
        unsafe extern "C" {
            fn clock_gettime(clock_id: i32, tp: *mut Timespec) -> i32;
            fn getrusage(who: i32, usage: *mut i64) -> i32;
        }
        // CLOCK_PROCESS_CPUTIME_ID
        const CLOCK: i32 = if cfg!(target_vendor = "apple") { 12 } else { 2 };
        const RUSAGE_CHILDREN: i32 = -1;
        let mut ts = Timespec { tv_sec: 0, tv_nsec: 0 };
        // Sound: `ts` is writable.
        assert_eq!(unsafe { clock_gettime(CLOCK, &mut ts) }, 0, "clock_gettime(CLOCK_PROCESS_CPUTIME_ID)");
        let mut children = [0i64; 18];
        // Sound: `children` is writable and as long as a struct rusage.
        assert_eq!(unsafe { getrusage(RUSAGE_CHILDREN, children.as_mut_ptr()) }, 0, "getrusage(RUSAGE_CHILDREN)");
        timespec_ns(ts.tv_sec, ts.tv_nsec) + child::rusage_cpu_ns(&children) + child::running_cpu_ns()
    }
}

/// A timespec as nanoseconds.
fn timespec_ns(sec: i64, nsec: i64) -> u64 {
    sec as u64 * 1_000_000_000 + nsec as u64
}

/// macOS QoS classes: user-interactive runs on P-cores, background on E-cores.
pub const USER_INTERACTIVE: u32 = 0x21;
pub const BACKGROUND: u32 = 0x09;

/// Put the calling thread in QoS class `class` (macOS; elsewhere nothing).
pub fn set_qos(class: u32) {
    imp::set_qos(class)
}

/// Make the calling thread's sleeps land when asked (Linux: timer slack
/// 1 μs in place of the 50 μs every normal thread gets, which landed a
/// 20 μs sleep at 72 μs and a 5 ms one up to 300 μs late on the
/// i7-12700K; threads made after this inherit it). Elsewhere nothing.
/// A pause before a call is the measurement's; its length should be the
/// one the method names.
pub fn tight_timers() {
    #[cfg(target_os = "linux")]
    {
        unsafe extern "C" {
            fn prctl(option: i32, a: std::ffi::c_long, b: std::ffi::c_long, c: std::ffi::c_long, d: std::ffi::c_long) -> i32;
        }
        const PR_SET_TIMERSLACK: i32 = 29;
        // Sound: a plain prctl on the calling thread.
        let set = unsafe { prctl(PR_SET_TIMERSLACK, 1_000, 0, 0, 0) };
        assert_eq!(set, 0, "the kernel lets any thread set its own timer slack");
    }
}

#[cfg(target_vendor = "apple")]
mod imp {
    use super::{Counts, Level};
    use std::sync::OnceLock;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct ThscTimeCpi {
        instructions: u64,
        cycles: u64,
        user_time_mach: u64,
        system_time_mach: u64,
    }
    #[repr(C)]
    struct MachTimebaseInfo {
        numer: u32,
        denom: u32,
    }
    unsafe extern "C" {
        fn thread_selfcounts(kind: u32, dst: *mut std::ffi::c_void, size: usize) -> i32;
        fn mach_timebase_info(info: *mut MachTimebaseInfo) -> i32;
        fn pthread_set_qos_class_self_np(qos: u32, relpri: i32) -> i32;
    }
    const THSC_TIME_CPI_PER_PERF_LEVEL: u32 = 4;

    /// hw.nperflevels is 2 on every Apple silicon Mac: index 0 P, 1 E.
    fn levels() -> Option<[ThscTimeCpi; 2]> {
        let mut levels = [ThscTimeCpi::default(); 2];
        // Sound: `levels` is writable for the size passed.
        let rc = unsafe { thread_selfcounts(THSC_TIME_CPI_PER_PERF_LEVEL, levels.as_mut_ptr().cast(), std::mem::size_of_val(&levels)) };
        (rc == 0).then_some(levels)
    }

    pub fn read() -> Option<Counts> {
        // (numer, denom) for mach ticks to ns, and whether the call works here.
        static SETUP: OnceLock<Option<(u64, u64)>> = OnceLock::new();
        let (numer, denom) = (*SETUP.get_or_init(|| {
            let mut info = MachTimebaseInfo { numer: 0, denom: 0 };
            // Sound: `info` is writable.
            let ok = unsafe { mach_timebase_info(&mut info) } == 0 && info.denom != 0 && levels().is_some();
            ok.then_some((u64::from(info.numer), u64::from(info.denom)))
        }))?;
        let levels = levels().expect("thread_selfcounts failed after succeeding once");
        // A Mac counts no cache events (METHODOLOGY): zero.
        let level = |l: ThscTimeCpi| Level {
            cycles: l.cycles,
            instructions: l.instructions,
            time_ns: (l.user_time_mach + l.system_time_mach) * numer / denom,
            ..Level::default()
        };
        Some(Counts { p: level(levels[0]), e: level(levels[1]) })
    }

    pub fn set_qos(class: u32) {
        // Sound: a plain call on the calling thread.
        assert_eq!(unsafe { pthread_set_qos_class_self_np(class, 0) }, 0, "pthread_set_qos_class_self_np");
    }

    /// Mach absolute-time ticks as nanoseconds.
    pub(crate) fn mach_to_ns(mach: u64) -> u64 {
        static TIMEBASE: OnceLock<(u64, u64)> = OnceLock::new();
        let (numer, denom) = *TIMEBASE.get_or_init(|| {
            let mut info = MachTimebaseInfo { numer: 0, denom: 0 };
            // Sound: `info` is writable.
            assert_eq!(unsafe { mach_timebase_info(&mut info) }, 0, "mach_timebase_info");
            (u64::from(info.numer), u64::from(info.denom))
        });
        mach * numer / denom
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod linux_counts;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod imp {
    pub use super::linux_counts::read;

    pub fn set_qos(_class: u32) {}
}

#[cfg(not(any(target_vendor = "apple", all(target_os = "linux", target_arch = "x86_64"))))]
mod imp {
    pub fn read() -> Option<super::Counts> {
        None
    }

    pub fn set_qos(_class: u32) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_measurement_counts_its_calls_and_time() {
        let before = process_cpu_ns();
        let mut completed = 0;
        let batches = measure(3, 100_000, || {
            std::hint::black_box((0..100u64).sum::<u64>());
            completed += 1;
        });
        let total_cpu = Some(process_cpu_ns() - before);
        assert_eq!(batches.len(), 3);
        assert_eq!(completed, 4 * batches[0].calls, "calibration and three measured batches");
        for b in &batches {
            assert!(b.calls > 0 && b.wall_ns > 0, "{b:?}");
            assert_eq!(b.calls, batches[0].calls);
            assert_eq!(b.cpu_ns.is_some(), total_cpu.is_some());
            // The batch's CPU time is the process's over the batch: at
            // most the whole measurement's, and its work took some.
            assert!(b.cpu_ns.unwrap() <= total_cpu.unwrap() && b.cpu_ns.unwrap() >= b.wall_ns / 10, "{b:?} within {total_cpu:?}");
            assert!(b.show().contains("ns/call"));
        }
        if let Some(total) = total_cpu {
            assert!(batches.iter().map(|b| b.cpu_ns.unwrap()).sum::<u64>() <= total);
        }
    }

    #[test]
    fn tiny_budgets_still_complete_calls() {
        let mut completed = 0;
        let batches = measure(3, 1, || completed += 1);
        assert!(batches.iter().all(|b| b.calls > 0));
        assert_eq!(completed, 4 * batches[0].calls);
    }

    #[test]
    fn counts_subtract_and_rate() {
        let later = Counts { p: Level { cycles: 4_400, instructions: 9_000, time_ns: 1_000, ..Level::default() }, e: Level::default() };
        let d = later.since(Counts::default());
        assert_eq!((d.mhz(), d.e_percent()), (4_400, 0));
    }

    #[test]
    fn a_timespec_is_its_seconds_and_nanoseconds() {
        assert_eq!(timespec_ns(2, 5), 2_000_000_005);
        assert_eq!(timespec_ns(0, 999_999_999), 999_999_999);
    }

    /// The clock rate is both kinds' cycles over both kinds' time, rounded
    /// to the nearest MHz; the E share is the E time's share of the time,
    /// rounded to the nearest percent; the batch's line rounds its ns a
    /// call to tenths and names the kind by the share (P under 5%, E over
    /// 95%, the percentage between), or says it has no counts.
    #[test]
    fn rates_shares_and_the_batch_line_round_to_nearest() {
        let level = |cycles, time_ns| Level { cycles, time_ns, ..Level::default() };
        let mixed = Counts { p: level(3_000, 1_000), e: level(1_000, 3_000) };
        assert_eq!(mixed.mhz(), 1_000, "4000 cycles over 4000 ns");
        assert_eq!(mixed.e_percent(), 75);
        assert_eq!(Counts { p: level(4_001, 1_000), e: level(0, 1_000) }.mhz(), 2_001, "8002 / 4 = 2000.5 rounds up");
        assert_eq!(Counts { p: level(0, 2), e: level(0, 1) }.e_percent(), 33);
        assert_eq!(Counts { p: level(0, 5), e: level(0, 3) }.e_percent(), 38, "37.5 rounds up");
        let batch = |calls, wall_ns, counts| Batch { calls, wall_ns, cpu_ns: None, counts, started_ns: 0 };
        assert_eq!(batch(4, 50, Some(mixed)).show(), "12.5 ns/call, 1000 MHz 75% E");
        assert_eq!(batch(3, 10, Some(Counts { p: level(4_400, 1_000), e: level(0, 40) })).show(), "3.3 ns/call, 4231 MHz P");
        assert_eq!(batch(1, 7, Some(Counts { p: level(0, 0), e: level(2_000, 100) })).show(), "7.0 ns/call, 20000 MHz E");
        assert_eq!(batch(2, 3, None).show(), "1.5 ns/call, no cycle counts");
        assert_eq!(batch(2, 3, Some(Counts::default())).show(), "1.5 ns/call, no cycle counts", "counts with no time: none");
    }

    /// since takes each field of each kind off, plus adds each; the kinds
    /// stay apart.
    #[test]
    fn counts_since_and_plus_act_on_every_field_of_each_kind() {
        let level = |k: u64| Level { cycles: 10 * k, instructions: 20 * k, cache_references: 30 * k, cache_misses: 40 * k, time_ns: 50 * k };
        let earlier = Counts { p: level(1), e: level(2) };
        let later = Counts { p: level(4), e: level(9) };
        assert_eq!(later.since(earlier), Counts { p: level(3), e: level(7) });
        assert_eq!(earlier.plus(later), Counts { p: level(5), e: level(11) });
        assert_eq!(Counts::default().plus(earlier), earlier);
        assert_eq!(earlier.since(earlier), Counts::default());
    }
    #[test]
    fn prepared_calls_follow_their_writes_and_exclude_gaps() {
        let mut input = 0;
        let mut observed = Vec::new();
        let started = now();
        let measured = measure_after_gaps_prepared(
            3,
            Gap::Busy(&[7; 128]),
            1_000_000,
            &mut input,
            |input| {
                *input += 1;
                busy_work(100_000);
            },
            |input| {
                observed.push(*input);
                busy_work(50_000);
            },
        );
        assert_eq!(observed, [1, 2, 3, 4], "one untimed call, then the three timed");
        assert_eq!(measured.calls.calls, 3);
        assert_eq!(measured.preparation.calls, 3);
        assert_eq!(measured.calls.cpu_ns, None);
        assert_eq!(measured.preparation.cpu_ns, None);
        assert!(measured.calls.wall_ns >= 150_000);
        assert!(measured.preparation.wall_ns >= 300_000);
        assert!(since_ns(started) >= 4_000_000 + measured.calls.wall_ns + measured.preparation.wall_ns);
    }

    #[test]
    #[should_panic(expected = "nonempty work buffer")]
    fn busy_gaps_require_work() {
        measure_after_gaps_prepared(1, Gap::Busy(&[]), 0, &mut (), |_| {}, |_| {});
    }

    #[test]
    fn idle_gaps_sleep_outside_the_intervals() {
        let mut calls = 0;
        let started = now();
        let measured = measure_after_gaps_prepared(5, Gap::Idle, 1_000_000, &mut calls, |c| *c += 1, |_| busy_work(10_000));
        assert_eq!((calls, measured.calls.calls), (6, 5), "one untimed call, then the five timed");
        let elapsed = since_ns(started);
        assert!(elapsed >= 6_000_000, "each gap sleeps its length");
        // The intervals and the six gaps are disjoint, and a sleep never ends
        // early, so the whole run holds both: elapsed >= intervals + 6 ms.
        // Gaps inside the intervals would count about 5 ms twice and break it.
        // No upper bound on time: a slow or loaded machine only adds to
        // `elapsed` (an earlier form, wall_ns < 5 ms, failed on one preemption).
        assert!(
            elapsed >= measured.calls.wall_ns + measured.preparation.wall_ns + 6_000_000,
            "the sleeps stay out of the sum: {elapsed} ns in all, {measured:?}"
        );
    }

    /// The machine's properties as this crate relies on them: timing, the
    /// OS's counters, children. Seconds each and sensitive to other load,
    /// so they run on purpose (`cargo nextest run -P machine`) before a
    /// release and on a new machine, and not beside every change or mutant.
    mod machine {
        use super::*;

        /// The wall clock is the raw counter: it only goes forward, it steps
        /// finely (under 100 ns a step: a clock stepping in whole microseconds,
        /// as Darwin's CLOCK_MONOTONIC through clock_gettime does, would fail
        /// here), and it reads at the cost of a function call. A change of the
        /// clock behind `now` shows here.
        /// On Linux a thread's sleeps land late by its timer slack (50 μs by
        /// default); after tight_timers a 20 μs sleep lands within 10 μs of
        /// it, in the median of 200. Elsewhere the call is nothing and the
        /// sleep lands as the OS lands it.
        #[test]
        #[cfg(target_os = "linux")]
        fn tight_timers_land_a_brief_sleep_on_time() {
            let landing = || {
                let mut v: Vec<u64> = (0..200)
                    .map(|_| {
                        let t = now();
                        std::thread::sleep(std::time::Duration::from_micros(20));
                        since_ns(t)
                    })
                    .collect();
                v.sort_unstable();
                v[100]
            };
            std::thread::spawn(move || {
                let loose = landing();
                tight_timers();
                let tight = landing();
                eprintln!("a 20 μs sleep lands at {loose} ns with the default slack, {tight} ns with 1 μs");
                assert!(tight < 30_000, "a 20 μs sleep lands within 10 μs: {tight} ns");
                assert!(tight < loose, "tight timers land sooner than the default slack ({loose} ns)");
            })
            .join()
            .unwrap();
        }

        #[test]
        fn the_wall_clock_is_fine_and_forward() {
            let mut last = now();
            let mut smallest_step = u64::MAX;
            let first = last;
            for _ in 0..100_000 {
                let t = now();
                assert!(t >= last, "the clock went back");
                let step = t.0 - last.0;
                if t > last && step < smallest_step {
                    smallest_step = step;
                }
                last = t;
            }
            assert!(smallest_step <= 100, "the clock's smallest step: {smallest_step} ns");
            let reads_ns = since_ns(first) / 100_000;
            assert!(reads_ns < 1_000, "a read costs about {reads_ns} ns");
        }

        /// On Linux the wall clock is CLOCK_MONOTONIC_RAW itself: a reading sits
        /// between two of that clock's, and away from CLOCK_MONOTONIC by the
        /// slew the time daemon has applied since boot (zero on a machine with
        /// none), which stays put across a short interval.
        #[test]
        #[cfg(all(unix, not(target_vendor = "apple")))]
        fn on_linux_the_wall_clock_is_the_raw_counter() {
            #[repr(C)]
            struct Timespec {
                sec: i64,
                nsec: i64,
            }
            unsafe extern "C" {
                fn clock_gettime(clock_id: i32, tp: *mut Timespec) -> i32;
            }
            let read = |clock: i32| {
                let mut ts = Timespec { sec: 0, nsec: 0 };
                assert_eq!(unsafe { clock_gettime(clock, &mut ts) }, 0);
                ts.sec as u64 * 1_000_000_000 + ts.nsec as u64
            };
            const CLOCK_MONOTONIC: i32 = 1;
            const CLOCK_MONOTONIC_RAW: i32 = 4;
            let before = read(CLOCK_MONOTONIC_RAW);
            let ours = now();
            let after = read(CLOCK_MONOTONIC_RAW);
            assert!(before <= ours.0 && ours.0 <= after, "a reading of ours lies between two of CLOCK_MONOTONIC_RAW's: {before} {} {after}", ours.0);
            // The slewed clock and ours differ by what the daemon has applied
            // since boot; the difference moves by at most the slew (500 ppm)
            // over 10 ms, under 5 μs, while a wrong clock behind `now` would
            // make it the full offset or a jump.
            let offset = || read(CLOCK_MONOTONIC) as i128 - now().0 as i128;
            let o1 = offset();
            std::thread::sleep(std::time::Duration::from_millis(10));
            let o2 = offset();
            assert!(
                (o1 - o2).abs() < 5_000 + 20_000,
                "the offset to CLOCK_MONOTONIC moved by {} ns in 10 ms (slew is under 5 μs; the rest a read's own time)",
                o1 - o2
            );
        }
    }
}

#[cfg(test)]
mod require_nextest {
    include!("../../tools/require_nextest.rs");
}

#[cfg(test)]
mod arrival_tests {
    use super::*;

    #[test]
    fn consumer_work_preserves_the_arrival_deadlines() {
        test_clock::freeze();
        let start = now();
        let mut sleeps = Vec::new();
        test_clock::advance(5_000);
        sleep_until_with(start, 20_000, |ns| {
            sleeps.push(ns);
            test_clock::advance(ns);
        });
        test_clock::advance(7_000);
        sleep_until_with(start, 40_000, |ns| {
            sleeps.push(ns);
            test_clock::advance(ns);
        });
        test_clock::advance(100_000);
        sleep_until_with(start, 60_000, |ns| sleeps.push(ns));
        test_clock::thaw();
        assert_eq!(sleeps, [15_000, 13_000], "late arrivals are already queued");
    }
}
