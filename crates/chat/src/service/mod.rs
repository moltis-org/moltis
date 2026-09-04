//! `LiveChatService` struct, constructors, and helper methods.

mod chat_impl;
mod types;

use types::QueuedMessage;
pub(crate) use types::{
    ActiveAssistantDraft, EventForwarder, build_persisted_assistant_message, commit_terminal_run,
};
pub use types::{
    ActiveToolCall, LiveChatService, build_tool_call_assistant_message, persist_tool_history_pair,
};
