//! Route command execution to a remote node via `node.invoke` with `system.run`.
//!
//! When `tools.exec.host = "node"`, the gateway forwards shell commands to a
//! connected headless node instead of executing them locally or in a sandbox.

use std::{collections::HashMap, sync::Arc, time::Duration};

use {
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
};

use crate::state::GatewayState;

/// Result of a remote command execution on a node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Environment variables that are safe to forward to a remote node.
const SAFE_ENV_ALLOWLIST: &[&str] = &["TERM", "LANG", "COLORTERM", "NO_COLOR", "FORCE_COLOR"];

/// Environment variable prefixes that are safe to forward.
const SAFE_ENV_PREFIX_ALLOWLIST: &[&str] = &["LC_"];

/// Environment variable patterns that must NEVER be forwarded to a remote node.
const BLOCKED_ENV_PREFIXES: &[&str] = &[
    "DYLD_",
    "LD_",
    "NODE_OPTIONS",
    "PYTHON",
    "PERL",
    "RUBYOPT",
    "SHELLOPTS",
    "PS4",
    // Security-sensitive keys
    "MOLTIS_",
    "OPENAI_",
    "ANTHROPIC_",
    "AWS_",
    "GOOGLE_",
    "AZURE_",
];

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

/// Filter environment variables to the safe allowlist.
fn filter_env(env: &HashMap<String, String>) -> HashMap<String, String> {
    env.iter()
        .filter(|(key, _)| is_safe_env(key))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn is_safe_env(key: &str) -> bool {
    // Block dangerous prefixes first.
    for prefix in BLOCKED_ENV_PREFIXES {
        if key.starts_with(prefix) {
            return false;
        }
    }

    // Allow exact matches.
    if SAFE_ENV_ALLOWLIST.contains(&key) {
        return true;
    }

    // Allow prefix matches.
    for prefix in SAFE_ENV_PREFIX_ALLOWLIST {
        if key.starts_with(prefix) {
            return true;
        }
    }

    false
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
}

impl GatewayNodeExecProvider {
    pub fn new(state: Arc<GatewayState>) -> Self {
        Self { state }
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
        let result = exec_on_node(&self.state, node_id, command, timeout_secs, cwd, env).await?;
        Ok(moltis_tools::exec::ExecResult {
            stdout: result.stdout,
            stderr: result.stderr,
            exit_code: result.exit_code,
        })
    }

    async fn resolve_node_id(&self, node_ref: &str) -> Option<String> {
        resolve_node_id(&self.state, node_ref).await
    }
}

// ── Node info provider ──────────────────────────────────────────────────────

/// Convert a `NodeSession` into a serializable `NodeInfo`.
fn node_to_info(n: &crate::nodes::NodeSession) -> moltis_tools::nodes::NodeInfo {
    moltis_tools::nodes::NodeInfo {
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
            .map(|p| moltis_tools::nodes::NodeProviderInfo {
                provider: p.provider.clone(),
                models: p.models.clone(),
            })
            .collect(),
    }
}

/// Bridge that implements [`moltis_tools::nodes::NodeInfoProvider`] by
/// reading from the `NodeRegistry` and session metadata in `GatewayState`.
pub struct GatewayNodeInfoProvider {
    state: Arc<GatewayState>,
}

impl GatewayNodeInfoProvider {
    pub fn new(state: Arc<GatewayState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl moltis_tools::nodes::NodeInfoProvider for GatewayNodeInfoProvider {
    async fn list_nodes(&self) -> Vec<moltis_tools::nodes::NodeInfo> {
        let inner = self.state.inner.read().await;
        inner.nodes.list().iter().map(|n| node_to_info(n)).collect()
    }

    async fn describe_node(&self, node_ref: &str) -> Option<moltis_tools::nodes::NodeInfo> {
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
                let id = resolve_node_id(&self.state, r)
                    .await
                    .ok_or_else(|| anyhow::anyhow!("node '{r}' not found or not connected"))?;
                Some(id)
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
    fn parse_error_result() {
        let value = serde_json::json!({
            "error": "command not found",
        });
        let result = parse_exec_result(&value);
        assert!(result.is_err());
    }
}
