"""tools/land.py's pure parts: which gates a change needs, and how a
commit's statuses and check runs read as gates."""

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import land


class Gates(unittest.TestCase):
    def test_documents_alone_need_no_gate(self):
        self.assertEqual(land.required(["AGENTS.md", "docs/core-api.md"]), ())

    def test_code_needs_every_gate_and_proof_inputs_the_proofs(self):
        base = ("CI", "check-builds", "mutants x86_64", "mutants aarch64", "mutants arm64", "tests aarch64", "tests arm64", "cells")
        shipped = base + ("review",)
        self.assertEqual(land.required(["src/lib.rs", "README.md"]), shipped)
        for path in ("threads/src/pipeline.rs", "b3sum/src/main.rs", "capi/b3.h", "capi/src/lib.rs", "build.rs", "Cargo.toml", "version/src/lib.rs"):
            self.assertEqual(land.required([path]), shipped, path)
        for path in ("tools/land.py", "tools/runner/runner.py", "tools/mutants/known-arm64.txt", ".github/workflows/ci.yml", "tests/lifecycle.rs", "b3sum/tests/cli_tests.rs"):
            self.assertEqual(land.required([path]), base, f"not shipped code: {path}")
        self.assertEqual(land.required(["c/blake3_neon_hybrid_aarch64.S"]), shipped + ("long proofs",))
        for path in ("tools/verify/canon.py", "tools/gen_neon_hybrid.py", "c2sp/BLAKE3/Blake3.lean"):
            self.assertEqual(land.required([path]), base + ("long proofs",), path)

    def test_the_newest_status_of_a_gate_stands(self):
        statuses = [{"context": "cells", "state": "success", "description": "new"}, {"context": "cells", "state": "failure", "description": "old"}]
        self.assertEqual(land.verdicts(statuses, [])["cells"], ("success", "new"))

    def test_ci_is_green_only_when_every_job_is(self):
        run = lambda *c: [{"conclusion": x} for x in c]
        self.assertNotIn("CI", land.verdicts([], []), "no job has run: the gate is missing")
        self.assertEqual(land.verdicts([], run("success", "success"))["CI"][0], "success")
        self.assertEqual(land.verdicts([], run("success", "failure"))["CI"][0], "failure")
        self.assertEqual(land.verdicts([], run("success", None))["CI"][0], "pending")
        self.assertEqual(land.verdicts([], run("success", "cancelled"))["CI"][0], "failure")

    def test_a_status_named_ci_is_not_the_tests(self):
        """A commit status of the context "CI" stood for the workflow
        where no run of it was found (a reviewer's input)."""
        posted = [{"context": "CI", "state": "success", "description": "posted by something else"}]
        self.assertNotIn("CI", land.verdicts(posted, []))
        self.assertEqual(land.verdicts(posted, [{"conclusion": "failure"}])["CI"][0], "failure")


class TheTestsWorkflow(unittest.TestCase):
    SHA = "a" * 40

    def lists(self, runs, jobs):
        """GitHub's answers: the workflow runs on a commit, and each run's jobs."""
        asked = []

        def pages(path, key):
            asked.append(path)
            if path.startswith("actions/runs?head_sha="):
                self.assertEqual((path, key), (f"actions/runs?head_sha={self.SHA}", "workflow_runs"))
                return runs
            self.assertEqual(key, "jobs")
            return jobs[path]

        return pages, asked

    def run_of(self, number, path=land.TESTS, event="workflow_dispatch", sha=None, status="completed", conclusion="success"):
        return {"id": 1000 + number, "run_number": number, "path": path, "event": event, "head_sha": sha or self.SHA, "status": status, "conclusion": conclusion}

    def test_a_run_that_has_not_ended_in_success_is_not_green(self):
        """One green job of a run still in progress, or of a run that
        failed, read as "1 of 1 jobs green"."""
        green = [{"conclusion": "success"}]
        for status, conclusion, want in (("in_progress", None, "pending"), ("queued", None, "pending"), ("completed", "failure", "failure"), ("completed", "cancelled", "failure"), ("completed", "success", "success")):
            pages, _ = self.lists([self.run_of(1, status=status, conclusion=conclusion)], {"actions/runs/1001/jobs?filter=latest": green})
            self.assertEqual(land.verdicts([], land.tests_jobs(self.SHA, pages))["CI"][0], want, (status, conclusion))

    def test_a_list_that_gives_fewer_than_it_says_is_no_answer(self):
        answers = {"actions/runs/1/jobs?filter=latest&per_page=100&page=1": {"total_count": 14, "jobs": [{"conclusion": "success"}]}}
        with self.assertRaises(SystemExit):
            land.pages("actions/runs/1/jobs?filter=latest", "jobs", api=answers.__getitem__)
        whole = {"x?per_page=100&page=1": {"total_count": 1, "jobs": [{"conclusion": "success"}]}}
        self.assertEqual(len(land.pages("x", "jobs", api=whole.__getitem__)), 1)

    def test_a_commit_the_tests_never_ran_on_has_no_ci(self):
        pages, _ = self.lists([], {})
        self.assertEqual(land.tests_jobs(self.SHA, pages), [])
        self.assertNotIn("CI", land.verdicts([], land.tests_jobs(self.SHA, pages)))

    def test_another_workflows_green_jobs_are_not_the_tests(self):
        """Any green job on the commit read as CI: a documents workflow
        that alone had run there made the gate pass."""
        pages, asked = self.lists([self.run_of(7, path=".github/workflows/docs.yml")], {})
        self.assertEqual(land.tests_jobs(self.SHA, pages), [])
        self.assertEqual(asked, [f"actions/runs?head_sha={self.SHA}"], "no other run's jobs are read")

    def test_a_pull_requests_run_and_another_commits_are_left_out(self):
        pages, _ = self.lists([self.run_of(8, event="pull_request"), self.run_of(9, sha="b" * 40)], {})
        self.assertEqual(land.tests_jobs(self.SHA, pages), [])

    def test_the_newest_run_of_the_tests_is_the_one_read(self):
        green = [{"conclusion": "success"}] * 14
        failed = [{"conclusion": "success"}, {"conclusion": "failure"}]
        jobs = {"actions/runs/1003/jobs?filter=latest": failed, "actions/runs/1012/jobs?filter=latest": green, "actions/runs/1005/jobs?filter=latest": failed}
        pages, asked = self.lists([self.run_of(3), self.run_of(12), self.run_of(5), self.run_of(20, path=".github/workflows/docs.yml")], jobs)
        self.assertEqual(land.verdicts([], land.tests_jobs(self.SHA, pages))["CI"], ("success", "14 of 14 jobs green"))
        self.assertEqual(asked[1:], ["actions/runs/1012/jobs?filter=latest"])
        # A newer run that failed stands over an older green one.
        jobs["actions/runs/1012/jobs?filter=latest"], jobs["actions/runs/1003/jobs?filter=latest"] = failed, green
        pages, _ = self.lists([self.run_of(3), self.run_of(12)], jobs)
        self.assertEqual(land.verdicts([], land.tests_jobs(self.SHA, pages))["CI"][0], "failure")

    def test_every_page_of_a_list_is_read(self):
        """One page of a hundred was read: the jobs past it were not there."""
        calls = []

        def api(path):
            calls.append(path)
            page = int(path.rsplit("page=", 1)[1])
            return {"jobs": [{"n": n} for n in range((page - 1) * 100, min(page * 100, 203))]}

        self.assertEqual([j["n"] for j in land.pages("actions/runs/5/jobs?filter=latest", "jobs", api)], list(range(203)))
        self.assertEqual(calls, [f"actions/runs/5/jobs?filter=latest&per_page=100&page={p}" for p in (1, 2, 3)])
        whole = lambda path: [{"context": "cells"}] * (100 if path.endswith("page=1") else 0)
        self.assertEqual(len(land.pages("commits/x/statuses", None, whole)), 100, "a full last page asks for one more, which is empty")


class Subjects(unittest.TestCase):
    def test_a_working_title_does_not_land(self):
        for subject in ("x", "wip: many cold files by direct reads", "WIP stuff for later on", "PROBE ONLY: loop the b3sum test", "LAB ONLY: k1 unrolled now",
                        "fix it", "The Hasher's stack holds rows (draft)", "fixup! the thing before this one"):
            self.assertIsNotNone(land.unfit(subject), subject)
        for subject in ("b3sum reads one cold file of 4 MiB or more past the page cache", "docs/hazmat-removal.md: what is left of hazmat",
                        "Pipeline: the workers' view of a Pipeline is never freed"):
            self.assertIsNone(land.unfit(subject), subject)


class Landable(unittest.TestCase):
    def test_the_status_text_fits_github_for_every_set_of_gates(self):
        base = "0123456789abcdef"
        for paths in (["AGENTS.md"], ["src/lib.rs"], ["c/x.S"]):
            text = land.landable(base, list(land.required(paths)))
            self.assertLessEqual(len(text), 140, paths)
            self.assertTrue(text.startswith("every gate green on top of 0123456789ab: "))
        self.assertIn("check-builds", land.landable(base, ["CI", "check-builds", "cells"]), "the gates by name while they fit")
        self.assertTrue(land.landable(base, list(land.required(["src/lib.rs"]))).endswith(": 9 gates"))
        self.assertTrue(land.landable(base, list(land.required(["c/x.S"]))).endswith(": 10 gates"))
        self.assertTrue(land.landable(base, []).endswith("*.md only"))


if __name__ == "__main__":
    unittest.main()
