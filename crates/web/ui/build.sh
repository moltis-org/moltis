#!/usr/bin/env bash
# Build Tailwind CSS for moltis gateway web UI.
#
# Usage:
#   ./build.sh          # production (minified)
#   ./build.sh --watch  # development (watch mode)

set -euo pipefail
cd "$(dirname "$0")"

# Resolve the tailwindcss binary: explicit override → local node_modules → global CLI.
# When TAILWINDCSS is set (e.g. standalone binary from CI), skip npm entirely.
if [[ -n "${TAILWINDCSS:-}" ]]; then
  TAILWIND="$TAILWINDCSS"
elif [[ -x node_modules/.bin/tailwindcss ]]; then
  TAILWIND="node_modules/.bin/tailwindcss"
elif command -v tailwindcss &>/dev/null; then
  TAILWIND="tailwindcss"
else
  # No binary found — install via npm and use the local copy.
  echo "tailwind deps missing — installing npm devDependencies..." >&2
  if [[ -f package-lock.json ]]; then
    npm ci --ignore-scripts
  else
    npm install --ignore-scripts
  fi
  TAILWIND="node_modules/.bin/tailwindcss"
fi

if [[ "${1:-}" == "--watch" ]]; then
  exec $TAILWIND -i input.css -o ../src/assets/style.css --watch
else
  exec $TAILWIND -i input.css -o ../src/assets/style.css --minify
fi
