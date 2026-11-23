//! The pool's park and wake: which workers are parked, how a worker parks
//! without missing work published as it does, and how a publisher wakes
//! one. The place a wake can be lost, so the place a model checker runs
//! every schedule it can: `tests/shuttle_park.rs` compiles this same file
//! beside Shuttle's primitives (a `park_sync` module on each side, so no
//! copy drifts), where the library's `park_sync` is the standard library's.
//! Nothing here names the rest of the crate.
//!
//! The handshake is Dekker's: a worker sets its parked bit, then looks once
//! more for work, then parks; a publisher publishes, then looks at the
//! parked bits and wakes one. Work publication and the visibility re-check
//! may use Release and Acquire. The two sides place a SeqCst fence between
//! publishing and checking, so one sees the other: the worker sees work
//! and does not park, or a publisher that must wake a worker sees its bit.
//! Shuttle checks the handshake under SC, not the weaker load order on Arm.

use super::park_sync::{AtomicU64, OnceLock, Ordering, fence, thread};

const OWED_COUNT_MASK: u64 = u32::MAX as u64;

/// Any value on a cache line of its own.
#[repr(align(128))]
pub(crate) struct Line<T>(pub(crate) T);

impl<T> std::ops::Deref for Line<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

pub(crate) struct Park {
    /// Workers parked: bit w of word w / 64 for worker w. A worker sets its
    /// own bit before it parks; a waker clears one (fetch_and), which makes
    /// it the one to unpark that worker. Each worker parks on its own
    /// thread's token (std's park), so no two share a lock or a futex word.
    parked: Box<[Line<AtomicU64>]>,
    /// Each worker's thread, set as it starts.
    handles: Box<[OnceLock<thread::Thread>]>,
    /// Wakes still owed, packed as `(generation: u32) << 32 | (count: u32)`:
    /// each woken worker pays up to `fanout` of `count` before it works.
    /// Every publisher addition increments `generation` so an empty-scan
    /// clear from an earlier generation cannot wipe out freshly published
    /// debt via ABA.
    owed: AtomicU64,
    /// The most wakes any one waker makes (FANOUT; a test may choose).
    fanout: usize,
    /// Set once the workers have places (machines with more than one
    /// last-level cache): for each place, the workers there as bits like
    /// `parked`'s, and the place of the thread that asks. A waker wakes a
    /// worker of its own place first.
    near: OnceLock<(Box<[Box<[u64]>]>, Box<[Box<[u64]>]>, fn() -> usize)>,
}

/// The most wakes any one waker makes, publisher or woken worker: the rest
/// stay owed for the workers it woke, so a job's wakes spread as a tree,
/// FANOUT more a hop, and no thread makes more than FANOUT system calls
/// before its own work (BLAKE3 #119). Every wake direct put up to 47 calls
/// before the publisher's first piece on the EPYC 9255 (14 pool cells
/// 4-12% slower); one wake a hop made each worker wait for the one before
/// (the i7-12700K's 1-2 MiB one-shots 16-25% slower); a publisher of 8 with
/// workers paying the rest unbounded let one worker make every owed wake
/// in a quarter of the i7's 2 MiB jobs (data 2026-10-25/wake-counts).
pub(crate) const FANOUT: usize = 8;

/// A waker's share of `need` wakes: the ones it makes now, at most
/// `fanout`, and the ones it leaves owed.
pub(crate) fn split(need: usize, fanout: usize) -> (usize, usize) {
    let now = need.min(fanout);
    (now, need - now)
}

impl Park {
    pub(crate) fn new(workers: usize, fanout: usize) -> Park {
        assert!(fanout >= 1, "a waker makes at least one wake");
        Park {
            parked: (0..workers.div_ceil(64).max(1)).map(|_| Line(AtomicU64::new(0))).collect(),
            handles: (0..workers).map(|_| OnceLock::new()).collect(),
            owed: AtomicU64::new(0),
            fanout,
            near: OnceLock::new(),
        }
    }

    /// Worker `me` records its thread, once, as it starts.
    pub(crate) fn register(&self, me: usize) {
        assert!(self.handles[me].set(thread::current()).is_ok(), "each worker sets its thread once");
    }

    /// Worker `me`'s word and its bit in it.
    fn place(&self, me: usize) -> (&AtomicU64, u64) {
        (&self.parked[me / 64], 1u64.wrapping_shl(me as u32))
    }

    /// How many workers are parked.
    pub(crate) fn parked_count(&self) -> usize {
        self.parked.iter().map(|w| w.load(Ordering::SeqCst).count_ones() as usize).sum()
    }

    /// Wake one parked worker, if any: whether one was. With none parked,
    /// one load a word.
    pub(crate) fn wake_one(&self) -> bool {
        // The waker's own place first, where the workers have places.
        // The waker's own place first, then the places beside it (its
        // socket's), where the workers have places.
        if let Some((places, beside, here)) = self.near.get() {
            let here = here();
            if places.get(here).is_some_and(|mine| self.wake_one_of(Some(mine))) || beside.get(here).is_some_and(|b| self.wake_one_of(Some(b))) {
                return true;
            }
        }
        self.wake_one_of(None)
    }

    /// Wake every worker that is parked now, each once: how many. (A
    /// loop of `wake_one` would wake the first ones again as they park
    /// again, and never reach the last.)
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn wake_all(&self) -> usize {
        let mut woken = 0;
        for (word, handles) in self.parked.iter().zip(self.handles.chunks(64)) {
            // Every bit taken at once: this thread is then the one to
            // unpark each of those workers.
            let bits = word.swap(0, Ordering::SeqCst);
            for (worker, handle) in handles.iter().enumerate() {
                if bits >> worker & 1 == 1 {
                    handle.get().expect("a parked worker has set its thread").unpark();
                    woken += 1;
                }
            }
        }
        woken
    }

    /// Wake one parked worker of the waker's own place, if any (any
    /// worker, where the workers have no places): whether one was.
    pub(crate) fn wake_one_near(&self) -> bool {
        match self.near.get() {
            Some((places, _, here)) => places.get(here()).is_some_and(|mine| self.wake_one_of(Some(mine))),
            None => self.wake_one_of(None),
        }
    }

    /// The workers' places, once: `places[p]` the workers of place p,
    /// `beside[p]` those of p's socket, `here` the asking thread's place.
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn set_places(&self, places: Box<[Box<[u64]>]>, beside: Box<[Box<[u64]>]>, here: fn() -> usize) {
        assert!(self.near.set((places, beside, here)).is_ok(), "the workers' places are set once");
    }

    /// [`Park::wake_one`] among the workers of `among` (None: all).
    fn wake_one_of(&self, among: Option<&[u64]>) -> bool {
        for (i, (word, handles)) in self.parked.iter().zip(self.handles.chunks(64)).enumerate() {
            let mask = among.map_or(u64::MAX, |m| m[i]);
            let mut bits = word.load(Ordering::SeqCst) & mask;
            while bits != 0 {
                let bit = 1u64 << bits.trailing_zeros();
                let before = word.fetch_and(!bit, Ordering::SeqCst);
                if before & bit != 0 {
                    handles[bit.trailing_zeros() as usize].get().expect("a parked worker has set its thread").unpark();
                    return true;
                }
                bits = before & !bit & mask;
            }
        }
        false
    }

    /// Wake workers for `wanted` pieces of new work, given `workers`
    /// workers in all: up to `fanout` now, the rest owed, which the woken
    /// workers pay, `fanout` each, before they work. How many it woke. A
    /// publisher's read of `parked` follows its publication (a SeqCst store
    /// or a fence), as a parking worker's re-check follows its bit: one of
    /// the two sees the other.
    pub(crate) fn wake_for(&self, wanted: usize, workers: usize) -> usize {
        let parked = self.parked_count();
        // Nobody parked, or enough awake: nothing to wake.
        let (now, later) = split(wanted.saturating_sub(workers - parked).min(parked), self.fanout);
        if later > 0 {
            let add = later.min(u32::MAX as usize) as u64;
            loop {
                let cur = self.owed.load(Ordering::SeqCst);
                let next_gen = (cur >> 32).wrapping_add(1);
                let next_count = (cur & OWED_COUNT_MASK).saturating_add(add).min(OWED_COUNT_MASK);
                if self.owed.compare_exchange(cur, (next_gen << 32) | next_count, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                    break;
                }
            }
        }
        (0..now).take_while(|_| self.wake_one()).count()
    }

    /// Worker `me` parks unless `visible()` finds work: the bit first,
    /// then one more look (a publisher stores, then reads `parked`; this
    /// sets the bit, then looks: one of the two sees the other), then the
    /// token, until a waker clears the bit. Returns having found work
    /// (false) or having been woken (true); a woken worker then pays the
    /// wakes owed ([`Park::pay_owed`]).
    pub(crate) fn park_unless(&self, me: usize, visible: impl Fn() -> bool) -> bool {
        let (word, bit) = self.place(me);
        word.fetch_or(bit, Ordering::SeqCst);
        // An acquire-only visibility check does not order the parked bit.
        fence(Ordering::SeqCst);
        if visible() {
            // Ours to clear, unless a waker cleared it first: then its wake
            // was spent on a worker that was leaving anyway (its unpark
            // leaves a token a later park returns from at once), and a
            // worker still parked may be the one it was for. Pass the wake
            // on, and return as woken so the caller pays what a woken
            // worker owes. Dropping it left a parked worker unwoken with
            // work published: Shuttle found both schedules (October 23).
            if word.fetch_and(!bit, Ordering::SeqCst) & bit == 0 {
                self.wake_one();
                return true;
            }
            return false;
        }
        while word.load(Ordering::SeqCst) & bit != 0 {
            thread::park();
        }
        true
    }

    /// A woken worker pays up to `fanout` of the wakes owed before it
    /// works, each a worker that pays in turn: how many it woke.
    pub(crate) fn pay_owed(&self) -> usize {
        let mut made = 0;
        while made < self.fanout {
            let cur = self.owed.load(Ordering::SeqCst);
            let count = cur & OWED_COUNT_MASK;
            if count == 0 {
                break;
            }
            let next = cur - 1;
            if self.owed.compare_exchange(cur, next, Ordering::SeqCst, Ordering::SeqCst).is_err() {
                continue;
            }
            if !self.wake_one() {
                // Retire only obsolete debt from the exact generation we
                // decremented: any concurrent publisher bumps the upper 32-bit
                // generation, preventing ABA from wiping fresh fan-out debt.
                let cleared = next & !OWED_COUNT_MASK;
                let _ = self.owed.compare_exchange(next, cleared, Ordering::SeqCst, Ordering::SeqCst);
                break;
            }
            made += 1;
        }
        made
    }

    #[cfg(all(test, not(shuttle)))]
    fn owed_count(&self) -> usize {
        (self.owed.load(Ordering::SeqCst) & OWED_COUNT_MASK) as usize
    }
}

#[cfg(test)]
mod test {
    #[cfg(not(shuttle))]
    use super::Park;
    use super::split;
    #[cfg(not(shuttle))]
    use std::sync::atomic::Ordering;

    /// A park of `workers` slots, every slot this thread's (an unpark of
    /// the test's own thread leaves a token and nothing more), with the
    /// workers in `parked` parked. Outside the Shuttle build, where park.rs
    /// is compiled again under Shuttle's threads and the test thread has no
    /// handle to register.
    #[cfg(not(shuttle))]
    fn park_with(workers: usize, fanout: usize, parked: &[usize]) -> Park {
        let park = Park::new(workers, fanout);
        for me in 0..workers {
            park.register(me);
        }
        for &me in parked {
            let (word, bit) = park.place(me);
            word.fetch_or(bit, Ordering::SeqCst);
        }
        park
    }

    #[cfg(not(shuttle))]
    std::thread_local! {
        /// The place the test's thread says it is in.
        static HERE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// Where the workers have places, a waker wakes its own place's
    /// first, then its socket's, then any, the lowest-numbered of each;
    /// `wake_one_near` wakes its own place's alone; a waker in a place
    /// that is not listed wakes any. Six workers: places {0, 1}, {2, 3}
    /// and {4, 5}, the first two places one socket.
    #[test]
    #[cfg(not(shuttle))]
    fn a_waker_wakes_its_place_then_its_socket_then_any() {
        let parked = |park: &Park| (0..6).filter(|&w| park.parked[0].load(Ordering::SeqCst) >> w & 1 == 1).collect::<Vec<_>>();
        let with_places = || {
            let park = park_with(6, 4, &[0, 1, 2, 3, 4, 5]);
            let places: Box<[Box<[u64]>]> = [0b000011u64, 0b001100, 0b110000].iter().map(|&w| vec![w].into_boxed_slice()).collect();
            let beside: Box<[Box<[u64]>]> = [0b001111u64, 0b001111, 0b110000].iter().map(|&w| vec![w].into_boxed_slice()).collect();
            park.set_places(places, beside, || HERE.with(|h| h.get()));
            park
        };
        let park = with_places();
        HERE.with(|h| h.set(1));
        for left in [vec![0, 1, 3, 4, 5], vec![0, 1, 4, 5], vec![1, 4, 5], vec![4, 5], vec![5], vec![]] {
            assert!(park.wake_one());
            assert_eq!(parked(&park), left, "place 1's, then its socket's, then the rest");
        }
        assert!(!park.wake_one(), "nobody parked");
        let park = with_places();
        HERE.with(|h| h.set(2));
        assert!(park.wake_one_near() && park.wake_one_near());
        assert_eq!(parked(&park), [0, 1, 2, 3]);
        assert!(!park.wake_one_near(), "its place's workers are awake: no other is woken");
        assert_eq!(parked(&park), [0, 1, 2, 3]);
        assert!(park.wake_one(), "any waker wakes the others");
        assert_eq!(parked(&park), [1, 2, 3]);
        // A place that is not listed (a CPU the process was not on at the start).
        HERE.with(|h| h.set(9));
        assert!(!park.wake_one_near());
        assert!(park.wake_one());
        assert_eq!(parked(&park), [2, 3]);
        // Every parked worker, each once.
        let park = with_places();
        assert!(park.wake_one());
        assert_eq!(park.wake_all(), 5);
        assert_eq!(parked(&park), Vec::<usize>::new());
        assert_eq!(park.wake_all(), 0);
        // Without places, the near wake is any worker's.
        let park = park_with(3, 4, &[1, 2]);
        assert!(park.wake_one_near());
        assert_eq!(park.parked_count(), 1);
        HERE.with(|h| h.set(0));
    }

    /// (Outside the Shuttle build, where park.rs is compiled again under
    /// Shuttle's threads and this thread has no handle to register.)
    /// The wakes a publisher makes and owes, exactly (BLAKE3 #219): of the
    /// `wanted`, the workers awake already cover their share; of the rest,
    /// at most the parked, at most `fanout` now and the others owed; a
    /// woken worker pays up to `fanout` of the owed, and owed wakes with no
    /// one parked are dropped.
    #[test]
    #[cfg(not(shuttle))]
    fn wakes_made_and_owed_exactly() {
        let all: Vec<usize> = (0..12).collect();
        let park = park_with(12, 4, &all);
        assert_eq!(park.wake_for(1, 12), 1, "one wanted, twelve parked: one wake");
        assert_eq!((park.parked_count(), park.owed_count()), (11, 0));
        // Ten wanted, one worker awake already: nine to wake, four now
        // and five owed.
        assert_eq!(park.wake_for(10, 12), 4);
        assert_eq!((park.parked_count(), park.owed_count()), (7, 5));
        // A woken worker pays four of the five, then another the last.
        assert_eq!(park.pay_owed(), 4);
        assert_eq!((park.parked_count(), park.owed_count()), (3, 1));
        assert_eq!(park.pay_owed(), 1);
        assert_eq!((park.parked_count(), park.owed_count()), (2, 0));
        assert_eq!(park.pay_owed(), 0, "nothing owed: nothing paid");
        // Five wanted with ten awake: they cover it.
        assert_eq!(park.wake_for(5, 12), 0);
        assert_eq!((park.parked_count(), park.owed_count()), (2, 0));
        // A hundred wanted, two parked: those two, nothing owed.
        assert_eq!(park.wake_for(100, 12), 2);
        assert_eq!((park.parked_count(), park.owed_count()), (0, 0));
        // Owed wakes with no one parked are dropped, preserving generation.
        let before_gen = park.owed.load(Ordering::SeqCst) >> 32;
        park.owed.store((before_gen << 32) | 3, Ordering::SeqCst);
        assert_eq!(park.pay_owed(), 0);
        assert_eq!(park.owed_count(), 0);
        assert_eq!(park.owed.load(Ordering::SeqCst) >> 32, before_gen);
        // Fresh publisher additions bump the generation so an empty-scan clear
        // carrying an older generation with the same numeric count cannot ABA-clear it.
        let park = park_with(8, 2, &[0, 1, 2, 3, 4, 5, 6, 7]);
        let gen0 = park.owed.load(Ordering::SeqCst) >> 32;
        assert_eq!(park.wake_for(5, 8), 2);
        assert_eq!(park.owed_count(), 3);
        let gen1 = park.owed.load(Ordering::SeqCst) >> 32;
        assert_eq!(gen1, gen0.wrapping_add(1));
        // A second publisher addition on the same Park bumps gen1 -> gen2
        // (proving high-32 generation bits are read with `>> 32`, not `<< 32`).
        assert_eq!(park.wake_for(8, 8), 2);
        assert_eq!(park.owed_count(), 7);
        let gen2 = park.owed.load(Ordering::SeqCst) >> 32;
        assert_eq!(gen2, gen1.wrapping_add(1));
        // Exactly the fan-out wanted of as many parked: all now, none owed.
        let park = park_with(4, 4, &[0, 1, 2, 3]);
        assert_eq!(park.wake_for(4, 4), 4);
        assert_eq!((park.parked_count(), park.owed_count()), (0, 0));
    }

    /// A waker's share: at most its fan-out now, the rest owed, nothing
    /// lost or added.
    #[test]
    fn a_waker_wakes_at_most_its_fanout_and_owes_the_rest() {
        for need in 0..100 {
            for fanout in 1..20 {
                let (now, later) = split(need, fanout);
                assert!(now <= fanout, "need {need}, fan-out {fanout}");
                assert_eq!(now + later, need, "need {need}, fan-out {fanout}");
                assert_eq!(later > 0, need > fanout, "need {need}, fan-out {fanout}");
            }
        }
    }
}
