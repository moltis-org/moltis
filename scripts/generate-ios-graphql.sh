#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
IOS_DIR="${REPO_ROOT}/apps/ios"
CLI_DIR="${IOS_DIR}/.tools"
CLI_BIN="${CLI_DIR}/apollo-ios-cli"
CLI_ARCHIVE="${CLI_DIR}/apollo-ios-cli.tar.gz"
APOLLO_IOS_VERSION="2.1.0"

mkdir -p "${CLI_DIR}"

CURRENT_VERSION=""
if [[ -x "${CLI_BIN}" ]]; then
  CURRENT_VERSION="$("${CLI_BIN}" --version 2>/dev/null || true)"
fi

if [[ "${CURRENT_VERSION}" != "${APOLLO_IOS_VERSION}" ]]; then
  echo "Installing apollo-ios-cli ${APOLLO_IOS_VERSION}..."
  curl -fsSL -o "${CLI_ARCHIVE}" \
    "https://github.com/apollographql/apollo-ios/releases/download/${APOLLO_IOS_VERSION}/apollo-ios-cli.tar.gz"
  rm -f "${CLI_BIN}"
  tar -xzf "${CLI_ARCHIVE}" -C "${CLI_DIR}"
  chmod +x "${CLI_BIN}"
fi

cd "${IOS_DIR}"
"${CLI_BIN}" generate --path "${IOS_DIR}/apollo-codegen-config.json"

echo "Generated Apollo GraphQL Swift types in ${IOS_DIR}/Sources/GraphQL/Generated"
