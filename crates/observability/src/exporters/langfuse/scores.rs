//! Score ingestion.
//!
//! Scores are the one part of the model OTLP cannot carry, so they take a
//! separate path: a dedicated sink that filters the fanout down to
//! [`Event::Score`] and posts stable requests to the Scores API.
//!
//! Without this sink `TurnRecorder::score` would emit into a void — the OTLP
//! mapping drops score events by design, so the Langfuse trace transport alone
//! silently discards every score.

use std::{sync::Arc, time::Duration};

use {
    async_trait::async_trait,
    serde::Serialize,
    tracing::{debug, warn},
};

use {
    super::{client::LangfuseClient, config::SCORES_PATH},
    crate::{
        model::{Event, ScoreDeleteRecord, ScoreRecord, ScoreValue},
        runtime::{BatchConfig, BatchSink, Transport, TransportError},
        sink::ObservationSink,
    },
};

/// Value shape accepted by the dedicated Scores API. BOOLEAN uses 0/1 on this
/// API even though the recorder models it as a real boolean.
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum ScoreApiValue<'a> {
    Numeric(f64),
    Text(&'a str),
}

/// Langfuse create-score request.
#[derive(Debug, Serialize)]
struct ScoreBody<'a> {
    id: &'a str,
    #[serde(rename = "traceId")]
    trace_id: String,
    #[serde(rename = "observationId", skip_serializing_if = "Option::is_none")]
    observation_id: Option<String>,
    name: &'a str,
    value: ScoreApiValue<'a>,
    #[serde(rename = "dataType")]
    data_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    comment: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    environment: Option<&'a str>,
}

impl<'a> ScoreBody<'a> {
    fn new(score: &'a ScoreRecord, default_environment: Option<&'a str>) -> Self {
        let (value, data_type) = match &score.value {
            ScoreValue::Numeric(value) => (ScoreApiValue::Numeric(*value), "NUMERIC"),
            ScoreValue::Categorical(value) => (ScoreApiValue::Text(value), "CATEGORICAL"),
            ScoreValue::Boolean(value) => (
                ScoreApiValue::Numeric(if *value {
                    1.0
                } else {
                    0.0
                }),
                "BOOLEAN",
            ),
        };
        Self {
            id: &score.id,
            trace_id: score.trace_id.as_otel_hex(),
            observation_id: score
                .observation_id
                .as_ref()
                .map(crate::model::ObservationId::as_otel_hex),
            name: &score.name,
            value,
            data_type,
            comment: score.comment.as_deref(),
            environment: score.environment.as_deref().or(default_environment),
        }
    }
}

impl LangfuseClient {
    /// Submit scores. Langfuse upserts on score id, so re-submitting a score
    /// with the same id overwrites rather than duplicating — which is what
    /// makes a user changing their reaction idempotent.
    pub async fn submit_scores(&self, scores: &[ScoreRecord]) -> anyhow::Result<()> {
        if scores.is_empty() {
            return Ok(());
        }

        self.submit_scores_for_transport(scores)
            .await
            .map_err(anyhow::Error::new)?;

        debug!(count = scores.len(), "submitted scores to Langfuse");
        Ok(())
    }

    async fn submit_scores_for_transport(
        &self,
        scores: &[ScoreRecord],
    ) -> Result<(), TransportError> {
        for score in scores {
            let body = ScoreBody::new(score, self.config().environment.as_deref());
            let response = self
                .post(SCORES_PATH)
                .json(&body)
                .send()
                .await
                .map_err(|error| TransportError::Retryable(error.to_string()))?;

            let status = response.status();
            if status.is_success() {
                continue;
            }
            let message = format!("Langfuse rejected score {} with HTTP {status}", score.id);
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS
                || status == reqwest::StatusCode::REQUEST_TIMEOUT
                || status.is_server_error()
            {
                return Err(TransportError::Retryable(message));
            }
            return Err(TransportError::Fatal(message));
        }
        Ok(())
    }

    /// Delete a score by id.
    ///
    /// Backs reaction removal: a user who takes their thumb back has retracted
    /// the opinion, and leaving the score in place would keep counting a vote
    /// they withdrew. A 404 is success — the score is already gone, which is
    /// the state the caller wanted.
    pub async fn delete_score(&self, score_id: &str) -> anyhow::Result<()> {
        let response = self
            .delete(&format!("{SCORES_PATH}/{score_id}"))
            .send()
            .await?;

        let status = response.status();
        if status.is_success() || status == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }

        Err(anyhow::anyhow!(
            "Langfuse rejected the score deletion with HTTP {status}"
        ))
    }

    async fn delete_score_for_transport(
        &self,
        deletion: &ScoreDeleteRecord,
    ) -> Result<(), TransportError> {
        let response = self
            .delete(&format!("{SCORES_PATH}/{}", deletion.score_id))
            .send()
            .await
            .map_err(|error| TransportError::Retryable(error.to_string()))?;
        let status = response.status();
        if status.is_success() || status == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        let message = format!(
            "Langfuse rejected score deletion {} with HTTP {status}",
            deletion.score_id
        );
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS
            || status == reqwest::StatusCode::REQUEST_TIMEOUT
            || status.is_server_error()
        {
            Err(TransportError::Retryable(message))
        } else {
            Err(TransportError::Fatal(message))
        }
    }
}

/// Batch transport that posts scores to the dedicated Scores API.
pub struct ScoreTransport {
    client: Arc<LangfuseClient>,
}

impl ScoreTransport {
    /// Build a transport over an existing client.
    #[must_use]
    pub const fn new(client: Arc<LangfuseClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Transport for ScoreTransport {
    fn name(&self) -> &str {
        "langfuse-scores"
    }

    async fn send(&self, batch: &[Event]) -> Result<(), TransportError> {
        for event in batch {
            match event {
                Event::Score(score) => {
                    self.client
                        .submit_scores_for_transport(std::slice::from_ref(score.as_ref()))
                        .await?;
                },
                Event::ScoreDelete(deletion) => {
                    self.client
                        .delete_score_for_transport(deletion.as_ref())
                        .await?;
                },
                _ => {},
            }
        }
        Ok(())
    }
}

/// Sink that carries only scores.
///
/// The fanout hands every event to every sink, so this wrapper drops
/// non-score events before they reach the queue — otherwise a busy agent
/// would fill the score queue with trace events and evict real scores.
pub struct ScoreSink {
    inner: BatchSink,
}

impl ScoreSink {
    /// Spawn a score sink over `client`.
    #[must_use]
    pub fn spawn(client: Arc<LangfuseClient>, mut config: BatchConfig) -> Self {
        // Each score is a distinct HTTP mutation. Single-event batches keep
        // retries and delivery statistics exact without sacrificing queue order.
        config.max_batch_events = 1;
        Self {
            inner: BatchSink::spawn(Arc::new(ScoreTransport::new(client)), config),
        }
    }
}

#[async_trait]
impl ObservationSink for ScoreSink {
    fn name(&self) -> &str {
        "langfuse-scores"
    }

    fn record(&self, event: Event) {
        if matches!(event, Event::Score(_) | Event::ScoreDelete(_)) {
            self.inner.record(event);
        }
    }

    async fn flush(&self, timeout: Duration) -> anyhow::Result<()> {
        if let Err(error) = self.inner.flush(timeout).await {
            warn!(%error, "score sink flush failed");
            return Err(error);
        }
        Ok(())
    }

    fn delivery_stats(&self) -> Vec<crate::runtime::SinkStatsSnapshot> {
        self.inner.delivery_stats()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use {
        std::time::Duration,
        wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{body_json, method, path},
        },
    };

    use {
        super::*,
        crate::{
            exporters::langfuse::config::LangfuseConfig,
            model::{ObservationId, TraceId, TraceRecord},
        },
        secrecy::SecretString,
    };

    fn config(host: String) -> LangfuseConfig {
        LangfuseConfig {
            host,
            public_key: "pk-lf-test".into(),
            secret_key: SecretString::new("sk-lf-secret".to_string()),
            environment: Some("production".into()),
            release: Some("20260726.01".into()),
            timeout: Duration::from_secs(5),
        }
    }

    fn score(name: &str) -> ScoreRecord {
        ScoreRecord::new(
            TraceId("01234567-89ab-cdef-0123-456789abcdef".into()),
            name,
            ScoreValue::Numeric(1.0),
        )
    }

    #[tokio::test]
    async fn scores_post_to_the_dedicated_api_with_wire_ids() {
        let server = MockServer::start().await;
        let expected = serde_json::json!({
            "id": "score-1",
            "traceId": "0123456789abcdef0123456789abcdef",
            "observationId": "fedcba9876543210",
            "name": "user-feedback",
            "value": 1.0,
            "dataType": "NUMERIC",
            "comment": "helpful",
            "environment": "production"
        });
        Mock::given(method("POST"))
            .and(path(SCORES_PATH))
            .and(body_json(expected))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "score-1"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let mut score = score("user-feedback");
        score.id = "score-1".into();
        score.observation_id = Some(ObservationId("fedcba98-7654-3210-fedc-ba9876543210".into()));
        score.comment = Some("helpful".into());

        LangfuseClient::new(config(server.uri()))
            .submit_scores(&[score])
            .await
            .expect("score submission should succeed");
    }

    #[tokio::test]
    async fn boolean_scores_use_the_boolean_data_type() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(SCORES_PATH))
            .and(body_json(serde_json::json!({
                "id": "boolean-score",
                "traceId": "0123456789abcdef0123456789abcdef",
                "name": "user-feedback",
                "value": 0.0,
                "dataType": "BOOLEAN",
                "environment": "production"
            })))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let mut score = score("user-feedback");
        score.id = "boolean-score".into();
        score.value = ScoreValue::Boolean(false);
        LangfuseClient::new(config(server.uri()))
            .submit_scores(&[score])
            .await
            .expect("boolean score should be accepted");
    }

    #[tokio::test]
    async fn empty_score_batches_skip_the_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        LangfuseClient::new(config(server.uri()))
            .submit_scores(&[])
            .await
            .expect("empty batch is a no-op");
    }

    #[test]
    fn categorical_and_numeric_scores_declare_their_data_type() {
        let numeric = ScoreValue::Numeric(0.5);
        let categorical = ScoreValue::Categorical("helpful".into());

        assert_eq!(
            serde_json::to_value(&numeric).expect("serializable"),
            serde_json::json!(0.5)
        );
        assert_eq!(
            serde_json::to_value(&categorical).expect("serializable"),
            serde_json::json!("helpful")
        );
        assert_eq!(
            serde_json::to_value(ScoreValue::Boolean(true)).expect("serializable"),
            serde_json::json!(true)
        );
    }

    #[tokio::test]
    async fn the_transport_ignores_non_score_events() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let client = Arc::new(LangfuseClient::new(config(server.uri())));
        let transport = ScoreTransport::new(client);
        let batch = vec![Event::Trace(Box::new(TraceRecord::new("agent-run")))];

        transport.send(&batch).await.expect("no scores, no request");
    }

    #[tokio::test]
    async fn the_sink_drops_trace_events_before_they_reach_the_queue() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(SCORES_PATH))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let client = Arc::new(LangfuseClient::new(config(server.uri())));
        let sink = ScoreSink::spawn(client, BatchConfig::default());

        // A busy agent emits far more trace events than scores; if they were
        // queued here they would evict the scores we actually care about.
        for _ in 0..64 {
            sink.record(Event::Trace(Box::new(TraceRecord::new("agent-run"))));
        }
        sink.record(Event::Score(Box::new(score("user-feedback"))));

        sink.flush(Duration::from_secs(5))
            .await
            .expect("flush should succeed");
    }

    #[tokio::test]
    async fn retracting_a_score_deletes_it_by_id() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path(format!("{SCORES_PATH}/score-1")))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        LangfuseClient::new(config(server.uri()))
            .delete_score("score-1")
            .await
            .expect("deletion should succeed");
    }

    #[tokio::test]
    async fn deleting_an_already_absent_score_succeeds() {
        let server = MockServer::start().await;
        // Reaction-removal events can arrive twice, or after the score was
        // cleaned up. Already-gone is the state the caller asked for.
        Mock::given(method("DELETE"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        LangfuseClient::new(config(server.uri()))
            .delete_score("missing")
            .await
            .expect("404 is success for a deletion");
    }

    #[tokio::test]
    async fn a_failed_deletion_is_surfaced() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let error = LangfuseClient::new(config(server.uri()))
            .delete_score("score-1")
            .await
            .expect_err("500 should fail");

        assert!(error.to_string().contains("500"), "{error}");
    }

    #[tokio::test]
    async fn a_rejected_batch_is_retryable_rather_than_dropped() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(SCORES_PATH))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let client = Arc::new(LangfuseClient::new(config(server.uri())));
        let transport = ScoreTransport::new(client);
        let batch = vec![Event::Score(Box::new(score("user-feedback")))];

        let error = transport.send(&batch).await.expect_err("503 should fail");
        assert!(
            matches!(error, TransportError::Retryable(_)),
            "a transient rejection must not discard user feedback, got {error:?}"
        );
    }

    #[tokio::test]
    async fn invalid_scores_are_fatal() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(SCORES_PATH))
            .respond_with(ResponseTemplate::new(400))
            .mount(&server)
            .await;

        let client = Arc::new(LangfuseClient::new(config(server.uri())));
        let transport = ScoreTransport::new(client);
        let batch = vec![Event::Score(Box::new(score("user-feedback")))];

        let error = transport.send(&batch).await.expect_err("400 should fail");
        assert!(matches!(error, TransportError::Fatal(_)), "{error:?}");
    }

    #[test]
    fn retry_payload_is_stable() {
        let mut score = score("user-feedback");
        score.id = "stable-score".into();
        score.value = ScoreValue::Boolean(true);
        let first =
            serde_json::to_vec(&ScoreBody::new(&score, Some("production"))).expect("serializable");
        let second =
            serde_json::to_vec(&ScoreBody::new(&score, Some("production"))).expect("serializable");
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn create_and_delete_are_delivered_in_queue_order() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(SCORES_PATH))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path(format!("{SCORES_PATH}/ordered-score")))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = Arc::new(LangfuseClient::new(config(server.uri())));
        let transport = ScoreTransport::new(client);
        let mut created = score("user-feedback");
        created.id = "ordered-score".into();
        let deleted = ScoreDeleteRecord::new(created.trace_id.clone(), created.id.clone());

        transport
            .send(&[
                Event::Score(Box::new(created)),
                Event::ScoreDelete(Box::new(deleted)),
            ])
            .await
            .expect("ordered mutations should succeed");

        let requests = server.received_requests().await.expect("request log");
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].method.as_str(), "POST");
        assert_eq!(requests[1].method.as_str(), "DELETE");
    }
}
