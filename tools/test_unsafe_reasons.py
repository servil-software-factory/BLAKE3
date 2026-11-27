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

    def test_non_test_cfgs_mentioning_test_are_still_checked(self):
        for cfg in ('#[cfg(not(test))]', '#[cfg(any(test, feature = "std"))]'):
            text = f"{cfg}\nmod shipped {{\n    fn f() {{ unsafe {{ g() }} }}\n}}\n"
            self.assertEqual([n for n, _ in ur.without_reason(text)], [3], cfg)

    def test_string_braces_in_test_module_do_not_hide_following_shipped_code(self):
        text = '#[cfg(test)]\nmod t {\n    const S: &str = "}}}";\n    fn ok() { unsafe { g() } }\n}\nfn shipped() { unsafe { h() } }\n'
        self.assertEqual([n for n, _ in ur.without_reason(text)], [6])

    def test_shipped_file_discovery_includes_self_test_and_version_build_helpers(self):
        discovered = set(ur.shipped_files())
        for must_include in (
            "src/self_test.rs",
            "src/lib.rs",
            "threads/src/pipeline.rs",
            "capi/src/lib.rs",
            "version/src/lib.rs",
            "build.rs",
            "version/build.rs",
            "version/build_version.rs",
        ):
            self.assertIn(must_include, discovered, f"shipped file missing from discovery: {must_include}")
        for must_exclude in (
            "src/test.rs",
            "src/test_support.rs",
            "src/lifecycle_counts.rs",
            "src/ffi_avx512_32_guard_tests.rs",
            "threads/src/forms_test.rs",
        ):
            self.assertNotIn(must_exclude, discovered, f"test-only file unexpectedly scanned: {must_exclude}")

    def test_debug_assert_requires_debug_only_comment(self):
        ok = "fn f() {\n    // Debug-only: hot loop invariant.\n    debug_assert!(true);\n    debug_assert_eq!(1, 1);\n}\n"
        self.assertEqual(ur.debug_assert_without_reason(ok), [])
        bad = "fn f() {\n    // Just a comment.\n    debug_assert!(true);\n    debug_assert_eq !(1, 1);\n}\n"
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(bad)], [3, 4])

    def test_debug_assert_rejects_string_spoof_empty_reason_and_sibling_borrowing(self):
        spoof = 'fn f() {\n    debug_assert!(true, "// Debug-only: not a comment");\n}\n'
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(spoof)], [2])
        empty = "fn f() {\n    // Debug-only:   \n    debug_assert!(true);\n}\n"
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(empty)], [3])
        empty_block = "fn f() {\n    /* Debug-only: */\n    debug_assert!(true);\n}\n"
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(empty_block)], [3])
        empty_sound_block = "fn f() {\n    /* Sound: */\n    let _ = unsafe { 1 };\n}\n"
        self.assertEqual([n for n, _ in ur.without_reason(empty_sound_block)], [3])
        sibling = (
            "fn f(a: bool) {\n"
            "    if a {\n"
            "        // Debug-only: first branch.\n"
            "        debug_assert!(true);\n"
            "    } else {\n"
            "        debug_assert!(true);\n"
            "    }\n"
            "}\n"
        )
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(sibling)], [6])
        oneline_branches = (
            "fn f(a: bool) {\n"
            "    // Debug-only: first branch.\n"
            "    if a { debug_assert!(a); }\n"
            "    else { debug_assert!(!a); }\n"
            "}\n"
        )
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(oneline_branches)], [3, 4])
        two_on_one_line = "fn f() {\n    debug_assert!(true); debug_assert!(true); // Debug-only: only trailing\n}\n"
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(two_on_one_line)], [2])
        split_bang = "fn f() {\n    debug_assert_eq\n        !(1, 1);\n}\n"
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(split_bang)], [2])

    def test_cfg_test_skipping_preserves_adjacent_shipped_code_and_cfg_attr(self):
        # Closure braces inside parens (`#[cfg(test)] NOTED.with(|n| { ... });`)
        # must stop at `;` and never hide following shipped code (lanes.rs).
        closure_stmt = (
            "fn noted() {\n"
            "    #[cfg(test)]\n"
            "    NOTED.with(|n| {\n"
            "        n.set(1);\n"
            "    });\n"
            "    debug_assert!(true);\n"
            "    let _ = unsafe { 1 };\n"
            "}\n"
        )
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(closure_stmt)], [6])
        self.assertEqual([n for n, _ in ur.without_reason(closure_stmt)], [7])
        # Single-line `#[cfg(test)] const` must not swallow the next shipped item.
        oneline_const = (
            "#[cfg(test)] const TEST: u8 = 1;\n"
            "fn shipped() { debug_assert!(true); }\n"
        )
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(oneline_const)], [2])
        # Test-only struct field (with or without comma) must not hide following items.
        struct_field = (
            "struct S {\n"
            "    #[cfg(test)]\n"
            "    counter: usize,\n"
            "    #[cfg(test)]\n"
            "    last: usize\n"
            "}\n"
            "fn shipped() { debug_assert!(true); }\n"
        )
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(struct_field)], [7])
        # `#[cfg_attr(..., cfg(test))]` is shipped when the outer predicate is false.
        cfg_attr = (
            '#[cfg_attr(feature = "nightly", cfg(test))]\n'
            "fn shipped() { debug_assert!(true); }\n"
        )
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(cfg_attr)], [2])
        # Test-only last function parameter must stop at `)` and never mask the body.
        last_param = (
            "fn prod(x: usize, #[cfg(test)] _trace: bool) -> u8 {\n"
            "    debug_assert!(x < 256);\n"
            "    let _ = unsafe { 1 };\n"
            "    x.to_le_bytes()[0]\n"
            "}\n"
        )
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(last_param)], [2])
        self.assertEqual([n for n, _ in ur.without_reason(last_param)], [3])
        # Test-only `pub` struct field must terminate at `,`, not swallow the next field/fn.
        pub_field = (
            "struct S {\n"
            "    #[cfg(test)]\n"
            "    pub counter: usize,\n"
            "    pub real: usize,\n"
            "}\n"
            "fn prod() { debug_assert!(true); }\n"
        )
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(pub_field)], [6])
        # Whitespace inside `#[cfg (test)]` and `#[allow(...)]` stacked BEFORE `#[cfg(test)]`.
        stacked = (
            "#[allow(clippy::cast_possible_truncation)]\n"
            "#[cfg (test)]\n"
            "fn helper() { debug_assert!(true); }\n"
        )
        self.assertEqual(ur.debug_assert_without_reason(stacked), [])
        self.assertNotIn("cast_possible_truncation", ur.masked_code(stacked)[0])
        # Test-only generic parameter must stop at `>` and never mask the shipped function.
        generic_param = (
            "pub fn only<#[cfg(test)] T: Clone>(p: &u32) -> u32 {\n"
            "    debug_assert!(*p < 10);\n"
            "    unsafe { *p }\n"
            "}\n"
        )
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(generic_param)], [2])
        self.assertEqual([n for n, _ in ur.without_reason(generic_param)], [3])
        # Defaulted last generic parameter (`T = u8>`) must still stop at `>` and
        # not swallow the following shipped struct/function body.
        defaulted_generic = (
            "pub struct S<#[cfg(test)] T = u8> {\n"
            "    pub arr: [u8; { debug_assert!(true); unsafe { 1 } }],\n"
            "}\n"
        )
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(defaulted_generic)], [2])
        self.assertEqual([n for n, _ in ur.without_reason(defaulted_generic)], [2])
        # Nested empty block-comment rationale `/* old /* Debug-only: */ */` must fail.
        nested_empty = (
            "fn f(x: u32) {\n"
            "    /* old note /* Debug-only: */ */\n"
            "    debug_assert!(x < 10);\n"
            "    /* old note /* Sound: */ */\n"
            "    let _ = unsafe { x };\n"
            "}\n"
        )
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(nested_empty)], [3])
        self.assertEqual([n for n, _ in ur.without_reason(nested_empty)], [5])
        # A comment attached to a masked `#[cfg(test)]` assertion cannot bleed
        # across the masked statement onto a shipped assertion below it.
        donated = (
            "fn f(x: u32) {\n"
            "    // Debug-only: mock-only check.\n"
            "    #[cfg(test)]\n"
            "    debug_assert!(x != 8);\n"
            "    debug_assert!(x > 5);\n"
            "}\n"
        )
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(donated)], [5])
        # A shift expression in a test-only enum variant must not open a generic
        # parameter list and swallow the following shipped variant.
        enum_shift = (
            "pub enum E {\n"
            "    #[cfg(test)] A = 1 << 2,\n"
            "    B = {\n"
            "        debug_assert!(3 > 2);\n"
            "        unsafe { core::ptr::read(&7) }\n"
            "    },\n"
            "}\n"
        )
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(enum_shift)], [4])
        self.assertEqual([n for n, _ in ur.without_reason(enum_shift)], [5])
        # A comment attached to a masked `#[cfg(test)]` assertion on the same
        # line (or line above) cannot bleed onto a following shipped assertion.
        same_line_donated = (
            "fn f(x: u32) {\n"
            "    // Debug-only: test fixture.\n"
            "    #[cfg(test)] debug_assert!(x < 99); debug_assert!(x != 0);\n"
            "    /* Debug-only: inline test. */ #[cfg(test)] debug_assert!(x < 88); debug_assert!(x != 1);\n"
            "}\n"
        )
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(same_line_donated)], [3, 4])
        # Nested generics (`Vec<Vec<u8>>`) in a test-only struct field must
        # close both `<` levels on `>>` and stop at `,`, never masking the next field.
        nested_generic_field = (
            "pub struct S {\n"
            "    #[cfg(test)] scratch: Vec<Vec<u8>>,\n"
            "    pub shipped: [u8; { debug_assert!(true); unsafe { 1 } }],\n"
            "}\n"
        )
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(nested_generic_field)], [3])
        self.assertEqual([n for n, _ in ur.without_reason(nested_generic_field)], [3])
        # Trailing comment after a same-line masked `#[cfg(test)]` assertion
        # cannot satisfy an earlier shipped assertion on the same line.
        trailing_donated = (
            "fn f(x: u32) {\n"
            "    debug_assert!(x > 0); #[cfg(test)] debug_assert!(x < 10); // Debug-only: test upper bound.\n"
            "}\n"
        )
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(trailing_donated)], [2])
        # Underscores alone are punctuation, not a stated rationale.
        underscores = (
            "fn f(p: &u8) -> u8 {\n"
            "    // Debug-only: ___\n"
            "    debug_assert!(*p < 10);\n"
            "    // Sound: ___\n"
            "    unsafe { *p }\n"
            "}\n"
        )
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(underscores)], [3])
        self.assertEqual([n for n, _ in ur.without_reason(underscores)], [5])
        # Non-ASCII Unicode rationales must still be accepted.
        unicode_ok = (
            "fn f(p: &u8) -> u8 {\n"
            "    // Debug-only: ホットパス。\n"
            "    debug_assert!(*p < 10);\n"
            "    // Sound: 有効な参照。\n"
            "    unsafe { *p }\n"
            "}\n"
        )
        self.assertEqual(ur.debug_assert_without_reason(unicode_ok), [])
        self.assertEqual(ur.without_reason(unicode_ok), [])
        # Comments inside a preceding statement or `#[cfg(test)]` block cannot
        # satisfy a following shipped `debug_assert!`.
        prev_stmt = (
            "fn f(x: u32) {\n"
            "    if x == 0 { /* Debug-only: zero case. */ debug_assert_eq!(x, 0); }\n"
            "    debug_assert_ne!(x, 1);\n"
            "}\n"
        )
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(prev_stmt)], [3])
        masked_comm = (
            "fn f() {\n"
            "    #[cfg(test)]\n"
            "    {\n"
            "        // Debug-only: test block comment.\n"
            "    }\n"
            "    debug_assert!(true);\n"
            "}\n"
        )
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(masked_comm)], [6])
        # Valid multiline block rationales pass for both Sound: and Debug-only:.
        multiline = (
            "fn f() {\n"
            "    /* Sound: No raw pointers here.\n"
            "       Detailed explanation. */\n"
            "    unsafe { () };\n"
            "    /* Debug-only: expensive invariant.\n"
            "       Second line. */\n"
            "    debug_assert!(true);\n"
            "}\n"
        )
        self.assertEqual(ur.without_reason(multiline), [])
        self.assertEqual(ur.debug_assert_without_reason(multiline), [])

    def test_cfg_test_fn_items_are_excluded(self):
        src = (
            "#[cfg(test)]\n"
            "fn helper() {\n"
            "    debug_assert!(true);\n"
            "    let _ = unsafe { 1 };\n"
            "}\n"
            "fn shipped() {\n"
            "    debug_assert!(true);\n"
            "}\n"
        )
        self.assertEqual([n for n, _ in ur.debug_assert_without_reason(src)], [7])
        self.assertEqual(ur.without_reason(src), [])

    def test_all_shipped_files_pass(self):
        files = ur.shipped_files()
        self.assertGreaterEqual(len(files), 40)
        for path in files:
            with open(os.path.join(ur.ROOT, path)) as f:
                text = f.read()
                self.assertEqual(ur.without_reason(text), [], path)
                self.assertEqual(ur.debug_assert_without_reason(text), [], path)


if __name__ == "__main__":
    unittest.main()
