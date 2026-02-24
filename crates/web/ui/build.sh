#!/usr/bin/env bash
# Build Tailwind CSS for moltis gateway web UI.
# Requires the Tailwind standalone CLI: https://tailwindcss.com/blog/standalone-cli
#
# Usage:
#   ./build.sh          # production (minified)
#   ./build.sh --watch  # development (watch mode)

set -euo pipefail
cd "$(dirname "$0")"

TAILWIND="${TAILWINDCSS:-tailwindcss}"

if ! command -v "$TAILWIND" &>/dev/null; then
  echo "Error: tailwindcss CLI not found."
  echo "Install: https://tailwindcss.com/blog/standalone-cli"
  echo "  or: npm install -g @tailwindcss/cli"
  exit 1
fi

if [[ "${1:-}" == "--watch" ]]; then
  exec "$TAILWIND" -i input.css -o ../src/assets/style.css --watch
else
  exec "$TAILWIND" -i input.css -o ../src/assets/style.css --minify
fi
