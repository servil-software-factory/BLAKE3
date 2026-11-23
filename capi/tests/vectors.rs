//! The C API against the official test vectors (test_vectors.json), through
//! every call: one-shot, and a hasher fed in uneven pieces, its digest and
//! its extended output at an offset.

use b3_servil::*;
use core::ffi::c_void;

#[path = "../../test_vectors/src/parse.rs"]
mod vectors;

#[test]
fn every_call_against_the_test_vectors() {
    let data = vectors::parse_json(include_str!("../../test_vectors/test_vectors.json"));
    let key: [u8; 32] = data.key.as_bytes().try_into().unwrap();
    let context = std::ffi::CString::new(data.context_string.as_str()).unwrap();
    let modes = unsafe { [("hash", b3_mode_plain()), ("keyed_hash", b3_mode_keyed(key.as_ptr())), ("derive_key", b3_mode_derive_key(context.as_ptr()))] };
    let mut hasher: Box<B3Hasher> = unsafe { Box::new(core::mem::zeroed()) };
    for case in &data.cases {
        let len = case.input_len;
        let input: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
        for (name, mode) in &modes {
            let expected = match *name {
                "hash" => case.hash.as_str(),
                "keyed_hash" => case.keyed_hash.as_str(),
                "derive_key" => case.derive_key.as_str(),
                _ => unreachable!(),
            };
            let what = format!("{name}, {len} bytes");
            unsafe {
                assert_eq!(vectors::hex_encode(&b3_hash(mode, input.as_ptr() as *const c_void, len).bytes), expected[..64], "b3_hash, {what}");
                // A hasher fed in pieces of 1, 1000, and the rest.
                b3_hasher_new(&mut *hasher, mode, 0, 0);
                let (a, rest) = input.split_at(len.min(1));
                let (b, c) = rest.split_at(rest.len().min(1000));
                b3_hasher_update(&mut *hasher, a.as_ptr() as *const c_void, a.len());
                b3_hasher_update(&mut *hasher, b.as_ptr() as *const c_void, b.len());
                b3_hasher_update(&mut *hasher, c.as_ptr() as *const c_void, c.len());
                assert_eq!(vectors::hex_encode(&b3_hasher_finalize(&mut *hasher).bytes), expected[..64], "b3_hasher_finalize, {what}");
                let full = expected.len() / 2;
                let mut output = vec![0u8; full - 7];
                // A finalize leaves the hasher as it was: asked again it
                // answers alike, in either form.
                b3_hasher_finalize_xof(&mut *hasher, 7, output.as_mut_ptr() as *mut c_void, output.len());
                assert_eq!(vectors::hex_encode(&output), expected[14..], "b3_hasher_finalize_xof from byte 7, {what}");
                assert_eq!(vectors::hex_encode(&b3_hasher_finalize(&mut *hasher).bytes), expected[..64], "b3_hasher_finalize again, {what}");
                // And updates after a finalize extend the message: the
                // input in two halves, a finalize between them.
                b3_hasher_new(&mut *hasher, mode, 0, 0);
                let (first, second) = input.split_at(len / 2);
                b3_hasher_update(&mut *hasher, first.as_ptr() as *const c_void, first.len());
                assert_eq!(b3_hasher_finalize(&mut *hasher).bytes, b3_hash(mode, first.as_ptr() as *const c_void, first.len()).bytes, "the first half, {what}");
                b3_hasher_update(&mut *hasher, second.as_ptr() as *const c_void, second.len());
                assert_eq!(vectors::hex_encode(&b3_hasher_finalize(&mut *hasher).bytes), expected[..64], "extended after a finalize, {what}");
            }
        }
    }
}

/// The empty input through NULL pointers, as the contract allows.
#[test]
fn null_with_nothing() {
    let mode = b3_mode_plain();
    let empty = unsafe { b3_hash(&mode, core::ptr::null(), 0) };
    assert_eq!(empty.bytes, *blake3_servil::convenience::hash(b"").as_bytes());
}
