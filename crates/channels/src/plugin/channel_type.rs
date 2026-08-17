//! The channel type enum and its conversions.
//!
//! Split out of `plugin` to keep both files inside the file-size limit;
//! `ChannelType` and its parsing are a self-contained concern.

use crate::{
    Error,
    plugin::{ChannelCapabilities, ChannelDescriptor, InboundMode},
};

/// Supported channel types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum ChannelType {
    Telegram,
    Whatsapp,
    #[serde(rename = "msteams")]
    MsTeams,
    Discord,
    Slack,
    Matrix,
    Nostr,
    Signal,
    Telephony,
}

impl ChannelType {
    /// Returns the channel type identifier as a string slice.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Telegram => "telegram",
            Self::Whatsapp => "whatsapp",
            Self::MsTeams => "msteams",
            Self::Discord => "discord",
            Self::Slack => "slack",
            Self::Matrix => "matrix",
            Self::Nostr => "nostr",
            Self::Signal => "signal",
            Self::Telephony => "telephony",
        }
    }

    /// Human-readable display name for UI labels.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Telegram => "Telegram",
            Self::Whatsapp => "WhatsApp",
            Self::MsTeams => "Microsoft Teams",
            Self::Discord => "Discord",
            Self::Slack => "Slack",
            Self::Matrix => "Matrix",
            Self::Nostr => "Nostr",
            Self::Signal => "Signal",
            Self::Telephony => "Phone Call",
        }
    }

    /// Best-effort chat classification for hook and prompt context.
    #[must_use]
    pub fn classify_chat(&self, chat_id: &str) -> Option<String> {
        crate::chat_classification::classify_chat(*self, chat_id)
    }

    /// Whether a chat can contain messages from multiple principals.
    /// Unknown platform chat kinds fail closed as shared.
    #[must_use]
    pub fn is_shared_chat(&self, chat_id: &str) -> bool {
        crate::chat_classification::is_shared_chat(*self, chat_id)
    }

    /// Top-level config fields that must be treated as persisted secrets.
    pub fn secret_fields(&self) -> &'static [&'static str] {
        match self {
            Self::Telegram => &["token"],
            Self::Whatsapp => &[],
            Self::MsTeams => &["app_password", "webhook_secret"],
            Self::Discord => &["token"],
            Self::Slack => &["bot_token", "app_token", "signing_secret"],
            Self::Matrix => &["access_token", "password"],
            Self::Nostr => &["secret_key"],
            Self::Signal => &[],
            Self::Telephony => &["auth_token"],
        }
    }
}

impl std::fmt::Display for ChannelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ChannelType {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "telegram" => Ok(Self::Telegram),
            "whatsapp" => Ok(Self::Whatsapp),
            "msteams" | "microsoft_teams" | "microsoft-teams" | "teams" => Ok(Self::MsTeams),
            "discord" => Ok(Self::Discord),
            "slack" => Ok(Self::Slack),
            "matrix" | "element" => Ok(Self::Matrix),
            "nostr" => Ok(Self::Nostr),
            "signal" => Ok(Self::Signal),
            "telephony" | "phone" | "voice_call" | "voicecall" => Ok(Self::Telephony),
            other => Err(Error::invalid_input(format!(
                "unknown channel type: {other}"
            ))),
        }
    }
}

impl ChannelType {
    /// All known channel types.
    pub const ALL: &[ChannelType] = &[
        Self::Telegram,
        Self::Whatsapp,
        Self::MsTeams,
        Self::Discord,
        Self::Slack,
        Self::Matrix,
        Self::Nostr,
        Self::Signal,
        Self::Telephony,
    ];

    /// Returns the static descriptor for this channel type.
    #[must_use]
    pub fn descriptor(&self) -> ChannelDescriptor {
        match self {
            Self::Telegram => ChannelDescriptor {
                channel_type: *self,
                display_name: "Telegram",
                capabilities: ChannelCapabilities {
                    inbound_mode: InboundMode::Polling,
                    supports_outbound: true,
                    supports_streaming: true,
                    supports_interactive: false,
                    supports_threads: false,
                    supports_voice_ingest: true,
                    supports_pairing: false,
                    supports_otp: true,
                    supports_reactions: false,
                    supports_location: true,
                },
            },
            Self::Whatsapp => ChannelDescriptor {
                channel_type: *self,
                display_name: "WhatsApp",
                capabilities: ChannelCapabilities {
                    inbound_mode: InboundMode::GatewayLoop,
                    supports_outbound: true,
                    supports_streaming: true,
                    supports_interactive: false,
                    supports_threads: false,
                    supports_voice_ingest: true,
                    supports_pairing: true,
                    supports_otp: true,
                    supports_reactions: false,
                    supports_location: false,
                },
            },
            Self::MsTeams => ChannelDescriptor {
                channel_type: *self,
                display_name: "Microsoft Teams",
                capabilities: ChannelCapabilities {
                    inbound_mode: InboundMode::Webhook,
                    supports_outbound: true,
                    supports_streaming: true,
                    supports_interactive: true,
                    supports_threads: true,
                    supports_voice_ingest: false,
                    supports_pairing: false,
                    supports_otp: true,
                    supports_reactions: true,
                    supports_location: true,
                },
            },
            Self::Discord => ChannelDescriptor {
                channel_type: *self,
                display_name: "Discord",
                capabilities: ChannelCapabilities {
                    inbound_mode: InboundMode::GatewayLoop,
                    supports_outbound: true,
                    supports_streaming: true,
                    supports_interactive: true,
                    supports_threads: true,
                    supports_voice_ingest: true,
                    supports_pairing: false,
                    supports_otp: false,
                    supports_reactions: false,
                    supports_location: true,
                },
            },
            Self::Slack => ChannelDescriptor {
                channel_type: *self,
                display_name: "Slack",
                capabilities: ChannelCapabilities {
                    inbound_mode: InboundMode::SocketMode,
                    supports_outbound: true,
                    supports_streaming: true,
                    supports_interactive: true,
                    supports_threads: true,
                    supports_voice_ingest: false,
                    supports_pairing: false,
                    supports_otp: true,
                    supports_reactions: true,
                    supports_location: false,
                },
            },
            Self::Matrix => ChannelDescriptor {
                channel_type: *self,
                display_name: "Matrix",
                capabilities: ChannelCapabilities {
                    inbound_mode: InboundMode::GatewayLoop,
                    supports_outbound: true,
                    supports_streaming: true,
                    supports_interactive: true,
                    supports_threads: true,
                    supports_voice_ingest: true,
                    supports_pairing: false,
                    supports_otp: true,
                    supports_reactions: true,
                    supports_location: true,
                },
            },
            Self::Nostr => ChannelDescriptor {
                channel_type: *self,
                display_name: "Nostr",
                capabilities: ChannelCapabilities {
                    inbound_mode: InboundMode::GatewayLoop,
                    supports_outbound: true,
                    supports_streaming: false,
                    supports_interactive: false,
                    supports_threads: false,
                    supports_voice_ingest: false,
                    supports_pairing: false,
                    supports_otp: true,
                    supports_reactions: false,
                    supports_location: false,
                },
            },
            Self::Signal => ChannelDescriptor {
                channel_type: *self,
                display_name: "Signal",
                capabilities: ChannelCapabilities {
                    inbound_mode: InboundMode::GatewayLoop,
                    supports_outbound: true,
                    supports_streaming: false,
                    supports_interactive: false,
                    supports_threads: false,
                    supports_voice_ingest: false,
                    supports_pairing: false,
                    supports_otp: true,
                    supports_reactions: false,
                    supports_location: false,
                },
            },
            Self::Telephony => ChannelDescriptor {
                channel_type: *self,
                display_name: "Phone Call",
                capabilities: ChannelCapabilities {
                    inbound_mode: InboundMode::Webhook,
                    supports_outbound: true,
                    supports_streaming: false,
                    supports_interactive: false,
                    supports_threads: false,
                    supports_voice_ingest: true,
                    supports_pairing: false,
                    supports_otp: false,
                    supports_reactions: false,
                    supports_location: false,
                },
            },
        }
    }
}

// ── Channel capabilities ──────────────────────────────────────────────────
