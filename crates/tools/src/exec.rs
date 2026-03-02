use std::{path::PathBuf, sync::Arc, time::Duration};

#[cfg(feature = "metrics")]
use std::time::Instant;

use {
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
    tokio::process::Command,
    tracing::{debug, info, warn},
};

use crate::{Result, error::Error};

#[cfg(feature = "metrics")]
use moltis_metrics::{
    counter, gauge, histogram, labels, sandbox as sandbox_metrics, tools as tools_metrics,
};

use moltis_agents::tool_registry::AgentTool;

/// Event describing a completed exec invocation, passed to the completion callback.
#[derive(Debug, Clone)]
pub struct ExecCompletionEvent {
    pub command: String,
    pub exit_code: i32,
    pub stdout_preview: String,
    pub stderr_preview: String,
}

/// Callback fired after every exec completion. Used to enqueue system events
/// and wake the heartbeat.
pub type ExecCompletionFn = Arc<dyn Fn(ExecCompletionEvent) + Send + Sync>;

use crate::{
    approval::{ApprovalAction, ApprovalDecision, ApprovalManager},
    sandbox::{NoSandbox, Sandbox, SandboxId, SandboxRouter},
};

const MAX_SANDBOX_RECOVERY_RETRIES: usize = 1;

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

fn truncate_output_for_display(output: &mut String, max_output_bytes: usize) {
    if output.len() <= max_output_bytes {
        return;
    }
    output.truncate(output.floor_char_boundary(max_output_bytes));
    output.push_str("\n... [output truncated]");
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
                Error::message(format!(
                    "failed to start command: working directory '{}' does not exist",
                    dir.display()
                ))
            } else {
                Error::message("failed to start command: shell 'sh' not found")
            }
        } else {
            Error::message(format!("failed to start command: {e}"))
        }
    })?;

    let result = tokio::time::timeout(opts.timeout, child.wait_with_output()).await;

    match result {
        Ok(Ok(output)) => {
            let mut stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();

            // Truncate if exceeding limit.
            truncate_output_for_display(&mut stdout, opts.max_output_bytes);
            truncate_output_for_display(&mut stderr, opts.max_output_bytes);

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
        Ok(Err(e)) => Err(Error::message(format!("failed to run command: {e}"))),
        Err(_) => {
            warn!(command, "exec timeout");
            Err(Error::message(format!(
                "command timed out after {}s",
                opts.timeout.as_secs()
            )))
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
    completion_callback: Option<ExecCompletionFn>,
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
            completion_callback: None,
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

    /// Attach a callback that fires after every exec completion.
    pub fn with_completion_callback(mut self, cb: ExecCompletionFn) -> Self {
        self.completion_callback = Some(cb);
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

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        #[cfg(feature = "metrics")]
        let start = Instant::now();
        #[cfg(feature = "metrics")]
        gauge!(tools_metrics::EXECUTIONS_IN_FLIGHT, labels::TOOL => "exec").increment(1.0);

        let command = params
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("missing 'command' parameter"))?;

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
        // backend is "none" or "restricted-host" (no container runtime),
        // commands run directly on the host even when the session mode says
        // "sandboxed".  Using /home/sandbox as the working directory would
        // fail with ENOENT on the host, so we must fall back to the host
        // data directory.
        let has_container_backend = if let Some(ref router) = self.sandbox_router {
            !matches!(router.backend_name(), "none" | "restricted-host")
        } else {
            !matches!(self.sandbox.backend_name(), "none" | "restricted-host")
        };

        // Resolve working directory.  When sandboxed *with a real container
        // backend* the host CWD doesn't exist inside the container, so default
        // to "/home/sandbox".  When running on the host (no container), default
        // to $HOME so the LLM operates in a familiar location.
        let explicit_working_dir = params
            .get("working_dir")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .or_else(|| self.working_dir.clone());

        let runs_on_host = !(is_sandboxed && has_container_backend);
        let host_default_dir = || moltis_config::home_dir().unwrap_or_else(moltis_config::data_dir);

        // When running on the host, validate that the explicit working dir
        // actually exists — the LLM may remember a container path like
        // /home/sandbox from an earlier sandboxed run.
        let validated_explicit = if runs_on_host {
            match explicit_working_dir {
                Some(ref dir) if dir.is_dir() => explicit_working_dir,
                Some(ref dir) => {
                    debug!(
                        path = %dir.display(),
                        "explicit working_dir does not exist on host, using default"
                    );
                    None
                },
                None => None,
            }
        } else {
            match explicit_working_dir {
                // Relative paths are resolved under the sandbox home.
                Some(ref dir) if !dir.is_absolute() => {
                    Some(PathBuf::from("/home/sandbox").join(dir))
                },
                // Absolute paths are only allowed inside the sandbox home.
                Some(ref dir) if dir.starts_with("/home/sandbox") => explicit_working_dir,
                Some(ref dir) => {
                    debug!(
                        path = %dir.display(),
                        "explicit working_dir is outside /home/sandbox while sandboxed, using default"
                    );
                    None
                },
                None => None,
            }
        };

        let using_default_working_dir = validated_explicit.is_none();
        let mut working_dir = validated_explicit.or_else(|| {
            if !runs_on_host {
                Some(PathBuf::from("/home/sandbox"))
            } else {
                Some(host_default_dir())
            }
        });

        // Ensure default host working directory exists so command spawning does
        // not fail on fresh machines where $HOME has not been created yet.
        if runs_on_host
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
                        return Err(
                            Error::message(format!("command denied by user: {command}")).into()
                        );
                    },
                    ApprovalDecision::Timeout => {
                        return Err(Error::message(format!(
                            "approval timed out for command: {command}"
                        ))
                        .into());
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
                let announce_prepare = router.mark_preparing_once(sk).await;
                if announce_prepare {
                    router.emit_event(crate::sandbox::SandboxEvent::Preparing {
                        session_key: sk.to_string(),
                        backend: backend.backend_name().to_string(),
                        image: image.clone(),
                    });
                }

                if let Err(error) = backend.ensure_ready(&id, Some(&image)).await {
                    if announce_prepare {
                        router.clear_prepared_session(sk).await;
                        router.emit_event(crate::sandbox::SandboxEvent::PrepareFailed {
                            session_key: sk.to_string(),
                            backend: backend.backend_name().to_string(),
                            image: image.clone(),
                            error: error.to_string(),
                        });
                    }
                    return Err(error.into());
                }

                if announce_prepare {
                    router.emit_event(crate::sandbox::SandboxEvent::Prepared {
                        session_key: sk.to_string(),
                        backend: backend.backend_name().to_string(),
                        image: image.clone(),
                    });
                }
                debug!(session = sk, sandbox_id = %id, command, "sandbox running command");
                let mut sandbox_result = backend.exec(&id, command, &opts).await?;
                for retry_idx in 1..=MAX_SANDBOX_RECOVERY_RETRIES {
                    if sandbox_result.exit_code == 0
                        || !is_container_not_running_exec_error(&sandbox_result.stderr)
                    {
                        break;
                    }

                    warn!(
                        session = sk,
                        sandbox_id = %id,
                        command,
                        retry_idx,
                        max_retries = MAX_SANDBOX_RECOVERY_RETRIES,
                        "sandbox exec failed because container is unavailable, reinitializing and retrying"
                    );
                    if let Err(error) = backend.cleanup(&id).await {
                        warn!(
                            session = sk,
                            sandbox_id = %id,
                            %error,
                            "failed to clean up stale sandbox before retry, continuing"
                        );
                    }
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
            for retry_idx in 1..=MAX_SANDBOX_RECOVERY_RETRIES {
                if sandbox_result.exit_code == 0
                    || !is_container_not_running_exec_error(&sandbox_result.stderr)
                {
                    break;
                }

                warn!(
                    sandbox_id = %id,
                    command,
                    retry_idx,
                    max_retries = MAX_SANDBOX_RECOVERY_RETRIES,
                    "sandbox exec failed because container is unavailable, reinitializing and retrying"
                );
                if let Err(error) = self.sandbox.cleanup(id).await {
                    warn!(
                        sandbox_id = %id,
                        %error,
                        "failed to clean up stale sandbox before retry, continuing"
                    );
                }
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

        // Fire completion callback (used to enqueue heartbeat events).
        if let Some(ref cb) = self.completion_callback {
            let preview_len = 200;
            let stdout_preview = result.stdout.chars().take(preview_len).collect();
            let stderr_preview = result.stderr.chars().take(preview_len).collect();
            cb(ExecCompletionEvent {
                command: command.to_string(),
                exit_code: result.exit_code,
                stdout_preview,
                stderr_preview,
            });
        }

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
    let lower = stderr.to_ascii_lowercase();
    lower.contains("cannot exec: container is not running")
        || lower.contains("container is stopped")
        || (lower.contains("no sandbox client exists") && lower.contains("container is stopped"))
        || (lower.contains("failed to create process in container")
            && lower.contains("container")
            && lower.contains("not running"))
        || (lower.contains("invalidstate")
            && lower.contains("container")
            && lower.contains("is not running"))
        || (lower.contains("container")
            && lower.contains("not running")
            && lower.contains("failed to create process"))
        || lower.contains("notfound")
        || (lower.contains("not found") && lower.contains("container"))
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {
        super::*,
        std::sync::atomic::{AtomicBool, AtomicUsize, Ordering},
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

    #[test]
    fn truncate_output_for_display_handles_multibyte_boundary() {
        let mut output = format!("{}л{}", "a".repeat(1999), "z".repeat(10));
        truncate_output_for_display(&mut output, 2000);
        assert!(output.contains("[output truncated]"));
        assert!(!output.contains('л'));
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

        async fn exec(
            &self,
            _id: &SandboxId,
            _command: &str,
            _opts: &ExecOpts,
        ) -> Result<ExecResult> {
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

        async fn exec(
            &self,
            _id: &SandboxId,
            _command: &str,
            opts: &ExecOpts,
        ) -> Result<ExecResult> {
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
}
