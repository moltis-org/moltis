use std::{
    cmp::Ordering,
    collections::HashMap,
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::Result;

/// External agent transport kind for session binding.
///
/// Defined in `moltis-sessions` so runtime crates can share the persisted type
/// without introducing a dependency from session storage to agent runtimes.
/// Serialises to kebab-case strings matching the canonical agent identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExternalAgentKind {
    ClaudeCode,
    Opencode,
    Codex,
    PiAgent,
    Acp,
}

impl ExternalAgentKind {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Opencode => "opencode",
            Self::Codex => "codex",
            Self::PiAgent => "pi-agent",
            Self::Acp => "acp",
        }
    }
}

impl std::fmt::Display for ExternalAgentKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ExternalAgentKind {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "claude-code" => Ok(Self::ClaudeCode),
            "opencode" => Ok(Self::Opencode),
            "codex" => Ok(Self::Codex),
            "pi-agent" => Ok(Self::PiAgent),
            "acp" => Ok(Self::Acp),
            other => Err(format!("unknown external agent kind: {other}")),
        }
    }
}

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
    pub sandbox_backend: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_agent_kind: Option<ExternalAgentKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_session_id: Option<String>,
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
                sandbox_backend: None,
                channel_binding: None,
                parent_session_key: None,
                fork_point: None,
                mcp_disabled: None,
                preview: None,
                agent_id: None,
                mode_id: None,
                node_id: None,
                external_agent_kind: None,
                external_session_id: None,
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

    /// Set the archived flag for a session.
    pub fn set_archived(&mut self, key: &str, archived: bool) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.archived = archived;
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

    /// Set the sandbox_backend override for a session.
    pub fn set_sandbox_backend(&mut self, key: &str, backend: Option<String>) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.sandbox_backend = backend;
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

    /// Assign (or clear) a session mode.
    pub fn set_mode_id(&mut self, key: &str, mode_id: Option<String>) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.mode_id = mode_id;
            entry.updated_at = now_ms();
            entry.version += 1;
        }
    }

    /// Assign (or unassign) a session to a remote node.
    pub fn set_node_id(&mut self, key: &str, node_id: Option<String>) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.node_id = node_id;
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
    sandbox_backend: Option<String>,
    channel_binding: Option<String>,
    parent_session_key: Option<String>,
    fork_point: Option<i32>,
    mcp_disabled: Option<i32>,
    preview: Option<String>,
    agent_id: Option<String>,
    mode_id: Option<String>,
    node_id: Option<String>,
    external_agent_kind: Option<String>,
    external_session_id: Option<String>,
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
            sandbox_backend: r.sandbox_backend,
            channel_binding: r.channel_binding,
            parent_session_key: r.parent_session_key,
            fork_point: r.fork_point.map(|v| v as u32),
            mcp_disabled: r.mcp_disabled.map(|v| v != 0),
            preview: r.preview,
            agent_id: r.agent_id,
            mode_id: r.mode_id,
            node_id: r.node_id,
            external_agent_kind: r
                .external_agent_kind
                .as_deref()
                .and_then(|kind| kind.parse().ok()),
            external_session_id: r.external_session_id,
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
                sandbox_backend     TEXT,
                channel_binding     TEXT,
                parent_session_key  TEXT,
                fork_point          INTEGER,
                mcp_disabled        INTEGER,
                preview             TEXT,
                agent_id            TEXT,
                mode_id             TEXT,
                node_id             TEXT,
                external_agent_kind TEXT,
                external_session_id TEXT,
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
                thread_id    TEXT    NOT NULL DEFAULT '',
                session_key  TEXT    NOT NULL,
                updated_at   INTEGER NOT NULL,
                PRIMARY KEY (channel_type, account_id, chat_id, thread_id)
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

    pub async fn set_external_agent(
        &self,
        key: &str,
        kind: Option<ExternalAgentKind>,
        external_session_id: Option<String>,
    ) {
        let now = now_ms() as i64;
        let kind = kind.map(|kind| kind.as_str().to_string());
        sqlx::query(
            "UPDATE sessions SET external_agent_kind = ?, external_session_id = ?, updated_at = ?, version = version + 1 WHERE key = ?",
        )
        .bind(&kind)
        .bind(&external_session_id)
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

    pub async fn set_archived(&self, key: &str, archived: bool) {
        let now = now_ms() as i64;
        let val = if archived {
            1
        } else {
            0
        };
        sqlx::query(
            "UPDATE sessions SET archived = ?, updated_at = ?, version = version + 1 WHERE key = ?",
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

    pub async fn set_sandbox_backend(&self, key: &str, backend: Option<String>) {
        let now = now_ms() as i64;
        sqlx::query(
            "UPDATE sessions SET sandbox_backend = ?, updated_at = ?, version = version + 1 WHERE key = ?",
        )
        .bind(&backend)
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

    /// Assign (or clear) a session mode.
    pub async fn set_mode_id(&self, key: &str, mode_id: Option<&str>) -> Result<()> {
        let now = now_ms() as i64;
        sqlx::query(
            "UPDATE sessions SET mode_id = ?, updated_at = ?, version = version + 1 WHERE key = ?",
        )
        .bind(mode_id)
        .bind(now)
        .bind(key)
        .execute(&self.pool)
        .await?;
        self.emit(crate::session_events::SessionEvent::Patched {
            session_key: key.to_string(),
        });
        Ok(())
    }

    /// Assign (or unassign) a session to a remote node.
    pub async fn set_node_id(&self, key: &str, node_id: Option<&str>) -> Result<()> {
        let now = now_ms() as i64;
        sqlx::query(
            "UPDATE sessions SET node_id = ?, updated_at = ?, version = version + 1 WHERE key = ?",
        )
        .bind(node_id)
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
        thread_id: Option<&str>,
    ) -> Option<String> {
        let tid = thread_id.unwrap_or("");
        sqlx::query_scalar::<_, String>(
            "SELECT session_key FROM channel_sessions WHERE channel_type = ? AND account_id = ? AND chat_id = ? AND thread_id = ?",
        )
        .bind(channel_type)
        .bind(account_id)
        .bind(chat_id)
        .bind(tid)
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
        thread_id: Option<&str>,
        session_key: &str,
    ) {
        let now = now_ms() as i64;
        let tid = thread_id.unwrap_or("");
        sqlx::query(
            r#"INSERT INTO channel_sessions (channel_type, account_id, chat_id, thread_id, session_key, updated_at)
               VALUES (?, ?, ?, ?, ?, ?)
               ON CONFLICT(channel_type, account_id, chat_id, thread_id) DO UPDATE SET
                 session_key = excluded.session_key,
                 updated_at = excluded.updated_at"#,
        )
        .bind(channel_type)
        .bind(account_id)
        .bind(chat_id)
        .bind(tid)
        .bind(session_key)
        .bind(now)
        .execute(&self.pool)
        .await
        .ok();
    }

    /// Clear any explicit channel chat mappings that currently point at the
    /// given session key.
    pub async fn clear_active_session_mappings(&self, session_key: &str) {
        sqlx::query("DELETE FROM channel_sessions WHERE session_key = ?")
            .bind(session_key)
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

#[cfg(test)]
mod tests;
