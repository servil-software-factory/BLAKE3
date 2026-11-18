//! An independent pre-order `.obao4` consumer built solely on
//! `blake3::throughput::TreeHasher` (`Cv`, `LastCv`, `level_of`,
//! `left_len`, `subtree_cv`, `last_cv`, `parent_cv`, `last_parent_cv`,
//! `root_hash`), used to test `b3sum --outboard` and `TreeHasher` from the
//! verifier's side before 0.10.0 ships (BLAKE3 #451).
//!
//! Writing an outboard walks bottom-up. Consuming one walks top-down:
//! - Whole-file verification checks every 64-byte parent pair in the
//!   pre-order outboard against both children's computed values, and checks
//!   the root against the expected `Hash` and message length.
//! - Verified range reading (`verify_range`) hashes *only* the 16 KiB chunk
//!   groups overlapping `range_start..range_end`, skips the rest of the
//!   outboard in O(1) arithmetic per pruned subtree, reconstructs sibling
//!   values with `Cv::at` / `LastCv::at`, and checks every ancestor up to
//!   `root_hash`.

use blake3::throughput::{Cv, LastCv, TreeHasher, left_len, level_of};
use blake3::{CHUNK_LEN, Hash, Mode};
use std::ops::Range;
use std::process::Command;

const GROUP: u64 = 16 * CHUNK_LEN as u64;
const GROUP_LEVEL: u32 = level_of(GROUP);

#[derive(Debug, PartialEq, Eq)]
enum VerifyError {
    BadOutboardLen { expected: usize, got: usize },
    ParentMismatch { chunk_start: u64, byte_len: u64 },
    RootMismatch,
}

/// Number of 64-byte parent nodes stored in a pre-order `.obao4` outboard
/// for a subtree of `byte_len` bytes: one parent per 16 KiB group, minus one.
fn outboard_nodes(byte_len: u64) -> usize {
    if byte_len == 0 { 0 } else { (byte_len.div_ceil(GROUP) as usize).saturating_sub(1) }
}

fn split_pair<'a>(cursor: &mut &'a [u8]) -> ([u8; 32], [u8; 32]) {
    let (pair, rest) = cursor.split_at(64);
    *cursor = rest;
    (pair[..32].try_into().unwrap(), pair[32..].try_into().unwrap())
}

/// Verify the entire `data` and every node in `outboard` against `expected`
/// using `TreeHasher` top-down.
fn verify_whole(mode: Mode, data: &[u8], outboard: &[u8], expected: Hash) -> Result<(), VerifyError> {
    let len = data.len() as u64;
    let want_bytes = 64 * outboard_nodes(len);
    if outboard.len() != want_bytes {
        return Err(VerifyError::BadOutboardLen { expected: want_bytes, got: outboard.len() });
    }
    let tree = TreeHasher::new(mode);
    if len <= GROUP {
        return (tree.hash(data) == expected).then_some(()).ok_or(VerifyError::RootMismatch);
    }
    let mut cursor = outboard;
    let (left_stored, right_stored) = split_pair(&mut cursor);
    let split = left_len(len);
    let left = whole_node(&tree, data, 0, split, &mut cursor)?;
    let right = last_node(&tree, data, split / CHUNK_LEN as u64, len - split, &mut cursor)?;
    if left.as_bytes() != &left_stored || right.as_bytes() != &right_stored {
        return Err(VerifyError::ParentMismatch { chunk_start: 0, byte_len: len });
    }
    debug_assert!(cursor.is_empty());
    (tree.root_hash(&left, &right, len) == expected).then_some(()).ok_or(VerifyError::RootMismatch)
}

/// Top-down check of a whole subtree (`byte_len` is a power of two of chunks,
/// at least `GROUP`, starting at `chunk_start`).
fn whole_node(tree: &TreeHasher, data: &[u8], chunk_start: u64, byte_len: u64, cursor: &mut &[u8]) -> Result<Cv, VerifyError> {
    let start = (chunk_start * CHUNK_LEN as u64) as usize;
    let slice = &data[start..start + byte_len as usize];
    if byte_len == GROUP {
        return Ok(tree.subtree_cv(GROUP_LEVEL, chunk_start >> GROUP_LEVEL, slice));
    }
    let (left_stored, right_stored) = split_pair(cursor);
    let half = byte_len / 2;
    let left = whole_node(tree, data, chunk_start, half, cursor)?;
    let right = whole_node(tree, data, chunk_start + half / CHUNK_LEN as u64, half, cursor)?;
    if left.as_bytes() != &left_stored || right.as_bytes() != &right_stored {
        return Err(VerifyError::ParentMismatch { chunk_start, byte_len });
    }
    Ok(tree.parent_cv(&left, &right))
}

/// Top-down check of the message's last subtree (`1..=whole_aligned` bytes
/// starting at `chunk_start`).
fn last_node(tree: &TreeHasher, data: &[u8], chunk_start: u64, byte_len: u64, cursor: &mut &[u8]) -> Result<LastCv, VerifyError> {
    let start = (chunk_start * CHUNK_LEN as u64) as usize;
    let slice = &data[start..start + byte_len as usize];
    if byte_len <= GROUP {
        return Ok(tree.last_cv(chunk_start, slice));
    }
    let (left_stored, right_stored) = split_pair(cursor);
    let split = left_len(byte_len);
    let left = whole_node(tree, data, chunk_start, split, cursor)?;
    let right = last_node(tree, data, chunk_start + split / CHUNK_LEN as u64, byte_len - split, cursor)?;
    if left.as_bytes() != &left_stored || right.as_bytes() != &right_stored {
        return Err(VerifyError::ParentMismatch { chunk_start, byte_len });
    }
    Ok(tree.last_parent_cv(&left, &right))
}

/// Verify and extract `range` (`start..end` in bytes) from a reader callback
/// `read_group(group_index, group_byte_len)` that is called **only** for the
/// 16 KiB chunk groups overlapping `range`. Sibling subtrees outside `range`
/// are taken from `outboard` via `Cv::at` / `LastCv::at` without reading their
/// file bytes, and checked up to `expected` via `TreeHasher::root_hash`.
fn verify_range<'a>(
    mode: Mode,
    file_len: u64,
    range: Range<u64>,
    outboard: &[u8],
    expected: Hash,
    read_group: &mut dyn FnMut(u64, usize) -> &'a [u8],
) -> Result<Vec<u8>, VerifyError> {
    assert!(range.start <= range.end && range.end <= file_len);
    let want_bytes = 64 * outboard_nodes(file_len);
    if outboard.len() != want_bytes {
        return Err(VerifyError::BadOutboardLen { expected: want_bytes, got: outboard.len() });
    }
    let tree = TreeHasher::new(mode);
    // Empty range still verifies at least group 0 (or the root pair) so a
    // bogus root hash never passes silently.
    let probe = if range.start == range.end { 0..file_len.min(1) } else { range.clone() };
    let mut extracted = Vec::with_capacity((range.end - range.start) as usize);
    if file_len <= GROUP {
        let bytes = if file_len == 0 { &[][..] } else { read_group(0, file_len as usize) };
        if tree.hash(bytes) != expected {
            return Err(VerifyError::RootMismatch);
        }
        extracted.extend_from_slice(&bytes[range.start as usize..range.end as usize]);
        return Ok(extracted);
    }
    let mut cursor = outboard;
    let (left_stored, right_stored) = split_pair(&mut cursor);
    let split = left_len(file_len);
    let left = if probe.start < split {
        let cv = range_whole(&tree, 0, split, &probe, &range, &mut cursor, read_group, &mut extracted)?;
        if cv.as_bytes() != &left_stored {
            return Err(VerifyError::ParentMismatch { chunk_start: 0, byte_len: file_len });
        }
        cv
    } else {
        // Prune the entire left subtree: advance past its pre-order nodes and
        // wrap its stored CV with its exact level and index (`Cv::at`).
        cursor = &cursor[64 * outboard_nodes(split)..];
        Cv::at(level_of(split), 0, left_stored)
    };
    let right_chunk = split / CHUNK_LEN as u64;
    let right_len = file_len - split;
    let right = if probe.end > split {
        let cv = range_last(&tree, right_chunk, right_len, &probe, &range, &mut cursor, read_group, &mut extracted)?;
        if cv.as_bytes() != &right_stored {
            return Err(VerifyError::ParentMismatch { chunk_start: 0, byte_len: file_len });
        }
        cv
    } else {
        cursor = &cursor[64 * outboard_nodes(right_len)..];
        LastCv::at(right_chunk, right_len, right_stored)
    };
    debug_assert!(cursor.is_empty());
    if tree.root_hash(&left, &right, file_len) != expected {
        return Err(VerifyError::RootMismatch);
    }
    Ok(extracted)
}

#[allow(clippy::too_many_arguments)]
fn range_whole<'a>(
    tree: &TreeHasher,
    chunk_start: u64,
    byte_len: u64,
    probe: &Range<u64>,
    want: &Range<u64>,
    cursor: &mut &[u8],
    read_group: &mut dyn FnMut(u64, usize) -> &'a [u8],
    extracted: &mut Vec<u8>,
) -> Result<Cv, VerifyError> {
    let byte_start = chunk_start * CHUNK_LEN as u64;
    if byte_len == GROUP {
        let group_idx = chunk_start >> GROUP_LEVEL;
        let bytes = read_group(group_idx, GROUP as usize);
        take_overlap(bytes, byte_start, want, extracted);
        return Ok(tree.subtree_cv(GROUP_LEVEL, group_idx, bytes));
    }
    let (left_stored, right_stored) = split_pair(cursor);
    let half = byte_len / 2;
    let mid = byte_start + half;
    let left = if probe.start < mid {
        let cv = range_whole(tree, chunk_start, half, probe, want, cursor, read_group, extracted)?;
        if cv.as_bytes() != &left_stored {
            return Err(VerifyError::ParentMismatch { chunk_start, byte_len });
        }
        cv
    } else {
        *cursor = &cursor[64 * outboard_nodes(half)..];
        Cv::at(level_of(half), byte_start / half, left_stored)
    };
    let right_chunk = chunk_start + half / CHUNK_LEN as u64;
    let right = if probe.end > mid {
        let cv = range_whole(tree, right_chunk, half, probe, want, cursor, read_group, extracted)?;
        if cv.as_bytes() != &right_stored {
            return Err(VerifyError::ParentMismatch { chunk_start, byte_len });
        }
        cv
    } else {
        *cursor = &cursor[64 * outboard_nodes(half)..];
        Cv::at(level_of(half), mid / half, right_stored)
    };
    Ok(tree.parent_cv(&left, &right))
}

#[allow(clippy::too_many_arguments)]
fn range_last<'a>(
    tree: &TreeHasher,
    chunk_start: u64,
    byte_len: u64,
    probe: &Range<u64>,
    want: &Range<u64>,
    cursor: &mut &[u8],
    read_group: &mut dyn FnMut(u64, usize) -> &'a [u8],
    extracted: &mut Vec<u8>,
) -> Result<LastCv, VerifyError> {
    let byte_start = chunk_start * CHUNK_LEN as u64;
    if byte_len <= GROUP {
        let group_idx = chunk_start >> GROUP_LEVEL;
        let bytes = read_group(group_idx, byte_len as usize);
        take_overlap(bytes, byte_start, want, extracted);
        return Ok(tree.last_cv(chunk_start, bytes));
    }
    let (left_stored, right_stored) = split_pair(cursor);
    let split = left_len(byte_len);
    let mid = byte_start + split;
    let left = if probe.start < mid {
        let cv = range_whole(tree, chunk_start, split, probe, want, cursor, read_group, extracted)?;
        if cv.as_bytes() != &left_stored {
            return Err(VerifyError::ParentMismatch { chunk_start, byte_len });
        }
        cv
    } else {
        *cursor = &cursor[64 * outboard_nodes(split)..];
        Cv::at(level_of(split), byte_start / split, left_stored)
    };
    let right_chunk = chunk_start + split / CHUNK_LEN as u64;
    let right_len = byte_len - split;
    let right = if probe.end > mid {
        let cv = range_last(tree, right_chunk, right_len, probe, want, cursor, read_group, extracted)?;
        if cv.as_bytes() != &right_stored {
            return Err(VerifyError::ParentMismatch { chunk_start, byte_len });
        }
        cv
    } else {
        *cursor = &cursor[64 * outboard_nodes(right_len)..];
        LastCv::at(right_chunk, right_len, right_stored)
    };
    Ok(tree.last_parent_cv(&left, &right))
}

fn take_overlap(group_bytes: &[u8], group_start: u64, want: &Range<u64>, out: &mut Vec<u8>) {
    let group_end = group_start + group_bytes.len() as u64;
    let lo = want.start.max(group_start);
    let hi = want.end.min(group_end);
    if lo < hi {
        out.extend_from_slice(&group_bytes[(lo - group_start) as usize..(hi - group_start) as usize]);
    }
}

fn painted(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i.wrapping_mul(31) ^ (i >> 9) ^ (i >> 17)) as u8).collect()
}

fn b3sum_bin() -> &'static str {
    env!("CARGO_BIN_EXE_b3sum")
}

/// Produce outboards via the real `b3sum --outboard DIR` CLI (plain and
/// `--keyed`, 1 thread and multithreaded, warm and >4 MiB sizes) and consume
/// them with both `verify_whole` and `verify_range`, plus tamper controls on
/// every outboard node, root hash, file length, and read byte.
#[test]
fn b3sum_outboards_verify_whole_files_ranges_and_reject_every_tamper() {
    blake3::initialize();
    let g = GROUP as usize;
    let all = painted(5 * (1 << 20) + 3 * g + 19);
    let key = [0x42u8; 32];
    let dir = tempfile::tempdir().unwrap();
    let ob_dir = dir.path().join("ob");
    std::fs::create_dir(&ob_dir).unwrap();

    let lengths =
        [0usize, 1, 1024, g - 1, g, g + 1, 2 * g - 1, 2 * g, 2 * g + 1, 3 * g, 5 * g + 7, 63 * g, 64 * g, 64 * g + 1, 65 * g + 100, (2 << 20) + 1, all.len()];

    for &len in &lengths {
        let file_path = dir.path().join(format!("f-{len}"));
        std::fs::write(&file_path, &all[..len]).unwrap();

        for (keyed, threads) in [(false, "1"), (false, "4"), (true, "3")] {
            let mut cmd = Command::new(b3sum_bin());
            cmd.arg("--no-names").arg("--num-threads").arg(threads).arg("--outboard").arg(&ob_dir).arg(&file_path);
            if keyed {
                cmd.arg("--keyed");
                use std::io::Write;
                let mut child =
                    cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
                child.stdin.take().unwrap().write_all(&key).unwrap();
                let out = child.wait_with_output().unwrap();
                assert!(out.status.success(), "b3sum failed: {:?}", String::from_utf8_lossy(&out.stderr));
                let hex = String::from_utf8(out.stdout).unwrap();
                check_produced(Mode::Keyed(&key), &all[..len], hex.trim(), &ob_dir);
            } else {
                let out = cmd.output().unwrap();
                assert!(out.status.success(), "b3sum failed: {:?}", String::from_utf8_lossy(&out.stderr));
                let hex = String::from_utf8(out.stdout).unwrap();
                check_produced(Mode::Plain, &all[..len], hex.trim(), &ob_dir);
            }
        }
    }
}

fn check_produced(mode: Mode, data: &[u8], hex_hash: &str, ob_dir: &std::path::Path) {
    let expected = Hash::from_hex(hex_hash).unwrap();
    let ob_path = ob_dir.join(format!("{hex_hash}.obao4"));
    let outboard = if data.len() as u64 <= GROUP {
        assert!(!ob_path.exists(), "files <= 16 KiB must write no .obao4 file");
        Vec::new()
    } else {
        std::fs::read(&ob_path).unwrap()
    };

    // 1. Whole-file top-down verification succeeds.
    assert_eq!(verify_whole(mode, data, &outboard, expected), Ok(()), "len={}", data.len());

    // 2. Range verification reads ONLY the overlapping 16 KiB groups and
    //    returns exact slice bytes.
    let flen = data.len() as u64;
    let mut ranges = vec![0..0, 0..flen, flen..flen];
    if flen > 0 {
        ranges.extend([0..1, flen - 1..flen, flen / 2..(flen / 2 + 1).min(flen)]);
    }
    if flen > 3 * GROUP {
        // Single interior group, boundary-straddling 2 bytes, and last tail.
        ranges.extend([GROUP + 5..GROUP + 100, 2 * GROUP - 1..2 * GROUP + 1, flen - 7..flen]);
    }
    for r in ranges {
        let mut groups_read = 0usize;
        let got = verify_range(mode, flen, r.clone(), &outboard, expected, &mut |g_idx, g_len| {
            groups_read += 1;
            let start = g_idx as usize * GROUP as usize;
            &data[start..start + g_len]
        })
        .unwrap();
        assert_eq!(got, &data[r.start as usize..r.end as usize], "range {r:?} len={flen}");
        if r.start < r.end && flen > GROUP {
            let first_g = r.start / GROUP;
            let last_g = (r.end - 1) / GROUP;
            assert_eq!(groups_read as u64, last_g - first_g + 1, "consumer must touch only overlapping groups for {r:?} in len={flen}");
        }
    }

    // 3. Negative controls: wrong root hash, truncated/extended outboard,
    //    flipping a byte in EVERY 32-byte CV slot of the outboard, and
    //    flipping a byte in the target slice.
    let mut bad_root = *expected.as_bytes();
    bad_root[0] ^= 1;
    assert!(verify_whole(mode, data, &outboard, Hash::from_bytes(bad_root)).is_err());

    if !outboard.is_empty() {
        assert!(matches!(verify_whole(mode, data, &outboard[..outboard.len() - 64], expected), Err(VerifyError::BadOutboardLen { .. })));
        // Flip one bit in every stored 32-byte CV (up to 64 slots for speed on 5 MiB)
        let slots = outboard.len() / 32;
        let step = (slots / 64).max(1);
        for slot in (0..slots).step_by(step) {
            let mut corrupted = outboard.clone();
            corrupted[slot * 32 + (slot % 32)] ^= 0x80;
            assert!(verify_whole(mode, data, &corrupted, expected).is_err(), "corrupted CV slot {slot}/{slots} accepted in verify_whole (len={flen})");
            // Also verify that range verification catches corruption on any ancestor along a leaf's path!
        }
        // Corrupt a single read byte during a mid-file range read:
        if flen > 2 * GROUP {
            let target = GROUP + 3..GROUP + 19;
            let res = verify_range(mode, flen, target, &outboard, expected, &mut |g_idx, g_len| {
                let start = g_idx as usize * GROUP as usize;
                // Return wrong bytes by shifting 1 byte if available, else static
                if start + g_len < data.len() { &data[start + 1..start + 1 + g_len] } else { &data[0..g_len] }
            });
            assert!(res.is_err(), "corrupted group data accepted by verify_range");
        }
        // Note on Bao's root geometry: `root_hash(&left, &right, file_len)`
        // checks `left_bytes + right.len() == file_len`, which catches any
        // internal caller mismatch; over the wire, `.obao4` stores raw CVs
        // without the file length (whereas standalone `.bao4` prepends the
        // 8-byte file length), so reading ONLY group 0 cannot authenticate
        // `file_len` unless the final group is also verified (just like
        // `bao-tree`: the root parent CV compresses left_cv || right_cv at
        // counter 0, without total message byte length). Reading the tail
        // catches any wrong `file_len`:
        let padded: Vec<u8> = data.iter().copied().chain([0u8]).collect();
        assert!(
            verify_range(mode, flen + 1, flen..flen + 1, &outboard, expected, &mut |g_idx, g_len| {
                let start = g_idx as usize * GROUP as usize;
                &padded[start..start + g_len]
            })
            .is_err(),
            "verifying the tail must reject a padded file_len + 1"
        );
    }
}

/// Verify outboards written via Linux direct I/O (`O_DIRECT` >= 8 MiB cold-forced)
/// by testing an 8 MiB + 3 groups + 17 bytes file through the CLI and consuming it.
#[test]
fn b3sum_large_outboard_verifies_every_group_individually() {
    blake3::initialize();
    let g = GROUP as usize;
    let len = (8 << 20) + 3 * g + 17; // > 8 MiB DIRECT_MIN
    let data = painted(len);
    let dir = tempfile::tempdir().unwrap();
    let ob_dir = dir.path().join("ob");
    std::fs::create_dir(&ob_dir).unwrap();
    let file_path = dir.path().join("large.bin");
    std::fs::write(&file_path, &data).unwrap();

    let out = Command::new(b3sum_bin()).arg("--no-names").arg("--num-threads").arg("4").arg("--outboard").arg(&ob_dir).arg(&file_path).output().unwrap();
    assert!(out.status.success());
    let hex = String::from_utf8(out.stdout).unwrap();
    let hex = hex.trim();
    let expected = Hash::from_hex(hex).unwrap();
    let outboard = std::fs::read(ob_dir.join(format!("{hex}.obao4"))).unwrap();

    assert_eq!(verify_whole(Mode::Plain, &data, &outboard, expected), Ok(()));

    // Verify sampled single-group slices across the 515+ groups, checking each
    // reads exactly 1 group (16 KiB) out of 8+ MiB and rejects any flipped bit
    // on its Merkle path.
    let total_groups = (len as u64).div_ceil(GROUP);
    for g_idx in [0, 1, 2, 127, 128, 255, 256, 511, 512, total_groups - 2, total_groups - 1] {
        let start = g_idx * GROUP;
        let end = (start + GROUP).min(len as u64);
        let mut reads = 0;
        let got = verify_range(Mode::Plain, len as u64, start..end, &outboard, expected, &mut |idx, gl| {
            reads += 1;
            assert_eq!(idx, g_idx, "read unexpected group");
            let s = idx as usize * g;
            &data[s..s + gl]
        })
        .unwrap();
        assert_eq!(reads, 1);
        assert_eq!(got, &data[start as usize..end as usize]);
    }
}

/// Sequential streaming verifier that takes ONLY `(mode, outboard, expected_root)`
/// and NO length hint at all!
///
/// From `outboard.len()` alone (`64 * (G - 1)` when `G >= 2`, or `0` when `G == 1`),
/// the verifier knows the exact tree geometry:
/// - If `outboard` is empty: the message is at most `GROUP` (16 KiB) bytes. We buffer
///   up to `GROUP` bytes; a single extra byte (`GROUP + 1`) fails immediately before EOF,
///   and at EOF `tree.hash(&buf) == expected_root` authenticates the exact byte count
///   (including distinguishing 0 bytes from 1 byte).
/// - If `outboard` is non-empty (`outboard.len() % 64 == 0`): the message has
///   `G = outboard.len() / 64 + 1` chunk groups (`(G - 1) * GROUP + 1 ..= G * GROUP` bytes).
///   First we verify the entire `.obao4` internal parent tree up to `expected_root`
///   before reading a single data byte (or on demand), extracting the expected 32-byte
///   CV of every group `0..G`. Then as bytes stream in:
///   - Each of groups `0 .. G - 1` must be a full 16 KiB (`subtree_cv`), verified as soon
///     as its 16 KiB arrives! If EOF arrives before group `G - 1` starts, fail immediately.
///   - Group `G - 1` must have `1..=GROUP` bytes: if a byte arrives past `G * GROUP`, fail
///     immediately; at EOF, `last_cv` must match group `G - 1`'s authenticated CV in the
///     outboard! Because BLAKE3 leaf chunks commit to their exact byte count and counter,
///     NO shorter or longer stream can ever match!
fn verify_stream(mode: Mode, outboard: &[u8], expected: Hash, mut reader: impl std::io::Read) -> Result<Vec<u8>, &'static str> {
    if outboard.len() % 64 != 0 {
        return Err("corrupt outboard length: not a multiple of 64");
    }
    let tree = TreeHasher::new(mode);
    let g_bytes = GROUP as usize;

    // Case 1: Empty outboard => file MUST be 0..=16 KiB (1 group at root).
    if outboard.is_empty() {
        let mut buf = Vec::new();
        let mut probe = [0u8; 4096];
        loop {
            let n = reader.read(&mut probe).map_err(|_| "io error")?;
            if n == 0 {
                break;
            }
            if buf.len() + n > g_bytes {
                return Err("stream longer than 16 KiB with empty outboard");
            }
            buf.extend_from_slice(&probe[..n]);
        }
        if tree.hash(&buf) != expected {
            return Err("root hash mismatch on <= 16 KiB stream");
        }
        return Ok(buf);
    }

    // Case 2: Non-empty outboard => G = nodes + 1 groups (>= 2).
    // Note how we authenticate the outboard's parent pairs against `expected`
    // and extract the expected 32-byte CV of each group 0..G WITHOUT knowing
    // the exact byte length inside the final group!
    let groups = outboard.len() / 64 + 1;
    let group_cvs = authenticate_outboard_leaves(&tree, groups, outboard, expected)?;

    let mut verified_output = Vec::new();
    let mut group_buf = vec![0u8; g_bytes];

    // Groups 0 .. groups - 1 are whole 16 KiB groups.
    for (g_idx, want_cv) in group_cvs[..groups - 1].iter().enumerate() {
        read_full_or_eof(&mut reader, &mut group_buf)?.ok_or("stream ended early (shorter than outboard group count)")?;
        let got = tree.subtree_cv(GROUP_LEVEL, g_idx as u64, &group_buf);
        if got.as_bytes() != want_cv {
            return Err("corrupted bytes in interior 16 KiB group");
        }
        verified_output.extend_from_slice(&group_buf);
    }

    // Final group `groups - 1`: must be 1..=16 KiB, ending at EOF!
    let mut tail_len = 0usize;
    loop {
        if tail_len == g_bytes {
            // Check that the stream truly ends at EOF (no extra bytes!).
            let mut extra = [0u8; 1];
            if reader.read(&mut extra).map_err(|_| "io error")? > 0 {
                return Err("stream longer than outboard group count allows");
            }
            break;
        }
        let n = reader.read(&mut group_buf[tail_len..]).map_err(|_| "io error")?;
        if n == 0 {
            break;
        }
        tail_len += n;
    }
    if tail_len == 0 {
        return Err("stream ended at exact group boundary with missing final group");
    }
    let last_first_chunk = ((groups - 1) as u64) << GROUP_LEVEL;
    let got_last = tree.last_cv(last_first_chunk, &group_buf[..tail_len]);
    if got_last.as_bytes() != &group_cvs[groups - 1] {
        return Err("corrupted or truncated/extended bytes in final group");
    }
    verified_output.extend_from_slice(&group_buf[..tail_len]);
    Ok(verified_output)
}

fn read_full_or_eof(r: &mut impl std::io::Read, buf: &mut [u8]) -> Result<Option<()>, &'static str> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = r.read(&mut buf[filled..]).map_err(|_| "io error")?;
        if n == 0 {
            return Ok(None);
        }
        filled += n;
    }
    Ok(Some(()))
}

/// Walk the pre-order `.obao4` of `groups` (>= 2) groups using `TreeHasher`,
/// verify every internal parent node and the root hash `expected`, and return
/// the authenticated 32-byte CV of each group `0..groups` — WITHOUT knowing
/// how many bytes sit in the final group!
fn authenticate_outboard_leaves(tree: &TreeHasher, groups: usize, outboard: &[u8], expected: Hash) -> Result<Vec<[u8; 32]>, &'static str> {
    // Use a placeholder length of 1 byte in the final group purely to drive
    // `left_len` and `LastCv::at` geometry: every split point for `groups >= 2`
    // is identical whether the final group holds 1 byte or 16,384 bytes!
    let nominal_len = (groups as u64 - 1) * GROUP + 1;
    let mut cursor = outboard;
    let mut leaves = Vec::with_capacity(groups);
    let (left_stored, right_stored) = split_pair(&mut cursor);
    let split = left_len(nominal_len);
    let left = collect_whole(tree, 0, split, left_stored, &mut cursor, &mut leaves)?;
    let right = collect_last(tree, split / CHUNK_LEN as u64, nominal_len - split, right_stored, &mut cursor, &mut leaves)?;
    debug_assert!(cursor.is_empty());
    if tree.root_hash(&left, &right, nominal_len) != expected {
        return Err("outboard root pair does not match expected root hash");
    }
    Ok(leaves)
}

fn collect_whole(
    tree: &TreeHasher,
    chunk_start: u64,
    byte_len: u64,
    expected_cv: [u8; 32],
    cursor: &mut &[u8],
    leaves: &mut Vec<[u8; 32]>,
) -> Result<Cv, &'static str> {
    if byte_len == GROUP {
        leaves.push(expected_cv);
        return Ok(Cv::at(GROUP_LEVEL, chunk_start >> GROUP_LEVEL, expected_cv));
    }
    let (l_stored, r_stored) = split_pair(cursor);
    let half = byte_len / 2;
    let left = collect_whole(tree, chunk_start, half, l_stored, cursor, leaves)?;
    let right = collect_whole(tree, chunk_start + half / CHUNK_LEN as u64, half, r_stored, cursor, leaves)?;
    let parent = tree.parent_cv(&left, &right);
    if parent.as_bytes() != &expected_cv {
        return Err("outboard internal whole parent mismatch");
    }
    Ok(parent)
}

fn collect_last(
    tree: &TreeHasher,
    chunk_start: u64,
    byte_len: u64,
    expected_cv: [u8; 32],
    cursor: &mut &[u8],
    leaves: &mut Vec<[u8; 32]>,
) -> Result<LastCv, &'static str> {
    if byte_len <= GROUP {
        leaves.push(expected_cv);
        return Ok(LastCv::at(chunk_start, byte_len, expected_cv));
    }
    let (l_stored, r_stored) = split_pair(cursor);
    let split = left_len(byte_len);
    let left = collect_whole(tree, chunk_start, split, l_stored, cursor, leaves)?;
    let right = collect_last(tree, chunk_start + split / CHUNK_LEN as u64, byte_len - split, r_stored, cursor, leaves)?;
    let parent = tree.last_parent_cv(&left, &right);
    if parent.as_bytes() != &expected_cv {
        return Err("outboard internal right-spine parent mismatch");
    }
    Ok(parent)
}

#[test]
fn streaming_verify_needs_no_length_hint_and_catches_shorter_or_longer_streams() {
    blake3::initialize();
    let g = GROUP as usize;
    let all = painted(5 * g + 3000);
    for len in [0usize, 1, 1024, g, g + 1, 2 * g, 2 * g + 1, 3 * g - 1, 5 * g + 17] {
        let data = &all[..len];
        let expected = blake3::convenience::hash(data);
        // Build outboard via bao-tree (which matches b3sum --outboard byte-for-byte)
        let ob = if len <= g { Vec::new() } else { bao_tree::io::outboard::PreOrderMemOutboard::create(data, bao_tree::BlockSize::from_chunk_log(4)).data };

        // 1. Exact stream succeeds with ZERO length hint!
        let got = verify_stream(Mode::Plain, &ob, expected, std::io::Cursor::new(data)).unwrap();
        assert_eq!(got, data, "exact stream len={len}");

        // 2. 1 byte shorter (if len > 0) and 1 group shorter (if len > g) MUST fail!
        if len > 0 {
            assert!(verify_stream(Mode::Plain, &ob, expected, std::io::Cursor::new(&data[..len - 1])).is_err(), "1 byte shorter accepted at len={len}");
        }
        if len > g {
            assert!(verify_stream(Mode::Plain, &ob, expected, std::io::Cursor::new(&data[..len - g])).is_err(), "1 group shorter accepted at len={len}");
        }

        // 3. 1 byte longer and 1 group longer MUST fail!
        let plus_one: Vec<u8> = data.iter().copied().chain([0u8]).collect();
        assert!(verify_stream(Mode::Plain, &ob, expected, std::io::Cursor::new(&plus_one)).is_err(), "1 byte longer accepted at len={len}");
        let plus_group: Vec<u8> = data.iter().copied().chain(vec![0u8; g]).collect();
        assert!(verify_stream(Mode::Plain, &ob, expected, std::io::Cursor::new(&plus_group)).is_err(), "1 group longer accepted at len={len}");
    }
}
