#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
docs_path="$repo_root/docs/src/installation.md"

if [[ ! -f "$docs_path" ]]; then
  echo "missing installation docs: $docs_path" >&2
  exit 1
fi

pattern='releases/latest/download/moltis'

if hits=$(grep -n "$pattern" "$docs_path") && [[ -n "$hits" ]]; then
  echo "installation docs contain versionless GitHub asset URLs that drift from release filenames" >&2
  echo "$hits" >&2
  exit 1
fi

echo "installation docs avoid versionless GitHub asset URLs"
