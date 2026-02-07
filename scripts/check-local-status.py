#!/usr/bin/env python3

import json
import os
import sys
import time
import urllib.request


def main() -> int:
    repo = os.environ["REPO"]
    sha = os.environ["PR_HEAD_SHA"]
    token = os.environ["GH_TOKEN"]
    required = os.environ["REQUIRED_CONTEXT"]
    timeout_secs = int(os.environ.get("LOCAL_STATUS_WAIT_SECS", "900"))
    poll_secs = int(os.environ.get("LOCAL_STATUS_POLL_SECS", "10"))

    deadline = time.time() + timeout_secs
    while True:
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

        if time.time() >= deadline:
            if found_state is None:
                print(f"Missing required local status: {required}", file=sys.stderr)
            else:
                print(
                    f"Local status {required} is '{found_state}', expected 'success'",
                    file=sys.stderr,
                )
            return 1

        state = found_state if found_state is not None else "missing"
        print(
            f"Waiting for {required}=success (current: {state}), retrying in {poll_secs}s..."
        )
        time.sleep(poll_secs)


if __name__ == "__main__":
    raise SystemExit(main())
