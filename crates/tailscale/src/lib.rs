//! Tailscale Serve/Funnel integration.
//!
//! Shells out to the `tailscale` CLI to manage HTTPS proxying:
//! - **Serve**: exposes the gateway over HTTPS within the tailnet.
//! - **Funnel**: exposes the gateway to the public internet via Tailscale.

pub mod error;
mod manager;

pub use {
    error::{Error, Result},
    manager::*,
};
