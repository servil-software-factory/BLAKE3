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
