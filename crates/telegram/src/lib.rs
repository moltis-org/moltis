//! Telegram channel plugin for moltis.
//!
//! Implements `ChannelPlugin` using the teloxide library to receive and send
//! messages via the Telegram Bot API, including edit-in-place streaming.

pub mod access;
pub mod bot;
pub mod config;
pub mod handlers;
pub mod markdown;
pub mod otp;
pub mod outbound;
pub mod plugin;
pub mod state;

pub use {config::TelegramAccountConfig, plugin::TelegramPlugin};
