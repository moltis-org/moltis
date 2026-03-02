# Plan: Contract Test Harness for Integrations

**Status:** Channel contracts done; Provider/Memory/Tool contracts not started
**Priority:** High
**Complexity:** Medium
**Goal:** Prevent integration regressions by requiring every adapter/backend to pass shared behavior contracts.

## Context

The registry-driven channel architecture is in place (`a80527c1`) and Slack is the first channel
implemented against it (`d15d21e7`). Per-channel unit tests exist (e.g. 15+ registry tests,
33+ Slack tests), but there is no shared contract suite that all channels must pass.
This plan is a prerequisite before the channel expansion waves in the parity plan.

## Scope

Contract suites for:

1. Channels
2. Providers
3. Memory backends
4. Tool executors

## Channel Contract Suite — PARTIALLY DONE

Must validate:

1. ~~Start/stop idempotency~~
2. ~~Unknown account error semantics~~
3. Outbound send behavior and retry mapping
4. Streaming behavior consistency
5. Health probe behavior
6. ~~Capability metadata correctness~~

**Implemented in:** `crates/channels/src/contract.rs` (4 shared test functions: `lifecycle_start_stop`, `double_start_same_account`, `stop_unknown_account`, `config_view_after_start`). Descriptor coherence tests added to all 5 channel crates. Commit `ebf48743`.

## Provider Contract Suite

Must validate:

1. Non-stream and stream response parity
2. Tool-call handling behavior
3. Error classification mapping (retryable vs fatal)
4. Rate-limit handling contracts

## Memory Contract Suite

Must validate:

1. Ingest and retrieval invariants
2. Keyword/vector retrieval consistency expectations
3. Delete and update visibility
4. Snapshot export/import roundtrip

## Tool Executor Contract Suite

Must validate:

1. Timeout behavior
2. Sandbox policy enforcement
3. Output truncation semantics
4. Error reporting format

## Implementation Phases

### Phase 0: Harness Base — DONE (channels)

1. ~~Build test fixtures and trait-based reusable assertions.~~
2. ~~Add fake transport adapters for deterministic tests.~~

**Implemented in:** `crates/channels/src/contract.rs` + updated `TestPlugin` with `NullOutbound`/`NullStreamOutbound`/`TestConfigView`. Commit `ebf48743`.

### Phase 1: Migrate Existing Implementations — PARTIALLY DONE

1. ~~Telegram, Discord, Teams, WhatsApp, Slack channel contracts.~~
2. Existing provider and memory backends.

### Phase 2: CI Enforcement

1. Add required CI job for contract suites.
2. Require new integration PRs to include contract tests.

## Acceptance Criteria

1. Any new integration can be validated by plugging into shared contract harness.
2. Integration regressions are caught before E2E.
3. Contract failures produce actionable diagnostics.
