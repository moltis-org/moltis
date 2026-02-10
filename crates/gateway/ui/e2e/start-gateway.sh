#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/../../../.." && pwd)"

PORT="${MOLTIS_E2E_PORT:-18789}"
RUNTIME_ROOT="${MOLTIS_E2E_RUNTIME_DIR:-${REPO_ROOT}/target/e2e-runtime}"
CONFIG_DIR="${RUNTIME_ROOT}/config"
DATA_DIR="${RUNTIME_ROOT}/data"

rm -rf "${RUNTIME_ROOT}"
mkdir -p "${CONFIG_DIR}" "${DATA_DIR}"

cat > "${DATA_DIR}/IDENTITY.md" <<'EOF'
---
name: e2e-bot
---

# IDENTITY.md

This file is managed by Moltis settings.
EOF

cat > "${DATA_DIR}/USER.md" <<'EOF'
---
name: e2e-user
---

# USER.md

This file is managed by Moltis settings.
EOF

cd "${REPO_ROOT}"

export MOLTIS_CONFIG_DIR="${CONFIG_DIR}"
export MOLTIS_DATA_DIR="${DATA_DIR}"
export MOLTIS_SERVER__PORT="${PORT}"

# Prefer a pre-built binary to avoid recompiling every test run.
BINARY="${MOLTIS_BINARY:-}"
if [ -z "${BINARY}" ]; then
	for candidate in target/release/moltis target/debug/moltis; do
		if [ -x "${candidate}" ]; then
			BINARY="${candidate}"
			break
		fi
	done
fi

if [ -n "${BINARY}" ]; then
	exec "${BINARY}" --no-tls --bind 127.0.0.1 --port "${PORT}"
else
	exec cargo run --bin moltis -- --no-tls --bind 127.0.0.1 --port "${PORT}"
fi
