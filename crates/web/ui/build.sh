#!/usr/bin/env bash
# Build Tailwind CSS for moltis gateway web UI.
#
# Usage:
#   ./build.sh          # production (minified)
#   ./build.sh --watch  # development (watch mode)

set -euo pipefail
cd "$(dirname "$0")"

# Ensure local node_modules include Tailwind CLI deps before resolving a binary.
if [[ ! -d node_modules/@tailwindcss/cli || ! -d node_modules/tailwindcss ]]; then
  echo "tailwind deps missing — installing npm devDependencies..." >&2
  if [[ -f package-lock.json ]]; then
    npm ci --ignore-scripts
  else
    npm install --ignore-scripts
  fi
fi

# Resolve the tailwindcss binary: explicit override → local node_modules → global CLI.
if [[ -n "${TAILWINDCSS:-}" ]]; then
  TAILWIND="$TAILWINDCSS"
elif [[ -x node_modules/.bin/tailwindcss ]]; then
  TAILWIND="node_modules/.bin/tailwindcss"
elif command -v tailwindcss &>/dev/null; then
  TAILWIND="tailwindcss"
else
  TAILWIND="npx --no-install @tailwindcss/cli"
fi

if [[ "${1:-}" == "--watch" ]]; then
  exec $TAILWIND -i input.css -o ../src/assets/style.css --watch
else
  exec $TAILWIND -i input.css -o ../src/assets/style.css --minify
fi
