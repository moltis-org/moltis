use crate::gating::{DmPolicy, GroupPolicy};

/// Typed read-only view of common channel account config fields.
///
/// Each plugin's concrete config type (e.g. `TelegramAccountConfig`) implements
/// this trait. The gateway and registry access typed fields instead of digging
/// into raw `serde_json::Value`.
///
/// The store persists `Value` via `StoredChannel`. This trait is purely for
/// typed read access to shared config fields.
pub trait ChannelConfigView: Send + Sync + std::fmt::Debug {
    /// DM user/peer allowlist.
    fn allowlist(&self) -> &[String];

    /// Group/chat ID allowlist.
    fn group_allowlist(&self) -> &[String];

    /// DM access policy.
    fn dm_policy(&self) -> DmPolicy;

    /// Group access policy.
    fn group_policy(&self) -> GroupPolicy;

    /// Default model ID for this channel account.
    fn model(&self) -> Option<&str>;

    /// Provider name associated with the model.
    fn model_provider(&self) -> Option<&str>;

    /// Default agent id for this channel account.
    fn agent_id(&self) -> Option<&str> {
        None
    }

    /// Whether `sender_id`, as this channel reports it, is on [`Self::allowlist`].
    ///
    /// Privileged commands (`/approve`, `/deny`, `/update`) authorize against
    /// this, so it has to agree with how the channel spells identities on the
    /// wire *and* how an operator is likely to have written them in config. The
    /// default compares text and is right for channels with one spelling per
    /// identity; override it where an identity has several (Nostr keys are
    /// either `npub1…` bech32 or 64-char hex, and both are valid to configure).
    ///
    /// Inherits [`gating::is_allowed`](crate::gating::is_allowed)'s convention
    /// that an empty allowlist matches everyone — callers that treat "nobody is
    /// authorized" differently must check for empty themselves.
    fn sender_on_allowlist(&self, sender_id: &str) -> bool {
        crate::gating::sender_matches_allowlist(sender_id, self.allowlist())
    }

    // ── Per-channel / per-user override methods ─────────────────────────────

    /// Model override for a specific channel/chat ID.
    fn channel_model(&self, _channel_id: &str) -> Option<&str> {
        None
    }

    /// Provider override for a specific channel/chat ID.
    fn channel_model_provider(&self, _channel_id: &str) -> Option<&str> {
        None
    }

    /// Model override for a specific user.
    fn user_model(&self, _user_id: &str) -> Option<&str> {
        None
    }

    /// Provider override for a specific user.
    fn user_model_provider(&self, _user_id: &str) -> Option<&str> {
        None
    }

    /// Agent override for a specific channel/chat ID.
    fn channel_agent_id(&self, _channel_id: &str) -> Option<&str> {
        None
    }

    /// Agent override for a specific user.
    fn user_agent_id(&self, _user_id: &str) -> Option<&str> {
        None
    }

    /// Resolve effective model: user > channel > account default.
    fn resolve_model(&self, channel_id: &str, user_id: &str) -> Option<&str> {
        self.user_model(user_id)
            .or_else(|| self.channel_model(channel_id))
            .or_else(|| self.model())
    }

    /// Resolve effective provider: user > channel > account default.
    fn resolve_model_provider(&self, channel_id: &str, user_id: &str) -> Option<&str> {
        self.user_model_provider(user_id)
            .or_else(|| self.channel_model_provider(channel_id))
            .or_else(|| self.model_provider())
    }

    /// Resolve effective agent id: user > channel > account default.
    fn resolve_agent_id(&self, channel_id: &str, user_id: &str) -> Option<&str> {
        self.user_agent_id(user_id)
            .or_else(|| self.channel_agent_id(channel_id))
            .or_else(|| self.agent_id())
    }
}
