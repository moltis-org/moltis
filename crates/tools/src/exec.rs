use std::{path::PathBuf, sync::Arc, time::Duration};

#[cfg(feature = "metrics")]
use std::time::Instant;

use {
    anyhow::{Result, bail},
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
    tokio::process::Command,
    tracing::{debug, info, warn},
};

#[cfg(feature = "metrics")]
use moltis_metrics::{
    counter, gauge, histogram, labels, sandbox as sandbox_metrics, tools as tools_metrics,
};

use moltis_agents::tool_registry::AgentTool;

use crate::{
    approval::{ApprovalAction, ApprovalDecision, ApprovalManager},
    sandbox::{NoSandbox, Sandbox, SandboxId, SandboxRouter},
};

/// Broadcaster that notifies connected clients about pending approval requests.
#[async_trait]
pub trait ApprovalBroadcaster: Send + Sync {
    async fn broadcast_request(&self, request_id: &str, command: &str) -> Result<()>;
}

/// Provider of environment variables to inject into sandbox execution.
/// Values are wrapped in `Secret` to prevent accidental logging.
#[async_trait]
pub trait EnvVarProvider: Send + Sync {
    async fn get_env_vars(&self) -> Vec<(String, secrecy::Secret<String>)>;
}

/// Result of a shell command execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Options controlling exec behavior.
#[derive(Debug, Clone)]
pub struct ExecOpts {
    pub timeout: Duration,
    pub max_output_bytes: usize,
    pub working_dir: Option<PathBuf>,
    pub env: Vec<(String, String)>,
}

impl Default for ExecOpts {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            max_output_bytes: 200 * 1024, // 200KB
            working_dir: None,
            env: Vec::new(),
        }
    }
}

/// Execute a shell command with timeout and output limits.
pub async fn exec_command(command: &str, opts: &ExecOpts) -> Result<ExecResult> {
    debug!(
        command,
        timeout_secs = opts.timeout.as_secs(),
        "exec_command"
    );

    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);

    if let Some(ref dir) = opts.working_dir {
        cmd.current_dir(dir);
    }
    for (k, v) in &opts.env {
        cmd.env(k, v);
    }

    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    // Prevent the child from inheriting stdin.
    cmd.stdin(std::process::Stdio::null());

    let child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            if let Some(ref dir) = opts.working_dir {
                anyhow::anyhow!(
                    "failed to start command: working directory '{}' does not exist",
                    dir.display()
                )
            } else {
                anyhow::anyhow!("failed to start command: shell 'sh' not found")
            }
        } else {
            anyhow::anyhow!("failed to start command: {e}")
        }
    })?;

    let result = tokio::time::timeout(opts.timeout, child.wait_with_output()).await;

    match result {
        Ok(Ok(output)) => {
            let mut stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();

            // Truncate if exceeding limit.
            if stdout.len() > opts.max_output_bytes {
                stdout.truncate(opts.max_output_bytes);
                stdout.push_str("\n... [output truncated]");
            }
            if stderr.len() > opts.max_output_bytes {
                stderr.truncate(opts.max_output_bytes);
                stderr.push_str("\n... [output truncated]");
            }

            let exit_code = output.status.code().unwrap_or(-1);
            debug!(
                exit_code,
                stdout_len = stdout.len(),
                stderr_len = stderr.len(),
                "exec done"
            );

            Ok(ExecResult {
                stdout,
                stderr,
                exit_code,
            })
        },
        Ok(Err(e)) => bail!("failed to run command: {e}"),
        Err(_) => {
            warn!(command, "exec timeout");
            bail!("command timed out after {}s", opts.timeout.as_secs())
        },
    }
}

/// The exec tool exposed to the agent tool registry.
pub struct ExecTool {
    pub default_timeout: Duration,
    pub max_output_bytes: usize,
    pub working_dir: Option<PathBuf>,
    approval_manager: Option<Arc<ApprovalManager>>,
    broadcaster: Option<Arc<dyn ApprovalBroadcaster>>,
    sandbox: Arc<dyn Sandbox>,
    sandbox_id: Option<SandboxId>,
    sandbox_router: Option<Arc<SandboxRouter>>,
    env_provider: Option<Arc<dyn EnvVarProvider>>,
}

impl Default for ExecTool {
    fn default() -> Self {
        Self {
            default_timeout: Duration::from_secs(30),
            max_output_bytes: 200 * 1024,
            working_dir: None,
            approval_manager: None,
            broadcaster: None,
            sandbox: Arc::new(NoSandbox),
            sandbox_id: None,
            sandbox_router: None,
            env_provider: None,
        }
    }
}

impl ExecTool {
    /// Attach approval gating to this exec tool.
    pub fn with_approval(
        mut self,
        manager: Arc<ApprovalManager>,
        broadcaster: Arc<dyn ApprovalBroadcaster>,
    ) -> Self {
        self.approval_manager = Some(manager);
        self.broadcaster = Some(broadcaster);
        self
    }

    /// Attach a sandbox backend and ID for sandboxed execution (legacy static mode).
    pub fn with_sandbox(mut self, sandbox: Arc<dyn Sandbox>, id: SandboxId) -> Self {
        self.sandbox = sandbox;
        self.sandbox_id = Some(id);
        self
    }

    /// Attach a sandbox router for per-session dynamic sandbox resolution.
    pub fn with_sandbox_router(mut self, router: Arc<SandboxRouter>) -> Self {
        self.sandbox_router = Some(router);
        self
    }

    /// Attach an environment variable provider for sandbox injection.
    pub fn with_env_provider(mut self, provider: Arc<dyn EnvVarProvider>) -> Self {
        self.env_provider = Some(provider);
        self
    }

    /// Clean up sandbox resources. Call on session end.
    pub async fn cleanup(&self) -> Result<()> {
        if let Some(ref id) = self.sandbox_id {
            self.sandbox.cleanup(id).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl AgentTool for ExecTool {
    fn name(&self) -> &str {
        "exec"
    }

    fn description(&self) -> &str {
        "Execute a shell command on the server. Returns stdout, stderr, and exit code."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds (default 30, max 1800)"
                },
                "working_dir": {
                    "type": "string",
                    "description": "Working directory for the command"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        #[cfg(feature = "metrics")]
        let start = Instant::now();
        #[cfg(feature = "metrics")]
        gauge!(tools_metrics::EXECUTIONS_IN_FLIGHT, labels::TOOL => "exec").increment(1.0);

        let command = params
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'command' parameter"))?;

        let timeout_secs = params
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(self.default_timeout.as_secs())
            .min(1800); // cap at 30 minutes

        // Check sandbox state early — we need it for working_dir resolution.
        let session_key = params.get("_session_key").and_then(|v| v.as_str());
        let is_sandboxed = if let Some(ref router) = self.sandbox_router {
            router.is_sandboxed(session_key.unwrap_or("main")).await
        } else {
            self.sandbox_id.is_some()
        };

        // Check whether the backend is a real container runtime.  When the
        // backend is "none" (no Docker/container runtime available), commands
        // run directly on the host even when the session mode says "sandboxed".
        // Using /home/sandbox as the working directory would fail with ENOENT
        // on the host, so we must fall back to the host data directory.
        let has_container_backend = if let Some(ref router) = self.sandbox_router {
            router.backend_name() != "none"
        } else {
            self.sandbox.backend_name() != "none"
        };

        // Resolve working directory.  When sandboxed *with a real container
        // backend* the host CWD doesn't exist inside the container, so default
        // to "/home/sandbox".  When the backend is "none" (no container), fall
        // back to the host data directory to avoid ENOENT.
        let explicit_working_dir = params
            .get("working_dir")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .or_else(|| self.working_dir.clone());

        let using_default_working_dir = explicit_working_dir.is_none();
        let mut working_dir = explicit_working_dir.or_else(|| {
            if is_sandboxed && has_container_backend {
                Some(PathBuf::from("/home/sandbox"))
            } else {
                Some(moltis_config::data_dir())
            }
        });

        // Ensure default host working directory exists so command spawning does
        // not fail on fresh machines where ~/.moltis has not been created yet.
        if !(is_sandboxed && has_container_backend)
            && using_default_working_dir
            && let Some(dir) = working_dir.as_ref()
            && let Err(e) = tokio::fs::create_dir_all(dir).await
        {
            warn!(path = %dir.display(), error = %e, "failed to create default working dir, falling back to process cwd");
            working_dir = None;
        }

        info!(
            command,
            timeout_secs,
            ?working_dir,
            is_sandboxed,
            "exec tool invoked"
        );

        // Approval gating.
        if !is_sandboxed && let Some(ref mgr) = self.approval_manager {
            let action = mgr.check_command(command).await?;
            if action == ApprovalAction::NeedsApproval {
                info!(command, "command needs approval, waiting...");
                let (req_id, rx) = mgr.create_request(command).await;

                // Broadcast to connected clients.
                if let Some(ref bc) = self.broadcaster
                    && let Err(e) = bc.broadcast_request(&req_id, command).await
                {
                    warn!(error = %e, "failed to broadcast approval request");
                }

                let decision = mgr.wait_for_decision(rx).await;
                match decision {
                    ApprovalDecision::Approved => {
                        info!(command, "command approved");
                    },
                    ApprovalDecision::Denied => {
                        bail!("command denied by user: {command}");
                    },
                    ApprovalDecision::Timeout => {
                        bail!("approval timed out for command: {command}");
                    },
                }
            }
        }

        let secret_env = if let Some(ref provider) = self.env_provider {
            provider.get_env_vars().await
        } else {
            Vec::new()
        };

        // Expose secrets only at the injection boundary.
        use secrecy::ExposeSecret;
        let env: Vec<(String, String)> = secret_env
            .iter()
            .map(|(k, v)| (k.clone(), v.expose_secret().clone()))
            .collect();

        let opts = ExecOpts {
            timeout: Duration::from_secs(timeout_secs),
            max_output_bytes: self.max_output_bytes,
            working_dir,
            env: env.clone(),
        };

        // Resolve sandbox: dynamic per-session router takes priority over static sandbox.
        let result = if let Some(ref router) = self.sandbox_router {
            let sk = session_key.unwrap_or("main");
            if is_sandboxed {
                let id = router.sandbox_id_for(sk);
                let image = router.resolve_image(sk, None).await;
                let backend = router.backend();
                info!(session = sk, sandbox_id = %id, backend = backend.backend_name(), image, "sandbox ensure_ready");
                backend.ensure_ready(&id, Some(&image)).await?;
                debug!(session = sk, sandbox_id = %id, command, "sandbox running command");
                let mut sandbox_result = backend.exec(&id, command, &opts).await?;
                if sandbox_result.exit_code != 0
                    && is_container_not_running_exec_error(&sandbox_result.stderr)
                {
                    warn!(
                        session = sk,
                        sandbox_id = %id,
                        command,
                        "sandbox exec failed because container is not running, reinitializing and retrying once"
                    );
                    backend.ensure_ready(&id, Some(&image)).await?;
                    sandbox_result = backend.exec(&id, command, &opts).await?;
                }
                sandbox_result
            } else {
                debug!(session = sk, command, "running unsandboxed");
                exec_command(command, &opts).await?
            }
        } else if let Some(ref id) = self.sandbox_id {
            debug!(sandbox_id = %id, command, "static sandbox running command");
            self.sandbox.ensure_ready(id, None).await?;
            let mut sandbox_result = self.sandbox.exec(id, command, &opts).await?;
            if sandbox_result.exit_code != 0
                && is_container_not_running_exec_error(&sandbox_result.stderr)
            {
                warn!(
                    sandbox_id = %id,
                    command,
                    "sandbox exec failed because container is not running, reinitializing and retrying once"
                );
                self.sandbox.ensure_ready(id, None).await?;
                sandbox_result = self.sandbox.exec(id, command, &opts).await?;
            }
            sandbox_result
        } else {
            exec_command(command, &opts).await?
        };

        // Redact env var values from output so secrets don't leak to the LLM.
        // Covers the raw value plus common encodings (base64, hex) that could
        // be used to exfiltrate secrets via `echo $SECRET | base64` etc.
        let mut result = result;
        for (_, v) in &env {
            if !v.is_empty() {
                for needle in redaction_needles(v) {
                    result.stdout = result.stdout.replace(&needle, "[REDACTED]");
                    result.stderr = result.stderr.replace(&needle, "[REDACTED]");
                }
            }
        }

        info!(
            command,
            exit_code = result.exit_code,
            stdout_len = result.stdout.len(),
            stderr_len = result.stderr.len(),
            "exec tool completed"
        );

        // Record metrics
        #[cfg(feature = "metrics")]
        {
            let duration = start.elapsed().as_secs_f64();
            let success = result.exit_code == 0;

            counter!(
                tools_metrics::EXECUTIONS_TOTAL,
                labels::TOOL => "exec".to_string(),
                labels::SUCCESS => success.to_string()
            )
            .increment(1);

            histogram!(
                tools_metrics::EXECUTION_DURATION_SECONDS,
                labels::TOOL => "exec".to_string()
            )
            .record(duration);

            if !success {
                counter!(
                    tools_metrics::EXECUTION_ERRORS_TOTAL,
                    labels::TOOL => "exec".to_string()
                )
                .increment(1);
            }

            // Track sandbox-specific metrics
            if is_sandboxed {
                counter!(
                    sandbox_metrics::COMMAND_EXECUTIONS_TOTAL,
                    labels::SUCCESS => success.to_string()
                )
                .increment(1);

                histogram!(sandbox_metrics::COMMAND_DURATION_SECONDS).record(duration);

                if !success {
                    counter!(sandbox_metrics::COMMAND_ERRORS_TOTAL).increment(1);
                }
            }

            gauge!(tools_metrics::EXECUTIONS_IN_FLIGHT, labels::TOOL => "exec").decrement(1.0);
        }

        Ok(serde_json::to_value(&result)?)
    }
}

/// Build a set of strings to redact for a given secret value:
/// the raw value, its base64 encoding, and its hex encoding.
fn redaction_needles(value: &str) -> Vec<String> {
    use base64::Engine;

    let mut needles = vec![value.to_string()];

    // base64 (standard + URL-safe, with and without padding)
    let b64_std = base64::engine::general_purpose::STANDARD.encode(value.as_bytes());
    let b64_url = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value.as_bytes());
    if b64_std != value {
        needles.push(b64_std);
    }
    if b64_url != value {
        needles.push(b64_url);
    }

    // Hex encoding (lowercase)
    let hex = value
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    if hex != value {
        needles.push(hex);
    }

    needles
}

fn is_container_not_running_exec_error(stderr: &str) -> bool {
    stderr.contains("cannot exec: container is not running")
        || (stderr.contains("failed to create process in container")
            && stderr.contains("container is not running"))
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {
        super::*,
        std::sync::atomic::{AtomicBool, Ordering},
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
        async fn broadcast_request(&self, _request_id: &str, _command: &str) -> Result<()> {
            self.called.store(true, Ordering::SeqCst);
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
            .execute(serde_json::json!({ "command": "curl http://example.com" }))
            .await;
        handle.await.unwrap();
        assert!(bc.called.load(Ordering::SeqCst));
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
}
