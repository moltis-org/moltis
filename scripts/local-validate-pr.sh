#!/usr/bin/env bash

set -euo pipefail

if ! command -v gh >/dev/null 2>&1; then
  echo "gh CLI is required" >&2
  exit 1
fi

if [[ -z "${GH_TOKEN:-}" ]]; then
  echo "GH_TOKEN is required (repo:status or equivalent access)" >&2
  exit 1
fi

PR_NUMBER="${1:-}"
if [[ -z "$PR_NUMBER" ]]; then
  PR_NUMBER="$(gh pr view --json number -q .number)"
fi

REPO="$(gh repo view --json nameWithOwner -q .nameWithOwner)"
SHA="$(gh pr view "$PR_NUMBER" --json headRefOid -q .headRefOid)"

fmt_cmd="${LOCAL_VALIDATE_FMT_CMD:-cargo +nightly fmt --all -- --check}"
lint_cmd="${LOCAL_VALIDATE_LINT_CMD:-cargo clippy --workspace --all-features -- -D warnings}"
test_cmd="${LOCAL_VALIDATE_TEST_CMD:-cargo test --all-features}"

set_status() {
  local state="$1"
  local context="$2"
  local description="$3"
  gh api "repos/$REPO/statuses/$SHA" \
    -f state="$state" \
    -f context="$context" \
    -f description="$description" \
    -f target_url="https://github.com/$REPO/pull/$PR_NUMBER" >/dev/null
}

run_check() {
  local context="$1"
  local cmd="$2"

  set_status pending "$context" "Running locally"
  if bash -lc "$cmd"; then
    set_status success "$context" "Passed locally"
  else
    set_status failure "$context" "Failed locally"
    return 1
  fi
}

echo "Validating PR #$PR_NUMBER ($SHA) in $REPO"

run_check "local/fmt" "$fmt_cmd"
run_check "local/lint" "$lint_cmd"
run_check "local/test" "$test_cmd"

echo "All local validation statuses published successfully."
