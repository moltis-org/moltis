//! Session storage and management.
//!
//! Sessions are stored as JSONL files (one message per line) under
//! `<data_dir>/sessions/v1/`, using a reversible filesystem-safe encoding of
//! each session key. Legacy files are copied lazily when their key is accessed.

pub mod compaction;
pub mod error;
pub mod key;
pub mod message;
pub mod metadata;
pub mod session_events;
pub mod state_store;
mod storage_layout;
pub mod store;

pub use {
    error::{Error, Result},
    key::SessionKey,
    message::{ContentBlock, MessageContent, PersistedMessage, UserDocument},
    store::SearchResult,
};

/// Run database migrations for the sessions crate.
///
/// This creates the `sessions` and `channel_sessions` tables. Should be called
/// at application startup after [`moltis_projects::run_migrations`] (sessions
/// has a foreign key to projects).
pub async fn run_migrations(pool: &sqlx::SqlitePool) -> Result<()> {
    sqlx::migrate!("./migrations")
        .set_ignore_missing(true)
        .run(pool)
        .await?;
    Ok(())
}
