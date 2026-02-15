//! Typed parameter structs for session RPC methods.
//!
//! These structs are deserialized from the JSON-RPC `params` value at the
//! service boundary, eliminating ad-hoc `.get(...)` chains in business logic.

use serde::Deserialize;

use crate::share_store::ShareVisibility;

/// Params for `session.preview`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewParams {
    pub key: String,
    #[serde(default = "default_preview_limit")]
    pub limit: usize,
}

fn default_preview_limit() -> usize {
    5
}

/// Params for `session.resolve`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveParams {
    pub key: String,
}

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
    #[serde(default, deserialize_with = "double_option")]
    pub project_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub worktree_branch: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub sandbox_image: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub mcp_disabled: Option<Option<bool>>,
    #[serde(default, deserialize_with = "double_option")]
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

/// Params for `session.share.create`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareCreateParams {
    pub key: String,
    #[serde(
        default = "default_share_visibility",
        deserialize_with = "deserialize_share_visibility"
    )]
    pub visibility: ShareVisibility,
}

fn default_share_visibility() -> ShareVisibility {
    ShareVisibility::Public
}

fn deserialize_share_visibility<'de, D>(deserializer: D) -> Result<ShareVisibility, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt: Option<String> = Option::deserialize(deserializer)?;
    match opt {
        Some(s) => s
            .parse::<ShareVisibility>()
            .map_err(serde::de::Error::custom),
        None => Ok(ShareVisibility::Public),
    }
}

/// Params for `session.share.list`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareListParams {
    pub key: String,
}

/// Params for `session.share.revoke`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareRevokeParams {
    pub id: String,
}

/// Params for `session.reset`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetParams {
    pub key: String,
}

/// Params for `session.delete`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteParams {
    pub key: String,
    #[serde(default)]
    pub force: bool,
}

/// Params for `session.fork`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkParams {
    pub key: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub fork_point: Option<usize>,
}

/// Params for `session.branches`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchesParams {
    pub key: String,
}

/// Params for `session.search`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchParams {
    #[serde(default)]
    pub query: String,
    #[serde(default = "default_search_limit")]
    pub limit: usize,
}

fn default_search_limit() -> usize {
    20
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
    fn preview_params_with_defaults() {
        let p: PreviewParams = serde_json::from_value(json!({"key": "main"})).unwrap();
        assert_eq!(p.key, "main");
        assert_eq!(p.limit, 5);
    }

    #[test]
    fn preview_params_with_limit() {
        let p: PreviewParams = serde_json::from_value(json!({"key": "s1", "limit": 10})).unwrap();
        assert_eq!(p.key, "s1");
        assert_eq!(p.limit, 10);
    }

    #[test]
    fn preview_params_missing_key() {
        let result = serde_json::from_value::<PreviewParams>(json!({"limit": 5}));
        assert!(result.is_err());
    }

    #[test]
    fn resolve_params_basic() {
        let p: ResolveParams = serde_json::from_value(json!({"key": "session:abc"})).unwrap();
        assert_eq!(p.key, "session:abc");
    }

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
    fn share_create_default_visibility() {
        let p: ShareCreateParams = serde_json::from_value(json!({"key": "main"})).unwrap();
        assert!(matches!(p.visibility, ShareVisibility::Public));
    }

    #[test]
    fn share_create_private() {
        let p: ShareCreateParams =
            serde_json::from_value(json!({"key": "main", "visibility": "private"})).unwrap();
        assert!(matches!(p.visibility, ShareVisibility::Private));
    }

    #[test]
    fn delete_params_defaults() {
        let p: DeleteParams = serde_json::from_value(json!({"key": "s1"})).unwrap();
        assert_eq!(p.key, "s1");
        assert!(!p.force);
    }

    #[test]
    fn fork_params_minimal() {
        let p: ForkParams = serde_json::from_value(json!({"key": "main"})).unwrap();
        assert_eq!(p.key, "main");
        assert!(p.label.is_none());
        assert!(p.fork_point.is_none());
    }

    #[test]
    fn fork_params_with_fork_point() {
        let p: ForkParams =
            serde_json::from_value(json!({"key": "main", "forkPoint": 10, "label": "branch"}))
                .unwrap();
        assert_eq!(p.fork_point, Some(10));
        assert_eq!(p.label.as_deref(), Some("branch"));
    }

    #[test]
    fn search_params_defaults() {
        let p: SearchParams = serde_json::from_value(json!({})).unwrap();
        assert_eq!(p.query, "");
        assert_eq!(p.limit, 20);
    }

    #[test]
    fn search_params_with_values() {
        let p: SearchParams =
            serde_json::from_value(json!({"query": "hello", "limit": 5})).unwrap();
        assert_eq!(p.query, "hello");
        assert_eq!(p.limit, 5);
    }

    #[test]
    fn parse_params_helper() {
        let v = json!({"key": "main", "limit": 3});
        let p: PreviewParams = parse_params(v).unwrap();
        assert_eq!(p.key, "main");
        assert_eq!(p.limit, 3);
    }

    #[test]
    fn parse_params_error() {
        let v = json!({"limit": 3}); // missing key
        let err = parse_params::<PreviewParams>(v).unwrap_err();
        assert!(err.contains("key"));
    }
}
