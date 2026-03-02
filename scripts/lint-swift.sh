#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
MACOS_APP_DIR="${REPO_ROOT}/apps/macos"

if ! command -v swiftlint >/dev/null 2>&1; then
  echo "error: swiftlint is required (install with: brew install swiftlint)" >&2
  exit 1
fi

# SwiftLint/SourceKit may fail when only CommandLineTools are selected.
if [ -z "${DEVELOPER_DIR:-}" ] && [ -d "/Applications/Xcode.app/Contents/Developer" ]; then
  export DEVELOPER_DIR="/Applications/Xcode.app/Contents/Developer"
fi

cd "${MACOS_APP_DIR}"
swiftlint lint --config .swiftlint.yml --strict

echo "SwiftLint passed for swift app"
