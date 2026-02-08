//! Local GGUF LLM provider using llama-cpp-2.
//!
//! Provides offline LLM inference via quantized GGUF models. Supports automatic
//! model download from HuggingFace and system memory detection for model suggestions.
//!
//! Requires the `local-llm` feature flag and CMake + C++ compiler at build time.

pub mod chat_templates;
pub mod models;
pub mod system_info;

use std::{num::NonZeroU32, path::PathBuf, pin::Pin, sync::Arc, time::Instant};

use {
    anyhow::{Context, Result, bail},
    async_trait::async_trait,
    llama_cpp_2::{
        context::params::LlamaContextParams,
        llama_backend::LlamaBackend,
        llama_batch::LlamaBatch,
        model::{LlamaModel, params::LlamaModelParams},
        sampling::LlamaSampler,
        token::LlamaToken,
    },
    tokio::sync::Mutex,
    tokio_stream::Stream,
    tracing::{debug, info, warn},
};

#[cfg(feature = "metrics")]
use moltis_metrics::{gauge, histogram, labels, llm as llm_metrics};

use crate::model::{ChatMessage, CompletionResponse, LlmProvider, StreamEvent, Usage};

use {
    chat_templates::{ChatTemplateHint, format_messages},
    models::{GgufModelDef, find_model},
};

/// Wrapper around `LlamaBackend` that opts into `Send + Sync`.
///
/// `LlamaBackend` is `!Send` because `llama-cpp-2` doesn't mark its FFI
/// handle as thread-safe. In practice the backend is an opaque init token
/// with no mutable state after construction, so sharing across threads is
/// safe. Wrapping it in a newtype keeps the `unsafe` declaration localised.
struct SendSyncBackend(LlamaBackend);

// SAFETY: LlamaBackend is an immutable init handle with no thread-local state.
unsafe impl Send for SendSyncBackend {}
unsafe impl Sync for SendSyncBackend {}

/// Configuration for the local GGUF provider.
#[derive(Debug, Clone)]
pub struct LocalGgufConfig {
    /// Model ID from the registry, or custom model name.
    pub model_id: String,
    /// Direct path to a GGUF file (skips auto-download).
    pub model_path: Option<PathBuf>,
    /// Context size in tokens (default: from model definition or 8192).
    pub context_size: Option<u32>,
    /// Number of layers to offload to GPU (0 = CPU only).
    pub gpu_layers: u32,
    /// Sampling temperature.
    pub temperature: f32,
    /// Directory for caching downloaded models.
    pub cache_dir: PathBuf,
}

impl Default for LocalGgufConfig {
    fn default() -> Self {
        Self {
            model_id: String::new(),
            model_path: None,
            context_size: None,
            gpu_layers: 0,
            temperature: 0.7,
            cache_dir: models::default_models_dir(),
        }
    }
}

/// Local GGUF LLM provider.
pub struct LocalGgufProvider {
    backend: Arc<SendSyncBackend>,
    model: Arc<Mutex<LlamaModel>>,
    model_id: String,
    model_def: Option<&'static GgufModelDef>,
    context_size: u32,
    temperature: f32,
}

impl LocalGgufProvider {
    /// Create a new provider with an already-loaded model.
    fn new_with_model(
        backend: LlamaBackend,
        model: LlamaModel,
        model_id: String,
        model_def: Option<&'static GgufModelDef>,
        context_size: u32,
        temperature: f32,
    ) -> Self {
        info!(
            model = %model_id,
            context_size,
            "local GGUF provider initialized"
        );
        Self {
            backend: Arc::new(SendSyncBackend(backend)),
            model: Arc::new(Mutex::new(model)),
            model_id,
            model_def,
            context_size,
            temperature,
        }
    }

    /// Load a provider from configuration.
    ///
    /// This will download the model if needed.
    pub async fn from_config(config: LocalGgufConfig) -> Result<Self> {
        let load_start = Instant::now();

        // Resolve model path
        let (model_path, model_def) = if let Some(path) = config.model_path {
            // User provided direct path
            if !path.exists() {
                bail!("model file not found: {}", path.display());
            }
            (path, find_model(&config.model_id))
        } else {
            // Look up in registry and download if needed
            let Some(def) = find_model(&config.model_id) else {
                bail!(
                    "unknown model '{}'. Use model_path for custom GGUF files.",
                    config.model_id
                );
            };

            // MLX models require the MLX backend (Python mlx_lm), not llama.cpp
            if matches!(def.backend, models::ModelBackend::Mlx) {
                bail!(
                    "Model '{}' requires the MLX backend which is not available in this context. \
                     MLX models need Python with mlx_lm installed. \
                     Please select a GGUF model instead (e.g., 'llama-3.2-1b-q4_k_m').",
                    config.model_id
                );
            }

            let path = models::ensure_model(def, &config.cache_dir).await?;
            (path, Some(def))
        };

        // Determine context size
        let context_size = config
            .context_size
            .or_else(|| model_def.map(|d| d.context_window))
            .unwrap_or(8192);

        // Load the model
        debug!(model = %config.model_id, "initializing llama backend");
        let backend = LlamaBackend::init().context("initializing llama backend")?;

        let mut model_params = LlamaModelParams::default();

        // GPU offloading
        if config.gpu_layers > 0 {
            model_params = model_params.with_n_gpu_layers(config.gpu_layers);
            info!(gpu_layers = config.gpu_layers, "GPU offloading enabled");
        }

        debug!(
            path = %model_path.display(),
            model = %config.model_id,
            "loading GGUF model file"
        );
        let model = LlamaModel::load_from_file(&backend, &model_path, &model_params)
            .map_err(|e| anyhow::anyhow!("failed to load GGUF model: {e}"))?;

        let load_duration = load_start.elapsed();

        info!(
            path = %model_path.display(),
            model = %config.model_id,
            context_size,
            load_duration_secs = load_duration.as_secs_f64(),
            "loaded local GGUF model"
        );

        // Record model load metrics
        #[cfg(feature = "metrics")]
        {
            // Use a local-llm specific metric for model loading
            histogram!(
                "moltis_local_llm_model_load_duration_seconds",
                labels::MODEL => config.model_id.clone()
            )
            .record(load_duration.as_secs_f64());

            gauge!(
                "moltis_local_llm_models_loaded",
                labels::MODEL => config.model_id.clone()
            )
            .increment(1.0);
        }

        Ok(Self::new_with_model(
            backend,
            model,
            config.model_id,
            model_def,
            context_size,
            config.temperature,
        ))
    }

    /// Get the chat template hint for this model.
    fn chat_template(&self) -> ChatTemplateHint {
        self.model_def
            .map(|d| d.chat_template)
            .unwrap_or(ChatTemplateHint::Auto)
    }

    /// Generate text synchronously (called from blocking context).
    fn generate_sync(&self, prompt: &str, max_tokens: u32) -> Result<(String, u32, u32)> {
        let generation_start = Instant::now();
        let model = self.model.blocking_lock();
        let backend = &self.backend.0;

        // Batch size for processing (default in llama.cpp is 2048, we use 512 for safety)
        let batch_size: usize = 512;

        // Create context
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(self.context_size))
            .with_n_batch(batch_size as u32);
        let mut ctx = model
            .new_context(backend, ctx_params)
            .map_err(|e| anyhow::anyhow!("failed to create llama context: {e}"))?;

        // Tokenize prompt
        let tokens = model
            .str_to_token(prompt, llama_cpp_2::model::AddBos::Always)
            .map_err(|e| anyhow::anyhow!("tokenization failed: {e}"))?;

        let input_tokens = tokens.len() as u32;
        debug!(input_tokens, batch_size, "tokenized prompt");

        if tokens.is_empty() {
            bail!("empty token sequence");
        }

        // Process prompt in batches to avoid exceeding n_batch
        debug!(
            num_chunks = tokens.len().div_ceil(batch_size),
            "processing prompt"
        );
        let mut batch = LlamaBatch::new(batch_size, 1);
        for (chunk_idx, chunk) in tokens.chunks(batch_size).enumerate() {
            batch.clear();
            let chunk_start = chunk_idx * batch_size;
            let is_last_chunk = chunk_start + chunk.len() == tokens.len();

            for (i, &token) in chunk.iter().enumerate() {
                let pos = (chunk_start + i) as i32;
                // Only compute logits for the very last token
                let is_last = is_last_chunk && i == chunk.len() - 1;
                batch
                    .add(token, pos, &[0], is_last)
                    .map_err(|e| anyhow::anyhow!("batch add failed: {e}"))?;
            }

            ctx.decode(&mut batch)
                .map_err(|e| anyhow::anyhow!("prompt decode failed: {e}"))?;
        }

        let prompt_eval_time = generation_start.elapsed();
        debug!(
            prompt_eval_secs = prompt_eval_time.as_secs_f64(),
            "prompt evaluation complete"
        );

        // Set up sampler chain: temperature -> random distribution
        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::temp(self.temperature),
            LlamaSampler::dist(42), // Seed
        ]);

        // Generate tokens
        let token_gen_start = Instant::now();
        let mut output_tokens = Vec::new();
        let mut pos = tokens.len() as i32;
        let eos_token = model.token_eos();

        for _ in 0..max_tokens {
            // Sample from the last position in the batch
            let token = sampler.sample(&ctx, batch.n_tokens() - 1);

            if token == eos_token {
                debug!("reached EOS token");
                break;
            }

            output_tokens.push(token);
            sampler.accept(token);

            // Decode next token
            batch.clear();
            batch
                .add(token, pos, &[0], true)
                .map_err(|e| anyhow::anyhow!("batch add token failed: {e}"))?;
            ctx.decode(&mut batch)
                .map_err(|e| anyhow::anyhow!("token decode failed: {e}"))?;

            pos += 1;
        }

        let token_gen_duration = token_gen_start.elapsed();
        let total_duration = generation_start.elapsed();
        let output_token_count = output_tokens.len() as u32;

        // Calculate tokens per second
        let tokens_per_sec = if token_gen_duration.as_secs_f64() > 0.0 {
            output_token_count as f64 / token_gen_duration.as_secs_f64()
        } else {
            0.0
        };

        debug!(
            output_tokens = output_token_count,
            generation_secs = token_gen_duration.as_secs_f64(),
            total_secs = total_duration.as_secs_f64(),
            tokens_per_sec = format!("{:.1}", tokens_per_sec),
            "generation complete"
        );

        // Detokenize output
        let output_text = detokenize(&model, &output_tokens)?;

        Ok((output_text, input_tokens, output_token_count))
    }
}

/// Detokenize a sequence of tokens into a string.
fn detokenize(model: &LlamaModel, tokens: &[LlamaToken]) -> Result<String> {
    let mut decoder = encoding_rs::UTF_8.new_decoder();
    let mut output = String::new();
    for &token in tokens {
        let piece = model
            .token_to_piece(token, &mut decoder, true, None)
            .map_err(|e| anyhow::anyhow!("detokenization failed: {e}"))?;
        output.push_str(&piece);
    }
    Ok(output)
}

#[async_trait]
impl LlmProvider for LocalGgufProvider {
    fn name(&self) -> &str {
        "local-llm"
    }

    fn id(&self) -> &str {
        &self.model_id
    }

    fn context_window(&self) -> u32 {
        self.context_size
    }

    fn supports_tools(&self) -> bool {
        // Tool calling requires grammar-constrained generation
        false
    }

    async fn complete(
        &self,
        messages: &[ChatMessage],
        _tools: &[serde_json::Value],
    ) -> Result<CompletionResponse> {
        let prompt = format_messages(messages, self.chat_template());
        let max_tokens = 4096u32;

        // Clone what we need for the blocking task
        let backend = Arc::clone(&self.backend);
        let model = Arc::clone(&self.model);
        let context_size = self.context_size;
        let temperature = self.temperature;
        let model_id = self.model_id.clone();
        let model_def = self.model_def;

        // Run generation in blocking context
        let (text, input_tokens, output_tokens) = tokio::task::spawn_blocking(move || {
            let provider = LocalGgufProvider {
                backend,
                model,
                model_id,
                model_def,
                context_size,
                temperature,
            };
            provider.generate_sync(&prompt, max_tokens)
        })
        .await
        .context("generation task panicked")??;

        Ok(CompletionResponse {
            text: Some(text),
            tool_calls: vec![],
            usage: Usage {
                input_tokens,
                output_tokens,
            },
        })
    }

    fn stream(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        let prompt = format_messages(&messages, self.chat_template());
        let max_tokens = 4096u32;

        let backend = Arc::clone(&self.backend);
        let model = Arc::clone(&self.model);
        let context_size = self.context_size;
        let temperature = self.temperature;

        Box::pin(async_stream::stream! {
            let (tx, mut rx) = tokio::sync::mpsc::channel::<StreamEvent>(32);

            // Spawn blocking generation task
            let handle = tokio::task::spawn_blocking(move || {
                stream_generate_sync(
                    &backend.0,
                    &model,
                    &prompt,
                    max_tokens,
                    context_size,
                    temperature,
                    tx,
                )
            });

            // Yield events from the channel
            while let Some(event) = rx.recv().await {
                let is_done = matches!(event, StreamEvent::Done(_) | StreamEvent::Error(_));
                yield event;
                if is_done {
                    break;
                }
            }

            // Wait for the blocking task to complete
            if let Err(e) = handle.await {
                warn!("generation task error: {e}");
            }
        })
    }
}

/// Streaming generation in a blocking context.
fn stream_generate_sync(
    backend: &LlamaBackend,
    model: &Mutex<LlamaModel>,
    prompt: &str,
    max_tokens: u32,
    context_size: u32,
    temperature: f32,
    tx: tokio::sync::mpsc::Sender<StreamEvent>,
) {
    let generation_start = Instant::now();
    // Batch size for processing (default in llama.cpp is 2048, we use 512 for safety)
    let batch_size: usize = 512;

    let result = (|| -> Result<(u32, u32)> {
        let model = model.blocking_lock();

        // Create context
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(context_size))
            .with_n_batch(batch_size as u32);
        let mut ctx = model
            .new_context(backend, ctx_params)
            .map_err(|e| anyhow::anyhow!("failed to create llama context: {e}"))?;

        // Tokenize prompt
        let tokens = model
            .str_to_token(prompt, llama_cpp_2::model::AddBos::Always)
            .map_err(|e| anyhow::anyhow!("tokenization failed: {e}"))?;

        let input_tokens = tokens.len() as u32;
        debug!(input_tokens, "tokenized prompt for streaming");

        if tokens.is_empty() {
            bail!("empty token sequence");
        }

        // Process prompt in batches to avoid exceeding n_batch
        let mut batch = LlamaBatch::new(batch_size, 1);
        for (chunk_idx, chunk) in tokens.chunks(batch_size).enumerate() {
            batch.clear();
            let chunk_start = chunk_idx * batch_size;
            let is_last_chunk = chunk_start + chunk.len() == tokens.len();

            for (i, &token) in chunk.iter().enumerate() {
                let pos = (chunk_start + i) as i32;
                // Only compute logits for the very last token
                let is_last = is_last_chunk && i == chunk.len() - 1;
                batch
                    .add(token, pos, &[0], is_last)
                    .map_err(|e| anyhow::anyhow!("batch add failed: {e}"))?;
            }

            ctx.decode(&mut batch)
                .map_err(|e| anyhow::anyhow!("prompt decode failed: {e}"))?;
        }

        let prompt_eval_time = generation_start.elapsed();
        debug!(
            prompt_eval_secs = prompt_eval_time.as_secs_f64(),
            "prompt evaluation complete, starting token generation"
        );

        // Set up sampler chain: temperature -> random distribution
        let mut sampler =
            LlamaSampler::chain_simple([LlamaSampler::temp(temperature), LlamaSampler::dist(42)]);

        // Generate tokens
        let token_gen_start = Instant::now();
        let mut first_token_time: Option<std::time::Duration> = None;
        let mut output_tokens = 0u32;
        let mut pos = tokens.len() as i32;
        let eos_token = model.token_eos();
        let mut decoder = encoding_rs::UTF_8.new_decoder();

        for _ in 0..max_tokens {
            // Sample from the last position in the batch
            let token = sampler.sample(&ctx, batch.n_tokens() - 1);

            if token == eos_token {
                debug!("reached EOS token");
                break;
            }

            output_tokens += 1;

            // Record time to first token
            if first_token_time.is_none() {
                first_token_time = Some(generation_start.elapsed());
                debug!(
                    ttft_secs = first_token_time.unwrap().as_secs_f64(),
                    "time to first token"
                );

                #[cfg(feature = "metrics")]
                histogram!(llm_metrics::TIME_TO_FIRST_TOKEN_SECONDS)
                    .record(first_token_time.unwrap().as_secs_f64());
            }

            sampler.accept(token);

            // Detokenize and send
            let piece = model
                .token_to_piece(token, &mut decoder, true, None)
                .map_err(|e| anyhow::anyhow!("detokenization failed: {e}"))?;

            if tx.blocking_send(StreamEvent::Delta(piece)).is_err() {
                // Receiver dropped, stop generation
                debug!("receiver dropped, stopping generation");
                break;
            }

            // Decode next token
            batch.clear();
            batch
                .add(token, pos, &[0], true)
                .map_err(|e| anyhow::anyhow!("batch add token failed: {e}"))?;
            ctx.decode(&mut batch)
                .map_err(|e| anyhow::anyhow!("token decode failed: {e}"))?;

            pos += 1;
        }

        let token_gen_duration = token_gen_start.elapsed();
        let total_duration = generation_start.elapsed();

        // Calculate and log tokens per second
        let tokens_per_sec = if token_gen_duration.as_secs_f64() > 0.0 {
            output_tokens as f64 / token_gen_duration.as_secs_f64()
        } else {
            0.0
        };

        debug!(
            output_tokens,
            generation_secs = token_gen_duration.as_secs_f64(),
            total_secs = total_duration.as_secs_f64(),
            tokens_per_sec = format!("{:.1}", tokens_per_sec),
            "streaming generation complete"
        );

        #[cfg(feature = "metrics")]
        if tokens_per_sec > 0.0 {
            histogram!(llm_metrics::TOKENS_PER_SECOND).record(tokens_per_sec);
        }

        Ok((input_tokens, output_tokens))
    })();

    match result {
        Ok((input_tokens, output_tokens)) => {
            let _ = tx.blocking_send(StreamEvent::Done(Usage {
                input_tokens,
                output_tokens,
            }));
        },
        Err(e) => {
            warn!(error = %e, "streaming generation failed");
            let _ = tx.blocking_send(StreamEvent::Error(e.to_string()));
        },
    }
}

/// Lazy-loading wrapper for LocalGgufProvider.
///
/// The actual model is loaded on first use (complete/stream call).
/// This allows registration at startup without blocking on model download.
pub struct LazyLocalGgufProvider {
    config: LocalGgufConfig,
    inner: tokio::sync::RwLock<Option<LocalGgufProvider>>,
}

impl LazyLocalGgufProvider {
    /// Create a new lazy provider with the given config.
    pub fn new(config: LocalGgufConfig) -> Self {
        Self {
            config,
            inner: tokio::sync::RwLock::new(None),
        }
    }

    /// Ensure the inner provider is loaded, loading it if necessary.
    async fn ensure_loaded(&self) -> Result<()> {
        // Check if already loaded (read lock is cheaper)
        {
            let guard = self.inner.read().await;
            if guard.is_some() {
                return Ok(());
            }
        }

        // Need to load - acquire write lock
        let mut guard = self.inner.write().await;

        // Double-check after acquiring write lock
        if guard.is_some() {
            return Ok(());
        }

        info!(model = %self.config.model_id, "loading local GGUF model on first use");
        let provider = LocalGgufProvider::from_config(self.config.clone()).await?;
        *guard = Some(provider);
        Ok(())
    }
}

#[async_trait]
impl LlmProvider for LazyLocalGgufProvider {
    fn name(&self) -> &str {
        "local-llm"
    }

    fn id(&self) -> &str {
        &self.config.model_id
    }

    fn context_window(&self) -> u32 {
        // Use model definition if available, otherwise default
        models::find_model(&self.config.model_id)
            .map(|d| d.context_window)
            .unwrap_or(8192)
    }

    fn supports_tools(&self) -> bool {
        false
    }

    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> Result<CompletionResponse> {
        self.ensure_loaded().await?;
        let guard = self.inner.read().await;
        let provider = guard.as_ref().expect("provider should be loaded");
        provider.complete(messages, tools).await
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

            let guard = self.inner.read().await;
            let provider = guard.as_ref().expect("provider should be loaded");

            // We need to create a stream from the inner provider
            // But we can't hold the guard across the stream, so we need to
            // copy the necessary state
            let backend = Arc::clone(&provider.backend);
            let model = Arc::clone(&provider.model);
            let context_size = provider.context_size;
            let temperature = provider.temperature;
            let chat_template = provider.chat_template();

            drop(guard);

            let prompt = format_messages(&messages, chat_template);
            let max_tokens = 4096u32;

            let (tx, mut rx) = tokio::sync::mpsc::channel::<StreamEvent>(32);

            let handle = tokio::task::spawn_blocking(move || {
                stream_generate_sync(
                    &backend.0,
                    &model,
                    &prompt,
                    max_tokens,
                    context_size,
                    temperature,
                    tx,
                )
            });

            while let Some(event) = rx.recv().await {
                let is_done = matches!(event, StreamEvent::Done(_) | StreamEvent::Error(_));
                yield event;
                if is_done {
                    break;
                }
            }

            if let Err(e) = handle.await {
                warn!("generation task error: {e}");
            }
        })
    }
}

/// Log system info and suggest models on startup.
pub fn log_system_info_and_suggestions() {
    let sys = system_info::SystemInfo::detect();
    let tier = sys.memory_tier();
    let cache_dir = models::default_models_dir();

    info!(
        total_ram_gb = sys.total_ram_gb(),
        available_ram_gb = sys.available_ram_gb(),
        has_metal = sys.has_metal,
        has_cuda = sys.has_cuda,
        tier = %tier,
        "local-llm system info"
    );
    info!(
        cache_dir = %cache_dir.display(),
        "local-llm model cache directory"
    );

    if let Some(suggested) = models::suggest_model(tier) {
        info!(
            model = suggested.id,
            display_name = suggested.display_name,
            min_ram_gb = suggested.min_ram_gb,
            "suggested local model for your system"
        );
    }

    let cached_ids: Vec<&str> = models::MODEL_REGISTRY
        .iter()
        .filter(|m| models::is_model_cached(m, &cache_dir))
        .map(|m| m.id)
        .collect();
    info!(
        cached_models = ?cached_ids,
        cached_count = cached_ids.len(),
        "cached local models in model cache directory"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = LocalGgufConfig::default();
        assert!(config.model_id.is_empty());
        assert!(config.model_path.is_none());
        assert_eq!(config.gpu_layers, 0);
        assert!((config.temperature - 0.7).abs() < 0.01);
    }

    #[test]
    fn test_chat_template_selection() {
        // When model_def is None, should return Auto
        // (Can't test with actual provider without loading a model)
    }
}
