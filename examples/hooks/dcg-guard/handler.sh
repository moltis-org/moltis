#!/usr/bin/env bash
# Hook handler: translates Moltis BeforeToolCall payload to dcg format.
# When dcg is not installed the hook is a no-op (all commands pass through).

set -euo pipefail

# Gracefully skip when dcg is not installed.
if ! command -v dcg >/dev/null 2>&1; then
    cat >/dev/null   # drain stdin
    exit 0
fi

INPUT=$(cat)

# Only inspect exec tool calls.
TOOL_NAME=$(printf '%s' "$INPUT" | grep -o '"tool_name":"[^"]*"' | head -1 | cut -d'"' -f4)
if [ "$TOOL_NAME" != "exec" ]; then
    exit 0
fi

# Extract the command string from the arguments object.
COMMAND=$(printf '%s' "$INPUT" | grep -o '"command":"[^"]*"' | head -1 | cut -d'"' -f4)
if [ -z "$COMMAND" ]; then
    exit 0
fi

# Build the payload dcg expects and pipe it in.
DCG_INPUT=$(printf '{"tool_name":"Bash","tool_input":{"command":"%s"}}' "$COMMAND")
DCG_RESULT=$(printf '%s' "$DCG_INPUT" | dcg 2>&1) || {
    # dcg returned non-zero — command is destructive.
    echo "$DCG_RESULT" >&2
    exit 1
}

# dcg returned 0 — command is safe.
exit 0
