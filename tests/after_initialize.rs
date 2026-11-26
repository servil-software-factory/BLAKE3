// The library as a program builds it (no cfg(test)): after initialize(),
// each kind of call gives official BLAKE3's digest. The unit tests run a
// build that initializes itself, and the test vectors are a crate of
// their own, so this is the suite's one look at the shipped paths.

use blake3_servil_core::{Mode, convenience};

#[test]
fn after_initialize_each_kind_of_call_hashes() {
    blake3_servil_core::initialize();
    let input: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
    for len in [0, 1, 64, 1024, 1025, 5000] {
        let message = &input[..len];
        let want = *blake3::hash(message).as_bytes();
        assert_eq!(*convenience::hash(message).as_bytes(), want, "hash, {len} bytes");
        let mut hasher = convenience::Hasher::new(Mode::Plain, None);
        hasher.update(&message[..len / 2]).update(&message[len / 2..]);
        assert_eq!(*hasher.finalize().as_bytes(), want, "the Hasher, {len} bytes");
    }
    let key = [7u8; 32];
    assert_eq!(*convenience::hash_with(Mode::Keyed(&key), &input).as_bytes(), *blake3::keyed_hash(&key, &input).as_bytes(), "the keyed hash");
    {
        use blake3_servil_core::throughput;
        let mut batch = throughput::Batch::new(100, 5);
        for i in 0..5 {
            batch.set(i, &input[i * 100..(i + 1) * 100]);
        }
        let mut out = [[0u8; 32]; 5];
        throughput::hash_many(Mode::Plain, &batch, &mut out);
        for (i, digest) in out.iter().enumerate() {
            assert_eq!(*digest, *blake3::hash(&input[i * 100..(i + 1) * 100]).as_bytes(), "hash_many, message {i}");
        }
    }
}
