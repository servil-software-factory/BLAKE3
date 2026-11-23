//! Special-purpose command-line parser for `b3sum`.
//!
//! Replaces `clap` and its ~28 transitive crates (including all proc-macros)
//! while preserving every flag, short option, conflict, requirement, `--`
//! separator, non-UTF-8 path, `--help`, and `--version`.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

pub(crate) const HELP_TEXT: &str = "\
Usage: b3sum [OPTIONS] [FILE]...

Arguments:
  [FILE]...  Files to hash, or checkfiles to check

Options:
      --keyed                 Use the keyed mode, reading the 32-byte key from stdin
      --derive-key <CONTEXT>  Use the key derivation mode, with the given context string
  -l, --length <LEN>          The number of output bytes, before hex encoding [default: 32]
      --seek <SEEK>           The starting output byte offset, before hex encoding
                              [default: 0]
      --num-threads <NUM>     The most threads hashing at once [default: as fast as b3sum can]
      --no-mmap               Read every file and map none: a file that another program shortens
                              while b3sum hashes it then cannot stop the run
      --no-names              Omit filenames in the output
      --raw                   Write raw output bytes to stdout, rather than hex
      --tag                   Output BSD-style checksums: BLAKE3 ([FILE]) = [HASH]
  -c, --check                 Read BLAKE3 sums from the [FILE]s and check them
      --quiet                 Skip printing OK for each checked file
      --outboard <DIR>        Also write each file's outboard to DIR/<HASH>.obao4: its BLAKE3 tree
                              over 16 KiB chunk groups in bao-tree's pre-order form, for verified
                              streaming
  -h, --help                  Print help
  -V, --version               Print version

More information: b3sum/README.md at
https://github.com/servil-software-factory/BLAKE3/blob/main/b3sum/README.md
";

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParsedArgs {
    pub(crate) file: Vec<PathBuf>,
    pub(crate) keyed: bool,
    pub(crate) derive_key: Option<String>,
    pub(crate) length: u64,
    pub(crate) seek: u64,
    pub(crate) num_threads: Option<std::num::NonZeroUsize>,
    pub(crate) no_mmap: bool,
    pub(crate) no_names: bool,
    pub(crate) raw: bool,
    pub(crate) tag: bool,
    pub(crate) check: bool,
    pub(crate) quiet: bool,
    pub(crate) outboard: Option<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ParseOutcome {
    Parsed(ParsedArgs),
    Help,
    Version,
}

fn require_utf8(flag: &str, val: &OsStr) -> Result<String, String> {
    val.to_str().map(str::to_owned).ok_or_else(|| format!("invalid UTF-8 for '{flag}'"))
}

fn parse_u64(flag: &str, val: &OsStr) -> Result<u64, String> {
    let s = require_utf8(flag, val)?;
    s.parse::<u64>().map_err(|e| format!("invalid value '{s}' for '{flag}': {e}"))
}

fn parse_nonzero_usize(flag: &str, val: &OsStr) -> Result<std::num::NonZeroUsize, String> {
    let s = require_utf8(flag, val)?;
    s.parse::<std::num::NonZeroUsize>().map_err(|e| format!("invalid value '{s}' for '{flag}': {e}"))
}

fn split_long_eq(arg: &OsStr) -> Option<(&str, OsString)> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let bytes = arg.as_bytes();
        let eq = bytes.iter().position(|&b| b == b'=')?;
        let name = std::str::from_utf8(&bytes[..eq]).ok()?;
        let val = OsString::from_vec(bytes[eq + 1..].to_vec());
        Some((name, val))
    }
    #[cfg(not(unix))]
    {
        let s = arg.to_str()?;
        let (name, val) = s.split_once('=')?;
        Some((name, OsString::from(val)))
    }
}

fn reject_duplicate(flag: &str, seen: bool) -> Result<(), String> {
    if seen { Err(format!("the argument '{flag}' cannot be used multiple times")) } else { Ok(()) }
}

/// Option-shaped tokens (`--` or `-...` longer than 1 byte) are rejected as
/// separate option values, so a missing value before another flag (like
/// `--derive-key --check sums`) fails with exit 2 instead of silently eating
/// `--check`. Inline `--opt=-val` and bare `-` stay accepted.
fn is_option_like(val: &OsStr) -> bool {
    let bytes = val.as_encoded_bytes();
    bytes.len() > 1 && bytes[0] == b'-'
}

fn push_positional(file: &mut Vec<PathBuf>, arg: OsString) -> Result<(), String> {
    if arg.is_empty() {
        return Err("a value is required for '[FILE]...' but an empty path was supplied".to_owned());
    }
    file.push(PathBuf::from(arg));
    Ok(())
}

pub(crate) fn parse_from<I, T>(iter: I) -> Result<ParseOutcome, String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let mut it = iter.into_iter().map(Into::into);
    let _prog = it.next();

    let mut file: Vec<PathBuf> = Vec::new();
    let mut keyed = false;
    let mut derive_key: Option<String> = None;
    let mut length: Option<u64> = None;
    let mut seek: Option<u64> = None;
    let mut num_threads: Option<std::num::NonZeroUsize> = None;
    let mut no_mmap = false;
    let mut no_names = false;
    let mut raw = false;
    let mut tag = false;
    let mut check = false;
    let mut quiet = false;
    let mut outboard: Option<PathBuf> = None;

    let mut after_double_dash = false;

    while let Some(arg) = it.next() {
        if after_double_dash {
            push_positional(&mut file, arg)?;
            continue;
        }
        if arg == "--" {
            after_double_dash = true;
            continue;
        }
        if arg == "-" {
            file.push(PathBuf::from(arg));
            continue;
        }

        // Check for `--long` or `--long=VAL` (supports non-UTF-8 VAL on Unix, e.g. `--outboard=<bytes>`).
        let is_long = arg.as_encoded_bytes().starts_with(b"--");
        if is_long {
            let (name, inline_val) = if let Some((n, v)) = split_long_eq(&arg) {
                (n.to_owned(), Some(v))
            } else {
                let s = arg.to_str().ok_or_else(|| format!("unexpected argument '{}'", arg.to_string_lossy()))?;
                (s.to_owned(), None)
            };

            let take_val = |flag: &str, inline: Option<OsString>, it: &mut dyn Iterator<Item = OsString>| -> Result<OsString, String> {
                match inline {
                    Some(v) => Ok(v),
                    None => {
                        let v = it.next().ok_or_else(|| format!("a value is required for '{flag}' but none was supplied"))?;
                        if is_option_like(&v) {
                            return Err(format!("a value is required for '{flag}' but none was supplied (found '{}')", v.to_string_lossy()));
                        }
                        Ok(v)
                    }
                }
            };
            let reject_val = |flag: &str, inline: &Option<OsString>| -> Result<(), String> {
                if inline.is_some() { Err(format!("unexpected value for '{flag}'")) } else { Ok(()) }
            };

            match name.as_str() {
                "--help" => {
                    reject_val("--help", &inline_val)?;
                    return Ok(ParseOutcome::Help);
                }
                "--version" => {
                    reject_val("--version", &inline_val)?;
                    return Ok(ParseOutcome::Version);
                }
                "--keyed" => {
                    reject_val("--keyed", &inline_val)?;
                    reject_duplicate("--keyed", keyed)?;
                    keyed = true;
                }
                "--derive-key" => {
                    reject_duplicate("--derive-key", derive_key.is_some())?;
                    let v = take_val("--derive-key", inline_val, &mut it)?;
                    derive_key = Some(require_utf8("--derive-key", &v)?);
                }
                "--length" => {
                    reject_duplicate("--length", length.is_some())?;
                    let v = take_val("--length", inline_val, &mut it)?;
                    length = Some(parse_u64("--length", &v)?);
                }
                "--seek" => {
                    reject_duplicate("--seek", seek.is_some())?;
                    let v = take_val("--seek", inline_val, &mut it)?;
                    seek = Some(parse_u64("--seek", &v)?);
                }
                "--num-threads" => {
                    reject_duplicate("--num-threads", num_threads.is_some())?;
                    let v = take_val("--num-threads", inline_val, &mut it)?;
                    num_threads = Some(parse_nonzero_usize("--num-threads", &v)?);
                }
                "--no-mmap" => {
                    reject_val("--no-mmap", &inline_val)?;
                    reject_duplicate("--no-mmap", no_mmap)?;
                    no_mmap = true;
                }
                "--no-names" => {
                    reject_val("--no-names", &inline_val)?;
                    reject_duplicate("--no-names", no_names)?;
                    no_names = true;
                }
                "--raw" => {
                    reject_val("--raw", &inline_val)?;
                    reject_duplicate("--raw", raw)?;
                    raw = true;
                }
                "--tag" => {
                    reject_val("--tag", &inline_val)?;
                    reject_duplicate("--tag", tag)?;
                    tag = true;
                }
                "--check" => {
                    reject_val("--check", &inline_val)?;
                    reject_duplicate("--check", check)?;
                    check = true;
                }
                "--quiet" => {
                    reject_val("--quiet", &inline_val)?;
                    reject_duplicate("--quiet", quiet)?;
                    quiet = true;
                }
                "--outboard" => {
                    reject_duplicate("--outboard", outboard.is_some())?;
                    let v = take_val("--outboard", inline_val, &mut it)?;
                    if v.is_empty() {
                        return Err("a value is required for '--outboard <DIR>' but none was supplied".to_owned());
                    }
                    outboard = Some(PathBuf::from(v));
                }
                other => return Err(format!("unexpected argument '{other}' found")),
            }
            continue;
        }

        // Short options: `-c`, `-h`, `-V`, `-l <LEN>`, `-l<LEN>`, `-l=<LEN>`, and clusters like `-ch`.
        let is_short = arg.as_encoded_bytes().starts_with(b"-");
        if is_short {
            let s = arg.to_str().ok_or_else(|| format!("unexpected argument '{}'", arg.to_string_lossy()))?;
            let mut chars = s[1..].char_indices();
            while let Some((idx, ch)) = chars.next() {
                match ch {
                    'h' => return Ok(ParseOutcome::Help),
                    'V' => return Ok(ParseOutcome::Version),
                    'c' => {
                        reject_duplicate("--check", check)?;
                        check = true;
                    }
                    'l' => {
                        reject_duplicate("--length", length.is_some())?;
                        let rest = &s[1 + idx + ch.len_utf8()..];
                        let val_os = if !rest.is_empty() {
                            OsString::from(rest.strip_prefix('=').unwrap_or(rest))
                        } else {
                            let v = it.next().ok_or_else(|| "a value is required for '--length <LEN>' but none was supplied".to_owned())?;
                            if is_option_like(&v) {
                                return Err(format!("a value is required for '--length <LEN>' but none was supplied (found '{}')", v.to_string_lossy()));
                            }
                            v
                        };
                        length = Some(parse_u64("--length", &val_os)?);
                        break;
                    }
                    other => return Err(format!("unexpected argument '-{other}' found")),
                }
            }
            continue;
        }

        push_positional(&mut file, arg)?;
    }

    // Requirements and conflicts (identical to the previous clap definition).
    if keyed && file.is_empty() {
        return Err("the following required arguments were not provided:\n  <FILE>...".to_owned());
    }
    if derive_key.is_some() && keyed {
        return Err("the argument '--derive-key <CONTEXT>' cannot be used with '--keyed'".to_owned());
    }
    if check {
        for (present, other) in [
            (derive_key.is_some(), "--derive-key <CONTEXT>"),
            (keyed, "--keyed"),
            (length.is_some(), "--length <LEN>"),
            (raw, "--raw"),
            (tag, "--tag"),
            (no_names, "--no-names"),
            (seek.is_some(), "--seek <SEEK>"),
            (outboard.is_some(), "--outboard <DIR>"),
        ] {
            if present {
                return Err(format!("the argument '--check' cannot be used with '{other}'"));
            }
        }
    }
    if quiet && !check {
        return Err("the following required arguments were not provided:\n  --check".to_owned());
    }
    if outboard.is_some() {
        for (present, other) in
            [(derive_key.is_some(), "--derive-key <CONTEXT>"), (length.is_some(), "--length <LEN>"), (raw, "--raw"), (seek.is_some(), "--seek <SEEK>")]
        {
            if present {
                return Err(format!("the argument '--outboard <DIR>' cannot be used with '{other}'"));
            }
        }
    }

    Ok(ParseOutcome::Parsed(ParsedArgs {
        file,
        keyed,
        derive_key,
        length: length.unwrap_or(blake3::OUT_LEN as u64),
        seek: seek.unwrap_or(0),
        num_threads,
        no_mmap,
        no_names,
        raw,
        tag,
        check,
        quiet,
        outboard,
    }))
}
