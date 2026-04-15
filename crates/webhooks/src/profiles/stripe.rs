//! Stripe source profile.

use axum::http::HeaderMap;

use crate::types::{AuthMode, EventCatalogEntry, NormalizedPayload};

use super::SourceProfile;

pub struct StripeProfile;

impl SourceProfile for StripeProfile {
    fn id(&self) -> &str {
        "stripe"
    }

    fn display_name(&self) -> &str {
        "Stripe"
    }

    fn default_auth_mode(&self) -> AuthMode {
        AuthMode::StripeWebhookSignature
    }

    fn event_catalog(&self) -> Vec<EventCatalogEntry> {
        vec![
            EventCatalogEntry {
                event_type: "checkout.session.completed".into(),
                description: "Successful checkout".into(),
                common_use_case: Some("Fulfill order".into()),
            },
            EventCatalogEntry {
                event_type: "payment_intent.succeeded".into(),
                description: "Payment captured".into(),
                common_use_case: Some("Update order status".into()),
            },
            EventCatalogEntry {
                event_type: "payment_intent.payment_failed".into(),
                description: "Payment failed".into(),
                common_use_case: Some("Notify customer".into()),
            },
            EventCatalogEntry {
                event_type: "invoice.paid".into(),
                description: "Invoice paid".into(),
                common_use_case: Some("Activate subscription".into()),
            },
            EventCatalogEntry {
                event_type: "invoice.payment_failed".into(),
                description: "Invoice payment failed".into(),
                common_use_case: Some("Dunning".into()),
            },
            EventCatalogEntry {
                event_type: "customer.subscription.created".into(),
                description: "New subscription".into(),
                common_use_case: Some("Provision access".into()),
            },
            EventCatalogEntry {
                event_type: "customer.subscription.updated".into(),
                description: "Subscription changed".into(),
                common_use_case: Some("Adjust access".into()),
            },
            EventCatalogEntry {
                event_type: "customer.subscription.deleted".into(),
                description: "Subscription canceled".into(),
                common_use_case: Some("Revoke access".into()),
            },
            EventCatalogEntry {
                event_type: "charge.dispute.created".into(),
                description: "Chargeback opened".into(),
                common_use_case: Some("Alert, gather evidence".into()),
            },
        ]
    }

    fn parse_event_type(&self, _headers: &HeaderMap, body: &[u8]) -> Option<String> {
        serde_json::from_slice::<serde_json::Value>(body)
            .ok()
            .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(String::from))
    }

    fn parse_delivery_key(&self, _headers: &HeaderMap, body: &[u8]) -> Option<String> {
        serde_json::from_slice::<serde_json::Value>(body)
            .ok()
            .and_then(|v| v.get("id").and_then(|i| i.as_str()).map(String::from))
    }

    fn entity_key(&self, event_type: &str, body: &serde_json::Value) -> Option<String> {
        let data = body.get("data").and_then(|d| d.get("object"))?;

        if event_type.starts_with("customer.subscription") {
            let sub_id = data.get("id").and_then(|i| i.as_str())?;
            return Some(format!("stripe:{sub_id}"));
        }
        if event_type.starts_with("invoice") {
            let sub_id = data.get("subscription").and_then(|s| s.as_str())?;
            return Some(format!("stripe:{sub_id}"));
        }
        if event_type.starts_with("charge.dispute") {
            let charge_id = data.get("charge").and_then(|c| c.as_str())?;
            return Some(format!("stripe:dispute:{charge_id}"));
        }
        None
    }

    fn normalize_payload(&self, event_type: &str, body: &serde_json::Value) -> NormalizedPayload {
        let mut summary = format!("Stripe event: {event_type}\n\n");

        if let Some(id) = body.get("id").and_then(|i| i.as_str()) {
            summary.push_str(&format!("Event ID: {id}\n"));
        }
        if let Some(livemode) = body.get("livemode").and_then(|l| l.as_bool()) {
            summary.push_str(&format!("Livemode: {livemode}\n"));
        }

        if let Some(data) = body.get("data").and_then(|d| d.get("object")) {
            summary.push('\n');
            if let Some(customer) = data.get("customer").and_then(|c| c.as_str()) {
                summary.push_str(&format!("Customer: {customer}\n"));
            }
            if let Some(email) = data
                .get("customer_email")
                .or_else(|| data.get("email"))
                .and_then(|e| e.as_str())
            {
                summary.push_str(&format!("Email: {email}\n"));
            }
            if let Some(amount) = data
                .get("amount_total")
                .or_else(|| data.get("amount"))
                .and_then(|a| a.as_i64())
            {
                let currency = data
                    .get("currency")
                    .and_then(|c| c.as_str())
                    .unwrap_or("usd");
                let formatted = amount as f64 / 100.0;
                summary.push_str(&format!(
                    "Amount: {formatted:.2} {}\n",
                    currency.to_uppercase()
                ));
            }
            if let Some(status) = data.get("status").and_then(|s| s.as_str()) {
                summary.push_str(&format!("Status: {status}\n"));
            }
        }

        summary.push_str("\nFull payload available via webhook_get_full_payload tool.");

        NormalizedPayload {
            summary,
            full_payload: body.clone(),
        }
    }

    fn has_response_tools(&self) -> bool {
        true
    }

    fn setup_guide(&self) -> &str {
        "## Stripe Webhook Setup\n\n\
         1. Go to Stripe Dashboard -> Developers -> Webhooks -> Add endpoint\n\
         2. Endpoint URL: paste the webhook URL\n\
         3. Select events to listen to\n\
         4. Copy the signing secret (`whsec_...`) into Moltis\n\n\
         For response actions, create a restricted API key with only needed permissions.\n\n\
         Reference: https://docs.stripe.com/webhooks"
    }
}
