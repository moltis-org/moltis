//! Route command execution to a remote node or SSH target.
//!
//! When `tools.exec.host = "node"`, the gateway forwards shell commands to a
//! connected headless node via `node.invoke`. When `tools.exec.host = "ssh"`,
//! it forwards commands through the system `ssh` client using a configured
//! target alias or `user@host`.

use std::{
    collections::HashMap,
    io::Write,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use {
    async_trait::async_trait,
    secrecy::ExposeSecret,
    tokio::{io::AsyncReadExt, process::Command},
    tracing::warn,
};

use crate::{
    auth::{CredentialStore, SshAuthMode, SshResolvedTarget},
    state::GatewayState,
};

use moltis_tools::nodes::{NodeInfo, NodeInfoProvider, NodeProviderInfo};

/// Result of a remote command execution on a node.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct NodeExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Environment variables that are safe to forward to a remote node.
pub const SAFE_ENV_ALLOWLIST: &[&str] = &["TERM", "LANG", "COLORTERM", "NO_COLOR", "FORCE_COLOR"];

/// Environment variable prefixes that are safe to forward.
pub const SAFE_ENV_PREFIX_ALLOWLIST: &[&str] = &["LC_"];

/// Environment variable patterns that must NEVER be forwarded to a remote node.
pub const BLOCKED_ENV_PREFIXES: &[&str] = &[
    "DYLD_",
    "LD_",
    "NODE_OPTIONS",
    "PYTHON",
    "PERL",
    "RUBYOPT",
    "SHELLOPTS",
    "PS4",
    "MOLTIS_",
    "OPENAI_",
    "ANTHROPIC_",
    "AWS_",
    "GOOGLE_",
    "AZURE_",
];

/// SSH node ID prefix.
pub const SSH_ID_PREFIX: &str = "ssh:";

/// SSH target ID prefix.
pub const SSH_TARGET_ID_PREFIX: &str = "ssh:target:";

pub fn ssh_node_id(target: &str) -> String {
    format!("{SSH_ID_PREFIX}{target}")
}

pub fn ssh_stored_node_id(id: i64) -> String {
    format!("{SSH_TARGET_ID_PREFIX}{id}")
}

pub fn ssh_target_matches(node_ref: &str, target: &str) -> bool {
    node_ref == "ssh" || node_ref == target || node_ref.strip_prefix(SSH_ID_PREFIX) == Some(target)
}

pub fn filter_env(env: &HashMap<String, String>) -> HashMap<String, String> {
    env.iter()
        .filter(|(key, _)| is_safe_env(key) && is_valid_env_key(key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

pub fn is_safe_env(key: &str) -> bool {
    for prefix in BLOCKED_ENV_PREFIXES {
        if key.starts_with(prefix) {
            return false;
        }
    }

    if SAFE_ENV_ALLOWLIST.contains(&key) {
        return true;
    }

    for prefix in SAFE_ENV_PREFIX_ALLOWLIST {
        if key.starts_with(prefix) {
            return true;
        }
    }

    false
}

pub fn is_valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(ch) if ch.is_ascii_alphabetic() || ch == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

pub(crate) fn ssh_node_info(target: &str) -> NodeInfo {
    NodeInfo {
        node_id: ssh_node_id(target),
        display_name: Some(format!("SSH: {target}")),
        platform: "ssh".to_string(),
        capabilities: vec!["system.run".to_string()],
        commands: vec!["system.run".to_string()],
        remote_ip: None,
        mem_total: None,
        mem_available: None,
        cpu_count: None,
        cpu_usage: None,
        uptime_secs: None,
        services: vec!["ssh".to_string()],
        telemetry_stale: false,
        disk_total: None,
        disk_available: None,
        runtimes: Vec::new(),
        providers: Vec::new(),
    }
}

fn ssh_target_node_info(target: &SshResolvedTarget) -> NodeInfo {
    let auth_service = match target.auth_mode {
        SshAuthMode::System => "ssh-system",
        SshAuthMode::Managed => "ssh-managed",
    };
    NodeInfo {
        node_id: ssh_stored_node_id(target.id),
        display_name: Some(format!("SSH: {}", target.label)),
        platform: "ssh".to_string(),
        capabilities: vec!["system.run".to_string()],
        commands: vec!["system.run".to_string()],
        remote_ip: None,
        mem_total: None,
        mem_available: None,
        cpu_count: None,
        cpu_usage: None,
        uptime_secs: None,
        services: vec!["ssh".to_string(), auth_service.to_string()],
        telemetry_stale: false,
        disk_total: None,
        disk_available: None,
        runtimes: Vec::new(),
        providers: Vec::new(),
    }
}

/// Forward a shell command to a connected node for execution.
///
/// Uses `node.invoke` internally with `system.run` as the command.
/// Returns the stdout/stderr/exit_code from the remote execution.
pub async fn exec_on_node(
    state: &Arc<GatewayState>,
    node_id: &str,
    command: &str,
    timeout_secs: u64,
    cwd: Option<&str>,
    env: Option<&HashMap<String, String>>,
) -> anyhow::Result<NodeExecResult> {
    // Build the args for system.run.
    let mut args = serde_json::json!({
        "command": command,
        "timeout": timeout_secs * 1000, // ms
    });

    if let Some(cwd) = cwd {
        args["cwd"] = serde_json::json!(cwd);
    }

    // Filter env to safe allowlist.
    if let Some(env_map) = env {
        let filtered = filter_env(env_map);
        if !filtered.is_empty() {
            args["env"] = serde_json::to_value(filtered)?;
        }
    }

    // Look up node connection.
    let conn_id = {
        let inner = state.inner.read().await;
        let node = inner
            .nodes
            .get(node_id)
            .ok_or_else(|| anyhow::anyhow!("node '{node_id}' not connected"))?;
        node.conn_id.clone()
    };

    // Build and send the invoke request.
    let invoke_id = uuid::Uuid::new_v4().to_string();
    let invoke_event = moltis_protocol::EventFrame::new(
        "node.invoke.request",
        serde_json::json!({
            "invokeId": invoke_id,
            "command": "system.run",
            "args": args,
        }),
        state.next_seq(),
    );
    let event_json = serde_json::to_string(&invoke_event)?;

    {
        let inner = state.inner.read().await;
        let node_client = inner
            .clients
            .get(&conn_id)
            .ok_or_else(|| anyhow::anyhow!("node connection lost"))?;
        if !node_client.send(&event_json) {
            anyhow::bail!("failed to send invoke to node");
        }
    }

    // Register the pending invoke and wait for result.
    let (tx, rx) = tokio::sync::oneshot::channel();
    {
        let mut inner = state.inner.write().await;
        inner
            .pending_invokes
            .insert(invoke_id.clone(), crate::state::PendingInvoke {
                request_id: invoke_id.clone(),
                sender: tx,
                created_at: std::time::Instant::now(),
            });
    }

    let timeout = Duration::from_secs(timeout_secs.max(5));
    let result = match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(value)) => value,
        Ok(Err(_)) => {
            anyhow::bail!("node invoke cancelled");
        },
        Err(_) => {
            state.inner.write().await.pending_invokes.remove(&invoke_id);
            anyhow::bail!("node invoke timeout after {timeout_secs}s");
        },
    };

    // Parse the result.
    parse_exec_result(&result)
}

async fn exec_over_ssh(
    target: &str,
    port: Option<u16>,
    identity_file: Option<&Path>,
    known_host: Option<&str>,
    command: &str,
    timeout_secs: u64,
    cwd: Option<&str>,
    env: Option<&HashMap<String, String>>,
    max_output_bytes: usize,
) -> anyhow::Result<NodeExecResult> {
    let known_hosts_file = if let Some(known_host) = known_host {
        let mut file = tempfile::NamedTempFile::new()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(known_host.as_bytes())?;
        file.write_all(b"\n")?;
        file.flush()?;
        Some(file)
    } else {
        None
    };
    let mut ssh = Command::new("ssh");
    ssh.arg("-T")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg(format!("ConnectTimeout={}", timeout_secs.clamp(5, 30)));
    if let Some(known_hosts_file) = known_hosts_file.as_ref() {
        ssh.arg("-o").arg("StrictHostKeyChecking=yes");
        ssh.arg("-o").arg(format!(
            "UserKnownHostsFile={}",
            ssh_config_quote_path(known_hosts_file.path())
        ));
        ssh.arg("-o").arg("GlobalKnownHostsFile=/dev/null");
    }
    if let Some(identity_file) = identity_file {
        ssh.arg("-o").arg("IdentitiesOnly=yes");
        ssh.arg("-i").arg(identity_file);
    }
    if let Some(port) = port {
        ssh.arg("-p").arg(port.to_string());
    }
    let remote_command = format!(
        "sh -lc {}",
        shell_single_quote(&build_remote_shell_script(command, cwd, env))
    );
    for arg in ssh_destination_args(target, remote_command) {
        ssh.arg(arg);
    }
    ssh.stdout(std::process::Stdio::piped());
    ssh.stderr(std::process::Stdio::piped());
    ssh.stdin(std::process::Stdio::null());

    let mut child = ssh.spawn()?;
    let stdout_task = child.stdout.take().map(spawn_pipe_reader);
    let stderr_task = child.stderr.take().map(spawn_pipe_reader);
    let status =
        match tokio::time::timeout(Duration::from_secs(timeout_secs.max(5)), child.wait()).await {
            Ok(result) => result?,
            Err(_) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                let _ = read_pipe_task(stdout_task).await;
                let _ = read_pipe_task(stderr_task).await;
                anyhow::bail!("ssh execution timed out after {timeout_secs}s");
            },
        };

    let stdout = read_pipe_task(stdout_task).await?;
    let stderr = read_pipe_task(stderr_task).await?;
    let mut stdout = String::from_utf8_lossy(&stdout).into_owned();
    let mut stderr = String::from_utf8_lossy(&stderr).into_owned();
    truncate_output_for_display(&mut stdout, max_output_bytes);
    truncate_output_for_display(&mut stderr, max_output_bytes);

    Ok(NodeExecResult {
        stdout,
        stderr,
        exit_code: status.code().unwrap_or(-1),
    })
}

fn spawn_pipe_reader<R>(mut reader: R) -> tokio::task::JoinHandle<std::io::Result<Vec<u8>>>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        Ok(bytes)
    })
}

async fn read_pipe_task(
    task: Option<tokio::task::JoinHandle<std::io::Result<Vec<u8>>>>,
) -> anyhow::Result<Vec<u8>> {
    match task {
        Some(task) => Ok(task.await??),
        None => Ok(Vec::new()),
    }
}

fn write_temp_ssh_private_key(
    private_key: &secrecy::Secret<String>,
) -> anyhow::Result<tempfile::NamedTempFile> {
    let mut file = tempfile::NamedTempFile::new()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(private_key.expose_secret().as_bytes())?;
    file.flush()?;
    Ok(file)
}

pub async fn exec_resolved_ssh_target(
    credential_store: &CredentialStore,
    target: &SshResolvedTarget,
    command: &str,
    timeout_secs: u64,
    cwd: Option<&str>,
    env: Option<&HashMap<String, String>>,
    max_output_bytes: usize,
) -> anyhow::Result<NodeExecResult> {
    match target.auth_mode {
        SshAuthMode::System => {
            exec_over_ssh(
                &target.target,
                target.port,
                None,
                target.known_host.as_deref(),
                command,
                timeout_secs,
                cwd,
                env,
                max_output_bytes,
            )
            .await
        },
        SshAuthMode::Managed => {
            let key_id = target
                .key_id
                .ok_or_else(|| anyhow::anyhow!("managed ssh target has no key configured"))?;
            let private_key = credential_store
                .get_ssh_private_key(key_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("ssh key {key_id} not found"))?;
            let temp_key = write_temp_ssh_private_key(&private_key)?;
            exec_over_ssh(
                &target.target,
                target.port,
                Some(temp_key.path()),
                target.known_host.as_deref(),
                command,
                timeout_secs,
                cwd,
                env,
                max_output_bytes,
            )
            .await
        },
    }
}

/// Query a node for its available LLM providers via `system.providers`.
pub async fn query_node_providers(
    state: &Arc<GatewayState>,
    node_id: &str,
) -> anyhow::Result<Vec<crate::nodes::NodeProviderEntry>> {
    // Find the node's conn_id.
    let conn_id = {
        let inner = state.inner.read().await;
        let node = inner
            .nodes
            .get(node_id)
            .ok_or_else(|| anyhow::anyhow!("node '{node_id}' not connected"))?;
        node.conn_id.clone()
    };

    let invoke_id = uuid::Uuid::new_v4().to_string();
    let invoke_event = moltis_protocol::EventFrame::new(
        "node.invoke.request",
        serde_json::json!({
            "invokeId": invoke_id,
            "command": "system.providers",
            "args": {},
        }),
        state.next_seq(),
    );
    let event_json = serde_json::to_string(&invoke_event)?;

    {
        let inner = state.inner.read().await;
        let node_client = inner
            .clients
            .get(&conn_id)
            .ok_or_else(|| anyhow::anyhow!("node connection lost"))?;
        if !node_client.send(&event_json) {
            anyhow::bail!("failed to send providers invoke to node");
        }
    }

    let (tx, rx) = tokio::sync::oneshot::channel();
    {
        let mut inner = state.inner.write().await;
        inner
            .pending_invokes
            .insert(invoke_id.clone(), crate::state::PendingInvoke {
                request_id: invoke_id.clone(),
                sender: tx,
                created_at: std::time::Instant::now(),
            });
    }

    let result = match tokio::time::timeout(Duration::from_secs(10), rx).await {
        Ok(Ok(value)) => value,
        Ok(Err(_)) => anyhow::bail!("providers invoke cancelled"),
        Err(_) => {
            state.inner.write().await.pending_invokes.remove(&invoke_id);
            anyhow::bail!("providers invoke timeout");
        },
    };

    // Parse the result.
    let providers_arr = result
        .get("providers")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let entries = providers_arr
        .into_iter()
        .filter_map(|p| {
            let provider = p.get("provider")?.as_str()?.to_string();
            let models = p
                .get("models")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            Some(crate::nodes::NodeProviderEntry { provider, models })
        })
        .collect();

    Ok(entries)
}

/// Resolve a node identifier (id or display name) to a node_id.
pub async fn resolve_node_id(state: &Arc<GatewayState>, node_ref: &str) -> Option<String> {
    let inner = state.inner.read().await;

    // Try direct id match first.
    if inner.nodes.get(node_ref).is_some() {
        return Some(node_ref.to_string());
    }

    // Try display name match (case-insensitive).
    let lower = node_ref.to_lowercase();
    for node in inner.nodes.list() {
        if let Some(name) = &node.display_name
            && name.to_lowercase() == lower
        {
            return Some(node.node_id.clone());
        }
    }

    None
}

fn truncate_output_for_display(output: &mut String, max_output_bytes: usize) {
    if output.len() <= max_output_bytes {
        return;
    }
    output.truncate(output.floor_char_boundary(max_output_bytes));
    output.push_str("\n... [output truncated]");
}

fn build_remote_shell_script(
    command: &str,
    cwd: Option<&str>,
    env: Option<&HashMap<String, String>>,
) -> String {
    let mut parts = Vec::new();

    if let Some(cwd) = cwd {
        parts.push(format!("cd {}", shell_single_quote(cwd)));
    }

    if let Some(env) = env {
        let filtered = filter_env(env);
        let mut keys: Vec<&String> = filtered.keys().collect();
        keys.sort();
        for key in keys {
            if let Some(value) = filtered.get(key) {
                parts.push(format!("export {}={}", key, shell_single_quote(value)));
            }
        }
    }

    parts.push(command.to_string());
    parts.join(" && ")
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn ssh_destination_args(target: &str, remote_command: String) -> [String; 3] {
    ["--".to_string(), target.to_string(), remote_command]
}

fn ssh_config_quote_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn parse_exec_result(value: &serde_json::Value) -> anyhow::Result<NodeExecResult> {
    // Try structured result first.
    if let Some(stdout) = value.get("stdout").and_then(|v| v.as_str()) {
        return Ok(NodeExecResult {
            stdout: stdout.to_string(),
            stderr: value
                .get("stderr")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            exit_code: value.get("exitCode").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
        });
    }

    // Check for error.
    if let Some(error) = value.get("error").and_then(|v| v.as_str()) {
        anyhow::bail!("node exec error: {error}");
    }

    // Return the raw value as stdout.
    Ok(NodeExecResult {
        stdout: value.to_string(),
        stderr: String::new(),
        exit_code: 0,
    })
}

/// Bridge that implements [`moltis_tools::exec::NodeExecProvider`] by
/// delegating to [`exec_on_node`] / [`resolve_node_id`] with a shared
/// `GatewayState`.
pub struct GatewayNodeExecProvider {
    state: Arc<GatewayState>,
    node_count: Arc<AtomicUsize>,
    ssh_target_count: Arc<AtomicUsize>,
    legacy_ssh_target: Option<String>,
    max_output_bytes: usize,
}

impl GatewayNodeExecProvider {
    /// Create with the shared node counter from `GatewayState` so that
    /// `has_connected_nodes()` reflects the real connection state.
    pub fn new(
        state: Arc<GatewayState>,
        node_count: Arc<AtomicUsize>,
        ssh_target_count: Arc<AtomicUsize>,
        legacy_ssh_target: Option<String>,
        max_output_bytes: usize,
    ) -> Self {
        Self {
            state,
            node_count,
            ssh_target_count,
            legacy_ssh_target,
            max_output_bytes,
        }
    }
}

#[async_trait]
impl moltis_tools::exec::NodeExecProvider for GatewayNodeExecProvider {
    async fn exec_on_node(
        &self,
        node_id: &str,
        command: &str,
        timeout_secs: u64,
        cwd: Option<&str>,
        env: Option<&HashMap<String, String>>,
    ) -> anyhow::Result<moltis_tools::exec::ExecResult> {
        if node_id.starts_with(SSH_ID_PREFIX) {
            if let Some(store) = self.state.credential_store.as_ref()
                && let Some(target) = store.resolve_ssh_target(node_id).await?
            {
                let result = exec_resolved_ssh_target(
                    store,
                    &target,
                    command,
                    timeout_secs,
                    cwd,
                    env,
                    self.max_output_bytes,
                )
                .await?;
                return Ok(moltis_tools::exec::ExecResult {
                    stdout: result.stdout,
                    stderr: result.stderr,
                    exit_code: result.exit_code,
                });
            }

            if let Some(target) = node_id.strip_prefix(SSH_ID_PREFIX) {
                let result = exec_over_ssh(
                    target,
                    None,
                    None,
                    None,
                    command,
                    timeout_secs,
                    cwd,
                    env,
                    self.max_output_bytes,
                )
                .await?;
                return Ok(moltis_tools::exec::ExecResult {
                    stdout: result.stdout,
                    stderr: result.stderr,
                    exit_code: result.exit_code,
                });
            }
        }

        let result = exec_on_node(&self.state, node_id, command, timeout_secs, cwd, env).await?;
        Ok(moltis_tools::exec::ExecResult {
            stdout: result.stdout,
            stderr: result.stderr,
            exit_code: result.exit_code,
        })
    }

    async fn resolve_node_id(&self, node_ref: &str) -> Option<String> {
        if let Some(store) = self.state.credential_store.as_ref() {
            match store.resolve_ssh_target(node_ref).await {
                Ok(Some(target)) => return Some(target.node_id),
                Ok(None) => {},
                Err(error) => warn!(%error, node_ref, "failed to resolve managed ssh target"),
            }
        }

        if let Some(target) = &self.legacy_ssh_target
            && ssh_target_matches(node_ref, target)
        {
            return Some(ssh_node_id(target));
        }

        resolve_node_id(&self.state, node_ref).await
    }

    fn has_connected_nodes(&self) -> bool {
        self.node_count.load(Ordering::Relaxed) > 0
            || self.ssh_target_count.load(Ordering::Relaxed) > 0
            || self.legacy_ssh_target.is_some()
    }

    async fn default_node_ref(&self) -> Option<String> {
        if let Some(store) = self.state.credential_store.as_ref() {
            match store.get_default_ssh_target().await {
                Ok(Some(target)) => return Some(target.node_id),
                Ok(None) => {},
                Err(error) => warn!(%error, "failed to load default ssh target"),
            }
        }

        self.legacy_ssh_target
            .as_ref()
            .map(|target| ssh_node_id(target))
    }
}

// ── Node info provider ──────────────────────────────────────────────────────

/// Convert a `NodeSession` into a serializable `NodeInfo`.
fn node_to_info(n: &crate::nodes::NodeSession) -> NodeInfo {
    NodeInfo {
        node_id: n.node_id.clone(),
        display_name: n.display_name.clone(),
        platform: n.platform.clone(),
        capabilities: n.capabilities.clone(),
        commands: n.commands.clone(),
        remote_ip: n.remote_ip.clone(),
        mem_total: n.mem_total,
        mem_available: n.mem_available,
        cpu_count: n.cpu_count,
        cpu_usage: n.cpu_usage,
        uptime_secs: n.uptime_secs,
        services: n.services.clone(),
        telemetry_stale: n
            .last_telemetry
            .is_some_and(|t| t.elapsed() > Duration::from_secs(120)),
        disk_total: n.disk_total,
        disk_available: n.disk_available,
        runtimes: n.runtimes.clone(),
        providers: n
            .providers
            .iter()
            .map(|p| NodeProviderInfo {
                provider: p.provider.clone(),
                models: p.models.clone(),
            })
            .collect(),
    }
}

/// Bridge that implements [`NodeInfoProvider`] by
/// reading from the `NodeRegistry` and session metadata in `GatewayState`.
pub struct GatewayNodeInfoProvider {
    state: Arc<GatewayState>,
    legacy_ssh_target: Option<String>,
}

impl GatewayNodeInfoProvider {
    pub fn new(state: Arc<GatewayState>, legacy_ssh_target: Option<String>) -> Self {
        Self {
            state,
            legacy_ssh_target,
        }
    }
}

#[async_trait]
impl NodeInfoProvider for GatewayNodeInfoProvider {
    async fn list_nodes(&self) -> Vec<NodeInfo> {
        let inner = self.state.inner.read().await;
        let mut nodes: Vec<_> = inner.nodes.list().iter().map(|n| node_to_info(n)).collect();
        drop(inner);

        if let Some(store) = self.state.credential_store.as_ref() {
            match store.list_ssh_targets().await {
                Ok(targets) => {
                    for target in targets {
                        nodes.push(ssh_target_node_info(&SshResolvedTarget {
                            id: target.id,
                            node_id: ssh_stored_node_id(target.id),
                            label: target.label,
                            target: target.target,
                            port: target.port,
                            known_host: target.known_host,
                            auth_mode: target.auth_mode,
                            key_id: target.key_id,
                            key_name: target.key_name,
                        }));
                    }
                },
                Err(error) => warn!(%error, "failed to list managed ssh targets"),
            }
        }

        if let Some(target) = &self.legacy_ssh_target
            && !nodes.iter().any(|node| node.node_id == ssh_node_id(target))
        {
            nodes.push(ssh_node_info(target));
        }
        nodes
    }

    async fn describe_node(&self, node_ref: &str) -> Option<NodeInfo> {
        if let Some(store) = self.state.credential_store.as_ref() {
            match store.resolve_ssh_target(node_ref).await {
                Ok(Some(target)) => return Some(ssh_target_node_info(&target)),
                Ok(None) => {},
                Err(error) => warn!(%error, node_ref, "failed to describe managed ssh target"),
            }
        }

        if let Some(target) = &self.legacy_ssh_target
            && ssh_target_matches(node_ref, target)
        {
            return Some(ssh_node_info(target));
        }
        let resolved = resolve_node_id(&self.state, node_ref).await?;
        let inner = self.state.inner.read().await;
        inner.nodes.get(&resolved).map(node_to_info)
    }

    async fn set_session_node(
        &self,
        session_key: &str,
        node_ref: Option<&str>,
    ) -> anyhow::Result<Option<String>> {
        let resolved = match node_ref {
            Some(r) => {
                if let Some(store) = self.state.credential_store.as_ref()
                    && let Some(target) = store.resolve_ssh_target(r).await?
                {
                    Some(target.node_id)
                } else if self
                    .legacy_ssh_target
                    .as_deref()
                    .is_some_and(|target| ssh_target_matches(r, target))
                {
                    self.legacy_ssh_target
                        .as_ref()
                        .map(|target| ssh_node_id(target))
                } else {
                    let id = resolve_node_id(&self.state, r)
                        .await
                        .ok_or_else(|| anyhow::anyhow!("node '{r}' not found or not connected"))?;
                    Some(id)
                }
            },
            None => None,
        };

        let meta = self
            .state
            .services
            .session_metadata
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("session metadata not available"))?;

        meta.upsert(session_key, None).await?;
        meta.set_node_id(session_key, resolved.as_deref()).await?;

        Ok(resolved)
    }

    async fn resolve_node_id(&self, node_ref: &str) -> Option<String> {
        if let Some(store) = self.state.credential_store.as_ref() {
            match store.resolve_ssh_target(node_ref).await {
                Ok(Some(target)) => return Some(target.node_id),
                Ok(None) => {},
                Err(error) => warn!(%error, node_ref, "failed to resolve managed ssh target"),
            }
        }

        if let Some(target) = &self.legacy_ssh_target
            && ssh_target_matches(node_ref, target)
        {
            return Some(ssh_node_id(target));
        }
        resolve_node_id(&self.state, node_ref).await
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn filter_env_safe_only() {
        let mut env = HashMap::new();
        env.insert("TERM".into(), "xterm-256color".into());
        env.insert("LANG".into(), "en_US.UTF-8".into());
        env.insert("LC_ALL".into(), "en_US.UTF-8".into());
        env.insert("LC_$(id)".into(), "en_US.UTF-8".into());
        env.insert("DYLD_INSERT_LIBRARIES".into(), "/evil.dylib".into());
        env.insert("LD_PRELOAD".into(), "/evil.so".into());
        env.insert("NODE_OPTIONS".into(), "--inspect".into());
        env.insert("OPENAI_API_KEY".into(), "sk-secret".into());
        env.insert("MOLTIS_AUTH_TOKEN".into(), "token".into());
        env.insert("CUSTOM_VAR".into(), "value".into());

        let filtered = filter_env(&env);
        assert!(filtered.contains_key("TERM"));
        assert!(filtered.contains_key("LANG"));
        assert!(filtered.contains_key("LC_ALL"));
        assert!(!filtered.contains_key("LC_$(id)"));
        assert!(!filtered.contains_key("DYLD_INSERT_LIBRARIES"));
        assert!(!filtered.contains_key("LD_PRELOAD"));
        assert!(!filtered.contains_key("NODE_OPTIONS"));
        assert!(!filtered.contains_key("OPENAI_API_KEY"));
        assert!(!filtered.contains_key("MOLTIS_AUTH_TOKEN"));
        assert!(!filtered.contains_key("CUSTOM_VAR"));
    }

    #[test]
    fn parse_structured_result() {
        let value = serde_json::json!({
            "stdout": "hello\n",
            "stderr": "",
            "exitCode": 0,
        });
        let result = parse_exec_result(&value).unwrap();
        assert_eq!(result.stdout, "hello\n");
        assert_eq!(result.stderr, "");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn ssh_target_matching_accepts_aliases() {
        assert!(ssh_target_matches("ssh", "deploy@box"));
        assert!(ssh_target_matches("deploy@box", "deploy@box"));
        assert!(ssh_target_matches("ssh:deploy@box", "deploy@box"));
        assert!(!ssh_target_matches("other", "deploy@box"));
    }

    #[test]
    fn ssh_node_info_uses_canonical_id() {
        let info = ssh_node_info("deploy@box");
        assert_eq!(info.node_id, "ssh:deploy@box");
        assert_eq!(info.display_name.as_deref(), Some("SSH: deploy@box"));
        assert_eq!(info.platform, "ssh");
        assert_eq!(info.capabilities, vec!["system.run".to_string()]);
        assert_eq!(info.services, vec!["ssh".to_string()]);
    }

    #[test]
    fn parse_error_result() {
        let value = serde_json::json!({
            "error": "command not found",
        });
        let result = parse_exec_result(&value);
        assert!(result.is_err());
    }

    #[test]
    fn build_remote_shell_script_quotes_cwd_and_env() {
        let mut env = HashMap::new();
        env.insert("LANG".into(), "en_US.UTF-8".into());
        env.insert("LC_$(id)".into(), "boom".into());
        env.insert("OPENAI_API_KEY".into(), "secret".into());

        let script = build_remote_shell_script("printf '%s' hi", Some("/tmp/it's"), Some(&env));
        assert!(script.contains("cd '/tmp/it'\"'\"'s'"));
        assert!(script.contains("export LANG='en_US.UTF-8'"));
        assert!(!script.contains("LC_$(id)"));
        assert!(!script.contains("OPENAI_API_KEY"));
        assert!(script.ends_with("printf '%s' hi"));
    }

    #[test]
    fn ssh_destination_args_insert_end_of_options_separator() {
        let args = ssh_destination_args("deploy@example.com", "sh -lc 'true'".to_string());
        assert_eq!(args, [
            "--".to_string(),
            "deploy@example.com".to_string(),
            "sh -lc 'true'".to_string()
        ]);
    }

    #[test]
    fn ssh_config_quote_path_wraps_and_escapes() {
        let path = Path::new("/tmp/ssh known_hosts\"file");
        assert_eq!(
            ssh_config_quote_path(path),
            "\"/tmp/ssh known_hosts\\\"file\""
        );
    }
}
