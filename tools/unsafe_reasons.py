#!/usr/bin/env python3
"""Every `unsafe` block in the listed files says why it is sound.

    python3 tools/unsafe_reasons.py

A reason is a comment with "Sound:" in the eight lines above the block
(blocks that stand together share one). Exit status 1 names each block
without one. The files are those whose unsafe code no proof and no model
covers line by line; a file joins the list once each of its blocks has
its reason, and then cannot gain a block without one. Test modules are
not held to it.

Why: the 0.10.0 audit's memory-safety findings were in files whose unsafe
blocks stated no reason (threads/src/pipeline.rs had 79 and none), while
the files that state theirs had none (BLAKE3 #411, #417, #418, #427).
"""

import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FILES = ["threads/src/pipeline.rs"]
REACH = 8
UNSAFE = re.compile(r"\bunsafe\s*\{|\bunsafe\s+impl\b")


def without_reason(text):
    """The lines (numbers from 1, and the line) of `text`'s unsafe blocks
    that have no "Sound:" comment within REACH lines above, up to its
    test module."""
    lines = text.split("\n")
    found = []
    for i, line in enumerate(lines):
        if re.match(r"\s*(#\[cfg\(test\)\]\s*)?mod tests?\b", line) and i and "cfg(test)" in lines[i - 1] + line:
            break
        if line.lstrip().startswith("//") or not UNSAFE.search(line):
            continue
        above = lines[max(0, i - REACH):i + 1]
        if not any("Sound:" in a and a.lstrip().startswith("//") for a in above[:-1]) and "// Sound:" not in line:
            found.append((i + 1, line.strip()))
    return found


def main():
    bad = 0
    for path in FILES:
        with open(os.path.join(ROOT, path)) as f:
            for number, line in without_reason(f.read()):
                bad += 1
                print(f"{path}:{number}: no reason (a `// Sound: ...` comment above): {line[:100]}")
    if bad:
        sys.exit(f"unsafe_reasons: {bad} unsafe blocks state no reason")
    print(f"unsafe_reasons: every unsafe block of {', '.join(FILES)} states its reason")


if __name__ == "__main__":
    main()
