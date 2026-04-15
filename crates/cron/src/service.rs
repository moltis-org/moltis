//! Core cron scheduler: timer loop, job execution, CRUD operations.

use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use {
    tokio::{
        sync::{Mutex, Notify, RwLock},
        task::JoinHandle,
    },
    tracing::{debug, error, info, warn},
};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, cron as cron_metrics, gauge, histogram};

use crate::{
    Error, Result, schedule::compute_next_run, store::CronStore, system_events::SystemEventsQueue,
    types::*,
};

/// Result of an agent turn, including optional token usage.
#[derive(Debug, Clone)]
pub struct AgentTurnResult {
    pub output: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// The session key used for this turn (links to the session store).
    pub session_key: Option<String>,
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
pub type NotifyFn = Arc<dyn Fn(CronNotification) + Send + Sync>;

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
            return Err(Error::message(format!(
                "rate limit exceeded: max {} jobs per {} seconds",
                self.config.max_per_window,
                self.config.window_ms / 1000
            )));
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
    events_queue: Arc<SystemEventsQueue>,
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
        Self::with_events_queue(
            store,
            on_system_event,
            on_agent_turn,
            on_notify,
            rate_limit_config,
            SystemEventsQueue::new(),
        )
    }

    /// Create a new cron service with a pre-created events queue.
    ///
    /// Use this when the queue must be shared with closures created before
    /// the service (e.g. the `on_agent_turn` callback).
    pub fn with_events_queue(
        store: Arc<dyn CronStore>,
        on_system_event: SystemEventFn,
        on_agent_turn: AgentTurnFn,
        on_notify: Option<NotifyFn>,
        rate_limit_config: RateLimitConfig,
        events_queue: Arc<SystemEventsQueue>,
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
            events_queue,
        })
    }

    /// Access the shared events queue for enqueueing system events.
    pub fn events_queue(&self) -> &Arc<SystemEventsQueue> {
        &self.events_queue
    }

    /// Wake the heartbeat by setting its next run to now.
    ///
    /// Multiple wake calls coalesce naturally: they all set `next_run_at_ms = now`
    /// idempotently, and `running_at_ms` prevents the heartbeat from firing twice.
    pub async fn wake(&self, reason: &str) {
        let now = now_ms();
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == "__heartbeat__")
            && job.enabled
            && job.state.running_at_ms.is_none()
        {
            debug!(reason, "waking heartbeat");
            job.state.next_run_at_ms = Some(now);
        }
        drop(jobs);
        self.wake_notify.notify_one();
    }

    /// Emit a notification if a callback is registered.
    fn notify(&self, notification: CronNotification) {
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
            wake_mode: create.wake_mode,
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
        self.notify(CronNotification::Created { job: job.clone() });
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
            .ok_or_else(|| Error::job_not_found(id))?;

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
        if let Some(wake_mode) = patch.wake_mode {
            job.wake_mode = wake_mode;
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
        self.notify(CronNotification::Updated {
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
        self.notify(CronNotification::Removed {
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
                .ok_or_else(|| Error::job_not_found(id))?
        };

        if !job.enabled && !force {
            return Err(Error::message(
                "job is disabled (use force=true to override)",
            ));
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
                    session_key: None,
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
        let (status, error_msg, output, input_tokens, output_tokens, session_key) = match &result {
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
                    r.session_key.clone(),
                )
            },
            Err(e) => {
                error!(id = %job.id, error = %e, "cron job failed");
                #[cfg(feature = "metrics")]
                counter!(cron_metrics::ERRORS_TOTAL).increment(1);
                (
                    RunStatus::Error,
                    Some(e.to_string()),
                    None,
                    None,
                    None,
                    None,
                )
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
            session_key,
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

        // Wake heartbeat immediately if this job requested it.
        if job.wake_mode == CronWakeMode::Now && job.id != "__heartbeat__" {
            self.wake("cron-event").await;
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
            return Err(Error::message(
                "sessionTarget=main requires payload kind=systemEvent",
            ));
        },
        (SessionTarget::Isolated | SessionTarget::Named(_), CronPayload::SystemEvent { .. }) => {
            return Err(Error::message(
                "sessionTarget=isolated/named requires payload kind=agentTurn",
            ));
        },
        _ => {},
    }
    if let CronPayload::AgentTurn {
        deliver: true,
        channel,
        to,
        ..
    } = &job.payload
    {
        match (channel.as_deref(), to.as_deref()) {
            (None | Some(""), _) => {
                return Err(Error::message(
                    "deliver=true requires a non-empty 'channel' (account_id)",
                ));
            },
            (_, None | Some("")) => {
                return Err(Error::message(
                    "deliver=true requires a non-empty 'to' (chat_id)",
                ));
            },
            _ => {},
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
