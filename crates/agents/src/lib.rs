//! LLM agent runtime: model selection, prompt building, tool execution, streaming.

pub mod auth_profiles;
pub mod json_repair;
pub mod memory_writer;
pub mod model;
pub mod multimodal;
pub mod prompt;
pub mod runner;
pub mod tool_parsing;
pub use {
    model::{ChatMessage, ContentPart, UserContent},
    runner::AgentRunError,
};
pub mod provider_chain;
pub mod response_sanitizer;
pub mod silent_turn;
pub mod skills;
pub mod tool_registry;
