import importlib.util
import os
import unittest

spec = importlib.util.spec_from_file_location("unsafe_reasons", os.path.join(os.path.dirname(os.path.abspath(__file__)), "unsafe_reasons.py"))
ur = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ur)


class Reasons(unittest.TestCase):
    def test_a_block_with_its_reason_above_passes_and_blocks_together_share_it(self):
        text = "fn f() {\n    // Sound: the pointer is ours.\n    let a = unsafe { *p };\n    let b = unsafe { *q };\n}\n"
        self.assertEqual(ur.without_reason(text), [])

    def test_a_block_with_no_reason_is_named(self):
        text = "fn f() {\n    // The pointer is ours.\n    let a = unsafe { *p };\n}\nunsafe impl Send for X {}\n"
        self.assertEqual(ur.without_reason(text), [(3, "let a = unsafe { *p };"), (5, "unsafe impl Send for X {}")])

    def test_a_reason_too_far_above_does_not_count(self):
        text = "// Sound: once.\n" + "let x = 1;\n" * ur.REACH + "let a = unsafe { *p };\n"
        self.assertEqual([n for n, _ in ur.without_reason(text)], [ur.REACH + 2])
        text = "// Sound: once.\n" + "let x = 1;\n" * (ur.REACH - 1) + "let a = unsafe { *p };\n"
        self.assertEqual(ur.without_reason(text), [])

    def test_the_word_in_code_or_a_string_is_no_reason(self):
        text = 'let s = "Sound: no";\nlet a = unsafe { *p };\n'
        self.assertEqual([n for n, _ in ur.without_reason(text)], [2])

    def test_unsafe_fn_and_comments_are_not_blocks_and_tests_are_not_held(self):
        text = "// unsafe { in a comment }\npub unsafe fn f() {}\n#[cfg(test)]\nmod test {\n    fn t() { unsafe { g() } }\n}\n"
        self.assertEqual(ur.without_reason(text), [])
        text = "#[cfg(all(test, not(shuttle)))]\nmod test {\n    fn t() { unsafe { g() } }\n}\n"
        self.assertEqual(ur.without_reason(text), [])
        text = "#[cfg(feature = \"std\")]\nmod tests {\n    fn t() { unsafe { g() } }\n}\n"
        self.assertEqual([n for n, _ in ur.without_reason(text)], [3], "a module that is not the tests' is held to it")

    def test_the_listed_files_pass(self):
        for path in ur.FILES:
            with open(os.path.join(ur.ROOT, path)) as f:
                self.assertEqual(ur.without_reason(f.read()), [], path)


if __name__ == "__main__":
    unittest.main()
