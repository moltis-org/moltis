/// Local GGUF embedding provider using llama-cpp-2.
///
/// Provides offline embedding via small GGUF models (e.g. EmbeddingGemma-300M).
/// Requires the `local-embeddings` feature flag and CMake + C++ compiler at build time.
use std::path::PathBuf;

use {
    anyhow::{Context, Result, bail},
    async_trait::async_trait,
    llama_cpp_2::{
        context::params::LlamaContextParams,
        llama_backend::LlamaBackend,
        llama_batch::LlamaBatch,
        model::{LlamaModel, params::LlamaModelParams},
    },
    tokio::sync::Mutex,
    tracing::info,
};

use crate::embeddings::EmbeddingProvider;

/// Default model: EmbeddingGemma-300M quantized to Q8_0 (~300MB, 768 dims).
const DEFAULT_MODEL_FILENAME: &str = "embeddinggemma-300M-Q8_0.gguf";
const DEFAULT_MODEL_URL: &str = "https://huggingface.co/lmstudio-community/EmbeddingGemma-300M-GGUF/resolve/main/EmbeddingGemma-300M-Q8_0.gguf";
const DEFAULT_DIMS: usize = 768;

/// Wrapper around `LlamaBackend` that opts into `Send + Sync`.
///
/// `LlamaBackend` is `!Send` because `llama-cpp-2` doesn't mark its FFI
/// handle as thread-safe. In practice the backend is an opaque init token
/// with no mutable state after construction, so sharing across threads is
/// safe. Wrapping it in a newtype keeps the `unsafe` declaration localised
/// rather than applying `unsafe impl` to the entire provider struct.
struct SendSyncBackend(LlamaBackend);

// SAFETY: LlamaBackend is an immutable init handle with no thread-local state.
unsafe impl Send for SendSyncBackend {}
unsafe impl Sync for SendSyncBackend {}

pub struct LocalGgufEmbeddingProvider {
    backend: SendSyncBackend,
    model: Mutex<LlamaModel>,
    dims: usize,
}

impl LocalGgufEmbeddingProvider {
    /// Load a GGUF model from a specific path.
    pub fn new(model_path: PathBuf) -> Result<Self> {
        let backend = LlamaBackend::init()?;
        let model_params = LlamaModelParams::default();
        let model = LlamaModel::load_from_file(&backend, &model_path, &model_params)
            .map_err(|e| anyhow::anyhow!("failed to load GGUF model: {e}"))?;

        let dims = DEFAULT_DIMS;

        info!(path = %model_path.display(), dims, "loaded local GGUF embedding model");

        Ok(Self {
            backend: SendSyncBackend(backend),
            model: Mutex::new(model),
            dims,
        })
    }

    /// Ensure the default model exists in the cache directory, downloading if needed.
    pub async fn ensure_model(cache_dir: PathBuf) -> Result<PathBuf> {
        let model_path = cache_dir.join(DEFAULT_MODEL_FILENAME);
        if model_path.exists() {
            info!(path = %model_path.display(), "local embedding model found in cache");
            return Ok(model_path);
        }

        tokio::fs::create_dir_all(&cache_dir)
            .await
            .context("creating model cache dir")?;

        info!(url = DEFAULT_MODEL_URL, "downloading local embedding model");

        let response = reqwest::get(DEFAULT_MODEL_URL)
            .await
            .context("downloading GGUF model")?
            .error_for_status()
            .context("GGUF model download failed")?;

        let bytes = response.bytes().await.context("reading model bytes")?;

        let tmp_path = model_path.with_extension("tmp");
        tokio::fs::write(&tmp_path, &bytes)
            .await
            .context("writing model file")?;
        tokio::fs::rename(&tmp_path, &model_path)
            .await
            .context("renaming model file")?;

        info!(
            path = %model_path.display(),
            size_mb = bytes.len() / (1024 * 1024),
            "local embedding model downloaded"
        );

        Ok(model_path)
    }

    /// Default cache directory: `~/.moltis/models/`.
    pub fn default_cache_dir() -> PathBuf {
        directories::ProjectDirs::from("", "", "moltis")
            .map(|d: directories::ProjectDirs| d.data_dir().join("models"))
            .unwrap_or_else(|| PathBuf::from(".moltis/models"))
    }
}

/// Embed a text using the given model and backend. Must be called from a sync context.
fn embed_sync(backend: &LlamaBackend, model: &LlamaModel, text: &str) -> Result<Vec<f32>> {
    let ctx_params = LlamaContextParams::default()
        .with_n_ctx(std::num::NonZeroU32::new(512))
        .with_embeddings(true);
    let mut ctx = model
        .new_context(backend, ctx_params)
        .map_err(|e| anyhow::anyhow!("failed to create llama context: {e}"))?;

    let tokens = model
        .str_to_token(text, llama_cpp_2::model::AddBos::Always)
        .map_err(|e| anyhow::anyhow!("tokenization failed: {e}"))?;

    if tokens.is_empty() {
        bail!("empty token sequence");
    }

    let mut batch = LlamaBatch::new(tokens.len(), 1);
    for (i, &token) in tokens.iter().enumerate() {
        let is_last = i == tokens.len() - 1;
        batch
            .add(token, i as i32, &[0], is_last)
            .map_err(|e| anyhow::anyhow!("batch add failed: {e}"))?;
    }

    ctx.decode(&mut batch)
        .map_err(|e| anyhow::anyhow!("decode failed: {e}"))?;

    let embeddings = ctx
        .embeddings_seq_ith(0)
        .map_err(|e| anyhow::anyhow!("get embeddings failed: {e}"))?;

    Ok(embeddings.to_vec())
}

#[async_trait]
impl EmbeddingProvider for LocalGgufEmbeddingProvider {
    async fn embed(&self, text: &str) -> crate::error::Result<Vec<f32>> {
        let model = self.model.lock().await;
        let text = text.to_string();
        // llama-cpp-2 is CPU-bound; use block_in_place to avoid starving the async runtime
        let backend = &self.backend.0;
        let model_ref = &*model;
        let result = tokio::task::block_in_place(move || embed_sync(backend, model_ref, &text))
            .map_err(|e| crate::error::Error::Embedding(e.to_string()))?;
        Ok(result)
    }

    fn model_name(&self) -> &str {
        "local-gguf"
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    fn provider_key(&self) -> &str {
        "local-gguf"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_cache_dir() {
        let dir = LocalGgufEmbeddingProvider::default_cache_dir();
        assert!(dir.to_string_lossy().contains("models"));
    }
}
