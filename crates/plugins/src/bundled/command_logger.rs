//! `command-logger` hook: logs all Command events to a JSONL file.

use std::{path::PathBuf, sync::Mutex};

use {anyhow::Result, async_trait::async_trait, tracing::warn};

use moltis_common::hooks::{HookAction, HookEvent, HookHandler, HookPayload};

/// Appends JSONL entries for every `Command` event.
pub struct CommandLoggerHook {
    log_path: PathBuf,
    /// Buffer writes through a mutex to ensure atomic appends.
    file: Mutex<Option<std::fs::File>>,
}

impl CommandLoggerHook {
    pub fn new(log_path: PathBuf) -> Self {
        Self {
            log_path,
            file: Mutex::new(None),
        }
    }

    /// Default log path: `~/.moltis/logs/commands.log`
    pub fn default_path() -> Option<PathBuf> {
        Some(moltis_config::data_dir().join("logs/commands.log"))
    }

    fn ensure_file(&self) -> Result<()> {
        let mut guard = self.file.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            if let Some(parent) = self.log_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.log_path)?;
            *guard = Some(file);
        }
        Ok(())
    }
}

#[async_trait]
impl HookHandler for CommandLoggerHook {
    fn name(&self) -> &str {
        "command-logger"
    }

    fn events(&self) -> &[HookEvent] {
        &[HookEvent::Command]
    }

    async fn handle(&self, _event: HookEvent, payload: &HookPayload) -> Result<HookAction> {
        if let HookPayload::Command {
            session_key,
            action,
            sender_id,
        } = payload
        {
            if let Err(e) = self.ensure_file() {
                warn!(error = %e, "command-logger: failed to open log file");
                return Ok(HookAction::Continue);
            }

            let entry = serde_json::json!({
                "ts": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                "session_key": session_key,
                "action": action,
                "sender_id": sender_id,
            });

            use std::io::Write;
            let mut guard = self.file.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(ref mut f) = *guard
                && let Err(e) = writeln!(f, "{}", entry)
            {
                warn!(error = %e, "command-logger: failed to write log entry");
            }
        }
        Ok(HookAction::Continue)
    }

    fn handle_sync(&self, _event: HookEvent, payload: &HookPayload) -> Result<HookAction> {
        // Synchronous variant for hot-path use.
        if let HookPayload::Command {
            session_key,
            action,
            sender_id,
        } = payload
            && self.ensure_file().is_ok()
        {
            let entry = serde_json::json!({
                "ts": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                "session_key": session_key,
                "action": action,
                "sender_id": sender_id,
            });
            use std::io::Write;
            let mut guard = self.file.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(ref mut f) = *guard {
                let _ = writeln!(f, "{}", entry);
            }
        }
        Ok(HookAction::Continue)
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn logs_command_events() {
        let tmp = tempfile::tempdir().unwrap();
        let log_path = tmp.path().join("commands.log");
        let hook = CommandLoggerHook::new(log_path.clone());

        let payload = HookPayload::Command {
            session_key: "sess-1".into(),
            action: "new".into(),
            sender_id: Some("user-1".into()),
        };
        hook.handle(HookEvent::Command, &payload).await.unwrap();
        hook.handle(HookEvent::Command, &payload).await.unwrap();

        let content = std::fs::read_to_string(&log_path).unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 2);

        let entry: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(entry["action"], "new");
        assert_eq!(entry["session_key"], "sess-1");
    }

    #[tokio::test]
    async fn ignores_non_command_payloads() {
        let tmp = tempfile::tempdir().unwrap();
        let log_path = tmp.path().join("commands.log");
        let hook = CommandLoggerHook::new(log_path.clone());

        let payload = HookPayload::SessionStart {
            session_key: "test".into(),
        };
        hook.handle(HookEvent::Command, &payload).await.unwrap();
        // File shouldn't even be created
        assert!(!log_path.exists());
    }
}
