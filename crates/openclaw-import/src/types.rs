//! Concrete types for OpenClaw configuration files.
//!
//! These map the known shape of `~/.openclaw/openclaw.json` (JSON5) and
//! related files into typed Rust structs. Uses `#[serde(default)]`
//! liberally for forward-compatibility with unknown OpenClaw versions.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ── openclaw.json root ───────────────────────────────────────────────────────

/// Root of `~/.openclaw/openclaw.json`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct OpenClawConfig {
    pub agents: OpenClawAgentsConfig,
    pub channels: OpenClawChannelsConfig,
    pub memory: OpenClawMemoryConfig,
    pub ui: OpenClawUiConfig,
}

// ── agents ───────────────────────────────────────────────────────────────────

/// `agents` section of the OpenClaw config.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct OpenClawAgentsConfig {
    pub defaults: OpenClawAgentDefaults,
    #[serde(default)]
    pub list: Vec<OpenClawAgentEntry>,
}

/// `agents.defaults` — global agent defaults.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct OpenClawAgentDefaults {
    pub model: OpenClawModelConfig,
    #[serde(rename = "userTimezone")]
    pub user_timezone: Option<String>,
    #[serde(rename = "userName")]
    pub user_name: Option<String>,
    pub workspace: Option<String>,
}

/// Model selection: `{primary, fallbacks}`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct OpenClawModelConfig {
    pub primary: Option<String>,
    #[serde(default)]
    pub fallbacks: Vec<String>,
}

/// A single agent in `agents.list[]`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct OpenClawAgentEntry {
    pub id: String,
    #[serde(default)]
    pub default: bool,
    pub name: Option<String>,
    pub workspace: Option<String>,
    pub model: Option<OpenClawAgentModelConfig>,
}

/// Agent-level model override — can be a string or `{primary, fallbacks}`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum OpenClawAgentModelConfig {
    Simple(String),
    Full(OpenClawModelConfig),
}

// ── channels ─────────────────────────────────────────────────────────────────

/// `channels` section.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct OpenClawChannelsConfig {
    pub telegram: Option<OpenClawTelegramConfig>,
    /// Unsupported channels — kept as raw values for TODO reporting.
    pub whatsapp: Option<serde_json::Value>,
    pub discord: Option<OpenClawDiscordConfig>,
    pub slack: Option<serde_json::Value>,
    pub signal: Option<serde_json::Value>,
    pub imessage: Option<serde_json::Value>,
}

/// Telegram channel config. OpenClaw supports both flat and `accounts` map forms.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct OpenClawTelegramConfig {
    /// Named accounts: `telegram.accounts.<id>`.
    pub accounts: Option<HashMap<String, OpenClawTelegramAccount>>,
    /// Flat top-level fields (legacy single-account form).
    #[serde(rename = "botToken")]
    pub bot_token: Option<String>,
    #[serde(rename = "dmPolicy")]
    pub dm_policy: Option<String>,
    #[serde(rename = "allowFrom", default)]
    pub allow_from: Vec<serde_json::Value>,
}

/// A single Telegram account config.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct OpenClawTelegramAccount {
    #[serde(rename = "botToken")]
    pub bot_token: Option<String>,
    #[serde(rename = "tokenFile")]
    pub token_file: Option<String>,
    #[serde(rename = "dmPolicy")]
    pub dm_policy: Option<String>,
    #[serde(rename = "allowFrom", default)]
    pub allow_from: Vec<serde_json::Value>,
    pub enabled: Option<bool>,
    pub name: Option<String>,
}

/// Discord channel config. OpenClaw supports both flat and `accounts` map forms.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct OpenClawDiscordConfig {
    /// Named accounts: `discord.accounts.<id>`.
    pub accounts: Option<HashMap<String, OpenClawDiscordAccount>>,
    /// Flat top-level fields (legacy single-account form).
    pub token: Option<String>,
    #[serde(rename = "tokenFile")]
    pub token_file: Option<String>,
    #[serde(rename = "dmPolicy")]
    pub dm_policy: Option<String>,
    #[serde(rename = "groupPolicy")]
    pub group_policy: Option<String>,
    #[serde(rename = "mentionMode")]
    pub mention_mode: Option<String>,
    #[serde(rename = "allowFrom", default)]
    pub allow_from: Vec<serde_json::Value>,
    #[serde(rename = "guildAllowlist", default)]
    pub guild_allowlist: Vec<serde_json::Value>,
    pub enabled: Option<bool>,
    pub name: Option<String>,
}

/// A single Discord account config.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct OpenClawDiscordAccount {
    pub token: Option<String>,
    #[serde(rename = "tokenFile")]
    pub token_file: Option<String>,
    #[serde(rename = "dmPolicy")]
    pub dm_policy: Option<String>,
    #[serde(rename = "groupPolicy")]
    pub group_policy: Option<String>,
    #[serde(rename = "mentionMode")]
    pub mention_mode: Option<String>,
    #[serde(rename = "allowFrom", default)]
    pub allow_from: Vec<serde_json::Value>,
    #[serde(rename = "guildAllowlist", default)]
    pub guild_allowlist: Vec<serde_json::Value>,
    pub enabled: Option<bool>,
    pub name: Option<String>,
}

// ── memory ───────────────────────────────────────────────────────────────────

/// `memory` section.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct OpenClawMemoryConfig {
    pub backend: Option<String>,
}

// ── ui ───────────────────────────────────────────────────────────────────────

/// `ui` section.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct OpenClawUiConfig {
    pub assistant: Option<OpenClawAssistantConfig>,
}

/// `ui.assistant` — agent display identity.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct OpenClawAssistantConfig {
    pub name: Option<String>,
    pub avatar: Option<String>,
    pub theme: Option<String>,
    pub creature: Option<String>,
    pub vibe: Option<String>,
}

// ── auth-profiles.json ───────────────────────────────────────────────────────

/// Root of `agents/<id>/agent/auth-profiles.json`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct OpenClawAuthProfileStore {
    pub version: u32,
    #[serde(default)]
    pub profiles: HashMap<String, OpenClawAuthProfile>,
    pub order: Option<HashMap<String, Vec<String>>>,
}

/// A single auth profile credential entry.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OpenClawAuthProfile {
    ApiKey {
        provider: String,
        key: Option<String>,
        email: Option<String>,
    },
    Token {
        provider: String,
        token: Option<String>,
        expires: Option<u64>,
        email: Option<String>,
    },
    Oauth {
        provider: String,
        #[serde(rename = "clientId")]
        client_id: Option<String>,
        email: Option<String>,
        #[serde(rename = "accessToken", alias = "access")]
        access_token: Option<String>,
        #[serde(rename = "refreshToken", alias = "refresh")]
        refresh_token: Option<String>,
        expires: Option<u64>,
        #[serde(rename = "accountId")]
        account_id: Option<String>,
    },
}

impl OpenClawAuthProfile {
    /// The provider name for this profile.
    pub fn provider(&self) -> &str {
        match self {
            Self::ApiKey { provider, .. }
            | Self::Token { provider, .. }
            | Self::Oauth { provider, .. } => provider,
        }
    }

    /// Extract the API key (only for `api_key` profiles).
    pub fn api_key(&self) -> Option<&str> {
        match self {
            Self::ApiKey { key, .. } => key.as_deref(),
            _ => None,
        }
    }
}

// ── Session JSONL records ────────────────────────────────────────────────────

/// A single line in an OpenClaw session JSONL file.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum OpenClawSessionRecord {
    Message {
        #[serde(default)]
        timestamp: Option<OpenClawTimestamp>,
        message: OpenClawMessage,
    },
    SessionMeta {
        #[serde(rename = "agentId")]
        agent_id: Option<String>,
        #[serde(flatten)]
        extra: HashMap<String, serde_json::Value>,
    },
    Custom {
        #[serde(rename = "customType")]
        custom_type: Option<String>,
        data: Option<serde_json::Value>,
    },
}

/// An OpenClaw chat message within a session record.
#[derive(Debug, Clone, Deserialize)]
pub struct OpenClawMessage {
    pub role: OpenClawRole,
    pub content: Option<OpenClawContent>,
    #[serde(rename = "toolUseId")]
    pub tool_use_id: Option<String>,
    pub name: Option<String>,
    #[serde(default)]
    pub timestamp: Option<OpenClawTimestamp>,
}

/// A timestamp in OpenClaw records.
///
/// OpenClaw emits mixed formats:
/// - integer unix milliseconds (`1769582795764`)
/// - floating-point seconds or milliseconds
/// - ISO-8601 strings (`2026-01-28T06:46:35.758Z`)
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum OpenClawTimestamp {
    Integer(i64),
    Float(f64),
    Iso(String),
}

impl OpenClawTimestamp {
    /// Convert to unix milliseconds.
    #[must_use]
    pub fn to_millis(&self) -> Option<u64> {
        match self {
            Self::Integer(v) => normalize_epoch(*v as f64),
            Self::Float(v) => normalize_epoch(*v),
            Self::Iso(s) => parse_iso_to_millis(s),
        }
    }
}

fn normalize_epoch(value: f64) -> Option<u64> {
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    // Treat very large values as milliseconds, otherwise seconds.
    let ms = if value >= 1_000_000_000_000.0 {
        value
    } else {
        value * 1000.0
    };
    Some(ms as u64)
}

fn parse_iso_to_millis(s: &str) -> Option<u64> {
    use time::format_description::well_known::Rfc3339;
    let dt = time::OffsetDateTime::parse(s, &Rfc3339).ok()?;
    let nanos = dt.unix_timestamp_nanos();
    if nanos < 0 {
        return None;
    }
    Some((nanos / 1_000_000) as u64)
}

/// OpenClaw message roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OpenClawRole {
    User,
    Assistant,
    Tool,
    ToolResult,
    System,
}

/// Message content — either a string or an array of content blocks.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum OpenClawContent {
    Text(String),
    Blocks(Vec<serde_json::Value>),
}

impl OpenClawContent {
    /// Extract as plain text, joining blocks if necessary.
    pub fn as_text(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Blocks(blocks) => blocks
                .iter()
                .filter_map(|b| {
                    b.get("text")
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string())
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

// ── Session index (sessions.json) ────────────────────────────────────────────

/// A single entry in the `sessions.json` index file.
///
/// The index maps logical session keys (e.g. `agent:main:main`) to metadata
/// including timestamps and origin information.
#[derive(Debug, Deserialize)]
pub struct OpenClawSessionIndexEntry {
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
    #[serde(rename = "updatedAt")]
    pub updated_at: Option<u64>,
    pub origin: Option<OpenClawSessionOrigin>,
}

/// Origin metadata for a session in `sessions.json`.
#[derive(Debug, Deserialize)]
pub struct OpenClawSessionOrigin {
    pub label: Option<String>,
    pub provider: Option<String>,
    #[serde(rename = "chatType")]
    pub chat_type: Option<String>,
}

// ── MCP servers (OpenClaw format) ────────────────────────────────────────────

/// A single MCP server entry in OpenClaw's config.
///
/// OpenClaw typically configures MCP servers in `mcp-servers.json` or within
/// `openclaw.json` plugins — the format matches the Claude Code MCP standard.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenClawMcpServer {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_config() {
        let json = r#"{ "agents": { "defaults": { "model": { "primary": "anthropic/claude-opus-4-6" } } } }"#;
        let config: OpenClawConfig = json5::from_str(json).unwrap_or_default();
        assert_eq!(
            config.agents.defaults.model.primary.as_deref(),
            Some("anthropic/claude-opus-4-6")
        );
    }

    #[test]
    fn parse_config_with_comments() {
        let json5_str = r#"{
            // This is a JSON5 comment
            "agents": {
                "defaults": {
                    "model": {
                        "primary": "openai/gpt-4o",
                        "fallbacks": ["anthropic/claude-sonnet-4-20250514"],
                    },
                },
            },
        }"#;
        let config: OpenClawConfig = json5::from_str(json5_str).unwrap_or_default();
        assert_eq!(
            config.agents.defaults.model.primary.as_deref(),
            Some("openai/gpt-4o")
        );
        assert_eq!(config.agents.defaults.model.fallbacks.len(), 1);
    }

    #[test]
    fn parse_telegram_flat_config() {
        let json = r#"{
            "channels": {
                "telegram": {
                    "botToken": "123:ABC",
                    "dmPolicy": "pairing",
                    "allowFrom": [12345, "tg:67890"]
                }
            }
        }"#;
        let config: OpenClawConfig = json5::from_str(json).unwrap_or_default();
        let tg = config.channels.telegram.as_ref().unwrap();
        assert_eq!(tg.bot_token.as_deref(), Some("123:ABC"));
        assert_eq!(tg.dm_policy.as_deref(), Some("pairing"));
        assert_eq!(tg.allow_from.len(), 2);
    }

    #[test]
    fn parse_telegram_accounts_config() {
        let json = r#"{
            "channels": {
                "telegram": {
                    "accounts": {
                        "default": {
                            "botToken": "456:DEF",
                            "allowFrom": [111],
                            "dmPolicy": "allowlist"
                        }
                    }
                }
            }
        }"#;
        let config: OpenClawConfig = json5::from_str(json).unwrap_or_default();
        let tg = config.channels.telegram.as_ref().unwrap();
        let acct = tg.accounts.as_ref().unwrap().get("default").unwrap();
        assert_eq!(acct.bot_token.as_deref(), Some("456:DEF"));
        assert_eq!(acct.dm_policy.as_deref(), Some("allowlist"));
    }

    #[test]
    fn parse_discord_flat_config() {
        let json = r#"{
            "channels": {
                "discord": {
                    "token": "Bot abc",
                    "dmPolicy": "pairing",
                    "groupPolicy": "allowlist",
                    "mentionMode": "always",
                    "allowFrom": ["123", 456],
                    "guildAllowlist": ["789"]
                }
            }
        }"#;
        let config: OpenClawConfig = json5::from_str(json).unwrap_or_default();
        let dc = config.channels.discord.as_ref().unwrap();
        assert_eq!(dc.token.as_deref(), Some("Bot abc"));
        assert_eq!(dc.dm_policy.as_deref(), Some("pairing"));
        assert_eq!(dc.group_policy.as_deref(), Some("allowlist"));
        assert_eq!(dc.mention_mode.as_deref(), Some("always"));
        assert_eq!(dc.allow_from.len(), 2);
        assert_eq!(dc.guild_allowlist.len(), 1);
    }

    #[test]
    fn parse_discord_accounts_config() {
        let json = r#"{
            "channels": {
                "discord": {
                    "accounts": {
                        "bot1": {
                            "token": "Bot xyz",
                            "dmPolicy": "open",
                            "groupPolicy": "disabled",
                            "mentionMode": "mention",
                            "allowFrom": ["111"],
                            "guildAllowlist": ["222"]
                        }
                    }
                }
            }
        }"#;
        let config: OpenClawConfig = json5::from_str(json).unwrap_or_default();
        let dc = config.channels.discord.as_ref().unwrap();
        let acct = dc.accounts.as_ref().unwrap().get("bot1").unwrap();
        assert_eq!(acct.token.as_deref(), Some("Bot xyz"));
        assert_eq!(acct.dm_policy.as_deref(), Some("open"));
        assert_eq!(acct.group_policy.as_deref(), Some("disabled"));
        assert_eq!(acct.mention_mode.as_deref(), Some("mention"));
    }

    #[test]
    fn parse_auth_profile_api_key() {
        let json = r#"{
            "version": 1,
            "profiles": {
                "anthropic-main": {
                    "type": "api_key",
                    "provider": "anthropic",
                    "key": "sk-ant-123"
                }
            }
        }"#;
        let store: OpenClawAuthProfileStore = serde_json::from_str(json).unwrap_or_default();
        let profile = store.profiles.get("anthropic-main").unwrap();
        assert_eq!(profile.provider(), "anthropic");
        assert_eq!(profile.api_key(), Some("sk-ant-123"));
    }

    #[test]
    fn parse_session_record_message() {
        let line = r#"{"type":"message","message":{"role":"user","content":"Hello"}}"#;
        let record: OpenClawSessionRecord = serde_json::from_str(line).unwrap();
        match record {
            OpenClawSessionRecord::Message { message, .. } => {
                assert_eq!(message.role, OpenClawRole::User);
                assert_eq!(message.content.unwrap().as_text(), "Hello");
            },
            _ => panic!("expected Message record"),
        }
    }

    #[test]
    fn parse_session_record_meta() {
        let line = r#"{"type":"session-meta","agentId":"main"}"#;
        let record: OpenClawSessionRecord = serde_json::from_str(line).unwrap();
        match record {
            OpenClawSessionRecord::SessionMeta { agent_id, .. } => {
                assert_eq!(agent_id.as_deref(), Some("main"));
            },
            _ => panic!("expected SessionMeta record"),
        }
    }

    #[test]
    fn parse_session_record_custom() {
        let line = r#"{"type":"custom","customType":"model-snapshot","data":{"model":"gpt-4o"}}"#;
        let record: OpenClawSessionRecord = serde_json::from_str(line).unwrap();
        match record {
            OpenClawSessionRecord::Custom { custom_type, .. } => {
                assert_eq!(custom_type.as_deref(), Some("model-snapshot"));
            },
            _ => panic!("expected Custom record"),
        }
    }

    #[test]
    fn parse_agent_model_simple() {
        let json = r#"{"id":"main","model":"anthropic/claude-opus-4-6"}"#;
        let entry: OpenClawAgentEntry = serde_json::from_str(json).unwrap_or_default();
        match entry.model {
            Some(OpenClawAgentModelConfig::Simple(s)) => {
                assert_eq!(s, "anthropic/claude-opus-4-6");
            },
            _ => panic!("expected Simple model config"),
        }
    }

    #[test]
    fn unsupported_channels_detected() {
        let json = r#"{
            "channels": {
                "whatsapp": { "enabled": true },
                "discord": { "token": "abc" }
            }
        }"#;
        let config: OpenClawConfig = json5::from_str(json).unwrap_or_default();
        assert!(config.channels.whatsapp.is_some());
        assert!(config.channels.discord.is_some());
        assert_eq!(
            config
                .channels
                .discord
                .as_ref()
                .and_then(|d| d.token.as_deref()),
            Some("abc")
        );
        assert!(config.channels.telegram.is_none());
    }

    #[test]
    fn parse_session_index_entry() {
        let json = r#"{
            "agent:main:main": {
                "sessionId": "d8da3601-1234-4abc-9def-aabbccddeeff",
                "updatedAt": 1770000000000,
                "origin": {
                    "label": "Fabien (@fabienpenso) id:377114917",
                    "provider": "telegram",
                    "chatType": "direct"
                }
            }
        }"#;
        let entries: HashMap<String, OpenClawSessionIndexEntry> =
            serde_json::from_str(json).unwrap();
        let entry = entries.get("agent:main:main").unwrap();
        assert_eq!(
            entry.session_id.as_deref(),
            Some("d8da3601-1234-4abc-9def-aabbccddeeff")
        );
        assert_eq!(entry.updated_at, Some(1770000000000));
        let origin = entry.origin.as_ref().unwrap();
        assert_eq!(
            origin.label.as_deref(),
            Some("Fabien (@fabienpenso) id:377114917")
        );
        assert_eq!(origin.provider.as_deref(), Some("telegram"));
        assert_eq!(origin.chat_type.as_deref(), Some("direct"));
    }

    #[test]
    fn parse_session_index_entry_minimal() {
        let json = r#"{"updatedAt": 1000}"#;
        let entry: OpenClawSessionIndexEntry = serde_json::from_str(json).unwrap();
        assert!(entry.session_id.is_none());
        assert!(entry.origin.is_none());
        assert_eq!(entry.updated_at, Some(1000));
    }

    #[test]
    fn content_text_extraction() {
        let text = OpenClawContent::Text("hello".to_string());
        assert_eq!(text.as_text(), "hello");

        let blocks = OpenClawContent::Blocks(vec![
            serde_json::json!({"type": "text", "text": "hello "}),
            serde_json::json!({"type": "text", "text": "world"}),
        ]);
        assert_eq!(blocks.as_text(), "hello \nworld");
    }
}
