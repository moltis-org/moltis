//! Bounded, account-fair work queue for inbound channel callbacks.
//!
//! Channel platforms expect a callback to be acknowledged within a few seconds,
//! far less than an agent turn takes, so callbacks are admitted synchronously
//! and processed afterwards. That makes admission the interesting part, and it
//! has three requirements that a plain `mpsc` channel does not meet:
//!
//! - **Per-account ordering.** An account is never processed on more than one
//!   worker at a time, so its callbacks are handled in the order they arrived.
//! - **Fairness across accounts.** Ready accounts are served round-robin and
//!   each is capped, so one busy workspace cannot head-of-line block the others
//!   or consume the whole queue.
//! - **Bounded memory with an explicit rejection.** A full queue reports
//!   [`Admission::Rejected`] so the caller can ask the platform to redeliver,
//!   rather than dropping the callback silently.
//!
//! [`FairQueue::admit`] is deliberately non-blocking: it is called from request
//! handlers and socket callbacks that must not await. It never blocks on a lock
//! and never waits for capacity — it rejects instead.

use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    sync::{Arc, Mutex, TryLockError},
};

use {
    tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc},
    tokio_util::sync::CancellationToken,
    tracing::{error, warn},
};

/// A unit of work that belongs to one channel account.
pub trait FairQueueJob: Send + 'static {
    /// Account this job belongs to. Jobs sharing an account are processed in
    /// admission order, one at a time.
    fn account_id(&self) -> &str;
}

/// Outcome of offering a job to the queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Admission {
    /// Accepted; a worker will process it.
    Admitted,
    /// Refused because the queue (or this account's share of it) is full, or
    /// because the queue is shutting down. The caller should ask the platform to
    /// redeliver rather than treat the callback as handled.
    Rejected,
}

/// Sizing for a [`FairQueue`].
#[derive(Clone, Copy, Debug)]
pub struct FairQueueConfig {
    /// Total jobs that may be queued or in flight across all accounts.
    pub capacity: usize,
    /// Concurrent workers, and so the maximum number of accounts processed at
    /// once.
    pub workers: usize,
    /// Cap on one account's queued-plus-active jobs. Keeps a single busy account
    /// from consuming the whole queue.
    pub per_account_capacity: usize,
}

impl FairQueueConfig {
    /// Config for `capacity` total jobs across `workers` workers, giving each
    /// account an equal share of the queue.
    #[must_use]
    pub fn new(capacity: usize, workers: usize) -> Self {
        Self {
            capacity,
            workers: workers.max(1),
            per_account_capacity: (capacity / workers.max(1)).max(1),
        }
    }
}

/// A queued job together with the capacity permit it holds.
///
/// The permit lives as long as the job does — through queueing *and*
/// processing — and is released when the worker drops the slot, so
/// [`FairQueueConfig::capacity`] bounds total outstanding work rather than just
/// queue depth.
struct Slot<J> {
    job: J,
    _permit: OwnedSemaphorePermit,
}

impl<J: FairQueueJob> FairQueueJob for Slot<J> {
    fn account_id(&self) -> &str {
        self.job.account_id()
    }
}

struct AccountQueue<J> {
    jobs: VecDeque<J>,
    active: bool,
}

impl<J> Default for AccountQueue<J> {
    fn default() -> Self {
        Self {
            jobs: VecDeque::new(),
            active: false,
        }
    }
}

struct Queues<J> {
    accounts: HashMap<String, AccountQueue<J>>,
    /// Accounts with work and no worker currently on them, in service order.
    ready: VecDeque<String>,
    per_account_capacity: usize,
}

impl<J: FairQueueJob> Queues<J> {
    fn new(per_account_capacity: usize) -> Self {
        Self {
            accounts: HashMap::new(),
            ready: VecDeque::new(),
            per_account_capacity,
        }
    }

    fn has_capacity(&self, account_id: &str) -> bool {
        self.accounts.get(account_id).is_none_or(|account| {
            account.jobs.len() + usize::from(account.active) < self.per_account_capacity
        })
    }

    fn try_enqueue(&mut self, job: J) -> bool {
        let account_id = job.account_id().to_string();
        if !self.has_capacity(&account_id) {
            return false;
        }
        let account = self.accounts.entry(account_id.clone()).or_default();
        // Only becomes newly servable if nothing is queued and no worker holds
        // it; otherwise it is already in `ready` or will be re-added on
        // completion.
        let became_ready = !account.active && account.jobs.is_empty();
        account.jobs.push_back(job);
        if became_ready {
            self.ready.push_back(account_id);
        }
        true
    }

    /// Undo the most recent `try_enqueue` for an account, for when a later step
    /// of admission fails and the job must not stay queued.
    fn rollback_last(&mut self, account_id: &str) {
        let remove_account = if let Some(account) = self.accounts.get_mut(account_id) {
            let _ = account.jobs.pop_back();
            !account.active && account.jobs.is_empty()
        } else {
            false
        };
        if remove_account {
            self.ready.retain(|ready| ready != account_id);
            self.accounts.remove(account_id);
        }
    }

    /// Next job in round-robin account order, marking its account active so no
    /// second worker can pick the same account up.
    fn take_next(&mut self) -> Option<J> {
        while let Some(account_id) = self.ready.pop_front() {
            let Some(account) = self.accounts.get_mut(&account_id) else {
                continue;
            };
            let Some(job) = account.jobs.pop_front() else {
                continue;
            };
            account.active = true;
            return Some(job);
        }
        None
    }

    /// Release an account, re-queueing it at the back if it still has work.
    fn complete(&mut self, account_id: &str) {
        let remove_account = if let Some(account) = self.accounts.get_mut(account_id) {
            account.active = false;
            if account.jobs.is_empty() {
                true
            } else {
                self.ready.push_back(account_id.to_string());
                false
            }
        } else {
            false
        };
        if remove_account {
            self.accounts.remove(account_id);
        }
    }

    fn is_empty(&self) -> bool {
        self.accounts.is_empty()
    }
}

/// Handle to a running fair queue.
///
/// The workers stop when this handle is dropped or the shutdown token fires,
/// after draining whatever was already admitted. Wrap it in an `Arc` to share.
pub struct FairQueue<J> {
    queues: Arc<Mutex<Queues<Slot<J>>>>,
    wake: mpsc::Sender<()>,
    permits: Arc<Semaphore>,
    capacity: usize,
    cancel: CancellationToken,
}

impl<J: FairQueueJob> FairQueue<J> {
    /// Start the worker pool.
    ///
    /// `process` runs one job to completion. It may panic without taking the
    /// pool down. Cancelling `shutdown` drains the jobs already admitted — they
    /// were acknowledged to the platform and will not be redelivered — and then
    /// stops the workers.
    ///
    /// The queue watches a *child* of `shutdown`, so dropping the queue stops
    /// only its own workers and leaves the caller's token — which usually also
    /// governs the connection that feeds it — untouched.
    pub fn start<F, Fut>(config: FairQueueConfig, shutdown: &CancellationToken, process: F) -> Self
    where
        F: Fn(J) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let cancel = shutdown.child_token();
        let queues = Arc::new(Mutex::new(Queues::new(config.per_account_capacity)));
        let (wake, wake_receiver) = mpsc::channel(1);
        tokio::spawn(run_supervisor(
            Arc::clone(&queues),
            wake_receiver,
            cancel.clone(),
            config.workers,
            process,
        ));
        Self {
            queues,
            wake,
            permits: Arc::new(Semaphore::new(config.capacity)),
            capacity: config.capacity,
            cancel,
        }
    }

    /// Offer a job without blocking or awaiting.
    ///
    /// Rejects rather than waiting when the queue is shutting down, this
    /// account's share is full, global capacity is exhausted, or the internal
    /// lock is momentarily contended.
    pub fn admit(&self, job: J) -> Admission {
        if self.cancel.is_cancelled() {
            return Admission::Rejected;
        }
        // `try_lock`, not `lock`: admission runs on request-handling tasks that
        // must not block. Contention is brief, and the platform will redeliver.
        let mut queues = match self.queues.try_lock() {
            Ok(queues) => queues,
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
            Err(TryLockError::WouldBlock) => return Admission::Rejected,
        };
        let account_id = job.account_id().to_string();
        if !queues.has_capacity(&account_id) {
            return Admission::Rejected;
        }
        // Taken before enqueueing and carried by the slot until the job has been
        // processed, so capacity covers queued *and* in-flight work. Dropping
        // the slot returns it.
        let Ok(permit) = Arc::clone(&self.permits).try_acquire_owned() else {
            return Admission::Rejected;
        };
        if !queues.try_enqueue(Slot {
            job,
            _permit: permit,
        }) {
            return Admission::Rejected;
        }

        match self.wake.try_send(()) {
            // A pending wake is as good as a new one: the supervisor re-checks
            // the queue after every job.
            Ok(()) | Err(mpsc::error::TrySendError::Full(())) => Admission::Admitted,
            Err(mpsc::error::TrySendError::Closed(())) => {
                // The supervisor is gone and would never pick this up. Undoing
                // the enqueue drops the slot, which returns its permit.
                queues.rollback_last(&account_id);
                Admission::Rejected
            },
        }
    }

    /// Jobs currently queued or being processed. Test and diagnostic use.
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.capacity
            .saturating_sub(self.permits.available_permits())
    }
}

impl<J> Drop for FairQueue<J> {
    fn drop(&mut self) {
        // Stop this queue's workers when the handle goes away, after they finish
        // the jobs already admitted. Cancels only the child token, so the
        // caller's shutdown token is unaffected.
        self.cancel.cancel();
    }
}

async fn run_supervisor<J, F, Fut>(
    queues: Arc<Mutex<Queues<Slot<J>>>>,
    mut wake_receiver: mpsc::Receiver<()>,
    cancel: CancellationToken,
    workers: usize,
    process: F,
) where
    J: FairQueueJob,
    F: Fn(J) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut worker_senders = Vec::with_capacity(workers);
    let mut idle = (0..workers).collect::<VecDeque<_>>();
    let (completion_sender, mut completions) = mpsc::channel(workers);
    let mut pool = tokio::task::JoinSet::new();
    for index in 0..workers {
        let (sender, receiver) = mpsc::channel(1);
        worker_senders.push(sender);
        pool.spawn(run_worker(
            index,
            receiver,
            completion_sender.clone(),
            process.clone(),
        ));
    }
    drop(completion_sender);

    let mut accepting = true;
    loop {
        // Hand work to every idle worker we can fill.
        while let Some(index) = idle.pop_front() {
            let job = lock(&queues).take_next();
            let Some(job) = job else {
                idle.push_front(index);
                break;
            };
            if worker_senders[index].try_send(job).is_err() {
                error!(
                    worker = index,
                    "channel callback worker stopped accepting work"
                );
                return;
            }
        }

        // Nothing left to admit and nothing left to drain.
        if !accepting && lock(&queues).is_empty() {
            break;
        }

        tokio::select! {
            () = cancel.cancelled(), if accepting => {
                // Stop taking new work but finish what was already admitted.
                accepting = false;
                wake_receiver.close();
            },
            wake = wake_receiver.recv(), if accepting => {
                if wake.is_none() {
                    accepting = false;
                }
            },
            completion = completions.recv() => {
                let Some((index, account_id)) = completion else {
                    error!("all channel callback workers stopped unexpectedly");
                    return;
                };
                lock(&queues).complete(&account_id);
                idle.push_back(index);
            },
            result = pool.join_next(), if !pool.is_empty() => {
                match result {
                    Some(Ok(())) => error!("channel callback worker stopped unexpectedly"),
                    Some(Err(error)) => error!("channel callback worker failed: {error}"),
                    None => {},
                }
                return;
            },
        }
    }

    drop(worker_senders);
    while let Some(result) = pool.join_next().await {
        if let Some(error) = result.err() {
            error!("channel callback worker failed: {error}");
        }
    }
}

fn lock<J>(queues: &Arc<Mutex<Queues<J>>>) -> std::sync::MutexGuard<'_, Queues<J>> {
    queues.lock().unwrap_or_else(|error| error.into_inner())
}

async fn run_worker<J, F, Fut>(
    index: usize,
    mut receiver: mpsc::Receiver<Slot<J>>,
    completions: mpsc::Sender<(usize, String)>,
    process: F,
) where
    J: FairQueueJob,
    F: Fn(J) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    while let Some(slot) = receiver.recv().await {
        let account_id = slot.account_id().to_string();
        {
            // Scoped so the permit is released as soon as the job is done,
            // before completion is reported.
            let Slot { job, _permit } = slot;
            // Run on a dedicated task so a panic inside one callback cannot take
            // the worker — and with it every later callback — down. Awaited
            // immediately, so the account's ordering is preserved.
            if let Err(error) = tokio::spawn(process(job)).await {
                warn!(worker = index, %account_id, "channel callback panicked: {error}");
            }
        }
        // Report completion even after a panic, or the account would stay marked
        // active and its remaining callbacks would never be served.
        if completions.send((index, account_id)).await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    struct TestJob {
        account_id: String,
        label: String,
    }

    impl FairQueueJob for TestJob {
        fn account_id(&self) -> &str {
            &self.account_id
        }
    }

    fn job(account_id: &str, label: &str) -> TestJob {
        TestJob {
            account_id: account_id.to_string(),
            label: label.to_string(),
        }
    }

    #[test]
    fn per_account_capacity_divides_the_queue() {
        let config = FairQueueConfig::new(256, 16);
        assert_eq!(config.per_account_capacity, 16);
        // Never zero, however small the queue relative to the pool.
        assert_eq!(FairQueueConfig::new(4, 16).per_account_capacity, 1);
        assert_eq!(FairQueueConfig::new(8, 0).workers, 1);
    }

    #[tokio::test]
    async fn one_accounts_jobs_run_in_admission_order() {
        let order = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        // Several workers, but a per-account share wide enough that the cap is
        // not what this test is exercising.
        let config = FairQueueConfig {
            capacity: 16,
            workers: 4,
            per_account_capacity: 16,
        };
        let queue = {
            let order = Arc::clone(&order);
            FairQueue::start(config, &CancellationToken::new(), move |job: TestJob| {
                let order = Arc::clone(&order);
                async move {
                    // Yield so a scheduler that ran an account concurrently
                    // would interleave here.
                    tokio::task::yield_now().await;
                    order.lock().await.push(job.label);
                }
            })
        };

        for index in 0..6 {
            assert_eq!(
                queue.admit(job("acct", &format!("job-{index}"))),
                Admission::Admitted
            );
        }
        while queue.in_flight() > 0 {
            tokio::task::yield_now().await;
        }

        assert_eq!(
            *order.lock().await,
            (0..6).map(|i| format!("job-{i}")).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn a_blocked_account_does_not_stall_other_accounts() {
        let release = Arc::new(tokio::sync::Notify::new());
        let started = Arc::new(tokio::sync::Notify::new());
        let other_ran = Arc::new(tokio::sync::Notify::new());
        let queue = {
            let (release, started, other_ran) = (
                Arc::clone(&release),
                Arc::clone(&started),
                Arc::clone(&other_ran),
            );
            FairQueue::start(
                FairQueueConfig::new(16, 4),
                &CancellationToken::new(),
                move |job: TestJob| {
                    let (release, started, other_ran) = (
                        Arc::clone(&release),
                        Arc::clone(&started),
                        Arc::clone(&other_ran),
                    );
                    async move {
                        if job.account_id == "busy" {
                            started.notify_one();
                            release.notified().await;
                        } else {
                            other_ran.notify_one();
                        }
                    }
                },
            )
        };

        assert_eq!(queue.admit(job("busy", "blocking")), Admission::Admitted);
        started.notified().await;
        assert_eq!(
            queue.admit(job("quiet", "independent")),
            Admission::Admitted
        );

        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), other_ran.notified())
                .await
                .is_ok(),
            "a busy account head-of-line blocked an unrelated one"
        );
        release.notify_one();
    }

    #[tokio::test]
    async fn one_account_cannot_consume_the_whole_queue() {
        let release = Arc::new(tokio::sync::Notify::new());
        let queue = {
            let release = Arc::clone(&release);
            FairQueue::start(
                FairQueueConfig::new(8, 4),
                &CancellationToken::new(),
                move |_job: TestJob| {
                    let release = Arc::clone(&release);
                    async move { release.notified().await }
                },
            )
        };

        // per_account_capacity == 2, so the third is refused while global
        // capacity (8) is still available to other accounts.
        assert_eq!(queue.admit(job("hot", "a")), Admission::Admitted);
        assert_eq!(queue.admit(job("hot", "b")), Admission::Admitted);
        assert_eq!(queue.admit(job("hot", "c")), Admission::Rejected);
        assert_eq!(queue.admit(job("cold", "a")), Admission::Admitted);

        release.notify_waiters();
    }

    #[tokio::test]
    async fn a_panicking_job_does_not_stop_the_account_or_the_pool() {
        let ran = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let queue = {
            let ran = Arc::clone(&ran);
            FairQueue::start(
                FairQueueConfig::new(16, 2),
                &CancellationToken::new(),
                move |job: TestJob| {
                    let ran = Arc::clone(&ran);
                    async move {
                        ran.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        assert_ne!(job.label, "boom", "intentional test panic");
                    }
                },
            )
        };

        assert_eq!(queue.admit(job("acct", "boom")), Admission::Admitted);
        assert_eq!(queue.admit(job("acct", "after")), Admission::Admitted);
        while queue.in_flight() > 0 {
            tokio::task::yield_now().await;
        }

        assert_eq!(
            ran.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "the job after the panicking one never ran"
        );
    }

    #[tokio::test]
    async fn a_completed_job_returns_its_capacity() {
        let queue = FairQueue::start(
            FairQueueConfig::new(2, 1),
            &CancellationToken::new(),
            move |_job: TestJob| async move {},
        );

        for index in 0..6 {
            // Capacity is 2, so this only keeps succeeding if permits come back.
            let label = format!("job-{index}");
            while queue.admit(job("acct", &label)) == Admission::Rejected {
                tokio::task::yield_now().await;
            }
        }
        while queue.in_flight() > 0 {
            tokio::task::yield_now().await;
        }
        assert_eq!(queue.in_flight(), 0);
    }

    #[tokio::test]
    async fn cancellation_drains_already_admitted_jobs() {
        // Those callbacks were acknowledged to the platform and will not be
        // redelivered, so shutdown must finish them rather than drop them.
        let ran = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let cancel = CancellationToken::new();
        let queue = {
            let ran = Arc::clone(&ran);
            FairQueue::start(
                FairQueueConfig::new(16, 1),
                &cancel,
                move |_job: TestJob| {
                    let ran = Arc::clone(&ran);
                    async move {
                        tokio::task::yield_now().await;
                        ran.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                },
            )
        };

        for index in 0..4 {
            assert_eq!(
                queue.admit(job("acct", &format!("job-{index}"))),
                Admission::Admitted
            );
        }
        cancel.cancel();

        assert_eq!(queue.admit(job("acct", "too-late")), Admission::Rejected);
        while queue.in_flight() > 0 {
            tokio::task::yield_now().await;
        }
        assert_eq!(ran.load(std::sync::atomic::Ordering::SeqCst), 4);
    }
}
