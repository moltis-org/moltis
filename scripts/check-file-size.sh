#!/usr/bin/env bash
# Fail if any Rust or TypeScript source file exceeds MAX_LINES (unless allowlisted).
# Allowlisted files are tracked for decomposition — remove entries as they're split.

set -euo pipefail

MAX_LINES=1500

# Files queued for decomposition — remove as they're split below the limit.
ALLOW_LIST=$'
'

# Check if a file is in the allowlist (bash 3.2 compatible).
is_allowed() {
  local needle="$1"
  [[ "$ALLOW_LIST" == *$'\n'"$needle"$'\n'* ]]
}

allowlisted_count() {
  printf '%s' "$ALLOW_LIST" | awk 'NF { count += 1 } END { print count + 0 }'
}

violations=0

while IFS=$'\t' read -r lines file; do
  rel="${file#./}"
  if is_allowed "$rel"; then
    continue
  fi
  echo "FAIL: $rel ($lines lines > $MAX_LINES)"
  violations=$((violations + 1))
done < <(
  find . \( -name '*.rs' -o -name '*.ts' -o -name '*.tsx' \) \
    -not -path './target/*' \
    -not -path './.claude/*' \
    -not -path '*/node_modules/*' \
    -not -path '*/e2e/*' \
    -print0 \
  | xargs -0 wc -l \
  | awk -v max="$MAX_LINES" '$2 != "total" && $1 > max { printf "%d\t%s\n", $1, $2 }' \
  | sort -rn
)

if [[ $violations -gt 0 ]]; then
  echo ""
  echo "$violations file(s) exceed $MAX_LINES lines."
  echo "Split them into smaller modules or add to the allowlist in $0."
  exit 1
fi

echo "All Rust and TypeScript files within $MAX_LINES-line limit ($(allowlisted_count) allowlisted)."
