//! LLM provider implementations and registry.

pub mod anthropic;
pub mod async_openai_provider;
mod client;
pub mod config_helpers;
pub mod discovered_model;
pub mod error;
#[cfg(feature = "provider-genai")]
pub mod genai_provider;
#[cfg(feature = "provider-github-copilot")]
pub mod github_copilot;
pub mod http;
#[cfg(feature = "provider-kimi-code")]
pub mod kimi_code;
#[cfg(feature = "local-llm")]
pub mod local_gguf;
#[cfg(feature = "local-llm")]
pub mod local_llm;
pub mod model_capabilities;
pub mod model_catalogs;
pub mod model_id;
pub mod ollama;
pub mod openai;
#[cfg(feature = "provider-openai-codex")]
pub mod openai_codex;
pub mod openai_compat;
pub mod registry;
pub mod ws_pool;

#[cfg(test)]
pub mod contract;

pub use client::{init_shared_http_client, shared_http_client};

#[allow(unused_imports)]
pub(crate) use config_helpers::{
    configured_models_for_provider, env_value, normalize_unique_models, oauth_discovery_enabled,
    resolve_api_key, should_fetch_models, subscription_preference_rank,
};
#[allow(unused_imports)]
pub(crate) use discovered_model::{
    merge_discovered_with_fallback_catalog, merge_preferred_and_discovered_models,
};
#[allow(unused_imports)]
pub(crate) use http::{retry_after_ms_from_headers, with_retry_after_marker};
#[allow(unused_imports)]
pub(crate) use model_id::{
    MODEL_ID_NAMESPACE_SEP, REASONING_SUFFIX_SEP, REASONING_SUFFIXES, namespaced_model_id,
    raw_model_id, split_reasoning_suffix,
};
#[allow(unused_imports)]
pub(crate) use ollama::normalize_ollama_api_base_url;
pub use {
    discovered_model::{DiscoveredModel, catalog_to_discovered},
    model_capabilities::{
        ModelCapabilities, ModelInfo, context_window_for_model,
        context_window_for_model_with_config, extract_cw_overrides, is_chat_capable_model, supports_reasoning_for_model,
        supports_tools_for_model, supports_vision_for_model,
    },
    registry::{
        PendingDiscoveries, ProviderRegistry, RediscoveryResult, fetch_discoverable_models,
    },
};
