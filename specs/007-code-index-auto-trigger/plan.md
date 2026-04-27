# Plan 007: Code Index Auto-Trigger

## Tech Stack

- **Language:** Rust (edition 2024, nightly-2025-12-27)
- **Async runtime:** Tokio
- **Logging:** `tracing` crate with `#[instrument]`
- **Feature flags:** `qmd`, `code-index-builtin`, `file-watcher` (existing)
- **Storage:** SQLite via sqlx (existing)
- **Tests:** Tokio async tests, coverage via `cargo test`

## Architecture

### New Module: `index_job_manager.rs`

Create `crates/code-index/src/index_job_manager.rs` — coordinates all indexing operations.

```
IndexJobManager
├── code_index: Arc<CodeIndex>
├── project_store: Arc<dyn ProjectStore>     // injected from gateway
├── active_jobs: Mutex<HashMap<String, Arc<Mutex<()>>>>  // per-project dedup
├── watchers: Mutex<HashMap<String, FileWatcher>>        // active watchers
├── max_concurrent: usize                   // default: 2
├── semaphore: Arc<Semaphore>               // limits concurrent indexing
└── cancel: CancellationToken               // shutdown signal
```

**Responsibilities:**
1. Queue and deduplicate indexing jobs per project
2. Start/stop file watchers on index success/failure
3. Run periodic re-index loop
4. Provide status API for active jobs

### Integration Points

#### 1. Startup Auto-Index (`prepare_core.rs`)

After `CodeIndex` initialization and project store setup, spawn a one-shot background task:

```rust
let job_manager = Arc::new(IndexJobManager::new(code_index, project_store, config));
// Spawn startup indexing in background — don't block gateway readiness
tokio::spawn({
    let jm = Arc::clone(&job_manager);
    async move { jm.index_all_enabled_projects().await }
});
```

#### 2. Project Upsert Hook (`admin.rs`)

In the `projects.upsert` handler, after the existing off→on transition logic, also trigger indexing for new projects with `code_index_enabled = true`:

```rust
// After upsert completes, trigger indexing if enabled
if new_enabled == Some(true) {
    job_manager.spawn_index(project_id, project_dir);
}
```

#### 3. Project Delete Hook (`admin.rs`)

Stop watcher and clean up when a project is deleted:

```rust
job_manager.stop_watcher(project_id);
```

#### 4. Graceful Shutdown

The `CancellationToken` on `IndexJobManager` cancels all running index jobs and watchers on gateway shutdown.

### New Config Fields

In `crates/config/src/schema/code_index.rs`:

```rust
pub struct CodeIndexTomlConfig {
    // ... existing fields ...
    /// Index all enabled projects at startup. Default: true.
    pub auto_index_on_startup: bool,
    /// Index project when created or enabled. Default: true.
    pub auto_index_on_create: bool,
    /// Periodic re-index interval (e.g. "30m", "1h"). Default: "30m".
    pub periodic_reindex_interval: String,
    /// Maximum concurrent indexing jobs. Default: 2.
    pub max_concurrent_jobs: u32,
}
```

Mirror in `crates/code-index/src/config.rs`:

```rust
pub struct CodeIndexConfig {
    // ... existing fields ...
    pub auto_index_on_startup: bool,
    pub auto_index_on_create: bool,
    pub periodic_reindex_interval: Duration,
    pub max_concurrent_jobs: usize,
}
```

### Data Flow

```
Gateway Startup
    │
    ├─► init_code_index() → CodeIndex (existing)
    ├─► load projects from ProjectStore
    ├─► Create IndexJobManager
    │
    └─► Background: IndexJobManager::index_all_enabled_projects()
              │
              ├─► For each project with code_index_enabled=true:
              │     ├─► Acquire per-project Mutex (dedup)
              │     ├─► Acquire semaphore slot (concurrency limit)
              │     ├─► code_index.index_project(id, false, dir)
              │     ├─► On success: start_watcher() if file-watcher feature
              │     └─► Release semaphore + mutex
              │
              └─► Start periodic re-index loop

User creates project (projects.upsert)
    │
    └─► IndexJobManager::spawn_index(project_id, dir)
              └─► Same flow as above (single project)

File changes detected (watcher)
    │
    └─► CodeIndex::reindex_files() (existing)

Periodic timer fires
    │
    └─► IndexJobManager::index_all_enabled_projects()
              └─► Same as startup, but uses incremental (force=false)

User disables indexing (projects.upsert)
    │
    └─► IndexJobManager::stop_watcher(project_id)
```

### File Changes Summary

| File | Change Type | Description |
|------|------------|-------------|
| `crates/code-index/src/index_job_manager.rs` | **NEW** | Job coordination, dedup, periodic loop |
| `crates/code-index/src/lib.rs` | MODIFY | Export new module |
| `crates/code-index/src/config.rs` | MODIFY | Add new config fields |
| `crates/config/src/schema/code_index.rs` | MODIFY | Add new TOML fields + defaults |
| `crates/config/src/validate.rs` | MODIFY | Register new fields in schema map |
| `crates/gateway/src/server/init_code_index.rs` | MODIFY | Return job manager alongside code index |
| `crates/gateway/src/server/prepare_core/post_state.rs` | MODIFY | Wire job manager into state, startup indexing |
| `crates/gateway/src/server/prepare_core.rs` | MODIFY | Startup auto-index spawn |
| `crates/gateway/src/methods/services/admin.rs` | MODIFY | Trigger index on project create/delete |
| `crates/gateway/src/state.rs` | MODIFY | Add job manager to state |
| Tests | **NEW** | Unit tests for IndexJobManager, integration tests |

### Error Handling

- Index failures are logged as warnings, not propagated
- Watcher failures are logged and retried on next periodic cycle
- Periodic loop uses `tokio::time::interval` with jitter to avoid thundering herd
- Semaphore prevents resource exhaustion on large project sets

### Testing Strategy

1. **Unit tests** for `IndexJobManager` with mock `CodeIndex` and `ProjectStore`
2. **Integration test** for startup auto-index with temp git repos
3. **Integration test** for project upsert trigger
4. **Test** deduplication (concurrent spawn_index for same project)
5. **Test** watcher lifecycle (start on index, stop on disable)
6. **Test** periodic re-index with configurable interval
7. **Test** config parsing for new fields
8. **Test** graceful shutdown cancels in-flight jobs
