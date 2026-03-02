use {
    moltis_channels::gating::{DmPolicy, GroupPolicy, MentionMode},
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Serialize},
};

/// Configuration for a single Microsoft Teams bot account.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MsTeamsAccountConfig {
    /// Microsoft App ID (bot registration client ID).
    pub app_id: String,

    /// Microsoft App Password (client secret).
    #[serde(serialize_with = "serialize_secret")]
    pub app_password: Secret<String>,

    /// OAuth tenant segment for Bot Framework token issuance.
    pub oauth_tenant: String,

    /// OAuth scope for Bot Framework connector API.
    pub oauth_scope: String,

    /// DM access policy.
    pub dm_policy: DmPolicy,

    /// Group access policy.
    pub group_policy: GroupPolicy,

    /// Mention activation mode for group chats.
    pub mention_mode: MentionMode,

    /// User allowlist (AAD object IDs or channel user IDs).
    pub allowlist: Vec<String>,

    /// Group/team allowlist.
    pub group_allowlist: Vec<String>,

    /// Optional shared secret validated against `?secret=...` on webhook calls.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_option_secret"
    )]
    pub webhook_secret: Option<Secret<String>>,

    /// Default model ID for this channel account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Provider name associated with `model`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
}

impl std::fmt::Debug for MsTeamsAccountConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MsTeamsAccountConfig")
            .field("app_id", &self.app_id)
            .field("app_password", &"[REDACTED]")
            .field("oauth_tenant", &self.oauth_tenant)
            .field("oauth_scope", &self.oauth_scope)
            .field("dm_policy", &self.dm_policy)
            .field("group_policy", &self.group_policy)
            .field("mention_mode", &self.mention_mode)
            .field("allowlist", &self.allowlist)
            .field("group_allowlist", &self.group_allowlist)
            .field(
                "webhook_secret",
                &self.webhook_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("model", &self.model)
            .field("model_provider", &self.model_provider)
            .finish()
    }
}

fn serialize_secret<S: serde::Serializer>(
    secret: &Secret<String>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(secret.expose_secret())
}

fn serialize_option_secret<S: serde::Serializer>(
    secret: &Option<Secret<String>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match secret {
        Some(s) => serializer.serialize_some(s.expose_secret()),
        None => serializer.serialize_none(),
    }
}

impl Default for MsTeamsAccountConfig {
    fn default() -> Self {
        Self {
            app_id: String::new(),
            app_password: Secret::new(String::new()),
            oauth_tenant: "botframework.com".into(),
            oauth_scope: "https://api.botframework.com/.default".into(),
            dm_policy: DmPolicy::Allowlist,
            group_policy: GroupPolicy::Open,
            mention_mode: MentionMode::Mention,
            allowlist: Vec::new(),
            group_allowlist: Vec::new(),
            webhook_secret: None,
            model: None,
            model_provider: None,
        }
    }
}
