//! Telegram inbound handler entrypoint.
//!
//! The full implementation lives under `handlers/` so the crate root stays
//! readable and the file-size guard reflects the actual module boundaries.

#[path = "handlers/implementation.rs"]
mod implementation;

pub use self::implementation::*;
