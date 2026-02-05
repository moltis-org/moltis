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
pub mod system_info;

use std::{path::PathBuf, pin::Pin};

use {
    anyhow::Result, async_trait::async_trait, tokio::sync::RwLock, tokio_stream::Stream,
    tracing::info,
};

use crate::model::{CompletionResponse, LlmProvider, StreamEvent};

pub use {
    backend::{BackendType, LocalBackend},
    models::{LocalModelDef, ModelFormat},
};

/// Configuration for the local LLM provider.
#[derive(Debug, Clone)]
pub struct LocalLlmConfig {
    /// Model ID from the registry.
    pub model_id: String,
    /// Direct path to a model file (skips auto-download).
    pub model_path: Option<PathBuf>,
    /// Preferred backend (auto-detected if None).
    pub backend: Option<BackendType>,
    /// Context size in tokens (default: from model definition).
    pub context_size: Option<u32>,
    /// Number of layers to offload to GPU (GGUF only, 0 = CPU only).
    pub gpu_layers: u32,
    /// Sampling temperature.
    pub temperature: f32,
    /// Directory for caching downloaded models.
    pub cache_dir: PathBuf,
}

impl Default for LocalLlmConfig {
    fn default() -> Self {
        Self {
            model_id: String::new(),
            model_path: None,
            backend: None,
            context_size: None,
            gpu_layers: 0,
            temperature: 0.7,
            cache_dir: models::default_models_dir(),
        }
    }
}

/// Local LLM provider with lazy loading.
///
/// Automatically selects the best backend for the current platform and
/// loads the model on first use.
pub struct LocalLlmProvider {
    config: LocalLlmConfig,
    inner: RwLock<Option<Box<dyn LocalBackend>>>,
    selected_backend: RwLock<Option<BackendType>>,
}

impl LocalLlmProvider {
    /// Create a new lazy-loading local LLM provider.
    pub fn new(config: LocalLlmConfig) -> Self {
        Self {
            config,
            inner: RwLock::new(None),
            selected_backend: RwLock::new(None),
        }
    }

    /// Get the backend type that will be (or was) used.
    pub async fn backend_type(&self) -> BackendType {
        if let Some(bt) = *self.selected_backend.read().await {
            return bt;
        }
        self.config
            .backend
            .unwrap_or_else(backend::detect_best_backend)
    }

    /// Ensure the backend is loaded.
    async fn ensure_loaded(&self) -> Result<()> {
        // Fast path: check if already loaded
        {
            let guard = self.inner.read().await;
            if guard.is_some() {
                return Ok(());
            }
        }

        // Slow path: load the backend
        let mut guard = self.inner.write().await;

        // Double-check after acquiring write lock
        if guard.is_some() {
            return Ok(());
        }

        let backend_type = self
            .config
            .backend
            .unwrap_or_else(backend::detect_best_backend);
        info!(
            model = %self.config.model_id,
            backend = ?backend_type,
            "loading local LLM model"
        );

        *self.selected_backend.write().await = Some(backend_type);

        let backend = backend::create_backend(backend_type, &self.config).await?;
        *guard = Some(backend);

        Ok(())
    }
}

#[async_trait]
impl LlmProvider for LocalLlmProvider {
    fn name(&self) -> &str {
        "local-llm"
    }

    fn id(&self) -> &str {
        &self.config.model_id
    }

    fn context_window(&self) -> u32 {
        self.config
            .context_size
            .or_else(|| models::find_model(&self.config.model_id).map(|m| m.context_window))
            .unwrap_or(8192)
    }

    fn supports_tools(&self) -> bool {
        // Local models don't support tool calling yet
        false
    }

    async fn complete(
        &self,
        messages: &[serde_json::Value],
        _tools: &[serde_json::Value],
    ) -> Result<CompletionResponse> {
        self.ensure_loaded().await?;

        let guard = self.inner.read().await;
        let backend = guard.as_ref().expect("backend should be loaded");
        backend.complete(messages).await
    }

    fn stream(
        &self,
        messages: Vec<serde_json::Value>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        Box::pin(async_stream::stream! {
            if let Err(e) = self.ensure_loaded().await {
                yield StreamEvent::Error(format!("failed to load model: {e}"));
                return;
            }

            let guard = self.inner.read().await;
            let backend = guard.as_ref().expect("backend should be loaded");

            let mut stream = backend.stream(&messages);
            while let Some(event) = futures::StreamExt::next(&mut stream).await {
                yield event;
            }
        })
    }
}

/// Log system info and backend availability.
pub fn log_system_info() {
    let sys = system_info::SystemInfo::detect();
    let tier = sys.memory_tier();
    let best_backend = backend::detect_best_backend();

    info!(
        total_ram_gb = sys.total_ram_gb(),
        available_ram_gb = sys.available_ram_gb(),
        has_metal = sys.has_metal,
        has_cuda = sys.has_cuda,
        is_apple_silicon = sys.is_apple_silicon,
        tier = %tier,
        best_backend = ?best_backend,
        "local-llm system info"
    );

    // Log available backends
    let available_backends = backend::available_backends();
    info!(backends = ?available_backends, "available local LLM backends");

    // Suggest models
    if let Some(suggested) = models::suggest_model(tier, best_backend) {
        info!(
            model = suggested.id,
            display_name = suggested.display_name,
            backend = ?suggested.format.backend_type(),
            "suggested local model for your system"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = LocalLlmConfig::default();
        assert!(config.model_id.is_empty());
        assert!(config.model_path.is_none());
        assert!(config.backend.is_none());
        assert_eq!(config.gpu_layers, 0);
    }

    #[tokio::test]
    async fn test_backend_detection() {
        let backend = backend::detect_best_backend();
        // Should always return something
        assert!(matches!(backend, BackendType::Gguf | BackendType::Mlx));
    }

    #[test]
    fn test_available_backends() {
        let backends = backend::available_backends();
        // GGUF should always be available when compiled with local-llm feature
        assert!(backends.contains(&BackendType::Gguf));
    }
}
