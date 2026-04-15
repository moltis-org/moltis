//! Background worker for processing webhook deliveries.

use {
    std::{future::Future, pin::Pin, sync::Arc},
    tokio::sync::mpsc,
    tracing::{error, info, instrument, warn},
};

use crate::{
    profiles::SourceProfile,
    store::{DeliveryUpdate, WebhookStore},
    types::DeliveryStatus,
};

/// Result of processing a webhook delivery via chat.send_sync.
pub struct ProcessResult {
    pub output: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub session_key: String,
}

/// Callback type for executing an agent turn.
pub type ExecuteFn = Arc<
    dyn Fn(ExecuteRequest) -> Pin<Box<dyn Future<Output = anyhow::Result<ProcessResult>> + Send>>
        + Send
        + Sync,
>;

/// Request to execute a webhook delivery.
pub struct ExecuteRequest {
    pub webhook_id: i64,
    pub delivery_id: i64,
    pub session_key: String,
    pub agent_id: Option<String>,
    pub model: Option<String>,
    pub tool_policy: Option<crate::types::ToolPolicy>,
    pub message: String,
}

/// Background worker that processes queued webhook deliveries.
pub struct WebhookWorker {
    rx: mpsc::Receiver<i64>,
    store: Arc<dyn WebhookStore>,
    execute_fn: ExecuteFn,
}

impl WebhookWorker {
    pub fn new(
        rx: mpsc::Receiver<i64>,
        store: Arc<dyn WebhookStore>,
        execute_fn: ExecuteFn,
    ) -> Self {
        Self {
            rx,
            store,
            execute_fn,
        }
    }

    /// Run the worker loop, processing deliveries from the channel.
    ///
    /// On startup, drains any deliveries left in `received` or `queued` state
    /// from a previous run before listening for new work on the channel.
    #[instrument(skip_all, name = "webhook_worker")]
    pub async fn run(mut self) {
        info!("webhook worker started");

        // Crash recovery: re-process deliveries stuck from a previous run.
        match self.store.list_unprocessed_deliveries().await {
            Ok(ids) => {
                if !ids.is_empty() {
                    info!(
                        count = ids.len(),
                        "re-processing unfinished deliveries from previous run"
                    );
                }
                for id in ids {
                    if let Err(e) = self.process_delivery(id).await {
                        error!(delivery_id = id, error = %e, "failed to reprocess existing delivery");
                    }
                }
            },
            Err(e) => error!(error = %e, "failed to load unprocessed deliveries at startup"),
        }

        while let Some(delivery_id) = self.rx.recv().await {
            if let Err(e) = self.process_delivery(delivery_id).await {
                error!(delivery_id, error = %e, "webhook delivery processing failed");
            }
        }
        info!("webhook worker stopped");
    }

    #[instrument(skip(self), fields(delivery_id))]
    async fn process_delivery(&self, delivery_id: i64) -> anyhow::Result<()> {
        // Mark as processing
        self.store
            .update_delivery_status(delivery_id, DeliveryStatus::Processing, DeliveryUpdate {
                started_at: Some(now_iso()),
                ..Default::default()
            })
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        // Load delivery and webhook
        let delivery = self
            .store
            .get_delivery(delivery_id)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let webhook = self
            .store
            .get_webhook(delivery.webhook_id)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        // Load body for normalization
        let body_bytes = self
            .store
            .get_delivery_body(delivery_id)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .unwrap_or_default();

        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap_or_default();

        // Get the source profile for normalization
        let profile_registry = crate::profiles::ProfileRegistry::new();
        let profile = profile_registry.get(&webhook.source_profile);

        // Normalize payload
        let event_type = delivery.event_type.as_deref().unwrap_or("unknown");
        let normalized = match profile {
            Some(p) => p.normalize_payload(event_type, &body),
            None => crate::profiles::generic::GenericProfile.normalize_payload(event_type, &body),
        };

        // Build the delivery message
        let message = crate::normalize::build_delivery_message(
            &webhook,
            delivery.event_type.as_deref(),
            delivery.delivery_key.as_deref(),
            &delivery.received_at,
            &normalized.summary,
        );

        let session_key = build_session_key(&webhook, delivery_id, delivery.entity_key.as_deref());

        let start = std::time::Instant::now();

        // Execute via callback
        let result = (self.execute_fn)(ExecuteRequest {
            webhook_id: webhook.id,
            delivery_id,
            session_key: session_key.clone(),
            agent_id: webhook.agent_id.clone(),
            model: webhook.model.clone(),
            tool_policy: webhook.tool_policy.clone(),
            message,
        })
        .await;

        let duration_ms = start.elapsed().as_millis() as i64;

        match result {
            Ok(process_result) => {
                self.store
                    .update_delivery_status(
                        delivery_id,
                        DeliveryStatus::Completed,
                        DeliveryUpdate {
                            session_key: Some(process_result.session_key),
                            finished_at: Some(now_iso()),
                            duration_ms: Some(duration_ms),
                            input_tokens: process_result.input_tokens,
                            output_tokens: process_result.output_tokens,
                            ..Default::default()
                        },
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
            },
            Err(e) => {
                warn!(delivery_id, error = %e, "webhook execution failed");
                self.store
                    .update_delivery_status(delivery_id, DeliveryStatus::Failed, DeliveryUpdate {
                        run_error: Some(e.to_string()),
                        finished_at: Some(now_iso()),
                        duration_ms: Some(duration_ms),
                        ..Default::default()
                    })
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
            },
        }

        Ok(())
    }
}

fn build_session_key(
    webhook: &crate::types::Webhook,
    delivery_id: i64,
    entity_key: Option<&str>,
) -> String {
    match webhook.session_mode {
        crate::types::SessionMode::PerDelivery => {
            format!("webhook:{}:{}", webhook.public_id, delivery_id)
        },
        crate::types::SessionMode::PerEntity => match entity_key {
            Some(entity_key) => format!("webhook:{}:{entity_key}", webhook.public_id),
            None => format!("webhook:{}:{}", webhook.public_id, delivery_id),
        },
        crate::types::SessionMode::NamedSession => webhook
            .named_session_key
            .clone()
            .unwrap_or_else(|| format!("webhook:{}", webhook.public_id)),
    }
}

fn now_iso() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}

#[cfg(test)]
mod tests {
    use crate::types::{AuthMode, EventFilter, SessionMode, Webhook};

    use super::build_session_key;

    fn webhook(session_mode: SessionMode) -> Webhook {
        Webhook {
            id: 1,
            name: "test".into(),
            description: None,
            enabled: true,
            public_id: "wh_public".into(),
            agent_id: Some("code-reviewer".into()),
            model: None,
            system_prompt_suffix: None,
            tool_policy: None,
            auth_mode: AuthMode::StaticHeader,
            auth_config: None,
            source_profile: "github".into(),
            source_config: None,
            event_filter: EventFilter::default(),
            session_mode,
            named_session_key: Some("named".into()),
            allowed_cidrs: Vec::new(),
            max_body_bytes: 1024,
            rate_limit_per_minute: 60,
            delivery_count: 0,
            last_delivery_at: None,
            created_at: "2026-04-07T00:00:00Z".into(),
            updated_at: "2026-04-07T00:00:00Z".into(),
        }
    }

    #[test]
    fn per_entity_session_keys_are_namespaced_by_webhook() {
        let key = build_session_key(
            &webhook(SessionMode::PerEntity),
            42,
            Some("github:repo:pr:7"),
        );
        assert_eq!(key, "webhook:wh_public:github:repo:pr:7");
    }

    #[test]
    fn per_entity_without_entity_key_falls_back_to_delivery_session() {
        let key = build_session_key(&webhook(SessionMode::PerEntity), 42, None);
        assert_eq!(key, "webhook:wh_public:42");
    }

    #[test]
    fn named_sessions_keep_explicit_name() {
        let key = build_session_key(&webhook(SessionMode::NamedSession), 42, None);
        assert_eq!(key, "named");
    }
}
