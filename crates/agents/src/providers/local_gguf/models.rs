//! Model registry for local LLM models (GGUF and MLX).
//!
//! Defines available models with HuggingFace URLs, memory requirements,
//! and chat template hints.

use std::path::PathBuf;

use {
    anyhow::Context,
    futures::StreamExt,
    tracing::{debug, info},
};

use super::{chat_templates::ChatTemplateHint, system_info::MemoryTier};

/// Progress info for model downloads.
#[derive(Debug, Clone)]
pub struct DownloadProgress {
    /// Bytes downloaded so far.
    pub downloaded: u64,
    /// Total bytes (if known from Content-Length).
    pub total: Option<u64>,
}

/// Backend type for local models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelBackend {
    /// GGUF format (llama.cpp)
    Gguf,
    /// MLX format (Apple Silicon native)
    Mlx,
}

impl std::fmt::Display for ModelBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelBackend::Gguf => write!(f, "GGUF"),
            ModelBackend::Mlx => write!(f, "MLX"),
        }
    }
}

/// Definition of a local LLM model in the registry.
#[derive(Debug, Clone)]
pub struct GgufModelDef {
    /// Model identifier (e.g., "qwen2.5-coder-7b-q4_k_m").
    pub id: &'static str,
    /// Human-readable display name.
    pub display_name: &'static str,
    /// HuggingFace repository (e.g., "Qwen/Qwen2.5-Coder-7B-Instruct-GGUF").
    pub hf_repo: &'static str,
    /// Filename in the repository (for GGUF) or empty for MLX (uses whole repo).
    pub hf_filename: &'static str,
    /// Minimum RAM required in GB.
    pub min_ram_gb: u32,
    /// Context window size in tokens.
    pub context_window: u32,
    /// Chat template hint for formatting messages.
    pub chat_template: ChatTemplateHint,
    /// Backend type (GGUF or MLX).
    pub backend: ModelBackend,
}

impl GgufModelDef {
    /// HuggingFace download URL for this model.
    #[must_use]
    pub fn hf_url(&self) -> String {
        format!(
            "https://huggingface.co/{}/resolve/main/{}",
            self.hf_repo, self.hf_filename
        )
    }
}

/// Model registry — all known local LLM models organized by backend and memory tier.
///
/// Models are listed in recommended order within each tier.
pub static MODEL_REGISTRY: &[GgufModelDef] = &[
    // ════════════════════════════════════════════════════════════════════════
    // GGUF Models (llama.cpp)
    // ════════════════════════════════════════════════════════════════════════
    // ── 4GB tier (Tiny) ────────────────────────────────────────────────────
    GgufModelDef {
        id: "qwen2.5-coder-1.5b-q4_k_m",
        display_name: "Qwen 2.5 Coder 1.5B (Q4_K_M)",
        hf_repo: "Qwen/Qwen2.5-Coder-1.5B-Instruct-GGUF",
        hf_filename: "qwen2.5-coder-1.5b-instruct-q4_k_m.gguf",
        min_ram_gb: 4,
        context_window: 32_768,
        chat_template: ChatTemplateHint::ChatML,
        backend: ModelBackend::Gguf,
    },
    GgufModelDef {
        id: "llama-3.2-1b-q4_k_m",
        display_name: "Llama 3.2 1B (Q4_K_M)",
        hf_repo: "bartowski/Llama-3.2-1B-Instruct-GGUF",
        hf_filename: "Llama-3.2-1B-Instruct-Q4_K_M.gguf",
        min_ram_gb: 4,
        context_window: 128_000,
        chat_template: ChatTemplateHint::Llama3,
        backend: ModelBackend::Gguf,
    },
    // ── 8GB tier (Small) ───────────────────────────────────────────────────
    GgufModelDef {
        id: "qwen2.5-coder-7b-q4_k_m",
        display_name: "Qwen 2.5 Coder 7B (Q4_K_M)",
        hf_repo: "Qwen/Qwen2.5-Coder-7B-Instruct-GGUF",
        hf_filename: "qwen2.5-coder-7b-instruct-q4_k_m.gguf",
        min_ram_gb: 8,
        context_window: 32_768,
        chat_template: ChatTemplateHint::ChatML,
        backend: ModelBackend::Gguf,
    },
    GgufModelDef {
        id: "llama-3.2-3b-q4_k_m",
        display_name: "Llama 3.2 3B (Q4_K_M)",
        hf_repo: "bartowski/Llama-3.2-3B-Instruct-GGUF",
        hf_filename: "Llama-3.2-3B-Instruct-Q4_K_M.gguf",
        min_ram_gb: 8,
        context_window: 128_000,
        chat_template: ChatTemplateHint::Llama3,
        backend: ModelBackend::Gguf,
    },
    GgufModelDef {
        id: "deepseek-coder-6.7b-q4_k_m",
        display_name: "DeepSeek Coder 6.7B (Q4_K_M)",
        hf_repo: "TheBloke/deepseek-coder-6.7B-instruct-GGUF",
        hf_filename: "deepseek-coder-6.7b-instruct.Q4_K_M.gguf",
        min_ram_gb: 8,
        context_window: 16_384,
        chat_template: ChatTemplateHint::DeepSeek,
        backend: ModelBackend::Gguf,
    },
    // ── 16GB tier (Medium) ─────────────────────────────────────────────────
    GgufModelDef {
        id: "qwen2.5-coder-14b-q4_k_m",
        display_name: "Qwen 2.5 Coder 14B (Q4_K_M)",
        hf_repo: "Qwen/Qwen2.5-Coder-14B-Instruct-GGUF",
        hf_filename: "qwen2.5-coder-14b-instruct-q4_k_m.gguf",
        min_ram_gb: 16,
        context_window: 32_768,
        chat_template: ChatTemplateHint::ChatML,
        backend: ModelBackend::Gguf,
    },
    GgufModelDef {
        id: "codestral-22b-q4_k_m",
        display_name: "Codestral 22B (Q4_K_M)",
        hf_repo: "bartowski/Codestral-22B-v0.1-GGUF",
        hf_filename: "Codestral-22B-v0.1-Q4_K_M.gguf",
        min_ram_gb: 16,
        context_window: 32_768,
        chat_template: ChatTemplateHint::Mistral,
        backend: ModelBackend::Gguf,
    },
    GgufModelDef {
        id: "mistral-7b-q5_k_m",
        display_name: "Mistral 7B Instruct (Q5_K_M)",
        hf_repo: "TheBloke/Mistral-7B-Instruct-v0.2-GGUF",
        hf_filename: "mistral-7b-instruct-v0.2.Q5_K_M.gguf",
        min_ram_gb: 12,
        context_window: 32_768,
        chat_template: ChatTemplateHint::Mistral,
        backend: ModelBackend::Gguf,
    },
    GgufModelDef {
        id: "llama-3.1-8b-q4_k_m",
        display_name: "Llama 3.1 8B (Q4_K_M)",
        hf_repo: "bartowski/Meta-Llama-3.1-8B-Instruct-GGUF",
        hf_filename: "Meta-Llama-3.1-8B-Instruct-Q4_K_M.gguf",
        min_ram_gb: 12,
        context_window: 128_000,
        chat_template: ChatTemplateHint::Llama3,
        backend: ModelBackend::Gguf,
    },
    // ── 32GB tier (Large) ──────────────────────────────────────────────────
    GgufModelDef {
        id: "qwen2.5-coder-32b-q4_k_m",
        display_name: "Qwen 2.5 Coder 32B (Q4_K_M)",
        hf_repo: "Qwen/Qwen2.5-Coder-32B-Instruct-GGUF",
        hf_filename: "qwen2.5-coder-32b-instruct-q4_k_m.gguf",
        min_ram_gb: 32,
        context_window: 32_768,
        chat_template: ChatTemplateHint::ChatML,
        backend: ModelBackend::Gguf,
    },
    GgufModelDef {
        id: "deepseek-coder-33b-q4_k_m",
        display_name: "DeepSeek Coder 33B (Q4_K_M)",
        hf_repo: "TheBloke/deepseek-coder-33B-instruct-GGUF",
        hf_filename: "deepseek-coder-33b-instruct.Q4_K_M.gguf",
        min_ram_gb: 32,
        context_window: 16_384,
        chat_template: ChatTemplateHint::DeepSeek,
        backend: ModelBackend::Gguf,
    },
    GgufModelDef {
        id: "llama-3.1-70b-q2_k",
        display_name: "Llama 3.1 70B (Q2_K)",
        hf_repo: "bartowski/Meta-Llama-3.1-70B-Instruct-GGUF",
        hf_filename: "Meta-Llama-3.1-70B-Instruct-Q2_K.gguf",
        min_ram_gb: 48,
        context_window: 128_000,
        chat_template: ChatTemplateHint::Llama3,
        backend: ModelBackend::Gguf,
    },
    // ════════════════════════════════════════════════════════════════════════
    // MLX Models (Apple Silicon native)
    // ════════════════════════════════════════════════════════════════════════
    // ── 4GB tier (Tiny) ────────────────────────────────────────────────────
    GgufModelDef {
        id: "mlx-qwen2.5-coder-1.5b-4bit",
        display_name: "Qwen 2.5 Coder 1.5B (4-bit MLX)",
        hf_repo: "mlx-community/Qwen2.5-Coder-1.5B-Instruct-4bit",
        hf_filename: "",
        min_ram_gb: 4,
        context_window: 32_768,
        chat_template: ChatTemplateHint::ChatML,
        backend: ModelBackend::Mlx,
    },
    GgufModelDef {
        id: "mlx-llama-3.2-1b-4bit",
        display_name: "Llama 3.2 1B (4-bit MLX)",
        hf_repo: "mlx-community/Llama-3.2-1B-Instruct-4bit",
        hf_filename: "",
        min_ram_gb: 4,
        context_window: 128_000,
        chat_template: ChatTemplateHint::Llama3,
        backend: ModelBackend::Mlx,
    },
    // ── 8GB tier (Small) ───────────────────────────────────────────────────
    GgufModelDef {
        id: "mlx-qwen2.5-coder-7b-4bit",
        display_name: "Qwen 2.5 Coder 7B (4-bit MLX)",
        hf_repo: "mlx-community/Qwen2.5-Coder-7B-Instruct-4bit",
        hf_filename: "",
        min_ram_gb: 8,
        context_window: 32_768,
        chat_template: ChatTemplateHint::ChatML,
        backend: ModelBackend::Mlx,
    },
    GgufModelDef {
        id: "mlx-llama-3.2-3b-4bit",
        display_name: "Llama 3.2 3B (4-bit MLX)",
        hf_repo: "mlx-community/Llama-3.2-3B-Instruct-4bit",
        hf_filename: "",
        min_ram_gb: 8,
        context_window: 128_000,
        chat_template: ChatTemplateHint::Llama3,
        backend: ModelBackend::Mlx,
    },
    // ── 16GB tier (Medium) ─────────────────────────────────────────────────
    GgufModelDef {
        id: "mlx-qwen2.5-coder-14b-4bit",
        display_name: "Qwen 2.5 Coder 14B (4-bit MLX)",
        hf_repo: "mlx-community/Qwen2.5-Coder-14B-Instruct-4bit",
        hf_filename: "",
        min_ram_gb: 16,
        context_window: 32_768,
        chat_template: ChatTemplateHint::ChatML,
        backend: ModelBackend::Mlx,
    },
    GgufModelDef {
        id: "mlx-mistral-7b-4bit",
        display_name: "Mistral 7B Instruct (4-bit MLX)",
        hf_repo: "mlx-community/Mistral-7B-Instruct-v0.3-4bit",
        hf_filename: "",
        min_ram_gb: 8,
        context_window: 32_768,
        chat_template: ChatTemplateHint::Mistral,
        backend: ModelBackend::Mlx,
    },
    GgufModelDef {
        id: "mlx-llama-3.1-8b-4bit",
        display_name: "Llama 3.1 8B (4-bit MLX)",
        hf_repo: "mlx-community/Meta-Llama-3.1-8B-Instruct-4bit",
        hf_filename: "",
        min_ram_gb: 8,
        context_window: 128_000,
        chat_template: ChatTemplateHint::Llama3,
        backend: ModelBackend::Mlx,
    },
    // ── 32GB tier (Large) ──────────────────────────────────────────────────
    GgufModelDef {
        id: "mlx-qwen2.5-coder-32b-4bit",
        display_name: "Qwen 2.5 Coder 32B (4-bit MLX)",
        hf_repo: "mlx-community/Qwen2.5-Coder-32B-Instruct-4bit",
        hf_filename: "",
        min_ram_gb: 32,
        context_window: 32_768,
        chat_template: ChatTemplateHint::ChatML,
        backend: ModelBackend::Mlx,
    },
    GgufModelDef {
        id: "mlx-llama-3.1-70b-4bit",
        display_name: "Llama 3.1 70B (4-bit MLX)",
        hf_repo: "mlx-community/Meta-Llama-3.1-70B-Instruct-4bit",
        hf_filename: "",
        min_ram_gb: 48,
        context_window: 128_000,
        chat_template: ChatTemplateHint::Llama3,
        backend: ModelBackend::Mlx,
    },
];

/// Find a model definition by ID.
#[must_use]
pub fn find_model(id: &str) -> Option<&'static GgufModelDef> {
    MODEL_REGISTRY.iter().find(|m| m.id == id)
}

/// Get models suitable for a given memory tier (all backends).
#[must_use]
pub fn models_for_tier(tier: MemoryTier) -> Vec<&'static GgufModelDef> {
    let max_ram = match tier {
        MemoryTier::Tiny => 4,
        MemoryTier::Small => 8,
        MemoryTier::Medium => 16,
        MemoryTier::Large => u32::MAX,
    };
    MODEL_REGISTRY
        .iter()
        .filter(|m| m.min_ram_gb <= max_ram)
        .collect()
}

/// Get models suitable for a given memory tier and backend.
#[must_use]
pub fn models_for_tier_and_backend(
    tier: MemoryTier,
    backend: ModelBackend,
) -> Vec<&'static GgufModelDef> {
    let max_ram = match tier {
        MemoryTier::Tiny => 4,
        MemoryTier::Small => 8,
        MemoryTier::Medium => 16,
        MemoryTier::Large => u32::MAX,
    };
    MODEL_REGISTRY
        .iter()
        .filter(|m| m.min_ram_gb <= max_ram && m.backend == backend)
        .collect()
}

/// Suggest the best model for a memory tier (all backends).
#[must_use]
pub fn suggest_model(tier: MemoryTier) -> Option<&'static GgufModelDef> {
    let models = models_for_tier(tier);
    // Return the last model that fits (usually the largest that works)
    models.iter().copied().max_by_key(|m| m.min_ram_gb)
}

/// Suggest the best model for a memory tier and backend.
#[must_use]
pub fn suggest_model_for_backend(
    tier: MemoryTier,
    backend: ModelBackend,
) -> Option<&'static GgufModelDef> {
    let models = models_for_tier_and_backend(tier, backend);
    models.iter().copied().max_by_key(|m| m.min_ram_gb)
}

/// Default cache directory for downloaded models.
///
/// Returns `~/.moltis/models` (same base as config/data directories).
#[must_use]
pub fn default_models_dir() -> PathBuf {
    moltis_config::data_dir().join("models")
}

/// Check if a GGUF model file is cached locally.
#[must_use]
pub fn is_gguf_model_cached(model: &GgufModelDef, cache_dir: &std::path::Path) -> bool {
    if model.backend != ModelBackend::Gguf {
        return false;
    }
    let model_path = cache_dir.join(model.hf_filename);
    model_path.exists()
}

/// Check if an MLX model directory is cached locally.
#[must_use]
pub fn is_mlx_model_cached(model: &GgufModelDef, cache_dir: &std::path::Path) -> bool {
    if model.backend != ModelBackend::Mlx {
        return false;
    }

    let model_dir_name = model.hf_repo.replace('/', "__");
    let model_dir = cache_dir.join("mlx").join(&model_dir_name);

    let config_path = model_dir.join("config.json");
    let model_path = model_dir.join("model.safetensors");
    let index_path = model_dir.join("model.safetensors.index.json");

    config_path.exists() && (model_path.exists() || index_path.exists())
}

/// Check if a model is cached (based on its backend type).
#[must_use]
pub fn is_model_cached(model: &GgufModelDef, cache_dir: &std::path::Path) -> bool {
    match model.backend {
        ModelBackend::Gguf => is_gguf_model_cached(model, cache_dir),
        ModelBackend::Mlx => is_mlx_model_cached(model, cache_dir),
    }
}

/// Ensure a model is downloaded, returning the path to the GGUF file.
///
/// Downloads from HuggingFace if not present in the cache.
pub async fn ensure_model(
    model: &GgufModelDef,
    cache_dir: &std::path::Path,
) -> anyhow::Result<PathBuf> {
    ensure_model_with_progress(model, cache_dir, |_| {}).await
}

/// Ensure a model is downloaded with progress reporting.
///
/// The progress callback is called periodically during download with the current progress.
pub async fn ensure_model_with_progress<F>(
    model: &GgufModelDef,
    cache_dir: &std::path::Path,
    mut on_progress: F,
) -> anyhow::Result<PathBuf>
where
    F: FnMut(DownloadProgress),
{
    let model_path = cache_dir.join(model.hf_filename);
    if model_path.exists() {
        info!(path = %model_path.display(), model = model.id, "model found in cache");
        return Ok(model_path);
    }

    debug!(cache_dir = %cache_dir.display(), "ensuring cache directory exists");
    tokio::fs::create_dir_all(cache_dir)
        .await
        .context("creating models cache dir")?;

    let url = model.hf_url();
    info!(
        url = %url,
        model = model.id,
        backend = %model.backend,
        "downloading model from HuggingFace"
    );

    let download_start = std::time::Instant::now();

    let response = reqwest::get(&url)
        .await
        .context("downloading GGUF model")?
        .error_for_status()
        .context("GGUF model download failed")?;

    let total = response.content_length();
    let mut downloaded: u64 = 0;

    if let Some(size) = total {
        debug!(total_size_mb = size / (1024 * 1024), "download size known");
    }

    // Report initial progress
    on_progress(DownloadProgress { downloaded, total });

    // Stream the download to a temp file
    let tmp_path = model_path.with_extension("tmp");
    debug!(tmp_path = %tmp_path.display(), "creating temp file for download");
    let mut file = tokio::fs::File::create(&tmp_path)
        .await
        .context("creating temp file")?;

    let mut stream = response.bytes_stream();
    let mut last_report = std::time::Instant::now();
    let mut last_log = std::time::Instant::now();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("reading chunk")?;
        downloaded += chunk.len() as u64;

        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .context("writing chunk")?;

        // Report progress at most every 100ms to avoid flooding
        if last_report.elapsed() >= std::time::Duration::from_millis(100) {
            on_progress(DownloadProgress { downloaded, total });
            last_report = std::time::Instant::now();
        }

        // Log progress every 5 seconds for visibility
        if last_log.elapsed() >= std::time::Duration::from_secs(5) {
            let percent = total
                .map(|t| (downloaded as f64 / t as f64 * 100.0) as u32)
                .unwrap_or(0);
            debug!(
                downloaded_mb = downloaded / (1024 * 1024),
                percent, "download progress"
            );
            last_log = std::time::Instant::now();
        }
    }

    // Final progress report
    on_progress(DownloadProgress { downloaded, total });

    // Flush and close the file before renaming
    tokio::io::AsyncWriteExt::flush(&mut file)
        .await
        .context("flushing file")?;
    drop(file);

    debug!(
        from = %tmp_path.display(),
        to = %model_path.display(),
        "renaming temp file to final location"
    );
    tokio::fs::rename(&tmp_path, &model_path)
        .await
        .context("renaming model file")?;

    let download_duration = download_start.elapsed();
    let download_speed_mbps = if download_duration.as_secs_f64() > 0.0 {
        (downloaded as f64 / (1024.0 * 1024.0)) / download_duration.as_secs_f64()
    } else {
        0.0
    };

    info!(
        path = %model_path.display(),
        size_mb = downloaded / (1024 * 1024),
        duration_secs = download_duration.as_secs_f64(),
        speed_mbps = format!("{:.1}", download_speed_mbps),
        model = model.id,
        "model downloaded successfully"
    );

    Ok(model_path)
}

// ── MLX Model Download ───────────────────────────────────────────────────────

/// Ensure an MLX model is downloaded, returning the path to the model directory.
///
/// MLX models are directories containing multiple files (config.json, model.safetensors, etc.).
pub async fn ensure_mlx_model(
    model: &GgufModelDef,
    cache_dir: &std::path::Path,
) -> anyhow::Result<PathBuf> {
    ensure_mlx_model_with_progress(model, cache_dir, |_| {}).await
}

/// Ensure an MLX model is downloaded with progress reporting.
pub async fn ensure_mlx_model_with_progress<F>(
    model: &GgufModelDef,
    cache_dir: &std::path::Path,
    mut on_progress: F,
) -> anyhow::Result<PathBuf>
where
    F: FnMut(DownloadProgress),
{
    if model.backend != ModelBackend::Mlx {
        anyhow::bail!(
            "model '{}' is not an MLX model (backend: {:?})",
            model.id,
            model.backend
        );
    }

    // Create model directory using sanitized repo name
    let model_dir_name = model.hf_repo.replace('/', "__");
    let model_dir = cache_dir.join("mlx").join(&model_dir_name);

    // Check if model is already fully downloaded
    let config_path = model_dir.join("config.json");
    let model_path = model_dir.join("model.safetensors");
    let index_path = model_dir.join("model.safetensors.index.json");

    // A model is considered cached if it has config.json and either model.safetensors or an index file
    if config_path.exists() && (model_path.exists() || index_path.exists()) {
        info!(
            path = %model_dir.display(),
            model = model.id,
            "MLX model found in cache"
        );
        return Ok(model_dir);
    }

    // Create the model directory
    tokio::fs::create_dir_all(&model_dir)
        .await
        .context("creating MLX model cache dir")?;

    info!(
        hf_repo = model.hf_repo,
        model = model.id,
        "downloading MLX model from HuggingFace"
    );

    // First, get the list of files in the repository
    let files = list_hf_repo_files(model.hf_repo).await?;
    debug!(file_count = files.len(), "found files in HuggingFace repo");

    // Filter to only the files we need
    let files_to_download: Vec<String> = files
        .into_iter()
        .filter(|f| {
            // Include essential config/tokenizer files
            matches!(
                f.as_str(),
                "config.json"
                    | "model.safetensors"
                    | "model.safetensors.index.json"
                    | "tokenizer.json"
                    | "tokenizer_config.json"
                    | "special_tokens_map.json"
                    | "generation_config.json"
            )
            // Include sharded weight files
            || (f.starts_with("model-") && f.ends_with(".safetensors"))
            || (f.starts_with("weights.") && f.ends_with(".safetensors"))
            // Include any .safetensors file
            || f.ends_with(".safetensors")
        })
        .collect();

    if files_to_download.is_empty() {
        anyhow::bail!(
            "no model files found in HuggingFace repo '{}'",
            model.hf_repo
        );
    }

    info!(
        file_count = files_to_download.len(),
        "downloading files for MLX model"
    );
    debug!(files = ?files_to_download, "files to download");

    // Download each file
    let mut total_downloaded: u64 = 0;
    for filename in &files_to_download {
        let file_path = model_dir.join(filename);

        // Skip if already downloaded
        if file_path.exists() {
            debug!(file = filename, "file already cached, skipping");
            continue;
        }

        // Create parent directories if needed (for sharded files)
        if let Some(parent) = file_path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }

        let url = format!(
            "https://huggingface.co/{}/resolve/main/{}",
            model.hf_repo, filename
        );
        debug!(url = %url, file = filename, "downloading file");

        let downloaded = download_mlx_file(&url, &file_path, |progress| {
            on_progress(DownloadProgress {
                downloaded: total_downloaded + progress.downloaded,
                total: None,
            });
        })
        .await
        .with_context(|| format!("downloading {}", filename))?;

        total_downloaded += downloaded;
        debug!(
            file = filename,
            size_mb = downloaded / (1024 * 1024),
            "file downloaded"
        );
    }

    // Final progress report
    on_progress(DownloadProgress {
        downloaded: total_downloaded,
        total: Some(total_downloaded),
    });

    info!(
        path = %model_dir.display(),
        total_size_mb = total_downloaded / (1024 * 1024),
        model = model.id,
        "MLX model downloaded successfully"
    );

    Ok(model_dir)
}

/// List files in a HuggingFace repository.
async fn list_hf_repo_files(repo: &str) -> anyhow::Result<Vec<String>> {
    let url = format!("https://huggingface.co/api/models/{}/tree/main", repo);

    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .header("User-Agent", "moltis/1.0")
        .send()
        .await
        .context("fetching HuggingFace repo file list")?
        .error_for_status()
        .with_context(|| format!("HuggingFace API error for repo '{}'", repo))?;

    let entries: Vec<serde_json::Value> = response
        .json()
        .await
        .context("parsing HuggingFace API response")?;

    // Extract file paths from the response
    let files: Vec<String> = entries
        .into_iter()
        .filter_map(|entry| {
            if entry["type"].as_str() == Some("file") {
                entry["path"].as_str().map(String::from)
            } else {
                None
            }
        })
        .collect();

    Ok(files)
}

/// Download a single file with progress reporting.
async fn download_mlx_file<F>(url: &str, path: &PathBuf, mut on_progress: F) -> anyhow::Result<u64>
where
    F: FnMut(DownloadProgress),
{
    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .header("User-Agent", "moltis/1.0")
        .send()
        .await
        .context("starting download")?
        .error_for_status()
        .context("download failed")?;

    let total = response.content_length();
    let mut downloaded: u64 = 0;

    on_progress(DownloadProgress { downloaded, total });

    let tmp_path = path.with_extension("tmp");
    let mut file = tokio::fs::File::create(&tmp_path)
        .await
        .context("creating temp file")?;

    let mut stream = response.bytes_stream();
    let mut last_report = std::time::Instant::now();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("reading chunk")?;
        downloaded += chunk.len() as u64;

        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .context("writing chunk")?;

        if last_report.elapsed() >= std::time::Duration::from_millis(100) {
            on_progress(DownloadProgress { downloaded, total });
            last_report = std::time::Instant::now();
        }
    }

    on_progress(DownloadProgress { downloaded, total });

    tokio::io::AsyncWriteExt::flush(&mut file)
        .await
        .context("flushing file")?;
    drop(file);

    tokio::fs::rename(&tmp_path, path)
        .await
        .context("renaming file")?;

    Ok(downloaded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_model() {
        assert!(find_model("qwen2.5-coder-7b-q4_k_m").is_some());
        assert!(find_model("nonexistent-model").is_none());
    }

    #[test]
    fn test_hf_url() {
        let model = find_model("qwen2.5-coder-7b-q4_k_m").unwrap();
        let url = model.hf_url();
        assert!(url.starts_with("https://huggingface.co/"));
        assert!(url.contains("Qwen"));
        assert!(url.ends_with(".gguf"));
    }

    #[test]
    fn test_models_for_tier() {
        let tiny = models_for_tier(MemoryTier::Tiny);
        assert!(!tiny.is_empty());
        for m in &tiny {
            assert!(m.min_ram_gb <= 4);
        }

        let small = models_for_tier(MemoryTier::Small);
        assert!(small.len() >= tiny.len());

        let medium = models_for_tier(MemoryTier::Medium);
        assert!(medium.len() >= small.len());

        let large = models_for_tier(MemoryTier::Large);
        assert_eq!(large.len(), MODEL_REGISTRY.len());
    }

    #[test]
    fn test_suggest_model() {
        // Each tier should have a suggestion
        assert!(suggest_model(MemoryTier::Tiny).is_some());
        assert!(suggest_model(MemoryTier::Small).is_some());
        assert!(suggest_model(MemoryTier::Medium).is_some());
        assert!(suggest_model(MemoryTier::Large).is_some());
    }

    #[test]
    fn test_default_models_dir() {
        let dir = default_models_dir();
        assert!(dir.to_string_lossy().contains("models"));
    }

    #[test]
    fn test_model_registry_unique_ids() {
        let mut ids: Vec<&str> = MODEL_REGISTRY.iter().map(|m| m.id).collect();
        ids.sort();
        let len_before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), len_before, "duplicate model IDs found");
    }

    #[test]
    fn test_model_registry_valid_urls() {
        for model in MODEL_REGISTRY {
            let url = model.hf_url();
            assert!(
                url.starts_with("https://huggingface.co/"),
                "invalid URL for {}: {}",
                model.id,
                url
            );
            // Only GGUF models should have .gguf URLs; MLX uses the repo directly
            if model.backend == ModelBackend::Gguf {
                assert!(
                    url.ends_with(".gguf"),
                    "GGUF URL should end with .gguf: {}",
                    url
                );
            }
        }
    }

    #[test]
    fn test_model_registry_context_windows() {
        for model in MODEL_REGISTRY {
            assert!(
                model.context_window > 0,
                "model {} has zero context window",
                model.id
            );
        }
    }

    #[test]
    fn test_models_for_tier_and_backend() {
        // GGUF models for small tier
        let gguf_small = models_for_tier_and_backend(MemoryTier::Small, ModelBackend::Gguf);
        assert!(!gguf_small.is_empty());
        for m in &gguf_small {
            assert_eq!(m.backend, ModelBackend::Gguf);
            assert!(m.min_ram_gb <= 8);
        }

        // MLX models for small tier
        let mlx_small = models_for_tier_and_backend(MemoryTier::Small, ModelBackend::Mlx);
        assert!(!mlx_small.is_empty());
        for m in &mlx_small {
            assert_eq!(m.backend, ModelBackend::Mlx);
            assert!(m.min_ram_gb <= 8);
        }

        // All GGUF models
        let all_gguf = models_for_tier_and_backend(MemoryTier::Large, ModelBackend::Gguf);
        for m in &all_gguf {
            assert_eq!(m.backend, ModelBackend::Gguf);
        }

        // All MLX models
        let all_mlx = models_for_tier_and_backend(MemoryTier::Large, ModelBackend::Mlx);
        for m in &all_mlx {
            assert_eq!(m.backend, ModelBackend::Mlx);
        }

        // Combined should equal total
        assert_eq!(all_gguf.len() + all_mlx.len(), MODEL_REGISTRY.len());
    }

    #[test]
    fn test_suggest_model_for_backend() {
        // Should suggest a GGUF model for GGUF backend
        let gguf_suggestion = suggest_model_for_backend(MemoryTier::Medium, ModelBackend::Gguf);
        assert!(gguf_suggestion.is_some());
        assert_eq!(gguf_suggestion.unwrap().backend, ModelBackend::Gguf);

        // Should suggest an MLX model for MLX backend
        let mlx_suggestion = suggest_model_for_backend(MemoryTier::Medium, ModelBackend::Mlx);
        assert!(mlx_suggestion.is_some());
        assert_eq!(mlx_suggestion.unwrap().backend, ModelBackend::Mlx);
    }

    #[test]
    fn test_model_backend_display() {
        assert_eq!(ModelBackend::Gguf.to_string(), "GGUF");
        assert_eq!(ModelBackend::Mlx.to_string(), "MLX");
    }

    #[test]
    fn test_is_gguf_model_cached_returns_false_when_not_exists() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path();

        // Get a GGUF model from registry
        let model = MODEL_REGISTRY
            .iter()
            .find(|m| m.backend == ModelBackend::Gguf)
            .expect("should have at least one GGUF model");

        // Model should not be cached in empty directory
        assert!(!is_gguf_model_cached(model, cache_dir));
    }

    #[test]
    fn test_is_gguf_model_cached_returns_true_when_exists() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path();

        // Get a GGUF model from registry
        let model = MODEL_REGISTRY
            .iter()
            .find(|m| m.backend == ModelBackend::Gguf)
            .expect("should have at least one GGUF model");

        // Create the model file
        let model_path = cache_dir.join(model.hf_filename);
        std::fs::write(&model_path, b"fake model content").unwrap();

        // Model should now be cached
        assert!(is_gguf_model_cached(model, cache_dir));
    }

    #[test]
    fn test_is_gguf_model_cached_returns_false_for_mlx_model() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path();

        // Get an MLX model from registry
        let model = MODEL_REGISTRY
            .iter()
            .find(|m| m.backend == ModelBackend::Mlx)
            .expect("should have at least one MLX model");

        // Should return false for MLX models (they use different caching)
        assert!(!is_gguf_model_cached(model, cache_dir));
    }

    #[test]
    fn test_is_mlx_model_cached_returns_false_when_not_exists() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path();

        // Get an MLX model from registry
        let model = MODEL_REGISTRY
            .iter()
            .find(|m| m.backend == ModelBackend::Mlx)
            .expect("should have at least one MLX model");

        // Model should not be cached in empty directory
        assert!(!is_mlx_model_cached(model, cache_dir));
    }

    #[test]
    fn test_is_mlx_model_cached_returns_true_when_exists() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path();

        // Get an MLX model from registry
        let model = MODEL_REGISTRY
            .iter()
            .find(|m| m.backend == ModelBackend::Mlx)
            .expect("should have at least one MLX model");

        // Create the model directory structure
        let model_dir_name = model.hf_repo.replace('/', "__");
        let model_dir = cache_dir.join("mlx").join(&model_dir_name);
        std::fs::create_dir_all(&model_dir).unwrap();

        // Create required files (config.json and either model.safetensors or index)
        std::fs::write(model_dir.join("config.json"), b"{}").unwrap();
        std::fs::write(model_dir.join("model.safetensors"), b"fake weights").unwrap();

        // Model should now be cached
        assert!(is_mlx_model_cached(model, cache_dir));
    }

    #[test]
    fn test_is_mlx_model_cached_with_sharded_model() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path();

        // Get an MLX model from registry
        let model = MODEL_REGISTRY
            .iter()
            .find(|m| m.backend == ModelBackend::Mlx)
            .expect("should have at least one MLX model");

        // Create the model directory structure
        let model_dir_name = model.hf_repo.replace('/', "__");
        let model_dir = cache_dir.join("mlx").join(&model_dir_name);
        std::fs::create_dir_all(&model_dir).unwrap();

        // Create config.json and index file (for sharded models)
        std::fs::write(model_dir.join("config.json"), b"{}").unwrap();
        std::fs::write(model_dir.join("model.safetensors.index.json"), b"{}").unwrap();

        // Model should be cached (index file instead of model.safetensors)
        assert!(is_mlx_model_cached(model, cache_dir));
    }

    #[test]
    fn test_is_mlx_model_cached_returns_false_for_gguf_model() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path();

        // Get a GGUF model from registry
        let model = MODEL_REGISTRY
            .iter()
            .find(|m| m.backend == ModelBackend::Gguf)
            .expect("should have at least one GGUF model");

        // Should return false for GGUF models
        assert!(!is_mlx_model_cached(model, cache_dir));
    }

    #[test]
    fn test_is_mlx_model_cached_returns_false_when_incomplete() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path();

        // Get an MLX model from registry
        let model = MODEL_REGISTRY
            .iter()
            .find(|m| m.backend == ModelBackend::Mlx)
            .expect("should have at least one MLX model");

        // Create the model directory structure
        let model_dir_name = model.hf_repo.replace('/', "__");
        let model_dir = cache_dir.join("mlx").join(&model_dir_name);
        std::fs::create_dir_all(&model_dir).unwrap();

        // Only create config.json (missing model.safetensors)
        std::fs::write(model_dir.join("config.json"), b"{}").unwrap();

        // Model should NOT be cached (incomplete)
        assert!(!is_mlx_model_cached(model, cache_dir));
    }

    #[test]
    fn test_is_model_cached_routes_to_correct_function() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path();

        // Test GGUF model
        let gguf_model = MODEL_REGISTRY
            .iter()
            .find(|m| m.backend == ModelBackend::Gguf)
            .expect("should have at least one GGUF model");

        // Create GGUF model file
        let gguf_path = cache_dir.join(gguf_model.hf_filename);
        std::fs::write(&gguf_path, b"fake").unwrap();
        assert!(is_model_cached(gguf_model, cache_dir));

        // Test MLX model
        let mlx_model = MODEL_REGISTRY
            .iter()
            .find(|m| m.backend == ModelBackend::Mlx)
            .expect("should have at least one MLX model");

        // MLX model not cached yet
        assert!(!is_model_cached(mlx_model, cache_dir));

        // Create MLX model directory
        let mlx_dir_name = mlx_model.hf_repo.replace('/', "__");
        let mlx_dir = cache_dir.join("mlx").join(&mlx_dir_name);
        std::fs::create_dir_all(&mlx_dir).unwrap();
        std::fs::write(mlx_dir.join("config.json"), b"{}").unwrap();
        std::fs::write(mlx_dir.join("model.safetensors"), b"fake").unwrap();

        assert!(is_model_cached(mlx_model, cache_dir));
    }

    #[test]
    fn test_find_mlx_model_in_legacy_registry() {
        // MLX models should be findable by their ID
        let model = find_model("mlx-llama-3.2-1b-4bit");
        assert!(model.is_some());
        let model = model.unwrap();
        assert_eq!(model.backend, ModelBackend::Mlx);
        assert_eq!(model.hf_repo, "mlx-community/Llama-3.2-1B-Instruct-4bit");
    }
}
