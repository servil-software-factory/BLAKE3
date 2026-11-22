//! The thread's counts on Linux x86-64 through `perf_event_open`. Hybrid
//! CPUs use one counter group (cycles, instructions, cache references,
//! cache misses) on each core kind's PMU, `cpu_core` and `cpu_atom`. Homogeneous CPUs use the generic hardware
//! PMU, recorded in `p`, with `e` zero. Cycles and instructions are user
//! space's; time is the group's running time, including kernel time, so
//! MHz reads low across system calls. The generic path requires both
//! hybrid PMU names to be absent. A sole `cpu_core` or `cpu_atom` gives no
//! counts, even on a homogeneous CPU: one missing PMU alone does not
//! establish the CPU's topology. A kernel that refuses perf access also
//! gives no counts.
//!
//! Each counter is read in user space where the kernel allows it
//! (`rdpmc`, with the counter's mmap page giving its index, offset and
//! the running time's TSC scale; 10-20 ns a read), else by the `read`
//! system call (180-490 ns). A read sits between a cell's pause and its
//! call, so its cost reaches the call: on the i7-12700K the system calls
//! made a 64 B call after 20 μs read 12% slower and after 5 ms 14% faster
//! than with no reads at all (bench-hashes #127, November 4, 2026).
use super::{Counts, Level};
use std::ffi::{c_int, c_long, c_void};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::OnceLock;

/// perf_event_attr, version 0 (64 bytes): the fields used, zero after.
#[repr(C)]
struct Attr {
    kind: u32,
    size: u32,
    config: u64,
    sample_period: u64,
    sample_type: u64,
    read_format: u64,
    flags: u64,
    reserved: [u64; 2],
}

unsafe extern "C" {
    fn syscall(number: c_long, ...) -> c_long;
    #[link_name = "read"]
    fn read_fd(fd: c_int, buffer: *mut c_void, size: usize) -> isize;
    fn mmap(addr: *mut c_void, len: usize, prot: c_int, flags: c_int, fd: c_int, offset: i64) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> c_int;
}

/// The head of a counter's mmap page (`perf_event_mmap_page`): the fields
/// user space reads under its seqlock.
#[repr(C)]
struct Page {
    version: u32,
    compat_version: u32,
    lock: u32,
    index: u32,
    offset: i64,
    time_enabled: u64,
    time_running: u64,
    capabilities: u64,
    pmc_width: u16,
    time_shift: u16,
    time_mult: u32,
    time_offset: u64,
}

const CAP_USER_RDPMC: u64 = 1 << 2;
const CAP_USER_TIME: u64 = 1 << 3;
const PAGE_LEN: usize = 4096;

/// A counter's mmap page, mapped while the counter is open.
struct Mapped(*const Page);

impl Mapped {
    /// The page of `fd`, where the kernel lets this process read the
    /// counter and its running time in user space.
    fn new(fd: &OwnedFd) -> Option<Mapped> {
        // Sound: one page of the event's mmap, read-only and shared; perf
        // keeps it while the descriptor lives, and Drop unmaps it first.
        let page = unsafe { mmap(std::ptr::null_mut(), PAGE_LEN, 1, 1, fd.as_raw_fd(), 0) };
        if page as isize == -1 {
            return None;
        }
        let mapped = Mapped(page as *const Page);
        // Sound: the page is mapped.
        let capabilities = unsafe { std::ptr::read_volatile(&(*mapped.0).capabilities) };
        (capabilities & CAP_USER_RDPMC != 0 && capabilities & CAP_USER_TIME != 0).then_some(mapped)
    }

    /// The count and the running time by rdpmc, as perf documents: under
    /// the page's seqlock, the index, offset and width, the counter
    /// register, and the running time with the TSC's advance since the
    /// kernel last wrote it. A counter not on this CPU (the other core
    /// kind's) has index 0: its count is its offset, its time stands.
    fn read(&self) -> (u64, u64) {
        // Sound: the page is mapped while self lives; rdpmc of an index the
        // page gives is allowed by its capabilities (checked in new).
        unsafe {
            let p = &*self.0;
            loop {
                let seq = std::ptr::read_volatile(&p.lock);
                core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
                let index = std::ptr::read_volatile(&p.index);
                let offset = std::ptr::read_volatile(&p.offset);
                let width = std::ptr::read_volatile(&p.pmc_width);
                let mut running = std::ptr::read_volatile(&p.time_running);
                let count = if index > 0 {
                    let (lo, hi): (u32, u32);
                    core::arch::asm!("rdpmc", in("ecx") index - 1, out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags));
                    let raw = (u64::from(hi) << 32) | u64::from(lo);
                    let cycles = core::arch::x86_64::_rdtsc();
                    let (time_shift, time_mult, time_offset) =
                        (std::ptr::read_volatile(&p.time_shift), std::ptr::read_volatile(&p.time_mult), std::ptr::read_volatile(&p.time_offset));
                    running = running.wrapping_add(tsc_to_ns(cycles, time_shift, time_mult, time_offset));
                    count_from_pmc(raw, width, offset)
                } else {
                    offset as u64
                };
                core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
                if std::ptr::read_volatile(&p.lock) == seq {
                    return (count, running);
                }
            }
        }
    }
}

/// The counter's value from its register: the register holds the low
/// `width` bits, sign-extended (perf's documented formula), added to the
/// page's offset.
fn count_from_pmc(raw: u64, width: u16, offset: i64) -> u64 {
    let shift = 64 - u32::from(width);
    offset.wrapping_add(((raw << shift) as i64) >> shift) as u64
}

/// TSC cycles as nanoseconds since the page's time base, as perf
/// documents: offset + quot * mult + (rem * mult) >> shift, the quotient
/// and remainder of the cycles by 2^shift.
fn tsc_to_ns(cycles: u64, shift: u16, mult: u32, offset: u64) -> u64 {
    let quot = cycles >> shift;
    let rem = cycles & ((1u64 << shift) - 1);
    offset.wrapping_add(quot * u64::from(mult) + ((rem * u64::from(mult)) >> shift))
}

impl Drop for Mapped {
    fn drop(&mut self) {
        // Sound: the page this mapped, unmapped once.
        unsafe { munmap(self.0 as *mut c_void, PAGE_LEN) };
    }
}

const SYS_PERF_EVENT_OPEN: c_long = 298;
const PERF_FLAG_FD_CLOEXEC: c_long = 8;

fn attr(pmu: u32, event: u64, leader: bool) -> Attr {
    Attr {
        // PERF_TYPE_HARDWARE, with the PMU in config's upper word.
        kind: 0,
        size: std::mem::size_of::<Attr>() as u32,
        config: (u64::from(pmu) << 32) | event,
        sample_period: 0,
        sample_type: 0,
        // PERF_FORMAT_TOTAL_TIME_ENABLED | _RUNNING | _GROUP.
        read_format: 1 | 2 | 8,
        // pinned (the leader: never multiplexed), exclude_kernel, exclude_hv.
        flags: (u64::from(leader) << 2) | (1 << 5) | (1 << 6),
        reserved: [0; 2],
    }
}

/// A counter of `event` on `pmu` for the calling thread, in `group` (or a
/// new group's leader, with -1).
fn open(pmu: u32, event: u64, group: c_int) -> Option<OwnedFd> {
    let a = attr(pmu, event, group == -1);
    // Sound: the x86-64 Linux system call, `a` an initialized attr of the
    // size it states; pid 0 and cpu -1 count the calling thread anywhere.
    let fd = unsafe { syscall(SYS_PERF_EVENT_OPEN, &a as *const Attr, 0 as c_long, -1 as c_long, c_long::from(group), PERF_FLAG_FD_CLOEXEC) };
    // Sound: a fresh descriptor, ours alone.
    (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(c_int::try_from(fd).unwrap()) })
}

/// The group's events, PERF_TYPE_HARDWARE's numbers: cycles leading, then
/// instructions, cache references and cache misses (the last two one
/// programmable counter each; the first two the fixed ones).
const EVENTS: [u64; 4] = [0, 1, 2, 3];

/// One core kind's group, and each counter's page where user space may
/// read them.
struct Group {
    fds: [OwnedFd; 4],
    pages: Option<[Mapped; 4]>,
}

impl Group {
    fn new(pmu: u32) -> Option<Group> {
        let leader = open(pmu, EVENTS[0], -1)?;
        let rest: Vec<OwnedFd> = EVENTS[1..].iter().map(|&event| open(pmu, event, leader.as_raw_fd())).collect::<Option<_>>()?;
        let [instructions, references, misses] = rest.try_into().ok()?;
        let fds = [leader, instructions, references, misses];
        let pages: Option<Vec<Mapped>> = fds.iter().map(Mapped::new).collect();
        let pages = pages.and_then(|pages| pages.try_into().ok());
        let group = Group { fds, pages };
        group.read_syscall()?;
        Some(group)
    }

    /// The group's counts so far: in user space where the pages allow,
    /// else by the system call.
    fn read(&self) -> Option<Level> {
        match &self.pages {
            Some([cycles, instructions, references, misses]) => {
                let (cycles, time_ns) = cycles.read();
                Some(Level { cycles, instructions: instructions.read().0, cache_references: references.read().0, cache_misses: misses.read().0, time_ns })
            }
            None => self.read_syscall(),
        }
    }

    /// The group's counts by the system call; `None` once a pinned group
    /// could not be scheduled (its reads then return nothing).
    fn read_syscall(&self) -> Option<Level> {
        // nr, time enabled, time running, then each event's count.
        let mut words = [0u64; 3 + EVENTS.len()];
        // Sound: `words` is writable for the size passed.
        let n = unsafe { read_fd(self.fds[0].as_raw_fd(), words.as_mut_ptr().cast(), std::mem::size_of_val(&words)) };
        if n == 0 {
            return None;
        }
        assert_eq!(n, std::mem::size_of_val(&words) as isize, "a perf group read gives its whole record");
        assert_eq!(words[0], EVENTS.len() as u64, "the group holds its four events");
        Some(Level { cycles: words[3], instructions: words[4], cache_references: words[5], cache_misses: words[6], time_ns: words[2] })
    }
}

/// Both hybrid PMU types, or generic hardware events when neither name
/// is present. A single named PMU deliberately remains unsupported.
/// Zero in config's upper word asks perf for its generic hardware event.
fn pmus() -> Option<(u32, Option<u32>)> {
    static PMUS: OnceLock<Option<(u32, Option<u32>)>> = OnceLock::new();
    *PMUS.get_or_init(|| pmus_at(std::path::Path::new("/sys/bus/event_source/devices")))
}

fn pmus_at(root: &std::path::Path) -> Option<(u32, Option<u32>)> {
    // Missing/unreadable sysfs is unavailable evidence about CPU kinds.
    if !root.try_exists().ok()? {
        return None;
    }
    let p = root.join("cpu_core").try_exists().ok()?;
    let e = root.join("cpu_atom").try_exists().ok()?;
    if !p && !e {
        return Some((0, None));
    }
    let kind = |name: &str| std::fs::read_to_string(root.join(name).join("type")).ok()?.trim().parse::<u32>().ok();
    Some((kind("cpu_core")?, Some(kind("cpu_atom")?)))
}

pub fn read() -> Option<Counts> {
    read_with(Group::read)
}

/// The thread's counts, each group through `read_group` (the user-space
/// read where allowed, or the system call for the test that compares them).
fn read_with(read_group: fn(&Group) -> Option<Level>) -> Option<Counts> {
    // Opened on the thread's first read; they count that thread alone and
    // close when it exits.
    std::thread_local! {
        static GROUPS: Option<(Group, Option<Group>)> = pmus().and_then(|(p, e)| {
            Some((Group::new(p)?, match e { Some(e) => Some(Group::new(e)?), None => None }))
        });
    }
    GROUPS.with(|groups| {
        let (p, e) = groups.as_ref()?;
        Some(Counts {
            p: read_group(p)?,
            e: match e {
                Some(e) => read_group(e)?,
                None => Level::default(),
            },
        })
    })
}

/// Whether this thread's groups read in user space.
#[cfg(test)]
fn reads_in_user_space() -> bool {
    read_with(|group| group.pages.as_ref().map(|_| Level::default())).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_attr_is_perf_event_attr_v0_counting_one_thread_unscaled() {
        assert_eq!(std::mem::size_of::<Attr>(), 64);
        let a = attr(8, 1, false);
        assert_eq!(a.config, (8u64 << 32) | 1);
        assert_eq!(attr(0, 0, true).config, 0);
        assert_eq!(attr(0, 1, false).config, 1);
        assert_eq!(a.read_format, 11);
        assert_eq!(a.flags, (1 << 5) | (1 << 6));
        assert_eq!(attr(8, 0, true).flags & (1 << 2), 1 << 2);
    }

    /// A 48-bit register's low bits, sign-extended and added to the offset:
    /// a value past 2^47 is a negative to perf (the kernel sets offset to
    /// make the sum right); a 40-bit register the same at 2^39.
    #[test]
    fn a_count_is_the_registers_low_bits_sign_extended_plus_the_offset() {
        assert_eq!(count_from_pmc(1000, 48, 500), 1500);
        assert_eq!(count_from_pmc(1 << 47, 48, 1 << 47), 0, "the sign bit set: -(2^47), which the offset cancels");
        assert_eq!(count_from_pmc((1 << 48) - 1, 48, 10), 9, "all ones: -1");
        assert_eq!(count_from_pmc(1 << 39, 40, 1 << 40), 1 << 39);
        assert_eq!(count_from_pmc(0xFFFF_0000_0000_0007, 48, 0), 7, "bits above the width are not the register's");
    }

    /// Perf's TSC scaling: at shift 10 and mult 1024 a cycle is a
    /// nanosecond; the remainder's share is kept.
    #[test]
    fn tsc_cycles_scale_to_nanoseconds_by_perfs_formula() {
        assert_eq!(tsc_to_ns(5_000, 10, 1024, 7), 5_007);
        assert_eq!(tsc_to_ns(5_000, 10, 512, 0), 2_500, "a 2 GHz TSC: half a nanosecond a cycle");
        assert_eq!(tsc_to_ns(1023, 10, 3, 0), (1023 * 3) >> 10, "under one quotient step: the remainder's share alone");
        assert_eq!(tsc_to_ns(3 << 10, 10, 3, 100), 109);
    }

    /// The PMUs the thread's groups open are this machine's, read once.
    #[test]
    fn the_pmus_are_this_machines() {
        assert_eq!(pmus(), pmus_at(std::path::Path::new("/sys/bus/event_source/devices")));
        assert_eq!(pmus(), pmus(), "read once, the same after");
    }

    #[test]
    fn pmu_selection_requires_a_complete_readable_hybrid_pair() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
        std::fs::create_dir_all(&base).unwrap();
        let root = base.join(format!("pmu-test-{}", std::process::id()));
        assert_eq!(pmus_at(&root), None, "missing sysfs");
        std::fs::create_dir(&root).unwrap();
        assert_eq!(pmus_at(&root), Some((0, None)), "homogeneous PMU");
        for (first, second) in [("cpu_core", "cpu_atom"), ("cpu_atom", "cpu_core")] {
            std::fs::create_dir(root.join(first)).unwrap();
            std::fs::write(root.join(first).join("type"), "8\n").unwrap();
            assert_eq!(pmus_at(&root), None, "one hybrid PMU is insufficient");
            std::fs::create_dir(root.join(second)).unwrap();
            let path = root.join(second).join("type");
            assert_eq!(pmus_at(&root), None, "missing type");
            std::fs::write(&path, "invalid\n").unwrap();
            assert_eq!(pmus_at(&root), None, "malformed type");
            std::fs::write(&path, "9\n").unwrap();
            let expected = if first == "cpu_core" { (8, Some(9)) } else { (9, Some(8)) };
            assert_eq!(pmus_at(&root), Some(expected));
            std::fs::remove_dir_all(root.join(first)).unwrap();
            std::fs::remove_dir_all(root.join(second)).unwrap();
        }
        std::fs::remove_dir(&root).unwrap();
    }

    #[test]
    #[ignore = "needs a homogeneous x86 CPU with perf access"]
    fn homogeneous_counts_cover_work() {
        assert_eq!(pmus(), Some((0, None)));
        for b in crate::measure(4, 2_000_000, || crate::busy_work(10_000)) {
            let c = b.counts.expect("generic hardware PMU readable here");
            assert!(c.p.cycles > 0 && c.p.instructions > 0 && c.p.time_ns > 0);
            assert_eq!(c.e, Level::default());
            println!("{}; {c:?}", b.show());
        }
    }

    /// On a hybrid CPU with perf allowed: run with the thread pinned to one
    /// kind, CLOCKS_EXPECT_KIND=P (taskset -c 0) or E (taskset -c 16).
    #[test]
    #[ignore = "needs Intel hybrid PMUs, perf allowed, and a declared pinning"]
    fn counts_fall_on_the_kind_that_ran() {
        let expected = std::env::var("CLOCKS_EXPECT_KIND").expect("CLOCKS_EXPECT_KIND=P or E");
        for b in crate::measure(4, 2_000_000, || crate::busy_work(10_000)) {
            let c = b.counts.expect("hybrid PMUs readable here");
            let (ran, idle) = if expected == "P" { (c.p, c.e) } else { (c.e, c.p) };
            assert!(ran.cycles > 0 && ran.instructions > 0 && ran.time_ns > 0);
            assert_eq!(idle, Level::default(), "the other kind counted nothing");
            println!("{}", b.show());
        }
    }

    /// The machine's properties as this crate relies on them: timing, the
    /// OS's counters, children. Seconds each and sensitive to other load,
    /// so they run on purpose (`cargo nextest run -P machine`) before a
    /// release and on a new machine, and not beside every change or mutant.
    mod machine {
        use super::*;

        /// Where the kernel lets a user open its CPU's counters (a PMU in
        /// sysfs and perf_event_paranoid at most 2, or root), this thread
        /// counts: cycles and instructions advance over work, on one kind
        /// of core, with running time; and where the kernel allows rdpmc
        /// (`cpu/rdpmc` or `cpu_core/rdpmc` reads 1), the read is in user
        /// space. A machine that offers counts and gets none is a bug.
        #[test]
        fn the_counts_the_machine_offers_are_read() {
            let paranoid: i64 = std::fs::read_to_string("/proc/sys/kernel/perf_event_paranoid").map(|s| s.trim().parse().unwrap_or(4)).unwrap_or(4);
            let root = std::fs::read_to_string("/proc/self/status").map_or(false, |s| s.lines().any(|l| l.starts_with("Uid:\t0\t")));
            if pmus().is_none() || (paranoid > 2 && !root) {
                eprintln!("no counters offered here (pmus {:?}, perf_event_paranoid {paranoid}); nothing to require", pmus());
                return;
            }
            let before = Counts::read().expect("the machine offers counters: the thread counts");
            crate::busy_work(2_000_000);
            let after = Counts::read().unwrap().since(before);
            let ran = if after.p.time_ns > 0 { after.p } else { after.e };
            assert!(ran.cycles > 1_000_000 && ran.instructions > 100_000 && ran.time_ns > 1_000_000, "2 ms of work counted: {after:?}");
            let allowed = ["cpu", "cpu_core"]
                .iter()
                .any(|pmu| std::fs::read_to_string(format!("/sys/bus/event_source/devices/{pmu}/rdpmc")).map_or(false, |s| s.trim() == "1"));
            if allowed {
                assert!(reads_in_user_space(), "the kernel allows rdpmc here: the read is in user space");
            }
        }

        /// The user-space read agrees with the system call's over the same
        /// work: each count within the bracketing reads' own cost, the running
        /// time within a few microseconds; both kinds' groups together, so a
        /// thread that moves between an i7's core kinds still counts. Where
        /// the kernel allows no user-space read (a VM without one), the
        /// groups read by the system call and the test has nothing to compare.
        #[test]
        fn rdpmc_agrees_with_the_system_call() {
            if read().is_none() || !reads_in_user_space() {
                eprintln!("no user-space read here; the system call serves");
                return;
            }
            const NAMES: [&str; 5] = ["cycles", "instructions", "cache references", "cache misses", "time ns"];
            let total = |c: Counts| {
                let sum = |f: fn(&Level) -> u64| f(&c.p) + f(&c.e);
                [sum(|l| l.cycles), sum(|l| l.instructions), sum(|l| l.cache_references), sum(|l| l.cache_misses), sum(|l| l.time_ns)]
            };
            let by_syscall = || total(read_with(Group::read_syscall).unwrap());
            let by_rdpmc = || total(read().unwrap());
            // 4 MiB strided at cache lines: work with misses in it.
            let buffer = vec![1u8; 4 << 20];
            // The system call brackets the user-space read, so its counts are
            // the larger by the reads' own work: measured on empty passes
            // (thousands of instructions in a debug build, hundreds in
            // release), and taken off each working pass's difference.
            let bracket = |work: &dyn Fn()| -> ([i128; 5], [i128; 5]) {
                let (s0, u0) = (by_syscall(), by_rdpmc());
                work();
                let (u1, s1) = (by_rdpmc(), by_syscall());
                (std::array::from_fn(|k| s1[k] as i128 - s0[k] as i128), std::array::from_fn(|k| u1[k] as i128 - u0[k] as i128))
            };
            // The median: a thread preempted between two bracketing reads (the
            // test suite runs beside) widens one pass's by the switch, which
            // says nothing about the reads.
            let median = |rows: &[[i128; 5]]| -> [i128; 5] {
                std::array::from_fn(|k| {
                    let mut column: Vec<i128> = rows.iter().map(|d| d[k]).collect();
                    column.sort_unstable();
                    column[column.len() / 2]
                })
            };
            let empty: Vec<[i128; 5]> = (0..200)
                .map(|_| {
                    let (by_s, by_u) = bracket(&|| {});
                    std::array::from_fn(|k| by_s[k] - by_u[k])
                })
                .collect();
            let own = median(&empty);
            let mut differences: Vec<[i128; 5]> = Vec::new();
            let mut first = None;
            for _ in 0..200 {
                let (by_s, by_u) = bracket(&|| {
                    let mut sum = 0u64;
                    for i in (0..buffer.len()).step_by(64) {
                        sum += u64::from(std::hint::black_box(&buffer)[i]);
                    }
                    std::hint::black_box(sum);
                });
                if first.is_none() {
                    first = Some((by_s, by_u));
                }
                differences.push(std::array::from_fn(|k| (by_s[k] - by_u[k] - own[k]).abs()));
            }
            let median = median(&differences);
            let (by_s, by_u) = first.unwrap();
            eprintln!(
                "one pass: by read() {by_s:?}, by rdpmc {by_u:?} ({NAMES:?}); the reads' own bracket {own:?}; median differences beyond it over 200: {median:?}"
            );
            assert!(by_u[0] > 10_000 && by_u[1] > 10_000 && by_u[2] > 1_000 && by_u[4] > 0, "the user-space read counts the work: {by_u:?}");
            assert!(
                median[0] < 5_000 && median[1] < 2_000 && median[2] < 500 && median[3] < 500,
                "the counts agree beyond the reads' own work: {median:?} ({NAMES:?})"
            );
            assert!(median[4] < 20_000, "the running time agrees beyond the reads' own time: {median:?}");
        }
    }
}
