#!/usr/bin/env python3
"""Unit tests for tools/stack_budget.py."""

import unittest

import stack_budget


SAMPLE_FUNCS_FIRST = """
StackSizes [
  Entry {
    Functions: [blake3_servil_core::tree::hash_tree::h0000000000000000]
    Size: 0x2000
  }
  Entry {
    Functions: [<blake3_servil_core::incremental::Hasher>::finalize (.llvm.11097525931525558011)]
    Size: 0x1000
  }
  Entry {
    Functions: [blake3_servil::lanes::run_work::h0000000000000000]
    Size: 0x800
  }
  Entry {
    Functions: [b3sums::main::h0000000000000000]
    Size: 0x400
  }
]
"""

SAMPLE_SIZE_FIRST = """
StackSizes [
  Entry {
    Size: 0x4001
    Functions: [blake3_servil::lanes::too_big::h0000000000000000]
  }
]
"""


def _make_entries(items: list[tuple[int, str]]) -> str:
    blocks = [f"  Entry {{\n    Functions: [{sym}]\n    Size: {hex(sz)}\n  }}" for sz, sym in items]
    return "StackSizes [\n" + "\n".join(blocks) + "\n]\n"


class StackBudgetTests(unittest.TestCase):
    def test_parses_and_passes_under_caps(self) -> None:
        entries = stack_budget.parse_llvm_readobj_stack_sizes(SAMPLE_FUNCS_FIRST)
        self.assertEqual(len(entries), 4)
        max_by_bucket, violations, budgeted = stack_budget.check_entries(entries)
        self.assertEqual(violations, [])
        self.assertEqual(budgeted, 4)
        self.assertEqual(max_by_bucket["core"][0], 0x2000)
        self.assertEqual(max_by_bucket["core::Hasher"][0], 0x1000)
        self.assertEqual(max_by_bucket["threads"][0], 0x800)
        self.assertEqual(max_by_bucket["b3sum"][0], 0x400)

    def test_fails_when_threads_exceeds_16k(self) -> None:
        entries = stack_budget.parse_llvm_readobj_stack_sizes(SAMPLE_SIZE_FIRST)
        _, violations, _ = stack_budget.check_entries(entries)
        self.assertEqual(len(violations), 1)
        self.assertIn("threads:", violations[0])
        self.assertIn("16385 B > 16384 B", violations[0])

    def test_fails_when_hasher_non_update_methods_or_closures_exceed_32k(self) -> None:
        for sym in (
            "_RNvMs1_NtCsdKSb5PDnG5P_18blake3_servil_core11incrementalNtB5_6Hasher8finalize.llvm.11097525931525558011",
            "_RNvMs1_NtCsdKSb5PDnG5P_18blake3_servil_core11incrementalNtB5_6Hasher12finalize_xof",
            "_RNCNvMs1_NtCsdKSb5PDnG5P_18blake3_servil_core11incrementalNtB7_6Hasher8finalize0B9_",
            "_ZN18blake3_servil_core11convenience6Hasher8finalize17h0000000000000000E",
            "_ZN18blake3_servil_core11incremental6Hasher8finalize27$u7b$$u7b$closure$u7d$$u7d$17h0123456789abcdefE",
            "blake3_servil_core::convenience::Hasher::finalize",
            "blake3_servil_core::convenience::Hasher::finalize_xof",
            "<blake3_servil_core::incremental::Hasher>::finalize (.llvm.42)",
        ):
            _, violations, _ = stack_budget.check_entries([(32 * 1024 + 8, sym)])
            self.assertEqual(len(violations), 1, sym)
            self.assertIn("core::Hasher:", violations[0], sym)

    def test_hasher_update_methods_take_core_cap(self) -> None:
        for sym in (
            "_RNvMs1_NtCsdKSb5PDnG5P_18blake3_servil_core11incrementalNtB5_6Hasher6update.llvm.11097525931525558011",
            "_ZN18blake3_servil_core11incremental6Hasher13update_reader17h0000000000000000E",
            "_ZN18blake3_servil_core11incremental6Hasher6update27$u7b$$u7b$closure$u7d$$u7d$17h0123456789abcdefE",
            "<blake3_servil_core::incremental::Hasher>::update (.llvm.11097525931525558011)",
            "<blake3_servil_core::incremental::Hasher>::update_with_join",
            "blake3_servil_core::convenience::Hasher::update_rayon",
        ):
            self.assertEqual(
                stack_budget.classify_symbol(sym),
                ("core", stack_budget.CAP_CORE_B3SUM),
                sym,
            )

    def test_fails_when_core_exceeds_128k_including_raw_v0_output_reader_fill(self) -> None:
        for sym in (
            "_RNvMsq_CsdKSb5PDnG5P_18blake3_servil_coreNtB5_12OutputReader4fill",
            "<blake3_servil_core::OutputReader>::fill",
            "<blake3_servil_core[4edb22372a9d6ceb]::OutputReader>::fill",
        ):
            _, violations, _ = stack_budget.check_entries([(143_496, sym)])
            self.assertEqual(len(violations), 1, sym)
            self.assertIn("core:", violations[0])

    def test_v0_impl_provenance_for_foreign_receiver_and_stdlib_blanket_impls(self) -> None:
        # Local `impl From<Hash> for [u8; 32]` defined in blake3_servil_core:
        raw_from_hash = "_RNvXs0_CsdKSb5PDnG5P_18blake3_servil_coreAhj20_INtNtCskADBYZwJmSU_4core7convert4FromNtB5_4HashE4from"
        self.assertEqual(
            stack_budget.classify_symbol(raw_from_hash),
            ("core", stack_budget.CAP_CORE_B3SUM),
        )
        _, violations, _ = stack_budget.check_entries([(150_040, raw_from_hash)])
        self.assertEqual(len(violations), 1)
        self.assertIn("core:", violations[0])

        demangled_from_hash = "<[u8; 32] as core::convert::From<blake3_servil_core::Hash>>::from"
        self.assertEqual(
            stack_budget.classify_symbol(demangled_from_hash),
            ("core", stack_budget.CAP_CORE_B3SUM),
        )

        # Converse: stdlib blanket `impl<T: Debug> Debug for &T` instantiated for `&Cv`
        # has v0 impl path in `core::fmt`, NOT `blake3_servil_core` -> must be ignored.
        raw_stdlib_ref_debug = "_RNvXs1g_NtCskADBYZwJmSU_4core3fmtRNtNtCsdKSb5PDnG5P_18blake3_servil_core10throughput2CvNtB6_5Debug3fmtBA_"
        self.assertIsNone(stack_budget.classify_symbol(raw_stdlib_ref_debug))

        # Our own `impl Debug for Cv` in `blake3_servil_core::throughput` -> must be budgeted.
        raw_own_cv_debug = "_RNvXs_NtCsdKSb5PDnG5P_18blake3_servil_core10throughputNtB4_2CvNtNtCskADBYZwJmSU_4core3fmt5Debug3fmt"
        self.assertEqual(
            stack_budget.classify_symbol(raw_own_cv_debug),
            ("core", stack_budget.CAP_CORE_B3SUM),
        )

    def test_inlined_shipped_closures_inside_std_generics_are_budgeted(self) -> None:
        # Real ELF symbol: planted 300 KiB local inside `blake3_servil::lanes::with_scratch`
        # closure inlined into `std::thread::LocalKey::with` must trip the 16 KiB `threads` cap.
        raw_localkey_with_scratch = (
            "_RINvMs2_NtNtCs8ZKwes1zzDd_3std6thread5localINtB6_8LocalKey"
            "INtNtCskADBYZwJmSU_4core4cell7RefCellNtNtCskmg1lIIckmr_13blake3_servil5lanes7ScratchEE"
            "4withNCINvB1x_12with_scratchjANtNtCshuPuHNVDJsN_18blake3_servil_core10throughput2Cvj2_"
            "NCINvB1x_8tree_cvsB2L_NCNvB1x_14subtree_halves0E0NCB3I_s_0E0B2L_EB1z_"
        )
        self.assertEqual(
            stack_budget.classify_symbol(raw_localkey_with_scratch),
            ("threads", stack_budget.CAP_THREADS),
        )
        _, violations, _ = stack_budget.check_entries([(307_800, raw_localkey_with_scratch)])
        self.assertEqual(len(violations), 1)
        self.assertIn("threads:", violations[0])

        # Real ELF symbol: pool worker body `std::sys::backtrace::__rust_begin_short_backtrace::<blake3_servil::lanes::initialize::{closure#1}>`
        raw_worker_body = "_RINvNtNtCs8ZKwes1zzDd_3std3sys9backtrace28___rust_begin_short_backtraceNCNvNtCskmg1lIIckmr_13blake3_servil5lanes10initializes1_0uEB1d_"
        self.assertEqual(
            stack_budget.classify_symbol(raw_worker_body),
            ("threads", stack_budget.CAP_THREADS),
        )

        # `std::sync::Once::call_once::<b3sum::init_pool_once::{closure#0}>::{closure#0}`
        raw_once_b3sum = "_RNCINvMs0_NtNtCs8ZKwes1zzDd_3std4sync4onceNtB8_4Once9call_onceNCNvCs9C2ZzjvkBXS_5b3sum14init_pool_once0E0B12_.llvm.11097525931525558011"
        self.assertEqual(
            stack_budget.classify_symbol(raw_once_b3sum),
            ("b3sum", stack_budget.CAP_CORE_B3SUM),
        )

        # Demangled form of a std sort/generic inlining a shipped closure
        demangled_sort = "core::slice::sort::stable::driftsort_main::<blake3_servil::lanes::places::assign::{closure#0}>"
        self.assertEqual(
            stack_budget.classify_symbol(demangled_sort),
            ("threads", stack_budget.CAP_THREADS),
        )

        # Pure non-callable type args (`drop_glue::<DirectPermits>`, `Channel::<Hash>`) stay ignored.
        for non_callable in (
            "_RINvNtCskADBYZwJmSU_4core3ptr9drop_glueNtCs9C2ZzjvkBXS_5b3sum13DirectPermitsEBD_",
            "core::ptr::drop_glue::<b3sum::DirectPermits>",
            "<std::sync::mpmc::list::Channel<(usize, blake3_servil_core::Hash)> as core::ops::drop::Drop>::drop",
        ):
            self.assertIsNone(stack_budget.classify_symbol(non_callable), non_callable)

    def test_v0_parses_every_rfc2603_basic_type_and_rejects_cyclic_backref(self) -> None:
        # Walk every RFC 2603 single-letter basic type, including `x` (i64) and `d` (f64).
        for tag in sorted(stack_budget._V0_BASIC_TYPES):
            sym_std = f"_RINvNtCskADBYZwJmSU_4core3mem4swap{tag}ECs9C2ZzjvkBXS_5b3sum"
            self.assertIsNone(stack_budget.classify_symbol(sym_std), tag)
            sym_core = f"_RINvCsdKSb5PDnG5P_18blake3_servil_core3foo{tag}EB2_"
            self.assertEqual(
                stack_budget.classify_symbol(sym_core),
                ("core", stack_budget.CAP_CORE_B3SUM),
                tag,
            )

        with self.assertRaises(stack_budget.SymbolParseError):
            stack_budget.classify_symbol("_RNvB_18blake3_servil_core3foo")

    def test_legacy_trait_impl_drop_with_rust_escapes_caught(self) -> None:
        legacy_impl = "_$LT$blake3_servil..pipeline..Pipeline$LT$T$C$B$C$P$GT$$u20$as$u20$core..ops..drop..Drop$GT$"
        raw_legacy_drop = f"_ZN{len(legacy_impl)}{legacy_impl}4drop17h0123456789abcdefE"
        self.assertEqual(
            stack_budget.classify_symbol(raw_legacy_drop),
            ("threads", stack_budget.CAP_THREADS),
        )
        _, violations, _ = stack_budget.check_entries([(24_680, raw_legacy_drop)])
        self.assertEqual(len(violations), 1)
        self.assertIn("threads:", violations[0])

    def test_ignores_address_size_header_and_empty_functions_entry(self) -> None:
        sample = """
File: target/stack-budget/release/b3sum
Format: elf64-x86-64
Arch: x86_64
AddressSize: 64bit
StackSizes [
  Entry {
    Functions: []
    Size: 0x10
  }
  Entry {
    Functions: [_RNvNtCs760tuxHRkot_13blake3_servil5lanes7too_big]
    Size: 0x5000
  }
  Entry {
    Functions: [_RNvNtCs6LKpzGEycGt_18blake3_servil_core4tree5small]
    Size: 0x40
  }
]
"""
        entries = stack_budget.parse_llvm_readobj_stack_sizes(sample)
        self.assertEqual(
            entries,
            [
                (0x5000, "_RNvNtCs760tuxHRkot_13blake3_servil5lanes7too_big"),
                (0x40, "_RNvNtCs6LKpzGEycGt_18blake3_servil_core4tree5small"),
            ],
        )
        _, violations, budgeted = stack_budget.check_entries(entries)
        self.assertEqual(budgeted, 2)
        self.assertEqual(len(violations), 1)
        self.assertIn("threads:", violations[0])

    def test_classifies_threads_v0_and_demangled_pipeline_as_threads(self) -> None:
        for sym in (
            "_RNvMsr_NtCs760tuxHRkot_13blake3_servil8pipelineINtB5_8PipelinejuNtNtCs6LKpzGEycGt_18blake3_servil_core5block5BatchE12poll_message",
            "<blake3_servil::pipeline::Pipeline<usize, (), blake3_servil_core::block::Batch>>::poll_message",
            "<blake3_servil::pipeline::Pipeline<u32, (), blake3_servil_core::block::Batch> as core::ops::drop::Drop>::drop",
        ):
            self.assertEqual(
                stack_budget.classify_symbol(sym),
                ("threads", stack_budget.CAP_THREADS),
                sym,
            )

    def test_tree_hasher_not_classified_as_incremental_hasher(self) -> None:
        for sym in (
            "_RNvMs0_NtCs6LKpzGEycGt_18blake3_servil_core10throughputNtB5_10TreeHasher9hash_each",
            "<blake3_servil_core::throughput::TreeHasher>::hash_each",
        ):
            self.assertEqual(
                stack_budget.classify_symbol(sym),
                ("core", stack_budget.CAP_CORE_B3SUM),
                sym,
            )

    def test_b3sum_closure_vtable_and_fn_classified_as_b3sum(self) -> None:
        for sym in (
            "_RNSNvYNCNvCs9C2ZzjvkBXS_5b3sum11hash_inputss0_0INtNtNtCskADBYZwJmSU_4core3ops8function6FnOnceTjINtNtBQ_6result6ResultNtCsdKSb5PDnG5P_18blake3_servil_core4HashNtNtB8_3err5ErrorEEE9call_once6vtableB8_",
            "<b3sum::hash_inputs::{closure#2} as core::ops::function::FnOnce<(usize, core::result::Result<blake3_servil_core::Hash, b3sum::err::Error>)>>::call_once::{shim:vtable#0}",
        ):
            self.assertEqual(
                stack_budget.classify_symbol(sym),
                ("b3sum", stack_budget.CAP_CORE_B3SUM),
                sym,
            )

    def test_evaluate_exit_codes_across_all_four_buckets_and_fail_closed_parsing(self) -> None:
        base_items = [
            (0x20, "main"),
            (0x100, "_RNvCs9C2ZzjvkBXS_5b3sum4main"),
            (0x200, "_RNvMsq_CsdKSb5PDnG5P_18blake3_servil_coreNtB5_12OutputReader4fill"),
            (0x300, "_RNvMs1_NtCsdKSb5PDnG5P_18blake3_servil_core11incrementalNtB5_6Hasher8finalize"),
            (0x400, "_RNvNtCs760tuxHRkot_13blake3_servil5lanes8run_work"),
        ]
        code, headline, details = stack_budget.evaluate_stack_sizes(_make_entries(base_items))
        self.assertEqual(code, 0, headline)
        self.assertIn("4/5 symbols budgeted", headline)
        self.assertEqual(details, [])

        # Missing any single expected bucket must fail with exit code 1.
        bucket_idx = {"b3sum": 1, "core": 2, "core::Hasher": 3, "threads": 4}
        for bucket, idx in bucket_idx.items():
            dropped = list(base_items)
            dropped[idx] = (0x20, "unrelated_fn")
            code, headline, _ = stack_budget.evaluate_stack_sizes(_make_entries(dropped))
            self.assertEqual(code, 1, f"missing {bucket} must fail")
            self.assertIn(bucket, headline)

        # Exceeding cap in any single bucket must return exit code 1 with detail.
        over_caps = {
            "b3sum": (1, stack_budget.CAP_CORE_B3SUM + 8),
            "core": (2, stack_budget.CAP_CORE_B3SUM + 8),
            "core::Hasher": (3, stack_budget.CAP_HASHER + 8),
            "threads": (4, stack_budget.CAP_THREADS + 8),
        }
        for bucket, (idx, oversize) in over_caps.items():
            inflated = list(base_items)
            inflated[idx] = (oversize, inflated[idx][1])
            code, headline, details = stack_budget.evaluate_stack_sizes(_make_entries(inflated))
            self.assertEqual(code, 1, f"over-cap {bucket} must fail")
            self.assertIn("FAILED", headline)
            self.assertEqual(len(details), 1)
            self.assertIn(f"{bucket}:", details[0])

        # Fail closed on malformed v0, legacy, and unbalanced demangled shipped symbols.
        for bad_sym in (
            "_R!18blake3_servil_core_broken",
            "_ZN999blake3_servil_coreE",
            "<blake3_servil_core::OutputReader::fill",
        ):
            code, headline, _ = stack_budget.evaluate_stack_sizes(_make_entries([(0x20, bad_sym)]))
            self.assertEqual(code, 1, bad_sym)
            self.assertIn("failed to parse shipped symbol", headline, bad_sym)


if __name__ == "__main__":
    unittest.main()
