#!/usr/bin/env bash
# Verifies that install.sh constructs download filenames matching the patterns
# produced by the release workflow for every package type.
#
# This catches drift between the installer and CI packaging (e.g. the -1
# revision suffix that cargo-deb does not emit).
#
# Expected filename patterns (from .github/workflows/release.yml):
#
#   deb:     moltis_${VERSION}_${ARCH}.deb
#            (cargo deb --deb-version "$VERSION" — no revision suffix)
#
#   rpm:     moltis-${VERSION}-1.${ARCH}.rpm
#            (cargo generate-rpm adds -1 release)
#
#   arch:    moltis-${VERSION}-1-${ARCH}.pkg.tar.zst
#            (manual tar with pkgver = ${VERSION}-1)
#
#   appimage: moltis-${VERSION}-${ARCH}.AppImage
#             (release workflow packages AppImage without a revision suffix)
#
#   binary:  moltis-${VERSION}-${TARGET}.tar.gz
#            (manual tar)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INSTALL_SH="$REPO_ROOT/install.sh"
RELEASE_YML="$REPO_ROOT/.github/workflows/release.yml"

if [[ ! -f "$INSTALL_SH" ]]; then
  echo "FAIL: install.sh not found at $INSTALL_SH" >&2
  exit 1
fi

FAILURES=0
CHECKS=0

fail() {
  echo "FAIL: $1" >&2
  FAILURES=$((FAILURES + 1))
}

pass() {
  echo "  ok: $1"
  CHECKS=$((CHECKS + 1))
}

# ---------------------------------------------------------------------------
# Check 1: .deb filename must NOT have a revision suffix
#
# Release workflow: cargo deb --deb-version "$MOLTIS_VERSION"
#   → produces moltis_${VERSION}_${ARCH}.deb (no -1)
#
# install.sh must use: moltis_${version}_${deb_arch}.deb
# ---------------------------------------------------------------------------

echo "Checking .deb filename pattern..."

deb_line=$(grep 'deb_file=' "$INSTALL_SH" | head -1)

if echo "$deb_line" | grep -q '_${version}_${deb_arch}\.deb'; then
  pass ".deb filename: no revision suffix (matches cargo-deb output)"
elif echo "$deb_line" | grep -q '_${version}-[0-9]'; then
  fail ".deb filename has revision suffix but cargo-deb --deb-version does not produce one. Line: $deb_line"
else
  fail ".deb filename pattern unrecognized. Line: $deb_line"
fi

# ---------------------------------------------------------------------------
# Check 2: .rpm filename must have -1 revision
#
# Release workflow: cargo generate-rpm --set-metadata="version=\"$VERSION\""
#   → produces moltis-${VERSION}-1.${ARCH}.rpm
# ---------------------------------------------------------------------------

echo "Checking .rpm filename pattern..."

rpm_line=$(grep 'rpm_file=' "$INSTALL_SH" | head -1)

if echo "$rpm_line" | grep -q '\-${version}-1\.${rpm_arch}\.rpm'; then
  pass ".rpm filename: has -1 revision (matches cargo-generate-rpm output)"
else
  fail ".rpm filename pattern does not match expected 'moltis-\${version}-1.\${rpm_arch}.rpm'. Line: $rpm_line"
fi

# ---------------------------------------------------------------------------
# Check 3: .pkg.tar.zst (Arch) filename must have -1 revision
#
# Release workflow: tar ... "moltis-${VERSION}-1-${MATRIX_ARCH}.pkg.tar.zst"
# ---------------------------------------------------------------------------

echo "Checking .pkg.tar.zst (Arch) filename pattern..."

arch_line=$(grep 'pkg_file=' "$INSTALL_SH" | head -1)

if echo "$arch_line" | grep -q '\-${version}-1-${arch}\.pkg\.tar\.zst'; then
  pass ".pkg.tar.zst filename: has -1 revision (matches release workflow)"
else
  fail ".pkg.tar.zst filename does not match expected 'moltis-\${version}-1-\${arch}.pkg.tar.zst'. Line: $arch_line"
fi

# ---------------------------------------------------------------------------
# Check 4: AppImage filename
#
# Release workflow: appimagetool ... "moltis-${VERSION}-${MATRIX_ARCH}.AppImage"
# ---------------------------------------------------------------------------

echo "Checking AppImage filename pattern..."

appimage_line=$(grep 'appimage_file=' "$INSTALL_SH" | head -1)

if echo "$appimage_line" | grep -q '\-${version}-${arch}\.AppImage'; then
  pass "AppImage filename: matches release workflow pattern"
else
  fail "AppImage filename does not match expected 'moltis-\${version}-\${arch}.AppImage'. Line: $appimage_line"
fi

# ---------------------------------------------------------------------------
# Check 5: binary tarball filename
#
# Release workflow: tar ... "moltis-${VERSION}-${BUILD_TARGET}.tar.gz"
# ---------------------------------------------------------------------------

echo "Checking binary tarball filename pattern..."

tarball_line=$(grep 'tarball=' "$INSTALL_SH" | head -1)

if echo "$tarball_line" | grep -q '${BINARY_NAME}-${version}-${target}\.tar\.gz'; then
  pass "binary tarball: matches release workflow pattern"
else
  fail "binary tarball pattern does not match expected '\${BINARY_NAME}-\${version}-\${target}.tar.gz'. Line: $tarball_line"
fi

# ---------------------------------------------------------------------------
# Check 6: release_tag() handles date-based and semver versions
#
# Date-based (YYYYMMDD.NN) → bare tag
# Semver → v-prefixed
# ---------------------------------------------------------------------------

echo "Checking release_tag() logic..."

# Source just the release_tag function
eval "$(sed -n '/^release_tag()/,/^}/p' "$INSTALL_SH")"

tag=$(release_tag "20260327.05")
CHECKS=$((CHECKS + 1))
if [[ "$tag" == "20260327.05" ]]; then
  pass "release_tag('20260327.05') = '20260327.05' (date-based, bare)"
else
  fail "release_tag('20260327.05'): expected '20260327.05', got '$tag'"
fi

tag=$(release_tag "0.1.3")
CHECKS=$((CHECKS + 1))
if [[ "$tag" == "v0.1.3" ]]; then
  pass "release_tag('0.1.3') = 'v0.1.3' (semver, v-prefixed)"
else
  fail "release_tag('0.1.3'): expected 'v0.1.3', got '$tag'"
fi

# ---------------------------------------------------------------------------
# Check 7: .deb arch mapping
# ---------------------------------------------------------------------------

echo "Checking architecture mappings..."

# deb: x86_64→amd64, aarch64→arm64
if grep -q 'x86_64) deb_arch="amd64"' "$INSTALL_SH"; then
  pass "deb arch: x86_64 → amd64"
else
  fail "deb arch mapping for x86_64 not found"
fi

if grep -q 'aarch64) deb_arch="arm64"' "$INSTALL_SH"; then
  pass "deb arch: aarch64 → arm64"
else
  fail "deb arch mapping for aarch64 not found"
fi

# ---------------------------------------------------------------------------
# Check 8: Cross-validate against release.yml if present
# ---------------------------------------------------------------------------

if [[ -f "$RELEASE_YML" ]]; then
  echo "Cross-validating against release.yml..."

  # Verify cargo-deb does NOT use --deb-revision (which would add -1)
  if grep -q '\-\-deb-revision' "$RELEASE_YML"; then
    fail "release.yml uses --deb-revision which adds a suffix, but install.sh assumes no suffix"
  else
    pass "release.yml: no --deb-revision flag (consistent with install.sh)"
  fi

  # Verify cargo-deb uses --deb-version
  if grep -q '\-\-deb-version' "$RELEASE_YML"; then
    pass "release.yml: uses --deb-version for .deb naming"
  else
    fail "release.yml: --deb-version not found — .deb naming may have changed"
  fi

  if grep -q "\"moltis-\${VERSION}-\${MATRIX_ARCH}.AppImage\"" "$RELEASE_YML"; then
    pass "release.yml: AppImage naming matches install.sh"
  else
    fail "release.yml: AppImage naming pattern changed — install.sh may be stale"
  fi
fi

# ---------------------------------------------------------------------------
# Check 9: Verify both install scripts are in sync (belt + suspenders)
# ---------------------------------------------------------------------------

echo "Checking install.sh sync..."

WEBSITE_SH="$REPO_ROOT/website/install.sh"
if [[ -f "$WEBSITE_SH" ]]; then
  if cmp -s "$INSTALL_SH" "$WEBSITE_SH"; then
    pass "install.sh and website/install.sh are identical"
  else
    fail "install.sh and website/install.sh have diverged — run ./scripts/sync-website-install.sh"
  fi
fi

# --- Summary ---
echo ""
if [[ "$FAILURES" -gt 0 ]]; then
  echo "FAILED: $FAILURES of $CHECKS checks failed"
  exit 1
else
  echo "All $CHECKS install package name checks passed"
fi
