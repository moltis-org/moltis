# Core Memory Lifecycle Unification

**Status:** Draft
**Priority:** Medium
**Complexity:** Medium
**Goal:** Move memory durability behavior fully into core runtime flow and deprecate hook-based memory snapshots.

## Overview

Today memory is handled in two places:

- Core compaction flow performs memory work (silent memory turn + compaction summary write).
- Bundled `session-memory` hook writes snapshots on command boundaries (`/new`, `/reset`).

This split makes behavior harder to reason about and can create user confusion about where durable memory comes from. This plan consolidates durable memory behavior into core and keeps hooks for optional, non-critical customization only.

## Desired End State

- Memory durability is guaranteed by core logic only.
- Hook system no longer owns required memory writes.
- Clear, documented memory lifecycle:
  1. Ongoing turns (optional threshold-triggered flush)
  2. Before compaction
  3. After compaction
  4. Session boundary events (`new`/`reset`) handled by core, not hook
- Existing users are migrated safely with deprecation path and clear release notes.

## Why This Change

- **Reliability:** required behavior should not depend on optional hook configuration.
- **Consistency:** one source of truth for when memory is written.
- **Observability:** one instrumentation path for memory events.
- **Security and safety:** fewer extension points for critical persistence behavior.

## Scope

### In scope

- Add/extend core memory lifecycle methods in gateway/chat/session flows.
- Deprecate then remove bundled `session-memory` hook behavior.
- Add config knobs for cadence/threshold where needed.
- Add tests for all lifecycle triggers.
- Update docs and changelog.

### Out of scope

- Replacing the hook framework itself.
- Changing storage backend format.
- Introducing provider-specific memory pipelines beyond existing abstractions.

## Proposed Architecture

Create a core `MemoryLifecycle` path (service or module) called from runtime flows:

- `on_turn_end(...)` for ongoing threshold-based memory flush.
- `before_compaction(...)` for pre-summary durable extraction.
- `after_compaction(...)` for summary metadata write/sync.
- `on_session_reset(...)` for end-of-session snapshot semantics currently tied to `/new` and `/reset`.

Implementation can live initially in `crates/gateway/src/chat.rs` with extraction to a dedicated module after tests stabilize.

## Trigger Mapping

| Current trigger | Current owner | New owner |
|---|---|---|
| `/new` `/reset` snapshot | `session-memory` hook | core session/command path |
| pre-compaction durable write | core | core (unchanged, refactored) |
| post-compaction summary write | core | core (unchanged, refactored) |
| near-context ongoing flush | partial/none in Moltis runtime path | core threshold-based turn hook |

## Implementation Plan

1. **Inventory and contracts**
   - Document all current memory write points and side effects.
   - Define one typed event enum for core memory lifecycle internal use.

2. **Introduce core lifecycle module**
   - Add module with explicit entry points (`on_turn_end`, `before_compaction`, `after_compaction`, `on_session_reset`).
   - Move existing compaction memory code into these functions without behavior change.

3. **Add ongoing near-compaction flush**
   - Add threshold check based on session token usage and model context window.
   - Reuse silent-turn machinery for durable memory extraction.
   - Add guardrails: avoid duplicate flush in same compaction window; skip when no writable memory target.

4. **Replace hook-owned reset/new snapshot**
   - Trigger core `on_session_reset` from command/session reset path.
   - Keep hook dispatch for compatibility but remove memory write responsibility.

5. **Deprecate bundled `session-memory` hook**
   - Mark as deprecated in docs.
   - Keep no-op/compat mode for one release cycle (configurable warning).
   - Remove from default bundled hooks in subsequent release.

6. **Instrumentation**
   - Add tracing spans around each lifecycle stage.
   - Add metrics counters/histograms for attempts, success/failure, duration, files written.

7. **Testing**
   - Unit tests for trigger conditions and dedupe behavior.
   - Integration tests for:
     - turn-based threshold flush
     - compaction pre/post behavior
     - reset/new snapshot without hook
     - failure tolerance (provider/memory sync failure does not break chat flow)

8. **Docs + changelog**
   - Add a single “Memory lifecycle” section in docs.
   - Document migration path for users who configured `session-memory` hook.
   - Update `CHANGELOG.md` under `[Unreleased]`.

## Compatibility and Migration

- **Short term:** keep hook event dispatch unchanged; bundled `session-memory` becomes optional/deprecated.
- **Default behavior:** remains functionally equivalent or stronger (ongoing flush support added).
- **User configs:** if user explicitly enables `session-memory`, surface a warning that core now handles durability.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| Duplicate writes from both core and hook during transition | De-dupe marker in session metadata; deprecation warnings |
| Performance hit from ongoing flush checks | cheap threshold pre-check; run expensive flush only when needed |
| Behavior drift across reset/compaction paths | golden integration tests on session history + memory files |
| Confusion during migration | explicit docs and startup warning when deprecated hook is active |

## Acceptance Criteria

- Memory durability works with bundled hooks disabled.
- `/new` and `/reset` still produce expected memory snapshot behavior via core path.
- Near-compaction ongoing flush can run without manual command boundaries.
- Compaction pre/post memory behavior remains correct.
- Tests cover all lifecycle triggers and error handling paths.

## Suggested Rollout

1. Release N: introduce core lifecycle, keep hook compatibility, emit deprecation warnings.
2. Release N+1: remove `session-memory` from defaults; keep optional compatibility shim.
3. Release N+2: remove shim if no user impact concerns remain.
