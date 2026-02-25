//! WhatsApp channel plugin for moltis.
//!
//! Implements `ChannelPlugin` using the `whatsapp-rust` library to receive and
//! send messages via WhatsApp Linked Devices (QR code pairing).

pub mod access;
pub mod config;
pub mod connection;
pub mod error;
pub mod handlers;
pub mod memory_store;
pub mod otp;
pub mod outbound;
pub mod plugin;
pub mod sled_store;
pub mod state;

pub use {
    config::WhatsAppAccountConfig,
    error::{Error, Result},
    plugin::WhatsAppPlugin,
};
