#!/usr/bin/env python3
"""Unit tests for tools/sync_readme_charts.py."""

import subprocess
import tempfile
import unittest
from pathlib import Path

import sync_readme_charts as charts

CARGO_011 = """\
[package]
name = "blake3-servil-core"
version = "0.11.0+dev"

[dependencies]
blake3-servil-version = { version = "=0.1.0" }
"""


def make_hash_svg(ver: str, measured: str = "2026-11-25 06:52:38 UTC") -> str:
    return (
        "<svg xmlns='http://www.w3.org/2000/svg'>\n"
        f"<text>AMD EPYC 9255 · measured {measured}</text>\n"
        f"<text>BLAKE3-Servil {ver}+abc · clean</text>\n"
        "</svg>\n"
    )


class SyncReadmeChartsTests(unittest.TestCase):
    def test_parse_cargo_ignores_dependency_versions(self):
        self.assertEqual(charts.parse_cargo_base_version(CARGO_011), "0.11.0")
        with self.assertRaises(ValueError):
            charts.parse_cargo_base_version('[dependencies]\nversion = "1.2.3"\n')

    def test_parse_chart_version_handles_both_naming_forms(self):
        old_style = "<svg><text>BLAKE3 servil 0.8.0 · clean</text></svg>"
        new_style = make_hash_svg("0.11.0")
        self.assertEqual(charts.parse_chart_version(old_style, "hash"), "0.8.0")
        self.assertEqual(charts.parse_chart_version(new_style, "hash"), "0.11.0")
        self.assertIsNone(charts.parse_chart_version("not an svg BLAKE3-Servil 0.11.0", "hash"))

    def test_check_chart_texts_accepts_matching_and_rejects_stale_or_missing(self):
        ok, detail = charts.check_chart_texts(CARGO_011, make_hash_svg("0.11.0"))
        self.assertTrue(ok, detail)

        ok, detail = charts.check_chart_texts(CARGO_011, make_hash_svg("0.8.0"))
        self.assertFalse(ok)
        self.assertIn("hash-epyc-128mib.svg is 0.8.0, expected 0.11.0", detail)

        ok, detail = charts.check_chart_texts(CARGO_011, None)
        self.assertFalse(ok)
        self.assertIn("hash-epyc-128mib.svg is missing", detail)

    def test_sync_picks_newest_measured_machine_and_validates(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = root / "BLAKE3"
            bench = root / "bench-hashes"
            (repo / "docs/images").mkdir(parents=True)
            (repo / "Cargo.toml").write_text(CARGO_011, encoding="utf-8")
            (repo / "docs/images/hash-epyc-128mib.svg").write_text(make_hash_svg("0.8.0"), encoding="utf-8")

            # Older kernel dir has 0.8.0; newer kernel dir has 0.11.0.
            old_dir = bench / "benchmark-results/AMDEPYC925524CoreProcessor.linux70038generic"
            new_dir = bench / "benchmark-results/AMDEPYC925524CoreProcessor.linux70043generic"
            old_dir.mkdir(parents=True)
            new_dir.mkdir(parents=True)
            (old_dir / "bench-hashes.chart.svg").write_text(make_hash_svg("0.8.0", "2026-11-03 00:13:52 UTC"), encoding="utf-8")
            (new_dir / "bench-hashes.chart.svg").write_text(make_hash_svg("0.11.0", "2026-11-25 06:52:38 UTC"), encoding="utf-8")

            self.assertEqual(charts.main(["--repo", str(repo), "--check"]), 1)
            self.assertEqual(charts.main(["--repo", str(repo), "--bench", str(bench)]), 0)
            self.assertEqual(charts.main(["--repo", str(repo), "--check"]), 0)
            self.assertIn("0.11.0", (repo / "docs/images/hash-epyc-128mib.svg").read_text(encoding="utf-8"))

            # Refuses to overwrite with stale charts unless --allow-mismatch is passed.
            (repo / "Cargo.toml").write_text('[package]\nname = "blake3-servil-core"\nversion = "0.12.0+dev"\n', encoding="utf-8")
            with self.assertRaises(SystemExit):
                charts.sync_charts(repo, bench)

    def test_sync_from_git_ref_and_check_commit(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bench = root / "bench-hashes"
            bench.mkdir()
            subprocess.run(["git", "-C", str(bench), "init", "-q", "-b", "next"], check=True)
            subprocess.run(["git", "-C", str(bench), "config", "user.name", "Test"], check=True)
            subprocess.run(["git", "-C", str(bench), "config", "user.email", "t@example.com"], check=True)
            subprocess.run(["git", "-C", str(bench), "config", "commit.gpgsign", "false"], check=True)
            d = bench / "benchmark-results/AMDEPYC925524CoreProcessor.linux70043generic"
            d.mkdir(parents=True)
            (d / "bench-hashes.chart.svg").write_text(make_hash_svg("0.11.0"), encoding="utf-8")
            subprocess.run(["git", "-C", str(bench), "add", "."], check=True)
            subprocess.run(["git", "-C", str(bench), "commit", "-qm", "charts"], check=True)

            repo = root / "BLAKE3"
            repo.mkdir()
            (repo / "Cargo.toml").write_text(CARGO_011, encoding="utf-8")
            src, _ = charts.sync_charts(repo, bench, git_ref="next")
            self.assertEqual(src, "AMDEPYC925524CoreProcessor.linux70043generic")
            ok, detail = charts.check_repo(repo)
            self.assertTrue(ok, detail)

    def test_live_repo_charts_are_valid_svgs(self):
        repo_root = Path(__file__).resolve().parent.parent
        for kind, rel_path, _ in charts.CHARTS:
            text = (repo_root / rel_path).read_text(encoding="utf-8")
            self.assertIsNotNone(charts.parse_chart_version(text, kind), f"{rel_path} lacks valid version footer")
            self.assertTrue(charts.parse_measured_timestamp(text), f"{rel_path} lacks measured timestamp")


if __name__ == "__main__":
    unittest.main()
