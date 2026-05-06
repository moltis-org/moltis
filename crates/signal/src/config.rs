//! Signal channel account configuration.

use {
    moltis_channels::{
        config_view::ChannelConfigView,
        gating::{DmPolicy, GroupPolicy, MentionMode},
    },
    serde::{Deserialize, Serialize},
};

const DEFAULT_HTTP_URL: &str = "http://127.0.0.1:8080";
const DEFAULT_TEXT_CHUNK_LIMIT: usize = 4000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SignalAccountConfig {
    /// Whether this account should be started.
    pub enabled: bool,
    /// Signal account loaded by signal-cli, usually an E.164 phone number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// Optional Signal account UUID used for self-message filtering.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_uuid: Option<String>,
    /// Base URL of the signal-cli HTTP daemon.
    pub http_url: String,
    /// DM access policy.
    pub dm_policy: DmPolicy,
    /// Signal identifiers allowed to send DMs.
    pub allowlist: Vec<String>,
    /// Group access policy.
    pub group_policy: GroupPolicy,
    /// Signal group IDs allowed when `group_policy = "allowlist"`.
    pub group_allowlist: Vec<String>,
    /// Group mention activation mode.
    pub mention_mode: MentionMode,
    /// Ignore Signal story messages.
    pub ignore_stories: bool,
    /// Enable OTP self-approval for unknown DM senders.
    pub otp_self_approval: bool,
    /// Cooldown in seconds after failed OTP attempts.
    pub otp_cooldown_secs: u64,
    /// Maximum text characters per outbound Signal message.
    pub text_chunk_limit: usize,
    /// Default model ID for sessions created from this account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Provider name associated with the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
    /// Default agent id for this channel account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

impl Default for SignalAccountConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            account: None,
            account_uuid: None,
            http_url: DEFAULT_HTTP_URL.to_string(),
            dm_policy: DmPolicy::Allowlist,
            allowlist: Vec::new(),
            group_policy: GroupPolicy::Disabled,
            group_allowlist: Vec::new(),
            mention_mode: MentionMode::Mention,
            ignore_stories: true,
            otp_self_approval: true,
            otp_cooldown_secs: 300,
            text_chunk_limit: DEFAULT_TEXT_CHUNK_LIMIT,
            model: None,
            model_provider: None,
            agent_id: None,
        }
    }
}

impl SignalAccountConfig {
    pub fn normalize(mut self, account_id: &str) -> Self {
        if self.account.as_deref().is_none_or(str::is_empty) {
            self.account = Some(account_id.to_string());
        }
        self.http_url = normalize_http_url(&self.http_url);
        if self.text_chunk_limit == 0 {
            self.text_chunk_limit = DEFAULT_TEXT_CHUNK_LIMIT;
        }
        self
    }

    pub fn account(&self) -> Option<&str> {
        self.account
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
    }
}

fn normalize_http_url(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return DEFAULT_HTTP_URL.to_string();
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    }
}

impl ChannelConfigView for SignalAccountConfig {
    fn allowlist(&self) -> &[String] {
        &self.allowlist
    }

    fn group_allowlist(&self) -> &[String] {
        &self.group_allowlist
    }

    fn dm_policy(&self) -> DmPolicy {
        self.dm_policy.clone()
    }

    fn group_policy(&self) -> GroupPolicy {
        self.group_policy.clone()
    }

    fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    fn model_provider(&self) -> Option<&str> {
        self.model_provider.as_deref()
    }

    fn agent_id(&self) -> Option<&str> {
        self.agent_id.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_safe() {
        let cfg = SignalAccountConfig::default();
        assert_eq!(cfg.dm_policy, DmPolicy::Allowlist);
        assert_eq!(cfg.group_policy, GroupPolicy::Disabled);
        assert!(cfg.ignore_stories);
        assert!(cfg.otp_self_approval);
        assert_eq!(cfg.http_url, "http://127.0.0.1:8080");
    }

    #[test]
    fn normalize_fills_account_and_url_scheme() {
        let cfg = SignalAccountConfig {
            http_url: "127.0.0.1:9999/".to_string(),
            ..Default::default()
        }
        .normalize("+15551234567");

        assert_eq!(cfg.account(), Some("+15551234567"));
        assert_eq!(cfg.http_url, "http://127.0.0.1:9999");
    }
}
