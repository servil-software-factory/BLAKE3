//! Lifecycles: sequences of every public operation on `Hasher`s, several
//! at once, each step checked against official BLAKE3 for every hasher in
//! play, the ones the step did not name included. So a grouped call is
//! held to "a batch equals one at a time": what it does to one hasher is
//! what the same bytes fed alone would do, whatever its neighbours hold.
//!
//! Why: 0.9's wrong hash (GHSA-v237-j9h9-8pf7) was two operations in
//! sequence, a trait implementation that left a value no constructor
//! makes and a grouped call that relied on every value being one a
//! constructor makes. Each was tested alone, against official BLAKE3; no
//! test ran the one and then the other. The lists below name every
//! public operation of the types walked here, and `tools/api.py --check`
//! (tools/check-builds.sh) refuses an operation that is in neither: a
//! new method or trait implementation joins the walk, or says why not.
#![cfg(feature = "std")]
use blake3_servil_core::{Mode, convenience::Hasher, initialize, throughput::*};
use rand::{RngExt, SeedableRng, rngs::StdRng};
use std::io::{Read, Seek, SeekFrom, Write};

// WALKED
// Hasher::new Hasher::update Hasher::update_with Hasher::update_reader Hasher::finalize Hasher::finalize_xof
// Hasher::Clone Hasher::Debug Hasher::Write
// throughput::update_each throughput::update_each_with throughput::finalize_each
// OutputReader::fill OutputReader::position OutputReader::set_position OutputReader::Clone OutputReader::Read OutputReader::Seek OutputReader::Debug
// NOT_WALKED
// END

const EDGES: [usize; 8] = [0, 1, 64, 1024, 1025, 16383, 16384, 16385];
const CONTEXT: &str = "BLAKE3-Servil lifecycle test";
fn mode(i: usize) -> Mode<'static> {
    match i % 5 {
        0 | 1 => Mode::Plain,
        2 => Mode::Keyed(&[99; 32]),
        3 => Mode::Keyed(&[42; 32]),
        _ => Mode::DeriveKey(CONTEXT),
    }
}
#[derive(Clone)]
struct Model {
    mode: Mode<'static>,
    bytes: Vec<u8>,
}
impl Model {
    fn fresh(&self, hint: Option<u64>, prepared: bool) -> Hasher {
        if let (Mode::DeriveKey(context), true) = (self.mode, prepared) {
            Hasher::new(Mode::DeriveKeyWith(&context_key(context)), hint)
        } else {
            Hasher::new(self.mode, hint)
        }
    }
    fn official(&self) -> blake3::Hasher {
        let mut h = match self.mode {
            Mode::Plain => blake3::Hasher::new(),
            Mode::Keyed(k) => blake3::Hasher::new_keyed(k),
            Mode::DeriveKey(s) => blake3::Hasher::new_derive_key(s),
            _ => unreachable!(),
        };
        h.update(&self.bytes);
        h
    }
}
#[derive(Clone, Copy)]
enum Step {
    Update,
    With,
    Reader,
    Write,
    WriteAll,
    Flush,
    Clone,
    CloneFrom,
    Turn,
    TurnWith,
    TurnSome,
    TurnNone,
    Finish,
    Debug,
    Xof,
    New,
}
use Step::*;
const STEPS: [Step; 16] = [Update, With, Reader, Write, WriteAll, Flush, Clone, CloneFrom, Turn, TurnWith, TurnSome, TurnNone, Finish, Debug, Xof, New];

struct Walk {
    h: Vec<Hasher>,
    m: Vec<Model>,
}
impl Walk {
    fn new(n: usize, hint: bool) -> Self {
        let m: Vec<_> = (0..n).map(|i| Model { mode: mode(i), bytes: vec![] }).collect();
        let h = m.iter().enumerate().map(|(i, model)| model.fresh(hint.then_some(1024), i % 2 == 0)).collect();
        Self { h, m }
    }
    fn check(&self) {
        for (i, (h, m)) in self.h.iter().zip(&self.m).enumerate().rev() {
            assert_eq!(h.finalize().as_bytes(), m.official().finalize().as_bytes(), "hasher {i}");
        }
    }
    fn step(&mut self, op: Step, i: usize, bytes: &[u8]) {
        let n = self.h.len();
        let j = (i + 1) % n;
        match op {
            Update => {
                self.h[i].update(bytes);
            }
            With => {
                self.h[i].update_with(bytes, 16384, &TreeHasher::subtree_halves);
            }
            Reader => {
                self.h[i].update_reader(bytes).unwrap();
            }
            Write => assert_eq!(self.h[i].write(bytes).unwrap(), bytes.len()),
            WriteAll => self.h[i].write_all(bytes).unwrap(),
            Flush => self.h[i].flush().unwrap(),
            Clone | CloneFrom => {
                if matches!(op, Clone) {
                    self.h[j] = self.h[i].clone();
                } else if j > i {
                    let (a, b) = self.h.split_at_mut(j);
                    b[0].clone_from(&a[i]);
                } else {
                    let (a, b) = self.h.split_at_mut(i);
                    a[j].clone_from(&b[0]);
                }
                self.m[j] = self.m[i].clone();
            }
            Turn | TurnWith => {
                let pieces: Vec<_> = (0..n).map(|k| ((i + k) % n, &bytes[k.min(bytes.len())..])).chain([(i, &bytes[..bytes.len().min(3)])]).collect();
                if matches!(op, Turn) {
                    update_each(&mut self.h, &pieces)
                } else {
                    update_each_with(&mut self.h, &pieces, 16384, &TreeHasher::subtree_halves)
                }
                for (k, b) in pieces {
                    self.m[k].bytes.extend_from_slice(b);
                }
            }
            // A turn that names some of the hashers, and one that names
            // none: the others hold what they held. (Every turn named
            // every hasher until an auditor's fault, an `update_each`
            // that fed a hasher its pieces did not name, passed.)
            TurnSome | TurnNone => {
                let pieces: Vec<_> =
                    if matches!(op, TurnSome) { (0..n).step_by(2).map(|k| ((i + k) % n, &bytes[k.min(bytes.len())..])).collect() } else { vec![] };
                if i % 2 == 0 {
                    update_each(&mut self.h, &pieces)
                } else {
                    update_each_with(&mut self.h, &pieces, 16384, &TreeHasher::subtree_halves)
                }
                for (k, b) in pieces {
                    self.m[k].bytes.extend_from_slice(b);
                }
            }
            Finish => {
                let count = if i % 2 == 0 { n } else { n.div_ceil(2) };
                let which: Vec<_> = (0..count).rev().map(|k| (i + k) % n).collect();
                let mut out = vec![[0; 32]; which.len()];
                finalize_each(&mut self.h, &which, &mut out);
                for (digest, k) in out.iter().zip(which) {
                    assert_eq!(digest, self.m[k].official().finalize().as_bytes());
                    self.m[k].bytes.clear();
                }
            }
            Debug => {
                // The key and message are not debugging output: a fresh hasher with the same flags prints identically.
                let public_mode = match self.m[i].mode {
                    Mode::Keyed(_) => Mode::Keyed(&[0; 32]),
                    m => m,
                };
                assert_eq!(format!("{:?}", self.h[i]), format!("{:?}", Hasher::new(public_mode, None)));
            }
            Xof => {
                let (mut a, mut b) = ([0; 137], [0; 137]);
                self.h[i].finalize_xof().fill(&mut a);
                self.m[i].official().finalize_xof().fill(&mut b);
                assert_eq!(a, b);
            }
            New => {
                self.m[i] = Model { mode: mode(bytes.len()), bytes: vec![] };
                self.h[i] = self.m[i].fresh((i % 2 == 0).then_some(bytes.len() as u64), i % 3 == 0);
            }
        }
        if matches!(op, Update | With | Reader | Write | WriteAll) {
            self.m[i].bytes.extend_from_slice(bytes);
        }
        self.check();
    }
}
#[test]
fn short_histories_preserve_every_message() {
    initialize();
    // Four operations to depth five: 1,365 prefixes (1,024 leaves) for each of eight starting lengths.
    for offset in 0..8 {
        for path in 0..1024 {
            let mut w = Walk::new(5, offset % 2 == 0);
            w.check();
            for d in 0..5 {
                let digit = path >> (2 * d) & 3;
                let len = EDGES[(offset + d) % 8];
                w.step([Update, Clone, Turn, Finish][digit], d, &vec![(offset + d) as u8; len]);
            }
        }
    }
}
/// Every step after every other, after every pair and after every three:
/// the 16 steps to depth four (65,536 sequences), on three hashers (two
/// plain and one keyed at the start; a step makes new ones in the other
/// modes), the first step on hasher 0, the second on 1, and so on, with
/// lengths on both sides of the 16 KiB group. A step that leaves a value
/// which a later one cannot take shows here, whichever they are. These
/// are the sequences of this file's steps, not of every argument each
/// operation takes.
#[test]
fn every_four_operations_in_a_row_preserve_every_message() {
    initialize();
    const DEPTH: usize = 4;
    for path in 0..STEPS.len().pow(DEPTH as u32) {
        let mut w = Walk::new(3, path % 2 == 0);
        for d in 0..DEPTH {
            let op = STEPS[path / STEPS.len().pow(d as u32) % STEPS.len()];
            let len = [16385, 1025, 16384, 1][(path + d) % 4];
            w.step(op, d % 3, &vec![(path + d) as u8; len]);
        }
    }
}
#[test]
fn long_histories_preserve_every_message() {
    initialize();
    const SEED: u64 = 0x104f_4d32;
    eprintln!("seed {SEED:#x}");
    let mut rng = StdRng::seed_from_u64(SEED);
    let mut w = Walk::new(17, false);
    w.check();
    let data: Vec<_> = (0..300_000).map(|_| rng.random()).collect();
    for step in 0..2500 {
        let len = if step % 61 == 0 { rng.random_range(200_000..data.len()) } else { EDGES[rng.random_range(0..8)] };
        let off = rng.random_range(0..=data.len() - len);
        w.step(STEPS[rng.random_range(0..STEPS.len())], rng.random_range(0..17), &data[off..off + len]);
    }
}
#[test]
fn a_repeated_finish_is_refused_before_any_message_changes() {
    initialize();
    let mut w = Walk::new(5, false);
    w.step(Turn, 0, b"message");
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| finalize_each(&mut w.h, &[1, 0, 1], &mut [[0; 32]; 3]))).is_err());
    w.check();
    w.step(Finish, 0, &[]);
}
#[test]
fn a_plain_neighbor_hashes_its_own_bytes() {
    initialize();
    let mut w = Walk::new(2, false);
    for _ in 0..5 {
        let pieces = [(0, &[3u8; 4096][..]), (1, &[5u8; 4096][..])];
        update_each(&mut w.h, &pieces);
        for (i, b) in pieces {
            w.m[i].bytes.extend_from_slice(b);
        }
        w.check();
    }
}
#[test]
fn reader_copies_seek_and_read_independently_to_the_end() {
    initialize();
    let mut h = Hasher::new(mode(2), None);
    h.update(b"reader");
    let mut a = h.finalize_xof();
    let mut b = Model { mode: mode(2), bytes: b"reader".to_vec() }.official().finalize_xof();
    for pos in [0, 1, 63, 64, 65, 1 << 42, u64::MAX - 94, u64::MAX] {
        a.set_position(pos);
        b.set_position(pos);
        assert_eq!(a.position(), b.position());
        let n = 95.min(u64::MAX - pos) as usize;
        let (mut x, mut y) = ([0; 95], [0; 95]);
        assert_eq!(a.read(&mut x).unwrap(), n);
        b.fill(&mut y[..n]);
        assert_eq!(x, y);
        assert_eq!(a.position(), b.position());
        let (mut c, mut d) = (a.clone(), b.clone());
        assert_eq!(c.seek(SeekFrom::Current(-1)).unwrap(), d.seek(SeekFrom::Current(-1)).unwrap());
        c.fill(&mut x[..1]);
        d.fill(&mut y[..1]);
        assert_eq!(x, y);
        assert_eq!(a.position(), b.position());
        assert!(c.seek(SeekFrom::End(0)).is_err());
        assert_eq!(c.seek(SeekFrom::Start(0)).unwrap(), 0);
        // A target before the start is refused, and one past the end
        // stops at the end, as official BLAKE3's reader does.
        assert!(c.seek(SeekFrom::Current(-1)).is_err() && d.seek(SeekFrom::Start(0)).is_ok() && d.seek(SeekFrom::Current(-1)).is_err());
        assert_eq!(c.position(), 0, "a refused seek moves nothing");
        c.set_position(u64::MAX - 3);
        d.set_position(u64::MAX - 3);
        assert_eq!(c.seek(SeekFrom::Current(i64::MAX)).unwrap(), u64::MAX);
        assert_eq!(d.seek(SeekFrom::Current(i64::MAX)).unwrap(), u64::MAX);
        assert_eq!(a.position(), pos + n as u64);
        assert_eq!(format!("{a:?}"), format!("OutputReader {{ position: {} }}", a.position()));
    }
}
