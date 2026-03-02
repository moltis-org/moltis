#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  ./scripts/check-changelog-guard.sh <base_sha> [head_sha]

Examples:
  ./scripts/check-changelog-guard.sh origin/main HEAD
  ./scripts/check-changelog-guard.sh "$BASE_SHA" "$HEAD_SHA"
EOF
}

if [[ $# -lt 1 || $# -gt 2 ]]; then
  usage >&2
  exit 2
fi

base_ref="$1"
head_ref="${2:-HEAD}"

if ! git rev-parse --verify "$base_ref^{commit}" >/dev/null 2>&1; then
  echo "base ref '$base_ref' is not a valid commit/ref" >&2
  exit 2
fi

if ! git rev-parse --verify "$head_ref^{commit}" >/dev/null 2>&1; then
  echo "head ref '$head_ref' is not a valid commit/ref" >&2
  exit 2
fi

if git diff --name-only "$base_ref" "$head_ref" | grep -qx 'CHANGELOG\.md'; then
  cat >&2 <<'EOF'
Manual CHANGELOG.md edits are blocked in pull requests.

Do this instead:
1) Use conventional commits (`feat:`, `fix:`, `refactor:`, etc.).
2) Preview unreleased notes with `just changelog-unreleased`.
3) Generate release notes only during release prep:
   `./scripts/prepare-release.sh <version> [YYYY-MM-DD]`
EOF
  exit 1
fi

echo "No manual CHANGELOG.md edits detected."
