//! Stdio transport: spawn a child process and communicate via JSON-RPC over stdin/stdout.

use std::{
    collections::HashMap,
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

/// Process isolation options for a stdio MCP server launch.
#[derive(Clone, Debug)]
pub struct StdioLaunchOptions {
    pub current_dir: Option<PathBuf>,
    pub inherit_parent_env: bool,
}

impl Default for StdioLaunchOptions {
    fn default() -> Self {
        Self {
            current_dir: None,
            inherit_parent_env: true,
        }
    }
}

use {
    futures::StreamExt,
    secrecy::{ExposeSecret, Secret},
    tokio::{
        io::AsyncWriteExt,
        process::{Child, Command},
        sync::{Mutex, oneshot},
    },
    tokio_util::codec::{FramedRead, LinesCodec},
    tracing::{debug, info, trace, warn},
};

use crate::{
    error::{Context, Error, Result},
    traits::McpTransport,
    types::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse},
};

const MAX_MCP_STDOUT_LINE_BYTES: usize = 4 * 1024 * 1024;
const MAX_MCP_STDERR_LINE_BYTES: usize = 64 * 1024;

/// Stdio-based transport for an MCP server process.
pub struct StdioTransport {
    child: Mutex<Child>,
    stdin: Mutex<tokio::process::ChildStdin>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,
    next_id: AtomicU64,
    request_timeout: Duration,
    reader_closed: Arc<AtomicBool>,
    /// Handle to the reader task so we can abort on drop.
    reader_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl StdioTransport {
    /// Spawn the server process and start the reader loop.
    pub async fn spawn(
        command: &str,
        args: &[String],
        env: &HashMap<String, Secret<String>>,
    ) -> Result<Arc<Self>> {
        Self::spawn_with_timeout(command, args, env, Duration::from_secs(30)).await
    }

    /// Spawn the server process with a custom request timeout and start the reader loop.
    pub async fn spawn_with_timeout(
        command: &str,
        args: &[String],
        env: &HashMap<String, Secret<String>>,
        request_timeout: Duration,
    ) -> Result<Arc<Self>> {
        Self::spawn_with_options(
            command,
            args,
            env,
            request_timeout,
            &StdioLaunchOptions::default(),
        )
        .await
    }

    /// Spawn with explicit process isolation and working-directory behavior.
    pub async fn spawn_with_options(
        command: &str,
        args: &[String],
        env: &HashMap<String, Secret<String>>,
        request_timeout: Duration,
        options: &StdioLaunchOptions,
    ) -> Result<Arc<Self>> {
        info!(command = %command, arg_count = args.len(), "spawning MCP server process");

        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if !options.inherit_parent_env {
            cmd.env_clear();
        }
        if let Some(current_dir) = options.current_dir.as_ref() {
            cmd.current_dir(current_dir);
        }
        for (name, value) in env {
            cmd.env(name, value.expose_secret());
        }

        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn MCP server: {command}"))?;

        let stdin = child.stdin.take().context("failed to capture stdin")?;
        let stdout = child.stdout.take().context("failed to capture stdout")?;
        let stderr = child.stderr.take();

        let pending: Arc<Mutex<HashMap<String, oneshot::Sender<JsonRpcResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let reader_closed = Arc::new(AtomicBool::new(false));

        let transport = Arc::new(Self {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            pending: Arc::clone(&pending),
            next_id: AtomicU64::new(1),
            request_timeout,
            reader_closed: Arc::clone(&reader_closed),
            reader_handle: Mutex::new(None),
        });

        // Start stderr reader task (log server errors).
        if let Some(stderr) = stderr {
            tokio::spawn(async move {
                let mut lines = FramedRead::new(
                    stderr,
                    LinesCodec::new_with_max_length(MAX_MCP_STDERR_LINE_BYTES),
                );
                while let Some(line) = lines.next().await {
                    match line {
                        Ok(line) => {
                            let trimmed = line.trim();
                            if !trimmed.is_empty() {
                                debug!(bytes = trimmed.len(), "MCP server wrote to stderr");
                            }
                        },
                        Err(error) => {
                            warn!(%error, "MCP server stderr line exceeded transport limit");
                        },
                    }
                }
            });
        }

        // Start stdout reader task.
        let pending_clone = Arc::clone(&pending);
        let reader_closed_clone = Arc::clone(&reader_closed);
        let handle = tokio::spawn(async move {
            let mut lines = FramedRead::new(
                stdout,
                LinesCodec::new_with_max_length(MAX_MCP_STDOUT_LINE_BYTES),
            );
            while let Some(line) = lines.next().await {
                match line {
                    Ok(line) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
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
                                debug!(error = %e, bytes = trimmed.len(), "MCP server sent non-response line");
                            },
                        }
                    },
                    Err(error) => {
                        warn!(%error, "MCP server stdout line exceeded transport limit");
                        break;
                    },
                }
            }
            reader_closed_clone.store(true, Ordering::Release);
            pending_clone.lock().await.clear();
            debug!("MCP server stdout closed");
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
        if self.reader_closed.load(Ordering::Acquire) {
            return Err(Error::message("MCP server stdout is closed"));
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = JsonRpcRequest::new(id, method, params);
        let id_key = req.id.to_string();

        let (tx, rx) = oneshot::channel();
        {
            let mut map = self.pending.lock().await;
            if self.reader_closed.load(Ordering::Acquire) {
                return Err(Error::message("MCP server stdout is closed"));
            }
            map.insert(id_key.clone(), tx);
        }

        let mut payload = serde_json::to_string(&req)?;
        payload.push('\n');

        debug!(method = %method, id = %id, "client -> MCP server");

        {
            let mut stdin = self.stdin.lock().await;
            if let Err(error) = stdin.write_all(payload.as_bytes()).await {
                self.pending.lock().await.remove(&id_key);
                return Err(error.into());
            }
            if let Err(error) = stdin.flush().await {
                self.pending.lock().await.remove(&id_key);
                return Err(error.into());
            }
        }

        let received = match tokio::time::timeout(self.request_timeout, rx).await {
            Ok(received) => received,
            Err(_) => {
                self.pending.lock().await.remove(&id_key);
                return Err(Error::message(format!(
                    "MCP request '{method}' timed out after {}s (no response from server)",
                    self.request_timeout.as_secs()
                )));
            },
        };
        let resp = received.with_context(|| {
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
        if self.reader_closed.load(Ordering::Acquire) {
            return false;
        }
        let mut child = self.child.lock().await;
        matches!(child.try_wait(), Ok(None))
    }

    async fn kill(&self) {
        self.reader_closed.store(true, Ordering::Release);
        self.pending.lock().await.clear();
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

    #[tokio::test]
    async fn test_request_uses_configured_timeout() {
        let args = vec!["-c".to_string(), "while read line; do :; done".to_string()];
        let transport = StdioTransport::spawn_with_timeout(
            "sh",
            &args,
            &HashMap::new(),
            Duration::from_secs(1),
        )
        .await
        .unwrap();

        let err = transport.request("tools/list", None).await.unwrap_err();
        assert!(err.to_string().contains("timed out after 1s"));
        assert!(transport.pending.lock().await.is_empty());

        transport.kill().await;
    }

    #[tokio::test]
    async fn test_stdout_closure_fails_pending_request_without_timeout() {
        let args = vec!["-c".to_string(), "read line; exit 0".to_string()];
        let transport = StdioTransport::spawn_with_timeout(
            "sh",
            &args,
            &HashMap::new(),
            Duration::from_secs(30),
        )
        .await
        .unwrap();

        let result = tokio::time::timeout(
            Duration::from_secs(1),
            transport.request("tools/list", None),
        )
        .await
        .expect("stdout closure should wake the request");
        assert!(result.is_err());
        assert!(transport.pending.lock().await.is_empty());
        assert!(!transport.is_alive().await);
    }
}
