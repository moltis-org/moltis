#!/usr/bin/env bash
# Build the Moltis Desktop AppImage (the gpui client in apps/desktop) with the
# moltis daemon inside, so it needs nothing else installed: when no daemon is
# running, the app copies the bundled one to ~/.moltis/bin and installs it as
# a user service (systemd, or the portable supervisor without it).
#
# Usage: scripts/package-desktop-linux.sh [OUTPUT_DIR]
#
# Environment:
#   MOLTIS_VERSION          version of the AppImage and both binaries
#                           (default 0.1.0)
#   DESKTOP_TARGET          cargo target triple (default: host)
#   DESKTOP_DAEMON=0        leave the daemon out (the app then needs `moltis`
#                           on PATH)
#   DESKTOP_DAEMON_BIN      ship this moltis binary instead of building one
#   DESKTOP_DAEMON_FEATURES daemon cargo features (default "portable"; release
#                           CI adds "embedded-assets,embedded-wasm", since the
#                           copied daemon has no share dir beside it)
#   APPIMAGETOOL            appimagetool to use (default: download the pinned
#                           release and check its digest)
#
# Output: OUTPUT_DIR/moltis-desktop-<version>-<arch>.AppImage
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:-$ROOT/apps/desktop/target/bundle}"
VERSION="${MOLTIS_VERSION:-0.1.0}"
APP_ID="org.moltis.desktop"
ICONS="$ROOT/apps/macos/Assets.xcassets/AppIcon.appiconset"

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "error: AppImages can only be built on Linux" >&2
  exit 1
fi

target="${DESKTOP_TARGET:-}"
arch="${target%%-*}"
arch="${arch:-$(uname -m)}"

# Path of a release binary built for $target ("" = host).
release_path() {
  local target_dir="$1" name="$2"
  if [[ -n "$target" ]]; then
    printf '%s/%s/release/%s' "$target_dir" "$target" "$name"
  else
    printf '%s/release/%s' "$target_dir" "$name"
  fi
}

target_args=()
if [[ -n "$target" ]]; then
  target_args=(--target "$target")
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

export MOLTIS_VERSION="$VERSION"
echo "==> Building moltis-desktop (release${target:+, $target})"
(cd "$ROOT/apps/desktop" && cargo build --release --locked "${target_args[@]}")
desktop_bin="$(release_path "$ROOT/apps/desktop/target" moltis-desktop)"

daemon=""
if [[ -n "${DESKTOP_DAEMON_BIN:-}" ]]; then
  daemon="$DESKTOP_DAEMON_BIN"
elif [[ "${DESKTOP_DAEMON:-1}" != "0" ]]; then
  export MOLTIS_BUILD_FEATURES="${DESKTOP_DAEMON_FEATURES:-portable}"
  echo "==> Building moltis daemon ($MOLTIS_BUILD_FEATURES)"
  "$ROOT/scripts/cargo-build-moltis.sh" --release --locked "${target_args[@]}"
  daemon="$(release_path "$ROOT/target" moltis)"
fi

app_dir="$work/Moltis.AppDir"
echo "==> Assembling $app_dir"
mkdir -p "$app_dir/usr/bin" "$app_dir/usr/share/applications" \
  "$app_dir/usr/share/icons/hicolor/256x256/apps"
install -m 755 "$desktop_bin" "$app_dir/usr/bin/moltis-desktop"
if [[ -n "$daemon" ]]; then
  echo "==> Bundling daemon"
  install -m 755 "$daemon" "$app_dir/usr/bin/moltis"
else
  echo "==> No daemon bundled: the app will need moltis on PATH"
fi

cat > "$app_dir/usr/share/applications/$APP_ID.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Moltis
Comment=Chat with your Moltis agents
Exec=moltis-desktop
Icon=$APP_ID
Categories=Network;Chat;
StartupWMClass=$APP_ID
Terminal=false
X-AppImage-Version=$VERSION
DESKTOP
cp "$app_dir/usr/share/applications/$APP_ID.desktop" "$app_dir/$APP_ID.desktop"

install -m 644 "$ICONS/icon-256.png" \
  "$app_dir/usr/share/icons/hicolor/256x256/apps/$APP_ID.png"
install -m 644 "$ICONS/icon-256.png" "$app_dir/$APP_ID.png"
ln -s "$APP_ID.png" "$app_dir/.DirIcon"

# moltis-desktop finds the daemon next to itself (usr/bin/moltis).
cat > "$app_dir/AppRun" <<'APPRUN'
#!/bin/sh
SELF=$(readlink -f "$0")
HERE=${SELF%/*}
exec "$HERE/usr/bin/moltis-desktop" "$@"
APPRUN
chmod 755 "$app_dir/AppRun"

# Pinned appimagetool and AppImage runtime, checked against their digests
# before use (the "continuous" builds change under the same URL).
APPIMAGETOOL_VERSION="1.9.1"
RUNTIME_VERSION="20251108"
case "$arch" in
  x86_64)
    appimagetool_sha256="ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0"
    runtime_sha256="2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d"
    ;;
  aarch64)
    appimagetool_sha256="f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158"
    runtime_sha256="00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444"
    ;;
  *)
    echo "error: no pinned appimagetool for $arch" >&2
    exit 1
    ;;
esac

# Download `url` to `output` and fail unless it has the expected digest.
fetch_verified() {
  local url="$1" output="$2" sha256="$3"
  curl -fsSL -o "$output" "$url"
  echo "$sha256  $output" | sha256sum -c --quiet -
}

appimagetool="${APPIMAGETOOL:-}"
if [[ -z "$appimagetool" ]]; then
  appimagetool="$work/appimagetool"
  echo "==> Downloading appimagetool $APPIMAGETOOL_VERSION ($arch)"
  fetch_verified \
    "https://github.com/AppImage/appimagetool/releases/download/${APPIMAGETOOL_VERSION}/appimagetool-${arch}.AppImage" \
    "$appimagetool" "$appimagetool_sha256"
  chmod 755 "$appimagetool"
fi

# Without --runtime-file, appimagetool fetches the latest runtime itself.
runtime="$work/runtime-$arch"
echo "==> Downloading AppImage runtime $RUNTIME_VERSION ($arch)"
fetch_verified \
  "https://github.com/AppImage/type2-runtime/releases/download/${RUNTIME_VERSION}/runtime-${arch}" \
  "$runtime" "$runtime_sha256"

mkdir -p "$OUT"
image="$OUT/moltis-desktop-${VERSION}-${arch}.AppImage"
# --appimage-extract-and-run: no FUSE needed to run appimagetool itself.
ARCH="$arch" "$appimagetool" --appimage-extract-and-run --runtime-file "$runtime" \
  "$app_dir" "$image"

echo "==> Built $image"
