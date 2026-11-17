#!/usr/bin/env python3
"""Each of b3sum's protections has a test that fails without it.

    python3 tools/b3sum_controls.py

A protection here is a check or a rule in b3sum's code that a bug once
lacked. For each, this takes the protection out with one edit, runs the
test that is there for it, and requires that test to fail; then the file
is put back. A protection whose test still passes is not protected
against being lost: this exits 1 naming it. The tests must also pass on
the code as it is.

b3sum is outside the mutation gate (tools/mutants), which is slow on it
(BLAKE3 #386); this is its bounded stand-in, for the protections named.
A build of b3sum's tests per protection: about four minutes in all.
"""

import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

READ_TO_END = "    file.read_to_end(bytes)?;\n    Ok(())\n}\n\n/// The hash of the input at `path`"
READ_TO_END_NOT = "    bytes.truncate(says);\n    Ok(())\n}\n\n/// The hash of the input at `path`"

# (the protection, its file, the text to find once, what replaces it, the test)
CONTROLS = [
    (
        "--no-mmap maps no file (#432)",
        "b3sum/src/main.rs",
        "        if self.inner.no_mmap {\n            return None;\n        }\n",
        "",
        "test_no_mmap_maps_no_file",
    ),
    (
        "a checkfile with no line is a failure (#407)",
        "b3sum/src/main.rs",
        '    ensure!(saw_line, "{}: no checksum entries", path.display());\n',
        "    let _ = saw_line;\n",
        "test_empty_checkfile_is_failure",
    ),
    (
        "an output error is a failure (#406)",
        "b3sum/src/main.rs",
        "                return Err(e);\n            }\n            bytes = &bytes[rc as usize..];",
        "                return Ok(());\n            }\n            bytes = &bytes[rc as usize..];",
        "test_output_error_is_failure",
    ),
    (
        "a direct read of whole pieces reads once more for the file's end (#424)",
        "b3sum/src/direct.rs",
        "    if len % PIECE == 0 {",
        "    if false {",
        "a_file_read_directly_hashes_as_its_bytes_do",
    ),
    (
        "a read for a known size goes on to where a read gives no byte (#424)",
        "b3sum/src/main.rs",
        READ_TO_END,
        READ_TO_END_NOT,
        "test_a_files_size_does_not_decide_where_it_ends",
    ),
    (
        "a batched small file is held to its slot before the copy",
        "b3sum/src/linux_pipe.rs",
        '        assert!(byte_len <= WARM_SLOT_BLOCKS * 64, "a batched file fits its slot");\n',
        "",
        "a_file_beyond_its_slot_is_refused_before_the_copy",
    ),
    (
        "a buffer is sized by what was read, not by the file's size (#440)",
        "b3sum/src/linux_pipe.rs",
        "    ensure_spare_cap(buf, (hint + 1).max(n + 4096));",
        "    ensure_spare_cap(buf, (hint + 1).max(4096));",
        "a_buffer_holds_what_was_read_whatever_the_size_said",
    ),
    (
        "growing the descriptor table lands on no open descriptor (#441)",
        "b3sum/src/linux_pipe.rs",
        "                let high = libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, target);",
        "                let high = libc::dup2(fd, target);",
        "growing_the_descriptor_table_closes_no_descriptor",
    ),
    (
        "a ring dropped with reads in flight stops the process (#442)",
        "b3sum/src/uring.rs",
        "        if self.in_flight != 0 {\n            use std::io::Write;",
        "        if false {\n            use std::io::Write;",
        "a_ring_dropped_with_a_read_in_flight_stops_the_process",
    ),
    (
        "a failed io_uring_enter stops the process (#409)",
        "b3sum/src/uring.rs",
        "stopping before pending reads' buffers can be released\");\n    std::process::abort()",
        "stopping before pending reads' buffers can be released\");\n    None",
        "failed_enter_stops_before_unwinding",
    ),
    (
        "the file opened is the file hashed where there is no io_uring",
        "b3sum/src/linux_pipe.rs",
        "        } else {\n            sink.put(i, crate::hash_open_file(args, file));\n        }\n",
        "        } else {\n            drop(file);\n            sink.put(i, crate::hash_path_hash(args, &paths[i]));\n        }\n",
        "without_a_ring_the_file_opened_is_the_file_hashed",
    ),
]


def test_passes(test):
    run = subprocess.run(["cargo", "test", "-q", "--release", "--manifest-path", "b3sum/Cargo.toml", "--", test], cwd=ROOT, capture_output=True, text=True)
    ran = [line for line in run.stdout.splitlines() if line.startswith("test result:")]
    if run.returncode != 0 and not ran and "running " not in run.stdout:
        return None, run.stdout[-1500:] + run.stderr[-1500:]
    if run.returncode == 0 and not any(" 1 passed" in line for line in ran):
        sys.exit(f"b3sum_controls: no test named {test} ran: update this script")
    return run.returncode == 0, ""


def main():
    bad = []
    for name, path, find, replace, test in CONTROLS:
        full = os.path.join(ROOT, path)
        with open(full) as f:
            original = f.read()
        if original.count(find) != 1:
            sys.exit(f"b3sum_controls: the place of '{name}' is not in {path} once: update this script")
        passes, why = test_passes(test)
        if passes is not True:
            sys.exit(f"b3sum_controls: {test} does not pass on the code as it is\n{why}")
        try:
            with open(full, "w") as f:
                f.write(original.replace(find, replace))
            passes, why = test_passes(test)
        finally:
            with open(full, "w") as f:
                f.write(original)
        verdict = "fails without it" if passes is False else "does not build without it" if passes is None else "STILL PASSES without it"
        print(f"{verdict:26}  {name}  ({test})", flush=True)
        if passes is not False:
            bad.append(name)
            if why:
                print(why)
    if bad:
        sys.exit(f"b3sum_controls: {len(bad)} protections have no test that fails without them")
    print(f"b3sum_controls: each of {len(CONTROLS)} protections has a test that fails without it")


if __name__ == "__main__":
    main()
