//! Local GGUF LLM provider using llama-cpp-2.
//!
//! Provides offline LLM inference via quantized GGUF models. Supports automatic
//! model download from HuggingFace and system memory detection for model suggestions.
//!
//! Requires the `local-llm` feature flag and CMake + C++ compiler at build time.

pub mod chat_templates;
pub mod models;
mod provider;
pub mod runtime_devices;
pub mod system_info;
pub mod tool_grammar;

pub use provider::{
    LazyLocalGgufProvider, LocalGgufConfig, LocalGgufProvider, log_system_info_and_suggestions,
};
