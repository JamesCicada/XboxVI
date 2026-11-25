#!/usr/bin/env python3
"""Post a GitHub push or release to a Discord channel through a webhook.

Usage (in GitHub Actions):
    python3 scripts/discord_notify.py --event push --event-path "$GITHUB_EVENT_PATH"
    python3 scripts/discord_notify.py --event release --event-path "$GITHUB_EVENT_PATH"

Locally, --dry-run prints the message instead of sending it:
    python3 scripts/discord_notify.py --event push --event-path event.json --dry-run

The webhook URL comes from the DISCORD_WEBHOOK_URL environment variable. When it
is unset the script prints a notice and exits 0, so forks and repositories
without the secret do not fail. Messages never ping anyone: allowed_mentions is
empty, whatever a commit message or release note contains.

Standard library only.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request
from pathlib import Path

USER_AGENT = "DiscordBot (https://github.com/JamesCicada/XboxVI, 1.0)"
GREEN = 0x2EA043
BLUE = 0x2F81F7
MAX_COMMITS = 10

# Discord embed limits.
TITLE_LIMIT = 256
DESCRIPTION_LIMIT = 4096


def clip(text: str, limit: int) -> str:
    return text if len(text) <= limit else text[: limit - 1] + "…"


def push_message(event: dict) -> dict | None:
    commits = event.get("commits") or []
    if not commits:
        return None
    lines = []
    for commit in commits[:MAX_COMMITS]:
        subject = (commit.get("message") or "").splitlines()[0] if commit.get("message") else ""
        author = (commit.get("author") or {}).get("name", "")
        sha = commit.get("id", "")[:7]
        url = commit.get("url", "")
        if sha and url:
            ref = f"[`{sha}`]({url}) "
        elif sha:
            ref = f"`{sha}` "
        else:
            ref = ""
        lines.append(f"{ref}{clip(subject, 120)}" + (f" · {author}" if author else ""))
    if len(commits) > MAX_COMMITS:
        lines.append(f"…and {len(commits) - MAX_COMMITS} more")

    count = len(commits)
    branch = event.get("ref") or ""
    heads_prefix = "refs/heads/"
    if branch.startswith(heads_prefix):
        branch = branch[len(heads_prefix):]
    embed_url = event.get("compare") or (event.get("repository") or {}).get("html_url", "")
    embed = {
        "title": clip(f"{count} new commit{'s' if count != 1 else ''} to {branch or 'main'}", TITLE_LIMIT),
        "description": clip("\n".join(lines), DESCRIPTION_LIMIT),
        "color": GREEN,
    }
    if embed_url:
        embed["url"] = embed_url
    return {"embeds": [embed], "allowed_mentions": {"parse": []}}


def release_message(event: dict) -> dict | None:
    if event.get("action") not in (None, "published"):
        return None
    release = event.get("release") or {}
    if not release or release.get("draft"):
        return None
    name = release.get("name") or release.get("tag_name") or "New release"
    body = release.get("body") or ""
    html_url = release.get("html_url", "")
    embed = {
        "title": clip(f"Release: {name}", TITLE_LIMIT),
        "description": clip(body, 2000) if body else "See the release page for details.",
        "color": BLUE,
    }
    if html_url:
        embed["url"] = html_url
    return {"embeds": [embed], "allowed_mentions": {"parse": []}}


def send(url: str, payload: dict) -> None:
    request = urllib.request.Request(
        url,
        data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json", "User-Agent": USER_AGENT},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            print(f"Posted to Discord (HTTP {response.status}).")
    except urllib.error.HTTPError as error:
        # Never print the URL: it is a secret.
        detail = error.read().decode("utf-8", "replace")[:500].replace(url, "<webhook>")
        raise SystemExit(f"discord_notify: Discord returned HTTP {error.code}: {detail}")
    except urllib.error.URLError as error:
        reason = str(error.reason).replace(url, "<webhook>")
        raise SystemExit(f"discord_notify: could not reach Discord: {reason}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--event", required=True, choices=("push", "release"))
    parser.add_argument("--event-path", required=True, type=Path, help="GitHub event payload JSON")
    parser.add_argument("--dry-run", action="store_true", help="print the message instead of sending it")
    args = parser.parse_args(argv)

    event = json.loads(args.event_path.read_text(encoding="utf-8"))
    payload = push_message(event) if args.event == "push" else release_message(event)
    if payload is None:
        print(f"Nothing to post for this {args.event} event.")
        return 0

    if args.dry_run:
        print(json.dumps(payload, indent=2, ensure_ascii=False))
        return 0

    url = os.environ.get("DISCORD_WEBHOOK_URL", "").strip()
    if not url:
        print("DISCORD_WEBHOOK_URL is not set; skipping the Discord post.")
        return 0
    send(url, payload)
    return 0


if __name__ == "__main__":
    sys.exit(main())
