use serde::{Deserialize, Serialize};

// ── Webhook definition ──────────────────────────────────────────────────

/// A configured generic webhook endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Webhook {
    pub id: i64,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// High-entropy public identifier for the ingress URL.
    pub public_id: String,
    /// Agent preset to run for each delivery.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Optional model override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Extra text appended to the agent system prompt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt_suffix: Option<String>,
    /// Tool allow/deny policy overrides.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_policy: Option<ToolPolicy>,
    /// Authentication mode.
    pub auth_mode: AuthMode,
    /// Auth-mode-specific configuration (secrets, header names, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_config: Option<serde_json::Value>,
    /// Source profile identifier (e.g. "generic", "github", "stripe").
    #[serde(default = "default_generic")]
    pub source_profile: String,
    /// Source-profile-specific settings (API tokens, base URLs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_config: Option<serde_json::Value>,
    /// Event type filter.
    #[serde(default)]
    pub event_filter: EventFilter,
    /// Session creation strategy.
    #[serde(default)]
    pub session_mode: SessionMode,
    /// Key for named_session mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub named_session_key: Option<String>,
    /// CIDR allowlist (optional additional restriction).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_cidrs: Vec<String>,
    /// Maximum request body size in bytes.
    #[serde(default = "default_max_body_bytes")]
    pub max_body_bytes: usize,
    /// Rate limit: max deliveries per minute.
    #[serde(default = "default_rate_limit")]
    pub rate_limit_per_minute: u32,
    /// Denormalized delivery count for UI.
    #[serde(default)]
    pub delivery_count: u64,
    /// Denormalized last delivery timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_delivery_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Webhook {
    /// Return a copy with secrets redacted for API responses.
    ///
    /// Replaces `auth_config` and `source_config` with a `"[REDACTED]"`
    /// sentinel so the UI can see that a secret exists without exposing it.
    #[must_use]
    pub fn redacted(mut self) -> Self {
        if self.auth_config.is_some() {
            self.auth_config = Some(serde_json::json!("[REDACTED]"));
        }
        if self.source_config.is_some() {
            self.source_config = Some(serde_json::json!("[REDACTED]"));
        }
        self
    }
}

/// Input for creating a new webhook.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookCreate {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt_suffix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_policy: Option<ToolPolicy>,
    #[serde(default = "default_auth_mode")]
    pub auth_mode: AuthMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_config: Option<serde_json::Value>,
    #[serde(default = "default_generic")]
    pub source_profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_config: Option<serde_json::Value>,
    #[serde(default)]
    pub event_filter: EventFilter,
    #[serde(default)]
    pub session_mode: SessionMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub named_session_key: Option<String>,
    #[serde(default)]
    pub allowed_cidrs: Vec<String>,
    #[serde(default = "default_max_body_bytes")]
    pub max_body_bytes: usize,
    #[serde(default = "default_rate_limit")]
    pub rate_limit_per_minute: u32,
}

/// Input for patching a webhook.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt_suffix: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_policy: Option<Option<ToolPolicy>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_mode: Option<AuthMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_config: Option<Option<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_config: Option<Option<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_filter: Option<EventFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_mode: Option<SessionMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub named_session_key: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_cidrs: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_body_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_minute: Option<u32>,
}

// ── Enums ──────────────────────────────────────────────────────────────

/// Authentication mode for verifying inbound webhook requests.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    None,
    #[default]
    StaticHeader,
    Bearer,
    GithubHmacSha256,
    GitlabToken,
    StripeWebhookSignature,
    LinearWebhookSignature,
    PagerdutyV2Signature,
    SentryWebhookSignature,
}

/// Session creation strategy for deliveries.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    #[default]
    PerDelivery,
    PerEntity,
    NamedSession,
}

/// Tool allow/deny policy.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPolicy {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny: Vec<String>,
}

/// Event type filter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventFilter {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny: Vec<String>,
}

impl EventFilter {
    /// Returns `true` if the event type passes this filter.
    pub fn accepts(&self, event_type: &str) -> bool {
        if !self.allow.is_empty() && !self.allow.iter().any(|a| a == event_type) {
            return false;
        }
        !self.deny.iter().any(|d| d == event_type)
    }
}

// ── Delivery ───────────────────────────────────────────────────────────

/// Status of a webhook delivery.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    Received,
    Filtered,
    Deduplicated,
    Rejected,
    Queued,
    Processing,
    Completed,
    Failed,
}

/// A persisted webhook delivery record.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookDelivery {
    pub id: i64,
    pub webhook_id: i64,
    pub received_at: String,
    pub status: DeliveryStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers_json: Option<String>,
    #[serde(default)]
    pub body_size: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejection_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<i64>,
}

/// A response action logged for audit.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookResponseAction {
    pub id: i64,
    pub delivery_id: i64,
    pub tool_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_json: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_json: Option<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub created_at: String,
}

/// Normalized payload summary for the agent prompt.
#[derive(Debug, Clone)]
pub struct NormalizedPayload {
    /// Human-readable summary of the event.
    pub summary: String,
    /// Full payload available via tool.
    pub full_payload: serde_json::Value,
}

/// An entry in a source profile's event catalog.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventCatalogEntry {
    pub event_type: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub common_use_case: Option<String>,
}

/// Summary of a source profile for the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSummary {
    pub id: String,
    pub display_name: String,
    pub default_auth_mode: AuthMode,
    pub event_catalog: Vec<EventCatalogEntry>,
    pub has_response_tools: bool,
    pub setup_guide: String,
}

// ── Helpers ────────────────────────────────────────────────────────────

fn default_true() -> bool {
    true
}

fn default_generic() -> String {
    "generic".into()
}

fn default_auth_mode() -> AuthMode {
    AuthMode::StaticHeader
}

fn default_max_body_bytes() -> usize {
    1_048_576 // 1 MB
}

fn default_rate_limit() -> u32 {
    60
}

/// Generate a high-entropy public ID for a webhook endpoint.
pub fn generate_public_id() -> String {
    use rand::Rng;
    let bytes: [u8; 18] = rand::rng().random();
    format!("wh_{}", hex::encode(bytes))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_public_id() {
        let id = generate_public_id();
        assert!(id.starts_with("wh_"));
        assert_eq!(id.len(), 3 + 36); // "wh_" + 36 hex chars
    }

    #[test]
    fn test_event_filter_empty_accepts_all() {
        let filter = EventFilter::default();
        assert!(filter.accepts("anything"));
    }

    #[test]
    fn test_event_filter_allow_list() {
        let filter = EventFilter {
            allow: vec!["push".into(), "pull_request.opened".into()],
            deny: vec![],
        };
        assert!(filter.accepts("push"));
        assert!(filter.accepts("pull_request.opened"));
        assert!(!filter.accepts("issues.opened"));
    }

    #[test]
    fn test_event_filter_deny_list() {
        let filter = EventFilter {
            allow: vec![],
            deny: vec!["ping".into()],
        };
        assert!(filter.accepts("push"));
        assert!(!filter.accepts("ping"));
    }

    #[test]
    fn test_event_filter_allow_and_deny() {
        let filter = EventFilter {
            allow: vec!["push".into(), "pull_request.opened".into()],
            deny: vec!["push".into()],
        };
        assert!(!filter.accepts("push")); // deny wins
        assert!(filter.accepts("pull_request.opened"));
    }

    #[test]
    fn test_auth_mode_serde() {
        let mode = AuthMode::GithubHmacSha256;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, "\"github_hmac_sha256\"");
        let roundtrip: AuthMode = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip, mode);
    }

    #[test]
    fn test_delivery_status_serde() {
        let status = DeliveryStatus::Completed;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"completed\"");
    }

    #[test]
    fn test_session_mode_default() {
        let mode = SessionMode::default();
        assert_eq!(mode, SessionMode::PerDelivery);
    }

    #[test]
    fn test_webhook_create_roundtrip() {
        let create = WebhookCreate {
            name: "GitHub PR Hook".into(),
            description: Some("Reviews PRs".into()),
            agent_id: Some("code-reviewer".into()),
            model: None,
            system_prompt_suffix: None,
            tool_policy: None,
            auth_mode: AuthMode::GithubHmacSha256,
            auth_config: Some(serde_json::json!({ "secret": "test" })),
            source_profile: "github".into(),
            source_config: None,
            event_filter: EventFilter {
                allow: vec!["pull_request.opened".into()],
                deny: vec![],
            },
            session_mode: SessionMode::PerEntity,
            named_session_key: None,
            allowed_cidrs: vec![],
            max_body_bytes: 1_048_576,
            rate_limit_per_minute: 60,
        };
        let json = serde_json::to_value(&create).unwrap();
        let roundtrip: WebhookCreate = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip.name, "GitHub PR Hook");
        assert_eq!(roundtrip.auth_mode, AuthMode::GithubHmacSha256);
    }
}
