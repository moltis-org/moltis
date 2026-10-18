//! Discord inbound handler entrypoint.

#[path = "handler/implementation.rs"]
mod implementation;
#[path = "handler/reactions.rs"]
mod reactions;
#[path = "handler/reply_target.rs"]
mod reply_target;

pub use self::implementation::*;
