//! Local LLM provider with pluggable backends.
//!
//! Supports multiple inference backends:
//! - GGUF (llama.cpp) - Cross-platform, CPU + GPU
//! - MLX - Apple Silicon optimized (macOS only)
//!
//! The provider automatically selects the best backend based on the platform
//! and available hardware.

pub mod backend;
pub mod models;
mod provider;
pub mod response_parser;
pub mod system_info;

pub use {
    backend::{BackendType, LocalBackend},
    models::{LocalModelDef, ModelFormat},
    provider::{LocalLlmConfig, LocalLlmProvider, log_system_info},
};

/// Total bytes currently held by loaded llama.cpp tensors for local GGUF
/// backends. This is updated when models are loaded/unloaded.
#[must_use]
pub fn loaded_llama_model_bytes() -> u64 {
    backend::loaded_llama_model_bytes()
}
