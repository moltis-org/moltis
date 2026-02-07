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

BASE_REPO="$(gh repo view --json nameWithOwner -q .nameWithOwner)"
SHA="$(gh pr view "$PR_NUMBER" --repo "$BASE_REPO" --json headRefOid -q .headRefOid)"
HEAD_OWNER="$(gh pr view "$PR_NUMBER" --repo "$BASE_REPO" --json headRepositoryOwner -q .headRepositoryOwner.login)"
HEAD_REPO_NAME="$(gh pr view "$PR_NUMBER" --repo "$BASE_REPO" --json headRepository -q .headRepository.name)"

if [[ -n "$HEAD_OWNER" && -n "$HEAD_REPO_NAME" ]]; then
  REPO="${HEAD_OWNER}/${HEAD_REPO_NAME}"
else
  REPO="$BASE_REPO"
fi

fmt_cmd="${LOCAL_VALIDATE_FMT_CMD:-cargo +nightly fmt --all -- --check}"
lint_cmd="${LOCAL_VALIDATE_LINT_CMD:-cargo clippy --workspace --all-features -- -D warnings}"
test_cmd="${LOCAL_VALIDATE_TEST_CMD:-cargo test --all-features}"

set_status() {
  local state="$1"
  local context="$2"
  local description="$3"
  if ! gh api "repos/$REPO/statuses/$SHA" \
    -f state="$state" \
    -f context="$context" \
    -f description="$description" \
    -f target_url="https://github.com/$BASE_REPO/pull/$PR_NUMBER" >/dev/null; then
    cat >&2 <<EOF
Failed to publish status '$context' to $REPO@$SHA.
Check that your token can write commit statuses for that repository.

Expected token access:
- classic PAT: repo:status (or repo)
- fine-grained PAT: Commit statuses (Read and write)
EOF
    return 1
  fi
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

echo "Validating PR #$PR_NUMBER ($SHA) in $BASE_REPO"
echo "Publishing commit statuses to: $REPO"

run_check "local/fmt" "$fmt_cmd"
run_check "local/lint" "$lint_cmd"
run_check "local/test" "$test_cmd"

echo "All local validation statuses published successfully."
