//! Project management for moltis.
//!
//! A project represents a codebase directory. When a session is bound to a
//! project, moltis loads `CLAUDE.md` and `AGENTS.md` context files from the
//! directory hierarchy and can create git worktrees for session isolation.

pub mod complete;
pub mod context;
pub mod detect;
pub mod store;
pub mod types;
pub mod worktree;

pub use {
    store::{ProjectStore, SqliteProjectStore, TomlProjectStore},
    types::{ContextFile, Project, ProjectContext},
    worktree::WorktreeManager,
};

/// Run database migrations for the projects crate.
///
/// This creates the `projects` table and indexes. Should be called at
/// application startup before using [`SqliteProjectStore`].
pub async fn run_migrations(pool: &sqlx::SqlitePool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations")
        .set_ignore_missing(true)
        .run(pool)
        .await?;
    Ok(())
}
