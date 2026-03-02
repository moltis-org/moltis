use std::{
    cmp::Ordering,
    collections::HashMap,
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::Result;

/// A single session entry in the metadata index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    pub id: String,
    pub key: String,
    pub label: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub message_count: u32,
    #[serde(default)]
    pub last_seen_message_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_image: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_binding: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_point: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_disabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub version: u64,
}

/// JSON file-backed index mapping session key → SessionEntry.
pub struct SessionMetadata {
    path: PathBuf,
    entries: HashMap<String, SessionEntry>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn compare_sidebar_order(lhs: &SessionEntry, rhs: &SessionEntry) -> Ordering {
    let lhs_main = lhs.key == "main";
    let rhs_main = rhs.key == "main";

    rhs_main
        .cmp(&lhs_main)
        .then_with(|| rhs.updated_at.cmp(&lhs.updated_at))
        .then_with(|| rhs.created_at.cmp(&lhs.created_at))
        .then_with(|| lhs.key.cmp(&rhs.key))
}

impl SessionMetadata {
    /// Load metadata from disk, or create an empty index.
    pub fn load(path: PathBuf) -> Result<Self> {
        let entries = if path.exists() {
            let data = fs::read_to_string(&path)?;
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            HashMap::new()
        };
        Ok(Self { path, entries })
    }

    /// Persist metadata to disk.
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(&self.entries)?;
        fs::write(&self.path, data)?;
        Ok(())
    }

    /// Get an entry by key.
    pub fn get(&self, key: &str) -> Option<&SessionEntry> {
        self.entries.get(key)
    }

    /// Insert or update an entry. If key doesn't exist, creates a new entry.
    pub fn upsert(&mut self, key: &str, label: Option<String>) -> &SessionEntry {
        let now = now_ms();
        self.entries
            .entry(key.to_string())
            .and_modify(|e| {
                if let Some(ref l) = label
                    && e.label.as_deref() != Some(l)
                {
                    e.label = label.clone();
                    e.updated_at = now;
                    e.version += 1;
                }
            })
            .or_insert_with(|| SessionEntry {
                id: uuid::Uuid::new_v4().to_string(),
                key: key.to_string(),
                label,
                model: None,
                created_at: now,
                updated_at: now,
                message_count: 0,
                last_seen_message_count: 0,
                project_id: None,
                archived: false,
                worktree_branch: None,
                sandbox_enabled: None,
                sandbox_image: None,
                channel_binding: None,
                parent_session_key: None,
                fork_point: None,
                mcp_disabled: None,
                preview: None,
                agent_id: None,
                version: 0,
            })
    }

    /// Update the model associated with a session.
    pub fn set_model(&mut self, key: &str, model: Option<String>) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.model = model;
            entry.updated_at = now_ms();
            entry.version += 1;
        }
    }

    /// Update message count and updated_at timestamp.
    pub fn touch(&mut self, key: &str, message_count: u32) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.message_count = message_count;
            entry.updated_at = now_ms();
            entry.version += 1;
        }
    }

    /// Set the project_id for a session.
    pub fn set_project_id(&mut self, key: &str, project_id: Option<String>) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.project_id = project_id;
            entry.updated_at = now_ms();
            entry.version += 1;
        }
    }

    /// Set the worktree branch for a session.
    pub fn set_worktree_branch(&mut self, key: &str, branch: Option<String>) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.worktree_branch = branch;
            entry.updated_at = now_ms();
            entry.version += 1;
        }
    }

    /// Set the sandbox_image for a session.
    pub fn set_sandbox_image(&mut self, key: &str, image: Option<String>) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.sandbox_image = image;
            entry.updated_at = now_ms();
            entry.version += 1;
        }
    }

    /// Set the sandbox_enabled override for a session.
    pub fn set_sandbox_enabled(&mut self, key: &str, enabled: Option<bool>) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.sandbox_enabled = enabled;
            entry.updated_at = now_ms();
            entry.version += 1;
        }
    }

    /// Set the mcp_disabled override for a session.
    pub fn set_mcp_disabled(&mut self, key: &str, disabled: Option<bool>) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.mcp_disabled = disabled;
            entry.updated_at = now_ms();
            entry.version += 1;
        }
    }

    /// Set the channel binding for a session.
    pub fn set_channel_binding(&mut self, key: &str, binding: Option<String>) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.channel_binding = binding;
            entry.updated_at = now_ms();
            entry.version += 1;
        }
    }

    /// Assign (or unassign) a session to an agent persona.
    pub fn set_agent_id(&mut self, key: &str, agent_id: Option<String>) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.agent_id = agent_id;
            entry.updated_at = now_ms();
            entry.version += 1;
        }
    }

    /// List all sessions belonging to a given agent.
    pub fn list_by_agent_id(&self, agent_id: &str) -> Vec<SessionEntry> {
        let mut entries: Vec<_> = self
            .entries
            .values()
            .filter(|e| e.agent_id.as_deref() == Some(agent_id))
            .cloned()
            .collect();
        entries.sort_by_key(|a| a.created_at);
        entries
    }

    /// Delete all sessions belonging to a given agent. Returns the number of
    /// sessions removed.
    pub fn delete_by_agent_id(&mut self, agent_id: &str) -> u64 {
        let keys: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, e)| e.agent_id.as_deref() == Some(agent_id))
            .map(|(k, _)| k.clone())
            .collect();
        let count = keys.len() as u64;
        for key in keys {
            self.entries.remove(&key);
        }
        count
    }

    /// Remove an entry by key. Returns the removed entry if found.
    pub fn remove(&mut self, key: &str) -> Option<SessionEntry> {
        self.entries.remove(key)
    }

    /// List all entries for sidebar rendering.
    /// `main` is pinned first, then sessions are sorted by recency.
    pub fn list(&self) -> Vec<SessionEntry> {
        let mut entries: Vec<_> = self.entries.values().cloned().collect();
        entries.sort_by(compare_sidebar_order);
        entries
    }
}

// ── SQLite-backed session metadata ──────────────────────────────────

/// SQLite-backed session metadata store.
pub struct SqliteSessionMetadata {
    pool: sqlx::SqlitePool,
    event_bus: Option<crate::session_events::SessionEventBus>,
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    key: String,
    id: String,
    label: Option<String>,
    model: Option<String>,
    created_at: i64,
    updated_at: i64,
    message_count: i32,
    last_seen_message_count: i32,
    project_id: Option<String>,
    archived: i32,
    worktree_branch: Option<String>,
    sandbox_enabled: Option<i32>,
    sandbox_image: Option<String>,
    channel_binding: Option<String>,
    parent_session_key: Option<String>,
    fork_point: Option<i32>,
    mcp_disabled: Option<i32>,
    preview: Option<String>,
    agent_id: Option<String>,
    version: i64,
}

impl From<SessionRow> for SessionEntry {
    fn from(r: SessionRow) -> Self {
        Self {
            key: r.key,
            id: r.id,
            label: r.label,
            model: r.model,
            created_at: r.created_at as u64,
            updated_at: r.updated_at as u64,
            message_count: r.message_count as u32,
            last_seen_message_count: r.last_seen_message_count as u32,
            project_id: r.project_id,
            archived: r.archived != 0,
            worktree_branch: r.worktree_branch,
            sandbox_enabled: r.sandbox_enabled.map(|v| v != 0),
            sandbox_image: r.sandbox_image,
            channel_binding: r.channel_binding,
            parent_session_key: r.parent_session_key,
            fork_point: r.fork_point.map(|v| v as u32),
            mcp_disabled: r.mcp_disabled.map(|v| v != 0),
            preview: r.preview,
            agent_id: r.agent_id,
            version: r.version as u64,
        }
    }
}

impl SqliteSessionMetadata {
    pub fn new(pool: sqlx::SqlitePool) -> Self {
        Self {
            pool,
            event_bus: None,
        }
    }

    /// Create with an event bus that auto-publishes on mutations.
    pub fn with_event_bus(
        pool: sqlx::SqlitePool,
        bus: crate::session_events::SessionEventBus,
    ) -> Self {
        Self {
            pool,
            event_bus: Some(bus),
        }
    }

    /// Accessor for the event bus (subscribers call `.subscribe()` on it).
    pub fn event_bus(&self) -> Option<&crate::session_events::SessionEventBus> {
        self.event_bus.as_ref()
    }

    /// Publish an event if a bus is configured.
    fn emit(&self, event: crate::session_events::SessionEvent) {
        if let Some(bus) = &self.event_bus {
            bus.publish(event);
        }
    }

    /// Initialize the sessions table schema.
    ///
    /// **Deprecated**: Schema is now managed by sqlx migrations in the gateway crate.
    /// This method is retained for tests that use in-memory databases.
    #[doc(hidden)]
    pub async fn init(pool: &sqlx::SqlitePool) -> Result<()> {
        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS sessions (
                key             TEXT    PRIMARY KEY,
                id              TEXT    NOT NULL,
                label           TEXT,
                model           TEXT,
                created_at      INTEGER NOT NULL,
                updated_at      INTEGER NOT NULL,
                message_count   INTEGER NOT NULL DEFAULT 0,
                last_seen_message_count INTEGER NOT NULL DEFAULT 0,
                project_id      TEXT    REFERENCES projects(id) ON DELETE SET NULL,
                archived        INTEGER NOT NULL DEFAULT 0,
                worktree_branch TEXT,
                sandbox_enabled     INTEGER,
                sandbox_image       TEXT,
                channel_binding     TEXT,
                parent_session_key  TEXT,
                fork_point          INTEGER,
                mcp_disabled        INTEGER,
                preview             TEXT,
                agent_id            TEXT,
                version             INTEGER NOT NULL DEFAULT 0
            )"#,
        )
        .execute(pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_sessions_created_at ON sessions(created_at)")
            .execute(pool)
            .await
            .ok();

        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS channel_sessions (
                channel_type TEXT    NOT NULL,
                account_id   TEXT    NOT NULL,
                chat_id      TEXT    NOT NULL,
                session_key  TEXT    NOT NULL,
                updated_at   INTEGER NOT NULL,
                PRIMARY KEY (channel_type, account_id, chat_id)
            )"#,
        )
        .execute(pool)
        .await?;

        Ok(())
    }

    pub async fn get(&self, key: &str) -> Option<SessionEntry> {
        match sqlx::query_as::<_, SessionRow>("SELECT * FROM sessions WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
        {
            Ok(row) => row.map(Into::into),
            Err(e) => {
                tracing::error!("sessions.get failed: {e}");
                None
            },
        }
    }

    /// Insert or update an entry. Returns the entry.
    pub async fn upsert(
        &self,
        key: &str,
        label: Option<String>,
    ) -> std::result::Result<SessionEntry, sqlx::Error> {
        let now = now_ms() as i64;
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            r#"INSERT INTO sessions (key, id, label, created_at, updated_at, version)
               VALUES (?, ?, ?, ?, ?, 0)
               ON CONFLICT(key) DO UPDATE SET
                 label = COALESCE(excluded.label, sessions.label),
                 version = sessions.version + 1"#,
        )
        .bind(key)
        .bind(&id)
        .bind(&label)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        let entry = self
            .get(key)
            .await
            .ok_or_else(|| sqlx::Error::RowNotFound)?;
        // version == 0 means freshly inserted; > 0 means conflict-updated.
        if entry.version == 0 {
            self.emit(crate::session_events::SessionEvent::Created {
                session_key: key.to_string(),
            });
        } else {
            self.emit(crate::session_events::SessionEvent::Patched {
                session_key: key.to_string(),
            });
        }
        Ok(entry)
    }

    pub async fn set_model(&self, key: &str, model: Option<String>) {
        let now = now_ms() as i64;
        sqlx::query(
            "UPDATE sessions SET model = ?, updated_at = ?, version = version + 1 WHERE key = ?",
        )
        .bind(&model)
        .bind(now)
        .bind(key)
        .execute(&self.pool)
        .await
        .ok();
        self.emit(crate::session_events::SessionEvent::Patched {
            session_key: key.to_string(),
        });
    }

    pub async fn touch(&self, key: &str, message_count: u32) {
        let now = now_ms() as i64;
        sqlx::query(
            "UPDATE sessions SET message_count = ?, updated_at = ?, version = version + 1 WHERE key = ?",
        )
        .bind(message_count as i32)
        .bind(now)
        .bind(key)
            .execute(&self.pool)
            .await
            .ok();
        self.emit(crate::session_events::SessionEvent::Patched {
            session_key: key.to_string(),
        });
    }

    /// Set imported timestamps and message counters without replacing them with "now".
    pub async fn set_timestamps_and_counts(
        &self,
        key: &str,
        created_at: u64,
        updated_at: u64,
        message_count: u32,
        last_seen_message_count: u32,
    ) {
        sqlx::query(
            "UPDATE sessions SET created_at = ?, updated_at = ?, message_count = ?, last_seen_message_count = ?, version = version + 1 WHERE key = ?",
        )
        .bind(created_at as i64)
        .bind(updated_at as i64)
        .bind(message_count as i32)
        .bind(last_seen_message_count as i32)
        .bind(key)
        .execute(&self.pool)
        .await
        .ok();
    }

    /// Store a short preview of the first user message for sidebar display.
    pub async fn set_preview(&self, key: &str, preview: Option<&str>) {
        sqlx::query("UPDATE sessions SET preview = ?, version = version + 1 WHERE key = ?")
            .bind(preview)
            .bind(key)
            .execute(&self.pool)
            .await
            .ok();
        self.emit(crate::session_events::SessionEvent::Patched {
            session_key: key.to_string(),
        });
    }

    /// Mark a session as "seen" by setting `last_seen_message_count` to the
    /// current `message_count`.
    pub async fn mark_seen(&self, key: &str) {
        sqlx::query(
            "UPDATE sessions SET last_seen_message_count = message_count, version = version + 1 WHERE key = ?",
        )
        .bind(key)
            .execute(&self.pool)
            .await
            .ok();
        self.emit(crate::session_events::SessionEvent::Patched {
            session_key: key.to_string(),
        });
    }

    pub async fn set_project_id(&self, key: &str, project_id: Option<String>) {
        let now = now_ms() as i64;
        sqlx::query(
            "UPDATE sessions SET project_id = ?, updated_at = ?, version = version + 1 WHERE key = ?",
        )
        .bind(&project_id)
        .bind(now)
        .bind(key)
            .execute(&self.pool)
            .await
            .ok();
        self.emit(crate::session_events::SessionEvent::Patched {
            session_key: key.to_string(),
        });
    }

    pub async fn set_sandbox_image(&self, key: &str, image: Option<String>) {
        let now = now_ms() as i64;
        sqlx::query(
            "UPDATE sessions SET sandbox_image = ?, updated_at = ?, version = version + 1 WHERE key = ?",
        )
        .bind(&image)
        .bind(now)
        .bind(key)
            .execute(&self.pool)
            .await
            .ok();
        self.emit(crate::session_events::SessionEvent::Patched {
            session_key: key.to_string(),
        });
    }

    pub async fn set_sandbox_enabled(&self, key: &str, enabled: Option<bool>) {
        let now = now_ms() as i64;
        let val = enabled.map(|b| b as i32);
        sqlx::query(
            "UPDATE sessions SET sandbox_enabled = ?, updated_at = ?, version = version + 1 WHERE key = ?",
        )
        .bind(val)
        .bind(now)
        .bind(key)
            .execute(&self.pool)
            .await
            .ok();
        self.emit(crate::session_events::SessionEvent::Patched {
            session_key: key.to_string(),
        });
    }

    pub async fn set_worktree_branch(&self, key: &str, branch: Option<String>) {
        let now = now_ms() as i64;
        sqlx::query(
            "UPDATE sessions SET worktree_branch = ?, updated_at = ?, version = version + 1 WHERE key = ?",
        )
        .bind(&branch)
        .bind(now)
        .bind(key)
            .execute(&self.pool)
            .await
            .ok();
        self.emit(crate::session_events::SessionEvent::Patched {
            session_key: key.to_string(),
        });
    }

    pub async fn set_mcp_disabled(&self, key: &str, disabled: Option<bool>) {
        let now = now_ms() as i64;
        let val = disabled.map(|b| b as i32);
        sqlx::query(
            "UPDATE sessions SET mcp_disabled = ?, updated_at = ?, version = version + 1 WHERE key = ?",
        )
        .bind(val)
        .bind(now)
        .bind(key)
            .execute(&self.pool)
            .await
            .ok();
        self.emit(crate::session_events::SessionEvent::Patched {
            session_key: key.to_string(),
        });
    }

    pub async fn set_channel_binding(&self, key: &str, binding: Option<String>) {
        let now = now_ms() as i64;
        sqlx::query(
            "UPDATE sessions SET channel_binding = ?, updated_at = ?, version = version + 1 WHERE key = ?",
        )
        .bind(&binding)
        .bind(now)
        .bind(key)
            .execute(&self.pool)
            .await
            .ok();
        self.emit(crate::session_events::SessionEvent::Patched {
            session_key: key.to_string(),
        });
    }

    /// Assign (or unassign) a session to an agent persona.
    pub async fn set_agent_id(&self, key: &str, agent_id: Option<&str>) -> Result<()> {
        let now = now_ms() as i64;
        sqlx::query(
            "UPDATE sessions SET agent_id = ?, updated_at = ?, version = version + 1 WHERE key = ?",
        )
        .bind(agent_id)
        .bind(now)
        .bind(key)
        .execute(&self.pool)
        .await?;
        self.emit(crate::session_events::SessionEvent::Patched {
            session_key: key.to_string(),
        });
        Ok(())
    }

    /// List all sessions belonging to a given agent.
    pub async fn list_by_agent_id(&self, agent_id: &str) -> Result<Vec<SessionEntry>> {
        let rows = sqlx::query_as::<_, SessionRow>(
            "SELECT * FROM sessions WHERE agent_id = ? ORDER BY created_at ASC",
        )
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Delete all sessions belonging to a given agent (cascade).
    pub async fn delete_by_agent_id(&self, agent_id: &str) -> Result<u64> {
        let result = sqlx::query("DELETE FROM sessions WHERE agent_id = ?")
            .bind(agent_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    /// Set the parent session key and fork point for a branched session.
    pub async fn set_parent(&self, key: &str, parent_key: Option<String>, fork_point: Option<u32>) {
        let now = now_ms() as i64;
        let fp = fork_point.map(|v| v as i32);
        sqlx::query(
            "UPDATE sessions SET parent_session_key = ?, fork_point = ?, updated_at = ?, version = version + 1 WHERE key = ?",
        )
        .bind(&parent_key)
        .bind(fp)
        .bind(now)
        .bind(key)
        .execute(&self.pool)
        .await
        .ok();
        self.emit(crate::session_events::SessionEvent::Patched {
            session_key: key.to_string(),
        });
    }

    /// List all sessions that are children of the given parent key.
    pub async fn list_children(&self, parent_key: &str) -> Vec<SessionEntry> {
        sqlx::query_as::<_, SessionRow>(
            "SELECT * FROM sessions WHERE parent_session_key = ? ORDER BY created_at ASC",
        )
        .bind(parent_key)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(Into::into)
        .collect()
    }

    pub async fn remove(&self, key: &str) -> Option<SessionEntry> {
        let entry = self.get(key).await;
        sqlx::query("DELETE FROM sessions WHERE key = ?")
            .bind(key)
            .execute(&self.pool)
            .await
            .ok();
        if entry.is_some() {
            self.emit(crate::session_events::SessionEvent::Deleted {
                session_key: key.to_string(),
            });
        }
        entry
    }

    pub async fn list(&self) -> Vec<SessionEntry> {
        sqlx::query_as::<_, SessionRow>(
            "SELECT * FROM sessions ORDER BY CASE WHEN key = 'main' THEN 0 ELSE 1 END ASC, updated_at DESC, created_at DESC, key ASC",
        )
            .fetch_all(&self.pool)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(Into::into)
            .collect()
    }

    /// Get the active session key for a channel chat, if one has been explicitly set.
    pub async fn get_active_session(
        &self,
        channel_type: &str,
        account_id: &str,
        chat_id: &str,
    ) -> Option<String> {
        sqlx::query_scalar::<_, String>(
            "SELECT session_key FROM channel_sessions WHERE channel_type = ? AND account_id = ? AND chat_id = ?",
        )
        .bind(channel_type)
        .bind(account_id)
        .bind(chat_id)
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten()
    }

    /// Set (upsert) the active session key for a channel chat.
    pub async fn set_active_session(
        &self,
        channel_type: &str,
        account_id: &str,
        chat_id: &str,
        session_key: &str,
    ) {
        let now = now_ms() as i64;
        sqlx::query(
            r#"INSERT INTO channel_sessions (channel_type, account_id, chat_id, session_key, updated_at)
               VALUES (?, ?, ?, ?, ?)
               ON CONFLICT(channel_type, account_id, chat_id) DO UPDATE SET
                 session_key = excluded.session_key,
                 updated_at = excluded.updated_at"#,
        )
        .bind(channel_type)
        .bind(account_id)
        .bind(chat_id)
        .bind(session_key)
        .bind(now)
        .execute(&self.pool)
        .await
        .ok();
    }

    /// List all sessions that have been bound to a given channel chat
    /// (i.e. sessions whose `channel_binding` JSON contains the matching chat_id + account_id).
    pub async fn list_channel_sessions(
        &self,
        channel_type: &str,
        account_id: &str,
        chat_id: &str,
    ) -> Vec<SessionEntry> {
        // Build the expected channel_binding JSON substring for matching.
        let binding_pattern = format!(
            r#"%"channel_type":"{channel_type}"%"account_id":"{account_id}"%"chat_id":"{chat_id}"%"#,
        );
        sqlx::query_as::<_, SessionRow>(
            "SELECT * FROM sessions WHERE channel_binding LIKE ? ORDER BY created_at ASC",
        )
        .bind(&binding_pattern)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(Into::into)
        .collect()
    }

    /// List all sessions bound to a given channel account (any chat).
    pub async fn list_account_sessions(
        &self,
        channel_type: &str,
        account_id: &str,
    ) -> Vec<SessionEntry> {
        let pattern = format!(r#"%"channel_type":"{channel_type}"%"account_id":"{account_id}"%"#,);
        sqlx::query_as::<_, SessionRow>(
            "SELECT * FROM sessions WHERE channel_binding LIKE ? ORDER BY created_at ASC",
        )
        .bind(&pattern)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(Into::into)
        .collect()
    }

    /// Get all active session mappings for a given channel account.
    pub async fn list_active_sessions(
        &self,
        channel_type: &str,
        account_id: &str,
    ) -> Vec<(String, String)> {
        sqlx::query_as::<_, (String, String)>(
            "SELECT chat_id, session_key FROM channel_sessions WHERE channel_type = ? AND account_id = ?",
        )
        .bind(channel_type)
        .bind(account_id)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default()
    }

    /// No-op — SQLite auto-persists.
    pub fn save(&self) -> Result<()> {
        Ok(())
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upsert_and_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meta.json");
        let mut meta = SessionMetadata::load(path.clone()).unwrap();

        meta.upsert("main", None);
        meta.upsert("session:abc", Some("My Chat".to_string()));

        let list = meta.list();
        assert_eq!(list.len(), 2);
        let keys: Vec<&str> = list.iter().map(|e| e.key.as_str()).collect();
        assert!(keys.contains(&"main"));
        assert!(keys.contains(&"session:abc"));
        let abc = list.iter().find(|e| e.key == "session:abc").unwrap();
        assert_eq!(abc.label.as_deref(), Some("My Chat"));
    }

    #[test]
    fn test_list_pins_main_then_sorts_by_recency() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meta.json");
        let mut meta = SessionMetadata::load(path).unwrap();

        meta.upsert("main", None);
        meta.upsert("session:older", None);
        meta.upsert("session:newer", None);

        if let Some(entry) = meta.entries.get_mut("main") {
            entry.created_at = 1;
            entry.updated_at = 1;
        }
        if let Some(entry) = meta.entries.get_mut("session:older") {
            entry.created_at = 100;
            entry.updated_at = 100;
        }
        if let Some(entry) = meta.entries.get_mut("session:newer") {
            entry.created_at = 200;
            entry.updated_at = 200;
        }

        let keys: Vec<String> = meta.list().into_iter().map(|entry| entry.key).collect();
        assert_eq!(keys, vec!["main", "session:newer", "session:older"]);
    }

    #[test]
    fn test_save_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meta.json");

        {
            let mut meta = SessionMetadata::load(path.clone()).unwrap();
            meta.upsert("main", Some("Main".to_string()));
            meta.save().unwrap();
        }

        let meta = SessionMetadata::load(path).unwrap();
        let entry = meta.get("main").unwrap();
        assert_eq!(entry.label.as_deref(), Some("Main"));
    }

    #[test]
    fn test_remove() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meta.json");
        let mut meta = SessionMetadata::load(path).unwrap();

        meta.upsert("main", None);
        assert!(meta.get("main").is_some());
        meta.remove("main");
        assert!(meta.get("main").is_none());
    }

    async fn sqlite_pool() -> sqlx::SqlitePool {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        // sessions table references projects, so create a stub projects table.
        sqlx::query("CREATE TABLE IF NOT EXISTS projects (id TEXT PRIMARY KEY)")
            .execute(&pool)
            .await
            .unwrap();
        SqliteSessionMetadata::init(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn test_sqlite_upsert_and_list() {
        let pool = sqlite_pool().await;
        let meta = SqliteSessionMetadata::new(pool);

        meta.upsert("main", None).await.unwrap();
        meta.upsert("session:abc", Some("My Chat".to_string()))
            .await
            .unwrap();

        let list = meta.list().await;
        assert_eq!(list.len(), 2);
        let abc = list.iter().find(|e| e.key == "session:abc").unwrap();
        assert_eq!(abc.label.as_deref(), Some("My Chat"));
    }

    #[tokio::test]
    async fn test_sqlite_list_pins_main_then_sorts_by_recency() {
        let pool = sqlite_pool().await;
        let meta = SqliteSessionMetadata::new(pool);

        meta.upsert("main", None).await.unwrap();
        meta.upsert("session:older", None).await.unwrap();
        meta.upsert("session:newer", None).await.unwrap();

        meta.set_timestamps_and_counts("main", 1, 1, 0, 0).await;
        meta.set_timestamps_and_counts("session:older", 100, 100, 0, 0)
            .await;
        meta.set_timestamps_and_counts("session:newer", 200, 200, 0, 0)
            .await;

        let keys: Vec<String> = meta
            .list()
            .await
            .into_iter()
            .map(|entry| entry.key)
            .collect();
        assert_eq!(keys, vec!["main", "session:newer", "session:older"]);
    }

    #[tokio::test]
    async fn test_sqlite_remove() {
        let pool = sqlite_pool().await;
        let meta = SqliteSessionMetadata::new(pool);

        meta.upsert("main", None).await.unwrap();
        assert!(meta.get("main").await.is_some());
        meta.remove("main").await;
        assert!(meta.get("main").await.is_none());
    }

    #[tokio::test]
    async fn test_sqlite_touch() {
        let pool = sqlite_pool().await;
        let meta = SqliteSessionMetadata::new(pool);

        meta.upsert("main", None).await.unwrap();
        meta.touch("main", 5).await;
        assert_eq!(meta.get("main").await.unwrap().message_count, 5);
    }

    #[tokio::test]
    async fn test_sqlite_set_timestamps_and_counts() {
        let pool = sqlite_pool().await;
        let meta = SqliteSessionMetadata::new(pool);

        meta.upsert("main", None).await.unwrap();
        meta.set_timestamps_and_counts("main", 100, 200, 5, 3).await;

        let entry = meta.get("main").await.unwrap();
        assert_eq!(entry.created_at, 100);
        assert_eq!(entry.updated_at, 200);
        assert_eq!(entry.message_count, 5);
        assert_eq!(entry.last_seen_message_count, 3);
    }

    #[tokio::test]
    async fn test_sqlite_mark_seen() {
        let pool = sqlite_pool().await;
        let meta = SqliteSessionMetadata::new(pool);

        meta.upsert("main", None).await.unwrap();
        // New session starts with last_seen_message_count = 0.
        assert_eq!(meta.get("main").await.unwrap().last_seen_message_count, 0);

        // Simulate receiving messages.
        meta.touch("main", 5).await;
        // touch does NOT change last_seen_message_count.
        assert_eq!(meta.get("main").await.unwrap().last_seen_message_count, 0);

        // Mark as seen.
        meta.mark_seen("main").await;
        let entry = meta.get("main").await.unwrap();
        assert_eq!(entry.last_seen_message_count, 5);
        assert_eq!(entry.message_count, 5);

        // More messages arrive — last_seen stays at previous value.
        meta.touch("main", 8).await;
        let entry = meta.get("main").await.unwrap();
        assert_eq!(entry.message_count, 8);
        assert_eq!(entry.last_seen_message_count, 5);
    }

    #[tokio::test]
    async fn test_sqlite_mark_seen_emits_patched_event() {
        let pool = sqlite_pool().await;
        let bus = crate::session_events::SessionEventBus::new();
        let meta = SqliteSessionMetadata::with_event_bus(pool, bus.clone());
        let mut rx = bus.subscribe();

        meta.upsert("main", None).await.unwrap();
        let created = rx.recv().await.unwrap();
        assert!(
            matches!(
                created,
                crate::session_events::SessionEvent::Created { session_key } if session_key == "main"
            ),
            "expected created event after upsert"
        );

        meta.mark_seen("main").await;
        let patched = rx.recv().await.unwrap();
        assert!(
            matches!(
                patched,
                crate::session_events::SessionEvent::Patched { session_key } if session_key == "main"
            ),
            "expected patched event after mark_seen"
        );
    }

    #[test]
    fn test_touch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meta.json");
        let mut meta = SessionMetadata::load(path).unwrap();

        meta.upsert("main", None);
        meta.touch("main", 5);
        assert_eq!(meta.get("main").unwrap().message_count, 5);
    }

    #[test]
    fn test_sandbox_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meta.json");
        let mut meta = SessionMetadata::load(path.clone()).unwrap();

        meta.upsert("main", None);
        assert!(meta.get("main").unwrap().sandbox_enabled.is_none());

        meta.set_sandbox_enabled("main", Some(true));
        assert_eq!(meta.get("main").unwrap().sandbox_enabled, Some(true));

        meta.set_sandbox_enabled("main", None);
        assert!(meta.get("main").unwrap().sandbox_enabled.is_none());

        // Verify it round-trips through save/load.
        meta.set_sandbox_enabled("main", Some(false));
        meta.save().unwrap();
        let reloaded = SessionMetadata::load(path).unwrap();
        assert_eq!(reloaded.get("main").unwrap().sandbox_enabled, Some(false));
    }

    #[test]
    fn test_worktree_branch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meta.json");
        let mut meta = SessionMetadata::load(path.clone()).unwrap();

        meta.upsert("main", None);
        assert!(meta.get("main").unwrap().worktree_branch.is_none());

        meta.set_worktree_branch("main", Some("moltis/abc".to_string()));
        assert_eq!(
            meta.get("main").unwrap().worktree_branch.as_deref(),
            Some("moltis/abc")
        );

        meta.set_worktree_branch("main", None);
        assert!(meta.get("main").unwrap().worktree_branch.is_none());

        // Round-trip through save/load.
        meta.set_worktree_branch("main", Some("moltis/xyz".to_string()));
        meta.save().unwrap();
        let reloaded = SessionMetadata::load(path).unwrap();
        assert_eq!(
            reloaded.get("main").unwrap().worktree_branch.as_deref(),
            Some("moltis/xyz")
        );
    }

    #[tokio::test]
    async fn test_sqlite_worktree_branch() {
        let pool = sqlite_pool().await;
        let meta = SqliteSessionMetadata::new(pool);

        meta.upsert("main", None).await.unwrap();
        assert!(meta.get("main").await.unwrap().worktree_branch.is_none());

        meta.set_worktree_branch("main", Some("moltis/abc".to_string()))
            .await;
        assert_eq!(
            meta.get("main").await.unwrap().worktree_branch.as_deref(),
            Some("moltis/abc")
        );

        meta.set_worktree_branch("main", None).await;
        assert!(meta.get("main").await.unwrap().worktree_branch.is_none());
    }

    #[test]
    fn test_sandbox_image() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meta.json");
        let mut meta = SessionMetadata::load(path.clone()).unwrap();

        meta.upsert("main", None);
        assert!(meta.get("main").unwrap().sandbox_image.is_none());

        meta.set_sandbox_image("main", Some("custom:latest".to_string()));
        assert_eq!(
            meta.get("main").unwrap().sandbox_image.as_deref(),
            Some("custom:latest")
        );

        meta.set_sandbox_image("main", None);
        assert!(meta.get("main").unwrap().sandbox_image.is_none());

        // Round-trip through save/load.
        meta.set_sandbox_image("main", Some("alpine:3.20".to_string()));
        meta.save().unwrap();
        let reloaded = SessionMetadata::load(path).unwrap();
        assert_eq!(
            reloaded.get("main").unwrap().sandbox_image.as_deref(),
            Some("alpine:3.20")
        );
    }

    #[tokio::test]
    async fn test_sqlite_sandbox_image() {
        let pool = sqlite_pool().await;
        let meta = SqliteSessionMetadata::new(pool);

        meta.upsert("main", None).await.unwrap();
        assert!(meta.get("main").await.unwrap().sandbox_image.is_none());

        meta.set_sandbox_image("main", Some("custom:latest".to_string()))
            .await;
        assert_eq!(
            meta.get("main").await.unwrap().sandbox_image.as_deref(),
            Some("custom:latest")
        );

        meta.set_sandbox_image("main", None).await;
        assert!(meta.get("main").await.unwrap().sandbox_image.is_none());
    }

    #[test]
    fn test_sandbox_enabled_serde_compat() {
        // Existing metadata without sandbox_enabled should deserialize fine.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meta.json");
        fs::write(
            &path,
            r#"{"main":{"id":"1","key":"main","label":null,"created_at":0,"updated_at":0,"message_count":0}}"#,
        )
        .unwrap();
        let meta = SessionMetadata::load(path).unwrap();
        assert!(meta.get("main").unwrap().sandbox_enabled.is_none());
    }

    #[test]
    fn test_channel_binding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meta.json");
        let mut meta = SessionMetadata::load(path.clone()).unwrap();

        meta.upsert("tg:bot1:123", None);
        assert!(meta.get("tg:bot1:123").unwrap().channel_binding.is_none());

        let binding = r#"{"channel_type":"telegram","account_id":"bot1","chat_id":"123"}"#;
        meta.set_channel_binding("tg:bot1:123", Some(binding.to_string()));
        assert_eq!(
            meta.get("tg:bot1:123").unwrap().channel_binding.as_deref(),
            Some(binding)
        );

        meta.set_channel_binding("tg:bot1:123", None);
        assert!(meta.get("tg:bot1:123").unwrap().channel_binding.is_none());

        // Round-trip through save/load.
        meta.set_channel_binding("tg:bot1:123", Some(binding.to_string()));
        meta.save().unwrap();
        let reloaded = SessionMetadata::load(path).unwrap();
        assert_eq!(
            reloaded
                .get("tg:bot1:123")
                .unwrap()
                .channel_binding
                .as_deref(),
            Some(binding)
        );
    }

    #[tokio::test]
    async fn test_sqlite_active_session() {
        let pool = sqlite_pool().await;
        let meta = SqliteSessionMetadata::new(pool);

        // No active session initially.
        assert!(
            meta.get_active_session("telegram", "bot1", "123")
                .await
                .is_none()
        );

        // Set and get.
        meta.set_active_session("telegram", "bot1", "123", "session:abc")
            .await;
        assert_eq!(
            meta.get_active_session("telegram", "bot1", "123")
                .await
                .as_deref(),
            Some("session:abc")
        );

        // Overwrite.
        meta.set_active_session("telegram", "bot1", "123", "session:def")
            .await;
        assert_eq!(
            meta.get_active_session("telegram", "bot1", "123")
                .await
                .as_deref(),
            Some("session:def")
        );

        // Different chat_id is independent.
        assert!(
            meta.get_active_session("telegram", "bot1", "456")
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_sqlite_list_channel_sessions() {
        let pool = sqlite_pool().await;
        let meta = SqliteSessionMetadata::new(pool);

        let binding =
            r#"{"channel_type":"telegram","account_id":"bot1","chat_id":"123"}"#.to_string();

        // Create two sessions with the same channel binding.
        meta.upsert("telegram:bot1:123", Some("Session 1".into()))
            .await
            .unwrap();
        meta.set_channel_binding("telegram:bot1:123", Some(binding.clone()))
            .await;

        meta.upsert("session:new1", Some("Session 2".into()))
            .await
            .unwrap();
        meta.set_channel_binding("session:new1", Some(binding.clone()))
            .await;

        let sessions = meta.list_channel_sessions("telegram", "bot1", "123").await;
        assert_eq!(sessions.len(), 2);
        let keys: Vec<&str> = sessions.iter().map(|s| s.key.as_str()).collect();
        assert!(keys.contains(&"telegram:bot1:123"));
        assert!(keys.contains(&"session:new1"));

        // Different chat should return empty.
        let other = meta.list_channel_sessions("telegram", "bot1", "999").await;
        assert!(other.is_empty());
    }

    #[tokio::test]
    async fn test_sqlite_channel_binding() {
        let pool = sqlite_pool().await;
        let meta = SqliteSessionMetadata::new(pool);

        meta.upsert("tg:bot1:123", None).await.unwrap();
        assert!(
            meta.get("tg:bot1:123")
                .await
                .unwrap()
                .channel_binding
                .is_none()
        );

        let binding = r#"{"channel_type":"telegram","account_id":"bot1","chat_id":"123"}"#;
        meta.set_channel_binding("tg:bot1:123", Some(binding.to_string()))
            .await;
        assert_eq!(
            meta.get("tg:bot1:123")
                .await
                .unwrap()
                .channel_binding
                .as_deref(),
            Some(binding)
        );

        meta.set_channel_binding("tg:bot1:123", None).await;
        assert!(
            meta.get("tg:bot1:123")
                .await
                .unwrap()
                .channel_binding
                .is_none()
        );
    }

    #[test]
    fn test_mcp_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meta.json");
        let mut meta = SessionMetadata::load(path.clone()).unwrap();

        meta.upsert("main", None);
        assert!(meta.get("main").unwrap().mcp_disabled.is_none());

        meta.set_mcp_disabled("main", Some(true));
        assert_eq!(meta.get("main").unwrap().mcp_disabled, Some(true));

        meta.set_mcp_disabled("main", Some(false));
        assert_eq!(meta.get("main").unwrap().mcp_disabled, Some(false));

        meta.set_mcp_disabled("main", None);
        assert!(meta.get("main").unwrap().mcp_disabled.is_none());

        // Round-trip through save/load.
        meta.set_mcp_disabled("main", Some(true));
        meta.save().unwrap();
        let reloaded = SessionMetadata::load(path).unwrap();
        assert_eq!(reloaded.get("main").unwrap().mcp_disabled, Some(true));
    }

    #[tokio::test]
    async fn test_sqlite_mcp_disabled() {
        let pool = sqlite_pool().await;
        let meta = SqliteSessionMetadata::new(pool);

        meta.upsert("main", None).await.unwrap();
        assert!(meta.get("main").await.unwrap().mcp_disabled.is_none());

        meta.set_mcp_disabled("main", Some(true)).await;
        assert_eq!(meta.get("main").await.unwrap().mcp_disabled, Some(true));

        meta.set_mcp_disabled("main", Some(false)).await;
        assert_eq!(meta.get("main").await.unwrap().mcp_disabled, Some(false));

        meta.set_mcp_disabled("main", None).await;
        assert!(meta.get("main").await.unwrap().mcp_disabled.is_none());
    }

    #[tokio::test]
    async fn test_version_starts_at_zero() {
        let pool = sqlite_pool().await;
        let meta = SqliteSessionMetadata::new(pool);

        let entry = meta.upsert("main", None).await.unwrap();
        assert_eq!(entry.version, 0);
    }

    #[tokio::test]
    async fn test_version_increments_on_mutation() {
        let pool = sqlite_pool().await;
        let meta = SqliteSessionMetadata::new(pool);

        meta.upsert("main", None).await.unwrap();
        assert_eq!(meta.get("main").await.unwrap().version, 0);

        meta.set_model("main", Some("gpt-4".to_string())).await;
        assert_eq!(meta.get("main").await.unwrap().version, 1);

        meta.touch("main", 5).await;
        assert_eq!(meta.get("main").await.unwrap().version, 2);

        // Insert a project row so the FK constraint is satisfied.
        sqlx::query("INSERT INTO projects (id) VALUES ('proj1')")
            .execute(&meta.pool)
            .await
            .unwrap();
        meta.set_project_id("main", Some("proj1".to_string())).await;
        assert_eq!(meta.get("main").await.unwrap().version, 3);

        meta.set_sandbox_enabled("main", Some(true)).await;
        assert_eq!(meta.get("main").await.unwrap().version, 4);

        meta.set_sandbox_image("main", Some("img:1".to_string()))
            .await;
        assert_eq!(meta.get("main").await.unwrap().version, 5);

        meta.set_worktree_branch("main", Some("branch".to_string()))
            .await;
        assert_eq!(meta.get("main").await.unwrap().version, 6);

        meta.set_mcp_disabled("main", Some(true)).await;
        assert_eq!(meta.get("main").await.unwrap().version, 7);

        meta.set_channel_binding("main", Some("{}".to_string()))
            .await;
        assert_eq!(meta.get("main").await.unwrap().version, 8);

        meta.set_parent("main", Some("parent".to_string()), Some(0))
            .await;
        assert_eq!(meta.get("main").await.unwrap().version, 9);

        meta.mark_seen("main").await;
        assert_eq!(meta.get("main").await.unwrap().version, 10);

        meta.set_preview("main", Some("hello")).await;
        assert_eq!(meta.get("main").await.unwrap().version, 11);

        meta.set_agent_id("main", Some("agent-1")).await.unwrap();
        assert_eq!(meta.get("main").await.unwrap().version, 12);
    }

    #[tokio::test]
    async fn test_version_increments_on_upsert_update() {
        let pool = sqlite_pool().await;
        let meta = SqliteSessionMetadata::new(pool);

        meta.upsert("main", Some("First".to_string()))
            .await
            .unwrap();
        assert_eq!(meta.get("main").await.unwrap().version, 0);

        // Upsert with existing key bumps version via ON CONFLICT.
        meta.upsert("main", Some("Second".to_string()))
            .await
            .unwrap();
        assert_eq!(meta.get("main").await.unwrap().version, 1);
    }

    #[tokio::test]
    async fn test_version_in_list() {
        let pool = sqlite_pool().await;
        let meta = SqliteSessionMetadata::new(pool);

        meta.upsert("main", None).await.unwrap();
        meta.touch("main", 3).await;

        let list = meta.list().await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].version, 1);
    }

    #[test]
    fn test_json_backend_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meta.json");
        let mut meta = SessionMetadata::load(path.clone()).unwrap();

        meta.upsert("main", None);
        assert_eq!(meta.get("main").unwrap().version, 0);

        meta.set_model("main", Some("gpt-4".to_string()));
        assert_eq!(meta.get("main").unwrap().version, 1);

        meta.touch("main", 5);
        assert_eq!(meta.get("main").unwrap().version, 2);

        // Upsert with label change bumps version.
        meta.upsert("main", Some("New Label".to_string()));
        assert_eq!(meta.get("main").unwrap().version, 3);

        // Upsert without change does not bump version.
        meta.upsert("main", Some("New Label".to_string()));
        assert_eq!(meta.get("main").unwrap().version, 3);

        // Round-trip through save/load preserves version.
        meta.save().unwrap();
        let reloaded = SessionMetadata::load(path).unwrap();
        assert_eq!(reloaded.get("main").unwrap().version, 3);
    }

    #[test]
    fn test_agent_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meta.json");
        let mut meta = SessionMetadata::load(path.clone()).unwrap();

        meta.upsert("main", None);
        assert!(meta.get("main").unwrap().agent_id.is_none());

        meta.set_agent_id("main", Some("agent-1".to_string()));
        assert_eq!(
            meta.get("main").unwrap().agent_id.as_deref(),
            Some("agent-1")
        );

        meta.set_agent_id("main", None);
        assert!(meta.get("main").unwrap().agent_id.is_none());

        // Round-trip through save/load.
        meta.set_agent_id("main", Some("agent-2".to_string()));
        meta.save().unwrap();
        let reloaded = SessionMetadata::load(path).unwrap();
        assert_eq!(
            reloaded.get("main").unwrap().agent_id.as_deref(),
            Some("agent-2")
        );
    }

    #[test]
    fn test_list_by_agent_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meta.json");
        let mut meta = SessionMetadata::load(path).unwrap();

        meta.upsert("s1", Some("Session 1".to_string()));
        meta.upsert("s2", Some("Session 2".to_string()));
        meta.upsert("s3", Some("Session 3".to_string()));

        meta.set_agent_id("s1", Some("agent-a".to_string()));
        meta.set_agent_id("s2", Some("agent-a".to_string()));
        meta.set_agent_id("s3", Some("agent-b".to_string()));

        let agent_a = meta.list_by_agent_id("agent-a");
        assert_eq!(agent_a.len(), 2);
        let keys: Vec<&str> = agent_a.iter().map(|e| e.key.as_str()).collect();
        assert!(keys.contains(&"s1"));
        assert!(keys.contains(&"s2"));

        let agent_b = meta.list_by_agent_id("agent-b");
        assert_eq!(agent_b.len(), 1);
        assert_eq!(agent_b[0].key, "s3");

        let none = meta.list_by_agent_id("agent-missing");
        assert!(none.is_empty());
    }

    #[test]
    fn test_delete_by_agent_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meta.json");
        let mut meta = SessionMetadata::load(path).unwrap();

        meta.upsert("s1", None);
        meta.upsert("s2", None);
        meta.upsert("s3", None);

        meta.set_agent_id("s1", Some("agent-a".to_string()));
        meta.set_agent_id("s2", Some("agent-a".to_string()));
        meta.set_agent_id("s3", Some("agent-b".to_string()));

        let deleted = meta.delete_by_agent_id("agent-a");
        assert_eq!(deleted, 2);
        assert!(meta.get("s1").is_none());
        assert!(meta.get("s2").is_none());
        assert!(meta.get("s3").is_some());

        // Deleting a non-existent agent returns 0.
        let deleted = meta.delete_by_agent_id("agent-missing");
        assert_eq!(deleted, 0);
    }

    #[tokio::test]
    async fn test_sqlite_agent_id() {
        let pool = sqlite_pool().await;
        let meta = SqliteSessionMetadata::new(pool);

        meta.upsert("main", None).await.unwrap();
        assert!(meta.get("main").await.unwrap().agent_id.is_none());

        meta.set_agent_id("main", Some("agent-1")).await.unwrap();
        assert_eq!(
            meta.get("main").await.unwrap().agent_id.as_deref(),
            Some("agent-1")
        );

        meta.set_agent_id("main", None).await.unwrap();
        assert!(meta.get("main").await.unwrap().agent_id.is_none());
    }

    #[tokio::test]
    async fn test_sqlite_list_by_agent_id() {
        let pool = sqlite_pool().await;
        let meta = SqliteSessionMetadata::new(pool);

        meta.upsert("s1", Some("Session 1".to_string()))
            .await
            .unwrap();
        meta.upsert("s2", Some("Session 2".to_string()))
            .await
            .unwrap();
        meta.upsert("s3", Some("Session 3".to_string()))
            .await
            .unwrap();

        meta.set_agent_id("s1", Some("agent-a")).await.unwrap();
        meta.set_agent_id("s2", Some("agent-a")).await.unwrap();
        meta.set_agent_id("s3", Some("agent-b")).await.unwrap();

        let agent_a = meta.list_by_agent_id("agent-a").await.unwrap();
        assert_eq!(agent_a.len(), 2);
        let keys: Vec<&str> = agent_a.iter().map(|e| e.key.as_str()).collect();
        assert!(keys.contains(&"s1"));
        assert!(keys.contains(&"s2"));

        let agent_b = meta.list_by_agent_id("agent-b").await.unwrap();
        assert_eq!(agent_b.len(), 1);
        assert_eq!(agent_b[0].key, "s3");

        let none = meta.list_by_agent_id("agent-missing").await.unwrap();
        assert!(none.is_empty());
    }

    #[tokio::test]
    async fn test_sqlite_delete_by_agent_id() {
        let pool = sqlite_pool().await;
        let meta = SqliteSessionMetadata::new(pool);

        meta.upsert("s1", None).await.unwrap();
        meta.upsert("s2", None).await.unwrap();
        meta.upsert("s3", None).await.unwrap();

        meta.set_agent_id("s1", Some("agent-a")).await.unwrap();
        meta.set_agent_id("s2", Some("agent-a")).await.unwrap();
        meta.set_agent_id("s3", Some("agent-b")).await.unwrap();

        let deleted = meta.delete_by_agent_id("agent-a").await.unwrap();
        assert_eq!(deleted, 2);
        assert!(meta.get("s1").await.is_none());
        assert!(meta.get("s2").await.is_none());
        assert!(meta.get("s3").await.is_some());

        // Deleting a non-existent agent returns 0.
        let deleted = meta.delete_by_agent_id("agent-missing").await.unwrap();
        assert_eq!(deleted, 0);
    }

    #[test]
    fn test_agent_id_serde_compat() {
        // Existing metadata without agent_id should deserialize fine.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meta.json");
        fs::write(
            &path,
            r#"{"main":{"id":"1","key":"main","label":null,"created_at":0,"updated_at":0,"message_count":0}}"#,
        )
        .unwrap();
        let meta = SessionMetadata::load(path).unwrap();
        assert!(meta.get("main").unwrap().agent_id.is_none());
    }
}
