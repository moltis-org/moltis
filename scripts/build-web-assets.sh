#!/usr/bin/env bash
# Build all web UI assets (Vite JS, Tailwind CSS, service worker).
#
# The standalone Tailwind binary can be passed via TAILWINDCSS env var
# to avoid the npm tailwindcss dependency (used by CI).
# All other builds require Node.js and npm.
#
# Usage:
#   ./scripts/build-web-assets.sh
#   TAILWINDCSS=./tailwindcss-linux-x64 ./scripts/build-web-assets.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WEB_UI="$SCRIPT_DIR/../crates/web/ui"

# Resolve TAILWINDCSS to an absolute path before changing directories.
if [[ -n "${TAILWINDCSS:-}" ]]; then
  TAILWINDCSS="$(cd "$(dirname "$TAILWINDCSS")" && pwd)/$(basename "$TAILWINDCSS")"
  export TAILWINDCSS
fi

cd "$WEB_UI"

# Ensure npm deps are installed (needed for Vite + esbuild, optionally Tailwind).
if [[ ! -d node_modules ]]; then
  echo "Installing npm dependencies..." >&2
  if [[ -f package-lock.json ]]; then
    npm ci --ignore-scripts
  else
    npm install --ignore-scripts
  fi
fi

# 1. Vite: TS/TSX -> dist/
echo "Building frontend (Vite)..." >&2
npm run build

# 2. Tailwind CSS
echo "Building CSS (Tailwind)..." >&2
if [[ -n "${TAILWINDCSS:-}" ]]; then
  "$TAILWINDCSS" -i input.css -o ../src/assets/css/style.css
else
  npm run build:css
fi
# Templates reference /assets/style.css (root level).
cp ../src/assets/css/style.css ../src/assets/style.css

# 3. Service worker
echo "Building service worker..." >&2
npm run build:sw

echo "Web assets built successfully." >&2
