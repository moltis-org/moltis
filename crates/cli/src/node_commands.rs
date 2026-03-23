use {anyhow::Result, clap::Subcommand, std::time::Duration};

/// `moltis node` subcommands — manage remote nodes.
#[derive(Subcommand)]
pub enum NodeAction {
    /// Generate a device token so a remote machine can connect as a node.
    ///
    /// Prints the token and the `moltis node add` command to run on the
    /// remote machine. This is the CLI equivalent of clicking "Generate Token"
    /// on the web UI Nodes page.
    GenerateToken {
        /// Display name for the new device.
        #[arg(long)]
        name: Option<String>,
        /// Gateway HTTP URL (used to call the RPC).
        #[arg(long, default_value = "http://localhost:9090")]
        host: String,
        /// API key or password for authentication.
        #[arg(long, env = "MOLTIS_API_KEY")]
        api_key: Option<String>,
    },

    /// List all connected nodes.
    List {
        /// Gateway HTTP URL.
        #[arg(long, default_value = "http://localhost:9090")]
        host: String,
        /// API key or password for authentication.
        #[arg(long, env = "MOLTIS_API_KEY")]
        api_key: Option<String>,
    },

    /// Join this machine to a gateway as a node.
    ///
    /// Saves the connection parameters and installs an OS service (launchd on
    /// macOS, systemd on Linux) that starts on boot and reconnects on failure.
    /// Pass --foreground to run in the current terminal instead.
    Add {
        /// Gateway WebSocket URL (e.g. ws://your-server:9090/ws).
        #[arg(long, env = "MOLTIS_GATEWAY_URL")]
        host: String,
        /// Device token from `moltis node generate-token`.
        #[arg(long, env = "MOLTIS_DEVICE_TOKEN")]
        token: String,
        /// Display name for this node.
        #[arg(long)]
        name: Option<String>,
        /// Custom node ID (defaults to a random UUID).
        #[arg(long)]
        node_id: Option<String>,
        /// Working directory for command execution.
        #[arg(long)]
        working_dir: Option<String>,
        /// Maximum command timeout in seconds.
        #[arg(long, default_value = "300")]
        timeout: u64,
        /// Run in the foreground instead of installing as a service.
        #[arg(long)]
        foreground: bool,
    },

    /// Run the node agent using saved config from `node.json`.
    ///
    /// This is the command invoked by the OS service (launchd / systemd).
    /// It reads connection parameters from `~/.moltis/node.json`, which
    /// is written by `moltis node add`.
    Run {
        /// Override the maximum command timeout in seconds.
        #[arg(long)]
        timeout: Option<u64>,
    },

    /// Disconnect this machine and remove the node service.
    Remove,

    /// Show the current node connection info and service status.
    Status,

    /// Print the path to the node log file.
    Logs,
}

pub async fn handle_node(action: NodeAction) -> Result<()> {
    match action {
        NodeAction::GenerateToken {
            name,
            host,
            api_key,
        } => cmd_generate_token(&host, api_key.as_deref(), name.as_deref()).await,

        NodeAction::List { host, api_key } => cmd_list(&host, api_key.as_deref()).await,

        NodeAction::Add {
            host,
            token,
            name,
            node_id,
            working_dir,
            timeout,
            foreground,
        } => {
            let resolved_node_id = node_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

            if foreground {
                let config = moltis_node_host::NodeConfig {
                    gateway_url: host,
                    device_token: token,
                    node_id: resolved_node_id,
                    display_name: name,
                    platform: std::env::consts::OS.into(),
                    caps: vec![
                        "system.run".into(),
                        "system.which".into(),
                        "system.providers".into(),
                    ],
                    commands: vec![
                        "system.run".into(),
                        "system.which".into(),
                        "system.providers".into(),
                    ],
                    exec_timeout: Duration::from_secs(timeout),
                    working_dir,
                };

                let node = moltis_node_host::NodeHost::new(config);
                node.run().await
            } else {
                let data_dir = moltis_config::data_dir();
                let svc_config = moltis_node_host::ServiceConfig {
                    gateway_url: host,
                    device_token: token,
                    node_id: Some(resolved_node_id),
                    display_name: name,
                    working_dir,
                    timeout,
                };

                moltis_node_host::service::install(&data_dir, &svc_config)?;
                println!("Node registered and service started.");
                println!(
                    "Logs: {}",
                    moltis_node_host::service::log_path(&data_dir).display()
                );
                Ok(())
            }
        },

        NodeAction::Run { timeout } => {
            let data_dir = moltis_config::data_dir();
            let config = moltis_node_host::ServiceConfig::load(&data_dir)
                .map_err(|e| anyhow::anyhow!(
                    "cannot load node config: {e}\nRun `moltis node add` first to register this machine."
                ))?;

            let exec_timeout = Duration::from_secs(timeout.unwrap_or(config.timeout));

            let node_config = moltis_node_host::NodeConfig {
                gateway_url: config.gateway_url,
                device_token: config.device_token,
                node_id: config
                    .node_id
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                display_name: config.display_name,
                platform: std::env::consts::OS.into(),
                caps: vec![
                    "system.run".into(),
                    "system.which".into(),
                    "system.providers".into(),
                ],
                commands: vec![
                    "system.run".into(),
                    "system.which".into(),
                    "system.providers".into(),
                ],
                exec_timeout,
                working_dir: config.working_dir,
            };

            let node = moltis_node_host::NodeHost::new(node_config);
            node.run().await
        },

        NodeAction::Remove => {
            let data_dir = moltis_config::data_dir();
            moltis_node_host::service::uninstall(&data_dir)?;
            println!("Node removed.");
            Ok(())
        },

        NodeAction::Status => {
            let data_dir = moltis_config::data_dir();
            let config_path = data_dir.join("node.json");

            if !config_path.exists() {
                println!("Not registered as a node.");
                return Ok(());
            }

            let config = moltis_node_host::ServiceConfig::load(&data_dir)?;
            let status = moltis_node_host::service::status()?;

            println!("Gateway: {}", config.gateway_url);
            if let Some(ref name) = config.display_name {
                println!("Name:    {name}");
            }
            println!("Service: {status}");
            Ok(())
        },

        NodeAction::Logs => {
            let data_dir = moltis_config::data_dir();
            println!(
                "{}",
                moltis_node_host::service::log_path(&data_dir).display()
            );
            Ok(())
        },
    }
}

// ── Gateway RPC helpers ────────────────────────────────────────────────────

/// Call `device.token.create` on the gateway and print the token + command.
async fn cmd_generate_token(host: &str, api_key: Option<&str>, name: Option<&str>) -> Result<()> {
    let mut params = serde_json::Map::new();
    if let Some(n) = name {
        params.insert("displayName".into(), serde_json::json!(n));
    }
    params.insert("platform".into(), serde_json::json!("remote"));

    let result = gateway_rpc(host, api_key, "device.token.create", params.into()).await?;

    let token = result
        .get("deviceToken")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("unexpected response: missing deviceToken"))?;

    let ws_url = http_to_ws(host);

    println!("Device token: {token}");
    println!();
    println!("Run this on the remote machine:");
    println!("  moltis node add --host {ws_url} --token {token}");
    println!();
    println!("The token is shown once and cannot be retrieved later.");

    Ok(())
}

/// Call `node.list` on the gateway and print connected nodes.
async fn cmd_list(host: &str, api_key: Option<&str>) -> Result<()> {
    let result = gateway_rpc(host, api_key, "node.list", serde_json::json!({})).await?;

    let nodes = result
        .get("nodes")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if nodes.is_empty() {
        println!("No nodes connected.");
        return Ok(());
    }

    for node in &nodes {
        let id = node.get("nodeId").and_then(|v| v.as_str()).unwrap_or("?");
        let name = node
            .get("displayName")
            .and_then(|v| v.as_str())
            .unwrap_or("(unnamed)");
        let platform = node.get("platform").and_then(|v| v.as_str()).unwrap_or("?");

        println!("{id}  {name}  ({platform})");
    }

    Ok(())
}

/// Send a single RPC request to the gateway over HTTP (JSON-RPC style).
///
/// The gateway exposes RPC methods over the WebSocket protocol, but for
/// one-shot CLI commands we use the `/api/rpc` endpoint when available,
/// falling back to a transient WebSocket connection.
async fn gateway_rpc(
    host: &str,
    api_key: Option<&str>,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value> {
    let url = format!("{}/api/rpc", host.trim_end_matches('/'));
    let body = serde_json::json!({
        "method": method,
        "params": params,
    });

    let client = reqwest::Client::new();
    let mut req = client.post(&url).json(&body);
    if let Some(key) = api_key {
        req = req.header("Authorization", format!("Bearer {key}"));
    }

    let resp = req
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("cannot reach gateway at {host}: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("gateway returned {status}: {body}");
    }

    let result: serde_json::Value = resp.json().await?;

    if let Some(err) = result.get("error") {
        let msg = err
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        anyhow::bail!("RPC error: {msg}");
    }

    Ok(result.get("payload").cloned().unwrap_or(result))
}

/// Convert `http://host:port` to `ws://host:port/ws`.
fn http_to_ws(http_url: &str) -> String {
    let ws = http_url
        .replace("https://", "wss://")
        .replace("http://", "ws://");
    let base = ws.trim_end_matches('/');
    format!("{base}/ws")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn http_to_ws_basic() {
        assert_eq!(
            http_to_ws("http://localhost:9090"),
            "ws://localhost:9090/ws"
        );
    }

    #[test]
    fn http_to_ws_https() {
        assert_eq!(
            http_to_ws("https://my-server.com"),
            "wss://my-server.com/ws"
        );
    }

    #[test]
    fn http_to_ws_trailing_slash() {
        assert_eq!(
            http_to_ws("http://localhost:9090/"),
            "ws://localhost:9090/ws"
        );
    }
}
