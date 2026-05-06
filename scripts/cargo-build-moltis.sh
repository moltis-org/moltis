#!/usr/bin/env bash
set -euo pipefail

features="${MOLTIS_BUILD_FEATURES:-full}"

cargo build -p moltis --no-default-features --features "$features" "$@"
