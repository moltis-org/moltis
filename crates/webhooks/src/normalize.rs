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

/// Render a template string with `{dot.notation}` variable substitution from a
/// JSON payload.
///
/// - `{pull_request.title}` → `payload["pull_request"]["title"]`
/// - `{__raw__}` → full payload as indented JSON (truncated at 4000 chars)
/// - Missing keys are left as literal `{key}` in the output.
/// - Nested objects/arrays are JSON-serialized (truncated at 2000 chars).
pub fn render_template(template: &str, payload: &serde_json::Value) -> String {
    let mut result = String::with_capacity(template.len() * 2);
    let mut chars = template.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '{' {
            // Collect until closing brace.
            let mut key = String::new();
            let mut found_close = false;
            for inner in chars.by_ref() {
                if inner == '}' {
                    found_close = true;
                    break;
                }
                key.push(inner);
            }
            if !found_close {
                // Malformed (no closing brace) — emit literal.
                result.push('{');
                result.push_str(&key);
                continue;
            }
            if key.is_empty() {
                // Empty braces `{}` — emit literal.
                result.push('{');
                result.push('}');
                continue;
            }

            if key == "__raw__" {
                let raw = serde_json::to_string_pretty(payload).unwrap_or_default();
                result.push_str(truncate_str(&raw, 4000));
                continue;
            }

            // Resolve dot-notation path.
            let resolved = resolve_path(payload, &key);
            match resolved {
                Some(serde_json::Value::String(s)) => result.push_str(s),
                Some(serde_json::Value::Number(n)) => result.push_str(&n.to_string()),
                Some(serde_json::Value::Bool(b)) => {
                    result.push_str(if *b {
                        "true"
                    } else {
                        "false"
                    });
                },
                Some(serde_json::Value::Null) => result.push_str("null"),
                Some(other) => {
                    let serialized = serde_json::to_string(&other).unwrap_or_default();
                    result.push_str(truncate_str(&serialized, 2000));
                },
                None => {
                    // Missing key — leave literal.
                    result.push('{');
                    result.push_str(&key);
                    result.push('}');
                },
            }
        } else {
            result.push(ch);
        }
    }

    result
}

/// Resolve a dot-notation path like `"pull_request.user.login"` against a JSON value.
fn resolve_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
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
