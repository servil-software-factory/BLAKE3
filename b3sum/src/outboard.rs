//! `--outboard DIR`: beside each file's hash, its outboard in bao-tree's
//! form, for verified streaming of the file (iroh).
//!
//! The outboard holds the BLAKE3 tree above the file's chunk groups of
//! 16 KiB: each parent node as its two children's chaining values (64
//! bytes), the nodes in pre-order (a parent, its left subtree, its right
//! subtree), with no length before them. A file of one group or less has
//! no parent, and an empty outboard. It is what bao-tree's
//! `PreOrderMemOutboard::create(data, BlockSize::from_chunk_log(4))`
//! holds, byte for byte (verified in tests against an independent recursive
//! Bao reference and frozen `bao-tree 0.17.0` golden vectors), written to
//! `DIR/<hash>.obao4`; an empty outboard is no file (Zooko, November
//! 14, 2026: creating a thousand empty files took longer than hashing
//! the thousand short files they were for).
//!
//! It is built on the library's tree API alone (`throughput::TreeHasher`):
//! each group's chaining value, on as many threads as the file has work
//! for, then the parents a layer at a time, side by side in the lanes.
//!
//! A long file that the page cache does not hold is read past the cache
//! (O_DIRECT, Linux), several reads at once: a set of large files hashed
//! once is the common case, and is too large to cache (Zooko, November
//! 14, 2026). The cache does not hold such a file afterwards.

use blake3::throughput::{Cv, LastCv, TreeHasher, level_of};
use std::path::Path;

#[cfg(target_os = "linux")]
use crate::direct::PIECE;
#[cfg(target_os = "linux")]
pub(crate) use crate::direct::Piece;

/// A chunk group: sixteen chunks, bao-tree's block size 4.
pub(crate) const GROUP: usize = 16 * blake3::CHUNK_LEN;

/// Checked when this compiles: a group is a whole subtree's size.
const LEVEL: u32 = level_of(GROUP as u64);

/// A thread takes at least this many groups (4 MiB): a file with less
/// work than that a thread is hashed by fewer, since the threads are
/// started for the file and a start costs what hashing a megabyte does.
const GROUPS_A_THREAD: usize = 256;

/// A thread's rooms, kept from file to file: for a file's bytes, for its
/// tree's chaining values, and for a piece read directly.
pub(crate) struct Rooms {
    pub(crate) bytes: Vec<u8>,
    pub(crate) cvs: Vec<Cv>,
    /// Made by the first direct read: most runs have none.
    #[cfg(target_os = "linux")]
    pub(crate) piece: Option<Piece>,
}

impl Rooms {
    pub(crate) fn new() -> Rooms {
        Rooms {
            bytes: Vec::new(),
            cvs: Vec::new(),
            #[cfg(target_os = "linux")]
            piece: None,
        }
    }
}

/// The hash of `data` in `mode`, and its outboard appended to `out`. `cvs`
/// is the caller's room for the tree's chaining values, kept from file to
/// file.
pub(crate) fn outboard(mode: blake3::Mode, data: &[u8], threads: usize, cvs: &mut Vec<Cv>, out: &mut Vec<u8>) -> blake3::Hash {
    if data.len() <= GROUP {
        return blake3::convenience::hash_with(mode, data);
    }
    let done: Result<_, core::convert::Infallible> = build(mode, data.len(), cvs, out, |tree, cvs| Ok(group_cvs(tree, data, threads, cvs)));
    match done {
        Ok(hash) => hash,
    }
}

/// A layer of the tree: its whole subtrees' values, `whole` of them from
/// `cvs[at]`, and beside them the value of the message's last subtree at
/// that layer.
#[derive(Clone, Copy)]
struct Layer {
    at: usize,
    whole: usize,
    last: LastCv,
}

/// The hash and outboard of a message of `len` bytes, more than a group.
/// `fill` is given a slot a group: it writes the whole groups' chaining
/// values (every group's but the last, whose slot it leaves) and returns
/// the last group's.
fn build<E>(
    mode: blake3::Mode,
    len: usize,
    cvs: &mut Vec<Cv>,
    out: &mut Vec<u8>,
    fill: impl FnOnce(&TreeHasher, &mut [Cv]) -> Result<LastCv, E>,
) -> Result<blake3::Hash, E> {
    assert!(len > GROUP);
    let tree = TreeHasher::new(mode);
    let groups = len.div_ceil(GROUP);
    // Every layer's whole values, one layer after another: the groups',
    // then each layer of parents, down to the root's left child. Each
    // layer holds half the one below, so under two values a group in all.
    // Every slot starts unset, so one that no call fills is refused.
    cvs.clear();
    cvs.resize(2 * groups, Cv::unset());
    let last = fill(&tree, &mut cvs[..groups])?;
    let mut layers: Vec<Layer> = vec![Layer { at: 0, whole: groups - 1, last }];
    loop {
        let Layer { at, whole, last } = *layers.last().unwrap();
        if whole == 1 {
            break;
        }
        let (children, parents) = cvs[at..].split_at_mut(whole);
        let pairs = whole / 2;
        tree.parent_cvs(&children[..2 * pairs], &mut parents[..pairs]);
        // An odd whole one out is the last subtree's left sibling; with
        // none, the last goes up as it is.
        let last = if whole % 2 == 1 { tree.last_parent_cv(&children[whole - 1], &last) } else { last };
        layers.push(Layer { at: at + whole, whole: pairs, last });
    }
    let top = *layers.last().unwrap();
    out.reserve(64 * (groups - 1));
    node(cvs, &layers, layers.len(), 0, out);
    Ok(tree.root_hash(&cvs[top.at], &top.last, len as u64))
}

/// The hash and outboard of the open `file`, read past the page cache
/// (`direct::read`): None where that is not the way to read it or where a
/// read failed, and then `out` is as it was and the caller reads the file
/// as ever, and reports what fails then.
#[cfg(target_os = "linux")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn outboard_direct(
    mode: blake3::Mode,
    file: &std::fs::File,
    len: u64,
    threads: usize,
    mine: &mut Option<Piece>,
    cvs: &mut Vec<Cv>,
    out: &mut Vec<u8>,
) -> Option<blake3::Hash> {
    if !crate::direct::is_for(file, len) {
        return None;
    }
    direct(mode, file, usize::try_from(len).ok()?, threads, mine.get_or_insert_with(Piece::new), cvs, out)
}

/// [`outboard_direct`]'s reading and hashing, for a file of `len` bytes,
/// PIECE or more: each piece's groups' chaining values are a run of the
/// file's (a piece is 128 groups).
#[cfg(target_os = "linux")]
fn direct(
    mode: blake3::Mode,
    file: &std::fs::File,
    len: usize,
    threads: usize,
    mine: &mut Piece,
    cvs: &mut Vec<Cv>,
    out: &mut Vec<u8>,
) -> Option<blake3::Hash> {
    let before = out.len();
    let done: Result<_, ()> = build(mode, len, cvs, out, |tree, cvs| {
        // Many of these run at once, one a core: each on its own core's kernels.
        let tree = tree.clone().on_this_core();
        let groups = cvs.len();
        let last = std::sync::OnceLock::new();
        crate::direct::read(file, len, threads, mine, cvs, PIECE / GROUP, |i, bytes, cvs| {
            for (g, cv) in cvs.iter_mut().enumerate() {
                let group = i * (PIECE / GROUP) + g;
                let bytes = &bytes[g * GROUP..bytes.len().min((g + 1) * GROUP)];
                if group + 1 == groups {
                    last.set(tree.last_cv((group << LEVEL) as u64, bytes)).expect("one thread reads the last group");
                } else {
                    *cv = tree.subtree_cv(LEVEL, group as u64, bytes);
                }
            }
        })
        .ok_or(())?;
        Ok(last.into_inner().expect("the last group was read"))
    });
    if done.is_err() {
        out.truncate(before);
    }
    done.ok()
}

/// Each whole group's chaining value into its slot of `cvs` (a slot a
/// group; the last group's is left), the groups shared out among the
/// threads; and the last group's, returned.
fn group_cvs(tree: &TreeHasher, data: &[u8], threads: usize, cvs: &mut [Cv]) -> LastCv {
    let whole = cvs.len() - 1;
    let threads = threads.min(cvs.len() / GROUPS_A_THREAD).max(1);
    let a_thread = cvs.len().div_ceil(threads);
    // Many of these run at once, one a core: each on its own core's kernels.
    let tree = tree.clone().on_this_core();
    let hash = |first: usize, cvs: &mut [Cv]| {
        for (i, cv) in cvs.iter_mut().enumerate() {
            let group = first + i;
            *cv = tree.subtree_cv(LEVEL, group as u64, &data[group * GROUP..(group + 1) * GROUP]);
        }
    };
    if threads == 1 {
        hash(0, &mut cvs[..whole]);
    } else {
        std::thread::scope(|scope| {
            for (t, part) in cvs[..whole].chunks_mut(a_thread).enumerate() {
                let hash = &hash;
                scope.spawn(move || hash(t * a_thread, part));
            }
        });
    }
    tree.last_cv((whole << LEVEL) as u64, &data[whole * GROUP..])
}

/// The bytes of value `j` of a layer: a whole subtree's, or after them
/// the last one's.
fn value<'a>(cvs: &'a [Cv], layer: &'a Layer, j: usize) -> &'a [u8; 32] {
    if j < layer.whole { cvs[layer.at + j].as_bytes() } else { layer.last.as_bytes() }
}

/// The node `index` of the layer under `layer` (`layers[layer - 1]`), and
/// the nodes below it, appended to `out` in pre-order: a parent as its two
/// children's values. A last value its layer passed up as it was, and a
/// group's own, are no parent, and write nothing of their own. The root
/// is node 0 of the layer above the top.
fn node(cvs: &[Cv], layers: &[Layer], layer: usize, index: usize, out: &mut Vec<u8>) {
    if layer == 0 {
        return;
    }
    let below = &layers[layer - 1];
    if 2 * index + 1 > below.whole {
        return node(cvs, layers, layer - 1, 2 * index, out);
    }
    out.extend_from_slice(value(cvs, below, 2 * index));
    out.extend_from_slice(value(cvs, below, 2 * index + 1));
    node(cvs, layers, layer - 1, 2 * index, out);
    node(cvs, layers, layer - 1, 2 * index + 1, out);
}

/// Where `hash`'s outboard goes in `dir`.
pub(crate) fn path_in(dir: &Path, hash: &blake3::Hash) -> std::path::PathBuf {
    dir.join(format!("{}.obao4", hash.to_hex()))
}

/// Write `hash`'s outboard, `nodes`, to its place in `dir`: whole, into a
/// new file of another name there, which then takes the place's name.
/// Nothing at that name is opened or written, so a link there is replaced
/// and what it points to is left alone, and a reader of an outboard
/// already there sees the old one whole or the new one whole (BLAKE3
/// #425: writing in place followed a link out of the directory, and
/// emptied an outboard that others were reading).
pub(crate) fn write_in(dir: &Path, hash: &blake3::Hash, nodes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let place = path_in(dir, hash);
    // A name no other writer has: this process's, and a count. A file a
    // stopped run left under it is not opened: the next name is tried.
    let (new, mut file) = loop {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let new = dir.join(format!(".{}.obao4.{}.{n}.new", hash.to_hex(), std::process::id()));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&new) {
            Ok(file) => break (new, file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    };
    let written = file.write_all(nodes).and_then(|()| file.sync_all()).and_then(|()| {
        drop(file);
        std::fs::rename(&new, &place)
    });
    if written.is_err() {
        let _ = std::fs::remove_file(&new);
    }
    written
}

#[cfg(test)]
pub(crate) mod test {
    use super::*;

    #[cfg(target_os = "linux")]
    use crate::direct::{DIRECT_MIN, TAKE_FOR_COLD};

    fn painted(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i.wrapping_mul(31) ^ (i >> 9)) as u8).collect()
    }

    /// Frozen golden vectors captured once from `bao-tree 0.17.0`
    /// (`PreOrderMemOutboard::{create, create_keyed}` at `BlockSize::from_chunk_log(4)`,
    /// crate archive SHA-256 `50d0e17303b78c9bb240014f3f401ecae5c0460438ace49a171e5f97e3ae57ba`,
    /// BLAKE3 `edb995bea4e041fb6e4c231ca7f49b6444fa0e4eda768eb831eef6bce2942b57`).
    /// Tuple fields: `(len, plain_root, outboard_len, plain_outboard_b3, keyed_root, keyed_outboard_b3)`
    /// for input `painted(len)` and key `[0x42; 32]`.
    const BAO_TREE_0_17_0_GOLDENS: &[(usize, &str, usize, &str, &str, &str)] = &[
        (
            0,
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
            0,
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
            "4fa213fa1c1f5ae802e09c6e384b60390a0b5a1b0f228d77d89af00a84b29a21",
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
        ),
        (
            1,
            "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213",
            0,
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
            "b3e7beacf9c55340ef554e5a52f6927adc4f3aa02d4cca354f4905c7d70ce513",
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
        ),
        (
            1024,
            "909318844df4daae7873a36a9e02021a88fb68ec5b73e1f71f82e9a6c9483290",
            0,
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
            "373c2f9dcec3c4d6c0fac0f6d993a306c5010da72be3e693590b1549a2977b32",
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
        ),
        (
            GROUP - 1,
            "bec11a4d15af86f90295b0009c85b4adfee4bd31fb4328e16ced755f7f8716c3",
            0,
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
            "2a797178066ec530644b5389cb03b5a2bebea49050bbed708c5a21025df90e44",
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
        ),
        (
            GROUP,
            "abd734ae8b87148e78ebafb7aaf2e8a4064bea6bccfac2fc7011c762ff86cb89",
            0,
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
            "5abaf9b0e9a475facd04aec216317528a3d09ae1453f669d80338311370a6307",
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
        ),
        (
            GROUP + 1,
            "6b800f009b63a53f051c0f79f14477b156836253bcf61f04c8603ec986cb1684",
            64,
            "e507c159d6b85e0b1c5f7faa19bb0a4cd81314fbce275ce2a30945ec40bcf50a",
            "553f0213d3caa6a84624921d693236e1147bcd54434567cf8b94f1ec4010331e",
            "bd8df8dd944c3fcba7f39965c4c5fa868d98a4d234c5db02c4a7a77c7dd89334",
        ),
        (
            2 * GROUP - 1,
            "3c400bcaaf14ec7cbae7b172fb788cb37162c9a1a70d565964af53db25b8fa6d",
            64,
            "c7e82fcd77ecda4458794382214dadbc7f35e840e4751d9792a0b6bb8d9c3b98",
            "78b28d5898c438680a6e6b5f1e330e318d56fe3db29b3e1f0a7dda56560ae09b",
            "eebd71553ae270ea93c9bc8f8df0556cbc2677610c9a3ea01e5254bfbc24b028",
        ),
        (
            2 * GROUP,
            "dff5629e15590959d9c81259605359d4b015885e6e14c11d9c01cbfee464f794",
            64,
            "3be175d9d5a4708b8084bcc2abaa47b8d17c21d4b6d0de620b04fd3d22909484",
            "5425ebd842c4cb03e7738cbf7ff4f66eccf23da8815aaab4d9f0953388880aad",
            "07e2b6a2c3c9dd728dd18368c3ef7afb613ec844a5d13d1a035c98d94e4145c2",
        ),
        (
            2 * GROUP + 1,
            "e6845c0551e7fbb0a26abded295d861f7f03b41dbbc13ec5689ba08e6cd450bb",
            128,
            "b3fd4022b1347d6aeb4ea74132dbae8171c36a2fa38385b8a7972626ad06026a",
            "b5083ad8bcd72959c0b79535332f8ea93d82801fab9869794470451e9f6cdf54",
            "4b95a64e13da07ddd0e8aa21249b04ce4912c4802525f6f14f4249876f336e7a",
        ),
        (
            3 * GROUP,
            "e212fc44d954e56630ecdd9c318338340551f5ac341329a012c65a0a87fb41a5",
            128,
            "fd58257d242406d3767b0ecff79547d393ff626f2faeaa34df5ae38f8db7ef39",
            "02c2071b54d2ec22528c505359ffb32f0bc27a068c3bb29a66151128e15affc7",
            "aa67c448a2fba7afcb3b325d5fde30d080fd426d7b235fcae7454d9de5b4b863",
        ),
        (
            50_000,
            "86d6791ff7452375c296d2442cd02504bb0a08c26ba1bb4fc81e3937cbdb8388",
            192,
            "c97784103048bccee5c9e27ba4df6912a11d9012eb36e153c21302d931490b4a",
            "a2eded4124b9bff26b444a6a9e6b641922557a61cc3db1b1cb43367bbdb3c9e0",
            "63705fab3a703aec8ceac3382d94916438c8642af814c54a0f5546095270a093",
        ),
        (
            5 * GROUP + 1,
            "75495495fb9f232f9e565dffd901a427ed96b8f488e6b02ffb1e42de5c9c4d5b",
            320,
            "4f5180c0919f2b4387c129822002c35abec5248a3c8d8bc80e63327385fc37ff",
            "a57477a02f3cb2011ad311ce1cec0d36fc846e3525d4469198d3d3a18017d096",
            "50cee83e9698b7774a3803288b1397abc7b6ea1db3382869bb1a80f903c990d8",
        ),
        (
            63 * GROUP,
            "cf6fb3ad442c61522dbe92fcfdb3b8b351ec9b6817eddbc266790e959865af14",
            3968,
            "2ba4c9a18d2f54f7970fcf3b2af71931cb98488ff795d681a3c94e315a0c7a2a",
            "541ad800a031eee009124117b8bbebc343d0e1c84508360561dbc0aa899bfd00",
            "39aac587044da360ce31e1c85ce3a84bf3853ee79b5dbf18b4040cf834f7adb3",
        ),
        (
            64 * GROUP,
            "7ecc67aa7d25a26f749dd6159641e21bbd4743d46e87f735b5aa6f87f8776907",
            4032,
            "2b495888de989b3ff6801a8c6c4d707ae4207d349ff7cb70dad73a1c6edb0478",
            "51fe7aa53bd6bab3b0de8346f8eab092cdf8f9aeebe467c9f8e5ae28872419d4",
            "6454824a3c578604e07cce236ae8409360be99a5489dc9eea7dc86d14753eeea",
        ),
        (
            64 * GROUP + 1,
            "4607555ea737ee7e772c6f1cf69c9d1fea2ec19e1b3d22953063ea8d88fe1aa1",
            4096,
            "9f1780dc508030091e4670c1c0ce4b3a27b352b9326f9ca16fd76a618e878c51",
            "54241597d16b5af37b82f3890e14603d8763cbd319deba65e45594b5f2ddb114",
            "a773b3d959c990f3e3f4d94c29beddd8553b4e0c62dbfa17facd09451a81324c",
        ),
        (
            65 * GROUP + 100,
            "349e2b11b48cb68fd2ba725d1c5b02e46fd71519a513bacf87693a942ef97e57",
            4160,
            "ed5af11c67ba605e7ed0cbb1ce2878669196642d8d1a7b07a5b46e50adc69de4",
            "755006e00db70e944ae001d976581578642fbc3a6e0ac6c25704d839af727c7b",
            "ce5aa7df36651d75e3fc5d8271437d16ca63195126041b8d851c20dee0af5a23",
        ),
        (
            512 * GROUP - 1,
            "b8ff0c04e7be9adb12b6b82a6049eb4e91b68fab1830dac3a3bd45958d346634",
            32704,
            "ebd3ecdaf94ddb9b0907f1c602ae076fed211b68111781e9616d28599b10f511",
            "5c514338a146cc229845ecc6787813a24daa144a18ffd557b3c0a3dda73822bd",
            "adbc73f72553ae56407187f69effb7035af76cc782d7a4b7726042608f853047",
        ),
        (
            512 * GROUP,
            "ae97742715abdf88e8662c80f2c975c7f0c3936d7b2566c763dc475d20d520e3",
            32704,
            "dd34e3493b0b31fa222caf30e7bc6a71dbadac7dff43adaf24551996f14a8a41",
            "26b7ab31292b23d5a468449e346226b48829d524b8d2087f2996d73ab06b1831",
            "44f97286bcb45b8cadc7c2e7ffc7c46a6aace4f06beeede2e76efad2c3be8219",
        ),
        (
            512 * GROUP + 1,
            "a46f8e22132cf50b90b91b8e5b1729042e683dab942805c2e9746b65eb205d2b",
            32768,
            "35c2fa30dfa8af9f67610c9591b923ea1c5bd2d00c4a8275cff3871f7e9fe07d",
            "b15b6347a9a813782a1c7393383e7ea2be51069bd632af5a053f5204f58baa16",
            "b1e290467b34f5f6b2dcdfcddb300f70981ee0bdba386008a74032326953dd06",
        ),
        (
            773 * GROUP + 17,
            "2194a42497bf2cb90498ed41cb6964f23bd00dfb7a5fe4cd547a5e6b2e664ee8",
            49472,
            "219e14e89d6002dbdc2a777689cd712734d6e817cf994b63b35debd76c18ebf6",
            "a4a6d2f456a7ac0e0d35730702c7c1bcc4c3a271e714b8069d39be92859d47f2",
            "3f053b6cffb76a3a0c02ab09b8af352c0dcf52756bb863df1ad05824f96fe060",
        ),
    ];

    /// Non-uniform key `[0x00, 0x01, ..., 0x1f]` golden from `bao-tree 0.17.0`
    /// at `5 * GROUP + 1` (81,921 bytes, ragged multi-group tree) so both byte
    /// and word ordering in keyed mode are pinned independently.
    const BAO_TREE_0_17_0_NONUNIFORM_KEY: (usize, &str, usize, &str) = (
        5 * GROUP + 1,
        "4c239856b5b16804bcee075ea719f88ad19251514a06cde934b6797f164ab006",
        320,
        "1a7d432e9cee1c1571fd87e2db52f53bb9b17da6da42a205db3f27144c2b7333",
    );

    /// Literal raw pre-order outboard bytes from `bao-tree 0.17.0` for three
    /// representative tree shapes: 1 parent (`2*GROUP`), 2 parents (`3*GROUP`),
    /// and 5 parents with a ragged right tail (`5*GROUP + 1`).
    const BAO_TREE_0_17_0_RAW: &[(usize, &str)] = &[
        (
            2 * GROUP,
            "b5278de06dc7bb1b0dc9fcc7047569b245920403be54031b46df16d41910b3d1\
             ed54f56b0d34573e0a9caf6e2333b005fb93f690851a4f751e611b960ee0e1f2",
        ),
        (
            3 * GROUP,
            "c18b9f66db7cf45748196b5d145ec960883431973362ac77a50ede0e6b349546\
             a0f89dce75168e5f4c62dba6dec59ddbf7ee677cc2c7f2f659442596092719a0\
             b5278de06dc7bb1b0dc9fcc7047569b245920403be54031b46df16d41910b3d1\
             ed54f56b0d34573e0a9caf6e2333b005fb93f690851a4f751e611b960ee0e1f2",
        ),
        (
            5 * GROUP + 1,
            "7a2e38251536d241c86a28dea93fd988fdc663ab06655696774906aa504092d0\
             308b301469827b05deef732f80fa1d49e315708a070c57c65e783adf786e4084\
             c18b9f66db7cf45748196b5d145ec960883431973362ac77a50ede0e6b349546\
             d09b1a681663e96c0d068902623891d1e27e183856d4d5f6a21203a5e412ea24\
             b5278de06dc7bb1b0dc9fcc7047569b245920403be54031b46df16d41910b3d1\
             ed54f56b0d34573e0a9caf6e2333b005fb93f690851a4f751e611b960ee0e1f2\
             a0f89dce75168e5f4c62dba6dec59ddbf7ee677cc2c7f2f659442596092719a0\
             a6a5c7e7cc9662d3cacbb59e462002ccf557d455e4633400eb8a0ca689fd63f8\
             0435e2e2f96e0efca21fa7a5823f82f3021ac1dd857961b0d2de9c9c8bcb79f3\
             bae522831e12b036866d391f5c5c570413a052c9816ec1914b303b0759d082e9",
        ),
    ];

    /// The outboard and the hash match both the independent recursive Bao
    /// reference (`test_bao_ref`) and frozen golden vectors from `bao-tree
    /// 0.17.0`, byte for byte: at every length around a group, around powers
    /// of two of groups, and at ragged ones; on one thread and on several;
    /// and in keyed mode too.
    #[test]
    fn the_outboard_is_bao_trees_byte_for_byte() {
        blake3::initialize();
        // Long enough for both the largest frozen golden (773*GROUP + 17) and
        // three threads' worth of groups around GROUPS_A_THREAD.
        let max_len = (773 * GROUP + 17).max(3 * GROUPS_A_THREAD * GROUP + 5 * GROUP + 17);
        let all = painted(max_len);
        let key = [0x42u8; 32];
        let mut cvs = Vec::new();
        for &(len, want_root, want_ob_len, want_ob_b3, want_kroot, want_kob_b3) in BAO_TREE_0_17_0_GOLDENS {
            let data = &all[..len];
            let theirs_plain = crate::bao_ref::create(data);
            assert_eq!(theirs_plain.root_hex(), want_root, "bao_ref plain root vs bao-tree 0.17.0 golden, len={len}");
            assert_eq!(theirs_plain.data.len(), want_ob_len, "bao_ref plain len vs golden, len={len}");
            assert_eq!(crate::bao_ref::to_hex(&crate::bao_ref::hash_plain(&theirs_plain.data)), want_ob_b3, "bao_ref plain outboard vs golden, len={len}");

            let theirs_keyed = crate::bao_ref::create_keyed(data, &key);
            assert_eq!(theirs_keyed.root_hex(), want_kroot, "bao_ref keyed root vs bao-tree 0.17.0 golden, len={len}");
            assert_eq!(theirs_keyed.data.len(), want_ob_len, "bao_ref keyed len vs golden, len={len}");
            assert_eq!(crate::bao_ref::to_hex(&crate::bao_ref::hash_plain(&theirs_keyed.data)), want_kob_b3, "bao_ref keyed outboard vs golden, len={len}");

            for threads in [1, 3, 16] {
                let mut plain = Vec::new();
                let hash = outboard(blake3::Mode::Plain, data, threads, &mut cvs, &mut plain);
                assert_eq!(hash.as_bytes(), &theirs_plain.root, "the hash, len={len} threads={threads}");
                assert_eq!(hash, blake3::convenience::hash(data), "len={len}");
                assert_eq!(plain, theirs_plain.data, "the outboard, len={len} threads={threads}");
                assert_eq!(plain.len(), 64 * len.div_ceil(GROUP).saturating_sub(1), "a node a group but one, len={len}");

                let mut keyed = Vec::new();
                let hash = outboard(blake3::Mode::Keyed(&key), data, threads, &mut cvs, &mut keyed);
                assert_eq!(hash.as_bytes(), &theirs_keyed.root, "the keyed hash, len={len} threads={threads}");
                assert_eq!(keyed, theirs_keyed.data, "the keyed outboard, len={len} threads={threads}");
            }
        }
        // Check around the live multithreaded scheduling threshold independently of the frozen lengths.
        for len in [2 * GROUPS_A_THREAD * GROUP - 1, 2 * GROUPS_A_THREAD * GROUP, 2 * GROUPS_A_THREAD * GROUP + 1] {
            let data = &all[..len];
            let theirs = crate::bao_ref::create(data);
            let mut plain = Vec::new();
            let hash = outboard(blake3::Mode::Plain, data, 3, &mut cvs, &mut plain);
            assert_eq!(hash.as_bytes(), &theirs.root, "threshold hash, len={len}");
            assert_eq!(plain, theirs.data, "threshold outboard, len={len}");
        }
        // Check a non-uniform key against its frozen bao-tree 0.17.0 golden.
        let (nu_len, nu_root, nu_ob_len, nu_ob_b3) = BAO_TREE_0_17_0_NONUNIFORM_KEY;
        let nu_key: [u8; 32] = core::array::from_fn(|i| i as u8);
        let theirs_nu = crate::bao_ref::create_keyed(&all[..nu_len], &nu_key);
        assert_eq!(theirs_nu.root_hex(), nu_root, "non-uniform key bao_ref root vs bao-tree 0.17.0 golden");
        assert_eq!(theirs_nu.data.len(), nu_ob_len, "non-uniform key outboard len vs golden");
        assert_eq!(crate::bao_ref::to_hex(&crate::bao_ref::hash_plain(&theirs_nu.data)), nu_ob_b3, "non-uniform key outboard digest vs golden");
        let mut got_nu = Vec::new();
        let hash_nu = outboard(blake3::Mode::Keyed(&nu_key), &all[..nu_len], 3, &mut cvs, &mut got_nu);
        assert_eq!(hash_nu.as_bytes(), &theirs_nu.root, "non-uniform key production root");
        assert_eq!(got_nu, theirs_nu.data, "non-uniform key production outboard");
        for &(len, want_hex) in BAO_TREE_0_17_0_RAW {
            let mut plain = Vec::new();
            let _ = outboard(blake3::Mode::Plain, &all[..len], 1, &mut cvs, &mut plain);
            assert_eq!(crate::bao_ref::to_hex(&plain), want_hex, "raw bao-tree 0.17.0 outboard bytes, len={len}");
        }
    }

    /// A file read directly gives the hash and outboard its bytes in
    /// memory give: at lengths around a piece and ragged ones, on one
    /// thread and several, plain and keyed. A file whose pieces are not
    /// the ones its length when opened promised gives None and leaves
    /// `out` as it was. (Where the test's directory has no direct reads, as a
    /// tmpfs before Linux 6.6, there is nothing to compare.)
    #[test]
    #[cfg(target_os = "linux")]
    fn a_file_read_directly_gives_the_same_outboard() {
        use std::os::unix::fs::OpenOptionsExt;
        blake3::initialize();
        let dir = crate::tempfile::tempdir().unwrap();
        let all = painted(5 * PIECE + 3 * GROUP + 17);
        let key = [0x42u8; 32];
        let (mut cvs, mut piece) = (Vec::new(), Piece::new());
        for len in [PIECE, PIECE + 1, 2 * PIECE, 2 * PIECE + 1, 3 * PIECE - GROUP + 5, 4 * PIECE + GROUP, all.len()] {
            let path = dir.path().join(format!("file-{len}"));
            std::fs::write(&path, &all[..len]).unwrap();
            if std::fs::OpenOptions::new().read(true).custom_flags(libc::O_DIRECT).open(&path).is_err() {
                eprintln!("no direct reads in {}", dir.path().display());
                return;
            }
            let file = std::fs::File::open(&path).unwrap();
            for mode in [blake3::Mode::Plain, blake3::Mode::Keyed(&key)] {
                let mut want = Vec::new();
                let hash = outboard(mode, &all[..len], 1, &mut cvs, &mut want);
                for threads in [1, 3] {
                    let mut got = vec![0xee; 5];
                    assert_eq!(direct(mode, &file, len, threads, &mut piece, &mut cvs, &mut got), Some(hash), "len={len} threads={threads}");
                    assert_eq!(got[..5], [0xee; 5]);
                    assert_eq!(got[5..], want, "len={len} threads={threads}");
                    for wrong in [len + 1, len - 1, len + PIECE, len - GROUP, len.next_multiple_of(PIECE).saturating_sub(PIECE)] {
                        if wrong == len || wrong < PIECE {
                            continue;
                        }
                        let mut got = vec![0xee; 5];
                        assert_eq!(direct(mode, &file, wrong, threads, &mut piece, &mut cvs, &mut got), None, "len={len} read as {wrong}");
                        assert_eq!(got, [0xee; 5]);
                    }
                }
            }
        }
        // What is not read directly: a short file, and one the page cache holds.
        let path = dir.path().join(format!("file-{}", all.len()));
        assert!(all.len() as u64 >= DIRECT_MIN);
        let file = std::fs::File::open(&path).unwrap();
        let mut made = None;
        assert_eq!(
            outboard_direct(blake3::Mode::Plain, &file, all.len() as u64, 1, &mut made, &mut cvs, &mut Vec::new()),
            None,
            "just written: the cache holds it"
        );
        assert!(made.is_none(), "no piece is made for a file that is not read directly");
        // Taken for cold, it is read directly, and a file under DIRECT_MIN is not.
        TAKE_FOR_COLD.with(|c| c.set(true));
        let mut want = Vec::new();
        let hash = outboard(blake3::Mode::Plain, &all, 1, &mut cvs, &mut want);
        let mut got = Vec::new();
        assert_eq!(outboard_direct(blake3::Mode::Plain, &file, all.len() as u64, 2, &mut made, &mut cvs, &mut got), Some(hash));
        assert_eq!(got, want);
        assert!(made.is_some());
        let short = dir.path().join(format!("file-{}", PIECE + 1));
        let file = std::fs::File::open(&short).unwrap();
        assert!(((PIECE + 1) as u64) < DIRECT_MIN);
        assert_eq!(outboard_direct(blake3::Mode::Plain, &file, (PIECE + 1) as u64, 1, &mut made, &mut cvs, &mut got), None);
        assert_eq!(outboard_direct(blake3::Mode::Plain, &file, DIRECT_MIN - 1, 1, &mut made, &mut cvs, &mut got), None, "the length decides");
        TAKE_FOR_COLD.with(|c| c.set(false));
    }
}
