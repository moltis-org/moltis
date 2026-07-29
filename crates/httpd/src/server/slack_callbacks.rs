use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex, RwLock, TryLockError},
};

use {
    bytes::Bytes,
    futures::FutureExt,
    moltis_gateway::channel_webhook_dedup::{ChannelWebhookAdmission, ChannelWebhookDedupeStore},
    tokio::{
        sync::{OwnedSemaphorePermit, Semaphore, mpsc},
        task::JoinSet,
    },
};

const CALLBACK_QUEUE_CAPACITY: usize = 256;
const CALLBACK_WORKER_LIMIT: usize = 16;
const CALLBACK_ACCOUNT_CAPACITY: usize = CALLBACK_QUEUE_CAPACITY / CALLBACK_WORKER_LIMIT;

#[derive(Clone, Copy)]
pub(super) enum SlackCallbackKind {
    Event,
    Interaction,
    Command,
}

impl SlackCallbackKind {
    fn endpoint(self) -> &'static str {
        match self {
            Self::Event => "events",
            Self::Interaction => "interactions",
            Self::Command => "commands",
        }
    }
}

struct SlackCallbackJob {
    account_id: String,
    body: Bytes,
    kind: SlackCallbackKind,
    _permit: OwnedSemaphorePermit,
}

#[derive(Default)]
struct AccountQueue {
    jobs: VecDeque<SlackCallbackJob>,
    active: bool,
}

#[derive(Default)]
struct CallbackQueues {
    accounts: HashMap<String, AccountQueue>,
    ready_accounts: VecDeque<String>,
}

impl CallbackQueues {
    fn account_has_capacity(&self, account_id: &str) -> bool {
        self.accounts.get(account_id).is_none_or(|account| {
            account.jobs.len() + usize::from(account.active) < CALLBACK_ACCOUNT_CAPACITY
        })
    }

    fn try_enqueue(&mut self, job: SlackCallbackJob) -> bool {
        let account_id = job.account_id.clone();
        if !self.account_has_capacity(&account_id) {
            return false;
        }

        let account = self.accounts.entry(account_id.clone()).or_default();
        let became_ready = !account.active && account.jobs.is_empty();
        account.jobs.push_back(job);
        if became_ready {
            self.ready_accounts.push_back(account_id);
        }
        true
    }

    fn rollback_last(&mut self, account_id: &str) {
        let remove_account = if let Some(account) = self.accounts.get_mut(account_id) {
            let _ = account.jobs.pop_back();
            !account.active && account.jobs.is_empty()
        } else {
            false
        };
        if remove_account {
            self.ready_accounts.retain(|ready| ready != account_id);
            self.accounts.remove(account_id);
        }
    }

    fn take_next(&mut self) -> Option<SlackCallbackJob> {
        while let Some(account_id) = self.ready_accounts.pop_front() {
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

    fn complete(&mut self, account_id: &str) {
        let remove_account = if let Some(account) = self.accounts.get_mut(account_id) {
            account.active = false;
            if account.jobs.is_empty() {
                true
            } else {
                self.ready_accounts.push_back(account_id.to_string());
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

/// Bounded callback queue with an owned supervisor for all processing tasks.
pub(super) struct SlackCallbackDispatcher {
    queues: Arc<Mutex<CallbackQueues>>,
    wake_sender: mpsc::Sender<()>,
    permits: Arc<Semaphore>,
    supervisor: tokio::task::JoinHandle<()>,
}

impl SlackCallbackDispatcher {
    pub(super) fn start(plugin: Arc<tokio::sync::RwLock<moltis_slack::SlackPlugin>>) -> Arc<Self> {
        let queues = Arc::new(Mutex::new(CallbackQueues::default()));
        let (wake_sender, wake_receiver) = mpsc::channel(1);
        let permits = Arc::new(Semaphore::new(CALLBACK_QUEUE_CAPACITY));
        let supervisor = tokio::spawn(run_supervisor(Arc::clone(&queues), wake_receiver, plugin));
        Arc::new(Self {
            queues,
            wake_sender,
            permits,
            supervisor,
        })
    }

    pub(super) fn admit(
        &self,
        dedup_store: &RwLock<ChannelWebhookDedupeStore>,
        account_id: String,
        idempotency_key: Option<&str>,
        kind: SlackCallbackKind,
        body: Bytes,
    ) -> ChannelWebhookAdmission {
        admit_callback(
            &self.queues,
            &self.wake_sender,
            &self.permits,
            dedup_store,
            account_id,
            idempotency_key,
            kind,
            body,
        )
    }
}

impl Drop for SlackCallbackDispatcher {
    fn drop(&mut self) {
        self.supervisor.abort();
    }
}

fn admit_callback(
    queues: &Arc<Mutex<CallbackQueues>>,
    wake_sender: &mpsc::Sender<()>,
    permits: &Arc<Semaphore>,
    dedup_store: &RwLock<ChannelWebhookDedupeStore>,
    account_id: String,
    idempotency_key: Option<&str>,
    kind: SlackCallbackKind,
    body: Bytes,
) -> ChannelWebhookAdmission {
    let endpoint = kind.endpoint();
    let Some(key) = idempotency_key else {
        return if try_enqueue_callback(queues, wake_sender, permits, account_id, kind, body) {
            ChannelWebhookAdmission::Admitted
        } else {
            ChannelWebhookAdmission::Rejected
        };
    };

    let job_account_id = account_id.clone();
    dedup_store
        .write()
        .unwrap_or_else(|error| error.into_inner())
        .admit_scoped("slack", &account_id, endpoint, key, || {
            try_enqueue_callback(queues, wake_sender, permits, job_account_id, kind, body)
        })
}

fn try_enqueue_callback(
    queues: &Arc<Mutex<CallbackQueues>>,
    wake_sender: &mpsc::Sender<()>,
    permits: &Arc<Semaphore>,
    account_id: String,
    kind: SlackCallbackKind,
    body: Bytes,
) -> bool {
    let mut queues = match queues.try_lock() {
        Ok(queues) => queues,
        Err(TryLockError::Poisoned(error)) => error.into_inner(),
        Err(TryLockError::WouldBlock) => return false,
    };
    if !queues.account_has_capacity(&account_id) {
        return false;
    }
    let Ok(permit) = Arc::clone(permits).try_acquire_owned() else {
        return false;
    };
    let job = SlackCallbackJob {
        account_id: account_id.clone(),
        body,
        kind,
        _permit: permit,
    };
    if !queues.try_enqueue(job) {
        return false;
    }

    match wake_sender.try_send(()) {
        Ok(()) | Err(mpsc::error::TrySendError::Full(())) => true,
        Err(mpsc::error::TrySendError::Closed(())) => {
            queues.rollback_last(&account_id);
            false
        },
    }
}

async fn run_supervisor(
    queues: Arc<Mutex<CallbackQueues>>,
    wake_receiver: mpsc::Receiver<()>,
    plugin: Arc<tokio::sync::RwLock<moltis_slack::SlackPlugin>>,
) {
    run_fair_workers(queues, wake_receiver, move |job| {
        process_callback(Arc::clone(&plugin), job)
    })
    .await;
}

/// Dispatch one callback per ready account in round-robin order. An account is
/// never active on more than one worker, preserving its admission order.
async fn run_fair_workers<F, Fut>(
    queues: Arc<Mutex<CallbackQueues>>,
    mut wake_receiver: mpsc::Receiver<()>,
    process: F,
) where
    F: Fn(SlackCallbackJob) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut workers = JoinSet::new();
    let mut worker_senders = Vec::with_capacity(CALLBACK_WORKER_LIMIT);
    let mut idle_workers = (0..CALLBACK_WORKER_LIMIT).collect::<VecDeque<_>>();
    let (completion_sender, mut completion_receiver) = mpsc::channel(CALLBACK_WORKER_LIMIT);
    for worker_index in 0..CALLBACK_WORKER_LIMIT {
        let (sender, worker_receiver) = mpsc::channel(1);
        worker_senders.push(sender);
        workers.spawn(run_serial_worker(
            worker_index,
            worker_receiver,
            completion_sender.clone(),
            process.clone(),
        ));
    }
    drop(completion_sender);

    let mut wake_open = true;
    loop {
        while let Some(worker_index) = idle_workers.pop_front() {
            let job = {
                queues
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .take_next()
            };
            let Some(job) = job else {
                idle_workers.push_front(worker_index);
                break;
            };
            if let Err(error) = worker_senders[worker_index].try_send(job) {
                tracing::error!(worker_index, %error, "Slack callback worker stopped accepting work");
                return;
            }
        }

        let queues_empty = queues
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_empty();
        if !wake_open && queues_empty {
            break;
        }

        tokio::select! {
            wake = wake_receiver.recv(), if wake_open => {
                if wake.is_none() {
                    wake_open = false;
                }
            },
            completion = completion_receiver.recv() => {
                let Some((worker_index, account_id)) = completion else {
                    tracing::error!("all Slack callback workers stopped unexpectedly");
                    return;
                };
                queues
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .complete(&account_id);
                idle_workers.push_back(worker_index);
            },
            result = workers.join_next(), if !workers.is_empty() => {
                log_worker_result(result);
                tracing::error!("Slack callback worker stopped unexpectedly");
                return;
            },
        }
    }

    drop(worker_senders);
    while !workers.is_empty() {
        log_worker_result(workers.join_next().await);
    }
}

async fn run_serial_worker<F, Fut>(
    worker_index: usize,
    mut receiver: mpsc::Receiver<SlackCallbackJob>,
    completion_sender: mpsc::Sender<(usize, String)>,
    process: F,
) where
    F: Fn(SlackCallbackJob) -> Fut,
    Fut: Future<Output = ()>,
{
    while let Some(job) = receiver.recv().await {
        let account_id = job.account_id.clone();
        let process_result = std::panic::catch_unwind(AssertUnwindSafe(|| process(job)));
        match process_result {
            Ok(future) => {
                if AssertUnwindSafe(future).catch_unwind().await.is_err() {
                    tracing::error!(worker_index, %account_id, "Slack callback worker panicked");
                }
            },
            Err(_) => {
                tracing::error!(worker_index, %account_id, "Slack callback worker panicked");
            },
        }
        if completion_sender
            .send((worker_index, account_id))
            .await
            .is_err()
        {
            return;
        }
    }
}

fn log_worker_result(result: Option<Result<(), tokio::task::JoinError>>) {
    if let Some(Err(error)) = result {
        tracing::error!("Slack callback worker failed: {error}");
    }
}

async fn process_callback(
    plugin: Arc<tokio::sync::RwLock<moltis_slack::SlackPlugin>>,
    job: SlackCallbackJob,
) {
    let plugin = plugin.read().await;
    let result = match job.kind {
        SlackCallbackKind::Event => plugin
            .ingest_verified_webhook(&job.account_id, &job.body)
            .await
            .map(|_| ()),
        SlackCallbackKind::Interaction => {
            plugin
                .ingest_verified_interaction_webhook(&job.account_id, &job.body)
                .await
        },
        SlackCallbackKind::Command => plugin
            .ingest_verified_command_webhook(&job.account_id, &job.body)
            .await
            .map(|_| ()),
    };
    if let Err(error) = result {
        tracing::warn!(
            account_id = %job.account_id,
            endpoint = job.kind.endpoint(),
            "Slack callback processing failed after acknowledgement: {error}"
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn account_job(
        permits: &Arc<Semaphore>,
        account_id: &str,
        body: &'static [u8],
    ) -> SlackCallbackJob {
        SlackCallbackJob {
            account_id: account_id.to_string(),
            body: Bytes::from_static(body),
            kind: SlackCallbackKind::Event,
            _permit: Arc::clone(permits).try_acquire_owned().unwrap(),
        }
    }

    fn admit_without_key(
        queues: &Arc<Mutex<CallbackQueues>>,
        wake_sender: &mpsc::Sender<()>,
        permits: &Arc<Semaphore>,
        dedup: &RwLock<ChannelWebhookDedupeStore>,
        account_id: &str,
    ) -> ChannelWebhookAdmission {
        admit_callback(
            queues,
            wake_sender,
            permits,
            dedup,
            account_id.to_string(),
            None,
            SlackCallbackKind::Event,
            Bytes::from_static(b"callback"),
        )
    }

    fn prior_worker_index(account_id: &str) -> usize {
        use std::hash::{DefaultHasher, Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        account_id.hash(&mut hasher);
        hasher.finish() as usize % CALLBACK_WORKER_LIMIT
    }

    #[test]
    fn failed_queue_admission_does_not_consume_dedup() {
        let queues = Arc::new(Mutex::new(CallbackQueues::default()));
        let (closed_sender, closed_receiver) = mpsc::channel(1);
        drop(closed_receiver);
        let permits = Arc::new(Semaphore::new(2));
        let dedup = RwLock::new(ChannelWebhookDedupeStore::new());

        assert_eq!(
            admit_callback(
                &queues,
                &closed_sender,
                &permits,
                &dedup,
                "account".to_string(),
                Some("event-1"),
                SlackCallbackKind::Event,
                Bytes::from_static(b"first"),
            ),
            ChannelWebhookAdmission::Rejected
        );

        let (wake_sender, _wake_receiver) = mpsc::channel(1);
        assert_eq!(
            admit_callback(
                &queues,
                &wake_sender,
                &permits,
                &dedup,
                "account".to_string(),
                Some("event-1"),
                SlackCallbackKind::Event,
                Bytes::from_static(b"retry"),
            ),
            ChannelWebhookAdmission::Admitted
        );
        let _remaining_permit = Arc::clone(&permits).try_acquire_owned().unwrap();
        assert_eq!(
            admit_callback(
                &queues,
                &wake_sender,
                &permits,
                &dedup,
                "account".to_string(),
                Some("event-1"),
                SlackCallbackKind::Event,
                Bytes::from_static(b"duplicate"),
            ),
            ChannelWebhookAdmission::Duplicate
        );
    }

    #[test]
    fn one_account_saturation_leaves_capacity_until_global_saturation() {
        let queues = Arc::new(Mutex::new(CallbackQueues::default()));
        let (wake_sender, _wake_receiver) = mpsc::channel(1);
        let permits = Arc::new(Semaphore::new(CALLBACK_QUEUE_CAPACITY));
        let dedup = RwLock::new(ChannelWebhookDedupeStore::new());

        for _ in 0..CALLBACK_ACCOUNT_CAPACITY {
            assert_eq!(
                admit_without_key(&queues, &wake_sender, &permits, &dedup, "hot-account"),
                ChannelWebhookAdmission::Admitted
            );
        }
        assert_eq!(
            admit_without_key(&queues, &wake_sender, &permits, &dedup, "hot-account"),
            ChannelWebhookAdmission::Rejected
        );
        assert_eq!(
            permits.available_permits(),
            CALLBACK_QUEUE_CAPACITY - CALLBACK_ACCOUNT_CAPACITY
        );

        assert_eq!(
            admit_without_key(&queues, &wake_sender, &permits, &dedup, "cold-account"),
            ChannelWebhookAdmission::Admitted
        );
        for index in 0..(CALLBACK_QUEUE_CAPACITY - CALLBACK_ACCOUNT_CAPACITY - 1) {
            assert_eq!(
                admit_without_key(
                    &queues,
                    &wake_sender,
                    &permits,
                    &dedup,
                    &format!("other-account-{index}"),
                ),
                ChannelWebhookAdmission::Admitted
            );
        }
        assert_eq!(permits.available_permits(), 0);
        assert_eq!(
            admit_without_key(&queues, &wake_sender, &permits, &dedup, "overflow-account"),
            ChannelWebhookAdmission::Rejected
        );
    }

    #[tokio::test]
    async fn callbacks_for_same_account_are_processed_in_admission_order() {
        let queues = Arc::new(Mutex::new(CallbackQueues::default()));
        let (wake_sender, wake_receiver) = mpsc::channel(1);
        let permits = Arc::new(Semaphore::new(CALLBACK_QUEUE_CAPACITY));
        let first_started = Arc::new(tokio::sync::Notify::new());
        let release_first = Arc::new(tokio::sync::Notify::new());
        let second_started = Arc::new(tokio::sync::Notify::new());
        let processed = Arc::new(tokio::sync::Mutex::new(Vec::new()));

        {
            let mut queues = queues.lock().unwrap();
            assert!(queues.try_enqueue(account_job(&permits, "account", b"first")));
            assert!(queues.try_enqueue(account_job(&permits, "account", b"second")));
        }
        wake_sender.try_send(()).unwrap();

        let supervisor = tokio::spawn(run_fair_workers(Arc::clone(&queues), wake_receiver, {
            let first_started = Arc::clone(&first_started);
            let release_first = Arc::clone(&release_first);
            let second_started = Arc::clone(&second_started);
            let processed = Arc::clone(&processed);
            move |job: SlackCallbackJob| {
                let first_started = Arc::clone(&first_started);
                let release_first = Arc::clone(&release_first);
                let second_started = Arc::clone(&second_started);
                let processed = Arc::clone(&processed);
                async move {
                    if job.body == Bytes::from_static(b"first") {
                        first_started.notify_one();
                        release_first.notified().await;
                        processed.lock().await.push("first");
                    } else {
                        second_started.notify_one();
                        processed.lock().await.push("second");
                    }
                }
            }
        }));

        drop(wake_sender);

        first_started.notified().await;
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(25),
                second_started.notified()
            )
            .await
            .is_err(),
            "the second callback started before the first completed"
        );
        release_first.notify_one();
        supervisor.await.unwrap();

        assert_eq!(*processed.lock().await, ["first", "second"]);
    }

    #[tokio::test]
    async fn accounts_colliding_under_prior_sharding_run_independently() {
        let queues = Arc::new(Mutex::new(CallbackQueues::default()));
        let (wake_sender, wake_receiver) = mpsc::channel(1);
        let permits = Arc::new(Semaphore::new(CALLBACK_QUEUE_CAPACITY));
        let mut accounts_by_prior_worker = HashMap::new();
        let (hot_account, colliding_account) = (0..=CALLBACK_WORKER_LIMIT)
            .find_map(|index| {
                let account = format!("colliding-account-{index}");
                accounts_by_prior_worker
                    .insert(prior_worker_index(&account), account.clone())
                    .map(|prior| (prior, account))
            })
            .unwrap();
        let hot_started = Arc::new(tokio::sync::Notify::new());
        let release_hot = Arc::new(tokio::sync::Notify::new());
        let colliding_started = Arc::new(tokio::sync::Notify::new());

        assert!(queues.lock().unwrap().try_enqueue(account_job(
            &permits,
            &hot_account,
            b"hot-blocking"
        )));
        wake_sender.try_send(()).unwrap();

        let supervisor = tokio::spawn(run_fair_workers(Arc::clone(&queues), wake_receiver, {
            let hot_started = Arc::clone(&hot_started);
            let release_hot = Arc::clone(&release_hot);
            let colliding_started = Arc::clone(&colliding_started);
            move |job: SlackCallbackJob| {
                let hot_started = Arc::clone(&hot_started);
                let release_hot = Arc::clone(&release_hot);
                let colliding_started = Arc::clone(&colliding_started);
                async move {
                    if job.body == Bytes::from_static(b"hot-blocking") {
                        hot_started.notify_one();
                        release_hot.notified().await;
                    } else if job.body == Bytes::from_static(b"colliding") {
                        colliding_started.notify_one();
                    }
                }
            }
        }));

        hot_started.notified().await;
        assert!(queues.lock().unwrap().try_enqueue(account_job(
            &permits,
            &colliding_account,
            b"colliding"
        )));
        assert!(
            !matches!(
                wake_sender.try_send(()),
                Err(mpsc::error::TrySendError::Closed(()))
            ),
            "scheduler stopped before colliding account was admitted"
        );

        assert!(
            tokio::time::timeout(
                std::time::Duration::from_secs(1),
                colliding_started.notified(),
            )
            .await
            .is_ok(),
            "accounts colliding under the old shard hash blocked each other"
        );

        release_hot.notify_one();
        drop(wake_sender);
        supervisor.await.unwrap();
    }
}
