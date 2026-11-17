//! These calls against official BLAKE3: every way of building the tree
//! that the calls allow gives the message's hash, and every wrong shape
//! is refused.

use blake3_servil_core::throughput::{Cv, LastCv, TreeHasher as Tree, left_len, level_of};
use blake3_servil_core::{CHUNK_LEN, Hash, Mode};
use std::panic::{AssertUnwindSafe, catch_unwind};

const KEY: [u8; 32] = [0x6b; 32];

fn message(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8 ^ (i / 251) as u8).collect()
}

/// Official BLAKE3's hash of `msg` in the mode `keyed` names.
fn official(msg: &[u8], keyed: bool) -> [u8; 32] {
    if keyed { *blake3::keyed_hash(&KEY, msg).as_bytes() } else { *blake3::hash(msg).as_bytes() }
}

fn tree(keyed: bool) -> Tree {
    Tree::new(if keyed { Mode::Keyed(&KEY) } else { Mode::Plain })
}

/// The lengths tried: every chunk count to 40 with a byte less, exact, and
/// a byte more, and a few longer.
fn lengths() -> Vec<usize> {
    let mut lens = vec![1, 2, 63, 64, 65, 1000];
    for chunks in 1..=40 {
        lens.extend([chunks * CHUNK_LEN - 1, chunks * CHUNK_LEN, chunks * CHUNK_LEN + 1]);
    }
    lens.extend([100 * CHUNK_LEN + 17, 128 * CHUNK_LEN, 257 * CHUNK_LEN - 1, 300 * CHUNK_LEN + 512]);
    lens
}

/// A file hashed in pieces of one size, then its parents a layer at a time
/// (b3sum's direct read, 2 MiB pieces; its outboard, 16 KiB groups).
fn by_pieces(tree: &Tree, msg: &[u8], level: u32) -> Hash {
    let piece = CHUNK_LEN << level;
    if msg.len() <= piece {
        return tree.hash(msg);
    }
    let n = msg.len().div_ceil(piece);
    let mut cvs: Vec<Cv> = (0..n - 1).map(|i| tree.subtree_cv(level, i as u64, &msg[i * piece..(i + 1) * piece])).collect();
    let mut last = tree.last_cv(((n - 1) << level) as u64, &msg[(n - 1) * piece..]);
    let mut next = vec![Cv::unset(); cvs.len() / 2];
    while cvs.len() > 1 {
        let pairs = cvs.len() / 2;
        tree.parent_cvs(&cvs[..2 * pairs], &mut next[..pairs]);
        // An odd whole one out is the last subtree's left sibling.
        if cvs.len() % 2 == 1 {
            last = tree.last_parent_cv(&cvs[2 * pairs], &last);
        }
        cvs.truncate(pairs);
        cvs.copy_from_slice(&next[..pairs]);
    }
    tree.root_hash(&cvs[0], &last, msg.len() as u64)
}

/// The tree walked from its root, splitting where `left_len` says (a
/// recursive builder, as bao-tree's is).
fn by_recursion(tree: &Tree, msg: &[u8]) -> Hash {
    fn whole(tree: &Tree, level: u32, index: u64, data: &[u8]) -> Cv {
        if level == 0 {
            return tree.subtree_cv(0, index, data);
        }
        let half = data.len() / 2;
        tree.parent_cv(&whole(tree, level - 1, 2 * index, &data[..half]), &whole(tree, level - 1, 2 * index + 1, &data[half..]))
    }
    fn last(tree: &Tree, first_chunk: u64, data: &[u8]) -> LastCv {
        if data.len() <= CHUNK_LEN {
            return tree.last_cv(first_chunk, data);
        }
        let left = left_len(data.len() as u64) as usize;
        let level = level_of(left as u64);
        let l = whole(tree, level, first_chunk >> level, &data[..left]);
        tree.last_parent_cv(&l, &last(tree, first_chunk + (left / CHUNK_LEN) as u64, &data[left..]))
    }
    if msg.len() <= CHUNK_LEN {
        return tree.hash(msg);
    }
    let left = left_len(msg.len() as u64) as usize;
    let level = level_of(left as u64);
    tree.root_hash(&whole(tree, level, 0, &msg[..left]), &last(tree, (left / CHUNK_LEN) as u64, &msg[left..]), msg.len() as u64)
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// A piece of a cut: a whole subtree, or the last one.
#[derive(Clone, Copy, Debug)]
enum Piece {
    Whole { level: u32, index: u64 },
    Last { first_chunk: u64, len: usize },
}

/// A random cut of a message of `len` bytes into subtrees of mixed sizes,
/// in order (the pool cuts a message into pieces of several sizes).
fn random_cut(rng: &mut Rng, len: usize) -> Vec<Piece> {
    fn whole(rng: &mut Rng, level: u32, index: u64, out: &mut Vec<Piece>) {
        if level > 0 && rng.next() % 3 != 0 {
            whole(rng, level - 1, 2 * index, out);
            whole(rng, level - 1, 2 * index + 1, out);
        } else {
            out.push(Piece::Whole { level, index });
        }
    }
    fn last(rng: &mut Rng, first_chunk: u64, len: usize, root: bool, out: &mut Vec<Piece>) {
        if len > CHUNK_LEN && (root || rng.next() % 3 != 0) {
            let left = left_len(len as u64) as usize;
            let level = level_of(left as u64);
            whole(rng, level, first_chunk >> level, out);
            last(rng, first_chunk + (left / CHUNK_LEN) as u64, len - left, false, out);
        } else {
            out.push(Piece::Last { first_chunk, len });
        }
    }
    let mut out = Vec::new();
    last(rng, 0, len, true, &mut out);
    out
}

/// The hash from a cut's pieces, taken in order: a stack of whole
/// subtrees, each joined to its left sibling as soon as it is there.
fn by_cut(tree: &Tree, msg: &[u8], cut: &[Piece]) -> Hash {
    let mut stack: Vec<Cv> = Vec::new();
    for piece in cut {
        match *piece {
            Piece::Whole { level, index } => {
                let start = (index << level) as usize * CHUNK_LEN;
                let mut cv = tree.subtree_cv(level, index, &msg[start..start + (CHUNK_LEN << level)]);
                while let Some(top) = stack.last() {
                    if top.level() != cv.level() || top.index() % 2 != 0 || top.index() + 1 != cv.index() {
                        break;
                    }
                    cv = tree.parent_cv(&stack.pop().unwrap(), &cv);
                }
                stack.push(cv);
            }
            Piece::Last { first_chunk, len } => {
                let start = first_chunk as usize * CHUNK_LEN;
                let mut last = tree.last_cv(first_chunk, &msg[start..start + len]);
                while stack.len() > 1 {
                    last = tree.last_parent_cv(&stack.pop().unwrap(), &last);
                }
                return tree.root_hash(&stack[0], &last, msg.len() as u64);
            }
        }
    }
    unreachable!("a cut ends with the last subtree")
}

#[test]
fn every_allowed_build_gives_blake3s_hash() {
    blake3_servil_core::initialize();
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut builds = 0;
    for len in lengths() {
        let msg = message(len);
        for keyed in [false, true] {
            let want = official(&msg, keyed);
            let tree = tree(keyed);
            for level in 0..=6 {
                assert_eq!(by_pieces(&tree, &msg, level).as_bytes(), &want, "pieces at level {level}, {len} bytes");
                builds += 1;
            }
            assert_eq!(by_recursion(&tree, &msg).as_bytes(), &want, "recursion, {len} bytes");
            if len > CHUNK_LEN {
                for _ in 0..8 {
                    let cut = random_cut(&mut rng, len);
                    assert_eq!(by_cut(&tree, &msg, &cut).as_bytes(), &want, "{len} bytes cut as {cut:?}");
                    builds += 1;
                }
            }
        }
    }
    assert!(builds > 3000, "{builds} builds");
}

/// A verifier's walk (verified streaming): each parent's two children are
/// 64 stored bytes, checked against the value their parent's pair gave.
#[test]
fn stored_values_verify_from_the_root_down() {
    blake3_servil_core::initialize();
    type Pairs = Vec<[[u8; 32]; 2]>;
    fn build_whole(tree: &Tree, level: u32, index: u64, data: &[u8], out: &mut Pairs) -> Cv {
        if level == 0 {
            return tree.subtree_cv(0, index, data);
        }
        let (at, half) = (out.len(), data.len() / 2);
        out.push([[0; 32]; 2]);
        let l = build_whole(tree, level - 1, 2 * index, &data[..half], out);
        let r = build_whole(tree, level - 1, 2 * index + 1, &data[half..], out);
        out[at] = [*l.as_bytes(), *r.as_bytes()];
        tree.parent_cv(&l, &r)
    }
    /// The children of the last subtree at `first_chunk`, of more than a chunk.
    fn build_last_children(tree: &Tree, first_chunk: u64, data: &[u8], out: &mut Pairs) -> (Cv, LastCv) {
        let at = out.len();
        out.push([[0; 32]; 2]);
        let left = left_len(data.len() as u64) as usize;
        let level = level_of(left as u64);
        let l = build_whole(tree, level, first_chunk >> level, &data[..left], out);
        let (rest, rest_chunk) = (&data[left..], first_chunk + (left / CHUNK_LEN) as u64);
        let r = if rest.len() <= CHUNK_LEN {
            tree.last_cv(rest_chunk, rest)
        } else {
            let (rl, rr) = build_last_children(tree, rest_chunk, rest, out);
            tree.last_parent_cv(&rl, &rr)
        };
        out[at] = [*l.as_bytes(), *r.as_bytes()];
        (l, r)
    }
    fn verify_whole(tree: &Tree, want: &Cv, data: &[u8], pairs: &mut std::slice::Iter<[[u8; 32]; 2]>) -> bool {
        let (level, index) = (want.level(), want.index());
        if level == 0 {
            return tree.subtree_cv(0, index, data) == *want;
        }
        let [l, r] = *pairs.next().unwrap();
        let (l, r) = (Cv::at(level - 1, 2 * index, l), Cv::at(level - 1, 2 * index + 1, r));
        let half = data.len() / 2;
        tree.parent_cv(&l, &r) == *want && verify_whole(tree, &l, &data[..half], pairs) && verify_whole(tree, &r, &data[half..], pairs)
    }
    /// The stored children of the last subtree at `first_chunk`, as values with their places.
    fn stored_children(first_chunk: u64, len: usize, pair: [[u8; 32]; 2]) -> (Cv, LastCv) {
        let left = left_len(len as u64) as usize;
        let level = level_of(left as u64);
        let rest_chunk = first_chunk + (left / CHUNK_LEN) as u64;
        (Cv::at(level, first_chunk >> level, pair[0]), LastCv::at(rest_chunk, (len - left) as u64, pair[1]))
    }
    fn verify_last(tree: &Tree, want: &LastCv, data: &[u8], pairs: &mut std::slice::Iter<[[u8; 32]; 2]>) -> bool {
        if data.len() <= CHUNK_LEN {
            return tree.last_cv(want.first_chunk(), data) == *want;
        }
        let (l, r) = stored_children(want.first_chunk(), data.len(), *pairs.next().unwrap());
        let left = left_len(data.len() as u64) as usize;
        tree.last_parent_cv(&l, &r) == *want && verify_whole(tree, &l, &data[..left], pairs) && verify_last(tree, &r, &data[left..], pairs)
    }
    fn verify(tree: &Tree, root: &Hash, data: &[u8], pairs: &Pairs) -> bool {
        let mut pairs = pairs.iter();
        let (l, r) = stored_children(0, data.len(), *pairs.next().unwrap());
        let left = left_len(data.len() as u64) as usize;
        tree.root_hash(&l, &r, data.len() as u64) == *root
            && verify_whole(tree, &l, &data[..left], &mut pairs)
            && verify_last(tree, &r, &data[left..], &mut pairs)
    }
    let tree = tree(false);
    for len in [CHUNK_LEN + 1, 5000, 13 * CHUNK_LEN, 16 * CHUNK_LEN, 37 * CHUNK_LEN + 3] {
        let msg = message(len);
        let mut pairs = Pairs::new();
        let (l, r) = build_last_children(&tree, 0, &msg, &mut pairs);
        let root = tree.root_hash(&l, &r, len as u64);
        assert_eq!(root.as_bytes(), &official(&msg, false));
        assert!(verify(&tree, &root, &msg, &pairs), "{len} bytes verify");
        // A changed byte of the message, and of any stored pair, is found.
        let mut bad = msg.clone();
        bad[len / 2] ^= 1;
        assert!(!verify(&tree, &root, &bad, &pairs), "{len} bytes: a changed message");
        for at in 0..pairs.len() {
            for side in 0..2 {
                let mut bad = pairs.clone();
                bad[at][side][at % 32] ^= 0x80;
                assert!(!verify(&tree, &root, &msg, &bad), "{len} bytes: pair {at}, child {side} changed");
            }
        }
    }
}

/// The message `f` panics with, if it panics.
fn refusal<T>(f: impl FnOnce() -> T) -> Option<String> {
    let payload = catch_unwind(AssertUnwindSafe(f)).err()?;
    Some(payload.downcast_ref::<String>().cloned().or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default())
}

/// `f` is refused, and for the reason `why` names: a call refused for
/// another reason would pass a test that only asked for a panic.
#[track_caller]
fn refused<T>(why: &str, what: &str, f: impl FnOnce() -> T) {
    match refusal(f) {
        Some(message) => assert!(message.contains(why), "{what}: refused, but with {message:?}, not {why:?}"),
        None => panic!("{what}: taken"),
    }
}

#[test]
fn every_wrong_shape_is_refused() {
    blake3_servil_core::initialize();
    let tree = tree(false);
    let msg = message(16 * CHUNK_LEN);
    let chunk = |i: usize| &msg[i * CHUNK_LEN..(i + 1) * CHUNK_LEN];
    let c: Vec<Cv> = (0..8).map(|i| tree.subtree_cv(0, i as u64, chunk(i))).collect();
    let p01 = tree.parent_cv(&c[0], &c[1]);
    let p23 = tree.parent_cv(&c[2], &c[3]);
    let end = tree.last_cv(8, &msg[8 * CHUNK_LEN..8 * CHUNK_LEN + 100]);

    // The bytes are not the subtree named.
    let size = "holds 4096 bytes (found 5120)";
    refused(size, "five chunks as a whole subtree", || tree.subtree_cv(2, 0, &msg[..5 * CHUNK_LEN]));
    refused("holds 1024 bytes (found 1023)", "a short chunk as a whole one", || tree.subtree_cv(0, 0, &msg[..CHUNK_LEN - 1]));
    refused("holds 2048 bytes (found 1024)", "one chunk as two", || tree.subtree_cv(1, 0, &msg[..CHUNK_LEN]));
    refused("at chunk 2 holds 1 to 2048 bytes (found 3072)", "a last subtree longer than the whole one aligned at its place", || {
        tree.last_cv(2, &msg[..3 * CHUNK_LEN])
    });
    refused("at chunk 1 holds 1 to 1024 bytes (found 1025)", "a last subtree at an odd chunk, longer than a chunk", || tree.last_cv(1, &msg[..CHUNK_LEN + 1]));
    refused("at chunk 6 holds 1 to 2048 bytes (found 2049)", "a last subtree a byte too long", || tree.last_cv(6, &msg[..2 * CHUNK_LEN + 1]));
    refused("at chunk 1 holds 1 to 1024 bytes (found 0)", "an empty last subtree", || tree.last_cv(1, &[]));
    refused("at most 53", "a level beyond the longest message's", || tree.subtree_cv(54, 0, chunk(0)));
    refused("ends beyond", "a chunk beyond the longest message's last", || Cv::at(0, u64::MAX >> 10, [0; 32]));
    refused("starts beyond", "a last subtree beyond the longest message's end", || tree.last_cv(1 << 54, chunk(0)));
    refused("starts beyond", "a stored one there", || LastCv::at(1 << 54, 1, [0; 32]));
    refused("last subtree holds", "a stored last subtree longer than its place", || LastCv::at(1, 1025, [0; 32]));
    refused("last subtree holds", "a stored last subtree of no bytes", || LastCv::at(1, 0, [0; 32]));
    refused("last subtree holds", "a stored last subtree that ends beyond the longest message", || LastCv::at((1 << 54) - 1, 1024, [0; 32]));
    // The longest ones are taken: a whole subtree's worth at its place, and the longest message's last 1023 bytes.
    assert_eq!(tree.last_cv(6, &msg[..2 * CHUNK_LEN]).len(), 2048);
    assert_eq!(LastCv::at((1 << 54) - 1, 1023, [0; 32]).first_chunk(), (1 << 54) - 1);
    assert_eq!(Cv::at(53, 0, [0; 32]).level(), 53);
    // At chunk 0 every size is aligned: a whole message's value, which no parent and no root takes.
    let whole_message = tree.last_cv(0, &msg[..5 * CHUNK_LEN]);
    assert_eq!((whole_message.first_chunk(), whole_message.len()), (0, 5120));

    // Two values that are not siblings in order.
    let siblings = "not siblings";
    refused(siblings, "siblings swapped", || tree.parent_cv(&c[1], &c[0]));
    refused(siblings, "a gap", || tree.parent_cv(&c[0], &c[2]));
    refused(siblings, "neighbours with different parents", || tree.parent_cv(&c[1], &c[2]));
    refused(siblings, "one subtree twice", || tree.parent_cv(&c[0], &c[0]));
    refused(siblings, "a fold from the left: a pair, then the next chunk", || tree.parent_cv(&p01, &c[2]));
    refused(siblings, "one pair twice", || tree.parent_cv(&p01, &p01));
    refused(siblings, "a slot never set, on the right", || tree.parent_cv(&c[0], &Cv::unset()));
    let unset = "a chaining value that was never set";
    refused(unset, "a slot never set, on the left", || tree.parent_cv(&Cv::unset(), &c[1]));
    refused(unset, "a slot never set beside the last subtree", || tree.last_parent_cv(&Cv::unset(), &end));
    refused(unset, "the level of a slot never set", || Cv::unset().level());
    refused(unset, "the index of a slot never set", || Cv::unset().index());
    refused(unset, "a slot never set, made the last subtree", || LastCv::from(Cv::unset()));
    refused(siblings, "a gap before the last subtree", || tree.last_parent_cv(&c[6], &end));
    refused(siblings, "the last subtree beside a subtree that ends earlier", || tree.last_parent_cv(&p23, &end));
    refused(siblings, "the last subtree's neighbour at an odd index", || tree.last_parent_cv(&c[7], &end));
    let after_odd = tree.last_cv(2, &msg[2 * CHUNK_LEN..2 * CHUNK_LEN + 9]);
    refused(siblings, "a last subtree after a left child at an odd index", || tree.last_parent_cv(&c[1], &after_odd));

    // A layer that is not consecutive siblings from an even index.
    let mut out = [Cv::unset(); 4];
    refused("odd index", "a layer from an odd index", || tree.parent_cvs(&c[1..5], &mut out[..2]));
    refused("not the next subtree", "a layer out of order", || tree.parent_cvs(&[c[0], c[1], c[3], c[2]], &mut out[..2]));
    refused("not the next subtree", "a layer with a gap", || tree.parent_cvs(&[c[0], c[1], c[4], c[5]], &mut out[..2]));
    refused("not the next subtree", "a layer of two sizes", || tree.parent_cvs(&[p01, p23, c[4], c[5]], &mut out[..2]));
    refused("not the next subtree", "a layer with a slot never set", || tree.parent_cvs(&[c[0], c[1], c[2], Cv::unset()], &mut out[..2]));
    refused(unset, "a layer that begins with a slot never set", || tree.parent_cvs(&[Cv::unset(), c[1]], &mut out[..1]));
    refused("in pairs", "an odd number of children", || tree.parent_cvs(&c[..3], &mut out[..1]));
    refused("one parent per pair", "too few outputs", || tree.parent_cvs(&c[..4], &mut out[..1]));
    // A layer longer than one call of the kernels takes, broken in its second part.
    let many: Vec<Cv> = (0..40).map(|i| Cv::at(0, i, [i as u8; 32])).collect();
    let mut broken = many.clone();
    broken[37] = many[36];
    let mut parents = vec![Cv::unset(); 20];
    refused("child 37 is not the next subtree", "a long layer broken late", || tree.parent_cvs(&broken, &mut parents));
    refused("in pairs", "a long layer with an odd child after the kernels' first call", || tree.parent_cvs(&many[..33], &mut parents[..16]));
    refused("one parent per pair", "a long layer with outputs for its first call alone", || tree.parent_cvs(&many, &mut parents[..16]));
    refused("one parent per pair", "a layer with an output too many", || tree.parent_cvs(&many[..32], &mut parents[..17]));

    // Two values that are not a root's children.
    let root = "not a root's children";
    let len = (8 * CHUNK_LEN + 100) as u64;
    refused(root, "a root whose left child does not begin the message", || tree.root_hash(&p23, &end, len));
    refused(root, "a root with a gap before its right child", || tree.root_hash(&p01, &end, len));
    refused(root, "a root whose right child is not the next subtree", || tree.root_hash(&p01, &c[3].into(), len));
    refused(root, "an extended output of the same wrong root", || tree.root_output(&p23, &end, len));
    refused(unset, "a root from a slot never set", || tree.root_hash(&Cv::unset(), &end, len));
    // A left child that begins later, with the last subtree where a first chunk's sibling would begin.
    let second = tree.last_cv(1, &msg[CHUNK_LEN..CHUNK_LEN + 9]);
    refused(root, "a root whose left child is the message's second chunk", || tree.root_hash(&c[1], &second, CHUNK_LEN as u64 + 9));
    refused(root, "a root over a whole message's value", || tree.root_hash(&c[0], &whole_message, 6144));
    refused("not siblings", "a parent over a whole message's value", || tree.last_parent_cv(&c[0], &whole_message));
    // A loop that stops a piece early: three chunks of a longer message, the third called its last.
    refused("hold 3072 bytes, and the message 8292", "a root that leaves out the message's end", || tree.root_hash(&p01, &c[2].into(), len));
    refused("hold 3072 bytes, and the message 8292", "an extended output that leaves it out", || tree.root_output(&p01, &c[2].into(), len));

    // The same calls with the right values are taken.
    let mut out = [Cv::unset(); 4];
    tree.parent_cvs(&c, &mut out);
    assert_eq!((out[0], out[1]), (p01, p23));
    let p0123 = tree.parent_cv(&out[0], &out[1]);
    let p4567 = tree.parent_cv(&out[2], &out[3]);
    let all = tree.parent_cv(&p0123, &p4567);
    assert_eq!(tree.root_hash(&all, &end, len).as_bytes(), &official(&msg[..8 * CHUNK_LEN + 100], false));
    assert_eq!(tree.root_hash(&p01, &p23.into(), 4 * CHUNK_LEN as u64).as_bytes(), &official(&msg[..4 * CHUNK_LEN], false));
    let mut xof = [0u8; 64];
    tree.root_output(&p01, &p23.into(), 4 * CHUNK_LEN as u64).fill(&mut xof);
    assert_eq!(xof[..32], official(&msg[..4 * CHUNK_LEN], false));
    // A long layer: its parents carry their places, so the next layer is taken too.
    tree.parent_cvs(&many, &mut parents);
    assert_eq!((parents[19].level(), parents[19].index()), (1, 19));
    let mut grandparents = vec![Cv::unset(); 10];
    tree.parent_cvs(&parents, &mut grandparents);
    assert_eq!((grandparents[9].level(), grandparents[9].index()), (2, 9));
}

/// BLAKE3 #421's report: a 5.5 MiB message in parts of 5 MiB. The bare
/// The first calls took the two parts and gave a hash that is not the
/// message's. Here no call takes a part of 5 MiB.
#[test]
fn parts_of_five_mebibytes() {
    blake3_servil_core::initialize();
    const MIB: usize = 1 << 20;
    let msg = message(5 * MIB + MIB / 2);
    let want = official(&msg, false);

    // These calls: 5 MiB is no level's size, whichever level the program names.
    let tree = tree(false);
    for level in 0..=13 {
        refused("(found 5242880)", "a part of 5 MiB as a whole subtree", || tree.subtree_cv(level, 0, &msg[..5 * MIB]));
    }
    refused("a power of two of chunks", "the level of 5 MiB", || level_of((5 * MIB) as u64));

    // Parts of 1 MiB are subtrees: the same message, its hash.
    assert_eq!(by_pieces(&tree, &msg, level_of(MIB as u64)).as_bytes(), &want);
    // And the shape the tree has: 4 MiB, then the rest.
    let left = tree.subtree_cv(level_of(4 * MIB as u64), 0, &msg[..4 * MIB]);
    let right = tree.last_cv((4 * MIB / CHUNK_LEN) as u64, &msg[4 * MIB..]);
    assert_eq!(tree.root_hash(&left, &right, msg.len() as u64).as_bytes(), &want);
}

/// Two values are equal when they are one subtree's and their bytes are
/// equal. `Debug` shows the place and never the bytes: a refusal's message
/// carries no chaining value of a keyed tree.
#[test]
fn equality_is_of_place_and_bytes_and_debug_shows_the_place() {
    let bytes = [0xabu8; 32];
    assert_eq!(Cv::at(3, 4, bytes), Cv::at(3, 4, bytes));
    assert_ne!(Cv::at(3, 4, bytes), Cv::at(3, 5, bytes), "another index");
    assert_ne!(Cv::at(3, 4, bytes), Cv::at(2, 4, bytes), "another level");
    assert_ne!(Cv::at(3, 4, bytes), Cv::at(3, 4, [0xac; 32]), "other bytes");
    assert_eq!(Cv::unset(), Cv::unset());
    // Zeroed memory is the unset value: a slot nothing wrote is refused.
    assert_eq!(unsafe { core::mem::zeroed::<Cv>() }, Cv::unset());
    assert_ne!(Cv::unset(), Cv::at(0, 0, [0; 32]));
    assert_eq!(Cv::at(3, 4, bytes).as_bytes(), &bytes);
    assert_eq!(LastCv::at(8, 100, bytes), LastCv::at(8, 100, bytes));
    assert_ne!(LastCv::at(8, 100, bytes), LastCv::at(8, 101, bytes), "another length");
    assert_ne!(LastCv::at(8, 100, bytes), LastCv::at(16, 100, bytes), "another first chunk");
    assert_ne!(LastCv::at(8, 100, bytes), LastCv::at(8, 100, [0xac; 32]), "other bytes");
    assert_eq!(LastCv::at(8, 100, bytes).as_bytes(), &bytes);
    assert_eq!(LastCv::from(Cv::at(3, 4, bytes)), LastCv::at(32, 8192, bytes), "a whole subtree as the last one keeps its place and bytes");
    assert_eq!(format!("{:?}", Cv::at(3, 4, bytes)), "the whole subtree at level 3 index 4");
    assert_eq!(format!("{:?}", Cv::unset()), "a value never set");
    assert_eq!(format!("{:?}", LastCv::at(8, 100, bytes)), "the last subtree, 100 bytes from chunk 8");
}

/// The sizes the pages give: a place is one word beside the 32 bytes.
#[test]
fn a_value_is_its_bytes_and_its_place() {
    assert_eq!(core::mem::size_of::<Cv>(), 40);
    assert_eq!(core::mem::size_of::<LastCv>(), 48);
}

/// A node of a message's tree, by an independent rule: `len` bytes from
/// the message's chunk `start`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
struct Node {
    start: u64,
    len: u64,
}

/// Every parent of the tree of a message of `len` bytes, as (parent's
/// start, left child, right child): BLAKE3's rule written out afresh.
/// The left child holds the largest power of two of chunks that leaves
/// the right child at least one byte.
fn parents_of(len: u64) -> Vec<(Node, Node)> {
    fn walk(node: Node, out: &mut Vec<(Node, Node)>) {
        if node.len <= 1024 {
            return;
        }
        let mut left = 1024;
        while 2 * left < node.len {
            left *= 2;
        }
        let (l, r) = (Node { start: node.start, len: left }, Node { start: node.start + left / 1024, len: node.len - left });
        out.push((l, r));
        walk(l, out);
        walk(r, out);
    }
    let mut out = Vec::new();
    walk(Node { start: 0, len }, &mut out);
    out
}

/// Whether `f` returns, with the panic's report kept off the test's output.
fn accepts<T>(f: impl FnOnce() -> T) -> bool {
    catch_unwind(AssertUnwindSafe(f)).is_ok()
}

/// The calls take exactly BLAKE3's trees: for every message of up to 16
/// chunks (each chunk count, its last chunk one byte, half full, or
/// full), every pair of values that can be made within 16 chunks is
/// taken by a join or by the root if and only if the two are children of
/// one parent in that message's tree, by a rule written apart from the
/// library's. A complete check of this bound: 31 whole subtrees and
/// every last subtree, in every pairing (9,889 pairings, 111 taken).
#[test]
fn the_calls_take_exactly_blake3s_trees_up_to_sixteen_chunks() {
    use std::collections::BTreeSet;
    blake3_servil_core::initialize();
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    const CHUNKS: u64 = 16;
    let tree = tree(false);
    let bytes = [0x5au8; 32];
    // Every whole subtree within 16 chunks, and its node.
    let mut wholes = Vec::new();
    for level in 0..=4u32 {
        for index in 0..CHUNKS >> level {
            wholes.push((Cv::at(level, index, bytes), Node { start: index << level, len: 1024 << level }));
        }
    }
    assert_eq!(wholes.len(), 31);
    // Every message length tried, and every last subtree `LastCv::at` makes within them.
    let lens: Vec<u64> = (0..CHUNKS).flat_map(|full| [full * 1024 + 1, full * 1024 + 512, full * 1024 + 1024]).collect();
    let mut lasts = Vec::new();
    for first_chunk in 0..CHUNKS {
        for &len in &lens {
            if len > first_chunk * 1024 && accepts(|| LastCv::at(first_chunk, len - first_chunk * 1024, bytes)) {
                lasts.push((LastCv::at(first_chunk, len - first_chunk * 1024, bytes), Node { start: first_chunk, len: len - first_chunk * 1024 }));
            }
        }
    }
    // The independent rule's pairs. Whole siblings are those of the full
    // 16-chunk message. A pair whose right child ends a message is a
    // parent on that message's right edge.
    let whole_pairs: BTreeSet<(Node, Node)> = parents_of(CHUNKS * 1024).into_iter().collect();
    let mut edge_pairs = BTreeSet::new();
    let mut roots = BTreeSet::new();
    for &len in &lens {
        let parents = parents_of(len);
        if let Some(&root) = parents.first() {
            roots.insert((root.0, root.1, len));
        }
        edge_pairs.extend(parents.into_iter().filter(|(_, r)| r.start * 1024 + r.len == len));
    }
    // Every node the rule names can be made: a right child is a `LastCv`.
    for (_, r) in &edge_pairs {
        assert!(lasts.iter().any(|(_, node)| node == r), "the rule's right child {r:?} cannot be made");
    }
    let (mut joins, mut taken) = (0, 0);
    for (a, na) in &wholes {
        for (b, nb) in &wholes {
            let ok = accepts(|| tree.parent_cv(a, b));
            assert_eq!(ok, whole_pairs.contains(&(*na, *nb)), "parent_cv of {na:?} and {nb:?}");
            assert_eq!(accepts(|| tree.parent_cvs(&[*a, *b], &mut [Cv::unset()])), ok, "parent_cvs of {na:?} and {nb:?}");
            if ok {
                let parent = tree.parent_cv(a, b);
                assert_eq!((parent.level(), parent.index() << parent.level()), (a.level() + 1, na.start), "the parent's place");
            }
            joins += 1;
            taken += ok as usize;
        }
        for (l, nl) in &lasts {
            let ok = accepts(|| tree.last_parent_cv(a, l));
            assert_eq!(ok, edge_pairs.contains(&(*na, *nl)), "last_parent_cv of {na:?} and {nl:?}");
            if ok {
                let parent = tree.last_parent_cv(a, l);
                assert_eq!((parent.first_chunk(), parent.len()), (na.start, na.len + nl.len), "the parent's place");
            }
            // The root takes the pair with the length it holds, where the
            // rule makes it a root's; and with no other length.
            let held = nl.start * 1024 + nl.len;
            assert_eq!(accepts(|| tree.root_hash(a, l, held)), roots.contains(&(*na, *nl, held)), "root_hash of {na:?} and {nl:?}");
            assert!(
                !accepts(|| tree.root_hash(a, l, held + 1)) && !accepts(|| tree.root_hash(a, l, held - 1)),
                "root_hash of {na:?} and {nl:?} with another length"
            );
            joins += 2;
            taken += ok as usize;
        }
    }
    std::panic::set_hook(hook);
    // The check is not empty: these many pairings, and the rule's every pair among the taken.
    assert_eq!(taken, whole_pairs.len() + edge_pairs.len());
    assert!(joins > 9_000 && taken > 100, "{joins} pairings, {taken} taken");
}
