use std::path::Path;

#[test]
fn test_parse_check_line() {
    blake3::initialize();
    // =========================
    // ===== Success Cases =====
    // =========================

    // the basic case
    let crate::ParsedCheckLine { file_string, is_escaped, file_path, expected_hash } =
        crate::parse_check_line("0909090909090909090909090909090909090909090909090909090909090909  foo", false).unwrap();
    assert_eq!(expected_hash, blake3::Hash::from([0x09; 32]));
    assert!(!is_escaped);
    assert_eq!(file_string, "foo");
    assert_eq!(file_path, Path::new("foo"));

    // regular whitespace
    let crate::ParsedCheckLine { file_string, is_escaped, file_path, expected_hash } =
        crate::parse_check_line("fafafafafafafafafafafafafafafafafafafafafafafafafafafafafafafafa   \t\r\n\n\r \t\r\n\n\r", false).unwrap();
    assert_eq!(expected_hash, blake3::Hash::from([0xfa; 32]));
    assert!(!is_escaped);
    assert_eq!(file_string, " \t\r\n\n\r \t");
    assert_eq!(file_path, Path::new(" \t\r\n\n\r \t"));

    // path is one space
    let crate::ParsedCheckLine { file_string, is_escaped, file_path, expected_hash } =
        crate::parse_check_line("4242424242424242424242424242424242424242424242424242424242424242   ", false).unwrap();
    assert_eq!(expected_hash, blake3::Hash::from([0x42; 32]));
    assert!(!is_escaped);
    assert_eq!(file_string, " ");
    assert_eq!(file_path, Path::new(" "));

    // *Unescaped* backslashes. Note that this line does *not* start with a
    // backslash, so something like "\" + "n" is interpreted as *two*
    // characters.
    let crate::ParsedCheckLine { file_string, is_escaped, file_path, expected_hash } =
        crate::parse_check_line("4343434343434343434343434343434343434343434343434343434343434343  fo\\a\\no", false).unwrap();
    assert_eq!(expected_hash, blake3::Hash::from([0x43; 32]));
    assert!(!is_escaped);
    assert_eq!(file_string, "fo\\a\\no");
    assert_eq!(file_path, Path::new("fo\\a\\no"));

    // escaped newlines
    let crate::ParsedCheckLine { file_string, is_escaped, file_path, expected_hash } =
        crate::parse_check_line("\\4444444444444444444444444444444444444444444444444444444444444444  fo\\r\\n\\n\\ro", false).unwrap();
    assert_eq!(expected_hash, blake3::Hash::from([0x44; 32]));
    assert!(is_escaped);
    assert_eq!(file_string, "fo\\r\\n\\n\\ro");
    assert_eq!(file_path, Path::new("fo\r\n\n\ro"));

    // Escaped newline and backslash.
    let crate::ParsedCheckLine { file_string, is_escaped, file_path, expected_hash } =
        crate::parse_check_line("\\4545454545454545454545454545454545454545454545454545454545454545  fo\\n\\\\o", false).unwrap();
    assert_eq!(expected_hash, blake3::Hash::from([0x45; 32]));
    assert!(is_escaped);
    assert_eq!(file_string, "fo\\n\\\\o");
    assert_eq!(file_path, Path::new("fo\n\\o"));

    // non-ASCII path
    let crate::ParsedCheckLine { file_string, is_escaped, file_path, expected_hash } =
        crate::parse_check_line("4646464646464646464646464646464646464646464646464646464646464646  否认", false).unwrap();
    assert_eq!(expected_hash, blake3::Hash::from([0x46; 32]));
    assert!(!is_escaped);
    assert_eq!(file_string, "否认");
    assert_eq!(file_path, Path::new("否认"));

    // untagged separator "  " in the file name
    let crate::ParsedCheckLine { file_string, is_escaped, file_path, expected_hash } =
        crate::parse_check_line("4747474747474747474747474747474747474747474747474747474747474747  foo  bar", false).unwrap();
    assert_eq!(expected_hash, blake3::Hash::from([0x47; 32]));
    assert!(!is_escaped);
    assert_eq!(file_string, "foo  bar");
    assert_eq!(file_path, Path::new("foo  bar"));

    // tagged separator ") = " in the file name
    let crate::ParsedCheckLine { file_string, is_escaped, file_path, expected_hash } =
        crate::parse_check_line("BLAKE3 (foo) = bar) = 4848484848484848484848484848484848484848484848484848484848484848", false).unwrap();
    assert_eq!(expected_hash, blake3::Hash::from([0x48; 32]));
    assert!(!is_escaped);
    assert_eq!(file_string, "foo) = bar");
    assert_eq!(file_path, Path::new("foo) = bar"));

    // =========================
    // ===== Failure Cases =====
    // =========================

    // too short
    crate::parse_check_line("", false).unwrap_err();
    crate::parse_check_line("0", false).unwrap_err();
    crate::parse_check_line("00", false).unwrap_err();
    crate::parse_check_line("0000000000000000000000000000000000000000000000000000000000000000", false).unwrap_err();
    crate::parse_check_line("0000000000000000000000000000000000000000000000000000000000000000  ", false).unwrap_err();

    // not enough spaces
    crate::parse_check_line("0000000000000000000000000000000000000000000000000000000000000000 foo", false).unwrap_err();

    // capital letter hex
    crate::parse_check_line("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA  foo", false).unwrap_err();

    // non-hex hex
    crate::parse_check_line("xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx  foo", false).unwrap_err();

    // non-ASCII hex
    crate::parse_check_line("你好, 我叫杰克. 认识你很高兴. 要不要吃个香蕉?  foo", false).unwrap_err();

    // invalid escape sequence
    crate::parse_check_line("\\0000000000000000000000000000000000000000000000000000000000000000  fo\\o", false).unwrap_err();

    // truncated escape sequence
    crate::parse_check_line("\\0000000000000000000000000000000000000000000000000000000000000000  foo\\", false).unwrap_err();

    // null char
    crate::parse_check_line("0000000000000000000000000000000000000000000000000000000000000000  fo\0o", false).unwrap_err();

    // Unicode replacement char
    crate::parse_check_line("0000000000000000000000000000000000000000000000000000000000000000  fo�o", false).unwrap_err();
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
    let dir = tempfile::tempdir().unwrap();
    let bytes: Vec<u8> = (0..crate::MAP_LEN + 4096).map(|i| (i * 31) as u8).collect();
    let paths: Vec<_> = (0..2).map(|i| dir.path().join(format!("file-{i}"))).collect();
    for path in &paths {
        std::fs::write(path, &bytes).unwrap();
    }
    let want = blake3::convenience::hash(&bytes);
    let run = |args: &crate::Args| {
        let before = crate::MAPPED.with(|m| m.get());
        assert_eq!(crate::hash_open_file(args, std::fs::File::open(&paths[0]).unwrap()).unwrap(), want);
        let one = crate::MAPPED.with(|m| m.get());
        let mut out = [0u8; 32];
        crate::hash_path(args, &paths[0]).unwrap().fill(&mut out);
        assert_eq!(out, *want.as_bytes());
        let xof = crate::MAPPED.with(|m| m.get());
        crate::linux_pipe::hash_paths(args, &paths, &mut |_, hash| {
            assert_eq!(hash.unwrap(), want);
            true
        });
        let many = crate::MAPPED.with(|m| m.get());
        let (mut rooms, mut nodes) = (crate::outboard::Rooms::new(), Vec::new());
        assert_eq!(crate::outboard_of(args, &paths[0], 1, &mut rooms, &mut nodes).unwrap(), want);
        let outboard = crate::MAPPED.with(|m| m.get());
        [one - before, xof - one, many - xof, outboard - many]
    };
    let mut args = crate::Args::for_test_plain(paths.clone());
    let mapped = run(&args);
    assert!(mapped.iter().all(|&n| n >= 1), "without the option each path maps: {mapped:?}");
    args.inner.no_mmap = true;
    assert_eq!(run(&args), [0; 4]);
}
