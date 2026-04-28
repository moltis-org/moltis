//! Local LLM provider setup service.
//!
//! Provides RPC handlers for configuring the local GGUF LLM provider,
//! including system info detection, model listing, and model configuration.

#[allow(unused_imports)]
use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

#[allow(unused_imports)]
use {
    async_trait::async_trait,
    base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD},
    serde::{Deserialize, Serialize},
    serde_json::Value,
    tokio::sync::{OnceCell, RwLock, watch},
    tracing::{info, warn},
};

#[allow(unused_imports)]
use moltis_providers::{ProviderRegistry, local_gguf, local_llm, model_id::raw_model_id};

#[allow(unused_imports)]
use crate::{
    broadcast::{BroadcastOpts, broadcast},
    services::{LocalLlmService, ServiceResult},
    state::GatewayState,
};

mod cache;
mod config;
mod lifecycle;
mod service;
#[cfg(test)]
mod tests;

pub use {
    cache::{LocalModelCacheError, LocalModelCacheResult, ensure_local_model_cached},
    config::{LocalLlmConfig, LocalModelEntry, register_saved_local_models},
    lifecycle::ModelLifecycleManager,
    service::{LiveLocalLlmService, LocalLlmStatus},
};
