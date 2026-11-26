//! Constant-time 32-byte digest comparison, implemented directly without
//! third-party crates.
//!
//! On x86-64 and AArch64, the comparison lives in a single inline-assembly
//! block using only data-independent integer loads, XOR/OR reduction, and
//! flag-to-register conversion (`sete` / `cset`), preventing the optimizer from
//! inserting early-exit branches. Every instruction used is on Intel's DOITM
//! list (x86-64) and Arm's FEAT_DIT list (AArch64); like hashing, equality
//! leaves Arm `PSTATE.DIT` as the calling thread has it.

/// Constant-time equality over two 32-byte digests.
#[inline]
pub(crate) fn eq_32(a: &[u8; 32], b: &[u8; 32]) -> bool {
    #[cfg(all(target_arch = "x86_64", not(miri)))]
    {
        let out: u8;
        // Sound: `a` and `b` are valid references to 32 readable bytes each;
        // unaligned 64-bit loads are architectural on x86-64. `acc` and `tmp`
        // are early-clobber outputs (`out(reg)`) so they cannot alias `a` or
        // `b`; `out` (`lateout(reg_byte)`) is written only on the final `sete`
        // after all loads and reductions finish.
        unsafe {
            core::arch::asm!(
                "mov {acc}, qword ptr [{a}]",
                "xor {acc}, qword ptr [{b}]",
                "mov {tmp}, qword ptr [{a} + 8]",
                "xor {tmp}, qword ptr [{b} + 8]",
                "or  {acc}, {tmp}",
                "mov {tmp}, qword ptr [{a} + 16]",
                "xor {tmp}, qword ptr [{b} + 16]",
                "or  {acc}, {tmp}",
                "mov {tmp}, qword ptr [{a} + 24]",
                "xor {tmp}, qword ptr [{b} + 24]",
                "or  {acc}, {tmp}",
                "test {acc}, {acc}",
                "sete {out}",
                a = in(reg) a.as_ptr(),
                b = in(reg) b.as_ptr(),
                acc = out(reg) _,
                tmp = out(reg) _,
                out = lateout(reg_byte) out,
                options(pure, readonly, nostack),
            );
        }
        out != 0
    }

    #[cfg(all(target_arch = "aarch64", not(miri)))]
    {
        let out: u64;
        // Sound: `a` and `b` are valid references to 32 readable bytes each;
        // unaligned `ldp` is architectural on Linux and macOS AArch64. Scratch
        // registers (`out(reg)`) cannot alias `a` or `b`; `out` (`lateout`) is
        // written only on the final `cset` after all loads and reductions end.
        unsafe {
            core::arch::asm!(
                "ldp {a0}, {a1}, [{a}]",
                "ldp {b0}, {b1}, [{b}]",
                "eor {a0}, {a0}, {b0}",
                "eor {a1}, {a1}, {b1}",
                "orr {a0}, {a0}, {a1}",
                "ldp {a1}, {b0}, [{a}, #16]",
                "ldp {b1}, {t}, [{b}, #16]",
                "eor {a1}, {a1}, {b1}",
                "eor {b0}, {b0}, {t}",
                "orr {a1}, {a1}, {b0}",
                "orr {a0}, {a0}, {a1}",
                "cmp {a0}, #0",
                "cset {out}, eq",
                a = in(reg) a.as_ptr(),
                b = in(reg) b.as_ptr(),
                a0 = out(reg) _,
                a1 = out(reg) _,
                b0 = out(reg) _,
                b1 = out(reg) _,
                t = out(reg) _,
                out = lateout(reg) out,
                options(pure, readonly, nostack),
            );
        }
        out != 0
    }

    #[cfg(miri)]
    {
        eq_32_portable(a, b)
    }
}

/// Constant-time equality when `b.len() == 32`; returns `false` immediately
/// when `b.len() != 32` (length is public).
#[inline]
pub(crate) fn eq_slice_32(a: &[u8; 32], b: &[u8]) -> bool {
    let Ok(b): Result<&[u8; 32], _> = b.try_into() else {
        return false;
    };
    eq_32(a, b)
}

#[cfg(any(test, miri))]
#[inline]
fn eq_32_portable(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let mut diff = 0u64;
    let mut i = 0;
    while i < 32 {
        let aw = u64::from_le_bytes([a[i], a[i + 1], a[i + 2], a[i + 3], a[i + 4], a[i + 5], a[i + 6], a[i + 7]]);
        let bw = u64::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3], b[i + 4], b[i + 5], b[i + 6], b[i + 7]]);
        diff |= core::hint::black_box(aw ^ bw);
        i += 8;
    }
    core::hint::black_box(diff) == 0
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn eq_32_matches_every_single_bit_flip() {
        let a = core::array::from_fn(|i| (i as u8).wrapping_mul(37).wrapping_add(11));
        let mut b = a;
        assert!(eq_32(&a, &b));
        assert!(eq_32_portable(&a, &b));
        assert!(eq_slice_32(&a, &b[..]));

        for byte in 0..32 {
            for bit in 0..8 {
                b[byte] ^= 1 << bit;
                assert!(!eq_32(&a, &b), "missed flip at byte {byte} bit {bit}");
                assert!(!eq_32_portable(&a, &b), "portable missed flip at byte {byte} bit {bit}");
                assert!(!eq_slice_32(&a, &b[..]), "slice missed flip at byte {byte} bit {bit}");
                b[byte] ^= 1 << bit;
            }
        }
    }

    #[test]
    fn eq_32_handles_unaligned_buffers_and_aliasing() {
        let mut backing = [0u8; 64];
        for (i, slot) in backing.iter_mut().enumerate() {
            *slot = (i as u8).wrapping_mul(13);
        }
        for offset in 0..8 {
            let a: &[u8; 32] = backing[offset..offset + 32].try_into().unwrap();
            let mut copy = [0u8; 40];
            copy[offset..offset + 32].copy_from_slice(a);
            let b: &[u8; 32] = copy[offset..offset + 32].try_into().unwrap();
            assert!(eq_32(a, a));
            assert!(eq_32(a, b));
            assert!(eq_slice_32(a, &copy[offset..offset + 32]));
            copy[offset + 31] ^= 0x80;
            let b_diff: &[u8; 32] = copy[offset..offset + 32].try_into().unwrap();
            assert!(!eq_32(a, b_diff));
            assert!(!eq_slice_32(a, &copy[offset..offset + 32]));
        }
    }

    #[test]
    fn eq_slice_32_rejects_other_lengths() {
        let a = [0x5au8; 32];
        assert!(!eq_slice_32(&a, &[]));
        assert!(!eq_slice_32(&a, &[0x5a; 31]));
        assert!(!eq_slice_32(&a, &[0x5a; 33]));
        assert!(!eq_slice_32(&a, &[0x5a; 64]));
        assert!(eq_slice_32(&a, &[0x5a; 32]));
    }

    #[test]
    fn eq_32_does_not_cancel_multi_word_differences() {
        let a = [0u8; 32];
        // Identical bit flips across distinct 64-bit words would cancel if an
        // `or` reduction step were mistakenly replaced with `xor`.
        let word_offsets = [0usize, 8, 16, 24];
        for i in 0..word_offsets.len() {
            for j in (i + 1)..word_offsets.len() {
                for bit in [0u8, 7] {
                    let mut b = a;
                    b[word_offsets[i]] ^= 1 << bit;
                    b[word_offsets[j]] ^= 1 << bit;
                    assert!(!eq_32(&a, &b), "words {i},{j} bit {bit} cancelled in eq_32");
                    assert!(!eq_32_portable(&a, &b), "words {i},{j} bit {bit} cancelled in eq_32_portable");
                    assert!(!eq_slice_32(&a, &b[..]), "words {i},{j} bit {bit} cancelled in eq_slice_32");
                }
            }
        }
        let mut all_four = a;
        for off in word_offsets {
            all_four[off] = 0xff;
        }
        assert!(!eq_32(&a, &all_four));
        assert!(!eq_32_portable(&a, &all_four));
        assert!(!eq_slice_32(&a, &all_four[..]));
    }
}
