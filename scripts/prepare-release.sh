#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage: ./scripts/prepare-release.sh [version]

Examples:
  ./scripts/prepare-release.sh              # auto: YYYYMMDD.NN from today + next seq
  ./scripts/prepare-release.sh 20260311.01  # explicit version

Version format: YYYYMMDD.NN (date + two-digit daily sequence number).

This command:
1) generates release notes for <version> via git-cliff from unreleased commits,
2) keeps a fresh empty [Unreleased] section at the top of CHANGELOG.md,
3) syncs Cargo.lock via cargo fetch.
EOF
}

if [[ $# -gt 1 ]]; then
  usage
  exit 1
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if ! command -v git-cliff >/dev/null 2>&1; then
  echo "git-cliff is required. Install it first (for example: cargo install git-cliff --locked)." >&2
  exit 1
fi

if [[ ! -f Cargo.toml || ! -f CHANGELOG.md || ! -f cliff.toml ]]; then
  echo "run this script from the repository root (Cargo.toml, CHANGELOG.md, cliff.toml required)" >&2
  exit 1
fi

# Compute or validate version
if [[ $# -eq 1 ]]; then
  new_version="$1"
  if ! [[ "$new_version" =~ ^[0-9]{8}\.[0-9]{1,2}$ ]]; then
    echo "invalid version: '$new_version' (expected YYYYMMDD.NN)" >&2
    exit 1
  fi
else
  # Auto-compute: today's date + next sequence number
  today="$(date -u +%Y%m%d)"
  # Find highest existing seq for today's tags
  max_seq=0
  for tag in $(git tag -l "${today}.*" 2>/dev/null); do
    seq="${tag#"${today}."}"
    if [[ "$seq" =~ ^[0-9]+$ ]] && [[ "10#$seq" -gt "10#$max_seq" ]]; then
      max_seq="$seq"
    fi
  done
  next_seq=$((10#$max_seq + 1))
  new_version="$(printf '%s.%02d' "$today" "$next_seq")"
  echo "auto-computed version: $new_version"
fi

release_date="$(echo "$new_version" | sed 's/^\([0-9]\{4\}\)\([0-9]\{2\}\)\([0-9]\{2\}\)\..*/\1-\2-\3/')"

if rg -q "^## \\[$new_version\\]" CHANGELOG.md; then
  echo "CHANGELOG.md already contains version $new_version" >&2
  exit 1
fi

release_section_tmp="$(mktemp)"
if ! git-cliff \
  --config cliff.toml \
  --unreleased \
  --tag "$new_version" \
  --strip all \
  > "$release_section_tmp"; then
  rm -f "$release_section_tmp"
  echo "failed to generate release notes via git-cliff" >&2
  exit 1
fi

dated_release_section_tmp="$(mktemp)"
if ! awk -v version="$new_version" -v date="$release_date" '
BEGIN {
  replaced = 0
}
{
  if (replaced == 0 && $0 ~ ("^## \\[" version "\\] - [0-9]{4}-[0-9]{2}-[0-9]{2}$")) {
    print "## [" version "] - " date
    replaced = 1
    next
  }
  print
}
END {
  if (replaced == 0) {
    exit 13
  }
}
' "$release_section_tmp" > "$dated_release_section_tmp"; then
  rc=$?
  rm -f "$release_section_tmp" "$dated_release_section_tmp"
  if [[ "$rc" -eq 13 ]]; then
    echo "git-cliff output did not contain expected release header for version $new_version" >&2
  fi
  exit 1
fi
mv "$dated_release_section_tmp" "$release_section_tmp"

changelog_tmp="$(mktemp)"
if ! awk -v release_section_file="$release_section_tmp" '
function print_empty_unreleased() {
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
}
function print_release_section(   line) {
  while ((getline line < release_section_file) > 0) {
    print line
  }
  close(release_section_file)
}
BEGIN {
  replaced = 0
  skipping_old_unreleased = 0
}
{
  if (replaced == 0 && $0 == "## [Unreleased]") {
    print_empty_unreleased()
    print ""
    print_release_section()
    print ""
    replaced = 1
    skipping_old_unreleased = 1
    next
  }
  if (skipping_old_unreleased == 1) {
    if ($0 ~ /^## \[[0-9]/) {
      skipping_old_unreleased = 0
      print
    }
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
  rm -f "$release_section_tmp" "$changelog_tmp"
  if [[ "$rc" -eq 12 ]]; then
    echo "failed to locate '## [Unreleased]' in CHANGELOG.md" >&2
  fi
  exit 1
fi
mv "$changelog_tmp" CHANGELOG.md
rm -f "$release_section_tmp"

cargo fetch
cargo fetch --locked

echo "Release prep complete:"
echo "  version: $new_version"
echo "  date:    $release_date"
