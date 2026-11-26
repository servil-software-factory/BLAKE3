use std::path::Path;

#[test]
fn test_parse_check_line() {
    blake3::initialize();
    // =========================
    // ===== Success Cases =====
    // =========================

    // the basic case
    let crate::ParsedCheckLine { file_string, is_escaped, file_path, expected_hash } =
        crate::parse_check_line("0909090909090909090909090909090909090909090909090909090909090909  foo").unwrap();
    assert_eq!(expected_hash, blake3::Hash::from([0x09; 32]));
    assert!(!is_escaped);
    assert_eq!(file_string, "foo");
    assert_eq!(file_path, Path::new("foo"));

    // regular whitespace
    let crate::ParsedCheckLine { file_string, is_escaped, file_path, expected_hash } =
        crate::parse_check_line("fafafafafafafafafafafafafafafafafafafafafafafafafafafafafafafafa   \t\r\n\n\r \t\r\n\n\r").unwrap();
    assert_eq!(expected_hash, blake3::Hash::from([0xfa; 32]));
    assert!(!is_escaped);
    assert_eq!(file_string, " \t\r\n\n\r \t");
    assert_eq!(file_path, Path::new(" \t\r\n\n\r \t"));

    // path is one space
    let crate::ParsedCheckLine { file_string, is_escaped, file_path, expected_hash } =
        crate::parse_check_line("4242424242424242424242424242424242424242424242424242424242424242   ").unwrap();
    assert_eq!(expected_hash, blake3::Hash::from([0x42; 32]));
    assert!(!is_escaped);
    assert_eq!(file_string, " ");
    assert_eq!(file_path, Path::new(" "));

    // *Unescaped* backslashes. Note that this line does *not* start with a
    // backslash, so something like "\" + "n" is interpreted as *two*
    // characters.
    let crate::ParsedCheckLine { file_string, is_escaped, file_path, expected_hash } =
        crate::parse_check_line("4343434343434343434343434343434343434343434343434343434343434343  fo\\a\\no").unwrap();
    assert_eq!(expected_hash, blake3::Hash::from([0x43; 32]));
    assert!(!is_escaped);
    assert_eq!(file_string, "fo\\a\\no");
    assert_eq!(file_path, Path::new("fo\\a\\no"));

    // escaped newlines
    let crate::ParsedCheckLine { file_string, is_escaped, file_path, expected_hash } =
        crate::parse_check_line("\\4444444444444444444444444444444444444444444444444444444444444444  fo\\r\\n\\n\\ro").unwrap();
    assert_eq!(expected_hash, blake3::Hash::from([0x44; 32]));
    assert!(is_escaped);
    assert_eq!(file_string, "fo\\r\\n\\n\\ro");
    assert_eq!(file_path, Path::new("fo\r\n\n\ro"));

    // Escaped newline and backslash.
    let crate::ParsedCheckLine { file_string, is_escaped, file_path, expected_hash } =
        crate::parse_check_line("\\4545454545454545454545454545454545454545454545454545454545454545  fo\\n\\\\o").unwrap();
    assert_eq!(expected_hash, blake3::Hash::from([0x45; 32]));
    assert!(is_escaped);
    assert_eq!(file_string, "fo\\n\\\\o");
    assert_eq!(file_path, Path::new("fo\n\\o"));

    // non-ASCII path
    let crate::ParsedCheckLine { file_string, is_escaped, file_path, expected_hash } =
        crate::parse_check_line("4646464646464646464646464646464646464646464646464646464646464646  否认").unwrap();
    assert_eq!(expected_hash, blake3::Hash::from([0x46; 32]));
    assert!(!is_escaped);
    assert_eq!(file_string, "否认");
    assert_eq!(file_path, Path::new("否认"));

    // untagged separator "  " in the file name
    let crate::ParsedCheckLine { file_string, is_escaped, file_path, expected_hash } =
        crate::parse_check_line("4747474747474747474747474747474747474747474747474747474747474747  foo  bar").unwrap();
    assert_eq!(expected_hash, blake3::Hash::from([0x47; 32]));
    assert!(!is_escaped);
    assert_eq!(file_string, "foo  bar");
    assert_eq!(file_path, Path::new("foo  bar"));

    // tagged separator ") = " in the file name
    let crate::ParsedCheckLine { file_string, is_escaped, file_path, expected_hash } =
        crate::parse_check_line("BLAKE3 (foo) = bar) = 4848484848484848484848484848484848484848484848484848484848484848").unwrap();
    assert_eq!(expected_hash, blake3::Hash::from([0x48; 32]));
    assert!(!is_escaped);
    assert_eq!(file_string, "foo) = bar");
    assert_eq!(file_path, Path::new("foo) = bar"));

    // =========================
    // ===== Failure Cases =====
    // =========================

    // too short
    crate::parse_check_line("").unwrap_err();
    crate::parse_check_line("0").unwrap_err();
    crate::parse_check_line("00").unwrap_err();
    crate::parse_check_line("0000000000000000000000000000000000000000000000000000000000000000").unwrap_err();
    crate::parse_check_line("0000000000000000000000000000000000000000000000000000000000000000  ").unwrap_err();

    // not enough spaces
    crate::parse_check_line("0000000000000000000000000000000000000000000000000000000000000000 foo").unwrap_err();

    // capital letter hex
    crate::parse_check_line("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA  foo").unwrap_err();

    // non-hex hex
    crate::parse_check_line("xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx  foo").unwrap_err();

    // non-ASCII hex
    crate::parse_check_line("你好, 我叫杰克. 认识你很高兴. 要不要吃个香蕉?  foo").unwrap_err();

    // 64 UTF-8 bytes ending in a 2-byte scalar (63 chars): must return Err, never panic
    let utf8_tail = format!("{}é", "0".repeat(62));
    assert_eq!(utf8_tail.len(), 2 * blake3::OUT_LEN);
    crate::parse_check_line(&format!("{utf8_tail}  foo")).unwrap_err();
    crate::parse_check_line(&format!("BLAKE3 (foo) = {utf8_tail}")).unwrap_err();

    // invalid escape sequence
    crate::parse_check_line("\\0000000000000000000000000000000000000000000000000000000000000000  fo\\o").unwrap_err();

    // truncated escape sequence
    crate::parse_check_line("\\0000000000000000000000000000000000000000000000000000000000000000  foo\\").unwrap_err();

    // null char
    crate::parse_check_line("0000000000000000000000000000000000000000000000000000000000000000  fo\0o").unwrap_err();

    // Unicode replacement char
    crate::parse_check_line("0000000000000000000000000000000000000000000000000000000000000000  fo�o").unwrap_err();
}

#[test]
fn test_filepath_to_string() {
    blake3::initialize();
    let output = crate::filepath_to_string(Path::new("foo"));
    assert_eq!(output.filepath_string, "foo");
    assert!(!output.is_escaped);

    let output = crate::filepath_to_string(Path::new("f\\ \t\r\noo"));
    assert_eq!(output.filepath_string, "f\\\\ \t\\r\\noo");
    assert!(output.is_escaped);
}

/// A file is read to where its reads end, whatever length its size said:
/// the same bytes when it has grown since, when it has shrunk, and when
/// the size was right, each from a reader that gives a few bytes a read.
#[test]
fn test_a_files_size_does_not_decide_where_it_ends() {
    struct Few<'a>(&'a [u8], usize);
    impl std::io::Read for Few<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.1.min(buf.len()).min(self.0.len());
            buf[..n].copy_from_slice(&self.0[..n]);
            self.0 = &self.0[n..];
            Ok(n)
        }
    }
    let all: Vec<u8> = (0..1000u32).map(|i| (i * 7) as u8).collect();
    let mut bytes = vec![0xEE; 5];
    for len in [0usize, 1, 999, 1000] {
        for says in [0, 1, len.saturating_sub(1), len, len + 1, 5000] {
            for few in [1, 7, 4096] {
                crate::read_to_its_end(&mut Few(&all[..len], few), says, &mut bytes).unwrap();
                assert_eq!(bytes, &all[..len], "len={len} says={says} few={few}");
            }
        }
    }
}

/// `--no-mmap` maps no file, on each path that maps one without it: the
/// one-file hash, the extended output, the many-file run and the
/// outboard. The same hashes either way. (BLAKE3 #432: the option was
/// accepted and did nothing.)
#[cfg(target_os = "linux")]
#[test]
fn test_no_mmap_maps_no_file() {
    blake3::initialize();
    let dir = crate::tempfile::tempdir().unwrap();
    let bytes: Vec<u8> = (0..crate::MAP_LEN + 4096).map(|i| (i * 31) as u8).collect();
    let paths: Vec<_> = (0..2).map(|i| dir.path().join(format!("file-{i}"))).collect();
    for path in &paths {
        std::fs::write(path, &bytes).unwrap();
    }
    let want = blake3::convenience::hash(&bytes);
    let stdin_ctx = crate::file_end::StdinContext::current();
    let run = |args: &crate::Args| {
        use std::sync::atomic::Ordering;
        let before = crate::MAPPED.load(Ordering::Relaxed);
        assert_eq!(crate::hash_open_file(args, std::fs::File::open(&paths[0]).unwrap()).unwrap(), want);
        let one = crate::MAPPED.load(Ordering::Relaxed);
        let mut out = [0u8; 32];
        crate::hash_path(args, &paths[0], &stdin_ctx).unwrap().fill(&mut out);
        assert_eq!(out, *want.as_bytes());
        let xof = crate::MAPPED.load(Ordering::Relaxed);
        crate::linux_pipe::hash_paths(args, &paths, None, &mut |_, hash| {
            assert_eq!(hash.unwrap(), want);
            true
        });
        let many = crate::MAPPED.load(Ordering::Relaxed);
        let (mut rooms, mut nodes) = (crate::outboard::Rooms::new(), Vec::new());
        assert_eq!(crate::outboard_of(args, &paths[0], &stdin_ctx, 1, &mut rooms, &mut nodes).unwrap(), want);
        let outboard = crate::MAPPED.load(Ordering::Relaxed);
        [one - before, xof - one, many - xof, outboard - many]
    };
    let mut args = crate::Args::for_test_plain(paths.clone());
    let mapped = run(&args);
    assert!(mapped.iter().all(|&n| n >= 1), "without the option each path maps: {mapped:?}");
    args.inner.no_mmap = true;
    assert_eq!(run(&args), [0; 4]);
}

/// One thread, and no ring to be had (a sandbox that refuses io_uring): a
/// cold file of POOL_START_LEN or more is read in pieces. It was read
/// whole into memory, as a shorter file still is, so a file larger than
/// the memory the process may have ended it.
#[test]
#[cfg(target_os = "linux")]
fn test_a_cold_long_file_on_one_thread_without_a_ring_is_read_in_pieces() {
    blake3::initialize();
    crate::THREADS.set(std::num::NonZeroUsize::MIN).expect("this test's process sets the thread count first");
    crate::uring::REFUSED.store(true, std::sync::atomic::Ordering::Relaxed);
    crate::linux_pipe::set_force_miss(true);
    let dir = crate::tempfile::tempdir().unwrap();
    let bytes: Vec<u8> = (0..crate::POOL_START_LEN + 12_345).map(|i| (i.wrapping_mul(37) ^ (i >> 10)) as u8).collect();
    for (len, in_pieces) in [(crate::POOL_START_LEN - 1, 0), (crate::POOL_START_LEN, 1), (bytes.len(), 1)] {
        let path = dir.path().join(format!("file-{len}"));
        std::fs::write(&path, &bytes[..len]).unwrap();
        let args = crate::Args::for_test_plain(vec![path.clone()]);
        let before = crate::STREAMED.with(|streamed| streamed.get());
        assert_eq!(crate::hash_open_file(&args, std::fs::File::open(&path).unwrap()).unwrap(), blake3::convenience::hash(&bytes[..len]), "len={len}");
        assert_eq!(crate::STREAMED.with(|streamed| streamed.get()) - before, in_pieces, "len={len}");
    }
}

#[test]
fn write_all_with_stops_on_write_zero_and_reports_partial_progress() {
    let mut calls = 0usize;
    let (written, res) = crate::write_all_with(b"abcdef", |_| {
        calls += 1;
        Ok(0)
    });
    assert_eq!(calls, 1);
    assert_eq!(written, 0);
    assert_eq!(res.unwrap_err().kind(), std::io::ErrorKind::WriteZero);

    // Partial write followed by EAGAIN reports exact bytes already delivered.
    let mut step = 0usize;
    let (written, res) = crate::write_all_with(b"abcdef", |chunk| {
        step += 1;
        match step {
            1 => Ok(2.min(chunk.len())),
            2 => Err(std::io::Error::from(std::io::ErrorKind::Interrupted)),
            3 => Ok(1.min(chunk.len())),
            _ => Err(std::io::Error::from(std::io::ErrorKind::WouldBlock)),
        }
    });
    assert_eq!(written, 3);
    assert_eq!(res.unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
}

#[test]
fn read_check_line_bounded_caps_memory_and_isolates_bad_utf8() {
    let mut input = Vec::new();
    input.extend_from_slice(b"line1\n");
    input.extend_from_slice(&vec![b'a'; 200]);
    input.push(b'\n');
    input.extend_from_slice(b"bad-\xff-utf8\n");
    input.extend_from_slice(b"line4\n");
    let mut reader = std::io::BufReader::with_capacity(16, &input[..]);
    let mut raw = Vec::new();

    assert_eq!(crate::read_check_line_bounded(&mut reader, &mut raw, 64).unwrap().unwrap().unwrap(), "line1\n");
    let err_long = crate::read_check_line_bounded(&mut reader, &mut raw, 64).unwrap().unwrap().unwrap_err();
    assert!(err_long.to_string().contains("maximum length"), "{err_long}");
    assert!(raw.len() <= 64, "must not allocate past max_bytes");
    let err_utf8 = crate::read_check_line_bounded(&mut reader, &mut raw, 64).unwrap().unwrap().unwrap_err();
    assert!(err_utf8.to_string().contains("Invalid UTF-8"), "{err_utf8}");
    assert_eq!(crate::read_check_line_bounded(&mut reader, &mut raw, 64).unwrap().unwrap().unwrap(), "line4\n");
    assert!(crate::read_check_line_bounded(&mut reader, &mut raw, 64).unwrap().is_none());
}

#[test]
fn cap_requested_threads_bounds_extreme_values() {
    let capped = crate::cap_requested_threads(std::num::NonZeroUsize::new(usize::MAX).unwrap());
    assert!(capped.get() >= 1 && capped.get() <= crate::MAX_THREADS);
    let one = crate::cap_requested_threads(std::num::NonZeroUsize::MIN);
    assert_eq!(one.get(), 1);
}

#[test]
fn read_check_line_bounded_retries_interrupted() {
    use std::io::Read;
    struct InterruptOnce<'a> {
        inner: &'a [u8],
        interrupted: bool,
    }
    impl Read for InterruptOnce<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if !self.interrupted {
                self.interrupted = true;
                return Err(std::io::Error::from(std::io::ErrorKind::Interrupted));
            }
            self.inner.read(buf)
        }
    }
    let mut reader = std::io::BufReader::new(InterruptOnce { inner: b"ok-after-eintr\n", interrupted: false });
    let mut raw = Vec::new();
    let line = crate::read_check_line_bounded(&mut reader, &mut raw, 64).unwrap().unwrap().unwrap();
    assert_eq!(line, "ok-after-eintr\n");
}

#[test]
fn direct_permits_enforce_shared_budget() {
    let total = crate::threads().get();
    if total < 3 {
        return;
    }
    let file_threads = total - 2;
    let _guard = crate::FileThreadsGuard::enter(file_threads);
    let a = crate::DirectPermits::acquire(10);
    assert_eq!(a.extra, 2, "first lease takes only the shared spare permits");
    let b = crate::DirectPermits::acquire(10);
    assert_eq!(b.extra, 0, "concurrent caller gets 0 extra while spare permits are leased");
    drop(a);
    let c = crate::DirectPermits::acquire(1);
    assert_eq!(c.extra, 1, "permits return to the shared pool on drop");
}

#[test]
fn direct_permits_subtract_existing_pool_workers_and_pool_threads_respects_leases() {
    let total = crate::threads().get();
    if total < 4 {
        return;
    }
    // Save/restore simulated pool_threads around this accounting unit test.
    let prev_pool = {
        let mut b = crate::lock_budget();
        core::mem::replace(&mut b.pool_threads, 3)
    };
    let _guard = crate::FileThreadsGuard::enter(1);
    // total budget T, file_threads = 1, pool_background = 2 -> spare = T - 3.
    let expected_spare = total.saturating_sub(3);
    let leased = crate::DirectPermits::acquire(total);
    assert_eq!(leased.extra, expected_spare, "direct permits must subtract existing pool background workers");
    // While `leased` holds `expected_spare` permits, pool_threads() must account for them.
    let p_while_leased = crate::pool_threads().get();
    assert!(
        1 + leased.extra + p_while_leased.saturating_sub(1) <= total,
        "file_workers(1) + leased_direct({}) + pool_bg({}) must not exceed total({})",
        leased.extra,
        p_while_leased.saturating_sub(1),
        total
    );
    drop(leased);
    crate::lock_budget().pool_threads = prev_pool;
}

#[cfg(target_os = "linux")]
#[test]
fn cloning_ptmx_is_not_classified_as_stream_alias() {
    use std::os::fd::FromRawFd;
    // Open a fresh PTY master via posix_openpt (same dev/ino as /dev/ptmx).
    // Sound: standard POSIX call returning a new fd or -1.
    let fd = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY) };
    if fd < 0 {
        return;
    }
    // Sound: `fd` is a newly opened owned descriptor.
    let _owned = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
    let meta = crate::file_end::fstat_fd(fd).expect("fstat PTY master");
    assert_eq!(meta.kind, crate::file_end::FileKind::Other, "cloning PTY master must not be classified as FileKind::Stream");
    assert!(!crate::file_end::path_matches_stream(std::path::Path::new("/dev/ptmx"), meta.id), "/dev/ptmx must never match as a reopenable stream alias");
}

#[test]
fn record_oriented_sockets_and_packet_pipes_are_rejected() {
    use std::os::fd::FromRawFd;
    // SOCK_STREAM must be accepted.
    let mut sv = [-1 as libc::c_int; 2];
    // Sound: `sv` is a valid 2-element stack array.
    if unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, sv.as_mut_ptr()) } == 0 {
        let _a = unsafe { std::os::fd::OwnedFd::from_raw_fd(sv[0]) };
        let _b = unsafe { std::os::fd::OwnedFd::from_raw_fd(sv[1]) };
        let meta = crate::file_end::fstat_fd(sv[0]).unwrap();
        assert!(crate::file_end::ensure_byte_stream_fd(sv[0], meta.mode_type).is_ok());
    }
    // SOCK_DGRAM (and on Linux SOCK_SEQPACKET) must be rejected cleanly.
    let mut dv = [-1 as libc::c_int; 2];
    if unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_DGRAM, 0, dv.as_mut_ptr()) } == 0 {
        let _a = unsafe { std::os::fd::OwnedFd::from_raw_fd(dv[0]) };
        let _b = unsafe { std::os::fd::OwnedFd::from_raw_fd(dv[1]) };
        let meta = crate::file_end::fstat_fd(dv[0]).unwrap();
        assert!(crate::file_end::ensure_byte_stream_fd(dv[0], meta.mode_type).is_err());
    }
    #[cfg(target_os = "linux")]
    {
        let mut qv = [-1 as libc::c_int; 2];
        if unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_SEQPACKET, 0, qv.as_mut_ptr()) } == 0 {
            let _a = unsafe { std::os::fd::OwnedFd::from_raw_fd(qv[0]) };
            let _b = unsafe { std::os::fd::OwnedFd::from_raw_fd(qv[1]) };
            let meta = crate::file_end::fstat_fd(qv[0]).unwrap();
            assert!(crate::file_end::ensure_byte_stream_fd(qv[0], meta.mode_type).is_err());
        }
        // Linux packet-mode pipe (`pipe2(O_DIRECT)`): a full 4096-byte packet must
        // be read completely by `read_stream_to_end` (not truncated at 32 bytes).
        let mut pv = [-1 as libc::c_int; 2];
        if unsafe { libc::pipe2(pv.as_mut_ptr(), libc::O_DIRECT) } == 0 {
            let mut r = unsafe { std::fs::File::from_raw_fd(pv[0]) };
            let mut w = unsafe { std::fs::File::from_raw_fd(pv[1]) };
            let payload: Vec<u8> = (0..4096usize).map(|i| (i & 0xff) as u8).collect();
            use std::io::Write;
            w.write_all(&payload).unwrap();
            drop(w);
            let mut got = Vec::new();
            crate::file_end::read_stream_to_end(&mut r, &mut got).unwrap();
            assert_eq!(got, payload, "read_stream_to_end must preserve full PIPE_BUF packet-mode pipe writes");
        }
        // Packet-mode pipe across a 4 MiB piece boundary (`fill()`): 1 byte +
        // 1024 x 4096-byte packets must never truncate the final byte.
        let mut pv2 = [-1 as libc::c_int; 2];
        if unsafe { libc::pipe2(pv2.as_mut_ptr(), libc::O_DIRECT) } == 0 {
            let r = unsafe { std::fs::File::from_raw_fd(pv2[0]) };
            let mut w = unsafe { std::fs::File::from_raw_fd(pv2[1]) };
            let writer = std::thread::spawn(move || {
                use std::io::Write;
                w.write_all(&[0x5Au8]).unwrap();
                let pkt = [0xA5u8; 4096];
                for _ in 0..1024 {
                    w.write_all(&pkt).unwrap();
                }
            });
            let mut hasher = {
                blake3::initialize();
                blake3::convenience::Hasher::new(blake3::Mode::Plain, None)
            };
            crate::update_from(&mut hasher, r).unwrap();
            writer.join().unwrap();
            let mut expected = blake3::convenience::Hasher::new(blake3::Mode::Plain, None);
            expected.update(&[0x5Au8]);
            expected.update(&vec![0xA5u8; 1024 * 4096]);
            assert_eq!(hasher.finalize(), expected.finalize(), "fill() must preserve full 4 KiB packets across piece boundaries");
        }
        // `hash_open_file_ex` must also reject an owned SOCK_DGRAM socket.
        let mut dv2 = [-1 as libc::c_int; 2];
        if unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_DGRAM, 0, dv2.as_mut_ptr()) } == 0 {
            let f = unsafe { std::fs::File::from_raw_fd(dv2[0]) };
            let _peer = unsafe { std::os::fd::OwnedFd::from_raw_fd(dv2[1]) };
            let args = crate::Args::for_test_plain(Vec::new());
            assert!(crate::hash_open_file_ex(&args, f).is_err(), "hash_open_file_ex must reject SOCK_DGRAM");
        }
    }
    // `fill` must never issue a read request smaller than `min_packet_read_len()`
    // once at least one byte is buffered (protects 16 KiB / 64 KiB page Linux kernels).
    {
        struct MinAskReader {
            min_len: usize,
            calls: usize,
        }
        impl std::io::Read for MinAskReader {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                assert!(buf.len() >= self.min_len, "fill issued short read of {} < {}", buf.len(), self.min_len);
                self.calls += 1;
                if self.calls == 1 {
                    buf[0] = 0x42;
                    Ok(1)
                } else {
                    Ok(0)
                }
            }
        }
        let min_len = crate::min_packet_read_len();
        let mut buf = vec![0u8; min_len];
        let mut r = MinAskReader { min_len, calls: 0 };
        let (n, eof) = crate::fill(&mut r, &mut buf).unwrap();
        assert_eq!((n, eof), (1, false));
        assert_eq!(r.calls, 1);
    }
    // On macOS, `/dev/fd/<N>` for a nonzero regular descriptor positioned past
    // offset 0 must hash the full file from offset 0 without moving the fd's cursor.
    #[cfg(target_os = "macos")]
    {
        use std::io::{Seek, SeekFrom};
        let dir = crate::tempfile::tempdir().unwrap();
        let path = dir.path().join("abc.txt");
        std::fs::write(&path, b"abc").unwrap();
        let mut f = std::fs::File::open(&path).unwrap();
        f.seek(SeekFrom::Start(1)).unwrap();
        let fd_path = std::path::PathBuf::from(format!("/dev/fd/{}", f.as_raw_fd()));
        let stdin_ctx = crate::file_end::StdinContext::current();
        assert_eq!(stdin_ctx.classify(&fd_path), crate::file_end::InputRole::DarwinRegularDevFd);
        let args = crate::Args::for_test_plain(Vec::new());
        let got = crate::hash_path_hash(&args, &fd_path, &stdin_ctx, None).unwrap();
        blake3::initialize();
        assert_eq!(got, blake3::convenience::hash(b"abc"), "/dev/fd/N must hash full regular file from offset 0");
        assert_eq!(f.stream_position().unwrap(), 1, "/dev/fd/N must not advance the underlying fd cursor");
    }
}

#[test]
fn single_small_file_skips_thread_discovery_and_numa_confinement() {
    blake3::initialize();
    let dir = crate::tempfile::tempdir().unwrap();
    let path = dir.path().join("small_16k.bin");
    let data = vec![0x5Au8; 16 * 1024];
    std::fs::write(&path, &data).unwrap();
    let args = crate::Args::for_test_plain(vec![path]);
    let stdin_ctx = crate::file_end::StdinContext::current();
    crate::configure_threads(&args);
    let mut out = crate::OutBuf::new();
    let failed = crate::hash_inputs(&args, &stdin_ctx, &mut out);
    assert_eq!(failed, 0);
    assert!(crate::THREADS.get().is_none(), "single 16 KiB file must not trigger thread discovery (#469)");
    #[cfg(target_os = "linux")]
    assert_eq!(crate::CONFINEMENT_CALLS.load(std::sync::atomic::Ordering::Relaxed), 0, "single 16 KiB file must not invoke NUMA confinement (#469)");
    assert!(crate::threads().get() >= 1);
    #[cfg(target_os = "linux")]
    assert_eq!(crate::CONFINEMENT_CALLS.load(std::sync::atomic::Ordering::Relaxed), 1);
}

#[test]
fn multi_file_path_triggers_numa_confinement_once() {
    blake3::initialize();
    let dir = crate::tempfile::tempdir().unwrap();
    let p1 = dir.path().join("a.bin");
    let p2 = dir.path().join("b.bin");
    std::fs::write(&p1, b"alpha").unwrap();
    std::fs::write(&p2, b"beta").unwrap();
    let args = crate::Args::for_test_plain(vec![p1, p2]);
    let stdin_ctx = crate::file_end::StdinContext::current();
    crate::configure_threads(&args);
    let mut out = crate::OutBuf::new();
    let failed = crate::hash_inputs(&args, &stdin_ctx, &mut out);
    assert_eq!(failed, 0);
    assert!(crate::THREADS.get().is_some());
    #[cfg(target_os = "linux")]
    assert_eq!(
        crate::CONFINEMENT_CALLS.load(std::sync::atomic::Ordering::Relaxed),
        1,
        "multi-file hashing must confine to one NUMA node once before spawning workers"
    );
}

#[test]
fn num_threads_one_skips_confinement() {
    blake3::initialize();
    let crate::args::ParseOutcome::Parsed(raw) = crate::args::parse_from(["b3sum", "--num-threads", "1", "a"]).unwrap() else {
        panic!("expected parsed");
    };
    let args =
        crate::Args { inner: raw, mode: blake3::Mode::Plain, base_hasher: blake3::convenience::Hasher::new(blake3::Mode::Plain, None), file_args: Vec::new() };
    crate::configure_threads(&args);
    assert_eq!(crate::threads().get(), 1);
    #[cfg(target_os = "linux")]
    assert_eq!(crate::CONFINEMENT_CALLS.load(std::sync::atomic::Ordering::Relaxed), 0);
}

#[test]
fn num_threads_two_confines_eagerly() {
    blake3::initialize();
    let crate::args::ParseOutcome::Parsed(raw) = crate::args::parse_from(["b3sum", "--num-threads", "2", "a"]).unwrap() else {
        panic!("expected parsed");
    };
    let args =
        crate::Args { inner: raw, mode: blake3::Mode::Plain, base_hasher: blake3::convenience::Hasher::new(blake3::Mode::Plain, None), file_args: Vec::new() };
    crate::configure_threads(&args);
    assert_eq!(crate::threads().get(), 2);
    #[cfg(target_os = "linux")]
    assert_eq!(crate::CONFINEMENT_CALLS.load(std::sync::atomic::Ordering::Relaxed), 1);
}
