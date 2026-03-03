#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_path="$repo_root/install.sh"
target_path="$repo_root/website/install.sh"

if [[ ! -f "$source_path" ]]; then
  echo "missing source install script: $source_path" >&2
  exit 1
fi

if [[ ! -d "$repo_root/website" ]]; then
  echo "missing website directory: $repo_root/website" >&2
  exit 1
fi

cp "$source_path" "$target_path"
chmod +x "$target_path"

echo "synced $target_path from $source_path"
