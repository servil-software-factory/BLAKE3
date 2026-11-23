"""python3 tools/test_dependency_policy.py: unit tests for tools/dependency_policy.py (`strangers` and
`strip_toml_crate_sections`)."""
import os
import sys
import tomllib
import unittest
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dependency_policy as policy

CHECKOUT = Path("/stage/checkout")
MEMBERS_LIST = ["checkout/version", "checkout", "checkout/threads", "checkout/b3sum", "checkout/capi", "checkout/test_vectors"]
REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"


def package(name, folder=None, source=None):
    """A package as `cargo metadata` gives it: one under CHECKOUT by folder, or one with a source."""
    place = CHECKOUT / folder if folder else Path("/stage/registry") / name
    return {"id": f"{name}#1.0.0", "name": name, "version": "1.0.0", "source": source,
            "manifest_path": str(place / "Cargo.toml")}


CLI_OURS = [package("blake3-servil-version", "version"), package("blake3-servil-core", "."),
            package("blake3-servil", "threads"), package("b3sums", "b3sum")]
OURS = CLI_OURS + [package("blake3-servil-c", "capi"), package("test_vectors", "test_vectors")]
ALL_SIX = set(policy.OURS)
ALL_SIX_IDS = [p["id"] for p in OURS]


class Strangers(unittest.TestCase):
    def test_the_six_crates_and_packages_from_a_registry_are_no_strangers(self):
        metadata = {"packages": OURS + [package("anyhow", source=REGISTRY)], "workspace_members": ALL_SIX_IDS}
        self.assertEqual(policy.strangers(metadata, CHECKOUT, ALL_SIX, workspace_members_list=MEMBERS_LIST), [])

    def test_standalone_cli_has_one_workspace_member_and_all_four_cli_crates(self):
        metadata = {"packages": CLI_OURS + [package("anyhow", source=REGISTRY)], "workspace_members": ["b3sums#1.0.0"]}
        self.assertEqual(policy.strangers(metadata, CHECKOUT, {"b3sums"}, ours=policy.CLI_OURS), [])

    def test_a_package_taken_by_path_that_is_no_member_is_named_with_its_folder(self):
        metadata = {"packages": OURS + [package("copied", "third-party/copied")], "workspace_members": ALL_SIX_IDS}
        wrong = policy.strangers(metadata, CHECKOUT, ALL_SIX, workspace_members_list=MEMBERS_LIST)
        self.assertEqual(len(wrong), 1)
        self.assertIn("copied 1.0.0 is taken by path, from third-party/copied/", wrong[0])

    def test_an_implicit_extra_workspace_member_is_caught(self):
        extra = package("copied", "copied")
        metadata = {"packages": OURS + [extra], "workspace_members": ALL_SIX_IDS + [extra["id"]]}
        wrong = policy.strangers(metadata, CHECKOUT, ALL_SIX, workspace_members_list=MEMBERS_LIST)
        self.assertTrue(any("Cargo workspace_members are" in w for w in wrong), wrong)
        self.assertTrue(any("copied 1.0.0 is taken by path" in w for w in wrong), wrong)

    def test_an_extra_member_added_to_workspace_toml_is_caught_too(self):
        extra = package("copied", "copied")
        metadata = {"packages": OURS + [extra], "workspace_members": ALL_SIX_IDS + [extra["id"]]}
        wrong = policy.strangers(metadata, CHECKOUT, ALL_SIX, workspace_members_list=MEMBERS_LIST + ["checkout/copied"])
        self.assertTrue(any("workspace.toml lists members" in w for w in wrong), wrong)
        self.assertTrue(any("copied 1.0.0 is taken by path" in w for w in wrong), wrong)

    def test_a_package_with_an_expected_name_in_another_folder_is_a_stranger(self):
        elsewhere = package("b3sums", "copy/b3sum")
        metadata = {"packages": OURS[:3] + [elsewhere] + OURS[4:], "workspace_members": ALL_SIX_IDS}
        wrong = policy.strangers(metadata, CHECKOUT, ALL_SIX, workspace_members_list=MEMBERS_LIST)
        self.assertTrue(any("b3sums 1.0.0 is taken by path, from copy/b3sum/" in w for w in wrong), wrong)
        self.assertTrue(any("expected crate b3sums at b3sum/Cargo.toml is absent" in w for w in wrong), wrong)

    def test_a_package_in_an_expected_folder_with_another_name_is_a_stranger(self):
        renamed = package("not-b3sums", "b3sum")
        metadata = {"packages": OURS[:3] + [renamed] + OURS[4:], "workspace_members": ALL_SIX_IDS[:3] + [renamed["id"]] + ALL_SIX_IDS[4:]}
        wrong = policy.strangers(metadata, CHECKOUT, ALL_SIX, workspace_members_list=MEMBERS_LIST)
        self.assertTrue(any("not-b3sums 1.0.0 is taken by path, from b3sum/" in w for w in wrong), wrong)
        self.assertTrue(any("expected crate b3sums at b3sum/Cargo.toml is absent" in w for w in wrong), wrong)

    def test_a_crate_that_the_graph_lacks_is_said(self):
        metadata = {"packages": OURS[:3] + OURS[4:], "workspace_members": ALL_SIX_IDS[:3] + ALL_SIX_IDS[4:]}
        wrong = policy.strangers(metadata, CHECKOUT, ALL_SIX, workspace_members_list=MEMBERS_LIST)
        self.assertTrue(any("expected crate b3sums at b3sum/Cargo.toml is absent" in w for w in wrong), wrong)


class StripEvidence(unittest.TestCase):
    def test_strips_exemptions_local_audits_imports_and_trust_while_keeping_neighbours(self):
        sample = (
            '[policy.anyhow]\nnotes = """\n[not.a.header]\n"""\n\n'
            '[[exemptions.anyhow]]\nversion = "1.0.104"\ncriteria = "safe-to-deploy"\n\n'
            '[[audits.anyhow]]\nwho = "Devon"\ncriteria = "safe-to-deploy"\nversion = "1.0.104"\nnotes = """\n[keep-inside]\n"""\n\n'
            '[[audits.google.audits.anyhow]]\ncriteria = "safe-to-deploy"\nversion = "1.0.104"\n\n'
            '[[trusted.anyhow]]\nuser-id = 1\n\n'
            '[[exemptions.anyhow-other]]\nversion = "1.0.0"\ncriteria = "safe-to-run"\n'
        )
        got = policy.strip_toml_crate_sections(sample, "anyhow")
        self.assertIn("[policy.anyhow]", got)
        self.assertIn("[not.a.header]", got)
        self.assertIn("[[exemptions.anyhow-other]]", got)
        self.assertNotIn("[[exemptions.anyhow]]", got)
        self.assertNotIn("[[audits.anyhow]]", got)
        self.assertNotIn("[[audits.google.audits.anyhow]]", got)
        self.assertNotIn("[[trusted.anyhow]]", got)
        self.assertNotIn("[keep-inside]", got)


class AgentCriteriaIsolation(unittest.TestCase):
    def test_agent_criteria_do_not_imply_builtin_human_criteria(self):
        repo_root = Path(__file__).resolve().parent.parent
        audits = tomllib.loads((repo_root / "tools" / "release" / "supply-chain" / "audits.toml").read_text())
        builtins = {"safe-to-deploy", "safe-to-run"}
        for name, entry in audits.get("criteria", {}).items():
            implies = entry.get("implies", [])
            if isinstance(implies, str):
                implies = [implies]
            self.assertEqual(
                set(implies) & builtins,
                set(),
                f"custom criterion {name} must stay separate from built-in {builtins}",
            )
        for crate, entries in audits.get("audits", {}).items():
            for entry in entries:
                criteria = entry.get("criteria", [])
                if isinstance(criteria, str):
                    criteria = [criteria]
                self.assertEqual(
                    set(criteria) & builtins,
                    set(),
                    f"local agent-authored audit for {crate} must use a custom agent-* criterion",
                )


if __name__ == "__main__":
    unittest.main()
