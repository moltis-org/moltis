# Tasks 007: Code Index Auto-Trigger - Implementation Status

## Status Summary

**Phase 1: Foundation** ✅ COMPLETE  (2026-04-27)
**Phase 2: Integration** 🔄 IN PROGRESS  
**Phase 3: File Watcher Activation** ⏳ PENDING  
**Phase 4: Periodic Re-Index** ⏳ PENDING  
**Phase 5: Validation & Polish** ⏳ PENDING

---

## Task Tracking

| Task | Status | Commit | Notes |
|------|--------|--------|-------|
| 1.1 Config fields | ✅ Done | 63b6cc59 | CodeIndexTomlConfig + CodeIndexConfig extended |
| 1.2 IndexJobManager | ✅ Done | 63b6cc59 | Full implementation with dedup, semaphore, watchers |
| 2.1 Wire to gateway | 🔄 In Progress | - | GatewayState field added, wiring pending |
| 2.2 Startup auto-index | ⏳ Pending | - | Depends on 2.1 |
| 2.3 Upsert/delete triggers | ⏳ Pending | - | Depends on 2.1 |
| 3.1 Watcher auto-start | ⏳ Pending | - | Implemented in IndexJobManager, needs feature gate |
| 3.2 Watcher stop on delete | ⏳ Pending | - | unregister_project() implemented |
| 4.1 Periodic loop | ⏳ Pending | - | Implemented in IndexJobManager |
| 5.1 Graceful shutdown | ⏳ Pending | - | shutdown() implemented |
| 5.2 E2E test | ⏳ Pending | - | Final validation |

---

## Remaining Work - Phase 2

### Task 2.1: Wire IndexJobManager into gateway

**Files to modify:**
1. `crates/gateway/src/server/prepare_core/post_state.rs` - Create IndexJobManager after CodeIndex init
2. `crates/gateway/src/state.rs` - ✅ Field already added
3. `crates/gateway/src/server/prepare_core.rs` - Pass config, wire into state construction

**Implementation:**
```rust
// In post_state.rs after line ~426 where code_index is available:
#[cfg(any(feature = "qmd", feature = "code-index-builtin"))]
let code_index_config_for_jm = code_index.config().clone();
#[cfg(any(feature = "qmd", feature = "code-index-builtin"))]
let job_manager_config = IndexJobManagerConfig {
    auto_index_on_startup: code_index_config_for_jm.auto_index_on_startup,
    auto_index_on_create: code_index_config_for_jm.auto_index_on_create,
    periodic_reindex_interval: code_index_config_for_jm.periodic_reindex_interval,
    max_concurrent_jobs: code_index_config_for_jm.max_concurrent_jobs,
};
#[cfg(any(feature = "qmd", feature = "code-index-builtin"))]
let job_manager = Arc::new(IndexJobManager::new(Arc::clone(&code_index), job_manager_config));
```

Then pass `job_manager` to GatewayState constructor.

### Task 2.2: Startup auto-index trigger

**File:** `crates/gateway/src/server/prepare_core.rs`

After state is assembled (near end of `prepare_core_state()`), spawn background task:

```rust
#[cfg(any(feature = "qmd", feature = "code-index-builtin"))]
{
    if config.code_index.auto_index_on_startup {
        let jm = Arc::clone(&state.index_job_manager);
        let projects_state = Arc::clone(&state.services.project);
        tokio::spawn(async move {
            let projects = projects_state.list().await.unwrap_or_default();
            let enabled: Vec<(String, PathBuf)> = projects.iter()
                .filter(|p| p.code_index_enabled)
                .map(|p| (p.id.clone(), p.directory.clone()))
                .collect();
            jm.index_all_enabled_projects(enabled.clone()).await;
            jm.start_periodic_reindex_loop(enabled);
        });
    }
}
```

### Task 2.3: Project upsert/delete triggers

**File:** `crates/gateway/src/methods/services/admin.rs`

In `projects.upsert` handler, after the existing off→on transition logic (around line 200), add:

```rust
#[cfg(any(feature = "qmd", feature = "code-index-builtin"))]
{
    // Register project with job manager and trigger index if enabled
    if new_enabled == Some(true) {
        if let Some(ref pid) = project_id {
            if let Ok(proj) = ctx.state.services.project.get(json!({ "id": pid })).await {
                ctx.state.index_job_manager
                    .register_project(pid.clone(), proj.directory.clone()).await;
                ctx.state.index_job_manager.spawn_index(pid.clone());
            }
        }
    }
}
```

In `projects.delete` handler (need to add if not present, or extend existing):

```rust
#[cfg(any(feature = "qmd", feature = "code-index-builtin"))]
{
    if let Some(pid) = project_id {
        ctx.state.index_job_manager.unregister_project(&pid).await;
    }
}
```

---

## Notes

- All IndexJobManager methods are already implemented in Phase 1
- Remaining work is purely wiring into existing gateway infrastructure
- Feature gates (`#[cfg(any(feature = "qmd", feature = "code-index-builtin"))]`) must be consistent
- No new dependencies required
