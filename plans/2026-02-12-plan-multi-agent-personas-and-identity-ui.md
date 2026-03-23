# Multi-Agent Support Plan

**Status:** Complete
**Priority:** High
**Complexity:** High
**Feature flag:** `agent` (enabled by default in CLI crate)
**Goal:** Support multiple agents — each with its own identity, soul, and memory — exposed in the Settings UI and bindable per-session.

## Summary

Current identity is global (`IDENTITY.md`, `SOUL.md`, `USER.md`) and prompt
assembly always loads one persona for all sessions.

This plan adds:

1. Multiple agents (create, edit, delete, set default)
2. Per-agent isolated workspaces (identity, soul, memory)
3. Session-level agent binding (each session sticks to one agent)
4. Agent switching mid-conversation (web UI selector, Telegram `/agent`)
5. Settings UI for managing agents (new **Settings > Agents** section or
   integrated into **Settings > Identity**)
6. Backward compatibility for existing `agent.identity.*` RPC methods

Follows OpenClaw's architecture where each agent is a fully scoped "brain"
with its own workspace. Default agent ID is `"main"`.

## Current State (reference)

- Identity is loaded globally in onboarding service and prompt persona loader:
  - `crates/onboarding/src/service.rs:233`
  - `crates/gateway/src/chat.rs:688`
- Identity files are global in data dir:
  - `crates/config/src/loader.rs:236`
  - `crates/config/src/loader.rs:246`
- Settings identity UI is single-agent:
  - `crates/gateway/src/assets/js/page-settings.js:230`
- Session metadata has no agent field:
  - `crates/sessions/src/metadata.rs:15`
  - `crates/sessions/migrations/20240205100001_init.sql:5`

## Storage Design

### Agent workspaces

Each agent gets an isolated directory under `data_dir()`:

```
data_dir()/
  agents/
    main/                          # default agent (migrated from root)
      IDENTITY.md
      SOUL.md
      MEMORY.md
      memory/
        YYYY-MM-DD.md              # daily logs
    ops/
      IDENTITY.md
      SOUL.md
      MEMORY.md
      memory/
        YYYY-MM-DD.md
```

Optional per-agent files (fall back to global if missing):
- `AGENTS.md`
- `TOOLS.md`

### Registry in SQLite

New `agents` table (migration in `crates/gateway/migrations/`):

```sql
CREATE TABLE IF NOT EXISTS agents (
    id          TEXT PRIMARY KEY,
    label       TEXT NOT NULL,
    is_default  BOOLEAN NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
```

- `"main"` is reserved and maps to the default agent.
- Only one row can have `is_default = 1` at a time.
- SQLite handles concurrency — no flat-file race conditions.

## Per-Agent Memory

Following OpenClaw's model, memory is **per-agent**:

- Each agent has its own `MEMORY.md` in its workspace
- Each agent has its own `memory/` directory for daily logs
- Memory search indexes are per-agent (keyed by `agent_id`)
- When an agent is created, it starts with empty memory
- When an agent is deleted, its memory is archived (not destroyed)

Rationale: different agents serve different roles (support bot vs. ops bot
vs. personal assistant). Mixing their memories would pollute context.

## Session Binding Model

Add `agent_id` to session metadata so agent is session-sticky.

### Migration

New migration in `crates/sessions/migrations/`:

```sql
ALTER TABLE sessions ADD COLUMN agent_id TEXT;
```

Nulls resolve as `"main"` in the service read path (no backfill needed).

Extend session entry and SQLite row mapping:
- `crates/sessions/src/metadata.rs`

### Behavior

- **New session** inherits `agent_id` from the session the user was viewing
  when they clicked "New". If no session context, uses the registry default
  (`"main"`).
- **Existing session** always uses its stored `agent_id`.
- **Forked session** inherits parent `agent_id`.
- **Telegram `/new`** creates a new session with the **same agent** as the
  current session.

### Mid-conversation agent switching

Both web and Telegram support switching a session's agent:

- **Web UI**: A selector (dropdown or card picker) at the top of the chat
  lets the user reassign the current session to a different agent. The
  conversation history stays, but the next turn uses the new agent's system
  prompt (SOUL.md, IDENTITY.md).
- **Telegram `/agent`**: Shows a list of available agents. Selecting one
  updates the current session's `agent_id`. The new agent gets the full
  conversation context with its own system prompt.

This is a "handoff" — the session continues, only the persona changes.

## Prompt Resolution Changes

Replace global persona load with session-aware agent load:

- Current: `load_prompt_persona()` in `crates/gateway/src/chat.rs:688`
- Target: `load_prompt_for_agent(agent_id)`

Resolution order for determining `agent_id`:

1. Session metadata `agent_id`
2. Registry default (`"main"`)

Per-agent loading:

- Load `IDENTITY.md`, `SOUL.md`, `MEMORY.md` from
  `data_dir()/agents/<agent_id>/`
- For `AGENTS.md`, `TOOLS.md`: load from agent workspace if present,
  otherwise fall back to global files

This keeps prompts functional even with partially configured agents.

## API / RPC Plan

### New methods

Following OpenClaw's naming convention:

- `agents.list` → `{ default_id, agents: [{ id, label, is_default }] }`
- `agents.get` → `{ id, label, is_default, identity, soul }`
- `agents.create` → `{ id, label, emoji?, soul? }`
- `agents.update` → `{ agent_id, label?, identity?, soul? }`
- `agents.delete` → `{ agent_id }` (cannot delete default)
- `agents.set_default` → `{ agent_id }`
- `agents.files.list` → list workspace files for an agent
- `agents.files.get` → read a specific workspace file
- `agents.files.set` → write a specific workspace file

### Backward compatibility

Keep existing methods operating on the session's current agent:

- `agent.identity.get` → reads from session's `agent_id`
- `agent.identity.update` → writes to session's `agent_id`
- `agent.identity.update_soul` → writes to session's `agent_id`

Payload shapes unchanged.

### Scope/auth updates

Update `READ_METHODS` / `WRITE_METHODS` in `crates/gateway/src/methods.rs`.
Gate all new methods behind `#[cfg(feature = "agent")]`.

## Settings UI Plan

Two pages work together:

### Settings > Identity (existing, unchanged for main agent)

The default `"main"` agent's identity is still edited here. This page
works exactly as it does today — same fields, same RPC calls. Users who
never create additional agents see no difference.

### Settings > Agents (new section)

A dedicated page for managing non-default agents. This is where users
create, configure, and delete additional agents.

### UX structure (Settings > Agents)

1. **Agent list panel** (left or top)
   - Cards for each agent (using the selection card UI pattern)
   - Badges: `Default` on main (non-deletable, links to Settings > Identity)
   - Actions: create, rename, delete (non-default), set default
2. **Agent editor panel** (right or bottom)
   - Identity fields: name, emoji, creature, vibe
   - Soul editor (textarea)
   - Memory summary (link to memory files)

### Behavior

- Selecting an agent in the list loads its identity/soul into the editor
- Deleting the current session's agent auto-switches session to default
- Default agent cannot be deleted
- Header title/emoji updates to reflect current session's agent

### Files likely touched

- `crates/gateway/src/assets/js/page-settings.js` (or new `page-agents.js`)
- `crates/gateway/src/server.rs` (`GonData` additions)
- `crates/gateway/src/assets/js/app.js` (navigation, header)
- `crates/gateway/src/assets/js/sessions.js` (agent indicator per session)
- `crates/gateway/src/assets/index.html` (new nav item if Option A)

## Service Layer Changes

### New agent store module

Add an agent service (likely in a new `crates/agents/` crate or in gateway):

- CRUD operations on `agents` SQLite table
- Workspace directory management (create/delete directories)
- Agent ID validation (slug-safe, no path traversal)
- Resolve workspace paths: `data_dir()/agents/<agent_id>/`

### Onboarding service

Extend `LiveOnboardingService` and gateway wrapper:

- Agent-aware identity get/update operations
- Keep non-agent methods as wrappers operating on current agent

Primary files:
- `crates/onboarding/src/service.rs`
- `crates/gateway/src/onboarding.rs`

## Migration and Compatibility

### Zero-break migration

On first run with new code:

1. Create `agents` table with one row: `id = "main"`, `is_default = 1`
2. Create `data_dir()/agents/main/` directory
3. **Copy** (not move) existing root `IDENTITY.md`, `SOUL.md`, `MEMORY.md`,
   and `memory/` into `data_dir()/agents/main/`
4. Keep root files as read-only fallback for one major version

### Existing sessions

- Sessions with null `agent_id` resolve as `"main"` without blocking.
- No backfill needed — the read path handles nulls.

## Edge Cases

- **Invalid agent_id in session**: fall back to `"main"`, emit `warn!` log
- **Deleted agent referenced by session**: fall back to `"main"` at runtime,
  optionally patch session on next write
- **Channel sessions**: first session uses default agent; existing sessions
  keep their stored `agent_id`
- **Cron**: add optional `agent_id` to cron payload in follow-up PR,
  default to `"main"` for now
- **Memory isolation**: memory search only searches within the current
  agent's workspace, never cross-agent

## Testing Plan

### Rust tests

- Agent store: CRUD, default switching, ID validation
- Workspace paths: directory creation, file resolution, fallback
- Session metadata: `agent_id` read/write, fork inheritance, null handling
- Prompt resolution: correct files loaded per agent
- Memory isolation: search scoped to agent workspace
- Methods auth: scope checks for `agents.*`

### E2E tests (required for UI changes)

Add Playwright coverage in `crates/gateway/ui/e2e/specs/`:

- Create agent in settings
- Edit soul/identity for selected agent
- Switch session agent and verify agent stickiness
- Delete non-default agent and fallback behavior
- Agent indicator visible in session list
- Ensure `watchPageErrors` stays empty

## Rollout Plan

Single PR, gated behind `#[cfg(feature = "agent")]`:

- `agents` SQLite table + workspace directory structure
- Session `agent_id` column migration
- Feature flag `agent` in workspace, enabled by default in CLI
- `agents.*` RPC methods + backward-compatible `agent.identity.*` wrappers
- Agent-aware prompt loading
- Settings > Agents page (new) + session agent selector in chat
- Telegram `/agent` command + channel support
- E2E tests for all UI changes
- Docs update (`docs/src/`, `README.md`, `CHANGELOG.md`)

## Resolved Decisions

1. **UI placement**: Settings > Agents (new page) for additional agents.
   Settings > Identity stays as-is for the default `"main"` agent.
2. **`USER.md`**: Stays **global** — Moltis is single-user, `USER.md`
   represents the human, not the agent.
3. **Root file cleanup**: Open — decide in a future version whether to
   deprecate root `IDENTITY.md` / `SOUL.md` in favor of
   `data_dir()/agents/main/`.
