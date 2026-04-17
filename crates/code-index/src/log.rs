//! Internal tracing re-exports.
//!
//! Source files that need tracing should do:
//! ```ignore
//! use crate::log::{debug, info, warn, trace, error};
//! ```
//!
//! The `tracing` feature flag is always-on (part of default features).
//! The `metrics` feature flag is opt-in for recording metrics at key points.

pub(crate) use tracing::{debug, info, trace, warn};
