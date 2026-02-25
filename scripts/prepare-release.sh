#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage: ./scripts/prepare-release.sh <version> [release-date]

Examples:
  ./scripts/prepare-release.sh 0.8.22
  ./scripts/prepare-release.sh 0.8.22 2026-02-13

This command:
1) bumps [workspace.package].version in Cargo.toml,
2) rotates CHANGELOG.md ([Unreleased] -> [<version>] - <date>),
3) inserts a fresh empty [Unreleased] section,
4) syncs Cargo.lock via cargo fetch.
EOF
}

if [[ $# -lt 1 || $# -gt 2 ]]; then
  usage
  exit 1
fi

new_version="$1"
release_date="${2:-$(date -u +%Y-%m-%d)}"

if ! [[ "$new_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "invalid version: '$new_version' (expected x.y.z)" >&2
  exit 1
fi

if ! [[ "$release_date" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]]; then
  echo "invalid release date: '$release_date' (expected YYYY-MM-DD)" >&2
  exit 1
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if [[ ! -f Cargo.toml || ! -f CHANGELOG.md ]]; then
  echo "run this script from the repository root (Cargo.toml and CHANGELOG.md required)" >&2
  exit 1
fi

if rg -q "^## \\[$new_version\\]" CHANGELOG.md; then
  echo "CHANGELOG.md already contains version $new_version" >&2
  exit 1
fi

cargo_tmp="$(mktemp)"
if ! awk -v version="$new_version" '
BEGIN {
  in_workspace_package = 0
  updated = 0
}
{
  if ($0 == "[workspace.package]") {
    in_workspace_package = 1
    print
    next
  }
  if (in_workspace_package == 1 && $0 ~ /^\[/) {
    in_workspace_package = 0
  }
  if (in_workspace_package == 1 && $0 ~ /^version[[:space:]]*=/) {
    sub(/"[^"]+"/, "\"" version "\"")
    updated = 1
  }
  print
}
END {
  if (updated == 0) {
    exit 11
  }
}
' Cargo.toml > "$cargo_tmp"; then
  rc=$?
  rm -f "$cargo_tmp"
  if [[ "$rc" -eq 11 ]]; then
    echo "failed to locate [workspace.package].version in Cargo.toml" >&2
  fi
  exit 1
fi
mv "$cargo_tmp" Cargo.toml

changelog_tmp="$(mktemp)"
if ! awk -v version="$new_version" -v date="$release_date" '
BEGIN {
  replaced = 0
}
{
  if (replaced == 0 && $0 == "## [Unreleased]") {
    print "## [Unreleased]"
    print ""
    print "### Added"
    print ""
    print "### Changed"
    print ""
    print "### Deprecated"
    print ""
    print "### Removed"
    print ""
    print "### Fixed"
    print ""
    print "### Security"
    print ""
    print "## [" version "] - " date
    print ""
    replaced = 1
    next
  }
  print
}
END {
  if (replaced == 0) {
    exit 12
  }
}
' CHANGELOG.md > "$changelog_tmp"; then
  rc=$?
  rm -f "$changelog_tmp"
  if [[ "$rc" -eq 12 ]]; then
    echo "failed to locate '## [Unreleased]' in CHANGELOG.md" >&2
  fi
  exit 1
fi
mv "$changelog_tmp" CHANGELOG.md

cargo fetch
cargo fetch --locked

echo "Release prep complete:"
echo "  version: $new_version"
echo "  date:    $release_date"
