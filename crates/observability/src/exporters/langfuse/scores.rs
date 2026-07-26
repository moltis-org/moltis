//! Score ingestion.
//!
//! Scores are the one part of the model OTLP cannot carry, so they take a
//! separate path: a dedicated sink that filters the fanout down to
//! [`Event::Score`] and posts batches to the ingestion API.
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
    super::{
        client::LangfuseClient,
        config::{INGESTION_PATH, SCORES_PATH},
    },
    crate::{
        model::{Event, ScoreRecord, ScoreValue},
        runtime::{BatchConfig, BatchSink, Transport, TransportError},
        sink::ObservationSink,
    },
};

/// A `score-create` event in the ingestion batch envelope.
#[derive(Debug, Serialize)]
struct IngestionEvent<'a> {
    id: String,
    timestamp: String,
    #[serde(rename = "type")]
    kind: &'static str,
    body: ScoreBody<'a>,
}

/// Langfuse `ScoreBody`.
#[derive(Debug, Serialize)]
struct ScoreBody<'a> {
    id: &'a str,
    #[serde(rename = "traceId")]
    trace_id: &'a str,
    #[serde(rename = "observationId", skip_serializing_if = "Option::is_none")]
    observation_id: Option<&'a str>,
    name: &'a str,
    value: &'a ScoreValue,
    #[serde(rename = "dataType")]
    data_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    comment: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    environment: Option<&'a str>,
}

/// The ingestion request envelope.
#[derive(Debug, Serialize)]
struct IngestionRequest<'a> {
    batch: Vec<IngestionEvent<'a>>,
}

impl LangfuseClient {
    /// Submit scores. Langfuse upserts on score id, so re-submitting a score
    /// with the same id overwrites rather than duplicating — which is what
    /// makes a user changing their reaction idempotent.
    pub async fn submit_scores(&self, scores: &[ScoreRecord]) -> anyhow::Result<()> {
        if scores.is_empty() {
            return Ok(());
        }

        let now = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();

        let batch: Vec<IngestionEvent<'_>> = scores
            .iter()
            .map(|score| IngestionEvent {
                id: uuid::Uuid::new_v4().to_string(),
                timestamp: now.clone(),
                kind: "score-create",
                body: ScoreBody {
                    id: &score.id,
                    trace_id: &score.trace_id.0,
                    observation_id: score.observation_id.as_ref().map(|o| o.0.as_str()),
                    name: &score.name,
                    value: &score.value,
                    data_type: match score.value {
                        ScoreValue::Numeric(_) => "NUMERIC",
                        ScoreValue::Categorical(_) => "CATEGORICAL",
                    },
                    comment: score.comment.as_deref(),
                    environment: score
                        .environment
                        .as_deref()
                        .or(self.config().environment.as_deref()),
                },
            })
            .collect();

        let response = self
            .post(INGESTION_PATH)
            .json(&IngestionRequest { batch })
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Langfuse rejected scores: HTTP {status}: {}",
                body.chars().take(512).collect::<String>()
            ));
        }

        // The ingestion endpoint reports per-event outcomes in a 207, so a 2xx
        // alone does not mean every score landed.
        let body: serde_json::Value = response.json().await.unwrap_or_default();
        if let Some(errors) = body.get("errors").and_then(|e| e.as_array())
            && !errors.is_empty()
        {
            return Err(anyhow::anyhow!(
                "Langfuse rejected {} of {} scores: {}",
                errors.len(),
                scores.len(),
                serde_json::to_string(errors).unwrap_or_default()
            ));
        }

        debug!(count = scores.len(), "submitted scores to Langfuse");
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

        let body = response.text().await.unwrap_or_default();
        Err(anyhow::anyhow!(
            "Langfuse rejected the score deletion: HTTP {status}: {}",
            body.chars().take(512).collect::<String>()
        ))
    }
}

/// Batch transport that posts scores to the ingestion API.
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
        let scores: Vec<ScoreRecord> = batch
            .iter()
            .filter_map(|event| match event {
                Event::Score(score) => Some((**score).clone()),
                _ => None,
            })
            .collect();

        if scores.is_empty() {
            return Ok(());
        }

        self.client.submit_scores(&scores).await.map_err(|error| {
            // A rejected score is worth retrying: the common causes are
            // transient (the trace has not landed yet, a 5xx) rather than the
            // payload being permanently invalid.
            TransportError::Retryable(error.to_string())
        })
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
    pub fn spawn(client: Arc<LangfuseClient>, config: BatchConfig) -> Self {
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
        if matches!(event, Event::Score(_)) {
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
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use {
        std::time::Duration,
        wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
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
        ScoreRecord::new(TraceId("trace-1".into()), name, ScoreValue::Numeric(1.0))
    }

    #[tokio::test]
    async fn scores_post_as_score_create_events() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(INGESTION_PATH))
            .respond_with(ResponseTemplate::new(207).set_body_json(serde_json::json!({
                "successes": [{ "id": "1", "status": 201 }], "errors": []
            })))
            .expect(1)
            .mount(&server)
            .await;

        let mut score = score("user-feedback");
        score.observation_id = Some(ObservationId("obs-1".into()));
        score.comment = Some("helpful".into());

        LangfuseClient::new(config(server.uri()))
            .submit_scores(&[score])
            .await
            .expect("score submission should succeed");
    }

    #[tokio::test]
    async fn partial_rejection_in_a_207_is_surfaced_as_an_error() {
        let server = MockServer::start().await;
        // A 2xx alone does not mean the data landed: the ingestion endpoint
        // reports per-event outcomes in the body.
        Mock::given(method("POST"))
            .and(path(INGESTION_PATH))
            .respond_with(ResponseTemplate::new(207).set_body_json(serde_json::json!({
                "successes": [],
                "errors": [{ "id": "1", "status": 400, "message": "invalid value" }]
            })))
            .mount(&server)
            .await;

        let error = LangfuseClient::new(config(server.uri()))
            .submit_scores(&[score("user-feedback")])
            .await
            .expect_err("partial failure should surface");

        assert!(error.to_string().contains("rejected 1 of 1"), "{error}");
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
            .and(path(INGESTION_PATH))
            .respond_with(ResponseTemplate::new(207).set_body_json(serde_json::json!({
                "successes": [], "errors": []
            })))
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
            .and(path(INGESTION_PATH))
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
}
