//! Redaction of sensitive values before export.
//!
//! Instrumentation ships conversation content, tool arguments and tool results
//! to a third party, so redaction is a security control, not a nicety. Two
//! independent passes run over every payload:
//!
//! 1. **Key-based**: any object key containing a configured needle (`password`,
//!    `token`, ...) has its value replaced wholesale.
//! 2. **Shape-based**: string values that look like credentials (provider key
//!    prefixes, bearer tokens, PEM private keys) are replaced even when the key
//!    name is innocuous, because tool results are unstructured.
//!
//! Both passes are applied to keys and values at every depth. Redaction failing
//! open would be a silent data leak, so unknown structures are traversed rather
//! than skipped.

use serde_json::Value;

/// Replacement text substituted for redacted values.
pub const REDACTED: &str = "[REDACTED]";

/// Marker appended to truncated strings.
const TRUNCATION_SUFFIX: &str = "…[truncated]";

/// Default key needles. Matched case-insensitively as substrings.
pub const DEFAULT_KEY_NEEDLES: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "api_key",
    "apikey",
    "authorization",
    "credential",
    "private_key",
    "session_key",
    "access_key",
];

/// Credential-looking string prefixes, matched case-sensitively because real
/// key formats are case-sensitive and lowering would cause false positives.
const SECRET_PREFIXES: &[&str] = &[
    "sk-",
    "sk_live_",
    "sk_test_",
    "pk_live_",
    "rk_live_",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "github_pat_",
    "xoxb-",
    "xoxp-",
    "xoxa-",
    "xapp-",
    "AIza",
    "ya29.",
    "AKIA",
    "ASIA",
    "hf_",
    "lf_",
    "Bearer ",
    "bearer ",
    "-----BEGIN",
];

/// Policy controlling what leaves the process.
#[derive(Debug, Clone)]
pub struct RedactionPolicy {
    /// Object keys whose values are always replaced.
    key_needles: Vec<String>,
    /// Whether to also scan string values for credential shapes.
    scan_values: bool,
    /// Maximum length of any single string before truncation. `0` disables.
    max_string_len: usize,
}

impl Default for RedactionPolicy {
    fn default() -> Self {
        Self {
            key_needles: DEFAULT_KEY_NEEDLES
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            scan_values: true,
            max_string_len: 32_768,
        }
    }
}

impl RedactionPolicy {
    /// Build a policy from configured needles, merging in the defaults so an
    /// operator cannot accidentally disable baseline protection by supplying a
    /// narrower list.
    #[must_use]
    pub fn from_needles(needles: &[String]) -> Self {
        let mut key_needles: Vec<String> = DEFAULT_KEY_NEEDLES
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        for needle in needles {
            let lowered = needle.to_lowercase();
            if !lowered.is_empty() && !key_needles.contains(&lowered) {
                key_needles.push(lowered);
            }
        }
        Self {
            key_needles,
            ..Self::default()
        }
    }

    /// Set the per-string truncation limit. `0` disables truncation.
    #[must_use]
    pub const fn with_max_string_len(mut self, max: usize) -> Self {
        self.max_string_len = max;
        self
    }

    /// Enable or disable credential-shape scanning of string values.
    #[must_use]
    pub const fn with_value_scanning(mut self, scan: bool) -> Self {
        self.scan_values = scan;
        self
    }

    /// Whether `key` names something that must never be exported.
    #[must_use]
    pub fn is_sensitive_key(&self, key: &str) -> bool {
        let lowered = key.to_lowercase();
        self.key_needles
            .iter()
            .any(|n| lowered.contains(n.as_str()))
    }

    /// Whether `value` looks like a credential regardless of its key.
    #[must_use]
    pub fn looks_like_secret(&self, value: &str) -> bool {
        if !self.scan_values {
            return false;
        }
        let trimmed = value.trim();
        // Very short strings cannot be meaningful credentials and matching them
        // produces false positives on ordinary prose.
        if trimmed.len() < 8 {
            return false;
        }
        SECRET_PREFIXES.iter().any(|prefix| {
            trimmed.match_indices(prefix).any(|(index, _)| {
                index == 0
                    || trimmed[..index].chars().next_back().is_some_and(|c| {
                        c.is_whitespace() || matches!(c, '=' | ':' | '\'' | '"' | '`')
                    })
            })
        })
    }

    /// Redact a JSON value in place-equivalent fashion, returning a clean copy.
    #[must_use]
    pub fn redact(&self, value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let cleaned = map
                    .iter()
                    .map(|(key, val)| {
                        if self.is_sensitive_key(key) {
                            (key.clone(), Value::String(REDACTED.to_string()))
                        } else {
                            (key.clone(), self.redact(val))
                        }
                    })
                    .collect();
                Value::Object(cleaned)
            },
            Value::Array(items) => {
                Value::Array(items.iter().map(|item| self.redact(item)).collect())
            },
            Value::String(text) => Value::String(self.redact_str(text)),
            other => other.clone(),
        }
    }

    /// Redact and truncate a single string.
    #[must_use]
    pub fn redact_str(&self, text: &str) -> String {
        if self.looks_like_secret(text) {
            return REDACTED.to_string();
        }
        self.truncate(text)
    }

    /// Truncate `text` to the configured limit on a character boundary.
    fn truncate(&self, text: &str) -> String {
        if self.max_string_len == 0 || text.len() <= self.max_string_len {
            return text.to_string();
        }
        // `floor_char_boundary` is unstable, so walk back to a boundary by hand
        // rather than slicing blindly and panicking on multi-byte input.
        let mut end = self.max_string_len;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}{TRUNCATION_SUFFIX}", &text[..end])
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use {super::*, serde_json::json};

    #[test]
    fn redacts_sensitive_keys_at_any_depth() {
        let policy = RedactionPolicy::default();
        let input = json!({
            "safe": "keep me",
            "nested": {
                "api_key": "abc123",
                "deeper": [{ "password": "hunter2" }]
            }
        });

        let out = policy.redact(&input);

        assert_eq!(out["safe"], json!("keep me"));
        assert_eq!(out["nested"]["api_key"], json!(REDACTED));
        assert_eq!(out["nested"]["deeper"][0]["password"], json!(REDACTED));
    }

    #[test]
    fn key_matching_is_case_insensitive_and_substring_based() {
        let policy = RedactionPolicy::default();
        let input = json!({
            "ANTHROPIC_API_KEY": "x",
            "userToken": "y",
            "Authorization": "z",
        });

        let out = policy.redact(&input);

        assert_eq!(out["ANTHROPIC_API_KEY"], json!(REDACTED));
        assert_eq!(out["userToken"], json!(REDACTED));
        assert_eq!(out["Authorization"], json!(REDACTED));
    }

    #[test]
    fn redacts_credential_shaped_values_under_innocuous_keys() {
        let policy = RedactionPolicy::default();
        // Tool results are unstructured: a leaked key often lands under a key
        // name that no needle list would ever catch.
        let input = json!({
            "stdout": "sk-ant-api03-abcdefghijklmnop",
            "header": "Bearer eyJhbGciOiJIUzI1NiJ9",
            "note": "the deploy finished successfully",
        });

        let out = policy.redact(&input);

        assert_eq!(out["stdout"], json!(REDACTED));
        assert_eq!(out["header"], json!(REDACTED));
        assert_eq!(out["note"], json!("the deploy finished successfully"));
    }

    #[test]
    fn redacts_credentials_embedded_in_shell_output_and_commands() {
        let policy = RedactionPolicy::default();
        for value in [
            "OPENAI_API_KEY=sk-abcdefghijklmnop",
            "curl -H 'Authorization: Bearer abcdefghijklmnop' https://example.com",
            "request failed: xoxb-example",
        ] {
            assert_eq!(policy.redact(&json!(value)), json!(REDACTED), "{value}");
        }
        assert!(!policy.looks_like_secret("ordinary task-sk-result text"));
    }

    #[test]
    fn redacts_pem_private_key_blocks() {
        let policy = RedactionPolicy::default();
        let input = json!("-----BEGIN RSA PRIVATE KEY-----\nMIIEow==\n");
        assert_eq!(policy.redact(&input), json!(REDACTED));
    }

    #[test]
    fn short_strings_are_not_treated_as_secrets() {
        let policy = RedactionPolicy::default();
        // "sk-1" is too short to be a real key; matching it would mangle prose.
        assert!(!policy.looks_like_secret("sk-1"));
        assert!(policy.looks_like_secret("sk-ant-api03-abcdefgh"));
    }

    #[test]
    fn value_scanning_can_be_disabled() {
        let policy = RedactionPolicy::default().with_value_scanning(false);
        assert!(!policy.looks_like_secret("sk-ant-api03-abcdefghijkl"));
    }

    #[test]
    fn custom_needles_extend_rather_than_replace_defaults() {
        let policy = RedactionPolicy::from_needles(&["internal_id".to_string()]);

        assert!(policy.is_sensitive_key("internal_id"));
        // The baseline must survive a narrower operator-supplied list.
        assert!(policy.is_sensitive_key("password"));
        assert!(policy.is_sensitive_key("api_key"));
    }

    #[test]
    fn empty_needles_are_ignored() {
        let policy = RedactionPolicy::from_needles(&[String::new()]);
        // An empty needle is a substring of every key and would redact
        // everything, so it must be dropped.
        assert!(!policy.is_sensitive_key("harmless"));
    }

    #[test]
    fn truncates_long_strings_on_a_char_boundary() {
        let policy = RedactionPolicy::default().with_max_string_len(8);
        // Multi-byte characters straddling the limit must not panic.
        let out = policy.redact_str("ααααααααααα");

        assert!(out.ends_with(TRUNCATION_SUFFIX));
        assert!(out.len() < 40);
    }

    #[test]
    fn truncation_can_be_disabled() {
        let policy = RedactionPolicy::default().with_max_string_len(0);
        let long = "a".repeat(100_000);
        assert_eq!(policy.redact_str(&long).len(), 100_000);
    }

    #[test]
    fn non_string_scalars_pass_through_unchanged() {
        let policy = RedactionPolicy::default();
        let input = json!({ "n": 42, "b": true, "nil": null, "f": 1.5 });
        assert_eq!(policy.redact(&input), input);
    }
}
