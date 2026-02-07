# Tauri Native IPC Desktop Plan

**Status:** Proposed
**Priority:** High
**Complexity:** High
**Platform:** macOS first (feature-gated), extensible to Windows/Linux

## Objective

Ship a desktop app using Tauri without a localhost HTTP/WebSocket loop for UI transport.
The UI should talk to Rust through native Tauri IPC (commands + events), with a trait-based transport boundary so browser and desktop modes can coexist cleanly during migration.

## Guiding principles

1. **No localhost requirement in desktop mode**
   - Desktop mode does not bind a local HTTP/WS listener for frontend/backend communication.
   - Existing server mode remains intact for CLI/browser deployments.

2. **Traits at boundaries**
   - Define behavior contracts in Rust and JS/TS transport layers.
   - Keep core domain logic independent from transport details.

3. **Incremental migration**
   - Avoid a big-bang rewrite.
   - Introduce interfaces first, then swap adapters behind feature flags.

4. **Security first**
   - Least-privilege Tauri command allowlist.
   - No secrets in logs/events.
   - Preserve existing auth guarantees while adapting desktop UX.

## What modern Tauri apps typically do

Most production Tauri apps use:

- Bundled frontend assets (`tauri://` or custom protocol)
- Request/response via `invoke()` commands
- Push updates via `emit()`/event listeners or channels
- Optional sidecar process only when needed for compatibility

Using localhost is common in migration-heavy apps, but it is not a requirement and not the preferred architecture for a mature native desktop app.

## Current repo realities (relevant to this effort)

- `crates/gateway/src/server.rs` currently centralizes startup around HTTP router/server.
- Frontend assets under `crates/gateway/src/assets/js/` are built around browser APIs:
  - `fetch("/api/...")`
  - WebSocket request/event flow in `websocket.js`
- Onboarding currently references setup code shown in process logs.

These are good candidates for transport extraction while preserving business logic.

## Target architecture

### Crate layout

- Add a new crate: `crates/desktop`
  - Owns Tauri app bootstrap, window lifecycle, desktop-specific IPC glue
  - Depends on existing workspace crates (`moltis-gateway`, `moltis-config`, etc.)
- Keep `crates/cli` as-is for non-desktop runtime

### Runtime modes

1. **Server mode (existing)**
   - HTTP + WS transport through Axum routes
2. **Desktop mode (new)**
   - No HTTP/WS transport for UI
   - Tauri command/event transport

### Transport abstraction (Rust)

Define a transport-facing trait boundary near method dispatch so core behavior can be reused:

- `FrontendTransport` trait (conceptual)
  - `request(method, params) -> Result<Value>`
  - `emit(event, payload)`
  - streaming callback/chunk forwarding support

Adapters:

- `WebSocketTransport` (current behavior)
- `TauriIpcTransport` (new behavior)

### Transport abstraction (frontend)

Create a JS transport module interface used by all pages:

- `call(method, params)`
- `onEvent(event, handler)`
- `stream(method, params, onChunk)` (or equivalent)

Adapters:

- Browser adapter (current fetch/ws path)
- Tauri adapter (`invoke`, event listeners)

This keeps UI logic shared while transport changes underneath.

## Trait-oriented refactor strategy

### 1) Separate core application state from server wiring

Goal: instantiate and run core services without requiring Axum routing.

- Extract gateway initialization into a reusable constructor API (state/services/method registry creation).
- Keep HTTP router assembly as one consumer of that core initializer.
- Desktop crate consumes the same initializer but exposes commands/events instead of routes.

### 2) Introduce command/event facade traits

Define thin traits for the desktop bridge, implemented by gateway-core adapter structs:

- `CommandFacade`: typed request/response wrappers around method registry calls
- `EventSink`: broadcast hooks for desktop windows
- `StreamSink`: token/chunk streaming to UI

This avoids desktop code depending directly on HTTP-oriented types.

### 3) Preserve method namespace parity during migration

Even if transport changes, keep existing method names and payload contracts initially.

Benefits:

- Lower regression risk
- Easier phased frontend migration
- Existing domain method handlers reused with minimal reshaping

Later, desktop-native ergonomics can be added on top.

## Authentication and setup in desktop mode

Desktop mode should not rely on terminal logs for setup code UX.

Recommended approach:

1. Keep setup-code semantics for security model consistency.
2. Expose setup status and setup code availability via desktop IPC in a controlled way.
3. Show setup flow in desktop UI directly.

Alternative: trusted-local desktop bootstrap path that bypasses setup code only on first launch.
This is simpler UX but changes security semantics and should be an explicit design decision.

## Streaming and events

Current UX depends heavily on WS events and streaming token flow. Desktop parity is mandatory.

Plan:

- Map existing broadcast events to Tauri events one-to-one where possible.
- For high-frequency streams, prefer buffered channel dispatch from Rust to UI to avoid UI thread pressure.
- Keep event names stable initially to reduce frontend churn.

## Feature gating and build strategy

- Add workspace member `crates/desktop`.
- Gate desktop crate/deps with platform cfg and feature flags:
  - `desktop-tauri`
  - `target_os = "macos"` for initial rollout
- Keep default CLI and release workflows unaffected.

## Proposed phases

### Phase 0: Scaffolding

- Create `crates/desktop` Tauri app shell.
- Wire workspace Cargo config and feature gates.
- Build a minimal window that loads bundled frontend.

### Phase 1: Frontend transport interface

- Introduce transport abstraction module in frontend.
- Route existing API and WS usage through interface.
- Keep browser adapter as default; no behavior change expected.

### Phase 2: Core initialization extraction

- Refactor gateway startup to expose reusable core initializer.
- Ensure server mode still composes from same core initializer.
- Add tests for core setup invariants.

### Phase 3: Tauri IPC adapter

- Implement command bridge from `invoke` to method handlers.
- Implement event bridge from backend broadcasts to Tauri emits.
- Implement stream pathway for model output chunks.

### Phase 4: Desktop auth/onboarding

- Replace terminal-log assumptions in onboarding copy/flow for desktop mode.
- Keep password/passkey/API-key flows consistent with current auth model.
- Verify first-run setup + restart behavior.

### Phase 5: Packaging hardening (macOS)

- App bundle correctness
- Signing + notarization
- Entitlements/hardened runtime review
- Crash/startup diagnostics

### Phase 6: Cross-platform extension (optional)

- Expand gates and CI to Windows/Linux
- Address OS-specific path, permission, and packaging differences

## Testing strategy

### Rust

- Unit tests for trait adapters (IPC command mapping, event mapping)
- Integration tests for core initializer independent of HTTP router
- Auth/setup behavior tests in desktop mode

### Frontend

- Adapter contract tests (browser vs tauri transport parity)
- Smoke flows: onboarding, chat send/stream, session/project views

### End-to-end

- Desktop launch -> onboarding -> provider setup -> first message -> restart persistence

## Security checklist

- No open localhost listener in desktop mode
- IPC command allowlist is explicit and minimal
- All secret-bearing structs remain `Secret<String>`
- No sensitive payloads emitted to logs/events
- Origin/CSWSH protections remain for server mode (unchanged)

## Risks and mitigations

1. **Refactor scope grows too large**
   - Mitigation: transport interface first, core extraction second, desktop adapter third.

2. **Streaming parity issues**
   - Mitigation: define stream contract tests before replacing WS path.

3. **Auth regressions**
   - Mitigation: keep auth model intact; change UX delivery path, not core policy.

4. **Release pipeline complexity**
   - Mitigation: macOS-only rollout first; treat signing/notarization as dedicated phase.

## Effort estimate

- MVP (desktop app functional, native IPC, macOS dev build): **2-4 weeks**
- Production-ready macOS distribution (sign/notarize/update polish): **+1-3 weeks**
- Cross-platform expansion: **+2-5 weeks** depending on parity requirements

## Recommended first milestone

"Desktop Dev Preview (macOS)" with:

- bundled UI
- no localhost UI transport
- command/event parity for core chat and settings flows
- first-run onboarding functional in-window

This milestone proves the architecture decision early while containing risk.

## Execution checklist

- [ ] Create `crates/desktop` Tauri crate and add it to workspace members.
- [ ] Add `desktop-tauri` feature wiring in `crates/cli/Cargo.toml` (enabled by default only if explicitly desired for desktop build profile).
- [ ] Add macOS-only cfg gates for Tauri dependencies and desktop entrypoint.
- [ ] Introduce frontend transport interface module in gateway assets JS.
- [ ] Implement browser transport adapter with no behavior changes.
- [ ] Route existing WS/fetch call sites through the new transport interface.
- [ ] Extract gateway core initialization from `start_gateway()` into reusable constructor(s).
- [ ] Add desktop IPC command bridge to call existing method handlers.
- [ ] Add desktop event bridge for broadcast + stream chunk delivery.
- [ ] Add desktop onboarding/setup UX path that does not require terminal-log inspection.
- [ ] Add tests for transport adapter parity and desktop command/event mapping.
- [ ] Add initial macOS desktop CI build lane (no signing yet).

## Concrete PR slices

### PR 1: Desktop crate scaffold + feature gates

**Goal:** compile a macOS desktop shell without changing runtime behavior.

**Likely files:**

- `Cargo.toml` (workspace member + optional workspace deps for tauri)
- `crates/desktop/Cargo.toml`
- `crates/desktop/src/main.rs`
- `crates/desktop/tauri.conf.json` (or equivalent Tauri v2 config)
- `crates/cli/Cargo.toml` (feature forward declarations only)
- `.github/workflows/ci.yml` (add desktop build job, macOS only)

**Acceptance criteria:**

- `cargo build -p moltis-desktop` works on macOS.
- Existing `moltis` CLI build/test paths are unchanged.
- No localhost transport decisions made yet.

### PR 2: Frontend transport abstraction (browser-compatible)

**Goal:** prepare UI for IPC by isolating transport behind one interface.

**Likely files:**

- `crates/gateway/src/assets/js/transport.js` (new abstraction)
- `crates/gateway/src/assets/js/websocket.js` (wrapped as browser adapter)
- `crates/gateway/src/assets/js/helpers.js` (route RPC calls through transport)
- `crates/gateway/src/assets/js/events.js` (route event subscription through transport)
- Pages that directly call `fetch`/WS internals (incremental touch only)

**Acceptance criteria:**

- Browser mode behavior remains unchanged.
- No direct WS/fetch usage remains in page modules targeted by this PR.
- JS lint/format passes and asset CSS rebuild is run if utility classes changed.

### PR 3: Gateway core extraction from HTTP server wiring

**Goal:** create reusable runtime initializer for server and desktop consumers.

**Likely files:**

- `crates/gateway/src/server.rs`
- `crates/gateway/src/state.rs`
- `crates/gateway/src/services.rs`
- `crates/gateway/src/methods.rs`
- `crates/gateway/src/lib.rs` (export reusable init APIs)

**Acceptance criteria:**

- `start_gateway()` still works and composes from extracted core.
- New initializer can be invoked without binding TCP listener.
- Added tests validate core initialization and method dispatch availability.

### PR 4: Desktop IPC command bridge (request/response)

**Goal:** call existing method handlers through Tauri `invoke`.

**Likely files:**

- `crates/desktop/src/main.rs`
- `crates/desktop/src/ipc.rs` (new)
- `crates/desktop/src/app_state.rs` (new)
- `crates/desktop/src/error.rs` (new)

**Acceptance criteria:**

- UI can execute core RPC-style methods over IPC in desktop mode.
- Command allowlist is explicit.
- Error mapping is structured and does not leak secrets.

### PR 5: Desktop event + streaming bridge

**Goal:** replace WS event stream with Tauri event delivery.

**Likely files:**

- `crates/desktop/src/events.rs` (new)
- `crates/gateway/src/broadcast.rs` (adapter hook-in)
- `crates/gateway/src/chat.rs` or streaming call path (chunk forwarding)
- `crates/gateway/src/assets/js/transport-tauri.js` (new)

**Acceptance criteria:**

- Streaming output appears progressively in desktop UI.
- Existing event names are preserved initially.
- Backpressure strategy prevents UI lockups.

### PR 6: Desktop onboarding/auth UX parity

**Goal:** remove terminal dependency in first-run desktop flow.

**Likely files:**

- `crates/gateway/src/assets/js/page-onboarding.js`
- `crates/desktop/src/ipc.rs`
- `crates/gateway/src/auth_routes.rs` (only if minimal API support is needed)

**Acceptance criteria:**

- First-run password setup can be completed entirely in-window.
- Setup-code policy remains explicit and tested.
- Passkey/password/API key paths still behave as expected.

## Task-level checklist for PR 1 and PR 2 (start here)

### PR 1 immediate tasks

- [ ] Add `crates/desktop` skeleton with basic Tauri window startup.
- [ ] Add workspace membership and dependency declarations.
- [ ] Add cfg gates to keep non-macOS builds clean.
- [ ] Add CI job that compiles desktop crate on macOS.
- [ ] Add minimal README snippet in crate for local run/build commands.

### PR 2 immediate tasks

- [ ] Create `transport.js` contract with `call`, `onEvent`, and `stream`.
- [ ] Implement browser-backed adapter that delegates to existing code.
- [ ] Switch one high-value flow first (`sendRpc` and chat connect path).
- [ ] Migrate remaining page modules incrementally.
- [ ] Run `biome check --write` and verify no runtime regressions manually.

## Suggested verification commands per PR

```bash
# Rust compile checks
cargo check
cargo build -p moltis

# Desktop crate compile (macOS)
cargo build -p moltis-desktop

# JS lint/format
biome check --write

# Project checks expected by repo
just format-check
just lint
cargo test
```

## Definition of done for "Desktop Dev Preview"

- Desktop app launches and renders bundled UI.
- UI transport is native IPC/events (no localhost loop for UI path).
- Chat request/response + streaming work end-to-end.
- Onboarding works in-window for first run.
- Existing CLI/server workflows remain functional.
