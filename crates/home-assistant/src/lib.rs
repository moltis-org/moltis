//! Home Assistant integration crate for Moltis.
//!
//! Provides REST and WebSocket API clients for controlling Home Assistant
//! instances, plus an [`AgentTool`] implementation for LLM-driven automation.

pub mod client;
pub mod client_ext;
pub mod config;
pub mod error;
pub mod tool;
pub mod types;
pub mod ws;

pub use {
    config::{HomeAssistantAccountConfig, HomeAssistantConfig},
    error::Error,
    ws::HaConnection,
};
