//! Shared normalization helpers for webhook delivery messages.

use crate::types::Webhook;

/// Truncate a string to at most `max_bytes` without splitting a UTF-8 codepoint.
pub fn truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    // Walk backwards from max_bytes to find a char boundary.
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Build the full delivery message injected into the agent chat session.
pub fn build_delivery_message(
    webhook: &Webhook,
    event_type: Option<&str>,
    delivery_key: Option<&str>,
    received_at: &str,
    normalized_summary: &str,
) -> String {
    let mut msg = String::with_capacity(2048);

    msg.push_str("Webhook delivery received.\n\n");
    msg.push_str(&format!(
        "Webhook: {} ({})\n",
        webhook.name, webhook.public_id
    ));
    msg.push_str(&format!("Source: {}\n", webhook.source_profile));
    if let Some(et) = event_type {
        msg.push_str(&format!("Event: {et}\n"));
    }
    if let Some(dk) = delivery_key {
        msg.push_str(&format!("Delivery: {dk}\n"));
    }
    msg.push_str(&format!("Received: {received_at}\n"));

    msg.push_str("\n---\n\n");
    msg.push_str(normalized_summary);

    if let Some(ref suffix) = webhook.system_prompt_suffix {
        msg.push_str("\n\n---\n\n");
        msg.push_str(suffix);
    }

    msg
}

/// Extract selected headers from a request for logging.
/// Only includes headers that are useful for debugging; skips auth secrets.
pub fn extract_safe_headers(headers: &axum::http::HeaderMap) -> serde_json::Value {
    let safe_prefixes = [
        "x-github-",
        "x-gitlab-",
        "x-stripe-",
        "x-pagerduty-",
        "x-linear-",
        "x-sentry-",
        "x-request-id",
        "x-delivery-id",
        "x-event-type",
        "content-type",
        "content-length",
        "user-agent",
        "idempotency-key",
    ];
    let deny = [
        "authorization",
        "x-gitlab-token",
        "cookie",
        // HMAC signatures — not secrets themselves but provide a signature
        // oracle if logged alongside the stored body.
        "x-hub-signature-256",
        "x-pagerduty-signature",
        "stripe-signature",
        "linear-signature",
        "sentry-hook-signature",
    ];

    let mut map = serde_json::Map::new();
    for (name, value) in headers {
        let name_lower = name.as_str().to_lowercase();
        if deny.iter().any(|d| name_lower == *d) {
            continue;
        }
        if safe_prefixes
            .iter()
            .any(|prefix| name_lower.starts_with(prefix) || name_lower == *prefix)
            && let Ok(v) = value.to_str()
        {
            map.insert(name_lower, serde_json::Value::String(v.into()));
        }
    }
    serde_json::Value::Object(map)
}
