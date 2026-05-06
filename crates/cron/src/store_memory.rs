//! In-memory store for testing.

use std::{collections::HashMap, sync::Mutex};

use async_trait::async_trait;

use crate::{
    Error, Result,
    store::CronStore,
    types::{CronJob, CronRunRecord},
};

/// In-memory store backed by `HashMap`. No persistence — for tests only.
pub struct InMemoryStore {
    jobs: Mutex<HashMap<String, CronJob>>,
    runs: Mutex<HashMap<String, Vec<CronRunRecord>>>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self {
            jobs: Mutex::new(HashMap::new()),
            runs: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CronStore for InMemoryStore {
    async fn load_jobs(&self) -> Result<Vec<CronJob>> {
        let jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        Ok(jobs.values().cloned().collect())
    }

    async fn save_job(&self, job: &CronJob) -> Result<()> {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        jobs.insert(job.id.clone(), job.clone());
        Ok(())
    }

    async fn delete_job(&self, id: &str) -> Result<()> {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        if jobs.remove(id).is_none() {
            return Err(Error::job_not_found(id));
        }
        Ok(())
    }

    async fn update_job(&self, job: &CronJob) -> Result<()> {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        if !jobs.contains_key(&job.id) {
            return Err(Error::job_not_found(job.id.clone()));
        }
        jobs.insert(job.id.clone(), job.clone());
        Ok(())
    }

    async fn append_run(&self, job_id: &str, run: &CronRunRecord) -> Result<()> {
        let mut runs = self.runs.lock().unwrap_or_else(|e| e.into_inner());
        runs.entry(job_id.to_string())
            .or_default()
            .push(run.clone());
        Ok(())
    }

    async fn get_runs(&self, job_id: &str, limit: usize) -> Result<Vec<CronRunRecord>> {
        let runs = self.runs.lock().unwrap_or_else(|e| e.into_inner());
        let records = runs.get(job_id).cloned().unwrap_or_default();
        // Return the most recent `limit` entries.
        let start = records.len().saturating_sub(limit);
        Ok(records[start..].to_vec())
    }

    async fn prune_runs_before(&self, before_ms: u64) -> Result<u64> {
        let mut runs = self.runs.lock().unwrap_or_else(|e| e.into_inner());
        let mut pruned = 0u64;
        for records in runs.values_mut() {
            let before_len = records.len();
            records.retain(|r| r.started_at_ms >= before_ms);
            pruned += (before_len - records.len()) as u64;
        }
        Ok(pruned)
    }

    async fn list_session_keys_before(&self, before_ms: u64) -> Result<Vec<String>> {
        let runs = self.runs.lock().unwrap_or_else(|e| e.into_inner());
        let mut keys = Vec::new();
        for records in runs.values() {
            for rec in records {
                if rec.started_at_ms < before_ms
                    && let Some(ref key) = rec.session_key
                    && !keys.contains(key)
                {
                    keys.push(key.clone());
                }
            }
        }
        Ok(keys)
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, crate::types::*};

    fn make_job(id: &str) -> CronJob {
        CronJob {
            id: id.into(),
            name: format!("job-{id}"),
            enabled: true,
            delete_after_run: false,
            schedule: CronSchedule::At { at_ms: 1000 },
            payload: CronPayload::SystemEvent { text: "hi".into() },
            session_target: SessionTarget::Main,
            state: CronJobState::default(),
            sandbox: CronSandboxConfig::default(),
            wake_mode: CronWakeMode::default(),
            system: false,
            created_at_ms: 1000,
            updated_at_ms: 1000,
        }
    }

    #[tokio::test]
    async fn test_save_load_roundtrip() {
        let store = InMemoryStore::new();
        let job = make_job("1");
        store.save_job(&job).await.unwrap();

        let jobs = store.load_jobs().await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, "1");
    }

    #[tokio::test]
    async fn test_delete() {
        let store = InMemoryStore::new();
        store.save_job(&make_job("1")).await.unwrap();
        store.delete_job("1").await.unwrap();
        assert!(store.load_jobs().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_delete_not_found() {
        let store = InMemoryStore::new();
        assert!(store.delete_job("nope").await.is_err());
    }

    #[tokio::test]
    async fn test_update() {
        let store = InMemoryStore::new();
        let mut job = make_job("1");
        store.save_job(&job).await.unwrap();
        job.name = "updated".into();
        store.update_job(&job).await.unwrap();
        let jobs = store.load_jobs().await.unwrap();
        assert_eq!(jobs[0].name, "updated");
    }

    #[tokio::test]
    async fn test_update_not_found() {
        let store = InMemoryStore::new();
        assert!(store.update_job(&make_job("1")).await.is_err());
    }

    #[tokio::test]
    async fn test_runs() {
        let store = InMemoryStore::new();
        for i in 0..5 {
            let run = CronRunRecord {
                job_id: "j1".into(),
                started_at_ms: i * 1000,
                finished_at_ms: i * 1000 + 500,
                status: RunStatus::Ok,
                error: None,
                duration_ms: 500,
                output: None,
                input_tokens: None,
                output_tokens: None,
                session_key: None,
            };
            store.append_run("j1", &run).await.unwrap();
        }
        let runs = store.get_runs("j1", 3).await.unwrap();
        assert_eq!(runs.len(), 3);
        // Should be the last 3
        assert_eq!(runs[0].started_at_ms, 2000);
    }

    #[tokio::test]
    async fn test_runs_empty() {
        let store = InMemoryStore::new();
        let runs = store.get_runs("none", 10).await.unwrap();
        assert!(runs.is_empty());
    }

    #[tokio::test]
    async fn test_prune_runs_before() {
        let store = InMemoryStore::new();
        for i in 0..5 {
            let run = CronRunRecord {
                job_id: "j1".into(),
                started_at_ms: i * 1000,
                finished_at_ms: i * 1000 + 500,
                status: RunStatus::Ok,
                error: None,
                duration_ms: 500,
                output: None,
                input_tokens: None,
                output_tokens: None,
                session_key: Some(format!("cron:{i}")),
            };
            store.append_run("j1", &run).await.unwrap();
        }
        let pruned = store.prune_runs_before(3000).await.unwrap();
        assert_eq!(pruned, 3);
        let remaining = store.get_runs("j1", 10).await.unwrap();
        assert_eq!(remaining.len(), 2);
    }

    #[tokio::test]
    async fn test_list_session_keys_before() {
        let store = InMemoryStore::new();
        for i in 0..4 {
            let run = CronRunRecord {
                job_id: "j1".into(),
                started_at_ms: i * 1000,
                finished_at_ms: i * 1000 + 500,
                status: RunStatus::Ok,
                error: None,
                duration_ms: 500,
                output: None,
                input_tokens: None,
                output_tokens: None,
                session_key: if i < 3 {
                    Some(format!("cron:sess-{i}"))
                } else {
                    None
                },
            };
            store.append_run("j1", &run).await.unwrap();
        }
        let keys = store.list_session_keys_before(2000).await.unwrap();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&"cron:sess-0".to_string()));
        assert!(keys.contains(&"cron:sess-1".to_string()));
    }
}
