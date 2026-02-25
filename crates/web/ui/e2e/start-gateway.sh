#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/../../../.." && pwd)"

PORT="${MOLTIS_E2E_PORT:-0}"
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

# Mark onboarding as complete so the app skips the wizard.
touch "${DATA_DIR}/.onboarded"

cd "${REPO_ROOT}"

export MOLTIS_CONFIG_DIR="${CONFIG_DIR}"
export MOLTIS_DATA_DIR="${DATA_DIR}"
export MOLTIS_SERVER__PORT="${PORT}"

binary_is_stale() {
	local binary="$1"
	if [ ! -f "${binary}" ]; then
		return 0
	fi
	if [ "${REPO_ROOT}/Cargo.toml" -nt "${binary}" ]; then
		return 0
	fi
	if [ -f "${REPO_ROOT}/Cargo.lock" ] && [ "${REPO_ROOT}/Cargo.lock" -nt "${binary}" ]; then
		return 0
	fi
	find "${REPO_ROOT}/crates" \
		-type f \
		\( -name "*.rs" -o -name "*.toml" -o -name "*.html" -o -name "*.js" -o -name "*.css" \) \
		-newer "${binary}" \
		-print -quit | grep -q .
}

# Prefer a pre-built binary to avoid recompiling every test run.
BINARY="${MOLTIS_BINARY:-}"
if [ -z "${BINARY}" ]; then
	# Pick the newest local build so tests don't accidentally run stale binaries.
	for candidate in target/debug/moltis target/release/moltis; do
		if [ -x "${candidate}" ] && { [ -z "${BINARY}" ] || [ "${candidate}" -nt "${BINARY}" ]; }; then
			BINARY="${candidate}"
		fi
	done
fi

if [ -n "${BINARY}" ] && binary_is_stale "${BINARY}"; then
	echo "Detected source changes newer than ${BINARY}; using cargo run for a fresh build." >&2
	BINARY=""
fi

if [ -n "${BINARY}" ]; then
	exec "${BINARY}" --no-tls --bind 127.0.0.1 --port "${PORT}"
else
	exec cargo run --bin moltis -- --no-tls --bind 127.0.0.1 --port "${PORT}"
fi
