//! Network client for a running Moltis daemon.
//!
//! - [`Connection`]: one WebSocket RPC v4 connection (handshake, calls,
//!   events).
//! - [`ManagedClient`]: a connection that reconnects with backoff.
//! - [`api::Rpc`]: typed helpers for chat, sessions, models and approvals.
//! - [`auth::AuthClient`]: password login and scoped API key creation.
//! - [`tls`]: CA, fingerprint (trust on first use) or system trust.
//! - `local` (feature): the daemon configured in this machine's `moltis.toml`.
//! - `mdns` (feature): daemons advertised on the local network.
//! - `ssh` (feature): reach a remote daemon through an SSH port forward.

pub mod api;
pub mod auth;
pub mod backoff;
mod connection;
pub mod endpoint;
mod error;
#[cfg(feature = "local")]
pub mod local;
mod managed;
#[cfg(feature = "mdns")]
pub mod mdns;
#[cfg(feature = "ssh")]
pub mod ssh;
#[cfg(test)]
mod test_support;
pub mod tls;

pub use {
    connection::{ClientIdentity, ConnectOptions, Connection},
    endpoint::Endpoint,
    error::{Error, FailureKind, Result},
    managed::{ClientUpdate, ManagedClient},
    moltis_protocol as protocol,
    secrecy::SecretString,
    tls::TlsTrust,
};
