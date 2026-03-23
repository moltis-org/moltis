//! Channel plugin system.
//!
//! Each channel (Telegram, Discord, Slack, WhatsApp, etc.) implements the
//! ChannelPlugin trait with sub-traits for config, auth, inbound/outbound
//! messaging, status, and gateway lifecycle.

pub mod channel_webhook_middleware;
pub mod config_view;
pub mod contract;
pub mod error;
pub mod gating;
pub mod message_log;
pub mod otp;
pub mod plugin;
pub mod registry;
pub mod store;

pub use {
    channel_webhook_middleware::{
        ChannelWebhookDedupeResult, ChannelWebhookRatePolicy, ChannelWebhookRejection,
        ChannelWebhookVerifier, TimestampGuard, VerifiedChannelWebhook,
    },
    config_view::ChannelConfigView,
    error::{Error, Result},
    plugin::{
        ButtonRow, ButtonStyle, ChannelAttachment, ChannelCapabilities, ChannelDescriptor,
        ChannelEvent, ChannelEventSink, ChannelHealthSnapshot, ChannelMessageKind,
        ChannelMessageMeta, ChannelOtpProvider, ChannelOutbound, ChannelPlugin, ChannelReplyTarget,
        ChannelStatus, ChannelStreamOutbound, ChannelThreadContext, ChannelType, InboundMode,
        InteractiveButton, InteractiveMessage, StreamEvent, StreamReceiver, StreamSender,
        ThreadMessage,
    },
    registry::{ChannelRegistry, RegistryOutboundRouter},
};
