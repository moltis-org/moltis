# Gateway UI E2E Tests

These tests use Playwright against a real `moltis` server process.

## Why this setup

- Exercises real browser behavior (routing, DOM updates, WebSocket lifecycle).
- Runs against real gateway APIs, not mocked frontend responses.
- Uses isolated config/data dirs per run to avoid local machine state leakage.

## Quickstart

From repo root:

```bash
just ui-e2e-install
just ui-e2e
```

Directly from `crates/web/ui`:

```bash
npm install
npm run e2e:install
npm run e2e
```

## Test Runtime

### Default server (`start-gateway.sh`)

1. Creates `target/e2e-runtime/{config,data}`.
2. Seeds `IDENTITY.md` and `USER.md` so onboarding is completed.
3. Sets `MOLTIS_CONFIG_DIR`, `MOLTIS_DATA_DIR`, and `MOLTIS_SERVER__PORT`.
4. Checks for a pre-built binary (`target/debug/moltis` or `target/release/moltis`)
   before falling back to `cargo run`. Set `MOLTIS_BINARY` to override.

### Onboarding server (`start-gateway-onboarding.sh`)

Same as above but does **not** seed `IDENTITY.md` or `USER.md`, so the
app enters onboarding mode. Uses a random free port by default.

## Playwright Projects

The test suite is split into seven Playwright projects, plus one opt-in live
project for local Ollama/Qwen validation:

| Project | Port | Spec files | Notes |
|---------|------|------------|-------|
| `default` | Random free port (`MOLTIS_E2E_PORT`) | All except `auth.spec.js` and `onboarding.spec.js` | Seeded identity, no password |
| `auth` | Same as `default` | `auth.spec.js` | Runs after `default`; sets a password to test login |
| `onboarding` | Random free port (`MOLTIS_E2E_ONBOARDING_PORT`) | `onboarding.spec.js` | Separate server without seeded identity |
| `onboarding-auth` | Random free port (`MOLTIS_E2E_ONBOARDING_AUTH_PORT`) | `onboarding-auth.spec.js` | Separate server with remote-auth simulation |
| `onboarding-anthropic` | Random free port (`MOLTIS_E2E_ONBOARDING_ANTHROPIC_PORT`) | `onboarding-anthropic.spec.js` | Separate server proving first-run Anthropic onboarding with zero providers at startup |
| `openai-live` | Random free port (`MOLTIS_E2E_OPENAI_LIVE_PORT`) | `openai-live.spec.js` | Separate server that preserves only the existing OpenAI env and proves a real OpenAI chat turn works |
| `ollama-qwen-live` | Random free port (`MOLTIS_E2E_OLLAMA_QWEN_LIVE_PORT`) + Ollama API port (`MOLTIS_E2E_OLLAMA_QWEN_API_PORT`, default `11435`) | `ollama-qwen-live.spec.js` | Opt-in server that starts a local Ollama instance, seeds a custom OpenAI-compatible Qwen provider, and proves the multiple-system-message regression is fixed |

## Spec Files

| File | Tests | Description |
|------|-------|-------------|
| `smoke.spec.js` | 6 | App shell loads, route navigation renders without errors |
| `websocket.spec.js` | 4 | WS connection, reconnection, tick events, RPC health |
| `sessions.spec.js` | 6 | Session list, create, switch, search, clear, panel visibility |
| `chat-input.spec.js` | 7 | Chat input focus, slash commands, Shift+Enter, model selector |
| `settings-nav.spec.js` | 17 | Settings subsection routing and rendering |
| `theme.spec.js` | 3 | Theme toggle, dark mode, localStorage persistence |
| `providers.spec.js` | 5 | Provider page load, add/detect buttons, guidance |
| `cron.spec.js` | 4 | Cron jobs page, heartbeat tab, create button |
| `skills.spec.js` | 4 | Skills page, install input, featured repos |
| `projects.spec.js` | 4 | Projects page, add input, auto-detect |
| `mcp.spec.js` | 3 | MCP tools page, featured servers |
| `monitoring.spec.js` | 3 | Monitoring dashboard, time range selector |
| `auth.spec.js` | 6 | Password setup, login, wrong password, Bearer auth |
| `onboarding.spec.js` | 5 | Onboarding redirect, steps, skip, identity input |
| `onboarding-auth.spec.js` | 1 | Remote onboarding auth flow with setup code and identity save |
| `onboarding-anthropic.spec.js` | 1 | Anthropic onboarding from empty startup, model discovery, model selection |
| `openai-live.spec.js` | 1 | Live OpenAI provider smoke test using the existing env and a real chat turn |
| `ollama-qwen-live.spec.js` | 1 | Opt-in live Ollama smoke test for the custom OpenAI-compatible Qwen regression path |

## Shared Helpers

`e2e/helpers.js` exports reusable utilities:

- `expectPageContentMounted(page)` — waits for `#pageContent` to have children
- `watchPageErrors(page)` — collects uncaught page errors
- `waitForWsConnected(page)` — waits for `#statusDot.connected`
- `navigateAndWait(page, path)` — goto + content mounted
- `createSession(page)` — clicks new session button, waits for navigation

## Running Specific Tests

```bash
# Run a single spec file
cd crates/web/ui && npx playwright test e2e/specs/sessions.spec.js

# Run a specific project
npx playwright test --project=auth

# Run the opt-in Ollama/Qwen live project
MOLTIS_E2E_OLLAMA_QWEN_LIVE=1 npx playwright test --project=ollama-qwen-live e2e/specs/ollama-qwen-live.spec.js

# Run with visible browser
just ui-e2e-headed

# Debug mode (step through)
npm run e2e:debug

# View HTML report
npx playwright show-report
```

## Tips

- **Build the binary first** (`cargo build`) to avoid recompilation on every
  test run. The startup script auto-detects `target/debug/moltis`.
- Set `MOLTIS_BINARY=/path/to/moltis` to use a specific binary.
- Tests run serially (`workers: 1`) because they share a single server.
- On failure, traces, screenshots, and videos are saved in `test-results/`.
