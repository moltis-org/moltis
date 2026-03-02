# Plan: Channel Architecture Rewrite (Registry + Capability Modes)

**Status:** Nearly Complete (Phase 5 UI metadata remaining)
**Priority:** High
**Complexity:** High
**Goal:** Refactor Moltis channel handling so adding channel `N+1` is mostly adapter work, not gateway surgery.

## Why This Rewrite

Current channel wiring is explicit and repetitive:

- `LiveChannelService` branches over each plugin in many methods.
- `MultiChannelOutbound` resolves account ownership by scanning plugins.
- Startup in `server.rs` manually loops each configured channel map and each stored account type.

This works for four channels, but it does not scale cleanly to many channels or mixed maturity levels (full duplex vs send-only).

## Current Constraints to Preserve

- Existing behavior for Telegram, Discord, Microsoft Teams, and WhatsApp must remain stable.
- Existing config shape must remain backward-compatible for at least one release.
- Existing channel security model (allowlist, OTP flows, account disable on runtime errors) must stay intact.
- Existing RPC/API payload shape should remain stable unless explicitly versioned.

## Key Design Principles

1. **Capability-first**: channels declare what they can do, the runtime routes accordingly.
2. **Registry-driven**: core runtime should not require per-channel `match` blocks.
3. **Typed metadata**: replace stringly mode checks with enums.
4. **Progressive channel maturity**: support `send-only` and `webhook-only` as first-class states.
5. **No hidden behavior changes**: migration path and compatibility tests are mandatory.

## Target Architecture

### 1) Channel Descriptor and Capabilities

Add typed descriptors in `crates/channels`:

- `ChannelDescriptor`:
  - `channel_type: ChannelType`
  - `display_name: &'static str`
  - `capabilities: ChannelCapabilities`
  - `account_config_schema: Option<...>` (if/when UI schema export is needed)
- `ChannelCapabilities`:
  - `inbound_mode: InboundMode` (`none`, `polling`, `webhook`, `gateway_loop`)
  - `supports_outbound: bool`
  - `supports_streaming: bool`
  - `supports_pairing: bool`
  - `supports_allowlist: bool`
  - `supports_voice_ingest: bool`

This lets UI and gateway truthfully represent channel maturity.

### 2) Runtime Registry and Account Index

Add a runtime registry in gateway:

- `ChannelRuntimeRegistry` maps `ChannelType -> Arc<RwLock<dyn ChannelPlugin>>` (or a typed wrapper preserving trait object safety).
- `ChannelAccountIndex` maps `account_id -> ChannelType` and is updated on start/stop/add/remove/update.

Outcome:

- `resolve_channel_type()` becomes O(1) index lookup in steady state.
- Outbound routing does not scan every plugin each call.

### 3) Generic Service Layer

Replace per-channel branching in `LiveChannelService` with descriptor/registry iteration:

- Status collection iterates registered channels.
- Start/stop/update operations are routed through registry lookups.
- Store hydration loads persisted channels and asks registry if the channel type is currently available.

### 4) Generic Outbound Router

Refactor `MultiChannelOutbound`:

- Use `ChannelAccountIndex` + registry instead of hardcoded plugin fields.
- Keep `ChannelOutbound` and `ChannelStreamOutbound` public interfaces unchanged initially.
- Return structured unknown-account errors when index miss occurs.

### 5) Config and Validation Alignment

Keep the current config keys, but drive validation from registry metadata:

- Valid channel types in `channels.offered` should be generated from registered descriptors.
- Fix immediate mismatch: include `whatsapp` in valid offered set.
- Add optional metadata for UI filtering by capability (for example, hide channels without inbound mode in onboarding flows that require inbound).

## Implementation Plan

### Phase 0: Baseline and Safety Net — DONE

1. ~~Add integration tests that snapshot current behavior for account add/remove/update, status listing, outbound routing.~~
2. ~~Add regression test for `channels.offered` accepting `whatsapp`.~~
3. ~~Add benchmark-ish unit test for account resolution path.~~

**Implemented in:** `a80527c1`, `2cabfc5f`

### Phase 1: Channel Descriptor Layer — DONE

1. ~~Add `ChannelCapabilities`, `InboundMode`, `ChannelDescriptor` to `crates/channels`.~~
2. ~~Add descriptor function to each plugin crate.~~
3. ~~Expose descriptor list from gateway startup.~~

**Implemented in:** `crates/channels/src/plugin.rs` (types + `ChannelType::ALL` + `descriptor()`), `crates/channels/src/registry.rs` (`descriptors()` method). Commit `ebf48743`.

### Phase 2: Registry + Account Index — DONE

1. ~~Introduce `ChannelRuntimeRegistry`.~~
2. ~~Introduce `ChannelAccountIndex` with bind/unbind/lookup.~~
3. ~~Populate index on startup from config + store hydration; keep in sync on runtime mutations.~~

**Implemented in:** `crates/channels/src/registry.rs` (678 lines, 15+ unit tests). Commit `a80527c1`.

### Phase 3: Generic Channel Service Refactor — DONE

1. ~~Refactor `LiveChannelService` storage from explicit plugin fields to registry handles.~~
2. ~~Replace channel-specific `match` blocks in start/stop, status, allowlist read/update paths.~~
3. ~~Keep feature-gated behavior for missing channels explicit and logged.~~

**Implemented in:** `crates/gateway/src/channel.rs` (438→240 lines). Commit `a80527c1`.

### Phase 4: Generic Outbound Router Refactor — DONE

1. ~~Refactor `MultiChannelOutbound` to registry + index lookup.~~
2. ~~Remove per-channel plugin and outbound fields.~~
3. ~~Preserve current outbound semantics and error behavior.~~

**Implemented in:** `RegistryOutboundRouter` in `crates/channels/src/registry.rs`. Deleted `crates/gateway/src/channel_outbound.rs` (246 lines). Commit `a80527c1`.

### Phase 5: Validation + UI Metadata — PARTIALLY DONE

1. ~~Generate valid `channels.offered` set from descriptors (requires Phase 1).~~
2. Add capability metadata to GON payload/API so UI can present channel maturity clearly.
3. Update docs and onboarding copy to distinguish full duplex, webhook/polling inbound, outbound-only.

**Partially implemented in:** `crates/config/src/schema.rs` (`KNOWN_CHANNEL_TYPES` constant), `crates/config/src/validate.rs` (uses constant instead of hardcoded vec). Commit `ebf48743`.

### Phase 6: Prove Extensibility with One New Adapter — DONE

~~Add one intentionally thin channel adapter to prove new architecture cost is lower.~~

**Implemented:** Slack added as a full-duplex channel (`crates/slack/`) with zero edits to `LiveChannelService` or outbound router internals. Commits `d15d21e7`, `9495e0ba`.

## Remaining Work

- **Phase 5** (remaining): GON payload/API capability metadata + docs/onboarding copy updates
