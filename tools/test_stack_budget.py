#!/usr/bin/env python3
"""Unit tests for tools/stack_budget.py."""

import unittest

import stack_budget


SAMPLE_FUNCS_FIRST = """
StackSizes [
  Entry {
    Functions: [_RNvNtCs6LKpzGEycGt_18blake3_servil_core4tree9hash_tree]
    Size: 0x2000
  }
  Entry {
    Functions: [_RNvMs1_NtCsdKSb5PDnG5P_18blake3_servil_core11incrementalNtB5_6Hasher8finalize.llvm.11097525931525558011]
    Size: 0x1000
  }
  Entry {
    Functions: [_RNvNtCs760tuxHRkot_13blake3_servil5lanes8run_work]
    Size: 0x800
  }
  Entry {
    Functions: [_RNvCs9C2ZzjvkBXS_5b3sum4main]
    Size: 0x400
  }
]
"""

SAMPLE_SIZE_FIRST = """
StackSizes [
  Entry {
    Size: 0x4001
    Functions: [_RNvNtCs760tuxHRkot_13blake3_servil5lanes7too_big]
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
        self.assertEqual(budgeted, 3)
        self.assertEqual(max_by_bucket["core"][0], 0x2000)
        self.assertEqual(max_by_bucket["core::Hasher"][0], 0x1000)
        self.assertEqual(max_by_bucket["threads"][0], 0x800)

    def test_fails_when_threads_exceeds_16k(self) -> None:
        entries = stack_budget.parse_llvm_readobj_stack_sizes(SAMPLE_SIZE_FIRST)
        _, violations, _ = stack_budget.check_entries(entries)
        self.assertEqual(len(violations), 1)
        self.assertIn("threads:", violations[0])
        self.assertIn("16385 B > 16384 B", violations[0])

    def test_fails_when_hasher_non_update_methods_or_closures_exceed_32k(self) -> None:
        for sym in (
            "_RNvMs1_NtCsdKSb5PDnG5P_18blake3_servil_core11incrementalNtB5_6Hasher8finalize.llvm.11097525931525558011",
            "_RNvMs1_NtCsdKSb5PDnG5P_18blake3_servil_core11incrementalNtB5_6Hasher12finalize_xof.cold",
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

    def test_v0_impl_provenance_for_foreign_receiver_and_inlined_std_trait_methods(self) -> None:
        # Local `impl From<Hash> for [u8; 32]` defined in blake3_servil_core:
        raw_from_hash = "_RNvXs0_CsdKSb5PDnG5P_18blake3_servil_coreAhj20_INtNtCskADBYZwJmSU_4core7convert4FromNtB5_4HashE4from"
        self.assertEqual(
            stack_budget.classify_symbol(raw_from_hash),
            ("core", stack_budget.CAP_CORE_B3SUM),
        )
        _, violations, _ = stack_budget.check_entries([(150_040, raw_from_hash)])
        self.assertEqual(len(violations), 1)
        self.assertIn("core:", violations[0])

        # Shipped `impl std::io::Read for OutputReader` inlined into `Take<OutputReader>::read_buf`
        # must be budgeted under `core` (128 KiB) so a 300 KiB local cannot escape.
        raw_take_read = (
            "_RNvXs3_NtNtCsgFX3jaOk65H_5alloc2io4util"
            "INtB5_4TakeNtCsdKSb5PDnG5P_18blake3_servil_core12OutputReaderE"
            "NtNtB9_4read4Read8read_buf"
        )
        self.assertEqual(
            stack_budget.classify_symbol(raw_take_read),
            ("core", stack_budget.CAP_CORE_B3SUM),
        )
        _, violations, _ = stack_budget.check_entries([(307_272, raw_take_read)])
        self.assertEqual(len(violations), 1)
        self.assertIn("core:", violations[0])

        # Shipped `<RoomFut as Future>::poll` inlined into `<Pin<&mut RoomFut> as Future>::poll`
        # must be budgeted under `threads` (16 KiB).
        raw_pin_roomfut = (
            "_RNvXs0_NtCskADBYZwJmSU_4core6future"
            "INtNtCskADBYZwJmSU_4core3pin3PinQNtNtCs760tuxHRkot_13blake3_servil8pipeline7RoomFutE"
            "NtNtCskADBYZwJmSU_4core6future6Future4poll"
        )
        self.assertEqual(
            stack_budget.classify_symbol(raw_pin_roomfut),
            ("threads", stack_budget.CAP_THREADS),
        )
        _, violations, _ = stack_budget.check_entries([(20_520, raw_pin_roomfut)])
        self.assertEqual(len(violations), 1)
        self.assertIn("threads:", violations[0])

        # Pure stdlib symbol mentioning a shipped crate ONLY as its trailing instantiating crate
        # must NOT be budgeted (returns None).
        raw_std_instantiated_in_b3sum = "_RINvNtCskADBYZwJmSU_4core3mem4swapxECs9C2ZzjvkBXS_5b3sum"
        self.assertIsNone(stack_budget.classify_symbol(raw_std_instantiated_in_b3sum))

    def test_inlined_shipped_closures_fn_items_and_tightest_cap_selection(self) -> None:
        # Real ELF symbol: `LocalKey::with` inlining `blake3_servil::lanes::with_scratch::{closure}`
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

        # Real ELF symbol: pool worker body `__rust_begin_short_backtrace::<initialize::{closure#1}>`
        raw_worker_body = "_RINvNtNtCs8ZKwes1zzDd_3std3sys9backtrace28___rust_begin_short_backtraceNCNvNtCskmg1lIIckmr_13blake3_servil5lanes10initializes1_0uEB1d_"
        self.assertEqual(
            stack_budget.classify_symbol(raw_worker_body),
            ("threads", stack_budget.CAP_THREADS),
        )

        # Shipped closure inside a tuple generic arg, fn item inside an array generic arg,
        # and blanket `<&mut F as FnMut>::call_mut` where F is a shipped closure:
        raw_tuple_closure = "_RINvNtCskADBYZwJmSU_4core3mem4dropTNCNvNtCs760tuxHRkot_13blake3_servil5lanes3foo0jEE"
        self.assertEqual(stack_budget.classify_symbol(raw_tuple_closure), ("threads", stack_budget.CAP_THREADS))

        raw_array_fn_item = "_RINvNtCskADBYZwJmSU_4core3mem4dropANvCsdKSb5PDnG5P_18blake3_servil_core4workj2_E"
        self.assertEqual(stack_budget.classify_symbol(raw_array_fn_item), ("core", stack_budget.CAP_CORE_B3SUM))

        raw_fnmut_ref = (
            "_RNvXs0_NtNtCskADBYZwJmSU_4core3ops8function"
            "QNCNvNtCs760tuxHRkot_13blake3_servil5lanes3foo0"
            "INtB5_5FnMutTuEE8call_mut"
        )
        self.assertEqual(stack_budget.classify_symbol(raw_fnmut_ref), ("threads", stack_budget.CAP_THREADS))

        # Tightest cap selection: std generic mentioning BOTH `b3sum` (128 KiB) and `threads` (16 KiB)
        # must choose the tighter 16 KiB `threads` cap (regardless of argument order).
        raw_mixed_b3sum_then_threads = (
            "_RINvNtCskADBYZwJmSU_4core3mem4swap"
            "TNtCs9C2ZzjvkBXS_5b3sum4ArgsNtNtCs760tuxHRkot_13blake3_servil5lanes7ScratchEE"
        )
        self.assertEqual(
            stack_budget.classify_symbol(raw_mixed_b3sum_then_threads),
            ("threads", stack_budget.CAP_THREADS),
        )
        demangled_mixed = "std::foo::<b3sum::Args, blake3_servil::lanes::Scratch>"
        self.assertEqual(
            stack_budget.classify_symbol(demangled_mixed),
            ("threads", stack_budget.CAP_THREADS),
        )

    def test_v0_parses_every_rfc2603_basic_type_and_rejects_cyclic_backref(self) -> None:
        # Literal 21 RFC 2603 single-letter basic types (do not read from module under test).
        rfc2603_basic_types = "abcdefhijlmnopstuvxyz"
        self.assertEqual(len(rfc2603_basic_types), 21)
        for tag in rfc2603_basic_types:
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

    def test_evaluate_exit_codes_across_all_three_buckets_and_fail_closed_parsing(self) -> None:
        base_items = [
            (0x20, "main"),
            (0x200, "_RNvMsq_CsdKSb5PDnG5P_18blake3_servil_coreNtB5_12OutputReader4fill"),
            (0x300, "_RNvMs1_NtCsdKSb5PDnG5P_18blake3_servil_core11incrementalNtB5_6Hasher8finalize"),
            (0x400, "_RNvNtCs760tuxHRkot_13blake3_servil5lanes8run_work"),
        ]
        code, headline, details = stack_budget.evaluate_stack_sizes(_make_entries(base_items))
        self.assertEqual(code, 0, headline)
        self.assertIn("3/4 symbols budgeted", headline)
        self.assertEqual(details, [])

        # Missing any single expected bucket must fail with exit code 1.
        bucket_idx = {"core": 1, "core::Hasher": 2, "threads": 3}
        for bucket, idx in bucket_idx.items():
            dropped = list(base_items)
            dropped[idx] = (0x20, "unrelated_fn")
            code, headline, _ = stack_budget.evaluate_stack_sizes(_make_entries(dropped))
            self.assertEqual(code, 1, f"missing {bucket} must fail")
            self.assertIn(bucket, headline)

        # Exceeding cap in any single bucket must return exit code 1 with detail.
        over_caps = {
            "core": (1, stack_budget.CAP_CORE_B3SUM + 8),
            "core::Hasher": (2, stack_budget.CAP_HASHER + 8),
            "threads": (3, stack_budget.CAP_THREADS + 8),
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

    def test_collect_build_artifacts_uses_cargo_json_only(self) -> None:
        import json
        from pathlib import Path
        lines = [
            "non-json banner",
            json.dumps({
                "reason": "compiler-artifact",
                "target": {"name": "blake3_servil_core", "kind": ["lib"]},
                "filenames": ["/tmp/target/release/deps/libblake3_servil_core-aaa.rlib", "/tmp/target/release/deps/libblake3_servil_core-aaa.rmeta"],
            }),
            json.dumps({
                "reason": "compiler-artifact",
                "target": {"name": "blake3_servil", "kind": ["lib"]},
                "filenames": ["/tmp/target/release/deps/libblake3_servil-bbb.rlib"],
            }),
            json.dumps({
                "reason": "compiler-artifact",
                "target": {"name": "stack_budget_probe", "kind": ["example"]},
                "executable": "/tmp/target/release/examples/stack_budget_probe",
                "filenames": ["/tmp/target/release/examples/stack_budget_probe"],
            }),
            json.dumps({"reason": "build-finished", "success": True}),
        ]
        got = stack_budget.collect_build_artifacts("\n".join(lines))
        self.assertEqual(
            got,
            [
                Path("/tmp/target/release/examples/stack_budget_probe"),
                Path("/tmp/target/release/deps/libblake3_servil_core-aaa.rlib"),
                Path("/tmp/target/release/deps/libblake3_servil-bbb.rlib"),
            ],
        )


if __name__ == "__main__":
    unittest.main()
