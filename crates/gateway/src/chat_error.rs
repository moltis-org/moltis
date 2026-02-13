//! Structured error parsing for chat error messages.
//!
//! Converts raw error strings from agent runners / LLM providers into
//! structured JSON payloads that the frontend can render directly.

use serde_json::Value;

/// Parse a raw error string into a structured error object with `type`, `icon`,
/// `title`, `detail`, and optionally `provider` and `resetsAt` fields.
pub fn parse_chat_error(raw: &str, provider_name: Option<&str>) -> Value {
    let mut error = try_parse_known_error(raw);

    if let Some(name) = provider_name
        && let Some(obj) = error.as_object_mut()
    {
        obj.insert("provider".into(), Value::String(name.to_string()));
    }

    error
}

fn try_parse_known_error(raw: &str) -> Value {
    let http_status = extract_http_status(raw);

    // Try to extract embedded JSON from the error string.
    if let Some(start) = raw.find('{')
        && let Ok(parsed) = serde_json::from_str::<Value>(&raw[start..])
    {
        let err_obj = parsed.get("error").unwrap_or(&parsed);

        // Usage limit
        if matches_type_or_message(err_obj, "usage_limit_reached", "usage limit") {
            let plan_type = err_obj
                .get("plan_type")
                .and_then(|v| v.as_str())
                .unwrap_or("current");
            let resets_at = extract_resets_at(err_obj);
            return build_error(
                "usage_limit_reached",
                "",
                "Usage limit reached",
                &format!("Your {} plan limit has been reached.", plan_type),
                resets_at,
            );
        }

        // Rate limit
        if matches_type_or_message(err_obj, "rate_limit_exceeded", "rate limit")
            || matches_type_or_message(err_obj, "rate_limit_exceeded", "quota exceeded")
            || http_status == Some(429)
        {
            let detail = err_obj
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Too many requests. Please wait a moment.");
            let resets_at = extract_resets_at(err_obj);
            return build_error(
                "rate_limit_exceeded",
                "\u{26A0}\u{FE0F}",
                "Rate limited",
                detail,
                resets_at,
            );
        }

        // Generic JSON error with a message field
        if let Some(msg) = extract_message(err_obj)
            && is_unsupported_model_message(msg)
        {
            return build_error(
                "unsupported_model",
                "\u{26A0}\u{FE0F}",
                "Model not supported",
                msg,
                None,
            );
        }

        // Generic JSON error with a message field
        if let Some(msg) = err_obj.get("message").and_then(|v| v.as_str()) {
            return build_error("api_error", "\u{26A0}\u{FE0F}", "Error", msg, None);
        }
    }

    // Check for HTTP status codes in the raw message.
    if let Some(code) = http_status {
        match code {
            401 | 403 => {
                return build_error(
                    "auth_error",
                    "\u{1F512}",
                    "Authentication error",
                    "Your session may have expired or credentials are invalid.",
                    None,
                );
            },
            429 => {
                return build_error(
                    "rate_limit_exceeded",
                    "",
                    "Rate limited",
                    "Too many requests. Please wait a moment and try again.",
                    None,
                );
            },
            code if code >= 500 => {
                return build_error(
                    "server_error",
                    "\u{1F6A8}",
                    "Server error",
                    "The upstream provider returned an error. Please try again later.",
                    None,
                );
            },
            _ => {},
        }
    }

    if is_unsupported_model_message(raw) {
        return build_error(
            "unsupported_model",
            "\u{26A0}\u{FE0F}",
            "Model not supported",
            raw,
            None,
        );
    }

    // Default: pass through raw message.
    build_error("unknown", "\u{26A0}\u{FE0F}", "Error", raw, None)
}

fn extract_message(obj: &Value) -> Option<&str> {
    obj.get("detail")
        .and_then(|v| v.as_str())
        .or_else(|| obj.get("message").and_then(|v| v.as_str()))
        .or_else(|| {
            obj.get("error")
                .and_then(|v| v.get("message"))
                .and_then(|v| v.as_str())
        })
        .or_else(|| {
            obj.get("error")
                .and_then(|v| v.get("detail"))
                .and_then(|v| v.as_str())
        })
}

fn is_unsupported_model_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();

    // Non-chat modality errors (audio, image, video models probed via chat
    // completion). These don't mention "model" or "supported" but clearly
    // indicate the model isn't meant for /v1/chat/completions.
    if lower.contains("input content or output modality contain audio")
        || lower.contains("requires audio")
    {
        return true;
    }

    let has_model = lower.contains("model");
    let unsupported = lower.contains("not supported")
        || lower.contains("unsupported")
        || lower.contains("not available")
        || lower.contains("not a chat model")
        || lower.contains("does not support chat")
        || lower.contains("only supported in v1/responses")
        || lower.contains("v1/chat/completions");
    has_model && unsupported
}

fn matches_type_or_message(obj: &Value, type_str: &str, message_substr: &str) -> bool {
    if let Some(t) = obj.get("type").and_then(|v| v.as_str())
        && t == type_str
    {
        return true;
    }
    if let Some(m) = obj.get("message").and_then(|v| v.as_str())
        && m.to_lowercase().contains(message_substr)
    {
        return true;
    }
    false
}

fn extract_resets_at(obj: &Value) -> Option<u64> {
    obj.get("resets_at").and_then(|v| v.as_u64())
}

fn extract_http_status(raw: &str) -> Option<u16> {
    // Match patterns like "HTTP 429", "status 503", "status: 401", "status=429"
    let patterns = ["HTTP ", "status= ", "status=", "status: ", "status "];
    for pat in &patterns {
        if let Some(idx) = raw.find(pat) {
            let after = &raw[idx + pat.len()..];
            let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(code) = digits.parse::<u16>() {
                return Some(code);
            }
        }
    }
    None
}

fn build_error(
    error_type: &str,
    icon: &str,
    title: &str,
    detail: &str,
    resets_at: Option<u64>,
) -> Value {
    let mut obj = serde_json::json!({
        "type": error_type,
        "icon": icon,
        "title": title,
        "detail": detail,
    });
    if let Some(ts) = resets_at {
        // Send as milliseconds for the frontend.
        if let Some(map) = obj.as_object_mut() {
            map.insert("resetsAt".into(), Value::Number((ts * 1000).into()));
        }
    }
    obj
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_usage_limit_json() {
        let raw = r#"Provider error: {"error":{"type":"usage_limit_reached","plan_type":"plus","resets_at":1769972721,"message":"Usage limit reached"}}"#;
        let result = parse_chat_error(raw, Some("openai-codex"));
        assert_eq!(result["type"], "usage_limit_reached");
        assert_eq!(result["title"], "Usage limit reached");
        assert!(result["detail"].as_str().unwrap().contains("plus"));
        assert_eq!(result["resetsAt"], 1769972721000u64);
        assert_eq!(result["provider"], "openai-codex");
    }

    #[test]
    fn test_rate_limit_json() {
        let raw = r#"{"type":"rate_limit_exceeded","message":"Rate limit exceeded, retry after 30s","resets_at":1700000000}"#;
        let result = parse_chat_error(raw, None);
        assert_eq!(result["type"], "rate_limit_exceeded");
        assert_eq!(result["title"], "Rate limited");
        assert_eq!(result["resetsAt"], 1700000000000u64);
    }

    #[test]
    fn test_http_401() {
        let raw = "Request failed with HTTP 401 Unauthorized";
        let result = parse_chat_error(raw, None);
        assert_eq!(result["type"], "auth_error");
        assert_eq!(result["icon"], "\u{1F512}");
    }

    #[test]
    fn test_http_429() {
        let raw = "Request failed with HTTP 429";
        let result = parse_chat_error(raw, None);
        assert_eq!(result["type"], "rate_limit_exceeded");
    }

    #[test]
    fn test_http_500() {
        let raw = "Request failed with HTTP 502 Bad Gateway";
        let result = parse_chat_error(raw, None);
        assert_eq!(result["type"], "server_error");
    }

    #[test]
    fn test_status_colon_format() {
        let raw = "upstream returned status: 503";
        let result = parse_chat_error(raw, None);
        assert_eq!(result["type"], "server_error");
    }

    #[test]
    fn test_status_equals_429_format() {
        let raw = "github-copilot API error status=429 Too Many Requests body=quota exceeded";
        let result = parse_chat_error(raw, Some("github-copilot"));
        assert_eq!(result["type"], "rate_limit_exceeded");
        assert_eq!(result["provider"], "github-copilot");
    }

    #[test]
    fn test_quota_exceeded_json_maps_to_rate_limit() {
        let raw = r#"provider error: {"error":{"message":"quota exceeded"}}"#;
        let result = parse_chat_error(raw, None);
        assert_eq!(result["type"], "rate_limit_exceeded");
    }

    #[test]
    fn test_generic_json_error() {
        let raw = r#"Something went wrong: {"message":"unexpected token"}"#;
        let result = parse_chat_error(raw, None);
        assert_eq!(result["type"], "api_error");
        assert_eq!(result["detail"], "unexpected token");
    }

    #[test]
    fn test_plain_text_fallback() {
        let raw = "Connection timed out";
        let result = parse_chat_error(raw, None);
        assert_eq!(result["type"], "unknown");
        assert_eq!(result["detail"], "Connection timed out");
    }

    #[test]
    fn test_openai_responses_only_message_maps_to_unsupported_model() {
        let raw = r#"OpenAI API error HTTP 404: {"error":{"message":"This model is only supported in v1/responses and not in v1/chat/completions."}}"#;
        let result = parse_chat_error(raw, Some("openai"));
        assert_eq!(result["type"], "unsupported_model");
        assert_eq!(result["provider"], "openai");
    }

    #[test]
    fn test_openai_not_chat_model_message_maps_to_unsupported_model() {
        let raw = r#"OpenAI API error HTTP 404: {"error":{"message":"This is not a chat model and thus not supported in the v1/chat/completions endpoint."}}"#;
        let result = parse_chat_error(raw, Some("openai"));
        assert_eq!(result["type"], "unsupported_model");
    }

    #[test]
    fn test_provider_included() {
        let raw = "Connection timed out";
        let result = parse_chat_error(raw, Some("anthropic"));
        assert_eq!(result["provider"], "anthropic");
    }

    #[test]
    fn test_no_resets_at_when_absent() {
        let raw = r#"{"type":"rate_limit_exceeded","message":"slow down"}"#;
        let result = parse_chat_error(raw, None);
        assert!(result.get("resetsAt").is_none());
    }

    #[test]
    fn test_usage_limit_message_substring() {
        let raw = r#"{"message":"You have hit the usage limit for your plan"}"#;
        let result = parse_chat_error(raw, None);
        assert_eq!(result["type"], "usage_limit_reached");
    }

    #[test]
    fn test_unsupported_model_from_detail() {
        let raw = r#"openai-codex API error HTTP 400: {"detail":"The 'gpt-5.3' model is not supported when using Codex with a ChatGPT account."}"#;
        let result = parse_chat_error(raw, Some("openai-codex"));
        assert_eq!(result["type"], "unsupported_model");
        assert_eq!(result["title"], "Model not supported");
        assert_eq!(result["provider"], "openai-codex");
    }

    #[test]
    fn test_unsupported_model_from_plain_text() {
        let raw = "The requested model is unsupported for this account";
        let result = parse_chat_error(raw, None);
        assert_eq!(result["type"], "unsupported_model");
    }

    #[test]
    fn test_audio_model_error_maps_to_unsupported_model() {
        // Audio models return this when probed via /v1/chat/completions.
        let raw = r#"OpenAI API error HTTP 400: {"error":{"message":"This model requires that either input content or output modality contain audio.","type":"invalid_request_error","param":"model","code":"invalid_value"}}"#;
        let result = parse_chat_error(raw, Some("openai"));
        assert_eq!(result["type"], "unsupported_model");
        assert_eq!(result["provider"], "openai");
    }
}
