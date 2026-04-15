//! Microsoft Teams channel plugin for moltis.
//!
//! Implements a Bot Framework adapter with inbound webhook handling and
//! outbound message delivery via OAuth client-credentials.

pub mod activity;
pub mod attachments;
pub mod auth;
pub mod cards;
pub mod channel_webhook_verifier;
pub mod chunking;
pub mod config;
pub mod errors;
pub mod graph;
pub mod jwt;
pub mod outbound;
pub mod plugin;
pub mod state;
pub mod streaming;

pub use {config::MsTeamsAccountConfig, plugin::MsTeamsPlugin};
