#!/usr/bin/env python3
"""Check the sleep announcements and epoch-registry Dekker handshakes (#463, #465).

Shuttle checks schedules under SC, not order against weaker hardware loads
(Arm RCpc, x86-64 store-buffer reordering). Compile the ACTUAL production
modules (`waker.rs`, `park.rs`, `registry.rs`) plus the `sme2_serve`
announcement with real atomics, then remove or misplace each fence in scratch
copies to prove the probe detects the loss. No target linker or hardware is
needed.
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
mod registry_sync {
    pub use std::sync::atomic::{AtomicPtr, AtomicU64, Ordering, fence};
    #[inline(always)]
    pub(crate) fn spin() { core::hint::spin_loop(); }
    #[inline]
    pub(crate) fn pause_slow() { std::thread::sleep(std::time::Duration::from_micros(50)); }
}
mod registry;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[unsafe(no_mangle)]
pub(crate) fn register_then_recheck(
    slot: &waker::WakerSlot,
    cx: &core::task::Context<'_>,
    completed: &AtomicUsize,
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
pub(crate) fn reader_guard_then_load(
    token: &mut registry::ReaderToken<'_>,
    reg: &registry::EpochRegistry<AtomicUsize, 1>,
) -> bool {
    let guard = token.enter();
    reg.find_from(&guard, 0, |x| x.load(Ordering::Relaxed) != 0).is_some()
}

#[unsafe(no_mangle)]
pub(crate) fn drain_store_then_load(
    reg: &registry::EpochRegistry<AtomicUsize, 1>,
    reader: &registry::ReaderCell,
) {
    reg.unregister_and_drain(0, Some(core::slice::from_ref(reader)));
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
    stores = list(re.finditer(rf"^[ \t]+{announce}\b[^\n]*\n", body, re.M))
    if not stores:
        return False
    after = body[stores[-1].end():]
    barriers = list(re.finditer(r"^[ \t]+dmb[ \t]+ish\b[^\n]*\n", after, re.M))
    if not barriers:
        return False
    after = after[barriers[0].end():]
    return re.search(r"^[ \t]+(?:ldapr|ldar)[bh]?\b", after, re.M) is not None


def ordered_cfg(asm, function, announce):
    """Same announce -> dmb ish -> ldapr/ldar check along control-flow edges."""
    body = function_body(asm, function)
    blocks = []
    label_to_idx = {}
    cur_label = "__entry"
    cur_insns = []
    for raw in body.splitlines():
        line = raw.strip()
        if not line or line.startswith(";"):
            continue
        if line.endswith(":"):
            blocks.append((cur_label, cur_insns))
            label_to_idx[cur_label] = len(blocks) - 1
            cur_label = line[:-1]
            cur_insns = []
        elif not line.startswith("."):
            cur_insns.append(line)
    blocks.append((cur_label, cur_insns))
    label_to_idx[cur_label] = len(blocks) - 1

    def targets(idx):
        _, insns = blocks[idx]
        out = []
        fallthrough = True
        for insn in insns:
            m = re.match(r"^(?:b|b\.[a-z]+|cbz|cbnz|tbz|tbnz)\b.*?\b(LBB\w+)\b", insn)
            if m and m.group(1) in label_to_idx:
                out.append(label_to_idx[m.group(1)])
            if re.match(r"^(?:b\s|ret\b|br\b|brk\b)", insn):
                fallthrough = False
        if fallthrough and idx + 1 < len(blocks):
            out.append(idx + 1)
        return out

    seen = set()
    stack = [(0, 0)]
    while stack:
        idx, stage = stack.pop()
        if (idx, stage) in seen:
            continue
        seen.add((idx, stage))
        cur = stage
        for insn in blocks[idx][1]:
            if cur == 0 and re.match(rf"^{announce}\b", insn):
                cur = 1
            elif cur == 1 and re.match(r"^dmb\s+ish\b", insn):
                cur = 2
            elif cur == 2 and re.match(r"^(?:ldapr|ldar)[bh]?\b", insn):
                return True
        for nxt in targets(idx):
            stack.append((nxt, cur))
    return False


def ordered_x86_store_load(asm, function):
    """Verify on x86-64 that the Dekker pair emits:
      1. a memory-destination store (or `xchg*` to memory),
      2. a full hardware barrier (`lock`/`xchg*`/`mfence`) ordered after/with it, and
      3. a subsequent memory-source load from the second argument (`%rsi`)
         with no intervening memory store after the barrier.
    Weakening to a plain `Release` store, deleting the barrier, moving the
    barrier before the store, or emitting a barrier+store with no load all fail."""
    body = function_body(asm, function, target="x86_64-unknown-linux-gnu")
    insns = [
        line.strip()
        for line in body.splitlines()
        if line.strip() and not line.strip().startswith(".") and not line.strip().endswith(":")
    ]
    state = 0
    for insn in insns:
        if re.match(r"^xchg[bwlq]?\b.*,\s*[^,]*\(", insn):
            state = max(state, 2)
        elif re.match(r"^mov(?:q|l|b|w)?\b[^,]*,\s*[^,]*\(", insn):
            # Any store puts a fresh buffered write ahead of the partner load,
            # requiring a barrier after THIS store (so a fence placed before
            # the store resets back to state 1).
            state = 1
        elif re.match(r"^(?:lock\b|mfence\b)", insn):
            if state == 1:
                state = 2
        elif re.match(r"^mov(?:q|l|b|w|zbq|zwq|zbl|zwl)?\b[^,]*\(%rsi\)\s*,\s*%", insn):
            if state == 2:
                return True
    return False


def replace_nth(text, old, new, nth):
    parts = text.split(old)
    if len(parts) <= nth + 1:
        raise SystemExit(f"sleep ordering: expected occurrence #{nth} of {old!r}")
    return old.join(parts[:nth + 1]) + new + old.join(parts[nth + 1:])


def main():
    src = Path(__file__).resolve().parents[1] / "threads/src"
    sources = {name: (src / name).read_text() for name in ("waker.rs", "park.rs", "registry.rs")}
    lanes = (src / "lanes.rs").read_text()
    advertisement = re.search(
        r"(SME2_PARKED\.store\(true, Ordering::SeqCst\);\s+"
        r"core::sync::atomic::fence\(Ordering::SeqCst\);)\s+if sources::any_active\(\) && \{\s+"
        r"let guard = token\.enter\(\);\s+"
        r"sources::visible\(&guard, true\)", lanes
    )
    if advertisement is None:
        raise SystemExit("sleep ordering: SME2 must order its announcement before the work re-check")
    probe = PROBE.replace("// SME2_ADVERTISEMENT", advertisement[1].replace("SME2_PARKED", "flag"))
    paths = {
        "waker.rs": [("register_then_recheck", "stlr", ordered_cfg)],
        "park.rs": [("park_then_recheck", "ldsetal", ordered)],
    }
    fence = "fence(Ordering::SeqCst);"
    if sources["registry.rs"].count(fence) != 2:
        raise SystemExit("sleep ordering: expected two SeqCst fences in registry.rs (enter and drain_readers)")

    with tempfile.TemporaryDirectory(prefix="sleep-order-") as tmp:
        directory = Path(tmp)
        sources["probe.rs"] = probe
        asm = compile_probe(sources, directory, target="aarch64-apple-darwin")
        asm_x86 = compile_probe(sources, directory, target="x86_64-unknown-linux-gnu")

        for name, checks in paths.items():
            if sources[name].count(fence) != 1:
                raise SystemExit(f"sleep ordering: expected one SeqCst fence in {name}")
            unfenced = compile_probe({**sources, name: sources[name].replace(fence, "")}, directory)
            for function, announce, check in checks:
                if not check(asm, function, announce):
                    raise SystemExit(f"sleep ordering: missing dmb ish in {function}")
                if check(unfenced, function, announce):
                    raise SystemExit(f"sleep ordering: negative control retained a barrier in {function}")
                print(f"sleep ordering: {function} has {announce}/dmb ish/ldapr; unfenced control rejected")

        sme2_snippet = advertisement[1].replace("SME2_PARKED", "flag")
        sme2_fence = "core::sync::atomic::fence(Ordering::SeqCst);"
        if not ordered(asm, "sme2_then_recheck", "stlrb"):
            raise SystemExit("sleep ordering: missing stlrb/dmb ish/ldapr in sme2_then_recheck")
        unfenced_sme2 = compile_probe({**sources, "probe.rs": probe.replace(sme2_snippet, sme2_snippet.replace(sme2_fence, ""))}, directory)
        if ordered(unfenced_sme2, "sme2_then_recheck", "stlrb"):
            raise SystemExit("sleep ordering: negative unfenced control still passed sme2_then_recheck")
        print("sleep ordering: sme2_then_recheck has stlrb/dmb ish/ldapr; unfenced control rejected")

        # Direct compilation of production registry.rs on Arm64 and x86-64,
        # with individual fence removal and misplaced-fence negative controls.
        reg_checks = (
            ("reader_guard_then_load", 0, "self.cell.store(self.seq, Ordering::Release);"),
            ("drain_store_then_load", 1, "self.slots[slot].store(Self::draining(), Ordering::Release);"),
        )
        for function, fence_idx, store_stmt in reg_checks:
            reg_src = sources["registry.rs"]
            if not ordered_cfg(asm, function, "stlr"):
                raise SystemExit(f"sleep ordering: missing stlr/dmb ish/ldapr in {function}")
            unfenced_reg = replace_nth(reg_src, fence, "", fence_idx)
            unfenced_arm = compile_probe({**sources, "registry.rs": unfenced_reg}, directory)
            if ordered_cfg(unfenced_arm, function, "stlr"):
                raise SystemExit(f"sleep ordering: negative arm64 unfenced control still passed {function}")
            print(f"sleep ordering: {function} has stlr/dmb ish/ldapr (registry.rs); unfenced control rejected")

            if not ordered_x86_store_load(asm_x86, function):
                raise SystemExit(f"sleep ordering: missing x86-64 store/barrier/load in {function}")
            unfenced_x86 = compile_probe({**sources, "registry.rs": unfenced_reg}, directory, target="x86_64-unknown-linux-gnu")
            if ordered_x86_store_load(unfenced_x86, function):
                raise SystemExit(f"sleep ordering: negative x86-64 unfenced control still passed {function}")
            # Misplaced barrier control: move fence BEFORE the store.
            if store_stmt not in unfenced_reg:
                raise SystemExit(f"sleep ordering: could not locate {store_stmt!r} in registry.rs")
            misplaced_reg = unfenced_reg.replace(store_stmt, f"{fence}\n        {store_stmt}", 1)
            misplaced_x86 = compile_probe({**sources, "registry.rs": misplaced_reg}, directory, target="x86_64-unknown-linux-gnu")
            if ordered_x86_store_load(misplaced_x86, function):
                raise SystemExit(f"sleep ordering: negative x86-64 misplaced-barrier control still passed {function}")
            print(f"sleep ordering: {function} has x86-64 store/barrier/load (registry.rs); unfenced and misplaced-barrier controls rejected")


if __name__ == "__main__":
    main()
