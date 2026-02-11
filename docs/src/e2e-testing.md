# End-to-End Testing

This project uses Playwright to run browser-level tests against a real `moltis` gateway process.

The goal is simple: catch web UI regressions before they ship.

## Why This Approach

- Tests run in a real browser (Chromium), not a DOM mock.
- Tests hit real gateway routes and WebSocket behavior.
- Runtime state is isolated so local machine config does not leak into test outcomes.

## Current Setup

The e2e harness lives in `crates/gateway/ui`:

- `playwright.config.js` configures Playwright and web server startup.
- `e2e/start-gateway.sh` boots the gateway in deterministic test mode.
- `e2e/specs/smoke.spec.js` contains smoke coverage for critical routes.

## How Startup Works

`e2e/start-gateway.sh`:

1. Creates isolated runtime directories under `target/e2e-runtime`.
2. Seeds `IDENTITY.md` and `USER.md` so onboarding does not block tests.
3. Exports `MOLTIS_CONFIG_DIR`, `MOLTIS_DATA_DIR`, and test port env.
4. Starts the gateway with:

```bash
cargo run --bin moltis -- --no-tls --bind 127.0.0.1 --port <PORT>
```

`--no-tls` is intentional here so Playwright can probe `http://.../health` during readiness checks.

## Running E2E

From repo root (recommended):

```bash
just ui-e2e-install
just ui-e2e
```

Headed mode:

```bash
just ui-e2e-headed
```

Directly from `crates/gateway/ui`:

```bash
npm install
npm run e2e:install
npm run e2e
```

## Test Artifacts

On failures, Playwright stores artifacts in:

- `crates/gateway/ui/test-results/` (screenshots, video, traces)
- `crates/gateway/ui/playwright-report/` (HTML report)

Open a trace with:

```bash
cd crates/gateway/ui
npx playwright show-trace test-results/<test-dir>/trace.zip
```

## Writing Stable Tests

- Prefer stable IDs/selectors over broad text matching.
- Assert route + core UI state, avoid over-asserting cosmetic details.
- Keep smoke tests fast and deterministic.
- Add focused scenario tests for high-risk features (chat send flow, settings persistence, skills, projects, crons).

## CI Integration

The `just ui-e2e` target is the intended command for CI.

Pull requests use the local-validation flow: the E2E workflow waits for a
`local/e2e` commit status, published by `./scripts/local-validate.sh`.

Pushes to `main`, tags, and manual dispatch still run the hosted E2E job.
