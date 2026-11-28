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
        c = counts.measure_files({"src/x.rs": src})
        self.assertEqual(c["debug_asserts"], 2)
        self.assertEqual(c["truncation_allows"], 0)
        self.assertEqual(c["undocumented_unsafe"], 0)

    def test_detects_allow_truncation_and_undocumented_unsafe(self):
        src = (
            '#[allow(clippy::cast_possible_truncation)]\n'
            'fn bad_allow() {\n'
            '    let _ = unsafe { 1 };\n'
            '}\n'
            '#[expect(clippy::cast_possible_truncation)]\n'
            'fn bad_expect(x: usize) -> u8 {\n'
            '    x as u8\n'
            '}\n'
            '#[expect(clippy :: cast_possible_truncation)]\n'
            'fn bad_spaced_expect(x: usize) -> u8 {\n'
            '    x as u8\n'
            '}\n'
            '#[cfg_attr(unix, expect(clippy :: cast_possible_truncation))]\n'
            'fn bad_cfg_attr_expect(x: usize) -> u8 {\n'
            '    x as u8\n'
            '}\n'
            '#[warn(clippy::cast_possible_truncation)]\n'
            'fn bad_warn(x: usize) -> u8 {\n'
            '    x as u8\n'
            '}\n'
            '# [warn(clippy :: cast_possible_truncation)]\n'
            'fn bad_spaced_hash(x: usize) -> u8 {\n'
            '    x as u8\n'
            '}\n'
            '#[warn(clippy :: r#cast_possible_truncation)]\n'
            'fn bad_raw_ident(x: usize) -> u8 {\n'
            '    x as u8\n'
            '}\n'
        )
        c = counts.measure_files({"src/x.rs": src})
        self.assertEqual(c["truncation_allows"], 7)
        self.assertEqual(c["undocumented_unsafe"], 1)

    def test_detects_undocumented_debug_assert_and_silent_narrowings(self):
        src = (
            'fn f(x: usize) -> u8 {\n'
            '    debug_assert_eq !(x, 0);\n'
            '    x.to_le_bytes( )[0]\n'
            '}\n'
        )
        c = counts.measure_files({"src/x.rs": src})
        self.assertEqual(c["debug_asserts"], 1)
        self.assertEqual(c["undocumented_debug_asserts"], 1)
        self.assertEqual(c["silent_narrowings"], 1)
        clean = counts.measure_files({"src/x.rs": "fn f() {}\n"})
        rose = counts.check_counts(clean, c)
        self.assertTrue(any(r.startswith("undocumented_debug_asserts:") for r in rose), rose)
        self.assertTrue(any(r.startswith("silent_narrowings:") for r in rose), rose)
        self.assertTrue(any(r.startswith("debug_asserts:") for r in rose), rose)

    def test_ignores_cfg_test_items_and_mods_but_checks_cfg_not_test(self):
        src = (
            '#[cfg(test)]\n'
            'mod t { fn x() { debug_assert!(true); } }\n'
            '#[cfg(test)]\n'
            'fn test_helper() { debug_assert!(true); }\n'
            '#[cfg(test)] const ONE_LINE: u8 = 1;\n'
            '#[cfg(not(test))]\n'
            'mod prod { fn y() { debug_assert!(true); } }\n'
            'fn after_closure(x: usize) -> u8 {\n'
            '    #[cfg(test)]\n'
            '    NOTED.with(|n| { n.set(1); });\n'
            '    debug_assert\n'
            '        !(x == 0);\n'
            '    x.to_le_bytes()[0]\n'
            '}\n'
        )
        c = counts.measure_files({"src/x.rs": src})
        self.assertEqual(c["debug_asserts"], 2)
        self.assertEqual(c["undocumented_debug_asserts"], 2)
        self.assertEqual(c["silent_narrowings"], 1)


if __name__ == "__main__":
    unittest.main()
