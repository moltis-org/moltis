#!/usr/bin/env bash
set -euo pipefail

# Same as start-gateway-onboarding.sh but with an isolated runtime
# dedicated to Anthropic onboarding E2E.

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/../../../.." && pwd)"

PORT="${MOLTIS_E2E_ONBOARDING_ANTHROPIC_PORT:-0}"
RUNTIME_ROOT="${MOLTIS_E2E_ONBOARDING_ANTHROPIC_RUNTIME_DIR:-${REPO_ROOT}/target/e2e-runtime-onboarding-anthropic}"
CONFIG_DIR="${RUNTIME_ROOT}/config"
DATA_DIR="${RUNTIME_ROOT}/data"
HOME_DIR="${RUNTIME_ROOT}/home"
ORIGINAL_HOME="${HOME:-}"

rm -rf "${RUNTIME_ROOT}"
mkdir -p "${CONFIG_DIR}" "${DATA_DIR}" "${HOME_DIR}"

# Deliberately NOT creating IDENTITY.md or USER.md so onboarding triggers.

cd "${REPO_ROOT}"

export MOLTIS_CONFIG_DIR="${CONFIG_DIR}"
export MOLTIS_DATA_DIR="${DATA_DIR}"
export MOLTIS_SERVER__PORT="${PORT}"
# Keep rustup/cargo toolchains available after HOME isolation so
# stale-binary fallback can still rebuild when needed.
if [ -z "${RUSTUP_HOME:-}" ] && [ -n "${ORIGINAL_HOME}" ]; then
	export RUSTUP_HOME="${ORIGINAL_HOME}/.rustup"
fi
if [ -z "${CARGO_HOME:-}" ] && [ -n "${ORIGINAL_HOME}" ]; then
	export CARGO_HOME="${ORIGINAL_HOME}/.cargo"
fi
# Isolate HOME so auto-detection cannot read user-global OAuth/key stores.
export HOME="${HOME_DIR}"

# Preserve key for the Playwright runner while preventing gateway startup
# provider auto-detection from env vars.
export MOLTIS_E2E_ANTHROPIC_API_KEY="${MOLTIS_E2E_ANTHROPIC_API_KEY:-${ANTHROPIC_API_KEY:-}}"
unset ANTHROPIC_API_KEY
unset OPENAI_API_KEY
unset GEMINI_API_KEY
unset GROQ_API_KEY
unset XAI_API_KEY
unset DEEPSEEK_API_KEY
unset MISTRAL_API_KEY
unset OPENROUTER_API_KEY
unset CEREBRAS_API_KEY
unset MINIMAX_API_KEY
unset MOONSHOT_API_KEY
unset VENICE_API_KEY
unset OLLAMA_API_KEY
unset KIMI_API_KEY

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
	exec cargo +nightly-2025-11-30 run --bin moltis -- --no-tls --bind 127.0.0.1 --port "${PORT}"
fi
