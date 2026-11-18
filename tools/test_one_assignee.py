"""Unit tests for tools/one_assignee.py: parsing `Next move:` lines, resolving
handles and aliases, atomic single-assignee replacement, bot error replies, and
which calls an event leads to (against a stand-in for GitHub)."""

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import one_assignee as oa

ASSIGNABLE = {
    "devonjonte": "devonjonte",
    "johnservil": "johnservil",
    "samshackleton-tech": "samshackleton-tech",
}


class Parsing(unittest.TestCase):
    def test_extracts_last_unquoted_unfenced_next_move_line(self):
        body = """Earlier quote:
> Next move: Sam

```
Next move: John
```

Summary text.

Next move: @johnservil, on the release check.

— Devon Jonte
"""
        self.assertEqual(
            oa.next_move_line(body),
            ("Next move: @johnservil, on the release check.", "@johnservil, on the release check."),
        )

    def test_accepts_markdown_bold_and_bullets(self):
        self.assertEqual(oa.next_move_line("- **Next move:** Sam, on ops #15."), ("- **Next move:** Sam, on ops #15.", "Sam, on ops #15."))
        self.assertIsNone(oa.next_move_line("My next move is to check the logs."))
        self.assertIsNone(oa.next_move_line("> Next move: John"))


class Resolution(unittest.TestCase):
    def test_resolves_aliases_handles_and_self_words(self):
        self.assertEqual(oa.resolve_target("John, on #447.", "devonjonte", ASSIGNABLE), ("assign", "johnservil"))
        self.assertEqual(oa.resolve_target("still Sam's.", "devonjonte", ASSIGNABLE), ("assign", "samshackleton-tech"))
        self.assertEqual(oa.resolve_target("@samshackleton-tech: allow memlock", "johnservil", ASSIGNABLE), ("assign", "samshackleton-tech"))
        self.assertEqual(oa.resolve_target("mine, the Mac's repeat; then @johnservil.", "devonjonte", ASSIGNABLE), ("assign", "devonjonte"))
        self.assertEqual(oa.resolve_target("Devon.", "johnservil", ASSIGNABLE), ("assign", "devonjonte"))

    def test_flags_unassignable_and_unrecognized_directives(self):
        self.assertEqual(oa.resolve_target("Zooko, then @johnservil.", "devonjonte", ASSIGNABLE), ("unassignable", "zooko"))
        self.assertEqual(oa.resolve_target("@not-a-collaborator", "devonjonte", ASSIGNABLE), ("unassignable", "not-a-collaborator"))
        self.assertEqual(oa.resolve_target("land after CI passes.", "devonjonte", ASSIGNABLE), ("unrecognized", ""))
        self.assertEqual(oa.resolve_target("none.", "devonjonte", ASSIGNABLE), ("none", ""))


class Directives(unittest.TestCase):
    def run_directive(self, body, author="devonjonte", initial=("devonjonte", "johnservil"), states=("open",)):
        assigned, comments, sleeps = [], [], []
        state_list = list(states)

        def get_item(_n):
            state = state_list.pop(0) if len(state_list) > 1 else state_list[0]
            return {"state": state, "assignees": [{"login": u} for u in initial]}

        parsed = oa.next_move_line(body)
        self.assertIsNotNone(parsed)
        oa.apply_next_move(
            "servil-software-factory/bench-hashes",
            242,
            author,
            parsed[0],
            parsed[1],
            ASSIGNABLE,
            get_item,
            lambda n, login: assigned.append((n, login)),
            lambda n, msg: comments.append((n, msg)),
            sleep_fn=lambda s: sleeps.append(s),
        )
        return assigned, comments, sleeps

    def test_replaces_assignees_with_named_person(self):
        assigned, comments, _ = self.run_directive("Done.\n\nNext move: John.")
        self.assertEqual(assigned, [(242, "johnservil")])
        self.assertEqual(comments, [])

    def test_skips_api_write_when_already_sole_assignee(self):
        assigned, comments, _ = self.run_directive("Next move: @johnservil", initial=("johnservil",))
        self.assertEqual(assigned, [])
        self.assertEqual(comments, [])

    def test_replies_when_named_person_cannot_be_assigned(self):
        assigned, comments, _ = self.run_directive("Next move: Zooko, on landing #242.")
        self.assertEqual(assigned, [])
        self.assertEqual(len(comments), 1)
        self.assertIn("@devonjonte", comments[0][1])
        self.assertIn("`@zooko`", comments[0][1])
        self.assertIn("cannot be assigned", comments[0][1])

    def test_replies_when_line_names_nobody(self):
        assigned, comments, _ = self.run_directive("Next move: rerun the probe first.")
        self.assertEqual(assigned, [])
        self.assertEqual(len(comments), 1)
        self.assertIn("could not assign anyone", comments[0][1])

    def test_waits_briefly_for_closing_script_on_next_move_none(self):
        assigned, comments, sleeps = self.run_directive("Closing.\n\nNext move: none.", states=("open", "closed"))
        self.assertEqual(sleeps, [3])
        self.assertEqual(assigned, [])
        self.assertEqual(comments, [])

        _, comments_still_open, _ = self.run_directive("Next move: none.", states=("open", "open"))
        self.assertEqual(len(comments_still_open), 1)

    def test_bot_and_untrusted_guards(self):
        self.assertTrue(oa.is_bot({"login": "github-actions[bot]", "type": "Bot"}))
        self.assertFalse(oa.is_bot({"login": "devonjonte", "type": "User"}))
        self.assertTrue(oa.is_trusted("zooko", "NONE", ASSIGNABLE))
        self.assertTrue(oa.is_trusted("johnservil", "NONE", ASSIGNABLE))
        self.assertTrue(oa.is_trusted("someone", "MEMBER", ASSIGNABLE))
        self.assertFalse(oa.is_trusted("random-passerby", "NONE", ASSIGNABLE))


class Events(unittest.TestCase):
    """`handle_event` against a stand-in for GitHub: the writes an event leads to."""

    def run_event(self, name, event, assignees=("devonjonte",), state="open"):
        calls = []

        def fake_call(_token, method, url, body=None):
            calls.append((method, url.split("/repos/")[1], body))
            if method == "GET" and "/assignees?" in url:
                return [{"login": u} for u in ASSIGNABLE.values()] if url.endswith("page=1") else []
            if method == "GET":
                return {"state": state, "assignees": [{"login": u} for u in assignees]}
            return {}

        real, oa.call = oa.call, fake_call
        try:
            result = oa.handle_event("token", "o/r", name, event)
        finally:
            oa.call = real
        return result, calls, [c for c in calls if c[0] != "GET"]

    def comment(self, body, login="devonjonte", association="MEMBER", action="created", before=None, kind="User"):
        event = {"action": action, "issue": {"number": 7},
                 "comment": {"body": body, "author_association": association, "user": {"login": login, "type": kind}}}
        if before is not None:
            event["changes"] = {"body": {"from": before}}
        return event

    def test_a_collaborators_comment_assigns_whom_it_names(self):
        result, _, writes = self.run_event("issue_comment", self.comment("Done.\n\nNext move: John."))
        self.assertEqual(result, (True, 7))
        self.assertEqual(writes, [("PATCH", "o/r/issues/7", {"assignees": ["johnservil"]})])

    def test_a_passer_by_and_a_bot_are_not_obeyed(self):
        _, _, writes = self.run_event("issue_comment", self.comment("Next move: John.", login="random-passerby", association="NONE"))
        self.assertEqual(writes, [])
        result, calls, _ = self.run_event("issue_comment", self.comment("Next move: John.", login="github-actions[bot]", kind="Bot"))
        self.assertEqual((result, calls), ((True, None), []))

    def test_an_edit_that_names_the_same_person_does_nothing(self):
        # John has moved the item on to himself since; Devon mends a word in his older comment.
        edit = self.comment("Done, at 4bef90cc.\n\nNext move: Sam.", action="edited", before="Done, at 4bef90c.\n\nNext move: Sam.")
        result, _, writes = self.run_event("issue_comment", edit, assignees=("johnservil",))
        self.assertEqual(result, (True, None))
        self.assertEqual(writes, [])
        # ... and neither does an edit event that says nothing of the body.
        bare = self.comment("Next move: Sam.", action="edited")
        self.assertEqual(self.run_event("issue_comment", bare, assignees=("johnservil",))[2], [])

    def test_an_edit_that_names_someone_else_or_adds_the_line_assigns(self):
        renamed = self.comment("Next move: Devon.", login="johnservil", action="edited", before="Next move: yours.")
        self.assertEqual(self.run_event("issue_comment", renamed, assignees=("johnservil",))[2],
                         [("PATCH", "o/r/issues/7", {"assignees": ["devonjonte"]})])
        added = self.comment("Read.\n\nNext move: Sam.", action="edited", before="Read.")
        self.assertEqual(self.run_event("issue_comment", added)[2], [("PATCH", "o/r/issues/7", {"assignees": ["samshackleton-tech"]})])

    def test_the_body_speaks_at_the_opening_not_at_a_reopening(self):
        item = {"number": 9, "body": "A fault.\n\nNext move: Devon.", "author_association": "MEMBER", "user": {"login": "johnservil", "type": "User"}}
        result, _, writes = self.run_event("issues", {"action": "opened", "issue": item}, assignees=())
        self.assertEqual(result, (False, 9))
        self.assertEqual(writes, [("PATCH", "o/r/issues/9", {"assignees": ["devonjonte"]})])
        # Reopened after Sam took it over: the body's old line must not hand it back to Devon.
        result, calls, _ = self.run_event("issues", {"action": "reopened", "issue": item}, assignees=("samshackleton-tech",))
        self.assertEqual((result, calls), ((False, None), []))
        pull = dict(item, number=11)
        self.assertEqual(self.run_event("pull_request_target", {"action": "reopened", "pull_request": pull})[1], [])


if __name__ == "__main__":
    unittest.main()
