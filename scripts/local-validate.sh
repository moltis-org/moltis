#!/usr/bin/env bash

set -euo pipefail

ACTIVE_PIDS=()
CURRENT_PID=""
RUN_CHECK_ASYNC_PID=""

remove_active_pid() {
  local target="$1"
  local kept=()
  local pid
  for pid in "${ACTIVE_PIDS[@]}"; do
    if [[ "$pid" != "$target" ]]; then
      kept+=("$pid")
    fi
  done
  ACTIVE_PIDS=("${kept[@]}")
}

handle_interrupt() {
  echo "Interrupted: stopping local validation..." >&2

  if [[ -n "$CURRENT_PID" ]]; then
    kill -TERM "$CURRENT_PID" 2>/dev/null || true
  fi

  local pid
  for pid in "${ACTIVE_PIDS[@]}"; do
    kill -TERM "$pid" 2>/dev/null || true
  done

  sleep 1

  if [[ -n "$CURRENT_PID" ]]; then
    kill -KILL "$CURRENT_PID" 2>/dev/null || true
  fi

  for pid in "${ACTIVE_PIDS[@]}"; do
    kill -KILL "$pid" 2>/dev/null || true
  done

  exit 130
}

trap handle_interrupt INT TERM

# Detect local-only mode: no PR argument and no current PR on this branch.
LOCAL_ONLY=0
PR_NUMBER="${1:-}"

if [[ -z "$PR_NUMBER" ]]; then
  if command -v gh >/dev/null 2>&1 && PR_NUMBER="$(gh pr view --json number -q .number 2>/dev/null)"; then
    : # found a PR for the current branch
  else
    LOCAL_ONLY=1
  fi
fi

if [[ "$LOCAL_ONLY" -eq 0 ]]; then
  if ! command -v gh >/dev/null 2>&1; then
    echo "gh CLI is required for PR mode" >&2
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

  BASE_REPO="$(gh repo view --json nameWithOwner -q .nameWithOwner)"
  SHA="$(gh pr view "$PR_NUMBER" --repo "$BASE_REPO" --json headRefOid -q .headRefOid)"
  HEAD_OWNER="$(gh pr view "$PR_NUMBER" --repo "$BASE_REPO" --json headRepositoryOwner -q .headRepositoryOwner.login)"
  HEAD_REPO_NAME="$(gh pr view "$PR_NUMBER" --repo "$BASE_REPO" --json headRepository -q .headRepository.name)"

  if [[ -n "$HEAD_OWNER" && -n "$HEAD_REPO_NAME" ]]; then
    REPO="${HEAD_OWNER}/${HEAD_REPO_NAME}"
  else
    REPO="$BASE_REPO"
  fi

  if [[ "$(git rev-parse HEAD)" != "$SHA" ]]; then
    cat >&2 <<EOF
Current checkout does not match PR head commit.
  local HEAD: $(git rev-parse --short HEAD)
  PR head:    ${SHA:0:7}

Check out the PR head commit before running local validation.
EOF
    exit 1
  fi
else
  SHA="$(git rev-parse HEAD)"
fi

# Reject dirty working trees in all modes. Validating with uncommitted changes
# gives misleading results (local-only) or publishes statuses for the wrong
# content (PR mode).
if ! git diff --quiet --ignore-submodules -- || \
   ! git diff --cached --quiet --ignore-submodules -- || \
   [[ -n "$(git ls-files --others --exclude-standard)" ]]; then
  cat >&2 <<EOF
Working tree is not clean.

Commit or stash all local changes (including untracked files) before running
local validation.
EOF
  exit 1
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

  if [[ "$LOCAL_ONLY" -eq 1 ]]; then
    return 0
  fi

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
  local log_file=""

  start="$(date +%s)"
  set_status pending "$context" "Running locally"

  if [[ "$context" == "local/test" && -z "${LOCAL_VALIDATE_TEST_VERBOSE:-}" ]]; then
    log_file="$(mktemp -t local-validate-test.XXXXXX.log)"
    bash -lc "$cmd" >"$log_file" 2>&1 &
  else
    bash -lc "$cmd" &
  fi

  CURRENT_PID="$!"
  if wait "$CURRENT_PID"; then
    end="$(date +%s)"
    duration="$((end - start))"
    CURRENT_PID=""
    if [[ -n "$log_file" ]]; then
      rm -f "$log_file"
    fi
    set_status success "$context" "Passed locally"
    echo "[$context] passed in ${duration}s"
  else
    end="$(date +%s)"
    duration="$((end - start))"
    CURRENT_PID=""
    if [[ -n "$log_file" ]]; then
      echo "[$context] failed; showing captured output:" >&2
      cat "$log_file" >&2
      rm -f "$log_file"
    fi
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
    if run_check "$context" "$cmd" >&2; then
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
  local pid="$!"
  ACTIVE_PIDS+=("$pid")
  RUN_CHECK_ASYNC_PID="$pid"
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
    remove_active_pid "$2"
    echo "[$context] total ${duration}s"
    [[ "$status_word" == "ok" ]]
    return
  fi

  echo "[$context] missing timing result" >&2
  return 1
}

if [[ "$LOCAL_ONLY" -eq 1 ]]; then
  echo "Local-only validation (${SHA:0:7}) — no statuses will be published"
else
  echo "Validating PR #$PR_NUMBER ($SHA) in $BASE_REPO"
  echo "Publishing commit statuses to: $REPO"

  PR_CHECKS_URL="https://github.com/$BASE_REPO/pull/$PR_NUMBER/checks"
  RUN_URL="$(gh api "repos/$BASE_REPO/actions/runs?head_sha=$SHA&event=pull_request&per_page=1" --jq '.workflow_runs[0].html_url // empty' 2>/dev/null || true)"
  if [[ -n "$RUN_URL" ]]; then
    echo "Current CI workflow: $RUN_URL"
  else
    echo "Current CI checks: $PR_CHECKS_URL"
  fi
fi

# macOS local builds can leave stale cmake output dirs where configure was skipped
# but no generator files remain. Clean those up before lint/test.
repair_stale_llama_build_dirs

# Run fast independent checks in parallel.
run_check_async "local/fmt" "$fmt_cmd"
fmt_pid="$RUN_CHECK_ASYNC_PID"
run_check_async "local/biome" "$biome_cmd"
biome_pid="$RUN_CHECK_ASYNC_PID"
run_check_async "local/zizmor" "$zizmor_cmd"
zizmor_pid="$RUN_CHECK_ASYNC_PID"

parallel_failed=0
if ! wait "$fmt_pid"; then parallel_failed=1; fi
if ! report_async_result "local/fmt" "$fmt_pid"; then parallel_failed=1; fi
if ! wait "$biome_pid"; then parallel_failed=1; fi
if ! report_async_result "local/biome" "$biome_pid"; then parallel_failed=1; fi

if [[ "$parallel_failed" -ne 0 ]]; then
  echo "One or more parallel local checks failed." >&2
  exit 1
fi

# Verify Cargo.lock is in sync (same as CI's `cargo fetch --locked`).
run_check "local/lockfile" "cargo fetch --locked"

# Keep lint/test sequential to maximize incremental compile reuse.
# These do not wait on local/zizmor (advisory and non-blocking).
run_check "local/lint" "$lint_cmd"
run_check "local/test" "$test_cmd"

# Collect local/zizmor result at the end without affecting pass/fail.
if wait "$zizmor_pid"; then
  report_async_result "local/zizmor" "$zizmor_pid" || true
else
  report_async_result "local/zizmor" "$zizmor_pid" || true
fi

if [[ "$LOCAL_ONLY" -eq 1 ]]; then
  echo "All local checks passed."
else
  echo "All local validation statuses published successfully."
fi
