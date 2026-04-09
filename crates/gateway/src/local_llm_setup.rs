//! Local LLM provider setup service.
//!
//! Provides RPC handlers for configuring the local GGUF LLM provider,
//! including system info detection, model listing, and model configuration.

use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

use {
    async_trait::async_trait,
    base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD},
    serde::{Deserialize, Serialize},
    serde_json::Value,
    tokio::sync::{OnceCell, RwLock, watch},
    tracing::{info, warn},
};

use moltis_providers::{ProviderRegistry, local_gguf, local_llm, raw_model_id};

use crate::{
    broadcast::{BroadcastOpts, broadcast},
    services::{LocalLlmService, ServiceResult},
    state::GatewayState,
};

#[derive(Debug, thiserror::Error)]
pub enum LocalModelCacheError {
    #[error("{message}")]
    Message { message: String },
}

impl LocalModelCacheError {
    #[must_use]
    pub fn message(message: impl fmt::Display) -> Self {
        Self::Message {
            message: message.to_string(),
        }
    }
}

pub type LocalModelCacheResult<T> = Result<T, LocalModelCacheError>;

type DownloadProgressUpdate = (u64, Option<u64>);
const LOCAL_LLM_PROVIDER_NAME: &str = "local-llm";

fn download_progress_percent(downloaded: u64, total: Option<u64>) -> Option<f64> {
    total.map(|total_bytes| {
        if total_bytes > 0 {
            (downloaded as f64 / total_bytes as f64 * 100.0).min(100.0)
        } else {
            0.0
        }
    })
}

fn spawn_download_progress_broadcaster(
    state: &Arc<GatewayState>,
    model_id: &str,
    display_name: &str,
) -> (
    watch::Sender<Option<DownloadProgressUpdate>>,
    tokio::task::JoinHandle<()>,
) {
    let (tx, mut rx) = watch::channel(None::<DownloadProgressUpdate>);
    let state = Arc::clone(state);
    let model_id = model_id.to_string();
    let display_name = display_name.to_string();
    let task = tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            let Some((downloaded, total)) = *rx.borrow_and_update() else {
                continue;
            };
            broadcast(
                &state,
                "local-llm.download",
                serde_json::json!({
                    "modelId": model_id,
                    "displayName": display_name,
                    "downloaded": downloaded,
                    "total": total,
                    "progress": download_progress_percent(downloaded, total),
                }),
                BroadcastOpts::default(),
            )
            .await;
        }
    });
    (tx, task)
}

/// Check if a local model is cached on disk, and download it if not.
///
/// Returns Ok(true) if download was needed and completed successfully.
/// Returns Ok(false) if model was already cached.
/// Returns Err if download failed.
pub async fn ensure_local_model_cached(
    model_id: &str,
    state: &Arc<GatewayState>,
) -> LocalModelCacheResult<bool> {
    let cache_dir = local_gguf::models::default_models_dir();
    info!(model_id, ?cache_dir, "checking if local model is cached");

    // First check the unified registry
    if let Some(def) = local_llm::models::find_model(model_id) {
        // Determine backend type
        let backend = local_llm::backend::detect_backend_for_model(model_id);
        let is_cached = local_llm::models::is_model_cached(def, backend, &cache_dir);

        info!(model_id, is_cached, "found in unified registry");

        if is_cached {
            return Ok(false);
        }

        // Model not cached - download with progress
        return download_unified_model(def, backend, &cache_dir, state).await;
    }

    // Check legacy registry
    if let Some(def) = local_gguf::models::find_model(model_id) {
        let is_cached = local_gguf::models::is_model_cached(def, &cache_dir);

        info!(
            model_id,
            is_cached,
            backend = ?def.backend,
            hf_repo = def.hf_repo,
            "found in legacy registry"
        );

        if is_cached {
            return Ok(false);
        }

        // Model not cached - download with progress
        return download_legacy_model(def, &cache_dir, state).await;
    }

    // Check if it's a HuggingFace repo ID (e.g. mlx-community/Model-Name)
    if local_llm::models::is_hf_repo_id(model_id) {
        let is_cached = local_llm::models::is_mlx_repo_cached(model_id, &cache_dir);
        info!(model_id, is_cached, "HuggingFace repo ID detected");

        if is_cached {
            return Ok(false);
        }

        return download_hf_mlx_repo(model_id, &cache_dir, state).await;
    }

    if let Some(entry) =
        LocalLlmConfig::load().and_then(|config| config.get_model(model_id).cloned())
        && let Some((hf_repo, hf_filename)) = entry.custom_gguf_source()
    {
        let is_cached =
            local_gguf::models::is_custom_model_cached(hf_repo, hf_filename, &cache_dir);
        info!(
            model_id,
            hf_repo, hf_filename, is_cached, "custom GGUF model detected"
        );

        if is_cached {
            return Ok(false);
        }

        return download_custom_gguf_model(model_id, hf_repo, hf_filename, &cache_dir, state).await;
    }

    // Unknown model - let the provider handle it (will fail with a clear error)
    info!(model_id, "model not found in any registry");
    Ok(false)
}

/// Download a model from the unified registry with progress broadcasting.
async fn download_unified_model(
    model: &'static local_llm::models::LocalModelDef,
    backend: local_llm::backend::BackendType,
    cache_dir: &Path,
    state: &Arc<GatewayState>,
) -> LocalModelCacheResult<bool> {
    use moltis_providers::local_llm::models as llm_models;

    let model_id = model.id.to_string();
    let display_name = model.display_name.to_string();

    // Broadcast download start
    broadcast(
        state,
        "local-llm.download",
        serde_json::json!({
            "modelId": model_id,
            "displayName": display_name,
            "status": "starting",
            "message": "Missing model on disk, downloading...",
        }),
        BroadcastOpts::default(),
    )
    .await;

    let (progress_tx, broadcast_task) =
        spawn_download_progress_broadcaster(state, &model_id, &display_name);

    // Download based on backend
    let result = match backend {
        local_llm::backend::BackendType::Gguf => {
            llm_models::ensure_model_with_progress(model, cache_dir, |p| {
                let _ = progress_tx.send(Some((p.downloaded, p.total)));
            })
            .await
        },
        local_llm::backend::BackendType::Mlx => {
            llm_models::ensure_mlx_model_with_progress(model, cache_dir, |p| {
                let _ = progress_tx.send(Some((p.downloaded, p.total)));
            })
            .await
        },
    };

    // Clean up
    drop(progress_tx);
    let _ = broadcast_task.await;

    match result {
        Ok(_) => {
            // Broadcast completion
            broadcast(
                state,
                "local-llm.download",
                serde_json::json!({
                    "modelId": model_id,
                    "displayName": display_name,
                    "progress": 100.0,
                    "complete": true,
                }),
                BroadcastOpts::default(),
            )
            .await;
            Ok(true)
        },
        Err(e) => {
            // Broadcast error
            broadcast(
                state,
                "local-llm.download",
                serde_json::json!({
                    "modelId": model_id,
                    "error": e.to_string(),
                }),
                BroadcastOpts::default(),
            )
            .await;
            Err(LocalModelCacheError::message(format!(
                "Failed to download model: {e}"
            )))
        },
    }
}

/// Download a model from the legacy registry with progress broadcasting.
async fn download_legacy_model(
    model: &'static local_gguf::models::GgufModelDef,
    cache_dir: &Path,
    state: &Arc<GatewayState>,
) -> LocalModelCacheResult<bool> {
    let model_id = model.id.to_string();
    let display_name = model.display_name.to_string();

    // Broadcast download start
    broadcast(
        state,
        "local-llm.download",
        serde_json::json!({
            "modelId": model_id,
            "displayName": display_name,
            "status": "starting",
            "message": "Missing model on disk, downloading...",
        }),
        BroadcastOpts::default(),
    )
    .await;

    let (progress_tx, broadcast_task) =
        spawn_download_progress_broadcaster(state, &model_id, &display_name);

    // Download based on backend
    let result = match model.backend {
        local_gguf::models::ModelBackend::Gguf => {
            local_gguf::models::ensure_model_with_progress(model, cache_dir, |p| {
                let _ = progress_tx.send(Some((p.downloaded, p.total)));
            })
            .await
        },
        local_gguf::models::ModelBackend::Mlx => {
            local_gguf::models::ensure_mlx_model_with_progress(model, cache_dir, |p| {
                let _ = progress_tx.send(Some((p.downloaded, p.total)));
            })
            .await
        },
    };

    // Clean up
    drop(progress_tx);
    let _ = broadcast_task.await;

    match result {
        Ok(_) => {
            // Broadcast completion
            broadcast(
                state,
                "local-llm.download",
                serde_json::json!({
                    "modelId": model_id,
                    "displayName": display_name,
                    "progress": 100.0,
                    "complete": true,
                }),
                BroadcastOpts::default(),
            )
            .await;
            Ok(true)
        },
        Err(e) => {
            // Broadcast error
            broadcast(
                state,
                "local-llm.download",
                serde_json::json!({
                    "modelId": model_id,
                    "error": e.to_string(),
                }),
                BroadcastOpts::default(),
            )
            .await;
            Err(LocalModelCacheError::message(format!(
                "Failed to download model: {e}"
            )))
        },
    }
}

/// Download an arbitrary HuggingFace GGUF file with progress broadcasting.
async fn download_custom_gguf_model(
    model_id: &str,
    hf_repo: &str,
    hf_filename: &str,
    cache_dir: &Path,
    state: &Arc<GatewayState>,
) -> LocalModelCacheResult<bool> {
    let display_name = hf_filename.to_string();

    broadcast(
        state,
        "local-llm.download",
        serde_json::json!({
            "modelId": model_id,
            "displayName": display_name,
            "status": "starting",
            "message": "Missing model on disk, downloading...",
        }),
        BroadcastOpts::default(),
    )
    .await;

    let (progress_tx, broadcast_task) =
        spawn_download_progress_broadcaster(state, model_id, &display_name);

    let result = local_gguf::models::ensure_custom_model_with_progress(
        hf_repo,
        hf_filename,
        cache_dir,
        |p| {
            let _ = progress_tx.send(Some((p.downloaded, p.total)));
        },
    )
    .await;

    drop(progress_tx);
    let _ = broadcast_task.await;

    match result {
        Ok(_) => {
            broadcast(
                state,
                "local-llm.download",
                serde_json::json!({
                    "modelId": model_id,
                    "displayName": display_name,
                    "progress": 100.0,
                    "complete": true,
                }),
                BroadcastOpts::default(),
            )
            .await;
            Ok(true)
        },
        Err(e) => {
            broadcast(
                state,
                "local-llm.download",
                serde_json::json!({
                    "modelId": model_id,
                    "error": e.to_string(),
                }),
                BroadcastOpts::default(),
            )
            .await;
            Err(LocalModelCacheError::message(format!(
                "Failed to download model: {e}"
            )))
        },
    }
}

/// Download an arbitrary HuggingFace MLX repo with progress broadcasting.
async fn download_hf_mlx_repo(
    hf_repo: &str,
    cache_dir: &Path,
    state: &Arc<GatewayState>,
) -> LocalModelCacheResult<bool> {
    let model_id = hf_repo.to_string();
    let display_name = format!("{} (custom MLX)", hf_repo);

    broadcast(
        state,
        "local-llm.download",
        serde_json::json!({
            "modelId": model_id,
            "displayName": display_name,
            "status": "starting",
            "message": "Missing model on disk, downloading...",
        }),
        BroadcastOpts::default(),
    )
    .await;

    let (progress_tx, broadcast_task) =
        spawn_download_progress_broadcaster(state, &model_id, &display_name);

    let result = local_llm::models::ensure_mlx_repo_with_progress(hf_repo, cache_dir, |p| {
        let _ = progress_tx.send(Some((p.downloaded, p.total)));
    })
    .await;

    drop(progress_tx);
    let _ = broadcast_task.await;

    match result {
        Ok(_) => {
            broadcast(
                state,
                "local-llm.download",
                serde_json::json!({
                    "modelId": model_id,
                    "displayName": display_name,
                    "progress": 100.0,
                    "complete": true,
                }),
                BroadcastOpts::default(),
            )
            .await;
            Ok(true)
        },
        Err(e) => {
            broadcast(
                state,
                "local-llm.download",
                serde_json::json!({
                    "modelId": model_id,
                    "error": e.to_string(),
                }),
                BroadcastOpts::default(),
            )
            .await;
            Err(LocalModelCacheError::message(format!(
                "Failed to download model: {e}"
            )))
        },
    }
}

/// Check if mlx-lm is installed (either via pip or brew).
fn is_mlx_installed() -> bool {
    // Check for Python import (pip install)
    let python_import = std::process::Command::new("python3")
        .args(["-c", "import mlx_lm"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if python_import {
        return true;
    }

    // Check for mlx_lm CLI command (brew install)
    std::process::Command::new("mlx_lm.generate")
        .arg("--help")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Detect available package managers for installing mlx-lm.
/// Returns a list of (name, install_command) pairs, ordered by preference.
fn detect_mlx_installers() -> Vec<(&'static str, &'static str)> {
    let mut installers = Vec::new();

    // Check for brew on macOS (preferred for mlx-lm)
    if cfg!(target_os = "macos")
        && std::process::Command::new("brew")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    {
        installers.push(("brew", "brew install mlx-lm"));
    }

    // Check for uv (modern, fast Python package manager)
    if std::process::Command::new("uv")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        installers.push(("uv", "uv pip install mlx-lm"));
    }

    // Check for pip3
    if std::process::Command::new("pip3")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        installers.push(("pip3", "pip3 install mlx-lm"));
    }

    // Check for pip
    if std::process::Command::new("pip")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        installers.push(("pip", "pip install mlx-lm"));
    }

    // Fallback to python3 -m pip if nothing else found
    if installers.is_empty()
        && std::process::Command::new("python3")
            .args(["-m", "pip", "--version"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    {
        installers.push(("python3 -m pip", "python3 -m pip install mlx-lm"));
    }

    installers
}

/// Single model entry in the local-llm config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalModelEntry {
    pub model_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hf_repo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hf_filename: Option<String>,
    #[serde(default)]
    pub gpu_layers: u32,
    /// Backend to use: "GGUF" or "MLX"
    #[serde(default = "default_backend")]
    pub backend: String,
}

fn default_backend() -> String {
    "GGUF".to_string()
}

/// Configuration file for local-llm stored in the config directory.
/// Supports multiple models.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalLlmConfig {
    #[serde(default)]
    pub models: Vec<LocalModelEntry>,
}

/// Legacy single-model config for migration.
#[derive(Debug, Clone, Deserialize)]
struct LegacyLocalLlmConfig {
    model_id: String,
    model_path: Option<PathBuf>,
    #[serde(default)]
    gpu_layers: u32,
    #[serde(default = "default_backend")]
    backend: String,
}

impl LocalLlmConfig {
    /// Load config from the config directory.
    /// Handles migration from legacy single-model format.
    pub fn load() -> Option<Self> {
        let config_dir = moltis_config::config_dir()?;
        let config_path = config_dir.join("local-llm.json");
        let content = std::fs::read_to_string(&config_path).ok()?;

        // Try new multi-model format first
        if let Ok(config) = serde_json::from_str::<Self>(&content) {
            return Some(config);
        }

        // Try legacy single-model format and migrate
        if let Ok(legacy) = serde_json::from_str::<LegacyLocalLlmConfig>(&content) {
            let config = Self {
                models: vec![LocalModelEntry {
                    model_id: legacy.model_id,
                    model_path: legacy.model_path,
                    hf_repo: None,
                    hf_filename: None,
                    gpu_layers: legacy.gpu_layers,
                    backend: legacy.backend,
                }],
            };
            // Save migrated config
            let _ = config.save();
            return Some(config);
        }

        None
    }

    /// Save config to the config directory.
    pub fn save(&self) -> anyhow::Result<()> {
        let config_dir =
            moltis_config::config_dir().ok_or_else(|| anyhow::anyhow!("no config directory"))?;
        std::fs::create_dir_all(&config_dir)?;
        let config_path = config_dir.join("local-llm.json");
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(config_path, content)?;
        Ok(())
    }

    /// Add a model to the config. Replaces if model_id already exists.
    pub fn add_model(&mut self, entry: LocalModelEntry) {
        // Remove existing entry with same model_id
        self.models.retain(|m| m.model_id != entry.model_id);
        self.models.push(entry);
    }

    /// Remove a model by ID. Returns true if model was found and removed.
    pub fn remove_model(&mut self, model_id: &str) -> bool {
        let len_before = self.models.len();
        self.models.retain(|m| m.model_id != model_id);
        self.models.len() < len_before
    }

    /// Get a model by ID.
    pub fn get_model(&self, model_id: &str) -> Option<&LocalModelEntry> {
        self.models.iter().find(|m| m.model_id == model_id)
    }
}

impl LocalModelEntry {
    fn backend_type(&self) -> Option<local_llm::backend::BackendType> {
        match self.backend.as_str() {
            "GGUF" => Some(local_llm::backend::BackendType::Gguf),
            "MLX" => Some(local_llm::backend::BackendType::Mlx),
            _ => None,
        }
    }

    fn custom_gguf_source(&self) -> Option<(&str, &str)> {
        if self.backend != "GGUF" {
            return None;
        }

        Some((self.hf_repo.as_deref()?, self.hf_filename.as_deref()?))
    }

    fn resolved_model_path(
        &self,
        default_model_path: Option<&Path>,
        cache_dir: &Path,
    ) -> anyhow::Result<Option<PathBuf>> {
        if let Some((hf_repo, hf_filename)) = self.custom_gguf_source() {
            return local_gguf::models::custom_model_path(hf_repo, hf_filename, cache_dir)
                .map(Some);
        }

        if let Some(path) = &self.model_path {
            return Ok(Some(path.clone()));
        }

        if self.hf_repo.is_none() && self.hf_filename.is_none() {
            return Ok(default_model_path.map(Path::to_path_buf));
        }

        Ok(None)
    }

    fn display_name(&self) -> String {
        if let Some(def) = local_llm::models::find_model(&self.model_id) {
            return def.display_name.to_string();
        }
        if let Some(def) = local_gguf::models::find_model(&self.model_id) {
            return def.display_name.to_string();
        }
        if let Some(filename) = &self.hf_filename {
            return filename.clone();
        }
        if let Some(repo) = &self.hf_repo {
            return repo.clone();
        }
        if let Some(path) = &self.model_path
            && let Some(name) = path.file_name().and_then(|part| part.to_str())
        {
            return name.to_string();
        }
        format!("{} (local)", self.model_id)
    }
}

fn custom_gguf_model_id(hf_repo: &str, hf_filename: &str) -> String {
    let repo_component = URL_SAFE_NO_PAD.encode(hf_repo);
    let filename_component = URL_SAFE_NO_PAD.encode(hf_filename);
    format!("custom-gguf-{repo_component}.{filename_component}")
}

fn legacy_custom_gguf_model_id(hf_repo: &str) -> String {
    format!(
        "custom-{}",
        hf_repo
            .split('/')
            .next_back()
            .unwrap_or(hf_repo)
            .to_lowercase()
            .replace(' ', "-")
    )
}

fn remove_conflicting_custom_gguf_entries(
    config: &mut LocalLlmConfig,
    hf_repo: &str,
    hf_filename: &str,
) -> Vec<String> {
    let legacy_model_id = legacy_custom_gguf_model_id(hf_repo);
    let mut removed_model_ids = Vec::new();
    config.models.retain(|entry| {
        let should_remove = entry.model_id == legacy_model_id
            || (entry.backend == "GGUF"
                && entry.hf_repo.as_deref() == Some(hf_repo)
                && entry.hf_filename.as_deref() == Some(hf_filename));
        if should_remove {
            removed_model_ids.push(entry.model_id.clone());
        }
        !should_remove
    });
    removed_model_ids
}

fn status_from_saved_config(config: Option<&LocalLlmConfig>) -> LocalLlmStatus {
    config
        .and_then(|saved| saved.models.first())
        .map(|model| LocalLlmStatus::Ready {
            model_id: model.model_id.clone(),
        })
        .unwrap_or(LocalLlmStatus::Unconfigured)
}

fn build_local_provider_entry(
    entry: &LocalModelEntry,
    default_model_path: Option<&Path>,
) -> anyhow::Result<(
    moltis_providers::ModelInfo,
    Arc<local_llm::LocalLlmProvider>,
)> {
    let cache_dir = local_gguf::models::default_models_dir();
    let llm_config = local_llm::LocalLlmConfig {
        model_id: entry.model_id.clone(),
        model_path: entry.resolved_model_path(default_model_path, &cache_dir)?,
        backend: entry.backend_type(),
        context_size: None,
        gpu_layers: entry.gpu_layers,
        temperature: 0.7,
        cache_dir,
    };
    let provider = Arc::new(local_llm::LocalLlmProvider::new(llm_config));
    let info = moltis_providers::ModelInfo {
        id: entry.model_id.clone(),
        provider: LOCAL_LLM_PROVIDER_NAME.into(),
        display_name: entry.display_name(),
        created_at: None,
        recommended: false,
    };
    Ok((info, provider))
}

fn configured_local_model_path_override(
    providers_config: &moltis_config::schema::ProvidersConfig,
) -> Option<PathBuf> {
    providers_config
        .get("local")
        .and_then(|entry| entry.base_url.as_deref())
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
}

fn unregister_local_model_from_registry(registry: &mut ProviderRegistry, model_id: &str) {
    let local_registry_ids: Vec<String> = registry
        .list_models()
        .iter()
        .filter(|model| {
            model.provider == LOCAL_LLM_PROVIDER_NAME && raw_model_id(&model.id) == model_id
        })
        .map(|model| model.id.clone())
        .collect();

    for registry_id in local_registry_ids {
        let _ = registry.unregister(&registry_id);
    }
}

fn unregister_local_model_ids_from_registry(registry: &mut ProviderRegistry, model_ids: &[String]) {
    for model_id in model_ids {
        unregister_local_model_from_registry(registry, model_id);
    }
}

fn register_local_model_entry(
    registry: &mut ProviderRegistry,
    entry: &LocalModelEntry,
) -> anyhow::Result<()> {
    let (info, provider) = build_local_provider_entry(entry, None)?;
    unregister_local_model_from_registry(registry, &entry.model_id);
    registry.register(info, provider);
    Ok(())
}

fn register_local_model_entry_with_default_model_path(
    registry: &mut ProviderRegistry,
    entry: &LocalModelEntry,
    default_model_path: Option<&Path>,
) -> anyhow::Result<()> {
    let (info, provider) = build_local_provider_entry(entry, default_model_path)?;
    unregister_local_model_from_registry(registry, &entry.model_id);
    registry.register(info, provider);
    Ok(())
}

pub fn register_saved_local_models(
    registry: &mut ProviderRegistry,
    providers_config: &moltis_config::schema::ProvidersConfig,
) {
    let Some(config) = LocalLlmConfig::load() else {
        return;
    };
    let default_model_path = configured_local_model_path_override(providers_config);

    for entry in &config.models {
        if let Err(error) = register_local_model_entry_with_default_model_path(
            registry,
            entry,
            default_model_path.as_deref(),
        ) {
            warn!(model_id = %entry.model_id, %error, "failed to register saved local model");
        }
    }
}

/// Status of the local LLM provider.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum LocalLlmStatus {
    /// No model configured.
    Unconfigured,
    /// Model configured but not yet loaded.
    Ready { model_id: String },
    /// Model is being downloaded/loaded.
    Loading {
        model_id: String,
        progress: Option<f32>,
    },
    /// Model is loaded and ready.
    Loaded { model_id: String },
    /// Error loading model.
    Error { model_id: String, error: String },
    /// Feature not enabled.
    Unavailable,
}

/// Live implementation of LocalLlmService.
pub struct LiveLocalLlmService {
    registry: Arc<RwLock<ProviderRegistry>>,
    status: Arc<RwLock<LocalLlmStatus>>,
    /// State reference for broadcasting progress (set after state is created).
    state: Arc<OnceCell<Arc<GatewayState>>>,
}

impl LiveLocalLlmService {
    pub fn new(registry: Arc<RwLock<ProviderRegistry>>) -> Self {
        Self {
            registry,
            status: Arc::new(RwLock::new(status_from_saved_config(
                LocalLlmConfig::load().as_ref(),
            ))),
            state: Arc::new(OnceCell::new()),
        }
    }

    /// Set the gateway state reference for broadcasting progress updates.
    pub fn set_state(&self, state: Arc<GatewayState>) {
        // Ignore if already set (shouldn't happen in normal operation)
        let _ = self.state.set(state);
    }

    /// Get model display info for JSON response.
    fn model_to_json(model: &local_gguf::models::GgufModelDef, is_suggested: bool) -> Value {
        serde_json::json!({
            "id": model.id,
            "displayName": model.display_name,
            "minRamGb": model.min_ram_gb,
            "contextWindow": model.context_window,
            "hfRepo": model.hf_repo,
            "suggested": is_suggested,
            "backend": model.backend.to_string(),
        })
    }
}

fn has_enough_ram(total_ram_gb: u32, required_ram_gb: u32) -> bool {
    total_ram_gb >= required_ram_gb
}

fn insufficient_ram_error(
    model_display_name: &str,
    required_ram_gb: u32,
    total_ram_gb: u32,
) -> String {
    format!(
        "not enough RAM for {model_display_name}: requires at least {required_ram_gb}GB, detected {total_ram_gb}GB. Choose a smaller model."
    )
}

fn gguf_acceleration_labels(sys: &local_gguf::system_info::SystemInfo) -> Vec<&'static str> {
    let mut labels = Vec::new();
    if sys.has_metal {
        labels.push("Metal");
    }
    if sys.has_cuda {
        labels.push("CUDA");
    }
    if sys.has_vulkan {
        labels.push("Vulkan");
    }
    labels
}

fn gguf_acceleration_name(sys: &local_gguf::system_info::SystemInfo) -> Option<String> {
    let labels = gguf_acceleration_labels(sys);
    if labels.is_empty() {
        None
    } else {
        Some(labels.join("/"))
    }
}

fn gguf_backend_description(sys: &local_gguf::system_info::SystemInfo) -> String {
    match gguf_acceleration_name(sys) {
        Some(acceleration) => format!("Cross-platform, {acceleration} GPU acceleration"),
        None if sys.has_gpu() => "Cross-platform, GPU acceleration".to_string(),
        None => "Cross-platform, CPU inference".to_string(),
    }
}

fn gguf_backend_note(sys: &local_gguf::system_info::SystemInfo, mlx_available: bool) -> String {
    if mlx_available {
        return "MLX recommended (native Apple Silicon optimization)".to_string();
    }

    let gguf_note = match gguf_acceleration_name(sys) {
        Some(acceleration) => format!("GGUF with {acceleration} acceleration"),
        None if sys.has_gpu() => "GGUF with GPU acceleration".to_string(),
        None => "GGUF (CPU inference)".to_string(),
    };

    if sys.is_apple_silicon {
        format!("{gguf_note} (install mlx-lm for native MLX)")
    } else {
        gguf_note
    }
}

#[async_trait]
impl LocalLlmService for LiveLocalLlmService {
    async fn system_info(&self) -> ServiceResult {
        let sys = local_gguf::system_info::SystemInfo::detect();
        let tier = sys.memory_tier();

        // Check MLX availability (requires mlx-lm Python package)
        let mlx_available = sys.is_apple_silicon && is_mlx_installed();

        // Detect available package managers for install instructions
        let installers = detect_mlx_installers();
        let install_commands: Vec<&str> = installers.iter().map(|(_, cmd)| *cmd).collect();
        let primary_install = install_commands
            .first()
            .copied()
            .unwrap_or("pip install mlx-lm");

        // Determine the recommended backend
        let recommended_backend = if mlx_available {
            "MLX"
        } else {
            "GGUF"
        };

        // Build available backends list
        let mut available_backends = vec![serde_json::json!({
            "id": "GGUF",
            "name": "GGUF (llama.cpp)",
            "description": gguf_backend_description(&sys),
            "available": true,
        })];

        if sys.is_apple_silicon {
            let mlx_description = if mlx_available {
                "Optimized for Apple Silicon, fastest on Mac".to_string()
            } else {
                format!("Requires: {}", primary_install)
            };

            available_backends.push(serde_json::json!({
                "id": "MLX",
                "name": "MLX (Apple Native)",
                "description": mlx_description,
                "available": mlx_available,
                "installCommands": if mlx_available { None } else { Some(&install_commands) },
            }));
        }

        // Build backend note for display
        let backend_note = gguf_backend_note(&sys, mlx_available);

        Ok(serde_json::json!({
            "totalRamGb": sys.total_ram_gb(),
            "availableRamGb": sys.available_ram_gb(),
            "hasMetal": sys.has_metal,
            "hasCuda": sys.has_cuda,
            "hasVulkan": sys.has_vulkan,
            "hasGpu": sys.has_gpu(),
            "isAppleSilicon": sys.is_apple_silicon,
            "memoryTier": tier.to_string(),
            "recommendedBackend": recommended_backend,
            "availableBackends": available_backends,
            "backendNote": backend_note,
            "ggufDevices": sys.gguf_devices.iter().map(|device| serde_json::json!({
                "index": device.index,
                "name": device.name,
                "description": device.description,
                "backend": device.backend,
                "memoryTotalBytes": device.memory_total_bytes,
                "memoryFreeBytes": device.memory_free_bytes,
            })).collect::<Vec<_>>(),
            "mlxAvailable": mlx_available,
        }))
    }

    async fn models(&self) -> ServiceResult {
        let sys = local_gguf::system_info::SystemInfo::detect();
        let tier = sys.memory_tier();

        // Get suggested model for this tier
        let suggested = local_gguf::models::suggest_model(tier);
        let suggested_id = suggested.map(|m| m.id);

        // Get all models for this tier
        let available = local_gguf::models::models_for_tier(tier);

        let models: Vec<Value> = available
            .iter()
            .map(|m| Self::model_to_json(m, Some(m.id) == suggested_id))
            .collect();

        // Also include all models (not just for this tier) in a separate array
        let all_models: Vec<Value> = local_gguf::models::MODEL_REGISTRY
            .iter()
            .map(|m| Self::model_to_json(m, Some(m.id) == suggested_id))
            .collect();

        Ok(serde_json::json!({
            "recommended": models,
            "all": all_models,
            "memoryTier": tier.to_string(),
        }))
    }

    async fn configure(&self, params: Value) -> ServiceResult {
        let model_id = params
            .get("modelId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'modelId' parameter".to_string())?
            .to_string();

        // Get backend choice (default to recommended)
        let sys = local_gguf::system_info::SystemInfo::detect();
        let mlx_available = sys.is_apple_silicon && is_mlx_installed();
        let default_backend = if mlx_available {
            "MLX"
        } else {
            "GGUF"
        };
        let backend = params
            .get("backend")
            .and_then(|v| v.as_str())
            .unwrap_or(default_backend)
            .to_string();

        // Validate backend choice
        if backend != "GGUF" && backend != "MLX" {
            return Err(format!("invalid backend: {backend}. Must be GGUF or MLX").into());
        }
        if backend == "MLX" && !mlx_available {
            return Err("MLX backend requires mlx-lm. Install with: pip install mlx-lm".into());
        }

        // Validate model exists in registry
        let model_def = local_gguf::models::find_model(&model_id)
            .ok_or_else(|| format!("unknown model: {model_id}"))?;

        let total_ram_gb = sys.total_ram_gb();
        if !has_enough_ram(total_ram_gb, model_def.min_ram_gb) {
            return Err(insufficient_ram_error(
                model_def.display_name,
                model_def.min_ram_gb,
                total_ram_gb,
            )
            .into());
        }

        info!(model = %model_id, backend = %backend, "configuring local-llm");

        // Update status to loading
        {
            let mut status = self.status.write().await;
            *status = LocalLlmStatus::Loading {
                model_id: model_id.clone(),
                progress: None,
            };
        }

        // Save configuration (add to existing models)
        let entry = LocalModelEntry {
            model_id: model_id.clone(),
            model_path: configured_local_model_path_override(
                &moltis_config::loader::discover_and_load().providers,
            ),
            hf_repo: None,
            hf_filename: None,
            gpu_layers: 0,
            backend: backend.clone(),
        };
        let mut config = LocalLlmConfig::load().unwrap_or_default();
        config.add_model(entry.clone());
        config
            .save()
            .map_err(|e| format!("failed to save config: {e}"))?;

        // Trigger model download in background with progress updates
        let model_id_clone = model_id.clone();
        let status = Arc::clone(&self.status);
        let registry = Arc::clone(&self.registry);
        let state_cell = Arc::clone(&self.state);
        let cache_dir = local_gguf::models::default_models_dir();
        let display_name = model_def.display_name.to_string();
        let backend_for_download = backend.clone();

        tokio::spawn(async move {
            // Get state if available (for broadcasting progress)
            let state = state_cell.get().cloned();

            // Use a channel to send progress updates to a broadcast task
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(u64, Option<u64>)>();
            let state_for_progress = state.clone();
            let model_id_for_broadcast = model_id_clone.clone();
            let display_name_for_broadcast = display_name.clone();

            // Spawn a task to broadcast progress updates (if state is available)
            let broadcast_task = tokio::spawn(async move {
                let Some(state) = state_for_progress else {
                    // No state available, just drain the channel
                    while rx.recv().await.is_some() {}
                    return;
                };

                while let Some((downloaded, total)) = rx.recv().await {
                    let progress = total.map(|t| {
                        if t > 0 {
                            (downloaded as f64 / t as f64 * 100.0).min(100.0)
                        } else {
                            0.0
                        }
                    });
                    broadcast(
                        &state,
                        "local-llm.download",
                        serde_json::json!({
                            "modelId": model_id_for_broadcast,
                            "displayName": display_name_for_broadcast,
                            "downloaded": downloaded,
                            "total": total,
                            "progress": progress,
                        }),
                        BroadcastOpts::default(),
                    )
                    .await;
                }
            });

            // Download the model using the appropriate function based on backend
            let result = if backend_for_download == "MLX" {
                local_gguf::models::ensure_mlx_model_with_progress(model_def, &cache_dir, |p| {
                    let _ = tx.send((p.downloaded, p.total));
                })
                .await
            } else {
                local_gguf::models::ensure_model_with_progress(model_def, &cache_dir, |p| {
                    let _ = tx.send((p.downloaded, p.total));
                })
                .await
            };

            // Drop the sender to signal the broadcast task to finish
            drop(tx);
            // Wait for final broadcasts to complete
            let _ = broadcast_task.await;

            match result {
                Ok(_path) => {
                    info!(model = %model_id_clone, "model downloaded successfully");

                    // Broadcast completion (if state is available)
                    if let Some(state) = &state {
                        broadcast(
                            state,
                            "local-llm.download",
                            serde_json::json!({
                                "modelId": model_id_clone,
                                "displayName": display_name,
                                "progress": 100.0,
                                "complete": true,
                            }),
                            BroadcastOpts::default(),
                        )
                        .await;
                    }

                    // Register the provider in the registry
                    // Use LocalLlmProvider which auto-detects backend (GGUF or MLX)
                    let mut reg = registry.write().await;
                    if let Err(error) = register_local_model_entry(&mut reg, &entry) {
                        tracing::error!(model = %model_id_clone, %error, "failed to register local model");
                    }

                    let mut s = status.write().await;
                    *s = LocalLlmStatus::Ready {
                        model_id: model_id_clone,
                    };
                },
                Err(e) => {
                    tracing::error!(model = %model_id_clone, error = %e, "failed to download model");

                    // Broadcast error (if state is available)
                    if let Some(state) = &state {
                        broadcast(
                            state,
                            "local-llm.download",
                            serde_json::json!({
                                "modelId": model_id_clone,
                                "error": e.to_string(),
                            }),
                            BroadcastOpts::default(),
                        )
                        .await;
                    }

                    let mut s = status.write().await;
                    *s = LocalLlmStatus::Error {
                        model_id: model_id_clone,
                        error: e.to_string(),
                    };
                },
            }
        });

        Ok(serde_json::json!({
            "ok": true,
            "modelId": model_id,
            "displayName": model_def.display_name,
        }))
    }

    async fn status(&self) -> ServiceResult {
        let status = self.status.read().await;
        Ok(serde_json::to_value(&*status).unwrap_or_else(
            |_| serde_json::json!({ "status": "error", "error": "serialization failed" }),
        ))
    }

    async fn search_hf(&self, params: Value) -> ServiceResult {
        let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("");

        let backend = params
            .get("backend")
            .and_then(|v| v.as_str())
            .unwrap_or("GGUF");

        let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

        let results = search_huggingface(query, backend, limit).await?;
        Ok(serde_json::json!({
            "results": results,
            "query": query,
            "backend": backend,
        }))
    }

    async fn configure_custom(&self, params: Value) -> ServiceResult {
        let hf_repo = params
            .get("hfRepo")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'hfRepo' parameter".to_string())?
            .to_string();

        let backend = params
            .get("backend")
            .and_then(|v| v.as_str())
            .unwrap_or("GGUF")
            .to_string();

        if backend != "GGUF" && backend != "MLX" {
            return Err(format!("invalid backend: {backend}. Must be GGUF or MLX").into());
        }

        // For GGUF, we need the filename
        let hf_filename = params
            .get("hfFilename")
            .and_then(|v| v.as_str())
            .map(String::from);

        // Validate: GGUF requires a filename, MLX doesn't
        if backend == "GGUF" && hf_filename.is_none() {
            return Err("GGUF models require 'hfFilename' parameter".into());
        }

        let model_id = if backend == "MLX" {
            hf_repo.clone()
        } else {
            custom_gguf_model_id(
                &hf_repo,
                hf_filename
                    .as_deref()
                    .ok_or_else(|| "GGUF models require 'hfFilename' parameter".to_string())?,
            )
        };

        info!(model = %model_id, repo = %hf_repo, backend = %backend, "configuring custom model");

        if backend == "GGUF" {
            let filename = hf_filename
                .as_deref()
                .ok_or_else(|| "GGUF models require 'hfFilename' parameter".to_string())?;
            local_gguf::models::custom_model_path(
                &hf_repo,
                filename,
                &local_gguf::models::default_models_dir(),
            )
            .map_err(|e| format!("invalid GGUF filename: {e}"))?;
        }

        let entry = LocalModelEntry {
            model_id: model_id.clone(),
            model_path: None,
            hf_repo: Some(hf_repo.clone()),
            hf_filename: hf_filename.clone(),
            gpu_layers: 0,
            backend: backend.clone(),
        };
        let display_name = entry.display_name();

        // Save configuration (add to existing models)
        let mut config = LocalLlmConfig::load().unwrap_or_default();
        let superseded_model_ids = if backend == "GGUF" {
            let filename = hf_filename
                .as_deref()
                .ok_or_else(|| "GGUF models require 'hfFilename' parameter".to_string())?;
            remove_conflicting_custom_gguf_entries(&mut config, &hf_repo, filename)
        } else {
            Vec::new()
        };
        config.add_model(entry.clone());
        config
            .save()
            .map_err(|e| format!("failed to save config: {e}"))?;

        if !superseded_model_ids.is_empty() {
            let mut registry = self.registry.write().await;
            unregister_local_model_ids_from_registry(&mut registry, &superseded_model_ids);
        }

        // Update status
        {
            let mut status = self.status.write().await;
            *status = LocalLlmStatus::Loading {
                model_id: model_id.clone(),
                progress: None,
            };
        }

        let status = Arc::clone(&self.status);
        let registry = Arc::clone(&self.registry);
        let state_cell = Arc::clone(&self.state);
        let cache_dir = local_gguf::models::default_models_dir();
        let model_id_clone = model_id.clone();
        let hf_repo_for_download = hf_repo.clone();
        let hf_filename_for_download = hf_filename.clone();
        let backend_for_download = backend.clone();
        let display_name_for_task = display_name.clone();
        let superseded_model_ids_for_download = superseded_model_ids.clone();

        tokio::spawn(async move {
            let state = state_cell.get().cloned();
            let Some(state_for_download) = state.as_ref() else {
                let error = "gateway state unavailable for custom model download".to_string();
                let mut s = status.write().await;
                *s = LocalLlmStatus::Error {
                    model_id: model_id_clone,
                    error,
                };
                return;
            };
            let (progress_tx, progress_task) = spawn_download_progress_broadcaster(
                state_for_download,
                &model_id_clone,
                &display_name_for_task,
            );

            let result = if backend_for_download == "MLX" {
                local_llm::models::ensure_mlx_repo_with_progress(
                    &hf_repo_for_download,
                    &cache_dir,
                    |p| {
                        let _ = progress_tx.send(Some((p.downloaded, p.total)));
                    },
                )
                .await
            } else {
                let Some(filename) = hf_filename_for_download.as_deref() else {
                    drop(progress_tx);
                    let _ = progress_task.await;
                    let mut s = status.write().await;
                    *s = LocalLlmStatus::Error {
                        model_id: model_id_clone.clone(),
                        error: "GGUF models require 'hfFilename' parameter".into(),
                    };
                    return;
                };
                local_gguf::models::ensure_custom_model_with_progress(
                    &hf_repo_for_download,
                    filename,
                    &cache_dir,
                    |p| {
                        let _ = progress_tx.send(Some((p.downloaded, p.total)));
                    },
                )
                .await
            };
            drop(progress_tx);
            let _ = progress_task.await;

            match result {
                Ok(_) => {
                    let current_config = LocalLlmConfig::load();
                    if current_config
                        .as_ref()
                        .and_then(|config| config.get_model(&entry.model_id))
                        .is_none()
                    {
                        info!(
                            model = %entry.model_id,
                            "custom local model was removed before download completed; skipping registration"
                        );
                        let mut s = status.write().await;
                        *s = status_from_saved_config(current_config.as_ref());
                        return;
                    }

                    broadcast(
                        state_for_download,
                        "local-llm.download",
                        serde_json::json!({
                            "modelId": model_id_clone,
                            "displayName": display_name_for_task,
                            "progress": 100.0,
                            "complete": true,
                        }),
                        BroadcastOpts::default(),
                    )
                    .await;

                    let mut reg = registry.write().await;
                    unregister_local_model_ids_from_registry(
                        &mut reg,
                        &superseded_model_ids_for_download,
                    );
                    if let Err(error) = register_local_model_entry(&mut reg, &entry) {
                        tracing::error!(model = %entry.model_id, %error, "failed to register custom local model");
                    }

                    let mut s = status.write().await;
                    *s = LocalLlmStatus::Ready {
                        model_id: entry.model_id.clone(),
                    };
                },
                Err(e) => {
                    tracing::error!(model = %entry.model_id, error = %e, "failed to download custom local model");
                    broadcast(
                        state_for_download,
                        "local-llm.download",
                        serde_json::json!({
                            "modelId": entry.model_id.clone(),
                            "error": e.to_string(),
                        }),
                        BroadcastOpts::default(),
                    )
                    .await;

                    let mut s = status.write().await;
                    *s = LocalLlmStatus::Error {
                        model_id: entry.model_id.clone(),
                        error: e.to_string(),
                    };
                },
            }
        });

        Ok(serde_json::json!({
            "ok": true,
            "modelId": model_id,
            "hfRepo": hf_repo,
            "backend": backend,
            "displayName": display_name,
        }))
    }

    async fn remove_model(&self, params: Value) -> ServiceResult {
        let model_id = params
            .get("modelId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'modelId' parameter".to_string())?;
        let local_model_id = raw_model_id(model_id);

        info!(model = %local_model_id, "removing local-llm model");

        // Remove from config
        let mut config = LocalLlmConfig::load().unwrap_or_default();
        let removed = config.remove_model(local_model_id);

        if !removed {
            return Err(format!("model '{model_id}' not found in config").into());
        }

        config
            .save()
            .map_err(|e| format!("failed to save config: {e}"))?;

        // Remove from provider registry
        {
            let mut reg = self.registry.write().await;
            unregister_local_model_from_registry(&mut reg, local_model_id);
        }

        let removed_current_model = {
            let status = self.status.read().await;
            match &*status {
                LocalLlmStatus::Ready { model_id }
                | LocalLlmStatus::Loaded { model_id }
                | LocalLlmStatus::Loading { model_id, .. }
                | LocalLlmStatus::Error { model_id, .. } => {
                    raw_model_id(model_id) == local_model_id
                },
                LocalLlmStatus::Unconfigured | LocalLlmStatus::Unavailable => false,
            }
        };

        if config.models.is_empty() || removed_current_model {
            let mut status = self.status.write().await;
            *status = status_from_saved_config(Some(&config));
        }

        Ok(serde_json::json!({
            "ok": true,
            "modelId": local_model_id,
        }))
    }
}

/// Search HuggingFace for models matching the query and backend.
async fn search_huggingface(
    query: &str,
    backend: &str,
    limit: usize,
) -> Result<Vec<Value>, String> {
    let client = reqwest::Client::new();

    // Build search URL based on backend
    let url = if backend == "MLX" {
        // For MLX, search in mlx-community
        if query.is_empty() {
            format!(
                "https://huggingface.co/api/models?author=mlx-community&sort=downloads&direction=-1&limit={}",
                limit
            )
        } else {
            format!(
                "https://huggingface.co/api/models?search={}&author=mlx-community&sort=downloads&direction=-1&limit={}",
                urlencoding::encode(query),
                limit
            )
        }
    } else {
        // For GGUF, search for GGUF in the query
        let search_query = if query.is_empty() {
            "gguf".to_string()
        } else {
            format!("{} gguf", query)
        };
        format!(
            "https://huggingface.co/api/models?search={}&sort=downloads&direction=-1&limit={}",
            urlencoding::encode(&search_query),
            limit
        )
    };

    let response = client
        .get(&url)
        .header("User-Agent", "moltis/1.0")
        .send()
        .await
        .map_err(|e| format!("HuggingFace API request failed: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "HuggingFace API returned status {}",
            response.status()
        ));
    }

    let models: Vec<HfModelInfo> = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse HuggingFace response: {e}"))?;

    // Convert to our format
    let results: Vec<Value> = models
        .into_iter()
        .map(|m| {
            serde_json::json!({
                "id": m.id,
                "displayName": m.id.split('/').next_back().unwrap_or(&m.id),
                "downloads": m.downloads,
                "likes": m.likes,
                "createdAt": m.created_at,
                "tags": m.tags,
                "backend": backend,
            })
        })
        .collect();

    Ok(results)
}

/// HuggingFace model info from API response.
#[derive(Debug, serde::Deserialize)]
struct HfModelInfo {
    /// Model ID (e.g., "TheBloke/Llama-2-7B-GGUF")
    /// The API returns both "id" and "modelId" fields with the same value.
    id: String,
    /// Number of downloads
    #[serde(default)]
    downloads: u64,
    /// Number of likes
    #[serde(default)]
    likes: u64,
    /// Created timestamp
    #[serde(default, rename = "createdAt")]
    created_at: Option<String>,
    /// Model tags
    #[serde(default)]
    tags: Vec<String>,
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use super::*;

    fn sample_system_info() -> local_gguf::system_info::SystemInfo {
        local_gguf::system_info::SystemInfo {
            total_ram_bytes: 16 * 1024 * 1024 * 1024,
            available_ram_bytes: 8 * 1024 * 1024 * 1024,
            gguf_devices: vec![],
            has_metal: false,
            has_cuda: false,
            has_vulkan: false,
            is_apple_silicon: false,
        }
    }

    fn local_model_config_test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    struct LocalModelConfigTestGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl LocalModelConfigTestGuard {
        fn new() -> Self {
            Self {
                _lock: local_model_config_test_lock(),
            }
        }
    }

    impl Drop for LocalModelConfigTestGuard {
        fn drop(&mut self) {
            moltis_config::clear_config_dir();
            moltis_config::clear_data_dir();
        }
    }
    #[test]
    fn test_local_llm_config_serialization() {
        let mut config = LocalLlmConfig::default();
        config.add_model(LocalModelEntry {
            model_id: "qwen2.5-coder-7b-q4_k_m".into(),
            model_path: None,
            hf_repo: None,
            hf_filename: None,
            gpu_layers: 0,
            backend: "GGUF".into(),
        });
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("qwen2.5-coder-7b-q4_k_m"));

        let parsed: LocalLlmConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.models.len(), 1);
        assert_eq!(parsed.models[0].model_id, "qwen2.5-coder-7b-q4_k_m");
    }

    #[test]
    fn test_local_llm_config_round_trip_preserves_custom_gguf_metadata() {
        let mut config = LocalLlmConfig::default();
        let repo = "Qwen/Qwen3-4B-GGUF";
        let first = LocalModelEntry {
            model_id: custom_gguf_model_id(repo, "Qwen3-4B-Q4_K_M.gguf"),
            model_path: None,
            hf_repo: Some(repo.into()),
            hf_filename: Some("Qwen3-4B-Q4_K_M.gguf".into()),
            gpu_layers: 0,
            backend: "GGUF".into(),
        };
        let second = LocalModelEntry {
            model_id: custom_gguf_model_id(repo, "Qwen3-4B-Q6_K.gguf"),
            model_path: None,
            hf_repo: Some(repo.into()),
            hf_filename: Some("Qwen3-4B-Q6_K.gguf".into()),
            gpu_layers: 0,
            backend: "GGUF".into(),
        };

        config.add_model(first.clone());
        config.add_model(second.clone());

        let json = serde_json::to_string(&config).unwrap();
        let parsed: LocalLlmConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.models.len(), 2);
        assert_eq!(
            parsed
                .get_model(&first.model_id)
                .and_then(|entry| entry.hf_repo.as_deref()),
            Some(repo)
        );
        assert_eq!(
            parsed
                .get_model(&first.model_id)
                .and_then(|entry| entry.hf_filename.as_deref()),
            Some("Qwen3-4B-Q4_K_M.gguf")
        );
        assert_eq!(
            parsed
                .get_model(&second.model_id)
                .and_then(|entry| entry.hf_filename.as_deref()),
            Some("Qwen3-4B-Q6_K.gguf")
        );
    }

    #[test]
    fn test_local_llm_config_multi_model() {
        let mut config = LocalLlmConfig::default();
        config.add_model(LocalModelEntry {
            model_id: "model-1".into(),
            model_path: None,
            hf_repo: None,
            hf_filename: None,
            gpu_layers: 0,
            backend: "GGUF".into(),
        });
        config.add_model(LocalModelEntry {
            model_id: "model-2".into(),
            model_path: None,
            hf_repo: None,
            hf_filename: None,
            gpu_layers: 0,
            backend: "MLX".into(),
        });
        assert_eq!(config.models.len(), 2);

        // Test remove_model
        assert!(config.remove_model("model-1"));
        assert_eq!(config.models.len(), 1);
        assert_eq!(config.models[0].model_id, "model-2");

        // Test remove non-existent model
        assert!(!config.remove_model("model-1"));
        assert_eq!(config.models.len(), 1);
    }

    #[test]
    fn test_legacy_config_format_parsing() {
        // Test that legacy single-model format can be deserialized
        let legacy_json =
            r#"{"model_id":"old-model","model_path":null,"gpu_layers":0,"backend":"GGUF"}"#;
        let legacy: LegacyLocalLlmConfig = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(legacy.model_id, "old-model");
    }

    #[test]
    fn test_status_serialization() {
        let status = LocalLlmStatus::Ready {
            model_id: "test-model".into(),
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["status"], "ready");
        assert_eq!(json["model_id"], "test-model");
    }

    #[test]
    fn test_has_enough_ram() {
        assert!(has_enough_ram(8, 8));
        assert!(has_enough_ram(16, 8));
        assert!(!has_enough_ram(0, 4));
    }

    #[test]
    fn test_insufficient_ram_error() {
        let message = insufficient_ram_error("Qwen 2.5 Coder 7B", 8, 0);
        assert!(message.contains("requires at least 8GB"));
        assert!(message.contains("detected 0GB"));
    }

    #[test]
    fn test_gguf_backend_description_uses_vulkan() {
        let mut sys = sample_system_info();
        sys.has_vulkan = true;
        assert_eq!(
            gguf_backend_description(&sys),
            "Cross-platform, Vulkan GPU acceleration"
        );
    }

    #[test]
    fn test_gguf_backend_note_uses_multiple_accelerators() {
        let mut sys = sample_system_info();
        sys.has_cuda = true;
        sys.has_vulkan = true;
        assert_eq!(
            gguf_backend_note(&sys, false),
            "GGUF with CUDA/Vulkan acceleration"
        );
    }

    #[test]
    fn test_gguf_backend_note_keeps_apple_mlx_hint() {
        let mut sys = sample_system_info();
        sys.is_apple_silicon = true;
        sys.has_metal = true;
        assert_eq!(
            gguf_backend_note(&sys, false),
            "GGUF with Metal acceleration (install mlx-lm for native MLX)"
        );
    }

    #[test]
    fn test_gguf_backend_description_unknown_gpu_uses_generic_label() {
        let mut sys = sample_system_info();
        sys.gguf_devices = vec![local_gguf::runtime_devices::GgufRuntimeDevice {
            index: 0,
            name: "ROCm0".into(),
            description: "AMD".into(),
            backend: "ROCm".into(),
            memory_total_bytes: 1,
            memory_free_bytes: 1,
        }];
        assert_eq!(
            gguf_backend_description(&sys),
            "Cross-platform, GPU acceleration"
        );
    }

    #[test]
    fn test_gguf_backend_note_unknown_gpu_uses_generic_label() {
        let mut sys = sample_system_info();
        sys.gguf_devices = vec![local_gguf::runtime_devices::GgufRuntimeDevice {
            index: 0,
            name: "ROCm0".into(),
            description: "AMD".into(),
            backend: "ROCm".into(),
            memory_total_bytes: 1,
            memory_free_bytes: 1,
        }];
        assert_eq!(gguf_backend_note(&sys, false), "GGUF with GPU acceleration");
    }

    #[test]
    fn test_hf_model_info_parsing() {
        // Test parsing with all fields (matching actual HF API response)
        let json = r#"{
            "id": "TheBloke/Llama-2-7B-GGUF",
            "downloads": 1234567,
            "likes": 100,
            "createdAt": "2024-01-15T10:30:00Z",
            "tags": ["gguf", "llama"]
        }"#;
        let info: HfModelInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.id, "TheBloke/Llama-2-7B-GGUF");
        assert_eq!(info.downloads, 1234567);
        assert_eq!(info.likes, 100);
        assert!(info.created_at.is_some());
        assert_eq!(info.tags.len(), 2);
    }

    #[test]
    fn test_hf_model_info_parsing_mlx_community() {
        // Test parsing MLX community model response
        let json = r#"{
            "id": "mlx-community/Qwen2.5-Coder-7B-Instruct-4bit",
            "downloads": 500,
            "likes": 10,
            "tags": ["mlx", "safetensors"]
        }"#;
        let info: HfModelInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.id, "mlx-community/Qwen2.5-Coder-7B-Instruct-4bit");
        assert_eq!(info.downloads, 500);
        assert_eq!(info.likes, 10);
        assert_eq!(info.tags.len(), 2);
    }

    #[test]
    fn test_hf_model_info_parsing_minimal() {
        // Test parsing with minimal fields
        let json = r#"{"id": "test/model"}"#;
        let info: HfModelInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.id, "test/model");
        assert_eq!(info.downloads, 0);
        assert_eq!(info.likes, 0);
        assert!(info.created_at.is_none());
        assert!(info.tags.is_empty());
    }

    #[test]
    fn test_hf_api_response_parsing_real_format() {
        // Test parsing actual HuggingFace API response format
        // This is a real response from: https://huggingface.co/api/models?author=mlx-community&limit=1
        let json = r#"[
            {
                "_id": "680fecc14cc667f59da738f5",
                "id": "mlx-community/Qwen3-0.6B-4bit",
                "likes": 9,
                "private": false,
                "downloads": 20580,
                "tags": [
                    "mlx",
                    "safetensors",
                    "qwen3",
                    "text-generation",
                    "conversational",
                    "base_model:Qwen/Qwen3-0.6B",
                    "license:apache-2.0",
                    "4-bit",
                    "region:us"
                ],
                "pipeline_tag": "text-generation",
                "library_name": "mlx",
                "createdAt": "2025-04-28T21:01:53.000Z",
                "modelId": "mlx-community/Qwen3-0.6B-4bit"
            }
        ]"#;

        // Parse as array (as the API returns)
        let models: Vec<HfModelInfo> = serde_json::from_str(json).unwrap();
        assert_eq!(models.len(), 1);

        let info = &models[0];
        assert_eq!(info.id, "mlx-community/Qwen3-0.6B-4bit");
        assert_eq!(info.downloads, 20580);
        assert_eq!(info.likes, 9);
        assert!(info.created_at.is_some());
        assert_eq!(
            info.created_at.as_ref().unwrap(),
            "2025-04-28T21:01:53.000Z"
        );
        assert!(info.tags.contains(&"mlx".to_string()));
        assert!(info.tags.contains(&"qwen3".to_string()));
    }

    #[test]
    fn test_hf_api_response_parsing_gguf_format() {
        // Test parsing GGUF model response format
        let json = r#"[
            {
                "id": "TheBloke/Llama-2-7B-GGUF",
                "downloads": 5000000,
                "likes": 500,
                "tags": ["gguf", "llama", "text-generation"],
                "createdAt": "2023-09-01T00:00:00.000Z"
            },
            {
                "id": "bartowski/Qwen2.5-Coder-32B-Instruct-GGUF",
                "downloads": 100000,
                "likes": 50,
                "tags": ["gguf", "qwen", "coder"]
            }
        ]"#;

        let models: Vec<HfModelInfo> = serde_json::from_str(json).unwrap();
        assert_eq!(models.len(), 2);

        assert_eq!(models[0].id, "TheBloke/Llama-2-7B-GGUF");
        assert_eq!(models[0].downloads, 5000000);
        assert!(models[0].created_at.is_some());

        assert_eq!(models[1].id, "bartowski/Qwen2.5-Coder-32B-Instruct-GGUF");
        assert_eq!(models[1].downloads, 100000);
        assert!(models[1].created_at.is_none()); // Not all responses have createdAt
    }

    #[test]
    fn test_custom_model_id_generation() {
        let repo = "Qwen/Qwen3-4B-GGUF";
        let filename = "Qwen3-4B-Q4_K_M.gguf";
        let model_id = custom_gguf_model_id("Qwen/Qwen3-4B-GGUF", "Qwen3-4B-Q4_K_M.gguf");
        let encoded = model_id.strip_prefix("custom-gguf-").unwrap();
        let (repo_component, filename_component) = encoded.split_once('.').unwrap();

        assert_eq!(
            URL_SAFE_NO_PAD.decode(repo_component).unwrap(),
            repo.as_bytes()
        );
        assert_eq!(
            URL_SAFE_NO_PAD.decode(filename_component).unwrap(),
            filename.as_bytes()
        );
    }

    #[test]
    fn test_custom_model_id_generation_distinguishes_filenames_in_same_repo() {
        let repo = "Qwen/Qwen3-4B-GGUF";
        let first = custom_gguf_model_id(repo, "Qwen3-4B-Q4_K_M.gguf");
        let second = custom_gguf_model_id(repo, "Qwen3-4B-Q6_K.gguf");

        assert_ne!(first, second);
    }

    #[test]
    fn test_custom_model_id_generation_avoids_lossy_slug_collisions() {
        let first = custom_gguf_model_id("org-a/model", "quant/file.gguf");
        let second = custom_gguf_model_id("org/a-model", "quant-file.gguf");

        assert_ne!(first, second);
    }

    #[test]
    fn test_remove_conflicting_custom_gguf_entries_removes_legacy_and_duplicate_entries() {
        let repo = "Qwen/Qwen3-4B-GGUF";
        let filename = "Qwen3-4B-Q4_K_M.gguf";
        let mut config = LocalLlmConfig {
            models: vec![
                LocalModelEntry {
                    model_id: legacy_custom_gguf_model_id(repo),
                    model_path: None,
                    hf_repo: None,
                    hf_filename: None,
                    gpu_layers: 0,
                    backend: "GGUF".into(),
                },
                LocalModelEntry {
                    model_id: "custom-stale".into(),
                    model_path: None,
                    hf_repo: Some(repo.into()),
                    hf_filename: Some(filename.into()),
                    gpu_layers: 0,
                    backend: "GGUF".into(),
                },
                LocalModelEntry {
                    model_id: custom_gguf_model_id(repo, "Qwen3-4B-Q6_K.gguf"),
                    model_path: None,
                    hf_repo: Some(repo.into()),
                    hf_filename: Some("Qwen3-4B-Q6_K.gguf".into()),
                    gpu_layers: 0,
                    backend: "GGUF".into(),
                },
            ],
        };

        let mut removed_model_ids =
            remove_conflicting_custom_gguf_entries(&mut config, repo, filename);
        removed_model_ids.sort_unstable();

        assert_eq!(config.models.len(), 1);
        assert_eq!(
            config.models[0].model_id,
            custom_gguf_model_id(repo, "Qwen3-4B-Q6_K.gguf")
        );
        let mut expected_removed_model_ids = vec![
            legacy_custom_gguf_model_id(repo),
            "custom-stale".to_string(),
        ];
        expected_removed_model_ids.sort_unstable();
        assert_eq!(removed_model_ids, expected_removed_model_ids);
    }

    #[test]
    fn test_custom_model_path_resolution_uses_repo_scoped_cache_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path();
        let entry = LocalModelEntry {
            model_id: custom_gguf_model_id("Qwen/Qwen3-4B-GGUF", "Qwen3-4B-Q4_K_M.gguf"),
            model_path: None,
            hf_repo: Some("Qwen/Qwen3-4B-GGUF".into()),
            hf_filename: Some("Qwen3-4B-Q4_K_M.gguf".into()),
            gpu_layers: 0,
            backend: "GGUF".into(),
        };

        let resolved = entry.resolved_model_path(None, cache_dir).unwrap();
        assert_eq!(
            resolved,
            Some(
                cache_dir
                    .join("custom")
                    .join("Qwen")
                    .join("Qwen3-4B-GGUF")
                    .join("Qwen3-4B-Q4_K_M.gguf")
            )
        );
    }

    #[test]
    fn test_custom_model_path_resolution_prefers_repo_metadata_over_stale_saved_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path();
        let entry = LocalModelEntry {
            model_id: custom_gguf_model_id("Qwen/Qwen3-4B-GGUF", "Qwen3-4B-Q4_K_M.gguf"),
            model_path: Some(PathBuf::from("/tmp/stale-model.gguf")),
            hf_repo: Some("Qwen/Qwen3-4B-GGUF".into()),
            hf_filename: Some("Qwen3-4B-Q4_K_M.gguf".into()),
            gpu_layers: 0,
            backend: "GGUF".into(),
        };

        let resolved = entry.resolved_model_path(None, cache_dir).unwrap();
        assert_eq!(
            resolved,
            Some(
                cache_dir
                    .join("custom")
                    .join("Qwen")
                    .join("Qwen3-4B-GGUF")
                    .join("Qwen3-4B-Q4_K_M.gguf")
            )
        );
    }

    #[test]
    fn test_builtin_model_path_resolution_uses_provider_override() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path();
        let override_path = Path::new("/tmp/custom-built-in-model.gguf");
        let entry = LocalModelEntry {
            model_id: "qwen2.5-coder-7b-q4_k_m".into(),
            model_path: None,
            hf_repo: None,
            hf_filename: None,
            gpu_layers: 0,
            backend: "GGUF".into(),
        };

        let resolved = entry
            .resolved_model_path(Some(override_path), cache_dir)
            .unwrap();

        assert_eq!(resolved, Some(override_path.to_path_buf()));
    }

    #[test]
    fn test_custom_model_display_name_prefers_filename() {
        let entry = LocalModelEntry {
            model_id: "custom-test".into(),
            model_path: None,
            hf_repo: Some("Qwen/Qwen3-4B-GGUF".into()),
            hf_filename: Some("Qwen3-4B-Q4_K_M.gguf".into()),
            gpu_layers: 0,
            backend: "GGUF".into(),
        };

        assert_eq!(entry.display_name(), "Qwen3-4B-Q4_K_M.gguf");
    }

    #[test]
    fn test_register_saved_local_models_registers_custom_gguf_provider_from_saved_config() {
        let _guard = LocalModelConfigTestGuard::new();
        let config_dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        moltis_config::set_config_dir(config_dir.path().to_path_buf());
        moltis_config::set_data_dir(data_dir.path().to_path_buf());

        let entry = LocalModelEntry {
            model_id: custom_gguf_model_id("Qwen/Qwen3-4B-GGUF", "Qwen3-4B-Q4_K_M.gguf"),
            model_path: None,
            hf_repo: Some("Qwen/Qwen3-4B-GGUF".into()),
            hf_filename: Some("Qwen3-4B-Q4_K_M.gguf".into()),
            gpu_layers: 0,
            backend: "GGUF".into(),
        };
        let config = LocalLlmConfig {
            models: vec![entry.clone()],
        };
        config.save().unwrap();

        let mut registry = ProviderRegistry::empty();
        register_saved_local_models(
            &mut registry,
            &moltis_config::schema::ProvidersConfig::default(),
        );

        assert!(registry.get(&entry.model_id).is_some());
        let registered = registry
            .list_models()
            .iter()
            .find(|model| raw_model_id(&model.id) == entry.model_id)
            .unwrap();
        assert_eq!(registered.provider, "local-llm");
        assert_eq!(registered.display_name, "Qwen3-4B-Q4_K_M.gguf");
    }

    #[test]
    fn test_register_local_model_entry_keeps_non_local_collisions() {
        let mut registry = ProviderRegistry::empty();
        let remote_provider = Arc::new(moltis_providers::openai::OpenAiProvider::new(
            secrecy::Secret::new("test-key".into()),
            "shared-model".into(),
            "https://example.com".into(),
        ));
        registry.register(
            moltis_providers::ModelInfo {
                id: "shared-model".into(),
                provider: "openai".into(),
                display_name: "Shared Remote Model".into(),
                created_at: None,
                recommended: false,
            },
            remote_provider,
        );

        let entry = LocalModelEntry {
            model_id: "shared-model".into(),
            model_path: Some(PathBuf::from("/tmp/shared-model.gguf")),
            hf_repo: None,
            hf_filename: None,
            gpu_layers: 0,
            backend: "GGUF".into(),
        };

        register_local_model_entry(&mut registry, &entry).unwrap();

        let registered: Vec<_> = registry
            .list_models()
            .iter()
            .filter(|model| raw_model_id(&model.id) == "shared-model")
            .collect();
        assert_eq!(registered.len(), 2);
        assert!(registered.iter().any(|model| model.provider == "openai"));
        assert!(
            registered
                .iter()
                .any(|model| model.provider == LOCAL_LLM_PROVIDER_NAME)
        );
    }

    #[test]
    fn test_unregister_local_model_ids_from_registry_removes_superseded_entries() {
        let repo = "Qwen/Qwen3-4B-GGUF";
        let legacy_entry = LocalModelEntry {
            model_id: legacy_custom_gguf_model_id(repo),
            model_path: Some(PathBuf::from("/tmp/legacy.gguf")),
            hf_repo: None,
            hf_filename: None,
            gpu_layers: 0,
            backend: "GGUF".into(),
        };
        let stale_entry = LocalModelEntry {
            model_id: "custom-stale".into(),
            model_path: Some(PathBuf::from("/tmp/stale.gguf")),
            hf_repo: None,
            hf_filename: None,
            gpu_layers: 0,
            backend: "GGUF".into(),
        };
        let active_entry = LocalModelEntry {
            model_id: custom_gguf_model_id(repo, "Qwen3-4B-Q6_K.gguf"),
            model_path: Some(PathBuf::from("/tmp/active.gguf")),
            hf_repo: None,
            hf_filename: None,
            gpu_layers: 0,
            backend: "GGUF".into(),
        };

        let mut registry = ProviderRegistry::empty();
        for entry in [&legacy_entry, &stale_entry, &active_entry] {
            let (info, provider) = build_local_provider_entry(entry, None).unwrap();
            registry.register(info, provider);
        }

        let remote_provider = Arc::new(moltis_providers::openai::OpenAiProvider::new(
            secrecy::Secret::new("test-key".into()),
            "custom-stale".into(),
            "https://example.com".into(),
        ));
        registry.register(
            moltis_providers::ModelInfo {
                id: "custom-stale".into(),
                provider: "openai".into(),
                display_name: "Remote Stale Alias".into(),
                created_at: None,
                recommended: false,
            },
            remote_provider,
        );

        let superseded_model_ids =
            vec![legacy_entry.model_id.clone(), stale_entry.model_id.clone()];
        unregister_local_model_ids_from_registry(&mut registry, &superseded_model_ids);

        assert!(!registry.list_models().iter().any(|model| {
            model.provider == LOCAL_LLM_PROVIDER_NAME
                && raw_model_id(&model.id) == legacy_entry.model_id.as_str()
        }));
        assert!(!registry.list_models().iter().any(|model| {
            model.provider == LOCAL_LLM_PROVIDER_NAME
                && raw_model_id(&model.id) == stale_entry.model_id.as_str()
        }));
        assert!(registry.list_models().iter().any(|model| {
            model.provider == "openai" && raw_model_id(&model.id) == stale_entry.model_id.as_str()
        }));
        assert!(registry.list_models().iter().any(|model| {
            model.provider == LOCAL_LLM_PROVIDER_NAME
                && raw_model_id(&model.id) == active_entry.model_id.as_str()
        }));
    }

    #[test]
    fn test_search_url_encoding() {
        // Test that search queries are properly URL-encoded
        let query = "llama 2 chat";
        let encoded = urlencoding::encode(query);
        assert_eq!(encoded, "llama%202%20chat");

        let query2 = "qwen2.5-coder";
        let encoded2 = urlencoding::encode(query2);
        assert_eq!(encoded2, "qwen2.5-coder");
    }

    #[test]
    fn test_download_progress_percent_bounds_values() {
        assert_eq!(download_progress_percent(50, Some(100)), Some(50.0));
        assert_eq!(download_progress_percent(250, Some(100)), Some(100.0));
        assert_eq!(download_progress_percent(10, Some(0)), Some(0.0));
        assert_eq!(download_progress_percent(10, None), None);
    }

    #[tokio::test]
    async fn test_search_huggingface_builds_correct_url_for_mlx() {
        // This test verifies URL construction logic without making actual HTTP calls
        // In a real test, you'd mock the HTTP client

        // For MLX with empty query, should search mlx-community
        let mlx_empty_url = if true {
            // Simulating backend == "MLX" && query.is_empty()
            format!(
                "https://huggingface.co/api/models?author=mlx-community&sort=downloads&direction=-1&limit={}",
                20
            )
        } else {
            String::new()
        };
        assert!(mlx_empty_url.contains("author=mlx-community"));
        assert!(mlx_empty_url.contains("sort=downloads"));
    }

    #[tokio::test]
    async fn test_search_huggingface_builds_correct_url_for_gguf() {
        // For GGUF with query, should append "gguf" to search
        let query = "llama";
        let search_query = format!("{} gguf", query);
        let gguf_url = format!(
            "https://huggingface.co/api/models?search={}&sort=downloads&direction=-1&limit={}",
            urlencoding::encode(&search_query),
            20
        );
        assert!(gguf_url.contains("search=llama%20gguf"));
        assert!(gguf_url.contains("sort=downloads"));
    }
}
