//! Typed request params and response payloads of the RPC methods that native
//! clients use (`sessions.list`, `chat.send`, `models.list`, approvals).
//!
//! Response types deserialize leniently (`#[serde(default)]`) so a client keeps
//! working when the daemon adds fields.

use serde::{Deserialize, Serialize};

/// RPC method names used by native clients.
pub mod methods {
    pub const SUBSCRIBE: &str = "subscribe";
    pub const SESSIONS_LIST: &str = "sessions.list";
    pub const SESSIONS_DELETE: &str = "sessions.delete";
    pub const CHAT_SEND: &str = "chat.send";
    pub const CHAT_ABORT: &str = "chat.abort";
    pub const CHAT_HISTORY: &str = "chat.history";
    pub const MODELS_LIST: &str = "models.list";
    pub const EXEC_APPROVAL_RESOLVE: &str = "exec.approval.resolve";
    pub const STATUS: &str = "status";
    pub const CHAT_CONTEXT: &str = "chat.context";
    pub const AGENT_IDENTITY_GET: &str = "agent.identity.get";
}

/// Event names native clients subscribe to.
pub mod events {
    pub const CHAT: &str = crate::chat_event::CHAT_EVENT;
    pub const SESSION: &str = "session";
    pub const TICK: &str = "tick";
    pub const EXEC_APPROVAL_REQUESTED: &str = "exec.approval.requested";
    pub const EXEC_APPROVAL_RESOLVED: &str = "exec.approval.resolved";
    pub const MODELS_UPDATED: &str = "models.updated";
}

/// Params of `subscribe` / `unsubscribe`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SubscribeParams {
    pub events: Vec<String>,
}

/// One entry of the `sessions.list` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionEntry {
    pub key: String,
    pub label: Option<String>,
    pub model: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub message_count: u32,
    pub last_seen_message_count: u32,
    pub preview: Option<String>,
    pub archived: bool,
    pub parent_session_key: Option<String>,
    pub agent_id: Option<String>,
    /// Whether a run is currently streaming in this session.
    pub replying: bool,
    pub version: u64,
}

impl SessionEntry {
    /// The label, else the preview, else the key.
    #[must_use]
    pub fn title(&self) -> &str {
        self.label
            .as_deref()
            .filter(|label| !label.is_empty())
            .or(self
                .preview
                .as_deref()
                .filter(|preview| !preview.is_empty()))
            .unwrap_or(&self.key)
    }

    /// Whether messages arrived since the session was last viewed.
    #[must_use]
    pub fn has_unread(&self) -> bool {
        self.message_count > self.last_seen_message_count
    }
}

/// Kind of a `session` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventKind {
    Patched,
    #[serde(other)]
    Other,
}

/// Payload of the `session` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEvent {
    pub kind: SessionEventKind,
    #[serde(default)]
    pub session_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
}

/// Params of `chat.send`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSendParams {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Client sequence number, echoed back on events of this turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
}

/// Params of `chat.history`, `chat.abort` and other per-session methods.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionKeyParams {
    pub session_key: String,
}

/// One entry of the `models.list` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelEntry {
    pub id: String,
    pub provider: String,
    pub display_name: Option<String>,
    pub supports_tools: bool,
    pub supports_vision: bool,
    pub supports_reasoning: bool,
    pub preferred: bool,
    pub recommended: bool,
}

impl ModelEntry {
    #[must_use]
    pub fn name(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.id)
    }
}

/// Payload of `exec.approval.requested`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExecApprovalRequest {
    pub request_id: String,
    pub command: String,
    pub session_key: Option<String>,
}

/// A user's answer to an approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approved,
    Denied,
}

/// Params of `exec.approval.resolve`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecApprovalResolveParams {
    pub request_id: String,
    pub decision: ApprovalDecision,
}

/// The part of the `chat.context` response native clients read.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatContext {
    pub token_usage: ContextTokenUsage,
}

/// Token accounting of a session, from `chat.context`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ContextTokenUsage {
    /// Tokens of the latest request (input and output).
    pub current_total: u64,
    pub estimated_next_input_tokens: u64,
    /// The model's context window, in tokens.
    pub context_window: u64,
}

impl ContextTokenUsage {
    /// Tokens the conversation occupies in the context window.
    #[must_use]
    pub fn used(&self) -> u64 {
        if self.current_total > 0 {
            self.current_total
        } else {
            self.estimated_next_input_tokens
        }
    }

    /// Share of the context window in use, `0.0..=1.0`, when the window is
    /// known.
    #[must_use]
    pub fn fraction(&self) -> Option<f32> {
        (self.context_window > 0)
            .then(|| (self.used() as f64 / self.context_window as f64).clamp(0.0, 1.0) as f32)
    }
}

/// The part of the `agent.identity.get` response native clients read.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentIdentity {
    /// The agent's name ("moltis" unless renamed).
    pub name: String,
    pub emoji: Option<String>,
    /// The owner's name from the user profile.
    pub user_name: Option<String>,
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, serde_json::json};

    #[test]
    fn session_entry_parses_gateway_shape() {
        let entry: SessionEntry = serde_json::from_value(json!({
            "id": "abc", "key": "main", "label": null, "model": "gpt",
            "createdAt": 1, "updatedAt": 2, "messageCount": 4, "lastSeenMessageCount": 2,
            "preview": "hi there", "archived": false, "agent_id": "a", "agentId": "a",
            "replying": true, "version": 7, "sandbox_enabled": null
        }))
        .unwrap();
        assert_eq!(entry.key, "main");
        assert_eq!(entry.title(), "hi there");
        assert!(entry.has_unread());
        assert!(entry.replying);
        assert_eq!(entry.agent_id.as_deref(), Some("a"));
    }

    #[test]
    fn session_title_falls_back_to_key() {
        let entry = SessionEntry {
            key: "k".into(),
            label: Some(String::new()),
            ..SessionEntry::default()
        };
        assert_eq!(entry.title(), "k");
        let labelled = SessionEntry {
            label: Some("Plan".into()),
            ..entry
        };
        assert_eq!(labelled.title(), "Plan");
    }

    #[test]
    fn chat_send_skips_empty_options() {
        let value = serde_json::to_value(ChatSendParams {
            text: "hi".into(),
            session_key: Some("main".into()),
            ..ChatSendParams::default()
        })
        .unwrap();
        assert_eq!(value, json!({"text": "hi", "sessionKey": "main"}));
    }

    #[test]
    fn approval_resolve_serializes_decision() {
        let value = serde_json::to_value(ExecApprovalResolveParams {
            request_id: "r".into(),
            decision: ApprovalDecision::Denied,
        })
        .unwrap();
        assert_eq!(value, json!({"requestId": "r", "decision": "denied"}));
    }

    #[test]
    fn session_event_unknown_kind() {
        let event: SessionEvent =
            serde_json::from_value(json!({"kind": "created", "sessionKey": "x"})).unwrap();
        assert_eq!(event.kind, SessionEventKind::Other);
        let patched: SessionEvent = serde_json::from_value(
            json!({"kind": "patched", "sessionKey": "x", "label": "T", "version": 2}),
        )
        .unwrap();
        assert_eq!(patched.label.as_deref(), Some("T"));
    }

    #[test]
    fn model_name_prefers_display_name() {
        let model: ModelEntry = serde_json::from_value(json!({
            "id": "openai::gpt", "provider": "openai", "displayName": "GPT",
            "supportsTools": true, "createdAt": 5
        }))
        .unwrap();
        assert_eq!(model.name(), "GPT");
        assert!(model.supports_tools);
    }

    #[test]
    fn context_usage_prefers_current_total() {
        let context: ChatContext = serde_json::from_value(serde_json::json!({
            "sessionKey": "main",
            "tokenUsage": {"currentTotal": 38_000, "estimatedNextInputTokens": 1, "contextWindow": 200_000, "total": 9}
        }))
        .unwrap();
        let usage = context.token_usage;
        assert_eq!(usage.used(), 38_000);
        assert_eq!(usage.fraction(), Some(0.19));

        let fallback = ContextTokenUsage {
            estimated_next_input_tokens: 500,
            context_window: 400,
            ..ContextTokenUsage::default()
        };
        assert_eq!(fallback.used(), 500);
        assert_eq!(fallback.fraction(), Some(1.0));
        assert_eq!(ContextTokenUsage::default().fraction(), None);
    }

    #[test]
    fn identity_reads_snake_case_fields() {
        let identity: AgentIdentity = serde_json::from_value(serde_json::json!({
            "name": "moltis", "emoji": null, "user_name": "Fabien", "soul": "…"
        }))
        .unwrap();
        assert_eq!(identity.user_name.as_deref(), Some("Fabien"));
        assert_eq!(identity.name, "moltis");
    }
}
