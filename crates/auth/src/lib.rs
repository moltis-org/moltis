//! Authentication and credential management.
//!
//! This crate provides:
//! - `CredentialStore`: SQLite-backed storage for passwords, passkeys, API keys, and sessions
//! - `WebAuthnState`/`WebAuthnRegistry`: WebAuthn (passkey) challenge management
//! - Connection locality detection for auth decisions

pub mod credential_store;
pub mod locality;
pub mod webauthn;

pub use {
    credential_store::*,
    locality::{has_proxy_headers, is_local_connection},
    webauthn::{WebAuthnRegistry, WebAuthnState, load_passkeys},
};

#[cfg(feature = "vault")]
pub use moltis_vault;
