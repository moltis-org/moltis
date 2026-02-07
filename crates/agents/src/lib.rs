//! LLM agent runtime: model selection, prompt building, tool execution, streaming.

// FFI wrappers for llama-cpp-2 require unsafe Send/Sync impls when local-llm feature is enabled.
#![cfg_attr(feature = "local-llm", allow(unsafe_code))]

pub mod auth_profiles;
pub mod model;
pub mod multimodal;
pub mod prompt;
pub mod providers;
pub mod runner;
pub use {
    model::{ChatMessage, ContentPart, UserContent},
    runner::AgentRunError,
};
pub mod provider_chain;
pub mod silent_turn;
pub mod skills;
pub mod tool_registry;
