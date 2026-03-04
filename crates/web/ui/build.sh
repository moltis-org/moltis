#!/usr/bin/env bash
# Build Tailwind CSS for moltis gateway web UI.
#
# Usage:
#   ./build.sh          # production (minified)
#   ./build.sh --watch  # development (watch mode)

set -euo pipefail
cd "$(dirname "$0")"

# Resolve the tailwindcss binary: explicit override → local node_modules.
# When TAILWINDCSS is set (e.g. standalone binary from CI), skip npm entirely.
if [[ -n "${TAILWINDCSS:-}" ]]; then
  TAILWIND="$TAILWINDCSS"
else
  # Tailwind v4 resolves imports like `@import "tailwindcss"` from local
  # dependencies, so we must ensure node_modules exists before invoking any CLI.
  if [[ ! -x node_modules/.bin/tailwindcss || ! -d node_modules/tailwindcss ]]; then
    echo "tailwind deps missing — installing npm devDependencies..." >&2
    if [[ -f package-lock.json ]]; then
      npm ci --ignore-scripts
    else
      npm install --ignore-scripts
    fi
  fi

  if [[ -x node_modules/.bin/tailwindcss ]]; then
    TAILWIND="node_modules/.bin/tailwindcss"
  elif command -v tailwindcss &>/dev/null; then
    # Last-resort fallback for unusual environments.
    TAILWIND="tailwindcss"
  else
    echo "tailwindcss CLI not found (local or global)" >&2
    exit 1
  fi
fi

if [[ "${1:-}" == "--watch" ]]; then
  exec $TAILWIND -i input.css -o ../src/assets/style.css --watch
else
  exec $TAILWIND -i input.css -o ../src/assets/style.css --minify
fi
