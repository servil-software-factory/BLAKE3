#![cfg(feature = "std")]
use blake3_servil_core::{Mode, convenience::hash_with, initialize, throughput::*};

fn check<P: Prepared>(mode: Mode, batch: &P, expected: &[[u8; 32]]) {
    let mut out = vec![[0; 32]; expected.len()];
    hash_many(mode, batch, &mut out);
    assert_eq!(out, expected);
    let start = out.len() / 3;
    out.fill([0; 32]);
    hash_many(mode, &batch.part(start..out.len()), &mut out[start..]);
    assert_eq!(out[start..], expected[start..]);
}
fn packed<const N: usize>(mode: Mode, count: usize) {
    let mut p = Packed::<N>::new(count);
    for (i, m) in p.iter_mut().enumerate() {
        *m = [i as u8; N];
    }
    let expected: Vec<_> = p.iter().map(|b| *hash_with(mode, b).as_bytes()).collect();
    check(mode, &p, &expected);
}
#[test]
fn every_batch_slot_hashes_as_its_message_alone() {
    initialize();
    let ctx = context_key("batch test");
    let lengths = [0, 1, 64, 1024, 1025, 16383, 16384, 16385];
    for mode in [Mode::Plain, Mode::Keyed(&[3; 32]), Mode::DeriveKey("batch test"), Mode::DeriveKeyWith(&ctx)] {
        for count in (0..=40).chain([127, 128, 129]) {
            packed::<16>(mode, count);
            packed::<32>(mode, count);
            for len in lengths {
                let mut batch = Batch::new(len, count);
                let mut expected = vec![];
                for i in 0..count {
                    let m = batch.message_mut(i);
                    for (j, x) in m.iter_mut().enumerate() {
                        *x = (i + 3 * j) as u8;
                    }
                    expected.push(*hash_with(mode, m).as_bytes());
                }
                check(mode, &batch, &expected);
                if len % 64 == 0 {
                    check(mode, &Batch::from_blocks(batch.blocks(), len), &expected);
                }
            }
            let blocks: Vec<_> = (0..count).map(|i| vec![Block([i as u8; 64]); lengths[i % 8].div_ceil(64)]).collect();
            let items: Vec<_> = blocks.iter().enumerate().map(|(i, b)| (b.as_slice(), lengths[i % 8])).collect();
            let mut out = vec![[0; 32]; count];
            hash_each(mode, &items, &mut out);
            for ((b, len), got) in items.into_iter().zip(out) {
                assert_eq!(got, *hash_with(mode, &Block::bytes(b)[..len]).as_bytes());
            }
        }
    }
}
