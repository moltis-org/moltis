//! Provider setup service entrypoint.
//!
//! The runtime implementation lives in `service/implementation/`; this
//! façade keeps the crate root small and exposes only the public API.

#[path = "service/implementation/mod.rs"]
mod implementation;

pub use self::implementation::{ErrorParser, LiveProviderSetupService};
