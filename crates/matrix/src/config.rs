use std::collections::HashMap;

use {
    moltis_channels::{
        config_view::ChannelConfigView,
        gating::{DmPolicy, GroupPolicy, MentionMode},
    },
    moltis_common::secret_serde,
    secrecy::Secret,
    serde::{Deserialize, Serialize, ser::SerializeStruct},
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StreamMode {
    #[default]
    EditInPlace,
    Off,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutoJoinPolicy {
    Always,
    Allowlist,
    #[default]
    Off,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MatrixAccountConfig {
    pub homeserver_url: String,

    #[serde(serialize_with = "secret_serde::serialize_secret")]
    pub access_token: Secret<String>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "secret_serde::serialize_option_secret"
    )]
    pub password: Option<Secret<String>>,

    pub user_id: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_display_name: Option<String>,

    pub encryption: bool,

    pub allow_private_network: bool,

    pub dm_policy: DmPolicy,

    pub group_policy: GroupPolicy,

    pub mention_mode: MentionMode,

    pub allowlist: Vec<String>,

    pub room_allowlist: Vec<String>,

    pub stream_mode: StreamMode,

    pub edit_throttle_ms: u64,

    pub stream_notify_on_complete: bool,

    pub stream_min_initial_chars: usize,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,

    pub otp_self_approval: bool,

    pub otp_cooldown_secs: u64,

    pub auto_join: AutoJoinPolicy,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub auto_join_allowlist: Vec<String>,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub channel_overrides: HashMap<String, ChannelOverride>,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub user_overrides: HashMap<String, UserOverride>,
}

impl std::fmt::Debug for MatrixAccountConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MatrixAccountConfig")
            .field("homeserver_url", &self.homeserver_url)
            .field("user_id", &self.user_id)
            .field("access_token", &"[REDACTED]")
            .field("dm_policy", &self.dm_policy)
            .field("group_policy", &self.group_policy)
            .finish_non_exhaustive()
    }
}

pub struct RedactedConfig<'a>(pub &'a MatrixAccountConfig);

impl Serialize for RedactedConfig<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let c = self.0;
        let mut s = serializer.serialize_struct("MatrixAccountConfig", 16)?;
        s.serialize_field("homeserver_url", &c.homeserver_url)?;
        s.serialize_field("access_token", secret_serde::REDACTED)?;
        s.serialize_field("user_id", &c.user_id)?;
        s.serialize_field("encryption", &c.encryption)?;
        s.serialize_field("dm_policy", &c.dm_policy)?;
        s.serialize_field("group_policy", &c.group_policy)?;
        s.serialize_field("mention_mode", &c.mention_mode)?;
        s.serialize_field("allowlist", &c.allowlist)?;
        s.serialize_field("room_allowlist", &c.room_allowlist)?;
        s.serialize_field("stream_mode", &c.stream_mode)?;
        s.serialize_field("edit_throttle_ms", &c.edit_throttle_ms)?;
        s.serialize_field("otp_self_approval", &c.otp_self_approval)?;
        s.serialize_field("auto_join", &c.auto_join)?;
        if c.model.is_some() {
            s.serialize_field("model", &c.model)?;
        }
        if c.model_provider.is_some() {
            s.serialize_field("model_provider", &c.model_provider)?;
        }
        if !c.channel_overrides.is_empty() {
            s.serialize_field("channel_overrides", &c.channel_overrides)?;
        }
        s.end()
    }
}

impl ChannelConfigView for MatrixAccountConfig {
    fn allowlist(&self) -> &[String] {
        &self.allowlist
    }

    fn group_allowlist(&self) -> &[String] {
        &self.room_allowlist
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

    fn channel_model(&self, channel_id: &str) -> Option<&str> {
        self.channel_overrides
            .get(channel_id)
            .and_then(|o| o.model.as_deref())
    }

    fn channel_model_provider(&self, channel_id: &str) -> Option<&str> {
        self.channel_overrides
            .get(channel_id)
            .and_then(|o| o.model_provider.as_deref())
    }

    fn user_model(&self, user_id: &str) -> Option<&str> {
        self.user_overrides
            .get(user_id)
            .and_then(|o| o.model.as_deref())
    }

    fn user_model_provider(&self, user_id: &str) -> Option<&str> {
        self.user_overrides
            .get(user_id)
            .and_then(|o| o.model_provider.as_deref())
    }
}

impl Default for MatrixAccountConfig {
    fn default() -> Self {
        Self {
            homeserver_url: String::new(),
            access_token: Secret::new(String::new()),
            password: None,
            user_id: String::new(),
            device_id: None,
            device_display_name: None,
            encryption: true,
            allow_private_network: false,
            dm_policy: DmPolicy::default(),
            group_policy: GroupPolicy::default(),
            mention_mode: MentionMode::default(),
            allowlist: Vec::new(),
            room_allowlist: Vec::new(),
            stream_mode: StreamMode::default(),
            edit_throttle_ms: 300,
            stream_notify_on_complete: false,
            stream_min_initial_chars: 30,
            model: None,
            model_provider: None,
            otp_self_approval: true,
            otp_cooldown_secs: 300,
            auto_join: AutoJoinPolicy::default(),
            auto_join_allowlist: Vec::new(),
            channel_overrides: HashMap::new(),
            user_overrides: HashMap::new(),
        }
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;

    use super::*;

    #[test]
    fn default_config() {
        let cfg = MatrixAccountConfig::default();
        assert_eq!(cfg.dm_policy, DmPolicy::Allowlist);
        assert_eq!(cfg.group_policy, GroupPolicy::Open);
        assert_eq!(cfg.mention_mode, MentionMode::Mention);
        assert_eq!(cfg.stream_mode, StreamMode::EditInPlace);
        assert_eq!(cfg.edit_throttle_ms, 300);
        assert!(cfg.encryption);
        assert!(!cfg.allow_private_network);
        assert!(cfg.otp_self_approval);
        assert_eq!(cfg.auto_join, AutoJoinPolicy::Off);
    }

    #[test]
    fn deserialize_from_json() {
        let json = r#"{
            "homeserver_url": "https://matrix.example.org",
            "access_token": "syt_abc123",
            "user_id": "@bot:example.org",
            "dm_policy": "open",
            "stream_mode": "off",
            "allowlist": ["@alice:example.org"]
        }"#;
        let cfg: MatrixAccountConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.access_token.expose_secret(), "syt_abc123");
        assert_eq!(cfg.dm_policy, DmPolicy::Open);
        assert_eq!(cfg.stream_mode, StreamMode::Off);
        assert_eq!(cfg.allowlist, vec!["@alice:example.org"]);
        assert_eq!(cfg.group_policy, GroupPolicy::Open);
    }

    #[test]
    fn redacted_hides_token() {
        let cfg = MatrixAccountConfig {
            homeserver_url: "https://matrix.example.org".into(),
            access_token: Secret::new("syt_secret".into()),
            user_id: "@bot:example.org".into(),
            ..Default::default()
        };
        let redacted = serde_json::to_value(RedactedConfig(&cfg)).unwrap();
        assert_eq!(redacted["access_token"], "[REDACTED]");
        assert_eq!(redacted["homeserver_url"], "https://matrix.example.org");

        let storage = serde_json::to_value(&cfg).unwrap();
        assert_eq!(storage["access_token"], "syt_secret");
    }

    #[test]
    fn overrides_round_trip() {
        let json = serde_json::json!({
            "homeserver_url": "https://matrix.example.org",
            "access_token": "tok",
            "user_id": "@bot:example.org",
            "channel_overrides": {
                "!room1:example.org": { "model": "gpt-4" }
            },
            "user_overrides": {
                "@alice:example.org": { "model": "claude-sonnet", "model_provider": "anthropic" }
            }
        });
        let cfg: MatrixAccountConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.channel_model("!room1:example.org"), Some("gpt-4"));
        assert_eq!(cfg.user_model("@alice:example.org"), Some("claude-sonnet"));
        assert_eq!(
            cfg.user_model_provider("@alice:example.org"),
            Some("anthropic")
        );
    }
}
