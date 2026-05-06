//! WebSocket client that connects to a gateway as a headless node.

use std::{process::Stdio, time::Duration};

use {
    futures::{SinkExt, StreamExt},
    tokio::process::Command,
    tokio_tungstenite::{connect_async, tungstenite::Message},
    tracing::{debug, error, info, warn},
};

use crate::error::{Error, Result};

use moltis_protocol::{
    ClientInfo, ConnectAuth, ConnectParamsV4, GatewayFrame, PROTOCOL_VERSION, ProtocolRange,
    RequestFrame, ResponseFrame, roles,
};

/// Configuration for connecting a node to a gateway.
#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// Gateway WebSocket URL (e.g. `ws://localhost:9090/ws`).
    pub gateway_url: String,
    /// Device token obtained from the pairing flow.
    pub device_token: String,
    /// Unique node identifier.
    pub node_id: String,
    /// Human-readable display name.
    pub display_name: Option<String>,
    /// Platform string (e.g. "macos", "linux").
    pub platform: String,
    /// Capabilities this node advertises (e.g. "system.run").
    pub caps: Vec<String>,
    /// Commands this node supports (e.g. "system.run", "system.which").
    pub commands: Vec<String>,
    /// Maximum time for a single command execution.
    pub exec_timeout: Duration,
    /// Working directory for commands (defaults to $HOME).
    pub working_dir: Option<String>,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            gateway_url: "ws://localhost:9090/ws".into(),
            device_token: String::new(),
            node_id: uuid::Uuid::new_v4().to_string(),
            display_name: None,
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
            exec_timeout: Duration::from_secs(300),
            working_dir: None,
        }
    }
}

/// Detect installed runtimes using `which`.
fn detect_runtimes() -> Vec<String> {
    let candidates = ["python3", "python", "node", "ruby", "go", "rustc", "java"];
    let mut found = Vec::new();
    for name in candidates {
        // Skip "python" if we already found "python3".
        if name == "python" && found.contains(&"python3".to_string()) {
            continue;
        }
        if which::which(name).is_ok() {
            found.push(name.to_string());
        }
    }
    found
}

/// Collect system telemetry using `sysinfo`.
fn collect_system_telemetry(node_id: &str) -> serde_json::Value {
    use sysinfo::{Disks, System};

    let mut sys = System::new();
    sys.refresh_memory();
    sys.refresh_cpu_all();

    let uptime = System::uptime();

    // Disk: find the root partition.
    let disks = Disks::new_with_refreshed_list();
    let root_disk = disks
        .iter()
        .find(|d| d.mount_point() == std::path::Path::new("/"));
    let disk_total = root_disk.map(|d| d.total_space());
    let disk_available = root_disk.map(|d| d.available_space());

    let runtimes = detect_runtimes();

    serde_json::json!({
        "nodeId": node_id,
        "mem": {
            "total": sys.total_memory(),
            "available": sys.available_memory(),
        },
        "cpuCount": sys.cpus().len(),
        "cpuUsage": sys.global_cpu_usage(),
        "uptime": uptime,
        "services": [],
        "disk": {
            "total": disk_total,
            "available": disk_available,
        },
        "runtimes": runtimes,
    })
}

/// A headless node host that connects to a gateway and handles commands.
pub struct NodeHost {
    config: NodeConfig,
}

impl NodeHost {
    pub fn new(config: NodeConfig) -> Self {
        Self { config }
    }

    /// Connect to the gateway and run the message loop until disconnected.
    ///
    /// Returns `Ok(())` on clean shutdown, `Err` on connection/protocol errors.
    pub async fn run(&self) -> Result<()> {
        // Install a rustls CryptoProvider before any TLS connection.
        // Without this, `connect_async` on a `wss://` URL panics because
        // tokio-tungstenite uses rustls under the hood and no provider is
        // registered in the node-host code path (the gateway sets its own
        // in `gateway.rs`). See #744.
        let _ = rustls::crypto::ring::default_provider().install_default();

        // Validate URL first, then pass string to connect_async.
        let _url = url::Url::parse(&self.config.gateway_url)?;
        info!(url = %self.config.gateway_url, node_id = %self.config.node_id, "connecting to gateway");

        let (ws_stream, _response) = connect_async(&self.config.gateway_url).await?;
        let (mut ws_tx, mut ws_rx) = ws_stream.split();

        info!("websocket connected, sending handshake");

        // Build and send connect request (v4 format).
        let connect_id = uuid::Uuid::new_v4().to_string();
        let connect_params = ConnectParamsV4 {
            protocol: ProtocolRange {
                min: PROTOCOL_VERSION,
                max: PROTOCOL_VERSION,
            },
            client: ClientInfo {
                id: self.config.node_id.clone(),
                display_name: self.config.display_name.clone(),
                version: moltis_config::VERSION.into(),
                platform: self.config.platform.clone(),
                device_family: None,
                model_identifier: None,
                mode: "headless".into(),
                instance_id: None,
            },
            role: Some(roles::NODE.into()),
            scopes: None,
            auth: Some(ConnectAuth {
                token: None,
                password: None,
                api_key: None,
                device_token: Some(self.config.device_token.clone()),
            }),
            locale: None,
            timezone: None,
            extensions: {
                let mut ext = std::collections::HashMap::new();
                ext.insert(
                    "moltis".into(),
                    serde_json::json!({
                        "caps": self.config.caps,
                        "commands": self.config.commands,
                    }),
                );
                ext
            },
        };

        let connect_req = RequestFrame {
            r#type: "req".into(),
            id: connect_id.clone(),
            method: "connect".into(),
            params: Some(serde_json::to_value(&connect_params)?),
            channel: None,
        };

        let connect_json = serde_json::to_string(&connect_req)?;
        ws_tx.send(Message::Text(connect_json.into())).await?;

        // Wait for hello-ok response.
        let hello = match tokio::time::timeout(Duration::from_secs(10), ws_rx.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                let resp: ResponseFrame = serde_json::from_str(&text)?;
                if resp.id != connect_id {
                    return Err(Error::Protocol(format!(
                        "unexpected response id: expected {connect_id}, got {}",
                        resp.id
                    )));
                }
                if !resp.ok {
                    let err_msg = resp
                        .error
                        .map(|e| format!("{}: {}", e.code, e.message))
                        .unwrap_or_else(|| "unknown error".into());
                    return Err(Error::Protocol(format!("handshake failed: {err_msg}")));
                }
                resp
            },
            Ok(Some(Ok(_))) => {
                return Err(Error::Protocol(
                    "unexpected non-text message during handshake".into(),
                ));
            },
            Ok(Some(Err(e))) => {
                return Err(Error::Protocol(format!(
                    "websocket error during handshake: {e}"
                )));
            },
            Ok(None) => return Err(Error::Protocol("connection closed during handshake".into())),
            Err(_) => return Err(Error::Protocol("handshake timeout".into())),
        };

        info!(
            server_version = hello
                .payload
                .as_ref()
                .and_then(|p| p.get("server"))
                .and_then(|s| s.get("version"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown"),
            "handshake complete, node registered"
        );

        // Main message loop with telemetry ticker.
        let mut telemetry_interval = tokio::time::interval(Duration::from_secs(30));
        // Skip the immediate first tick — send telemetry after the first interval.
        telemetry_interval.tick().await;

        loop {
            tokio::select! {
                msg = ws_rx.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            self.handle_message(&text, &mut ws_tx).await;
                        },
                        Some(Ok(Message::Ping(data))) => {
                            if let Err(e) = ws_tx.send(Message::Pong(data)).await {
                                warn!(error = %e, "failed to send pong");
                                break;
                            }
                        },
                        Some(Ok(Message::Close(_))) => {
                            info!("gateway closed connection");
                            break;
                        },
                        Some(Ok(_)) => {},
                        Some(Err(e)) => {
                            error!(error = %e, "websocket error");
                            break;
                        },
                        None => {
                            info!("websocket stream ended");
                            break;
                        },
                    }
                },
                _ = telemetry_interval.tick() => {
                    self.send_telemetry(&mut ws_tx).await;
                },
            }
        }

        info!("node disconnected");
        Ok(())
    }

    async fn handle_message(
        &self,
        text: &str,
        ws_tx: &mut (impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    ) {
        // Try to parse as a GatewayFrame to determine the type.
        let frame: GatewayFrame = match serde_json::from_str(text) {
            Ok(f) => f,
            Err(e) => {
                debug!(error = %e, "failed to parse frame, ignoring");
                return;
            },
        };

        match frame {
            GatewayFrame::Event(event) => {
                if event.event == "node.invoke.request" {
                    self.handle_invoke(event.payload, ws_tx).await;
                } else {
                    debug!(event = %event.event, "ignoring event");
                }
            },
            GatewayFrame::Request(req) => {
                debug!(method = %req.method, id = %req.id, "ignoring server request");
            },
            GatewayFrame::Response(_) => {
                debug!("ignoring response frame");
            },
        }
    }

    async fn handle_invoke(
        &self,
        payload: Option<serde_json::Value>,
        ws_tx: &mut (impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    ) {
        let payload = match payload {
            Some(p) => p,
            None => return,
        };

        let invoke_id = match payload.get("invokeId").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => {
                warn!("invoke request missing invokeId");
                return;
            },
        };

        let command = match payload.get("command").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => {
                warn!(invoke_id = %invoke_id, "invoke request missing command");
                self.send_invoke_error(&invoke_id, "missing command", ws_tx)
                    .await;
                return;
            },
        };

        let args = payload.get("args").cloned().unwrap_or_default();

        info!(invoke_id = %invoke_id, command = %command, "handling invoke");

        let result = match command {
            "system.run" => self.handle_system_run(&args).await,
            "system.which" => self.handle_system_which(&args).await,
            "system.providers" => self.handle_system_providers().await,
            other => {
                warn!(command = %other, "unsupported invoke command");
                Err(Error::Command(format!("unsupported command: {other}")))
            },
        };

        match result {
            Ok(value) => {
                self.send_invoke_result(&invoke_id, value, ws_tx).await;
            },
            Err(e) => {
                self.send_invoke_error(&invoke_id, &e.to_string(), ws_tx)
                    .await;
            },
        }
    }

    async fn handle_system_run(&self, args: &serde_json::Value) -> Result<serde_json::Value> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Command("missing 'command' in args".into()))?;

        let timeout_ms = args
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(self.config.exec_timeout.as_millis() as u64);
        let timeout = Duration::from_millis(timeout_ms);

        let cwd = args
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| self.config.working_dir.clone());

        info!(command = %command, timeout_ms, "system.run");

        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::null());

        if let Some(ref dir) = cwd {
            cmd.current_dir(dir);
        }

        // Apply environment from args if provided.
        if let Some(env_obj) = args.get("env").and_then(|v| v.as_object()) {
            for (k, v) in env_obj {
                if let Some(val) = v.as_str() {
                    cmd.env(k, val);
                }
            }
        }

        let child = cmd.spawn()?;
        let result = tokio::time::timeout(timeout, child.wait_with_output()).await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
                let exit_code = output.status.code().unwrap_or(-1);

                Ok(serde_json::json!({
                    "stdout": stdout,
                    "stderr": stderr,
                    "exitCode": exit_code,
                }))
            },
            Ok(Err(e)) => Err(Error::Command(format!("failed to execute command: {e}"))),
            Err(_) => Err(Error::Command(format!(
                "command timed out after {timeout_ms}ms"
            ))),
        }
    }

    async fn handle_system_which(&self, args: &serde_json::Value) -> Result<serde_json::Value> {
        let binary = args
            .get("binary")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Command("missing 'binary' in args".into()))?;

        let output = Command::new("which").arg(binary).output().await?;

        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let found = output.status.success();

        Ok(serde_json::json!({
            "found": found,
            "path": if found { Some(path) } else { None },
        }))
    }

    async fn handle_system_providers(&self) -> Result<serde_json::Value> {
        let mut providers = Vec::new();

        // Check Ollama at localhost:11434.
        if let Ok(resp) = reqwest::Client::new()
            .get("http://localhost:11434/api/tags")
            .timeout(Duration::from_secs(3))
            .send()
            .await
            && resp.status().is_success()
            && let Ok(body) = resp.json::<serde_json::Value>().await
        {
            let models: Vec<String> = body
                .get("models")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            providers.push(serde_json::json!({
                "provider": "ollama",
                "models": models,
            }));
        }

        // Check for known API key env vars (presence only, never the values).
        let api_key_checks = [
            ("OPENAI_API_KEY", "openai"),
            ("ANTHROPIC_API_KEY", "anthropic"),
            ("GOOGLE_API_KEY", "google"),
            ("MISTRAL_API_KEY", "mistral"),
            ("GROQ_API_KEY", "groq"),
            ("TOGETHER_API_KEY", "together"),
        ];
        for (env_var, provider_name) in api_key_checks {
            if std::env::var(env_var).is_ok_and(|v| !v.is_empty()) {
                providers.push(serde_json::json!({
                    "provider": provider_name,
                    "models": [],
                }));
            }
        }

        Ok(serde_json::json!({ "providers": providers }))
    }

    async fn send_invoke_result(
        &self,
        invoke_id: &str,
        result: serde_json::Value,
        ws_tx: &mut (impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    ) {
        let frame = serde_json::json!({
            "type": "req",
            "id": uuid::Uuid::new_v4().to_string(),
            "method": "node.invoke.result",
            "params": {
                "invokeId": invoke_id,
                "result": result,
            }
        });

        if let Ok(json) = serde_json::to_string(&frame)
            && let Err(e) = ws_tx.send(Message::Text(json.into())).await
        {
            warn!(invoke_id = %invoke_id, error = %e, "failed to send invoke result");
        }
    }

    async fn send_telemetry(
        &self,
        ws_tx: &mut (impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    ) {
        let telemetry = collect_system_telemetry(&self.config.node_id);
        let frame = serde_json::json!({
            "type": "req",
            "id": uuid::Uuid::new_v4().to_string(),
            "method": "node.event",
            "params": {
                "event": "node.telemetry",
                "payload": telemetry,
            }
        });

        if let Ok(json) = serde_json::to_string(&frame)
            && let Err(e) = ws_tx.send(Message::Text(json.into())).await
        {
            debug!(error = %e, "failed to send telemetry");
        }
    }

    async fn send_invoke_error(
        &self,
        invoke_id: &str,
        error: &str,
        ws_tx: &mut (impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    ) {
        let frame = serde_json::json!({
            "type": "req",
            "id": uuid::Uuid::new_v4().to_string(),
            "method": "node.invoke.result",
            "params": {
                "invokeId": invoke_id,
                "result": { "error": error },
            }
        });

        if let Ok(json) = serde_json::to_string(&frame)
            && let Err(e) = ws_tx.send(Message::Text(json.into())).await
        {
            warn!(invoke_id = %invoke_id, error = %e, "failed to send invoke error");
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn default_config_has_system_run_cap() {
        let config = NodeConfig::default();
        assert!(config.caps.contains(&"system.run".to_string()));
        assert!(config.commands.contains(&"system.run".to_string()));
    }

    #[test]
    fn default_config_platform_is_current_os() {
        let config = NodeConfig::default();
        assert_eq!(config.platform, std::env::consts::OS);
    }

    #[tokio::test]
    async fn system_which_finds_sh() {
        let config = NodeConfig::default();
        let host = NodeHost::new(config);
        let args = serde_json::json!({ "binary": "sh" });
        let result = host.handle_system_which(&args).await.unwrap();
        assert_eq!(result["found"], true);
        assert!(result["path"].as_str().unwrap().contains("sh"));
    }

    #[tokio::test]
    async fn system_which_missing_binary() {
        let config = NodeConfig::default();
        let host = NodeHost::new(config);
        let args = serde_json::json!({ "binary": "definitely_not_a_real_binary_123456" });
        let result = host.handle_system_which(&args).await.unwrap();
        assert_eq!(result["found"], false);
    }

    #[tokio::test]
    async fn system_run_echo() {
        let config = NodeConfig::default();
        let host = NodeHost::new(config);
        let args = serde_json::json!({
            "command": "echo hello",
            "timeout": 5000,
        });
        let result = host.handle_system_run(&args).await.unwrap();
        assert_eq!(result["stdout"].as_str().unwrap().trim(), "hello");
        assert_eq!(result["exitCode"], 0);
    }

    #[tokio::test]
    async fn system_run_captures_stderr() {
        let config = NodeConfig::default();
        let host = NodeHost::new(config);
        let args = serde_json::json!({
            "command": "echo err >&2",
            "timeout": 5000,
        });
        let result = host.handle_system_run(&args).await.unwrap();
        assert_eq!(result["stderr"].as_str().unwrap().trim(), "err");
    }

    /// Regression test for #744: `connect_async("wss://...")` panicked on
    /// Windows because no rustls `CryptoProvider` was installed in the
    /// node-host code path.  After the fix, `run()` returns a connection
    /// error instead of panicking.
    #[tokio::test]
    async fn wss_url_does_not_panic_without_crypto_provider() {
        let config = NodeConfig {
            gateway_url: "wss://127.0.0.1:1/ws".into(),
            device_token: "test-token".into(),
            ..Default::default()
        };
        let node = NodeHost::new(config);
        // Should return Err (unreachable host), NOT panic.
        // Wrap in a timeout so the test doesn't hang if a firewall silently
        // drops packets to 127.0.0.1:1 instead of refusing immediately.
        let result = tokio::time::timeout(Duration::from_secs(5), node.run())
            .await
            .expect("timed out — possible firewall drop on 127.0.0.1:1");
        assert!(result.is_err(), "expected connection error, got Ok");
    }

    #[tokio::test]
    async fn system_run_exit_code() {
        let config = NodeConfig::default();
        let host = NodeHost::new(config);
        let args = serde_json::json!({
            "command": "exit 42",
            "timeout": 5000,
        });
        let result = host.handle_system_run(&args).await.unwrap();
        assert_eq!(result["exitCode"], 42);
    }
}
