"""What is mine to move: the open issues and pull requests assigned to
you in both repositories, newest update first, each with who spoke last
and when (CONTRIBUTING.md, "One assignee makes the next move"). The team's
private notes (servil-software-factory/ops) are read too, for whoever
has access to them: a note written there for you shows here.

    python3 tools/my_moves.py [LOGIN]

Run it at the start of a session and between pieces of work: an item
another person handed you shows here, with their comment last. LOGIN
defaults to the token's own account; the token is GITHUB_TOKEN, or the
one `git credential fill` gives for github.com.
"""
import json
import os
import subprocess
import sys
import urllib.error
import urllib.request

REPOS = ["servil-software-factory/BLAKE3", "servil-software-factory/bench-hashes"]
# The team's alone: GitHub answers 404 to an account without access.
PRIVATE = ["servil-software-factory/ops"]


def token():
    if os.environ.get("GITHUB_TOKEN"):
        return os.environ["GITHUB_TOKEN"]
    out = subprocess.run(["git", "credential", "fill"], input="protocol=https\nhost=github.com\n\n", capture_output=True, text=True, check=True).stdout
    return dict(line.split("=", 1) for line in out.splitlines() if "=" in line)["password"]


def get(tok, url):
    req = urllib.request.Request(url, headers={"Authorization": f"Bearer {tok}", "Accept": "application/vnd.github+json"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)


def main():
    tok = token()
    login = sys.argv[1] if len(sys.argv) > 1 else get(tok, "https://api.github.com/user")["login"]
    rows = []
    for repo in REPOS + PRIVATE:
        page = 1
        while True:
            try:
                items = get(tok, f"https://api.github.com/repos/{repo}/issues?state=open&assignee={login}&per_page=100&page={page}")
            except urllib.error.HTTPError as e:
                if repo in PRIVATE and e.code == 404:
                    break
                raise
            if not items:
                break
            rows += [(i, repo) for i in items]
            page += 1
    rows.sort(key=lambda r: r[0]["updated_at"], reverse=True)
    print(f"{len(rows)} open items assigned to {login}, newest update first (* = someone else spoke last)")
    for item, repo in rows:
        last = item["user"]["login"]
        if item["comments"]:
            comments = get(tok, f"{item['comments_url']}?per_page=1&page={item['comments']}")
            if comments:
                last = comments[-1]["user"]["login"]
        kind = "PR" if "pull_request" in item else "  "
        milestone = (item.get("milestone") or {}).get("title", "-")
        mark = "*" if last != login else " "
        print(f"{mark} {item['updated_at'][5:16].replace('T', ' ')}  {repo.split('/')[1]:<12} #{item['number']:<4} {kind} [{milestone[:9]:<9}] last: {last:<18} {item['title'][:64]}")


if __name__ == "__main__":
    main()
