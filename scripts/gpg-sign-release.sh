#!/usr/bin/env bash

set -euo pipefail

# GPG-sign release artifacts using a local key (e.g. YubiKey-resident).
#
# This script downloads release artifacts from a GitHub release, creates
# detached GPG signatures (.asc), and uploads them back to the release.
# It complements the CI-generated Sigstore signatures with a personal
# GPG signature tied to the release maintainer's identity.
#
# Prerequisites:
#   - gh (GitHub CLI) authenticated with repo access
#   - gpg with your signing key available (e.g. YubiKey plugged in)
#
# Usage:
#   ./scripts/gpg-sign-release.sh [VERSION]
#   ./scripts/gpg-sign-release.sh 20260331.01
#   ./scripts/gpg-sign-release.sh              # signs the latest release

usage() {
  cat <<'EOF'
Usage: ./scripts/gpg-sign-release.sh [VERSION]

Signs release artifacts with a local GPG key (e.g. YubiKey-resident).

  VERSION   Tag to sign (e.g. 20260331.01). Defaults to the latest release.

Options:
  -k, --key KEY_ID    GPG key ID or fingerprint to sign with
  -n, --dry-run       Download and sign but do not upload .asc files
  -y, --yes           Skip confirmation prompt
  -h, --help          Show this help

Environment:
  GPG_KEY_ID          Default GPG key (overridden by --key)
  MOLTIS_REPO         GitHub repo (default: moltis-org/moltis)

Examples:
  ./scripts/gpg-sign-release.sh                          # latest, default key
  ./scripts/gpg-sign-release.sh -k 0xABCD1234 20260331.01
  GPG_KEY_ID=0xABCD1234 ./scripts/gpg-sign-release.sh
EOF
}

# --- Parse arguments ---
KEY_ID="${GPG_KEY_ID:-}"
REPO="${MOLTIS_REPO:-moltis-org/moltis}"
DRY_RUN=false
SKIP_CONFIRM=false
VERSION=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    -k|--key)    KEY_ID="$2"; shift 2 ;;
    -n|--dry-run) DRY_RUN=true; shift ;;
    -y|--yes)    SKIP_CONFIRM=true; shift ;;
    -h|--help)   usage; exit 0 ;;
    -*)          echo "Unknown option: $1" >&2; usage; exit 1 ;;
    *)           VERSION="$1"; shift ;;
  esac
done

# --- Preflight checks ---
for cmd in gh gpg; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "error: $cmd is required but not found" >&2
    exit 1
  fi
done

# Resolve version
if [[ -z "$VERSION" ]]; then
  VERSION="$(gh release view --repo "$REPO" --json tagName --jq '.tagName')"
  echo "Resolved latest release: $VERSION"
fi

# Validate tag format
if [[ ! "$VERSION" =~ ^[0-9]{8}\.[0-9]{1,2}$ ]]; then
  echo "error: '$VERSION' does not match YYYYMMDD.NN format" >&2
  exit 1
fi

# Verify the release exists
if ! gh release view "$VERSION" --repo "$REPO" --json tagName >/dev/null 2>&1; then
  echo "error: release '$VERSION' not found in $REPO" >&2
  exit 1
fi

# Verify GPG key is available
GPG_SIGN_ARGS=()
if [[ -n "$KEY_ID" ]]; then
  GPG_SIGN_ARGS+=(--local-user "$KEY_ID")
  echo "Using GPG key: $KEY_ID"
else
  # Let GPG use its default key; verify one exists
  DEFAULT_KEY="$(gpg --list-secret-keys --keyid-format long 2>/dev/null | head -1 || true)"
  if [[ -z "$DEFAULT_KEY" ]]; then
    echo "error: no GPG secret key found. Specify --key or set GPG_KEY_ID" >&2
    exit 1
  fi
  echo "Using default GPG signing key"
fi

# Quick liveness check — attempt a throwaway signature to confirm the key
# is accessible (prompts for YubiKey PIN/touch if needed early).
PROBE="$(mktemp)"
echo "gpg-sign-release probe" > "$PROBE"
if ! gpg --batch "${GPG_SIGN_ARGS[@]}" --armor --detach-sign "$PROBE" 2>/dev/null; then
  rm -f "$PROBE" "${PROBE}.asc"
  echo "error: GPG signing failed. Is your YubiKey inserted and unlocked?" >&2
  exit 1
fi
rm -f "$PROBE" "${PROBE}.asc"
echo "GPG key verified (signing probe succeeded)"

# --- Download release artifacts ---
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

echo ""
echo "Downloading release artifacts for $VERSION..."
gh release download "$VERSION" \
  --repo "$REPO" \
  --dir "$WORK_DIR" \
  --pattern '*.deb' \
  --pattern '*.rpm' \
  --pattern '*.pkg.tar.zst' \
  --pattern '*.AppImage' \
  --pattern '*.snap' \
  --pattern '*.tar.gz' \
  --pattern '*.zip' \
  --pattern '*.exe' \
  --pattern '*.cdx.json' \
  --pattern '*.spdx.json'

ARTIFACTS=()
while IFS= read -r -d '' f; do
  ARTIFACTS+=("$f")
done < <(find "$WORK_DIR" -maxdepth 1 -type f \
  \( -name '*.deb' -o -name '*.rpm' -o -name '*.pkg.tar.zst' \
     -o -name '*.AppImage' -o -name '*.snap' -o -name '*.tar.gz' \
     -o -name '*.zip' -o -name '*.exe' \
     -o -name '*.cdx.json' -o -name '*.spdx.json' \) \
  -print0 | sort -z)

if [[ ${#ARTIFACTS[@]} -eq 0 ]]; then
  echo "error: no signable artifacts found in release $VERSION" >&2
  exit 1
fi

echo ""
echo "Artifacts to sign (${#ARTIFACTS[@]}):"
for f in "${ARTIFACTS[@]}"; do
  echo "  $(basename "$f")"
done

# --- Confirm ---
if [[ "$SKIP_CONFIRM" != true ]]; then
  echo ""
  if [[ "$DRY_RUN" == true ]]; then
    read -r -p "Sign these artifacts (dry-run, no upload)? [y/N] " confirm
  else
    read -r -p "Sign and upload .asc files to release $VERSION? [y/N] " confirm
  fi
  if [[ ! "$confirm" =~ ^[Yy]$ ]]; then
    echo "Aborted."
    exit 0
  fi
fi

# --- Sign ---
echo ""
ASC_FILES=()
for file in "${ARTIFACTS[@]}"; do
  name="$(basename "$file")"
  echo "Signing: $name"

  # Verify existing checksum if available (defense against tampered downloads)
  sha256_file="${WORK_DIR}/${name}.sha256"
  if [[ -f "$sha256_file" ]]; then
    echo "  Verifying SHA256..."
  elif gh release download "$VERSION" --repo "$REPO" --dir "$WORK_DIR" --pattern "${name}.sha256" 2>/dev/null; then
    sha256_file="${WORK_DIR}/${name}.sha256"
  else
    sha256_file=""
  fi

  if [[ -n "$sha256_file" && -f "$sha256_file" ]]; then
    expected="$(awk '{print $1}' "$sha256_file")"
    actual="$(sha256sum "$file" 2>/dev/null | awk '{print $1}' || shasum -a 256 "$file" | awk '{print $1}')"
    if [[ "$expected" != "$actual" ]]; then
      echo "  ERROR: SHA256 mismatch for $name!" >&2
      echo "    expected: $expected" >&2
      echo "    actual:   $actual" >&2
      exit 1
    fi
    echo "  SHA256 verified"
  else
    echo "  WARNING: no SHA256 checksum found for $name — skipping integrity check" >&2
  fi

  gpg --batch "${GPG_SIGN_ARGS[@]}" --armor --detach-sign "$file"
  ASC_FILES+=("${file}.asc")
  echo "  Created: ${name}.asc"
done

echo ""
echo "Signed ${#ASC_FILES[@]} artifacts."

# --- Upload ---
if [[ "$DRY_RUN" == true ]]; then
  echo ""
  echo "Dry run — skipping upload. Signatures in: $WORK_DIR"
  echo "To upload manually:"
  echo "  gh release upload $VERSION --repo $REPO ${WORK_DIR}/*.asc"
  # Keep work dir on dry-run so user can inspect
  trap - EXIT
  exit 0
fi

echo ""
echo "Uploading .asc files to release $VERSION..."

# Check if any .asc files already exist on the release
EXISTING_ASC="$(gh release view "$VERSION" --repo "$REPO" --json assets --jq '[.assets[].name | select(endswith(".asc"))] | join(", ")')"
if [[ -n "$EXISTING_ASC" ]]; then
  echo "  NOTE: replacing existing signatures: $EXISTING_ASC"
fi

gh release upload "$VERSION" \
  --repo "$REPO" \
  --clobber \
  "${ASC_FILES[@]}"

echo ""
echo "Done! GPG signatures uploaded to release $VERSION."
echo ""
echo "Users can verify with:"
echo "  ./scripts/verify-release.sh --version $VERSION"
echo ""
echo "Or manually:"
echo "  curl -fsSL https://pen.so/gpg.asc | gpg --import"
echo "  gpg --verify moltis-${VERSION}-x86_64-unknown-linux-gnu.tar.gz.asc \\"
echo "               moltis-${VERSION}-x86_64-unknown-linux-gnu.tar.gz"
