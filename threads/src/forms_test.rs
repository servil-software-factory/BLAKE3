//! The multithreaded forms of the core's calls, against the core's own:
//! the halves of the core's tests that need threads. Each test here has
//! its twin in the core, which checks the one-thread form against the
//! fixed values; these check the multithreaded form against that.

use crate::test::guard::{Guarded, digests, filled};
use crate::throughput::{self, Batch, Block, Packed};
use crate::{CHUNK_LEN, Hasher, KEY_LEN, Mode, OUT_LEN};

fn input(len: usize, seed: u8) -> Vec<u8> {
    (0..len).map(|i| (i as u32).wrapping_mul(0x9E37_79B1).to_le_bytes()[3] ^ seed).collect()
}

/// A batch and an input, from two threads at once through the
/// multithreaded full forms.
#[test]
fn test_pool_from_two_callers() {
    crate::initialize_multithreaded(None);
    let batch = input(1024 * 64, 1);
    let tree = input(65 * 1024 + 3, 2);
    let mut expected = vec![[0u8; OUT_LEN]; 1024];
    throughput::hash_many(Mode::Plain, &Batch::in_place(&crate::test::blocks(&batch), 64), &mut expected);
    let expected_tree = crate::convenience::hash(&tree);
    std::thread::scope(|scope| {
        for caller in 0..2 {
            let (batch, tree, expected) = (&batch, &tree, &expected);
            scope.spawn(move || {
                let mut out = vec![[0u8; OUT_LEN]; 1024];
                throughput::hash_many_multithreaded(Mode::Plain, &Batch::in_place(&crate::test::blocks(batch), 64), &mut out);
                assert_eq!(&out, expected, "batch, caller {caller}");
                assert_eq!(throughput::hash_multithreaded(Mode::Plain, &crate::test::message(tree), tree.len()), expected_tree, "tree, caller {caller}");
            });
        }
    });
}

/// hash_multithreaded, the message's whole blocks flush against an
/// inaccessible page (after them, then before): it may read to their end,
/// no further. The lengths of the core's test_hash_inside_guard_pages
/// that reach the pool.
#[test]
fn test_hash_multithreaded_inside_guard_pages() {
    crate::initialize_multithreaded(None);
    let mut lens: Vec<usize> = Vec::new();
    for chunks in [64, 100, 128, 256, 1024, 1025] {
        for delta in [-65i64, -64, -1, 0, 1, 63, 64, 65] {
            lens.push(((chunks * CHUNK_LEN) as i64 + delta) as usize);
        }
    }
    let mut ran = 0;
    for len in lens {
        for at_end in [true, false] {
            if len >= 64 * CHUNK_LEN && len % 7 == 0 {
                let mut blocks = filled(len.div_ceil(64) * 64, at_end);
                let message = blocks.bytes()[..len].to_vec();
                let input = Block::from_bytes(blocks.bytes()).expect("a page-end region of whole blocks starts aligned");
                assert_eq!(throughput::hash_multithreaded(Mode::Plain, input, len), crate::convenience::hash(&message), "mt, {len} bytes");
                ran += 1;
            }
        }
    }
    assert!(ran >= 8, "the sweep reached the pool's lengths ({ran})");
}

/// hash_many_multithreaded, the batch flush against a page and its
/// digests against another: the core's test_hash_many_inside_guard_pages'
/// batches of 64 KiB and more.
#[test]
fn test_hash_many_multithreaded_inside_guard_pages() {
    crate::initialize_multithreaded(None);
    let mut ran = 0;
    for len in [1, 63, 64, 100, 128, 192, 200, 256, 640, 1000, 1024, 1088, 2000, 2048, 3072, 4096, 4100, 8192, 15360, 15359, 16384] {
        for count in [1, 2, 3, 4, 5, 6, 7, 9, 10, 12, 15, 16, 17, 21, 24, 31, 33, 127, 128, 129] {
            for at_end in [true, false] {
                let slot = crate::slot_len(len);
                if slot * count < 64 * CHUNK_LEN {
                    continue;
                }
                let mut input = filled(slot * count, at_end);
                for message in input.bytes().chunks_exact_mut(slot) {
                    message[len..].fill(0);
                }
                let data = input.bytes().to_vec();
                let (_page, mt) = digests(count, at_end);
                throughput::hash_many_multithreaded(
                    Mode::Plain,
                    &Batch::in_place(Block::from_bytes(input.bytes()).expect("a page-end batch starts aligned"), len),
                    mt,
                );
                for (i, digest) in mt.iter().enumerate() {
                    assert_eq!(*digest, *crate::convenience::hash(&data[i * slot..][..len]).as_bytes(), "mt, {count} x {len} B, message {i}, at end {at_end}");
                }
                ran += 1;
            }
        }
    }
    assert!(ran > 100, "the sweep reached the pool's batches ({ran})");
}

/// hash_each_multithreaded, each item in its own pages flush against a
/// guard page (then flush after one), its digests against another.
#[test]
fn test_hash_each_multithreaded_each_item_against_a_guard_page() {
    crate::initialize_multithreaded(None);
    let mut lens: Vec<usize> = (0..=130).collect();
    lens.extend([255, 256, 1000, 1023, 1024, 1025, 1089, 1500, 2047, 2048, 3072, 3839, 4096, 7935, 8191, 16383, 16384, 16385, 65536 + 7]);
    for count in [1, 2, 3, 4, 5, 7, 8, 9, 16, 17, 33] {
        for &len in &lens {
            for at_end in [true, false] {
                let lens: Vec<usize> = (0..count).map(|i| len + (i % 3)).collect();
                let guarded: Vec<Guarded> = lens.iter().map(|&l| filled(l.div_ceil(64).max(1) * 64, at_end)).collect();
                let regions: Vec<&[u8]> = guarded.iter().map(|g| unsafe { core::slice::from_raw_parts(g.base.add(g.start), g.len) }).collect();
                let in_blocks: Vec<(&[Block], usize)> =
                    regions.iter().zip(&lens).map(|(r, &l)| (Block::from_bytes(r).expect("a page-end region of whole blocks starts aligned"), l)).collect();
                let want: Vec<[u8; 32]> = regions.iter().zip(&lens).map(|(r, &l)| *crate::convenience::hash(&r[..l]).as_bytes()).collect();
                let (_page, got) = digests(count, at_end);
                throughput::hash_each_multithreaded(Mode::Plain, &in_blocks, got);
                assert_eq!(got, &want[..], "hash_each_multithreaded, {count} x {len} B, at end {at_end}");
            }
        }
    }
}

/// Every call of this crate that reads a caller's bytes or writes into a
/// caller's places, in every mode, its inputs and outputs each flush
/// against a guard page, after them and then before: hash_multithreaded,
/// the Hasher's multithreaded update, and the stream's three ways in with
/// its digests. (The core's test_every_call_inside_guard_pages has the
/// one-thread calls, and the fixed values.)
#[test]
fn test_every_multithreaded_call_inside_guard_pages() {
    crate::initialize_multithreaded(None);
    let lens =
        [0, 1, 63, 64, 65, 127, 128, 1023, 1024, 1025, 2047, 2048, 2049, 3839, 4470, 7935, 8192, 16383, 16384, 16385, 65543, 131073, 524291, (1 << 20) + 5];
    for at_end in [true, false] {
        let mut key_page = filled(KEY_LEN, at_end);
        // Sound: the page holds KEY_LEN bytes and outlives the modes.
        let key: &[u8; KEY_LEN] = unsafe { &*(key_page.bytes().as_ptr() as *const [u8; KEY_LEN]) };
        let context_text = "guard pages 2026-10-25 every call";
        let mut context_page = Guarded::new(context_text.len(), at_end);
        context_page.bytes().copy_from_slice(context_text.as_bytes());
        let context = core::str::from_utf8(context_page.bytes()).unwrap();
        for (name, mode) in [("hash", Mode::Plain), ("keyed", Mode::Keyed(key)), ("derive key", Mode::DeriveKey(context))] {
            for len in lens {
                let mut flush = filled(len, at_end);
                let message = flush.bytes().to_vec();
                let what = format!("{name}, {len} B, at end {at_end}");
                let want = *crate::convenience::hash_with(mode, &message).as_bytes();
                // The message's whole blocks flush against the page.
                let mut region = filled(len.div_ceil(64).max(1) * 64, at_end);
                region.bytes()[..len].copy_from_slice(&message);
                let blocks = Block::from_bytes(unsafe { core::slice::from_raw_parts(region.base.add(region.start), region.len) })
                    .expect("a page-end region of whole blocks starts aligned");
                assert_eq!(*throughput::hash_multithreaded(mode, blocks, len).as_bytes(), want, "hash_multithreaded, {what}");
                let mut hasher = Hasher::new(mode, None);
                throughput::update_multithreaded(&mut hasher, flush.bytes());
                assert_eq!(*hasher.finalize().as_bytes(), want, "update_multithreaded, {what}");
                // A stream: the message where it is, the message written into
                // its rooms, and a batch of three, every digest into places
                // flush against a page.
                let count = 3;
                let slot = crate::slot_len(len);
                let mut batch = filled(slot * count, at_end);
                for i in 0..count {
                    let m = &mut batch.bytes()[i * slot..][..slot];
                    m[..len].copy_from_slice(&message);
                    m[len..].fill(0);
                }
                let batch_blocks =
                    Block::from_bytes(unsafe { core::slice::from_raw_parts(batch.base.add(batch.start), batch.len) }).expect("a page-end batch starts aligned");
                let input: &[u8] = unsafe { core::slice::from_raw_parts(flush.base.add(flush.start), flush.len) };
                let _ = batch_blocks;
                let mut owned_batch = Batch::new(len, count);
                for i in 0..count {
                    owned_batch.set(i, &message);
                }
                let mut pipe = throughput::Pipeline::new(mode, throughput::Limits { messages: 4, rooms: 2 });
                throughput::block_on(async {
                    let m_lent = pipe.message(0u8, None).await;
                    let guard = unsafe { pipe.lent(m_lent, &Block::bytes(blocks)[..len]) };

                    let mut m_room = pipe.message(1u8, None).await;
                    for part in input.chunks(throughput::ROOM_LEN) {
                        let mut r = pipe.room().await;
                        r.bytes_mut()[..part.len()].copy_from_slice(part);
                        pipe.commit(&mut m_room, &mut r, part.len());
                    }
                    pipe.end(m_room);

                    let m_batch = pipe.message(2u8, None).await;
                    pipe.batch(m_batch, owned_batch, std::vec![[0u8; 32]; count].into_boxed_slice());

                    let mut seen = 0;
                    while seen < 3 {
                        if let Some(ev) = pipe.next_event() {
                            seen += 1;
                            match ev {
                                throughput::Event::Hashed { tag: 0, hash, .. } => assert_eq!(*hash.as_bytes(), want, "pipeline lent, {what}"),
                                throughput::Event::Hashed { tag: 1, hash, .. } => assert_eq!(*hash.as_bytes(), want, "pipeline rooms, {what}"),
                                throughput::Event::Batch { tag: 2, digests, .. } => {
                                    assert!(digests.iter().all(|h| *h == want), "pipeline batch, {what}");
                                }
                                _ => unreachable!(),
                            }
                        } else {
                            pipe.ready().await;
                        }
                    }
                    drop(guard);
                });
            }
        }
    }
}

/// The Hasher's stage with update and update_multithreaded mixed: pieces
/// of many lengths across 16 KiB boundaries, finalize midway and after
/// more input, against one-shot hashes.
#[test]
fn test_hasher_gathers_with_multithreaded_updates() {
    crate::initialize_multithreaded(None);
    let mut input = vec![0u8; 300_000];
    crate::test::paint_test_input(&mut input);
    let pieces = [1, 1448, 4096, 1448, 16384, 31, 16383, 65536, 16385, 7, 1000];
    for start in 0..pieces.len() {
        let mut hasher = Hasher::new(Mode::Plain, None);
        let mut done = 0;
        let mut k = start;
        while done < input.len() {
            let n = pieces[k % pieces.len()].min(input.len() - done);
            if k % 3 == 0 {
                throughput::update_multithreaded(&mut hasher, &input[done..done + n]);
            } else {
                hasher.update(&input[done..done + n]);
            }
            done += n;
            k += 1;
            if k % 5 == 0 {
                assert_eq!(hasher.clone().finalize(), crate::convenience::hash(&input[..done]), "after {done} bytes");
            }
        }
        assert_eq!(hasher.finalize(), crate::convenience::hash(&input), "pieces from {start}");
    }
}

/// hash_each_multithreaded takes the pool from MIN_SPLIT_LEN bytes of
/// items on, exactly, and the calling thread below them. Both give the
/// same digests, so only where the call ran tells (BLAKE3 #219).
#[test]
fn test_hash_each_multithreaded_splits_at_min_split_len_exactly() {
    crate::initialize_multithreaded(None);
    let min = crate::lanes::MIN_SPLIT_LEN;
    let input = vec![7u8; min];
    for total in [min - 1, min] {
        // Items under the split each, so none hashes alone.
        let items: Vec<&[u8]> = input[..total].chunks(64 * 1024 - 3).collect();
        let (mut a, mut b) = (vec![[0u8; OUT_LEN]; items.len()], vec![[0u8; OUT_LEN]; items.len()]);
        crate::RAN.with(|r| r.set(0));
        throughput::hash_each_multithreaded(Mode::Plain, &crate::test::items(&crate::test::messages(&items), &items), &mut a);
        if total < min {
            assert_eq!(crate::RAN.with(|r| r.get()) & crate::Ran::Pool as u32, 0, "{total} bytes of items stay on the calling thread");
        } else {
            crate::lanes::assert_past_the_split("MIN_SPLIT_LEN bytes of items");
        }
        throughput::hash_each(Mode::Plain, &crate::test::items(&crate::test::messages(&items), &items), &mut b);
        assert_eq!(a, b);
    }
}

/// The pool's ranges of items (lanes::Work::Each), just past where the
/// pool takes them, against the calling thread's.
#[test]
fn test_hash_each_multithreaded_past_the_split() {
    crate::initialize_multithreaded(None);
    let mut input = vec![0u8; crate::lanes::MIN_SPLIT_LEN + 8 * 1024];
    crate::test::paint_test_input(&mut input);
    let items: Vec<&[u8]> = input.chunks(crate::lanes::MIN_SPLIT_LEN / 40 + 7).collect();
    let (mut a, mut b) = (vec![[0u8; OUT_LEN]; items.len()], vec![[0u8; OUT_LEN]; items.len()]);
    crate::RAN.with(|r| r.set(0));
    throughput::hash_each_multithreaded(Mode::Plain, &crate::test::items(&crate::test::messages(&items), &items), &mut a);
    crate::lanes::assert_past_the_split("the items");
    throughput::hash_each(Mode::Plain, &crate::test::items(&crate::test::messages(&items), &items), &mut b);
    assert_eq!(a, b);
}

/// A collection past the split (items around chunk, lane, and the
/// split's own length, a few of 512 KiB and more), every mode: the
/// multithreaded form gives hash_each's digests.
/// Keep: it detects, by hanging, mutants the clock then stops
/// (tools/mutants/timeouts-x86_64.txt).
#[test]
fn test_hash_each_multithreaded() {
    crate::initialize_multithreaded(None);
    let mut input = vec![0u8; 3 << 20];
    crate::test::paint_test_input(&mut input);
    let lens = [0, 1, 1024, 1025, 3000, 15 * 1024, 16 * 1024, 100_000, 511 * 1024, 512 * 1024, 700_000];
    let items: Vec<&[u8]> = (0..400)
        .map(|k| {
            let n = lens[k * 7 % lens.len()].min(input.len() - k);
            &input[k..k + n]
        })
        .collect();
    let key = [9u8; 32];
    for mode in [Mode::Plain, Mode::Keyed(&key), Mode::DeriveKey("each mt")] {
        let (mut a, mut b) = (vec![[0u8; OUT_LEN]; items.len()], vec![[0u8; OUT_LEN]; items.len()]);
        throughput::hash_each_multithreaded(mode, &crate::test::items(&crate::test::messages(&items), &items), &mut a);
        throughput::hash_each(mode, &crate::test::items(&crate::test::messages(&items), &items), &mut b);
        assert_eq!(a, b);
        // Below the split, on the calling thread: the items of 16 KiB
        // or less, which sum to less than the split's length.
        let small: Vec<usize> = (0..items.len()).filter(|&k| items[k].len() <= 16 * 1024).take(30).collect();
        assert!(small.iter().map(|&k| items[k].len()).sum::<usize>() < crate::lanes::MIN_SPLIT_LEN);
        let small_items: Vec<&[u8]> = small.iter().map(|&k| items[k]).collect();
        let mut c = vec![[0u8; OUT_LEN]; small.len()];
        throughput::hash_each_multithreaded(mode, &crate::test::items(&crate::test::messages(&small_items), &small_items), &mut c);
        assert!(small.iter().zip(&c).all(|(&k, d)| *d == b[k]));
    }
}

fn packed<const N: usize>(count: usize, seed: u8) -> Packed<N> {
    let mut leaves = Packed::<N>::new(count);
    for (i, leaf) in leaves.iter_mut().enumerate() {
        for (j, b) in leaf.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(31).wrapping_add(j as u8).wrapping_add(seed);
        }
    }
    leaves
}

/// A packed batch's digests through hash_many_multithreaded are each
/// message's hash, in every mode, at counts around the kernels' group of
/// sixteen and beyond the pool's split, 16 and 32 bytes a message.
#[test]
fn packed_messages_hash_multithreaded_as_each_message_alone() {
    crate::initialize_multithreaded(None);
    let key = [9u8; 32];
    for (name, mode) in [("plain", Mode::Plain), ("keyed", Mode::Keyed(&key)), ("derive", Mode::DeriveKey("packed test"))] {
        for count in [0, 1, 2, 3, 4, 5, 7, 8, 9, 15, 16, 17, 31, 33, 100, 1000, 40_000] {
            let leaves = packed::<32>(count, 3);
            let mut out_mt = vec![[0u8; 32]; count];
            throughput::hash_many_multithreaded(mode, &leaves, &mut out_mt);
            for (i, leaf) in leaves.iter().enumerate() {
                assert_eq!(out_mt[i], *crate::convenience::hash_with(mode, leaf).as_bytes(), "32 B multithreaded, {count} messages, message {i}, {name}");
            }
            let short = packed::<16>(count, 5);
            throughput::hash_many_multithreaded(mode, &short, &mut out_mt);
            for (i, leaf) in short.iter().enumerate() {
                assert_eq!(out_mt[i], *crate::convenience::hash_with(mode, leaf).as_bytes(), "16 B multithreaded, {count} messages, message {i}, {name}");
            }
        }
    }
}

/// Blocks borrowed that hold no whole number of messages are refused by
/// hash_many_multithreaded above the pool's split, as hash_many refuses
/// them: before the 0.10 fixes' own review (BLAKE3 #461) it hashed the
/// whole messages and passed over the ragged end.
#[test]
#[should_panic(expected = "input holds one slot per output")]
fn a_ragged_borrowed_batch_is_refused_above_the_split() {
    crate::initialize_multithreaded(None);
    // Messages of two blocks in an odd number of blocks, 640,064 bytes.
    let blocks = vec![Block::ZERO; 2 * 5000 + 1];
    let mut out = vec![[0u8; 32]; 5000];
    throughput::hash_many_multithreaded(Mode::Plain, &Batch::from_blocks(&blocks, 128), &mut out);
}

/// hash_many_multithreaded of messages of 2 to 16 whole blocks, at counts
/// that fill SME2 groups and leave remainders, against hash_many (the
/// core's test_hash_many_whole_blocks checks that against hash()).
#[test]
fn test_hash_many_multithreaded_whole_blocks() {
    crate::initialize_multithreaded(None);
    for blocks in 2..=16 {
        for count in [0, 1, 2, 3, 4, 5, 6, 7, 15, 16, 17, 18, 31, 32, 33, 127, 128, 129, 130, 300] {
            let len = blocks * crate::BLOCK_LEN;
            let mut bytes: Vec<u8> = (0..(len * count).div_ceil(8) as u64).flat_map(|i| ((len as u64) << 48 | i).to_le_bytes()).collect();
            bytes.truncate(len * count);
            let batch_blocks = crate::test::blocks(&bytes);
            let (mut out, mut mt) = (vec![[0u8; OUT_LEN]; count], vec![[0u8; OUT_LEN]; count]);
            throughput::hash_many(Mode::Plain, &Batch::in_place(&batch_blocks, len), &mut out);
            throughput::hash_many_multithreaded(Mode::Plain, &Batch::in_place(&batch_blocks, len), &mut mt);
            assert_eq!(mt, out, "multithreaded, {count} messages of {len} bytes");
        }
    }
}
