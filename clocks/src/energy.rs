//! The energy the CPU package has used: **a lab instrument**, for
//! experiments and probes, never the benchmark, its reports, or the gate
//! (Zooko, October 17, 2026: a number in every report becomes a goal, and
//! the benchmark carries wall time and CPU time alone). It counts what CPU
//! time cannot: the power a measurement's threads drew, and what they cost
//! the rest of the package (a clock its cores share).
//!
//! **What it counts.** Linux: the `power` PMU's `energy-pkg` event (Intel
//! and AMD RAPL), read through `perf_event_open` for the whole package,
//! which needs `perf_event_paranoid` at 0 or below (or CAP_PERFMON). It is
//! the package's, every program's: a measurement's share is valid on a
//! quiet machine, which [`crate::load`] checks. The counter's unit is
//! the scale sysfs gives (2^-32 J on Intel, 2^-16 J on AMD), read as an
//! exact decimal, so nanojoules come out exactly (integer arithmetic; the
//! scale parsing from #48). Elsewhere, or without the permission,
//! [`package_nj`] is `None`, and results say so.
//!
//! **Resolution.** RAPL updates about once a millisecond, so a sample of a
//! millisecond reads within about one update's energy; an experiment
//! summing many samples' energy averages that out.

use std::sync::OnceLock;

/// The package's energy since the counter opened, in nanojoules (it only
/// counts up while the process lives): `None` where the platform gives no
/// package energy or this process may not read it. A difference of two
/// readings is the energy between them.
pub fn package_nj() -> Option<u64> {
    static COUNTER: OnceLock<Option<imp::Counter>> = OnceLock::new();
    COUNTER.get_or_init(imp::Counter::open).as_ref().map(imp::Counter::read_nj)
}

#[cfg(target_os = "linux")]
mod imp {
    /// The `energy-pkg` event's counter, open on one CPU of the package,
    /// and its unit as an exact decimal: count times `mantissa` times
    /// 10^`exponent` joules.
    pub struct Counter {
        /// The counter's descriptor, read through std (no foreign `read`
        /// of our own, so none to disagree with another module's).
        file: std::fs::File,
        mantissa: u128,
        exponent: i32,
    }

    /// "2.3283064365386962890625e-10" as (23283064365386962890625, -32):
    /// the scale perf gives, exactly. Requires decimal digits, one point at
    /// most, and an optional exponent.
    pub fn parse_scale(text: &str) -> Option<(u128, i32)> {
        let text = text.trim();
        let (digits, exponent) = match text.split_once(['e', 'E']) {
            Some((d, e)) => (d, e.parse::<i32>().ok()?),
            None => (text, 0),
        };
        let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
        let mantissa = format!("{whole}{fraction}").parse::<u128>().ok()?;
        Some((mantissa, exponent - fraction.len() as i32))
    }

    unsafe extern "C" {
        fn syscall(number: i64, ...) -> i64;
    }

    #[cfg(target_arch = "x86_64")]
    const SYS_PERF_EVENT_OPEN: i64 = 298;
    #[cfg(target_arch = "aarch64")]
    const SYS_PERF_EVENT_OPEN: i64 = 241;
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    const SYS_PERF_EVENT_OPEN: i64 = -1;

    fn sysfs(name: &str) -> Option<String> {
        std::fs::read_to_string(format!("/sys/bus/event_source/devices/power/{name}")).ok().map(|s| s.trim().to_owned())
    }

    impl Counter {
        pub fn open() -> Option<Counter> {
            if SYS_PERF_EVENT_OPEN < 0 {
                return None;
            }
            let pmu: u64 = sysfs("type")?.parse().ok()?;
            let config = u64::from_str_radix(sysfs("events/energy-pkg")?.strip_prefix("event=0x")?, 16).ok()?;
            let (mantissa, exponent) = parse_scale(&sysfs("events/energy-pkg.scale")?)?;
            let cpu: i64 = sysfs("cpumask")?.split([',', '-']).next()?.parse().ok()?;
            // struct perf_event_attr, as 64-bit words: type and size, then
            // config; everything else zero (counting, enabled at open).
            let mut attr = [0u64; 16];
            attr[0] = pmu | ((attr.len() as u64 * 8) << 32);
            attr[1] = config;
            // Sound: `attr` is a perf_event_attr of the size it states; pid
            // -1 with a CPU counts that CPU's package, every process.
            let fd = unsafe { syscall(SYS_PERF_EVENT_OPEN, attr.as_ptr(), -1i64, cpu, -1i64, 0u64) };
            // Sound: a descriptor perf_event_open just returned, ours alone.
            (fd >= 0).then(|| Counter { file: unsafe { std::os::fd::FromRawFd::from_raw_fd(fd as i32) }, mantissa, exponent })
        }

        pub fn read_nj(&self) -> u64 {
            let mut count = [0u8; 8];
            std::io::Read::read_exact(&mut &self.file, &mut count).expect("reading the package energy counter");
            super::scale_nj(u64::from_ne_bytes(count), self.mantissa, self.exponent)
        }
    }
}

/// A counter's `count` in nanojoules: count x mantissa x 10^(exponent + 9),
/// rounded down (the scale "2.3283064365386962890625e-10" J a count, as
/// parse_scale reads it, is mantissa 23283064365386962890625 at exponent -32).
#[cfg(any(target_os = "linux", test))]
fn scale_nj(count: u64, mantissa: u128, exponent: i32) -> u64 {
    let scaled = u128::from(count) * mantissa;
    let exponent = exponent + 9;
    let nj = if exponent >= 0 { scaled * 10u128.pow(exponent as u32) } else { scaled / 10u128.pow(exponent.unsigned_abs()) };
    u64::try_from(nj).expect("energy fits in u64")
}

#[cfg(not(target_os = "linux"))]
mod imp {
    pub struct Counter;

    impl Counter {
        pub fn open() -> Option<Counter> {
            None
        }

        pub fn read_nj(&self) -> u64 {
            unreachable!("no counter opens here")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Intel's and AMD's scales, exactly (#48's cases).
    #[cfg(target_os = "linux")]
    #[test]
    fn scales_parse_exactly() {
        assert_eq!(imp::parse_scale("2.3283064365386962890625e-10\n"), Some((23283064365386962890625, -32)));
        assert_eq!(imp::parse_scale("1.52587890625e-5"), Some((152587890625, -16)));
        assert_eq!(imp::parse_scale("1"), Some((1, 0)));
        assert_eq!(imp::parse_scale("joules"), None);
    }

    /// A count scaled to nanojoules, exactly: Intel's 2^-32 J a count makes
    /// 2^32 counts one joule, AMD's 2^-16 J makes 65536 counts one, and a
    /// whole-joule scale multiplies by 10^9.
    #[test]
    fn counts_scale_to_nanojoules_exactly() {
        assert_eq!(scale_nj(1 << 32, 23283064365386962890625, -32), 1_000_000_000);
        assert_eq!(scale_nj(3, 23283064365386962890625, -32), 0, "under a nanojoule rounds down");
        assert_eq!(scale_nj(65536, 152587890625, -16), 1_000_000_000);
        assert_eq!(scale_nj(7, 1, 0), 7_000_000_000);
    }

    /// The machine's properties as this crate relies on them: timing, the
    /// OS's counters, children. Seconds each and sensitive to other load,
    /// so they run on purpose (`cargo nextest run -P machine`) before a
    /// release and on a new machine, and not beside every change or mutant.
    mod machine {
        use super::*;

        /// Where the package energy can be read, it counts up, and a second of
        /// busy work costs the package between 0.1 and 500 J (a desktop's
        /// package draws a few watts idle and a few hundred at most).
        #[test]
        fn package_energy_counts_up_by_a_plausible_amount() {
            // Offered: the power PMU's energy-pkg event in sysfs, and the
            // kernel letting a user open a CPU's event (perf_event_paranoid
            // at most 0, or root).
            let offered = std::path::Path::new("/sys/bus/event_source/devices/power/events/energy-pkg").exists()
                && (std::fs::read_to_string("/proc/sys/kernel/perf_event_paranoid").map(|s| s.trim().parse::<i64>().unwrap_or(4)).unwrap_or(4) <= 0
                    || std::fs::read_to_string("/proc/self/status").map_or(false, |s| s.lines().any(|l| l.starts_with("Uid:\t0\t"))));
            let Some(before) = package_nj() else {
                assert!(!offered, "the machine offers the package energy counter: it is read");
                eprintln!("no package energy here; nothing to check");
                return;
            };
            crate::busy_work(1_000_000_000);
            let millijoules = (package_nj().unwrap() - before) / 1_000_000;
            eprintln!("package energy over a busy second: {millijoules} mJ");
            assert!((100..=500_000).contains(&millijoules), "{millijoules} mJ over a busy second");
        }
    }
}
