use std::{
    path::PathBuf,
    pin::Pin,
    sync::atomic::{AtomicI64, Ordering},
};

use {
    anyhow::Result, async_trait::async_trait, tokio::sync::RwLock, tokio_stream::Stream,
    tracing::info,
};

use moltis_agents::model::{ChatMessage, CompletionResponse, LlmProvider, StreamEvent};

use super::{backend, models};

/// Configuration for the local LLM provider.
#[derive(Debug, Clone)]
pub struct LocalLlmConfig {
    /// Model ID from the registry.
    pub model_id: String,
    /// Direct path to a model file (skips auto-download).
    pub model_path: Option<PathBuf>,
    /// Preferred backend (auto-detected if None).
    pub backend: Option<backend::BackendType>,
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
/// loads the model on first use. Supports on-demand unloading to free RAM.
pub struct LocalLlmProvider {
    config: LocalLlmConfig,
    inner: RwLock<Option<Box<dyn backend::LocalBackend>>>,
    selected_backend: RwLock<Option<backend::BackendType>>,
    /// Unix timestamp (seconds) of the last `complete()` or `stream()` call.
    last_activity: AtomicI64,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

impl LocalLlmProvider {
    /// Create a new lazy-loading local LLM provider.
    pub fn new(config: LocalLlmConfig) -> Self {
        Self {
            config,
            inner: RwLock::new(None),
            selected_backend: RwLock::new(None),
            last_activity: AtomicI64::new(0),
        }
    }

    /// Get the model ID for this provider.
    pub fn model_id(&self) -> &str {
        &self.config.model_id
    }

    /// Get the backend type that will be (or was) used.
    pub async fn backend_type(&self) -> backend::BackendType {
        if let Some(bt) = *self.selected_backend.read().await {
            return bt;
        }
        self.config
            .backend
            .unwrap_or_else(backend::detect_best_backend)
    }

    /// Whether the model backend is currently loaded in memory.
    pub async fn is_loaded(&self) -> bool {
        self.inner.read().await.is_some()
    }

    /// Unix timestamp (seconds) of the most recent inference call.
    pub fn last_activity_secs(&self) -> i64 {
        self.last_activity.load(Ordering::Relaxed)
    }

    /// Model size in bytes if the backend is loaded, 0 otherwise.
    pub async fn model_size_bytes(&self) -> u64 {
        self.inner
            .read()
            .await
            .as_ref()
            .map_or(0, |b| b.model_size_bytes())
    }

    /// Drop the loaded backend to free RAM.
    ///
    /// Returns `true` if the backend was loaded and has now been dropped.
    /// The next `complete()`/`stream()` call will reload from disk.
    pub async fn unload(&self) -> bool {
        let mut guard = self.inner.write().await;
        if guard.is_some() {
            *guard = None;
            true
        } else {
            false
        }
    }

    /// Ensure the backend is loaded, updating last-activity.
    pub async fn ensure_loaded(&self) -> Result<()> {
        // Fast path: check if already loaded
        {
            let guard = self.inner.read().await;
            if guard.is_some() {
                self.touch_activity();
                return Ok(());
            }
        }

        // Slow path: load the backend
        let mut guard = self.inner.write().await;

        // Double-check after acquiring write lock
        if guard.is_some() {
            self.touch_activity();
            return Ok(());
        }

        let backend_type = self
            .config
            .backend
            .unwrap_or_else(|| backend::detect_backend_for_model(&self.config.model_id));
        info!(
            model = %self.config.model_id,
            backend = ?backend_type,
            "loading local LLM model"
        );

        *self.selected_backend.write().await = Some(backend_type);

        let backend = backend::create_backend(backend_type, &self.config).await?;
        *guard = Some(backend);

        self.touch_activity();
        Ok(())
    }

    fn touch_activity(&self) {
        self.last_activity.store(now_secs(), Ordering::Relaxed);
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
        true
    }

    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> Result<CompletionResponse> {
        self.ensure_loaded().await?;
        self.touch_activity();

        let guard = self.inner.read().await;
        let backend = guard
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("backend should be loaded after ensure_loaded"))?;
        let result = if tools.is_empty() {
            backend.complete(messages).await
        } else {
            backend.complete_with_tools(messages, tools).await
        };
        self.touch_activity();
        result
    }

    fn stream(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        Box::pin(async_stream::stream! {
            if let Err(e) = self.ensure_loaded().await {
                yield StreamEvent::Error(format!("failed to load model: {e}"));
                return;
            }
            self.touch_activity();

            let guard = self.inner.read().await;
            let Some(backend) = guard.as_ref() else {
                yield StreamEvent::Error("backend should be loaded after ensure_loaded".into());
                return;
            };

            let mut stream = backend.stream(&messages);
            while let Some(event) = futures::StreamExt::next(&mut stream).await {
                yield event;
            }
            self.touch_activity();
        })
    }
}

/// Log system info and backend availability.
pub fn log_system_info() {
    let sys = super::system_info::SystemInfo::detect();
    let tier = sys.memory_tier();
    let best_backend = backend::detect_best_backend();

    info!(
        total_ram_gb = sys.total_ram_gb(),
        available_ram_gb = sys.available_ram_gb(),
        has_metal = sys.has_metal,
        has_cuda = sys.has_cuda,
        has_vulkan = sys.has_vulkan,
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
        assert!(matches!(
            backend,
            backend::BackendType::Gguf | backend::BackendType::Mlx
        ));
    }

    #[test]
    fn test_available_backends() {
        let backends = backend::available_backends();
        // GGUF should always be available when compiled with local-llm feature
        assert!(backends.contains(&backend::BackendType::Gguf));
    }

    #[tokio::test]
    async fn test_is_loaded_default_false() {
        let provider = LocalLlmProvider::new(LocalLlmConfig::default());
        assert!(!provider.is_loaded().await);
    }

    #[tokio::test]
    async fn test_unload_when_not_loaded() {
        let provider = LocalLlmProvider::new(LocalLlmConfig::default());
        assert!(!provider.unload().await);
    }

    #[test]
    fn test_last_activity_default_zero() {
        let provider = LocalLlmProvider::new(LocalLlmConfig::default());
        assert_eq!(provider.last_activity_secs(), 0);
    }

    #[test]
    fn test_model_id() {
        let config = LocalLlmConfig {
            model_id: "test-model".into(),
            ..Default::default()
        };
        let provider = LocalLlmProvider::new(config);
        assert_eq!(provider.model_id(), "test-model");
    }
}
