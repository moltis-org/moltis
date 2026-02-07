//! Local LLM backend trait and implementations.
//!
//! Backends handle the actual model loading and inference.

use std::pin::Pin;

use {anyhow::Result, async_trait::async_trait, tokio_stream::Stream};

use crate::model::{ChatMessage, CompletionResponse, StreamEvent};

use super::LocalLlmConfig;

/// Types of local LLM backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendType {
    /// GGUF format via llama.cpp - cross-platform
    Gguf,
    /// MLX format - Apple Silicon optimized
    Mlx,
}

impl BackendType {
    /// Human-readable name for this backend.
    #[must_use]
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Gguf => "GGUF (llama.cpp)",
            Self::Mlx => "MLX (Apple)",
        }
    }

    /// Whether this backend is optimized for the current platform.
    #[must_use]
    pub fn is_native(&self) -> bool {
        match self {
            Self::Gguf => true, // Works everywhere
            Self::Mlx => cfg!(target_os = "macos") && cfg!(target_arch = "aarch64"),
        }
    }
}

impl std::fmt::Display for BackendType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// Trait for local LLM inference backends.
#[async_trait]
pub trait LocalBackend: Send + Sync {
    /// Get the backend type.
    fn backend_type(&self) -> BackendType;

    /// Get the model ID.
    fn model_id(&self) -> &str;

    /// Get the context window size.
    fn context_window(&self) -> u32;

    /// Run completion (non-streaming).
    async fn complete(&self, messages: &[ChatMessage]) -> Result<CompletionResponse>;

    /// Run streaming completion.
    fn stream<'a>(
        &'a self,
        messages: &'a [ChatMessage],
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + 'a>>;
}

/// Detect the best backend for the current system (ignoring model type).
#[must_use]
pub fn detect_best_backend() -> BackendType {
    let sys = super::system_info::SystemInfo::detect();

    // On Apple Silicon, prefer MLX if available
    if sys.is_apple_silicon && is_mlx_available() {
        return BackendType::Mlx;
    }

    // Default to GGUF (always available when compiled with local-llm feature)
    BackendType::Gguf
}

/// Detect the best backend for a specific model.
///
/// This checks:
/// 1. Legacy MLX models (mlx-* prefix) -> requires MLX backend
/// 2. Unified registry models with MLX support -> prefers MLX on Apple Silicon
/// 3. GGUF-only models -> uses GGUF backend
/// 4. Unknown models -> falls back to system detection
#[must_use]
pub fn detect_backend_for_model(model_id: &str) -> BackendType {
    // Check legacy MLX models first (from local_gguf registry)
    if let Some(def) = crate::providers::local_gguf::models::find_model(model_id) {
        if matches!(
            def.backend,
            crate::providers::local_gguf::models::ModelBackend::Mlx
        ) {
            // MLX model from legacy registry - requires MLX backend
            if is_mlx_available() {
                return BackendType::Mlx;
            } else {
                // MLX not available but model requires it - log warning, return Mlx anyway
                // (will fail with a clear error when loading)
                tracing::warn!(
                    model = model_id,
                    "MLX model selected but MLX backend not available"
                );
                return BackendType::Mlx;
            }
        }
        // GGUF model from legacy registry
        return BackendType::Gguf;
    }

    // Check unified registry
    if let Some(def) = super::models::find_model(model_id) {
        // If model has MLX support and MLX is available, prefer MLX
        if def.has_mlx() && is_mlx_available() {
            return BackendType::Mlx;
        }
        // Otherwise use GGUF
        return BackendType::Gguf;
    }

    // Unknown model - fall back to system detection
    detect_best_backend()
}

/// Get list of available backends on this system.
#[must_use]
pub fn available_backends() -> Vec<BackendType> {
    let mut backends = vec![BackendType::Gguf]; // Always available

    if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") && is_mlx_available() {
        backends.push(BackendType::Mlx);
    }

    backends
}

/// How mlx-lm is installed on this system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MlxInstallation {
    /// Installed as a Python package (pip install mlx-lm)
    PythonPackage,
    /// Installed as a standalone command (brew install mlx-lm)
    HomebrewCli,
}

/// Detect how mlx-lm is installed, if at all.
#[must_use]
pub fn detect_mlx_installation() -> Option<MlxInstallation> {
    // Check if we're on Apple Silicon macOS
    if !(cfg!(target_os = "macos") && cfg!(target_arch = "aarch64")) {
        return None;
    }

    // Method 1: Check if mlx_lm can be imported in Python (pip install)
    let python_available = std::process::Command::new("python3")
        .args(["-c", "import mlx_lm"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if python_available {
        return Some(MlxInstallation::PythonPackage);
    }

    // Method 2: Check if mlx_lm command exists (Homebrew installation)
    let cli_available = std::process::Command::new("which")
        .arg("mlx_lm")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if cli_available {
        return Some(MlxInstallation::HomebrewCli);
    }

    None
}

/// Check if MLX backend is available.
#[must_use]
pub fn is_mlx_available() -> bool {
    detect_mlx_installation().is_some()
}

/// Create a backend instance for the given type and config.
pub async fn create_backend(
    backend_type: BackendType,
    config: &LocalLlmConfig,
) -> Result<Box<dyn LocalBackend>> {
    match backend_type {
        BackendType::Gguf => {
            let backend = gguf::GgufBackend::from_config(config).await?;
            Ok(Box::new(backend))
        },
        BackendType::Mlx => {
            let backend = mlx::MlxBackend::from_config(config).await?;
            Ok(Box::new(backend))
        },
    }
}

// ── GGUF Backend ─────────────────────────────────────────────────────────────

pub mod gguf {
    //! GGUF backend using llama-cpp-2.

    use std::{num::NonZeroU32, pin::Pin, sync::Arc};

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

    use crate::model::{ChatMessage, CompletionResponse, StreamEvent, Usage};

    use {
        super::{BackendType, LocalBackend, LocalLlmConfig},
        crate::providers::local_llm::models::{
            self, LocalModelDef,
            chat_templates::{ChatTemplateHint, format_messages},
        },
    };

    /// Wrapper around `LlamaBackend` that opts into `Send + Sync`.
    struct SendSyncBackend(LlamaBackend);

    // SAFETY: LlamaBackend is an immutable init handle with no thread-local state.
    unsafe impl Send for SendSyncBackend {}
    unsafe impl Sync for SendSyncBackend {}

    /// GGUF backend implementation.
    pub struct GgufBackend {
        backend: Arc<SendSyncBackend>,
        model: Arc<Mutex<LlamaModel>>,
        model_id: String,
        model_def: Option<&'static LocalModelDef>,
        context_size: u32,
        temperature: f32,
    }

    impl GgufBackend {
        /// Load a GGUF backend from configuration.
        pub async fn from_config(config: &LocalLlmConfig) -> Result<Self> {
            // Resolve model path
            let (model_path, model_def) = if let Some(path) = &config.model_path {
                if !path.exists() {
                    bail!("model file not found: {}", path.display());
                }
                (path.clone(), models::find_model(&config.model_id))
            } else {
                let Some(def) = models::find_model(&config.model_id) else {
                    bail!(
                        "unknown model '{}'. Use model_path for custom GGUF files.",
                        config.model_id
                    );
                };
                let path = models::ensure_model(def, &config.cache_dir).await?;
                (path, Some(def))
            };

            // Determine context size
            let context_size = config
                .context_size
                .or_else(|| model_def.map(|d| d.context_window))
                .unwrap_or(8192);

            // Load the model
            let backend = LlamaBackend::init().context("initializing llama backend")?;

            let mut model_params = LlamaModelParams::default();

            if config.gpu_layers > 0 {
                model_params = model_params.with_n_gpu_layers(config.gpu_layers);
                info!(gpu_layers = config.gpu_layers, "GPU offloading enabled");
            }

            let model = LlamaModel::load_from_file(&backend, &model_path, &model_params)
                .map_err(|e| anyhow::anyhow!("failed to load GGUF model: {e}"))?;

            info!(
                path = %model_path.display(),
                model = %config.model_id,
                context_size,
                "loaded GGUF model"
            );

            Ok(Self {
                backend: Arc::new(SendSyncBackend(backend)),
                model: Arc::new(Mutex::new(model)),
                model_id: config.model_id.clone(),
                model_def,
                context_size,
                temperature: config.temperature,
            })
        }

        /// Get the chat template hint for this model.
        fn chat_template(&self) -> ChatTemplateHint {
            self.model_def
                .and_then(|d| d.chat_template)
                .unwrap_or(ChatTemplateHint::Auto)
        }

        /// Generate text synchronously.
        fn generate_sync(&self, prompt: &str, max_tokens: u32) -> Result<(String, u32, u32)> {
            let model = self.model.blocking_lock();
            let backend = &self.backend.0;

            let batch_size: usize = 512;

            let ctx_params = LlamaContextParams::default()
                .with_n_ctx(NonZeroU32::new(self.context_size))
                .with_n_batch(batch_size as u32);
            let mut ctx = model
                .new_context(backend, ctx_params)
                .map_err(|e| anyhow::anyhow!("failed to create llama context: {e}"))?;

            let tokens = model
                .str_to_token(prompt, llama_cpp_2::model::AddBos::Always)
                .map_err(|e| anyhow::anyhow!("tokenization failed: {e}"))?;

            let input_tokens = tokens.len() as u32;
            debug!(input_tokens, batch_size, "tokenized prompt");

            if tokens.is_empty() {
                bail!("empty token sequence");
            }

            // Process prompt in batches
            let mut batch = LlamaBatch::new(batch_size, 1);
            for (chunk_idx, chunk) in tokens.chunks(batch_size).enumerate() {
                batch.clear();
                let chunk_start = chunk_idx * batch_size;
                let is_last_chunk = chunk_start + chunk.len() == tokens.len();

                for (i, &token) in chunk.iter().enumerate() {
                    let pos = (chunk_start + i) as i32;
                    let is_last = is_last_chunk && i == chunk.len() - 1;
                    batch
                        .add(token, pos, &[0], is_last)
                        .map_err(|e| anyhow::anyhow!("batch add failed: {e}"))?;
                }

                ctx.decode(&mut batch)
                    .map_err(|e| anyhow::anyhow!("prompt decode failed: {e}"))?;
            }

            let mut sampler = LlamaSampler::chain_simple([
                LlamaSampler::temp(self.temperature),
                LlamaSampler::dist(42),
            ]);

            let mut output_tokens = Vec::new();
            let mut pos = tokens.len() as i32;
            let eos_token = model.token_eos();

            for _ in 0..max_tokens {
                let token = sampler.sample(&ctx, batch.n_tokens() - 1);

                if token == eos_token {
                    debug!("reached EOS token");
                    break;
                }

                output_tokens.push(token);
                sampler.accept(token);

                batch.clear();
                batch
                    .add(token, pos, &[0], true)
                    .map_err(|e| anyhow::anyhow!("batch add token failed: {e}"))?;
                ctx.decode(&mut batch)
                    .map_err(|e| anyhow::anyhow!("token decode failed: {e}"))?;

                pos += 1;
            }

            let output_text = detokenize(&model, &output_tokens)?;

            Ok((output_text, input_tokens, output_tokens.len() as u32))
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
    impl LocalBackend for GgufBackend {
        fn backend_type(&self) -> BackendType {
            BackendType::Gguf
        }

        fn model_id(&self) -> &str {
            &self.model_id
        }

        fn context_window(&self) -> u32 {
            self.context_size
        }

        async fn complete(&self, messages: &[ChatMessage]) -> Result<CompletionResponse> {
            let prompt = format_messages(messages, self.chat_template());
            let max_tokens = 4096u32;

            let backend = Arc::clone(&self.backend);
            let model = Arc::clone(&self.model);
            let context_size = self.context_size;
            let temperature = self.temperature;
            let model_id = self.model_id.clone();
            let model_def = self.model_def;

            let (text, input_tokens, output_tokens) = tokio::task::spawn_blocking(move || {
                let provider = GgufBackend {
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

        fn stream<'a>(
            &'a self,
            messages: &'a [ChatMessage],
        ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + 'a>> {
            let prompt = format_messages(messages, self.chat_template());
            let max_tokens = 4096u32;

            let backend = Arc::clone(&self.backend);
            let model = Arc::clone(&self.model);
            let context_size = self.context_size;
            let temperature = self.temperature;

            Box::pin(async_stream::stream! {
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
        let batch_size: usize = 512;

        let result = (|| -> Result<(u32, u32)> {
            let model = model.blocking_lock();

            let ctx_params = LlamaContextParams::default()
                .with_n_ctx(NonZeroU32::new(context_size))
                .with_n_batch(batch_size as u32);
            let mut ctx = model
                .new_context(backend, ctx_params)
                .map_err(|e| anyhow::anyhow!("failed to create llama context: {e}"))?;

            let tokens = model
                .str_to_token(prompt, llama_cpp_2::model::AddBos::Always)
                .map_err(|e| anyhow::anyhow!("tokenization failed: {e}"))?;

            let input_tokens = tokens.len() as u32;

            if tokens.is_empty() {
                bail!("empty token sequence");
            }

            // Process prompt in batches
            let mut batch = LlamaBatch::new(batch_size, 1);
            for (chunk_idx, chunk) in tokens.chunks(batch_size).enumerate() {
                batch.clear();
                let chunk_start = chunk_idx * batch_size;
                let is_last_chunk = chunk_start + chunk.len() == tokens.len();

                for (i, &token) in chunk.iter().enumerate() {
                    let pos = (chunk_start + i) as i32;
                    let is_last = is_last_chunk && i == chunk.len() - 1;
                    batch
                        .add(token, pos, &[0], is_last)
                        .map_err(|e| anyhow::anyhow!("batch add failed: {e}"))?;
                }

                ctx.decode(&mut batch)
                    .map_err(|e| anyhow::anyhow!("prompt decode failed: {e}"))?;
            }

            let mut sampler = LlamaSampler::chain_simple([
                LlamaSampler::temp(temperature),
                LlamaSampler::dist(42),
            ]);

            let mut output_tokens = 0u32;
            let mut pos = tokens.len() as i32;
            let eos_token = model.token_eos();
            let mut decoder = encoding_rs::UTF_8.new_decoder();

            for _ in 0..max_tokens {
                let token = sampler.sample(&ctx, batch.n_tokens() - 1);

                if token == eos_token {
                    break;
                }

                output_tokens += 1;
                sampler.accept(token);

                let piece = model
                    .token_to_piece(token, &mut decoder, true, None)
                    .map_err(|e| anyhow::anyhow!("detokenization failed: {e}"))?;

                if tx.blocking_send(StreamEvent::Delta(piece)).is_err() {
                    break;
                }

                batch.clear();
                batch
                    .add(token, pos, &[0], true)
                    .map_err(|e| anyhow::anyhow!("batch add token failed: {e}"))?;
                ctx.decode(&mut batch)
                    .map_err(|e| anyhow::anyhow!("token decode failed: {e}"))?;

                pos += 1;
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
                let _ = tx.blocking_send(StreamEvent::Error(e.to_string()));
            },
        }
    }
}

// ── MLX Backend ──────────────────────────────────────────────────────────────

pub mod mlx {
    //! MLX backend for Apple Silicon.
    //!
    //! Uses mlx-lm Python package via subprocess for inference.
    //! This provides native Apple Silicon optimization through MLX.

    use std::{
        io::{BufRead, BufReader},
        path::PathBuf,
        pin::Pin,
        process::{Command, Stdio},
    };

    use {
        anyhow::{Context, Result, bail},
        async_trait::async_trait,
        tokio_stream::Stream,
        tracing::{info, warn},
    };

    use crate::model::{ChatMessage, CompletionResponse, StreamEvent, Usage};

    use {
        super::{BackendType, LocalBackend, LocalLlmConfig},
        crate::providers::local_llm::models::{
            LocalModelDef,
            chat_templates::{ChatTemplateHint, format_messages},
        },
    };

    // Import the models module for model lookup and download
    use crate::providers::local_llm::models;

    /// MLX backend implementation.
    pub struct MlxBackend {
        model_id: String,
        model_path: PathBuf,
        model_def: Option<&'static LocalModelDef>,
        context_size: u32,
        temperature: f32,
        installation: super::MlxInstallation,
    }

    impl MlxBackend {
        /// Create an MLX backend from configuration.
        pub async fn from_config(config: &LocalLlmConfig) -> Result<Self> {
            // Check if MLX is available and detect installation method
            let installation = super::detect_mlx_installation().ok_or_else(|| {
                anyhow::anyhow!(
                    "MLX backend requires mlx-lm. Install with: brew install mlx-lm (or pip install mlx-lm)"
                )
            })?;

            info!(installation = ?installation, "detected MLX installation");

            // Resolve model - for MLX, we always download models to local cache
            let (model_path, model_def, context_size) = resolve_mlx_model(config).await?;

            info!(
                model = %config.model_id,
                path = %model_path.display(),
                context_size,
                "initialized MLX backend with locally cached model"
            );

            Ok(Self {
                model_id: config.model_id.clone(),
                model_path,
                model_def,
                context_size,
                temperature: config.temperature,
                installation,
            })
        }

        /// Get the chat template hint for this model.
        fn chat_template(&self) -> ChatTemplateHint {
            self.model_def
                .and_then(|d| d.chat_template)
                .unwrap_or(ChatTemplateHint::Auto)
        }

        /// Generate text using mlx-lm.
        async fn generate(&self, prompt: &str, max_tokens: u32) -> Result<(String, u32, u32)> {
            let model_path = self.model_path.to_string_lossy().to_string();
            let prompt = prompt.to_string();
            let temperature = self.temperature;
            let installation = self.installation;

            tokio::task::spawn_blocking(move || {
                generate_with_mlx(&model_path, &prompt, max_tokens, temperature, installation)
            })
            .await
            .context("MLX generation task panicked")?
        }
    }

    /// Generate text using mlx-lm based on installation method.
    fn generate_with_mlx(
        model_path: &str,
        prompt: &str,
        max_tokens: u32,
        temperature: f32,
        installation: super::MlxInstallation,
    ) -> Result<(String, u32, u32)> {
        match installation {
            super::MlxInstallation::PythonPackage => {
                generate_with_python(model_path, prompt, max_tokens, temperature)
            },
            super::MlxInstallation::HomebrewCli => {
                generate_with_cli(model_path, prompt, max_tokens, temperature)
            },
        }
    }

    /// Generate text using mlx-lm as a Python package.
    fn generate_with_python(
        model_path: &str,
        prompt: &str,
        max_tokens: u32,
        temperature: f32,
    ) -> Result<(String, u32, u32)> {
        let script = format!(
            r#"
import mlx_lm
import json

model, tokenizer = mlx_lm.load("{model_path}")
prompt = {prompt_json}
response = mlx_lm.generate(
    model,
    tokenizer,
    prompt=prompt,
    max_tokens={max_tokens},
    temp={temperature},
)
# Estimate tokens (mlx-lm doesn't provide exact counts easily)
input_tokens = len(tokenizer.encode(prompt))
output_tokens = len(tokenizer.encode(response))
print(json.dumps({{"text": response, "input_tokens": input_tokens, "output_tokens": output_tokens}}))
"#,
            model_path = model_path,
            prompt_json = serde_json::to_string(&prompt).unwrap_or_default(),
            max_tokens = max_tokens,
            temperature = temperature,
        );

        let output = Command::new("python3")
            .args(["-c", &script])
            .output()
            .context("failed to run mlx-lm via Python")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("mlx-lm (Python) failed: {}", stderr);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let result: serde_json::Value =
            serde_json::from_str(&stdout).context("failed to parse mlx-lm output")?;

        let text = result["text"].as_str().unwrap_or("");
        let text = strip_chat_template_tokens(text);
        let input_tokens = result["input_tokens"].as_u64().unwrap_or(0) as u32;
        let output_tokens = result["output_tokens"].as_u64().unwrap_or(0) as u32;

        Ok((text, input_tokens, output_tokens))
    }

    /// Strip common chat template stop tokens from model output.
    fn strip_chat_template_tokens(text: &str) -> String {
        // Common stop tokens from various chat templates
        const STOP_TOKENS: &[&str] = &[
            "<|im_end|>",    // ChatML (Qwen, Yi, etc.)
            "<|eot_id|>",    // Llama 3
            "</s>",          // Llama 2, Mistral
            "<|end|>",       // Phi
            "<|endoftext|>", // GPT-2 style
        ];

        let mut result = text.to_string();
        for token in STOP_TOKENS {
            // Strip from end (most common case)
            if result.ends_with(token) {
                result = result[..result.len() - token.len()].to_string();
            }
            // Also handle if it appears mid-response (model continued after stop)
            if let Some(pos) = result.find(token) {
                result = result[..pos].to_string();
            }
        }
        result.trim_end().to_string()
    }

    /// Generate text using mlx-lm CLI (Homebrew installation).
    fn generate_with_cli(
        model_path: &str,
        prompt: &str,
        max_tokens: u32,
        temperature: f32,
    ) -> Result<(String, u32, u32)> {
        use crate::providers::local_llm::response_parser::{MlxCliResponseParser, ResponseParser};

        // mlx_lm generate --model <model> --prompt <prompt> --max-tokens <N> --temp <T>
        let output = Command::new("mlx_lm")
            .arg("generate")
            .args(["--model", model_path])
            .args(["--prompt", prompt])
            .args(["--max-tokens", &max_tokens.to_string()])
            .args(["--temp", &temperature.to_string()])
            .output()
            .context("failed to run mlx_lm CLI")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("mlx_lm CLI failed: {}", stderr);
        }

        let raw_output = String::from_utf8_lossy(&output.stdout);

        // Use the MlxCliResponseParser to parse the decorated output
        let parser = MlxCliResponseParser;
        let parsed = parser.parse(&raw_output);

        // Strip chat template stop tokens from the response
        let text = strip_chat_template_tokens(&parsed.text);

        // Fallback to estimation if parser didn't get token counts
        let input_tokens = parsed.input_tokens.unwrap_or((prompt.len() / 4) as u32);
        let output_tokens = parsed.output_tokens.unwrap_or((text.len() / 4) as u32);

        Ok((text, input_tokens, output_tokens))
    }

    #[async_trait]
    impl LocalBackend for MlxBackend {
        fn backend_type(&self) -> BackendType {
            BackendType::Mlx
        }

        fn model_id(&self) -> &str {
            &self.model_id
        }

        fn context_window(&self) -> u32 {
            self.context_size
        }

        async fn complete(&self, messages: &[ChatMessage]) -> Result<CompletionResponse> {
            let prompt = format_messages(messages, self.chat_template());
            let (text, input_tokens, output_tokens) = self.generate(&prompt, 4096).await?;

            Ok(CompletionResponse {
                text: Some(text),
                tool_calls: vec![],
                usage: Usage {
                    input_tokens,
                    output_tokens,
                },
            })
        }

        fn stream<'a>(
            &'a self,
            messages: &'a [ChatMessage],
        ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + 'a>> {
            let prompt = format_messages(messages, self.chat_template());
            let model_path = self.model_path.to_string_lossy().to_string();
            let temperature = self.temperature;
            let installation = self.installation;

            Box::pin(async_stream::stream! {
                // Use spawn_blocking for the streaming generation
                let (tx, mut rx) = tokio::sync::mpsc::channel::<StreamEvent>(32);

                let handle = tokio::task::spawn_blocking(move || {
                    stream_generate_mlx(&model_path, &prompt, 4096, temperature, installation, tx)
                });

                while let Some(event) = rx.recv().await {
                    let is_done = matches!(event, StreamEvent::Done(_) | StreamEvent::Error(_));
                    yield event;
                    if is_done {
                        break;
                    }
                }

                if let Err(e) = handle.await {
                    warn!("MLX generation task error: {e}");
                }
            })
        }
    }

    /// Streaming generation using mlx-lm.
    fn stream_generate_mlx(
        model_path: &str,
        prompt: &str,
        max_tokens: u32,
        temperature: f32,
        installation: super::MlxInstallation,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) {
        let result = match installation {
            super::MlxInstallation::PythonPackage => {
                stream_generate_python(model_path, prompt, max_tokens, temperature, &tx)
            },
            super::MlxInstallation::HomebrewCli => {
                stream_generate_cli(model_path, prompt, max_tokens, temperature, &tx)
            },
        };

        match result {
            Ok((input_tokens, output_tokens)) => {
                let _ = tx.blocking_send(StreamEvent::Done(Usage {
                    input_tokens,
                    output_tokens,
                }));
            },
            Err(e) => {
                let _ = tx.blocking_send(StreamEvent::Error(e.to_string()));
            },
        }
    }

    /// Streaming generation using mlx-lm Python package.
    fn stream_generate_python(
        model_path: &str,
        prompt: &str,
        max_tokens: u32,
        temperature: f32,
        tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<(u32, u32)> {
        let script = format!(
            r#"
import mlx_lm
import sys

model, tokenizer = mlx_lm.load("{model_path}")
prompt = {prompt_json}

input_tokens = len(tokenizer.encode(prompt))
output_tokens = 0

for token in mlx_lm.stream_generate(
    model,
    tokenizer,
    prompt=prompt,
    max_tokens={max_tokens},
    temp={temperature},
):
    output_tokens += 1
    print(token, end="", flush=True)

# Print token counts at the end (special marker)
print(f"\n__TOKENS__:{{input_tokens}}:{{output_tokens}}", flush=True)
"#,
            model_path = model_path,
            prompt_json = serde_json::to_string(&prompt).unwrap_or_default(),
            max_tokens = max_tokens,
            temperature = temperature,
        );

        let mut child = Command::new("python3")
            .args(["-c", &script])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("failed to spawn mlx-lm")?;

        let stdout = child.stdout.take().context("no stdout")?;
        let reader = BufReader::new(stdout);

        let mut input_tokens = 0u32;
        let mut output_tokens = 0u32;

        // Read lines from the process output
        for line in reader.lines() {
            let line = line.context("failed to read line")?;

            if line.starts_with("__TOKENS__:") {
                // Parse token counts
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() >= 3 {
                    input_tokens = parts[1].parse().unwrap_or(0);
                    output_tokens = parts[2].parse().unwrap_or(0);
                }
            } else {
                // Send as delta
                if tx.blocking_send(StreamEvent::Delta(line)).is_err() {
                    break;
                }
            }
        }

        let status = child.wait().context("failed to wait for mlx-lm")?;
        if !status.success() {
            bail!("mlx-lm (Python) exited with error");
        }

        Ok((input_tokens, output_tokens))
    }

    /// Streaming generation using mlx-lm CLI (Homebrew).
    ///
    /// Note: The CLI doesn't support true streaming output, so we collect all
    /// output, parse it with the response parser, and send the cleaned text.
    fn stream_generate_cli(
        model_path: &str,
        prompt: &str,
        max_tokens: u32,
        temperature: f32,
        tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<(u32, u32)> {
        // Use the non-streaming generate function which already uses the parser
        let (text, input_tokens, output_tokens) =
            generate_with_cli(model_path, prompt, max_tokens, temperature)?;

        // Send the parsed text as a single delta
        let _ = tx.blocking_send(StreamEvent::Delta(text));

        Ok((input_tokens, output_tokens))
    }

    /// Resolve an MLX model, downloading it if necessary.
    ///
    /// Checks both the unified and legacy registries, downloads the model
    /// to the local cache, and returns the path to the model directory.
    async fn resolve_mlx_model(
        config: &LocalLlmConfig,
    ) -> Result<(PathBuf, Option<&'static LocalModelDef>, u32)> {
        // If a custom path is provided, use it directly (if it exists)
        if let Some(path) = &config.model_path {
            if path.exists() {
                info!(
                    path = %path.display(),
                    "using custom MLX model path"
                );
                let model_def = models::find_model(&config.model_id);
                let context_size = config
                    .context_size
                    .or_else(|| model_def.map(|d| d.context_window))
                    .unwrap_or(8192);
                return Ok((path.clone(), model_def, context_size));
            } else {
                bail!("model path not found: {}", path.display());
            }
        }

        // First, check the unified registry
        if let Some(def) = models::find_model(&config.model_id)
            && def.has_mlx()
        {
            info!(
                model = config.model_id,
                mlx_repo = ?def.mlx_repo,
                "found model in unified registry, downloading"
            );
            let model_path = models::ensure_mlx_model(def, &config.cache_dir).await?;
            let context_size = config.context_size.unwrap_or(def.context_window);
            return Ok((model_path, Some(def), context_size));
        }

        // Check the legacy registry (for models like mlx-qwen2.5-coder-1.5b-4bit)
        if let Some(legacy_def) = crate::providers::local_gguf::models::find_model(&config.model_id)
            && legacy_def.backend == crate::providers::local_gguf::models::ModelBackend::Mlx
        {
            info!(
                model = config.model_id,
                hf_repo = legacy_def.hf_repo,
                "found model in legacy registry, downloading"
            );
            let model_path = crate::providers::local_gguf::models::ensure_mlx_model(
                legacy_def,
                &config.cache_dir,
            )
            .await?;
            let context_size = config.context_size.unwrap_or(legacy_def.context_window);
            return Ok((model_path, None, context_size));
        }

        bail!(
            "unknown MLX model '{}'. Use model_path for custom MLX models.",
            config.model_id
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_type_display() {
        assert_eq!(BackendType::Gguf.display_name(), "GGUF (llama.cpp)");
        assert_eq!(BackendType::Mlx.display_name(), "MLX (Apple)");
    }

    #[test]
    fn test_detect_best_backend() {
        let backend = detect_best_backend();
        // Should return a valid backend
        assert!(matches!(backend, BackendType::Gguf | BackendType::Mlx));
    }

    #[test]
    fn test_available_backends_includes_gguf() {
        let backends = available_backends();
        assert!(backends.contains(&BackendType::Gguf));
    }

    // ── detect_backend_for_model tests ─────────────────────────────────────

    #[test]
    fn test_detect_backend_for_legacy_mlx_model() {
        // Legacy MLX models (mlx-* prefix) should select MLX backend
        let backend = detect_backend_for_model("mlx-llama-3.2-1b-4bit");
        assert_eq!(backend, BackendType::Mlx);
    }

    #[test]
    fn test_detect_backend_for_legacy_mlx_qwen_model() {
        // Another legacy MLX model
        let backend = detect_backend_for_model("mlx-qwen2.5-coder-1.5b-4bit");
        assert_eq!(backend, BackendType::Mlx);
    }

    #[test]
    fn test_detect_backend_for_gguf_model_from_legacy_registry() {
        // GGUF models from legacy registry should select GGUF backend
        let backend = detect_backend_for_model("llama-3.2-1b-q4_k_m");
        // This model is in the legacy GGUF registry, should be GGUF
        assert_eq!(backend, BackendType::Gguf);
    }

    #[test]
    fn test_detect_backend_for_unified_registry_gguf_model() {
        // Models from unified registry without MLX should use GGUF
        // deepseek-coder-6.7b-q4_k_m has no MLX version
        let backend = detect_backend_for_model("deepseek-coder-6.7b-q4_k_m");
        assert_eq!(backend, BackendType::Gguf);
    }

    #[test]
    fn test_detect_backend_for_unknown_model() {
        // Unknown models should fall back to system detection
        let backend = detect_backend_for_model("unknown-model-12345");
        // Should return a valid backend (GGUF or MLX based on system)
        assert!(matches!(backend, BackendType::Gguf | BackendType::Mlx));
    }

    #[test]
    fn test_detect_backend_for_unified_model_with_mlx_support() {
        // Models from unified registry with MLX support
        // Should prefer MLX on Apple Silicon, GGUF otherwise
        let backend = detect_backend_for_model("qwen2.5-coder-1.5b-q4_k_m");
        // On non-Apple Silicon, should be GGUF. On Apple Silicon with mlx_lm, MLX.
        assert!(matches!(backend, BackendType::Gguf | BackendType::Mlx));
    }

    #[test]
    fn test_detect_mlx_installation_consistency() {
        // detect_mlx_installation and is_mlx_available should be consistent
        let installation = detect_mlx_installation();
        let available = is_mlx_available();

        // If installation is Some, available should be true
        // If installation is None, available should be false
        assert_eq!(installation.is_some(), available);
    }

    #[test]
    fn test_mlx_installation_enum_values() {
        // Test that MlxInstallation enum values exist and can be compared
        let python = MlxInstallation::PythonPackage;
        let homebrew = MlxInstallation::HomebrewCli;

        assert_ne!(python, homebrew);
        assert_eq!(python, MlxInstallation::PythonPackage);
        assert_eq!(homebrew, MlxInstallation::HomebrewCli);
    }
}
