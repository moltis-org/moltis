#!/usr/bin/env bash

set -euo pipefail

# Verify GPG signatures on Moltis release artifacts.
#
# Downloads the maintainer's public key from https://pen.so/gpg.asc,
# then verifies the detached GPG signature (.asc) for each artifact
# in the current directory or a specified release.
#
# Prerequisites:
#   - gpg
#   - curl (for key fetch) or gh (for release download)
#
# Usage:
#   ./scripts/verify-release.sh [OPTIONS] [FILE...]
#   ./scripts/verify-release.sh moltis-20260331.01-x86_64-unknown-linux-gnu.tar.gz
#   ./scripts/verify-release.sh --version 20260331.01
#   ./scripts/verify-release.sh *.tar.gz *.deb

GPG_KEY_URL="https://pen.so/gpg.asc"
EXPECTED_FINGERPRINT="310320A8CC1C5BA86AD09040C0451BADF7649BBF"
REPO="${MOLTIS_REPO:-moltis-org/moltis}"

usage() {
  cat <<'EOF'
Usage: ./scripts/verify-release.sh [OPTIONS] [FILE...]

Verifies GPG signatures on Moltis release artifacts.

  FILE          One or more local artifact files to verify (must have
                matching .asc files alongside them)

Options:
  -V, --version VER   Download and verify all artifacts for this release
  -k, --key URL       GPG public key URL (default: https://pen.so/gpg.asc)
  -s, --skip-key      Skip key import (already in keyring)
  --checksums         Also verify SHA256 checksums
  -h, --help          Show this help

Environment:
  MOLTIS_REPO         GitHub repo (default: moltis-org/moltis)

Examples:
  ./scripts/verify-release.sh --version 20260331.01
  ./scripts/verify-release.sh --checksums --version 20260331.01
  ./scripts/verify-release.sh moltis-*.tar.gz
EOF
}

# --- Parse arguments ---
VERSION=""
SKIP_KEY=false
VERIFY_CHECKSUMS=false
FILES=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    -V|--version)  VERSION="$2"; shift 2 ;;
    -k|--key)      GPG_KEY_URL="$2"; shift 2 ;;
    -s|--skip-key) SKIP_KEY=true; shift ;;
    --checksums)   VERIFY_CHECKSUMS=true; shift ;;
    -h|--help)     usage; exit 0 ;;
    -*)            echo "Unknown option: $1" >&2; usage; exit 1 ;;
    *)             FILES+=("$1"); shift ;;
  esac
done

# --- Preflight ---
if ! command -v gpg >/dev/null 2>&1; then
  echo "error: gpg is required but not found" >&2
  exit 1
fi

# --- Import maintainer key ---
if [[ "$SKIP_KEY" != true ]]; then
  echo "Fetching maintainer GPG key from $GPG_KEY_URL..."
  KEY_DATA="$(curl -fsSL "$GPG_KEY_URL")"
  if [[ -z "$KEY_DATA" ]]; then
    echo "error: failed to fetch GPG key from $GPG_KEY_URL" >&2
    exit 1
  fi
  # Verify fingerprint before importing into the real keyring
  ACTUAL_FINGERPRINT="$(echo "$KEY_DATA" | gpg --with-colons --import-options show-only --import 2>/dev/null \
    | awk -F: '/^fpr/ { print $10; exit }')"
  if [[ "$ACTUAL_FINGERPRINT" != "$EXPECTED_FINGERPRINT" ]]; then
    echo "error: fetched key fingerprint does not match expected maintainer key" >&2
    echo "  expected: $EXPECTED_FINGERPRINT" >&2
    echo "  actual:   $ACTUAL_FINGERPRINT" >&2
    exit 1
  fi

  echo "$KEY_DATA" | gpg --import 2>&1 || true
  echo ""
fi

# --- Download release artifacts if --version given ---
WORK_DIR=""
if [[ -n "$VERSION" ]]; then
  if ! command -v gh >/dev/null 2>&1; then
    echo "error: gh (GitHub CLI) is required for --version download" >&2
    exit 1
  fi

  if [[ ! "$VERSION" =~ ^[0-9]{8}\.[0-9]{1,2}$ ]]; then
    echo "error: '$VERSION' does not match YYYYMMDD.NN format" >&2
    exit 1
  fi

  WORK_DIR="$(mktemp -d)"
  trap 'rm -rf "$WORK_DIR"' EXIT

  echo "Downloading release artifacts for $VERSION..."
  PATTERNS=(
    '*.deb' '*.rpm' '*.pkg.tar.zst' '*.AppImage' '*.snap'
    '*.tar.gz' '*.zip' '*.exe'
    '*.cdx.json' '*.spdx.json'
    '*.asc'
  )
  if [[ "$VERIFY_CHECKSUMS" == true ]]; then
    PATTERNS+=('*.sha256')
  fi

  DL_ARGS=()
  for p in "${PATTERNS[@]}"; do
    DL_ARGS+=(--pattern "$p")
  done

  gh release download "$VERSION" \
    --repo "$REPO" \
    --dir "$WORK_DIR" \
    "${DL_ARGS[@]}"

  # Collect artifact files (not .asc/.sha256)
  while IFS= read -r -d '' f; do
    FILES+=("$f")
  done < <(find "$WORK_DIR" -maxdepth 1 -type f \
    \( -name '*.deb' -o -name '*.rpm' -o -name '*.pkg.tar.zst' \
       -o -name '*.AppImage' -o -name '*.snap' -o -name '*.tar.gz' \
       -o -name '*.zip' -o -name '*.exe' \
       -o -name '*.cdx.json' -o -name '*.spdx.json' \) \
    -print0 | sort -z)
fi

if [[ ${#FILES[@]} -eq 0 ]]; then
  echo "error: no files to verify. Provide files or use --version." >&2
  usage
  exit 1
fi

# --- Verify ---
PASS=0
FAIL=0
SKIP=0

for file in "${FILES[@]}"; do
  name="$(basename "$file")"
  asc="${file}.asc"

  echo "Verifying: $name"

  # SHA256 checksum (optional)
  if [[ "$VERIFY_CHECKSUMS" == true ]]; then
    sha256_file="${file}.sha256"
    if [[ -f "$sha256_file" ]]; then
      expected="$(awk '{print $1}' "$sha256_file")"
      actual="$(sha256sum "$file" 2>/dev/null | awk '{print $1}' || shasum -a 256 "$file" | awk '{print $1}')"
      if [[ "$expected" != "$actual" ]]; then
        echo "  FAIL: SHA256 mismatch" >&2
        echo "    expected: $expected" >&2
        echo "    actual:   $actual" >&2
        FAIL=$((FAIL + 1))
        continue
      fi
      echo "  SHA256: OK"
    else
      echo "  SHA256: no checksum file found" >&2
    fi
  fi

  # GPG signature
  if [[ ! -f "$asc" ]]; then
    echo "  SKIP: no .asc signature file found" >&2
    SKIP=$((SKIP + 1))
    continue
  fi

  GPG_OUTPUT="$(gpg --batch --verify "$asc" "$file" 2>&1)" && GPG_RC=0 || GPG_RC=$?
  if [[ $GPG_RC -eq 0 ]]; then
    # Show the signer identity (e.g. "Good signature from ...")
    echo "$GPG_OUTPUT" | grep -i 'good signature' | sed 's/^/  /' || echo "  GPG: OK"
    PASS=$((PASS + 1))
  else
    echo "  FAIL: GPG signature verification failed" >&2
    echo "$GPG_OUTPUT" | sed 's/^/    /' >&2
    FAIL=$((FAIL + 1))
  fi
done

# --- Summary ---
echo ""
echo "Results: $PASS passed, $FAIL failed, $SKIP skipped (${#FILES[@]} total)"

if [[ $FAIL -gt 0 ]]; then
  echo ""
  echo "ERROR: $FAIL artifact(s) failed verification!" >&2
  exit 1
fi

if [[ $PASS -eq 0 ]]; then
  echo ""
  echo "WARNING: no artifacts were verified (all skipped)" >&2
  exit 1
fi
