use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::PathBuf;

fn is_label_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'$'
}

/// Replace whole-label occurrences of `from` with `to` in `line`, preserving
/// arbitrary UTF-8 substrings outside the match.
fn replace_label(line: &str, from: &str, to: &str) -> String {
    if !line.contains(from) {
        return line.to_owned();
    }
    let bytes = line.as_bytes();
    let fbytes = from.as_bytes();
    let mut out = String::with_capacity(line.len() + 32);
    let mut last = 0;
    let mut i = 0;
    while i + fbytes.len() <= bytes.len() {
        if &bytes[i..i + fbytes.len()] == fbytes {
            let before_ok = i == 0 || !is_label_byte(bytes[i - 1]);
            let after_ok = i + fbytes.len() == bytes.len() || !is_label_byte(bytes[i + fbytes.len()]);
            if before_ok && after_ok {
                out.push_str(&line[last..i]);
                out.push_str(to);
                i += fbytes.len();
                last = i;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&line[last..]);
    out
}

/// Parse a private label definition at the start of a trimmed line, if any.
fn defined_label(trimmed: &str) -> Option<&str> {
    if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') || trimmed.starts_with('#') {
        return None;
    }
    let colon = trimmed.find(':')?;
    let name = trimmed[..colon].trim_end();
    if name.is_empty() || name.as_bytes()[0].is_ascii_digit() {
        return None;
    }
    if name.bytes().all(is_label_byte) { Some(name) } else { None }
}

/// Validate and strip only the exact approved C-preprocessor lines in `c/*.S`,
/// namespace every private label per file (Mach-O `L...` on macOS, ELF `.L...`
/// on Linux) so fat-LTO and Mach-O relocations never collide, emit `endbr64`
/// for `_CET_ENDBR` entry pads, and `.purgem` every `.macro` at file end (#487).
fn preprocess_asm(src_rel: &str, src: &str, is_apple: bool, tag: &str) -> String {
    #[derive(Clone, Copy)]
    enum Cond {
        Keep,
        Drop,
    }

    let mut globals: HashSet<&str> = HashSet::new();
    let mut macros: Vec<&str> = Vec::new();
    let mut filtered: Vec<String> = Vec::with_capacity(src.lines().count() + 16);
    let mut stack: Vec<Cond> = Vec::new();

    for (idx, raw_line) in src.lines().enumerate() {
        let lineno = idx + 1;
        let trimmed = raw_line.trim();
        if trimmed.starts_with('#') {
            match trimmed {
                "#if defined(__ELF__) && defined(__linux__)"
                | "#if defined(__ELF__) && defined(__CET__) && defined(__has_include)"
                | "#if __has_include(<cet.h>)"
                | "#if !defined(_CET_ENDBR)" => {
                    stack.push(Cond::Drop);
                }
                "#ifdef __APPLE__" => {
                    stack.push(if is_apple { Cond::Keep } else { Cond::Drop });
                }
                "#else" => {
                    let top = stack.last_mut().unwrap_or_else(|| panic!("{src_rel}:{lineno}: #else without matching #if"));
                    *top = match *top {
                        Cond::Keep => Cond::Drop,
                        Cond::Drop => Cond::Keep,
                    };
                }
                "#endif" => {
                    stack.pop().unwrap_or_else(|| panic!("{src_rel}:{lineno}: #endif without matching #if"));
                }
                "#include <cet.h>" | "#define _CET_ENDBR" => {}
                other => panic!("{src_rel}:{lineno}: unapproved preprocessor directive in assembly source: {other}"),
            }
            continue;
        }

        if stack.iter().any(|c| matches!(c, Cond::Drop)) {
            continue;
        }
        if trimmed == ".intel_syntax noprefix" || trimmed.starts_with(".section .note.GNU-stack") {
            continue;
        }
        if trimmed == "_CET_ENDBR" {
            filtered.push("        endbr64".to_owned());
            continue;
        }
        if !is_apple && (trimmed == ".text" || trimmed == ".section .text") {
            filtered.push(".section .text.blake3_servil_asm,\"ax\",@progbits".to_owned());
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix(".global ").or_else(|| trimmed.strip_prefix(".globl ")) {
            if let Some(sym) = rest.split_whitespace().next() {
                globals.insert(sym);
            }
        }
        if let Some(rest) = trimmed.strip_prefix(".macro ") {
            if let Some(name) = rest.split(|c: char| c.is_whitespace() || c == ',').next()
                && !name.is_empty()
            {
                macros.push(name);
            }
        }
        filtered.push(raw_line.to_owned());
    }

    assert!(stack.is_empty(), "{src_rel}: unterminated #if block in assembly source");

    // Every kernel file exports paired Mach-O (`_sym`) and ELF (`sym`) labels.
    // Under `core::arch::global_asm!` on macOS, LLVM emits `.subsections_via_symbols`,
    // which makes `ld64` split atoms at every non-`L` symbol: a paired `_sym:` /
    // `sym:` leaves `_sym` as a 0-byte atom that `-Wl,-order_file` or `-dead_strip`
    // separates from the code! Drop the wrong-ABI alias on both platforms, and on
    // Mach-O mark every global label after the first in each file `.alt_entry`
    // (with a trailing `brk #0` sentinel so trailing `*_end` labels stay strictly
    // inside the atom) so fallthrough entries (`k1` -> `c1`, `sme2` -> `.Lchunks_body`)
    // and `*_end` prefetch bounds stay glued inside one atom, matching standalone `cc` (#487).
    let mut drop_globals: HashSet<&str> = HashSet::new();
    for &sym in &globals {
        if is_apple {
            if !sym.starts_with('_') && globals.contains(format!("_{sym}").as_str()) {
                drop_globals.insert(sym);
            }
        } else if let Some(bare) = sym.strip_prefix('_')
            && globals.contains(bare)
        {
            drop_globals.insert(sym);
        }
    }

    // Collect every non-exported label defined in this file so Mach-O local
    // branch targets (`L...`) and ELF fat-LTO private symbols (`.L...`) are
    // both unique across the crate and any downstream consumer.
    let mut private_labels: Vec<String> = Vec::new();
    let mut seen_priv: HashSet<String> = HashSet::new();
    for line in &filtered {
        if let Some(lbl) = defined_label(line.trim())
            && !globals.contains(lbl)
            && seen_priv.insert(lbl.to_owned())
        {
            private_labels.push(lbl.to_owned());
        }
    }
    // Longest first so e.g. `BLAKE3_IV_0` is rewritten before `BLAKE3_IV`.
    private_labels.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));

    let local_prefix = if is_apple { "L_b3s_" } else { ".L_b3s_" };
    let rewrites: Vec<(String, String)> = private_labels
        .into_iter()
        .map(|lbl| {
            let clean = lbl.trim_start_matches('.').replace(['.', '$'], "_");
            let new_lbl = format!("{local_prefix}{tag}_{clean}");
            (lbl, new_lbl)
        })
        .collect();

    let mut out = String::with_capacity(src.len() + 256);
    let mut emitted_first_global = false;
    for mut line in filtered {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(".global ").or_else(|| trimmed.strip_prefix(".globl "))
            && let Some(sym) = rest.split_whitespace().next()
            && drop_globals.contains(sym)
        {
            continue;
        }
        if let Some(lbl) = defined_label(trimmed)
            && trimmed == format!("{lbl}:")
        {
            if drop_globals.contains(lbl) {
                continue;
            }
            if is_apple && globals.contains(lbl) {
                if emitted_first_global {
                    out.push_str(".alt_entry ");
                    out.push_str(lbl);
                    out.push('\n');
                } else {
                    emitted_first_global = true;
                }
            }
        }
        if !rewrites.is_empty() {
            for (from, to) in &rewrites {
                if line.contains(from.as_str()) {
                    line = replace_label(&line, from, to);
                }
            }
        }
        out.push_str(&line);
        out.push('\n');
    }

    if is_apple && emitted_first_global {
        // Keep a trailing `*_end` `.alt_entry` strictly inside the file's atom.
        out.push_str("        brk #0\n");
    }
    for mac in macros {
        out.push_str(".purgem ");
        out.push_str(mac);
        out.push('\n');
    }
    // Always restore `.text` so subsequent module-level `global_asm!` items or
    // codegen units never inherit `.rodata` or `.static_data`.
    out.push_str(".text\n");
    out
}

fn stage_asm(out_dir: &PathBuf, src_rel: &str, out_name: &str, is_apple: bool, tag: &str) {
    println!("cargo::rerun-if-changed={src_rel}");
    let src = fs::read_to_string(src_rel).unwrap_or_else(|e| panic!("failed to read {src_rel}: {e}"));
    let processed = preprocess_asm(src_rel, &src, is_apple, tag);
    fs::write(out_dir.join(out_name), processed).unwrap_or_else(|e| panic!("failed to write {out_name}: {e}"));
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The supported targets, and every other one stopped here, before
    // anything is built: a build elsewhere would run code no machine of
    // ours tests, where official BLAKE3 supports it (#160, #180).
    let var = |name: &str| env::var(name).unwrap_or_default();
    let target = var("TARGET");
    let (arch, os) = (var("CARGO_CFG_TARGET_ARCH"), var("CARGO_CFG_TARGET_OS"));
    // Pointers of 64 bits too: the 64-bit architectures have targets with
    // pointers of 32 (x86_64-unknown-linux-gnux32,
    // aarch64-unknown-linux-gnu_ilp32), where this library's sizes, which
    // are checked for 64 bits, would wrap. Reject arm64e (pointer-auth ABI,
    // untested here) explicitly rather than letting arch == "aarch64" pass it.
    let supported = var("CARGO_CFG_TARGET_ENDIAN") == "little"
        && var("CARGO_CFG_TARGET_POINTER_WIDTH") == "64"
        && !target.starts_with("arm64e-")
        && match arch.as_str() {
            "x86_64" => os == "linux",
            "aarch64" => os == "linux" || os == "macos",
            _ => false,
        };
    assert!(
        supported,
        "blake3-servil runs on x86-64 Linux and on 64-bit Arm Linux and macOS; for {target}, use official BLAKE3 (https://github.com/BLAKE3-team/BLAKE3)",
    );

    // As of Rust 1.80, unrecognized config names are warnings. Give Cargo all of our config names.
    // `shuttle` gates tests/shuttle_*.rs (RUSTFLAGS="--cfg shuttle"); the
    // library compiles the same either way.
    let all_cfgs = ["blake3_sse41_ffi", "blake3_avx2_ffi", "blake3_avx512_ffi", "blake3_neon", "blake3_sme2", "blake3_has_std", "shuttle"];
    for cfg_name in all_cfgs {
        println!("cargo::rustc-check-cfg=cfg({cfg_name}, values(none()))");
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let is_apple = os == "macos";

    if arch == "x86_64" {
        println!("cargo::rustc-cfg=blake3_sse41_ffi");
        println!("cargo::rustc-cfg=blake3_avx2_ffi");
        println!("cargo::rustc-cfg=blake3_avx512_ffi");
        stage_asm(&out_dir, "c/blake3_sse41_x86-64_unix.S", "blake3_sse41.s", false, "sse41");
        stage_asm(&out_dir, "c/blake3_avx2_x86-64_unix.S", "blake3_avx2.s", false, "avx2");
        stage_asm(&out_dir, "c/blake3_partial_x86-64_unix.S", "blake3_partial.s", false, "partial");
        stage_asm(&out_dir, "c/blake3_chunk_root_x86-64_unix.S", "blake3_chunk_root.s", false, "chunk_root");
        stage_asm(&out_dir, "c/blake3_avx512_x86-64_unix.S", "blake3_avx512.s", false, "avx512");
        stage_asm(&out_dir, "c/blake3_avx512_32.S", "blake3_avx512_32.s", false, "avx512_32");
        stage_asm(&out_dir, "c/blake3_packed_x86-64_unix.S", "blake3_packed_x86.s", false, "packed_x86");
        if env::var_os("CARGO_FEATURE_STD").is_some() {
            stage_asm(&out_dir, "c/blake3_output16.S", "blake3_output16.s", false, "output16");
            stage_asm(&out_dir, "c/blake3_output8.S", "blake3_output8.s", false, "output8");
        }
    }

    if arch == "aarch64" {
        println!("cargo::rustc-cfg=blake3_neon");
        println!("cargo::rustc-cfg=blake3_sme2");
        println!("cargo::metadata=sme2=1");
        stage_asm(&out_dir, "c/blake3_neon_hybrid_aarch64.S", "blake3_neon_hybrid.s", is_apple, "neon");
        stage_asm(&out_dir, "c/blake3_packed_neon_aarch64.S", "blake3_packed_neon.s", is_apple, "packed_neon");
        stage_asm(&out_dir, "c/blake3_sme2_aarch64.S", "blake3_sme2.s", is_apple, "sme2");
        stage_asm(&out_dir, "c/blake3_packed_sme2_aarch64.S", "blake3_packed_sme2.s", is_apple, "packed_sme2");
        stage_asm(&out_dir, "c/blake3_sme2_hybrid_aarch64.S", "blake3_sme2_hybrid.s", is_apple, "sme2_hybrid");
    }

    println!("cargo::rerun-if-changed=build.rs");
    Ok(())
}
