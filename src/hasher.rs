//! The Hasher's stack of chaining values, its loop over the input, a
//! chunk's state, and a node's output, in safe Rust over their kernels.

use crate::{BLOCK_LEN, CHUNK_END, CHUNK_LEN, CHUNK_START, PARENT, ROOT};
use crate::{CVBytes as Cv, MAX_DEPTH};
const DEPTH: usize = MAX_DEPTH + 1;

/// The parents of chaining values: the stack's use of the kernels.
pub trait Parent {
    fn parent(&self, left: &Cv, right: &Cv) -> Cv;
    /// A layer of parents, side by side in the kernels' lanes: `out[i]`
    /// the parent of `children[2 * i]` and `children[2 * i + 1]`, for an
    /// even count of children from 2 to 2 * ROW. Returns their count.
    fn parents(&self, children: &[Cv], out: &mut [Cv]) -> usize;
}

/// The most values a row holds: x86-64 Linux's widest kernels' lanes. The
/// same on every target (the C header gives a hasher's size as one
/// number); kernels that cut wider (the SME2 unit's) make no rows.
pub const ROW: usize = 32;
/// The rows a stack holds at once (the sizes from a group of 16 chunks up
/// to 2 MiB); a row past them is condensed to its one value.
const ROWS: usize = 8;

/// The values of the subtrees finished so far, bottom first, each one
/// value or a row.
///
/// A row is a subtree's cut `n` wide (n a power of two, 4 to ROW): the
/// values of its n equal parts, in order. Two rows of equal subtrees
/// merge into the row of their parent in one call of n parents side by
/// side, where two single values merge by one parent alone; a stream fed
/// 16 KiB at a time spent a tenth of its time on parents hashed one, two,
/// four and eight at a time (Zooko's layout: a parent has its slot before
/// its children are known to pair, and children wait for a full batch).
/// A row is condensed to its one value when the output is computed, or
/// when it merges with a single value. A message's first subtree is never
/// a row: it may be the whole message, whose root needs its two halves,
/// and the kernels have their fastest path for that (`Feed::children`).
pub struct CvStack {
    /// The single values: entry i's is written before it is read (an
    /// entry is read below the length, after its push or its merge). The
    /// storage is never zeroed and never moved as a value of its own: a
    /// stack that started it as `None` with 1.7 KiB to follow was copied,
    /// those bytes and all, into every new Hasher on AArch64 (41 ns).
    cvs: core::mem::MaybeUninit<[Cv; DEPTH]>,
    len: usize,
    /// Bit i: entry i is a row, kept in `rows[i % ROWS]`.
    wide: u64,
    /// Each row's width.
    widths: [u8; ROWS],
    /// Bit s: the rows' place s has been written whole (zeroed as it was
    /// first given out).
    ready: u64,
    rows: core::mem::MaybeUninit<[[Cv; ROW]; ROWS]>,
}

impl Clone for CvStack {
    /// One literal: the rows' storage copied whole, once, into its place.
    /// (A clone made with empty storage and then given the rows in use
    /// is a value written to after it is made, which is copied again to
    /// where it goes.)
    fn clone(&self) -> CvStack {
        CvStack { cvs: self.cvs, len: self.len, wide: self.wide, widths: self.widths, ready: self.ready, rows: self.rows }
    }

    /// In place, only what the source holds: its entries' values, and the
    /// rows among them (not the 8 KiB of the rows' storage).
    fn clone_from(&mut self, source: &CvStack) {
        // Sound: `len` values of each array, plain bytes; entries that are
        // rows have stale single values, copied as bytes and never read.
        unsafe { core::ptr::copy_nonoverlapping(source.cvs.as_ptr() as *const Cv, self.cvs.as_mut_ptr() as *mut Cv, source.len) };
        self.len = source.len;
        self.wide = 0;
        for i in (0..source.len).filter(|&i| source.is_row(i)) {
            self.set_row(i, source.row(i));
        }
    }
}

impl CvStack {
    #[inline(always)]
    pub fn new() -> CvStack {
        CvStack { cvs: core::mem::MaybeUninit::uninit(), len: 0, wide: 0, widths: [0; ROWS], ready: 0, rows: core::mem::MaybeUninit::uninit() }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// Entry `i`'s single value, written.
    fn set(&mut self, i: usize, cv: Cv) {
        assert!(i < DEPTH, "the stack holds DEPTH entries");
        // Sound: within the array.
        unsafe { (self.cvs.as_mut_ptr() as *mut Cv).add(i).write(cv) };
    }

    fn is_row(&self, i: usize) -> bool {
        self.wide >> i & 1 == 1
    }

    /// Entry `i`'s row (it is one).
    fn row(&self, i: usize) -> &[Cv] {
        debug_assert!(self.is_row(i));
        let n = self.widths[i % ROWS] as usize;
        // Sound: a row's first `width` values were written when it was pushed or merged.
        unsafe { core::slice::from_raw_parts((self.rows.as_ptr() as *const Cv).add((i % ROWS) * ROW), n) }
    }

    fn set_row(&mut self, i: usize, row: &[Cv]) {
        let slot = i % ROWS;
        // Sound: MaybeUninit has the same layout as Cv and permits every
        // byte here to be unwritten. clone_from may write a fresh slot.
        let rows = unsafe { &mut *self.rows.as_mut_ptr().cast::<[[core::mem::MaybeUninit<Cv>; ROW]; ROWS]>() };
        rows[slot][..row.len()].write_copy_of_slice(row);
        self.widths[slot] = row.len() as u8;
        self.wide |= 1 << i;
    }

    /// Value `i`, below the length: a single entry's own, a row's
    /// condensed a layer of parents at a time down to its two halves,
    /// then their parent.
    pub fn get<P: Parent>(&self, p: &P, i: usize) -> Cv {
        assert!(i < self.len, "a value below the stack's length");
        if !self.is_row(i) {
            // Sound: a single entry below the length was written (push, merge, narrow).
            return unsafe { *(self.cvs.as_ptr() as *const Cv).add(i) };
        }
        let mut a = [[0u8; 32]; ROW];
        let mut b = [[0u8; 32]; ROW];
        let row = self.row(i);
        let mut n = row.len();
        assert!(n >= 4, "a row is 4 wide or more");
        a[..n].copy_from_slice(row);
        for _ in 0..n.trailing_zeros() - 1 {
            n = p.parents(&a[..n], &mut b);
            a[..n].copy_from_slice(&b[..n]);
        }
        assert_eq!(n, 2, "a row's two halves");
        p.parent(&a[0], &a[1])
    }

    /// `cv` on top; the stack holds fewer than DEPTH values.
    pub fn push(&mut self, cv: Cv) {
        self.set(self.len, cv);
        // (A bit at the length or above is left from an entry merged
        // away: each push sets its own.)
        self.wide &= !(1 << self.len);
        self.len += 1;
    }

    /// `row` on top, a subtree's cut, copied into its place: the tests'
    /// way to push one they hold.
    #[cfg(test)]
    pub fn push_row<P: Parent>(&mut self, p: &P, row: &[Cv]) {
        let place = self.place_on_top(p);
        place[..row.len()].copy_from_slice(row);
        self.pushed_row(row.len());
    }

    /// The place of a row about to go on top, for its values to be
    /// written into where they will stay ([`CvStack::pushed_row`] then
    /// says how many): a cut written elsewhere and copied here touched
    /// its 16 cache lines twice and a kilobyte of zeroed scratch besides.
    /// The rows below that keep theirs in the same place (ROWS, 2 ROWS,
    /// ... entries down) give it up, condensed to their one value. A
    /// place is zeroed the first time it is given out, once a stack: the
    /// kernels then write into memory that holds values.
    pub fn place_on_top<P: Parent>(&mut self, p: &P) -> &mut [Cv; ROW] {
        for below in (self.len % ROWS..self.len).step_by(ROWS) {
            if self.is_row(below) {
                self.narrow(p, below);
            }
        }
        let slot = self.len % ROWS;
        let place = unsafe { (self.rows.as_mut_ptr() as *mut [Cv; ROW]).add(slot) };
        if self.ready >> slot & 1 == 0 {
            // Sound: the slot's own storage.
            unsafe { place.write_bytes(0, 1) };
            self.ready |= 1 << slot;
        }
        // Sound: the slot's storage, every byte of it written (now or before).
        unsafe { &mut *place }
    }

    /// The row written into [`CvStack::place_on_top`]'s place, `width`
    /// values of it, is the top entry.
    pub fn pushed_row(&mut self, width: usize) {
        assert!((4..=ROW).contains(&width) && width.is_power_of_two(), "a row of 4 to ROW values, a power of two");
        self.widths[self.len % ROWS] = width as u8;
        self.wide |= 1 << self.len;
        self.len += 1;
    }

    /// Entry `i`, a row, becomes the row half as wide: its values' parents.
    fn halve<P: Parent>(&mut self, p: &P, i: usize) {
        assert!(self.row(i).len() >= 8, "a row stays 4 wide or more");
        let mut parents = [[0u8; 32]; ROW];
        let made = p.parents(self.row(i), &mut parents);
        assert_eq!(2 * made, self.row(i).len(), "a parent for every two children");
        self.set_row(i, &parents[..made]);
    }

    /// Entry `i`, a row, becomes its one value.
    fn narrow<P: Parent>(&mut self, p: &P, i: usize) {
        let cv = self.get(p, i);
        self.set(i, cv);
        self.wide &= !(1 << i);
    }

    #[cfg(any(feature = "std", test))]
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// How many entries are rows.
    #[cfg(test)]
    fn rows_in_use(&self) -> usize {
        (0..self.len).filter(|&i| self.is_row(i)).count()
    }

    /// While the stack holds more than `target` entries, its top two become
    /// their parent (merge_cv_stack): two rows of one width, the row of
    /// their parents; any other two, the parent of their values. Two are
    /// there to merge whenever `target` is at least 1, as the Hasher's are.
    /// A loop of a count fixed at its start (no condition a change could
    /// keep true forever: a mutation test's endless loop, which only a clock
    /// stops, becomes a wrong value a test catches).
    pub fn merge<P: Parent>(&mut self, p: &P, target: usize) {
        for _ in target..self.len {
            assert!(self.len >= 2, "two values to merge");
            let (a, b) = (self.len - 2, self.len - 1);
            if self.is_row(a) && self.is_row(b) {
                // Two rows of different widths (one subtree cut by wider
                // kernels than its sibling was, or cut whole where the
                // sibling grew from narrower rows): the wider is halved
                // to the other's width first.
                // (Widths are powers of two: the wider is the other's
                // width times two to the count of halvings.)
                for (wide, narrow) in [(a, b), (b, a)] {
                    // (The narrower over the wider is 0: no halving.)
                    let times = self.row(wide).len() / self.row(narrow).len();
                    for _ in 0..times.max(1).trailing_zeros() {
                        self.halve(p, wide);
                    }
                }
                // The two rows' values in one place, then their parents
                // into the lower row's place. The place for the children
                // is written before it is read, and never zeroed first:
                // zeroing 2 KiB a merge cost more than the merge saved.
                let (left, right) = (self.row(a), self.row(b));
                let n = left.len();
                assert_eq!(n, right.len(), "two rows merge at one width");
                let mut children = core::mem::MaybeUninit::<[Cv; 2 * ROW]>::uninit();
                let at = children.as_mut_ptr() as *mut Cv;
                // Sound: 2n values written into room for 2 ROW, then read.
                let children = unsafe {
                    core::ptr::copy_nonoverlapping(left.as_ptr(), at, n);
                    core::ptr::copy_nonoverlapping(right.as_ptr(), at.add(n), n);
                    core::slice::from_raw_parts(at, 2 * n)
                };
                // Sound: row a's place, its first n values written (they are row a).
                let place = unsafe { core::slice::from_raw_parts_mut((self.rows.as_mut_ptr() as *mut Cv).add((a % ROWS) * ROW), n) };
                let made = p.parents(children, place);
                assert_eq!(made, n, "a parent for every two children");
            } else {
                let parent = p.parent(&self.get(p, a), &self.get(p, b));
                self.set(a, parent);
                self.wide &= !(1 << a);
            }
            self.len -= 1;
        }
    }
}

/// A chunk's state: its number and the bytes it has so far (at most
/// CHUNK_LEN).
pub trait Chunk {
    fn counter(&self) -> u64;
    fn len(&self) -> usize;
    /// The next bytes, at most CHUNK_LEN - len() of them.
    fn update(&mut self, input: &[u8]);
    /// An empty chunk's number, advanced by `chunks`.
    fn advance(&mut self, chunks: u64);
}

/// The kernels the Hasher's loop calls, beyond the stack's `Parent`.
pub trait Feed<C: Chunk>: Parent {
    /// An empty chunk, numbered `counter`.
    fn new_chunk(&self, counter: u64) -> C;
    /// A full chunk's chaining value.
    fn chunk_cv(&self, chunk: &C) -> Cv;
    /// The chaining value of `chunk` (CHUNK_LEN bytes), numbered `counter`.
    fn whole_chunk_cv(&self, chunk: &[u8], counter: u64) -> Cv;
    /// The chaining value of the subtree over `input` (more than one chunk),
    /// its first chunk numbered `counter`, computed another way where the
    /// kernels have one (None: they have none for it). `ahead` bytes follow.
    fn subtree_cv(&self, input: &[u8], ahead: usize, counter: u64) -> Option<Cv>;
    /// The chaining values of the two children of the subtree over `input`
    /// (more than one chunk), its first chunk numbered `counter`.
    fn children(&self, input: &[u8], ahead: usize, counter: u64) -> [Cv; 2];
    /// Whether the kernels give a cut of a subtree of `len` bytes.
    fn cuts(&self, len: usize) -> bool;
    /// The subtree's cut into `out`, its width returned, 4 or more (asked
    /// only where `cuts` says so).
    fn cut(&self, input: &[u8], ahead: usize, counter: u64, out: &mut [Cv; ROW]) -> usize;
}

/// The nodes a Hasher's output is computed from.
pub trait Nodes<C: Chunk, N> {
    /// The node of `chunk`, the last.
    fn chunk_node(&self, chunk: &C) -> N;
    /// The parent node of two chaining values.
    fn parent_node(&self, left: &Cv, right: &Cv) -> N;
    /// A node's chaining value.
    fn node_cv(&self, node: &N) -> Cv;
}

/// The number of 1-bits of `x` (its popcount, which the loop this replaced
/// compiled to anyway; with no loop, no change to it can run forever).
pub fn ones(x: u64) -> usize {
    x.count_ones() as usize
}

/// A whole subtree's length to hash next, from chunk `counter` with
/// `remaining` (more than CHUNK_LEN) bytes to go: the largest power of two
/// at most `remaining` that divides the bytes so far, CHUNK_LEN times a
/// power of two dividing `counter`.
/// A power of two divides `so_far` exactly when it is at most its lowest
/// 1-bit, so the length is the smaller of the two, with no loop (one that
/// halved until it divided could, changed, run forever).
pub fn subtree_len(counter: u64, remaining: usize) -> usize {
    let len = 1usize << (usize::BITS - 1 - remaining.leading_zeros());
    let so_far = counter * CHUNK_LEN as u64;
    if so_far == 0 {
        len
    } else {
        // The lowest 1-bit as a mask, compared in u64: no shift to bound,
        // and the result is at most `len`, so it fits a usize.
        (len as u64).min(so_far & so_far.wrapping_neg()) as usize
    }
}

/// `cv`, the subtree from chunk `counter`, onto the stack: the stack's
/// values merged to the subtrees before it first (lazily: the top value
/// may still merge with the next).
pub fn push<C: Chunk, K: Feed<C>>(k: &K, stack: &mut CvStack, initial: u64, cv: Cv, counter: u64) {
    stack.merge(k, ones(counter - initial));
    stack.push(cv);
}

/// The partial chunk filled from `input`; when more input follows, the full
/// chunk goes onto the stack and a new chunk begins. Returns the rest.
pub fn fill<'a, C: Chunk, K: Feed<C>>(k: &K, chunk: &mut C, stack: &mut CvStack, initial: u64, input: &'a [u8]) -> &'a [u8] {
    if chunk.len() == 0 {
        return input;
    }
    let take = if CHUNK_LEN - chunk.len() < input.len() { CHUNK_LEN - chunk.len() } else { input.len() };
    chunk.update(&input[..take]);
    let rest = &input[take..];
    if rest.is_empty() {
        return rest;
    }
    let counter = chunk.counter();
    push(k, stack, initial, k.chunk_cv(chunk), counter);
    *chunk = k.new_chunk(counter + 1);
    rest
}

/// `input` after an empty chunk: whole subtrees onto the stack, then at most
/// a chunk into the chunk, and the stack merged behind it.
pub fn feed<C: Chunk, K: Feed<C>>(k: &K, chunk: &mut C, stack: &mut CvStack, initial: u64, input: &[u8]) {
    let mut input = input;
    while input.len() > CHUNK_LEN {
        let counter = chunk.counter();
        let len = subtree_len(counter, input.len());
        let chunks = (len / CHUNK_LEN) as u64;
        let ahead = input.len() - len;
        let sub = &input[..len];
        let flat = if counter > initial { k.subtree_cv(sub, ahead, counter) } else { None };
        match flat {
            Some(cv) => push(k, stack, initial, cv, counter),
            None => {
                if len <= CHUNK_LEN {
                    push(k, stack, initial, k.whole_chunk_cv(sub, counter), counter);
                } else if counter > initial && k.cuts(len) {
                    // The merges the stack owes first, then the cut,
                    // written into the place it keeps.
                    stack.merge(k, ones(counter - initial));
                    let n = k.cut(sub, ahead, counter, stack.place_on_top(k));
                    stack.pushed_row(n);
                } else {
                    let [left, right] = k.children(sub, ahead, counter);
                    push(k, stack, initial, left, counter);
                    push(k, stack, initial, right, counter + chunks / 2);
                }
            }
        }
        chunk.advance(chunks);
        input = &input[len..];
    }
    if !input.is_empty() {
        chunk.update(input);
        stack.merge(k, ones(chunk.counter() - initial));
    }
}

/// The root's node: the last chunk's (or the top two values' parent),
/// folded into the stack's values from the top.
pub fn final_node<C: Chunk, N, K: Nodes<C, N> + Parent>(k: &K, chunk: &C, stack: &CvStack) -> N {
    if stack.len() == 0 {
        return k.chunk_node(chunk);
    }
    let mut n = stack.len();
    // (With an empty chunk the stack holds two entries or more: a
    // message's first subtree goes onto it as its two halves, never as
    // one row, so the root's two children are there to take.)
    let mut out = if chunk.len() > 0 {
        k.chunk_node(chunk)
    } else {
        n -= 2;
        k.parent_node(&stack.get(k, n), &stack.get(k, n + 1))
    };
    while n > 0 {
        n -= 1;
        out = k.parent_node(&stack.get(k, n), &k.node_cv(&out));
    }
    out
}

/// The root's node, for a message that goes on past the stack's subtrees:
/// `out` is the node of everything after them (a whole subtree, shorter
/// than the stack's last), and the stack's entries stand for `target`
/// subtrees or more. Entries beyond `target` are the merges the stack
/// has put off (it merges lazily): the top ones become one value first,
/// folded from the top, and that value's parent with `out` is folded
/// into the rest. The stack is only read.
pub fn above<C: Chunk, N, K: Nodes<C, N> + Parent>(k: &K, stack: &CvStack, target: usize, out: N) -> N {
    assert!(target >= 1 && stack.len() >= target, "a subtree or more before the rest, each with its entry");
    let mut top = stack.get(k, stack.len() - 1);
    for i in (target - 1..stack.len() - 1).rev() {
        top = k.parent(&stack.get(k, i), &top);
    }
    let mut out = k.parent_node(&top, &k.node_cv(&out));
    for i in (0..target - 1).rev() {
        out = k.parent_node(&stack.get(k, i), &k.node_cv(&out));
    }
    out
}

/// What the Hasher's kernels take from the library beyond the compression
/// function: Feed's methods of those names, as Feed describes them.
pub trait Hooks {
    fn subtree_cv(&self, input: &[u8], ahead: usize, counter: u64) -> Option<Cv>;
    fn children(&self, input: &[u8], ahead: usize, counter: u64) -> [Cv; 2];
    fn cuts(&self, len: usize) -> bool;
    fn cut(&self, input: &[u8], ahead: usize, counter: u64, out: &mut [Cv; ROW]) -> usize;
    fn parents(&self, children: &[Cv], out: &mut [Cv]) -> usize;
}

/// The Hasher's kernels: chunks, nodes, and parents on `key`, `flags`, and
/// `platform`, and the rest from `hooks`.
pub struct Kernels<'a, P, H> {
    pub key: &'a [u32; 8],
    pub flags: u8,
    pub platform: P,
    pub hooks: H,
}

impl<P: Blocks + Copy, H: Hooks> Parent for Kernels<'_, P, H> {
    #[inline(always)]
    fn parent(&self, left: &Cv, right: &Cv) -> Cv {
        parent_output(left, right, self.key, self.flags, self.platform).chaining_value()
    }

    #[inline(always)]
    fn parents(&self, children: &[Cv], out: &mut [Cv]) -> usize {
        self.hooks.parents(children, out)
    }
}

impl<P: Blocks + Copy, H: Hooks> Feed<ChunkState<P>> for Kernels<'_, P, H> {
    #[inline(always)]
    fn new_chunk(&self, counter: u64) -> ChunkState<P> {
        ChunkState::new(self.key, counter, self.flags, self.platform)
    }

    #[inline(always)]
    fn chunk_cv(&self, chunk: &ChunkState<P>) -> Cv {
        chunk.output().chaining_value()
    }

    #[inline(always)]
    fn whole_chunk_cv(&self, chunk: &[u8], counter: u64) -> Cv {
        let mut state = ChunkState::new(self.key, counter, self.flags, self.platform);
        state.absorb(chunk);
        state.output().chaining_value()
    }

    #[inline(always)]
    fn subtree_cv(&self, input: &[u8], ahead: usize, counter: u64) -> Option<Cv> {
        self.hooks.subtree_cv(input, ahead, counter)
    }

    #[inline(always)]
    fn children(&self, input: &[u8], ahead: usize, counter: u64) -> [Cv; 2] {
        self.hooks.children(input, ahead, counter)
    }

    #[inline(always)]
    fn cuts(&self, len: usize) -> bool {
        self.hooks.cuts(len)
    }

    #[inline(always)]
    fn cut(&self, input: &[u8], ahead: usize, counter: u64, out: &mut [Cv; ROW]) -> usize {
        self.hooks.cut(input, ahead, counter, out)
    }
}

impl<P: Blocks + Copy, H: Hooks> Nodes<ChunkState<P>, Output<P>> for Kernels<'_, P, H> {
    #[inline(always)]
    fn chunk_node(&self, chunk: &ChunkState<P>) -> Output<P> {
        chunk.output()
    }

    #[inline(always)]
    fn parent_node(&self, left: &Cv, right: &Cv) -> Output<P> {
        parent_output(left, right, self.key, self.flags, self.platform)
    }

    #[inline(always)]
    fn node_cv(&self, node: &Output<P>) -> Cv {
        node.chaining_value()
    }
}

/// The compression kernels a chunk's state calls.
pub trait Blocks {
    /// `cv` compressed with `block`, its first `block_len` bytes the
    /// message's and the rest zero.
    fn compress_in_place(&self, cv: &mut [u32; 8], block: &[u8; BLOCK_LEN], block_len: u8, counter: u64, flags: u8);
    /// `cv` compressed with the `count` (1 to 16) whole blocks at the start
    /// of `blocks` in turn: the first with `flags | flags_start`, the
    /// others with `flags`.
    fn compress_blocks(&self, cv: &mut [u32; 8], blocks: &[u8], count: usize, counter: u64, flags: u8, flags_start: u8);
}

/// A chunk's state: the chaining value after its blocks so far but the
/// last, which waits in `buf` (zero past `buf_len`) for the output. `buf`
/// comes first and the state is 64-byte aligned, as Output's block (below)
/// explains.
#[repr(C, align(64))]
pub struct ChunkState<P> {
    pub buf: [u8; BLOCK_LEN],
    pub cv: [u32; 8],
    pub chunk_counter: u64,
    pub buf_len: u8,
    pub blocks_compressed: u8,
    pub flags: u8,
    pub platform: P,
}

impl<P: Blocks> ChunkState<P> {
    pub fn new(key: &[u32; 8], chunk_counter: u64, flags: u8, platform: P) -> Self {
        ChunkState { buf: [0; BLOCK_LEN], cv: *key, chunk_counter, buf_len: 0, blocks_compressed: 0, flags, platform }
    }

    /// The bytes so far.
    pub fn count(&self) -> usize {
        BLOCK_LEN * self.blocks_compressed as usize + self.buf_len as usize
    }

    /// As much of `input` as `buf` has room for, into it; returns how much.
    fn fill_buf(&mut self, input: &[u8]) -> usize {
        let want = BLOCK_LEN - self.buf_len as usize;
        let take = if want < input.len() { want } else { input.len() };
        let start = self.buf_len as usize;
        self.buf[start..start + take].copy_from_slice(&input[..take]);
        self.buf_len += take as u8;
        take
    }

    pub fn start_flag(&self) -> u8 {
        if self.blocks_compressed == 0 { CHUNK_START } else { 0 }
    }

    /// The next bytes, `count() + input.len()` at most CHUNK_LEN. Whole
    /// blocks compress straight from `input`; the last block always waits.
    pub fn absorb(&mut self, input: &[u8]) {
        let mut input = input;
        if self.buf_len > 0 {
            let take = self.fill_buf(input);
            input = &input[take..];
            if !input.is_empty() {
                let block_flags = self.flags | self.start_flag();
                self.platform.compress_in_place(&mut self.cv, &self.buf, BLOCK_LEN as u8, self.chunk_counter, block_flags);
                self.buf_len = 0;
                self.buf = [0; BLOCK_LEN];
                self.blocks_compressed += 1;
            }
        }
        if input.len() > BLOCK_LEN {
            let count = (input.len() - 1) / BLOCK_LEN;
            let start_flag = self.start_flag();
            self.platform.compress_blocks(&mut self.cv, input, count, self.chunk_counter, self.flags, start_flag);
            self.blocks_compressed += count as u8;
            input = &input[count * BLOCK_LEN..];
        }
        self.fill_buf(input);
    }
}

/// A node: the inputs of its last compression. `block` comes first and the
/// node is 64-byte aligned: glibc's AVX-512 memcpy writes a 64-byte copy
/// with a pair of overlapping unaligned 64-byte vector stores, and the
/// immediate reload of the destination then fails store-to-load forwarding
/// whenever the stack frame leaves it misaligned (address & 31 != 0). Stack
/// layout decides which case each process gets, so the same binary hashed
/// 64-byte inputs 35-45% slower in some processes than in others (Zen 5).
/// Aligning the block pins every process to the forwarding-friendly case
/// (<https://github.com/zooko/bench-hashes/issues/2>,
/// <https://github.com/BLAKE3-team/BLAKE3/pull/582>).
#[repr(C, align(64))]
pub struct Output<P> {
    pub block: [u8; BLOCK_LEN],
    pub input_chaining_value: [u32; 8],
    pub block_len: u8,
    pub counter: u64,
    pub flags: u8,
    pub platform: P,
}

/// The bytes of `words`, least significant first.
pub fn le_bytes(words: &[u32; 8]) -> [u8; 32] {
    // Each word into its four bytes' slot: no index or count to get wrong
    // (a counted loop, changed, could run forever).
    let mut out = [0u8; 32];
    for (bytes, word) in out.chunks_exact_mut(4).zip(words) {
        bytes.copy_from_slice(&word.to_le_bytes());
    }
    out
}

impl<P: Blocks> Output<P> {
    /// The node's chaining value.
    pub fn chaining_value(&self) -> [u8; 32] {
        let mut cv = self.input_chaining_value;
        self.platform.compress_in_place(&mut cv, &self.block, self.block_len, self.counter, self.flags);
        le_bytes(&cv)
    }

    /// The first 32 bytes of the root's output (the node the root, its
    /// counter 0).
    #[inline]
    pub fn root_bytes(&self) -> [u8; 32] {
        let mut cv = self.input_chaining_value;
        self.platform.compress_in_place(&mut cv, &self.block, self.block_len, 0, self.flags | ROOT);
        le_bytes(&cv)
    }
}

impl<P: Blocks + Copy> ChunkState<P> {
    /// The chunk's node.
    pub fn output(&self) -> Output<P> {
        Output {
            block: self.buf,
            input_chaining_value: self.cv,
            block_len: self.buf_len,
            counter: self.chunk_counter,
            flags: self.flags | self.start_flag() | CHUNK_END,
            platform: self.platform,
        }
    }
}

/// The parent node of two chaining values.
pub fn parent_output<P>(left: &[u8; 32], right: &[u8; 32], key: &[u32; 8], flags: u8, platform: P) -> Output<P> {
    let mut block = [0u8; BLOCK_LEN];
    block[..32].copy_from_slice(left);
    block[32..].copy_from_slice(right);
    Output { block, input_chaining_value: *key, block_len: BLOCK_LEN as u8, counter: 0, flags: flags | PARENT, platform }
}

/// The chunk state the Hasher's loop (above) works with.
impl<P: Blocks> Chunk for ChunkState<P> {
    #[inline(always)]
    fn counter(&self) -> u64 {
        self.chunk_counter
    }

    #[inline(always)]
    fn len(&self) -> usize {
        self.count()
    }

    #[inline(always)]
    fn update(&mut self, input: &[u8]) {
        self.absorb(input)
    }

    #[inline(always)]
    fn advance(&mut self, chunks: u64) {
        self.chunk_counter += chunks
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::platform::Platform;
    use core::cell::Cell;

    /// The platform's compressions, counted: blocks compressed from the
    /// buffer one at a time, and calls of the multi-block kernel.
    struct Counting {
        platform: Platform,
        buffered: Cell<usize>,
        runs: Cell<usize>,
    }

    impl Blocks for &Counting {
        fn compress_in_place(&self, cv: &mut [u32; 8], block: &[u8; BLOCK_LEN], block_len: u8, counter: u64, flags: u8) {
            self.buffered.set(self.buffered.get() + 1);
            Blocks::compress_in_place(&self.platform, cv, block, block_len, counter, flags)
        }
        fn compress_blocks(&self, cv: &mut [u32; 8], blocks: &[u8], count: usize, counter: u64, flags: u8, flags_start: u8) {
            self.runs.set(self.runs.get() + 1);
            Blocks::compress_blocks(&self.platform, cv, blocks, count, counter, flags, flags_start)
        }
    }

    /// Hooks that record which subtrees feed asks for flat (subtree_cv) and
    /// answer none, so every subtree then takes the children path.
    struct Asking {
        platform: Platform,
        asked: core::cell::RefCell<std::vec::Vec<u64>>,
        seen: core::cell::RefCell<std::vec::Vec<u64>>,
    }

    impl Hooks for &Asking {
        fn subtree_cv(&self, _input: &[u8], _ahead: usize, counter: u64) -> Option<Cv> {
            self.asked.borrow_mut().push(counter);
            None
        }
        fn children(&self, input: &[u8], ahead: usize, counter: u64) -> [Cv; 2] {
            self.seen.borrow_mut().push(counter);
            let node = crate::compress_subtree_to_parent_node(input, ahead, crate::IV, counter, 0, self.platform);
            [node[..32].try_into().unwrap(), node[32..].try_into().unwrap()]
        }
        fn cuts(&self, _len: usize) -> bool {
            false
        }
        fn cut(&self, _input: &[u8], _ahead: usize, _counter: u64, _out: &mut [Cv; ROW]) -> usize {
            unreachable!("no subtree is cut")
        }
        fn parents(&self, children: &[Cv], out: &mut [Cv]) -> usize {
            crate::compress_parents_parallel(children.as_flattened(), crate::IV, 0, self.platform, out.as_flattened_mut())
        }
    }

    /// feed asks the kernels for a flat subtree (subtree_cv) for every
    /// subtree but a fresh hasher's first, whose counter is the initial one:
    /// that one takes the children path whatever the kernels offer. The
    /// digest is the same either way (flat or children), so the asks are
    /// counted (BLAKE3 #219, #238).
    #[test]
    fn test_feed_asks_for_a_flat_subtree_after_the_first() {
        let platform = Platform::detect();
        // Nine chunks and a part: from chunk 0 the subtrees are 8, then the last chunk waits.
        let input: std::vec::Vec<u8> = (0..(9 * crate::CHUNK_LEN + 100) as u32).map(|i| (i % 251) as u8).collect();
        for initial in [0u64, 3, 6] {
            let hooks = Asking { platform, asked: Default::default(), seen: Default::default() };
            let k = Kernels { key: crate::IV, flags: 0, platform, hooks: &hooks };
            let mut chunk = k.new_chunk(initial);
            let mut stack = CvStack::new();
            feed(&k, &mut chunk, &mut stack, initial, &input);
            let asked = hooks.asked.borrow().clone();
            assert!(!asked.is_empty() && asked.iter().all(|&c| c > initial), "from chunk {initial}: asked for every subtree after the first, {asked:?}");
            assert!(!asked.contains(&initial), "from chunk {initial}: the first subtree is never asked for flat");
            // Every subtree of more than one chunk after the first went to children too (none was flat).
            for c in hooks.seen.borrow().iter().filter(|&&c| c != initial) {
                assert!(asked.contains(c), "from chunk {initial}: subtree {c} asked before its children");
            }
        }
    }

    /// Hooks that cut a whole group or more into a row `width(len)` wide
    /// (0: no row, the children path), and record every call of parents
    /// (its children's count) and of children (its subtree's length).
    struct Rows<W: Fn(usize) -> usize> {
        platform: Platform,
        width: W,
        parents: core::cell::RefCell<std::vec::Vec<usize>>,
        children: core::cell::RefCell<std::vec::Vec<usize>>,
    }

    impl<W: Fn(usize) -> usize> Hooks for &Rows<W> {
        fn subtree_cv(&self, _input: &[u8], _ahead: usize, _counter: u64) -> Option<Cv> {
            None
        }
        fn children(&self, input: &[u8], ahead: usize, counter: u64) -> [Cv; 2] {
            self.children.borrow_mut().push(input.len());
            let node = crate::compress_subtree_to_parent_node(input, ahead, crate::IV, counter, 0, self.platform);
            [node[..32].try_into().unwrap(), node[32..].try_into().unwrap()]
        }
        fn cuts(&self, len: usize) -> bool {
            (self.width)(len) >= 4 && len >= 16 * CHUNK_LEN
        }
        /// The cut of the width asked for, whatever the machine's kernels
        /// (a row narrower or wider than theirs, where a test wants one):
        /// each of the subtree's equal parts hashed to its own value.
        fn cut(&self, input: &[u8], ahead: usize, counter: u64, out: &mut [Cv; ROW]) -> usize {
            let want = (self.width)(input.len());
            let part = input.len() / want;
            assert!(part >= CHUNK_LEN && want <= ROW, "a part is a chunk or more");
            for (i, bytes) in input.chunks(part).enumerate() {
                let at = counter + (i * part / CHUNK_LEN) as u64;
                out[i] = if part == CHUNK_LEN {
                    let mut state = ChunkState::new(crate::IV, at, 0, self.platform);
                    state.absorb(bytes);
                    state.output().chaining_value()
                } else {
                    let node = crate::compress_subtree_to_parent_node(bytes, ahead + input.len() - (i + 1) * part, crate::IV, at, 0, self.platform);
                    parent_output(node[..32].try_into().unwrap(), node[32..].try_into().unwrap(), crate::IV, 0, self.platform).chaining_value()
                };
            }
            want
        }
        fn parents(&self, children: &[Cv], out: &mut [Cv]) -> usize {
            self.parents.borrow_mut().push(children.len());
            crate::compress_parents_parallel(children.as_flattened(), crate::IV, 0, self.platform, out.as_flattened_mut())
        }
    }

    /// `input` fed in `pieces` (their lengths in turn, the last repeated)
    /// to a hasher of rows `width` wide: its root hash, and the stack and
    /// the hooks' records as the input ended.
    fn fed<W: Fn(usize) -> usize>(input: &[u8], pieces: &[usize], width: W) -> ([u8; 32], CvStack, std::vec::Vec<usize>, std::vec::Vec<usize>) {
        let hooks = Rows { platform: Platform::detect(), width, parents: Default::default(), children: Default::default() };
        let k = Kernels { key: crate::IV, flags: 0, platform: hooks.platform, hooks: &hooks };
        let mut chunk = k.new_chunk(0);
        let mut stack = CvStack::new();
        let (mut at, mut turn) = (0, 0);
        while at < input.len() {
            let n = pieces[turn.min(pieces.len() - 1)].min(input.len() - at);
            turn += 1;
            let rest = fill(&k, &mut chunk, &mut stack, 0, &input[at..at + n]);
            feed(&k, &mut chunk, &mut stack, 0, rest);
            at += n;
        }
        let (parents, children) = (hooks.parents.borrow().clone(), hooks.children.borrow().clone());
        let root = final_node(&k, &chunk, &stack).root_bytes();
        (root, stack, parents, children)
    }

    /// The root is the message's hash whatever the stack holds: rows of
    /// the kernels' width, rows of 4, rows of widths that differ from one
    /// subtree to the next, or single values; at lengths around a group
    /// and around powers of two of groups, ragged ones, and one of 511
    /// groups and a part (nine entries, each a row's subtree); fed whole,
    /// a group at a time, in pieces off the boundaries, and in pieces
    /// that cross subtrees of every size.
    #[test]
    fn rows_of_any_width_and_single_values_give_the_message_its_hash() {
        const GROUP: usize = 16 * CHUNK_LEN;
        let input: std::vec::Vec<u8> = (0..511 * GROUP + 5000).map(|i| (i % 251) as u8).collect();
        let mut lens = std::vec![
            1,
            CHUNK_LEN,
            GROUP - 1,
            GROUP,
            GROUP + 1,
            2 * GROUP,
            2 * GROUP + CHUNK_LEN,
            3 * GROUP,
            4 * GROUP,
            5 * GROUP + 77,
            8 * GROUP,
            16 * GROUP - 1
        ];
        lens.extend([31 * GROUP, 32 * GROUP, 33 * GROUP + 1, 64 * GROUP, 100 * GROUP + 3 * CHUNK_LEN + 9, 256 * GROUP, input.len()]);
        for len in lens {
            let message = &input[..len];
            let want = *crate::convenience::hash(message).as_bytes();
            // The last pattern puts a subtree grown from two groups' rows
            // beside one of two groups cut whole: rows of two widths to
            // merge, where a row's width goes by its subtree's size.
            for pieces in [
                &[usize::MAX][..],
                &[GROUP],
                &[4 * GROUP],
                &[21845],
                &[GROUP + CHUNK_LEN, 7 * GROUP],
                &[3 * GROUP, GROUP, 64 * GROUP, 1],
                &[4 * GROUP, GROUP, GROUP, 2 * GROUP, 8 * GROUP],
            ] {
                // 511 groups a few bytes at a time would be thousands of calls of nothing new.
                if len > 300 * GROUP && pieces[0] == 21845 {
                    continue;
                }
                assert_eq!(
                    fed(message, pieces, |sub| ROW.min(sub / CHUNK_LEN)).0,
                    want,
                    "rows as wide as may be (16 for a group, 32 from two): {len} bytes in pieces of {pieces:?}"
                );
                assert_eq!(fed(message, pieces, |_| 4).0, want, "rows of 4: {len} bytes in pieces of {pieces:?}");
                assert_eq!(
                    fed(message, pieces, |sub| if sub % (3 * GROUP) < 2 * GROUP { 16 } else { 4 }).0,
                    want,
                    "rows of 16 and of 4: {len} bytes in pieces of {pieces:?}"
                );
                assert_eq!(
                    fed(message, pieces, |sub| if sub == GROUP { 8 } else { 0 }).0,
                    want,
                    "rows for single groups alone: {len} bytes in pieces of {pieces:?}"
                );
                assert_eq!(fed(message, pieces, |_| 0).0, want, "no rows: {len} bytes in pieces of {pieces:?}");
            }
        }
    }

    /// What the rows are for, counted (the digest is the same either way):
    /// 64 groups fed a group at a time. The first asks for its two
    /// children, as a message's first subtree does (it may be the whole
    /// message, which that path hashes fastest); the 63 after make rows.
    /// Two rows merge in one call of a full layer of parents, 52 times;
    /// the five subtrees beside the first's values (of 1, 2, 4, 8 and 16
    /// groups) are each condensed to one value to merge with them, a call
    /// a layer. Seven entries are left (the stack merges lazily), six of
    /// them rows. Without rows every group asks for its children.
    #[test]
    fn a_stream_of_groups_merges_rows_a_full_layer_at_a_time() {
        const GROUP: usize = 16 * CHUNK_LEN;
        let input: std::vec::Vec<u8> = (0..64 * GROUP).map(|i| (i % 251) as u8).collect();
        let width = Platform::detect().simd_degree().min(16);
        if width < 4 {
            return;
        }
        let (root, stack, parents, children) = fed(&input, &[GROUP], |_| width);
        assert_eq!(root, *crate::convenience::hash(&input).as_bytes());
        assert_eq!((stack.len(), stack.rows_in_use()), (7, 6), "64 groups: the subtrees of 32, 16, 8, 4, 2, 1 and 1 groups, all rows but the first");
        assert_eq!(children, [GROUP], "the first group alone");
        assert_eq!(parents.iter().filter(|&&n| n == 2 * width).count(), 52, "two rows' children at a time: {parents:?}");
        // A row condensed to its two halves: layers of width, width / 2, ... 4 children.
        let layers = width.trailing_zeros() as usize - 1;
        assert_eq!(parents.iter().filter(|&&n| n < 2 * width).count(), 5 * layers, "five rows condensed: {parents:?}");
        let (_, stack, parents, children) = fed(&input, &[GROUP], |_| 0);
        assert_eq!((stack.len(), stack.rows_in_use()), (8, 0), "the last group's two halves wait apart");
        assert_eq!((parents.len(), children.len()), (0, 64), "no rows: no layer of parents, and every group's two children");
    }

    /// Rows merge by the chunks since the hasher's first, not by the
    /// counter. A subtree that begins at chunk 64, hashed on its own a
    /// group at a time, owes its second group one merge (16 chunks in),
    /// where the counter, 80, has two bits set. Rows of four, whatever
    /// the machine's kernels cut: an SME2 Mac's cut none, so no test
    /// there told `counter - initial` from `counter + initial`. Against
    /// official BLAKE3's value for the subtree.
    #[test]
    fn rows_of_a_subtree_that_begins_past_chunk_zero_merge_by_its_own_chunks() {
        use blake3::hazmat::HasherExt;
        const GROUP: usize = 16 * CHUNK_LEN;
        const FIRST: u64 = 64;
        let input: std::vec::Vec<u8> = (0..4 * GROUP).map(|i| (i % 251) as u8).collect();
        let hooks = Rows { platform: Platform::detect(), width: |_| 4, parents: Default::default(), children: Default::default() };
        let k = Kernels { key: crate::IV, flags: 0, platform: hooks.platform, hooks: &hooks };
        let mut chunk = k.new_chunk(FIRST);
        let mut stack = CvStack::new();
        for group in input.chunks_exact(GROUP) {
            feed(&k, &mut chunk, &mut stack, FIRST, group);
        }
        assert_eq!(hooks.children.borrow().as_slice(), [GROUP], "the first group alone asks for its children: the three after are rows");
        let want = blake3::Hasher::new().set_input_offset(FIRST * CHUNK_LEN as u64).update(&input).finalize_non_root();
        assert_eq!(final_node(&k, &chunk, &stack).chaining_value(), want);
    }

    /// A row gives up its place to the row ROWS entries above it, whatever
    /// lies between and wherever the two stand: two single values, then
    /// nine rows (entries 2 to 10), and the first row, entry 2, is
    /// condensed as the ninth arrives; every entry's value is then the one
    /// its row or value had. Three more rows (to entry 13) condense
    /// entries 3 to 5 in turn.
    #[test]
    fn a_row_gives_up_its_place_to_the_row_rows_above() {
        let hooks = Rows { platform: Platform::detect(), width: |_| 4, parents: Default::default(), children: Default::default() };
        let k = Kernels { key: crate::IV, flags: 0, platform: hooks.platform, hooks: &hooks };
        let value = |i: usize, j: usize| -> Cv { core::array::from_fn(|b| (31 * i + 7 * j + b) as u8) };
        // A row's one value, computed apart: four values, two parents, one.
        let condensed = |i: usize| k.parent(&k.parent(&value(i, 0), &value(i, 1)), &k.parent(&value(i, 2), &value(i, 3)));
        let mut stack = CvStack::new();
        stack.push(value(0, 0));
        stack.push(value(1, 0));
        for i in 2..14 {
            let row: [Cv; 4] = core::array::from_fn(|j| value(i, j));
            stack.push_row(&k, &row);
            // The rows are the last ROWS entries pushed as rows.
            let first_row = (i + 1).saturating_sub(ROWS).max(2);
            assert_eq!((0..=i).filter(|&e| stack.is_row(e)).collect::<std::vec::Vec<_>>(), (first_row..=i).collect::<std::vec::Vec<_>>(), "after entry {i}");
            for e in 0..=i {
                assert_eq!(stack.get(&k, e), if e < 2 { value(e, 0) } else { condensed(e) }, "entry {e} after entry {i}");
            }
        }
    }

    /// A row's place is zeroed the first time it is given out, and never
    /// again: each of the ROWS places, painted 0xff before its first use,
    /// comes out zeros; painted again and given out a second time (the
    /// stack emptied between), it comes out as painted.
    #[test]
    fn a_rows_place_is_zeroed_once() {
        let hooks = Rows { platform: Platform::detect(), width: |_| 4, parents: Default::default(), children: Default::default() };
        let k = Kernels { key: crate::IV, flags: 0, platform: hooks.platform, hooks: &hooks };
        let mut stack = CvStack::new();
        // Sound: the rows' own storage, every byte written.
        unsafe { stack.rows.as_mut_ptr().write_bytes(0xff, 1) };
        for round in 0..2 {
            for entry in 0..ROWS {
                let place = stack.place_on_top(&k);
                assert_eq!(*place, [[if round == 0 { 0 } else { 0xff }; 32]; ROW], "place {entry}, time {round}");
                *place = [[0xff; 32]; ROW];
                stack.pushed_row(4);
            }
            assert_eq!(stack.ready, (1 << ROWS) - 1);
            stack.clear();
        }
    }

    /// A stack cloned in place holds the source's entries, whatever it held
    /// before: its own rows and values are gone, the source's are there.
    #[test]
    fn a_stack_cloned_in_place_holds_the_sources_entries() {
        const GROUP: usize = 16 * CHUNK_LEN;
        let input: std::vec::Vec<u8> = (0..23 * GROUP).map(|i| (i % 251) as u8).collect();
        let hooks = Rows { platform: Platform::detect(), width: |_| 8, parents: Default::default(), children: Default::default() };
        let k = Kernels { key: crate::IV, flags: 0, platform: hooks.platform, hooks: &hooks };
        for (source_groups, target_groups) in [(23, 14), (14, 23), (1, 23), (23, 1), (0, 7), (7, 0)] {
            let (_, source, _, _) = fed(&input[..source_groups * GROUP], &[GROUP], |_| 8);
            let (_, mut target, _, _) = fed(&input[..target_groups * GROUP], &[GROUP], |_| 4);
            target.clone_from(&source);
            assert_eq!((target.len(), target.rows_in_use()), (source.len(), source.rows_in_use()), "{source_groups} groups over {target_groups}");
            for i in 0..source.len() {
                assert_eq!(
                    (target.is_row(i), target.get(&k, i)),
                    (source.is_row(i), source.get(&k, i)),
                    "entry {i}: {source_groups} groups over {target_groups}"
                );
                if source.is_row(i) {
                    assert_eq!(target.row(i), source.row(i), "entry {i}'s row");
                }
            }
        }
    }

    #[test]
    #[should_panic(expected = "two rows merge at one width")]
    fn unequal_rows_stop_before_copying() {
        let hooks = Rows { platform: Platform::detect(), width: |_| 4, parents: Default::default(), children: Default::default() };
        let k = Kernels { key: crate::IV, flags: 0, platform: hooks.platform, hooks: &hooks };
        let mut stack = CvStack::new();
        stack.push_row(&k, &[[1; crate::OUT_LEN]; 4]);
        stack.push_row(&k, &[[2; crate::OUT_LEN]; 4]);
        // Both slots are initialized in full. Model a bad recorded width
        // that integer division would round down without halving the row.
        stack.widths[0] = 6;
        stack.merge(&k, 1);
    }

    /// A stack keeps at most ROWS rows: with 1023 groups (ten subtrees,
    /// of 512 groups down to one, all but the first rows) the largest row
    /// is condensed to its one value as the ninth arrives. A clone holds the same entries, and
    /// a row's place given up is never read: the clone's root is the
    /// stack's.
    #[test]
    fn a_stack_keeps_rows_rows_and_condenses_the_one_below() {
        const GROUP: usize = 16 * CHUNK_LEN;
        let input: std::vec::Vec<u8> = (0..1023 * GROUP).map(|i| (i % 251) as u8).collect();
        let (root, stack, _, _) = fed(&input, &[GROUP], |_| 8);
        assert_eq!(root, *crate::convenience::hash(&input).as_bytes());
        // Ten subtrees, of 512 groups down to one: the first's entry is
        // single values' parent, the nine after it were rows.
        assert_eq!((stack.len(), stack.rows_in_use()), (10, ROWS));
        assert!(!stack.is_row(0) && !stack.is_row(1) && (2..10).all(|i| stack.is_row(i)), "the largest row gave up its place to the ninth");
        let hooks = Rows { platform: Platform::detect(), width: |_| 8, parents: Default::default(), children: Default::default() };
        let k = Kernels { key: crate::IV, flags: 0, platform: hooks.platform, hooks: &hooks };
        let clone = stack.clone();
        assert_eq!((clone.len(), clone.rows_in_use()), (10, ROWS));
        assert_eq!(final_node(&k, &k.new_chunk(1023 * 16), &clone).root_bytes(), root);
        for i in 0..10 {
            assert_eq!(clone.get(&k, i), stack.get(&k, i), "entry {i}");
        }
    }

    /// ChunkState::absorb compresses whole blocks straight from the input
    /// when its buffer is empty, keeps a last whole block waiting, and
    /// calls the multi-block kernel only for one block or more. A first
    /// block through the empty buffer, or a kernel call for no block, gives
    /// the same state at more work, so only the counts tell (BLAKE3 #219).
    #[test]
    fn test_absorb_takes_whole_blocks_straight_from_the_input() {
        let input: std::vec::Vec<u8> = (0..crate::CHUNK_LEN as u32).map(|i| (i % 251) as u8).collect();
        let platform = Platform::detect();
        // Pieces absorbed in turn: (their lengths, blocks compressed from the buffer, multi-block calls).
        for (pieces, buffered, runs) in [
            (&[130][..], 0, 1), // two blocks from the input, two bytes wait
            (&[64][..], 0, 0),  // one whole block waits
            (&[64, 64][..], 1, 0),
            (&[1, 64][..], 1, 0), // the buffer's byte and 63 more, then one byte waits
            (&[1, 200][..], 1, 1),
        ] {
            let k = Counting { platform, buffered: Cell::new(0), runs: Cell::new(0) };
            let (mut counted, mut plain) = (ChunkState::new(crate::IV, 0, 0, &k), ChunkState::new(crate::IV, 0, 0, platform));
            let mut at = 0;
            for &n in pieces {
                counted.absorb(&input[at..at + n]);
                plain.absorb(&input[at..at + n]);
                at += n;
            }
            assert_eq!((k.buffered.get(), k.runs.get()), (buffered, runs), "{pieces:?}");
            assert_eq!((counted.cv, counted.buf, counted.buf_len, counted.blocks_compressed), (plain.cv, plain.buf, plain.buf_len, plain.blocks_compressed));
        }
    }
}
