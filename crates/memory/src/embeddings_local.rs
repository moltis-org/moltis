/// Local GGUF embedding provider using llama-cpp-2.
///
/// Provides offline embedding via small GGUF models (e.g. EmbeddingGemma-300M).
/// Requires the `local-embeddings` feature flag and CMake + C++ compiler at build time.
use std::{num::NonZeroU32, path::PathBuf};

use {
    anyhow::{Context, Result},
    async_trait::async_trait,
    llama_cpp_2::{
        context::params::LlamaContextParams,
        llama_backend::LlamaBackend,
        llama_batch::LlamaBatch,
        model::{LlamaModel, params::LlamaModelParams},
    },
    tokio::sync::Mutex,
    tracing::{info, warn},
};

use crate::embeddings::EmbeddingProvider;

/// Default model: EmbeddingGemma-300M quantized to Q8_0 (~300MB, 768 dims).
const DEFAULT_MODEL_FILENAME: &str = "embeddinggemma-300M-Q8_0.gguf";
const DEFAULT_MODEL_URL: &str = "https://huggingface.co/ggml-org/embeddinggemma-300M-GGUF/resolve/main/embeddinggemma-300M-Q8_0.gguf";
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

fn embedding_context_size(model: &LlamaModel) -> Result<NonZeroU32> {
    NonZeroU32::new(model.n_ctx_train()).context("embedding model reports a zero context size")
}

fn embedding_context_params(context_size: NonZeroU32) -> LlamaContextParams {
    // Non-causal encoders must process the complete input in one physical batch.
    // Keeping all three limits equal prevents llama.cpp from aborting when an input
    // is larger than its default 512-token ubatch.
    LlamaContextParams::default()
        .with_n_ctx(Some(context_size))
        .with_n_batch(context_size.get())
        .with_n_ubatch(context_size.get())
        .with_embeddings(true)
}

fn truncate_tokens_to_context<T>(tokens: &mut Vec<T>, context_size: NonZeroU32) -> bool {
    let max_tokens = context_size.get() as usize;
    if tokens.len() <= max_tokens {
        return false;
    }

    tokens.truncate(max_tokens);
    true
}

/// Embed a text using the given model and backend. Must be called from a sync context.
fn embed_sync(backend: &LlamaBackend, model: &LlamaModel, text: &str) -> Result<Vec<f32>> {
    let max_context_size = embedding_context_size(model)?;

    let mut tokens = model
        .str_to_token(text, llama_cpp_2::model::AddBos::Always)
        .map_err(|e| anyhow::anyhow!("tokenization failed: {e}"))?;

    let input_tokens = tokens.len();
    if truncate_tokens_to_context(&mut tokens, max_context_size) {
        warn!(
            input_tokens,
            max_tokens = max_context_size.get(),
            "local embedding input exceeds model context; truncating"
        );
    }

    let batch_size = u32::try_from(tokens.len())
        .context("embedding token count exceeds the supported batch size")?;
    let batch_size = NonZeroU32::new(batch_size).context("empty token sequence")?;
    let ctx_params = embedding_context_params(batch_size);

    let mut ctx = model
        .new_context(backend, ctx_params)
        .map_err(|e| anyhow::anyhow!("failed to create llama context: {e}"))?;

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
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_default_cache_dir() {
        let dir = LocalGgufEmbeddingProvider::default_cache_dir();
        assert!(dir.to_string_lossy().contains("models"));
    }

    #[test]
    fn context_params_use_one_full_encoder_batch() {
        let context_size = NonZeroU32::new(2_048).expect("test context must be non-zero");
        let params = embedding_context_params(context_size);

        assert_eq!(params.n_ctx(), Some(context_size));
        assert_eq!(params.n_batch(), context_size.get());
        assert_eq!(params.n_ubatch(), context_size.get());
    }

    #[test]
    fn token_truncation_preserves_inputs_at_or_below_context() {
        let context_size = NonZeroU32::new(3).expect("test context must be non-zero");
        let mut below = vec![1, 2];
        let mut exact = vec![1, 2, 3];

        assert!(!truncate_tokens_to_context(&mut below, context_size));
        assert_eq!(below, vec![1, 2]);
        assert!(!truncate_tokens_to_context(&mut exact, context_size));
        assert_eq!(exact, vec![1, 2, 3]);
    }

    #[test]
    fn token_truncation_caps_inputs_above_context() {
        let context_size = NonZeroU32::new(3).expect("test context must be non-zero");
        let mut tokens = vec![1, 2, 3, 4];

        assert!(truncate_tokens_to_context(&mut tokens, context_size));
        assert_eq!(tokens, vec![1, 2, 3]);
    }

    /// Exercises the real default GGUF without making the 300 MB artifact a CI dependency.
    ///
    /// Run with:
    /// `MOLTIS_TEST_LOCAL_MODEL=/path/to/model.gguf cargo test -p moltis-memory \
    ///   --features local-embeddings real_default_model_handles_context_edges_and_indexing \
    ///   -- --ignored`
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn real_default_model_handles_context_edges_and_indexing() {
        use crate::{
            config::MemoryConfig, manager::MemoryManager, schema::run_migrations,
            store_sqlite::SqliteMemoryStore,
        };

        let model_path = std::env::var("MOLTIS_TEST_LOCAL_MODEL")
            .expect("MOLTIS_TEST_LOCAL_MODEL must point to the default EmbeddingGemma GGUF");
        let backend = LlamaBackend::init().expect("llama backend should initialize");
        let model = LlamaModel::load_from_file(
            &backend,
            PathBuf::from(model_path),
            &LlamaModelParams::default(),
        )
        .expect("default embedding model should load");
        let within_context = "Uma memória semântica em português brasileiro. ".repeat(160);
        let within_context_tokens = model
            .str_to_token(&within_context, llama_cpp_2::model::AddBos::Always)
            .expect("test text should tokenize");

        assert!(
            within_context_tokens.len() > 512,
            "fixture must reproduce the old limit"
        );
        assert!(
            within_context_tokens.len() <= model.n_ctx_train() as usize,
            "fixture must fit the model context"
        );

        assert_valid_embedding(
            embed_sync(&backend, &model, &within_context)
                .expect("input within the model context should embed"),
        );

        let over_context = "Conteúdo longo para testar um limite seguro. ".repeat(600);
        let over_context_tokens = model
            .str_to_token(&over_context, llama_cpp_2::model::AddBos::Always)
            .expect("oversized test text should tokenize");
        assert!(
            over_context_tokens.len() > model.n_ctx_train() as usize,
            "fixture must exceed the model context"
        );
        assert_valid_embedding(
            embed_sync(&backend, &model, &over_context)
                .expect("oversized input should be truncated instead of aborting"),
        );

        assert_valid_embedding(
            embed_sync(&backend, &model, "")
                .expect("empty text should still embed its tokenizer special token"),
        );
        assert_valid_embedding(
            embed_sync(
                &backend,
                &model,
                "Texto Unicode: informação, coração, ação, 日本語, 🧠 e 🚀.",
            )
            .expect("multilingual Unicode text should embed"),
        );

        let memory_dir = tempfile::tempdir().expect("temporary memory directory should exist");
        let garden_text = "O manjericão é uma erva aromática que cresce bem em vasos com luz suave, solo úmido e rega equilibrada. ".repeat(40);
        let garden_tokens = model
            .str_to_token(&garden_text, llama_cpp_2::model::AddBos::Always)
            .expect("synthetic memory should tokenize");
        assert!(
            garden_tokens.len() > 512,
            "indexed fixture must reproduce the old limit"
        );
        assert!(
            garden_tokens.len() <= model.n_ctx_train() as usize,
            "indexed fixture must fit the model context"
        );

        std::fs::write(memory_dir.path().join("garden.md"), garden_text)
            .expect("synthetic garden memory should be written");
        std::fs::write(
            memory_dir.path().join("systems.md"),
            "Backups de banco de dados precisam de retenção, verificação e testes de restauração.",
        )
        .expect("synthetic systems memory should be written");

        let provider = LocalGgufEmbeddingProvider {
            backend: SendSyncBackend(backend),
            model: Mutex::new(model),
            dims: DEFAULT_DIMS,
        };
        let pool = sqlx::SqlitePool::connect(":memory:")
            .await
            .expect("in-memory SQLite should connect");
        run_migrations(&pool)
            .await
            .expect("memory migrations should run");
        let config = MemoryConfig {
            db_path: ":memory:".into(),
            memory_dirs: vec![memory_dir.path().to_path_buf()],
            ..Default::default()
        };
        let manager = MemoryManager::new(
            config,
            Box::new(SqliteMemoryStore::new(pool)),
            Box::new(provider),
        );

        let report = manager
            .sync()
            .await
            .expect("synthetic memories should index");
        assert_eq!(report.files_updated, 2);
        assert_eq!(report.errors, 0);
        assert!(report.cache_misses >= 2);

        let status = manager.status().await.expect("memory status should load");
        assert_eq!(status.total_files, 2);
        assert!(status.total_chunks >= 2);
        assert_eq!(status.embedding_model, "local-gguf");

        let results = manager
            .search(
                "Qual erva aromática cresce bem dentro de casa em um vaso?",
                2,
            )
            .await
            .expect("Portuguese semantic search should succeed");
        let first = results.first().expect("search should return a result");
        assert!(
            first.text.contains("manjericão"),
            "the semantically related memory should rank first"
        );

        let unchanged = manager
            .sync()
            .await
            .expect("repeated sync should remain healthy");
        assert_eq!(unchanged.files_unchanged, 2);
        assert_eq!(unchanged.errors, 0);
    }

    fn assert_valid_embedding(embedding: Vec<f32>) {
        assert_eq!(embedding.len(), DEFAULT_DIMS);
        assert!(embedding.iter().all(|value| value.is_finite()));
    }
}
