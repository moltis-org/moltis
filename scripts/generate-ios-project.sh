#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
IOS_APP_DIR="${REPO_ROOT}/apps/ios"

if ! command -v xcodegen >/dev/null 2>&1; then
  echo "error: xcodegen is required (install with: brew install xcodegen)" >&2
  exit 1
fi

cd "${IOS_APP_DIR}"
xcodegen generate --spec project.yml

echo "Generated ${IOS_APP_DIR}/Moltis.xcodeproj"
