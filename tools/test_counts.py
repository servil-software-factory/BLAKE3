import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import counts


class CountsControls(unittest.TestCase):
    def test_counts_multiple_invocations_per_line_and_ignores_strings_and_comments(self):
        src = (
            'fn f() {\n'
            '    let s = "debug_assert!(ok); println!(\\"hi\\"); #[allow(clippy::cast_possible_truncation)]";\n'
            '    // debug_assert!(commented)\n'
            '    /* debug_assert_eq!(1, 1); */\n'
            '    debug_assert!(true); debug_assert_eq!(1, 1);\n'
            '    #[deny(clippy::cast_possible_truncation)]\n'
            '    let x = 1;\n'
            '}\n'
        )
        c = counts.measure_files({"b3sum/src/x.rs": src})
        self.assertEqual(c["debug_asserts"], 2)
        self.assertEqual(c["b3sum_print_macros"], 0)
        self.assertEqual(c["truncation_allows"], 0)
        self.assertEqual(c["undocumented_unsafe"], 0)

    def test_detects_allow_truncation_and_b3sum_print_and_undocumented_unsafe(self):
        src = (
            '#[allow(clippy::cast_possible_truncation)]\n'
            'fn bad() {\n'
            '    println!("nope");\n'
            '    let _ = unsafe { 1 };\n'
            '}\n'
        )
        c = counts.measure_files({"b3sum/src/x.rs": src})
        self.assertEqual(c["truncation_allows"], 1)
        self.assertEqual(c["b3sum_print_macros"], 1)
        self.assertEqual(c["undocumented_unsafe"], 1)

    def test_ignores_cfg_test_but_checks_cfg_not_test(self):
        src = (
            '#[cfg(test)]\n'
            'mod t { fn x() { debug_assert!(true); println!("ok"); } }\n'
            '#[cfg(not(test))]\n'
            'mod prod { fn y() { debug_assert!(true); } }\n'
        )
        c = counts.measure_files({"b3sum/src/x.rs": src})
        self.assertEqual(c["debug_asserts"], 1)
        self.assertEqual(c["b3sum_print_macros"], 0)


if __name__ == "__main__":
    unittest.main()
