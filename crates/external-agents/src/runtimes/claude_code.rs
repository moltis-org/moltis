use std::{collections::HashMap, path::PathBuf, pin::Pin, process::Stdio, time::Duration};

use {
    async_trait::async_trait,
    futures::{Stream, stream},
    serde_json::Value,
    tokio::{io::AsyncWriteExt, process::Command},
};

use crate::{
    runtimes::process::build_process_input,
    transport::{ExternalAgentSession, ExternalAgentTransport},
    types::{
        AgentTransportKind, ContextSnapshot, ExternalAgentEvent, ExternalAgentSpec,
        ExternalAgentStatus,
    },
};

const BINARY_NAME: &str = "claude";

/// Transport for Claude Code CLI agent.
pub struct ClaudeCodeTransport;

impl ClaudeCodeTransport {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for ClaudeCodeTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExternalAgentTransport for ClaudeCodeTransport {
    fn name(&self) -> &str {
        "claude-code"
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self)))]
    async fn is_available(&self) -> bool {
        which::which(BINARY_NAME).is_ok()
    }

    fn supported_kinds(&self) -> &[AgentTransportKind] {
        &[AgentTransportKind::ClaudeCode]
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self, spec)))]
    async fn start_session(
        &self,
        spec: &ExternalAgentSpec,
    ) -> anyhow::Result<Box<dyn ExternalAgentSession>> {
        let binary = spec
            .binary
            .clone()
            .unwrap_or_else(|| BINARY_NAME.to_string());
        let args = if spec.args.is_empty() {
            vec![
                "-p".to_string(),
                "--output-format".to_string(),
                "json".to_string(),
            ]
        } else {
            spec.args.clone()
        };
        Ok(Box::new(ClaudeCodeSession::new(
            binary,
            args,
            spec.env.clone(),
            spec.working_dir.clone(),
            spec.timeout_secs,
        )))
    }
}

struct ClaudeCodeSession {
    binary: String,
    base_args: Vec<String>,
    env: HashMap<String, String>,
    working_dir: Option<PathBuf>,
    timeout: Duration,
    session_id: Option<String>,
    status: ExternalAgentStatus,
}

impl ClaudeCodeSession {
    fn new(
        binary: String,
        base_args: Vec<String>,
        env: HashMap<String, String>,
        working_dir: Option<PathBuf>,
        timeout_secs: Option<u64>,
    ) -> Self {
        Self {
            binary,
            base_args,
            env,
            working_dir,
            timeout: Duration::from_secs(timeout_secs.unwrap_or(300)),
            session_id: None,
            status: ExternalAgentStatus::Idle,
        }
    }

    fn args_for_turn(&self) -> Vec<String> {
        let mut args = self.base_args.clone();
        if let Some(session_id) = &self.session_id
            && !has_resume_arg(&args)
        {
            args.push("--resume".to_string());
            args.push(session_id.clone());
        }
        args
    }
}

#[async_trait]
impl ExternalAgentSession for ClaudeCodeSession {
    fn external_session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    async fn send_prompt(
        &mut self,
        prompt: &str,
        context: Option<&ContextSnapshot>,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = ExternalAgentEvent> + Send>>> {
        self.status = ExternalAgentStatus::Running;
        let input = build_process_input(prompt, context);
        let mut command = Command::new(&self.binary);
        command.args(self.args_for_turn());
        if let Some(working_dir) = &self.working_dir {
            command.current_dir(working_dir);
        }
        command.envs(&self.env);
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        command.kill_on_drop(true);

        let mut child = command.spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(input.as_bytes()).await?;
            stdin.shutdown().await?;
        }
        let output = tokio::time::timeout(self.timeout, child.wait_with_output()).await??;
        if !output.status.success() {
            self.status = ExternalAgentStatus::Error;
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let message = if stderr.is_empty() {
                format!("Claude Code exited with status {}", output.status)
            } else {
                stderr
            };
            return Ok(Box::pin(stream::iter(vec![ExternalAgentEvent::Error(
                message,
            )])));
        }

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let parsed = parse_claude_output(&stdout);
        if let Some(session_id) = parsed.session_id {
            self.session_id = Some(session_id);
        }
        self.status = ExternalAgentStatus::Idle;
        Ok(Box::pin(stream::iter(vec![
            ExternalAgentEvent::TextDelta(parsed.text),
            ExternalAgentEvent::Done { usage: None },
        ])))
    }

    async fn is_alive(&self) -> bool {
        self.status != ExternalAgentStatus::Stopped
    }

    async fn shutdown(&mut self) -> anyhow::Result<()> {
        self.status = ExternalAgentStatus::Stopped;
        Ok(())
    }

    fn status(&self) -> ExternalAgentStatus {
        self.status
    }
}

struct ClaudeOutput {
    text: String,
    session_id: Option<String>,
}

fn parse_claude_output(output: &str) -> ClaudeOutput {
    let Ok(value) = serde_json::from_str::<Value>(output) else {
        return ClaudeOutput {
            text: output.to_string(),
            session_id: None,
        };
    };
    let text = value
        .get("result")
        .and_then(Value::as_str)
        .or_else(|| value.get("text").and_then(Value::as_str))
        .or_else(|| value.get("message").and_then(Value::as_str))
        .unwrap_or(output)
        .to_string();
    let session_id = value
        .get("session_id")
        .or_else(|| value.get("sessionId"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    ClaudeOutput { text, session_id }
}

fn has_resume_arg(args: &[String]) -> bool {
    args.iter()
        .any(|arg| matches!(arg.as_str(), "--resume" | "-r" | "--continue" | "-c"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_claude_json_output() {
        let output = parse_claude_output(r#"{"result":"done","session_id":"abc"}"#);
        assert_eq!(output.text, "done");
        assert_eq!(output.session_id.as_deref(), Some("abc"));
    }

    #[test]
    fn falls_back_to_plain_text() {
        let output = parse_claude_output("plain");
        assert_eq!(output.text, "plain");
        assert!(output.session_id.is_none());
    }

    #[test]
    fn adds_resume_arg_after_session_id_is_known() {
        let mut session = ClaudeCodeSession::new(
            "claude".to_string(),
            vec![
                "-p".to_string(),
                "--output-format".to_string(),
                "json".to_string(),
            ],
            HashMap::new(),
            None,
            None,
        );
        assert!(!session.args_for_turn().iter().any(|arg| arg == "--resume"));
        session.session_id = Some("sid".to_string());
        assert_eq!(session.args_for_turn(), vec![
            "-p",
            "--output-format",
            "json",
            "--resume",
            "sid"
        ]);
    }
}
