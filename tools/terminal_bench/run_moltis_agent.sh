#!/usr/bin/env bash
set -euo pipefail

: "${MOLTIS_API_KEY:?MOLTIS_API_KEY must be set}"

MOLTIS_CTL_BIN="${MOLTIS_CTL_BIN:-moltis-ctl}"
MOLTIS_GATEWAY_URL="${MOLTIS_GATEWAY_URL:-http://host.docker.internal:13131}"
MOLTIS_SESSION_KEY="${MOLTIS_SESSION_KEY:-terminal-bench:${HARBOR_TASK_ID:-task}}"

if (($#)); then
  instruction="$*"
elif [[ -n "${HARBOR_TASK_INSTRUCTION:-}" ]]; then
  instruction="$HARBOR_TASK_INSTRUCTION"
else
  instruction="$(cat)"
fi

if [[ -z "${instruction//[[:space:]]/}" ]]; then
  echo "terminal-bench: missing task instruction" >&2
  exit 2
fi

exec "$MOLTIS_CTL_BIN" \
  --gateway-url "$MOLTIS_GATEWAY_URL" \
  --api-key "$MOLTIS_API_KEY" \
  chat \
  --session-key "$MOLTIS_SESSION_KEY" \
  "$instruction"
