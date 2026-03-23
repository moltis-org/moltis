//! Network-filter crate: domain filtering, proxy, and audit logging.
//!
//! Feature flags:
//! - `proxy`   — domain approval manager and HTTP CONNECT proxy server
//! - `service` — in-memory audit buffer with broadcast and file persistence
//! - `metrics` — counters/histograms via `moltis-metrics`

pub mod error;
pub mod types;

#[cfg(feature = "proxy")]
pub mod domain_approval;
#[cfg(feature = "proxy")]
pub mod proxy;

#[cfg(feature = "service")]
pub mod buffer;

pub use {
    error::{Error, Result},
    types::*,
};
