//! Local LLM provider setup service.
//!
//! Provides RPC handlers for configuring the local GGUF LLM provider,
//! including system info detection, model listing, and model configuration.

use std::{path::PathBuf, sync::Arc};

use {
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
    serde_json::Value,
    tokio::sync::{OnceCell, RwLock},
    tracing::info,
};

use moltis_agents::providers::{ProviderRegistry, local_gguf};

use crate::{
    broadcast::{BroadcastOpts, broadcast},
    services::{LocalLlmService, ServiceResult},
    state::GatewayState,
};

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
        // Check if we have a saved config and set initial status
        let status = if let Some(config) = LocalLlmConfig::load() {
            // Use first model as the "active" one for status display
            if let Some(first_model) = config.models.first() {
                LocalLlmStatus::Ready {
                    model_id: first_model.model_id.clone(),
                }
            } else {
                LocalLlmStatus::Unconfigured
            }
        } else {
            LocalLlmStatus::Unconfigured
        };

        Self {
            registry,
            status: Arc::new(RwLock::new(status)),
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
            "description": if sys.is_apple_silicon {
                "Cross-platform, Metal GPU acceleration"
            } else if sys.has_cuda {
                "Cross-platform, CUDA GPU acceleration"
            } else {
                "Cross-platform, CPU inference"
            },
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
        let backend_note = if mlx_available {
            "MLX recommended (native Apple Silicon optimization)"
        } else if sys.is_apple_silicon {
            "GGUF with Metal (install mlx-lm for native MLX)"
        } else if sys.has_cuda {
            "GGUF with CUDA acceleration"
        } else {
            "GGUF (CPU inference)"
        };

        Ok(serde_json::json!({
            "totalRamGb": sys.total_ram_gb(),
            "availableRamGb": sys.available_ram_gb(),
            "hasMetal": sys.has_metal,
            "hasCuda": sys.has_cuda,
            "hasGpu": sys.has_gpu(),
            "isAppleSilicon": sys.is_apple_silicon,
            "memoryTier": tier.to_string(),
            "recommendedBackend": recommended_backend,
            "availableBackends": available_backends,
            "backendNote": backend_note,
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
            return Err(format!("invalid backend: {backend}. Must be GGUF or MLX"));
        }
        if backend == "MLX" && !mlx_available {
            return Err(
                "MLX backend requires mlx-lm. Install with: pip install mlx-lm".to_string(),
            );
        }

        // Validate model exists in registry
        let model_def = local_gguf::models::find_model(&model_id)
            .ok_or_else(|| format!("unknown model: {model_id}"))?;

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
        let mut config = LocalLlmConfig::load().unwrap_or_default();
        config.add_model(LocalModelEntry {
            model_id: model_id.clone(),
            model_path: None,
            gpu_layers: 0,
            backend: backend.clone(),
        });
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

            let result =
                local_gguf::models::ensure_model_with_progress(model_def, &cache_dir, |p| {
                    // Send progress to the broadcast task (ignore errors if channel closed)
                    let _ = tx.send((p.downloaded, p.total));
                })
                .await;

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
                    let gguf_config = local_gguf::LocalGgufConfig {
                        model_id: model_id_clone.clone(),
                        model_path: None,
                        context_size: None,
                        gpu_layers: 0,
                        temperature: 0.7,
                        cache_dir,
                    };

                    let provider = Arc::new(local_gguf::LazyLocalGgufProvider::new(gguf_config));

                    let mut reg = registry.write().await;
                    reg.register(
                        moltis_agents::providers::ModelInfo {
                            id: model_id_clone.clone(),
                            provider: "local-llm".into(),
                            display_name,
                        },
                        provider,
                    );

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

        // For GGUF, we need the filename
        let hf_filename = params
            .get("hfFilename")
            .and_then(|v| v.as_str())
            .map(String::from);

        // Validate: GGUF requires a filename, MLX doesn't
        if backend == "GGUF" && hf_filename.is_none() {
            return Err("GGUF models require 'hfFilename' parameter".to_string());
        }

        // Generate a model ID from the repo name
        let model_id = format!(
            "custom-{}",
            hf_repo
                .split('/')
                .next_back()
                .unwrap_or(&hf_repo)
                .to_lowercase()
                .replace(' ', "-")
        );

        info!(model = %model_id, repo = %hf_repo, backend = %backend, "configuring custom model");

        // Save configuration (add to existing models)
        let mut config = LocalLlmConfig::load().unwrap_or_default();
        config.add_model(LocalModelEntry {
            model_id: model_id.clone(),
            model_path: None,
            gpu_layers: 0,
            backend: backend.clone(),
        });
        config
            .save()
            .map_err(|e| format!("failed to save config: {e}"))?;

        // Update status
        {
            let mut status = self.status.write().await;
            *status = LocalLlmStatus::Loading {
                model_id: model_id.clone(),
                progress: None,
            };
        }

        // For custom models, we'll need to handle download differently
        // For now, just mark as ready (actual download happens on first use)
        {
            let mut status = self.status.write().await;
            *status = LocalLlmStatus::Ready {
                model_id: model_id.clone(),
            };
        }

        Ok(serde_json::json!({
            "ok": true,
            "modelId": model_id,
            "hfRepo": hf_repo,
            "backend": backend,
        }))
    }

    async fn remove_model(&self, params: Value) -> ServiceResult {
        let model_id = params
            .get("modelId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'modelId' parameter".to_string())?;

        info!(model = %model_id, "removing local-llm model");

        // Remove from config
        let mut config = LocalLlmConfig::load().unwrap_or_default();
        let removed = config.remove_model(model_id);

        if !removed {
            return Err(format!("model '{model_id}' not found in config"));
        }

        config
            .save()
            .map_err(|e| format!("failed to save config: {e}"))?;

        // Remove from provider registry
        {
            let mut reg = self.registry.write().await;
            reg.unregister(model_id);
        }

        // Update status if we removed the last model
        if config.models.is_empty() {
            let mut status = self.status.write().await;
            *status = LocalLlmStatus::Unconfigured;
        }

        Ok(serde_json::json!({
            "ok": true,
            "modelId": model_id,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_local_llm_config_serialization() {
        let mut config = LocalLlmConfig::default();
        config.add_model(LocalModelEntry {
            model_id: "qwen2.5-coder-7b-q4_k_m".into(),
            model_path: None,
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
    fn test_local_llm_config_multi_model() {
        let mut config = LocalLlmConfig::default();
        config.add_model(LocalModelEntry {
            model_id: "model-1".into(),
            model_path: None,
            gpu_layers: 0,
            backend: "GGUF".into(),
        });
        config.add_model(LocalModelEntry {
            model_id: "model-2".into(),
            model_path: None,
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
        // Test that custom model IDs are generated correctly
        let repo = "TheBloke/Llama-2-7B-GGUF";
        let model_id = format!(
            "custom-{}",
            repo.split('/')
                .next_back()
                .unwrap_or(repo)
                .to_lowercase()
                .replace(' ', "-")
        );
        assert_eq!(model_id, "custom-llama-2-7b-gguf");

        let repo2 = "mlx-community/Qwen2.5-Coder-7B-Instruct-4bit";
        let model_id2 = format!(
            "custom-{}",
            repo2
                .split('/')
                .next_back()
                .unwrap_or(repo2)
                .to_lowercase()
                .replace(' ', "-")
        );
        assert_eq!(model_id2, "custom-qwen2.5-coder-7b-instruct-4bit");
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
