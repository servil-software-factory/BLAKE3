//! A hasher wiped with `Zeroize` and then used in a grouped call must not
//! change the hash of a hasher beside it.
#![cfg(feature = "zeroize")]
use blake3_servil_core as b3;
use zeroize::Zeroize;

#[test]
fn a_wiped_hasher_leaves_its_neighbours_hash_alone() {
    b3::initialize();
    let input: Vec<u8> = (0..64 * 1024).map(|i| (i % 251) as u8).collect();
    let want = blake3::hash(&input);
    for wiped in [0, 1] {
        for was in [b3::Mode::Plain, b3::Mode::Keyed(&[7; 32])] {
            let mut hashers = [b3::convenience::Hasher::new(b3::Mode::Plain, None), b3::convenience::Hasher::new(b3::Mode::Plain, None)];
            hashers[wiped] = b3::convenience::Hasher::new(was, None);
            hashers[wiped].update(&input[..5000]);
            hashers[wiped].zeroize();
            for piece in input.chunks(4096) {
                b3::throughput::update_each(&mut hashers, &[(0, piece), (1, piece)]);
            }
            // Each is checked against official BLAKE3: the wiped one is an empty plain hasher.
            for hasher in hashers {
                assert_eq!(hasher.finalize().as_bytes(), want.as_bytes(), "wiped hasher at index {wiped}");
            }
        }
    }
}
