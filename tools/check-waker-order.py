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
    pub use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering, fence},
    };
}
mod waker;
mod park_sync {
    pub use std::sync::OnceLock;
    pub use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering, fence};
    pub use std::thread;
}
mod park;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[unsafe(no_mangle)]
pub(crate) fn register_then_recheck(
    slot: &waker::WakerSlot,
    cx: &core::task::Context<'_>,
    completed: &core::sync::atomic::AtomicBool,
) -> bool {
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
"""


def compile_probe(sources, directory):
    for name, source in sources.items():
        (directory / name).write_text(source)
    asm = directory / "probe.s"
    subprocess.run([
        "rustc", "--edition", "2024", "--crate-type", "lib", "-O", "-C", "panic=abort",
        "--target", "aarch64-apple-darwin", "--emit", "asm", "-o", str(asm),
        str(directory / "probe.rs"),
    ], check=True)
    return asm.read_text()


def function_body(asm, function):
    body = asm.split(f"\n_{function}:\n", 1)[1]
    return re.split(r"^[ \t]*\.(?:cfi_endproc|globl|subsections_via_symbols)\b", body, maxsplit=1, flags=re.M)[0]


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


def ordered_cfg(asm, function, announce, load_op="ldapr"):
    """Verify every CFG path from entry to every `load_op` passes through
    `announce` followed later by `dmb ish` before reaching `load_op`.

    Strips trailing `;` assembly comments before tokenizing and fails closed
    on any branch whose target label cannot be resolved inside the function.
    """
    body = function_body(asm, function)
    blocks = []
    cur_label = "__entry__"
    cur_insns = []
    for raw in body.splitlines():
        # Strip Mach-O `;` comments before checking labels or instruction tokens.
        line = raw.split(";", 1)[0].strip()
        if not line or line.startswith("."):
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
    seen_load = 0
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
            elif op == load_op:
                seen_load += 1
                if s != 2:
                    return False
            elif op == "bl" and re.fullmatch(
                r"__?RNvNtCs[A-Za-z0-9]+_4core6result13unwrap_failed",
                insn.split()[1],
            ):
                # Only exact Rust v0-mangled `core::result::unwrap_failed` is
                # treated as `-> !` (noreturn); any other `bl` (even one whose
                # name contains `unwrap_failed`) is assumed to return and fall
                # through to the next instruction.
                fallthrough = False
                break
            elif op == "ret":
                fallthrough = False
                break
            elif op == "b":
                target = insn.split()[1]
                # Fail closed on every unresolved direct jump (including
                # numeric local labels like `2f` or unknown symbols).
                if target not in idx_of:
                    return False
                work.append((idx_of[target], s))
                fallthrough = False
                break
            elif op.startswith("b.") or op in ("cbz", "cbnz", "tbz", "tbnz"):
                target = insn.split(",")[-1].split()[-1].strip()
                if target not in idx_of:
                    return False
                work.append((idx_of[target], s))
            elif op in ("br", "blr"):
                # Reject indirect branches unless they are calls (`blr` returns).
                if op == "br":
                    return False
        if fallthrough and bi + 1 < len(blocks):
            work.append((bi + 1, s))
    return seen_load > 0 and seen_fenced > 0


def check_one_bad_edge_control():
    """Verify `ordered_cfg` rejects a single unfenced branch even with a
    trailing Mach-O `;` comment on the branch line."""
    good_asm = (
        "\n_ctrl:\n"
        "\tcbz\tx0, LBB0_1 ; fast path\n"
        "\tstlrb\tw8, [x0]\n"
        "\tdmb\tish\n"
        "\tb\tLBB0_2 ; join\n"
        "LBB0_1:\n"
        "\tstlrb\tw8, [x0]\n"
        "\tdmb\tish\n"
        "LBB0_2:\n"
        "\tldaprb\tw0, [x1]\n"
        "\tret\n"
        "\t.cfi_endproc\n"
    )
    bad_comment_asm = (
        "\n_ctrl:\n"
        "\tb.eq\tLBB0_2 ; unfenced bypass with comment\n"
        "\tstlrb\tw8, [x0]\n"
        "\tdmb\tish\n"
        "LBB0_2:\n"
        "\tldaprb\tw0, [x1]\n"
        "\tret\n"
        "\t.cfi_endproc\n"
    )
    bad_unknown_asm = (
        "\n_ctrl:\n"
        "\tb.eq\tLBB0_99 ; unresolved target\n"
        "\tstlrb\tw8, [x0]\n"
        "\tdmb\tish\n"
        "\tldaprb\tw0, [x1]\n"
        "\tret\n"
        "\t.cfi_endproc\n"
    )
    bad_unknown_uncond_asm = (
        "\n_ctrl:\n"
        "\tcbz\tx0, LBB0_1\n"
        "\tb\t2f ; unresolved numeric forward jump\n"
        "LBB0_1:\n"
        "\tstlrb\tw8, [x0]\n"
        "\tdmb\tish\n"
        "2:\n"
        "\tldaprb\tw0, [x1]\n"
        "\tret\n"
        "\t.cfi_endproc\n"
    )
    bad_returning_call_asm = (
        "\n_ctrl:\n"
        "\tcbz\tx0, LBB0_1\n"
        "\tbl\t_note_unwrap_failed\n"
        "\tldaprb\tw0, [x1]\n"
        "\tret\n"
        "LBB0_1:\n"
        "\tstlrb\tw8, [x0]\n"
        "\tdmb\tish\n"
        "\tldaprb\tw0, [x1]\n"
        "\tret\n"
        "\t.cfi_endproc\n"
    )
    if not ordered_cfg(good_asm, "ctrl", "stlrb", "ldaprb"):
        raise SystemExit("sleep ordering: ordered_cfg self-test rejected valid commented CFG")
    if ordered_cfg(bad_comment_asm, "ctrl", "stlrb", "ldaprb"):
        raise SystemExit("sleep ordering: ordered_cfg failed to reject unfenced branch with trailing comment")
    if ordered_cfg(bad_unknown_asm, "ctrl", "stlrb", "ldaprb"):
        raise SystemExit("sleep ordering: ordered_cfg failed to reject unresolved branch target")
    if ordered_cfg(bad_unknown_uncond_asm, "ctrl", "stlrb", "ldaprb"):
        raise SystemExit("sleep ordering: ordered_cfg failed to reject unresolved unconditional jump")
    if ordered_cfg(bad_returning_call_asm, "ctrl", "stlrb", "ldaprb"):
        raise SystemExit("sleep ordering: ordered_cfg treated returning *_unwrap_failed call as noreturn")


def main():
    check_one_bad_edge_control()
    src = Path(__file__).resolve().parents[1] / "threads/src"
    sources = {name: (src / name).read_text() for name in ("waker.rs", "park.rs")}
    # Reuse the actual SME2 announcement; compile only this prelude, not
    # the hardware-dependent worker loop (which needs the Apple SDK).
    lanes = (src / "lanes.rs").read_text()
    advertisement = re.search(
        r"(SME2_PARKED\.store\(true, Ordering::SeqCst\);\s+"
        r"core::sync::atomic::fence\(Ordering::SeqCst\);)\s+if sources::visible\(true\)", lanes
    )
    if advertisement is None:
        raise SystemExit("sleep ordering: SME2 must order its announcement before the work re-check")
    probe = PROBE.replace("// SME2_ADVERTISEMENT", advertisement[1].replace("SME2_PARKED", "flag"))
    paths = {
        "waker.rs": [("register_then_recheck", "stlrb", "ldaprb", lambda a, f, n: ordered_cfg(a, f, n, "ldaprb"))],
        "park.rs": [("park_then_recheck", "ldsetal", "ldapr", ordered)],
        "probe.rs": [("sme2_then_recheck", "stlrb", "ldapr", ordered)],
    }
    with tempfile.TemporaryDirectory(prefix="sleep-order-") as tmp:
        directory = Path(tmp)
        sources["probe.rs"] = probe
        asm = compile_probe(sources, directory)
        for name, checks in paths.items():
            prefix = "core::sync::atomic::" if name == "probe.rs" else ""
            fence = prefix + "fence(Ordering::SeqCst);"
            if sources[name].count(fence) != 1:
                raise SystemExit(f"sleep ordering: expected one SeqCst fence in {name}")
            unfenced = compile_probe({**sources, name: sources[name].replace(fence, "")}, directory)
            for function, announce, load_op, check in checks:
                if not check(asm, function, announce):
                    raise SystemExit(f"sleep ordering: missing dmb ish in {function}")
                if check(unfenced, function, announce):
                    raise SystemExit(f"sleep ordering: negative control retained a barrier in {function}")
                print(f"sleep ordering: {function} has {announce}/dmb ish/{load_op}; unfenced control rejected")


if __name__ == "__main__":
    main()
