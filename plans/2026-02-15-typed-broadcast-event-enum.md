# Plan: Typed `BroadcastEvent` enum + sandbox notice in service layer

## Summary

Replace the untyped `broadcast(state, "event-name", serde_json::Value, opts)` signature
with a typed `broadcast(state, BroadcastEvent::Variant(payload), opts)` enum across
all ~80 call sites. Also move sandbox-toggle notice generation into the service layer
where change detection already lives.

## Files to create/modify

| File | Action |
|------|--------|
| `crates/gateway/src/broadcast_types.rs` | **NEW** — all event/payload types |
| `crates/gateway/src/broadcast.rs` | Modify signature, add bridge |
| `crates/gateway/src/lib.rs` | Add `mod broadcast_types` |
| `crates/gateway/src/session.rs` | Return `PatchResult` with `sandbox_changed` |
| `crates/gateway/src/session_types.rs` | Add `PatchResult`, `SandboxChange` |
| `crates/gateway/src/services.rs` | Update `SessionService::patch` return type |
| `crates/gateway/src/chat.rs` | Migrate ~30 calls, absorb `ChatFinalBroadcast`/`ChatErrorBroadcast` |
| `crates/gateway/src/methods.rs` | Migrate ~20 calls, sandbox notice reads `PatchResult` |
| `crates/gateway/src/server.rs` | Migrate ~15 calls |
| `crates/gateway/src/channel_events.rs` | Migrate ~10 calls |
| `crates/gateway/src/local_llm_setup.rs` | Migrate ~11 calls |
| `crates/gateway/src/ws.rs` | Migrate 2 calls |
| `crates/gateway/src/approval.rs` | Migrate 1 call |
| `crates/gateway/src/mcp_health.rs` | Migrate 1 call |
| `crates/gateway/src/push_routes.rs` | Migrate 2 calls |

---

## Step 1: Create `broadcast_types.rs` with type hierarchy

### Top-level enum

```rust
pub enum BroadcastEvent {
    Chat(ChatEvent),
    Session(SessionEvent),
    ModelsUpdated(ModelsUpdatedEvent),
    Presence(PresenceEvent),
    McpStatus(Vec<serde_json::Value>),        // already-serialized MCP status
    Channel(serde_json::Value),               // ChannelEvent serialized upstream
    Tick(TickPayload),
    SandboxImageBuild(PhaseEvent<SandboxBuildStart, SandboxBuildDone>),
    SandboxHostProvision(PhaseEvent<SandboxHostStart, SandboxHostDone>),
    BrowserImagePull(PhaseEvent<BrowserPullContext, BrowserPullContext>),
    SkillsInstallProgress(PhaseEvent<SkillsInstallContext, SkillsInstallContext>),
    SkillsChanged,
    LocalLlmDownload(LocalLlmDownloadEvent),
    VoiceConfigChanged(VoiceConfigChangedPayload),
    ExecApprovalRequested(ExecApprovalPayload),
    NodePairRequested(PairRequestedPayload),
    NodePairResolved(PairResolvedPayload),
    DevicePairResolved(PairResolvedPayload),
    HooksStatus(HooksStatusPayload),
    PushSubscriptions(PushSubscriptionPayload),
    UpdateAvailable(serde_json::Value),       // update check result
    MetricsUpdate(MetricsUpdatePayload),
    LogsEntry(serde_json::Value),             // log entry
    CronJobCreated { job: serde_json::Value },
    CronJobUpdated { job: serde_json::Value },
    CronJobRemoved { job_id: String },
    /// Escape hatch for dynamic RPC broadcasts (system-event, node.event).
    Custom { event: String, payload: serde_json::Value },
}
```

Two methods on the enum:
- `fn event_name(&self) -> &str` — maps variant to wire event name string
- `fn into_payload(self) -> serde_json::Value` — serializes inner type

### `ChatEvent` sub-enum (tagged by `state`)

```rust
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ChatEvent {
    Start(ChatRunRef),
    Thinking(ChatRunRef),
    ThinkingText(ChatThinkingText),
    ThinkingDone(ChatRunRef),
    VoicePending(ChatRunRef),
    ToolCallStart(ChatToolCall),
    ToolCallEnd(ChatToolCall),
    Delta(ChatDelta),
    Final(ChatFinal),
    Error(ChatError),
    AutoCompact(ChatAutoCompact),
    Notice(ChatNotice),
    Queued(ChatQueued),
    QueueCleared(ChatQueueCleared),
    SessionCleared(ChatSessionCleared),
    ChannelUser(ChatChannelUser),
    Retrying(ChatRunRef),
    SubAgentStart(ChatSubAgent),
    SubAgentDone(ChatSubAgent),
}
```

Existing `ChatFinalBroadcast` and `ChatErrorBroadcast` in `chat.rs` become
`ChatFinal` and `ChatError` payload structs (remove hardcoded `state` field,
serde tag handles it).

### `SessionEvent` sub-enum (tagged by `kind`)

```rust
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionEvent {
    Created { session_key: String },
    Patched { session_key: String, version: u64 },
    Switched { session_key: String },
}
```

### Phase-based pattern

Several events follow `phase: start | done | error`. Use a tagged enum:

```rust
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum PhaseEvent<S, D> {
    Start(S),
    Done(D),
    Error { #[serde(flatten)] context: S, error: String },
}
```

Concrete types fill the start/done context (e.g. `SandboxBuildStart` has
`packages`, `SandboxBuildDone` has `tag` + `built`).

### Variants kept as `serde_json::Value`

- `Channel` — `ChannelEvent` is in `moltis_channels` crate, already serialized
- `UpdateAvailable` — opaque update check result
- `LogsEntry` — opaque log entry
- `CronJob*` — cron job objects from `moltis_cron`
- `McpStatus` — array of server status objects from `moltis_mcp`

These can be typed later when those crates export shared types.

---

## Step 2: Update `broadcast()` with bridge strategy

1. Rename current `broadcast()` to `broadcast_raw()` (private, same signature)
2. Add new typed `broadcast()` that extracts `event_name` + `payload` and calls `broadcast_raw()`
3. This allows incremental migration — call sites switch one-at-a-time

```rust
pub async fn broadcast(state: &Arc<GatewayState>, event: BroadcastEvent, opts: BroadcastOpts) {
    let name = event.event_name().to_string();
    let payload = event.into_payload();
    broadcast_raw(state, &name, payload, opts).await;
}
```

After all sites are migrated, inline `broadcast_raw()` back into `broadcast()`.

---

## Step 3: Sandbox notice — service layer changes

### `session_types.rs`: Add `PatchResult` and `SandboxChange`

```rust
pub struct PatchResult {
    pub entry: serde_json::Value,   // the session metadata JSON (existing return)
    pub sandbox_changed: Option<SandboxChange>,
}

pub enum SandboxChange {
    Enabled,
    Disabled,
    Cleared,  // override removed, using global setting
}
```

### `services.rs`: Update trait

```rust
async fn patch(&self, params: Value) -> ServiceResult<PatchResult>;
```

Update `NoopSessionService::patch()` to return `PatchResult { entry: json!({}), sandbox_changed: None }`.

### `session.rs`: `patch()` returns `PatchResult`

Move sandbox change detection (currently lines 999-1030) to populate
`sandbox_changed` field. The system notification append stays in the service.
The UI notice broadcasting moves to the caller in `methods.rs` using the
returned `SandboxChange`.

### `methods.rs`: Session patch handler reads `PatchResult`

```rust
let result = ctx.state.services.session.patch(ctx.params.clone()).await?;
let version = result.entry.get("version")...;
broadcast(&ctx.state, BroadcastEvent::Session(SessionEvent::Patched { ... }), opts).await;

if let Some(change) = result.sandbox_changed {
    let message = match change {
        SandboxChange::Enabled => "Sandbox enabled -- commands now run in container.",
        SandboxChange::Disabled => "Sandbox disabled -- commands now run on host.",
        SandboxChange::Cleared => "Sandbox override cleared -- using global setting.",
    };
    broadcast(&ctx.state, BroadcastEvent::Chat(ChatEvent::Notice(ChatNotice {
        session_key: key, title: "Sandbox".into(), message: message.into(),
    })), BroadcastOpts::default()).await;
}
```

---

## Step 4: Migrate call sites (file-by-file order)

Migrate in order of complexity, verifying compilation after each file:

1. `broadcast.rs` — `broadcast_tick()` uses `BroadcastEvent::Tick`
2. `push_routes.rs` (2 calls) — `PushSubscriptions`
3. `approval.rs` (1 call) — `ExecApprovalRequested`
4. `mcp_health.rs` (1 call) — `McpStatus`
5. `ws.rs` (2 calls) — `Presence`
6. `server.rs` phase-based events (~9 calls) — sandbox build, host provision, browser pull
7. `server.rs` remaining (~6 calls) — cron, skills.changed, metrics, logs, update
8. `methods.rs` session/pairing/voice/skills (~20 calls)
9. `channel_events.rs` (~10 calls) — session + chat + channel events
10. `local_llm_setup.rs` (~11 calls) — `LocalLlmDownload`
11. `chat.rs` (~30 calls) — absorb existing typed structs, convert RunnerEvent mapping

After all migrated, remove `broadcast_raw()`.

---

## Step 5: Tests

### Serialization round-trip tests (in `broadcast_types.rs`)

One test per event variant verifying the JSON wire format matches what the
frontend expects. Key checks:
- `ChatEvent` variants include correct `state` tag + `camelCase` keys
- `SessionEvent` variants include correct `kind` tag
- `PhaseEvent` variants include correct `phase` tag
- `event_name()` returns the correct string for each variant
- Optional fields are omitted when `None`

### Wire-format golden test

A comprehensive test with one `(BroadcastEvent, expected_event_name, expected_json)`
tuple per variant, asserting exact JSON equality.

### Existing tests

- `session.rs` tests (`patch_sandbox_toggle_appends_system_notification`, etc.)
  updated for `PatchResult` return type
- E2E tests in `crates/gateway/ui/e2e/specs/sessions.spec.js` serve as
  regression guard for wire compatibility — must pass unchanged

---

## Verification

```bash
# Compile check after each file migration
cargo check -p moltis-gateway

# Run gateway unit tests
cargo test -p moltis-gateway

# Run session-specific tests
cargo test -p moltis-gateway -- session

# Format check
cargo +nightly-2025-11-30 fmt --all -- --check

# Clippy
cargo +nightly-2025-11-30 clippy -Z unstable-options --workspace --all-targets --timings -- -D warnings

# E2E tests (wire format regression)
cd crates/gateway/ui && npx playwright test
```
