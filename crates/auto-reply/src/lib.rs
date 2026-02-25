//! Inbound message processing pipeline — the glue between channels and agents.
//!
//! Flow: channel message → normalize MsgContext → resolve route → load session →
//! apply media understanding → parse directives → invoke agent → chunk response →
//! deliver via channel outbound.

pub mod chunk;
pub mod directives;
pub mod error;
pub mod queue;
pub mod reply;

pub use error::{Error, Result};
