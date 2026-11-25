#!/usr/bin/env python3
"""Unit tests for tools/stack_budget.py."""

import shutil
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

    def test_fails_when_hasher_exceeds_32k_in_angle_or_plain_form(self) -> None:
        for sym in (
            "blake3_servil_core::convenience::Hasher::finalize",
            "<blake3_servil_core::incremental::Hasher>::finalize (.llvm.42)",
        ):
            _, violations, _ = stack_budget.check_entries([(32 * 1024 + 8, sym)])
            self.assertEqual(len(violations), 1, sym)
            self.assertIn("core::Hasher:", violations[0])

    def test_hasher_update_methods_take_core_cap(self) -> None:
        for sym in (
            "<blake3_servil_core::incremental::Hasher>::update (.llvm.11097525931525558011)",
            "<blake3_servil_core::incremental::Hasher>::update_with_join",
            "blake3_servil_core::convenience::Hasher::update_rayon",
        ):
            self.assertEqual(
                stack_budget.classify_symbol(sym),
                ("core", stack_budget.CAP_CORE_B3SUM),
                sym,
            )

    def test_fails_when_core_exceeds_128k_including_output_reader_fill(self) -> None:
        for sym in (
            "blake3_servil_core::tree::too_deep",
            "<blake3_servil_core::OutputReader>::fill",
        ):
            _, violations, _ = stack_budget.check_entries([(128 * 1024 + 16, sym)])
            self.assertEqual(len(violations), 1, sym)
            self.assertIn("core:", violations[0])

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
    Functions: [blake3_servil::lanes::too_big]
    Size: 0x5000
  }
  Entry {
    Functions: [blake3_servil_core::tree::small]
    Size: 0x40
  }
]
"""
        entries = stack_budget.parse_llvm_readobj_stack_sizes(sample)
        self.assertEqual(
            entries,
            [
                (0x5000, "blake3_servil::lanes::too_big"),
                (0x40, "blake3_servil_core::tree::small"),
            ],
        )
        _, violations, budgeted = stack_budget.check_entries(entries)
        self.assertEqual(budgeted, 2)
        self.assertEqual(len(violations), 1)
        self.assertIn("threads:", violations[0])

    def test_classifies_threads_inherent_and_trait_impls_as_threads(self) -> None:
        for sym in (
            "<blake3_servil::pipeline::Pipeline<usize, (), blake3_servil_core::block::Batch>>::poll_message",
            "<blake3_servil::pipeline::Pipeline<u32, (), blake3_servil_core::block::Batch> as core::ops::drop::Drop>::drop",
        ):
            self.assertEqual(
                stack_budget.classify_symbol(sym),
                ("threads", stack_budget.CAP_THREADS),
                sym,
            )
            _, violations, _ = stack_budget.check_entries([(24 * 1024, sym)])
            self.assertEqual(len(violations), 1)
            self.assertIn("threads:", violations[0])

    def test_tree_hasher_not_classified_as_incremental_hasher(self) -> None:
        sym = "<blake3_servil_core::throughput::TreeHasher>::hash_each"
        self.assertEqual(stack_budget.classify_symbol(sym), ("core", stack_budget.CAP_CORE_B3SUM))

    def test_ignores_stdlib_generics_mentioning_shipped_crates(self) -> None:
        for sym in (
            "core::slice::sort::stable::driftsort_main::<blake3_servil::lanes::places::assign::{closure#0}>",
            "core::ptr::drop_glue::<b3sum::DirectPermits>",
            "<std::sync::mpmc::list::Channel<(usize, blake3_servil_core::Hash)> as core::ops::drop::Drop>::drop",
            "<std::sync::once::Once>::call_once::<b3sum::init_pool_once::{closure#0}>::{closure#0}",
        ):
            self.assertIsNone(stack_budget.classify_symbol(sym), sym)

    def test_b3sum_closure_trait_impl_classified_as_b3sum(self) -> None:
        sym = "<b3sum::hash_inputs::{closure#2} as core::ops::function::FnOnce<(usize, core::result::Result<blake3_servil_core::Hash, b3sum::err::Error>)>>::call_once::{shim:vtable#0}"
        self.assertEqual(stack_budget.classify_symbol(sym), ("b3sum", stack_budget.CAP_CORE_B3SUM))

    @unittest.skipUnless(shutil.which("llvm-cxxfilt"), "needs llvm-cxxfilt")
    def test_demangles_real_v0_and_legacy_elf_symbols(self) -> None:
        # Both real ELF symbols from #557: v0 inherent impl (`OutputReader::fill`)
        # and legacy `$LT$...$GT$` trait impl (`Pipeline as Drop::drop`).
        raw_v0_output_reader = "_RNvMs0_Cs6LKpzGEycGt_18blake3_servil_coreNtB5_12OutputReader4fill"
        raw_v0_pipeline = "_RNvMsr_NtCs760tuxHRkot_13blake3_servil8pipelineINtB5_8PipelinejuNtNtCs6LKpzGEycGt_18blake3_servil_core5block5BatchE12poll_message"
        legacy_impl = "_$LT$blake3_servil..pipeline..Pipeline$u20$as$u20$core..ops..drop..Drop$GT$"
        raw_legacy_drop = f"_ZN{len(legacy_impl)}{legacy_impl}4drop17h0123456789abcdefE"

        demangled = stack_budget.demangle_symbols(
            [raw_v0_output_reader, raw_v0_pipeline, raw_legacy_drop],
            cmd=[shutil.which("llvm-cxxfilt")],
        )
        self.assertEqual(demangled[0], "<blake3_servil_core::OutputReader>::fill")
        self.assertEqual(
            stack_budget.classify_symbol(demangled[0]),
            ("core", stack_budget.CAP_CORE_B3SUM),
        )
        self.assertEqual(
            stack_budget.classify_symbol(demangled[1]),
            ("threads", stack_budget.CAP_THREADS),
        )
        self.assertEqual(
            stack_budget.classify_symbol(demangled[2]),
            ("threads", stack_budget.CAP_THREADS),
        )
        _, violations, _ = stack_budget.check_entries(
            [(143_496, demangled[0]), (24_680, demangled[2])]
        )
        self.assertEqual(len(violations), 2)
        self.assertIn("core:", violations[0])
        self.assertIn("threads:", violations[1])


if __name__ == "__main__":
    unittest.main()
