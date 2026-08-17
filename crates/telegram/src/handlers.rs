//! Telegram inbound handler entrypoint.
//!
//! The full implementation lives under `handlers/` so the crate root stays
//! readable and the file-size guard reflects the actual module boundaries.

#[path = "handlers/implementation.rs"]
mod implementation;
#[path = "handlers/reactions.rs"]
mod reactions;

pub use self::{implementation::*, reactions::handle_message_reaction};
