use std::{future::Future, sync::Arc};

use {
    moltis_channels::{
        fair_queue::{Admission, FairQueue, FairQueueConfig, FairQueueJob},
        plugin::{ChannelEventSink, ChannelReplyTarget},
    },
    slack_morphism::prelude::SlackPushEventCallback,
    tracing::warn,
};

use crate::{
    outbound::post_response_url,
    state::{AccountStateMap, DedupKind, EventDedup},
};

/// Total Socket Mode callbacks that may be queued or in flight, across every
/// account.
const CALLBACK_QUEUE_CAPACITY: usize = 256;
/// Callbacks processed concurrently, and so the number of accounts served at
/// once. Matches the HTTP callback path so both behave the same under load.
const CALLBACK_WORKER_LIMIT: usize = 16;

pub(crate) enum CallbackJob {
    Push {
        event: Box<SlackPushEventCallback>,
        account_id: String,
        accounts: AccountStateMap,
    },
    Command {
        sink: Arc<dyn ChannelEventSink>,
        command: String,
        reply_to: ChannelReplyTarget,
        sender_id: String,
        response_url: String,
    },
    Interaction {
        sink: Arc<dyn ChannelEventSink>,
        action_id: String,
        reply_to: ChannelReplyTarget,
        sender_id: String,
        response_url: Option<String>,
    },
    ResponseUrl {
        account_id: String,
        response_url: String,
        text: String,
    },
}

impl FairQueueJob for CallbackJob {
    fn account_id(&self) -> &str {
        match self {
            Self::Push { account_id, .. } | Self::ResponseUrl { account_id, .. } => account_id,
            // A command or interaction is addressed to the account its reply
            // target names.
            Self::Command { reply_to, .. } | Self::Interaction { reply_to, .. } => {
                &reply_to.account_id
            },
        }
    }
}

/// Bounded, account-fair queue for Socket Mode callbacks.
///
/// Slack expects the socket callback to be acknowledged in seconds, so work is
/// admitted synchronously and processed afterwards. Accounts are served
/// round-robin with a capped share each, so one busy workspace cannot
/// head-of-line block the others — the same scheduler the HTTP callback path
/// uses.
#[derive(Clone)]
pub(crate) struct CallbackQueue {
    queue: Arc<FairQueue<CallbackJob>>,
    cancel: tokio_util::sync::CancellationToken,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CallbackAdmission {
    Queued,
    Duplicate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CallbackAdmissionError {
    Full,
    Canceled,
}

impl std::fmt::Display for CallbackAdmissionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full => formatter.write_str("Slack callback queue is full"),
            Self::Canceled => formatter.write_str("Slack callback queue is canceled"),
        }
    }
}

impl std::error::Error for CallbackAdmissionError {}

impl CallbackQueue {
    pub(crate) fn start(cancel: tokio_util::sync::CancellationToken) -> Self {
        Self::start_with(
            FairQueueConfig::new(CALLBACK_QUEUE_CAPACITY, CALLBACK_WORKER_LIMIT),
            cancel,
            process,
        )
    }

    /// Start with an explicit config and processor, so tests can use a small
    /// capacity and a processor that does not reach the network.
    fn start_with<F, Fut>(
        config: FairQueueConfig,
        cancel: tokio_util::sync::CancellationToken,
        process: F,
    ) -> Self
    where
        F: Fn(CallbackJob) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let queue = FairQueue::start(config, &cancel, process);
        Self {
            queue: Arc::new(queue),
            cancel,
        }
    }

    /// Admit work immediately, allowing the Socket Mode callback to ACK in time.
    pub(crate) fn try_send(&self, job: CallbackJob) -> Result<(), CallbackAdmissionError> {
        if self.cancel.is_cancelled() {
            return Err(CallbackAdmissionError::Canceled);
        }
        match self.queue.admit(job) {
            Admission::Admitted => Ok(()),
            Admission::Rejected => Err(CallbackAdmissionError::Full),
        }
    }

    /// Atomically check deduplication and commit it only after queue admission.
    ///
    /// Committing only on success matters: a callback refused for capacity gets
    /// no acknowledgement, so Slack redelivers it, and that retry must be
    /// admitted rather than dropped as an already-seen duplicate.
    pub(crate) fn try_send_deduplicated(
        &self,
        dedup: &mut EventDedup,
        kind: DedupKind,
        id: &str,
        job: CallbackJob,
    ) -> Result<CallbackAdmission, CallbackAdmissionError> {
        if dedup.contains(kind, id) {
            return Ok(CallbackAdmission::Duplicate);
        }
        self.try_send(job)?;
        let inserted = dedup.insert_new(kind, id);
        debug_assert!(inserted, "dedup changed while its lock was held");
        Ok(CallbackAdmission::Queued)
    }

    /// Callbacks queued or being processed. Test and diagnostic use.
    #[cfg(test)]
    fn in_flight(&self) -> usize {
        self.queue.in_flight()
    }
}

async fn process(job: CallbackJob) {
    match job {
        CallbackJob::Push {
            event,
            account_id,
            accounts,
        } => crate::socket::handle_push_event(*event, &account_id, &accounts).await,
        CallbackJob::Command {
            sink,
            command,
            reply_to,
            sender_id,
            response_url,
        } => {
            let response = sink
                .dispatch_command(&command, reply_to, Some(&sender_id))
                .await
                .unwrap_or_else(|error| format!("Error: {error}"));
            if let Err(error) = post_response_url(&response_url, &response).await {
                warn!("failed to send Slack command response: {error}");
            }
        },
        CallbackJob::Interaction {
            sink,
            action_id,
            reply_to,
            sender_id,
            response_url,
        } => {
            let response = sink
                .dispatch_interaction(&action_id, reply_to, Some(&sender_id))
                .await
                .unwrap_or_else(|error| format!("Error: {error}"));
            if let Some(response_url) = response_url
                && let Err(error) = post_response_url(&response_url, &response).await
            {
                warn!("failed to send Slack interaction response: {error}");
            }
        },
        CallbackJob::ResponseUrl {
            response_url,
            text,
            account_id: _,
        } => {
            if let Err(error) = post_response_url(&response_url, &text).await {
                warn!("failed to send Slack callback response: {error}");
            }
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    use {
        moltis_channels::{
            ChannelType,
            error::Result as ChannelResult,
            plugin::{ChannelEvent, ChannelMessageMeta},
        },
        std::sync::Mutex,
        tokio_util::sync::CancellationToken,
    };

    #[derive(Default)]
    struct InteractionSink {
        sender_ids: Mutex<Vec<Option<String>>>,
    }

    #[async_trait::async_trait]
    impl ChannelEventSink for InteractionSink {
        async fn emit(&self, _event: ChannelEvent) {}

        async fn dispatch_to_chat(
            &self,
            _text: &str,
            _reply_to: ChannelReplyTarget,
            _meta: ChannelMessageMeta,
        ) {
        }

        async fn dispatch_command(
            &self,
            _command: &str,
            _reply_to: ChannelReplyTarget,
            _sender_id: Option<&str>,
        ) -> ChannelResult<String> {
            Ok(String::new())
        }

        async fn dispatch_interaction(
            &self,
            _callback_data: &str,
            _reply_to: ChannelReplyTarget,
            sender_id: Option<&str>,
        ) -> ChannelResult<String> {
            self.sender_ids
                .lock()
                .unwrap()
                .push(sender_id.map(String::from));
            Ok(String::new())
        }

        async fn request_disable_account(
            &self,
            _channel_type: &str,
            _account_id: &str,
            _reason: &str,
        ) {
        }
    }

    fn job(account_id: &str) -> CallbackJob {
        CallbackJob::ResponseUrl {
            account_id: account_id.to_string(),
            response_url: "https://hooks.slack.com/actions/test".to_string(),
            text: "body".to_string(),
        }
    }

    #[tokio::test]
    async fn interaction_jobs_preserve_the_sender_principal() {
        let sink = Arc::new(InteractionSink::default());
        process(CallbackJob::Interaction {
            sink: sink.clone(),
            action_id: "approve".to_string(),
            reply_to: ChannelReplyTarget {
                ack_message_id: None,
                channel_type: ChannelType::Slack,
                account_id: "default".to_string(),
                chat_id: "C123".to_string(),
                message_id: None,
                thread_id: None,
            },
            sender_id: "U123".to_string(),
            response_url: None,
        })
        .await;

        assert_eq!(*sink.sender_ids.lock().unwrap(), vec![Some(
            "U123".to_string()
        )]);
    }

    /// A queue whose jobs block on `gate` until it is cancelled, so admission
    /// can be driven to saturation without touching the network.
    fn blocking_queue(capacity: usize, gate: CancellationToken) -> CallbackQueue {
        CallbackQueue::start_with(
            FairQueueConfig {
                capacity,
                workers: 1,
                per_account_capacity: capacity,
            },
            CancellationToken::new(),
            move |_job| {
                let gate = gate.clone();
                async move { gate.cancelled().await }
            },
        )
    }

    #[tokio::test]
    async fn a_rejected_callback_does_not_commit_dedup() {
        // Slack got no ACK for a refused callback and will redeliver it, so the
        // retry must be admitted rather than dropped as already-seen.
        let gate = CancellationToken::new();
        let queue = blocking_queue(1, gate.clone());
        let mut dedup = EventDedup::default();

        assert_eq!(
            queue.try_send_deduplicated(&mut dedup, DedupKind::Event, "first", job("acct")),
            Ok(CallbackAdmission::Queued)
        );
        assert_eq!(
            queue.try_send_deduplicated(&mut dedup, DedupKind::Event, "first", job("acct")),
            Ok(CallbackAdmission::Duplicate)
        );
        assert_eq!(
            queue.try_send_deduplicated(&mut dedup, DedupKind::Event, "retry", job("acct")),
            Err(CallbackAdmissionError::Full)
        );
        assert!(!dedup.contains(DedupKind::Event, "retry"));

        gate.cancel();
        while queue.in_flight() > 0 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            queue.try_send_deduplicated(&mut dedup, DedupKind::Event, "retry", job("acct")),
            Ok(CallbackAdmission::Queued)
        );
    }

    #[tokio::test]
    async fn a_canceled_queue_admits_nothing_and_commits_nothing() {
        let cancel = CancellationToken::new();
        let queue = CallbackQueue::start_with(
            FairQueueConfig::new(16, 1),
            cancel.clone(),
            |_job| async move {},
        );
        let mut dedup = EventDedup::default();
        cancel.cancel();

        assert_eq!(
            queue.try_send_deduplicated(&mut dedup, DedupKind::Event, "canceled", job("acct")),
            Err(CallbackAdmissionError::Canceled)
        );
        assert!(!dedup.contains(DedupKind::Event, "canceled"));
    }

    #[tokio::test]
    async fn a_busy_account_does_not_block_another_accounts_callbacks() {
        // The regression this queue exists to prevent: before it, one worker
        // served every account, so a slow workspace stalled all the others.
        let gate = CancellationToken::new();
        let ran = Arc::new(tokio::sync::Notify::new());
        let queue = {
            let (gate, ran) = (gate.clone(), Arc::clone(&ran));
            CallbackQueue::start_with(
                FairQueueConfig::new(16, 4),
                CancellationToken::new(),
                move |job: CallbackJob| {
                    let (gate, ran) = (gate.clone(), Arc::clone(&ran));
                    async move {
                        if job.account_id() == "busy" {
                            gate.cancelled().await;
                        } else {
                            ran.notify_one();
                        }
                    }
                },
            )
        };

        assert!(queue.try_send(job("busy")).is_ok());
        assert!(queue.try_send(job("quiet")).is_ok());

        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), ran.notified())
                .await
                .is_ok(),
            "a busy Slack account head-of-line blocked an unrelated one"
        );
        gate.cancel();
    }
}
