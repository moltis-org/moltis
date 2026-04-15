use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

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
    async fn broadcast_request(
        &self,
        request_id: &str,
        command: &str,
        session_key: Option<&str>,
    ) -> Result<()>;
}

/// Provider of environment variables to inject into sandbox execution.
/// Values are wrapped in `Secret` to prevent accidental logging.
#[async_trait]
pub trait EnvVarProvider: Send + Sync {
    async fn get_env_vars(&self) -> Vec<(String, secrecy::Secret<String>)>;
}

/// Provider that routes command execution to a remote node.
///
/// Implemented by the gateway crate to bridge `ExecTool` (in tools) with
/// `node_exec::exec_on_node` (in gateway) without a direct dependency.
#[async_trait]
pub trait NodeExecProvider: Send + Sync {
    /// Execute a shell command on a remote node.
    async fn exec_on_node(
        &self,
        node_id: &str,
        command: &str,
        timeout_secs: u64,
        cwd: Option<&str>,
        env: Option<&HashMap<String, String>>,
    ) -> anyhow::Result<ExecResult>;

    /// Resolve a node reference (id or display name) to a node_id.
    async fn resolve_node_id(&self, node_ref: &str) -> Option<String>;

    /// Whether any nodes are currently connected.  This is called from the
    /// sync `parameters_schema()` path so it must not block.
    fn has_connected_nodes(&self) -> bool;

    /// Return the current default remote target, if one exists.
    async fn default_node_ref(&self) -> Option<String>;
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
#[tracing::instrument(skip(opts), fields(timeout_secs = opts.timeout.as_secs()))]
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
    /// When set, commands are forwarded to a remote node instead of local exec.
    node_provider: Option<Arc<dyn NodeExecProvider>>,
    /// Default node id or display name (from `tools.exec.node` config).
    default_node: Option<String>,
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
            node_provider: None,
            default_node: None,
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

    /// Override the default command timeout.
    pub fn with_default_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }

    /// Override the maximum output bytes per command.
    pub fn with_max_output_bytes(mut self, max_bytes: usize) -> Self {
        self.max_output_bytes = max_bytes;
        self
    }

    /// Route command execution to a remote node instead of local/sandbox.
    pub fn with_node_provider(
        mut self,
        provider: Arc<dyn NodeExecProvider>,
        default_node: Option<String>,
    ) -> Self {
        self.node_provider = Some(provider);
        self.default_node = default_node;
        self
    }

    /// Check whether any remote nodes are currently connected.
    fn has_connected_nodes(&self) -> bool {
        self.node_provider
            .as_ref()
            .is_some_and(|p| p.has_connected_nodes())
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
        if self.has_connected_nodes() {
            "Execute a shell command on the server or a remote node. Returns stdout, stderr, and exit code."
        } else {
            "Execute a shell command on the server. Returns stdout, stderr, and exit code."
        }
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let timeout_default = self.default_timeout.as_secs();
        let mut properties = serde_json::json!({
            "command": {
                "type": "string",
                "description": "The shell command to execute"
            },
            "timeout": {
                "type": "integer",
                "description": format!("Timeout in seconds (default {timeout_default}, max 1800)")
            },
            "working_dir": {
                "type": "string",
                "description": "Working directory for the command"
            }
        });

        if self.has_connected_nodes()
            && let Some(obj) = properties.as_object_mut()
        {
            obj.insert(
                "node".to_string(),
                serde_json::json!({
                    "type": "string",
                    "description": "Node name or ID to run on. Omit to use the session's default node."
                }),
            );
        }

        serde_json::json!({
            "type": "object",
            "properties": properties,
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

        // Node execution: forward to a remote node if configured.
        // When a node is explicitly requested via param or a default is set, route
        // to that node. Otherwise fall through to local/sandbox execution.
        // Filter empty/whitespace-only strings — some models pass `""` when they
        // don't know what value to use, which should be treated as "not specified".
        let model_node = params
            .get("node")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(String::from);
        // Determine the effective node reference, distinguishing model-supplied
        // values from the admin-configured default.  When no nodes are connected:
        // - Model-hallucinated values are silently dropped (fall through to local).
        // - A configured `default_node` produces a clear error so the admin knows
        //   the intended remote host is unavailable.
        let node_ref = if let Some(provider) = &self.node_provider {
            if provider.has_connected_nodes() {
                match model_node.or_else(|| self.default_node.clone()) {
                    Some(node_ref) => Some(node_ref),
                    None => provider.default_node_ref().await,
                }
            } else if let Some(ref dn) = self.default_node {
                return Err(Error::message(format!(
                    "default node '{dn}' is configured but no nodes are currently connected"
                ))
                .into());
            } else {
                if model_node.is_some() {
                    debug!("ignoring model-supplied node parameter — no nodes are connected");
                }
                None
            }
        } else {
            None
        };
        if let (Some(provider), Some(node_ref)) = (&self.node_provider, &node_ref) {
            let node_id = provider.resolve_node_id(node_ref).await.ok_or_else(|| {
                Error::message(format!("node '{node_ref}' not found or not connected"))
            })?;

            let cwd = params.get("working_dir").and_then(|v| v.as_str());

            info!(
                command,
                node_id = %node_id,
                timeout_secs,
                "exec forwarding to remote node"
            );

            let result = provider
                .exec_on_node(&node_id, command, timeout_secs, cwd, None)
                .await
                .map_err(|e| Error::message(format!("node exec failed: {e}")))?;

            if let Some(ref cb) = self.completion_callback {
                let preview_len = 200;
                cb(ExecCompletionEvent {
                    command: command.to_string(),
                    exit_code: result.exit_code,
                    stdout_preview: result.stdout.chars().take(preview_len).collect(),
                    stderr_preview: result.stderr.chars().take(preview_len).collect(),
                });
            }

            return Ok(serde_json::to_value(&result)?);
        }

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
                let (req_id, rx) = mgr.create_request(command, session_key).await;

                // Broadcast to connected clients.
                if let Some(ref bc) = self.broadcaster
                    && let Err(e) = bc.broadcast_request(&req_id, command, session_key).await
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

#[cfg(test)]
mod tests;
