//! The thread's diagnostic events: one set, named the same on every
//! platform, for the question "cycles moved and instructions did not: which
//! event moved with them?" (bench-hashes #160). The routine counts
//! ([`crate::Counts`]) stay as they are, the cheapest read each platform
//! offers. These are for diagnosis around diluted work: many calls, or
//! several prepared sets, inside one pair of reads.
//!
//! The set, after the Mac's (John Servil's `events_sampler`, Mac job 2073),
//! and what each is on each platform:
//!
//! | name | Apple M4 (kpc, root) | Intel P core (`cpu_core`) | Intel E core (`cpu_atom`) | other Linux x86-64 |
//! |---|---|---|---|---|
//! | `cycles` | the fixed cycles counter | generic cycles | generic cycles | generic cycles |
//! | `instructions` | the fixed instructions counter | generic instructions | generic instructions | generic instructions |
//! | `l1d_load_misses` | L1D_CACHE_MISS_LD | L1-dcache-load-misses | MEM_LOAD_UOPS_RETIRED.L1_MISS (retired loads) | L1-dcache-load-misses |
//! | `l1i_misses` | L1I_CACHE_MISS_DEMAND | L1-icache-load-misses | L1-icache-load-misses | L1-icache-load-misses |
//! | `dtlb_load_misses` | L1D_TLB_MISS | dTLB-load-misses | dTLB-load-misses | dTLB-load-misses |
//! | `backend_stalls` | MAP_STALL (cycles) | CYCLE_ACTIVITY.STALLS_TOTAL (cycles) | TOPDOWN_BE_BOUND.ALL (issue slots, 5 a cycle) | stalled-cycles-backend (cycles) |
//! | `branch_misses` | a branch-mispredict event (not yet in the Mac's set) | branch-misses | branch-misses | branch-misses |
//!
//! An event a platform or kernel does not give reads `None`: "not
//! measured", never zero. Linux counts user space alone, on the calling
//! thread, in one pinned group a core kind (never multiplexed: a group
//! the PMU cannot hold reads `None` whole). This implementation is
//! Linux's; the Mac's (kpc, root only) lives in BLAKE3-Servil's
//! `tools/studies/events_sampler.rs` and reads the same names.
use std::ffi::{c_int, c_long, c_void};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

/// The events, in the order of every record of them.
pub const NAMES: [&str; 7] = ["cycles", "instructions", "l1d_load_misses", "l1i_misses", "dtlb_load_misses", "backend_stalls", "branch_misses"];

/// One core kind's events, and the time its group ran.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Level {
    pub time_ns: u64,
    pub values: [Option<u64>; NAMES.len()],
}

/// The thread's events so far on performance cores (`p`) and efficiency
/// cores (`e`); a homogeneous CPU's are in `p`, with `e` all `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Events {
    pub p: Level,
    pub e: Level,
}

impl Events {
    /// The calling thread's events, or `None` where the platform gives none.
    pub fn read() -> Option<Events> {
        imp::read()
    }

    /// The events since `earlier`, a read on the same thread (as
    /// `Counts::since`), each event where both reads have it.
    pub fn since(self, earlier: Events) -> Events {
        let level =
            |now: Level, then: Level| Level { time_ns: now.time_ns - then.time_ns, values: std::array::from_fn(|i| Some(now.values[i]? - then.values[i]?)) };
        Events { p: level(self.p, earlier.p), e: level(self.e, earlier.e) }
    }

    /// The two core kinds together: each event where both measured it, or
    /// where one measured it and the other kind did not run (no time).
    pub fn total(self) -> Level {
        Level {
            time_ns: self.p.time_ns + self.e.time_ns,
            values: std::array::from_fn(|i| match (self.p.values[i], self.e.values[i]) {
                (Some(a), Some(b)) => Some(a + b),
                (Some(a), None) if self.e.time_ns == 0 => Some(a),
                (None, Some(b)) if self.p.time_ns == 0 => Some(b),
                _ => None,
            }),
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod imp {
    use super::*;
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
    }

    const SYS_PERF_EVENT_OPEN: c_long = 298;
    const PERF_FLAG_FD_CLOEXEC: c_long = 8;
    const HARDWARE: u32 = 0;
    const HW_CACHE: u32 = 3;
    /// HW_CACHE's config: cache | op << 8 | result << 16.
    const fn cache_miss(cache: u64) -> u64 {
        cache | (0 << 8) | (1 << 16) // OP_READ, RESULT_MISS
    }

    /// An event: perf's type and config (a hybrid PMU's type goes in the
    /// upper word of a generic event's config, or is the raw event's type).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) struct Event {
        pub kind: u32,
        pub config: u64,
    }

    /// The set on a PMU: `pmu` is a hybrid PMU's type, or None for the
    /// generic hardware events of a homogeneous CPU; `atom` says which
    /// hybrid PMU (their raw stall events differ).
    pub(super) fn set(pmu: Option<u32>, atom: bool) -> [Option<Event>; NAMES.len()] {
        let ext = |config: u64| config | (u64::from(pmu.unwrap_or(0)) << 32);
        let generic = |config: u64| Some(Event { kind: HARDWARE, config: ext(config) });
        let cache = |id: u64| Some(Event { kind: HW_CACHE, config: ext(cache_miss(id)) });
        let stalls = match pmu {
            // CYCLE_ACTIVITY.STALLS_TOTAL: event 0xa3, umask 0x04, cmask 4.
            Some(pmu) if !atom => Some(Event { kind: pmu, config: 0xa3 | (0x04 << 8) | (4 << 24) }),
            // TOPDOWN_BE_BOUND.ALL: event 0x74, umask 0: issue slots.
            Some(pmu) => Some(Event { kind: pmu, config: 0x74 }),
            // PERF_COUNT_HW_STALLED_CYCLES_BACKEND, where the kernel gives it.
            None => generic(8),
        };
        // The E PMU has no generic L1D read-miss event (perf: not
        // supported): MEM_LOAD_UOPS_RETIRED.L1_MISS (0xd1, umask 0x08),
        // retired loads that missed.
        let l1d = match pmu {
            Some(pmu) if atom => Some(Event { kind: pmu, config: 0xd1 | (0x08 << 8) }),
            _ => cache(0),
        };
        [generic(0), generic(1), l1d, cache(1), cache(3), stalls, generic(5)]
    }

    fn attr(event: Event, leader: bool) -> Attr {
        Attr {
            kind: event.kind,
            size: std::mem::size_of::<Attr>() as u32,
            config: event.config,
            sample_period: 0,
            sample_type: 0,
            // PERF_FORMAT_TOTAL_TIME_ENABLED | _RUNNING | _GROUP.
            read_format: 1 | 2 | 8,
            // pinned (the leader), exclude_kernel, exclude_hv.
            flags: (u64::from(leader) << 2) | (1 << 5) | (1 << 6),
            reserved: [0; 2],
        }
    }

    fn open(event: Event, group: c_int) -> Option<OwnedFd> {
        let a = attr(event, group == -1);
        // Sound: the x86-64 Linux system call, `a` an initialized attr of the
        // size it states; pid 0 and cpu -1 count the calling thread anywhere.
        let fd = unsafe { syscall(SYS_PERF_EVENT_OPEN, &a as *const Attr, 0 as c_long, -1 as c_long, c_long::from(group), PERF_FLAG_FD_CLOEXEC) };
        // Sound: a fresh descriptor, ours alone.
        (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(c_int::try_from(fd).unwrap()) })
    }

    /// One core kind's group: its leader (cycles), and which of the set's
    /// events it holds, in its read order.
    pub(super) struct Group {
        fds: Vec<OwnedFd>,
        held: Vec<usize>,
    }

    impl Group {
        fn new(set: [Option<Event>; NAMES.len()]) -> Option<Group> {
            Self::new_from(&set)
        }

        /// The events' group: `events[0]` is the leader; a member that will
        /// not open is left out. Every member opens alone, yet the pinned
        /// group must also fit the PMU's counters together: on a Zen 5, six
        /// fit and the seventh put the leader in error, so every read gave
        /// nothing and no event was counted (ops #10). A pinned leader the
        /// PMU once failed to schedule stays in error, so each narrower try
        /// is a fresh group: the last member goes, until what is left
        /// schedules. Each kept event's count stays exact, and a dropped one
        /// reads None as an event the PMU lacks does.
        pub(super) fn new_from(events: &[Option<Event>]) -> Option<Group> {
            let mut wanted: Vec<usize> = (0..events.len()).filter(|&i| events[i].is_some()).collect();
            if wanted.first() != Some(&0) {
                return None;
            }
            while !wanted.is_empty() {
                if let Some(group) = Self::of(events, &mut wanted) {
                    return Some(group);
                }
                wanted.pop();
            }
            None
        }

        /// One try at `wanted`'s events as a fresh group. A member that will
        /// not open leaves `wanted`, so a narrower retry drops a member that
        /// opened instead of asking again for one that will not.
        fn of(events: &[Option<Event>], wanted: &mut Vec<usize>) -> Option<Group> {
            let leader = open(events[wanted[0]]?, -1)?;
            let mut fds = vec![leader];
            let mut held = vec![wanted[0]];
            for &i in &wanted[1..] {
                if let Some(fd) = events[i].and_then(|e| open(e, fds[0].as_raw_fd())) {
                    fds.push(fd);
                    held.push(i);
                }
            }
            *wanted = held.clone();
            let group = Group { fds, held };
            group.counts()?;
            Some(group)
        }

        pub(super) fn len(&self) -> usize {
            self.held.len()
        }

        /// The group's running time and each held event's count, by its
        /// place in the events the group was made from; `None` where the
        /// PMU could not schedule the pinned group whole.
        pub(super) fn counts(&self) -> Option<(u64, Vec<(usize, u64)>)> {
            // nr, time enabled, time running, then each event's count.
            let mut words = vec![0u64; 3 + self.held.len()];
            // Sound: `words` is writable for the size passed.
            let n = unsafe { read_fd(self.fds[0].as_raw_fd(), words.as_mut_ptr().cast(), words.len() * 8) };
            // A pinned group the PMU could not hold reads nothing (perf puts
            // it in error); a group that has not run (the other core kind's)
            // reads its counts with no running time.
            if n <= 0 {
                return None;
            }
            assert_eq!(n as usize, words.len() * 8, "a perf group read gives its whole record");
            Some((words[2], self.held.iter().enumerate().map(|(k, &i)| (i, words[3 + k])).collect()))
        }

        fn read(&self) -> Option<Level> {
            let (time_ns, counts) = self.counts()?;
            let mut values = [None; NAMES.len()];
            for (i, count) in counts {
                values[i] = Some(count);
            }
            Some(Level { time_ns, values })
        }
    }

    /// The hybrid PMUs' types, or None for a homogeneous CPU's generic
    /// events; the same rule as the routine counts (linux_counts).
    fn pmus() -> Option<Option<(u32, u32)>> {
        static PMUS: OnceLock<Option<Option<(u32, u32)>>> = OnceLock::new();
        *PMUS.get_or_init(|| {
            let root = std::path::Path::new("/sys/bus/event_source/devices");
            if !root.try_exists().ok()? {
                return None;
            }
            let (p, e) = (root.join("cpu_core").try_exists().ok()?, root.join("cpu_atom").try_exists().ok()?);
            if !p && !e {
                return Some(None);
            }
            let kind = |name: &str| std::fs::read_to_string(root.join(name).join("type")).ok()?.trim().parse::<u32>().ok();
            Some(Some((kind("cpu_core")?, kind("cpu_atom")?)))
        })
    }

    pub fn read() -> Option<Events> {
        std::thread_local! {
            static GROUPS: Option<(Group, Option<Group>)> = pmus().and_then(|pmus| match pmus {
                Some((p, e)) => Some((Group::new(set(Some(p), false))?, Some(Group::new(set(Some(e), true))?))),
                None => Some((Group::new(set(None, false))?, None)),
            });
        }
        GROUPS.with(|groups| {
            let (p, e) = groups.as_ref()?;
            Some(Events {
                p: p.read()?,
                e: match e {
                    Some(e) => e.read()?,
                    None => Level::default(),
                },
            })
        })
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
mod imp {
    pub fn read() -> Option<super::Events> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn since_subtracts_each_event_and_keeps_none() {
        let level = |k: u64| Level { time_ns: 10 * k, values: [Some(k), Some(2 * k), None, Some(3 * k), Some(k), Some(k), Some(k)] };
        let d = Events { p: level(5), e: level(7) }.since(Events { p: level(1), e: level(2) });
        assert_eq!(d.p.time_ns, 40);
        assert_eq!(d.p.values, [Some(4), Some(8), None, Some(12), Some(4), Some(4), Some(4)]);
        assert_eq!(d.e.values[0], Some(5));
    }

    #[test]
    fn total_adds_the_core_kinds_and_keeps_a_homogeneous_cpus_p() {
        let p = Level { time_ns: 5, values: [Some(1); NAMES.len()] };
        let mut e = Level { time_ns: 7, values: [Some(2); NAMES.len()] };
        e.values[5] = None;
        let t = Events { p, e }.total();
        assert_eq!((t.time_ns, t.values[0], t.values[5]), (12, Some(3), None), "a kind that ran without measuring an event leaves it unmeasured");
        assert_eq!(Events { p, e: Level::default() }.total().values[5], Some(1), "a homogeneous CPU's events are its p's");
        assert_eq!(Events { p: Level { time_ns: 0, ..p }, e }.total().values[5], None, "the kind that ran measured nothing");
        assert_eq!(Events { p, e: Level { time_ns: 0, ..e } }.total().values[5], Some(1), "a kind that did not run adds nothing");
    }

    /// A pinned group that cannot fit the PMU's counters whole keeps its
    /// leading members: the tail goes until what is left reads, and the
    /// kept events still count (this machine's own group, grown past what
    /// any PMU schedules: 64 extra instruction counters).
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn an_unschedulable_group_keeps_its_leading_members() {
        if imp::read().is_none() {
            eprintln!("no perf events here (a container without perf_event_open): nothing to shrink");
            return;
        }
        let instructions = imp::Event { kind: 0, config: 1 };
        let wide: Vec<Option<imp::Event>> =
            std::iter::once(Some(imp::Event { kind: 0, config: 0 })).chain(std::iter::repeat_n(Some(instructions), 64)).collect();
        let group = imp::Group::new_from(&wide).expect("the leader alone schedules");
        assert!(group.len() < 65, "a PMU holds fewer than 65 counters at once");
        let (time_ns, counts) = group.counts().expect("the kept group reads");
        let _ = time_ns;
        assert_eq!(counts.len(), group.len());
        assert_eq!(counts[0].0, 0, "the leader stays");
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn the_hybrid_sets_name_the_documented_events() {
        let p = imp::set(Some(8), false);
        assert_eq!(p[0].unwrap().config, 8 << 32, "cycles: generic 0 on the P PMU");
        assert_eq!(p[2].unwrap().config, (8 << 32) | (1 << 16), "L1D read misses: cache 0, op read, result miss");
        assert_eq!(p[4].unwrap().config, (8 << 32) | 3 | (1 << 16), "dTLB read misses: cache 3");
        assert_eq!((p[5].unwrap().kind, p[5].unwrap().config), (8, 0x0400_04a3), "STALLS_TOTAL on the P PMU, raw");
        let e = imp::set(Some(10), true);
        assert_eq!((e[5].unwrap().kind, e[5].unwrap().config), (10, 0x74), "TOPDOWN_BE_BOUND.ALL on the E PMU, raw");
        assert_eq!(imp::set(None, false)[5].unwrap().config, 8, "stalled-cycles-backend, generic");
    }
}
