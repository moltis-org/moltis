//! `LiveProviderSetupService` — the runtime implementation of
//! `ProviderSetupService` that manages provider credentials, OAuth flows,
//! key validation, and provider registry rebuilds.

#[path = "../support.rs"]
mod support;

mod available;
mod credentials;
mod custom;
mod oauth;
mod service;
mod validate;

pub use service::*;

// Re-export items needed by tests via `super::*`.
#[cfg(test)]
use {crate::known_providers::known_providers, secrecy::Secret};

#[cfg(test)]
#[path = "../tests.rs"]
mod tests;
