#!/usr/bin/env bash

set -euo pipefail

if ! command -v gh >/dev/null 2>&1; then
  echo "gh CLI is required" >&2
  exit 1
fi

if [[ -z "${GH_TOKEN:-}" ]]; then
  if GH_TOKEN="$(gh auth token 2>/dev/null)"; then
    export GH_TOKEN
  else
    echo "GH_TOKEN is required (repo:status or equivalent access)" >&2
    echo "Tip: run 'gh auth login' or export GH_TOKEN with proper scopes." >&2
    exit 1
  fi
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
biome_cmd="${LOCAL_VALIDATE_BIOME_CMD:-biome ci crates/gateway/src/assets/js/}"
zizmor_cmd="${LOCAL_VALIDATE_ZIZMOR_CMD:-zizmor .}"
lint_cmd="${LOCAL_VALIDATE_LINT_CMD:-cargo clippy --workspace --all-features -- -D warnings}"
test_cmd="${LOCAL_VALIDATE_TEST_CMD:-cargo test --all-features}"

if [[ "$(uname -s)" == "Darwin" ]] && ! command -v nvcc >/dev/null 2>&1; then
  if [[ -z "${LOCAL_VALIDATE_LINT_CMD:-}" ]]; then
    lint_cmd="cargo clippy --workspace -- -D warnings"
  fi
  if [[ -z "${LOCAL_VALIDATE_TEST_CMD:-}" ]]; then
    test_cmd="cargo test"
  fi
  echo "Detected macOS without nvcc; using non-CUDA local validation commands." >&2
  echo "Override with LOCAL_VALIDATE_LINT_CMD / LOCAL_VALIDATE_TEST_CMD if needed." >&2
fi

if [[ -z "${LOCAL_VALIDATE_ZIZMOR_CMD:-}" ]] && ! command -v zizmor >/dev/null 2>&1; then
  echo "zizmor CLI not found. Install it or set LOCAL_VALIDATE_ZIZMOR_CMD." >&2
  exit 1
fi

repair_stale_llama_build_dirs() {
  shopt -s nullglob
  for dir in target/*/build/llama-cpp-sys-2-* target/*/build/llama-cpp-2-*; do
    if [[ -d "$dir" ]]; then
      echo "Removing cached llama build dir: $dir"
      rm -rf "$dir"
    fi
  done
  shopt -u nullglob
}

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

If this is an org with SSO enforcement, authorize the token for the org.
If GH_TOKEN is set in your shell, try unsetting it to use your gh auth token:
  unset GH_TOKEN
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

# macOS local builds can leave stale cmake output dirs where configure was skipped
# but no generator files remain. Clean those up before lint/test.
repair_stale_llama_build_dirs

run_check "local/fmt" "$fmt_cmd"
run_check "local/biome" "$biome_cmd"
run_check "local/zizmor" "$zizmor_cmd"
run_check "local/lint" "$lint_cmd"
run_check "local/test" "$test_cmd"

echo "All local validation statuses published successfully."
