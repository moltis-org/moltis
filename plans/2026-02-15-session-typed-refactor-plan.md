# Plan: Session Typed Refactor (No Raw `Value` Access)

## Goal
Refactor `crates/gateway/src/session.rs` so session business logic uses typed Rust data, not ad-hoc JSON traversal (`.get("runId")`, `.get("messageIndex")`, etc.).

Primary outcomes:
- No direct `msg.get(...)` parsing in core session logic.
- No direct `params.get(...)` parsing in session RPC handlers.
- Message/history operations operate on typed message enums/structs.

## Scope
In scope:
- `crates/gateway/src/session.rs`
- `crates/sessions/src/store.rs` (typed read/write helpers)
- `crates/sessions/src/message.rs` (small extensions if needed)
- Session-specific tests in `crates/gateway/src/session.rs`

Out of scope for this PR:
- Full gateway-wide RPC type system migration.
- Rewriting every service trait in `crates/gateway/src/services.rs`.

## Design Constraints
- Keep JSON-RPC transport as-is for now, but parse once at boundary into typed inputs.
- Avoid `serde_json::Value` in session business logic paths.
- Prefer explicit conversion logic (`TryFrom<Value>` or dedicated parser helpers) over repeated field probing.
- Preserve wire compatibility: accept both `runId` and existing index-based targeting fields.

## Target Architecture
### 1) Typed RPC input structs (session-local)
Create request structs in `crates/gateway/src/session.rs` (or `session_types.rs`):
- `PreviewParams { key: String, limit: usize }`
- `ResolveParams { key: String }`
- `PatchParams { key: String, ...optional patch fields... }`
- `VoiceGenerateParams { key: String, target: VoiceTarget }`
- `ShareCreateParams { key: String, visibility: ShareVisibility }`
- `ShareListParams { key: String }`
- `ShareRevokeParams { id: String }`
- `ResetParams { key: String }`
- `DeleteParams { key: String, force: bool }`
- `ForkParams { key: String, label: Option<String>, fork_point: Option<usize> }`
- `BranchesParams { key: String }`
- `SearchParams { query: String, limit: usize }`

`VoiceTarget` should model precedence explicitly:
- `ByRunId(String)`
- `ByMessageIndex(usize)`

### 2) Typed session message adapter
Use `moltis_sessions::message::PersistedMessage` as the canonical type in `session.rs`.

Add adapter helpers:
- `fn parse_history(values: Vec<Value>) -> Result<Vec<PersistedMessage>, String>`
- `fn encode_history(messages: &[PersistedMessage]) -> Result<Vec<Value>, String>`

Longer-term improvement (same PR if clean): typed store APIs so callers do not manually convert.

### 3) Typed store helpers
Add in `crates/sessions/src/store.rs`:
- `read_typed(&self, key: &str) -> Result<Vec<PersistedMessage>>`
- `read_last_n_typed(&self, key: &str, n: usize) -> Result<Vec<PersistedMessage>>`
- `replace_history_typed(&self, key: &str, messages: &[PersistedMessage]) -> Result<()>`

Keep existing `Value` APIs temporarily for compatibility with unaffected call sites.

### 4) Share projection type boundary
For share creation helpers in `session.rs`, pattern match on `PersistedMessage` variants instead of probing `Value` maps.

For truly dynamic tool payloads (`ToolResult.arguments/result`), isolate shape extraction to one adapter function with typed intermediates where possible.

## Implementation Phases
### Phase 1: Typed params for session RPC handlers
- Add typed param structs and parsing helpers.
- Replace `.get(...)` chains in:
  - `preview`
  - `resolve`
  - `patch`
  - `voice_generate`
  - `share_create`
  - `share_list`
  - `share_revoke`
  - `reset`
  - `delete`
  - `fork`
  - `branches`
  - `search`
- Preserve current defaults and error messages where practical.

Acceptance criteria:
- No `.get(...)` on `params` in `session.rs` RPC methods.
- `voice_generate` target resolution is explicit and tested.

### Phase 2: Typed history/message handling
- Refactor helpers to consume `PersistedMessage` instead of `Value`:
  - `filter_ui_history`
  - preview extractors
  - share conversion helpers
  - voice message targeting logic
- Remove direct role/content field probing from these helpers.

Acceptance criteria:
- Session message role/content logic uses enum pattern matching.
- No `.get("role")`/`.get("content")` in core helper paths.

### Phase 3: Store typed APIs + migration
- Add typed APIs to `SessionStore`.
- Migrate `session.rs` to typed APIs.
- Keep old APIs for now to avoid cross-crate churn.

Acceptance criteria:
- `session.rs` reads/writes typed messages end-to-end.
- Conversion to/from JSON values only at service response/request boundaries.

### Phase 4: Cleanup and hardening
- Deduplicate legacy helper code used only by `Value` paths.
- Keep one narrow adapter for dynamic tool-result subtrees.
- Add strict tests for parser errors and fallback compatibility.

## Test Plan
Targeted tests (not full suite):
- Existing `session.rs` tests updated to typed fixtures.
- New tests for request parsing:
  - missing required keys
  - invalid types
  - `runId` precedence over `messageIndex/historyIndex`
- New tests for typed history handling:
  - assistant filtering behavior
  - share projection behavior
  - voice targeting against `run_id`
- Store typed API tests in `crates/sessions/src/store.rs`.

Suggested commands:
- `cargo test -p moltis-gateway session::tests::`
- `cargo test -p moltis-sessions store::tests::`
- `cargo fmt`
- `cargo clippy --all --benches --tests --examples --all-features`

## Risks and Mitigations
- Risk: Behavior drift in mixed legacy JSON shapes.
  - Mitigation: Add compatibility tests using old payload shapes.
- Risk: Tool result payloads remain partially dynamic.
  - Mitigation: Contain dynamic access to one adapter module/function.
- Risk: Large diff in `session.rs`.
  - Mitigation: land in small commits by phase.

## Definition of Done
- `crates/gateway/src/session.rs` has no ad-hoc `params.get(...)` or message-role `.get(...)` traversal in business logic.
- Session logic is type-driven using `PersistedMessage` and typed param structs.
- Existing behavior for user-visible session operations is preserved.
- Targeted tests pass.
