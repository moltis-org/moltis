use std::{
    pin::Pin,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};

use {
    async_trait::async_trait,
    futures::{Stream, stream},
    serde_json::{Value, json},
    tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
        process::{Child, ChildStderr, ChildStdin, ChildStdout, Command},
    },
};

use crate::{
    transport::{ExternalAgentSession, ExternalAgentTransport},
    types::{
        AgentTransportKind, ContextSnapshot, ExternalAgentEvent, ExternalAgentSpec,
        ExternalAgentStatus,
    },
};

const BINARY_NAME: &str = "codex";

/// Transport for Codex CLI agent (JSON-RPC over stdio).
pub struct CodexTransport;

impl CodexTransport {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for CodexTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExternalAgentTransport for CodexTransport {
    fn name(&self) -> &str {
        "codex"
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self)))]
    async fn is_available(&self) -> bool {
        which::which(BINARY_NAME).is_ok()
    }

    fn supported_kinds(&self) -> &[AgentTransportKind] {
        &[AgentTransportKind::Codex]
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self, _spec)))]
    async fn start_session(
        &self,
        spec: &ExternalAgentSpec,
    ) -> anyhow::Result<Box<dyn ExternalAgentSession>> {
        let binary = spec
            .binary
            .clone()
            .unwrap_or_else(|| BINARY_NAME.to_string());
        let args = if spec.args.is_empty() {
            vec!["app-server".to_string()]
        } else {
            spec.args.clone()
        };
        Ok(Box::new(
            CodexAppServerSession::start(
                binary,
                args,
                spec.env.clone(),
                spec.working_dir.clone(),
                spec.timeout_secs,
            )
            .await?,
        ))
    }
}

struct CodexAppServerSession {
    child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    thread_id: String,
    next_request_id: u64,
    timeout: Duration,
    status: ExternalAgentStatus,
    working_dir: Option<std::path::PathBuf>,
    stderr_lines: Arc<Mutex<Vec<String>>>,
}

impl CodexAppServerSession {
    async fn start(
        binary: String,
        args: Vec<String>,
        env: std::collections::HashMap<String, String>,
        working_dir: Option<std::path::PathBuf>,
        timeout_secs: Option<u64>,
    ) -> anyhow::Result<Self> {
        let timeout = Duration::from_secs(timeout_secs.unwrap_or(300));
        let mut command = Command::new(binary);
        command.args(args);
        if let Some(working_dir) = &working_dir {
            command.current_dir(working_dir);
        }
        command.envs(env);
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        command.kill_on_drop(true);

        let mut child = command.spawn()?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("codex app-server stdin unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("codex app-server stdout unavailable"))?;
        let stderr_lines = Arc::new(Mutex::new(Vec::new()));
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(collect_stderr(stderr, Arc::clone(&stderr_lines)));
        }
        let mut lines = BufReader::new(stdout).lines();

        write_json_line(
            &mut stdin,
            &json!({
                "id": 1u64,
                "method": "initialize",
                "params": {
                    "clientInfo": {"name": "moltis", "version": env!("CARGO_PKG_VERSION")},
                    "capabilities": {"experimentalApi": true}
                }
            }),
        )
        .await?;
        wait_for_response(&mut child, &mut lines, 1, timeout, &stderr_lines).await?;
        write_json_line(&mut stdin, &json!({"method": "initialized", "params": {}})).await?;
        let mut thread_params = json!({});
        if let Some(working_dir) = &working_dir {
            thread_params["cwd"] = json!(working_dir);
        }
        write_json_line(
            &mut stdin,
            &json!({
                "id": 2u64,
                "method": "thread/start",
                "params": thread_params
            }),
        )
        .await?;
        let thread_response =
            wait_for_response(&mut child, &mut lines, 2, timeout, &stderr_lines).await?;
        let thread_id = thread_response["result"]["thread"]["id"]
            .as_str()
            .or_else(|| thread_response["result"]["id"].as_str())
            .ok_or_else(|| anyhow::anyhow!("thread/start response missing thread id"))?
            .to_string();

        Ok(Self {
            child,
            stdin,
            lines,
            thread_id,
            next_request_id: 3,
            timeout,
            status: ExternalAgentStatus::Idle,
            working_dir,
            stderr_lines,
        })
    }

    async fn consume_turn(&mut self) -> anyhow::Result<Vec<ExternalAgentEvent>> {
        let mut events = Vec::new();
        loop {
            let line = tokio::time::timeout(self.timeout, self.lines.next_line()).await??;
            let Some(line) = line else {
                anyhow::bail!(
                    "codex app-server exited{}",
                    stderr_suffix(&self.stderr_lines)
                )
            };
            let value: Value = serde_json::from_str(&line)?;
            if let Some(message) = extract_message(&value) {
                events.push(ExternalAgentEvent::TextDelta(message));
            }
            match value["method"].as_str() {
                Some("turn/completed") => {
                    events.push(ExternalAgentEvent::Done {
                        usage: extract_usage(&value),
                    });
                    return Ok(events);
                },
                Some("turn/failed" | "turn/cancelled") => {
                    let message =
                        extract_message(&value).unwrap_or_else(|| "codex turn failed".to_string());
                    events.push(ExternalAgentEvent::Error(message));
                    return Ok(events);
                },
                _ => {},
            }
        }
    }
}

#[async_trait]
impl ExternalAgentSession for CodexAppServerSession {
    fn external_session_id(&self) -> Option<&str> {
        Some(&self.thread_id)
    }

    async fn send_prompt(
        &mut self,
        prompt: &str,
        _context: Option<&ContextSnapshot>,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = ExternalAgentEvent> + Send>>> {
        self.status = ExternalAgentStatus::Running;
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        let result = async {
            let mut params = json!({
                "threadId": self.thread_id,
                "input": [{"type": "text", "text": prompt}],
                "title": "Moltis chat turn",
            });
            if let Some(working_dir) = &self.working_dir {
                params["cwd"] = json!(working_dir);
            }
            write_json_line(
                &mut self.stdin,
                &json!({
                    "id": request_id,
                    "method": "turn/start",
                    "params": params
                }),
            )
            .await?;
            wait_for_response(
                &mut self.child,
                &mut self.lines,
                request_id,
                self.timeout,
                &self.stderr_lines,
            )
            .await?;
            self.consume_turn().await
        }
        .await;
        match result {
            Ok(events) => {
                self.status = ExternalAgentStatus::Idle;
                Ok(Box::pin(stream::iter(events)))
            },
            Err(error) => {
                self.status = ExternalAgentStatus::Stopped;
                Err(error)
            },
        }
    }

    async fn is_alive(&self) -> bool {
        self.status != ExternalAgentStatus::Stopped
    }

    async fn shutdown(&mut self) -> anyhow::Result<()> {
        self.status = ExternalAgentStatus::Stopped;
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
        Ok(())
    }

    fn status(&self) -> ExternalAgentStatus {
        self.status
    }
}

async fn write_json_line(stdin: &mut ChildStdin, value: &Value) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    stdin.write_all(&bytes).await?;
    stdin.write_all(b"\n").await?;
    stdin.flush().await?;
    Ok(())
}

async fn collect_stderr(stderr: ChildStderr, stderr_lines: Arc<Mutex<Vec<String>>>) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Ok(mut stderr_lines) = stderr_lines.lock() {
            stderr_lines.push(line);
            if stderr_lines.len() > 20 {
                stderr_lines.remove(0);
            }
        }
    }
}

async fn wait_for_response(
    child: &mut Child,
    lines: &mut Lines<BufReader<ChildStdout>>,
    id: u64,
    timeout: Duration,
    stderr_lines: &Arc<Mutex<Vec<String>>>,
) -> anyhow::Result<Value> {
    loop {
        let line = tokio::time::timeout(timeout, lines.next_line()).await??;
        let Some(line) = line else {
            let status = child.try_wait()?;
            anyhow::bail!(
                "codex app-server exited before response {id}: {status:?}{}",
                stderr_suffix(stderr_lines)
            )
        };
        let value: Value = serde_json::from_str(&line)?;
        if value["id"].as_u64() == Some(id) {
            if let Some(error) = value.get("error") {
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown app-server error");
                anyhow::bail!(format!("{message}{}", stderr_suffix(stderr_lines)))
            }
            return Ok(value);
        }
    }
}

fn stderr_suffix(stderr_lines: &Arc<Mutex<Vec<String>>>) -> String {
    let lines = stderr_lines
        .lock()
        .ok()
        .map(|lines| lines.clone())
        .unwrap_or_default();
    if lines.is_empty() {
        String::new()
    } else {
        format!("; stderr: {}", lines.join("\n"))
    }
}

fn extract_message(value: &Value) -> Option<String> {
    value
        .pointer("/params/message")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            value
                .pointer("/params/text")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            value
                .pointer("/result/message")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
}

fn extract_usage(value: &Value) -> Option<crate::types::TokenUsage> {
    let usage = value
        .pointer("/params/usage")
        .or_else(|| value.pointer("/result/usage"))?;
    let input_tokens = token_count_field(usage, &["input_tokens", "inputTokens"])?;
    let output_tokens = token_count_field(usage, &["output_tokens", "outputTokens"])?;
    Some(crate::types::TokenUsage {
        input_tokens,
        output_tokens,
    })
}

fn token_count_field(value: &Value, fields: &[&str]) -> Option<u32> {
    fields.iter().find_map(|field| {
        value
            .get(*field)
            .and_then(Value::as_u64)
            .and_then(|count| u32::try_from(count).ok())
    })
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        futures::StreamExt,
        serde_json::json,
        std::{fs, time::SystemTime},
    };

    #[test]
    fn extracts_messages_from_codex_shapes() {
        assert_eq!(
            extract_message(&json!({"params": {"message": "done"}})).as_deref(),
            Some("done")
        );
        assert_eq!(
            extract_message(&json!({"params": {"text": "delta"}})).as_deref(),
            Some("delta")
        );
        assert_eq!(
            extract_message(&json!({"result": {"message": "ok"}})).as_deref(),
            Some("ok")
        );
    }

    #[test]
    fn extracts_usage_from_codex_shapes() {
        let snake = extract_usage(&json!({
            "params": {
                "usage": {
                    "input_tokens": 13,
                    "output_tokens": 21
                }
            }
        }))
        .unwrap_or_default();
        assert_eq!(snake.input_tokens, 13);
        assert_eq!(snake.output_tokens, 21);

        let camel = extract_usage(&json!({
            "result": {
                "usage": {
                    "inputTokens": 34,
                    "outputTokens": 55
                }
            }
        }))
        .unwrap_or_default();
        assert_eq!(camel.input_tokens, 34);
        assert_eq!(camel.output_tokens, 55);
    }

    #[tokio::test]
    async fn codex_session_reuses_thread_for_multiple_prompts() -> anyhow::Result<()> {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("moltis-codex-test-{unique}"));
        fs::create_dir_all(&dir)?;
        let script = dir.join("fake-codex-app-server.sh");
        fs::write(
            &script,
            r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"id":1'*'"method":"initialize"'*)
      printf '%s\n' '{"id":1,"result":{}}'
      ;;
    *'"method":"initialized"'*)
      ;;
    *'"method":"thread/start"'*)
      printf '%s\n' '{"id":2,"result":{"thread":{"id":"thread-1"}}}'
      ;;
    *'"id":3'*'"method":"turn/start"'*'"threadId":"thread-1"'*)
      printf '%s\n' '{"id":3,"result":{"turn":{"id":"turn-1"}}}'
      printf '%s\n' '{"method":"turn/completed","params":{"message":"first"}}'
      ;;
    *'"id":4'*'"method":"turn/start"'*'"threadId":"thread-1"'*)
      printf '%s\n' '{"id":4,"result":{"turn":{"id":"turn-2"}}}'
      printf '%s\n' '{"method":"turn/completed","params":{"message":"second"}}'
      ;;
    *)
      printf '%s\n' '{"id":999,"error":{"message":"unexpected request"}}'
      ;;
  esac
done
"#,
        )?;

        let mut spec = ExternalAgentSpec::new(AgentTransportKind::Codex);
        spec.binary = Some("/bin/sh".to_string());
        spec.args = vec![script.to_string_lossy().to_string()];
        spec.timeout_secs = Some(5);

        let transport = CodexTransport::new();
        let mut session = transport.start_session(&spec).await?;
        assert_eq!(session.external_session_id(), Some("thread-1"));

        let first = session
            .send_prompt("hello", None)
            .await?
            .collect::<Vec<_>>()
            .await;
        let second = session
            .send_prompt("again", None)
            .await?
            .collect::<Vec<_>>()
            .await;

        assert_eq!(session.external_session_id(), Some("thread-1"));
        assert!(
            matches!(first.first(), Some(ExternalAgentEvent::TextDelta(text)) if text == "first")
        );
        assert!(
            matches!(second.first(), Some(ExternalAgentEvent::TextDelta(text)) if text == "second")
        );
        session.shutdown().await?;
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}
