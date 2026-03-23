# E2E Tests for OAuth Provider Connection

## Goal

Add end-to-end tests that exercise the OAuth provider connection flow in the
gateway UI. Two complementary approaches: a mock OAuth server for CI (no secrets
needed), and optional real-token tests for verifying refresh flows against live
providers.

---

## Approach 1 — Mock OAuth Server (CI-safe, no secrets)

### What it tests

The full browser-side OAuth flow: user clicks "Connect" → `start_oauth` RPC →
redirect to auth URL → callback with code → `complete_oauth` RPC → token stored
→ provider shows as connected. This proves the gateway's own code paths work
without depending on external providers.

### Implementation steps

#### 1. Create `e2e/mock-oauth-server.js`

A minimal Node HTTP server that implements two endpoints:

- **`GET /authorize`** — validates `client_id`, `redirect_uri`, `code_challenge`,
  `state` query params. Immediately redirects back to `redirect_uri` with a
  predictable `code=mock-auth-code&state=<state>`.
- **`POST /token`** — validates `grant_type`, `code`, `code_verifier`. Returns
  a JSON token response:
  ```json
  {
    "access_token": "mock-access-token",
    "refresh_token": "mock-refresh-token",
    "token_type": "Bearer",
    "expires_in": 3600
  }
  ```
  For `grant_type=refresh_token`, return a new `access_token` with a different
  value to prove refresh worked.

The server should:
- Listen on port 0 (OS-assigned) and write the port to a temp file or stdout
- Validate PKCE: verify that `SHA256(code_verifier) == code_challenge` from the
  authorize step (store challenge in memory keyed by state)
- Return 400 with descriptive errors for invalid requests (helps debug test
  failures)
- Support a `GET /calls` endpoint that returns request history for assertions

#### 2. Create `e2e/start-gateway-oauth.sh`

New startup script (similar to existing `start-gateway.sh`) that:

- Starts the mock OAuth server first, captures its port
- Sets env vars to override the built-in OAuth config:
  ```bash
  export MOLTIS_OAUTH_OPENAI_CODEX_AUTH_URL="http://127.0.0.1:${MOCK_PORT}/authorize"
  export MOLTIS_OAUTH_OPENAI_CODEX_TOKEN_URL="http://127.0.0.1:${MOCK_PORT}/token"
  export MOLTIS_OAUTH_OPENAI_CODEX_CLIENT_ID="test-client-id"
  export MOLTIS_OAUTH_OPENAI_CODEX_REDIRECT_URI="http://localhost:1455/auth/callback"
  ```
- Starts the moltis gateway as usual (isolated data dir, no TLS)
- Seeds identity/user files like the default script

#### 3. Add Playwright project in `playwright.config.js`

```js
{
  name: "oauth",
  testMatch: /oauth\.spec/,
  use: { baseURL: `http://127.0.0.1:${process.env.MOLTIS_E2E_OAUTH_PORT || 4010}` },
}
```

With a corresponding `webServer` entry that runs `start-gateway-oauth.sh`.

#### 4. Create `e2e/specs/oauth.spec.js`

Test cases:

1. **Provider list shows OAuth providers as disconnected**
   - Navigate to providers page
   - Assert OpenAI Codex shows "Connect" button (not "Connected")

2. **OAuth PKCE flow completes successfully**
   - Click "Connect" on OpenAI Codex
   - Playwright follows the redirect to mock server → callback → back to gateway
   - Assert provider now shows as "Connected"
   - Assert mock server received valid PKCE challenge/verifier pair

3. **OAuth state mismatch is rejected**
   - Manually navigate to `/auth/callback?code=x&state=wrong-state`
   - Assert error is shown, provider stays disconnected

4. **Token refresh works**
   - Pre-seed `oauth_tokens.json` with an expired mock token
   - Trigger an action that requires the token (e.g. list models)
   - Assert mock server received a refresh request
   - Assert new token is stored

5. **Disconnect removes tokens**
   - Start with a connected provider (pre-seed tokens)
   - Click "Disconnect" / delete
   - Assert `oauth_tokens.json` no longer has the provider entry
   - Assert UI shows "Connect" again

6. **Error handling — token exchange fails**
   - Configure mock server to return 400 on `/token`
   - Attempt OAuth flow
   - Assert user sees an error message, provider stays disconnected

#### 5. Update CI workflow

Add the `oauth` project to the existing `e2e` job in `ci.yml`. No secrets
needed — everything runs against the mock server.

```yaml
- name: Run E2E tests (including OAuth)
  run: npx playwright test
  env:
    CI: true
```

### Files to create/modify

| File | Action |
|------|--------|
| `crates/gateway/ui/e2e/mock-oauth-server.js` | Create |
| `crates/gateway/ui/e2e/start-gateway-oauth.sh` | Create |
| `crates/gateway/ui/e2e/specs/oauth.spec.js` | Create |
| `crates/gateway/ui/playwright.config.js` | Add `oauth` project + webServer |
| `.github/workflows/ci.yml` | No change needed (runs all Playwright projects) |

---

## Approach 2 — Real Provider Tokens (optional, for integration confidence)

### What it tests

That token refresh actually works against the real provider APIs, and that
stored OAuth tokens can be used to list models / make API calls. This is a
smoke test, not a full flow (you can't automate the provider's login page).

### GitHub Actions setup

#### Secrets to add

Create a GitHub Environment called `e2e-oauth` with **required reviewers**
(prevents fork PRs from accessing secrets):

| Secret name | Description |
|-------------|-------------|
| `MOLTIS_OAUTH_OPENAI_CODEX_ACCESS_TOKEN` | Valid access token from a test OpenAI account |
| `MOLTIS_OAUTH_OPENAI_CODEX_REFRESH_TOKEN` | Refresh token for the same account |
| `MOLTIS_OAUTH_GITHUB_COPILOT_ACCESS_TOKEN` | Valid Copilot token (optional) |
| `MOLTIS_OAUTH_GITHUB_COPILOT_REFRESH_TOKEN` | Copilot refresh token (optional) |

#### Obtaining test tokens

1. Run moltis locally: `cargo run -- serve`
2. Connect the provider via the UI (triggers real OAuth flow)
3. Copy tokens from `~/.config/moltis/oauth_tokens.json`
4. Store in GitHub Secrets

Tokens will need periodic rotation when the refresh token expires (provider
dependent — OpenAI refresh tokens are long-lived).

#### CI workflow addition

Add a **separate job** that only runs on `main` (never on PRs from forks):

```yaml
e2e-oauth-integration:
  runs-on: ubuntu-latest
  if: github.ref == 'refs/heads/main'
  environment: e2e-oauth  # requires reviewer approval
  needs: [e2e]
  steps:
    - uses: actions/checkout@v4
    - name: Build
      run: cargo build --bin moltis
    - name: Setup Node
      uses: actions/setup-node@v4
      with: { node-version: 22 }
    - name: Install deps
      run: npm ci
      working-directory: crates/gateway/ui
    - name: Install Playwright
      run: npx playwright install --with-deps chromium
      working-directory: crates/gateway/ui
    - name: Seed OAuth tokens
      run: |
        mkdir -p target/e2e-runtime-oauth/.config/moltis
        cat > target/e2e-runtime-oauth/.config/moltis/oauth_tokens.json << 'SEED'
        {
          "openai-codex": {
            "access_token": "${{ secrets.MOLTIS_OAUTH_OPENAI_CODEX_ACCESS_TOKEN }}",
            "refresh_token": "${{ secrets.MOLTIS_OAUTH_OPENAI_CODEX_REFRESH_TOKEN }}",
            "expires_at": 0
          }
        }
        SEED
    - name: Run OAuth integration tests
      run: npx playwright test --project=oauth-integration
      working-directory: crates/gateway/ui
      env:
        CI: true
```

### Test cases (separate spec file)

`e2e/specs/oauth-integration.spec.js`:

1. **Token refresh against real provider** — start with expired `access_token`
   + valid `refresh_token`, trigger a list-models call, assert it succeeds
   (proves refresh flow works end-to-end).

2. **Provider shows as connected** — navigate to providers page, assert the
   pre-seeded provider shows "Connected" status.

3. **List models returns results** — call `list_models` RPC for the connected
   provider, assert non-empty model list returned.

### Security guardrails

- **GitHub Environment with reviewers**: secrets only available in approved runs
- **`if: github.ref == 'refs/heads/main'`**: never runs on fork PRs
- **Dedicated test account**: don't use personal credentials; create a test org
  or separate account for CI
- **Scrub Playwright reports**: ensure HTML report doesn't capture env vars in
  screenshots or console output. Add `filterEnv` or avoid `console.log` of
  tokens in test code
- **Token rotation**: document how to refresh the stored tokens (manual process,
  quarterly cadence suggested)

### Files to create/modify

| File | Action |
|------|--------|
| `crates/gateway/ui/e2e/specs/oauth-integration.spec.js` | Create |
| `crates/gateway/ui/e2e/start-gateway-oauth-integration.sh` | Create |
| `crates/gateway/ui/playwright.config.js` | Add `oauth-integration` project |
| `.github/workflows/ci.yml` | Add `e2e-oauth-integration` job |

---

## Order of work

1. **Approach 1 first** — mock server + PKCE flow tests. This is self-contained,
   requires no secrets, and covers the majority of the value.
2. **Approach 2 later** — only after approach 1 is stable and you want confidence
   in real provider refresh flows. Can be skipped entirely if mock tests are
   sufficient.

## Open questions

- Should the mock server also support device flow (for GitHub Copilot / Kimi)?
  Device flow is harder to E2E test since it requires polling. Could add a
  `POST /device/code` + `POST /token` (device grant) to the mock.
- Should token refresh be tested in approach 1 by pre-seeding expired tokens
  and having the mock server handle refresh, or is that better left to unit
  tests in `crates/oauth/`?
