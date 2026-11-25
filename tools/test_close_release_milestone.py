import io
import unittest
from urllib.parse import parse_qs, urlsplit

import close_release_milestone as mod


class CloseReleaseMilestoneTests(unittest.TestCase):
    def test_version_from_tag(self):
        self.assertEqual(mod.version_from_tag("v0.11.0+deadbeef"), "0.11.0")
        self.assertEqual(mod.version_from_tag("v0.10.0"), "0.10.0")
        self.assertEqual(mod.version_from_tag("0.11.0-rc.1+abc1234"), "0.11.0-rc.1")
        with self.assertRaises(ValueError):
            mod.version_from_tag("not-a-tag")

    def test_closes_completed_milestone(self):
        calls = []

        def fake_request(method, url, token, payload=None):
            calls.append((method, url, token, payload))
            if method == "GET":
                return [
                    {"number": 1, "title": "post 0.10, pre 1.0", "open_issues": 5, "closed_issues": 20},
                    {"number": 4, "title": "0.11.0", "open_issues": 0, "closed_issues": 12},
                ]
            return {"number": 4, "state": "closed"}

        out = io.StringIO()
        status = mod.close_completed_milestone("v0.11.0+1b6f3178", "owner/repo", "tok", request=fake_request, out=out)
        self.assertEqual(status, "closed")
        self.assertIn("closed #4 (0.11.0)", out.getvalue())
        self.assertEqual(len(calls), 2)
        self.assertEqual(calls[1][0], "PATCH")
        self.assertTrue(calls[1][1].endswith("/repos/owner/repo/milestones/4"))
        self.assertEqual(calls[1][3], {"state": "closed"})

    def test_matches_v_prefixed_milestone_title(self):
        calls = []

        def fake_request(method, url, token, payload=None):
            calls.append((method, url, payload))
            if method == "GET":
                return [{"number": 7, "title": "v0.11.0", "open_issues": 0, "closed_issues": 3}]
            return {"number": 7, "state": "closed"}

        out = io.StringIO()
        status = mod.close_completed_milestone("v0.11.0+abcdef", "owner/repo", "tok", request=fake_request, out=out)
        self.assertEqual(status, "closed")
        self.assertEqual(calls[1][0], "PATCH")

    def test_leaves_milestone_with_open_issues_open(self):
        calls = []

        def fake_request(method, url, token, payload=None):
            calls.append(method)
            return [{"number": 4, "title": "0.11.0", "open_issues": 2, "closed_issues": 10}]

        out = io.StringIO()
        status = mod.close_completed_milestone("v0.11.0+abcdef", "owner/repo", "tok", request=fake_request, out=out)
        self.assertEqual(status, "still_open")
        self.assertIn("still has 2 open issue(s)", out.getvalue())
        self.assertEqual(calls, ["GET"])

    def test_missing_or_already_closed_milestone_is_noop(self):
        calls = []

        def fake_request(method, url, token, payload=None):
            calls.append(method)
            return [{"number": 1, "title": "post 0.10, pre 1.0", "open_issues": 0, "closed_issues": 10}]

        out = io.StringIO()
        status = mod.close_completed_milestone("v0.11.0+abcdef", "owner/repo", "tok", request=fake_request, out=out)
        self.assertEqual(status, "not_found")
        self.assertIn("nothing to close", out.getvalue())
        self.assertEqual(calls, ["GET"])

    def _parse_milestone_get(self, method, url, expected_page):
        self.assertEqual(method, "GET")
        parts = urlsplit(url)
        self.assertTrue(parts.path.endswith("/repos/owner/repo/milestones"), url)
        qs = parse_qs(parts.query, keep_blank_values=True)
        self.assertEqual(qs.get("state"), ["open"], url)
        self.assertEqual(qs.get("per_page"), ["100"], url)
        self.assertEqual(qs.get("page"), [str(expected_page)], url)

    def test_paginates_past_100_open_milestones_and_closes_on_page_3(self):
        milestones = [
            {"number": i, "title": f"0.0.{i}", "open_issues": 1, "closed_issues": 0}
            for i in range(1, 201)
        ] + [{"number": 201, "title": "0.11.0", "open_issues": 0, "closed_issues": 39}]
        pages_requested = []
        patched = []

        def fake_request(method, url, token, payload=None):
            if method == "GET":
                expected_page = len(pages_requested) + 1
                self._parse_milestone_get(method, url, expected_page)
                pages_requested.append(expected_page)
                start = (expected_page - 1) * 100
                return milestones[start : start + 100]
            self.assertEqual(method, "PATCH")
            patched.append((url, payload))
            return {"number": 201, "state": "closed"}

        out = io.StringIO()
        status = mod.close_completed_milestone("v0.11.0+deadbeef", "owner/repo", "tok", request=fake_request, out=out)
        self.assertEqual(status, "closed")
        self.assertEqual(pages_requested, [1, 2, 3])
        self.assertEqual(len(patched), 1)
        self.assertTrue(patched[0][0].endswith("/repos/owner/repo/milestones/201"))
        self.assertEqual(patched[0][1], {"state": "closed"})

    def test_pagination_stops_early_when_found_on_page_2(self):
        milestones = [
            {"number": i, "title": f"0.0.{i}", "open_issues": 1, "closed_issues": 0}
            for i in range(1, 206)
        ]
        milestones[100] = {"number": 101, "title": "0.11.0", "open_issues": 3, "closed_issues": 10}
        pages_requested = []

        def fake_request(method, url, token, payload=None):
            expected_page = len(pages_requested) + 1
            self.assertLessEqual(expected_page, 2, "must stop once found on page 2")
            self._parse_milestone_get(method, url, expected_page)
            pages_requested.append(expected_page)
            start = (expected_page - 1) * 100
            return milestones[start : start + 100]

        out = io.StringIO()
        status = mod.close_completed_milestone("v0.11.0+deadbeef", "owner/repo", "tok", request=fake_request, out=out)
        self.assertEqual(status, "still_open")
        self.assertEqual(pages_requested, [1, 2])
        self.assertIn("still has 3 open issue(s)", out.getvalue())

    def test_pagination_exact_100_boundary_stops_on_empty_page_2(self):
        milestones = [
            {"number": i, "title": f"0.0.{i}", "open_issues": 1, "closed_issues": 0}
            for i in range(1, 101)
        ]
        pages_requested = []

        def fake_request(method, url, token, payload=None):
            expected_page = len(pages_requested) + 1
            self.assertLessEqual(expected_page, 2, "must stop after empty page 2")
            self._parse_milestone_get(method, url, expected_page)
            pages_requested.append(expected_page)
            start = (expected_page - 1) * 100
            return milestones[start : start + 100]

        out = io.StringIO()
        status = mod.close_completed_milestone("v0.11.0+deadbeef", "owner/repo", "tok", request=fake_request, out=out)
        self.assertEqual(status, "not_found")
        self.assertEqual(pages_requested, [1, 2])


if __name__ == "__main__":
    unittest.main()
