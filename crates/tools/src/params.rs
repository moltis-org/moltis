//! Shared helpers for extracting typed parameters from `serde_json::Value`.
//!
//! These reduce boilerplate in `AgentTool::execute` implementations that
//! manually pull fields from a JSON object.

use serde_json::Value;

use crate::Error;

/// Extract a trimmed, non-empty `&str` from a JSON object field.
///
/// Returns `None` when the key is absent, null, not a string, empty,
/// or whitespace-only.
pub fn str_param<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

/// Try multiple keys in order and return the first non-empty string match.
pub fn str_param_any<'a>(params: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| str_param(params, key))
}

/// Like [`str_param`] but returns a `crate::Error` when missing.
pub fn require_str<'a>(params: &'a Value, key: &str) -> crate::Result<&'a str> {
    str_param(params, key)
        .ok_or_else(|| Error::message(format!("missing required parameter: {key}")))
}

/// Extract a boolean, defaulting to `default` when absent.
pub fn bool_param(params: &Value, key: &str, default: bool) -> bool {
    params.get(key).and_then(Value::as_bool).unwrap_or(default)
}

/// Extract a `u64`, defaulting to `default` when absent.
pub fn u64_param(params: &Value, key: &str, default: u64) -> u64 {
    params.get(key).and_then(Value::as_u64).unwrap_or(default)
}

/// Extract an owned `String` from the first matching key.
pub fn owned_str_param(params: &Value, keys: &[&str]) -> Option<String> {
    str_param_any(params, keys).map(String::from)
}

#[cfg(test)]
mod tests {
    use {super::*, serde_json::json};

    #[test]
    fn str_param_extracts_trimmed_value() {
        let p = json!({"name": "  hello  "});
        assert_eq!(str_param(&p, "name"), Some("hello"));
    }

    #[test]
    fn str_param_returns_none_for_missing_key() {
        let p = json!({});
        assert_eq!(str_param(&p, "name"), None);
    }

    #[test]
    fn str_param_returns_none_for_empty_string() {
        let p = json!({"name": ""});
        assert_eq!(str_param(&p, "name"), None);
    }

    #[test]
    fn str_param_returns_none_for_whitespace_only() {
        let p = json!({"name": "   "});
        assert_eq!(str_param(&p, "name"), None);
    }

    #[test]
    fn str_param_any_finds_first_match() {
        let p = json!({"chatId": "42"});
        assert_eq!(str_param_any(&p, &["chat_id", "chatId"]), Some("42"));
    }

    #[test]
    fn require_str_errors_when_missing() {
        let p = json!({});
        assert!(require_str(&p, "key").is_err());
    }

    #[test]
    fn bool_param_returns_value_or_default() {
        let p = json!({"force": true});
        assert!(bool_param(&p, "force", false));
        assert!(!bool_param(&p, "missing", false));
    }

    #[test]
    fn u64_param_returns_value_or_default() {
        let p = json!({"limit": 50});
        assert_eq!(u64_param(&p, "limit", 20), 50);
        assert_eq!(u64_param(&p, "missing", 20), 20);
    }
}
