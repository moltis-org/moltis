#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

cat >"$tmp/moltis-ctl" <<'MOCK'
#!/usr/bin/env bash
printf '%s\n' "$@" >"${CAPTURE_FILE:?}"
MOCK
chmod +x "$tmp/moltis-ctl"

export CAPTURE_FILE="$tmp/args"
export MOLTIS_API_KEY="test-key"
export MOLTIS_CTL_BIN="$tmp/moltis-ctl"
export MOLTIS_GATEWAY_URL="http://gateway:13131"
export HARBOR_TASK_ID="representative-task"

"$script_dir/run_moltis_agent.sh" "write" "the file"

mapfile -t args <"$CAPTURE_FILE"
expected=(
  --gateway-url http://gateway:13131
  --api-key test-key
  chat
  --session-key terminal-bench:representative-task
  "write the file"
)
[[ "${args[*]}" == "${expected[*]}" ]]

unset HARBOR_TASK_ID
export MOLTIS_SESSION_KEY="terminal-bench:stdin-task"
printf 'instruction from stdin\n' | "$script_dir/run_moltis_agent.sh"
mapfile -t args <"$CAPTURE_FILE"
[[ "${args[6]}" == "terminal-bench:stdin-task" ]]
[[ "${args[7]}" == "instruction from stdin" ]]

if MOLTIS_API_KEY='' "$script_dir/run_moltis_agent.sh" test 2>/dev/null; then
  echo "expected missing API key to fail" >&2
  exit 1
fi

echo "terminal-bench wrapper tests passed"
