//! Discord inbound handler entrypoint.

#[path = "handler/implementation.rs"]
mod implementation;

pub use self::implementation::*;
