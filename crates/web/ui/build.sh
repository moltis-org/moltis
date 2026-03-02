#!/usr/bin/env bash
# Build Tailwind CSS for moltis gateway web UI.
#
# Usage:
#   ./build.sh          # production (minified)
#   ./build.sh --watch  # development (watch mode)

set -euo pipefail
cd "$(dirname "$0")"

# Resolve the tailwindcss binary: explicit override → global CLI → local node_modules.
if [[ -n "${TAILWINDCSS:-}" ]]; then
  TAILWIND="$TAILWINDCSS"
elif command -v tailwindcss &>/dev/null; then
  TAILWIND="tailwindcss"
else
  # Ensure local node_modules are installed so npx can find @tailwindcss/cli.
  if [[ ! -d node_modules ]]; then
    echo "node_modules not found — running npm install..." >&2
    npm install --ignore-scripts
  fi
  TAILWIND="npx tailwindcss"
fi

if [[ "${1:-}" == "--watch" ]]; then
  exec $TAILWIND -i input.css -o ../src/assets/style.css --watch
else
  exec $TAILWIND -i input.css -o ../src/assets/style.css --minify
fi
