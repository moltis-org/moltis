# Spec 007: Code Index Auto-Trigger

## Problem Statement

The code indexing subsystem in Moltis has all the plumbing (discovery, filtering, chunking, storage, search, agent tools) but **no mechanism to actually trigger an initial index**. Currently:

1. `CodeIndex` is initialized at startup but no projects are indexed
2. The only trigger is a user toggling `code_index_enabled` from `false → true` via the `projects.upsert` RPC
3. If a project is created with `code_index_enabled = true` (the default), **no index is ever built**
4. The file watcher infrastructure exists but is never started for any project
5. Agent tools (`codebase_search`, `codebase_peek`, `codebase_status`) all fail or return empty because no data exists in the store

**Net effect:** Code indexing is a dead feature in practice. The entire subsystem is unreachable.

## Proposed Solution

Introduce automatic code indexing that triggers at predictable points in the project lifecycle:

### Trigger Points

1. **Startup Auto-Index:** At gateway boot, iterate all registered projects with `code_index_enabled = true` and index them (with deduplication and rate-limiting).
2. **Project Creation Auto-Index:** When a new project is created/upserted with `code_index_enabled = true`, trigger a background index.
3. **File Watcher Activation:** After a project is indexed, start the file watcher for incremental updates (if `file-watcher` feature is enabled).
4. **Watchdog Re-Index:** Periodic background re-index (configurable interval, default 30 minutes) to catch changes missed by the watcher.

### Design Constraints

- **Non-blocking:** All indexing runs in background tokio tasks. Gateway startup must not wait for indexing to complete.
- **Configurable:** New config fields in `[code_index]` section for auto-index behavior.
- **Graceful degradation:** If backend is config-only or unavailable, skip silently with a log message.
- **Rate-limited:** Only one index operation per project at a time. Concurrent requests for the same project should be deduplicated.
- **Per-project gate:** Respect the `code_index_enabled` flag on each project.

## User Stories

### US-1: Developer starts Moltis with existing projects
**As a** developer who has configured projects in Moltis,
**When** I start the Moltis gateway,
**Then** all projects with `code_index_enabled = true` are automatically indexed in the background,
**And** I can use `codebase_search` immediately after startup completes.

### US-2: Developer creates a new project via UI
**As a** developer using the web UI,
**When** I create a new project with code indexing enabled,
**Then** the project is automatically indexed in the background,
**And** I receive a WebSocket event when indexing is complete.

### US-3: File changes trigger incremental reindex
**As a** a developer actively working on a project,
**When** I save a file in my project directory,
**Then** the code index is incrementally updated within 2 seconds,
**And** subsequent searches reflect the change.

### US-4: Periodic sync catches missed changes
**As a** developer whose editor uses atomic saves or complex multi-file operations,
**When** file watcher events are occasionally missed,
**Then** a periodic background re-index catches the changes within 30 minutes.

### US-5: Developer disables code indexing
**As a** developer who wants to save resources,
**When** I disable code indexing on a project,
**Then** the file watcher for that project stops,
**And** no further automatic indexing occurs,
**And** existing indexed data is preserved.

## Acceptance Criteria

- [ ] AC-1: Gateway startup triggers background indexing for all enabled projects
- [ ] AC-2: `projects.upsert` with `code_index_enabled = true` on a new project triggers indexing
- [ ] AC-3: File watcher starts automatically after successful project index
- [ ] AC-4: File watcher stops when code indexing is disabled on a project
- [ ] AC-5: Periodic re-index runs on configurable interval (default 30 min)
- [ ] AC-6: Concurrent index requests for the same project are deduplicated (not queued)
- [ ] AC-7: Agent tools return meaningful status indicating indexing progress/state
- [ ] AC-8: All new code has tests with high coverage
- [ ] AC-9: No `unwrap()` or `expect()` in production code paths
- [ ] AC-10: Feature-gated behind existing `qmd` and `code-index-builtin` feature flags

## Out of Scope

- Changes to the chunking algorithm
- New search ranking signals
- UI changes for indexing progress indicators
- Embedding provider configuration changes
- Multi-node index synchronization

## Technical Context

### Key Files (Existing)

| File | Role |
|------|------|
| `crates/gateway/src/server/init_code_index.rs` | Initialization during startup |
| `crates/gateway/src/server/prepare_core/post_state.rs` | Tool registration, state assembly |
| `crates/gateway/src/server/prepare_core.rs` | Startup orchestration |
| `crates/gateway/src/methods/services/admin.rs` | `projects.upsert` handler |
| `crates/gateway/src/project_aware_tools.rs` | Per-project gating wrapper |
| `crates/code-index/src/index.rs` | Core `CodeIndex` struct and methods |
| `crates/code-index/src/watcher.rs` | File watcher implementation |
| `crates/code-index/src/config.rs` | Index configuration |
| `crates/projects/src/types.rs` | Project struct with `code_index_enabled` field |

### New Config Fields

```toml
[code_index]
enabled = true
# Existing fields...
auto_index_on_startup = true       # NEW: Index enabled projects at boot
auto_index_on_create = true        # NEW: Index when project is created/enabled
periodic_reindex_interval = "30m"  # NEW: Background re-index interval
```

### Architecture Decision: Index Job Manager

A new `IndexJobManager` will coordinate all indexing operations:

```
IndexJobManager
├── project_locks: HashMap<ProjectId, Mutex<()>>  // deduplication
├── code_index: Arc<CodeIndex>
├── project_store: Arc<dyn ProjectStore>
├── watchers: HashMap<ProjectId, FileWatcher>
└── periodic_handle: Option<JoinHandle>
```

This keeps the coordination logic out of the RPC handlers and the `CodeIndex` struct itself, following the existing pattern of separating orchestration from execution.

## Risks

| Risk | Mitigation |
|------|------------|
| Large monorepos slow startup | Index in background; don't block gateway readiness |
| Memory pressure from concurrent indexing | Limit concurrent indexing jobs (default: 2) |
| Watcher filesystem overhead | Only start watchers for actively indexed projects |
| Stale watchers for deleted projects | Clean up watchers on project delete |
