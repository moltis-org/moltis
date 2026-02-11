# Authentication

Moltis uses a unified authentication gate that protects all routes with a
single source of truth. This page explains how authentication works, the
decision logic, and the different credential types.

## Architecture

All HTTP requests pass through a single `auth_gate` middleware before
reaching any handler. The middleware calls `check_auth()` — the **only**
function in the codebase that decides whether a request is authenticated.
This eliminates the class of bugs where different code paths disagree on
auth status.

```
Request
  │
  ▼
auth_gate middleware
  │
  ├─ Public path? (/health, /assets/*, /api/auth/*, ...) → pass through
  │
  ├─ No credential store? → pass through
  │
  └─ check_auth()
       │
       ├─ Allowed    → insert AuthIdentity into request, continue
       ├─ SetupRequired → 401 (API/WS) or redirect to /onboarding (pages)
       └─ Unauthorized  → 401 (API/WS) or serve SPA login page (pages)
```

WebSocket connections also use `check_auth()` for the HTTP upgrade
handshake. After the upgrade, the WS protocol has its own param-based auth
(API key or password in the `connect` message) for clients that cannot set
HTTP headers.

## Decision Matrix

`check_auth()` evaluates conditions in order and returns the first match:

| # | Condition | Result | Auth method |
|---|-----------|--------|-------------|
| 1 | `auth_disabled` is true | **Allowed** | Loopback |
| 2 | Setup not complete + local connection | **Allowed** | Loopback |
| 3 | Setup not complete + remote connection | **SetupRequired** | — |
| 4 | Valid session cookie | **Allowed** | Password |
| 5 | Valid Bearer API key | **Allowed** | ApiKey |
| 6 | None of the above | **Unauthorized** | — |

```admonish info title="What is 'setup complete'?"
Setup is complete when at least one credential (password or passkey) has
been registered. The `setup_complete` flag is recomputed whenever
credentials are added or removed, so it correctly reflects passkey-only
setups — not just password presence.
```

## Three-Tier Model

The decision matrix above implements a three-tier authentication model:

| Tier | Condition | Behaviour |
|------|-----------|-----------|
| **1 — Full auth** | Password or passkey is configured | Auth **always** required (any IP) |
| **2 — Local dev** | No credentials + direct local connection | Full access (dev convenience) |
| **3 — Remote setup** | No credentials + remote/proxied connection | Setup flow only |

### Practical scenarios

| Scenario | No credentials | Credentials configured |
|----------|---------------|----------------------|
| Local browser on `localhost:18789` | Full access (Tier 2) | Login required (Tier 1) |
| Local CLI/wscat on `localhost:18789` | Full access (Tier 2) | Login required (Tier 1) |
| Internet via reverse proxy | Onboarding only (Tier 3) | Login required (Tier 1) |
| `MOLTIS_BEHIND_PROXY=true`, any source | Onboarding only (Tier 3) | Login required (Tier 1) |

### How "local" is determined

A connection is classified as **local** only when **all four** checks pass:

1. `MOLTIS_BEHIND_PROXY` env var is **not** set
2. No proxy headers present (`X-Forwarded-For`, `X-Real-IP`,
   `CF-Connecting-IP`, `Forwarded`)
3. The `Host` header resolves to a loopback address (or is absent)
4. The TCP source IP is loopback (`127.0.0.1`, `::1`)

If **any** check fails, the connection is treated as remote.

## Credential Types

### Password

- Set during initial setup or added later via Settings
- Hashed with Argon2id before storage
- Minimum 8 characters
- Verified against `auth_password` table

### Passkey (WebAuthn)

- Registered during setup or added later via Settings
- Supports hardware keys (YubiKey), platform authenticators (Touch ID,
  Windows Hello), and cross-platform authenticators
- Stored in `passkeys` table as serialized WebAuthn credential data
- Multiple passkeys can be registered per instance

### Session cookie

- HTTP-only `moltis_session` cookie, `SameSite=Strict`
- Created on successful login (password or passkey)
- 30-day expiry
- Validated against `auth_sessions` table
- When the request arrives on a `.localhost` subdomain (e.g.
  `moltis.localhost`), the cookie includes `Domain=localhost` so it is
  shared across all loopback hostnames

### API key

- Created in Settings > Security > API Keys
- Prefixed with `mk_` for identification
- Stored as SHA-256 hash (the raw key is shown once at creation)
- Passed via `Authorization: Bearer <key>` header (HTTP) or in the
  `connect` handshake `auth.api_key` field (WebSocket)
- **Must have at least one scope** — keys without scopes are denied

## API Key Scopes

| Scope | Permissions |
|-------|-------------|
| `operator.read` | View status, list jobs, read history |
| `operator.write` | Send messages, create jobs, modify configuration |
| `operator.admin` | All permissions (superset of all scopes) |
| `operator.approvals` | Handle command approval requests |
| `operator.pairing` | Manage device/node pairing |

```admonish tip
Use the minimum necessary scopes. A monitoring integration only needs
`operator.read`. A CI pipeline that triggers agent runs needs
`operator.read` and `operator.write`.
```

## Public Paths

These paths are accessible without authentication, even when credentials
are configured:

| Path | Purpose |
|------|---------|
| `/health` | Health check endpoint |
| `/api/auth/*` | Auth status, login, setup, passkey flows |
| `/assets/*` | Static assets (JS, CSS, images) |
| `/auth/callback` | OAuth callback |
| `/manifest.json` | PWA manifest |
| `/sw.js` | Service worker |

## Request Throttling

Moltis applies built-in endpoint throttling per client IP only when auth is
required for the current request.

Requests bypass IP throttling when:

- The request is already authenticated (session or API key)
- Auth is not currently enforced (`auth_disabled = true`)
- Setup is incomplete and the request is allowed by local Tier-2 access

Default limits:

| Scope | Default |
|------|---------|
| `POST /api/auth/login` | 5 requests per 60 seconds |
| Other `/api/auth/*` | 120 requests per 60 seconds |
| Other `/api/*` | 180 requests per 60 seconds |
| `/ws` upgrade | 30 requests per 60 seconds |

When a limit is exceeded:

- API endpoints return `429 Too Many Requests`
- Responses include `Retry-After` header
- JSON API responses also include `retry_after_seconds`

```admonish note
When `MOLTIS_BEHIND_PROXY=true`, throttling is keyed by forwarded client IP
headers (`X-Forwarded-For`, `X-Real-IP`, `CF-Connecting-IP`) instead of the
direct socket address.
```

## Setup Flow

On first run (no credentials configured):

1. A random 6-digit **setup code** is printed to the terminal
2. Local connections get full access (Tier 2) — no setup code needed
3. Remote connections are redirected to `/onboarding` (Tier 3) — the
   setup code is required to set a password or register a passkey
4. After setting up, the setup code is cleared and a session is created

```admonish warning
The setup code is single-use and only valid until the first credential is
registered. If you lose it, restart the server to generate a new one.
```

## Removing Authentication

The "Remove all auth" action in Settings:

1. Deletes all passwords, passkeys, sessions, and API keys
2. Sets `auth_disabled = true` in config
3. Generates a new setup code for re-setup
4. All subsequent requests are allowed through (Tier 1 check: `auth_disabled`)

To re-enable auth, complete the setup flow again with the new setup code.

## WebSocket Authentication

WebSocket connections are authenticated at two levels:

### 1. HTTP upgrade (header auth)

The WebSocket upgrade request passes through `check_auth()` like any
other HTTP request. If the browser has a valid session cookie, the
connection is pre-authenticated.

### 2. Connect message (param auth)

After the WebSocket is established, the client sends a `connect` message.
Non-browser clients (CLI tools, scripts) that cannot set HTTP headers
authenticate here:

```json
{
  "method": "connect",
  "params": {
    "client": { "id": "my-tool", "version": "1.0.0" },
    "auth": {
      "api_key": "mk_abc123..."
    }
  }
}
```

The `auth` object can contain `api_key` or `password`. If neither is
provided and the connection was not pre-authenticated via headers, the
connection is rejected.

## Reverse Proxy Considerations

When running behind a reverse proxy, authentication interacts with the
local-connection detection:

- **Most proxies** add `X-Forwarded-For` or similar headers, which
  automatically classify connections as remote
- **Bare proxies** (no forwarding headers) can appear local — set
  `MOLTIS_BEHIND_PROXY=true` to force all connections to be treated as
  remote
- The proxy must forward `Origin` and `Host` headers for WebSocket
  CSWSH protection
- TLS termination typically happens at the proxy

See [Security Architecture](security.md#reverse-proxy-deployments) for
detailed proxy deployment guidance.

## Session Management

| Operation | Endpoint | Auth required |
|-----------|----------|---------------|
| Check status | `GET /api/auth/status` | No |
| Set password (setup) | `POST /api/auth/setup` | Setup code |
| Login with password | `POST /api/auth/login` | No (validates password) |
| Login with passkey | `POST /api/auth/passkey/auth/*` | No (validates passkey) |
| Logout | `POST /api/auth/logout` | Session |
| Change password | `POST /api/auth/password/change` | Session |
| List API keys | `GET /api/auth/api-keys` | Session |
| Create API key | `POST /api/auth/api-keys` | Session |
| Revoke API key | `DELETE /api/auth/api-keys/{id}` | Session |
| Register passkey | `POST /api/auth/passkey/register/*` | Session |
| Remove passkey | `DELETE /api/auth/passkeys/{id}` | Session |
| Remove all auth | `POST /api/auth/reset` | Session |
