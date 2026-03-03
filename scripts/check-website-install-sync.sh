#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_path="$repo_root/install.sh"
target_path="$repo_root/website/install.sh"

if [[ ! -f "$source_path" ]]; then
  echo "missing source install script: $source_path" >&2
  exit 1
fi

if [[ ! -f "$target_path" ]]; then
  echo "missing website install script: $target_path" >&2
  exit 1
fi

if ! cmp -s "$source_path" "$target_path"; then
  echo "install.sh drift detected between repo root and website/" >&2
  echo "run ./scripts/sync-website-install.sh" >&2
  exit 1
fi

echo "install.sh is synced between repo root and website/"
