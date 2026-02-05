//! Scheduled agent runs with cron expressions.
//! Persistent storage at ~/.clawdbot/cron-jobs.json.
//! Isolated agent execution (no session), optional delivery to a channel.

pub mod heartbeat;
pub mod parse;
pub mod schedule;
pub mod service;
pub mod store;
pub mod store_file;
pub mod store_memory;
pub mod store_sqlite;
pub mod types;

/// Run database migrations for the cron crate.
///
/// This creates the `cron_jobs` and `cron_runs` tables. Should be called at
/// application startup when using [`store_sqlite::SqliteStore`].
pub async fn run_migrations(pool: &sqlx::SqlitePool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations")
        .set_ignore_missing(true)
        .run(pool)
        .await?;
    Ok(())
}
