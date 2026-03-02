//! Slack channel plugin for moltis.
//!
//! Connects to Slack via Socket Mode using the `slack-morphism` crate.
//! Handles inbound DMs and channel messages, applies access control
//! policies, and dispatches messages to the chat session.

pub mod commands;
pub mod config;
pub mod markdown;
pub mod outbound;
pub mod plugin;
pub mod socket;
pub mod state;
pub mod webhook;

pub use {config::SlackAccountConfig, plugin::SlackPlugin};
