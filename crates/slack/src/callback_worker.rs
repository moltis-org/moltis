use std::sync::Arc;

use {
    moltis_channels::plugin::{ChannelEventSink, ChannelReplyTarget},
    slack_morphism::prelude::SlackPushEventCallback,
    tracing::warn,
};

use crate::{
    outbound::post_response_url,
    state::{AccountStateMap, DedupKind, EventDedup},
};

const CALLBACK_QUEUE_CAPACITY: usize = 256;

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
        response_url: Option<String>,
    },
    ResponseUrl {
        response_url: String,
        text: String,
    },
}

#[derive(Clone)]
pub(crate) struct CallbackQueue {
    sender: tokio::sync::mpsc::Sender<CallbackJob>,
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
        let (sender, receiver) = bounded_channel(CALLBACK_QUEUE_CAPACITY);
        tokio::spawn(run_worker(receiver, cancel.clone()));
        Self { sender, cancel }
    }

    /// Admit work immediately, allowing the Socket Mode callback to ACK in time.
    pub(crate) fn try_send(&self, job: CallbackJob) -> Result<(), CallbackAdmissionError> {
        if self.cancel.is_cancelled() {
            return Err(CallbackAdmissionError::Canceled);
        }
        match self.sender.try_send(job) {
            Ok(()) => Ok(()),
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                Err(CallbackAdmissionError::Full)
            },
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                Err(CallbackAdmissionError::Canceled)
            },
        }
    }

    /// Atomically check deduplication and commit it only after queue admission.
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
}

fn bounded_channel<T>(
    capacity: usize,
) -> (tokio::sync::mpsc::Sender<T>, tokio::sync::mpsc::Receiver<T>) {
    tokio::sync::mpsc::channel(capacity)
}

async fn run_worker(
    mut receiver: tokio::sync::mpsc::Receiver<CallbackJob>,
    cancel: tokio_util::sync::CancellationToken,
) {
    loop {
        let job = tokio::select! {
            () = cancel.cancelled() => {
                receiver.close();
                while let Some(job) = receiver.recv().await {
                    process(job).await;
                }
                break;
            },
            job = receiver.recv() => match job {
                Some(job) => job,
                None => break,
            },
        };
        process(job).await;
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
            response_url,
        } => {
            let response = sink
                .dispatch_interaction(&action_id, reply_to)
                .await
                .unwrap_or_else(|error| format!("Error: {error}"));
            if let Some(response_url) = response_url
                && let Err(error) = post_response_url(&response_url, &response).await
            {
                warn!("failed to send Slack interaction response: {error}");
            }
        },
        CallbackJob::ResponseUrl { response_url, text } => {
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

    #[tokio::test]
    async fn queue_full_and_cancellation_do_not_commit_dedup() {
        let cancel = tokio_util::sync::CancellationToken::new();
        let (sender, mut receiver) = bounded_channel(1);
        let queue = CallbackQueue {
            sender,
            cancel: cancel.clone(),
        };
        let mut dedup = EventDedup::default();
        let job = |text: &str| CallbackJob::ResponseUrl {
            response_url: "https://hooks.slack.com/actions/test".to_string(),
            text: text.to_string(),
        };

        assert_eq!(
            queue.try_send_deduplicated(&mut dedup, DedupKind::Event, "first", job("first")),
            Ok(CallbackAdmission::Queued)
        );
        assert_eq!(
            queue.try_send_deduplicated(&mut dedup, DedupKind::Event, "first", job("duplicate")),
            Ok(CallbackAdmission::Duplicate)
        );
        assert_eq!(
            queue.try_send_deduplicated(&mut dedup, DedupKind::Event, "retry", job("full")),
            Err(CallbackAdmissionError::Full)
        );
        assert!(!dedup.contains(DedupKind::Event, "retry"));

        assert!(receiver.recv().await.is_some());
        assert_eq!(
            queue.try_send_deduplicated(&mut dedup, DedupKind::Event, "retry", job("retry")),
            Ok(CallbackAdmission::Queued)
        );
        assert!(receiver.recv().await.is_some());

        cancel.cancel();
        assert_eq!(
            queue.try_send_deduplicated(&mut dedup, DedupKind::Event, "canceled", job("canceled")),
            Err(CallbackAdmissionError::Canceled)
        );
        assert!(!dedup.contains(DedupKind::Event, "canceled"));
    }
}
