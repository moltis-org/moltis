//! Stdio transport: spawn a child process and communicate via JSON-RPC over stdin/stdout.

use std::{
    collections::HashMap,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use {
    tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
        process::{Child, Command},
        sync::{Mutex, oneshot},
    },
    tracing::{debug, info, trace, warn},
};

use crate::{
    error::{Context, Error, Result},
    traits::McpTransport,
    types::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse},
};

/// Stdio-based transport for an MCP server process.
pub struct StdioTransport {
    child: Mutex<Child>,
    stdin: Mutex<tokio::process::ChildStdin>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,
    next_id: AtomicU64,
    /// Handle to the reader task so we can abort on drop.
    reader_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl StdioTransport {
    /// Spawn the server process and start the reader loop.
    pub async fn spawn(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<Arc<Self>> {
        info!(
            command = %command,
            args = ?args,
            "spawning MCP server process"
        );

        let mut cmd = Command::new(command);
        cmd.args(args)
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn MCP server: {command}"))?;

        let stdin = child.stdin.take().context("failed to capture stdin")?;
        let stdout = child.stdout.take().context("failed to capture stdout")?;
        let stderr = child.stderr.take();

        let pending: Arc<Mutex<HashMap<String, oneshot::Sender<JsonRpcResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let transport = Arc::new(Self {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            pending: Arc::clone(&pending),
            next_id: AtomicU64::new(1),
            reader_handle: Mutex::new(None),
        });

        // Start stderr reader task (log server errors).
        if let Some(stderr) = stderr {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line).await {
                        Ok(0) => break,
                        Ok(_) => {
                            let trimmed = line.trim();
                            if !trimmed.is_empty() {
                                warn!(stderr = %trimmed, "MCP server stderr");
                            }
                        },
                        Err(_) => break,
                    }
                }
            });
        }

        // Start stdout reader task.
        let pending_clone = Arc::clone(&pending);
        let handle = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => {
                        debug!("MCP server stdout closed");
                        break;
                    },
                    Ok(_) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        debug!(raw = %trimmed, "MCP server -> client");

                        // Try to parse as response (has id field).
                        match serde_json::from_str::<JsonRpcResponse>(trimmed) {
                            Ok(resp) => {
                                let key = resp.id.to_string();
                                let mut map = pending_clone.lock().await;
                                if let Some(tx) = map.remove(&key) {
                                    let _ = tx.send(resp);
                                } else {
                                    warn!(id = %key, "received response for unknown request id");
                                }
                            },
                            Err(e) => {
                                debug!(error = %e, line = %trimmed, "MCP server sent non-response line");
                            },
                        }
                    },
                    Err(e) => {
                        warn!(error = %e, "error reading from MCP server stdout");
                        break;
                    },
                }
            }
        });

        *transport.reader_handle.lock().await = Some(handle);
        Ok(transport)
    }
}

#[async_trait::async_trait]
impl McpTransport for StdioTransport {
    async fn request(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<JsonRpcResponse> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = JsonRpcRequest::new(id, method, params);
        let id_key = req.id.to_string();

        let (tx, rx) = oneshot::channel();
        {
            let mut map = self.pending.lock().await;
            map.insert(id_key.clone(), tx);
        }

        let mut payload = serde_json::to_string(&req)?;
        payload.push('\n');

        debug!(method = %method, id = %id, "client -> MCP server");

        {
            let mut stdin = self.stdin.lock().await;
            stdin.write_all(payload.as_bytes()).await?;
            stdin.flush().await?;
        }

        let resp = tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .with_context(|| {
                format!("MCP request '{method}' timed out after 30s (no response from server)")
            })?
            .with_context(|| {
                format!("MCP reader task dropped while waiting for '{method}' response")
            })?;

        if let Some(ref err) = resp.error {
            return Err(Error::message(format!(
                "MCP error on '{method}': code={} message={}",
                err.code, err.message
            )));
        }

        Ok(resp)
    }

    async fn notify(&self, method: &str, params: Option<serde_json::Value>) -> Result<()> {
        let notif = JsonRpcNotification {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
        };

        let mut payload = serde_json::to_string(&notif)?;
        payload.push('\n');

        trace!(method = %method, "client -> MCP server (notification)");

        let mut stdin = self.stdin.lock().await;
        stdin.write_all(payload.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn is_alive(&self) -> bool {
        let mut child = self.child.lock().await;
        matches!(child.try_wait(), Ok(None))
    }

    async fn kill(&self) {
        if let Some(handle) = self.reader_handle.lock().await.take() {
            handle.abort();
        }
        let mut child = self.child.lock().await;
        let _ = child.kill().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_spawn_and_kill() {
        // Spawn a simple process that reads stdin (cat will echo back).
        let transport = StdioTransport::spawn("cat", &[], &HashMap::new())
            .await
            .unwrap();
        assert!(transport.is_alive().await);
        transport.kill().await;
        // After kill, process should be dead.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!transport.is_alive().await);
    }

    #[tokio::test]
    async fn test_spawn_nonexistent_command() {
        let result =
            StdioTransport::spawn("nonexistent_command_xyz_42", &[], &HashMap::new()).await;
        assert!(result.is_err());
    }
}
