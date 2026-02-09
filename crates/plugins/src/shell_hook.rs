//! Shell-based hook handler that executes external commands.
//!
//! The handler spawns a child process for each event, passing the
//! [`HookPayload`] as JSON on stdin and interpreting the response:
//!
//! - Exit 0, no stdout → [`HookAction::Continue`]
//! - Exit 0, stdout JSON `{"action": "modify", "data": {...}}` → [`HookAction::ModifyPayload`]
//! - Exit 1 → [`HookAction::Block`] with stderr as reason
//! - Timeout → error (non-fatal, logged by registry)

use std::{collections::HashMap, time::Duration};

use {
    anyhow::{Context, Result, bail},
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
    serde_json::Value,
    tokio::{io::AsyncWriteExt, process::Command},
    tracing::{debug, warn},
};

use crate::hooks::{HookAction, HookEvent, HookHandler, HookPayload, ShellHookConfig};

/// Response format expected from shell hooks on stdout.
#[derive(Debug, Deserialize, Serialize)]
struct ShellHookResponse {
    action: String,
    #[serde(default)]
    data: Option<Value>,
}

/// A hook handler that executes an external shell command.
pub struct ShellHookHandler {
    hook_name: String,
    command: String,
    subscribed_events: Vec<HookEvent>,
    timeout: Duration,
    env: HashMap<String, String>,
}

impl ShellHookHandler {
    pub fn new(
        name: impl Into<String>,
        command: impl Into<String>,
        events: Vec<HookEvent>,
        timeout: Duration,
        env: HashMap<String, String>,
    ) -> Self {
        Self {
            hook_name: name.into(),
            command: command.into(),
            subscribed_events: events,
            timeout,
            env,
        }
    }

    /// Create from a [`ShellHookConfig`].
    pub fn from_config(config: &ShellHookConfig) -> Self {
        Self::new(
            config.name.clone(),
            config.command.clone(),
            config.events.clone(),
            Duration::from_secs(config.timeout),
            config.env.clone(),
        )
    }
}

#[async_trait]
impl HookHandler for ShellHookHandler {
    fn name(&self) -> &str {
        &self.hook_name
    }

    fn events(&self) -> &[HookEvent] {
        &self.subscribed_events
    }

    async fn handle(&self, _event: HookEvent, payload: &HookPayload) -> Result<HookAction> {
        let payload_json =
            serde_json::to_string(payload).context("failed to serialize hook payload")?;

        debug!(
            hook = %self.hook_name,
            command = %self.command,
            payload_len = payload_json.len(),
            "spawning shell hook"
        );

        let mut child = Command::new("sh")
            .arg("-c")
            .arg(&self.command)
            .envs(&self.env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to spawn hook command: {}", self.command))?;

        // Write payload to stdin (ignore broken pipe if child doesn't read it).
        if let Some(mut stdin) = child.stdin.take()
            && let Err(e) = stdin.write_all(payload_json.as_bytes()).await
            && e.kind() != std::io::ErrorKind::BrokenPipe
        {
            return Err(e.into());
        }

        // Wait with timeout.
        let output = tokio::time::timeout(self.timeout, child.wait_with_output())
            .await
            .with_context(|| {
                format!(
                    "hook '{}' timed out after {:?}",
                    self.hook_name, self.timeout
                )
            })?
            .with_context(|| format!("hook '{}' failed to complete", self.hook_name))?;

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        debug!(
            hook = %self.hook_name,
            exit_code,
            stdout_len = stdout.len(),
            stderr_len = stderr.len(),
            "shell hook completed"
        );

        if exit_code == 1 {
            let reason = match stderr.is_empty() {
                true => format!("hook '{}' blocked the action", self.hook_name),
                false => stderr.trim().to_string(),
            };
            return Ok(HookAction::Block(reason));
        }

        if exit_code != 0 {
            bail!(
                "hook '{}' exited with code {}: {}",
                self.hook_name,
                exit_code,
                stderr.trim()
            );
        }

        // Exit 0 — check for modify response on stdout.
        let stdout_trimmed = stdout.trim();
        if stdout_trimmed.is_empty() {
            return Ok(HookAction::Continue);
        }

        match serde_json::from_str::<ShellHookResponse>(stdout_trimmed) {
            Ok(resp) if resp.action == "modify" => {
                if let Some(data) = resp.data {
                    Ok(HookAction::ModifyPayload(data))
                } else {
                    warn!(hook = %self.hook_name, "modify action without data, continuing");
                    Ok(HookAction::Continue)
                }
            },
            Ok(_) => Ok(HookAction::Continue),
            Err(e) => {
                warn!(hook = %self.hook_name, error = %e, "failed to parse hook stdout as JSON, continuing");
                Ok(HookAction::Continue)
            },
        }
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    fn test_payload() -> HookPayload {
        HookPayload::SessionStart {
            session_key: "test-123".into(),
        }
    }

    #[tokio::test]
    async fn shell_hook_continue_on_exit_zero() {
        let handler = ShellHookHandler::new(
            "test-continue",
            "exit 0",
            vec![HookEvent::SessionStart],
            Duration::from_secs(5),
            HashMap::new(),
        );
        let result = handler
            .handle(HookEvent::SessionStart, &test_payload())
            .await
            .unwrap();
        assert!(matches!(result, HookAction::Continue));
    }

    #[tokio::test]
    async fn shell_hook_block_on_exit_one() {
        let handler = ShellHookHandler::new(
            "test-block",
            "echo 'blocked by policy' >&2; exit 1",
            vec![HookEvent::SessionStart],
            Duration::from_secs(5),
            HashMap::new(),
        );
        let result = handler
            .handle(HookEvent::SessionStart, &test_payload())
            .await
            .unwrap();
        match result {
            HookAction::Block(reason) => assert_eq!(reason, "blocked by policy"),
            _ => panic!("expected Block"),
        }
    }

    #[tokio::test]
    async fn shell_hook_modify_payload() {
        let handler = ShellHookHandler::new(
            "test-modify",
            r#"echo '{"action":"modify","data":{"redacted":true}}'"#,
            vec![HookEvent::SessionStart],
            Duration::from_secs(5),
            HashMap::new(),
        );
        let result = handler
            .handle(HookEvent::SessionStart, &test_payload())
            .await
            .unwrap();
        match result {
            HookAction::ModifyPayload(v) => assert_eq!(v, serde_json::json!({"redacted": true})),
            _ => panic!("expected ModifyPayload"),
        }
    }

    #[tokio::test]
    async fn shell_hook_receives_payload_on_stdin() {
        let handler = ShellHookHandler::new(
            "test-stdin",
            // Read stdin, parse, and emit modify with the session_key from payload.
            r#"INPUT=$(cat); KEY=$(echo "$INPUT" | grep -o '"session_key":"[^"]*"' | head -1 | cut -d'"' -f4); echo "{\"action\":\"modify\",\"data\":{\"key\":\"$KEY\"}}"  "#,
            vec![HookEvent::SessionStart],
            Duration::from_secs(5),
            HashMap::new(),
        );
        let result = handler
            .handle(HookEvent::SessionStart, &test_payload())
            .await
            .unwrap();
        match result {
            HookAction::ModifyPayload(v) => assert_eq!(v["key"], "test-123"),
            _ => panic!("expected ModifyPayload, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn shell_hook_timeout() {
        let handler = ShellHookHandler::new(
            "test-timeout",
            "sleep 60",
            vec![HookEvent::SessionStart],
            Duration::from_millis(100),
            HashMap::new(),
        );
        let result = handler
            .handle(HookEvent::SessionStart, &test_payload())
            .await;
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("timed out"),
            "should mention timeout"
        );
    }

    #[tokio::test]
    async fn shell_hook_env_vars() {
        let mut env = HashMap::new();
        env.insert("MY_HOOK_VAR".into(), "hello_hook".into());
        let handler = ShellHookHandler::new(
            "test-env",
            r#"echo "{\"action\":\"modify\",\"data\":{\"var\":\"$MY_HOOK_VAR\"}}"  "#,
            vec![HookEvent::SessionStart],
            Duration::from_secs(5),
            env,
        );
        let result = handler
            .handle(HookEvent::SessionStart, &test_payload())
            .await
            .unwrap();
        match result {
            HookAction::ModifyPayload(v) => assert_eq!(v["var"], "hello_hook"),
            _ => panic!("expected ModifyPayload"),
        }
    }

    #[tokio::test]
    async fn shell_hook_nonzero_exit_is_error() {
        let handler = ShellHookHandler::new(
            "test-error",
            "exit 2",
            vec![HookEvent::SessionStart],
            Duration::from_secs(5),
            HashMap::new(),
        );
        let result = handler
            .handle(HookEvent::SessionStart, &test_payload())
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn shell_hook_invalid_json_stdout_continues() {
        let handler = ShellHookHandler::new(
            "test-bad-json",
            "echo 'not json'",
            vec![HookEvent::SessionStart],
            Duration::from_secs(5),
            HashMap::new(),
        );
        let result = handler
            .handle(HookEvent::SessionStart, &test_payload())
            .await
            .unwrap();
        assert!(matches!(result, HookAction::Continue));
    }

    #[tokio::test]
    async fn from_config_works() {
        let config = ShellHookConfig {
            name: "test".into(),
            command: "exit 0".into(),
            events: vec![HookEvent::BeforeToolCall],
            timeout: 3,
            env: HashMap::new(),
        };
        let handler = ShellHookHandler::from_config(&config);
        assert_eq!(handler.name(), "test");
        assert_eq!(handler.events(), &[HookEvent::BeforeToolCall]);
        assert_eq!(handler.timeout, Duration::from_secs(3));
    }
}
