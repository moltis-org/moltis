use {super::*, crate::local_llm_setup::cache::LOCAL_LLM_PROVIDER_NAME};

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

pub(super) fn default_backend() -> String {
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
pub(super) struct LegacyLocalLlmConfig {
    pub(super) model_id: String,
    pub(super) model_path: Option<PathBuf>,
    #[serde(default)]
    pub(super) gpu_layers: u32,
    #[serde(default = "default_backend")]
    pub(super) backend: String,
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

    pub(super) fn custom_gguf_source(&self) -> Option<(&str, &str)> {
        if self.backend != "GGUF" {
            return None;
        }

        Some((self.hf_repo.as_deref()?, self.hf_filename.as_deref()?))
    }

    pub(super) fn resolved_model_path(
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

    pub(super) fn display_name(&self) -> String {
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

pub(super) fn custom_gguf_model_id(hf_repo: &str, hf_filename: &str) -> String {
    let repo_component = URL_SAFE_NO_PAD.encode(hf_repo);
    let filename_component = URL_SAFE_NO_PAD.encode(hf_filename);
    format!("custom-gguf-{repo_component}.{filename_component}")
}

pub(super) fn legacy_custom_gguf_model_id(hf_repo: &str) -> String {
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

pub(super) fn remove_conflicting_custom_gguf_entries(
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

pub(super) fn status_from_saved_config(config: Option<&LocalLlmConfig>) -> LocalLlmStatus {
    config
        .and_then(|saved| saved.models.first())
        .map(|model| LocalLlmStatus::Ready {
            model_id: model.model_id.clone(),
        })
        .unwrap_or(LocalLlmStatus::Unconfigured)
}

pub(super) fn build_local_provider_entry(
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
        capabilities: moltis_providers::ModelCapabilities::infer(&entry.model_id),
    };
    Ok((info, provider))
}

pub(super) fn configured_local_model_path_override(
    providers_config: &moltis_config::schema::ProvidersConfig,
) -> Option<PathBuf> {
    providers_config
        .get("local")
        .and_then(|entry| entry.base_url.as_deref())
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
}

pub(super) fn unregister_local_model_from_registry(
    registry: &mut ProviderRegistry,
    model_id: &str,
) {
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

pub(super) fn unregister_local_model_ids_from_registry(
    registry: &mut ProviderRegistry,
    model_ids: &[String],
) {
    for model_id in model_ids {
        unregister_local_model_from_registry(registry, model_id);
    }
}

pub(super) fn register_local_model_entry(
    registry: &mut ProviderRegistry,
    entry: &LocalModelEntry,
) -> anyhow::Result<()> {
    let (info, provider) = build_local_provider_entry(entry, None)?;
    unregister_local_model_from_registry(registry, &entry.model_id);
    registry.register(info, provider);
    Ok(())
}

pub(super) fn register_local_model_entry_with_default_model_path(
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
