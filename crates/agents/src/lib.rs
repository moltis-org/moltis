//! LLM agent runtime: model selection, prompt building, tool execution, streaming.

// FFI wrappers for llama-cpp-2 require unsafe Send/Sync impls when local-llm feature is enabled.
#![cfg_attr(feature = "local-llm", allow(unsafe_code))]

pub mod auth_profiles;

/// Shared HTTP client for LLM providers.
///
/// All providers that don't need custom redirect/proxy settings should
/// reuse this client to share connection pools, DNS cache, and TLS sessions.
pub fn shared_http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::LazyLock<reqwest::Client> =
        std::sync::LazyLock::new(reqwest::Client::new);
    &CLIENT
}
pub mod memory_writer;
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
