#!/usr/bin/env bash
set -euo pipefail

# GitHub-hosted runners have limited disk. E2E only needs the built CLI binary
# plus web assets; the bulky Rust build intermediates can be removed safely.
if [ -x target/debug/moltis ]; then
  mkdir -p target/e2e-bin
  cp target/debug/moltis target/e2e-bin/moltis
  chmod +x target/e2e-bin/moltis
  echo "Preserved E2E binary at target/e2e-bin/moltis"
fi

rm -rf \
  target/debug/build \
  target/debug/deps \
  target/debug/examples \
  target/debug/incremental \
  target/debug/.fingerprint \
  target/debug/.cargo-lock \
  target/debug/lib*.rlib \
  target/debug/lib*.rmeta \
  target/debug/*.d \
  target/debug/*.dSYM \
  target/cargo-timings

df -h .
