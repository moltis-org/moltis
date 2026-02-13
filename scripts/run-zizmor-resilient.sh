#!/usr/bin/env bash

set -euo pipefail

max_attempts="${ZIZMOR_ONLINE_RETRIES:-3}"

if [[ "$#" -lt 1 ]]; then
  echo "Usage: $0 <zizmor args...>" >&2
  exit 2
fi

is_network_error() {
  local log_file="$1"
  rg -q \
    "request error while accessing GitHub API|connection closed before message completed|couldn't list tags|fatal: no audit was performed|git-upload-pack|error sending request for url" \
    "$log_file"
}

attempt=1
while [[ "$attempt" -le "$max_attempts" ]]; do
  log_file="$(mktemp -t zizmor-local.XXXXXX.log)"
  if zizmor "$@" 2>&1 | tee "$log_file"; then
    rm -f "$log_file"
    exit 0
  fi

  if ! is_network_error "$log_file"; then
    rm -f "$log_file"
    exit 1
  fi

  rm -f "$log_file"
  if [[ "$attempt" -lt "$max_attempts" ]]; then
    echo "zizmor online audits hit network errors (attempt ${attempt}/${max_attempts}); retrying..." >&2
    sleep "$attempt"
  fi
  attempt=$((attempt + 1))
done

if [[ "$*" == *"--no-online-audits"* ]]; then
  echo "zizmor failed after ${max_attempts} network retries." >&2
  exit 1
fi

echo "zizmor online audits unavailable after ${max_attempts} attempts; falling back to --no-online-audits" >&2
exec zizmor "$@" --no-online-audits
