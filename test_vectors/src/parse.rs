//! Minimal, dependency-free JSON parser/formatter and hex codec for
//! `test_vectors/test_vectors.json`, shared by `test_vectors`, `capi`, and
//! `threads` tests.

#[derive(Debug)]
pub struct Cases {
    pub _comment: String,
    pub key: String,
    pub context_string: String,
    pub cases: Vec<Case>,
}

#[derive(Debug)]
pub struct Case {
    pub input_len: usize,
    pub hash: String,
    pub keyed_hash: String,
    pub derive_key: String,
}

#[allow(dead_code)]
pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[allow(dead_code)]
pub fn hex_decode(hex: &str) -> Vec<u8> {
    assert!(hex.len() % 2 == 0, "hex string must have even length");
    let nibble = |b: u8| match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => panic!("invalid hex byte: {b:#x}"),
    };
    let bytes = hex.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        out.push((nibble(pair[0]) << 4) | nibble(pair[1]));
    }
    out
}

#[allow(dead_code)]
fn push_json_string(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[allow(dead_code)]
pub fn format_cases(data: &Cases) -> String {
    use core::fmt::Write;
    let mut out = String::new();
    out.push_str("{\n  \"_comment\": ");
    push_json_string(&mut out, &data._comment);
    out.push_str(",\n  \"key\": ");
    push_json_string(&mut out, &data.key);
    out.push_str(",\n  \"context_string\": ");
    push_json_string(&mut out, &data.context_string);
    out.push_str(",\n  \"cases\": [\n");
    for (idx, case) in data.cases.iter().enumerate() {
        if idx > 0 {
            out.push_str(",\n");
        }
        out.push_str("    {\n      \"input_len\": ");
        let _ = write!(out, "{}", case.input_len);
        out.push_str(",\n      \"hash\": ");
        push_json_string(&mut out, &case.hash);
        out.push_str(",\n      \"keyed_hash\": ");
        push_json_string(&mut out, &case.keyed_hash);
        out.push_str(",\n      \"derive_key\": ");
        push_json_string(&mut out, &case.derive_key);
        out.push_str("\n    }");
    }
    out.push_str("\n  ]\n}\n");
    out
}

struct Scanner<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Scanner<'a> {
    fn new(s: &'a str) -> Self {
        Self { src: s.as_bytes(), pos: 0 }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.src.len() && self.src[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.src.get(self.pos).copied()
    }

    fn expect(&mut self, byte: u8) {
        self.skip_ws();
        assert_eq!(self.src.get(self.pos).copied(), Some(byte), "expected '{}' at byte {}", byte as char, self.pos);
        self.pos += 1;
    }

    fn string(&mut self) -> String {
        self.expect(b'"');
        let mut out = String::new();
        while self.pos < self.src.len() {
            let b = self.src[self.pos];
            self.pos += 1;
            match b {
                b'"' => return out,
                b'\\' => {
                    assert!(self.pos < self.src.len(), "truncated escape");
                    let esc = self.src[self.pos];
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        other => panic!("unsupported JSON escape: \\{}", other as char),
                    }
                }
                _ => {
                    // test_vectors.json is pure ASCII; reject invalid UTF-8 cleanly.
                    assert!(b.is_ascii(), "non-ASCII byte {b:#x} in test_vectors.json");
                    out.push(b as char);
                }
            }
        }
        panic!("unterminated JSON string");
    }

    fn usize(&mut self) -> usize {
        self.skip_ws();
        let start = self.pos;
        while self.pos < self.src.len() && self.src[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        assert!(self.pos > start, "expected integer at byte {}", self.pos);
        core::str::from_utf8(&self.src[start..self.pos]).unwrap().parse().unwrap()
    }

    fn parse_case(&mut self) -> Case {
        self.expect(b'{');
        let (mut input_len, mut hash, mut keyed_hash, mut derive_key) = (None, None, None, None);
        loop {
            let key = self.string();
            self.expect(b':');
            match key.as_str() {
                "input_len" => input_len = Some(self.usize()),
                "hash" => hash = Some(self.string()),
                "keyed_hash" => keyed_hash = Some(self.string()),
                "derive_key" => derive_key = Some(self.string()),
                other => panic!("unexpected case field {other:?}"),
            }
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                other => panic!("expected ',' or '}}' in case, got {other:?}"),
            }
        }
        Case {
            input_len: input_len.expect("missing input_len"),
            hash: hash.expect("missing hash"),
            keyed_hash: keyed_hash.expect("missing keyed_hash"),
            derive_key: derive_key.expect("missing derive_key"),
        }
    }
}

pub fn parse_json(text: &str) -> Cases {
    let mut s = Scanner::new(text);
    s.expect(b'{');
    let (mut comment, mut key, mut context, mut cases) = (None, None, None, None);
    loop {
        let field = s.string();
        s.expect(b':');
        match field.as_str() {
            "_comment" => comment = Some(s.string()),
            "key" => key = Some(s.string()),
            "context_string" => context = Some(s.string()),
            "cases" => {
                s.expect(b'[');
                let mut list = Vec::new();
                if s.peek() != Some(b']') {
                    loop {
                        list.push(s.parse_case());
                        match s.peek() {
                            Some(b',') => s.pos += 1,
                            Some(b']') => break,
                            other => panic!("expected ',' or ']' in cases, got {other:?}"),
                        }
                    }
                }
                s.expect(b']');
                cases = Some(list);
            }
            other => panic!("unexpected top-level field {other:?}"),
        }
        match s.peek() {
            Some(b',') => s.pos += 1,
            Some(b'}') => {
                s.pos += 1;
                break;
            }
            other => panic!("expected ',' or '}}' at top level, got {other:?}"),
        }
    }
    assert_eq!(s.peek(), None, "trailing bytes after JSON object");
    Cases {
        _comment: comment.expect("missing _comment"),
        key: key.expect("missing key"),
        context_string: context.expect("missing context_string"),
        cases: cases.expect("missing cases"),
    }
}
