#!/usr/bin/env python3
"""Check the sleep announcements on Apple arm64 (#463).

Shuttle checks schedules under SC, not order against an RCpc completion
re-check. Compile the production wait paths with real atomics, then remove
just the fences in scratch copies to check that the probe detects the loss.
No target linker or hardware is needed.
"""
from pathlib import Path
import re
import subprocess
import tempfile


PROBE = """
#![allow(dead_code, unused_imports)]
mod waker_sync {
    pub use core::cell::UnsafeCell;
    pub use core::sync::atomic::{AtomicUsize, Ordering, fence};
    #[inline(always)]
    pub fn spin() { core::hint::spin_loop(); }
    #[inline(always)]
    pub unsafe fn with_mut<T, R>(cell: &UnsafeCell<T>, f: impl FnOnce(&mut T) -> R) -> R {
        f(unsafe { &mut *cell.get() })
    }
}
mod waker;
mod park_sync {
    pub use std::sync::OnceLock;
    pub use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering, fence};
    pub use std::thread;
}
mod park;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};

#[unsafe(no_mangle)]
pub(crate) fn register_then_recheck(
    slot: &waker::WakerSlot,
    cx: &core::task::Context<'_>,
    completed: &core::sync::atomic::AtomicUsize,
) -> usize {
    slot.register(cx);
    completed.load(Ordering::Acquire)
}

#[unsafe(no_mangle)]
pub(crate) fn park_then_recheck(park: &park::Park, worker: usize, published: &AtomicUsize) -> bool {
    park.park_unless(worker, || published.load(Ordering::Acquire) != 0)
}

#[unsafe(no_mangle)]
pub(crate) fn sme2_then_recheck(flag: &AtomicBool, published: &AtomicUsize) -> bool {
    // SME2_ADVERTISEMENT
    published.load(Ordering::Acquire) != 0
}

#[unsafe(no_mangle)]
pub(crate) fn reader_guard_then_load(cell: &AtomicU64, seq: u64, reg: &AtomicPtr<()>) -> *mut () {
    // READER_GUARD_HANDSHAKE
}

#[unsafe(no_mangle)]
pub(crate) fn drain_store_then_load(reg: &AtomicPtr<()>, draining: *mut (), cell: &AtomicU64) -> u64 {
    // DRAIN_HANDSHAKE
}
"""


def compile_probe(sources, directory, target="aarch64-apple-darwin"):
    for name, source in sources.items():
        (directory / name).write_text(source)
    asm = directory / f"probe-{target}.s"
    subprocess.run([
        "rustc", "--edition", "2024", "--crate-type", "lib", "-O", "-C", "panic=abort",
        "--target", target, "--emit", "asm", "-o", str(asm),
        str(directory / "probe.rs"),
    ], check=True)
    return asm.read_text()


def function_body(asm, function, target="aarch64-apple-darwin"):
    prefix = "_" if "apple-darwin" in target else ""
    sym = f"{prefix}{function}"
    alias = re.search(rf"^[ \t]*{re.escape(sym)}[ \t]*=[ \t]*([A-Za-z0-9_.$]+)$", asm, re.M)
    if alias:
        sym = alias.group(1)
    body = asm.split(f"\n{sym}:\n", 1)[1]
    return re.split(r"^[ \t]*\.(?:cfi_endproc|globl|subsections_via_symbols|size)\b", body, maxsplit=1, flags=re.M)[0]


def ordered(asm, function, announce):
    body = function_body(asm, function)
    # Fail rather than credit a different instruction sequence, or a
    # barrier belonging to another function.
    stores = list(re.finditer(rf"^[ \t]+{announce}\b[^\n]*\n", body, re.M))
    loads = list(re.finditer(r"^[ \t]+ldapr\b", body, re.M))
    if len(stores) != 1 or len(loads) != 1 or stores[0].end() > loads[0].start():
        raise SystemExit(f"sleep ordering: no unique {announce}/ldapr pair in {function}")
    between = body[stores[0].end():loads[0].start()]
    return re.match(r"[ \t]+dmb[ \t]+ish[ \t]*\n", between) is not None


def ordered_cfg(asm, function, announce):
    """Verify every CFG path from entry to every `ldapr` passes through
    `announce` followed later by `dmb ish` before reaching `ldapr`."""
    body = function_body(asm, function)
    blocks = []
    cur_label = "__entry__"
    cur_insns = []
    for raw in body.splitlines():
        line = raw.strip()
        if not line or line.startswith(".") or line.startswith(";"):
            continue
        m_lbl = re.match(r"^([A-Za-z0-9_.$]+):$", line)
        if m_lbl:
            blocks.append((cur_label, cur_insns))
            cur_label = m_lbl.group(1)
            cur_insns = []
        else:
            cur_insns.append(line)
    blocks.append((cur_label, cur_insns))
    idx_of = {lbl: i for i, (lbl, _) in enumerate(blocks)}

    # States: 0 = no announce yet, 1 = announced (no dmb ish since), 2 = announced + dmb ish.
    seen_ldapr = 0
    seen_fenced = 0
    visited = set()
    work = [(0, 0)]
    while work:
        bi, state = work.pop()
        if (bi, state) in visited:
            continue
        visited.add((bi, state))
        _, insns = blocks[bi]
        s = state
        fallthrough = True
        for insn in insns:
            op = insn.split()[0]
            if op == announce:
                s = 1
            elif re.match(r"^dmb\s+ish$", insn):
                if s == 1:
                    s = 2
                    seen_fenced += 1
            elif op == "ldapr":
                seen_ldapr += 1
                if s != 2:
                    return False
            elif op == "ret":
                fallthrough = False
                break
            elif op == "b":
                target = insn.split()[1]
                if target in idx_of:
                    work.append((idx_of[target], s))
                fallthrough = False
                break
            elif op.startswith("b.") or op in ("cbz", "cbnz", "tbz", "tbnz"):
                target = insn.split(",")[-1].strip()
                if target in idx_of:
                    work.append((idx_of[target], s))
        if fallthrough and bi + 1 < len(blocks):
            work.append((bi + 1, s))
    return seen_ldapr > 0 and seen_fenced > 0


def ordered_x86_store_load(asm, function):
    """Verify on x86-64 that the store-then-load Dekker pair contains a full
    hardware barrier (`xchg*`, `lock`, or `mfence`) before the final load so a
    plain `Release` store without `fence(SeqCst)` fails."""
    body = function_body(asm, function, target="x86_64-unknown-linux-gnu")
    barriers = list(re.finditer(r"^[ \t]+(?:xchg[bwlq]?|lock\b|mfence)\b[^\n]*\n", body, re.M))
    if not barriers:
        return False
    after = body[barriers[0].end():]
    return re.search(r"^[ \t]+mov(?:q|l|b|zb|zw)?\b[^\n]*\(", after, re.M) is not None


def main():
    src = Path(__file__).resolve().parents[1] / "threads/src"
    sources = {name: (src / name).read_text() for name in ("waker.rs", "park.rs")}
    # Reuse the actual SME2 announcement and ReaderGuard/unregister_and_drain
    # Dekker handshakes from lanes.rs, requiring Release/SeqCst store +
    # fence(SeqCst) + Acquire load on both Arm64 and x86-64.
    lanes = (src / "lanes.rs").read_text()
    advertisement = re.search(
        r"(SME2_PARKED\.store\(true, Ordering::SeqCst\);\s+"
        r"core::sync::atomic::fence\(Ordering::SeqCst\);)\s+if \{\s+"
        r"let guard = sources::ReaderGuard::enter\(reader, &mut seq\);\s+"
        r"sources::visible\(&guard, true\)", lanes
    )
    if advertisement is None:
        raise SystemExit("sleep ordering: SME2 must order its announcement before the work re-check")
    guard_enter = re.search(
        r"pub\(crate\) fn enter\(cell: &'a AtomicU64, seq: &mut u64\) -> Self \{[\s\S]*?"
        r"(cell\.store\(\*seq, Ordering::(?:Release|SeqCst)\);\s+core::sync::atomic::fence\(Ordering::SeqCst\);)", lanes
    )
    guard_load = re.search(
        r"fn find_from\(_guard: &ReaderGuard<'_>, start: usize,[\s\S]*?"
        r"let ptr = (REGISTERED\[slot\]\.load\(Ordering::Acquire\));", lanes
    )
    drain_store = re.search(
        r"fn unregister_and_drain\(slot: usize\) \{[\s\S]*?"
        r"(REGISTERED\[slot\]\.store\(DRAINING, Ordering::(?:Release|SeqCst)\);\s+core::sync::atomic::fence\(Ordering::SeqCst\);)", lanes
    )
    drain_load = re.search(
        r"fn unregister_and_drain\(slot: usize\) \{[\s\S]*?"
        r"let initial = (reader\.0\.load\(Ordering::Acquire\));", lanes
    )
    if not all((guard_enter, guard_load, drain_store, drain_load)):
        raise SystemExit("sleep ordering: could not locate fenced ReaderGuard / unregister_and_drain Dekker handshake in lanes.rs")
    reader_body = f"{guard_enter[1].replace('*seq', 'seq')}\n    {guard_load[1].replace('REGISTERED[slot]', 'reg')}"
    drain_body = f"{drain_store[1].replace('REGISTERED[slot]', 'reg').replace('DRAINING', 'draining')}\n    {drain_load[1].replace('reader.0', 'cell')}"
    probe = (
        PROBE.replace("// SME2_ADVERTISEMENT", advertisement[1].replace("SME2_PARKED", "flag"))
        .replace("// READER_GUARD_HANDSHAKE", reader_body)
        .replace("// DRAIN_HANDSHAKE", drain_body)
    )
    paths = {
        "waker.rs": [("register_then_recheck", "stlr", ordered_cfg)],
        "park.rs": [("park_then_recheck", "ldsetal", ordered)],
    }
    with tempfile.TemporaryDirectory(prefix="sleep-order-") as tmp:
        directory = Path(tmp)
        sources["probe.rs"] = probe
        asm = compile_probe(sources, directory, target="aarch64-apple-darwin")
        asm_x86 = compile_probe(sources, directory, target="x86_64-unknown-linux-gnu")
        for name, checks in paths.items():
            fence = "fence(Ordering::SeqCst);"
            if sources[name].count(fence) != 1:
                raise SystemExit(f"sleep ordering: expected one SeqCst fence in {name}")
            unfenced = compile_probe({**sources, name: sources[name].replace(fence, "")}, directory)
            for function, announce, check in checks:
                if not check(asm, function, announce):
                    raise SystemExit(f"sleep ordering: missing dmb ish in {function}")
                if check(unfenced, function, announce):
                    raise SystemExit(f"sleep ordering: negative control retained a barrier in {function}")
                print(f"sleep ordering: {function} has {announce}/dmb ish/ldapr; unfenced control rejected")
        for function, announce, snippet in (
            ("sme2_then_recheck", "stlrb", advertisement[1].replace("SME2_PARKED", "flag")),
            ("reader_guard_then_load", "stlr", reader_body),
            ("drain_store_then_load", "stlr", drain_body),
        ):
            fence = "core::sync::atomic::fence(Ordering::SeqCst);"
            if snippet.count(fence) != 1:
                raise SystemExit(f"sleep ordering: expected one SeqCst fence in {function}")
            if not ordered(asm, function, announce):
                raise SystemExit(f"sleep ordering: missing {announce}/dmb ish/ldapr in {function}")
            unfenced = compile_probe({**sources, "probe.rs": probe.replace(snippet, snippet.replace(fence, ""))}, directory)
            if ordered(unfenced, function, announce):
                raise SystemExit(f"sleep ordering: negative unfenced control still passed {function}")
            print(f"sleep ordering: {function} has {announce}/dmb ish/ldapr; unfenced control rejected")
        for function, snippet in (
            ("reader_guard_then_load", reader_body),
            ("drain_store_then_load", drain_body),
        ):
            fence = "core::sync::atomic::fence(Ordering::SeqCst);"
            if not ordered_x86_store_load(asm_x86, function):
                raise SystemExit(f"sleep ordering: missing x86-64 hardware barrier in {function}")
            unfenced_x86 = compile_probe(
                {**sources, "probe.rs": probe.replace(snippet, snippet.replace(fence, ""))},
                directory,
                target="x86_64-unknown-linux-gnu",
            )
            if ordered_x86_store_load(unfenced_x86, function):
                raise SystemExit(f"sleep ordering: negative x86-64 unfenced control still passed {function}")
            print(f"sleep ordering: {function} has x86-64 hardware store-load barrier; unfenced control rejected")


if __name__ == "__main__":
    main()
