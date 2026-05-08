use std::{collections::HashMap, path::PathBuf, pin::Pin, process::Stdio, time::Duration};

use {
    futures::{Stream, stream},
    tokio::{io::AsyncWriteExt, process::Command},
};

use crate::{
    transport::ExternalAgentSession,
    types::{ContextSnapshot, ExternalAgentEvent, ExternalAgentStatus},
};

#[allow(dead_code)]
pub struct OneShotProcessSession {
    binary: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    working_dir: Option<PathBuf>,
    timeout: Duration,
    status: ExternalAgentStatus,
}

#[allow(dead_code)]
impl OneShotProcessSession {
    pub fn new(
        binary: String,
        args: Vec<String>,
        env: HashMap<String, String>,
        working_dir: Option<PathBuf>,
        timeout_secs: Option<u64>,
    ) -> Self {
        Self {
            binary,
            args,
            env,
            working_dir,
            timeout: Duration::from_secs(timeout_secs.unwrap_or(300)),
            status: ExternalAgentStatus::Idle,
        }
    }
}

#[async_trait::async_trait]
impl ExternalAgentSession for OneShotProcessSession {
    fn external_session_id(&self) -> Option<&str> {
        None
    }

    async fn send_prompt(
        &mut self,
        prompt: &str,
        context: Option<&ContextSnapshot>,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = ExternalAgentEvent> + Send>>> {
        self.status = ExternalAgentStatus::Running;
        let input = build_process_input(prompt, context);
        let mut command = Command::new(&self.binary);
        command.args(&self.args);
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
        self.status = ExternalAgentStatus::Idle;

        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let events = vec![
                ExternalAgentEvent::TextDelta(text),
                ExternalAgentEvent::Done { usage: None },
            ];
            Ok(Box::pin(stream::iter(events)))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let message = if stderr.is_empty() {
                format!("external agent exited with status {}", output.status)
            } else {
                stderr
            };
            Ok(Box::pin(stream::iter(vec![ExternalAgentEvent::Error(
                message,
            )])))
        }
    }

    async fn is_alive(&self) -> bool {
        true
    }

    async fn shutdown(&mut self) -> anyhow::Result<()> {
        self.status = ExternalAgentStatus::Stopped;
        Ok(())
    }

    fn status(&self) -> ExternalAgentStatus {
        self.status
    }
}

pub(crate) fn build_process_input(prompt: &str, context: Option<&ContextSnapshot>) -> String {
    let Some(context) = context else {
        return prompt.to_string();
    };

    let mut parts = Vec::new();
    if let Some(working_dir) = &context.working_dir {
        parts.push(format!("Working directory: {}", working_dir.display()));
    }
    if let Some(instructions) = &context.system_instructions {
        parts.push(format!("System instructions:\n{instructions}"));
    }
    if let Some(summary) = &context.summary {
        parts.push(format!("Conversation summary:\n{summary}"));
    }
    if !context.recent_turns.is_empty() {
        let turns = context
            .recent_turns
            .iter()
            .map(|turn| format!("{}: {}", turn.role, turn.content))
            .collect::<Vec<_>>()
            .join("\n");
        parts.push(format!("Recent conversation:\n{turns}"));
    }
    if let Some(project_context) = &context.project_context {
        parts.push(format!("Project context:\n{project_context}"));
    }
    parts.push(format!("User prompt:\n{prompt}"));
    parts.join("\n\n")
}

#[cfg(test)]
mod tests {
    use {super::*, crate::types::ContextTurn};

    #[test]
    fn process_input_includes_context_and_prompt() {
        let context = ContextSnapshot {
            system_instructions: Some("Be concise".to_string()),
            recent_turns: vec![ContextTurn {
                role: "user".to_string(),
                content: "previous".to_string(),
            }],
            project_context: Some("project details".to_string()),
            ..ContextSnapshot::default()
        };

        let input = build_process_input("next", Some(&context));

        assert!(input.contains("System instructions:\nBe concise"));
        assert!(input.contains("user: previous"));
        assert!(input.contains("Project context:\nproject details"));
        assert!(input.contains("User prompt:\nnext"));
    }

    #[test]
    fn process_input_without_context_is_prompt_only() {
        assert_eq!(build_process_input("hello", None), "hello");
    }
}
