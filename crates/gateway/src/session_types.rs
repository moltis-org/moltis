//! Typed parameter structs for complex session RPC methods.
//!
//! Only methods with non-trivial parameter shapes (multi-field with defaults,
//! null-vs-absent semantics, precedence logic) get dedicated structs here.
//! Simple key-only handlers use inline `.get(...)` directly.

use serde::Deserialize;

/// Params for `session.patch`.
///
/// All fields except `key` are optional — only provided fields are updated.
///
/// Fields that can be cleared (set to null) use `Option<Option<String>>`:
/// - outer `None` → field was absent from the request (no-op)
/// - `Some(None)` → field was explicitly `null` (clear it)
/// - `Some(Some(v))` → field was set to value `v`
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchParams {
    pub key: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default, deserialize_with = "double_option", alias = "project_id")]
    pub project_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option", alias = "worktree_branch")]
    pub worktree_branch: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option", alias = "sandbox_image")]
    pub sandbox_image: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option", alias = "mcp_disabled")]
    pub mcp_disabled: Option<Option<bool>>,
    #[serde(default, deserialize_with = "double_option", alias = "sandbox_enabled")]
    pub sandbox_enabled: Option<Option<bool>>,
}

/// Deserialize a field as `Some(inner)` when present (even if null),
/// vs `None` when absent (via `#[serde(default)]`).
fn double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Ok(Some(Option::<T>::deserialize(deserializer)?))
}

/// Params for `session.voice_generate`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceGenerateParams {
    pub key: String,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub message_index: Option<usize>,
    #[serde(default)]
    pub history_index: Option<usize>,
}

impl VoiceGenerateParams {
    /// Resolve the target specification. `run_id` takes precedence.
    pub fn target(&self) -> Result<VoiceTarget, &'static str> {
        if let Some(ref id) = self.run_id {
            let trimmed = id.trim();
            if !trimmed.is_empty() {
                return Ok(VoiceTarget::ByRunId(trimmed.to_string()));
            }
        }
        if let Some(idx) = self.message_index.or(self.history_index) {
            return Ok(VoiceTarget::ByMessageIndex(idx));
        }
        Err("missing 'messageIndex' or 'runId' parameter")
    }
}

/// How to locate the target assistant message for voice generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceTarget {
    /// Locate by agent run ID (stable across inserted tool_result messages).
    ByRunId(String),
    /// Locate by raw message index in the history array.
    ByMessageIndex(usize),
}

/// Parse a `serde_json::Value` into a typed param struct, mapping
/// deserialization errors to the service error format.
pub fn parse_params<T: serde::de::DeserializeOwned>(
    params: serde_json::Value,
) -> Result<T, String> {
    serde_json::from_value(params).map_err(|e| e.to_string())
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, serde_json::json};

    #[test]
    fn patch_params_minimal() {
        let p: PatchParams = serde_json::from_value(json!({"key": "main"})).unwrap();
        assert_eq!(p.key, "main");
        assert!(p.label.is_none());
        assert!(p.model.is_none());
        assert!(p.project_id.is_none());
        assert!(p.sandbox_enabled.is_none());
    }

    #[test]
    fn patch_params_with_fields() {
        let p: PatchParams = serde_json::from_value(json!({
            "key": "main",
            "label": "My Chat",
            "model": "gpt-4o",
            "sandboxEnabled": true,
            "mcpDisabled": false,
        }))
        .unwrap();
        assert_eq!(p.label.as_deref(), Some("My Chat"));
        assert_eq!(p.model.as_deref(), Some("gpt-4o"));
        assert_eq!(p.sandbox_enabled, Some(Some(true)));
        assert_eq!(p.mcp_disabled, Some(Some(false)));
    }

    #[test]
    fn patch_params_sandbox_enabled_false() {
        let p: PatchParams = serde_json::from_value(json!({
            "key": "main",
            "sandboxEnabled": false,
        }))
        .unwrap();
        assert_eq!(p.sandbox_enabled, Some(Some(false)));
    }

    #[test]
    fn patch_params_sandbox_enabled_null_clears() {
        let p: PatchParams = serde_json::from_value(json!({
            "key": "main",
            "sandboxEnabled": null,
        }))
        .unwrap();
        assert_eq!(p.sandbox_enabled, Some(None));
    }

    #[test]
    fn patch_params_accepts_legacy_snake_case_fields() {
        let p: PatchParams = serde_json::from_value(json!({
            "key": "main",
            "project_id": "proj-1",
            "worktree_branch": "feature/abc",
            "sandbox_image": "custom:latest",
            "sandbox_enabled": false,
            "mcp_disabled": true,
        }))
        .unwrap();
        assert_eq!(p.project_id, Some(Some("proj-1".to_string())));
        assert_eq!(p.worktree_branch, Some(Some("feature/abc".to_string())));
        assert_eq!(p.sandbox_image, Some(Some("custom:latest".to_string())));
        assert_eq!(p.sandbox_enabled, Some(Some(false)));
        assert_eq!(p.mcp_disabled, Some(Some(true)));
    }

    #[test]
    fn patch_params_null_project_id() {
        let p: PatchParams = serde_json::from_value(json!({
            "key": "main",
            "projectId": null,
        }))
        .unwrap();
        // Outer Some = field was present; inner None = value was null (clear).
        assert!(matches!(p.project_id, Some(None)));
    }

    #[test]
    fn patch_params_set_project_id() {
        let p: PatchParams = serde_json::from_value(json!({
            "key": "main",
            "projectId": "proj-1",
        }))
        .unwrap();
        assert_eq!(p.project_id, Some(Some("proj-1".to_string())));
    }

    #[test]
    fn voice_generate_run_id_precedence() {
        let p: VoiceGenerateParams = serde_json::from_value(json!({
            "key": "main",
            "runId": "run-abc",
            "messageIndex": 5,
        }))
        .unwrap();
        assert_eq!(p.target().unwrap(), VoiceTarget::ByRunId("run-abc".into()));
    }

    #[test]
    fn voice_generate_index_fallback() {
        let p: VoiceGenerateParams = serde_json::from_value(json!({
            "key": "main",
            "messageIndex": 3,
        }))
        .unwrap();
        assert_eq!(p.target().unwrap(), VoiceTarget::ByMessageIndex(3));
    }

    #[test]
    fn voice_generate_history_index_fallback() {
        let p: VoiceGenerateParams = serde_json::from_value(json!({
            "key": "main",
            "historyIndex": 7,
        }))
        .unwrap();
        assert_eq!(p.target().unwrap(), VoiceTarget::ByMessageIndex(7));
    }

    #[test]
    fn voice_generate_no_target() {
        let p: VoiceGenerateParams = serde_json::from_value(json!({"key": "main"})).unwrap();
        assert!(p.target().is_err());
    }

    #[test]
    fn voice_generate_blank_run_id_falls_back_to_index() {
        let p: VoiceGenerateParams = serde_json::from_value(json!({
            "key": "main",
            "runId": "  ",
            "messageIndex": 2,
        }))
        .unwrap();
        assert_eq!(p.target().unwrap(), VoiceTarget::ByMessageIndex(2));
    }

    #[test]
    fn parse_params_helper() {
        let v = json!({"key": "main"});
        let p: PatchParams = parse_params(v).unwrap();
        assert_eq!(p.key, "main");
    }

    #[test]
    fn parse_params_error() {
        let v = json!({"not_key": true});
        let err = parse_params::<PatchParams>(v).unwrap_err();
        assert!(err.contains("key"));
    }
}
