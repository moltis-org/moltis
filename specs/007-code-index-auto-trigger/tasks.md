# Tasks 007: Code Index Auto-Trigger

## Phase 1: Foundation

### Task 1.1 — Extend CodeIndexConfig with auto-index fields
**Files:** `crates/code-index/src/config.rs`, `crates/config/src/schema/code_index.rs`, `crates/config/src/validate.rs`

Add fields:
- `auto_index_on_startup: bool` (default: `true`)
- `auto_index_on_create: bool` (default: `true`)
- `periodic_reindex_interval: Duration` (default: 30 min)
- `max_concurrent_jobs: usize` (default: 2)

TOML config gets stringly-typed equivalents (`periodic_reindex_interval: String` → parsed to `Duration`).

Update `build_schema_map()` in validate.rs.

**Tests:** Config parsing, defaults, serde round-trip.

**[P]** Independent of other tasks.

---

### Task 1.2 — Create IndexJobManager struct
**File:** `crates/code-index/src/index_job_manager.rs` (NEW)

```rust
pub struct IndexJobManager {
    code_index: Arc<CodeIndex>,
    project_dirs: Mutex<HashMap<String, PathBuf>>,
    active_jobs: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    watchers: Mutex<HashMap<String, FileWatcher>>,
    max_concurrent: usize,
    semaphore: Arc<Semaphore>,
    cancel: CancellationToken,
    config: IndexJobManagerConfig,
}
```

Methods:
- `new(code_index, config) -> Self`
- `register_project(project_id, project_dir)` — remembers project directory
- `unregister_project(project_id)` — stops watcher, removes from maps
- `spawn_index(project_id)` — deduplicated, semaphore-limited background index
- `index_all_enabled_projects(projects: Vec<(String, PathBuf)>)` — batch startup index
- `start_periodic_reindex(projects: Vec<(String, PathBuf)>) -> JoinHandle`
- `shutdown()` — cancels all jobs, stops all watchers

**Tests:** Unit tests with mock code_index. Test dedup, concurrency limit.

**Depends on:** Task 1.1

---

## Phase 2: Integration

### Task 2.1 — Wire IndexJobManager into gateway state
**Files:** `crates/gateway/src/state.rs`, `crates/gateway/src/server/prepare_core/post_state.rs`, `crates/gateway/src/server/init_code_index.rs`

- `init_code_index.rs` returns `(Arc<CodeIndex>, Arc<IndexJobManager>)` or job manager is created in `post_state.rs` after code index init.
- Add `IndexJobManager` to `GatewayState`.
- Gate behind `#[cfg(any(feature = "qmd", feature = "code-index-builtin"))]`.

**Tests:** Compilation check. Existing tests still pass.

**Depends on:** Task 1.2

---

### Task 2.2 — Startup auto-index
**File:** `crates/gateway/src/server/prepare_core.rs`

After project store is initialized and state is assembled:

```rust
#[cfg(any(feature = "qmd", feature = "code-index-builtin"))]
{
    if code_index_config.auto_index_on_startup {
        let jm = Arc::clone(&job_manager);
        let projects = project_store.list().await.unwrap_or_default();
        let enabled: Vec<(String, PathBuf)> = projects.iter()
            .filter(|p| p.code_index_enabled)
            .map(|p| (p.id.clone(), p.directory.clone()))
            .collect();
        tokio::spawn(async move {
            jm.index_all_enabled_projects(enabled).await;
            jm.start_periodic_reindex_loop(enabled).await;
        });
    }
}
```

**Tests:** Integration test with temp project store. Verify `index_project` called for enabled projects only.

**Depends on:** Task 2.1

**[P]** Independent of Task 2.3.

---

### Task 2.3 — Project upsert/delete triggers
**File:** `crates/gateway/src/methods/services/admin.rs`

Extend the existing `projects.upsert` handler:
1. After upsert, if `code_index_enabled = true`, call `job_manager.spawn_index(project_id)`
2. Register project dir with `job_manager.register_project(project_id, dir)`
3. If `code_index_enabled` transitions `true → false`, call `job_manager.stop_watcher(project_id)`

Extend `projects.delete` handler:
1. Call `job_manager.unregister_project(project_id)`

Access `IndexJobManager` via `ctx.state`.

**Tests:** Test upsert triggers index. Test delete stops watcher. Test disable stops watcher.

**Depends on:** Task 2.1

**[P]** Independent of Task 2.2.

---

## Phase 3: File Watcher Activation

### Task 3.1 — Auto-start file watcher after successful index
**File:** `crates/code-index/src/index_job_manager.rs`

In `spawn_index()`, after `index_project()` succeeds:

```rust
#[cfg(feature = "file-watcher")]
{
    if let Ok(()) = self.code_index.start_watcher(project_id, project_dir) {
        // Watcher registered
    }
}
```

Handle the `Arc<Self>` requirement for `start_watcher` by structuring `IndexJobManager` as `Arc<IndexJobManagerInner>` or using `Arc<Self>` pattern.

**Tests:** Test watcher starts after index. Test watcher doesn't start on failed index.

**Depends on:** Task 1.2

---

### Task 3.2 — Stop watcher on project disable/delete
**File:** `crates/code-index/src/index_job_manager.rs`

- `unregister_project()` calls `self.code_index.stop_watcher(project_id)`
- Expose method for RPC handler to call

**Tests:** Test watcher stops. Test cleanup on delete.

**Depends on:** Task 3.1

---

## Phase 4: Periodic Re-Index

### Task 4.1 — Periodic re-index loop
**File:** `crates/code-index/src/index_job_manager.rs`

```rust
pub async fn start_periodic_reindex_loop(self: &Arc<Self>, projects: Vec<(String, PathBuf)>) {
    let interval = self.config.periodic_reindex_interval;
    loop {
        tokio::select! {
            _ = tokio::time::sleep(interval) => {
                self.index_all_enabled_projects(projects.clone()).await;
            }
            _ = self.cancel.cancelled() => break,
        }
    }
}
```

Uses incremental indexing (`force = false`). Catches changes missed by watcher.

**Tests:** Test loop runs. Test cancellation. Test interval respected.

**Depends on:** Task 1.2

---

## Phase 5: Validation & Polish

### Task 5.1 — Graceful shutdown
**Files:** `crates/code-index/src/index_job_manager.rs`, `crates/gateway/src/server/prepare_core.rs`

Wire `IndexJobManager::shutdown()` into gateway shutdown flow.
Cancels CancellationToken → all background tasks stop → watchers stop.

**Tests:** Test in-flight jobs cancelled on shutdown.

**Depends on:** Task 2.1

---

### Task 5.2 — End-to-end integration test
**File:** `crates/code-index/tests/integration_auto_index.rs` (NEW)

Full flow:
1. Create temp git repos
2. Init CodeIndex with builtin backend
3. Create IndexJobManager
4. Call `index_all_enabled_projects()`
5. Verify indexed via `codebase_search`
6. Modify a file
7. Trigger watcher reindex
8. Verify search returns updated content
9. Call `shutdown()`
10. Verify watchers stopped

**Depends on:** All prior tasks.

---

## Dependency Graph

```
1.1 ──► 1.2 ──► 2.1 ──► 2.2 (startup auto-index)
                  │      └──► 2.3 (upsert/delete triggers)
                  │
                  ├──► 3.1 ──► 3.2 (watcher lifecycle)
                  ├──► 4.1 (periodic re-index)
                  └──► 5.1 (graceful shutdown)

All ──► 5.2 (e2e test)
```

## Execution Order

| Phase | Tasks | Parallelizable |
|-------|-------|---------------|
| 1 | 1.1, then 1.2 | Sequential (1.2 depends on 1.1) |
| 2 | 2.1, then 2.2 + 2.3 | 2.2 and 2.3 parallel after 2.1 |
| 3 | 3.1, then 3.2 | Sequential |
| 4 | 4.1 | Independent (can run with Phase 3) |
| 5 | 5.1, then 5.2 | After all prior |
