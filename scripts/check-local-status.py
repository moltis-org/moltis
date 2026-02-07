#!/usr/bin/env python3

import json
import os
import sys
import urllib.request


def main() -> int:
    repo = os.environ["REPO"]
    sha = os.environ["PR_HEAD_SHA"]
    token = os.environ["GH_TOKEN"]
    required = os.environ["REQUIRED_CONTEXT"]

    req = urllib.request.Request(
        f"https://api.github.com/repos/{repo}/commits/{sha}/status",
        headers={
            "Authorization": f"Bearer {token}",
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )
    with urllib.request.urlopen(req) as resp:
        payload = json.loads(resp.read().decode("utf-8"))

    found_state = None
    for status in payload.get("statuses", []):
        if status.get("context") == required:
            found_state = status.get("state")
            break

    if found_state == "success":
        print(f"{required} is success")
        return 0

    if found_state is None:
        print(f"Missing required local status: {required}", file=sys.stderr)
    else:
        print(
            f"Local status {required} is '{found_state}', expected 'success'",
            file=sys.stderr,
        )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
