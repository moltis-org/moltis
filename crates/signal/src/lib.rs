//! Signal channel integration backed by an external `signal-cli` HTTP daemon.

pub mod client;
pub mod config;
pub mod inbound;
pub mod outbound;
pub mod plugin;
pub mod sse;
pub mod state;

pub use plugin::SignalPlugin;
