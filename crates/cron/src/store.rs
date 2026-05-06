//! Persistence trait and implementations for cron jobs.

use async_trait::async_trait;

use crate::{
    Result,
    types::{CronJob, CronRunRecord},
};

/// Persistence backend for cron jobs and run history.
#[async_trait]
pub trait CronStore: Send + Sync {
    async fn load_jobs(&self) -> Result<Vec<CronJob>>;
    async fn save_job(&self, job: &CronJob) -> Result<()>;
    async fn delete_job(&self, id: &str) -> Result<()>;
    async fn update_job(&self, job: &CronJob) -> Result<()>;
    async fn append_run(&self, job_id: &str, run: &CronRunRecord) -> Result<()>;
    async fn get_runs(&self, job_id: &str, limit: usize) -> Result<Vec<CronRunRecord>>;

    /// Delete all run records older than `before_ms` (epoch millis).
    /// Returns the number of deleted rows.
    async fn prune_runs_before(&self, _before_ms: u64) -> Result<u64> {
        Ok(0)
    }

    /// List distinct session keys from run records older than `before_ms`.
    async fn list_session_keys_before(&self, _before_ms: u64) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
}
