#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ -f .envrc ]]; then
  set -a
  # shellcheck disable=SC1091
  source .envrc >/dev/null 2>&1 || true
  set +a
fi

echo "Running provider E2E scenario checks..."

cargo test \
  -p moltis-providers \
  --test tool_arg_serialization_integration \
  -- \
  --ignored \
  --nocapture \
  --test-threads=1
