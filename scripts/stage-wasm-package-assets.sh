#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage: ./scripts/stage-wasm-package-assets.sh <target-release-dir>

Examples:
  ./scripts/stage-wasm-package-assets.sh target/release
  ./scripts/stage-wasm-package-assets.sh target/aarch64-unknown-linux-gnu/release

Copies WASM guest tools produced for wasm32-wasip2 into the requested
release directory so cargo-deb can package them using target/release paths.
EOF
}

if [[ $# -ne 1 ]]; then
  usage >&2
  exit 2
fi

dest_release_dir="$1"
source_dir="target/wasm32-wasip2/release"
artifacts=(
  "moltis_wasm_calc.wasm"
  "moltis_wasm_web_fetch.wasm"
  "moltis_wasm_web_search.wasm"
)

mkdir -p "$dest_release_dir"

for artifact in "${artifacts[@]}"; do
  src="$source_dir/$artifact"
  dest="$dest_release_dir/$artifact"
  if [[ ! -f "$src" ]]; then
    echo "missing wasm artifact: $src" >&2
    echo "run cargo build --target wasm32-wasip2 -p moltis-wasm-calc -p moltis-wasm-web-fetch -p moltis-wasm-web-search --release" >&2
    exit 1
  fi
  cp "$src" "$dest"
done

echo "Staged wasm packaging assets into $dest_release_dir"
