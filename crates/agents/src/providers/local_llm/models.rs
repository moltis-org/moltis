//! Model registry for local LLM models.
//!
//! Supports both GGUF and MLX model formats with automatic format selection
//! based on the current platform.

use std::path::PathBuf;

use {anyhow::Context, futures::StreamExt, tracing::info};

use super::{backend::BackendType, system_info::MemoryTier};

pub mod chat_templates;

pub use chat_templates::ChatTemplateHint;

/// Model format/backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelFormat {
    /// GGUF quantized model for llama.cpp
    Gguf,
    /// MLX format for Apple Silicon
    Mlx,
}

impl ModelFormat {
    /// Get the backend type for this format.
    #[must_use]
    pub fn backend_type(&self) -> BackendType {
        match self {
            Self::Gguf => BackendType::Gguf,
            Self::Mlx => BackendType::Mlx,
        }
    }
}

/// Definition of a local model in the registry.
#[derive(Debug, Clone)]
pub struct LocalModelDef {
    /// Model identifier (e.g., "qwen2.5-coder-7b-q4_k_m").
    pub id: &'static str,
    /// Human-readable display name.
    pub display_name: &'static str,
    /// HuggingFace repository for GGUF format.
    pub gguf_repo: &'static str,
    /// GGUF filename in the repository.
    pub gguf_filename: &'static str,
    /// HuggingFace repository for MLX format (if available).
    pub mlx_repo: Option<&'static str>,
    /// Minimum RAM required in GB.
    pub min_ram_gb: u32,
    /// Context window size in tokens.
    pub context_window: u32,
    /// Chat template hint for formatting messages.
    pub chat_template: Option<ChatTemplateHint>,
    /// Primary format for this model.
    pub format: ModelFormat,
}

impl LocalModelDef {
    /// HuggingFace download URL for the GGUF file.
    #[must_use]
    pub fn gguf_url(&self) -> String {
        format!(
            "https://huggingface.co/{}/resolve/main/{}",
            self.gguf_repo, self.gguf_filename
        )
    }

    /// Check if this model has an MLX version available.
    #[must_use]
    pub fn has_mlx(&self) -> bool {
        self.mlx_repo.is_some()
    }

    /// Get the best format for the given backend type.
    #[must_use]
    pub fn best_format_for(&self, backend: BackendType) -> ModelFormat {
        match backend {
            BackendType::Mlx if self.has_mlx() => ModelFormat::Mlx,
            _ => ModelFormat::Gguf,
        }
    }
}

/// Progress info for model downloads.
#[derive(Debug, Clone)]
pub struct DownloadProgress {
    /// Bytes downloaded so far.
    pub downloaded: u64,
    /// Total bytes (if known from Content-Length).
    pub total: Option<u64>,
}

/// Model registry — all known local models.
///
/// Models support both GGUF and MLX formats where available.
pub static MODEL_REGISTRY: &[LocalModelDef] = &[
    // ── 4GB tier (Tiny) ────────────────────────────────────────────────────
    LocalModelDef {
        id: "qwen2.5-coder-1.5b-q4_k_m",
        display_name: "Qwen 2.5 Coder 1.5B (Q4_K_M)",
        gguf_repo: "Qwen/Qwen2.5-Coder-1.5B-Instruct-GGUF",
        gguf_filename: "qwen2.5-coder-1.5b-instruct-q4_k_m.gguf",
        mlx_repo: Some("mlx-community/Qwen2.5-Coder-1.5B-Instruct-4bit"),
        min_ram_gb: 4,
        context_window: 32_768,
        chat_template: Some(ChatTemplateHint::ChatML),
        format: ModelFormat::Gguf,
    },
    LocalModelDef {
        id: "llama-3.2-1b-q4_k_m",
        display_name: "Llama 3.2 1B (Q4_K_M)",
        gguf_repo: "bartowski/Llama-3.2-1B-Instruct-GGUF",
        gguf_filename: "Llama-3.2-1B-Instruct-Q4_K_M.gguf",
        mlx_repo: Some("mlx-community/Llama-3.2-1B-Instruct-4bit"),
        min_ram_gb: 4,
        context_window: 128_000,
        chat_template: Some(ChatTemplateHint::Llama3),
        format: ModelFormat::Gguf,
    },
    // ── 8GB tier (Small) ───────────────────────────────────────────────────
    LocalModelDef {
        id: "qwen2.5-coder-7b-q4_k_m",
        display_name: "Qwen 2.5 Coder 7B (Q4_K_M)",
        gguf_repo: "Qwen/Qwen2.5-Coder-7B-Instruct-GGUF",
        gguf_filename: "qwen2.5-coder-7b-instruct-q4_k_m.gguf",
        mlx_repo: Some("mlx-community/Qwen2.5-Coder-7B-Instruct-4bit"),
        min_ram_gb: 8,
        context_window: 32_768,
        chat_template: Some(ChatTemplateHint::ChatML),
        format: ModelFormat::Gguf,
    },
    LocalModelDef {
        id: "llama-3.2-3b-q4_k_m",
        display_name: "Llama 3.2 3B (Q4_K_M)",
        gguf_repo: "bartowski/Llama-3.2-3B-Instruct-GGUF",
        gguf_filename: "Llama-3.2-3B-Instruct-Q4_K_M.gguf",
        mlx_repo: Some("mlx-community/Llama-3.2-3B-Instruct-4bit"),
        min_ram_gb: 8,
        context_window: 128_000,
        chat_template: Some(ChatTemplateHint::Llama3),
        format: ModelFormat::Gguf,
    },
    LocalModelDef {
        id: "deepseek-coder-6.7b-q4_k_m",
        display_name: "DeepSeek Coder 6.7B (Q4_K_M)",
        gguf_repo: "TheBloke/deepseek-coder-6.7B-instruct-GGUF",
        gguf_filename: "deepseek-coder-6.7b-instruct.Q4_K_M.gguf",
        mlx_repo: None, // No MLX version available
        min_ram_gb: 8,
        context_window: 16_384,
        chat_template: Some(ChatTemplateHint::DeepSeek),
        format: ModelFormat::Gguf,
    },
    // ── 16GB tier (Medium) ─────────────────────────────────────────────────
    LocalModelDef {
        id: "qwen2.5-coder-14b-q4_k_m",
        display_name: "Qwen 2.5 Coder 14B (Q4_K_M)",
        gguf_repo: "Qwen/Qwen2.5-Coder-14B-Instruct-GGUF",
        gguf_filename: "qwen2.5-coder-14b-instruct-q4_k_m.gguf",
        mlx_repo: Some("mlx-community/Qwen2.5-Coder-14B-Instruct-4bit"),
        min_ram_gb: 16,
        context_window: 32_768,
        chat_template: Some(ChatTemplateHint::ChatML),
        format: ModelFormat::Gguf,
    },
    LocalModelDef {
        id: "codestral-22b-q4_k_m",
        display_name: "Codestral 22B (Q4_K_M)",
        gguf_repo: "bartowski/Codestral-22B-v0.1-GGUF",
        gguf_filename: "Codestral-22B-v0.1-Q4_K_M.gguf",
        mlx_repo: Some("mlx-community/Codestral-22B-v0.1-4bit"),
        min_ram_gb: 16,
        context_window: 32_768,
        chat_template: Some(ChatTemplateHint::Mistral),
        format: ModelFormat::Gguf,
    },
    LocalModelDef {
        id: "mistral-7b-q5_k_m",
        display_name: "Mistral 7B Instruct (Q5_K_M)",
        gguf_repo: "TheBloke/Mistral-7B-Instruct-v0.2-GGUF",
        gguf_filename: "mistral-7b-instruct-v0.2.Q5_K_M.gguf",
        mlx_repo: Some("mlx-community/Mistral-7B-Instruct-v0.2-4bit"),
        min_ram_gb: 12,
        context_window: 32_768,
        chat_template: Some(ChatTemplateHint::Mistral),
        format: ModelFormat::Gguf,
    },
    LocalModelDef {
        id: "llama-3.1-8b-q4_k_m",
        display_name: "Llama 3.1 8B (Q4_K_M)",
        gguf_repo: "bartowski/Meta-Llama-3.1-8B-Instruct-GGUF",
        gguf_filename: "Meta-Llama-3.1-8B-Instruct-Q4_K_M.gguf",
        mlx_repo: Some("mlx-community/Meta-Llama-3.1-8B-Instruct-4bit"),
        min_ram_gb: 12,
        context_window: 128_000,
        chat_template: Some(ChatTemplateHint::Llama3),
        format: ModelFormat::Gguf,
    },
    // ── 32GB tier (Large) ──────────────────────────────────────────────────
    LocalModelDef {
        id: "qwen2.5-coder-32b-q4_k_m",
        display_name: "Qwen 2.5 Coder 32B (Q4_K_M)",
        gguf_repo: "Qwen/Qwen2.5-Coder-32B-Instruct-GGUF",
        gguf_filename: "qwen2.5-coder-32b-instruct-q4_k_m.gguf",
        mlx_repo: Some("mlx-community/Qwen2.5-Coder-32B-Instruct-4bit"),
        min_ram_gb: 32,
        context_window: 32_768,
        chat_template: Some(ChatTemplateHint::ChatML),
        format: ModelFormat::Gguf,
    },
    LocalModelDef {
        id: "deepseek-coder-33b-q4_k_m",
        display_name: "DeepSeek Coder 33B (Q4_K_M)",
        gguf_repo: "TheBloke/deepseek-coder-33B-instruct-GGUF",
        gguf_filename: "deepseek-coder-33b-instruct.Q4_K_M.gguf",
        mlx_repo: None,
        min_ram_gb: 32,
        context_window: 16_384,
        chat_template: Some(ChatTemplateHint::DeepSeek),
        format: ModelFormat::Gguf,
    },
    LocalModelDef {
        id: "llama-3.1-70b-q2_k",
        display_name: "Llama 3.1 70B (Q2_K)",
        gguf_repo: "bartowski/Meta-Llama-3.1-70B-Instruct-GGUF",
        gguf_filename: "Meta-Llama-3.1-70B-Instruct-Q2_K.gguf",
        mlx_repo: None, // Too large for most MLX setups
        min_ram_gb: 48,
        context_window: 128_000,
        chat_template: Some(ChatTemplateHint::Llama3),
        format: ModelFormat::Gguf,
    },
];

/// Find a model definition by ID.
#[must_use]
pub fn find_model(id: &str) -> Option<&'static LocalModelDef> {
    MODEL_REGISTRY.iter().find(|m| m.id == id)
}

/// Get models suitable for a given memory tier.
#[must_use]
pub fn models_for_tier(tier: MemoryTier) -> Vec<&'static LocalModelDef> {
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

/// Suggest the best model for a memory tier and backend.
#[must_use]
pub fn suggest_model(tier: MemoryTier, backend: BackendType) -> Option<&'static LocalModelDef> {
    let models = models_for_tier(tier);

    // Prefer models with MLX support if using MLX backend
    if backend == BackendType::Mlx {
        let mlx_models: Vec<_> = models.iter().filter(|m| m.has_mlx()).copied().collect();
        if !mlx_models.is_empty() {
            return mlx_models.into_iter().max_by_key(|m| m.min_ram_gb);
        }
    }

    // Otherwise return the largest model that fits
    models.into_iter().max_by_key(|m| m.min_ram_gb)
}

/// Get models that support a specific backend.
#[must_use]
pub fn models_for_backend(backend: BackendType) -> Vec<&'static LocalModelDef> {
    match backend {
        BackendType::Gguf => MODEL_REGISTRY.iter().collect(),
        BackendType::Mlx => MODEL_REGISTRY.iter().filter(|m| m.has_mlx()).collect(),
    }
}

/// Default cache directory for downloaded models.
#[must_use]
pub fn default_models_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "moltis")
        .map(|d| d.data_dir().join("models"))
        .unwrap_or_else(|| PathBuf::from(".moltis/models"))
}

/// Ensure a model is downloaded, returning the path to the file.
pub async fn ensure_model(model: &LocalModelDef, cache_dir: &PathBuf) -> anyhow::Result<PathBuf> {
    ensure_model_with_progress(model, cache_dir, |_| {}).await
}

/// Ensure a model is downloaded with progress reporting.
pub async fn ensure_model_with_progress<F>(
    model: &LocalModelDef,
    cache_dir: &PathBuf,
    mut on_progress: F,
) -> anyhow::Result<PathBuf>
where
    F: FnMut(DownloadProgress),
{
    let model_path = cache_dir.join(model.gguf_filename);
    if model_path.exists() {
        info!(path = %model_path.display(), model = model.id, "model found in cache");
        return Ok(model_path);
    }

    tokio::fs::create_dir_all(cache_dir)
        .await
        .context("creating models cache dir")?;

    let url = model.gguf_url();
    info!(url = %url, model = model.id, "downloading model");

    let response = reqwest::get(&url)
        .await
        .context("downloading GGUF model")?
        .error_for_status()
        .context("GGUF model download failed")?;

    let total = response.content_length();
    let mut downloaded: u64 = 0;

    on_progress(DownloadProgress { downloaded, total });

    let tmp_path = model_path.with_extension("tmp");
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

    tokio::fs::rename(&tmp_path, &model_path)
        .await
        .context("renaming model file")?;

    info!(
        path = %model_path.display(),
        size_mb = downloaded / (1024 * 1024),
        model = model.id,
        "model downloaded"
    );

    Ok(model_path)
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
    fn test_gguf_url() {
        let model = find_model("qwen2.5-coder-7b-q4_k_m").unwrap();
        let url = model.gguf_url();
        assert!(url.starts_with("https://huggingface.co/"));
        assert!(url.contains("Qwen"));
        assert!(url.ends_with(".gguf"));
    }

    #[test]
    fn test_has_mlx() {
        let qwen = find_model("qwen2.5-coder-7b-q4_k_m").unwrap();
        assert!(qwen.has_mlx());

        let deepseek = find_model("deepseek-coder-6.7b-q4_k_m").unwrap();
        assert!(!deepseek.has_mlx());
    }

    #[test]
    fn test_models_for_tier() {
        let tiny = models_for_tier(MemoryTier::Tiny);
        assert!(!tiny.is_empty());
        for m in &tiny {
            assert!(m.min_ram_gb <= 4);
        }

        let large = models_for_tier(MemoryTier::Large);
        assert_eq!(large.len(), MODEL_REGISTRY.len());
    }

    #[test]
    fn test_models_for_backend() {
        let gguf = models_for_backend(BackendType::Gguf);
        assert_eq!(gguf.len(), MODEL_REGISTRY.len());

        let mlx = models_for_backend(BackendType::Mlx);
        assert!(mlx.len() < MODEL_REGISTRY.len()); // Not all have MLX
        for m in &mlx {
            assert!(m.has_mlx());
        }
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
    fn test_suggest_model() {
        // Should always suggest something for each tier
        assert!(suggest_model(MemoryTier::Tiny, BackendType::Gguf).is_some());
        assert!(suggest_model(MemoryTier::Small, BackendType::Gguf).is_some());
        assert!(suggest_model(MemoryTier::Medium, BackendType::Gguf).is_some());
        assert!(suggest_model(MemoryTier::Large, BackendType::Gguf).is_some());
    }
}
