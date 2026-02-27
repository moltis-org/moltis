//! Discord channel plugin for moltis.
//!
//! Connects to the Discord Gateway API via a persistent WebSocket using
//! the serenity library. Handles inbound DMs and guild messages, applies
//! access control policies, and dispatches messages to the chat session.

pub mod access;
pub mod commands;
pub mod config;
pub mod error;
pub mod handler;
pub mod outbound;
pub mod plugin;
pub mod state;

pub use {
    config::{ActivityType, DiscordAccountConfig, OnlineStatus},
    plugin::DiscordPlugin,
};
