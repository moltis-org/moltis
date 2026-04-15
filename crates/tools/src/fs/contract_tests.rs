#![allow(clippy::unwrap_used, clippy::expect_used)]
//! End-to-end contract tests that drive each fs tool through
//! `ToolRegistry::register` + `AgentTool::execute`, mirroring the
//! gateway's actual call path. These catch registration regressions
//! and schema drift that the per-module unit tests can miss (they
//! bypass trait-object dispatch by calling impl methods directly).

use {
    super::*,
    crate::{
        approval::{ApprovalDecision, ApprovalManager},
        exec::{ApprovalBroadcaster, ExecOpts, ExecResult},
        sandbox::{Sandbox, SandboxConfig, SandboxId, SandboxRouter, types::BuildImageResult},
    },
    async_trait::async_trait,
    serde_json::json,
    std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

struct TestBroadcaster {
    called: AtomicBool,
}

impl TestBroadcaster {
    fn new() -> Self {
        Self {
            called: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl ApprovalBroadcaster for TestBroadcaster {
    async fn broadcast_request(
        &self,
        _request_id: &str,
        _command: &str,
        _session_key: Option<&str>,
    ) -> crate::Result<()> {
        self.called.store(true, Ordering::SeqCst);
        Ok(())
    }
}

struct ConcurrentExecProbeSandbox {
    active_execs: AtomicUsize,
    max_active_execs: AtomicUsize,
    exec_calls: AtomicUsize,
    first_exec_started: tokio::sync::Notify,
}

impl ConcurrentExecProbeSandbox {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            active_execs: AtomicUsize::new(0),
            max_active_execs: AtomicUsize::new(0),
            exec_calls: AtomicUsize::new(0),
            first_exec_started: tokio::sync::Notify::new(),
        })
    }

    async fn wait_for_first_exec(&self) {
        if self.exec_calls.load(Ordering::SeqCst) > 0 {
            return;
        }
        self.first_exec_started.notified().await;
    }

    fn max_active_execs(&self) -> usize {
        self.max_active_execs.load(Ordering::SeqCst)
    }

    fn exec_calls(&self) -> usize {
        self.exec_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Sandbox for ConcurrentExecProbeSandbox {
    fn backend_name(&self) -> &'static str {
        "probe"
    }

    async fn ensure_ready(
        &self,
        _id: &SandboxId,
        _image_override: Option<&str>,
    ) -> crate::Result<()> {
        Ok(())
    }

    async fn exec(
        &self,
        _id: &SandboxId,
        _command: &str,
        _opts: &ExecOpts,
    ) -> crate::Result<ExecResult> {
        let call_index = self.exec_calls.fetch_add(1, Ordering::SeqCst);
        if call_index == 0 {
            self.first_exec_started.notify_waiters();
        }

        let active = self.active_execs.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active_execs.fetch_max(active, Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        self.active_execs.fetch_sub(1, Ordering::SeqCst);

        Ok(ExecResult {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
        })
    }

    async fn cleanup(&self, _id: &SandboxId) -> crate::Result<()> {
        Ok(())
    }

    async fn build_image(
        &self,
        _base: &str,
        _packages: &[String],
    ) -> crate::Result<Option<BuildImageResult>> {
        Ok(None)
    }
}

fn build_registry(workspace_root: Option<PathBuf>) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    register_fs_tools(&mut registry, FsToolsContext {
        workspace_root,
        ..FsToolsContext::default()
    });
    registry
}

fn build_registry_with_state(fs_state: FsState) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    register_fs_tools(&mut registry, FsToolsContext {
        fs_state: Some(fs_state),
        ..FsToolsContext::default()
    });
    registry
}

fn build_registry_with_policy(policy: FsPathPolicy) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    register_fs_tools(&mut registry, FsToolsContext {
        path_policy: Some(policy),
        ..FsToolsContext::default()
    });
    registry
}

#[test]
fn register_fs_tools_adds_all_six_names() {
    let registry = build_registry(None);
    let names = registry.list_names();
    for expected in FS_TOOL_NAMES {
        assert!(
            names.iter().any(|n| n == expected),
            "missing tool: {expected}. Got: {names:?}"
        );
    }
}

#[test]
fn each_tool_has_a_parameters_schema_with_pattern_or_file_path() {
    let registry = build_registry(None);
    for name in FS_TOOL_NAMES {
        let tool = registry.get(name).unwrap();
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object", "{name} schema must be an object");
        let props = schema["properties"].as_object().expect("properties");
        let has_key = props.contains_key("file_path") || props.contains_key("pattern");
        assert!(has_key, "{name} must declare file_path or pattern");
    }
}

#[tokio::test]
async fn read_write_edit_multi_edit_via_registry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("contract.txt");
    let path_str = path.to_str().unwrap().to_string();
    let registry = build_registry(None);

    // Write via the registry.
    let write = registry.get("Write").unwrap();
    let w = write
        .execute(json!({ "file_path": &path_str, "content": "alpha beta gamma" }))
        .await
        .unwrap();
    assert_eq!(w["bytes_written"], 16);

    // Read back.
    let read = registry.get("Read").unwrap();
    let r = read
        .execute(json!({ "file_path": &path_str }))
        .await
        .unwrap();
    assert_eq!(r["kind"], "text");
    assert!(r["content"].as_str().unwrap().contains("alpha"));

    // Edit — unique replacement.
    let edit = registry.get("Edit").unwrap();
    let e = edit
        .execute(json!({
            "file_path": &path_str,
            "old_string": "beta",
            "new_string": "BETA",
        }))
        .await
        .unwrap();
    assert_eq!(e["replacements"], 1);

    // MultiEdit — sequential edits.
    let multi = registry.get("MultiEdit").unwrap();
    let m = multi
        .execute(json!({
            "file_path": &path_str,
            "edits": [
                { "old_string": "alpha", "new_string": "ALPHA" },
                { "old_string": "gamma", "new_string": "GAMMA" }
            ]
        }))
        .await
        .unwrap();
    assert_eq!(m["edits_applied"], 2);

    // Final state.
    let final_read = read
        .execute(json!({ "file_path": &path_str }))
        .await
        .unwrap();
    assert!(
        final_read["content"]
            .as_str()
            .unwrap()
            .contains("ALPHA BETA GAMMA")
    );
}

#[tokio::test]
async fn glob_and_grep_via_registry_with_workspace_root() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(dir.path().join("one.rs"), "fn alpha() {}")
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("two.rs"), "fn beta() {}")
        .await
        .unwrap();

    let registry = build_registry(Some(dir.path().to_path_buf()));

    let glob = registry.get("Glob").unwrap();
    let g = glob.execute(json!({ "pattern": "*.rs" })).await.unwrap();
    let paths = g["paths"].as_array().unwrap();
    assert_eq!(paths.len(), 2);

    let grep = registry.get("Grep").unwrap();
    let gr = grep
        .execute(json!({ "pattern": "alpha", "output_mode": "content", "-n": true }))
        .await
        .unwrap();
    let matches = gr["matches"].as_array().unwrap();
    assert!(!matches.is_empty());
}

#[tokio::test]
async fn read_via_registry_auto_pages_when_limit_is_omitted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.txt");
    let mut content = String::new();
    for i in 1..=3_000 {
        content.push_str(&format!("line {i}\n"));
    }
    tokio::fs::write(&path, content).await.unwrap();

    let registry = build_registry(None);
    let read = registry.get("Read").unwrap();
    let value = read
        .execute(json!({ "file_path": path.to_str().unwrap() }))
        .await
        .unwrap();

    assert_eq!(value["kind"], "text");
    assert_eq!(value["total_lines"], 3_000);
    assert_eq!(value["rendered_lines"], 3_000);
    assert_eq!(value["truncated"], false);
    let body = value["content"].as_str().unwrap();
    assert!(body.contains("line 1"));
    assert!(body.contains("line 2001"));
    assert!(body.contains("line 3000"));
}

#[tokio::test]
async fn must_read_before_write_rejects_unread_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secret.txt");
    tokio::fs::write(&path, "original").await.unwrap();

    let state = new_fs_state(true);
    let registry = build_registry_with_state(state);

    let write = registry.get("Write").unwrap();
    let value = write
        .execute(json!({
            "file_path": path.to_str().unwrap(),
            "content": "overwritten",
            "_session_key": "s1",
        }))
        .await
        .unwrap();

    assert_eq!(value["kind"], "must_read_before_write");
    // File must be unchanged.
    let contents = tokio::fs::read_to_string(&path).await.unwrap();
    assert_eq!(contents, "original");
}

#[tokio::test]
async fn must_read_before_write_allows_after_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ok.txt");
    tokio::fs::write(&path, "original").await.unwrap();

    let state = new_fs_state(true);
    let registry = build_registry_with_state(state);

    // Read first, same session_key.
    let read = registry.get("Read").unwrap();
    let r = read
        .execute(json!({
            "file_path": path.to_str().unwrap(),
            "_session_key": "s1",
        }))
        .await
        .unwrap();
    assert_eq!(r["kind"], "text");

    // Now Write succeeds.
    let write = registry.get("Write").unwrap();
    let w = write
        .execute(json!({
            "file_path": path.to_str().unwrap(),
            "content": "overwritten",
            "_session_key": "s1",
        }))
        .await
        .unwrap();
    assert_eq!(w["bytes_written"], 11);
}

#[tokio::test]
async fn must_read_before_write_allows_new_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("new.txt");

    let state = new_fs_state(true);
    let registry = build_registry_with_state(state);

    let write = registry.get("Write").unwrap();
    let value = write
        .execute(json!({
            "file_path": path.to_str().unwrap(),
            "content": "hello",
            "_session_key": "s1",
        }))
        .await
        .unwrap();
    // New file bypasses the check — nothing to have read yet.
    assert_eq!(value["bytes_written"], 5);
}

#[tokio::test]
async fn re_read_loop_detection_fires_warning() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hot.txt");
    tokio::fs::write(&path, "content").await.unwrap();

    let state = new_fs_state(false);
    let registry = build_registry_with_state(state);
    let read = registry.get("Read").unwrap();

    for _ in 0..2 {
        let v = read
            .execute(json!({
                "file_path": path.to_str().unwrap(),
                "_session_key": "s1",
            }))
            .await
            .unwrap();
        assert!(v.get("loop_warning").is_none(), "warning too early: {v:?}");
    }

    let third = read
        .execute(json!({
            "file_path": path.to_str().unwrap(),
            "_session_key": "s1",
        }))
        .await
        .unwrap();
    assert!(
        third.get("loop_warning").is_some(),
        "warning missing: {third:?}"
    );
}

#[tokio::test]
async fn re_read_loop_detection_fires_warning_for_auto_paged_reads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("long.txt");
    let long_line = "x".repeat(70_000);
    tokio::fs::write(&path, format!("{long_line}\n"))
        .await
        .unwrap();

    let state = new_fs_state(false);
    let registry = build_registry_with_state(state);
    let read = registry.get("Read").unwrap();

    for _ in 0..2 {
        let value = read
            .execute(json!({
                "file_path": path.to_str().unwrap(),
                "_session_key": "s1",
            }))
            .await
            .unwrap();
        assert!(
            value.get("loop_warning").is_none(),
            "warning too early on auto-paged read: {value:?}"
        );
    }

    let third = read
        .execute(json!({
            "file_path": path.to_str().unwrap(),
            "_session_key": "s1",
        }))
        .await
        .unwrap();
    assert!(
        third.get("loop_warning").is_some(),
        "warning missing on auto-paged read: {third:?}"
    );
}

#[tokio::test]
async fn path_policy_denies_read_and_write() {
    let dir = tempfile::tempdir().unwrap();
    let real = tokio::fs::canonicalize(dir.path()).await.unwrap();
    let secret = real.join("secret.txt");
    let public = real.join("public.txt");
    tokio::fs::write(&secret, "top secret").await.unwrap();
    tokio::fs::write(&public, "open").await.unwrap();

    // Deny the secret file specifically.
    let policy = FsPathPolicy::new(&[], &[secret.to_string_lossy().into_owned()]).unwrap();
    let registry = build_registry_with_policy(policy);

    // Read on the denied path returns a typed path_denied payload.
    let read = registry.get("Read").unwrap();
    let r = read
        .execute(json!({ "file_path": secret.to_str().unwrap() }))
        .await
        .unwrap();
    assert_eq!(r["kind"], "path_denied");

    // Write on the denied path is rejected (file unchanged).
    let write = registry.get("Write").unwrap();
    let w = write
        .execute(json!({
            "file_path": secret.to_str().unwrap(),
            "content": "overwritten",
        }))
        .await
        .unwrap();
    assert_eq!(w["kind"], "path_denied");
    let contents = tokio::fs::read_to_string(&secret).await.unwrap();
    assert_eq!(contents, "top secret");

    // The non-denied file still works.
    let r2 = read
        .execute(json!({ "file_path": public.to_str().unwrap() }))
        .await
        .unwrap();
    assert_eq!(r2["kind"], "text");
}

#[tokio::test]
async fn path_policy_allow_list_filters_glob_results() {
    let dir = tempfile::tempdir().unwrap();
    let real = tokio::fs::canonicalize(dir.path()).await.unwrap();
    let allowed = real.join("allowed.rs");
    let blocked = real.join("blocked.rs");
    tokio::fs::write(&allowed, "// a").await.unwrap();
    tokio::fs::write(&blocked, "// b").await.unwrap();

    // Allow only *.rs files whose stem starts with "allowed".
    let allow_glob = real.join("allowed*.rs").to_string_lossy().into_owned();
    let policy = FsPathPolicy::new(&[allow_glob], &[]).unwrap();

    let mut registry = ToolRegistry::new();
    register_fs_tools(&mut registry, FsToolsContext {
        workspace_root: Some(real.clone()),
        path_policy: Some(policy),
        ..FsToolsContext::default()
    });

    let glob_tool = registry.get("Glob").unwrap();
    let value = glob_tool
        .execute(json!({ "pattern": "*.rs" }))
        .await
        .unwrap();
    let paths: Vec<String> = value["paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(paths.len(), 1);
    assert!(paths[0].ends_with("allowed.rs"));
}

#[tokio::test]
async fn checkpoint_before_mutation_returns_id_and_backs_up_file() {
    use {crate::checkpoints::CheckpointManager, std::sync::Arc};

    let dir = tempfile::tempdir().unwrap();
    let real = tokio::fs::canonicalize(dir.path()).await.unwrap();
    let target = real.join("important.txt");
    tokio::fs::write(&target, "original state").await.unwrap();

    let checkpoint_dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(CheckpointManager::new(checkpoint_dir.path().to_path_buf()));

    let mut registry = ToolRegistry::new();
    register_fs_tools(&mut registry, FsToolsContext {
        checkpoint_manager: Some(manager.clone()),
        ..FsToolsContext::default()
    });

    // Write should checkpoint before overwriting.
    let write = registry.get("Write").unwrap();
    let value = write
        .execute(json!({
            "file_path": target.to_str().unwrap(),
            "content": "new state",
        }))
        .await
        .unwrap();

    let checkpoint_id = value["checkpoint_id"].as_str().unwrap();
    assert!(!checkpoint_id.is_empty());

    // The file is now overwritten.
    let contents = tokio::fs::read_to_string(&target).await.unwrap();
    assert_eq!(contents, "new state");

    // Restore from the checkpoint and verify the original state is back.
    manager.restore(checkpoint_id).await.unwrap();
    let restored = tokio::fs::read_to_string(&target).await.unwrap();
    assert_eq!(restored, "original state");
}

#[tokio::test]
async fn write_new_file_skips_checkpoint() {
    use {crate::checkpoints::CheckpointManager, std::sync::Arc};

    let dir = tempfile::tempdir().unwrap();
    let real = tokio::fs::canonicalize(dir.path()).await.unwrap();
    let target = real.join("brand-new.txt");

    let checkpoint_dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(CheckpointManager::new(checkpoint_dir.path().to_path_buf()));

    let mut registry = ToolRegistry::new();
    register_fs_tools(&mut registry, FsToolsContext {
        checkpoint_manager: Some(manager),
        ..FsToolsContext::default()
    });

    let write = registry.get("Write").unwrap();
    let value = write
        .execute(json!({
            "file_path": target.to_str().unwrap(),
            "content": "hello",
        }))
        .await
        .unwrap();

    // No checkpoint for new files — nothing to back up.
    assert!(value["checkpoint_id"].is_null());
}

#[tokio::test]
async fn sandbox_read_via_registry_round_trips_through_bridge() {
    use {
        crate::{
            exec::ExecResult,
            fs::sandbox_bridge::test_helpers::MockSandbox,
            sandbox::{Sandbox, SandboxConfig, SandboxRouter},
        },
        base64::{Engine as _, engine::general_purpose::STANDARD as BASE64},
        std::sync::Arc,
    };

    // Pre-program a successful base64 response for the sandbox read.
    let content = b"hello from inside the sandbox";
    let mock = MockSandbox::new(vec![ExecResult {
        stdout: BASE64.encode(content),
        stderr: String::new(),
        exit_code: 0,
    }]);
    let backend: Arc<dyn Sandbox> = mock;
    let router = Arc::new(SandboxRouter::with_backend(
        SandboxConfig::default(),
        backend,
    ));
    router.set_override("sandboxed", true).await;

    let mut registry = ToolRegistry::new();
    register_fs_tools(&mut registry, FsToolsContext {
        sandbox_router: Some(router),
        ..FsToolsContext::default()
    });

    let read = registry.get("Read").unwrap();
    let value = read
        .execute(json!({
            "file_path": "/data/hello.txt",
            "_session_key": "sandboxed",
        }))
        .await
        .unwrap();

    assert_eq!(value["kind"], "text");
    assert!(
        value["content"]
            .as_str()
            .unwrap()
            .contains("hello from inside the sandbox")
    );
}

#[tokio::test]
async fn sandbox_read_routes_to_host_when_session_not_sandboxed() {
    use {
        crate::{
            fs::sandbox_bridge::test_helpers::MockSandbox,
            sandbox::{Sandbox, SandboxConfig, SandboxMode, SandboxRouter},
        },
        std::sync::Arc,
    };

    // Mode = Off and no override → not sandboxed; mock never called.
    let mock = MockSandbox::new(vec![]);
    let backend: Arc<dyn Sandbox> = mock.clone();
    let cfg = SandboxConfig {
        mode: SandboxMode::Off,
        ..SandboxConfig::default()
    };
    let router = Arc::new(SandboxRouter::with_backend(cfg, backend));

    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("on_host.txt");
    tokio::fs::write(&target, "hosted content").await.unwrap();

    let mut registry = ToolRegistry::new();
    register_fs_tools(&mut registry, FsToolsContext {
        sandbox_router: Some(router),
        ..FsToolsContext::default()
    });

    let read = registry.get("Read").unwrap();
    let value = read
        .execute(json!({
            "file_path": target.to_str().unwrap(),
            "_session_key": "not-sandboxed",
        }))
        .await
        .unwrap();

    assert_eq!(value["kind"], "text");
    assert!(
        value["content"]
            .as_str()
            .unwrap()
            .contains("hosted content")
    );
    assert!(
        mock.last_command().is_none(),
        "mock sandbox should not be called for non-sandboxed sessions"
    );
}

#[tokio::test]
async fn adaptive_read_cap_shrinks_output_for_small_context_window() {
    // Build a file with enough lines to blow any small adaptive cap.
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("big.txt");
    let line = "x".repeat(100);
    let content: String = std::iter::repeat_n(line.as_str(), 2000)
        .collect::<Vec<&str>>()
        .join("\n");
    tokio::fs::write(&target, &content).await.unwrap();

    // Tight context window: 8K tokens → clamps to MIN_ADAPTIVE_READ_BYTES (50 KB).
    let mut registry = ToolRegistry::new();
    register_fs_tools(&mut registry, FsToolsContext {
        context_window_tokens: Some(8_000),
        ..FsToolsContext::default()
    });

    let read = registry.get("Read").unwrap();
    let value = read
        .execute(json!({ "file_path": target.to_str().unwrap() }))
        .await
        .unwrap();

    assert_eq!(value["kind"], "text");
    assert_eq!(value["truncated"], true);
    let next_offset = value["next_offset"]
        .as_u64()
        .expect("truncated registry read should advertise next_offset");
    assert!(next_offset > 1);
    assert_eq!(
        value["continuation_hint"],
        format!("File output was truncated. Re-run Read with offset={next_offset} to continue.")
    );
    let content_str = value["content"].as_str().unwrap();
    assert!(
        content_str.len() <= 50 * 1024,
        "output {} exceeded 50 KB floor",
        content_str.len()
    );
    // Must render *some* lines — small cap should still be usable.
    assert!(value["rendered_lines"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn sandbox_grep_via_registry_dispatches_through_bridge() {
    use {
        crate::{
            exec::ExecResult,
            fs::sandbox_bridge::test_helpers::MockSandbox,
            sandbox::{Sandbox, SandboxConfig, SandboxRouter},
        },
        std::sync::Arc,
    };

    let mock = MockSandbox::new(vec![ExecResult {
        stdout: "/data/lib.rs:3:fn alpha()\n/data/lib.rs:9:fn beta()\n".to_string(),
        stderr: String::new(),
        exit_code: 0,
    }]);
    let backend: Arc<dyn Sandbox> = mock.clone();
    let router = Arc::new(SandboxRouter::with_backend(
        SandboxConfig::default(),
        backend,
    ));
    router.set_override("sandboxed", true).await;

    let mut registry = ToolRegistry::new();
    register_fs_tools(&mut registry, FsToolsContext {
        sandbox_router: Some(router),
        ..FsToolsContext::default()
    });

    let grep = registry.get("Grep").unwrap();
    let value = grep
        .execute(json!({
            "pattern": "fn",
            "path": "/data",
            "output_mode": "content",
            "type": "rust",
            "_session_key": "sandboxed",
        }))
        .await
        .unwrap();

    assert_eq!(value["mode"], "content");
    let matches = value["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0]["line"], 3);
    let cmd = mock.last_command().unwrap();
    assert!(cmd.contains("-n"));
    // `type=rust` → `--include='*.rs'`.
    assert!(cmd.contains("--include="));
}

#[tokio::test]
async fn sandbox_grep_type_filter_expands_multi_extension_languages() {
    use {
        crate::{
            exec::ExecResult,
            fs::sandbox_bridge::test_helpers::MockSandbox,
            sandbox::{Sandbox, SandboxConfig, SandboxRouter},
        },
        std::sync::Arc,
    };

    let mock = MockSandbox::new(vec![ExecResult {
        stdout: "/data/app.ts:3:const x = 1\n".to_string(),
        stderr: String::new(),
        exit_code: 0,
    }]);
    let backend: Arc<dyn Sandbox> = mock.clone();
    let router = Arc::new(SandboxRouter::with_backend(
        SandboxConfig::default(),
        backend,
    ));
    router.set_override("sandboxed", true).await;

    let mut registry = ToolRegistry::new();
    register_fs_tools(&mut registry, FsToolsContext {
        sandbox_router: Some(router),
        ..FsToolsContext::default()
    });

    let grep = registry.get("Grep").unwrap();
    let _ = grep
        .execute(json!({
            "pattern": "const",
            "path": "/data",
            "output_mode": "content",
            "type": "ts",
            "_session_key": "sandboxed",
        }))
        .await
        .unwrap();

    let cmd = mock.last_command().unwrap();
    assert!(cmd.contains("--include='*.ts'"));
    assert!(cmd.contains("--include='*.tsx'"));
}

#[tokio::test]
async fn sandbox_write_via_registry_sends_base64_to_bridge() {
    use {
        crate::{
            exec::ExecResult,
            fs::sandbox_bridge::test_helpers::MockSandbox,
            sandbox::{Sandbox, SandboxConfig, SandboxRouter},
        },
        base64::{Engine as _, engine::general_purpose::STANDARD as BASE64},
        std::sync::Arc,
    };

    let mock = MockSandbox::new(vec![ExecResult {
        stdout: String::new(),
        stderr: String::new(),
        exit_code: 0,
    }]);
    let backend: Arc<dyn Sandbox> = mock.clone();
    let router = Arc::new(SandboxRouter::with_backend(
        SandboxConfig::default(),
        backend,
    ));
    router.set_override("sandboxed", true).await;

    let mut registry = ToolRegistry::new();
    register_fs_tools(&mut registry, FsToolsContext {
        sandbox_router: Some(router),
        ..FsToolsContext::default()
    });

    let write = registry.get("Write").unwrap();
    let value = write
        .execute(json!({
            "file_path": "/data/out.txt",
            "content": "sandboxed write",
            "_session_key": "sandboxed",
        }))
        .await
        .unwrap();

    assert_eq!(value["bytes_written"], 15);
    let cmd = mock.last_command().unwrap();
    assert!(cmd.contains("/data/out.txt"));
    assert!(cmd.contains(&BASE64.encode(b"sandboxed write")));
}

#[tokio::test]
async fn sandbox_write_serializes_same_file_mutations_via_registry() {
    let probe = ConcurrentExecProbeSandbox::new();
    let backend: Arc<dyn Sandbox> = probe.clone();
    let router = Arc::new(SandboxRouter::with_backend(
        SandboxConfig::default(),
        backend,
    ));
    router.set_override("sandboxed", true).await;

    let mut registry = ToolRegistry::new();
    register_fs_tools(&mut registry, FsToolsContext {
        sandbox_router: Some(router),
        ..FsToolsContext::default()
    });

    let write = registry.get("Write").unwrap();
    let first = {
        let write = Arc::clone(&write);
        tokio::spawn(async move {
            write
                .execute(json!({
                    "file_path": "/data/out.txt",
                    "content": "first",
                    "_session_key": "sandboxed",
                }))
                .await
        })
    };

    probe.wait_for_first_exec().await;

    let second = {
        let write = Arc::clone(&write);
        tokio::spawn(async move {
            write
                .execute(json!({
                    "file_path": "/data/out.txt",
                    "content": "second",
                    "_session_key": "sandboxed",
                }))
                .await
        })
    };

    let (first, second) = tokio::join!(first, second);
    let first = first.unwrap().unwrap();
    let second = second.unwrap().unwrap();

    assert_eq!(first["bytes_written"], 5);
    assert_eq!(second["bytes_written"], 6);
    assert_eq!(probe.exec_calls(), 2);
    assert_eq!(
        probe.max_active_execs(),
        1,
        "same-path sandbox mutations should not overlap at the backend exec layer"
    );
}

#[tokio::test]
async fn sandbox_read_denied_by_path_policy_before_bridge() {
    use {
        crate::{
            exec::ExecResult,
            fs::sandbox_bridge::test_helpers::MockSandbox,
            sandbox::{Sandbox, SandboxConfig, SandboxRouter},
        },
        std::sync::Arc,
    };

    let policy = FsPathPolicy::new(&[], &["/data/secrets/**".to_string()]).unwrap();
    let mock = MockSandbox::new(vec![ExecResult {
        stdout: String::new(),
        stderr: String::new(),
        exit_code: 0,
    }]);
    let backend: Arc<dyn Sandbox> = mock.clone();
    let router = Arc::new(SandboxRouter::with_backend(
        SandboxConfig::default(),
        backend,
    ));
    router.set_override("sandboxed", true).await;

    let mut registry = ToolRegistry::new();
    register_fs_tools(&mut registry, FsToolsContext {
        path_policy: Some(policy),
        sandbox_router: Some(router),
        ..FsToolsContext::default()
    });

    let read = registry.get("Read").unwrap();
    let value = read
        .execute(json!({
            "file_path": "/data/secrets/token.txt",
            "_session_key": "sandboxed",
        }))
        .await
        .unwrap();

    assert_eq!(value["kind"], "path_denied");
    assert!(mock.last_command().is_none());
}

#[tokio::test]
async fn sandbox_write_denied_by_path_policy_before_bridge() {
    use {
        crate::{
            exec::ExecResult,
            fs::sandbox_bridge::test_helpers::MockSandbox,
            sandbox::{Sandbox, SandboxConfig, SandboxRouter},
        },
        std::sync::Arc,
    };

    let policy = FsPathPolicy::new(&[], &["/data/secrets/**".to_string()]).unwrap();
    let mock = MockSandbox::new(vec![ExecResult {
        stdout: String::new(),
        stderr: String::new(),
        exit_code: 0,
    }]);
    let backend: Arc<dyn Sandbox> = mock.clone();
    let router = Arc::new(SandboxRouter::with_backend(
        SandboxConfig::default(),
        backend,
    ));
    router.set_override("sandboxed", true).await;

    let mut registry = ToolRegistry::new();
    register_fs_tools(&mut registry, FsToolsContext {
        path_policy: Some(policy),
        sandbox_router: Some(router),
        ..FsToolsContext::default()
    });

    let write = registry.get("Write").unwrap();
    let value = write
        .execute(json!({
            "file_path": "/data/secrets/token.txt",
            "content": "nope",
            "_session_key": "sandboxed",
        }))
        .await
        .unwrap();

    assert_eq!(value["kind"], "path_denied");
    assert!(mock.last_command().is_none());
}

#[tokio::test]
async fn sandbox_write_requires_approval_before_bridge() {
    use {
        crate::{
            exec::ExecResult,
            fs::sandbox_bridge::test_helpers::MockSandbox,
            sandbox::{Sandbox, SandboxConfig, SandboxRouter},
        },
        base64::{Engine as _, engine::general_purpose::STANDARD as BASE64},
        std::sync::Arc,
    };

    let mock = MockSandbox::new(vec![ExecResult {
        stdout: String::new(),
        stderr: String::new(),
        exit_code: 0,
    }]);
    let backend: Arc<dyn Sandbox> = mock.clone();
    let router = Arc::new(SandboxRouter::with_backend(
        SandboxConfig::default(),
        backend,
    ));
    router.set_override("sandboxed", true).await;

    let approval_manager = Arc::new(ApprovalManager::default());
    let broadcaster = Arc::new(TestBroadcaster::new());
    let broadcaster_dyn: Arc<dyn ApprovalBroadcaster> = broadcaster.clone();

    let mut registry = ToolRegistry::new();
    register_fs_tools(&mut registry, FsToolsContext {
        sandbox_router: Some(router),
        approval_manager: Some(approval_manager.clone()),
        broadcaster: Some(broadcaster_dyn),
        ..FsToolsContext::default()
    });

    let mgr = approval_manager.clone();
    let resolver = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let id = mgr
            .pending_ids()
            .await
            .first()
            .cloned()
            .expect("request id");
        mgr.resolve(&id, ApprovalDecision::Approved, Some("Write /data/out.txt"))
            .await;
    });

    let write = registry.get("Write").unwrap();
    let value = write
        .execute(json!({
            "file_path": "/data/out.txt",
            "content": "sandboxed write",
            "_session_key": "sandboxed",
        }))
        .await
        .unwrap();
    resolver.await.unwrap();

    assert_eq!(value["bytes_written"], 15);
    assert!(broadcaster.called.load(Ordering::SeqCst));
    let cmd = mock.last_command().unwrap();
    assert!(cmd.contains("/data/out.txt"));
    assert!(cmd.contains(&BASE64.encode(b"sandboxed write")));
}

#[tokio::test]
async fn edit_denied_without_approval_does_not_mutate_host_file() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("edit.txt");
    tokio::fs::write(&target, "before\n").await.unwrap();

    let approval_manager = Arc::new(ApprovalManager::default());
    let broadcaster = Arc::new(TestBroadcaster::new());
    let broadcaster_dyn: Arc<dyn ApprovalBroadcaster> = broadcaster.clone();

    let mut registry = ToolRegistry::new();
    register_fs_tools(&mut registry, FsToolsContext {
        approval_manager: Some(approval_manager.clone()),
        broadcaster: Some(broadcaster_dyn),
        ..FsToolsContext::default()
    });

    let mgr = approval_manager.clone();
    let resolver = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let id = mgr
            .pending_ids()
            .await
            .first()
            .cloned()
            .expect("request id");
        mgr.resolve(&id, ApprovalDecision::Denied, None).await;
    });

    let edit = registry.get("Edit").unwrap();
    let err = edit
        .execute(json!({
            "file_path": target.to_str().unwrap(),
            "old_string": "before",
            "new_string": "after",
        }))
        .await
        .unwrap_err();
    resolver.await.unwrap();

    assert!(err.to_string().contains("denied"));
    assert_eq!(
        tokio::fs::read_to_string(&target).await.unwrap(),
        "before\n"
    );
    assert!(broadcaster.called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn multi_edit_denied_without_approval_does_not_mutate_host_file() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("multi.txt");
    tokio::fs::write(&target, "alpha\nbeta\n").await.unwrap();

    let approval_manager = Arc::new(ApprovalManager::default());
    let broadcaster = Arc::new(TestBroadcaster::new());
    let broadcaster_dyn: Arc<dyn ApprovalBroadcaster> = broadcaster.clone();

    let mut registry = ToolRegistry::new();
    register_fs_tools(&mut registry, FsToolsContext {
        approval_manager: Some(approval_manager.clone()),
        broadcaster: Some(broadcaster_dyn),
        ..FsToolsContext::default()
    });

    let mgr = approval_manager.clone();
    let resolver = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let id = mgr
            .pending_ids()
            .await
            .first()
            .cloned()
            .expect("request id");
        mgr.resolve(&id, ApprovalDecision::Denied, None).await;
    });

    let multi_edit = registry.get("MultiEdit").unwrap();
    let err = multi_edit
        .execute(json!({
            "file_path": target.to_str().unwrap(),
            "edits": [
                { "old_string": "alpha", "new_string": "gamma" },
                { "old_string": "beta", "new_string": "delta" }
            ],
        }))
        .await
        .unwrap_err();
    resolver.await.unwrap();

    assert!(err.to_string().contains("denied"));
    assert_eq!(
        tokio::fs::read_to_string(&target).await.unwrap(),
        "alpha\nbeta\n"
    );
    assert!(broadcaster.called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn typed_not_found_survives_registry_dispatch() {
    let registry = build_registry(None);
    let read = registry.get("Read").unwrap();
    let v = read
        .execute(json!({ "file_path": "/tmp/does-not-exist-contract-99aa1" }))
        .await
        .unwrap();
    assert_eq!(v["kind"], "not_found");
}
