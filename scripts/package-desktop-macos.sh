#!/usr/bin/env bash
# Build "Moltis Desktop.app" (the gpui client in apps/desktop).
#
# Usage: scripts/package-desktop-macos.sh [OUTPUT_DIR]
#
# Environment:
#   MOLTIS_VERSION        version string for Info.plist (default 0.1.0)
#   DESKTOP_SIGN_IDENTITY codesign identity, e.g. "Developer ID Application: …".
#                         Unset: ad-hoc signature (local use only).
#   DESKTOP_TARGET        cargo target triple (default: host)
#   DESKTOP_DAEMON_BIN    a moltis binary to ship inside the app
#                         (Contents/MacOS/moltis). The app starts it as a user
#                         service when no daemon is running.
#
# The bundle is required for system notifications on macOS.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:-$ROOT/apps/desktop/target/bundle}"
VERSION="${MOLTIS_VERSION:-0.1.0}"
BUNDLE_ID="org.moltis.desktop"
APP="$OUT/Moltis Desktop.app"
ICONS="$ROOT/apps/macos/Assets.xcassets/AppIcon.appiconset"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "error: macOS bundles can only be built on macOS" >&2
  exit 1
fi

target_args=()
release_dir="$ROOT/apps/desktop/target/release"
if [[ -n "${DESKTOP_TARGET:-}" ]]; then
  target_args=(--target "$DESKTOP_TARGET")
  release_dir="$ROOT/apps/desktop/target/$DESKTOP_TARGET/release"
fi

echo "==> Building moltis-desktop (release)"
(cd "$ROOT/apps/desktop" && cargo build --release --locked "${target_args[@]}")

echo "==> Assembling $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$release_dir/moltis-desktop" "$APP/Contents/MacOS/moltis-desktop"
if [[ -n "${DESKTOP_DAEMON_BIN:-}" ]]; then
  echo "==> Bundling daemon $DESKTOP_DAEMON_BIN"
  cp "$DESKTOP_DAEMON_BIN" "$APP/Contents/MacOS/moltis"
fi

# Icon set from the existing app icon PNGs.
iconset="$(mktemp -d)/Moltis.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  cp "$ICONS/icon-$size.png" "$iconset/icon_${size}x${size}.png"
  double=$((size * 2))
  if [[ -f "$ICONS/icon-$double.png" ]]; then
    cp "$ICONS/icon-$double.png" "$iconset/icon_${size}x${size}@2x.png"
  fi
done
iconutil -c icns "$iconset" -o "$APP/Contents/Resources/Moltis.icns"
rm -rf "$(dirname "$iconset")"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Moltis</string>
  <key>CFBundleDisplayName</key><string>Moltis</string>
  <key>CFBundleIdentifier</key><string>${BUNDLE_ID}</string>
  <key>CFBundleExecutable</key><string>moltis-desktop</string>
  <key>CFBundleIconFile</key><string>Moltis</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
  <key>NSLocalNetworkUsageDescription</key>
  <string>Moltis finds moltis daemons on your local network.</string>
  <key>NSBonjourServices</key>
  <array><string>_moltis._tcp</string></array>
</dict>
</plist>
PLIST

if [[ -n "${DESKTOP_SIGN_IDENTITY:-}" ]]; then
  echo "==> Signing with $DESKTOP_SIGN_IDENTITY"
  sign_args=(--force --options runtime --timestamp --sign "$DESKTOP_SIGN_IDENTITY")
else
  echo "==> Ad-hoc signing (set DESKTOP_SIGN_IDENTITY for distribution)"
  sign_args=(--force --sign -)
fi
# Nested executables are signed before the bundle that seals them.
if [[ -f "$APP/Contents/MacOS/moltis" ]]; then
  codesign "${sign_args[@]}" "$APP/Contents/MacOS/moltis"
fi
codesign "${sign_args[@]}" "$APP"
codesign --verify --strict "$APP"

echo "==> Built $APP"
