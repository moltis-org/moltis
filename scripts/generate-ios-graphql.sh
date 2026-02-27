#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
IOS_DIR="${REPO_ROOT}/apps/ios"
CLI_DIR="${IOS_DIR}/.tools"
CLI_BIN="${CLI_DIR}/apollo-ios-cli"
CLI_ARCHIVE="${CLI_DIR}/apollo-ios-cli.tar.gz"

mkdir -p "${CLI_DIR}"

if [[ ! -x "${CLI_BIN}" ]]; then
  echo "Installing apollo-ios-cli..."
  curl -fsSL -o "${CLI_ARCHIVE}" \
    "https://github.com/apollographql/apollo-ios/releases/latest/download/apollo-ios-cli.tar.gz"
  tar -xzf "${CLI_ARCHIVE}" -C "${CLI_DIR}"
  chmod +x "${CLI_BIN}"
fi

cd "${IOS_DIR}"
"${CLI_BIN}" generate --path "${IOS_DIR}/apollo-codegen-config.json"

echo "Generated Apollo GraphQL Swift types in ${IOS_DIR}/Sources/GraphQL/Generated"
