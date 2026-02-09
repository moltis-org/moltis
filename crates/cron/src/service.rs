//! Core cron scheduler: timer loop, job execution, CRUD operations.

use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use {
    anyhow::{Result, bail},
    tokio::{
        sync::{Mutex, Notify, RwLock},
        task::JoinHandle,
    },
    tracing::{debug, error, info, warn},
};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, cron as cron_metrics, gauge, histogram};

use crate::{schedule::compute_next_run, store::CronStore, types::*};

/// Result of an agent turn, including optional token usage.
#[derive(Debug, Clone)]
pub struct AgentTurnResult {
    pub output: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

/// Callback for running an isolated agent turn.
pub type AgentTurnFn = Arc<
    dyn Fn(AgentTurnRequest) -> Pin<Box<dyn Future<Output = Result<AgentTurnResult>> + Send>>
        + Send
        + Sync,
>;

/// Callback for injecting a system event into the main session.
pub type SystemEventFn = Arc<dyn Fn(String) + Send + Sync>;

/// Callback for notifying about cron job changes.
pub type NotifyFn = Arc<dyn Fn(crate::types::CronNotification) + Send + Sync>;

/// Rate limiting configuration for cron job creation.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Maximum number of jobs that can be created within the window.
    pub max_per_window: usize,
    /// Window duration in milliseconds.
    pub window_ms: u64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_per_window: 10,
            window_ms: 60_000, // 1 minute
        }
    }
}

/// Simple sliding-window rate limiter.
struct RateLimiter {
    timestamps: VecDeque<u64>,
    config: RateLimitConfig,
}

impl RateLimiter {
    fn new(config: RateLimitConfig) -> Self {
        Self {
            timestamps: VecDeque::new(),
            config,
        }
    }

    /// Check if a new job can be created. Returns Ok(()) if allowed, Err if rate limited.
    fn check(&mut self) -> Result<()> {
        let now = now_ms();
        let cutoff = now.saturating_sub(self.config.window_ms);

        // Remove expired timestamps.
        while self.timestamps.front().is_some_and(|&ts| ts < cutoff) {
            self.timestamps.pop_front();
        }

        if self.timestamps.len() >= self.config.max_per_window {
            bail!(
                "rate limit exceeded: max {} jobs per {} seconds",
                self.config.max_per_window,
                self.config.window_ms / 1000
            );
        }

        // Record this attempt.
        self.timestamps.push_back(now);
        Ok(())
    }
}

/// Parameters passed to the agent turn callback.
#[derive(Debug, Clone)]
pub struct AgentTurnRequest {
    pub message: String,
    pub model: Option<String>,
    pub timeout_secs: Option<u64>,
    pub deliver: bool,
    pub channel: Option<String>,
    pub to: Option<String>,
    pub session_target: SessionTarget,
    pub sandbox: CronSandboxConfig,
}

/// The cron scheduler.
pub struct CronService {
    store: Arc<dyn CronStore>,
    jobs: RwLock<Vec<CronJob>>,
    timer_handle: Mutex<Option<JoinHandle<()>>>,
    wake_notify: Arc<Notify>,
    running: RwLock<bool>,
    on_system_event: SystemEventFn,
    on_agent_turn: AgentTurnFn,
    on_notify: Option<NotifyFn>,
    rate_limiter: Mutex<RateLimiter>,
}

/// Max time a job can be in "running" state before we consider it stuck (2 hours).
const STUCK_THRESHOLD_MS: u64 = 2 * 60 * 60 * 1000;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl CronService {
    pub fn new(
        store: Arc<dyn CronStore>,
        on_system_event: SystemEventFn,
        on_agent_turn: AgentTurnFn,
    ) -> Arc<Self> {
        Self::with_config(
            store,
            on_system_event,
            on_agent_turn,
            None,
            RateLimitConfig::default(),
        )
    }

    /// Create a new cron service with a notification callback.
    pub fn with_notify(
        store: Arc<dyn CronStore>,
        on_system_event: SystemEventFn,
        on_agent_turn: AgentTurnFn,
        on_notify: NotifyFn,
    ) -> Arc<Self> {
        Self::with_config(
            store,
            on_system_event,
            on_agent_turn,
            Some(on_notify),
            RateLimitConfig::default(),
        )
    }

    /// Create a new cron service with all configuration options.
    pub fn with_config(
        store: Arc<dyn CronStore>,
        on_system_event: SystemEventFn,
        on_agent_turn: AgentTurnFn,
        on_notify: Option<NotifyFn>,
        rate_limit_config: RateLimitConfig,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            jobs: RwLock::new(Vec::new()),
            timer_handle: Mutex::new(None),
            wake_notify: Arc::new(Notify::new()),
            running: RwLock::new(false),
            on_system_event,
            on_agent_turn,
            on_notify,
            rate_limiter: Mutex::new(RateLimiter::new(rate_limit_config)),
        })
    }

    /// Emit a notification if a callback is registered.
    fn notify(&self, notification: crate::types::CronNotification) {
        if let Some(ref notify_fn) = self.on_notify {
            notify_fn(notification);
        }
    }

    /// Load jobs from store and start the timer loop.
    pub async fn start(self: &Arc<Self>) -> Result<()> {
        let loaded = self.store.load_jobs().await?;
        info!(count = loaded.len(), "loaded cron jobs");

        {
            let mut jobs = self.jobs.write().await;
            *jobs = loaded;
        }

        // Recompute next runs for all enabled jobs.
        self.recompute_all_next_runs().await;

        *self.running.write().await = true;

        let svc = Arc::clone(self);
        let handle = tokio::spawn(async move {
            svc.timer_loop().await;
        });

        *self.timer_handle.lock().await = Some(handle);
        Ok(())
    }

    /// Stop the timer loop.
    pub async fn stop(&self) {
        *self.running.write().await = false;
        self.wake_notify.notify_one();

        let mut handle = self.timer_handle.lock().await;
        if let Some(h) = handle.take() {
            h.abort();
        }
        info!("cron service stopped");
    }

    /// Add a new job.
    pub async fn add(&self, create: CronJobCreate) -> Result<CronJob> {
        // Check rate limit (skip for system jobs like heartbeat).
        if !create.system {
            self.rate_limiter.lock().await.check()?;
        }

        let now = now_ms();
        let mut job = CronJob {
            id: create
                .id
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name: create.name,
            enabled: create.enabled,
            delete_after_run: create.delete_after_run,
            schedule: create.schedule,
            payload: create.payload,
            session_target: create.session_target,
            state: CronJobState::default(),
            sandbox: create.sandbox,
            system: create.system,
            created_at_ms: now,
            updated_at_ms: now,
        };

        // Validate session_target + payload combo.
        validate_job_spec(&job)?;

        // Compute next run.
        if job.enabled {
            job.state.next_run_at_ms = compute_next_run(&job.schedule, now)?;
        }

        self.store.save_job(&job).await?;

        {
            let mut jobs = self.jobs.write().await;
            jobs.push(job.clone());
        }

        self.wake_notify.notify_one();
        self.notify(crate::types::CronNotification::Created { job: job.clone() });
        info!(id = %job.id, name = %job.name, "cron job added");
        Ok(job)
    }

    /// Update an existing job.
    pub async fn update(&self, id: &str, patch: CronJobPatch) -> Result<CronJob> {
        let now = now_ms();
        let mut jobs = self.jobs.write().await;
        let job = jobs
            .iter_mut()
            .find(|j| j.id == id)
            .ok_or_else(|| anyhow::anyhow!("job not found: {id}"))?;

        if let Some(name) = patch.name {
            job.name = name;
        }
        if let Some(schedule) = patch.schedule {
            job.schedule = schedule;
        }
        if let Some(payload) = patch.payload {
            job.payload = payload;
        }
        if let Some(target) = patch.session_target {
            job.session_target = target;
        }
        if let Some(enabled) = patch.enabled {
            job.enabled = enabled;
        }
        if let Some(delete_after) = patch.delete_after_run {
            job.delete_after_run = delete_after;
        }
        if let Some(sandbox) = patch.sandbox {
            job.sandbox = sandbox;
        }

        job.updated_at_ms = now;

        validate_job_spec(job)?;

        // Recompute next run.
        if job.enabled {
            job.state.next_run_at_ms = compute_next_run(&job.schedule, now)?;
        } else {
            job.state.next_run_at_ms = None;
        }

        let updated = job.clone();
        self.store.update_job(&updated).await?;

        drop(jobs);
        self.wake_notify.notify_one();
        self.notify(crate::types::CronNotification::Updated {
            job: updated.clone(),
        });
        info!(id, "cron job updated");
        Ok(updated)
    }

    /// Remove a job.
    pub async fn remove(&self, id: &str) -> Result<()> {
        self.store.delete_job(id).await?;
        let mut jobs = self.jobs.write().await;
        jobs.retain(|j| j.id != id);
        drop(jobs);
        self.notify(crate::types::CronNotification::Removed {
            job_id: id.to_string(),
        });
        info!(id, "cron job removed");
        Ok(())
    }

    /// List all jobs.
    pub async fn list(&self) -> Vec<CronJob> {
        self.jobs.read().await.clone()
    }

    /// Force-run a job immediately.
    pub async fn run(self: &Arc<Self>, id: &str, force: bool) -> Result<()> {
        let job = {
            let jobs = self.jobs.read().await;
            jobs.iter()
                .find(|j| j.id == id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("job not found: {id}"))?
        };

        if !job.enabled && !force {
            bail!("job is disabled (use force=true to override)");
        }

        // Mark as running before executing (prevents duplicate runs).
        let now = now_ms();
        self.update_job_state(&job.id, |state| {
            state.running_at_ms = Some(now);
        })
        .await;

        self.execute_job(&job).await;
        Ok(())
    }

    /// Get run history for a job.
    pub async fn runs(&self, job_id: &str, limit: usize) -> Result<Vec<CronRunRecord>> {
        self.store.get_runs(job_id, limit).await
    }

    /// Get scheduler status.
    /// Counts exclude system jobs (e.g. heartbeat) to match what the UI shows.
    pub async fn status(&self) -> CronStatus {
        let jobs = self.jobs.read().await;
        let running = *self.running.read().await;
        // Exclude system jobs from counts (they're hidden in the UI).
        let user_jobs: Vec<_> = jobs.iter().filter(|j| !j.system).collect();
        let enabled_count = user_jobs.iter().filter(|j| j.enabled).count();
        let next_run_at_ms = user_jobs
            .iter()
            .filter_map(|j| j.state.next_run_at_ms)
            .min();

        #[cfg(feature = "metrics")]
        gauge!(cron_metrics::JOBS_SCHEDULED).set(user_jobs.len() as f64);

        CronStatus {
            running,
            job_count: user_jobs.len(),
            enabled_count,
            next_run_at_ms,
        }
    }

    // ── Internal ────────────────────────────────────────────────────────

    async fn timer_loop(self: &Arc<Self>) {
        loop {
            if !*self.running.read().await {
                break;
            }

            let sleep_ms = self.ms_until_next_wake().await;

            if sleep_ms > 0 {
                let notify = Arc::clone(&self.wake_notify);
                tokio::select! {
                    () = tokio::time::sleep(Duration::from_millis(sleep_ms)) => {},
                    () = notify.notified() => {
                        debug!("timer loop woken by notify");
                        continue;
                    },
                }
            }

            if !*self.running.read().await {
                break;
            }

            self.process_due_jobs().await;
        }
    }

    async fn ms_until_next_wake(&self) -> u64 {
        let jobs = self.jobs.read().await;
        let now = now_ms();
        jobs.iter()
            .filter(|j| j.enabled)
            .filter_map(|j| j.state.next_run_at_ms)
            .map(|t| t.saturating_sub(now))
            .min()
            .unwrap_or(60_000) // poll every 60s if no jobs
    }

    async fn process_due_jobs(self: &Arc<Self>) {
        let now = now_ms();
        let due_jobs: Vec<CronJob> = {
            let mut jobs = self.jobs.write().await;
            let mut due = Vec::new();
            for job in jobs.iter_mut() {
                if job.enabled
                    && job.state.next_run_at_ms.is_some_and(|t| t <= now)
                    && job.state.running_at_ms.is_none()
                {
                    // Mark as running under the write lock BEFORE spawning,
                    // so the next timer tick won't pick up the same job again.
                    job.state.running_at_ms = Some(now);
                    due.push(job.clone());
                }
            }
            due
        };

        // Clear stuck jobs.
        self.clear_stuck_jobs(now).await;

        for job in due_jobs {
            let svc = Arc::clone(self);
            let job_clone = job.clone();
            tokio::spawn(async move {
                svc.execute_job(&job_clone).await;
            });
        }
    }

    async fn execute_job(self: &Arc<Self>, job: &CronJob) {
        let started = now_ms();
        info!(id = %job.id, name = %job.name, "executing cron job");

        #[cfg(feature = "metrics")]
        counter!(cron_metrics::EXECUTIONS_TOTAL).increment(1);

        // running_at_ms was already set in process_due_jobs() before spawning.

        let result = match &job.payload {
            CronPayload::SystemEvent { text } => {
                (self.on_system_event)(text.clone());
                Ok(AgentTurnResult {
                    output: "system event injected".to_string(),
                    input_tokens: None,
                    output_tokens: None,
                })
            },
            CronPayload::AgentTurn {
                message,
                model,
                timeout_secs,
                deliver,
                channel,
                to,
            } => {
                let req = AgentTurnRequest {
                    message: message.clone(),
                    model: model.clone(),
                    timeout_secs: *timeout_secs,
                    deliver: *deliver,
                    channel: channel.clone(),
                    to: to.clone(),
                    session_target: job.session_target.clone(),
                    sandbox: job.sandbox.clone(),
                };
                (self.on_agent_turn)(req).await
            },
        };

        let finished = now_ms();
        let duration_ms = finished - started;
        let (status, error_msg, output, input_tokens, output_tokens) = match &result {
            Ok(r) => {
                #[cfg(feature = "metrics")]
                {
                    if let Some(input) = r.input_tokens {
                        counter!(cron_metrics::INPUT_TOKENS_TOTAL).increment(input);
                    }
                    if let Some(output) = r.output_tokens {
                        counter!(cron_metrics::OUTPUT_TOKENS_TOTAL).increment(output);
                    }
                }
                (
                    RunStatus::Ok,
                    None,
                    Some(r.output.clone()),
                    r.input_tokens,
                    r.output_tokens,
                )
            },
            Err(e) => {
                error!(id = %job.id, error = %e, "cron job failed");
                #[cfg(feature = "metrics")]
                counter!(cron_metrics::ERRORS_TOTAL).increment(1);
                (RunStatus::Error, Some(e.to_string()), None, None, None)
            },
        };

        #[cfg(feature = "metrics")]
        histogram!(cron_metrics::EXECUTION_DURATION_SECONDS).record(duration_ms as f64 / 1000.0);

        // Record run.
        let run = CronRunRecord {
            job_id: job.id.clone(),
            started_at_ms: started,
            finished_at_ms: finished,
            status,
            error: error_msg.clone(),
            duration_ms,
            output,
            input_tokens,
            output_tokens,
        };
        if let Err(e) = self.store.append_run(&job.id, &run).await {
            warn!(error = %e, "failed to record cron run");
        }

        // Update job state.
        let now = now_ms();
        let next_run = compute_next_run(&job.schedule, now).unwrap_or(None);

        self.update_job_state(&job.id, |state| {
            state.running_at_ms = None;
            state.last_run_at_ms = Some(finished);
            state.last_status = Some(status);
            state.last_error = error_msg;
            state.last_duration_ms = Some(duration_ms);
            state.next_run_at_ms = next_run;
        })
        .await;

        // Handle one-shot jobs.
        if next_run.is_none() {
            if job.delete_after_run {
                let _ = self.remove(&job.id).await;
                info!(id = %job.id, "one-shot job deleted after run");
            } else {
                // Disable it.
                let mut jobs = self.jobs.write().await;
                if let Some(j) = jobs.iter_mut().find(|j| j.id == job.id) {
                    j.enabled = false;
                    let _ = self.store.update_job(j).await;
                }
            }
        } else {
            // Persist updated state.
            let jobs = self.jobs.read().await;
            if let Some(j) = jobs.iter().find(|j| j.id == job.id) {
                let _ = self.store.update_job(j).await;
            }
        }

        info!(
            id = %job.id,
            status = ?status,
            duration_ms,
            "cron job finished"
        );
    }

    async fn update_job_state<F: FnOnce(&mut CronJobState)>(&self, id: &str, f: F) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            f(&mut job.state);
        }
    }

    async fn recompute_all_next_runs(&self) {
        let now = now_ms();
        let mut jobs = self.jobs.write().await;
        for job in jobs.iter_mut() {
            if job.enabled {
                job.state.next_run_at_ms = compute_next_run(&job.schedule, now).unwrap_or(None);
            }
        }
    }

    async fn clear_stuck_jobs(&self, now: u64) {
        let mut jobs = self.jobs.write().await;
        for job in jobs.iter_mut() {
            if let Some(running_at) = job.state.running_at_ms
                && now.saturating_sub(running_at) > STUCK_THRESHOLD_MS
            {
                warn!(id = %job.id, "clearing stuck cron job");
                job.state.running_at_ms = None;
                job.state.last_status = Some(RunStatus::Error);
                job.state.last_error = Some("stuck: exceeded 2h timeout".into());
                #[cfg(feature = "metrics")]
                counter!(cron_metrics::STUCK_JOBS_CLEARED_TOTAL).increment(1);
            }
        }
    }
}

/// Validate session_target + payload compatibility.
fn validate_job_spec(job: &CronJob) -> Result<()> {
    match (&job.session_target, &job.payload) {
        (SessionTarget::Main, CronPayload::AgentTurn { .. }) => {
            bail!("sessionTarget=main requires payload kind=systemEvent");
        },
        (SessionTarget::Isolated | SessionTarget::Named(_), CronPayload::SystemEvent { .. }) => {
            bail!("sessionTarget=isolated/named requires payload kind=agentTurn");
        },
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use {super::*, crate::store_memory::InMemoryStore};

    fn noop_system_event() -> SystemEventFn {
        Arc::new(|_text| {})
    }

    fn noop_agent_turn() -> AgentTurnFn {
        Arc::new(|_req| {
            Box::pin(async {
                Ok(AgentTurnResult {
                    output: "ok".into(),
                    input_tokens: None,
                    output_tokens: None,
                })
            })
        })
    }

    fn counting_system_event(counter: Arc<AtomicUsize>) -> SystemEventFn {
        Arc::new(move |_text| {
            counter.fetch_add(1, Ordering::SeqCst);
        })
    }

    fn counting_agent_turn(counter: Arc<AtomicUsize>) -> AgentTurnFn {
        Arc::new(move |_req| {
            let c = Arc::clone(&counter);
            Box::pin(async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(AgentTurnResult {
                    output: "done".into(),
                    input_tokens: None,
                    output_tokens: None,
                })
            })
        })
    }

    fn make_svc(
        store: Arc<InMemoryStore>,
        sys: SystemEventFn,
        agent: AgentTurnFn,
    ) -> Arc<CronService> {
        CronService::new(store, sys, agent)
    }

    #[tokio::test]
    async fn test_add_and_list() {
        let store = Arc::new(InMemoryStore::new());
        let svc = make_svc(store.clone(), noop_system_event(), noop_agent_turn());

        let job = svc
            .add(CronJobCreate {
                id: None,
                name: "test".into(),
                schedule: CronSchedule::Every {
                    every_ms: 60_000,
                    anchor_ms: None,
                },
                payload: CronPayload::AgentTurn {
                    message: "hi".into(),
                    model: None,
                    timeout_secs: None,
                    deliver: false,
                    channel: None,
                    to: None,
                },
                session_target: SessionTarget::Isolated,
                delete_after_run: false,
                enabled: true,
                system: false,
                sandbox: CronSandboxConfig::default(),
            })
            .await
            .unwrap();

        let jobs = svc.list().await;
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, job.id);
        assert!(jobs[0].state.next_run_at_ms.is_some());
    }

    #[tokio::test]
    async fn test_add_validates_session_target() {
        let store = Arc::new(InMemoryStore::new());
        let svc = make_svc(store, noop_system_event(), noop_agent_turn());

        // main + agentTurn should fail
        let result = svc
            .add(CronJobCreate {
                id: None,
                name: "bad".into(),
                schedule: CronSchedule::At {
                    at_ms: 9999999999999,
                },
                payload: CronPayload::AgentTurn {
                    message: "hi".into(),
                    model: None,
                    timeout_secs: None,
                    deliver: false,
                    channel: None,
                    to: None,
                },
                session_target: SessionTarget::Main,
                delete_after_run: false,
                enabled: true,
                system: false,
                sandbox: CronSandboxConfig::default(),
            })
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_update_job() {
        let store = Arc::new(InMemoryStore::new());
        let svc = make_svc(store, noop_system_event(), noop_agent_turn());

        let job = svc
            .add(CronJobCreate {
                id: None,
                name: "orig".into(),
                schedule: CronSchedule::Every {
                    every_ms: 60_000,
                    anchor_ms: None,
                },
                payload: CronPayload::AgentTurn {
                    message: "hi".into(),
                    model: None,
                    timeout_secs: None,
                    deliver: false,
                    channel: None,
                    to: None,
                },
                session_target: SessionTarget::Isolated,
                delete_after_run: false,
                enabled: true,
                system: false,
                sandbox: CronSandboxConfig::default(),
            })
            .await
            .unwrap();

        let updated = svc
            .update(&job.id, CronJobPatch {
                name: Some("renamed".into()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(updated.name, "renamed");
    }

    #[tokio::test]
    async fn test_remove_job() {
        let store = Arc::new(InMemoryStore::new());
        let svc = make_svc(store, noop_system_event(), noop_agent_turn());

        let job = svc
            .add(CronJobCreate {
                id: None,
                name: "del".into(),
                schedule: CronSchedule::Every {
                    every_ms: 60_000,
                    anchor_ms: None,
                },
                payload: CronPayload::AgentTurn {
                    message: "hi".into(),
                    model: None,
                    timeout_secs: None,
                    deliver: false,
                    channel: None,
                    to: None,
                },
                session_target: SessionTarget::Isolated,
                delete_after_run: false,
                enabled: true,
                system: false,
                sandbox: CronSandboxConfig::default(),
            })
            .await
            .unwrap();

        svc.remove(&job.id).await.unwrap();
        assert!(svc.list().await.is_empty());
    }

    #[tokio::test]
    async fn test_status() {
        let store = Arc::new(InMemoryStore::new());
        let svc = make_svc(store, noop_system_event(), noop_agent_turn());

        let status = svc.status().await;
        assert!(!status.running);
        assert_eq!(status.job_count, 0);
    }

    #[tokio::test]
    async fn test_force_run() {
        let counter = Arc::new(AtomicUsize::new(0));
        let store = Arc::new(InMemoryStore::new());
        let svc = make_svc(
            store,
            noop_system_event(),
            counting_agent_turn(counter.clone()),
        );

        let job = svc
            .add(CronJobCreate {
                id: None,
                name: "force".into(),
                schedule: CronSchedule::Every {
                    every_ms: 999_999_999,
                    anchor_ms: None,
                },
                payload: CronPayload::AgentTurn {
                    message: "go".into(),
                    model: None,
                    timeout_secs: None,
                    deliver: false,
                    channel: None,
                    to: None,
                },
                session_target: SessionTarget::Isolated,
                delete_after_run: false,
                enabled: true,
                system: false,
                sandbox: CronSandboxConfig::default(),
            })
            .await
            .unwrap();

        svc.run(&job.id, false).await.unwrap();
        // Give the spawned task a moment.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_run_disabled_fails_without_force() {
        let store = Arc::new(InMemoryStore::new());
        let svc = make_svc(store, noop_system_event(), noop_agent_turn());

        let job = svc
            .add(CronJobCreate {
                id: None,
                name: "disabled".into(),
                schedule: CronSchedule::Every {
                    every_ms: 60_000,
                    anchor_ms: None,
                },
                payload: CronPayload::AgentTurn {
                    message: "hi".into(),
                    model: None,
                    timeout_secs: None,
                    deliver: false,
                    channel: None,
                    to: None,
                },
                session_target: SessionTarget::Isolated,
                delete_after_run: false,
                enabled: false,
                system: false,
                sandbox: CronSandboxConfig::default(),
            })
            .await
            .unwrap();

        assert!(svc.run(&job.id, false).await.is_err());
        assert!(svc.run(&job.id, true).await.is_ok());
    }

    #[tokio::test]
    async fn test_system_event_execution() {
        let counter = Arc::new(AtomicUsize::new(0));
        let store = Arc::new(InMemoryStore::new());
        let svc = make_svc(
            store,
            counting_system_event(counter.clone()),
            noop_agent_turn(),
        );

        let job = svc
            .add(CronJobCreate {
                id: None,
                name: "sys".into(),
                schedule: CronSchedule::Every {
                    every_ms: 60_000,
                    anchor_ms: None,
                },
                payload: CronPayload::SystemEvent {
                    text: "ping".into(),
                },
                session_target: SessionTarget::Main,
                delete_after_run: false,
                enabled: true,
                system: false,
                sandbox: CronSandboxConfig::default(),
            })
            .await
            .unwrap();

        svc.run(&job.id, true).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_start_stop() {
        let store = Arc::new(InMemoryStore::new());
        let svc = make_svc(store, noop_system_event(), noop_agent_turn());

        svc.start().await.unwrap();
        let status = svc.status().await;
        assert!(status.running);

        svc.stop().await;
        let status = svc.status().await;
        assert!(!status.running);
    }

    #[tokio::test]
    async fn test_one_shot_disabled_after_run() {
        let store = Arc::new(InMemoryStore::new());
        let svc = make_svc(store, noop_system_event(), noop_agent_turn());

        // Use a past at_ms so compute_next_run returns None after execution.
        let job = svc
            .add(CronJobCreate {
                id: None,
                name: "oneshot".into(),
                schedule: CronSchedule::At { at_ms: 1000 }, // far past
                payload: CronPayload::AgentTurn {
                    message: "once".into(),
                    model: None,
                    timeout_secs: None,
                    deliver: false,
                    channel: None,
                    to: None,
                },
                session_target: SessionTarget::Isolated,
                delete_after_run: false,
                enabled: true,
                system: false,
                sandbox: CronSandboxConfig::default(),
            })
            .await
            .unwrap();

        // next_run_at_ms is None because at_ms is in the past, but job is still enabled.
        svc.run(&job.id, true).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        let jobs = svc.list().await;
        let j = jobs.iter().find(|j| j.id == job.id).unwrap();
        assert!(!j.enabled, "one-shot job should be disabled after run");
    }

    #[tokio::test]
    async fn test_rate_limiting() {
        let store = Arc::new(InMemoryStore::new());
        // Create service with strict rate limit: 3 jobs per 60 seconds.
        let svc = CronService::with_config(
            store,
            noop_system_event(),
            noop_agent_turn(),
            None,
            RateLimitConfig {
                max_per_window: 3,
                window_ms: 60_000,
            },
        );

        let create_job = || CronJobCreate {
            id: None,
            name: "test".into(),
            schedule: CronSchedule::Every {
                every_ms: 60_000,
                anchor_ms: None,
            },
            payload: CronPayload::AgentTurn {
                message: "hi".into(),
                model: None,
                timeout_secs: None,
                deliver: false,
                channel: None,
                to: None,
            },
            session_target: SessionTarget::Isolated,
            delete_after_run: false,
            enabled: true,
            system: false,
            sandbox: CronSandboxConfig::default(),
        };

        // First 3 jobs should succeed.
        svc.add(create_job()).await.unwrap();
        svc.add(create_job()).await.unwrap();
        svc.add(create_job()).await.unwrap();

        // 4th job should fail due to rate limit.
        let result = svc.add(create_job()).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("rate limit exceeded")
        );
    }

    #[tokio::test]
    async fn test_rate_limiting_skips_system_jobs() {
        let store = Arc::new(InMemoryStore::new());
        // Create service with strict rate limit: 1 job per 60 seconds.
        let svc = CronService::with_config(
            store,
            noop_system_event(),
            noop_agent_turn(),
            None,
            RateLimitConfig {
                max_per_window: 1,
                window_ms: 60_000,
            },
        );

        let create_system_job = || CronJobCreate {
            id: None,
            name: "system-job".into(),
            schedule: CronSchedule::Every {
                every_ms: 60_000,
                anchor_ms: None,
            },
            payload: CronPayload::SystemEvent {
                text: "heartbeat".into(),
            },
            session_target: SessionTarget::Main,
            delete_after_run: false,
            enabled: true,
            system: true, // This is a system job
            sandbox: CronSandboxConfig::default(),
        };

        // System jobs should bypass rate limiting.
        svc.add(create_system_job()).await.unwrap();
        svc.add(create_system_job()).await.unwrap();
        svc.add(create_system_job()).await.unwrap();

        // All should succeed.
        assert_eq!(svc.list().await.len(), 3);
    }

    #[tokio::test]
    async fn test_start_executes_due_jobs_and_records_runs() {
        let counter = Arc::new(AtomicUsize::new(0));
        let store = Arc::new(InMemoryStore::new());
        let svc = make_svc(
            store,
            noop_system_event(),
            counting_agent_turn(Arc::clone(&counter)),
        );

        let job = svc
            .add(CronJobCreate {
                id: None,
                name: "live-timer".into(),
                schedule: CronSchedule::Every {
                    every_ms: 25,
                    anchor_ms: None,
                },
                payload: CronPayload::AgentTurn {
                    message: "tick".into(),
                    model: None,
                    timeout_secs: None,
                    deliver: false,
                    channel: None,
                    to: None,
                },
                session_target: SessionTarget::Isolated,
                delete_after_run: false,
                enabled: true,
                system: false,
                sandbox: CronSandboxConfig::default(),
            })
            .await
            .unwrap();

        svc.start().await.unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            while counter.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("cron scheduler did not execute any due jobs in time");

        let runs = svc.runs(&job.id, 10).await.unwrap();
        assert!(
            !runs.is_empty(),
            "expected at least one persisted run record"
        );

        svc.stop().await;
    }

    #[tokio::test]
    async fn test_clear_stuck_jobs_handles_future_running_at_without_overflow() {
        let store = Arc::new(InMemoryStore::new());
        let svc = make_svc(store, noop_system_event(), noop_agent_turn());

        let job = svc
            .add(CronJobCreate {
                id: None,
                name: "future-running-at".into(),
                schedule: CronSchedule::Every {
                    every_ms: 60_000,
                    anchor_ms: None,
                },
                payload: CronPayload::AgentTurn {
                    message: "hi".into(),
                    model: None,
                    timeout_secs: None,
                    deliver: false,
                    channel: None,
                    to: None,
                },
                session_target: SessionTarget::Isolated,
                delete_after_run: false,
                enabled: true,
                system: false,
                sandbox: CronSandboxConfig::default(),
            })
            .await
            .unwrap();

        let now = now_ms();
        svc.update_job_state(&job.id, |state| {
            state.running_at_ms = Some(now + 1_000);
        })
        .await;

        svc.clear_stuck_jobs(now).await;

        let jobs = svc.list().await;
        let job_state = jobs
            .iter()
            .find(|j| j.id == job.id)
            .expect("job should exist");
        assert_eq!(job_state.state.running_at_ms, Some(now + 1_000));
        assert!(job_state.state.last_error.is_none());
    }
}
