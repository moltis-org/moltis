//! Discord inbound handler entrypoint.

#[path = "handler/attachments.rs"]
mod attachments;
#[path = "handler/documents.rs"]
mod documents;
#[path = "handler/implementation.rs"]
mod implementation;
#[path = "handler/reactions.rs"]
mod reactions;
#[path = "handler/reply_target.rs"]
mod reply_target;

pub use self::implementation::*;
