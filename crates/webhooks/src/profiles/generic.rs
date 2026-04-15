//! Generic source profile -- no provider-specific logic.

use axum::http::HeaderMap;

use crate::types::{AuthMode, EventCatalogEntry, NormalizedPayload};

use super::SourceProfile;

pub struct GenericProfile;

impl SourceProfile for GenericProfile {
    fn id(&self) -> &str {
        "generic"
    }

    fn display_name(&self) -> &str {
        "Generic"
    }

    fn default_auth_mode(&self) -> AuthMode {
        AuthMode::StaticHeader
    }

    fn event_catalog(&self) -> Vec<EventCatalogEntry> {
        vec![]
    }

    fn parse_event_type(&self, headers: &HeaderMap, _body: &[u8]) -> Option<String> {
        // Try common event-type headers
        for header in &["x-event-type", "x-webhook-event", "event-type"] {
            if let Some(val) = headers.get(*header).and_then(|v| v.to_str().ok()) {
                return Some(val.to_string());
            }
        }
        None
    }

    fn parse_delivery_key(&self, headers: &HeaderMap, _body: &[u8]) -> Option<String> {
        // Try common delivery ID headers.
        // Do NOT fall back to body hash — identical payloads are legitimate
        // for generic webhooks (e.g. repeated alerts, test deliveries).
        for header in &[
            "x-delivery-id",
            "x-request-id",
            "x-webhook-id",
            "idempotency-key",
        ] {
            if let Some(val) = headers.get(*header).and_then(|v| v.to_str().ok()) {
                return Some(val.to_string());
            }
        }
        None
    }

    fn entity_key(&self, _event_type: &str, _body: &serde_json::Value) -> Option<String> {
        None
    }

    fn normalize_payload(&self, event_type: &str, body: &serde_json::Value) -> NormalizedPayload {
        let pretty = serde_json::to_string_pretty(body).unwrap_or_default();
        let truncated = if pretty.len() > 8192 {
            format!(
                "{}\n\n... (truncated, {} bytes total. Use webhook_get_full_payload tool for full body.)",
                crate::normalize::truncate_str(&pretty, 8192),
                pretty.len()
            )
        } else {
            pretty
        };

        let event_line = if event_type.is_empty() {
            String::new()
        } else {
            format!("Event type: {event_type}\n\n")
        };

        NormalizedPayload {
            summary: format!("{event_line}JSON payload:\n{truncated}"),
            full_payload: body.clone(),
        }
    }

    fn setup_guide(&self) -> &str {
        "Configure the sending service to POST JSON to the webhook URL.\n\
         Use a static header for authentication (e.g., `X-Secret: your-token`)."
    }
}
