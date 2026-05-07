//! Telephony channel plugin for moltis.
//!
//! Provides phone call support via pluggable telephony providers (Twilio, etc.).
//! Implements `ChannelPlugin` so voice calls integrate with the gateway like
//! any other channel — webhook-driven inbound, outbound via `ChannelOutbound`.

pub mod audio;
pub mod config;
pub mod error;
pub mod manager;
pub mod outbound;
pub mod plugin;
pub mod provider;
pub mod providers;
pub mod tool;
pub mod types;
pub mod webhook;

pub use {
    config::TelephonyAccountConfig,
    error::{Error, Result},
    plugin::TelephonyPlugin,
    tool::VoiceCallTool,
};
