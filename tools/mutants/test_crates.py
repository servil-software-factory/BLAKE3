"""The tool on a repository of two crates (BLAKE3 #335): which crate a run is of, where
its lists are, and how one list's rows divide when files move to the other crate."""
import os
import unittest

import check

LIST = """# The mutants known to survive on x86-64.
# A line: file <TAB> mutant <TAB> places <TAB> reason.
files: src/block.rs src/lanes.rs src/lib.rs src/stream.rs src/stream/receiver.rs
model files: src/finish.rs src/park.rs
src/block.rs\treplace | with ^ in f\t1\tequivalent: disjoint bits
src/lanes.rs\treplace > with >= in g\t2\t[cpu has avx2] speed: either order
src/lib.rs\treplace h -> bool with true\t1\ttest-only code
src/stream/receiver.rs\treplace + with * in core\t1\tequivalent: the ring's bound
"""


class Crates(unittest.TestCase):
    def test_the_crate_is_the_roots_unless_named(self):
        self.assertEqual(check.crate_named(["check.py", "--whole"]), "")
        self.assertEqual(check.crate_named(["check.py", "--crate", "threads", "--whole"]), "threads")
        self.assertEqual(check.crate_named(["check.py", "--crate", "b3sum", "--whole"]), "b3sum")
        self.assertEqual(check.crate_named(["check.py", "--crate", "."]), "")
        for bad in (["check.py", "--crate"], ["check.py", "--crate", "../elsewhere"], ["check.py", "--crate", "/abs"], ["check.py", "--crate", "a/b"]):
            with self.assertRaises(SystemExit, msg=bad):
                check.crate_named(bad)

    def test_a_crates_lists_sit_in_a_folder_of_its_name(self):
        root = os.path.join(os.sep, "r")
        self.assertEqual(check.lists_dir(root, ""), os.path.join(root, "tools", "mutants"))
        self.assertEqual(check.lists_dir(root, "threads"), os.path.join(root, "tools", "mutants", "threads"))
        self.assertEqual(check.lists_dir(root, "b3sum"), os.path.join(root, "tools", "mutants", "b3sum"))

    def test_the_other_crates_are_the_folders_that_hold_lists(self):
        self.assertEqual(check.other_crates(["__pycache__", "threads", "known-x86_64.txt", "notes"], lambda d: d == "threads"), ["threads"])
        self.assertEqual(check.other_crates([], lambda d: True), [])

    def test_the_features_are_the_root_crates(self):
        self.assertEqual(check.build_args(""), ["--profile", "mutants", "--features", "serde,zeroize"])
        self.assertEqual(check.build_args("threads"), ["--profile", "mutants"])
        self.assertEqual(check.build_args("b3sum"), ["--profile", "mutants"])

    def test_binary_crates_run_unit_and_cli_tests_without_a_library_target(self):
        library = [{"kind": ["lib"]}, {"kind": ["test"]}]
        binary = [{"kind": ["bin"]}, {"kind": ["test"]}]
        self.assertEqual(check.test_args(library), ["--lib", "--tests"])
        self.assertEqual(check.test_args(binary), ["--tests"])
        self.assertEqual(check.test_args(library + binary), ["--lib", "--tests"])

    def test_a_cli_executable_must_not_be_skipped_by_test_harness_hashes(self):
        self.assertTrue(check.same_code_supported([{"kind": ["lib"]}, {"kind": ["test"]}]))
        self.assertFalse(check.same_code_supported([{"kind": ["bin"]}, {"kind": ["test"]}]))
        self.assertFalse(check.same_code_supported([{"kind": ["lib"]}, {"kind": ["bin"]}]))

    def test_a_list_divides_by_the_files_that_moved(self):
        moved = {"src/lanes.rs", "src/stream.rs", "src/stream/receiver.rs", "src/finish.rs", "src/park.rs"}
        stays, goes = check.divide_list(LIST, moved)
        # A row keeps its text: a file's path within its crate is the path it had.
        self.assertEqual(stays, """# The mutants known to survive on x86-64.
# A line: file <TAB> mutant <TAB> places <TAB> reason.
files: src/block.rs src/lib.rs
src/block.rs\treplace | with ^ in f\t1\tequivalent: disjoint bits
src/lib.rs\treplace h -> bool with true\t1\ttest-only code
""")
        self.assertEqual(goes, """# The mutants known to survive on x86-64.
# A line: file <TAB> mutant <TAB> places <TAB> reason.
files: src/lanes.rs src/stream.rs src/stream/receiver.rs
model files: src/finish.rs src/park.rs
src/lanes.rs\treplace > with >= in g\t2\t[cpu has avx2] speed: either order
src/stream/receiver.rs\treplace + with * in core\t1\tequivalent: the ring's bound
""")
        # Every row lands on exactly one side.
        rows = lambda text: [l for l in text.splitlines() if "\t" in l]
        self.assertEqual(sorted(rows(stays) + rows(goes)), sorted(rows(LIST)))
        # A model file that stays keeps its line where it is.
        stays, goes = check.divide_list(LIST, {"src/lanes.rs"})
        self.assertIn("model files: src/finish.rs src/park.rs\n", stays)
        self.assertNotIn("model files", goes)
        # A list without a files line (the time-outs') divides by its rows alone.
        stays, goes = check.divide_list("# hangs\nsrc/lanes.rs\treplace a with ()\t1\ta hang\nsrc/lib.rs\treplace b with ()\t1\ta hang\n", moved)
        self.assertEqual((stays, goes), ("# hangs\nsrc/lib.rs\treplace b with ()\t1\ta hang\n", "# hangs\nsrc/lanes.rs\treplace a with ()\t1\ta hang\n"))


if __name__ == "__main__":
    unittest.main()
