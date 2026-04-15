use {async_trait::async_trait, serde_json::Value};

pub mod error;

mod config_helpers;
mod custom_providers;
mod key_store;
mod known_providers;
mod oauth;
mod ollama;
mod service;

/// Callback for publishing events to connected clients.
///
/// The gateway wires this up to its WebSocket broadcast mechanism so the
/// provider-setup crate doesn't depend on the gateway's internal types.
#[async_trait]
pub trait SetupBroadcaster: Send + Sync {
    async fn broadcast(&self, topic: &str, payload: Value);
}

// ── Re-exports ─────────────────────────────────────────────────────────────
// Preserve the existing public API: all items previously accessible as
// `moltis_provider_setup::Foo` remain accessible at the crate root.

pub use {
    config_helpers::{
        AutoDetectedProviderSource, config_with_saved_keys,
        detect_auto_provider_sources_with_overrides, has_explicit_provider_settings,
    },
    key_store::{KeyStore, ProviderConfig},
    known_providers::{AuthType, KnownProvider, known_providers},
    oauth::import_detected_oauth_tokens,
    service::{ErrorParser, LiveProviderSetupService},
};
