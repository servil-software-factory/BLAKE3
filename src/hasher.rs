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

/// The most values a row holds: the widest kernels' lanes on any target
/// (x86-64 Linux's 32).
pub const ROW: usize = 32;
// Every kernel's cut fits a row, on every target (the row's size is the
// same everywhere: the C header gives a hasher's size as one number).
const _: () = assert!(crate::MAX_SIMD_DEGREE <= ROW);
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
/// A row is condensed to its one value, or its two halves' (the root's
/// children), when the output is computed.
pub struct CvStack {
    cvs: Option<[Cv; DEPTH]>,
    len: usize,
    /// Bit i: entry i is a row, kept in `rows[i % ROWS]`.
    wide: u64,
    /// Each row's width.
    widths: [u8; ROWS],
    rows: core::mem::MaybeUninit<[[Cv; ROW]; ROWS]>,
}

impl Clone for CvStack {
    /// One literal: the rows' storage copied whole, once, into its place.
    /// (A clone made with empty storage and then given the rows in use
    /// is a value written to after it is made, which is copied again to
    /// where it goes.)
    fn clone(&self) -> CvStack {
        CvStack { cvs: self.cvs, len: self.len, wide: self.wide, widths: self.widths, rows: self.rows }
    }
}

impl CvStack {
    #[inline(always)]
    pub fn new() -> CvStack {
        CvStack { cvs: None, len: 0, wide: 0, widths: [0; ROWS], rows: core::mem::MaybeUninit::uninit() }
    }

    pub fn len(&self) -> usize {
        self.len
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
        // Sound: within the rows' storage; plain bytes.
        unsafe { core::slice::from_raw_parts_mut((self.rows.as_mut_ptr() as *mut Cv).add((i % ROWS) * ROW), row.len()) }.copy_from_slice(row);
        self.widths[i % ROWS] = row.len() as u8;
        self.wide |= 1 << i;
    }

    /// Entry `i` as `want` values (1, or 2 for a row's two halves): a
    /// single entry is its own value; a row is condensed a layer at a time.
    fn condensed<P: Parent>(&self, p: &P, i: usize, want: usize) -> [Cv; 2] {
        assert!(i < self.len, "a value below the stack's length");
        if !self.is_row(i) {
            return match &self.cvs {
                Some(cvs) => [cvs[i], [0; 32]],
                None => panic!("a stack with values has its array"),
            };
        }
        let mut a = [[0u8; 32]; ROW];
        let mut b = [[0u8; 32]; ROW / 2];
        let row = self.row(i);
        let mut n = row.len();
        a[..n].copy_from_slice(row);
        // Layers of parents down to the two halves, then the one parent.
        while n > 2 {
            n = p.parents(&a[..n], &mut b);
            a[..n].copy_from_slice(&b[..n]);
        }
        if want == 1 { [p.parent(&a[0], &a[1]), [0; 32]] } else { [a[0], a[1]] }
    }

    /// Value `i`, below the length.
    pub fn get<P: Parent>(&self, p: &P, i: usize) -> Cv {
        self.condensed(p, i, 1)[0]
    }

    /// Whether the top entry is a row.
    pub fn top_is_row(&self) -> bool {
        self.len > 0 && self.is_row(self.len - 1)
    }

    /// The top entry's two halves (it is a row).
    pub fn top_halves<P: Parent>(&self, p: &P) -> [Cv; 2] {
        self.condensed(p, self.len - 1, 2)
    }

    /// `cv` on top; the stack holds fewer than DEPTH values.
    pub fn push(&mut self, cv: Cv) {
        if self.cvs.is_none() {
            self.cvs = Some([[0; 32]; DEPTH]);
        }
        if let Some(cvs) = &mut self.cvs {
            cvs[self.len] = cv;
        }
        self.wide &= !(1 << self.len);
        self.len += 1;
    }

    /// `row` on top, a subtree's cut; the stack holds fewer than DEPTH
    /// entries. A row ROWS entries below it gives up its place: it is
    /// condensed to its one value.
    pub fn push_row<P: Parent>(&mut self, p: &P, row: &[Cv]) {
        assert!((4..=ROW).contains(&row.len()) && row.len().is_power_of_two(), "a row of 4 to ROW values, a power of two");
        // The rows below that keep theirs in the same place: ROWS, 2 ROWS,
        // ... entries down.
        for below in (self.len % ROWS..self.len).step_by(ROWS) {
            if self.is_row(below) {
                self.narrow(p, below);
            }
        }
        self.set_row(self.len, row);
        self.len += 1;
    }

    /// Entry `i`, a row, becomes the row half as wide: its values' parents.
    fn halve<P: Parent>(&mut self, p: &P, i: usize) {
        let n = self.row(i).len();
        let mut children = [[0u8; 32]; ROW];
        children[..n].copy_from_slice(self.row(i));
        // Sound: row i's place, its first n / 2 values written (they are row i's).
        let place = unsafe { core::slice::from_raw_parts_mut((self.rows.as_mut_ptr() as *mut Cv).add((i % ROWS) * ROW), n / 2) };
        let made = p.parents(&children[..n], place);
        assert_eq!(made, n / 2, "a parent for every two children");
        self.widths[i % ROWS] = (n / 2) as u8;
    }

    /// Entry `i`, a row, becomes its one value.
    fn narrow<P: Parent>(&mut self, p: &P, i: usize) {
        let cv = self.get(p, i);
        if self.cvs.is_none() {
            self.cvs = Some([[0; 32]; DEPTH]);
        }
        if let Some(cvs) = &mut self.cvs {
            cvs[i] = cv;
        }
        self.wide &= !(1 << i);
    }

    #[cfg(any(feature = "std", test))]
    pub fn clear(&mut self) {
        self.len = 0;
        self.wide = 0;
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
                    let times = self.row(wide).len() / self.row(narrow).len();
                    for _ in 0..if times > 1 { times.trailing_zeros() } else { 0 } {
                        self.halve(p, wide);
                    }
                }
                // The two rows' values in one place, then their parents
                // into the lower row's place. The place for the children
                // is written before it is read, and never zeroed first:
                // zeroing 2 KiB a merge cost more than the merge saved.
                let n = self.row(a).len();
                let mut children = core::mem::MaybeUninit::<[Cv; 2 * ROW]>::uninit();
                let at = children.as_mut_ptr() as *mut Cv;
                // Sound: 2n values written into room for 2 ROW, then read.
                let children = unsafe {
                    core::ptr::copy_nonoverlapping(self.row(a).as_ptr(), at, n);
                    core::ptr::copy_nonoverlapping(self.row(b).as_ptr(), at.add(n), n);
                    core::slice::from_raw_parts(at, 2 * n)
                };
                // Sound: row a's place, its first n values written (they are row a).
                let place = unsafe { core::slice::from_raw_parts_mut((self.rows.as_mut_ptr() as *mut Cv).add((a % ROWS) * ROW), n) };
                let made = p.parents(children, place);
                assert_eq!(made, n, "a parent for every two children");
                self.wide &= !(1 << b);
            } else {
                let parent = p.parent(&self.get(p, a), &self.get(p, b));
                if self.cvs.is_none() {
                    self.cvs = Some([[0; 32]; DEPTH]);
                }
                if let Some(cvs) = &mut self.cvs {
                    cvs[a] = parent;
                }
                self.wide &= !(1 << a) & !(1 << b);
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
                    // (Zeroed here, for the subtrees that are cut, not
                    // once a call for every call.)
                    let mut row = [[0u8; 32]; ROW];
                    let n = k.cut(sub, ahead, counter, &mut row);
                    stack.merge(k, ones(counter - initial));
                    stack.push_row(k, &row[..n]);
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
    let mut out = if chunk.len() > 0 {
        k.chunk_node(chunk)
    } else if stack.top_is_row() {
        // The top subtree's two halves: the root's children, if it is all.
        n -= 1;
        let [left, right] = stack.top_halves(k);
        k.parent_node(&left, &right)
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

#[cfg(all(test, feature = "zeroize"))]
impl CvStack {
    /// The array, for the zeroize test.
    pub fn array(&self) -> Option<&[Cv; DEPTH]> {
        self.cvs.as_ref()
    }
}

#[cfg(feature = "zeroize")]
impl zeroize::Zeroize for CvStack {
    fn zeroize(&mut self) {
        if let Some(cvs) = &mut self.cvs {
            cvs.zeroize();
        }
        // Every row's place: a row merged away leaves its values behind.
        // Sound: the storage's bytes, written, never read as values here.
        unsafe { core::slice::from_raw_parts_mut(self.rows.as_mut_ptr() as *mut core::mem::MaybeUninit<u8>, core::mem::size_of::<[[Cv; ROW]; ROWS]>()) }
            .fill(core::mem::MaybeUninit::new(0));
        self.widths.zeroize();
        self.wide = 0;
        self.len = 0;
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
        fn cut(&self, input: &[u8], ahead: usize, counter: u64, out: &mut [Cv; ROW]) -> usize {
            let want = (self.width)(input.len());
            // The kernels' own cut, condensed to the width asked for.
            let kernels = crate::tree::PlatformKernels { key: crate::IV, flags: 0, platform: self.platform };
            let mut n = crate::tree::wide(&kernels, input, ahead, counter, out);
            let mut next = [[0u8; 32]; ROW];
            while n > want {
                n = crate::compress_parents_parallel(out[..n].as_flattened(), crate::IV, 0, self.platform, next.as_flattened_mut());
                out[..n].copy_from_slice(&next[..n]);
            }
            n
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
            for pieces in [&[usize::MAX][..], &[GROUP], &[4 * GROUP], &[21845], &[GROUP + CHUNK_LEN, 7 * GROUP], &[3 * GROUP, GROUP, 64 * GROUP, 1]] {
                // 511 groups a few bytes at a time would be thousands of calls of nothing new.
                if len > 300 * GROUP && pieces[0] == 21845 {
                    continue;
                }
                assert_eq!(fed(message, pieces, |_| ROW).0, want, "rows of the kernels' width: {len} bytes in pieces of {pieces:?}");
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

    /// A stack keeps at most ROWS rows: with 511 groups (nine subtrees,
    /// of 256 groups down to one) the largest is condensed to its one
    /// value as the ninth row arrives. A clone holds the same entries, and
    /// a row's place given up is never read: the clone's root is the
    /// stack's.
    #[test]
    fn a_stack_keeps_rows_rows_and_condenses_the_one_below() {
        const GROUP: usize = 16 * CHUNK_LEN;
        let input: std::vec::Vec<u8> = (0..511 * GROUP).map(|i| (i % 251) as u8).collect();
        let (root, stack, _, _) = fed(&input, &[GROUP], |_| 8);
        assert_eq!(root, *crate::convenience::hash(&input).as_bytes());
        assert_eq!((stack.len(), stack.rows_in_use()), (9, ROWS));
        assert!(!stack.is_row(0) && (1..9).all(|i| stack.is_row(i)), "the largest subtree's row gave up its place");
        let hooks = Rows { platform: Platform::detect(), width: |_| 8, parents: Default::default(), children: Default::default() };
        let k = Kernels { key: crate::IV, flags: 0, platform: hooks.platform, hooks: &hooks };
        let clone = stack.clone();
        assert_eq!((clone.len(), clone.rows_in_use()), (9, ROWS));
        assert_eq!(final_node(&k, &k.new_chunk(511 * 16), &clone).root_bytes(), root);
        for i in 0..9 {
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
