//! JSON file-backed cron store with atomic writes.

use std::path::PathBuf;

use {
    anyhow::{Context, Result},
    async_trait::async_trait,
    tokio::fs,
};

use crate::{
    store::CronStore,
    types::{CronJob, CronRunRecord},
};

/// File-backed store. Jobs in a single JSON file, runs as JSONL per job.
pub struct FileStore {
    jobs_path: PathBuf,
    runs_dir: PathBuf,
}

impl FileStore {
    pub fn new(jobs_path: PathBuf, runs_dir: PathBuf) -> Self {
        Self {
            jobs_path,
            runs_dir,
        }
    }

    /// Create a store using the default `~/.clawdbot/cron/` layout.
    pub fn default_path() -> Result<Self> {
        let home = dirs_next::home_dir().context("cannot determine home directory")?;
        let base = home.join(".clawdbot").join("cron");
        Ok(Self::new(base.join("jobs.json"), base.join("runs")))
    }

    async fn ensure_dirs(&self) -> Result<()> {
        if let Some(parent) = self.jobs_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::create_dir_all(&self.runs_dir).await?;
        Ok(())
    }

    /// Atomic write: write to temp, rename over target, keep `.bak`.
    async fn atomic_write_jobs(&self, jobs: &[CronJob]) -> Result<()> {
        self.ensure_dirs().await?;
        let json = serde_json::to_string_pretty(jobs)?;
        let tmp = self.jobs_path.with_extension("json.tmp");

        fs::write(&tmp, json.as_bytes()).await?;

        // Backup existing file.
        if fs::try_exists(&self.jobs_path).await.unwrap_or(false) {
            let bak = self.jobs_path.with_extension("json.bak");
            let _ = fs::rename(&self.jobs_path, &bak).await;
        }

        fs::rename(&tmp, &self.jobs_path).await?;
        Ok(())
    }

    fn runs_path(&self, job_id: &str) -> PathBuf {
        self.runs_dir.join(format!("{job_id}.jsonl"))
    }
}

#[async_trait]
impl CronStore for FileStore {
    async fn load_jobs(&self) -> Result<Vec<CronJob>> {
        if !fs::try_exists(&self.jobs_path).await.unwrap_or(false) {
            return Ok(Vec::new());
        }
        let data = fs::read_to_string(&self.jobs_path).await?;
        let jobs: Vec<CronJob> =
            serde_json::from_str(&data).context("failed to parse jobs.json")?;
        Ok(jobs)
    }

    async fn save_job(&self, job: &CronJob) -> Result<()> {
        let mut jobs = self.load_jobs().await?;
        // Replace existing or append.
        if let Some(pos) = jobs.iter().position(|j| j.id == job.id) {
            jobs[pos] = job.clone();
        } else {
            jobs.push(job.clone());
        }
        self.atomic_write_jobs(&jobs).await
    }

    async fn delete_job(&self, id: &str) -> Result<()> {
        let mut jobs = self.load_jobs().await?;
        let before = jobs.len();
        jobs.retain(|j| j.id != id);
        if jobs.len() == before {
            anyhow::bail!("job not found: {id}");
        }
        self.atomic_write_jobs(&jobs).await
    }

    async fn update_job(&self, job: &CronJob) -> Result<()> {
        let mut jobs = self.load_jobs().await?;
        let pos = jobs
            .iter()
            .position(|j| j.id == job.id)
            .ok_or_else(|| anyhow::anyhow!("job not found: {}", job.id))?;
        jobs[pos] = job.clone();
        self.atomic_write_jobs(&jobs).await
    }

    async fn append_run(&self, job_id: &str, run: &CronRunRecord) -> Result<()> {
        self.ensure_dirs().await?;
        let path = self.runs_path(job_id);
        let mut line = serde_json::to_string(run)?;
        line.push('\n');
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?
            .write_all(line.as_bytes())
            .await?;
        Ok(())
    }

    async fn get_runs(&self, job_id: &str, limit: usize) -> Result<Vec<CronRunRecord>> {
        let path = self.runs_path(job_id);
        if !fs::try_exists(&path).await.unwrap_or(false) {
            return Ok(Vec::new());
        }
        let data = fs::read_to_string(&path).await?;
        let all: Vec<CronRunRecord> = data
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        let start = all.len().saturating_sub(limit);
        Ok(all[start..].to_vec())
    }
}

use tokio::io::AsyncWriteExt;

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, crate::types::*, std::path::Path, tempfile::TempDir};

    fn make_store(dir: &Path) -> FileStore {
        FileStore::new(dir.join("jobs.json"), dir.join("runs"))
    }

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
            system: false,
            created_at_ms: 1000,
            updated_at_ms: 1000,
        }
    }

    #[tokio::test]
    async fn test_file_store_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let store = make_store(tmp.path());

        store.save_job(&make_job("1")).await.unwrap();
        store.save_job(&make_job("2")).await.unwrap();

        let jobs = store.load_jobs().await.unwrap();
        assert_eq!(jobs.len(), 2);
    }

    #[tokio::test]
    async fn test_file_store_delete() {
        let tmp = TempDir::new().unwrap();
        let store = make_store(tmp.path());

        store.save_job(&make_job("1")).await.unwrap();
        store.delete_job("1").await.unwrap();
        assert!(store.load_jobs().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_file_store_backup_created() {
        let tmp = TempDir::new().unwrap();
        let store = make_store(tmp.path());

        store.save_job(&make_job("1")).await.unwrap();
        store.save_job(&make_job("2")).await.unwrap();

        let bak = tmp.path().join("jobs.json.bak");
        assert!(bak.exists());
    }

    #[tokio::test]
    async fn test_file_store_runs() {
        let tmp = TempDir::new().unwrap();
        let store = make_store(tmp.path());

        let run = CronRunRecord {
            job_id: "j1".into(),
            started_at_ms: 1000,
            finished_at_ms: 2000,
            status: RunStatus::Ok,
            error: None,
            duration_ms: 1000,
            output: None,
            input_tokens: None,
            output_tokens: None,
        };
        store.append_run("j1", &run).await.unwrap();
        store.append_run("j1", &run).await.unwrap();

        let runs = store.get_runs("j1", 10).await.unwrap();
        assert_eq!(runs.len(), 2);
    }

    #[tokio::test]
    async fn test_file_store_load_empty() {
        let tmp = TempDir::new().unwrap();
        let store = make_store(tmp.path());
        assert!(store.load_jobs().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_file_store_update() {
        let tmp = TempDir::new().unwrap();
        let store = make_store(tmp.path());

        store.save_job(&make_job("1")).await.unwrap();
        let mut job = make_job("1");
        job.name = "updated".into();
        store.update_job(&job).await.unwrap();

        let jobs = store.load_jobs().await.unwrap();
        assert_eq!(jobs[0].name, "updated");
    }

    #[tokio::test]
    async fn test_file_store_save_replaces_existing() {
        let tmp = TempDir::new().unwrap();
        let store = make_store(tmp.path());

        store.save_job(&make_job("1")).await.unwrap();
        let mut job = make_job("1");
        job.name = "replaced".into();
        store.save_job(&job).await.unwrap();

        let jobs = store.load_jobs().await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].name, "replaced");
    }
}
