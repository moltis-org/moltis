# Plan: xAI Grok Subscription OAuth for Moltis

Date: 2026-08-24
Branch: `feat/xai-oauth`
Status: implementation in progress (core wiring landed; awaiting Rust toolchain / live QA)
Inspiration: [stnly/pi-grok](https://github.com/stnly/pi-grok) (already installed in Pi as `git:github.com/stnly/pi-grok@v0.9.0`)
Closest in-tree template: `kimi-code` (device-flow OAuth + OpenAI-compat transport)

## Goal

Let SuperGrok / SuperGrok Heavy / X Premium+ subscribers use Grok in Moltis
**without** an `XAI_API_KEY`, via RFC 8628 device-code OAuth against `auth.x.ai`.

Keep the existing API-key provider `xai` untouched as the billed developer-API
fallback.

## Spike results (Quentin / Super Heavy)

Live probe with existing Pi `xai-oauth` credentials (JWT `tier=5`,
`hasGrokCodeAccess=true`):

| Surface | Result |
|---|---|
| Device/token issuer | `https://auth.x.ai` (token endpoint `.../oauth2/token`) |
| `GET https://api.x.ai/v1/models` | **200** |
| `POST https://api.x.ai/v1/chat/completions` | **200** |
| `POST https://api.x.ai/v1/responses` | **200** |
| `GET https://cli-chat-proxy.grok.com/v1/models` | **200** |
| `GET https://cli-chat-proxy.grok.com/v1/user` | **200** |
| `POST https://cli-chat-proxy.grok.com/v1/responses` | **200** (model remapped `grok-4.5` → `grok-4.5-build`) |

Conclusion for this account: **both** surfaces work. Still prefer the CLI chat
proxy for the subscription provider, matching `pi-grok` / Hermes / OpenCode:

- subscription quota / build variants ride the proxy
- proxy remaps models onto subscription variants
- account/billing/privacy endpoints live on the proxy
- lower risk of tier/402 gating seen on other accounts against `api.x.ai`

API-key `xai` remains on `https://api.x.ai/v1`.

## Product shape

| Field | Value |
|---|---|
| Provider id | `xai-oauth` |
| Display name | `xAI Grok OAuth (SuperGrok)` |
| Auth | OAuth device code (default). Optional later: PKCE callback like pi-grok's `PI_XAI_LOGIN_METHOD=callback` |
| Transport | OpenAI Responses API (same family as Codex / pi-grok `openai-responses`) |
| Base URL | `https://cli-chat-proxy.grok.com/v1` |
| Fallback provider | existing `xai` + `XAI_API_KEY` |
| Default models | `grok-4.5`, `grok-4.3`, `grok-build`, `grok-composer-2.5-fast`, `grok-4.20-*` (mirror pi-grok fallback list; enrich from proxy `/models`) |

### OAuth constants (from pi-grok / Hermes)

```
issuer:              https://auth.x.ai
device_code_url:     https://auth.x.ai/oauth2/device/code
token_url:           https://auth.x.ai/oauth2/token
client_id:           b1a00492-073a-47ea-816f-4c329264a828   # public Grok CLI client
grant_type:          urn:ietf:params:oauth:grant-type:device_code
scope:               openid profile email offline_access grok-cli:access api:access conversations:read conversations:write
```

Device-code request extras used by pi-grok:

- form: `referrer=grok-build`
- headers: `x-grok-client-version`, `x-grok-client-surface: cli`

### CLI proxy identity headers (required)

```
User-Agent: grok-shell/<ver> (<os>; <arch>)
x-grok-client-identifier: grok-shell
x-grok-client-version: 0.2.101   # overridable
x-grok-client-mode: interactive
X-XAI-Token-Auth: xai-grok-cli
x-authenticateresponse: authenticate-response
x-grok-model-override: <model id>   # on inference requests
x-grok-conv-id: <session id>        # optional multi-turn
```

## Why not just document XAI_API_KEY?

Subscription OAuth is the path xAI documents for Hermes / OpenCode-class
agents. Heavy/SuperGrok users expect login-once browser auth, not developer
billing keys. Moltis already markets OAuth for Codex + Copilot; xAI is the
obvious gap.

## Implementation map

Closest templates:

1. Auth/device flow + known provider registration → `kimi-code`
2. Responses transport quirks → `openai-codex` / openai Responses helpers
3. Reference behavior / headers / model routing → `pi-grok`

### 1. `crates/oauth`

- Add builtin config in `defaults.rs` for `xai-oauth` (`device_flow: true`).
- **Bug to fix while here:** `device_flow.rs` currently posts `scope=""`.
  It must send `config.scopes.join(" ")` (empty scopes → omit field).
  Without this, xAI login will not receive `offline_access` / `grok-cli:access`.
- Optional: support extra form fields (`referrer`) and headers on device-code
  / token poll (pi-grok sends client-version headers). Prefer a small extension
  on the existing `*_with_headers` helpers rather than a one-off xAI path.
- Refresh path must persist rotated refresh tokens (xAI rotates; single-flight
  lock like pi-grok / kimi).

### 2. `crates/provider-setup`

- Register `KnownProvider { name: "xai-oauth", auth_type: Oauth, ... }` ahead
  of API-key providers (membership-first ordering).
- Wire headers / verification URI helpers in `oauth.rs` if needed.
- Ensure Settings → Providers card appears as OAuth (no env key).

### 3. `crates/providers`

- New `xai_oauth.rs` (or module) modeled on `kimi_code.rs`:
  - load tokens from `TokenStore`
  - refresh via `auth.x.ai/oauth2/token`
  - call `cli-chat-proxy.grok.com/v1/responses` (and `/models` for discovery)
  - inject proxy identity headers
- Reuse OpenAI Responses / SSE helpers; do not fork streaming state machines.
- Map entitlement failures distinctly:
  - refresh/inference **403** → “subscription not entitled to API / upgrade or use `XAI_API_KEY`” (do **not** say re-login)
  - **400/401 invalid_grant** → re-login required
- Feature-gate like other optional providers if that is the local convention.

### 4. Gateway / CLI / UI

- `moltis auth login --provider xai-oauth` (device code print + poll).
- Web Settings provider card + onboarding list entry.
- Docs: `docs/src/providers.md` + short page or section for xAI OAuth.
  Document Heavy/SuperGrok/Premium+, proxy routing, and API-key fallback.

### 5. Tests

- Unit: oauth defaults, scope serialization fix, refresh rotation persistence,
  proxy header builder, provider registration.
- Provider tests with mock auth + mock proxy (axum), patterned after kimi.
- UI e2e only if the providers settings card needs a new selector path;
  reuse mock OAuth server where practical (device-flow variant).

## Non-goals (v1)

- Browser PKCE callback flow (pi-grok optional path) — add later if requested.
- Importing Pi/Hermes credential files automatically (nice-to-have follow-up).
- TTS / image / video / X-search surfaces (pi-grok extras) — chat/completions
  first.
- Changing behavior of API-key `xai`.

## Suggested PR slice order

1. Fix `device_flow` scope serialization + add `xai-oauth` oauth defaults/tests.
2. Register known provider + provider-setup wiring.
3. Implement `XaiOauthProvider` against CLI proxy with mock tests.
4. Wire gateway auth login + docs.
5. Manual QA with Super Heavy account (device login → model list → one chat).
6. Open upstream PR with redacted session context (per CONTRIBUTING).

## Manual QA checklist

- [ ] Device login prints verification URI + user code; headless-friendly
- [ ] Tokens stored; restart still authenticated
- [ ] Refresh rotates and persists new refresh token
- [ ] Model picker shows proxy catalog / fallback list
- [ ] Chat + tool call against `grok-4.5` (expect build remap on proxy)
- [ ] API-key `xai` still works independently
- [ ] Forced 403 path shows entitlement message, not “please re-login”

## Open questions

1. Provider id: `xai-oauth` (pi/Hermes) vs `grok` — recommend **`xai-oauth`**.
2. Should v1 speak Responses-only, or also expose chat-completions on the proxy?
   Recommend Responses-first (pi-grok / Codex family).
3. Client version pinning (`0.2.101`) — make overridable via env
   `MOLTIS_XAI_CLIENT_VERSION` like pi-grok.
4. File upstream issue before large PR? Yes — align with maintainers on proxy
   routing vs api.x.ai-only.

## References

- Local Pi extension: `~/.pi/agent/git/github.com/stnly/pi-grok`
- Hermes docs: https://hermes-agent.nousresearch.com/docs/guides/xai-grok-oauth
- xAI Hermes announcement: https://x.ai/news/grok-hermes
- Moltis OAuth providers today: `openai-codex`, `github-copilot`, (`kimi-code` device flow)
- Moltis API-key xAI: `providers.xai` / `XAI_API_KEY` → `https://api.x.ai/v1`
