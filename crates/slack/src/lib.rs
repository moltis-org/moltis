//! Slack channel plugin for moltis.
//!
//! Connects to Slack via Socket Mode using the `slack-morphism` crate.
//! Handles inbound DMs and channel messages, applies access control
//! policies, and dispatches messages to the chat session.

pub mod blocks;
mod callback_worker;
pub mod channel_webhook_verifier;
pub mod client;
pub mod commands;
pub mod config;
pub mod emoji;
pub mod markdown;
mod native_stream;
pub mod outbound;
pub mod plugin;
pub mod socket;
mod socket_reconnect;
pub mod state;
pub mod webhook;

pub use {config::SlackAccountConfig, plugin::SlackPlugin};
