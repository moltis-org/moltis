use super::*;

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
pub(super) const LOCAL_LLM_PROVIDER_NAME: &str = "local-llm";

pub(super) fn download_progress_percent(downloaded: u64, total: Option<u64>) -> Option<f64> {
    total.map(|total_bytes| {
        if total_bytes > 0 {
            (downloaded as f64 / total_bytes as f64 * 100.0).min(100.0)
        } else {
            0.0
        }
    })
}

pub(super) fn spawn_download_progress_broadcaster(
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
pub(super) fn is_mlx_installed() -> bool {
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
pub(super) fn detect_mlx_installers() -> Vec<(&'static str, &'static str)> {
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
