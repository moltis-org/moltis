use std::sync::Arc;

pub use moltis_auth::webauthn::*;

/// Shared, concurrency-safe registry of WebAuthn relying-party instances.
pub type SharedWebAuthnRegistry = Arc<tokio::sync::RwLock<WebAuthnRegistry>>;
