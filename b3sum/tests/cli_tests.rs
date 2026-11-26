use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, prelude::*};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

#[path = "../src/test_bao_ref.rs"]
mod bao_ref;
#[path = "../src/test_temp.rs"]
mod tempfile;

fn hex_encode(bytes: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

#[derive(Clone)]
struct Cmd {
    program: OsString,
    args: Vec<OsString>,
    dir: Option<PathBuf>,
    stdin_bytes: Option<Vec<u8>>,
    stdout_capture: bool,
    stderr_capture: bool,
    stderr_null: bool,
    unchecked: bool,
}

fn cmd<P, I, A>(program: P, args: I) -> Cmd
where
    P: AsRef<OsStr>,
    I: IntoIterator<Item = A>,
    A: AsRef<OsStr>,
{
    Cmd {
        program: program.as_ref().to_os_string(),
        args: args.into_iter().map(|a| a.as_ref().to_os_string()).collect(),
        dir: None,
        stdin_bytes: None,
        stdout_capture: false,
        stderr_capture: false,
        stderr_null: false,
        unchecked: false,
    }
}

macro_rules! cmd {
    ($prog:expr $(, $arg:expr)* $(,)?) => {{
        #[allow(unused_mut)]
        let mut c = cmd($prog, ::core::iter::empty::<&OsStr>());
        $(c.args.push(AsRef::<OsStr>::as_ref(&$arg).to_os_string());)*
        c
    }};
}

impl Cmd {
    fn dir(mut self, path: impl AsRef<Path>) -> Self {
        self.dir = Some(path.as_ref().to_path_buf());
        self
    }

    fn stdin_bytes(mut self, bytes: impl AsRef<[u8]>) -> Self {
        self.stdin_bytes = Some(bytes.as_ref().to_vec());
        self
    }

    fn stdout_capture(mut self) -> Self {
        self.stdout_capture = true;
        self
    }

    fn stderr_capture(mut self) -> Self {
        self.stderr_capture = true;
        self
    }

    fn stderr_null(mut self) -> Self {
        self.stderr_null = true;
        self
    }

    fn unchecked(mut self) -> Self {
        self.unchecked = true;
        self
    }

    fn run(self) -> io::Result<Output> {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        if let Some(dir) = &self.dir {
            command.current_dir(dir);
        }
        command.stdin(if self.stdin_bytes.is_some() { Stdio::piped() } else { Stdio::inherit() });
        command.stdout(if self.stdout_capture { Stdio::piped() } else { Stdio::inherit() });
        command.stderr(if self.stderr_capture {
            Stdio::piped()
        } else if self.stderr_null {
            Stdio::null()
        } else {
            Stdio::inherit()
        });
        let mut child = command.spawn()?;
        let writer = self.stdin_bytes.map(|bytes| {
            let mut stdin = child.stdin.take().expect("piped stdin");
            std::thread::spawn(move || -> io::Result<()> {
                match stdin.write_all(&bytes) {
                    Ok(()) => Ok(()),
                    Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
                    Err(e) => Err(e),
                }
            })
        });
        let output = child.wait_with_output()?;
        if let Some(handle) = writer {
            match handle.join() {
                Ok(res) => res?,
                Err(payload) => std::panic::resume_unwind(payload),
            }
        }
        if !self.unchecked && !output.status.success() {
            return Err(io::Error::other(format!("command {:?} exited with {}", self.program, output.status)));
        }
        Ok(output)
    }

    fn read(self) -> io::Result<String> {
        let output = self.stdout_capture().run()?;
        let mut s = String::from_utf8(output.stdout).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        while s.ends_with('\n') || s.ends_with('\r') {
            s.pop();
        }
        Ok(s)
    }
}

pub fn b3sum_exe() -> PathBuf {
    env!("CARGO_BIN_EXE_b3sum").into()
}

/// A full output device is a failed run, even when the final flush is the
/// only write. A successful exit must mean that the output was delivered.
#[test]
#[cfg(target_os = "linux")]
fn test_output_error_is_failure() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("empty"), b"").unwrap();
    // Published BLAKE3 test vector for the empty input, not this binary's output.
    fs::write(dir.path().join("sums"), b"af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262  empty\n").unwrap();
    for args in [&[][..], &["--raw"][..], &["--length", "64"][..], &["--check", "sums"][..], &["--help"][..], &["--version"][..]] {
        let output = std::process::Command::new(b3sum_exe())
            .args(args)
            .current_dir(dir.path())
            .stdin(std::process::Stdio::null())
            .stdout(fs::OpenOptions::new().write(true).open("/dev/full").unwrap())
            .stderr(std::process::Stdio::piped())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{args:?}: expected clean exit 1 (not 0 or signal) after /dev/full");
        assert!(!output.stderr.is_empty(), "{args:?}: the error is reported");
    }
    // Usage error with stderr redirected to /dev/full must still exit 2 cleanly (no abort/panic).
    let output = std::process::Command::new(b3sum_exe())
        .arg("--not-a-flag")
        .stdin(std::process::Stdio::null())
        .stderr(fs::OpenOptions::new().write(true).open("/dev/full").unwrap())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "usage error with /dev/full stderr must exit 2 cleanly");
}

/// Missing option values before another flag or `--`, and empty positional paths,
/// must fail with exit 2 before any hashing or outboard side effects.
#[test]
fn test_missing_option_value_and_empty_positional_fail_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("sums");
    fs::write(&f, b"not a valid checkfile\n").unwrap();
    let out_dir = dir.path().join("--check");
    fs::create_dir(&out_dir).unwrap();

    for args in [
        &["--derive-key", "--check", "sums"][..],
        &["--derive-key", "--", "sums"][..],
        &["--outboard", "--check", "sums"][..],
        &["--outboard", "--", "sums"][..],
        &["sums", ""][..],
        &["--", "sums", ""][..],
    ] {
        let output = std::process::Command::new(b3sum_exe()).args(args).current_dir(dir.path()).stdin(std::process::Stdio::null()).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}: expected exit 2, got {:?}", output.status);
        assert!(output.stdout.is_empty(), "{args:?}: expected no stdout, got {:?}", String::from_utf8_lossy(&output.stdout));
        assert_eq!(fs::read_dir(&out_dir).unwrap().count(), 0, "{args:?}: expected no outboard side effects");
    }

    // Inline `--derive-key=` (empty context) and `--derive-key=--weird` remain valid.
    for args in [&["--derive-key=", "--no-names", "sums"][..], &["--derive-key=--weird", "--no-names", "sums"][..]] {
        let output = std::process::Command::new(b3sum_exe()).args(args).current_dir(dir.path()).stdin(std::process::Stdio::null()).output().unwrap();
        assert!(output.status.success(), "{args:?}: expected success, stderr={:?}", String::from_utf8_lossy(&output.stderr));
    }
}

/// A truncated-to-empty checkfile has checked nothing. Each checkfile
/// must hold at least one entry, including when another checkfile passed.
#[test]
fn test_empty_checkfile_is_failure() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("empty"), b"").unwrap();
    fs::write(dir.path().join("sums"), b"af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262  empty\n").unwrap();
    fs::write(dir.path().join("no-sums"), b"").unwrap();
    for args in [&["--check"][..], &["--check", "--quiet", "no-sums"][..], &["--check", "sums", "no-sums"][..]] {
        let output = std::process::Command::new(b3sum_exe()).args(args).current_dir(dir.path()).stdin(std::process::Stdio::null()).output().unwrap();
        assert!(!output.status.success(), "{args:?}: exit 0 after checking no entries");
        assert!(String::from_utf8_lossy(&output.stderr).contains("no checksum entries"), "{args:?}: {:?}", output.stderr);
    }
}

/// A reader that goes away ends b3sum at once and quietly (SIGPIPE), as
/// it ends GNU's tools: more results than a pipe holds, the reader gone
/// after one line.
#[test]
#[cfg(unix)]
fn test_reader_gone() {
    use std::io::BufRead;
    use std::os::unix::process::ExitStatusExt;
    let dir = tempfile::tempdir().unwrap();
    let files: Vec<PathBuf> = (0..2000)
        .map(|i| {
            let path = dir.path().join(format!("{i:04}"));
            fs::write(&path, i.to_string()).unwrap();
            path
        })
        .collect();
    let mut child =
        std::process::Command::new(b3sum_exe()).args(&files).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
    let mut line = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let output = child.wait_with_output().unwrap();
    const SIGPIPE: i32 = 13; // on Linux and macOS
    assert_eq!(output.status.signal(), Some(SIGPIPE), "{:?}", output.status);
    assert!(output.stderr.is_empty(), "{}", String::from_utf8_lossy(&output.stderr));
}

/// A file whose reported size is not its content (Linux's /proc and /sys
/// report 0 or 4096 bytes) hashes as its content, as other hash tools
/// hash it: b3sum uses a file's size to choose how to read it, never to
/// judge it.
#[test]
#[cfg(target_os = "linux")]
fn test_pseudo_file() {
    blake3::initialize();
    // /proc/version: reported as 0 bytes, its content the same on every read.
    let Ok(content) = fs::read("/proc/version") else { return };
    let output = cmd!(b3sum_exe(), "--no-names", "/proc/version").stdout_capture().stderr_capture().unchecked().run().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(blake3::convenience::hash(&content).to_hex().to_string(), String::from_utf8_lossy(&output.stdout).trim_end());
}

#[test]
fn test_hash_one() {
    blake3::initialize();
    let expected = format!("{}  -", blake3::convenience::hash(b"foo").to_hex());
    let output = cmd!(b3sum_exe()).stdin_bytes("foo").read().unwrap();
    assert_eq!(&*expected, output);
}

#[test]
fn test_hash_one_tag() {
    blake3::initialize();
    let expected = format!("BLAKE3 (-) = {}", blake3::convenience::hash(b"foo").to_hex());
    let output = cmd!(b3sum_exe(), "--tag").stdin_bytes("foo").read().unwrap();
    assert_eq!(&*expected, output);
}

#[test]
fn test_hash_one_raw() {
    blake3::initialize();
    let expected = blake3::convenience::hash(b"foo").as_bytes().to_owned();
    let output = cmd!(b3sum_exe(), "--raw").stdin_bytes("foo").stdout_capture().run().unwrap().stdout;
    assert_eq!(expected, output.as_slice());
}

#[test]
fn test_hash_many() {
    blake3::initialize();
    let dir = tempfile::tempdir().unwrap();
    let file1 = dir.path().join("file1");
    fs::write(&file1, b"foo").unwrap();
    let file2 = dir.path().join("file2");
    fs::write(&file2, b"bar").unwrap();

    let output = cmd!(b3sum_exe(), &file1, &file2).read().unwrap();
    let foo_hash = blake3::convenience::hash(b"foo");
    let bar_hash = blake3::convenience::hash(b"bar");
    let expected = format!("{}  {}\n{}  {}", foo_hash.to_hex(), file1.to_string_lossy(), bar_hash.to_hex(), file2.to_string_lossy());
    assert_eq!(expected, output);

    let output_no_names = cmd!(b3sum_exe(), "--no-names", &file1, &file2).read().unwrap();
    let expected_no_names = format!("{}\n{}", foo_hash.to_hex(), bar_hash.to_hex());
    assert_eq!(expected_no_names, output_no_names);
}

/// Many files of many sizes, more than b3sum keeps in flight (on Linux, up
/// to 64 read ahead), each digest the library's and in
/// the order given: empty files, small ones, and ones beside the 512 KiB
/// boundary where mapping takes over.
#[test]
fn test_hash_many_sizes() {
    blake3::initialize();
    let dir = tempfile::tempdir().unwrap();
    let sizes = [0usize, 1, 64, 1023, 1024, 1025, 16 << 10, (512 << 10) - 1, 512 << 10, (512 << 10) + 1];
    let mut expected = String::new();
    let mut files = Vec::new();
    for i in 0..300 {
        let len = sizes[i % sizes.len()];
        let bytes: Vec<u8> = (0..len).map(|j| (j as u32 * 31 + i as u32) as u8).collect();
        let path = dir.path().join(format!("{i:03}"));
        fs::write(&path, &bytes).unwrap();
        expected += &format!("{}\n", blake3::convenience::hash(&bytes).to_hex());
        files.push(path);
    }
    // One thread holds the deepest batch (64 files), two half of it.
    for threads in [&["--num-threads=1"][..], &["--num-threads=2"], &[]] {
        let args = ["--no-names"].iter().chain(threads).map(Into::into).chain(files.iter().map(|p| p.clone().into_os_string()));
        let output = cmd(b3sum_exe(), args.collect::<Vec<std::ffi::OsString>>()).read().unwrap();
        assert_eq!(expected.trim_end(), output, "{threads:?}");
    }
}

#[test]
fn test_hash_many_tag() {
    blake3::initialize();
    let dir = tempfile::tempdir().unwrap();
    let file1 = dir.path().join("file1");
    fs::write(&file1, b"foo").unwrap();
    let file2 = dir.path().join("file2");
    fs::write(&file2, b"bar").unwrap();

    let output = cmd!(b3sum_exe(), "--tag", &file1, &file2).read().unwrap();
    let foo_hash = blake3::convenience::hash(b"foo");
    let bar_hash = blake3::convenience::hash(b"bar");
    let expected = format!("BLAKE3 ({}) = {}\nBLAKE3 ({}) = {}", file1.to_string_lossy(), foo_hash.to_hex(), file2.to_string_lossy(), bar_hash.to_hex());
    assert_eq!(expected, output);
}

#[test]
fn test_missing_files() {
    blake3::initialize();
    let dir = tempfile::tempdir().unwrap();
    let file1 = dir.path().join("file1");
    fs::write(&file1, b"foo").unwrap();
    let file2 = dir.path().join("file2");
    fs::write(&file2, b"bar").unwrap();

    let output = cmd!(b3sum_exe(), "file1", "missing_file", "file2").dir(dir.path()).stdout_capture().stderr_capture().unchecked().run().unwrap();
    assert!(!output.status.success());

    let foo_hash = blake3::convenience::hash(b"foo");
    let bar_hash = blake3::convenience::hash(b"bar");
    let expected_stdout = format!("{}  file1\n{}  file2\n", foo_hash.to_hex(), bar_hash.to_hex());
    assert_eq!(expected_stdout.as_bytes(), &output.stdout[..]);

    let bing_error = fs::File::open(dir.path().join("missing_file")).unwrap_err();
    let expected_stderr = format!("b3sum: missing_file: {}\n", bing_error.to_string());
    assert_eq!(expected_stderr.as_bytes(), &output.stderr[..]);
}

#[test]
fn test_hash_length_and_seek() {
    blake3::initialize();
    let mut expected = [0; 100];
    let mut hasher = blake3::convenience::Hasher::new(blake3::Mode::Plain, None);
    hasher.update(b"foo");
    hasher.finalize_xof().fill(&mut expected);
    let output = cmd!(b3sum_exe(), "--raw", "--length=100").stdin_bytes("foo").stdout_capture().run().unwrap().stdout;
    assert_eq!(expected[..], output);

    let short_output = cmd!(b3sum_exe(), "--raw", "--length=99").stdin_bytes("foo").stdout_capture().run().unwrap().stdout;
    assert_eq!(expected[..99], short_output);

    let seek1_output = cmd!(b3sum_exe(), "--raw", "--length=99", "--seek=1").stdin_bytes("foo").stdout_capture().run().unwrap().stdout;
    assert_eq!(expected[1..], seek1_output);

    let seek99_output = cmd!(b3sum_exe(), "--raw", "--length=1", "--seek=99").stdin_bytes("foo").stdout_capture().run().unwrap().stdout;
    assert_eq!(expected[99..], seek99_output);
}

#[test]
fn test_keyed() {
    blake3::initialize();
    let key = [42; blake3::KEY_LEN];
    let f = tempfile::NamedTempFile::new().unwrap();
    f.as_file().write_all(b"foo").unwrap();
    f.as_file().flush().unwrap();
    let expected = blake3::convenience::hash_with(blake3::Mode::Keyed(&key), b"foo").to_hex();
    let output = cmd!(b3sum_exe(), "--keyed", "--no-names", f.path()).stdin_bytes(&key[..]).read().unwrap();
    assert_eq!(&*expected, &*output);

    // Make sure that keys of the wrong length lead to errors.
    for bad_length in [0, 1, blake3::KEY_LEN - 1, blake3::KEY_LEN + 1] {
        dbg!(bad_length);
        let output = cmd!(b3sum_exe(), "--keyed", f.path()).stdin_bytes(vec![0; bad_length]).stdout_capture().stderr_capture().unchecked().run().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        // Make sure the error message is relevant.
        let stderr = std::str::from_utf8(&output.stderr).unwrap();
        assert!(stderr.contains("key bytes"));
    }
}

#[test]
fn test_derive_key() {
    blake3::initialize();
    let context = "BLAKE3 2019-12-28 10:28:41 example context";
    let f = tempfile::NamedTempFile::new().unwrap();
    f.as_file().write_all(b"key material").unwrap();
    f.as_file().flush().unwrap();
    let expected = hex_encode(*blake3::convenience::hash_with(blake3::Mode::DeriveKey(context), b"key material").as_bytes());
    let output = cmd!(b3sum_exe(), "--derive-key", context, "--no-names", f.path()).read().unwrap();
    assert_eq!(&*expected, &*output);
}

/// --check reads hashes of whole outputs: with --seek it is refused, as
/// with --length (it was accepted, and the offset ignored).
#[test]
fn test_check_with_seek_is_an_error() {
    let f = tempfile::NamedTempFile::new().unwrap();
    for option in [["--seek", "1"], ["--length", "16"]] {
        let result = cmd!(b3sum_exe(), "--check", option[0], option[1], f.path()).stdout_capture().stderr_capture().unchecked().run().unwrap();
        assert!(!result.status.success(), "{option:?}");
        assert!(String::from_utf8_lossy(&result.stderr).contains("cannot be used with"), "{option:?}");
    }
}

#[test]
fn test_no_mmap() {
    blake3::initialize();
    let f = tempfile::NamedTempFile::new().unwrap();
    f.as_file().write_all(b"foo").unwrap();
    f.as_file().flush().unwrap();

    let expected = blake3::convenience::hash(b"foo").to_hex();
    let output = cmd!(b3sum_exe(), "--no-mmap", "--no-names", f.path()).read().unwrap();
    assert_eq!(&*expected, &*output);
}

#[test]
fn test_length_without_value_is_an_error() {
    blake3::initialize();
    let result = cmd!(b3sum_exe(), "--length").stdin_bytes("foo").stderr_capture().run();
    assert!(result.is_err());
}

#[test]
fn test_raw_with_multi_files_is_an_error() {
    blake3::initialize();
    let f1 = tempfile::NamedTempFile::new().unwrap();
    let f2 = tempfile::NamedTempFile::new().unwrap();

    // Make sure it doesn't error with just one file
    let result = cmd!(b3sum_exe(), "--raw", f1.path()).stdout_capture().run();
    assert!(result.is_ok());

    // Make sure it errors when both file are passed
    let result = cmd!(b3sum_exe(), "--raw", f1.path(), f2.path()).stderr_capture().run();
    assert!(result.is_err());
}

#[test]
#[cfg(unix)]
fn test_newline_and_backslash_escaping_on_unix() {
    blake3::initialize();
    let empty_hash = blake3::convenience::hash(b"").to_hex();
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("subdir")).unwrap();
    let names = ["abcdef", "abc\ndef", "abc\\def", "abc\rdef", "abc\r\ndef", "subdir/foo"];
    let mut paths = Vec::new();
    for name in &names {
        let path = dir.path().join(name);
        println!("creating file at {:?}", path);
        fs::write(&path, b"").unwrap();
        paths.push(path);
    }
    let output = cmd(b3sum_exe(), &names).dir(dir.path()).read().unwrap();
    let expected = format!(
        "\
{0}  abcdef
\\{0}  abc\\ndef
\\{0}  abc\\\\def
\\{0}  abc\\rdef
\\{0}  abc\\r\\ndef
{0}  subdir/foo",
        empty_hash,
    );
    println!("output");
    println!("======");
    println!("{}", output);
    println!();
    println!("expected");
    println!("========");
    println!("{}", expected);
    println!();
    assert_eq!(expected, output);
}

#[test]
#[cfg(unix)]
fn test_invalid_unicode_on_unix() {
    blake3::initialize();
    use std::os::unix::ffi::OsStringExt;

    let empty_hash = blake3::convenience::hash(b"").to_hex();
    let dir = tempfile::tempdir().unwrap();
    let names = ["abcdef".into(), OsString::from_vec(b"abc\xffdef".to_vec())];
    let mut paths = Vec::new();
    for name in &names {
        let path = dir.path().join(name);
        println!("creating file at {:?}", path);
        // Note: Some operating systems, macOS in particular, simply don't
        // allow invalid Unicode in filenames. On those systems, this write
        // will fail. That's fine, we'll just short-circuit this test in that
        // case. But assert that at least Linux allows this.
        let write_result = fs::write(&path, b"");
        if cfg!(target_os = "linux") {
            write_result.expect("Linux should allow invalid Unicode");
        } else if write_result.is_err() {
            return;
        }
        paths.push(path);
    }
    let output = cmd(b3sum_exe(), &names).dir(dir.path()).read().unwrap();
    let expected = format!(
        "\
{0}  abcdef
{0}  abc�def",
        empty_hash,
    );
    println!("output");
    println!("======");
    println!("{}", output);
    println!();
    println!("expected");
    println!("========");
    println!("{}", expected);
    println!();
    assert_eq!(expected, output);
}

#[test]
fn test_check() {
    blake3::initialize();
    // Make a directory full of files, and make sure the b3sum output in that
    // directory is what we expect.
    let a_hash = blake3::convenience::hash(b"a").to_hex();
    let b_hash = blake3::convenience::hash(b"b").to_hex();
    let cd_hash = blake3::convenience::hash(b"cd").to_hex();
    for tagged in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a"), b"a").unwrap();
        fs::write(dir.path().join("b"), b"b").unwrap();
        fs::create_dir(dir.path().join("c")).unwrap();
        fs::write(dir.path().join("c/d"), b"cd").unwrap();
        dbg!(tagged);
        let mut args = vec!["a", "b", "c/d"];
        if tagged {
            args.push("--tag");
        }
        let output = cmd(b3sum_exe(), args).dir(dir.path()).stdout_capture().stderr_capture().run().unwrap();
        let stdout = std::str::from_utf8(&output.stdout).unwrap();
        let stderr = std::str::from_utf8(&output.stderr).unwrap();
        let expected_checkfile = if tagged {
            format!(
                "BLAKE3 (a) = {}\n\
                 BLAKE3 (b) = {}\n\
                 BLAKE3 (c/d) = {}\n",
                a_hash, b_hash, cd_hash,
            )
        } else {
            format!(
                "{}  a\n\
                 {}  b\n\
                 {}  c/d\n",
                a_hash, b_hash, cd_hash,
            )
        };
        dbg!(&expected_checkfile);
        assert_eq!(expected_checkfile, stdout);
        assert_eq!("", stderr);

        // Now use the output we just validated as a checkfile, passed to stdin.
        let output = cmd!(b3sum_exe(), "--check").stdin_bytes(expected_checkfile.as_bytes()).dir(dir.path()).stdout_capture().stderr_capture().run().unwrap();
        let stdout = std::str::from_utf8(&output.stdout).unwrap();
        let stderr = std::str::from_utf8(&output.stderr).unwrap();
        let expected_check_output = "\
         a: OK\n\
         b: OK\n\
         c/d: OK\n";
        assert_eq!(expected_check_output, stdout);
        assert_eq!("", stderr);

        // Check the same file, but with Windows-style newlines.
        let windows_style = expected_checkfile.replace("\n", "\r\n");
        let output = cmd!(b3sum_exe(), "--check").stdin_bytes(windows_style.as_bytes()).dir(dir.path()).stdout_capture().stderr_capture().run().unwrap();
        let stdout = std::str::from_utf8(&output.stdout).unwrap();
        let stderr = std::str::from_utf8(&output.stderr).unwrap();
        let expected_check_output = "\
         a: OK\n\
         b: OK\n\
         c/d: OK\n";
        assert_eq!(expected_check_output, stdout);
        assert_eq!("", stderr);

        // Now pass the same checkfile twice on the command line just for fun.
        let checkfile_path = dir.path().join("checkfile");
        fs::write(&checkfile_path, &expected_checkfile).unwrap();
        let output = cmd!(b3sum_exe(), "--check", &checkfile_path, &checkfile_path).dir(dir.path()).stdout_capture().stderr_capture().run().unwrap();
        let stdout = std::str::from_utf8(&output.stdout).unwrap();
        let stderr = std::str::from_utf8(&output.stderr).unwrap();
        let mut double_check_output = String::new();
        double_check_output.push_str(&expected_check_output);
        double_check_output.push_str(&expected_check_output);
        assert_eq!(double_check_output, stdout);
        assert_eq!("", stderr);

        // Corrupt one of the files and check again.
        fs::write(dir.path().join("b"), b"CORRUPTION").unwrap();
        let output = cmd!(b3sum_exe(), "--check", &checkfile_path).dir(dir.path()).stdout_capture().stderr_capture().unchecked().run().unwrap();
        let stdout = std::str::from_utf8(&output.stdout).unwrap();
        let stderr = std::str::from_utf8(&output.stderr).unwrap();
        let expected_check_failure = "\
        a: OK\n\
        b: FAILED\n\
        c/d: OK\n";
        assert!(!output.status.success());
        assert_eq!(expected_check_failure, stdout);
        assert_eq!("b3sum: WARNING: 1 computed checksum did NOT match\n", stderr,);

        // Delete one of the files and check again.
        fs::remove_file(dir.path().join("b")).unwrap();
        let open_file_error = fs::File::open(dir.path().join("b")).unwrap_err();
        let output = cmd!(b3sum_exe(), "--check", &checkfile_path).dir(dir.path()).stdout_capture().stderr_capture().unchecked().run().unwrap();
        let stdout = std::str::from_utf8(&output.stdout).unwrap();
        let stderr = std::str::from_utf8(&output.stderr).unwrap();
        let expected_check_failure = format!(
            "a: OK\n\
         b: FAILED ({})\n\
         c/d: OK\n",
            open_file_error,
        );
        assert!(!output.status.success());
        assert_eq!(expected_check_failure, stdout);
        assert_eq!("b3sum: WARNING: 1 computed checksum did NOT match\n", stderr,);

        // Confirm that --quiet suppresses the OKs but not the FAILEDs.
        let output = cmd!(b3sum_exe(), "--check", "--quiet", &checkfile_path).dir(dir.path()).stdout_capture().stderr_capture().unchecked().run().unwrap();
        let stdout = std::str::from_utf8(&output.stdout).unwrap();
        let stderr = std::str::from_utf8(&output.stderr).unwrap();
        let expected_check_failure = format!("b: FAILED ({})\n", open_file_error);
        assert!(!output.status.success());
        assert_eq!(expected_check_failure, stdout);
        assert_eq!("b3sum: WARNING: 1 computed checksum did NOT match\n", stderr,);
    }
}

#[test]
fn test_check_invalid_characters() {
    blake3::initialize();
    // Check that a null character in the path fails.
    let output = cmd!(b3sum_exe(), "--check")
        .stdin_bytes("0000000000000000000000000000000000000000000000000000000000000000  \0")
        .stdout_capture()
        .stderr_capture()
        .unchecked()
        .run()
        .unwrap();
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    let stderr = std::str::from_utf8(&output.stderr).unwrap();
    let expected_stderr = "\
        b3sum: Null character in path\n\
        b3sum: WARNING: 1 computed checksum did NOT match\n";
    assert!(!output.status.success());
    assert_eq!("", stdout);
    assert_eq!(expected_stderr, stderr);

    // Check that a Unicode replacement character in the path fails.
    let output = cmd!(b3sum_exe(), "--check")
        .stdin_bytes("0000000000000000000000000000000000000000000000000000000000000000  �")
        .stdout_capture()
        .stderr_capture()
        .unchecked()
        .run()
        .unwrap();
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    let stderr = std::str::from_utf8(&output.stderr).unwrap();
    let expected_stderr = "\
        b3sum: Unicode replacement character in path\n\
        b3sum: WARNING: 1 computed checksum did NOT match\n";
    assert!(!output.status.success());
    assert_eq!("", stdout);
    assert_eq!(expected_stderr, stderr);

    // Check that an invalid escape sequence in the path fails.
    let output = cmd!(b3sum_exe(), "--check")
        .stdin_bytes("\\0000000000000000000000000000000000000000000000000000000000000000  \\a")
        .stdout_capture()
        .stderr_capture()
        .unchecked()
        .run()
        .unwrap();
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    let stderr = std::str::from_utf8(&output.stderr).unwrap();
    let expected_stderr = "\
        b3sum: Invalid backslash escape\n\
        b3sum: WARNING: 1 computed checksum did NOT match\n";
    assert!(!output.status.success());
    assert_eq!("", stdout);
    assert_eq!(expected_stderr, stderr);
}

#[test]
fn test_globbing() {
    blake3::initialize();
    // Globbing is the shell's.
    let dir = tempfile::tempdir().unwrap();
    let file1 = dir.path().join("file1");
    fs::write(&file1, b"foo").unwrap();
    let file2 = dir.path().join("file2");
    fs::write(&file2, b"bar").unwrap();

    let foo_hash = blake3::convenience::hash(b"foo");
    let bar_hash = blake3::convenience::hash(b"bar");
    // NOTE: This assumes that the glob will be expanded in alphabetical order,
    //       to "file1 file2" rather than "file2 file1". So far, this seems to
    //       be true (guaranteed?) of Unix shell behavior. It's possible that this could
    //       start failing in the future, though, or on some unknown platform.
    //       If that ever happens, we'll need to relax this test somehow,
    //       probably by just testing for both possible outputs. I'm not
    //       handling that case in advance, though, because I'd prefer to hear
    //       about it if it comes up.
    let expected = format!("{}  file1\n{}  file2", foo_hash.to_hex(), bar_hash.to_hex());

    let star_command = format!("{} *", b3sum_exe().to_str().unwrap());
    let output = cmd!("/bin/sh", "-c", star_command).dir(dir.path()).read().unwrap();
    assert_eq!(expected, output);
}

/// --num-threads 1 and 3 hash as the default does: a file past a read
/// piece and the pool's threshold beside a small one, and the same bytes
/// from stdin, every digest the library's. 0 is refused.
#[test]
fn test_num_threads() {
    blake3::initialize();
    let dir = tempfile::tempdir().unwrap();
    let big: Vec<u8> = (0..9u32 << 20).map(|i| (i * 7 + i / 4093) as u8).collect();
    let (file1, file2) = (dir.path().join("big"), dir.path().join("small"));
    fs::write(&file1, &big).unwrap();
    fs::write(&file2, b"bar").unwrap();
    let expected = format!("{}\n{}", blake3::convenience::hash(&big).to_hex(), blake3::convenience::hash(b"bar").to_hex());
    for threads in ["1", "3"] {
        let output = cmd!(b3sum_exe(), "--num-threads", threads, "--no-names", &file1, &file2).read().unwrap();
        assert_eq!(expected, output, "{threads} threads");
        let output = cmd!(b3sum_exe(), "--num-threads", threads, "--no-names").stdin_bytes(big.clone()).read().unwrap();
        assert_eq!(blake3::convenience::hash(&big).to_hex().to_string(), output, "{threads} threads, stdin");
    }
    let zero = cmd!(b3sum_exe(), "--num-threads", "0").stdin_bytes("foo").stderr_null().unchecked().run().unwrap();
    assert!(!zero.status.success(), "0 threads is refused");
}

/// Inputs around the 4 MiB read piece (one piece, one byte more, two
/// pieces, and more than two), as files and through stdin: each gives
/// hash()'s digest.
#[test]
fn test_inputs_around_the_read_piece() {
    blake3::initialize();
    let dir = tempfile::tempdir().unwrap();
    for len in [(3usize << 20) + 12345, 4 << 20, (4 << 20) + 1, 8 << 20, (9 << 20) + 12345] {
        let path = dir.path().join(format!("input-{len}"));
        let data: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
        fs::write(&path, &data).unwrap();
        let expected = blake3::convenience::hash(&data).to_hex().to_string();
        let output = cmd!(b3sum_exe(), &path).read().unwrap();
        assert_eq!(&output[..64], expected, "file of {len} bytes");
        let output = cmd!(b3sum_exe()).stdin_bytes(data).read().unwrap();
        assert_eq!(&output[..64], expected, "stdin of {len} bytes");
    }
}
/// `--num-threads 1` reads and hashes on one thread (no pool), from stdin
/// and from a file read rather than mapped, over several pieces: the same
/// digests as the default, and no file reported changed.
#[test]
fn test_one_thread_matches_the_default() {
    let input: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
    blake3::initialize();
    let expected = blake3::convenience::hash(&input).to_hex().to_string();
    for threads in [None, Some("1")] {
        let args: Vec<&str> = threads.map_or(vec!["--no-names"], |n| vec!["--no-names", "--num-threads", n]);
        let out = cmd(b3sum_exe(), &args).stdin_bytes(input.clone()).read().unwrap();
        assert_eq!(out, expected, "stdin, --num-threads {threads:?}");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        std::fs::write(&path, &input).unwrap();
        let mut file_args = args.clone();
        file_args.extend(["--no-mmap", path.to_str().unwrap()]);
        let out = cmd(b3sum_exe(), &file_args).read().unwrap();
        assert_eq!(out, expected, "a file read, --num-threads {threads:?}");
    }
}

/// Pseudo-files among many (the batched path on Linux): each hashes as its
/// content, whatever size it reports (/proc's report 0, /sys's 4096).
#[test]
#[cfg(target_os = "linux")]
fn test_pseudo_files_among_many() {
    blake3::initialize();
    let names = ["/proc/version", "/sys/kernel/mm/transparent_hugepage/enabled", "/proc/version"];
    let present: Vec<&str> = names.into_iter().filter(|p| fs::read(p).is_ok()).collect();
    if present.len() < 2 {
        return;
    }
    let output = cmd(b3sum_exe(), std::iter::once("--no-names").chain(present.iter().copied())).stdout_capture().stderr_capture().unchecked().run().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected: Vec<String> = present.iter().map(|p| blake3::convenience::hash(&fs::read(p).unwrap()).to_hex().to_string()).collect();
    assert_eq!(expected.join("\n"), String::from_utf8_lossy(&output.stdout).trim_end());
}

#[test]
#[cfg(target_os = "linux")]
fn test_linux_cold_evicted_mixed_batch_and_single_large() {
    use std::os::fd::AsRawFd;
    blake3::initialize();
    let dir = tempfile::tempdir_in(".").unwrap_or_else(|_| tempfile::tempdir().unwrap());
    let sizes = [0usize, 1, 63, 64, 65, 4096, 16384, 16385, 100_000, 600_000, 4_300_000];
    let mut files = Vec::new();
    let mut expected_plain = Vec::new();
    let mut expected_dk = Vec::new();
    for i in 0..72usize {
        let len = sizes[i % sizes.len()];
        let path = dir.path().join(format!("f-{i:03}"));
        let data: Vec<u8> = (0..len).map(|b| (b.wrapping_mul(17).wrapping_add(i)) as u8).collect();
        let mut wf = fs::File::create(&path).unwrap();
        wf.write_all(&data).unwrap();
        wf.sync_all().unwrap();
        expected_plain.push(format!("{}  {}", blake3::convenience::hash(&data).to_hex(), path.to_string_lossy()));
        let mut dk = blake3::convenience::Hasher::new(blake3::Mode::DeriveKey("cli-cold-ctx"), None);
        dk.update(&data);
        expected_dk.push(format!("{}  {}", dk.finalize().to_hex(), path.to_string_lossy()));
        files.push(path);
    }

    let evict_all = |paths: &[PathBuf]| {
        for p in paths {
            let f = fs::File::open(p).unwrap();
            unsafe { libc::posix_fadvise(f.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED) };
        }
    };

    // 1. Cold multi-threaded mixed batch
    evict_all(&files);
    let out_mt = cmd(b3sum_exe(), &files).read().unwrap();
    assert_eq!(out_mt, expected_plain.join("\n"));

    // 2. Cold `--num-threads 1` mixed batch
    evict_all(&files);
    let mut args_1t: Vec<OsString> = vec!["--num-threads".into(), "1".into()];
    args_1t.extend(files.iter().map(|p| p.as_os_str().to_owned()));
    let out_1t = cmd(b3sum_exe(), &args_1t).read().unwrap();
    assert_eq!(out_1t, expected_plain.join("\n"));

    // 3. Cold `--derive-key` mixed batch
    evict_all(&files);
    let mut args_dk: Vec<OsString> = vec!["--derive-key".into(), "cli-cold-ctx".into()];
    args_dk.extend(files.iter().map(|p| p.as_os_str().to_owned()));
    let out_dk = cmd(b3sum_exe(), &args_dk).read().unwrap();
    assert_eq!(out_dk, expected_dk.join("\n"));

    // 4. Cold `--num-threads 1` single > 4 MiB file (`hash_single_cold_large_file_1t`)
    let big_idx = sizes.iter().position(|&s| s > (4 << 20)).unwrap();
    evict_all(&files[big_idx..=big_idx]);
    let out_big_1t = cmd!(b3sum_exe(), "--num-threads", "1", &files[big_idx]).read().unwrap();
    assert_eq!(out_big_1t, expected_plain[big_idx]);

    // 5. Cold parallel `--check` over the 72-file manifest
    let check_path = dir.path().join("manifest.b3");
    fs::write(&check_path, expected_plain.join("\n") + "\n").unwrap();
    evict_all(&files);
    let chk_out = cmd!(b3sum_exe(), "--check", &check_path).read().unwrap();
    let expected_ok: Vec<String> = files.iter().map(|p| format!("{}: OK", p.to_string_lossy())).collect();
    assert_eq!(chk_out, expected_ok.join("\n"));

    evict_all(&files);
    let chk_quiet = cmd!(b3sum_exe(), "--check", "--quiet", &check_path).read().unwrap();
    assert!(chk_quiet.is_empty());
}

#[test]
fn test_parallel_check_preserves_line_order_across_errors_and_failures() {
    blake3::initialize();
    let dir = tempfile::tempdir().unwrap();
    let mut lines = Vec::new();
    let mut want_stdout = Vec::new();
    for i in 0..40usize {
        let p = dir.path().join(format!("item-{i:02}"));
        let data = vec![i as u8; i * 300 + 5];
        fs::write(&p, &data).unwrap();
        let h = blake3::convenience::hash(&data).to_hex();
        if i == 7 {
            lines.push("not-a-valid-check-line".to_string());
        } else if i == 15 {
            let missing = dir.path().join("does-not-exist");
            lines.push(format!("{h}  {}", missing.to_string_lossy()));
            want_stdout.push(format!("{}: FAILED (", missing.to_string_lossy()));
        } else if i == 23 {
            let wrong = "0".repeat(64);
            lines.push(format!("{wrong}  {}", p.to_string_lossy()));
            want_stdout.push(format!("{}: FAILED", p.to_string_lossy()));
        } else {
            lines.push(format!("{h}  {}", p.to_string_lossy()));
            want_stdout.push(format!("{}: OK", p.to_string_lossy()));
        }
    }
    let cf = dir.path().join("mixed.b3");
    fs::write(&cf, lines.join("\n") + "\n").unwrap();
    let out = cmd!(b3sum_exe(), "--check", &cf).stdout_capture().stderr_capture().unchecked().run().unwrap();
    assert!(!out.status.success());
    let stdout_str = String::from_utf8_lossy(&out.stdout);
    let got_lines: Vec<&str> = stdout_str.lines().collect();
    assert_eq!(got_lines.len(), want_stdout.len());
    for (got, want) in got_lines.iter().zip(&want_stdout) {
        if want.ends_with("FAILED (") {
            assert!(got.starts_with(want), "got={got} want_prefix={want}");
        } else {
            assert_eq!(*got, want.as_str());
        }
    }
    let stderr_str = String::from_utf8_lossy(&out.stderr);
    assert!(stderr_str.contains("Invalid check line format"), "{stderr_str}");
    assert!(stderr_str.contains("3 computed checksums did NOT match"), "{stderr_str}");
}

/// `--outboard DIR`: the hashes as ever, and beside them each file's
/// outboard at DIR/<hash>.obao4, the bytes bao-tree itself makes for 16 KiB
/// chunk groups: an empty file, files around a group and around powers of
/// two of groups, and standard input; in the keyed mode too. A file that
/// cannot be read fails alone, and the options an outboard cannot go with
/// are refused.
#[test]
fn test_outboard_files_are_bao_trees() {
    blake3::initialize();
    let dir = tempfile::tempdir().unwrap();
    let out_dir = dir.path().join("outboards");
    fs::create_dir(&out_dir).unwrap();
    let group = 16 * 1024;
    let lens = [0usize, 1, group, group + 1, 3 * group, 100_000, 64 * group, 64 * group + 1, 2_000_000];
    let mut files = Vec::new();
    let mut expected = Vec::new();
    for (i, &len) in lens.iter().enumerate() {
        let data: Vec<u8> = (0..len).map(|j| (j.wrapping_mul(37) ^ (j >> 7) ^ i) as u8).collect();
        let path = dir.path().join(format!("file-{i}"));
        fs::write(&path, &data).unwrap();
        let theirs = bao_ref::create(&data);
        assert_eq!(&theirs.root, blake3::convenience::hash(&data).as_bytes());
        let root_hex = theirs.root_hex();
        expected.push((format!("{root_hex}  {}", path.to_string_lossy()), out_dir.join(format!("{root_hex}.obao4")), theirs.data));
        files.push(path);
    }
    let mut args: Vec<OsString> = vec!["--outboard".into(), out_dir.clone().into()];
    args.extend(files.iter().map(|p| p.as_os_str().to_owned()));
    let output = cmd(b3sum_exe(), &args).read().unwrap();
    assert_eq!(output, expected.iter().map(|e| e.0.clone()).collect::<Vec<_>>().join("\n"));
    // An empty outboard (a file of one group or less) is no file.
    for (line, at, want) in &expected {
        if want.is_empty() {
            assert!(!at.exists(), "no file for the empty outboard of {line}");
        } else {
            assert_eq!(&fs::read(at).unwrap(), want, "the outboard of {line}");
        }
    }
    assert_eq!(lens.iter().filter(|&&len| len > group).count(), 6);
    assert_eq!(fs::read_dir(&out_dir).unwrap().count(), 6, "one outboard a file of more than a group");

    // Standard input, and one thread.
    let data: Vec<u8> = (0..300_000usize).map(|j| (j * 13) as u8).collect();
    let theirs = bao_ref::create(&data);
    let root_hex = theirs.root_hex();
    let output = cmd!(b3sum_exe(), "--num-threads", "1", "--outboard", &out_dir).stdin_bytes(data.clone()).read().unwrap();
    assert_eq!(output, format!("{root_hex}  -"));
    assert_eq!(fs::read(out_dir.join(format!("{root_hex}.obao4"))).unwrap(), theirs.data);

    // Keyed: the key on standard input, as ever.
    let key = [0x42u8; 32];
    let theirs = bao_ref::create_keyed(&data, &key);
    let root_hex = theirs.root_hex();
    let big = dir.path().join("keyed-input");
    fs::write(&big, &data).unwrap();
    let output = cmd!(b3sum_exe(), "--keyed", "--no-names", "--outboard", &out_dir, &big).stdin_bytes(key.to_vec()).read().unwrap();
    assert_eq!(output, root_hex);
    assert_eq!(fs::read(out_dir.join(format!("{root_hex}.obao4"))).unwrap(), theirs.data);

    // An outboard's place is taken by name, never written through. A link
    // there is replaced, and its target left as it was. An outboard
    // already there stays whole under its other name, as a reader that
    // had it open would go on seeing it. No other file is left.
    let (k, (_, place, want)) = expected.iter().enumerate().find(|(_, e)| !e.2.is_empty()).unwrap();
    for entry in fs::read_dir(&out_dir).unwrap() {
        fs::remove_file(entry.unwrap().path()).unwrap();
    }
    let victim = dir.path().join("victim");
    fs::write(&victim, b"not an outboard").unwrap();
    std::os::unix::fs::symlink(&victim, place).unwrap();
    cmd!(b3sum_exe(), "--outboard", &out_dir, &files[k]).read().unwrap();
    assert_eq!(fs::read(&victim).unwrap(), b"not an outboard");
    assert!(fs::symlink_metadata(place).unwrap().is_file());
    assert_eq!(&fs::read(place).unwrap(), want);
    fs::write(place, b"an older outboard, longer or shorter").unwrap();
    let held = out_dir.join("held-by-a-reader");
    fs::hard_link(place, &held).unwrap();
    cmd!(b3sum_exe(), "--outboard", &out_dir, &files[k], &files[k]).read().unwrap();
    assert_eq!(fs::read(&held).unwrap(), b"an older outboard, longer or shorter");
    assert_eq!(&fs::read(place).unwrap(), want);
    fs::remove_file(&held).unwrap();
    let left: Vec<_> = fs::read_dir(&out_dir).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(left, [place.file_name().unwrap()]);

    // Keyed, standard input cannot also be the message: the key was all
    // of it. An error, as without --outboard, and no hash of nothing.
    let result =
        cmd!(b3sum_exe(), "--keyed", "--outboard", &out_dir, "-").stdin_bytes(key.to_vec()).stdout_capture().stderr_capture().unchecked().run().unwrap();
    assert!(!result.status.success());
    assert_eq!(result.stdout, b"");
    assert!(String::from_utf8_lossy(&result.stderr).contains("Cannot open `-` in keyed mode"));

    // A file that is not there fails alone: the others' lines and outboards are made.
    let missing = dir.path().join("no-such-file");
    let result = cmd!(b3sum_exe(), "--outboard", &out_dir, &files[1], &missing, &files[3]).stdout_capture().stderr_capture().unchecked().run().unwrap();
    assert!(!result.status.success());
    assert_eq!(String::from_utf8_lossy(&result.stdout).trim_end(), format!("{}\n{}", expected[1].0, expected[3].0));
    assert!(String::from_utf8_lossy(&result.stderr).contains("no-such-file"));

    // Many files, short and long in turn, with one that is not there among
    // them: the short ones are hashed by the engine for many files and the
    // long ones on the threads, and the lines come in the files' order,
    // on one thread and on several.
    let many_dir = dir.path().join("many");
    fs::create_dir(&many_dir).unwrap();
    let mut many = Vec::new();
    let mut lines = Vec::new();
    let mut outboards = Vec::new();
    for i in 0..120usize {
        let len = [i * 139, group + 1 + i * 7919, group, 5 * group + i][i % 4];
        let data: Vec<u8> = (0..len).map(|j| (j.wrapping_mul(41) ^ (j >> 5) ^ i) as u8).collect();
        let path = many_dir.join(format!("f{i:03}"));
        fs::write(&path, &data).unwrap();
        let theirs = bao_ref::create(&data);
        let root_hex = theirs.root_hex();
        lines.push(format!("{root_hex}  {}", path.to_string_lossy()));
        outboards.push((format!("{root_hex}.obao4"), theirs.data));
        many.push(path);
        if i == 60 {
            many.push(missing.clone());
        }
    }
    for threads in ["1", "5"] {
        let fresh = dir.path().join(format!("outboards-{threads}"));
        fs::create_dir(&fresh).unwrap();
        let mut args: Vec<OsString> = vec!["--num-threads".into(), threads.into(), "--outboard".into(), fresh.clone().into()];
        args.extend(many.iter().map(|p| p.as_os_str().to_owned()));
        let result = cmd(b3sum_exe(), &args).stdout_capture().stderr_capture().unchecked().run().unwrap();
        assert!(!result.status.success(), "one file is not there");
        assert_eq!(String::from_utf8_lossy(&result.stdout).trim_end(), lines.join("\n"), "threads={threads}");
        assert_eq!(String::from_utf8_lossy(&result.stderr).matches("no-such-file").count(), 1);
        for (name, want) in outboards.iter().filter(|o| !o.1.is_empty()) {
            assert_eq!(&fs::read(fresh.join(name)).unwrap(), want, "{name}, threads={threads}");
        }
        let distinct: std::collections::HashSet<_> = outboards.iter().filter(|o| !o.1.is_empty()).map(|o| &o.0).collect();
        assert!(distinct.len() >= 60 && distinct.len() < outboards.len(), "long files and short");
        assert_eq!(fs::read_dir(&fresh).unwrap().count(), distinct.len(), "one outboard a hash of a long file, and no other file");
    }

    // A directory that is not there: nothing is hashed, though the file
    // is a short one, whose outboard would be no file.
    let result = cmd!(b3sum_exe(), "--outboard", dir.path().join("no-such-dir"), &files[1]).stdout_capture().stderr_capture().unchecked().run().unwrap();
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());

    // What an outboard cannot go with.
    for refused in [vec!["--check"], vec!["--raw"], vec!["--length", "16"], vec!["--seek", "1"], vec!["--derive-key", "context"]] {
        let mut args: Vec<OsString> = vec!["--outboard".into(), out_dir.clone().into()];
        args.extend(refused.iter().map(OsString::from));
        args.push(files[1].as_os_str().to_owned());
        let result = cmd(b3sum_exe(), &args).stdout_capture().stderr_capture().unchecked().run().unwrap();
        assert!(!result.status.success(), "--outboard with {refused:?}");
    }
}

/// `b3sum < file` hashes the file as `b3sum file` does, at every size
/// class (a stack buffer's, a map's, a direct read's where the file is
/// cold), and a file whose reader has moved on is hashed from where it
/// stands. Either way the file's position, which the caller shares, is
/// left at its end, as a stream read to its end leaves it: standard
/// input named a second time is the empty input, however the first was
/// taken in (a map and a read by position move no position themselves,
/// and gave the second name the same file again).
#[test]
#[cfg(unix)]
fn test_stdin_from_a_file_is_hashed_as_the_file() {
    use std::io::{Seek, SeekFrom};
    blake3::initialize();
    let dir = tempfile::tempdir().unwrap();
    let empty = blake3::convenience::hash(b"").to_hex();
    for len in [0usize, 1, 4096, 65_536, 600_000, 2 << 20, 5_000_000] {
        let data: Vec<u8> = (0..len).map(|i| (i.wrapping_mul(31) ^ (i >> 7)) as u8).collect();
        let path = dir.path().join(format!("in-{len}"));
        fs::write(&path, &data).unwrap();
        // b3sum's standard input is a copy of `file`'s descriptor, so the
        // two share one position: where b3sum leaves it is read here.
        let run = |file: &mut fs::File, args: &[&str]| {
            let out = std::process::Command::new(b3sum_exe()).args(args).stdin(file.try_clone().unwrap()).output().unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
            (String::from_utf8(out.stdout).unwrap(), file.stream_position().unwrap())
        };
        let hash = blake3::convenience::hash(&data).to_hex();
        for args in [&[][..], &["--no-mmap"], &["--num-threads", "1"]] {
            let mut file = fs::File::open(&path).unwrap();
            assert_eq!(run(&mut file, args), (format!("{hash}  -\n"), len as u64), "len={len} {args:?}");
            let mut file = fs::File::open(&path).unwrap();
            let twice = [args, &["-", "-"]].concat();
            assert_eq!(run(&mut file, &twice), (format!("{hash}  -\n{empty}  -\n"), len as u64), "len={len} {twice:?}");
        }
        if len > 100 {
            let mut file = fs::File::open(&path).unwrap();
            file.seek(SeekFrom::Start(100)).unwrap();
            let rest = blake3::convenience::hash(&data[100..]).to_hex();
            assert_eq!(run(&mut file, &[]), (format!("{rest}  -\n"), len as u64), "len={len}, from byte 100");
        }
    }
}

/// Standard input named twice is the whole stream and then the empty
/// input, on every path: here the one for other lengths than a hash's,
/// whose file threads each read the pipe in pieces and printed two
/// values that were neither. The stream is longer than two of their
/// reads.
#[test]
fn test_stdin_named_twice_is_read_once_in_order() {
    blake3::initialize();
    let data: Vec<u8> = (0..12_048_000usize).map(|i| (i.wrapping_mul(97) ^ (i >> 11)) as u8).collect();
    let xof = |input: &[u8], seek: u64| {
        let mut hasher = blake3::convenience::Hasher::new(blake3::Mode::Plain, None);
        hasher.update(input);
        let mut reader = hasher.finalize_xof();
        reader.set_position(seek);
        let mut out = [0u8; 33];
        reader.fill(&mut out);
        hex_encode(out)
    };
    for (seek, threads) in [(0u64, "2"), (0, "8"), (1, "2")] {
        let args = ["--length=33".to_string(), format!("--seek={seek}"), format!("--num-threads={threads}"), "-".into(), "-".into()];
        for _ in 0..4 {
            let output = cmd(b3sum_exe(), &args).stdin_bytes(data.clone()).read().unwrap();
            assert_eq!(output, format!("{}  -\n{}  -", xof(&data, seek), xof(b"", seek)), "{args:?}");
        }
    }
    // The same where one of the two names is another name for the
    // stream: the first named reads it, whichever that is.
    #[cfg(target_os = "linux")]
    for names in [["-", "/dev/stdin"], ["/dev/stdin", "-"], ["/dev/fd/0", "-"], ["/dev/stdin", "/dev/stdin"], ["/dev/stdin", "/proc/self/fd/0"]] {
        for length in ["32", "33"] {
            let output = cmd!(b3sum_exe(), "--length", length, names[0], names[1]).stdin_bytes(data.clone()).read().unwrap();
            let (whole, empty) = (xof(&data, 0), xof(b"", 0));
            let cut = 2 * length.parse::<usize>().unwrap();
            assert_eq!(output, format!("{}  {}\n{}  {}", &whole[..cut], names[0], &empty[..cut], names[1]), "{names:?} --length {length}");
        }
    }
    // And for outboards: the first name's is the stream's, the second's
    // the empty input's.
    let dir = tempfile::tempdir().unwrap();
    for _ in 0..4 {
        let output = cmd!(b3sum_exe(), "--outboard", dir.path(), "--num-threads=8", "-", "-").stdin_bytes(data.clone()).read().unwrap();
        assert_eq!(output, format!("{}  -\n{}  -", &xof(&data, 0)[..64], &xof(b"", 0)[..64]));
    }
}

/// A file under /proc on standard input: it reports no bytes, cannot
/// seek to its end, and is read through its position, which is left
/// where the reads ended. (A move to the end after every file on
/// standard input failed here, with no hash printed.)
#[test]
#[cfg(target_os = "linux")]
fn test_stdin_from_a_proc_file() {
    use std::io::Seek;
    blake3::initialize();
    let content = fs::read("/proc/version").unwrap();
    let mut file = fs::File::open("/proc/version").unwrap();
    let out = std::process::Command::new(b3sum_exe()).stdin(file.try_clone().unwrap()).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8(out.stdout).unwrap(), format!("{}  -\n", blake3::convenience::hash(&content).to_hex()));
    assert_eq!(file.stream_position().unwrap(), content.len() as u64);
}

/// A file on standard input that reports a size and holds no byte (some
/// under /sys) is at its end where it stands, and is left there; and a
/// process allowed no descriptor more than its three still gets its
/// hash (the position was once asked of a copy of descriptor 0).
#[test]
#[cfg(target_os = "linux")]
fn test_stdin_from_an_empty_file_that_reports_a_size_and_with_no_descriptor_to_spare() {
    use std::io::Seek;
    blake3::initialize();
    let empty = "/sys/class/net/lo/ifalias";
    if fs::read(empty).is_ok_and(|bytes| bytes.is_empty()) && fs::metadata(empty).is_ok_and(|meta| meta.len() > 0) {
        let mut file = fs::File::open(empty).unwrap();
        let out = std::process::Command::new(b3sum_exe()).stdin(file.try_clone().unwrap()).output().unwrap();
        assert_eq!(String::from_utf8(out.stdout).unwrap(), format!("{}  -\n", blake3::convenience::hash(b"").to_hex()));
        assert_eq!(file.stream_position().unwrap(), 0);
    }
    let dir = tempfile::tempdir().unwrap();
    let data = vec![3u8; 100_000];
    fs::write(dir.path().join("data"), &data).unwrap();
    // Descriptors the test's runner left open are closed first: the
    // system's loader needs one free to load the program's libraries.
    let script = format!("exec < data 3<&- 4<&- 5<&- 6<&- 7<&- 8<&- 9<&-; ulimit -n 4; exec '{}'", b3sum_exe().display());
    let out = cmd!("sh", "-c", script).dir(dir.path()).stdout_capture().stderr_capture().unchecked().run().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8(out.stdout).unwrap(), format!("{}  -\n", blake3::convenience::hash(&data).to_hex()));
}

/// Only the operand `-` itself is standard input. `-/` names a
/// directory, and there is none: an error, as a named checkfile's entry
/// too, where it was the hash of standard input, and "OK".
#[test]
#[cfg(unix)]
fn test_only_a_lone_dash_is_stdin() {
    blake3::initialize();
    let dir = tempfile::tempdir().unwrap();
    let foo = blake3::convenience::hash(b"foo").to_hex();
    let out = cmd!(b3sum_exe(), "--", "-/").stdin_bytes("foo").dir(dir.path()).stdout_capture().stderr_capture().unchecked().run().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty(), "{}", String::from_utf8_lossy(&out.stdout));
    fs::write(dir.path().join("list"), format!("{foo}  -/\n")).unwrap();
    let out = cmd!(b3sum_exe(), "--check", "list").stdin_bytes("foo").dir(dir.path()).stdout_capture().stderr_capture().unchecked().run().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8(out.stdout).unwrap().starts_with("-/: FAILED ("));
}

/// A --tag line checks, whatever its file's name: one with two spaces
/// in it was cut at them, as the other form of line is.
#[test]
fn test_a_tagged_line_of_a_name_with_two_spaces_checks() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a  b", "c) = d", "e  f) = g  h"] {
        fs::write(dir.path().join(name), name.as_bytes()).unwrap();
        let line = cmd!(b3sum_exe(), "--tag", name).dir(dir.path()).read().unwrap();
        assert!(line.starts_with(&format!("BLAKE3 ({name}) = ")), "{line}");
        let checked = cmd!(b3sum_exe(), "--check").stdin_bytes(format!("{line}\n")).dir(dir.path()).read().unwrap();
        assert_eq!(checked, format!("{name}: OK"));
        let plain = cmd!(b3sum_exe(), name).dir(dir.path()).read().unwrap();
        let checked = cmd!(b3sum_exe(), "--check").stdin_bytes(format!("{plain}\n")).dir(dir.path()).read().unwrap();
        assert_eq!(checked, format!("{name}: OK"));
    }
}

/// BLAKE3's output is 2^64 - 1 bytes long. Its last byte is printed, in
/// text as in raw bytes (text filled a whole block, which stopped the
/// process there), and a request for a byte past the end is refused
/// (raw output gave fewer bytes than were asked for, with success).
#[test]
fn test_the_outputs_last_byte_and_none_past_it() {
    blake3::initialize();
    let last = u64::MAX - 1;
    let mut reader = blake3::convenience::Hasher::new(blake3::Mode::Plain, None);
    reader.update(b"foo");
    let mut reader = reader.finalize_xof();
    reader.set_position(last);
    let mut byte = [0u8; 1];
    reader.fill(&mut byte);
    let seek = format!("--seek={last}");
    let text = cmd!(b3sum_exe(), "--no-names", "--length=1", &seek).stdin_bytes("foo").read().unwrap();
    assert_eq!(text, hex_encode(byte));
    let raw = cmd!(b3sum_exe(), "--raw", "--length=1", &seek).stdin_bytes("foo").stdout_capture().run().unwrap().stdout;
    assert_eq!(raw, byte);
    for args in [&["--length=2", &seek][..], &["--raw", "--length=2", &seek], &["--length=1", "--seek=18446744073709551615"]] {
        let refused = cmd(b3sum_exe(), args).stdin_bytes("foo").stdout_capture().stderr_capture().unchecked().run().unwrap();
        assert_eq!(refused.status.code(), Some(1), "{args:?}");
        assert!(refused.stdout.is_empty(), "{args:?}");
        assert!(String::from_utf8_lossy(&refused.stderr).contains("past its end"), "{args:?}");
    }
    // No byte, asked for at the very end, is no byte past it.
    let none = cmd!(b3sum_exe(), "--length=0", "--seek=18446744073709551615").stdin_bytes("foo").read().unwrap();
    assert_eq!(none, "  -");
}

/// A checkfile read from a pipe cannot check that pipe by another of
/// its names either (`/dev/stdin`), and a checkfile opened by such a
/// name cannot check `-`: the entry fails and the others are checked.
/// (The entry was read out of the checkfile's own lines, which were then
/// never read as entries: 4,096 entries on, a checkfile's last lines
/// went unchecked, with success.) A checkfile in a file of its own
/// names standard input still, and it is checked.
#[test]
#[cfg(target_os = "linux")]
fn test_a_checkfile_on_a_pipe_cannot_check_that_pipe() {
    blake3::initialize();
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("f"), b"foo").unwrap();
    let foo = blake3::convenience::hash(b"foo").to_hex();
    for (checkfile, entry) in
        [(None, "/dev/stdin"), (None, "/dev/fd/0"), (Some("/dev/stdin"), "-"), (Some("/dev/stdin"), "/dev/fd/0"), (Some("/dev/fd/0"), "/dev/stdin")]
    {
        let list = format!("{foo}  {entry}\n{foo}  f\n");
        let args: Vec<&str> = ["--check"].into_iter().chain(checkfile).collect();
        let out = cmd(b3sum_exe(), &args).stdin_bytes(list).dir(dir.path()).stdout_capture().stderr_capture().unchecked().run().unwrap();
        assert_eq!(out.status.code(), Some(1), "{args:?} {entry}");
        let expected = format!("{entry}: FAILED (standard input is the checkfile; it cannot also be a file to check)\nf: OK\n");
        assert_eq!(String::from_utf8(out.stdout).unwrap(), expected, "{args:?} {entry}");
    }
    // A named checkfile's entries that name the stream take it in the
    // order of their lines, as operands do: the list that `b3sum - NAME`
    // or `b3sum NAME -` wrote checks, on the same input.
    for names in [["-", "/dev/stdin"], ["/dev/stdin", "-"], ["/dev/stdin", "/dev/fd/0"]] {
        let sums = cmd!(b3sum_exe(), names[0], names[1], "f").stdin_bytes("foo").dir(dir.path()).read().unwrap();
        fs::write(dir.path().join("sums"), format!("{sums}\n")).unwrap();
        let out = cmd!(b3sum_exe(), "--check", "sums").stdin_bytes("foo").dir(dir.path()).read().unwrap();
        assert_eq!(out, format!("{}: OK\n{}: OK\nf: OK", names[0], names[1]), "{sums}");
    }
    // A checkfile that is a pipe named `-` in the directory still checks
    // standard input by the entry `-`.
    let dash = dir.path().join("-");
    assert!(std::process::Command::new("mkfifo").arg(&dash).status().unwrap().success());
    let writer = std::thread::spawn({
        let (dash, list) = (dash.clone(), format!("{foo}  -\n"));
        move || fs::write(dash, list).unwrap()
    });
    let out = cmd!(b3sum_exe(), "--check", "./-").stdin_bytes("foo").dir(dir.path()).read().unwrap();
    writer.join().unwrap();
    assert_eq!(out, "-: OK");
    fs::remove_file(&dash).unwrap();
    // A checkfile that is another pipe cannot check that pipe either.
    let fifo = dir.path().join("fifo");
    assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
    let list = format!("{foo}  {}\n{foo}  f\n", fifo.display());
    let writer = std::thread::spawn({
        let (fifo, list) = (fifo.clone(), list.clone());
        move || fs::write(fifo, list).unwrap()
    });
    let out = cmd!(b3sum_exe(), "--check", &fifo).dir(dir.path()).stdout_capture().stderr_capture().unchecked().run().unwrap();
    writer.join().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("{}: FAILED (this checkfile is a stream, and cannot also be a file to check)\nf: OK\n", fifo.display())
    );
    fs::write(dir.path().join("sums"), format!("{foo}  -\n{foo}  f\n")).unwrap();
    let out = cmd!(b3sum_exe(), "--check", "sums").stdin_bytes("foo").dir(dir.path()).read().unwrap();
    assert_eq!(out, "-: OK\nf: OK");
}

/// A checkfile read from standard input that names standard input as a
/// file to check: the entry fails, and the others are checked. (b3sum
/// waited for ever there, for the lock on standard input it held.)
#[test]
fn test_a_checkfile_on_stdin_cannot_check_stdin() {
    blake3::initialize();
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("f"), b"foo").unwrap();
    let foo = blake3::convenience::hash(b"foo").to_hex();
    let checkfile = format!("{foo}  -\n{foo}  f\n");
    for args in [&["--check"][..], &["--check", "-"]] {
        let out = cmd(b3sum_exe(), args).stdin_bytes(checkfile.as_bytes()).dir(dir.path()).stdout_capture().stderr_capture().unchecked().run().unwrap();
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        let stdout = String::from_utf8(out.stdout).unwrap();
        assert!(stdout.starts_with("-: FAILED (standard input is the checkfile"), "{stdout}");
        assert!(stdout.ends_with("f: OK\n"), "{stdout}");
    }
}

/// Relative spellings of `/dev/stdin` (e.g. `stdin` with cwd `/dev`) are
/// recognized by the unified classifier in both operand hashing and `--check`.
#[test]
#[cfg(unix)]
fn test_relative_dev_stdin_alias_and_checkfile_order() {
    blake3::initialize();
    let payload: Vec<u8> = (0..128 * 1024usize).map(|i| (i.wrapping_mul(13) ^ (i >> 5)) as u8).collect();
    let xof33 = |input: &[u8]| {
        let mut h = blake3::convenience::Hasher::new(blake3::Mode::Plain, None);
        h.update(input);
        let mut out = [0u8; 33];
        h.finalize_xof().fill(&mut out);
        hex_encode(out)
    };
    let whole33 = xof33(&payload);
    let empty33 = xof33(b"");
    for (first, second) in [("-", "stdin"), ("stdin", "-")] {
        for _ in 0..3 {
            let out = cmd!(b3sum_exe(), "--length=33", "--num-threads=2", first, second).dir("/dev").stdin_bytes(payload.clone()).read().unwrap();
            assert_eq!(out, format!("{whole33}  {first}\n{empty33}  {second}"), "order ({first}, {second})");
        }
    }
    // Same in `--check`: first line gets the pipe payload, second gets empty.
    let dir = tempfile::tempdir().unwrap();
    let whole = blake3::convenience::hash(b"hello-pipe").to_hex();
    let empty = blake3::convenience::hash(b"").to_hex();
    let manifest = dir.path().join("sums.b3");
    fs::write(&manifest, format!("{whole}  -\n{empty}  stdin\n")).unwrap();
    let checked = cmd!(b3sum_exe(), "--check", &manifest).dir("/dev").stdin_bytes("hello-pipe").read().unwrap();
    assert_eq!(checked, "-: OK\nstdin: OK");
}

/// On both Linux and macOS, `/dev/stdin` and `/dev/fd/0` pointing at a regular
/// file hash the full file independently from byte 0 without corrupting fd 0's
/// shared offset (catching Darwin `fd(4)` F_DUPFD offset sharing).
#[test]
#[cfg(unix)]
fn test_regular_file_stdin_aliases_work_independently_on_unix() {
    use std::io::{Seek, SeekFrom};
    blake3::initialize();
    let dir = tempfile::tempdir().unwrap();
    for len in [1usize, 37, 600_000] {
        let data: Vec<u8> = (0..len).map(|i| (i.wrapping_mul(41) ^ (i >> 6)) as u8).collect();
        let path = dir.path().join(format!("reg-{len}"));
        fs::write(&path, &data).unwrap();
        let want = blake3::convenience::hash(&data).to_hex();
        for threads in ["1", "2"] {
            // Two descriptor aliases: both hash the full file, and fd 0 offset stays 0.
            let mut file = fs::File::open(&path).unwrap();
            let out = std::process::Command::new(b3sum_exe())
                .args(["--num-threads", threads, "/dev/stdin", "/dev/fd/0"])
                .stdin(file.try_clone().unwrap())
                .output()
                .unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
            assert_eq!(String::from_utf8(out.stdout).unwrap(), format!("{want}  /dev/stdin\n{want}  /dev/fd/0\n"), "len={len} t={threads}");
            assert_eq!(file.stream_position().unwrap(), 0, "fd 0 offset must stay untouched");

            // Nonzero initial offset on fd 0: `/dev/stdin` and `/dev/fd/0` still hash from byte 0
            // and preserve the parent's nonzero cursor.
            if len > 10 {
                let mut file = fs::File::open(&path).unwrap();
                file.seek(SeekFrom::Start(5)).unwrap();
                let out = std::process::Command::new(b3sum_exe())
                    .args(["--num-threads", threads, "/dev/stdin", "/dev/fd/0"])
                    .stdin(file.try_clone().unwrap())
                    .output()
                    .unwrap();
                assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
                assert_eq!(String::from_utf8(out.stdout).unwrap(), format!("{want}  /dev/stdin\n{want}  /dev/fd/0\n"));
                assert_eq!(file.stream_position().unwrap(), 5);
            }

            // `-` paired with `/dev/stdin` in both orders: both print the file's digest,
            // and `-` advances fd 0 to EOF.
            for (a, b) in [("-", "/dev/stdin"), ("/dev/stdin", "-")] {
                let mut file = fs::File::open(&path).unwrap();
                let out = std::process::Command::new(b3sum_exe()).args(["--num-threads", threads, a, b]).stdin(file.try_clone().unwrap()).output().unwrap();
                assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
                assert_eq!(String::from_utf8(out.stdout).unwrap(), format!("{want}  {a}\n{want}  {b}\n"), "len={len} ({a},{b})");
                assert_eq!(file.stream_position().unwrap(), len as u64);
            }
        }
        // `--check` naming both `/dev/stdin` and `/dev/fd/0`.
        let sums = dir.path().join(format!("sums-{len}"));
        fs::write(&sums, format!("{want}  /dev/stdin\n{want}  /dev/fd/0\n")).unwrap();
        let mut file = fs::File::open(&path).unwrap();
        let out = std::process::Command::new(b3sum_exe()).args(["--check", sums.to_str().unwrap()]).stdin(file.try_clone().unwrap()).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8(out.stdout).unwrap(), "/dev/stdin: OK\n/dev/fd/0: OK\n");
        assert_eq!(file.stream_position().unwrap(), 0);
    }
}

/// Inherited Unix sockets and named FIFOs on stdin work with `/dev/stdin`
/// aliases because b3sum reuses fd 0 instead of reopening `/dev/stdin`
/// (avoiding Linux `ENXIO` on sockets and blocking reopen on closed FIFOs).
#[test]
#[cfg(unix)]
fn test_unix_socket_and_named_fifo_stdin_aliases() {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    blake3::initialize();
    let payload = b"socket-and-fifo-stream-payload";
    let whole = blake3::convenience::hash(payload).to_hex();
    let empty = blake3::convenience::hash(b"").to_hex();

    // 1. Connected Unix stream socket on stdin.
    for (first, second) in [("-", "/dev/stdin"), ("/dev/stdin", "-")] {
        let (mut writer, reader) = UnixStream::pair().unwrap();
        writer.write_all(payload).unwrap();
        writer.shutdown(std::net::Shutdown::Write).unwrap();
        drop(writer);
        let out = std::process::Command::new(b3sum_exe())
            .args([first, second])
            .stdin(std::process::Stdio::from(std::os::fd::OwnedFd::from(reader)))
            .output()
            .unwrap();
        assert!(out.status.success(), "socket ({first}, {second}): {}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8(out.stdout).unwrap(), format!("{whole}  {first}\n{empty}  {second}\n"));
    }

    // 2. Named FIFO on stdin with a single finite writer that closes.
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("in.fifo");
    assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
    let writer = std::thread::spawn({
        let fifo = fifo.clone();
        move || {
            let mut f = fs::OpenOptions::new().write(true).open(&fifo).unwrap();
            f.write_all(payload).unwrap();
        }
    });
    let reader = fs::File::open(&fifo).unwrap();
    let out = std::process::Command::new(b3sum_exe()).args(["-", "/dev/stdin"]).stdin(reader).output().unwrap();
    writer.join().unwrap();
    assert!(out.status.success(), "fifo: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8(out.stdout).unwrap(), format!("{whole}  -\n{empty}  /dev/stdin\n"));
}

/// Unwritable stderr (`/dev/full`) must never turn normal status-1 failures
/// (missing operand, `--check` mismatch) into an abort via `eprintln!` panic.
#[test]
#[cfg(target_os = "linux")]
fn test_stderr_full_does_not_abort_normal_failures() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("f");
    fs::write(&f, b"actual").unwrap();
    let sums = dir.path().join("sums.b3");
    fs::write(&sums, format!("{}  {}\n", "0".repeat(64), f.display())).unwrap();

    for args in [vec!["no-such-file".to_string()], vec!["--check".to_string(), sums.to_string_lossy().into_owned()]] {
        let full = fs::OpenOptions::new().write(true).open("/dev/full").unwrap();
        let out = std::process::Command::new(b3sum_exe()).args(&args).current_dir(dir.path()).stderr(full).output().unwrap();
        assert_eq!(out.status.code(), Some(1), "expected clean exit 1, not abort, for {args:?}");
    }
}

/// Extreme `--num-threads` values (e.g. `usize::MAX`) are capped cleanly as an
/// upper bound rather than panicking/aborting in thread-pool allocation.
#[test]
fn test_extreme_num_threads_is_capped_not_aborted() {
    blake3::initialize();
    let dir = tempfile::tempdir().unwrap();
    let big = dir.path().join("big.bin");
    let data = vec![0x5Au8; 4_500_000];
    fs::write(&big, &data).unwrap();
    let want = blake3::convenience::hash(&data).to_hex();
    let max = usize::MAX.to_string();
    let out = cmd!(b3sum_exe(), "--num-threads", &max, "--no-names", &big).read().unwrap();
    assert_eq!(out, want.as_str());
}

/// A tight `RLIMIT_NOFILE` budget still makes forward progress through the
/// Linux `io_uring` small-file batcher instead of failing queued opens with
/// `EMFILE`, both with `--num-threads=1` and with default multi-worker mode.
#[test]
#[cfg(target_os = "linux")]
fn test_low_nofile_budget_makes_progress_through_ring() {
    blake3::initialize();
    let dir = tempfile::tempdir().unwrap();
    let mut names = Vec::new();
    let mut expected = String::new();
    for i in 0..32 {
        let name = format!("f{i}");
        let body = format!("file-body-{i}");
        fs::write(dir.path().join(&name), &body).unwrap();
        expected.push_str(&format!("{}  {name}\n", blake3::convenience::hash(body.as_bytes()).to_hex()));
        names.push(name);
    }
    let files_arg = names.join(" ");
    for extra_flag in ["--num-threads=1", ""] {
        let script = format!("exec 3<&- 4<&- 5<&- 6<&- 7<&- 8<&- 9<&-; ulimit -n 8; exec '{}' --no-mmap {extra_flag} {files_arg}", b3sum_exe().display());
        let out = cmd!("sh", "-c", script).dir(dir.path()).stdout_capture().stderr_capture().unchecked().run().unwrap();
        assert!(out.status.success(), "flag={extra_flag:?} stderr={}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8(out.stdout).unwrap(), expected, "flag={extra_flag:?}");
    }
}

/// Lexical path similarity to `/dev/stdin` must never override OS path
/// resolution: `/dev/stdin/` fails as not-a-directory, and a symlink traversal
/// whose lexical form reduces to `/dev/stdin` still hashes the OS target file.
#[test]
#[cfg(unix)]
fn test_lexical_dev_stdin_does_not_override_os_stat() {
    blake3::initialize();

    // 1. Trailing slash on `/dev/stdin/` must fail (not silently read stdin).
    let out = cmd!(b3sum_exe(), "/dev/stdin/").stdin_bytes("from-stdin").stdout_capture().stderr_capture().unchecked().run().unwrap();
    assert_eq!(out.status.code(), Some(1), "trailing slash on /dev/stdin/ must fail");
    assert!(out.stdout.is_empty(), "stdout={:?}", String::from_utf8_lossy(&out.stdout));

    // 2. Linux/Unix `/dev/fd/7/../../stdin` where fd 7 is a directory two levels
    //    deep: lexical `..` collapses to `/dev/stdin`, while the OS resolves
    //    through fd 7 to the regular file `dir/stdin`. Must hash the real file!
    #[cfg(target_os = "linux")]
    {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("a").join("b");
        fs::create_dir_all(&sub).unwrap();
        let real_file = dir.path().join("stdin");
        let real_body = b"real-file-not-stdin";
        fs::write(&real_file, real_body).unwrap();
        let script = format!("exec 7<'{}'; exec '{}' --no-names /dev/fd/7/../../stdin", sub.display(), b3sum_exe().display());
        let out = cmd!("sh", "-c", script).stdin_bytes("from-stdin").stdout_capture().stderr_capture().unchecked().run().unwrap();
        assert!(out.status.success(), "stderr={}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8(out.stdout).unwrap().trim(), blake3::convenience::hash(real_body).to_hex().as_str());
    }
    let bad = cmd!(b3sum_exe(), "/dev/fd/0/../../stdin").stdin_bytes("from-stdin").stdout_capture().stderr_capture().unchecked().run().unwrap();
    assert_eq!(bad.status.code(), Some(1), "/dev/fd/0/../../stdin must fail as ENOTDIR, not read stdin");
    assert!(bad.stdout.is_empty(), "stdout={:?}", String::from_utf8_lossy(&bad.stdout));

    // And with `cwd = /`, relative `dev/stdin` on a connected Unix socket
    // resolves to stdin and succeeds without ENXIO.
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    let payload = b"from-root-cwd-socket";
    let (mut writer, reader) = UnixStream::pair().unwrap();
    writer.write_all(payload).unwrap();
    writer.shutdown(std::net::Shutdown::Write).unwrap();
    drop(writer);
    let out = std::process::Command::new(b3sum_exe())
        .args(["dev/stdin", "-"])
        .current_dir("/")
        .stdin(std::process::Stdio::from(std::os::fd::OwnedFd::from(reader)))
        .output()
        .unwrap();
    assert!(out.status.success(), "cwd=/ dev/stdin stderr={}", String::from_utf8_lossy(&out.stderr));
    let whole = blake3::convenience::hash(payload).to_hex();
    let empty = blake3::convenience::hash(b"").to_hex();
    assert_eq!(String::from_utf8(out.stdout).unwrap(), format!("{whole}  dev/stdin\n{empty}  -\n"));
}

/// `--check /dev/stdin` and `--check /dev/fd/0` when stdin is a regular
/// manifest file positioned past line 1 must still read the whole manifest from
/// offset 0 and leave the parent's fd 0 offset untouched (including on macOS
/// where opening `/dev/stdin` otherwise shares fd 0's offset via `fd(4)`).
#[test]
#[cfg(unix)]
fn test_check_dev_stdin_regular_manifest_from_nonzero_offset() {
    use std::io::{Seek, SeekFrom};
    blake3::initialize();
    let dir = tempfile::tempdir().unwrap();
    let f1 = dir.path().join("one.bin");
    let f2 = dir.path().join("two.bin");
    fs::write(&f1, b"actual-one").unwrap();
    fs::write(&f2, b"actual-two").unwrap();
    let h2 = blake3::convenience::hash(b"actual-two").to_hex();
    let bad_h1 = "0".repeat(64);
    let line1 = format!("{bad_h1}  one.bin\n");
    let line2 = format!("{h2}  two.bin\n");
    let manifest = dir.path().join("manifest.b3");
    fs::write(&manifest, format!("{line1}{line2}")).unwrap();

    for check_arg in ["/dev/stdin", "/dev/fd/0"] {
        let mut f = fs::File::open(&manifest).unwrap();
        let start_pos = line1.len() as u64;
        f.seek(SeekFrom::Start(start_pos)).unwrap();
        let out = std::process::Command::new(b3sum_exe()).args(["--check", check_arg]).current_dir(dir.path()).stdin(f.try_clone().unwrap()).output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{check_arg}: must catch line-1 mismatch");
        assert_eq!(String::from_utf8(out.stdout).unwrap(), "one.bin: FAILED\ntwo.bin: OK\n", "{check_arg}");
        assert_eq!(f.stream_position().unwrap(), start_pos, "{check_arg}: must not move parent fd 0 offset");
    }
}

/// A 64-byte UTF-8 hash field ending in a 2-byte scalar (63 chars) must be
/// reported as a bad line without aborting `--check`, so later valid entries
/// are still verified.
#[test]
fn test_check_64_byte_non_ascii_hex_does_not_abort() {
    blake3::initialize();
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("good.bin"), b"ok-data").unwrap();
    let good_hash = blake3::convenience::hash(b"ok-data").to_hex();
    let bad_hex = format!("{}é", "0".repeat(62));
    assert_eq!(bad_hex.len(), 64);

    for bad_line in [format!("{bad_hex}  bad.bin\n"), format!("BLAKE3 (bad.bin) = {bad_hex}\n")] {
        let manifest = format!("{bad_line}{good_hash}  good.bin\n");
        let out = cmd!(b3sum_exe(), "--check").dir(dir.path()).stdin_bytes(manifest.into_bytes()).stdout_capture().stderr_capture().unchecked().run().unwrap();
        assert_eq!(out.status.code(), Some(1), "bad line must exit 1, not abort");
        assert_eq!(String::from_utf8(out.stdout).unwrap(), "good.bin: OK\n");
    }
}

#[test]
fn test_raw_conflicts_with_tag() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("f.bin");
    fs::write(&file, b"hello").unwrap();
    for flags in [["--raw", "--tag"], ["--tag", "--raw"]] {
        let out = cmd!(b3sum_exe(), flags[0], flags[1], &file).stdout_capture().stderr_capture().unchecked().run().unwrap();
        assert_eq!(out.status.code(), Some(2), "{flags:?}");
        assert!(out.stdout.is_empty(), "{flags:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("--raw") && stderr.contains("--tag"), "{flags:?}: {stderr}");
    }
}

#[cfg(unix)]
mod pty_helpers {
    use std::ffi::CString;
    use std::fs;
    use std::io::Read as _;
    use std::os::fd::FromRawFd as _;
    use std::os::unix::ffi::OsStrExt as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    use std::path::Path;
    use std::process::Child;

    pub struct ChildGuard(pub Option<Child>);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if let Some(mut c) = self.0.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }

    pub fn make_fifo(path: &Path) {
        let cpath = CString::new(path.as_os_str().as_bytes()).expect("fifo path has no NUL");
        // Sound: `cpath` is a NUL-terminated path string.
        assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0, "mkfifo: {}", std::io::Error::last_os_error());
    }

    /// Allocate a PTY master/slave pair; caller keeps `slave` open until all
    /// expected output lines have been read from `master` (preventing macOS
    /// XNU `ptsclose` teardown from discarding buffered lines on child exit).
    pub fn open_pty() -> (fs::File, fs::File) {
        // Sound: standard POSIX PTY setup sequence on freshly opened master fd.
        let (master, slave_path) = unsafe {
            let fd = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
            assert!(fd >= 0, "posix_openpt: {}", std::io::Error::last_os_error());
            assert_eq!(libc::grantpt(fd), 0, "grantpt");
            assert_eq!(libc::unlockpt(fd), 0, "unlockpt");
            let name = libc::ptsname(fd);
            assert!(!name.is_null(), "ptsname");
            let path = std::ffi::CStr::from_ptr(name).to_str().unwrap().to_owned();
            (fs::File::from_raw_fd(fd), path)
        };
        let slave = fs::OpenOptions::new().read(true).write(true).custom_flags(libc::O_NOCTTY).open(&slave_path).unwrap();
        (master, slave)
    }

    /// Read blocking from `master` until `n` complete lines (`\n`-terminated)
    /// have accumulated in `buf`, returning those lines stripped of `\r\n`.
    /// Relies on nextest's single hang timeout rather than private wall clocks.
    pub fn read_lines(master: &mut fs::File, buf: &mut Vec<u8>, n: usize) -> Vec<String> {
        let mut tmp = [0u8; 512];
        loop {
            if buf.iter().filter(|&&b| b == b'\n').count() >= n {
                let text = String::from_utf8_lossy(buf);
                return text.split('\n').take(n).map(|l| l.trim_end_matches('\r').to_owned()).collect();
            }
            match master.read(&mut tmp) {
                Ok(0) => panic!("unexpected PTY EOF after reading {:?}", String::from_utf8_lossy(buf)),
                Ok(k) => buf.extend_from_slice(&tmp[..k]),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => panic!("PTY read error: {e}"),
            }
        }
    }
}

/// When stdout is a terminal, `b3sum` flushes each completed result line before
/// opening the next operand, so a later writerless FIFO never holds back earlier
/// hashes across single-thread, multi-thread, `--tag`, `--no-names`, and XOF
/// (`--length`) modes (#465).
#[test]
#[cfg(unix)]
fn test_terminal_stdout_flushes_before_opening_blocked_fifo() {
    use std::io::Write as _;
    blake3::initialize();

    let dir = tempfile::tempdir().unwrap();
    let first_bytes = b"immediate-terminal-line";
    let second_bytes = b"second-after-fifo";
    fs::write(dir.path().join("first.txt"), first_bytes).unwrap();
    let fifo = dir.path().join("gate.fifo");
    pty_helpers::make_fifo(&fifo);

    let h1 = blake3::convenience::hash(first_bytes).to_hex().to_string();
    let h2 = blake3::convenience::hash(second_bytes).to_hex().to_string();
    let mut xof1 = [0u8; 64];
    let mut xof2 = [0u8; 64];
    blake3::convenience::Hasher::new(blake3::Mode::Plain, None).update(first_bytes).finalize_xof().fill(&mut xof1);
    blake3::convenience::Hasher::new(blake3::Mode::Plain, None).update(second_bytes).finalize_xof().fill(&mut xof2);
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let x1 = hex(&xof1);
    let x2 = hex(&xof2);

    let cases: &[(&[&str], String, String)] = &[
        (&["--num-threads", "1", "first.txt", "gate.fifo"], format!("{h1}  first.txt"), format!("{h2}  gate.fifo")),
        (&["first.txt", "gate.fifo"], format!("{h1}  first.txt"), format!("{h2}  gate.fifo")),
        (&["--tag", "first.txt", "gate.fifo"], format!("BLAKE3 (first.txt) = {h1}"), format!("BLAKE3 (gate.fifo) = {h2}")),
        (&["--no-names", "first.txt", "gate.fifo"], h1.clone(), h2.clone()),
        (&["--length", "64", "first.txt", "gate.fifo"], format!("{x1}  first.txt"), format!("{x2}  gate.fifo")),
    ];

    for (cli_args, want1, want2) in cases {
        let (mut master, slave_keep) = pty_helpers::open_pty();
        let child_stdout = slave_keep.try_clone().unwrap();
        let mut guard = pty_helpers::ChildGuard(Some(
            std::process::Command::new(b3sum_exe())
                .current_dir(dir.path())
                .args(*cli_args)
                .stdin(std::process::Stdio::null())
                .stdout(child_stdout)
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap(),
        ));

        // No writer has opened `gate.fifo` yet: line 1 must arrive first!
        let mut pty_buf = Vec::new();
        let first = pty_helpers::read_lines(&mut master, &mut pty_buf, 1);
        assert_eq!(first[0], *want1, "case {cli_args:?}: line 1 must flush before opening writerless gate.fifo");

        // Now perform a causal writer-only rendezvous on `gate.fifo`: blocks
        // until the child opens `gate.fifo` for reading, then delivers payload+EOF.
        let mut writer = fs::OpenOptions::new().write(true).open(&fifo).unwrap();
        writer.write_all(second_bytes).unwrap();
        drop(writer);

        // Read line 2 while `slave_keep` is still held open (safe on macOS XNU).
        let both = pty_helpers::read_lines(&mut master, &mut pty_buf, 2);
        assert_eq!(both, vec![want1.clone(), want2.clone()], "case {cli_args:?}");
        drop(slave_keep);
        let status = guard.0.take().unwrap().wait().unwrap();
        assert!(status.success(), "case {cli_args:?}: exited {status:?}");
    }
}

/// Multi-file terminal run with a small file, a large `--no-mmap` file, and a
/// writerless FIFO must flush both earlier files before opening the FIFO even
/// with multiple threads (preventing channel-consumer stalls on a blocking
/// FIFO, #465).
#[test]
#[cfg(unix)]
fn test_terminal_stdout_flushes_multi_file_large_before_blocked_fifo() {
    use std::io::Write as _;
    blake3::initialize();

    let dir = tempfile::tempdir().unwrap();
    let mid_bytes = vec![0x41u8; 64 * 1024];
    let big_bytes = vec![0x5Au8; 2 * 1024 * 1024];
    let fifo_bytes = b"fifo-tail-bytes";
    fs::write(dir.path().join("mid.bin"), &mid_bytes).unwrap();
    fs::write(dir.path().join("big.bin"), &big_bytes).unwrap();
    let fifo = dir.path().join("gate.fifo");
    pty_helpers::make_fifo(&fifo);

    let want_mid = format!("{}  mid.bin", blake3::convenience::hash(&mid_bytes).to_hex());
    let want_big = format!("{}  big.bin", blake3::convenience::hash(&big_bytes).to_hex());
    let want_fifo = format!("{}  gate.fifo", blake3::convenience::hash(fifo_bytes).to_hex());

    let (mut master, slave_keep) = pty_helpers::open_pty();
    let child_stdout = slave_keep.try_clone().unwrap();
    let mut guard = pty_helpers::ChildGuard(Some(
        std::process::Command::new(b3sum_exe())
            .current_dir(dir.path())
            .args(["--num-threads", "2", "--no-mmap", "mid.bin", "big.bin", "gate.fifo"])
            .stdin(std::process::Stdio::null())
            .stdout(child_stdout)
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap(),
    ));

    let mut pty_buf = Vec::new();
    let first_two = pty_helpers::read_lines(&mut master, &mut pty_buf, 2);
    assert_eq!(first_two, vec![want_mid.clone(), want_big.clone()]);

    let mut writer = fs::OpenOptions::new().write(true).open(&fifo).unwrap();
    writer.write_all(fifo_bytes).unwrap();
    drop(writer);

    let all_three = pty_helpers::read_lines(&mut master, &mut pty_buf, 3);
    assert_eq!(all_three, vec![want_mid, want_big, want_fifo]);
    drop(slave_keep);
    let status = guard.0.take().unwrap().wait().unwrap();
    assert!(status.success(), "b3sum exited {status:?}");
}
