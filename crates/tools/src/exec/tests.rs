#![allow(clippy::unwrap_used, clippy::expect_used)]

use {
    super::*,
    std::sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct TestBroadcaster {
    called: AtomicBool,
    session_key: std::sync::Mutex<Option<String>>,
}

impl TestBroadcaster {
    fn new() -> Self {
        Self {
            called: AtomicBool::new(false),
            session_key: std::sync::Mutex::new(None),
        }
    }
}

#[test]
fn truncate_output_for_display_handles_multibyte_boundary() {
    let mut output = format!("{}л{}", "a".repeat(1999), "z".repeat(10));
    truncate_output_for_display(&mut output, 2000);
    assert!(output.contains("[output truncated]"));
    assert!(!output.contains('л'));
}

#[async_trait]
impl ApprovalBroadcaster for TestBroadcaster {
    async fn broadcast_request(
        &self,
        _request_id: &str,
        _command: &str,
        session_key: Option<&str>,
    ) -> Result<()> {
        self.called.store(true, Ordering::SeqCst);
        *self.session_key.lock().unwrap() = session_key.map(ToOwned::to_owned);
        Ok(())
    }
}

#[tokio::test]
async fn test_exec_echo() {
    let result = exec_command("echo hello", &ExecOpts::default())
        .await
        .unwrap();
    assert_eq!(result.stdout.trim(), "hello");
    assert_eq!(result.exit_code, 0);
}

#[tokio::test]
async fn test_exec_stderr() {
    let result = exec_command("echo err >&2", &ExecOpts::default())
        .await
        .unwrap();
    assert_eq!(result.stderr.trim(), "err");
}

#[tokio::test]
async fn test_exec_exit_code() {
    let result = exec_command("exit 42", &ExecOpts::default()).await.unwrap();
    assert_eq!(result.exit_code, 42);
}

#[tokio::test]
async fn test_exec_timeout() {
    let opts = ExecOpts {
        timeout: Duration::from_millis(100),
        ..Default::default()
    };
    let result = exec_command("sleep 10", &opts).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_exec_tool() {
    let temp_dir = tempfile::tempdir().unwrap();
    let tool = ExecTool {
        working_dir: Some(temp_dir.path().to_path_buf()),
        ..Default::default()
    };
    let result = tool
        .execute(serde_json::json!({ "command": "echo hello" }))
        .await
        .unwrap();
    assert_eq!(result["stdout"].as_str().unwrap().trim(), "hello");
    assert_eq!(result["exit_code"], 0);
}

#[tokio::test]
async fn test_exec_tool_empty_working_dir() {
    let temp_dir = tempfile::tempdir().unwrap();
    let tool = ExecTool {
        working_dir: Some(temp_dir.path().to_path_buf()),
        ..Default::default()
    };
    let result = tool
        .execute(serde_json::json!({ "command": "pwd", "working_dir": "" }))
        .await
        .unwrap();
    assert_eq!(result["exit_code"], 0);
    assert!(!result["stdout"].as_str().unwrap().trim().is_empty());
}

#[tokio::test]
async fn test_exec_tool_safe_command_no_approval_needed() {
    let mgr = Arc::new(ApprovalManager::default());
    let bc = Arc::new(TestBroadcaster::new());
    let bc_dyn: Arc<dyn ApprovalBroadcaster> = Arc::clone(&bc) as _;
    let temp_dir = tempfile::tempdir().unwrap();
    let mut tool = ExecTool::default().with_approval(Arc::clone(&mgr), bc_dyn);
    tool.working_dir = Some(temp_dir.path().to_path_buf());
    let result = tool
        .execute(serde_json::json!({ "command": "echo safe" }))
        .await
        .unwrap();
    assert_eq!(result["stdout"].as_str().unwrap().trim(), "safe");
    assert!(!bc.called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn test_exec_tool_approval_approved() {
    let mgr = Arc::new(ApprovalManager::default());
    let bc = Arc::new(TestBroadcaster::new());
    let bc_dyn: Arc<dyn ApprovalBroadcaster> = Arc::clone(&bc) as _;
    let temp_dir = tempfile::tempdir().unwrap();
    let mut tool = ExecTool::default().with_approval(Arc::clone(&mgr), bc_dyn);
    tool.working_dir = Some(temp_dir.path().to_path_buf());

    let mgr2 = Arc::clone(&mgr);
    let handle = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let ids = mgr2.pending_ids().await;
        let id = ids.first().unwrap().clone();
        mgr2.resolve(
            &id,
            ApprovalDecision::Approved,
            Some("curl http://example.com"),
        )
        .await;
    });

    let result = tool
        .execute(serde_json::json!({
            "command": "curl http://example.com",
            "_session_key": "session:abc"
        }))
        .await;
    handle.await.unwrap();
    assert!(bc.called.load(Ordering::SeqCst));
    assert_eq!(
        bc.session_key.lock().unwrap().as_deref(),
        Some("session:abc")
    );
    let _ = result;
}

#[tokio::test]
async fn test_exec_tool_approval_denied() {
    let mgr = Arc::new(ApprovalManager::default());
    let bc = Arc::new(TestBroadcaster::new());
    let bc_dyn: Arc<dyn ApprovalBroadcaster> = Arc::clone(&bc) as _;
    let temp_dir = tempfile::tempdir().unwrap();
    let mut tool = ExecTool::default().with_approval(Arc::clone(&mgr), bc_dyn);
    tool.working_dir = Some(temp_dir.path().to_path_buf());

    let mgr2 = Arc::clone(&mgr);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let ids = mgr2.pending_ids().await;
        let id = ids.first().unwrap().clone();
        mgr2.resolve(&id, ApprovalDecision::Denied, None).await;
    });

    let result = tool
        .execute(serde_json::json!({ "command": "rm -rf /" }))
        .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("denied"));
}

#[tokio::test]
async fn test_exec_tool_with_sandbox() {
    use crate::sandbox::{NoSandbox, SandboxScope};

    let sandbox: Arc<dyn Sandbox> = Arc::new(NoSandbox);
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "test-session".into(),
    };
    let temp_dir = tempfile::tempdir().unwrap();
    let mut tool = ExecTool::default().with_sandbox(sandbox, id);
    tool.working_dir = Some(temp_dir.path().to_path_buf());
    let result = tool
        .execute(serde_json::json!({ "command": "echo sandboxed" }))
        .await
        .unwrap();
    assert_eq!(result["stdout"].as_str().unwrap().trim(), "sandboxed");
    assert_eq!(result["exit_code"], 0);
}

struct RetryRecoverySandbox {
    ensure_ready_calls: AtomicUsize,
    cleanup_calls: AtomicUsize,
    exec_calls: AtomicUsize,
    cleanup_should_fail: bool,
    failures_before_success: usize,
}

impl RetryRecoverySandbox {
    fn new(cleanup_should_fail: bool, failures_before_success: usize) -> Self {
        Self {
            ensure_ready_calls: AtomicUsize::new(0),
            cleanup_calls: AtomicUsize::new(0),
            exec_calls: AtomicUsize::new(0),
            cleanup_should_fail,
            failures_before_success,
        }
    }
}

#[async_trait]
impl Sandbox for RetryRecoverySandbox {
    fn backend_name(&self) -> &'static str {
        "docker"
    }

    async fn ensure_ready(&self, _id: &SandboxId, _image_override: Option<&str>) -> Result<()> {
        self.ensure_ready_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn exec(&self, _id: &SandboxId, _command: &str, _opts: &ExecOpts) -> Result<ExecResult> {
        let call = self.exec_calls.fetch_add(1, Ordering::SeqCst);
        if call < self.failures_before_success {
            return Ok(ExecResult {
                    stdout: String::new(),
                    stderr: "Error: internalError: \"failed to create process in container\" (cause: \"invalidState: \\\"cannot exec: container is not running\\\"\")".to_string(),
                    exit_code: 1,
                });
        }
        Ok(ExecResult {
            stdout: "recovered".to_string(),
            stderr: String::new(),
            exit_code: 0,
        })
    }

    async fn cleanup(&self, _id: &SandboxId) -> Result<()> {
        self.cleanup_calls.fetch_add(1, Ordering::SeqCst);
        if self.cleanup_should_fail {
            return Err(Error::message("cleanup failed"));
        }
        Ok(())
    }
}

#[derive(Default)]
struct CaptureWorkingDirSandbox {
    last_working_dir: std::sync::Mutex<Option<PathBuf>>,
}

#[async_trait]
impl Sandbox for CaptureWorkingDirSandbox {
    fn backend_name(&self) -> &'static str {
        "docker"
    }

    async fn ensure_ready(&self, _id: &SandboxId, _image_override: Option<&str>) -> Result<()> {
        Ok(())
    }

    async fn exec(&self, _id: &SandboxId, _command: &str, opts: &ExecOpts) -> Result<ExecResult> {
        let mut guard = self
            .last_working_dir
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *guard = opts.working_dir.clone();
        Ok(ExecResult {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
        })
    }

    async fn cleanup(&self, _id: &SandboxId) -> Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn test_exec_tool_retries_container_not_running_with_cleanup() {
    use crate::sandbox::SandboxScope;

    let sandbox = Arc::new(RetryRecoverySandbox::new(false, 1));
    let sandbox_dyn: Arc<dyn Sandbox> = Arc::clone(&sandbox) as _;
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "retry-session".into(),
    };
    let result = ExecTool::default()
        .with_sandbox(sandbox_dyn, id)
        .execute(serde_json::json!({ "command": "echo hi" }))
        .await
        .unwrap();

    assert_eq!(result["exit_code"], 0);
    assert_eq!(result["stdout"].as_str().unwrap(), "recovered");
    assert_eq!(sandbox.ensure_ready_calls.load(Ordering::SeqCst), 2);
    assert_eq!(sandbox.cleanup_calls.load(Ordering::SeqCst), 1);
    assert_eq!(sandbox.exec_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_exec_tool_retries_container_not_running_when_cleanup_fails() {
    use crate::sandbox::SandboxScope;

    let sandbox = Arc::new(RetryRecoverySandbox::new(true, 1));
    let sandbox_dyn: Arc<dyn Sandbox> = Arc::clone(&sandbox) as _;
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "retry-cleanup-fail-session".into(),
    };
    let result = ExecTool::default()
        .with_sandbox(sandbox_dyn, id)
        .execute(serde_json::json!({ "command": "echo hi" }))
        .await
        .unwrap();

    assert_eq!(result["exit_code"], 0);
    assert_eq!(result["stdout"].as_str().unwrap(), "recovered");
    assert_eq!(sandbox.ensure_ready_calls.load(Ordering::SeqCst), 2);
    assert_eq!(sandbox.cleanup_calls.load(Ordering::SeqCst), 1);
    assert_eq!(sandbox.exec_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_exec_tool_stops_after_max_container_retries() {
    use crate::sandbox::SandboxScope;

    let sandbox = Arc::new(RetryRecoverySandbox::new(
        false,
        MAX_SANDBOX_RECOVERY_RETRIES + 1,
    ));
    let sandbox_dyn: Arc<dyn Sandbox> = Arc::clone(&sandbox) as _;
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "retry-max-session".into(),
    };
    let result = ExecTool::default()
        .with_sandbox(sandbox_dyn, id)
        .execute(serde_json::json!({ "command": "echo hi" }))
        .await
        .unwrap();

    assert_eq!(result["exit_code"], 1);
    assert!(is_container_not_running_exec_error(
        result["stderr"].as_str().unwrap_or_default()
    ));
    assert_eq!(
        sandbox.ensure_ready_calls.load(Ordering::SeqCst),
        MAX_SANDBOX_RECOVERY_RETRIES + 1
    );
    assert_eq!(
        sandbox.cleanup_calls.load(Ordering::SeqCst),
        MAX_SANDBOX_RECOVERY_RETRIES
    );
    assert_eq!(
        sandbox.exec_calls.load(Ordering::SeqCst),
        MAX_SANDBOX_RECOVERY_RETRIES + 1
    );
}

#[tokio::test]
async fn test_exec_tool_cleanup_no_sandbox() {
    let tool = ExecTool::default();
    tool.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_exec_tool_cleanup_with_sandbox() {
    use crate::sandbox::{NoSandbox, SandboxScope};

    let sandbox: Arc<dyn Sandbox> = Arc::new(NoSandbox);
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "cleanup-test".into(),
    };
    let tool = ExecTool::default().with_sandbox(sandbox, id);
    tool.cleanup().await.unwrap();
}

struct TestEnvProvider;

#[async_trait]
impl EnvVarProvider for TestEnvProvider {
    async fn get_env_vars(&self) -> Vec<(String, secrecy::Secret<String>)> {
        vec![(
            "TEST_INJECTED".into(),
            secrecy::Secret::new("hello_from_env".into()),
        )]
    }
}

#[tokio::test]
async fn test_exec_tool_with_env_provider() {
    let provider: Arc<dyn EnvVarProvider> = Arc::new(TestEnvProvider);
    let temp_dir = tempfile::tempdir().unwrap();
    let mut tool = ExecTool::default().with_env_provider(provider);
    tool.working_dir = Some(temp_dir.path().to_path_buf());
    let result = tool
        .execute(serde_json::json!({ "command": "echo $TEST_INJECTED" }))
        .await
        .unwrap();
    // The value is redacted in output.
    assert_eq!(result["stdout"].as_str().unwrap().trim(), "[REDACTED]");
}

#[tokio::test]
async fn test_env_var_redaction_base64_exfiltration() {
    let provider: Arc<dyn EnvVarProvider> = Arc::new(TestEnvProvider);
    let temp_dir = tempfile::tempdir().unwrap();
    let mut tool = ExecTool::default().with_env_provider(provider);
    tool.working_dir = Some(temp_dir.path().to_path_buf());
    let result = tool
        .execute(serde_json::json!({ "command": "echo $TEST_INJECTED | base64" }))
        .await
        .unwrap();
    let stdout = result["stdout"].as_str().unwrap().trim();
    assert!(
        !stdout.contains("aGVsbG9fZnJvbV9lbnY"),
        "base64 of secret should be redacted, got: {stdout}"
    );
}

#[tokio::test]
async fn test_env_var_redaction_hex_exfiltration() {
    let provider: Arc<dyn EnvVarProvider> = Arc::new(TestEnvProvider);
    let temp_dir = tempfile::tempdir().unwrap();
    let mut tool = ExecTool::default().with_env_provider(provider);
    tool.working_dir = Some(temp_dir.path().to_path_buf());
    let result = tool
        .execute(serde_json::json!({ "command": "printf '%s' \"$TEST_INJECTED\" | xxd -p" }))
        .await
        .unwrap();
    let stdout = result["stdout"].as_str().unwrap().trim();
    assert!(
        !stdout.contains("68656c6c6f5f66726f6d5f656e76"),
        "hex of secret should be redacted, got: {stdout}"
    );
}

#[tokio::test]
async fn test_env_var_redaction_file_exfiltration() {
    let provider: Arc<dyn EnvVarProvider> = Arc::new(TestEnvProvider);
    let temp_dir = tempfile::tempdir().unwrap();
    let mut tool = ExecTool::default().with_env_provider(provider);
    tool.working_dir = Some(temp_dir.path().to_path_buf());
    let result = tool
        .execute(serde_json::json!({
            "command": "f=$(mktemp); echo $TEST_INJECTED > $f; cat $f; rm $f"
        }))
        .await
        .unwrap();
    let stdout = result["stdout"].as_str().unwrap().trim();
    assert_eq!(stdout, "[REDACTED]", "file read-back should be redacted");
}

#[test]
fn test_redaction_needles() {
    let needles = redaction_needles("secret123");
    // Raw value
    assert!(needles.contains(&"secret123".to_string()));
    // base64
    assert!(needles.iter().any(|n| n.contains("c2VjcmV0MTIz")));
    // hex
    assert!(needles.iter().any(|n| n.contains("736563726574313233")));
}

#[test]
fn test_is_container_not_running_exec_error() {
    assert!(is_container_not_running_exec_error(
        "Error: internalError: \"failed to create process in container\" (cause: \"invalidState: \\\"cannot exec: container is not running\\\"\")"
    ));
    assert!(is_container_not_running_exec_error(
        "cannot exec: container is not running"
    ));
    assert!(is_container_not_running_exec_error(
        "Error: invalidState: \"container codex-stop-12016 is not running\""
    ));
    assert!(is_container_not_running_exec_error(
        "Error: internalError: \"failed to create process in container\" (cause: \"invalidState: \\\"no sandbox client exists: container is stopped\\\"\")"
    ));
    // notFound errors from get/inspect failures
    assert!(is_container_not_running_exec_error(
        "Error: notFound: \"get failed: container moltis-sandbox-main not found\""
    ));
    assert!(is_container_not_running_exec_error(
        "container not found: moltis-sandbox-session-abc"
    ));
    assert!(!is_container_not_running_exec_error(
        "permission denied: operation not permitted"
    ));
}

#[tokio::test]
async fn test_exec_tool_with_sandbox_router_off() {
    use crate::sandbox::{NoSandbox, SandboxConfig, SandboxRouter};

    let router = Arc::new(SandboxRouter::with_backend(
        SandboxConfig::default(),
        Arc::new(NoSandbox),
    ));
    let temp_dir = tempfile::tempdir().unwrap();
    let mut tool = ExecTool::default().with_sandbox_router(router);
    tool.working_dir = Some(temp_dir.path().to_path_buf());
    // No session key → defaults to "main", mode=Off → direct exec.
    let result = tool
        .execute(serde_json::json!({ "command": "echo direct" }))
        .await
        .unwrap();
    assert_eq!(result["stdout"].as_str().unwrap().trim(), "direct");
}

#[tokio::test]
async fn test_exec_tool_with_sandbox_router_session_key() {
    use crate::sandbox::{NoSandbox, SandboxConfig, SandboxRouter};

    let router = Arc::new(SandboxRouter::with_backend(
        SandboxConfig::default(),
        Arc::new(NoSandbox),
    ));
    // Override to enable sandbox for this session (NoSandbox backend → still executes directly).
    router.set_override("session:abc", true).await;
    let temp_dir = tempfile::tempdir().unwrap();
    let mut tool = ExecTool::default().with_sandbox_router(router);
    tool.working_dir = Some(temp_dir.path().to_path_buf());
    let result = tool
        .execute(serde_json::json!({
            "command": "echo routed",
            "_session_key": "session:abc"
        }))
        .await
        .unwrap();
    assert_eq!(result["stdout"].as_str().unwrap().trim(), "routed");
}

/// Regression test: when SandboxMode=All (the default) but the backend is
/// NoSandbox (no container runtime), the exec tool must NOT use
/// /home/sandbox as the working directory.  It should fall back to the host
/// data directory and execute successfully.
#[tokio::test]
async fn test_exec_tool_no_container_backend_with_sandbox_mode_all() {
    use crate::sandbox::{NoSandbox, SandboxConfig, SandboxRouter};

    // Default config has mode=All, so is_sandboxed() returns true for
    // every session.  But the backend is NoSandbox ("none") — no Docker.
    let router = Arc::new(SandboxRouter::with_backend(
        SandboxConfig::default(),
        Arc::new(NoSandbox),
    ));
    // No explicit working_dir — the tool must NOT default to /home/sandbox.
    let tool = ExecTool::default().with_sandbox_router(router);
    let result = tool
        .execute(serde_json::json!({ "command": "echo works" }))
        .await
        .unwrap();
    assert_eq!(result["stdout"].as_str().unwrap().trim(), "works");
    assert_eq!(result["exit_code"], 0);
}

#[tokio::test]
async fn test_exec_tool_sandbox_rewrites_host_absolute_working_dir() {
    use crate::sandbox::SandboxScope;

    let sandbox = Arc::new(CaptureWorkingDirSandbox::default());
    let sandbox_dyn: Arc<dyn Sandbox> = Arc::clone(&sandbox) as _;
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "rewrite-host-abs-path".into(),
    };

    ExecTool::default()
        .with_sandbox(sandbox_dyn, id)
        .execute(serde_json::json!({
            "command": "echo test",
            "working_dir": "/Users/fabien"
        }))
        .await
        .unwrap();

    let captured = sandbox
        .last_working_dir
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    assert_eq!(captured, Some(PathBuf::from("/home/sandbox")));
}

#[tokio::test]
async fn test_exec_tool_sandbox_resolves_relative_working_dir_under_home() {
    use crate::sandbox::SandboxScope;

    let sandbox = Arc::new(CaptureWorkingDirSandbox::default());
    let sandbox_dyn: Arc<dyn Sandbox> = Arc::clone(&sandbox) as _;
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "rewrite-relative-path".into(),
    };

    ExecTool::default()
        .with_sandbox(sandbox_dyn, id)
        .execute(serde_json::json!({
            "command": "echo test",
            "working_dir": "project"
        }))
        .await
        .unwrap();

    let captured = sandbox
        .last_working_dir
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    assert_eq!(captured, Some(PathBuf::from("/home/sandbox/project")));
}

#[tokio::test]
async fn test_exec_tool_sandbox_keeps_in_sandbox_absolute_working_dir() {
    use crate::sandbox::SandboxScope;

    let sandbox = Arc::new(CaptureWorkingDirSandbox::default());
    let sandbox_dyn: Arc<dyn Sandbox> = Arc::clone(&sandbox) as _;
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "keep-sandbox-abs-path".into(),
    };

    ExecTool::default()
        .with_sandbox(sandbox_dyn, id)
        .execute(serde_json::json!({
            "command": "echo test",
            "working_dir": "/home/sandbox/work"
        }))
        .await
        .unwrap();

    let captured = sandbox
        .last_working_dir
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    assert_eq!(captured, Some(PathBuf::from("/home/sandbox/work")));
}

#[tokio::test]
async fn test_exec_command_bad_working_dir_error_message() {
    let opts = ExecOpts {
        working_dir: Some(PathBuf::from("/nonexistent_dir_12345")),
        ..Default::default()
    };
    let err = exec_command("echo hello", &opts).await.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("/nonexistent_dir_12345"),
        "error should mention the bad directory, got: {msg}"
    );
    assert!(
        msg.contains("working directory"),
        "error should mention 'working directory', got: {msg}"
    );
}

#[tokio::test]
async fn test_completion_callback_fires() {
    let called = Arc::new(AtomicBool::new(false));
    let called_clone = Arc::clone(&called);
    let cb: ExecCompletionFn = Arc::new(move |event| {
        assert_eq!(event.command, "echo callback");
        assert_eq!(event.exit_code, 0);
        assert!(event.stdout_preview.contains("callback"));
        called_clone.store(true, Ordering::SeqCst);
    });
    let temp_dir = tempfile::tempdir().unwrap();
    let mut tool = ExecTool::default().with_completion_callback(cb);
    tool.working_dir = Some(temp_dir.path().to_path_buf());
    tool.execute(serde_json::json!({ "command": "echo callback" }))
        .await
        .unwrap();
    assert!(called.load(Ordering::SeqCst), "callback should have fired");
}

#[tokio::test]
async fn test_no_callback_by_default() {
    let temp_dir = tempfile::tempdir().unwrap();
    let tool = ExecTool {
        working_dir: Some(temp_dir.path().to_path_buf()),
        ..Default::default()
    };
    // Should work fine without a callback.
    let result = tool
        .execute(serde_json::json!({ "command": "echo default" }))
        .await
        .unwrap();
    assert_eq!(result["exit_code"], 0);
}

/// Stub node provider that never has connected nodes.
struct DisconnectedNodeProvider;

#[async_trait]
impl NodeExecProvider for DisconnectedNodeProvider {
    async fn exec_on_node(
        &self,
        _node_id: &str,
        _command: &str,
        _timeout_secs: u64,
        _cwd: Option<&str>,
        _env: Option<&HashMap<String, String>>,
    ) -> anyhow::Result<ExecResult> {
        unreachable!("should not route to a disconnected node");
    }

    async fn resolve_node_id(&self, _node_ref: &str) -> Option<String> {
        unreachable!("should not attempt to resolve when no nodes connected");
    }

    fn has_connected_nodes(&self) -> bool {
        false
    }

    async fn default_node_ref(&self) -> Option<String> {
        None
    }
}

#[tokio::test]
async fn test_exec_ignores_node_param_when_no_nodes_connected() {
    let temp_dir = tempfile::tempdir().unwrap();
    let tool = ExecTool {
        working_dir: Some(temp_dir.path().to_path_buf()),
        ..Default::default()
    }
    .with_node_provider(Arc::new(DisconnectedNodeProvider), None);

    // Model passes a bogus node value — should fall through to local exec.
    let result = tool
        .execute(serde_json::json!({
            "command": "echo fallthrough",
            "node": "host"
        }))
        .await
        .unwrap();
    assert_eq!(result["stdout"].as_str().unwrap().trim(), "fallthrough");
    assert_eq!(result["exit_code"], 0);
}

#[tokio::test]
async fn test_exec_ignores_empty_node_param() {
    let temp_dir = tempfile::tempdir().unwrap();
    let tool = ExecTool {
        working_dir: Some(temp_dir.path().to_path_buf()),
        ..Default::default()
    }
    .with_node_provider(Arc::new(DisconnectedNodeProvider), None);

    // Model passes an empty string for node — should fall through to local exec.
    let result = tool
        .execute(serde_json::json!({
            "command": "echo empty",
            "node": ""
        }))
        .await
        .unwrap();
    assert_eq!(result["stdout"].as_str().unwrap().trim(), "empty");
    assert_eq!(result["exit_code"], 0);
}

#[tokio::test]
async fn test_exec_ignores_whitespace_only_node_param() {
    let temp_dir = tempfile::tempdir().unwrap();
    let tool = ExecTool {
        working_dir: Some(temp_dir.path().to_path_buf()),
        ..Default::default()
    }
    .with_node_provider(Arc::new(DisconnectedNodeProvider), None);

    let result = tool
        .execute(serde_json::json!({
            "command": "echo spaces",
            "node": "   "
        }))
        .await
        .unwrap();
    assert_eq!(result["stdout"].as_str().unwrap().trim(), "spaces");
    assert_eq!(result["exit_code"], 0);
}

#[tokio::test]
async fn test_exec_schema_hides_node_when_no_nodes_connected() {
    let tool = ExecTool::default().with_node_provider(Arc::new(DisconnectedNodeProvider), None);

    let schema = tool.parameters_schema();
    let props = schema["properties"].as_object().unwrap();
    assert!(
        !props.contains_key("node"),
        "node param should be hidden when no nodes are connected"
    );
}

#[tokio::test]
async fn test_exec_errors_when_default_node_configured_but_disconnected() {
    let temp_dir = tempfile::tempdir().unwrap();
    let tool = ExecTool {
        working_dir: Some(temp_dir.path().to_path_buf()),
        ..Default::default()
    }
    .with_node_provider(
        Arc::new(DisconnectedNodeProvider),
        Some("production".into()),
    );

    // Admin configured a default node but it's not connected — must error,
    // not silently fall through to local execution.
    let err = tool
        .execute(serde_json::json!({ "command": "echo should-fail" }))
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("production"),
        "error should mention the configured node name, got: {msg}"
    );
    assert!(
        msg.contains("no nodes are currently connected"),
        "error should explain no nodes are connected, got: {msg}"
    );
}

#[test]
fn test_with_default_timeout() {
    let tool = ExecTool::default().with_default_timeout(Duration::from_secs(120));
    assert_eq!(tool.default_timeout, Duration::from_secs(120));
}

#[test]
fn test_with_max_output_bytes() {
    let tool = ExecTool::default().with_max_output_bytes(1024 * 1024);
    assert_eq!(tool.max_output_bytes, 1024 * 1024);
}

#[test]
fn test_schema_timeout_reflects_configured_default() {
    let tool = ExecTool::default().with_default_timeout(Duration::from_secs(300));
    let schema = tool.parameters_schema();
    let desc = schema["properties"]["timeout"]["description"]
        .as_str()
        .unwrap();
    assert!(
        desc.contains("default 300"),
        "schema should reflect configured timeout, got: {desc}"
    );
}

#[tokio::test]
async fn test_custom_timeout_causes_timeout() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mut tool = ExecTool::default().with_default_timeout(Duration::from_millis(200));
    tool.working_dir = Some(temp_dir.path().to_path_buf());

    // sleep 60 should be killed well before 60s by the 200ms timeout
    let result = tool
        .execute(serde_json::json!({ "command": "sleep 60" }))
        .await;
    match result {
        Err(ref e) => {
            let msg = e.to_string();
            assert!(
                msg.contains("timed out") || msg.contains("timeout"),
                "expected timeout error, got: {msg}"
            );
        },
        Ok(ref val) => {
            // Some platforms return an exit code instead of an error
            let exit_code = val["exit_code"].as_i64().unwrap_or(0);
            assert_ne!(
                exit_code, 0,
                "command should not succeed under short timeout"
            );
        },
    }
}

#[tokio::test]
async fn test_custom_max_output_bytes_truncates() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mut tool = ExecTool::default().with_max_output_bytes(50);
    tool.working_dir = Some(temp_dir.path().to_path_buf());

    let result = tool
        .execute(serde_json::json!({
            "command": "python3 -c \"print('A' * 500)\" 2>/dev/null || printf '%0.sA' $(seq 1 500)"
        }))
        .await
        .unwrap();
    let stdout = result["stdout"].as_str().unwrap_or("");
    assert!(
        stdout.contains("[truncated") || stdout.len() <= 100,
        "output should be truncated with 50-byte limit, got {} bytes: {stdout}",
        stdout.len()
    );
}
