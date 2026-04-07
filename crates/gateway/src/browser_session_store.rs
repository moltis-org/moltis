//! Persistent browser session history and action log.
//!
//! Stores session metadata and action log entries in SQLite so that
//! closed sessions remain visible in the browser management UI for
//! debugging and review.

use sqlx::SqlitePool;

/// Persistent browser session store backed by SQLite.
#[derive(Clone)]
pub struct BrowserSessionStore {
    pool: SqlitePool,
}

/// Summary of a browser session (active or closed).
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionRecord {
    pub session_id: String,
    pub sandboxed: bool,
    pub url: Option<String>,
    pub created_at: String,
    pub closed_at: Option<String>,
}

/// A single action logged against a browser session.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ActionLogEntry {
    pub id: i64,
    pub action: String,
    pub url: Option<String>,
    pub success: bool,
    pub error: Option<String>,
    pub duration_ms: i64,
    pub created_at: String,
}

impl BrowserSessionStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Record a new browser session.
    pub async fn start_session(&self, session_id: &str, sandboxed: bool) -> anyhow::Result<()> {
        sqlx::query("INSERT OR IGNORE INTO browser_sessions (session_id, sandboxed) VALUES (?, ?)")
            .bind(session_id)
            .bind(sandboxed)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Log a browser action.
    pub async fn log_action(
        &self,
        session_id: &str,
        action: &str,
        url: Option<&str>,
        success: bool,
        error: Option<&str>,
        duration_ms: u64,
    ) -> anyhow::Result<()> {
        // Also update the session's current URL
        if success {
            if let Some(url) = url {
                sqlx::query("UPDATE browser_sessions SET url = ? WHERE session_id = ?")
                    .bind(url)
                    .bind(session_id)
                    .execute(&self.pool)
                    .await?;
            }
        }

        sqlx::query(
            "INSERT INTO browser_action_log (session_id, action, url, success, error, duration_ms) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(action)
        .bind(url)
        .bind(success)
        .bind(error)
        .bind(duration_ms as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Mark a session as closed.
    pub async fn close_session(&self, session_id: &str) -> anyhow::Result<()> {
        sqlx::query("UPDATE browser_sessions SET closed_at = datetime('now') WHERE session_id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// List all sessions (active and closed), most recent first.
    pub async fn list_all(&self, limit: u32) -> anyhow::Result<Vec<SessionRecord>> {
        let rows = sqlx::query_as::<_, (String, bool, Option<String>, String, Option<String>)>(
            "SELECT session_id, sandboxed, url, created_at, closed_at \
             FROM browser_sessions ORDER BY created_at DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(
                |(session_id, sandboxed, url, created_at, closed_at)| SessionRecord {
                    session_id,
                    sandboxed,
                    url,
                    created_at,
                    closed_at,
                },
            )
            .collect())
    }

    /// Get the action log for a specific session.
    pub async fn get_actions(
        &self,
        session_id: &str,
        limit: u32,
    ) -> anyhow::Result<Vec<ActionLogEntry>> {
        let rows = sqlx::query_as::<
            _,
            (
                i64,
                String,
                Option<String>,
                bool,
                Option<String>,
                i64,
                String,
            ),
        >(
            "SELECT id, action, url, success, error, duration_ms, created_at \
             FROM browser_action_log WHERE session_id = ? ORDER BY created_at DESC LIMIT ?",
        )
        .bind(session_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(
                |(id, action, url, success, error, duration_ms, created_at)| ActionLogEntry {
                    id,
                    action,
                    url,
                    success,
                    error,
                    duration_ms,
                    created_at,
                },
            )
            .collect())
    }
}
