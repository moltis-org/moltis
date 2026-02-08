use std::sync::Arc;

#[cfg(feature = "metrics")]
use std::time::Instant;

use {
    anyhow::Result,
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
    tracing::{debug, info, warn},
};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, histogram, labels, tools as tools_metrics};

use moltis_agents::tool_registry::AgentTool;

use crate::{
    exec::ExecOpts,
    sandbox::{SandboxId, SandboxRouter},
};

/// Regex pattern for valid tmux session names: only `[a-zA-Z0-9_-]`.
fn is_valid_session_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Generate a short unique session name.
fn generate_session_name() -> String {
    let id = uuid::Uuid::new_v4();
    // Use first 8 hex chars for brevity.
    format!("proc-{}", &id.simple().to_string()[..8])
}

/// Actions supported by the process tool.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum ProcessAction {
    /// Start a command in a new tmux session.
    Start {
        command: String,
        #[serde(default)]
        session_name: Option<String>,
    },
    /// Capture current visible terminal output.
    Poll { session_name: String },
    /// Send keystrokes to a session (e.g. `q`, `Enter`, `C-c`).
    SendKeys { session_name: String, keys: String },
    /// Paste text into a session via tmux buffer.
    Paste { session_name: String, text: String },
    /// Kill a tmux session.
    Kill { session_name: String },
    /// List active tmux sessions.
    List,
}

/// Result returned from a process tool action.
#[derive(Debug, Clone, Serialize)]
struct ProcessResult {
    /// Whether the action succeeded.
    success: bool,
    /// The tmux session name (for start/poll/send_keys/paste/kill).
    #[serde(skip_serializing_if = "Option::is_none")]
    session_name: Option<String>,
    /// Captured terminal output (for poll) or command output.
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    /// Error message if the action failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl ProcessResult {
    fn ok(session_name: Option<String>, output: Option<String>) -> Self {
        Self {
            success: true,
            session_name,
            output,
            error: None,
        }
    }

    fn err(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            session_name: None,
            output: None,
            error: Some(msg.into()),
        }
    }
}

/// Tool for managing interactive/TUI processes via tmux inside the sandbox.
///
/// The LLM can start long-running or interactive programs (htop, vim, etc.)
/// in named tmux sessions, poll their output, send keystrokes, paste text,
/// and kill them.
#[derive(Default)]
pub struct ProcessTool {
    sandbox_router: Option<Arc<SandboxRouter>>,
}

impl ProcessTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach a sandbox router for per-session dynamic sandbox resolution.
    pub fn with_sandbox_router(mut self, router: Arc<SandboxRouter>) -> Self {
        self.sandbox_router = Some(router);
        self
    }

    /// Run a tmux command inside the sandbox container.
    async fn run_tmux(
        &self,
        session_key: &str,
        tmux_args: &str,
        timeout_secs: u64,
    ) -> Result<crate::exec::ExecResult> {
        let command = format!("tmux {tmux_args}");
        let opts = ExecOpts {
            timeout: std::time::Duration::from_secs(timeout_secs),
            working_dir: Some(std::path::PathBuf::from("/home/sandbox")),
            ..Default::default()
        };

        if let Some(ref router) = self.sandbox_router {
            let is_sandboxed = router.is_sandboxed(session_key).await;
            if is_sandboxed {
                let id = router.sandbox_id_for(session_key);
                let image = router.resolve_image(session_key, None).await;
                let backend = router.backend();
                backend.ensure_ready(&id, Some(&image)).await?;
                return backend.exec(&id, &command, &opts).await;
            }
        }

        // Fallback: run directly on host (for non-sandboxed or no router).
        crate::exec::exec_command(&command, &opts).await
    }

    /// Resolve the sandbox ID for a session key (for logging).
    fn sandbox_id_for(&self, session_key: &str) -> Option<SandboxId> {
        self.sandbox_router
            .as_ref()
            .map(|r| r.sandbox_id_for(session_key))
    }

    async fn handle_start(
        &self,
        session_key: &str,
        command: &str,
        session_name: Option<&str>,
    ) -> ProcessResult {
        // Treat empty strings as "not provided" — LLMs often send "" instead of omitting.
        let name = match session_name.filter(|n| !n.is_empty()) {
            Some(n) => {
                if !is_valid_session_name(n) {
                    return ProcessResult::err(
                        "invalid session_name: only [a-zA-Z0-9_-] allowed, max 64 chars",
                    );
                }
                n.to_string()
            },
            None => generate_session_name(),
        };

        // Escape single quotes in the command for safe shell embedding.
        let escaped_command = command.replace('\'', "'\\''");
        let tmux_cmd = format!("new-session -d -s '{name}' -x 200 -y 50 '{escaped_command}'");

        match self.run_tmux(session_key, &tmux_cmd, 10).await {
            Ok(result) if result.exit_code == 0 => {
                info!(session_name = %name, command, "tmux session started");
                ProcessResult::ok(Some(name), Some("session started".into()))
            },
            Ok(result) => {
                let err_output = if !result.stderr.is_empty() {
                    result.stderr
                } else {
                    result.stdout
                };
                warn!(session_name = %name, %err_output, "tmux start failed");
                ProcessResult::err(format!("tmux start failed: {err_output}"))
            },
            Err(e) => ProcessResult::err(format!("failed to start tmux session: {e}")),
        }
    }

    async fn handle_poll(&self, session_key: &str, session_name: &str) -> ProcessResult {
        if !is_valid_session_name(session_name) {
            return ProcessResult::err(
                "invalid session_name: only [a-zA-Z0-9_-] allowed, max 64 chars",
            );
        }

        let tmux_cmd = format!("capture-pane -t '{session_name}' -p");

        match self.run_tmux(session_key, &tmux_cmd, 10).await {
            Ok(result) if result.exit_code == 0 => {
                ProcessResult::ok(Some(session_name.into()), Some(result.stdout))
            },
            Ok(result) => {
                let err = if !result.stderr.is_empty() {
                    result.stderr
                } else {
                    result.stdout
                };
                ProcessResult::err(format!("poll failed: {err}"))
            },
            Err(e) => ProcessResult::err(format!("failed to poll session: {e}")),
        }
    }

    async fn handle_send_keys(
        &self,
        session_key: &str,
        session_name: &str,
        keys: &str,
    ) -> ProcessResult {
        if !is_valid_session_name(session_name) {
            return ProcessResult::err(
                "invalid session_name: only [a-zA-Z0-9_-] allowed, max 64 chars",
            );
        }

        // tmux send-keys accepts key names like Enter, C-c, etc.
        // Escape single quotes in key strings.
        let escaped_keys = keys.replace('\'', "'\\''");
        let tmux_cmd = format!("send-keys -t '{session_name}' '{escaped_keys}'");

        match self.run_tmux(session_key, &tmux_cmd, 10).await {
            Ok(result) if result.exit_code == 0 => {
                debug!(session_name, keys, "keys sent");
                ProcessResult::ok(Some(session_name.into()), Some("keys sent".into()))
            },
            Ok(result) => {
                let err = if !result.stderr.is_empty() {
                    result.stderr
                } else {
                    result.stdout
                };
                ProcessResult::err(format!("send_keys failed: {err}"))
            },
            Err(e) => ProcessResult::err(format!("failed to send keys: {e}")),
        }
    }

    async fn handle_paste(
        &self,
        session_key: &str,
        session_name: &str,
        text: &str,
    ) -> ProcessResult {
        if !is_valid_session_name(session_name) {
            return ProcessResult::err(
                "invalid session_name: only [a-zA-Z0-9_-] allowed, max 64 chars",
            );
        }

        // Use tmux set-buffer + paste-buffer to avoid shell interpolation.
        let escaped_text = text.replace('\'', "'\\''");
        let set_cmd = format!("set-buffer '{escaped_text}'");

        match self.run_tmux(session_key, &set_cmd, 10).await {
            Ok(result) if result.exit_code == 0 => {
                // Now paste the buffer into the target session.
                let paste_cmd = format!("paste-buffer -t '{session_name}'");
                match self.run_tmux(session_key, &paste_cmd, 10).await {
                    Ok(r) if r.exit_code == 0 => {
                        debug!(session_name, "text pasted");
                        ProcessResult::ok(Some(session_name.into()), Some("text pasted".into()))
                    },
                    Ok(r) => {
                        let err = if !r.stderr.is_empty() {
                            r.stderr
                        } else {
                            r.stdout
                        };
                        ProcessResult::err(format!("paste-buffer failed: {err}"))
                    },
                    Err(e) => ProcessResult::err(format!("failed to paste: {e}")),
                }
            },
            Ok(result) => {
                let err = if !result.stderr.is_empty() {
                    result.stderr
                } else {
                    result.stdout
                };
                ProcessResult::err(format!("set-buffer failed: {err}"))
            },
            Err(e) => ProcessResult::err(format!("failed to set buffer: {e}")),
        }
    }

    async fn handle_kill(&self, session_key: &str, session_name: &str) -> ProcessResult {
        if !is_valid_session_name(session_name) {
            return ProcessResult::err(
                "invalid session_name: only [a-zA-Z0-9_-] allowed, max 64 chars",
            );
        }

        let tmux_cmd = format!("kill-session -t '{session_name}'");

        match self.run_tmux(session_key, &tmux_cmd, 10).await {
            Ok(result) if result.exit_code == 0 => {
                info!(session_name, "tmux session killed");
                ProcessResult::ok(Some(session_name.into()), Some("session killed".into()))
            },
            Ok(result) => {
                let err = if !result.stderr.is_empty() {
                    result.stderr
                } else {
                    result.stdout
                };
                ProcessResult::err(format!("kill failed: {err}"))
            },
            Err(e) => ProcessResult::err(format!("failed to kill session: {e}")),
        }
    }

    async fn handle_list(&self, session_key: &str) -> ProcessResult {
        match self.run_tmux(session_key, "list-sessions", 10).await {
            Ok(result) if result.exit_code == 0 => ProcessResult::ok(None, Some(result.stdout)),
            Ok(result) => {
                // "no server running" is normal when there are no sessions.
                let output = if !result.stderr.is_empty() {
                    &result.stderr
                } else {
                    &result.stdout
                };
                if output.contains("no server running") || output.contains("no sessions") {
                    ProcessResult::ok(None, Some("no active sessions".into()))
                } else {
                    ProcessResult::err(format!("list failed: {output}"))
                }
            },
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("no server running") {
                    ProcessResult::ok(None, Some("no active sessions".into()))
                } else {
                    ProcessResult::err(format!("failed to list sessions: {e}"))
                }
            },
        }
    }
}

#[async_trait]
impl AgentTool for ProcessTool {
    fn name(&self) -> &str {
        "process"
    }

    fn description(&self) -> &str {
        "Manage interactive terminal processes (TUI apps, REPLs, long-running commands) \
         via tmux sessions in the sandbox. Actions: start, poll, send_keys, paste, kill, list."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["start", "poll", "send_keys", "paste", "kill", "list"],
                    "description": "The action to perform"
                },
                "command": {
                    "type": "string",
                    "description": "The command to run (for 'start' action)"
                },
                "session_name": {
                    "type": "string",
                    "description": "Tmux session name. Auto-generated if omitted for 'start'. Required for poll/send_keys/paste/kill."
                },
                "keys": {
                    "type": "string",
                    "description": "Keystrokes to send (for 'send_keys'). Examples: 'q', 'Enter', 'C-c', 'Up'"
                },
                "text": {
                    "type": "string",
                    "description": "Text to paste into the session (for 'paste' action)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        #[cfg(feature = "metrics")]
        let start = Instant::now();

        let session_key = params
            .get("_session_key")
            .and_then(|v| v.as_str())
            .unwrap_or("main");

        let action: ProcessAction = match serde_json::from_value(params.clone()) {
            Ok(a) => a,
            Err(e) => {
                let result = ProcessResult::err(format!("invalid parameters: {e}"));
                return Ok(serde_json::to_value(&result)?);
            },
        };

        let action_label = match &action {
            ProcessAction::Start { .. } => "start",
            ProcessAction::Poll { .. } => "poll",
            ProcessAction::SendKeys { .. } => "send_keys",
            ProcessAction::Paste { .. } => "paste",
            ProcessAction::Kill { .. } => "kill",
            ProcessAction::List => "list",
        };

        debug!(
            action = action_label,
            sandbox_id = ?self.sandbox_id_for(session_key),
            "process tool invoked"
        );

        let result = match action {
            ProcessAction::Start {
                command,
                session_name,
            } => {
                self.handle_start(session_key, &command, session_name.as_deref())
                    .await
            },
            ProcessAction::Poll { session_name } => {
                self.handle_poll(session_key, &session_name).await
            },
            ProcessAction::SendKeys { session_name, keys } => {
                self.handle_send_keys(session_key, &session_name, &keys)
                    .await
            },
            ProcessAction::Paste { session_name, text } => {
                self.handle_paste(session_key, &session_name, &text).await
            },
            ProcessAction::Kill { session_name } => {
                self.handle_kill(session_key, &session_name).await
            },
            ProcessAction::List => self.handle_list(session_key).await,
        };

        info!(
            action = action_label,
            success = result.success,
            "process tool completed"
        );

        #[cfg(feature = "metrics")]
        {
            let duration = start.elapsed().as_secs_f64();
            counter!(
                tools_metrics::EXECUTIONS_TOTAL,
                labels::TOOL => "process".to_string(),
                labels::SUCCESS => result.success.to_string()
            )
            .increment(1);

            histogram!(
                tools_metrics::EXECUTION_DURATION_SECONDS,
                labels::TOOL => "process".to_string()
            )
            .record(duration);

            if !result.success {
                counter!(
                    tools_metrics::EXECUTION_ERRORS_TOTAL,
                    labels::TOOL => "process".to_string()
                )
                .increment(1);
            }
        }

        Ok(serde_json::to_value(&result)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_session_names() {
        assert!(is_valid_session_name("my-session"));
        assert!(is_valid_session_name("my_session"));
        assert!(is_valid_session_name("session123"));
        assert!(is_valid_session_name("a"));
        assert!(is_valid_session_name("ABC-123_xyz"));
    }

    #[test]
    fn test_invalid_session_names() {
        assert!(!is_valid_session_name(""));
        assert!(!is_valid_session_name("has space"));
        assert!(!is_valid_session_name("has;semicolon"));
        assert!(!is_valid_session_name("has'quote"));
        assert!(!is_valid_session_name("has\"dquote"));
        assert!(!is_valid_session_name("has$dollar"));
        assert!(!is_valid_session_name("has`backtick"));
        assert!(!is_valid_session_name("has|pipe"));
        assert!(!is_valid_session_name("has&amp"));
        assert!(!is_valid_session_name("path/traversal"));
        assert!(!is_valid_session_name("dot.not.allowed"));
    }

    #[test]
    fn test_session_name_too_long() {
        let long_name: String = "a".repeat(65);
        assert!(!is_valid_session_name(&long_name));
        let max_name: String = "a".repeat(64);
        assert!(is_valid_session_name(&max_name));
    }

    #[test]
    fn test_generate_session_name() {
        let name = generate_session_name();
        assert!(name.starts_with("proc-"));
        assert_eq!(name.len(), 13); // "proc-" (5) + 8 hex chars
        assert!(is_valid_session_name(&name));

        // Names are unique.
        let name2 = generate_session_name();
        assert_ne!(name, name2);
    }

    #[test]
    fn test_process_result_ok() {
        let result = ProcessResult::ok(Some("my-sess".into()), Some("output".into()));
        assert!(result.success);
        assert_eq!(result.session_name.as_deref(), Some("my-sess"));
        assert_eq!(result.output.as_deref(), Some("output"));
        assert!(result.error.is_none());
    }

    #[test]
    fn test_process_result_err() {
        let result = ProcessResult::err("something went wrong");
        assert!(!result.success);
        assert!(result.session_name.is_none());
        assert!(result.output.is_none());
        assert_eq!(result.error.as_deref(), Some("something went wrong"));
    }

    #[test]
    fn test_process_result_serialization() {
        // Ensure None fields are omitted from JSON.
        let result = ProcessResult::ok(None, Some("output".into()));
        let json = serde_json::to_value(&result).unwrap();
        assert!(json.get("session_name").is_none());
        assert!(json.get("error").is_none());
        assert_eq!(json["success"], true);
        assert_eq!(json["output"], "output");
    }

    #[test]
    fn test_process_action_deserialize_start() {
        let json = serde_json::json!({
            "action": "start",
            "command": "htop",
            "session_name": "my-htop"
        });
        let action: ProcessAction = serde_json::from_value(json).unwrap();
        match action {
            ProcessAction::Start {
                command,
                session_name,
            } => {
                assert_eq!(command, "htop");
                assert_eq!(session_name.as_deref(), Some("my-htop"));
            },
            _ => panic!("expected Start action"),
        }
    }

    #[test]
    fn test_process_action_deserialize_start_no_name() {
        let json = serde_json::json!({
            "action": "start",
            "command": "python3"
        });
        let action: ProcessAction = serde_json::from_value(json).unwrap();
        match action {
            ProcessAction::Start {
                command,
                session_name,
            } => {
                assert_eq!(command, "python3");
                assert!(session_name.is_none());
            },
            _ => panic!("expected Start action"),
        }
    }

    #[test]
    fn test_process_action_deserialize_start_empty_name() {
        // LLMs often send "" instead of omitting — should deserialize as Some("").
        let json = serde_json::json!({
            "action": "start",
            "command": "htop",
            "session_name": ""
        });
        let action: ProcessAction = serde_json::from_value(json).unwrap();
        match action {
            ProcessAction::Start {
                command,
                session_name,
            } => {
                assert_eq!(command, "htop");
                assert_eq!(session_name.as_deref(), Some(""));
            },
            _ => panic!("expected Start action"),
        }
    }

    #[test]
    fn test_process_action_deserialize_poll() {
        let json = serde_json::json!({
            "action": "poll",
            "session_name": "my-htop"
        });
        let action: ProcessAction = serde_json::from_value(json).unwrap();
        match action {
            ProcessAction::Poll { session_name } => {
                assert_eq!(session_name, "my-htop");
            },
            _ => panic!("expected Poll action"),
        }
    }

    #[test]
    fn test_process_action_deserialize_send_keys() {
        let json = serde_json::json!({
            "action": "send_keys",
            "session_name": "repl",
            "keys": "C-c"
        });
        let action: ProcessAction = serde_json::from_value(json).unwrap();
        match action {
            ProcessAction::SendKeys { session_name, keys } => {
                assert_eq!(session_name, "repl");
                assert_eq!(keys, "C-c");
            },
            _ => panic!("expected SendKeys action"),
        }
    }

    #[test]
    fn test_process_action_deserialize_paste() {
        let json = serde_json::json!({
            "action": "paste",
            "session_name": "editor",
            "text": "print('hello')\n"
        });
        let action: ProcessAction = serde_json::from_value(json).unwrap();
        match action {
            ProcessAction::Paste { session_name, text } => {
                assert_eq!(session_name, "editor");
                assert_eq!(text, "print('hello')\n");
            },
            _ => panic!("expected Paste action"),
        }
    }

    #[test]
    fn test_process_action_deserialize_kill() {
        let json = serde_json::json!({
            "action": "kill",
            "session_name": "old-session"
        });
        let action: ProcessAction = serde_json::from_value(json).unwrap();
        match action {
            ProcessAction::Kill { session_name } => {
                assert_eq!(session_name, "old-session");
            },
            _ => panic!("expected Kill action"),
        }
    }

    #[test]
    fn test_process_action_deserialize_list() {
        let json = serde_json::json!({ "action": "list" });
        let action: ProcessAction = serde_json::from_value(json).unwrap();
        assert!(matches!(action, ProcessAction::List));
    }

    #[test]
    fn test_process_action_invalid_action() {
        let json = serde_json::json!({ "action": "invalid" });
        let result: std::result::Result<ProcessAction, _> = serde_json::from_value(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_process_action_missing_required_field() {
        // start without command
        let json = serde_json::json!({ "action": "start" });
        let result: std::result::Result<ProcessAction, _> = serde_json::from_value(json);
        assert!(result.is_err());

        // poll without session_name
        let json = serde_json::json!({ "action": "poll" });
        let result: std::result::Result<ProcessAction, _> = serde_json::from_value(json);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_process_tool_schema() {
        let tool = ProcessTool::new();
        assert_eq!(tool.name(), "process");
        assert!(!tool.description().is_empty());

        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["action"].is_object());
        assert!(schema["properties"]["command"].is_object());
        assert!(schema["properties"]["session_name"].is_object());
        assert!(schema["properties"]["keys"].is_object());
        assert!(schema["properties"]["text"].is_object());
    }

    #[tokio::test]
    async fn test_process_tool_invalid_params() {
        let tool = ProcessTool::new();
        let result = tool
            .execute(serde_json::json!({ "action": "bogus" }))
            .await
            .unwrap();
        assert!(!result["success"].as_bool().unwrap());
        assert!(
            result["error"]
                .as_str()
                .unwrap()
                .contains("invalid parameters")
        );
    }

    #[tokio::test]
    async fn test_process_tool_invalid_session_name_on_poll() {
        let tool = ProcessTool::new();
        let result = tool
            .execute(serde_json::json!({
                "action": "poll",
                "session_name": "bad;name"
            }))
            .await
            .unwrap();
        assert!(!result["success"].as_bool().unwrap());
        assert!(
            result["error"]
                .as_str()
                .unwrap()
                .contains("invalid session_name")
        );
    }

    #[tokio::test]
    async fn test_process_tool_invalid_session_name_on_send_keys() {
        let tool = ProcessTool::new();
        let result = tool
            .execute(serde_json::json!({
                "action": "send_keys",
                "session_name": "has space",
                "keys": "Enter"
            }))
            .await
            .unwrap();
        assert!(!result["success"].as_bool().unwrap());
        assert!(
            result["error"]
                .as_str()
                .unwrap()
                .contains("invalid session_name")
        );
    }

    #[tokio::test]
    async fn test_process_tool_invalid_session_name_on_paste() {
        let tool = ProcessTool::new();
        let result = tool
            .execute(serde_json::json!({
                "action": "paste",
                "session_name": "a|b",
                "text": "hello"
            }))
            .await
            .unwrap();
        assert!(!result["success"].as_bool().unwrap());
        assert!(
            result["error"]
                .as_str()
                .unwrap()
                .contains("invalid session_name")
        );
    }

    #[tokio::test]
    async fn test_process_tool_invalid_session_name_on_kill() {
        let tool = ProcessTool::new();
        let result = tool
            .execute(serde_json::json!({
                "action": "kill",
                "session_name": "$(whoami)"
            }))
            .await
            .unwrap();
        assert!(!result["success"].as_bool().unwrap());
        assert!(
            result["error"]
                .as_str()
                .unwrap()
                .contains("invalid session_name")
        );
    }

    #[tokio::test]
    async fn test_process_tool_list_no_sandbox() {
        // Without a sandbox router, list runs tmux on the host.
        // This may or may not have tmux installed, so we just check
        // the result is valid JSON with success or error.
        let tool = ProcessTool::new();
        let result = tool
            .execute(serde_json::json!({ "action": "list" }))
            .await
            .unwrap();
        // Should always return valid JSON with a success field.
        assert!(result.get("success").is_some());
    }

    #[tokio::test]
    async fn test_process_tool_start_without_command() {
        let tool = ProcessTool::new();
        let result = tool
            .execute(serde_json::json!({ "action": "start" }))
            .await
            .unwrap();
        assert!(!result["success"].as_bool().unwrap());
        assert!(
            result["error"]
                .as_str()
                .unwrap()
                .contains("invalid parameters")
        );
    }
}
