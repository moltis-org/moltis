#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/../../../.." && pwd)"

PORT="${MOLTIS_E2E_OLLAMA_QWEN_LIVE_PORT:-0}"
OLLAMA_API_PORT="${MOLTIS_E2E_OLLAMA_QWEN_API_PORT:-11435}"
MODEL="${MOLTIS_E2E_OLLAMA_QWEN_MODEL:-qwen2.5:0.5b}"
RUNTIME_ROOT="${MOLTIS_E2E_OLLAMA_QWEN_LIVE_RUNTIME_DIR:-${REPO_ROOT}/target/e2e-runtime-ollama-qwen-live}"
CONFIG_DIR="${RUNTIME_ROOT}/config"
DATA_DIR="${RUNTIME_ROOT}/data"
HOME_DIR="${RUNTIME_ROOT}/home"
OLLAMA_ROOT="${MOLTIS_E2E_OLLAMA_QWEN_OLLAMA_ROOT:-${REPO_ROOT}/target/e2e-ollama-qwen-live}"
OLLAMA_MODELS_DIR="${MOLTIS_E2E_OLLAMA_QWEN_MODELS_DIR:-${OLLAMA_ROOT}/models}"
OLLAMA_LOG="${OLLAMA_ROOT}/ollama.log"
ORIGINAL_HOME="${HOME:-}"
OLLAMA_BIND="127.0.0.1:${OLLAMA_API_PORT}"
OLLAMA_PID=""
GATEWAY_PID=""

if ! command -v ollama >/dev/null 2>&1; then
	echo "Missing ollama CLI. Install it or set MOLTIS_E2E_OLLAMA_QWEN_LIVE=0." >&2
	exit 1
fi

cleanup() {
	local pid=""
	for pid in "${GATEWAY_PID}" "${OLLAMA_PID}"; do
		if [[ -n "${pid}" ]] && kill -0 "${pid}" 2>/dev/null; then
			kill -TERM "${pid}" 2>/dev/null || true
		fi
	done

	sleep 1

	for pid in "${GATEWAY_PID}" "${OLLAMA_PID}"; do
		if [[ -n "${pid}" ]] && kill -0 "${pid}" 2>/dev/null; then
			kill -KILL "${pid}" 2>/dev/null || true
		fi
	done
}

trap cleanup EXIT INT TERM

rm -rf "${RUNTIME_ROOT}"
mkdir -p "${CONFIG_DIR}" "${DATA_DIR}" "${HOME_DIR}" "${OLLAMA_MODELS_DIR}"

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

touch "${DATA_DIR}/.onboarded"

cat > "${CONFIG_DIR}/moltis.toml" <<EOF
[providers]
offered = ["custom-ollama-qwen"]

[providers.custom-ollama-qwen]
enabled = true
api_key = "ollama"
base_url = "http://127.0.0.1:${OLLAMA_API_PORT}/v1"
models = ["${MODEL}"]
fetch_models = false
EOF

cd "${REPO_ROOT}"

export MOLTIS_CONFIG_DIR="${CONFIG_DIR}"
export MOLTIS_DATA_DIR="${DATA_DIR}"
export MOLTIS_SERVER__PORT="${PORT}"
if [[ -z "${RUSTUP_HOME:-}" ]] && [[ -n "${ORIGINAL_HOME}" ]]; then
	export RUSTUP_HOME="${ORIGINAL_HOME}/.rustup"
fi
if [[ -z "${CARGO_HOME:-}" ]] && [[ -n "${ORIGINAL_HOME}" ]]; then
	export CARGO_HOME="${ORIGINAL_HOME}/.cargo"
fi
export HOME="${HOME_DIR}"

unset OPENAI_API_KEY
unset ANTHROPIC_API_KEY
unset GEMINI_API_KEY
unset GROQ_API_KEY
unset XAI_API_KEY
unset DEEPSEEK_API_KEY
unset FIREWORKS_API_KEY
unset MISTRAL_API_KEY
unset OPENROUTER_API_KEY
unset CEREBRAS_API_KEY
unset MINIMAX_API_KEY
unset MOONSHOT_API_KEY
unset Z_API_KEY
unset Z_CODE_API_KEY
unset VENICE_API_KEY
unset OLLAMA_API_KEY
unset LMSTUDIO_API_KEY
unset KIMI_API_KEY

binary_is_stale() {
	local binary="$1"
	if [[ ! -f "${binary}" ]]; then
		return 0
	fi
	if [[ "${REPO_ROOT}/Cargo.toml" -nt "${binary}" ]]; then
		return 0
	fi
	if [[ -f "${REPO_ROOT}/Cargo.lock" ]] && [[ "${REPO_ROOT}/Cargo.lock" -nt "${binary}" ]]; then
		return 0
	fi
	find "${REPO_ROOT}/crates" \
		-type f \
		\( -name "*.rs" -o -name "*.toml" \) \
		-newer "${binary}" \
		-print -quit | grep -q .
}

BINARY="${MOLTIS_BINARY:-}"
if [[ -z "${BINARY}" ]]; then
	for candidate in target/debug/moltis target/release/moltis; do
		if [[ -x "${candidate}" ]] && { [[ -z "${BINARY}" ]] || [[ "${candidate}" -nt "${BINARY}" ]]; }; then
			BINARY="${candidate}"
		fi
	done
fi

if [[ -n "${BINARY}" ]] && binary_is_stale "${BINARY}"; then
	echo "Detected source changes newer than ${BINARY}; using cargo run for a fresh build." >&2
	BINARY=""
fi

OLLAMA_HOST="${OLLAMA_BIND}" OLLAMA_MODELS="${OLLAMA_MODELS_DIR}" ollama serve >"${OLLAMA_LOG}" 2>&1 &
OLLAMA_PID="$!"

for attempt in $(seq 1 60); do
	if curl -fsS "http://${OLLAMA_BIND}/api/tags" >/dev/null 2>&1; then
		break
	fi
	if ! kill -0 "${OLLAMA_PID}" 2>/dev/null; then
		echo "ollama serve exited unexpectedly. Recent log output:" >&2
		tail -n 50 "${OLLAMA_LOG}" >&2 || true
		exit 1
	fi
	sleep 1
	if [[ "${attempt}" == "60" ]]; then
		echo "Timed out waiting for Ollama to become ready on ${OLLAMA_BIND}." >&2
		tail -n 50 "${OLLAMA_LOG}" >&2 || true
		exit 1
	fi
done

OLLAMA_HOST="${OLLAMA_BIND}" OLLAMA_MODELS="${OLLAMA_MODELS_DIR}" ollama pull "${MODEL}"

if [[ -n "${BINARY}" ]]; then
	"${BINARY}" --no-tls --bind 127.0.0.1 --port "${PORT}" &
else
	cargo +nightly-2025-11-30 run --bin moltis -- --no-tls --bind 127.0.0.1 --port "${PORT}" &
fi
GATEWAY_PID="$!"
wait "${GATEWAY_PID}"
