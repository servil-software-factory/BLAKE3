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

#[test]
fn test_outbuf_boundaries() {
    let mut ob = crate::OutBuf::new();
    ob.write_bytes(&vec![b' '; crate::OUT_BUF_CAP]).unwrap();
    assert_eq!(ob.buf.len(), crate::OUT_BUF_CAP, "exact OUT_BUF_CAP stays buffered");
    ob.write_bytes(b" ").unwrap();
    assert_eq!(ob.buf.len(), 1, "exceeding OUT_BUF_CAP flushes prior bytes first");
    ob.write_bytes(&vec![b' '; crate::OUT_BUF_CAP]).unwrap();
    assert!(ob.buf.is_empty(), "single write >= OUT_BUF_CAP writes directly after flush");
}
