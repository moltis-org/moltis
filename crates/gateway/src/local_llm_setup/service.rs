use super::{
    cache::{detect_mlx_installers, is_mlx_installed, spawn_download_progress_broadcaster},
    config::*,
    *,
};

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
    /// Model lifecycle manager for idle-timeout unloading and manual load/unload.
    lifecycle: Arc<ModelLifecycleManager>,
}

impl LiveLocalLlmService {
    pub fn new(registry: Arc<RwLock<ProviderRegistry>>) -> Self {
        Self {
            registry,
            status: Arc::new(RwLock::new(status_from_saved_config(
                LocalLlmConfig::load().as_ref(),
            ))),
            state: Arc::new(OnceCell::new()),
            lifecycle: Arc::new(ModelLifecycleManager::new()),
        }
    }

    /// Set the gateway state reference for broadcasting progress updates.
    pub fn set_state(&self, state: Arc<GatewayState>) {
        // Ignore if already set (shouldn't happen in normal operation)
        let _ = self.state.set(state.clone());
        self.lifecycle.set_state(state);
    }

    /// Get a reference to the lifecycle manager.
    pub fn lifecycle(&self) -> &Arc<ModelLifecycleManager> {
        &self.lifecycle
    }

    /// Register all saved local models with the lifecycle manager.
    ///
    /// Re-registers providers in the registry so both share the same
    /// `Arc<LocalLlmProvider>` — ensuring `last_activity` and `unload()`
    /// affect the real instance used for inference.
    pub async fn populate_lifecycle(&self, global_timeout: Option<u64>) {
        let Some(config) = LocalLlmConfig::load() else {
            return;
        };
        let mut reg = self.registry.write().await;
        for entry in &config.models {
            let effective_timeout = entry.idle_timeout_secs.or(global_timeout);
            match register_local_model_entry(&mut reg, entry) {
                Ok(provider) => {
                    self.lifecycle
                        .register(entry.model_id.clone(), provider, effective_timeout)
                        .await;
                },
                Err(error) => {
                    warn!(model_id = %entry.model_id, %error, "failed to create provider for lifecycle manager");
                },
            }
        }
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

pub(super) fn has_enough_ram(total_ram_gb: u32, required_ram_gb: u32) -> bool {
    total_ram_gb >= required_ram_gb
}

pub(super) fn insufficient_ram_error(
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

pub(super) fn gguf_backend_description(sys: &local_gguf::system_info::SystemInfo) -> String {
    match gguf_acceleration_name(sys) {
        Some(acceleration) => format!("Cross-platform, {acceleration} GPU acceleration"),
        None if sys.has_gpu() => "Cross-platform, GPU acceleration".to_string(),
        None => "Cross-platform, CPU inference".to_string(),
    }
}

pub(super) fn gguf_backend_note(
    sys: &local_gguf::system_info::SystemInfo,
    mlx_available: bool,
) -> String {
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
            idle_timeout_secs: None,
        };
        let mut config = LocalLlmConfig::load().unwrap_or_default();
        config.add_model(entry.clone());
        config
            .save()
            .map_err(|e| format!("failed to save config: {e}"))?;

        // Resolve effective idle timeout: per-model override → global default → None.
        let effective_timeout = entry.idle_timeout_secs.or_else(|| {
            let cfg = moltis_config::loader::discover_and_load();
            cfg.providers
                .get("local")
                .or_else(|| cfg.providers.get("local-llm"))
                .and_then(|e| e.idle_timeout_secs)
        });

        // Trigger model download in background with progress updates
        let model_id_clone = model_id.clone();
        let status = Arc::clone(&self.status);
        let registry = Arc::clone(&self.registry);
        let state_cell = Arc::clone(&self.state);
        let lifecycle = Arc::clone(&self.lifecycle);
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

                    // Register the provider in the registry and lifecycle manager
                    // (same Arc so last_activity and unload affect the real instance)
                    let mut reg = registry.write().await;
                    match register_local_model_entry(&mut reg, &entry) {
                        Ok(provider) => {
                            lifecycle
                                .register(entry.model_id.clone(), provider, effective_timeout)
                                .await;
                        },
                        Err(error) => {
                            tracing::error!(model = %model_id_clone, %error, "failed to register local model");
                        },
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
            idle_timeout_secs: None,
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

        // Resolve effective idle timeout: per-model override → global default → None.
        let effective_timeout = entry.idle_timeout_secs.or_else(|| {
            let cfg = moltis_config::loader::discover_and_load();
            cfg.providers
                .get("local")
                .or_else(|| cfg.providers.get("local-llm"))
                .and_then(|e| e.idle_timeout_secs)
        });

        let status = Arc::clone(&self.status);
        let registry = Arc::clone(&self.registry);
        let state_cell = Arc::clone(&self.state);
        let lifecycle = Arc::clone(&self.lifecycle);
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
                    match register_local_model_entry(&mut reg, &entry) {
                        Ok(provider) => {
                            lifecycle
                                .register(entry.model_id.clone(), provider, effective_timeout)
                                .await;
                        },
                        Err(error) => {
                            tracing::error!(model = %entry.model_id, %error, "failed to register custom local model");
                        },
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

        // Remove from provider registry and lifecycle manager
        {
            let mut reg = self.registry.write().await;
            unregister_local_model_from_registry(&mut reg, local_model_id);
        }
        self.lifecycle.unregister(local_model_id).await;

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

    async fn load_model(&self, params: Value) -> ServiceResult {
        let model_id = params
            .get("modelId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'modelId' parameter".to_string())?;

        self.lifecycle
            .load_model(model_id)
            .await
            .map_err(|e| e.to_string())?;

        Ok(serde_json::json!({ "ok": true, "modelId": model_id }))
    }

    async fn unload_model(&self, params: Value) -> ServiceResult {
        let model_id = params
            .get("modelId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'modelId' parameter".to_string())?;

        self.lifecycle
            .unload_model(model_id)
            .await
            .map_err(|e| e.to_string())?;

        Ok(serde_json::json!({ "ok": true, "modelId": model_id }))
    }

    async fn model_states(&self) -> ServiceResult {
        let states = self.lifecycle.model_states().await;
        Ok(serde_json::to_value(&states).unwrap_or_else(|_| serde_json::json!([])))
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
pub(super) struct HfModelInfo {
    /// Model ID (e.g., "TheBloke/Llama-2-7B-GGUF")
    /// The API returns both "id" and "modelId" fields with the same value.
    pub(super) id: String,
    /// Number of downloads
    #[serde(default)]
    pub(super) downloads: u64,
    /// Number of likes
    #[serde(default)]
    pub(super) likes: u64,
    /// Created timestamp
    #[serde(default, rename = "createdAt")]
    pub(super) created_at: Option<String>,
    /// Model tags
    #[serde(default)]
    pub(super) tags: Vec<String>,
}
