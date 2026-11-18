"""Every open issue and pull request has one assignee, the person
whose move is next (AGENTS.md, "One assignee makes the next move").

Run by .github/workflows/one-assignee.yml on issue comments, on every
change to an item's assignees, and once a day, and by hand:
`python3 tools/one_assignee.py OWNER/REPO` with a token in GITHUB_TOKEN.

- When a collaborator's comment (or a newly opened item's body) has a
  `Next move: NAME` line, whoever that line names first becomes the
  item's sole assignee; if the line names someone who cannot be assigned
  or names nobody while the item stays open, the bot replies saying so.
  An edit of a comment counts only when it changes whom the line names,
  and a reopened item's body does not count: either would hand the item
  back to whoever an old line named, after others have moved it on.
- Otherwise an open item with no assignee gets its author, with a
  comment; one with several gets a comment asking for one.

Prints what it did; exits 0.
"""
import json
import os
import re
import sys
import time
import urllib.request

NONE = ("Assigned to you, the author, since this had no assignee. The assignee makes the next move: when yours is done, "
        "say what the next move is and assign whoever makes it, or close this.")
MANY = "Several assignees: please leave exactly one, the person whose move is next."

ALIASES = {
    "john": "johnservil",
    "sam": "samshackleton-tech",
    "devon": "devonjonte",
    "zooko": "zooko",
}
TRUSTED_ROLES = {"OWNER", "MEMBER", "COLLABORATOR"}
LEADING_FILLER = re.compile(r"^(?:is|stays|remains|still|with|on|for|to)\b\s*", re.IGNORECASE)
NEXT_MOVE_RE = re.compile(r"^\s*(?:[-*]\s*)?\*{0,2}next move\*{0,2}\s*:\*{0,2}\s*(.+?)\s*$", re.IGNORECASE)
FIRST_TOKEN_RE = re.compile(r"^(@?)([A-Za-z0-9][A-Za-z0-9-]*)(?:'s)?(?![A-Za-z0-9-])")
FENCE_RE = re.compile(r"^\s*(`{3,}|~{3,})")


def call(token, method, url, body=None):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(url, data=data, method=method,
                                 headers={"Authorization": f"Bearer {token}", "Accept": "application/vnd.github+json",
                                          "Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r) if r.status != 204 else None


def assignable_logins(token, repo):
    """Map lowercased login -> canonical GitHub login for everyone assignable in `repo`."""
    out, page = {}, 1
    while True:
        users = call(token, "GET", f"https://api.github.com/repos/{repo}/assignees?per_page=100&page={page}")
        if not users:
            break
        for u in users:
            out[u["login"].lower()] = u["login"]
        page += 1
    return out


def is_bot(user):
    login = (user or {}).get("login", "")
    return login.endswith("[bot]") or (user or {}).get("type") == "Bot"


def is_trusted(login, association, assignable):
    low = (login or "").lower()
    return low in assignable or association in TRUSTED_ROLES or low == "zooko"


def next_move_line(body):
    """Return `(matched_line, rest_after_colon)` for the last unquoted, unfenced `Next move:` line, or None."""
    if not body:
        return None
    kept, fence = [], None
    for raw in body.splitlines():
        m = FENCE_RE.match(raw)
        if m:
            marker = m.group(1)
            if fence is None:
                fence = marker[0] * len(marker)
            elif raw.lstrip().startswith(fence):
                fence = None
            continue
        if fence is not None or raw.lstrip().startswith(">"):
            continue
        kept.append(raw)
    for line in reversed(kept):
        m = NEXT_MOVE_RE.match(line)
        if m:
            return line.strip(), m.group(1).strip()
    return None


def resolve_target(rest, author, assignable):
    """Resolve `rest` after `Next move:` into `('assign', login)`, `('none', '')`,
    `('unassignable', candidate)`, or `('unrecognized', '')`."""
    trimmed = rest.strip()
    while True:
        nxt = LEADING_FILLER.sub("", trimmed, count=1)
        if nxt == trimmed:
            break
        trimmed = nxt.strip()
    m = FIRST_TOKEN_RE.match(trimmed)
    if not m:
        return "unrecognized", ""
    at, word = m.group(1), m.group(2)
    low = word.lower()
    if not at and low in {"none", "nobody", "nothing"}:
        return "none", ""
    if not at and low in {"mine", "me", "my", "myself"}:
        candidate = author
    elif not at and low in ALIASES:
        candidate = ALIASES[low]
    elif low in assignable:
        candidate = assignable[low]
    elif at or word[0].isupper():
        candidate = word
    else:
        return "unrecognized", ""
    if candidate.lower() in assignable:
        return "assign", assignable[candidate.lower()]
    return "unassignable", candidate


def edit_names_someone_new(event, parsed, author, assignable):
    """Whether an edit of a comment changed whom its `Next move:` line names. An edit that did not is
    no new directive: applied again, it would hand the item back to whoever an older comment named,
    after others have moved it on (a typing mistake mended a week later, say)."""
    changes = event.get("changes") or {}
    if "body" not in changes:
        return False
    before = next_move_line((changes.get("body") or {}).get("from") or "")
    return before is None or resolve_target(before[1], author, assignable) != resolve_target(parsed[1], author, assignable)


def allowed_list(assignable):
    return ", ".join(f"`@{u}`" for u in sorted(assignable.values(), key=str.lower)) or "none"


def apply_next_move(repo, number, author, matched_line, rest, assignable, get_item, set_assignee, post_comment, sleep_fn=time.sleep):
    """Apply one `Next move:` directive on issue/PR `number`. Returns True when handled."""
    kind, target = resolve_target(rest, author, assignable)
    item = get_item(number)
    if item["state"] != "open":
        return True
    if kind == "none":
        sleep_fn(3)
        item = get_item(number)
        if item["state"] != "open":
            return True
    if kind == "assign":
        current = [a["login"] for a in item.get("assignees", ())]
        if current != [target]:
            set_assignee(number, target)
            print(f"#{number}: Next move -> assigned {target}")
        else:
            print(f"#{number}: Next move -> {target} already sole assignee")
        return True
    allowed = allowed_list(assignable)
    if kind == "unassignable":
        msg = (f"@{author}: `Next move:` named `@{target}`, who cannot be assigned in `{repo}` "
               f"(assignable: {allowed}). Add `@{target}` as a collaborator, or name an assignable owner.")
    else:
        msg = (f"@{author}: could not assign anyone from `{matched_line}` while this item is open "
               f"(assignable: {allowed}). Name one assignable person after `Next move:` "
               f"(such as `@handle`, `John`, `Sam`, `Devon`, or `mine`), or close the item.")
    post_comment(number, msg)
    print(f"#{number}: Next move ({kind}: {target or rest}) -> replied to {author}")
    return True


def handle_event(token, repo, event_name, event):
    """Handle an Actions webhook event if it carries a `Next move:` directive.
    Returns `(stop_after_event, handled_item_number_or_None)`."""
    url_for = lambda n: f"https://api.github.com/repos/{repo}/issues/{n}"
    get_item = lambda n: call(token, "GET", url_for(n))
    set_assignee = lambda n, login: call(token, "PATCH", url_for(n), {"assignees": [login]})
    post_comment = lambda n, body: call(token, "POST", f"{url_for(n)}/comments", {"body": body})

    if event_name == "issue_comment":
        comment = event.get("comment") or {}
        user = comment.get("user") or {}
        if is_bot(user):
            return True, None
        parsed = next_move_line(comment.get("body") or "")
        if not parsed:
            return True, None
        assignable = assignable_logins(token, repo)
        author = user.get("login", "")
        if not is_trusted(author, comment.get("author_association", ""), assignable):
            return True, None
        number = event["issue"]["number"]
        if event.get("action") == "edited" and not edit_names_someone_new(event, parsed, author, assignable):
            print(f"#{number}: an edit that names the same next mover as before: nothing to do")
            return True, None
        apply_next_move(repo, number, author, parsed[0], parsed[1], assignable, get_item, set_assignee, post_comment)
        return True, number

    # (the body speaks at the opening only: at a reopening its line is as old as the item)
    if event_name in {"issues", "pull_request_target"} and event.get("action") == "opened":
        obj = event.get("issue") or event.get("pull_request") or {}
        user = obj.get("user") or {}
        if not is_bot(user):
            parsed = next_move_line(obj.get("body") or "")
            if parsed:
                assignable = assignable_logins(token, repo)
                author = user.get("login", "")
                if is_trusted(author, obj.get("author_association", ""), assignable):
                    number = obj["number"]
                    apply_next_move(repo, number, author, parsed[0], parsed[1], assignable, get_item, set_assignee, post_comment)
                    return False, number
    return False, None


def main():
    repo = sys.argv[1]
    token = os.environ["GITHUB_TOKEN"]
    event_name = os.environ.get("GITHUB_EVENT_NAME", "")
    event_path = os.environ.get("GITHUB_EVENT_PATH", "")
    skip_number = None
    if event_name and event_path and os.path.exists(event_path):
        with open(event_path, encoding="utf-8") as f:
            event = json.load(f)
        stop, skip_number = handle_event(token, repo, event_name, event)
        if stop:
            return

    page = 1
    while True:
        items = call(token, "GET", f"https://api.github.com/repos/{repo}/issues?state=open&per_page=100&page={page}")
        if not items:
            break
        for item in items:
            n = item["number"]
            if n == skip_number:
                continue
            assignees = [a["login"] for a in item["assignees"]]
            if len(assignees) == 1:
                continue
            url = f"https://api.github.com/repos/{repo}/issues/{n}"
            if not assignees:
                author = item["user"]["login"]
                call(token, "POST", f"{url}/assignees", {"assignees": [author]})
                call(token, "POST", f"{url}/comments", {"body": NONE})
                print(f"#{n}: assigned {author}")
            else:
                call(token, "POST", f"{url}/comments", {"body": MANY})
                print(f"#{n}: {len(assignees)} assignees, asked for one")
        page += 1


if __name__ == "__main__":
    main()
