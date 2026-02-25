//! Microsoft Teams channel plugin for moltis.
//!
//! Implements a Bot Framework adapter with inbound webhook handling and
//! outbound message delivery via OAuth client-credentials.

pub mod activity;
pub mod auth;
pub mod config;
pub mod outbound;
pub mod plugin;
pub mod state;

pub use {config::MsTeamsAccountConfig, plugin::MsTeamsPlugin};
