use std::sync::{Arc, RwLock};

use {
    axum::{
        http::StatusCode,
        response::{IntoResponse, Json, Response},
    },
    bytes::Bytes,
    moltis_channels::fair_queue::{Admission, FairQueue, FairQueueConfig, FairQueueJob},
    moltis_gateway::{
        channel_webhook_dedup::{ChannelWebhookAdmission, ChannelWebhookDedupeStore},
        state::GatewayState,
    },
    tokio_util::sync::CancellationToken,
};

/// Total Slack callbacks that may be queued or in flight, across every account.
const CALLBACK_QUEUE_CAPACITY: usize = 256;
/// Callbacks processed concurrently, and so the number of accounts served at once.
const CALLBACK_WORKER_LIMIT: usize = 16;

/// Slack callback endpoints served under `/api/channels/slack/{account_id}/`.
///
/// All three share the same admission pipeline (verify → rate limit → dedupe →
/// bounded queue) and differ only in how they acknowledge, so the variant is
/// what parameterizes [`handle_slack_callback`].
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

    /// Acknowledge an admission decision.
    ///
    /// Slash commands render their response body in the Slack client, so an
    /// accepted (or deduped) command answers with a bare 200 rather than JSON
    /// the user would see. Events and interactions are machine-read, so they
    /// report the decision as JSON.
    fn ack_response(self, admission: ChannelWebhookAdmission) -> Response {
        match (self, admission) {
            (
                Self::Command,
                ChannelWebhookAdmission::Admitted | ChannelWebhookAdmission::Duplicate,
            ) => StatusCode::OK.into_response(),
            (_, ChannelWebhookAdmission::Admitted) => {
                slack_json_response(StatusCode::OK, serde_json::json!({ "ok": true }))
            },
            (_, ChannelWebhookAdmission::Duplicate) => slack_json_response(
                StatusCode::OK,
                serde_json::json!({ "ok": true, "deduplicated": true }),
            ),
            (_, ChannelWebhookAdmission::Rejected) => slack_json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                serde_json::json!({ "ok": false, "error": "Slack callback queue unavailable" }),
            ),
        }
    }
}

fn slack_json_response(status: StatusCode, body: serde_json::Value) -> Response {
    (status, Json(body)).into_response()
}

/// Verify a Slack callback and hand it to the bounded queue.
///
/// Slack expects an acknowledgement within three seconds, so processing never
/// happens inline: the request is verified (HMAC, timestamp, rate limit),
/// deduplicated, queued, and acknowledged, and the queue's workers run the
/// actual ingest afterwards. A queue that cannot accept the callback answers
/// 503 so Slack retries rather than dropping the event silently.
pub(super) async fn handle_slack_callback(
    plugin: Arc<tokio::sync::RwLock<moltis_slack::SlackPlugin>>,
    dispatcher: Arc<SlackCallbackDispatcher>,
    gateway_state: Arc<GatewayState>,
    kind: SlackCallbackKind,
    account_id: String,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> Response {
    use moltis_channels::ChannelPlugin;

    let verifier = {
        let plugin = plugin.read().await;
        plugin.channel_webhook_verifier(&account_id)
    };
    let Some(verifier) = verifier else {
        return slack_json_response(
            StatusCode::NOT_FOUND,
            serde_json::json!({ "ok": false, "error": "unknown Slack account" }),
        );
    };

    let verified = match moltis_gateway::channel_webhook_middleware::verify_channel_webhook(
        verifier.as_ref(),
        &gateway_state.channel_webhook_rate_limiter,
        &account_id,
        kind.endpoint(),
        &headers,
        &body,
    ) {
        Ok(verified) => verified,
        Err(rejection) => {
            return crate::channel_webhook_middleware::rejection_into_response(rejection);
        },
    };

    if matches!(kind, SlackCallbackKind::Event)
        && let Some(response) = event_preflight(&verified.body)
    {
        return response;
    }

    kind.ack_response(dispatcher.admit(
        &gateway_state.channel_webhook_dedup,
        account_id,
        verified.idempotency_key.as_deref(),
        kind,
        verified.body,
    ))
}

/// Handle the two events-endpoint cases that must answer synchronously instead
/// of being queued: Slack's one-time `url_verification` handshake, which has to
/// echo the challenge back in the response body, and a malformed payload, which
/// is rejected rather than queued for a worker that could only discard it.
///
/// Returns `None` when the body is a normal event envelope.
fn event_preflight(body: &[u8]) -> Option<Response> {
    let payload: serde_json::Value = match serde_json::from_slice(body) {
        Ok(payload) => payload,
        Err(error) => {
            return Some(slack_json_response(
                StatusCode::BAD_REQUEST,
                serde_json::json!({ "ok": false, "error": error.to_string() }),
            ));
        },
    };
    if payload.get("type").and_then(serde_json::Value::as_str) != Some("url_verification") {
        return None;
    }
    Some(
        match payload.get("challenge").and_then(serde_json::Value::as_str) {
            Some(challenge) => slack_json_response(
                StatusCode::OK,
                serde_json::json!({ "challenge": challenge }),
            ),
            None => slack_json_response(
                StatusCode::BAD_REQUEST,
                serde_json::json!({ "ok": false, "error": "missing Slack challenge" }),
            ),
        },
    )
}

/// One verified Slack callback waiting to be ingested.
struct SlackCallbackJob {
    account_id: String,
    body: Bytes,
    kind: SlackCallbackKind,
}

impl FairQueueJob for SlackCallbackJob {
    fn account_id(&self) -> &str {
        &self.account_id
    }
}

/// Bounded, account-fair callback queue.
///
/// Owns the worker pool; dropping it stops the workers after they finish the
/// callbacks already admitted (those were acknowledged to Slack and will not be
/// redelivered).
pub(super) struct SlackCallbackDispatcher {
    queue: FairQueue<SlackCallbackJob>,
}

impl SlackCallbackDispatcher {
    pub(super) fn start(plugin: Arc<tokio::sync::RwLock<moltis_slack::SlackPlugin>>) -> Arc<Self> {
        let config = FairQueueConfig::new(CALLBACK_QUEUE_CAPACITY, CALLBACK_WORKER_LIMIT);
        let queue = FairQueue::start(config, &CancellationToken::new(), move |job| {
            process_callback(Arc::clone(&plugin), job)
        });
        Arc::new(Self { queue })
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
            &self.queue,
            dedup_store,
            account_id,
            idempotency_key,
            kind,
            body,
        )
    }
}

/// Deduplicate, then queue.
///
/// The dedupe entry is committed only if the queue actually accepted the job, so
/// a callback rejected for capacity is not remembered as "seen" — Slack's retry
/// of it must be admitted rather than silently swallowed as a duplicate.
fn admit_callback(
    queue: &FairQueue<SlackCallbackJob>,
    dedup_store: &RwLock<ChannelWebhookDedupeStore>,
    account_id: String,
    idempotency_key: Option<&str>,
    kind: SlackCallbackKind,
    body: Bytes,
) -> ChannelWebhookAdmission {
    let endpoint = kind.endpoint();
    let enqueue = |account_id: String| {
        queue.admit(SlackCallbackJob {
            account_id,
            body,
            kind,
        }) == Admission::Admitted
    };

    let Some(key) = idempotency_key else {
        return if enqueue(account_id) {
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
            enqueue(job_account_id)
        })
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

    fn preflight_status(body: &[u8]) -> Option<StatusCode> {
        event_preflight(body).map(|response| response.status())
    }

    #[test]
    fn url_verification_echoes_the_challenge_before_queueing() {
        assert_eq!(
            preflight_status(br#"{"type":"url_verification","challenge":"abc123"}"#),
            Some(StatusCode::OK)
        );
    }

    #[test]
    fn url_verification_without_a_challenge_is_rejected() {
        assert_eq!(
            preflight_status(br#"{"type":"url_verification"}"#),
            Some(StatusCode::BAD_REQUEST)
        );
    }

    #[test]
    fn malformed_event_body_is_rejected_instead_of_queued() {
        assert_eq!(preflight_status(b"not json"), Some(StatusCode::BAD_REQUEST));
    }

    #[test]
    fn a_normal_event_envelope_falls_through_to_the_queue() {
        assert!(
            event_preflight(br#"{"type":"event_callback","event":{"type":"message"}}"#).is_none()
        );
    }

    #[test]
    fn slash_commands_acknowledge_with_an_empty_body() {
        // A JSON body here would be rendered to the user in the Slack client.
        for admission in [
            ChannelWebhookAdmission::Admitted,
            ChannelWebhookAdmission::Duplicate,
        ] {
            let response = SlackCallbackKind::Command.ack_response(admission);
            assert_eq!(response.status(), StatusCode::OK);
            assert!(
                response
                    .headers()
                    .get(axum::http::header::CONTENT_TYPE)
                    .is_none()
            );
        }
    }

    #[test]
    fn machine_read_endpoints_acknowledge_with_json() {
        for kind in [SlackCallbackKind::Event, SlackCallbackKind::Interaction] {
            let response = kind.ack_response(ChannelWebhookAdmission::Admitted);
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers().get(axum::http::header::CONTENT_TYPE),
                Some(&axum::http::HeaderValue::from_static("application/json"))
            );
        }
    }

    #[test]
    fn a_saturated_queue_asks_slack_to_retry_on_every_endpoint() {
        // 503 (not 200) so Slack redelivers instead of dropping the callback.
        for kind in [
            SlackCallbackKind::Event,
            SlackCallbackKind::Interaction,
            SlackCallbackKind::Command,
        ] {
            assert_eq!(
                kind.ack_response(ChannelWebhookAdmission::Rejected)
                    .status(),
                StatusCode::SERVICE_UNAVAILABLE
            );
        }
    }

    /// Build a queue whose jobs block on `gate` until it is cancelled, so
    /// admission can be driven to saturation deterministically.
    ///
    /// A `CancellationToken` rather than a `Notify`: it stays signalled once
    /// cancelled, so a job that reaches its await *after* the release still
    /// proceeds. `notify_waiters` would drop that wakeup and hang the test.
    fn blocking_queue(capacity: usize, gate: CancellationToken) -> FairQueue<SlackCallbackJob> {
        FairQueue::start(
            FairQueueConfig {
                capacity,
                workers: 1,
                per_account_capacity: capacity,
            },
            &CancellationToken::new(),
            move |_job| {
                let gate = gate.clone();
                async move { gate.cancelled().await }
            },
        )
    }

    fn admit(
        queue: &FairQueue<SlackCallbackJob>,
        dedup: &RwLock<ChannelWebhookDedupeStore>,
        key: Option<&str>,
        body: &'static [u8],
    ) -> ChannelWebhookAdmission {
        admit_callback(
            queue,
            dedup,
            "account".to_string(),
            key,
            SlackCallbackKind::Event,
            Bytes::from_static(body),
        )
    }

    #[tokio::test]
    async fn a_rejected_callback_is_not_remembered_as_seen() {
        // Capacity rejection must not commit the dedupe entry: Slack retries a
        // callback it got no 200 for, and that retry has to be admitted rather
        // than swallowed as a duplicate.
        let gate = CancellationToken::new();
        let queue = blocking_queue(1, gate.clone());
        let dedup = RwLock::new(ChannelWebhookDedupeStore::new());

        // Fills the queue's single slot.
        assert_eq!(
            admit(&queue, &dedup, Some("event-1"), b"first"),
            ChannelWebhookAdmission::Admitted
        );
        // Saturated, so a different event is refused...
        assert_eq!(
            admit(&queue, &dedup, Some("event-2"), b"second"),
            ChannelWebhookAdmission::Rejected
        );
        // ...and refusing it did not record it, so its retry gets in once
        // capacity frees up.
        gate.cancel();
        while queue.in_flight() > 0 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            admit(&queue, &dedup, Some("event-2"), b"second-retry"),
            ChannelWebhookAdmission::Admitted
        );
        // A genuine redelivery of something already admitted is still deduped.
        assert_eq!(
            admit(&queue, &dedup, Some("event-1"), b"duplicate"),
            ChannelWebhookAdmission::Duplicate
        );
    }

    #[tokio::test]
    async fn a_callback_without_an_idempotency_key_is_always_admitted() {
        let gate = CancellationToken::new();
        gate.cancel();
        let queue = blocking_queue(4, gate);
        let dedup = RwLock::new(ChannelWebhookDedupeStore::new());

        for _ in 0..2 {
            assert_eq!(
                admit(&queue, &dedup, None, b"no-key"),
                ChannelWebhookAdmission::Admitted
            );
        }
    }
}
