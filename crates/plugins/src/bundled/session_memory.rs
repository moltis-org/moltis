//! `session-memory` hook: saves session conversation to a markdown file on `/new` or `/reset`.

use std::{path::PathBuf, sync::Arc};

use {
    anyhow::Result,
    async_trait::async_trait,
    tracing::{debug, info, warn},
};

use {
    moltis_common::hooks::{HookAction, HookEvent, HookHandler, HookPayload},
    moltis_sessions::store::SessionStore,
};

/// Format current UTC date as YYYY-MM-DD.
fn utc_date_string() -> String {
    let d = time::OffsetDateTime::now_utc().date();
    format!("{:04}-{:02}-{:02}", d.year(), d.month() as u8, d.day())
}

/// Saves session conversation log to `<workspace>/memory/session-<key>-<date>.md`
/// on session reset/new. The raw conversation is saved as markdown so the memory
/// system's chunker + embeddings will make it searchable.
pub struct SessionMemoryHook {
    workspace_dir: PathBuf,
    max_messages: usize,
    session_store: Arc<SessionStore>,
}

impl SessionMemoryHook {
    pub fn new(workspace_dir: PathBuf, session_store: Arc<SessionStore>) -> Self {
        Self {
            workspace_dir,
            max_messages: 50,
            session_store,
        }
    }

    pub fn with_max_messages(mut self, n: usize) -> Self {
        self.max_messages = n;
        self
    }
}

#[async_trait]
impl HookHandler for SessionMemoryHook {
    fn name(&self) -> &str {
        "session-memory"
    }

    fn events(&self) -> &[HookEvent] {
        &[HookEvent::Command]
    }

    async fn handle(&self, _event: HookEvent, payload: &HookPayload) -> Result<HookAction> {
        let HookPayload::Command {
            session_key,
            action,
            ..
        } = payload
        else {
            return Ok(HookAction::Continue);
        };

        // Only trigger on "new" / "reset" actions.
        if action != "new" && action != "reset" {
            return Ok(HookAction::Continue);
        }

        // Read the session's message history.
        let messages = match self.session_store.read(session_key).await {
            Ok(msgs) => msgs,
            Err(e) => {
                warn!(error = %e, session = %session_key, "session-memory: failed to read history");
                return Ok(HookAction::Continue);
            },
        };

        if messages.is_empty() {
            debug!(session = %session_key, "session-memory: empty session, skipping");
            return Ok(HookAction::Continue);
        }

        let memory_dir = self.workspace_dir.join("memory");
        if let Err(e) = tokio::fs::create_dir_all(&memory_dir).await {
            warn!(error = %e, "session-memory: failed to create memory dir");
            return Ok(HookAction::Continue);
        }

        // Generate a simple slug from the session key.
        let date = utc_date_string();
        let slug = session_key
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-')
            .take(30)
            .collect::<String>();
        let filename = format!("session-{slug}-{date}.md");
        let path = memory_dir.join(&filename);

        // Build markdown from conversation history.
        let mut content = format!(
            "# Session Log\n\n- **Session**: {session_key}\n- **Date**: {date}\n- **Action**: {action}\n- **Messages**: {}\n\n",
            messages.len()
        );

        let msg_limit = messages.len().min(self.max_messages);
        let start = messages.len().saturating_sub(msg_limit);
        if start > 0 {
            content.push_str(&format!("_({start} earlier messages omitted)_\n\n"));
        }

        for msg in &messages[start..] {
            let role = msg
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let text = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
            // Truncate very long messages to keep the memory file manageable.
            let truncated = if text.len() > 2000 {
                format!("{}...\n\n_(truncated)_", &text[..2000])
            } else {
                text.to_string()
            };
            content.push_str(&format!("## {role}\n\n{truncated}\n\n"));
        }

        match tokio::fs::write(&path, &content).await {
            Ok(()) => {
                info!(path = %path.display(), messages = messages.len(), "session-memory: saved session log");
            },
            Err(e) => {
                warn!(error = %e, "session-memory: failed to write memory file");
            },
        }

        debug!(
            session = %session_key,
            file = %filename,
            "session-memory hook completed"
        );

        Ok(HookAction::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn saves_memory_on_new_command() {
        let tmp = tempfile::tempdir().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let session_store = Arc::new(SessionStore::new(sessions_dir));

        // Populate a session with some messages.
        let _ = session_store
            .append(
                "test-session-123",
                &serde_json::json!({"role": "user", "content": "Hello"}),
            )
            .await;
        let _ = session_store
            .append(
                "test-session-123",
                &serde_json::json!({"role": "assistant", "content": "Hi there!"}),
            )
            .await;

        let hook = SessionMemoryHook::new(tmp.path().to_path_buf(), session_store);

        let payload = HookPayload::Command {
            session_key: "test-session-123".into(),
            action: "new".into(),
            sender_id: None,
        };
        hook.handle(HookEvent::Command, &payload).await.unwrap();

        let memory_dir = tmp.path().join("memory");
        assert!(memory_dir.is_dir());

        let files: Vec<_> = std::fs::read_dir(&memory_dir).unwrap().flatten().collect();
        assert_eq!(files.len(), 1);

        let content = std::fs::read_to_string(files[0].path()).unwrap();
        assert!(content.contains("test-session-123"));
        assert!(content.contains("Hello"));
        assert!(content.contains("Hi there!"));
    }

    #[tokio::test]
    async fn ignores_non_new_commands() {
        let tmp = tempfile::tempdir().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let session_store = Arc::new(SessionStore::new(sessions_dir));

        let hook = SessionMemoryHook::new(tmp.path().to_path_buf(), session_store);

        let payload = HookPayload::Command {
            session_key: "test".into(),
            action: "stop".into(),
            sender_id: None,
        };
        hook.handle(HookEvent::Command, &payload).await.unwrap();

        let memory_dir = tmp.path().join("memory");
        assert!(!memory_dir.exists());
    }

    #[tokio::test]
    async fn skips_empty_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let session_store = Arc::new(SessionStore::new(sessions_dir));

        let hook = SessionMemoryHook::new(tmp.path().to_path_buf(), session_store);

        let payload = HookPayload::Command {
            session_key: "empty-session".into(),
            action: "new".into(),
            sender_id: None,
        };
        hook.handle(HookEvent::Command, &payload).await.unwrap();

        let memory_dir = tmp.path().join("memory");
        // Should not create a file for an empty session.
        assert!(!memory_dir.exists());
    }

    #[tokio::test]
    async fn truncates_long_messages() {
        let tmp = tempfile::tempdir().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let session_store = Arc::new(SessionStore::new(sessions_dir));

        let long_text = "x".repeat(5000);
        let _ = session_store
            .append(
                "test",
                &serde_json::json!({"role": "user", "content": long_text}),
            )
            .await;

        let hook = SessionMemoryHook::new(tmp.path().to_path_buf(), session_store);

        let payload = HookPayload::Command {
            session_key: "test".into(),
            action: "reset".into(),
            sender_id: None,
        };
        hook.handle(HookEvent::Command, &payload).await.unwrap();

        let memory_dir = tmp.path().join("memory");
        let files: Vec<_> = std::fs::read_dir(&memory_dir).unwrap().flatten().collect();
        let content = std::fs::read_to_string(files[0].path()).unwrap();
        assert!(content.contains("_(truncated)_"));
        // Should not contain the full 5000 chars
        assert!(content.len() < 4000);
    }
}
