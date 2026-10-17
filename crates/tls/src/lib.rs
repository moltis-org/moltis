//! TLS certificate management and HTTPS server support.
//!
//! On first run, generates a local CA and server certificate (mkcert-style)
//! so the gateway can serve HTTPS out of the box. A companion plain-HTTP
//! server on a secondary port serves the CA cert for easy download and
//! redirects everything else to HTTPS.

mod ca;
mod certs;
pub mod error;
mod server;
pub mod trust;

pub use {
    certs::*,
    error::{Context, Error, Result},
    server::*,
};
