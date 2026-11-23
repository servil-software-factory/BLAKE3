use blake3_servil::{
    Mode,
    convenience::{Hasher, hash_with},
    throughput::*,
};
#[test]
fn splitting_work_across_threads_preserves_each_message() {
    blake3_servil::initialize_multithreaded(Some(3.try_into().unwrap()));
    const SPLIT: usize = 512 * 1024;
    for mode in [Mode::Plain, Mode::Keyed(&[4; 32]), Mode::DeriveKey("pool test")] {
        // Cross the total-size cutoff with small slots too, where pieces split between messages.
        for count in (0..=40).chain([127, 128, 129]) {
            let lens = [0, 1, 64, 1024, 1025, 16383, 16384, 16385];
            for len in lens {
                let mut batch = Batch::new(len, count);
                let mut expected = Vec::new();
                for i in 0..count {
                    let m = batch.message_mut(i);
                    m.fill(i as u8);
                    expected.push(*hash_with(mode, m).as_bytes());
                }
                let mut out = vec![[0; 32]; count];
                hash_many_multithreaded(mode, &batch, &mut out);
                assert_eq!(out, expected);
                let start = count / 3;
                out.fill([0; 32]);
                hash_many_multithreaded(mode, &batch.part(start..count), &mut out[start..]);
                assert_eq!(&out[start..], &expected[start..]);
            }
            let mut packed = Packed::<32>::new(count);
            for (i, m) in packed.iter_mut().enumerate() {
                m.fill(i as u8);
            }
            let mut out = vec![[0; 32]; count];
            hash_many_multithreaded(mode, &packed, &mut out);
            for (digest, m) in out.iter().zip(packed.iter()) {
                assert_eq!(digest, hash_with(mode, m).as_bytes());
            }
            let blocks: Vec<_> = (0..count).map(|i| vec![Block([i as u8; 64]); lens[i % 8].div_ceil(64)]).collect();
            let items: Vec<_> = blocks.iter().enumerate().map(|(i, b)| (b.as_slice(), lens[i % 8])).collect();
            hash_each_multithreaded(mode, &items, &mut out);
            for (digest, (b, len)) in out.iter().zip(items) {
                assert_eq!(digest, hash_with(mode, &Block::bytes(b)[..len]).as_bytes());
            }
        }
        for len in [SPLIT / 2 - 64, SPLIT / 2, SPLIT / 2 + 1, SPLIT - 1, SPLIT, SPLIT + 1, 2 * SPLIT + 1] {
            let mut batch = Batch::new(len, 2);
            let mut expect = [[0; 32]; 2];
            for (i, e) in expect.iter_mut().enumerate() {
                let m = batch.message_mut(i);
                for (j, x) in m.iter_mut().enumerate() {
                    *x = (j % 251 + i) as u8;
                }
                *e = *hash_with(mode, m).as_bytes();
            }
            let mut out = [[0; 32]; 2];
            hash_many_multithreaded(mode, &batch, &mut out);
            assert_eq!(out, expect);
            let blocks: Vec<_> = batch.blocks().chunks(len.div_ceil(64)).collect();
            let items = [(blocks[0], len), (blocks[1], len - 1)];
            hash_each_multithreaded(mode, &items, &mut out);
            let mut h = [Hasher::new(mode, None), Hasher::new(Mode::Plain, None)];
            let a = &Block::bytes(blocks[0])[..len];
            let b = &Block::bytes(blocks[1])[..len - 1];
            assert_eq!(out, [expect[0], *hash_with(mode, b).as_bytes()]);
            update_each(&mut h, &[(1, b), (0, a), (1, b"!")]);
            assert_eq!(h[0].finalize().as_bytes(), &expect[0]);
            assert_eq!(h[1].finalize().as_bytes(), blake3::Hasher::new().update(b).update(b"!").finalize().as_bytes());
            update_multithreaded(&mut h[0], b);
            assert_eq!(h[0].finalize(), hash_with(mode, &[a, b].concat()));
        }
    }
}
