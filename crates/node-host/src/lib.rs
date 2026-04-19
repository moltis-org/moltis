//! Headless node host: connects to a gateway as a node and executes commands.
//!
//! Usage: `moltis node run --host <gateway> --token <device-token>`
//!
//! The node host establishes a WebSocket connection to the gateway,
//! authenticates with a device token, and handles `system.run` commands
//! by executing them locally.

pub mod error;
pub mod runner;
pub mod service;

pub use {
    error::Error,
    runner::{NodeConfig, NodeHost},
    service::ServiceConfig,
};
