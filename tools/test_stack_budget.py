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

    def test_fails_when_hasher_exceeds_32k_in_v0_legacy_or_demangled_forms(self) -> None:
        for sym in (
            "_RNvMs1_NtCsdKSb5PDnG5P_18blake3_servil_core11incrementalNtB5_6Hasher8finalize.llvm.11097525931525558011",
            "_ZN18blake3_servil_core11convenience6Hasher8finalize17h0000000000000000E",
            "blake3_servil_core::convenience::Hasher::finalize",
            "<blake3_servil_core::incremental::Hasher>::finalize (.llvm.42)",
        ):
            _, violations, _ = stack_budget.check_entries([(32 * 1024 + 8, sym)])
            self.assertEqual(len(violations), 1, sym)
            self.assertIn("core::Hasher:", violations[0])

    def test_hasher_update_methods_take_core_cap(self) -> None:
        for sym in (
            "_RNvMs1_NtCsdKSb5PDnG5P_18blake3_servil_core11incrementalNtB5_6Hasher6update.llvm.11097525931525558011",
            "_ZN18blake3_servil_core11incremental6Hasher13update_reader17h0000000000000000E",
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
        # neither receiver (`[u8; 32]`) nor trait (`core::convert::From`) is local,
        # yet the v0 impl path encodes `blake3_servil_core`.
        raw_from_hash = "_RNvXs0_CsdKSb5PDnG5P_18blake3_servil_coreAhj20_INtNtCskADBYZwJmSU_4core7convert4FromNtB5_4HashE4from"
        self.assertEqual(
            stack_budget.classify_symbol(raw_from_hash),
            ("core", stack_budget.CAP_CORE_B3SUM),
        )
        _, violations, _ = stack_budget.check_entries([(150_040, raw_from_hash)])
        self.assertEqual(len(violations), 1)
        self.assertIn("core:", violations[0])

        # Also in demangled form (e.g. legacy mangling):
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

    def test_ignores_stdlib_generics_mentioning_shipped_crates(self) -> None:
        for sym in (
            "_RINvNtCskADBYZwJmSU_4core3ptr9drop_glueNtCs9C2ZzjvkBXS_5b3sum13DirectPermitsEBD_",
            "_RNCINvMs0_NtNtCs8ZKwes1zzDd_3std4sync4onceNtB8_4Once9call_onceNCNvCs9C2ZzjvkBXS_5b3sum14init_pool_once0E0B12_.llvm.11097525931525558011",
            "core::slice::sort::stable::driftsort_main::<blake3_servil::lanes::places::assign::{closure#0}>",
            "core::ptr::drop_glue::<b3sum::DirectPermits>",
            "<std::sync::mpmc::list::Channel<(usize, blake3_servil_core::Hash)> as core::ops::drop::Drop>::drop",
        ):
            self.assertIsNone(stack_budget.classify_symbol(sym), sym)

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

    def test_evaluate_reports_counts_and_fails_on_missing_bucket_or_bad_shipped_symbol(self) -> None:
        mixed_ok = """
StackSizes [
  Entry {
    Functions: [main]
    Size: 0x20
  }
  Entry {
    Functions: [_RNvCs9C2ZzjvkBXS_5b3sum4main]
    Size: 0x100
  }
  Entry {
    Functions: [_RNvMsq_CsdKSb5PDnG5P_18blake3_servil_coreNtB5_12OutputReader4fill]
    Size: 0x200
  }
  Entry {
    Functions: [_RNvMs1_NtCsdKSb5PDnG5P_18blake3_servil_core11incrementalNtB5_6Hasher8finalize]
    Size: 0x300
  }
  Entry {
    Functions: [_RNvNtCs760tuxHRkot_13blake3_servil5lanes8run_work]
    Size: 0x400
  }
]
"""
        code, headline, details = stack_budget.evaluate_stack_sizes(mixed_ok)
        self.assertEqual(code, 0, headline)
        self.assertIn("4/5 symbols budgeted", headline)
        self.assertEqual(details, [])

        missing_threads = mixed_ok.replace("_RNvNtCs760tuxHRkot_13blake3_servil5lanes8run_work", "other_fn")
        code, headline, _ = stack_budget.evaluate_stack_sizes(missing_threads)
        self.assertEqual(code, 1)
        self.assertIn("threads", headline)

        malformed_shipped = """
StackSizes [
  Entry {
    Functions: [_R!18blake3_servil_core_broken]
    Size: 0x20
  }
]
"""
        code, headline, _ = stack_budget.evaluate_stack_sizes(malformed_shipped)
        self.assertEqual(code, 1)
        self.assertIn("failed to parse shipped symbol", headline)


if __name__ == "__main__":
    unittest.main()
