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
biome_cmd="${LOCAL_VALIDATE_BIOME_CMD:-biome ci --diagnostic-level=error crates/gateway/src/assets/js/}"
zizmor_cmd="${LOCAL_VALIDATE_ZIZMOR_CMD:-zizmor . --min-severity high >/dev/null 2>&1 || true}"
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

ensure_zizmor() {
  if command -v zizmor >/dev/null 2>&1; then
    return 0
  fi

  case "$(uname -s)" in
    Darwin)
      if command -v brew >/dev/null 2>&1; then
        echo "zizmor not found; installing with Homebrew..." >&2
        brew install zizmor
      fi
      ;;
    Linux)
      if command -v apt-get >/dev/null 2>&1; then
        echo "zizmor not found; installing with apt..." >&2
        sudo apt-get update
        sudo apt-get install -y zizmor
      fi
      ;;
  esac

  if ! command -v zizmor >/dev/null 2>&1; then
    echo "zizmor CLI not found. Install it or set LOCAL_VALIDATE_ZIZMOR_CMD." >&2
    exit 1
  fi
}

if [[ -z "${LOCAL_VALIDATE_ZIZMOR_CMD:-}" ]]; then
  ensure_zizmor
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
  local start
  local end
  local duration

  start="$(date +%s)"
  set_status pending "$context" "Running locally"
  if bash -lc "$cmd"; then
    end="$(date +%s)"
    duration="$((end - start))"
    set_status success "$context" "Passed locally"
    echo "[$context] passed in ${duration}s"
  else
    end="$(date +%s)"
    duration="$((end - start))"
    set_status failure "$context" "Failed locally"
    echo "[$context] failed in ${duration}s" >&2
    return 1
  fi
}

run_check_async() {
  local context="$1"
  local cmd="$2"
  local safe_context
  safe_context="${context//\//_}"

  (
    local started
    local ended
    local duration
    started="$(date +%s)"
    if run_check "$context" "$cmd"; then
      ended="$(date +%s)"
      duration="$((ended - started))"
      printf 'ok %s\n' "$duration" >"/tmp/local-validate-${safe_context}.result"
      exit 0
    fi
    ended="$(date +%s)"
    duration="$((ended - started))"
    printf 'fail %s\n' "$duration" >"/tmp/local-validate-${safe_context}.result"
    exit 1
  ) &

  echo "$!"
}

report_async_result() {
  local context="$1"
  local safe_context
  local result_file
  local status_word
  local duration
  safe_context="${context//\//_}"
  result_file="/tmp/local-validate-${safe_context}.result"

  if [[ -f "$result_file" ]]; then
    read -r status_word duration <"$result_file"
    rm -f "$result_file"
    echo "[$context] total ${duration}s"
    [[ "$status_word" == "ok" ]]
    return
  fi

  echo "[$context] missing timing result" >&2
  return 1
}

echo "Validating PR #$PR_NUMBER ($SHA) in $BASE_REPO"
echo "Publishing commit statuses to: $REPO"

# macOS local builds can leave stale cmake output dirs where configure was skipped
# but no generator files remain. Clean those up before lint/test.
repair_stale_llama_build_dirs

# Run fast independent checks in parallel.
fmt_pid="$(run_check_async "local/fmt" "$fmt_cmd")"
biome_pid="$(run_check_async "local/biome" "$biome_cmd")"
zizmor_pid="$(run_check_async "local/zizmor" "$zizmor_cmd")"

parallel_failed=0
if ! wait "$fmt_pid"; then parallel_failed=1; fi
if ! report_async_result "local/fmt"; then parallel_failed=1; fi
if ! wait "$biome_pid"; then parallel_failed=1; fi
if ! report_async_result "local/biome"; then parallel_failed=1; fi
if ! wait "$zizmor_pid"; then parallel_failed=1; fi
if ! report_async_result "local/zizmor"; then parallel_failed=1; fi

if [[ "$parallel_failed" -ne 0 ]]; then
  echo "One or more parallel local checks failed." >&2
  exit 1
fi

# Keep lint/test sequential to maximize incremental compile reuse.
run_check "local/lint" "$lint_cmd"
run_check "local/test" "$test_cmd"

echo "All local validation statuses published successfully."
