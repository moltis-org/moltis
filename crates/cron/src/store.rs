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
}
