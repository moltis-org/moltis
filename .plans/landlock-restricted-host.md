# Implementation Plan: Landlock FS Isolation for restricted-host (Option A)

**Issue:** moltis-org/moltis#818
**Branch:** `feat/landlock-restricted-host`
**Author:** @dmitriikeler

---

## Overview

Add optional Landlock filesystem isolation to `RestrictedHostSandbox` using an allowlist model (`fs_allow_paths`). When configured, child processes spawned via `exec()` will be constrained to only access paths in the allowlist — enforced at the kernel VFS layer.

---

## Task Breakdown

### Task 1: Add `landlock` dependency + feature flag

**Files:**
- `Cargo.toml` (workspace root)
- `crates/tools/Cargo.toml`

**Changes:**

1. Add `landlock` to workspace `[workspace.dependencies]`:
   ```toml
   landlock = "0.4"
   ```

2. Add feature flag to `crates/tools/Cargo.toml`:
   ```toml
   [features]
   landlock = ["dep:landlock"]
   ```

3. Add to `[dependencies]`:
   ```toml
   landlock = { optional = true, workspace = true }
   ```

4. Add `"landlock"` to `default` features in `crates/cli/Cargo.toml` (opt-in from CLI, following the pattern in CLAUDE.md — actually, per CLAUDE.md, new features go into `crates/cli/Cargo.toml` defaults).

**Validation:** `cargo check -p moltis-tools --features landlock` compiles on Linux. `cargo check -p moltis-tools` compiles without the feature.

---

### Task 2: Add `fs_allow_paths` to config schema

**Files:**
- `crates/config/src/schema/tools.rs` — `SandboxConfig` struct (line 656)
- `crates/config/src/validate/schema_map.rs` — `sandbox()` function (line 72)

**Changes:**

1. Add field to `SandboxConfig` in `schema/tools.rs` (after line 700, before the closing `}`):
   ```rust
   /// Optional Landlock allowlist for restricted-host on Linux.
   /// Child processes can only access paths in this list.
   /// Ignored on non-Linux or non-restricted-host backends.
   /// Paths must be absolute. Symlinks are resolved at rule-add time.
   #[serde(default, skip_serializing_if = "Vec::is_empty")]
   pub fs_allow_paths: Vec<String>,
   ```

2. Add entry to `sandbox()` in `validate/schema_map.rs` (after line 91):
   ```rust
   ("fs_allow_paths", Array(Box::new(Leaf))),
   ```

3. Add to `config template.rs` after the `backend` comment block (~line 456):
   ```toml
   # fs_allow_paths = []            # Landlock FS allowlist (Linux, restricted-host only):
                                     #   Paths child processes can access. Everything else denied.
                                     #   Must be absolute paths. Empty = no Landlock restrictions.
   ```

**Validation:** `cargo check -p moltis-config`. Config with `fs_allow_paths = ["/usr", "/tmp"]` parses without warnings.

---

### Task 3: Add `fs_allow_paths` to runtime `SandboxConfig`

**Files:**
- `crates/tools/src/sandbox/types.rs`

**Changes:**

1. Add field to `SandboxConfig` struct (line 154, after `wasm_tool_limits`):
   ```rust
   /// Optional Landlock FS allowlist for restricted-host on Linux.
   pub fs_allow_paths: Vec<PathBuf>,
   ```

2. Add to `Default` impl (line 191):
   ```rust
   fs_allow_paths: Vec::new(),
   ```

3. Add mapping in `From<&moltis_config::schema::SandboxConfig>` impl (line 216, after `wasm_tool_limits` line ~270):
   ```rust
   fs_allow_paths: cfg
       .fs_allow_paths
       .iter()
       .map(|p| PathBuf::from(p.as_str()))
       .collect(),
   ```

**Validation:** `cargo check -p moltis-tools`.

---

### Task 4: Create Landlock restriction module

**New file:** `crates/tools/src/sandbox/landlock.rs`

**Contents (~90 LOC):**

```rust
//! Optional Landlock filesystem isolation for restricted-host sandbox (Linux only).

use std::path::PathBuf;

/// Result of attempting to apply Landlock restrictions.
#[derive(Debug)]
pub struct LandlockResult {
    /// Whether Landlock was successfully enforced.
    pub enforced: bool,
    /// Human-readable status message for logging.
    pub message: String,
}

/// Build and apply Landlock FS restrictions via pre_exec closure.
///
/// Returns a closure suitable for `Command::pre_exec()` that restricts the child
/// process to only access the given allowlist paths. If Landlock is unavailable
/// (old kernel, unsupported ABI), the closure is a no-op.
///
/// # Arguments
/// * `fs_allow_paths` - Absolute paths to allow access to.
///
/// # Returns
/// * `LandlockResult` indicating enforcement status (for logging in parent).
/// * `Box<dyn FnOnce() -> io::Result<()> + Send + 'static>` for `pre_exec`.
#[cfg(target_os = "linux")]
pub fn build_landlock_pre_exec(
    fs_allow_paths: &[PathBuf],
) -> (LandlockResult, Box<dyn FnOnce() -> std::io::Result<()> + Send + 'static>) {
    use landlock::{
        AccessFs, CompatLevel, PathBeneath, PathFd, Ruleset,
        RestrictionStatus, RulesetStatus,
    };

    if fs_allow_paths.is_empty() {
        return (
            LandlockResult {
                enforced: false,
                message: "fs_allow_paths empty, skipping Landlock".into(),
            },
            Box::new(|| Ok(())),
        );
    }

    // Use the highest ABI the kernel supports.
    let abi = match Ruleset::supported_abi() {
        Ok(abi) => abi,
        Err(e) => {
            return (
                LandlockResult {
                    enforced: false,
                    message: format!("Landlock ABI detection failed: {e}, skipping"),
                },
                Box::new(|| Ok(())),
            );
        }
    };

    // Request all handled access rights so we can restrict them.
    let handled = AccessFs::from_all(abi);

    match Ruleset::new()
        .set_compatibility(CompatLevel::BestEffort)
        .handle_access(handled)
        .and_then(|rs| rs.create())
    {
        Ok(mut ruleset) => {
            // Build rules from allowlist paths.
            let add_errors: Vec<String> = fs_allow_paths
                .iter()
                .filter_map(|path| {
                    PathFd::new(path).ok().and_then(|fd| {
                        ruleset
                            .add_rule(PathBeneath::new(fd, AccessFs::from_all(abi)))
                            .err()
                            .map(|e| format!("{path:?}: {e}"))
                    })
                })
                .collect();

            if !add_errors.is_empty() {
                return (
                    LandlockResult {
                        enforced: false,
                        message: format!("Landlock rule errors: {}", add_errors.join(", ")),
                    },
                    Box::new(|| Ok(())),
                );
            }

            // Move ruleset into the pre_exec closure.
            let closure: Box<dyn FnOnce() -> std::io::Result<()> + Send + 'static> =
                Box::new(move || {
                    let status = ruleset.restrict_self().map_err(|e| {
                        std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
                    })?;
                    match status {
                        RestrictionStatus {
                            ruleset: RulesetStatus::FullyEnforced,
                            ..
                        } => Ok(()),
                        RestrictionStatus {
                            ruleset: RulesetStatus::PartiallyEnforced,
                            ..
                        } => {
                            // Some rights couldn't be restricted — still better than nothing.
                            Ok(())
                        },
                        RestrictionStatus {
                            ruleset: RulesetStatus::NotEnforced,
                            ..
                        } => Err(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            "Landlock ruleset not enforced (kernel too old or no support)",
                        )),
                    }
                });

            (
                LandlockResult {
                    enforced: true,
                    message: format!(
                        "Landlock enforced (ABI {:?}, {} paths allowed)",
                        abi,
                        fs_allow_paths.len()
                    ),
                },
                closure,
            )
        },
        Err(e) => (
            LandlockResult {
                enforced: false,
                message: format!("Landlock ruleset creation failed: {e}, skipping"),
            },
            Box::new(|| Ok(())),
        ),
    }
}

/// No-op implementation for non-Linux platforms.
#[cfg(not(target_os = "linux"))]
pub fn build_landlock_pre_exec(
    _fs_allow_paths: &[PathBuf],
) -> (
    LandlockResult,
    Box<dyn FnOnce() -> std::io::Result<()> + Send + 'static>,
) {
    (
        LandlockResult {
            enforced: false,
            message: "Landlock not available on this platform".into(),
        },
        Box::new(|| Ok(())),
    )
}
```

**Register in `mod.rs`:**
- Add `pub(crate) mod landlock;` to `crates/tools/src/sandbox/mod.rs`

**Validation:** `cargo check -p moltis-tools --features landlock` on Linux. `cargo check -p moltis-tools` on all platforms.

---

### Task 5: Wire Landlock into `RestrictedHostSandbox::exec()`

**File:** `crates/tools/src/sandbox/platform.rs`

**Changes to `exec()` method (line 220):**

After line 224 (`cmd.args(["-c", &wrapped])`), before `cmd.env_clear()`:

```rust
#[cfg(all(target_os = "linux", feature = "landlock"))]
{
    let (result, pre_exec_fn) =
        super::landlock::build_landlock_pre_exec(&self.config.fs_allow_paths);
    tracing::info!(status = result.enforced, %result.message, "landlock");
    if result.enforced {
        cmd.pre_exec(pre_exec_fn);
    }
}
```

**Note:** The `#[cfg(feature = "landlock")]` guard ensures zero overhead when the feature is disabled. On non-Linux, the entire block is compiled out. On Linux without the feature, the config field exists but is unused.

**Update router log message** in `crates/tools/src/sandbox/router.rs` (line 236):
```rust
"restricted-host" => {
    let has_landlock = !config.fs_allow_paths.is_empty();
    tracing::info!(
        "sandbox backend: restricted-host (env clearing, rlimits{})",
        if has_landlock { ", landlock FS isolation" } else { "" },
    );
    Arc::new(RestrictedHostSandbox::new(config))
},
```

**Validation:** `cargo check -p moltis-tools --features landlock`. With `fs_allow_paths = ["/tmp"]` configured, `cat /etc/passwd` inside exec should fail.

---

### Task 6: Add integration tests

**File:** `crates/tools/src/sandbox/tests/restricted_host.rs`

Add tests gated behind `#[cfg(all(target_os = "linux", feature = "landlock"))]`:

1. **`test_landlock_blocks_outside_allowlist`** — Configure `fs_allow_paths = ["/tmp"]`, spawn `cat /etc/passwd`, assert exit code != 0.

2. **`test_landlock_allows_inside_allowlist`** — Configure `fs_allow_paths = ["/tmp"]`, write a file to `/tmp`, spawn `cat /tmp/testfile`, assert success.

3. **`test_landlock_empty_paths_no_restriction`** — Default config (empty `fs_allow_paths`), spawn `cat /etc/hostname`, assert success (no regression).

4. **`test_landlock_read_file_write_file_bypass`** — This is a documentation test. `read_file`/`write_file` use `native_host_*` which bypass Landlock (they run in the parent, not the child). Add a comment in the test explaining this known gap.

**Note on test reliability:** These tests must run in-process and are self-restricting. Each test creates a `RestrictedHostSandbox` with `fs_allow_paths` set, spawns a child via `exec()`. The Landlock restrictions only apply to the child — the parent (test process) is unaffected. This is safe.

However: **Landlock is one-way and per-thread.** If we apply `restrict_self()` in the test process itself, it would restrict the test runner. Since we only apply it via `pre_exec` (in the child), this is fine.

**Validation:** `cargo test -p moltis-tools --features landlock test_landlock`

---

### Task 7: Add `fs_allow_paths` default suggestions for common deployments

**File:** `crates/config/src/template.rs` (documentation only)

Update the `fs_allow_paths` comment to include common patterns:

```toml
# fs_allow_paths = []            # Landlock FS allowlist (Linux, restricted-host only):
                                     #   Child processes can only access these paths.
                                     #   Empty = no Landlock restrictions (default).
                                     #   Must be absolute paths.
                                     #   Common patterns:
                                     #     fs_allow_paths = ["/usr", "/bin", "/lib", "/tmp"]
                                     #     fs_allow_paths = ["/usr", "/bin", "/lib", "/tmp", "/workspace"]
                                     #   Symlinks are resolved at rule-add time.
```

**Validation:** Documentation only, no code validation needed.

---

## Design Decisions

### Why Option A (allowlist) only?
- Matches Landlock's kernel-native semantics (it IS an allowlist LSM)
- Simpler implementation — no need to curate a "safe default" deny set
- Clearer operator mental model: "these are the ONLY paths the sandbox can touch"
- Option B (`fs_deny_paths`) can be added in a follow-up PR if demand exists

### Why feature-gated, not cfg(target_os) only?
- Some Linux deployments may want to disable Landlock (e.g., custom kernels without LSM)
- Feature flag allows conditional compilation of the `landlock` crate dependency
- Non-Linux platforms (macOS) get a clean no-op without pulling in the crate

### Why `pre_exec` and not parent-side restriction?
- Landlock's `restrict_self()` restricts the calling thread
- Applying in the parent would permanently restrict the moltis process
- `pre_exec` runs after `fork()` but before `execve()` — perfect insertion point
- The closure must be `Send` (crosses thread boundary in tokio) — confirmed landlock types satisfy this

### Known gap: `read_file`/`write_file`/`list_files` bypass Landlock
These methods use `tokio::fs` directly in the parent process. Landlock only restricts child processes created via `exec()`. Addressing this requires a different approach (separate helper binary or per-call fork+restrict) and is out of scope for this PR. The gap should be documented in the template and code comments.

### `AccessFs::from_all(abi)` — why full access on allowed paths?
We want allowed paths to be fully usable (read, write, execute, etc.). The restriction comes from NOT listing paths — anything not in the allowlist gets no rights.

---

## Acceptance Criteria Mapping

| Criterion | Task |
|---|---|
| Configured allowlist causes `cat /data/secrets/foo` to fail | Task 5 + Task 6 |
| Empty/default config = identical behavior | Task 3 (default empty vec) + Task 6 |
| Works on kernel >= 5.13 via `best_effort` | Task 4 (`CompatLevel::BestEffort`) |
| `warn!` when Landlock can't be applied | Task 5 (tracing::info) |
| Compiles on macOS without Landlock code path | Task 4 (`#[cfg(not(target_os = "linux"))]`) |
| No regression in existing tests | Task 6 (test_landlock_empty_paths_no_restriction) |

---

## File Change Summary

| File | Change | LOC (est.) |
|---|---|---|
| `Cargo.toml` | Add workspace dep | +1 |
| `crates/tools/Cargo.toml` | Feature + dep | +3 |
| `crates/cli/Cargo.toml` | Add to defaults | +1 |
| `crates/config/src/schema/tools.rs` | New field | +7 |
| `crates/config/src/validate/schema_map.rs` | Schema entry | +1 |
| `crates/config/src/template.rs` | Documentation | +8 |
| `crates/tools/src/sandbox/types.rs` | New field + Default + From | +10 |
| `crates/tools/src/sandbox/mod.rs` | Module declaration | +1 |
| `crates/tools/src/sandbox/landlock.rs` | **New file** | +90 |
| `crates/tools/src/sandbox/platform.rs` | Wire pre_exec | +8 |
| `crates/tools/src/sandbox/router.rs` | Log message update | +4 |
| `crates/tools/src/sandbox/tests/restricted_host.rs` | Integration tests | +60 |

**Total: ~195 LOC net (90 in new module, ~105 across existing files)**

---

## Execution Order

```
Task 1 → Task 2 → Task 3 → Task 4 → Task 5 → Task 6 → Task 7
  dep       dep       dep       dep       dep       test       doc
```

Tasks 2+3 can be done in parallel. Task 4 depends on Task 1. Task 5 depends on Tasks 3+4. Task 6 depends on Task 5.
