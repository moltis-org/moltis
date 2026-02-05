//! Session storage and management.
//!
//! Sessions are stored as JSONL files (one message per line) at
//! ~/.clawdbot/agents/<agentId>/sessions/<sessionKey>.jsonl
//! with file locking for concurrent access.

pub mod compaction;
pub mod key;
pub mod metadata;
pub mod store;

pub use {key::SessionKey, store::SearchResult};

/// Run database migrations for the sessions crate.
///
/// This creates the `sessions` and `channel_sessions` tables. Should be called
/// at application startup after [`moltis_projects::run_migrations`] (sessions
/// has a foreign key to projects).
pub async fn run_migrations(pool: &sqlx::SqlitePool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations")
        .set_ignore_missing(true)
        .run(pool)
        .await?;
    Ok(())
}
