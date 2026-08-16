//! Discord inbound handler entrypoint.

#[path = "handler/implementation.rs"]
mod implementation;
#[path = "handler/reactions.rs"]
mod reactions;

pub use self::implementation::*;
