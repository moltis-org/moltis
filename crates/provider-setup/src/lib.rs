use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use secrecy::{ExposeSecret, Secret};

use {
    async_trait::async_trait,
    serde_json::{Map, Value},
    tokio::sync::{OnceCell, RwLock},
    tracing::{debug, info, warn},
};

pub mod error;

use {
    moltis_config::schema::ProvidersConfig,
    moltis_oauth::{
        CallbackServer, OAuthFlow, TokenStore, callback_port, device_flow, load_oauth_config,
    },
    moltis_providers::{ProviderRegistry, raw_model_id},
};

use moltis_service_traits::{ProviderSetupService, ServiceError, ServiceResult};

/// Callback for publishing events to connected clients.
///
/// The gateway wires this up to its WebSocket broadcast mechanism so the
/// provider-setup crate doesn't depend on the gateway's internal types.
#[async_trait]
pub trait SetupBroadcaster: Send + Sync {
    async fn broadcast(&self, topic: &str, payload: Value);
}

// ── Key store ──────────────────────────────────────────────────────────────

/// Per-provider stored configuration (API key, base URL, preferred models).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(
        default,
        alias = "model",
        deserialize_with = "deserialize_provider_models",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub models: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

fn deserialize_provider_models<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: Value = serde::Deserialize::deserialize(deserializer)?;
    let normalized = match value {
        Value::Null => Vec::new(),
        Value::String(model) => vec![model],
        Value::Array(values) => values
            .into_iter()
            .filter_map(|value| value.as_str().map(ToString::to_string))
            .collect(),
        _ => {
            return Err(serde::de::Error::custom(
                "models must be a string or string array",
            ));
        },
    };

    Ok(normalize_model_list(normalized))
}

fn normalize_model_list(models: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out = Vec::new();
    for model in models {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Persist raw IDs so provider-local preferences don't collide with
        // another provider's namespace (e.g. "openai::gpt-5.2").
        let normalized = raw_model_id(trimmed).trim().to_string();
        if normalized.is_empty() {
            continue;
        }
        if out.iter().any(|existing| existing == &normalized) {
            continue;
        }
        out.push(normalized);
    }
    out
}

fn parse_models_param(params: &Value) -> Vec<String> {
    let from_array = params
        .get("models")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();

    let mut models = normalize_model_list(from_array);
    if models.is_empty()
        && let Some(model) = params.get("model").and_then(Value::as_str)
    {
        models = normalize_model_list([model.to_string()]);
    }
    models
}

fn progress_payload(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

struct ProviderSetupTiming {
    operation: &'static str,
    provider: String,
    started: std::time::Instant,
}

impl ProviderSetupTiming {
    fn start(operation: &'static str, provider: Option<&str>) -> Self {
        let provider_name = provider.unwrap_or("<missing>").to_string();
        info!(
            operation,
            provider = %provider_name,
            "provider setup operation started"
        );
        Self {
            operation,
            provider: provider_name,
            started: std::time::Instant::now(),
        }
    }
}

impl Drop for ProviderSetupTiming {
    fn drop(&mut self) {
        info!(
            operation = self.operation,
            provider = %self.provider,
            elapsed_ms = self.started.elapsed().as_millis(),
            "provider setup operation finished"
        );
    }
}

/// File-based provider config storage at `~/.config/moltis/provider_keys.json`.
/// Stores per-provider configuration including API keys, base URLs, and models.
#[derive(Debug, Clone)]
pub struct KeyStore {
    inner: Arc<Mutex<KeyStoreInner>>,
}

#[derive(Debug)]
struct KeyStoreInner {
    path: PathBuf,
}

impl Default for KeyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyStore {
    pub fn new() -> Self {
        let path = moltis_config::config_dir()
            .unwrap_or_else(|| PathBuf::from(".config/moltis"))
            .join("provider_keys.json");
        Self {
            inner: Arc::new(Mutex::new(KeyStoreInner { path })),
        }
    }

    fn with_path(path: PathBuf) -> Self {
        Self {
            inner: Arc::new(Mutex::new(KeyStoreInner { path })),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, KeyStoreInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn path(&self) -> PathBuf {
        self.lock().path.clone()
    }

    /// Load all provider configs. Handles migration from old format (string values).
    fn load_all_configs_from_path(path: &PathBuf) -> HashMap<String, ProviderConfig> {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound {
                    warn!(
                        path = %path.display(),
                        error = %error,
                        "failed to read provider key store"
                    );
                }
                return HashMap::new();
            },
        };

        // Try parsing as new format first
        if let Ok(configs) = serde_json::from_str::<HashMap<String, ProviderConfig>>(&content) {
            return configs;
        }

        // Fall back to old format migration: { "provider": "api-key-string" }
        if let Ok(old_format) = serde_json::from_str::<HashMap<String, String>>(&content) {
            return old_format
                .into_iter()
                .map(|(k, v)| {
                    (k, ProviderConfig {
                        api_key: Some(v),
                        base_url: None,
                        models: Vec::new(),
                        display_name: None,
                    })
                })
                .collect();
        }

        warn!(
            path = %path.display(),
            "provider key store is invalid JSON and will be ignored"
        );
        HashMap::new()
    }

    pub fn load_all_configs(&self) -> HashMap<String, ProviderConfig> {
        let guard = self.lock();
        Self::load_all_configs_from_path(&guard.path)
    }

    /// Save all provider configs to disk.
    fn save_all_configs_to_path(
        path: &PathBuf,
        configs: &HashMap<String, ProviderConfig>,
    ) -> error::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                warn!(
                    path = %parent.display(),
                    error = %error,
                    "failed to create provider key store directory"
                );
                error::Error::external("failed to create provider key store directory", error)
            })?;
        }
        let data = serde_json::to_string_pretty(configs).map_err(|error| {
            warn!(error = %error, "failed to serialize provider key store");
            error
        })?;

        // Write atomically via temp file + rename so readers never observe
        // partially-written JSON.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let temp_path = path.with_extension(format!("json.tmp.{nanos}"));
        std::fs::write(&temp_path, &data).map_err(|error| {
            warn!(
                path = %temp_path.display(),
                error = %error,
                "failed to write provider key store temp file"
            );
            error::Error::external("failed to write provider key store temp file", error)
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o600));
        }

        std::fs::rename(&temp_path, path).map_err(|error| {
            warn!(
                temp_path = %temp_path.display(),
                path = %path.display(),
                error = %error,
                "failed to atomically replace provider key store"
            );
            error::Error::external("failed to atomically replace provider key store", error)
        })?;

        Ok(())
    }

    /// Load all API keys (used in tests).
    #[cfg_attr(not(test), allow(dead_code))]
    fn load_all(&self) -> HashMap<String, String> {
        self.load_all_configs()
            .into_iter()
            .filter_map(|(k, v)| v.api_key.map(|key| (k, key)))
            .collect()
    }

    /// Load a provider's API key.
    fn load(&self, provider: &str) -> Option<String> {
        self.load_all_configs()
            .get(provider)
            .and_then(|c| c.api_key.clone())
    }

    /// Load a provider's full config.
    pub fn load_config(&self, provider: &str) -> Option<ProviderConfig> {
        self.load_all_configs().get(provider).cloned()
    }

    /// Remove a provider's configuration.
    fn remove(&self, provider: &str) -> error::Result<()> {
        let guard = self.lock();
        let mut configs = Self::load_all_configs_from_path(&guard.path);
        configs.remove(provider);
        Self::save_all_configs_to_path(&guard.path, &configs)
    }

    /// Save a provider's API key (simple interface, used in tests).
    #[cfg_attr(not(test), allow(dead_code))]
    fn save(&self, provider: &str, api_key: &str) -> error::Result<()> {
        self.save_config(
            provider,
            Some(api_key.to_string()),
            None, // preserve existing base_url
            None, // preserve existing models
        )
    }

    /// Save a provider's full configuration.
    pub fn save_config(
        &self,
        provider: &str,
        api_key: Option<String>,
        base_url: Option<String>,
        models: Option<Vec<String>>,
    ) -> error::Result<()> {
        self.save_config_with_display_name(provider, api_key, base_url, models, None)
    }

    /// Save a provider's full configuration, including an optional display name.
    fn save_config_with_display_name(
        &self,
        provider: &str,
        api_key: Option<String>,
        base_url: Option<String>,
        models: Option<Vec<String>>,
        display_name: Option<String>,
    ) -> error::Result<()> {
        let guard = self.lock();
        let mut configs = Self::load_all_configs_from_path(&guard.path);
        let entry = configs.entry(provider.to_string()).or_default();

        // Only update fields that are provided (Some), preserve existing for None
        if let Some(key) = api_key {
            entry.api_key = Some(key);
        }
        if let Some(url) = base_url {
            entry.base_url = if url.is_empty() {
                None
            } else {
                Some(url)
            };
        }
        if let Some(models) = models {
            entry.models = normalize_model_list(models);
        }
        if let Some(name) = display_name {
            entry.display_name = Some(name);
        }

        Self::save_all_configs_to_path(&guard.path, &configs)
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Merge persisted provider configs into a ProvidersConfig so the registry rebuild
/// picks them up without needing env vars.
pub fn config_with_saved_keys(
    base: &ProvidersConfig,
    key_store: &KeyStore,
    local_model_ids: &[String],
) -> ProvidersConfig {
    let mut config = base.clone();
    if let Some((home_config, _)) = home_provider_config() {
        for (name, entry) in home_config.providers {
            let dst = config.providers.entry(name).or_default();
            if dst
                .api_key
                .as_ref()
                .is_none_or(|k| k.expose_secret().is_empty())
                && let Some(api_key) = entry.api_key
                && !api_key.expose_secret().is_empty()
            {
                dst.api_key = Some(api_key);
            }
            if dst.base_url.is_none()
                && let Some(base_url) = entry.base_url
                && !base_url.trim().is_empty()
            {
                dst.base_url = Some(base_url);
            }
            if dst.models.is_empty() && !entry.models.is_empty() {
                dst.models = normalize_model_list(entry.models);
            }
        }
    }

    // Merge home key store first, then current key store so current instance
    // values win when both have values.
    let mut saved_configs = HashMap::new();
    if let Some((home_store, _)) = home_key_store() {
        saved_configs.extend(home_store.load_all_configs());
    }
    for (name, saved) in key_store.load_all_configs() {
        let entry = saved_configs
            .entry(name)
            .or_insert_with(ProviderConfig::default);
        if saved.api_key.is_some() {
            entry.api_key = saved.api_key;
        }
        if saved.base_url.is_some() {
            entry.base_url = saved.base_url;
        }
        if !saved.models.is_empty() {
            entry.models = saved.models;
        }
    }

    for (name, saved) in saved_configs {
        let entry = config.providers.entry(name).or_default();

        // Only override API key if config doesn't already have one.
        if let Some(key) = saved.api_key
            && entry
                .api_key
                .as_ref()
                .is_none_or(|k| k.expose_secret().is_empty())
        {
            entry.api_key = Some(Secret::new(key));
        }

        // Only override base_url if config doesn't already have one.
        if let Some(url) = saved.base_url
            && entry.base_url.is_none()
        {
            entry.base_url = Some(url);
        }

        if !saved.models.is_empty() {
            // Merge: saved models (from "Choose model" UI) go first, then
            // config models. normalize_model_list deduplicates.
            let mut merged = saved.models;
            merged.append(&mut entry.models);
            entry.models = normalize_model_list(merged);
        }
    }

    // Merge local-llm model IDs (injected by the caller).
    if !local_model_ids.is_empty() {
        config.local_models = local_model_ids.to_vec();

        // Keep provider models in sync so model pickers can prioritize these.
        let entry = config.providers.entry("local".into()).or_default();
        if entry.models.is_empty() {
            entry.models = normalize_model_list(config.local_models.clone());
        }
    }

    config
}

// ── Custom provider helpers ────────────────────────────────────────────────

const CUSTOM_PROVIDER_PREFIX: &str = "custom-";

fn is_custom_provider(name: &str) -> bool {
    name.starts_with(CUSTOM_PROVIDER_PREFIX)
}

/// Derive a provider name from a URL, e.g. `https://api.together.ai/v1` → `custom-together-ai`.
fn derive_provider_name_from_url(raw: &str) -> Option<String> {
    let parsed = url::Url::parse(raw).ok()?;
    let host = parsed.host_str()?;
    let stripped = host.strip_prefix("api.").unwrap_or(host);
    let slug = stripped.replace('.', "-");
    Some(format!("{CUSTOM_PROVIDER_PREFIX}{slug}"))
}

/// Return a unique provider name by appending `-2`, `-3`, etc. if the base
/// name is already taken.
fn make_unique_provider_name(base: &str, existing: &HashMap<String, ProviderConfig>) -> String {
    if !existing.contains_key(base) {
        return base.to_string();
    }
    for i in 2.. {
        let candidate = format!("{base}-{i}");
        if !existing.contains_key(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

/// Extract a human-friendly display name from a URL.
/// `https://api.together.ai/v1` → `together.ai`
fn base_url_to_display_name(raw: &str) -> String {
    url::Url::parse(raw)
        .ok()
        .and_then(|u| u.host_str().map(ToOwned::to_owned))
        .map(|host| host.strip_prefix("api.").unwrap_or(&host).to_string())
        .unwrap_or_else(|| raw.to_string())
}

fn normalize_base_url_for_compare(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if let Ok(parsed) = url::Url::parse(trimmed) {
        let scheme = parsed.scheme().to_ascii_lowercase();
        let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
        let mut normalized = format!("{scheme}://{host}");
        if let Some(port) = parsed.port() {
            normalized.push(':');
            normalized.push_str(&port.to_string());
        }
        let path = parsed.path().trim_end_matches('/');
        normalized.push_str(path);
        return normalized;
    }

    trimmed.trim_end_matches('/').to_ascii_lowercase()
}

fn existing_custom_provider_for_base_url(
    base_url: &str,
    existing: &HashMap<String, ProviderConfig>,
) -> Option<String> {
    let target = normalize_base_url_for_compare(base_url);
    if target.is_empty() {
        return None;
    }

    existing
        .iter()
        .filter_map(|(name, cfg)| {
            if !is_custom_provider(name) {
                return None;
            }
            let existing_url = cfg.base_url.as_deref()?;
            (normalize_base_url_for_compare(existing_url) == target).then_some(name.clone())
        })
        .min_by(|a, b| a.len().cmp(&b.len()).then(a.cmp(b)))
}

fn validation_provider_name_for_endpoint(
    provider_name: &str,
    provider_default_base_url: Option<&str>,
    base_url: Option<&str>,
) -> String {
    if is_custom_provider(provider_name) {
        return provider_name.to_string();
    }

    if provider_name != "openai" {
        return provider_name.to_string();
    }

    let Some(endpoint) = base_url else {
        return provider_name.to_string();
    };

    let normalized_endpoint = normalize_base_url_for_compare(endpoint);
    if normalized_endpoint.is_empty() {
        return provider_name.to_string();
    }

    let normalized_default = normalize_base_url_for_compare(
        provider_default_base_url.unwrap_or("https://api.openai.com/v1"),
    );
    if normalized_default == normalized_endpoint {
        return provider_name.to_string();
    }

    derive_provider_name_from_url(endpoint).unwrap_or_else(|| provider_name.to_string())
}

const OLLAMA_DEFAULT_BASE_URL: &str = "http://localhost:11434";

fn normalize_ollama_openai_base_url(base_url: Option<&str>) -> String {
    let base = base_url
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(OLLAMA_DEFAULT_BASE_URL);
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1")
    }
}

fn normalize_ollama_api_base_url(base_url: Option<&str>) -> String {
    let openai_base = normalize_ollama_openai_base_url(base_url);
    openai_base
        .trim_end_matches('/')
        .strip_suffix("/v1")
        .unwrap_or(openai_base.as_str())
        .to_string()
}

fn normalize_ollama_model_id(model: &str) -> &str {
    model.strip_prefix("ollama::").unwrap_or(model)
}

fn ollama_model_matches(installed_model: &str, requested_model: &str) -> bool {
    installed_model == requested_model
        || installed_model.starts_with(&format!("{requested_model}:"))
}

#[derive(Debug, serde::Deserialize)]
struct OllamaTagsModel {
    name: String,
}

#[derive(Debug, serde::Deserialize)]
struct OllamaTagsResponse {
    #[serde(default)]
    models: Vec<OllamaTagsModel>,
}

async fn discover_ollama_models(base_url: &str) -> error::Result<Vec<String>> {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let response = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .map_err(|source| {
            error::Error::external("failed to query Ollama model discovery endpoint", source)
        })?;

    if !response.status().is_success() {
        return Err(error::Error::message(format!(
            "Ollama model discovery failed at {url} (HTTP {}).",
            response.status(),
        )));
    }

    let payload: OllamaTagsResponse = response.json().await.map_err(|source| {
        error::Error::external("invalid JSON from Ollama model discovery endpoint", source)
    })?;

    let mut models: Vec<String> = payload
        .models
        .into_iter()
        .map(|m| m.name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect();
    models.sort();
    models.dedup();
    Ok(models)
}

fn ollama_models_payload(models: &[String]) -> Vec<Value> {
    models
        .iter()
        .map(|model| {
            serde_json::json!({
                "id": format!("ollama::{model}"),
                "displayName": model,
                "provider": "ollama",
                "supportsTools": true,
            })
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthType {
    ApiKey,
    Oauth,
    Local,
}

impl AuthType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ApiKey => "api-key",
            Self::Oauth => "oauth",
            Self::Local => "local",
        }
    }
}

impl std::fmt::Display for AuthType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str((*self).as_str())
    }
}

/// Known provider definitions used to populate the "available providers" list.
pub struct KnownProvider {
    pub name: &'static str,
    pub display_name: &'static str,
    pub auth_type: AuthType,
    pub env_key: Option<&'static str>,
    /// Default base URL for this provider (for OpenAI-compatible providers).
    pub default_base_url: Option<&'static str>,
    /// Whether this provider requires a model to be specified.
    pub requires_model: bool,
    /// Whether the API key is optional (e.g. Ollama runs locally without auth).
    pub key_optional: bool,
}

/// Build the known providers list at runtime, including local-llm if enabled.
pub fn known_providers() -> Vec<KnownProvider> {
    let providers = vec![
        KnownProvider {
            name: "anthropic",
            display_name: "Anthropic",
            auth_type: AuthType::ApiKey,
            env_key: Some("ANTHROPIC_API_KEY"),
            default_base_url: Some("https://api.anthropic.com"),
            requires_model: false,
            key_optional: false,
        },
        KnownProvider {
            name: "openai",
            display_name: "OpenAI",
            auth_type: AuthType::ApiKey,
            env_key: Some("OPENAI_API_KEY"),
            default_base_url: Some("https://api.openai.com/v1"),
            requires_model: false,
            key_optional: false,
        },
        KnownProvider {
            name: "gemini",
            display_name: "Google Gemini",
            auth_type: AuthType::ApiKey,
            env_key: Some("GEMINI_API_KEY"),
            default_base_url: Some("https://generativelanguage.googleapis.com/v1beta"),
            requires_model: false,
            key_optional: false,
        },
        KnownProvider {
            name: "groq",
            display_name: "Groq",
            auth_type: AuthType::ApiKey,
            env_key: Some("GROQ_API_KEY"),
            default_base_url: Some("https://api.groq.com/openai/v1"),
            requires_model: false,
            key_optional: false,
        },
        KnownProvider {
            name: "xai",
            display_name: "xAI (Grok)",
            auth_type: AuthType::ApiKey,
            env_key: Some("XAI_API_KEY"),
            default_base_url: Some("https://api.x.ai/v1"),
            requires_model: false,
            key_optional: false,
        },
        KnownProvider {
            name: "deepseek",
            display_name: "DeepSeek",
            auth_type: AuthType::ApiKey,
            env_key: Some("DEEPSEEK_API_KEY"),
            default_base_url: Some("https://api.deepseek.com"),
            requires_model: false,
            key_optional: false,
        },
        KnownProvider {
            name: "mistral",
            display_name: "Mistral",
            auth_type: AuthType::ApiKey,
            env_key: Some("MISTRAL_API_KEY"),
            default_base_url: Some("https://api.mistral.ai/v1"),
            requires_model: false,
            key_optional: false,
        },
        KnownProvider {
            name: "openrouter",
            display_name: "OpenRouter",
            auth_type: AuthType::ApiKey,
            env_key: Some("OPENROUTER_API_KEY"),
            default_base_url: Some("https://openrouter.ai/api/v1"),
            requires_model: false,
            key_optional: false,
        },
        KnownProvider {
            name: "cerebras",
            display_name: "Cerebras",
            auth_type: AuthType::ApiKey,
            env_key: Some("CEREBRAS_API_KEY"),
            default_base_url: Some("https://api.cerebras.ai/v1"),
            requires_model: false,
            key_optional: false,
        },
        KnownProvider {
            name: "minimax",
            display_name: "MiniMax",
            auth_type: AuthType::ApiKey,
            env_key: Some("MINIMAX_API_KEY"),
            default_base_url: Some("https://api.minimax.io/v1"),
            requires_model: false,
            key_optional: false,
        },
        KnownProvider {
            name: "moonshot",
            display_name: "Moonshot",
            auth_type: AuthType::ApiKey,
            env_key: Some("MOONSHOT_API_KEY"),
            default_base_url: Some("https://api.moonshot.cn/v1"),
            requires_model: false,
            key_optional: false,
        },
        KnownProvider {
            name: "zai",
            display_name: "Z.AI",
            auth_type: AuthType::ApiKey,
            env_key: Some("Z_API_KEY"),
            default_base_url: Some("https://api.z.ai/api/paas/v4"),
            requires_model: false,
            key_optional: false,
        },
        KnownProvider {
            name: "venice",
            display_name: "Venice",
            auth_type: AuthType::ApiKey,
            env_key: Some("VENICE_API_KEY"),
            default_base_url: Some("https://api.venice.ai/api/v1"),
            requires_model: true,
            key_optional: false,
        },
        KnownProvider {
            name: "ollama",
            display_name: "Ollama",
            auth_type: AuthType::ApiKey,
            env_key: Some("OLLAMA_API_KEY"),
            default_base_url: Some("http://localhost:11434"),
            requires_model: false,
            key_optional: true,
        },
        KnownProvider {
            name: "openai-codex",
            display_name: "OpenAI Codex",
            auth_type: AuthType::Oauth,
            env_key: None,
            default_base_url: None,
            requires_model: false,
            key_optional: false,
        },
        KnownProvider {
            name: "github-copilot",
            display_name: "GitHub Copilot",
            auth_type: AuthType::Oauth,
            env_key: None,
            default_base_url: None,
            requires_model: false,
            key_optional: false,
        },
        KnownProvider {
            name: "kimi-code",
            display_name: "Kimi Code",
            auth_type: AuthType::ApiKey,
            env_key: Some("KIMI_API_KEY"),
            default_base_url: Some("https://api.kimi.com/coding/v1"),
            requires_model: false,
            key_optional: false,
        },
    ];

    // Add local-llm provider when the local-llm feature is enabled
    #[cfg(feature = "local-llm")]
    let providers = {
        let mut p = providers;
        p.push(KnownProvider {
            name: "local-llm",
            display_name: "Local LLM (Offline)",
            auth_type: AuthType::Local,
            env_key: None,
            default_base_url: None,
            requires_model: true,
            key_optional: false,
        });
        p
    };

    providers
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoDetectedProviderSource {
    pub provider: String,
    pub source: String,
}

fn current_config_dir() -> PathBuf {
    moltis_config::config_dir().unwrap_or_else(|| PathBuf::from(".config/moltis"))
}

fn home_config_dir_if_different() -> Option<PathBuf> {
    moltis_config::user_global_config_dir_if_different()
}

fn home_key_store() -> Option<(KeyStore, PathBuf)> {
    let dir = home_config_dir_if_different()?;
    let path = dir.join("provider_keys.json");
    Some((KeyStore::with_path(path.clone()), path))
}

fn home_token_store() -> Option<(TokenStore, PathBuf)> {
    let dir = home_config_dir_if_different()?;
    let path = dir.join("oauth_tokens.json");
    Some((TokenStore::with_path(path.clone()), path))
}

fn home_provider_config() -> Option<(ProvidersConfig, PathBuf)> {
    let path = moltis_config::find_user_global_config_file()?;
    let home_dir = home_config_dir_if_different()?;
    if !path.starts_with(&home_dir) {
        return None;
    }
    let loaded = moltis_config::loader::load_config(&path).ok()?;
    Some((loaded.providers, path))
}

fn codex_cli_auth_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".codex").join("auth.json"))
}

fn codex_cli_auth_has_access_token(path: &Path) -> bool {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(json) = serde_json::from_str::<Value>(&raw) else {
        return false;
    };
    json.get("tokens")
        .and_then(|t| t.get("access_token"))
        .and_then(|v| v.as_str())
        .is_some_and(|token| !token.trim().is_empty())
}

/// Parse Codex CLI `auth.json` content into `OAuthTokens`.
fn parse_codex_cli_tokens(data: &str) -> Option<moltis_oauth::OAuthTokens> {
    let json: Value = serde_json::from_str(data).ok()?;
    let tokens = json.get("tokens")?;
    let access_token = tokens.get("access_token")?.as_str()?.to_string();
    if access_token.trim().is_empty() {
        return None;
    }
    let id_token = tokens
        .get("id_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let account_id = tokens
        .get("account_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let refresh_token = tokens
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some(moltis_oauth::OAuthTokens {
        access_token: Secret::new(access_token),
        refresh_token: refresh_token.map(Secret::new),
        id_token: id_token.map(Secret::new),
        account_id,
        expires_at: None,
    })
}

/// Import auto-detected external OAuth tokens into the token store so all
/// providers read from a single location. Currently handles Codex CLI
/// `~/.codex/auth.json` → `openai-codex` in the token store.
pub fn import_detected_oauth_tokens(
    detected: &[AutoDetectedProviderSource],
    token_store: &TokenStore,
) {
    for source in detected {
        if source.provider == "openai-codex"
            && source.source.contains(".codex/auth.json")
            && token_store.load("openai-codex").is_none()
            && let Some(path) = codex_cli_auth_path()
            && let Ok(data) = std::fs::read_to_string(&path)
            && let Some(tokens) = parse_codex_cli_tokens(&data)
        {
            match token_store.save("openai-codex", &tokens) {
                Ok(()) => info!(
                    source = %path.display(),
                    "imported openai-codex tokens from Codex CLI auth"
                ),
                Err(e) => debug!(
                    error = %e,
                    "failed to import openai-codex tokens"
                ),
            }
        }
    }
}

fn set_provider_enabled_in_config(provider: &str, enabled: bool) -> ServiceResult<()> {
    moltis_config::update_config(|cfg| {
        let entry = cfg
            .providers
            .providers
            .entry(provider.to_string())
            .or_default();
        entry.enabled = enabled;
    })
    .map_err(ServiceError::message)?;
    Ok(())
}

fn normalize_provider_name(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn env_value_with_overrides(env_overrides: &HashMap<String, String>, key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            env_overrides
                .get(key)
                .cloned()
                .filter(|value| !value.trim().is_empty())
        })
}

fn ui_offered_provider_order(config: &ProvidersConfig) -> Vec<String> {
    let mut ordered = Vec::new();
    for name in &config.offered {
        let normalized = normalize_provider_name(name);
        if normalized.is_empty() || ordered.iter().any(|existing| existing == &normalized) {
            continue;
        }
        ordered.push(normalized);
    }
    ordered
}

fn ui_offered_provider_set(offered_order: &[String]) -> Option<BTreeSet<String>> {
    let offered: BTreeSet<String> = offered_order.iter().cloned().collect();
    (!offered.is_empty()).then_some(offered)
}

pub fn has_explicit_provider_settings(config: &ProvidersConfig) -> bool {
    config.providers.values().any(|entry| {
        entry
            .api_key
            .as_ref()
            .is_some_and(|k| !k.expose_secret().trim().is_empty())
            || entry.models.iter().any(|model| !model.trim().is_empty())
            || entry
                .base_url
                .as_deref()
                .is_some_and(|url| !url.trim().is_empty())
    })
}

pub fn detect_auto_provider_sources_with_overrides(
    config: &ProvidersConfig,
    deploy_platform: Option<&str>,
    env_overrides: &HashMap<String, String>,
) -> Vec<AutoDetectedProviderSource> {
    let is_cloud = deploy_platform.is_some();
    let key_store = KeyStore::new();
    let token_store = TokenStore::new();
    let home_key_store = home_key_store();
    let home_token_store = home_token_store();
    let home_provider_config = home_provider_config();
    let config_dir = current_config_dir();
    let provider_keys_path = config_dir.join("provider_keys.json");
    let oauth_tokens_path = config_dir.join("oauth_tokens.json");
    #[cfg(feature = "local-llm")]
    let local_llm_config_path = config_dir.join("local-llm.json");
    let codex_path = codex_cli_auth_path();

    let mut seen = BTreeSet::new();
    let mut detected = Vec::new();

    for provider in known_providers().into_iter().filter(|p| {
        if is_cloud {
            return p.auth_type != AuthType::Local && p.name != "ollama";
        }
        true
    }) {
        let mut sources = Vec::new();

        if let Some(env_key) = provider.env_key
            && env_value_with_overrides(env_overrides, env_key).is_some()
        {
            sources.push(format!("env:{env_key}"));
        }

        if config
            .get(provider.name)
            .and_then(|entry| entry.api_key.as_ref())
            .is_some_and(|k| !k.expose_secret().trim().is_empty())
        {
            sources.push(format!("config:[providers.{}].api_key", provider.name));
        }

        if home_provider_config
            .as_ref()
            .and_then(|(cfg, _)| cfg.get(provider.name))
            .and_then(|entry| entry.api_key.as_ref())
            .is_some_and(|k| !k.expose_secret().trim().is_empty())
            && let Some((_, path)) = home_provider_config.as_ref()
        {
            sources.push(format!(
                "file:{}:[providers.{}].api_key",
                path.display(),
                provider.name
            ));
        }

        if key_store.load(provider.name).is_some() {
            sources.push(format!("file:{}", provider_keys_path.display()));
        }
        if home_key_store
            .as_ref()
            .is_some_and(|(store, _)| store.load(provider.name).is_some())
            && let Some((_, path)) = home_key_store.as_ref()
        {
            sources.push(format!("file:{}", path.display()));
        }

        if (provider.auth_type == AuthType::Oauth || provider.name == "kimi-code")
            && token_store.load(provider.name).is_some()
        {
            sources.push(format!("file:{}", oauth_tokens_path.display()));
        }
        if (provider.auth_type == AuthType::Oauth || provider.name == "kimi-code")
            && home_token_store
                .as_ref()
                .is_some_and(|(store, _)| store.load(provider.name).is_some())
            && let Some((_, path)) = home_token_store.as_ref()
        {
            sources.push(format!("file:{}", path.display()));
        }

        if provider.name == "openai-codex"
            && codex_path
                .as_deref()
                .is_some_and(codex_cli_auth_has_access_token)
            && let Some(path) = codex_path.as_ref()
        {
            sources.push(format!("file:{}", path.display()));
        }

        #[cfg(feature = "local-llm")]
        if provider.name == "local-llm" && local_llm_config_path.exists() {
            sources.push(format!("file:{}", local_llm_config_path.display()));
        }

        for source in sources {
            if seen.insert((provider.name.to_string(), source.clone())) {
                detected.push(AutoDetectedProviderSource {
                    provider: provider.name.to_string(),
                    source,
                });
            }
        }
    }

    detected
}

/// Function that parses a raw error string into a structured error object.
pub type ErrorParser = fn(&str, Option<&str>) -> Value;

/// Default error parser that wraps the raw error text in a JSON object.
fn default_error_parser(raw: &str, _provider: Option<&str>) -> Value {
    serde_json::json!({ "type": "unknown", "detail": raw })
}

pub struct LiveProviderSetupService {
    registry: Arc<RwLock<ProviderRegistry>>,
    config: Arc<Mutex<ProvidersConfig>>,
    broadcaster: Arc<OnceCell<Arc<dyn SetupBroadcaster>>>,
    token_store: TokenStore,
    key_store: KeyStore,
    pending_oauth: Arc<RwLock<HashMap<String, PendingOAuthFlow>>>,
    /// When set, local-only providers (local-llm, ollama) are hidden from
    /// the available list because they cannot run on cloud VMs.
    deploy_platform: Option<String>,
    /// Shared priority models list from `LiveModelService`. Updated by
    /// `save_model` so the dropdown ordering reflects the latest preference.
    priority_models: Option<Arc<RwLock<Vec<String>>>>,
    /// Monotonic sequence used to drop stale async registry refreshes.
    registry_rebuild_seq: Arc<AtomicU64>,
    /// Static env overrides (for example config `[env]`) used when resolving
    /// provider credentials without mutating the process environment.
    env_overrides: HashMap<String, String>,
    /// Injected error parser for interpreting provider API errors.
    error_parser: ErrorParser,
    /// Address the OAuth callback server binds to. Defaults to `127.0.0.1`
    /// for local development; set to `0.0.0.0` in Docker / remote
    /// deployments so the callback port is reachable from the host.
    callback_bind_addr: String,
}

#[derive(Clone)]
struct PendingOAuthFlow {
    provider_name: String,
    oauth_config: moltis_oauth::OAuthConfig,
    verifier: String,
}

impl LiveProviderSetupService {
    pub fn new(
        registry: Arc<RwLock<ProviderRegistry>>,
        config: ProvidersConfig,
        deploy_platform: Option<String>,
    ) -> Self {
        Self {
            registry,
            config: Arc::new(Mutex::new(config)),
            broadcaster: Arc::new(OnceCell::new()),
            token_store: TokenStore::new(),
            key_store: KeyStore::new(),
            pending_oauth: Arc::new(RwLock::new(HashMap::new())),
            deploy_platform,
            priority_models: None,
            registry_rebuild_seq: Arc::new(AtomicU64::new(0)),
            env_overrides: HashMap::new(),
            error_parser: default_error_parser,
            callback_bind_addr: "127.0.0.1".to_string(),
        }
    }

    pub fn with_env_overrides(mut self, env_overrides: HashMap<String, String>) -> Self {
        self.env_overrides = env_overrides;
        self
    }

    /// Set a custom error parser for interpreting provider API errors.
    pub fn with_error_parser(mut self, parser: ErrorParser) -> Self {
        self.error_parser = parser;
        self
    }

    /// Set the bind address for the OAuth callback server.
    ///
    /// Defaults to `127.0.0.1`. Pass `0.0.0.0` when the gateway is
    /// bound to all interfaces (e.g. Docker) so the OAuth callback port
    /// is reachable from the host.
    pub fn with_callback_bind_addr(mut self, addr: String) -> Self {
        self.callback_bind_addr = addr;
        self
    }

    /// Wire the shared priority models handle from `LiveModelService` so
    /// `save_model` can update dropdown ordering at runtime.
    pub fn set_priority_models(&mut self, handle: Arc<RwLock<Vec<String>>>) {
        self.priority_models = Some(handle);
    }

    /// Set the broadcaster so validation can publish live progress events
    /// to the UI over WebSocket.
    pub fn set_broadcaster(&self, broadcaster: Arc<dyn SetupBroadcaster>) {
        let _ = self.broadcaster.set(broadcaster);
    }

    async fn emit_validation_progress(
        &self,
        provider: &str,
        request_id: Option<&str>,
        phase: &str,
        mut extra: Map<String, Value>,
    ) {
        let Some(broadcaster) = self.broadcaster.get() else {
            return;
        };

        let mut payload = Map::new();
        payload.insert("provider".to_string(), Value::String(provider.to_string()));
        payload.insert("phase".to_string(), Value::String(phase.to_string()));
        if let Some(id) = request_id {
            payload.insert("requestId".to_string(), Value::String(id.to_string()));
        }
        payload.append(&mut extra);

        broadcaster
            .broadcast("providers.validate.progress", Value::Object(payload))
            .await;
    }

    fn queue_registry_rebuild(&self, provider_name: &str, reason: &'static str) {
        let rebuild_seq = self.registry_rebuild_seq.fetch_add(1, Ordering::SeqCst) + 1;
        let latest_seq = Arc::clone(&self.registry_rebuild_seq);
        let registry = Arc::clone(&self.registry);
        let config = Arc::clone(&self.config);
        let key_store = self.key_store.clone();
        let env_overrides = self.env_overrides.clone();
        let provider_name = provider_name.to_string();

        tokio::spawn(async move {
            let started = std::time::Instant::now();
            info!(
                provider = %provider_name,
                reason,
                rebuild_seq,
                "provider registry async rebuild started"
            );

            let effective = {
                let base = config.lock().unwrap_or_else(|e| e.into_inner()).clone();
                config_with_saved_keys(&base, &key_store, &[])
            };

            let new_registry = match tokio::task::spawn_blocking(move || {
                ProviderRegistry::from_env_with_config_and_overrides(&effective, &env_overrides)
            })
            .await
            {
                Ok(registry) => registry,
                Err(error) => {
                    warn!(
                        provider = %provider_name,
                        reason,
                        rebuild_seq,
                        error = %error,
                        "provider registry async rebuild worker failed"
                    );
                    return;
                },
            };

            let current_seq = latest_seq.load(Ordering::Acquire);
            if rebuild_seq != current_seq {
                info!(
                    provider = %provider_name,
                    reason,
                    rebuild_seq,
                    latest_seq = current_seq,
                    elapsed_ms = started.elapsed().as_millis(),
                    "provider registry async rebuild skipped as stale"
                );
                return;
            }

            let provider_summary = new_registry.provider_summary();
            let model_count = new_registry.list_models().len();
            let mut reg = registry.write().await;
            *reg = new_registry;
            info!(
                provider = %provider_name,
                reason,
                rebuild_seq,
                provider_summary = %provider_summary,
                models = model_count,
                elapsed_ms = started.elapsed().as_millis(),
                "provider registry async rebuild finished"
            );
        });
    }

    fn config_snapshot(&self) -> ProvidersConfig {
        self.config
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn set_provider_enabled_in_memory(&self, provider: &str, enabled: bool) {
        let mut cfg = self.config.lock().unwrap_or_else(|e| e.into_inner());
        cfg.providers
            .entry(provider.to_string())
            .or_default()
            .enabled = enabled;
    }

    fn is_provider_configured(
        &self,
        provider: &KnownProvider,
        active_config: &ProvidersConfig,
    ) -> bool {
        // Disabled providers (by offered allowlist or explicit enabled=false)
        // should not show as configured, except subscription-backed OAuth
        // providers with valid local tokens.
        if !active_config.is_enabled(provider.name) {
            let subscription_with_tokens =
                matches!(provider.name, "openai-codex" | "github-copilot")
                    && active_config
                        .get(provider.name)
                        .is_none_or(|entry| entry.enabled)
                    && self.has_oauth_tokens(provider.name);
            if !subscription_with_tokens {
                return false;
            }
        }

        // Check if the provider has an API key set via env
        if let Some(env_key) = provider.env_key
            && env_value_with_overrides(&self.env_overrides, env_key).is_some()
        {
            return true;
        }
        // Check config file
        if let Some(entry) = active_config.get(provider.name)
            && entry
                .api_key
                .as_ref()
                .is_some_and(|k| !k.expose_secret().is_empty())
        {
            return true;
        }
        // Check home/global config file as fallback when using custom config dir.
        if home_provider_config()
            .as_ref()
            .and_then(|(cfg, _)| cfg.get(provider.name))
            .and_then(|entry| entry.api_key.as_ref())
            .is_some_and(|k| !k.expose_secret().is_empty())
        {
            return true;
        }
        // Check persisted key store
        if self.key_store.load(provider.name).is_some() {
            return true;
        }
        // Check persisted key store in user-global config dir.
        if home_key_store()
            .as_ref()
            .is_some_and(|(store, _)| store.load(provider.name).is_some())
        {
            return true;
        }
        // For OAuth providers, check token store
        if provider.auth_type == AuthType::Oauth || provider.name == "kimi-code" {
            if self.token_store.load(provider.name).is_some() {
                return true;
            }
            if home_token_store()
                .as_ref()
                .is_some_and(|(store, _)| store.load(provider.name).is_some())
            {
                return true;
            }
            // Match provider-registry behavior: openai-codex may be inferred from
            // Codex CLI auth at ~/.codex/auth.json.
            if provider.name == "openai-codex"
                && codex_cli_auth_path()
                    .as_deref()
                    .is_some_and(codex_cli_auth_has_access_token)
            {
                return true;
            }
            return false;
        }
        // For local providers, check if model is configured in local_llm config
        #[cfg(feature = "local-llm")]
        if provider.auth_type == AuthType::Local && provider.name == "local-llm" {
            // Check if local-llm model config file exists
            if let Some(config_dir) = moltis_config::config_dir() {
                let config_path = config_dir.join("local-llm.json");
                return config_path.exists();
            }
        }
        false
    }

    /// Start a device-flow OAuth for providers like GitHub Copilot.
    /// Returns `{ "userCode": "...", "verificationUri": "..." }` for the UI to display.
    async fn oauth_start_device_flow(
        &self,
        provider_name: String,
        oauth_config: moltis_oauth::OAuthConfig,
    ) -> ServiceResult {
        let client = reqwest::Client::new();
        let extra_headers = build_provider_headers(&provider_name);
        let device_resp = device_flow::request_device_code_with_headers(
            &client,
            &oauth_config,
            extra_headers.as_ref(),
        )
        .await
        .map_err(ServiceError::message)?;

        let user_code = device_resp.user_code.clone();
        let verification_uri = device_resp.verification_uri.clone();
        let verification_uri_complete = build_verification_uri_complete(
            &provider_name,
            &verification_uri,
            &user_code,
            device_resp.verification_uri_complete.clone(),
        );
        let device_code = device_resp.device_code.clone();
        let interval = device_resp.interval;

        // Spawn background task to poll for the token
        let token_store = self.token_store.clone();
        let registry = Arc::clone(&self.registry);
        let config = self.effective_config();
        let env_overrides = self.env_overrides.clone();
        let poll_headers = extra_headers.clone();
        tokio::spawn(async move {
            let poll_extra = poll_headers.as_ref();
            match device_flow::poll_for_token_with_headers(
                &client,
                &oauth_config,
                &device_code,
                interval,
                poll_extra,
            )
            .await
            {
                Ok(tokens) => {
                    if let Err(e) = token_store.save(&provider_name, &tokens) {
                        tracing::error!(
                            provider = %provider_name,
                            error = %e,
                            "failed to save device-flow OAuth tokens"
                        );
                        return;
                    }
                    let new_registry = ProviderRegistry::from_env_with_config_and_overrides(
                        &config,
                        &env_overrides,
                    );
                    let provider_summary = new_registry.provider_summary();
                    let model_count = new_registry.list_models().len();
                    let mut reg = registry.write().await;
                    *reg = new_registry;
                    info!(
                        provider = %provider_name,
                        provider_summary = %provider_summary,
                        models = model_count,
                        "device-flow OAuth complete, rebuilt provider registry"
                    );
                },
                Err(e) => {
                    tracing::error!(
                        provider = %provider_name,
                        error = %e,
                        "device-flow OAuth polling failed"
                    );
                },
            }
        });

        Ok(serde_json::json!({
            "deviceFlow": true,
            "userCode": user_code,
            "verificationUri": verification_uri,
            "verificationUriComplete": verification_uri_complete,
        }))
    }

    /// Build a ProvidersConfig that includes saved keys for registry rebuild.
    fn effective_config(&self) -> ProvidersConfig {
        let base = self.config_snapshot();
        config_with_saved_keys(&base, &self.key_store, &[])
    }

    fn build_registry(&self, config: &ProvidersConfig) -> ProviderRegistry {
        ProviderRegistry::from_env_with_config_and_overrides(config, &self.env_overrides)
    }

    fn has_oauth_tokens(&self, provider_name: &str) -> bool {
        has_oauth_tokens_for_provider(
            provider_name,
            &self.token_store,
            home_token_store().as_ref().map(|(store, _)| store),
        )
    }
}

fn has_oauth_tokens_for_provider(
    provider_name: &str,
    primary_store: &TokenStore,
    home_store: Option<&TokenStore>,
) -> bool {
    primary_store.load(provider_name).is_some()
        || home_store.is_some_and(|store| store.load(provider_name).is_some())
        || (provider_name == "openai-codex"
            && codex_cli_auth_path()
                .as_deref()
                .is_some_and(codex_cli_auth_has_access_token))
}

/// Build provider-specific extra headers for device-flow OAuth calls.
fn build_provider_headers(provider: &str) -> Option<reqwest::header::HeaderMap> {
    match provider {
        "kimi-code" => Some(moltis_oauth::kimi_headers()),
        _ => None,
    }
}

/// Some providers require visiting a URL that already embeds the user_code.
/// Prefer provider-returned `verification_uri_complete`; otherwise synthesize
/// one for known providers.
fn build_verification_uri_complete(
    provider: &str,
    verification_uri: &str,
    user_code: &str,
    provided_complete: Option<String>,
) -> Option<String> {
    if let Some(complete) = provided_complete
        && !complete.trim().is_empty()
    {
        return Some(complete);
    }

    if provider == "kimi-code" {
        let sep = if verification_uri.contains('?') {
            "&"
        } else {
            "?"
        };
        return Some(format!("{verification_uri}{sep}user_code={user_code}"));
    }

    None
}

#[async_trait]
impl ProviderSetupService for LiveProviderSetupService {
    async fn available(&self) -> ServiceResult {
        let is_cloud = self.deploy_platform.is_some();
        let active_config = self.config_snapshot();
        let offered_order = ui_offered_provider_order(&active_config);
        let offered = ui_offered_provider_set(&offered_order);
        let offered_rank: HashMap<String, usize> = offered_order
            .iter()
            .enumerate()
            .map(|(idx, provider)| (provider.clone(), idx))
            .collect();

        let mut providers: Vec<(Option<usize>, usize, Value)> = known_providers()
            .iter()
            .enumerate()
            .filter_map(|(known_idx, provider)| {
                // Hide local-only providers on cloud deployments.
                if is_cloud && (provider.auth_type == AuthType::Local || provider.name == "ollama")
                {
                    return None;
                }

                let configured = self.is_provider_configured(provider, &active_config);
                let normalized_name = normalize_provider_name(provider.name);
                if let Some(allowed) = offered.as_ref()
                    && !allowed.contains(&normalized_name)
                    && !configured
                {
                    return None;
                }

                // Get saved config for this provider (baseUrl, preferred models)
                let saved_config = self.key_store.load_config(provider.name);
                let base_url = saved_config.as_ref().and_then(|c| c.base_url.clone());
                let models = saved_config
                    .map(|c| normalize_model_list(c.models))
                    .unwrap_or_default();
                let model = models.first().cloned();

                Some((
                    offered_rank.get(&normalized_name).copied(),
                    known_idx,
                    serde_json::json!({
                        "name": provider.name,
                        "displayName": provider.display_name,
                        "authType": provider.auth_type.as_str(),
                        "configured": configured,
                        "defaultBaseUrl": provider.default_base_url,
                        "baseUrl": base_url,
                        "models": models,
                        "model": model,
                        "requiresModel": provider.requires_model,
                        "keyOptional": provider.key_optional,
                    }),
                ))
            })
            .collect();

        // Append custom providers from the key store.
        let known_count = providers.len();
        for (name, config) in self.key_store.load_all_configs() {
            if !is_custom_provider(&name) {
                continue;
            }
            if active_config.get(&name).is_some_and(|entry| !entry.enabled) {
                continue;
            }
            let display_name = config.display_name.clone().unwrap_or_else(|| name.clone());
            let base_url = config.base_url.clone();
            let models = normalize_model_list(config.models.clone());
            let model = models.first().cloned();

            providers.push((
                None,
                known_count, // sort after all known providers
                serde_json::json!({
                    "name": name,
                    "displayName": display_name,
                    "authType": "api-key",
                    "configured": true,
                    "defaultBaseUrl": base_url,
                    "baseUrl": base_url,
                    "models": models,
                    "model": model,
                    "requiresModel": true,
                    "keyOptional": false,
                    "isCustom": true,
                }),
            ));
        }

        providers.sort_by(
            |(a_offered, a_known, a_value), (b_offered, b_known, b_value)| {
                let offered_cmp = match (a_offered, b_offered) {
                    (Some(a), Some(b)) => a.cmp(b),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                };
                if offered_cmp != std::cmp::Ordering::Equal {
                    return offered_cmp;
                }

                let known_cmp = a_known.cmp(b_known);
                if known_cmp != std::cmp::Ordering::Equal {
                    return known_cmp;
                }

                let a_name = a_value
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let b_name = b_value
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                a_name.cmp(b_name)
            },
        );

        let providers: Vec<Value> = providers
            .into_iter()
            .enumerate()
            .map(|(idx, (_, _, mut value))| {
                if let Some(obj) = value.as_object_mut() {
                    obj.insert("uiOrder".into(), serde_json::json!(idx));
                }
                value
            })
            .collect();

        Ok(Value::Array(providers))
    }

    async fn save_key(&self, params: Value) -> ServiceResult {
        let _timing = ProviderSetupTiming::start(
            "providers.save_key",
            params.get("provider").and_then(Value::as_str),
        );
        let provider_name = params
            .get("provider")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'provider' parameter".to_string())?;

        // API key is optional for some providers (e.g., Ollama)
        let api_key = params.get("apiKey").and_then(|v| v.as_str());
        let base_url = params.get("baseUrl").and_then(|v| v.as_str());
        let models = parse_models_param(&params);

        // Custom providers bypass known_providers() validation.
        let is_custom = is_custom_provider(provider_name);
        if !is_custom {
            // Validate provider name - allow both api-key and local providers
            let known = known_providers();
            let provider = known
                .iter()
                .find(|p| {
                    p.name == provider_name
                        && (p.auth_type == AuthType::ApiKey || p.auth_type == AuthType::Local)
                })
                .ok_or_else(|| format!("unknown provider: {provider_name}"))?;

            // API key is required for api-key providers (except Ollama which is optional)
            if provider.auth_type == AuthType::ApiKey
                && provider_name != "ollama"
                && api_key.is_none()
            {
                return Err("missing 'apiKey' parameter".into());
            }
        } else if api_key.is_none() {
            return Err("missing 'apiKey' parameter".into());
        }

        let normalized_base_url = if provider_name == "ollama" {
            base_url.map(|url| normalize_ollama_openai_base_url(Some(url)))
        } else {
            base_url.map(String::from)
        };

        let key_store_path = self.key_store.path();
        info!(
            provider = provider_name,
            has_api_key = api_key.is_some(),
            has_base_url = normalized_base_url
                .as_ref()
                .is_some_and(|url| !url.trim().is_empty()),
            models = models.len(),
            key_store_path = %key_store_path.display(),
            "saving provider config"
        );

        // Persist full config to disk
        if let Err(error) = self.key_store.save_config(
            provider_name,
            api_key.map(String::from),
            normalized_base_url,
            (!models.is_empty()).then_some(models),
        ) {
            warn!(
                provider = provider_name,
                key_store_path = %key_store_path.display(),
                error = %error,
                "failed to persist provider config"
            );
            return Err(ServiceError::message(error));
        }
        set_provider_enabled_in_config(provider_name, true)?;
        self.set_provider_enabled_in_memory(provider_name, true);

        // Rebuild the provider registry with saved keys merged into config.
        let effective = self.effective_config();
        let new_registry = self.build_registry(&effective);
        let provider_summary = new_registry.provider_summary();
        let model_count = new_registry.list_models().len();
        let mut reg = self.registry.write().await;
        *reg = new_registry;

        info!(
            provider = provider_name,
            provider_summary = %provider_summary,
            models = model_count,
            "saved provider config to disk and rebuilt provider registry"
        );

        Ok(serde_json::json!({ "ok": true }))
    }

    async fn oauth_start(&self, params: Value) -> ServiceResult {
        let provider_name = params
            .get("provider")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'provider' parameter".to_string())?
            .to_string();

        let redirect_uri = params
            .get("redirectUri")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToOwned::to_owned);

        let mut oauth_config = load_oauth_config(&provider_name)
            .ok_or_else(|| format!("no OAuth config for provider: {provider_name}"))?;

        // User explicitly initiated OAuth for this provider; ensure it is enabled.
        set_provider_enabled_in_config(&provider_name, true)?;
        self.set_provider_enabled_in_memory(&provider_name, true);

        // If tokens already exist (for example imported from the main/home config),
        // skip launching a fresh OAuth flow and rebuild the registry immediately.
        if self.has_oauth_tokens(&provider_name) {
            let effective = self.effective_config();
            let new_registry = self.build_registry(&effective);
            let provider_summary = new_registry.provider_summary();
            let model_count = new_registry.list_models().len();
            let mut reg = self.registry.write().await;
            *reg = new_registry;
            info!(
                provider = %provider_name,
                provider_summary = %provider_summary,
                models = model_count,
                "oauth start skipped because provider already has tokens; rebuilt provider registry"
            );
            return Ok(serde_json::json!({
                "alreadyAuthenticated": true,
            }));
        }

        if oauth_config.device_flow {
            return self
                .oauth_start_device_flow(provider_name, oauth_config)
                .await;
        }

        // Providers with a pre-registered redirect_uri (e.g. openai-codex
        // registered as http://localhost:1455/auth/callback with OpenAI)
        // must always use that URI in the authorization request.
        // Overriding it with the gateway URL causes OAuth providers to
        // reject the request with "unknown_error".
        // For these providers we always use the local callback server.
        let has_registered_redirect = !oauth_config.redirect_uri.is_empty();
        let use_server_callback = redirect_uri.is_some() && !has_registered_redirect;
        if !has_registered_redirect && let Some(uri) = redirect_uri {
            oauth_config.redirect_uri = uri;
        }

        let port = callback_port(&oauth_config);
        let oauth_config_for_pending = oauth_config.clone();
        let flow = OAuthFlow::new(oauth_config);
        let auth_req = flow.start().map_err(ServiceError::message)?;

        let auth_url = auth_req.url.clone();
        let verifier = auth_req.pkce.verifier.clone();
        let expected_state = auth_req.state.clone();

        // Browser/server callback mode: callback lands on this gateway instance,
        // then `/auth/callback` completes the exchange with `oauth_complete`.
        if use_server_callback {
            let pending = PendingOAuthFlow {
                provider_name,
                oauth_config: oauth_config_for_pending,
                verifier,
            };
            self.pending_oauth
                .write()
                .await
                .insert(expected_state, pending);
            return Ok(serde_json::json!({
                "authUrl": auth_url,
            }));
        }

        // Spawn background task to wait for the callback and exchange the code
        let token_store = self.token_store.clone();
        let registry = Arc::clone(&self.registry);
        let config = self.effective_config();
        let env_overrides = self.env_overrides.clone();
        let bind_addr = self.callback_bind_addr.clone();
        tokio::spawn(async move {
            match CallbackServer::wait_for_code(port, expected_state, &bind_addr).await {
                Ok(code) => {
                    match flow.exchange(&code, &verifier).await {
                        Ok(tokens) => {
                            if let Err(e) = token_store.save(&provider_name, &tokens) {
                                tracing::error!(
                                    provider = %provider_name,
                                    error = %e,
                                    "failed to save OAuth tokens"
                                );
                                return;
                            }
                            // Rebuild registry with new tokens
                            let new_registry = ProviderRegistry::from_env_with_config_and_overrides(
                                &config,
                                &env_overrides,
                            );
                            let provider_summary = new_registry.provider_summary();
                            let model_count = new_registry.list_models().len();
                            let mut reg = registry.write().await;
                            *reg = new_registry;
                            info!(
                                provider = %provider_name,
                                provider_summary = %provider_summary,
                                models = model_count,
                                "OAuth flow complete, rebuilt provider registry"
                            );
                        },
                        Err(e) => {
                            tracing::error!(
                                provider = %provider_name,
                                error = %e,
                                "OAuth token exchange failed"
                            );
                        },
                    }
                },
                Err(e) => {
                    tracing::error!(
                        provider = %provider_name,
                        error = %e,
                        "OAuth callback failed"
                    );
                },
            }
        });

        Ok(serde_json::json!({
            "authUrl": auth_url,
        }))
    }

    async fn oauth_complete(&self, params: Value) -> ServiceResult {
        let code = params
            .get("code")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'code' parameter".to_string())?
            .to_string();
        let state = params
            .get("state")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'state' parameter".to_string())?
            .to_string();

        let pending = self
            .pending_oauth
            .write()
            .await
            .remove(&state)
            .ok_or_else(|| "unknown or expired OAuth state".to_string())?;

        let flow = OAuthFlow::new(pending.oauth_config);
        let tokens = flow
            .exchange(&code, &pending.verifier)
            .await
            .map_err(ServiceError::message)?;

        self.token_store
            .save(&pending.provider_name, &tokens)
            .map_err(ServiceError::message)?;
        set_provider_enabled_in_config(&pending.provider_name, true)?;
        self.set_provider_enabled_in_memory(&pending.provider_name, true);

        let effective = self.effective_config();
        let new_registry = self.build_registry(&effective);
        let provider_summary = new_registry.provider_summary();
        let model_count = new_registry.list_models().len();
        let mut reg = self.registry.write().await;
        *reg = new_registry;

        info!(
            provider = %pending.provider_name,
            provider_summary = %provider_summary,
            models = model_count,
            "OAuth callback complete, rebuilt provider registry"
        );

        Ok(serde_json::json!({
            "ok": true,
            "provider": pending.provider_name,
        }))
    }

    async fn remove_key(&self, params: Value) -> ServiceResult {
        let provider_name = params
            .get("provider")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'provider' parameter".to_string())?;

        if is_custom_provider(provider_name) {
            // Custom provider: remove key store entry + disable.
            self.key_store
                .remove(provider_name)
                .map_err(ServiceError::message)?;
            set_provider_enabled_in_config(provider_name, false)?;
            self.set_provider_enabled_in_memory(provider_name, false);
        } else {
            let providers = known_providers();
            let known = providers
                .iter()
                .find(|p| p.name == provider_name)
                .ok_or_else(|| format!("unknown provider: {provider_name}"))?;

            // Remove persisted API key
            if known.auth_type == AuthType::ApiKey {
                self.key_store
                    .remove(provider_name)
                    .map_err(ServiceError::message)?;
            }

            // Remove OAuth tokens
            if known.auth_type == AuthType::Oauth || provider_name == "kimi-code" {
                let _ = self.token_store.delete(provider_name);
            }

            // Persist explicit disable so auto-detected/global credentials do not
            // immediately re-enable the provider on next rebuild.
            set_provider_enabled_in_config(provider_name, false)?;
            self.set_provider_enabled_in_memory(provider_name, false);

            // Remove local-llm config
            #[cfg(feature = "local-llm")]
            if known.auth_type == AuthType::Local
                && provider_name == "local-llm"
                && let Some(config_dir) = moltis_config::config_dir()
            {
                let config_path = config_dir.join("local-llm.json");
                let _ = std::fs::remove_file(config_path);
            }
        }

        // Rebuild the provider registry without the removed provider.
        let effective = self.effective_config();
        let new_registry = self.build_registry(&effective);
        let mut reg = self.registry.write().await;
        *reg = new_registry;

        info!(
            provider = provider_name,
            "removed provider credentials and rebuilt registry"
        );

        Ok(serde_json::json!({ "ok": true }))
    }

    async fn oauth_status(&self, params: Value) -> ServiceResult {
        let provider_name = params
            .get("provider")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'provider' parameter".to_string())?;

        let has_tokens = self.has_oauth_tokens(provider_name);
        Ok(serde_json::json!({
            "provider": provider_name,
            "authenticated": has_tokens,
        }))
    }

    async fn validate_key(&self, params: Value) -> ServiceResult {
        use moltis_agents::model::ChatMessage;

        let provider_name = params
            .get("provider")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'provider' parameter".to_string())?;

        let api_key = params.get("apiKey").and_then(|v| v.as_str());
        let base_url = params.get("baseUrl").and_then(|v| v.as_str());
        let preferred_models = parse_models_param(&params);
        let request_id = params
            .get("requestId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(ToString::to_string);

        // Custom providers bypass known_providers() validation.
        let is_custom = is_custom_provider(provider_name);
        let provider_info = if is_custom {
            None
        } else {
            let known = known_providers();
            let info = known
                .iter()
                .find(|p| p.name == provider_name)
                .ok_or_else(|| format!("unknown provider: {provider_name}"))?;
            // API key is required for api-key providers (except Ollama).
            if info.auth_type == AuthType::ApiKey && provider_name != "ollama" && api_key.is_none()
            {
                return Err("missing 'apiKey' parameter".into());
            }
            Some(KnownProvider {
                name: info.name,
                display_name: info.display_name,
                auth_type: info.auth_type,
                env_key: info.env_key,
                default_base_url: info.default_base_url,
                requires_model: info.requires_model,
                key_optional: info.key_optional,
            })
        };

        if is_custom && api_key.is_none() {
            return Err("missing 'apiKey' parameter".into());
        }
        if is_custom && base_url.filter(|s| !s.trim().is_empty()).is_none() {
            return Err("missing 'baseUrl' parameter".into());
        }

        let selected_model = preferred_models.first().map(String::as_str);
        let base_url_value = base_url.filter(|s| !s.trim().is_empty());
        let validation_provider_name = validation_provider_name_for_endpoint(
            provider_name,
            provider_info.as_ref().and_then(|p| p.default_base_url),
            base_url_value,
        );
        let _timing =
            ProviderSetupTiming::start("providers.validate_key", Some(&validation_provider_name));
        self.emit_validation_progress(
            &validation_provider_name,
            request_id.as_deref(),
            "start",
            progress_payload(serde_json::json!({
                "message": "Starting provider validation.",
            })),
        )
        .await;

        // Ollama supports native model discovery through /api/tags.
        // If no model is supplied, return discovered models for UI selection.
        if provider_name == "ollama" {
            let ollama_api_base = normalize_ollama_api_base_url(
                base_url_value.or(provider_info.as_ref().and_then(|p| p.default_base_url)),
            );
            let discovered_models = match discover_ollama_models(&ollama_api_base).await {
                Ok(models) => models,
                Err(error) => {
                    let error = error.to_string();
                    self.emit_validation_progress(
                        &validation_provider_name,
                        request_id.as_deref(),
                        "error",
                        progress_payload(serde_json::json!({
                            "message": error.clone(),
                        })),
                    )
                    .await;
                    return Ok(serde_json::json!({
                        "valid": false,
                        "error": error,
                    }));
                },
            };

            if discovered_models.is_empty() {
                let error = "No Ollama models found. Install one first with `ollama pull <model>`.";
                self.emit_validation_progress(
                    &validation_provider_name,
                    request_id.as_deref(),
                    "error",
                    progress_payload(serde_json::json!({
                        "message": error,
                    })),
                )
                .await;
                return Ok(serde_json::json!({
                    "valid": false,
                    "error": error,
                }));
            }

            if let Some(requested_model) = selected_model {
                let requested_model = normalize_ollama_model_id(requested_model.trim());
                let installed = discovered_models
                    .iter()
                    .any(|installed_model| ollama_model_matches(installed_model, requested_model));
                if !installed {
                    let error = format!(
                        "Model '{requested_model}' is not installed in Ollama. Install it with `ollama pull {requested_model}`."
                    );
                    self.emit_validation_progress(
                        &validation_provider_name,
                        request_id.as_deref(),
                        "error",
                        progress_payload(serde_json::json!({
                            "message": error.clone(),
                        })),
                    )
                    .await;
                    return Ok(serde_json::json!({
                        "valid": false,
                        "error": error,
                    }));
                }
            } else {
                self.emit_validation_progress(
                    &validation_provider_name,
                    request_id.as_deref(),
                    "complete",
                    progress_payload(serde_json::json!({
                        "message": "Discovered installed Ollama models.",
                        "modelCount": discovered_models.len(),
                    })),
                )
                .await;
                return Ok(serde_json::json!({
                    "valid": true,
                    "models": ollama_models_payload(&discovered_models),
                }));
            }
        }

        let normalized_base_url = if provider_name == "ollama" {
            base_url_value.map(|url| normalize_ollama_openai_base_url(Some(url)))
        } else {
            base_url_value.map(String::from)
        };

        // Build a temporary ProvidersConfig with just this provider.
        let mut temp_config = ProvidersConfig::default();
        temp_config.providers.insert(
            validation_provider_name.clone(),
            moltis_config::schema::ProviderEntry {
                enabled: true,
                api_key: api_key.map(|k| Secret::new(k.to_string())),
                base_url: normalized_base_url,
                models: preferred_models,
                ..Default::default()
            },
        );

        // Build a temporary registry from the temp config.
        let temp_registry = self.build_registry(&temp_config);

        // Filter models for this provider.
        let models: Vec<_> = temp_registry
            .list_models()
            .iter()
            .filter(|m| {
                normalize_provider_name(&m.provider)
                    == normalize_provider_name(&validation_provider_name)
            })
            .cloned()
            .collect();

        if models.is_empty() {
            let error =
                "No models available for this provider. Check your credentials and try again.";
            self.emit_validation_progress(
                &validation_provider_name,
                request_id.as_deref(),
                "error",
                progress_payload(serde_json::json!({
                    "message": error,
                })),
            )
            .await;
            return Ok(serde_json::json!({
                "valid": false,
                "error": error,
            }));
        }

        info!(
            provider = %validation_provider_name,
            model_count = models.len(),
            "provider validation discovered candidate models for probing"
        );
        self.emit_validation_progress(
            &validation_provider_name,
            request_id.as_deref(),
            "candidates_discovered",
            progress_payload(serde_json::json!({
                "modelCount": models.len(),
                "message": format!("Discovered {} candidate models.", models.len()),
            })),
        )
        .await;

        const VALIDATION_MAX_MODEL_PROBES: usize = 8;
        const VALIDATION_MAX_TIMEOUTS: usize = 2;
        const VALIDATION_PROBE_TIMEOUT_SECS: u64 = 10;
        let total_probe_attempts = models.len().min(VALIDATION_MAX_MODEL_PROBES);

        let probe = [ChatMessage::user("ping")];
        let mut probe_attempted = false;
        let mut unsupported_errors = Vec::new();
        let mut last_error: Option<String> = None;
        let mut probe_succeeded = false;
        let mut timeout_count = 0usize;

        // Try multiple models because provider catalogs can include endpoint-
        // incompatible IDs. We only need one successful probe to validate creds.
        for (attempt, probe_model) in models.iter().take(VALIDATION_MAX_MODEL_PROBES).enumerate() {
            let Some(llm_provider) = temp_registry.get(&probe_model.id) else {
                continue;
            };

            probe_attempted = true;
            let probe_started = std::time::Instant::now();
            info!(
                provider = %validation_provider_name,
                model = %probe_model.id,
                attempt = attempt + 1,
                total_models = models.len(),
                "provider validation model probe started"
            );
            self.emit_validation_progress(
                &validation_provider_name,
                request_id.as_deref(),
                "probe_started",
                progress_payload(serde_json::json!({
                    "modelId": probe_model.id,
                    "attempt": attempt + 1,
                    "totalAttempts": total_probe_attempts,
                    "message": format!(
                        "Probing model {} of {}.",
                        attempt + 1,
                        total_probe_attempts
                    ),
                })),
            )
            .await;
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(VALIDATION_PROBE_TIMEOUT_SECS),
                llm_provider.complete(&probe, &[]),
            )
            .await;

            match result {
                Ok(Ok(_)) => {
                    let elapsed_ms = probe_started.elapsed().as_millis();
                    info!(
                        provider = %validation_provider_name,
                        model = %probe_model.id,
                        elapsed_ms,
                        "provider validation model probe succeeded"
                    );
                    self.emit_validation_progress(
                        &validation_provider_name,
                        request_id.as_deref(),
                        "probe_succeeded",
                        progress_payload(serde_json::json!({
                            "modelId": probe_model.id,
                            "elapsedMs": elapsed_ms,
                            "attempt": attempt + 1,
                            "totalAttempts": total_probe_attempts,
                            "message": "Model probe succeeded.",
                        })),
                    )
                    .await;
                    probe_succeeded = true;
                    break;
                },
                Ok(Err(err)) => {
                    let error_text = err.to_string();
                    let error_obj =
                        (self.error_parser)(&error_text, Some(&validation_provider_name));
                    let detail = error_obj
                        .get("detail")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&error_text)
                        .to_string();
                    let is_unsupported =
                        error_obj.get("type").and_then(|v| v.as_str()) == Some("unsupported_model");
                    let elapsed_ms = probe_started.elapsed().as_millis();
                    info!(
                        provider = %validation_provider_name,
                        model = %probe_model.id,
                        elapsed_ms,
                        unsupported = is_unsupported,
                        "provider validation model probe failed"
                    );
                    self.emit_validation_progress(
                        &validation_provider_name,
                        request_id.as_deref(),
                        "probe_failed",
                        progress_payload(serde_json::json!({
                            "modelId": probe_model.id,
                            "elapsedMs": elapsed_ms,
                            "attempt": attempt + 1,
                            "totalAttempts": total_probe_attempts,
                            "unsupported": is_unsupported,
                            "message": detail.clone(),
                        })),
                    )
                    .await;
                    if is_unsupported {
                        unsupported_errors.push(detail);
                        continue;
                    }
                    last_error = Some(detail);
                    break;
                },
                Err(_) => {
                    timeout_count += 1;
                    let elapsed_ms = probe_started.elapsed().as_millis();
                    warn!(
                        provider = %validation_provider_name,
                        model = %probe_model.id,
                        timeout_count,
                        max_timeouts = VALIDATION_MAX_TIMEOUTS,
                        elapsed_ms,
                        "provider validation model probe timed out"
                    );
                    self.emit_validation_progress(
                        &validation_provider_name,
                        request_id.as_deref(),
                        "probe_timeout",
                        progress_payload(serde_json::json!({
                            "modelId": probe_model.id,
                            "elapsedMs": elapsed_ms,
                            "attempt": attempt + 1,
                            "totalAttempts": total_probe_attempts,
                            "timeoutCount": timeout_count,
                            "maxTimeouts": VALIDATION_MAX_TIMEOUTS,
                            "message": format!(
                                "Timed out probing model after {VALIDATION_PROBE_TIMEOUT_SECS} seconds."
                            ),
                        })),
                    )
                    .await;
                    if timeout_count >= VALIDATION_MAX_TIMEOUTS {
                        last_error = Some(format!(
                            "Connection timed out after {VALIDATION_PROBE_TIMEOUT_SECS} seconds while validating models. Check your endpoint URL and try again."
                        ));
                        break;
                    }
                    continue;
                },
            }
        }

        if probe_succeeded {
            // Build model list for the frontend, excluding non-chat models.
            let model_list: Vec<Value> = models
                .iter()
                .filter(|m| moltis_providers::is_chat_capable_model(&m.id))
                .map(|m| {
                    let supports_tools =
                        temp_registry.get(&m.id).is_some_and(|p| p.supports_tools());
                    serde_json::json!({
                        "id": m.id,
                        "displayName": m.display_name,
                        "provider": m.provider,
                        "supportsTools": supports_tools,
                    })
                })
                .collect();

            self.emit_validation_progress(
                &validation_provider_name,
                request_id.as_deref(),
                "complete",
                progress_payload(serde_json::json!({
                    "message": "Validation complete.",
                    "modelCount": model_list.len(),
                })),
            )
            .await;
            return Ok(serde_json::json!({
                "valid": true,
                "models": model_list,
            }));
        }

        if !probe_attempted {
            let error = "Could not instantiate provider for probing.";
            self.emit_validation_progress(
                &validation_provider_name,
                request_id.as_deref(),
                "error",
                progress_payload(serde_json::json!({
                    "message": error,
                })),
            )
            .await;
            return Ok(serde_json::json!({
                "valid": false,
                "error": error,
            }));
        }

        if let Some(error) = last_error {
            self.emit_validation_progress(
                &validation_provider_name,
                request_id.as_deref(),
                "error",
                progress_payload(serde_json::json!({
                    "message": error,
                })),
            )
            .await;
            return Ok(serde_json::json!({
                "valid": false,
                "error": error,
            }));
        }

        let unsupported_error = unsupported_errors.into_iter().next().unwrap_or_else(|| {
            "No supported chat models were found for this provider.".to_string()
        });
        self.emit_validation_progress(
            &validation_provider_name,
            request_id.as_deref(),
            "error",
            progress_payload(serde_json::json!({
                "message": unsupported_error.clone(),
            })),
        )
        .await;
        Ok(serde_json::json!({
            "valid": false,
            "error": unsupported_error,
        }))
    }

    async fn save_model(&self, params: Value) -> ServiceResult {
        let provider_name = params
            .get("provider")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'provider' parameter".to_string())?;

        let model = params
            .get("model")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'model' parameter".to_string())?;

        // Validate provider exists (known or custom).
        if !is_custom_provider(provider_name) {
            let known = known_providers();
            if !known.iter().any(|p| p.name == provider_name) {
                return Err(format!("unknown provider: {provider_name}").into());
            }
        }

        // Prepend chosen model to existing saved models so it appears first,
        // while preserving any previously chosen models.
        let mut models = vec![model.to_string()];
        if let Some(existing) = self.key_store.load_config(provider_name) {
            models.extend(existing.models);
        }

        self.key_store
            .save_config(provider_name, None, None, Some(models))
            .map_err(ServiceError::message)?;

        // Update the cross-provider priority list so the dropdown puts
        // the chosen model at the top immediately.
        if let Some(ref priority) = self.priority_models {
            let mut list = priority.write().await;
            // Remove any existing occurrence and prepend.
            let normalized = model.to_string();
            list.retain(|m| m != &normalized);
            list.insert(0, normalized);
        }

        info!(
            provider = provider_name,
            model, "saved model preference and queued async registry rebuild"
        );
        self.queue_registry_rebuild(provider_name, "save_model");
        Ok(serde_json::json!({ "ok": true }))
    }

    async fn save_models(&self, params: Value) -> ServiceResult {
        let _timing = ProviderSetupTiming::start(
            "providers.save_models",
            params.get("provider").and_then(Value::as_str),
        );
        let provider_name = params
            .get("provider")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'provider' parameter".to_string())?;

        let models: Vec<String> = params
            .get("models")
            .and_then(|v| v.as_array())
            .ok_or_else(|| "missing 'models' array parameter".to_string())?
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();

        // Validate provider exists (known or custom).
        if !is_custom_provider(provider_name) {
            let known = known_providers();
            if !known.iter().any(|p| p.name == provider_name) {
                return Err(format!("unknown provider: {provider_name}").into());
            }
        }

        self.key_store
            .save_config(provider_name, None, None, Some(models.clone()))
            .map_err(ServiceError::message)?;

        // Update the cross-provider priority list.
        if let Some(ref priority) = self.priority_models {
            let mut list = priority.write().await;
            // Prepend all selected models in order, removing any existing
            // occurrences to avoid duplicates.
            for m in models.iter().rev() {
                list.retain(|existing| existing != m);
                list.insert(0, m.clone());
            }
        }

        info!(
            provider = provider_name,
            count = models.len(),
            models = ?models,
            "saved model preferences and queued async registry rebuild"
        );
        self.queue_registry_rebuild(provider_name, "save_models");
        Ok(serde_json::json!({ "ok": true }))
    }

    async fn add_custom(&self, params: Value) -> ServiceResult {
        let _timing = ProviderSetupTiming::start("providers.add_custom", None);

        let base_url = params
            .get("baseUrl")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| "missing 'baseUrl' parameter".to_string())?;

        let api_key = params
            .get("apiKey")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| "missing 'apiKey' parameter".to_string())?;

        let model = params
            .get("model")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty());

        let base_name = derive_provider_name_from_url(base_url)
            .ok_or_else(|| "could not parse endpoint URL".to_string())?;

        let existing = self.key_store.load_all_configs();
        let provider_name = existing_custom_provider_for_base_url(base_url, &existing)
            .unwrap_or_else(|| make_unique_provider_name(&base_name, &existing));
        let reused_existing_provider = existing.contains_key(&provider_name);
        let display_name = base_url_to_display_name(base_url);

        let models = model.map(|m| vec![m.to_string()]);

        self.key_store
            .save_config_with_display_name(
                &provider_name,
                Some(api_key.to_string()),
                Some(base_url.to_string()),
                models,
                Some(display_name.clone()),
            )
            .map_err(ServiceError::message)?;

        set_provider_enabled_in_config(&provider_name, true)?;
        self.set_provider_enabled_in_memory(&provider_name, true);

        // Rebuild synchronously so the just-added custom provider is immediately
        // available for model probing in the same UI flow.
        let effective = self.effective_config();
        let new_registry = self.build_registry(&effective);
        let provider_summary = new_registry.provider_summary();
        let model_count = new_registry.list_models().len();
        let mut reg = self.registry.write().await;
        *reg = new_registry;

        info!(
            provider = %provider_name,
            display_name = %display_name,
            reused = reused_existing_provider,
            provider_summary = %provider_summary,
            models = model_count,
            "saved custom OpenAI-compatible provider and rebuilt provider registry"
        );

        Ok(serde_json::json!({
            "ok": true,
            "providerName": provider_name,
            "displayName": display_name,
        }))
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {
        super::*, moltis_config::schema::ProviderEntry, moltis_oauth::OAuthTokens, secrecy::Secret,
    };

    #[test]
    fn known_providers_have_valid_auth_types() {
        for p in known_providers() {
            assert!(
                p.auth_type == AuthType::ApiKey
                    || p.auth_type == AuthType::Oauth
                    || p.auth_type == AuthType::Local,
                "invalid auth type for {}: {}",
                p.name,
                p.auth_type
            );
        }
    }

    #[test]
    fn api_key_providers_have_env_key() {
        for p in known_providers() {
            if p.auth_type == AuthType::ApiKey {
                assert!(
                    p.env_key.is_some(),
                    "api-key provider {} missing env_key",
                    p.name
                );
            }
        }
    }

    #[test]
    fn oauth_providers_have_no_env_key() {
        for p in known_providers() {
            if p.auth_type == AuthType::Oauth {
                assert!(
                    p.env_key.is_none(),
                    "oauth provider {} should not have env_key",
                    p.name
                );
            }
        }
    }

    #[test]
    fn local_providers_have_no_env_key() {
        for p in known_providers() {
            if p.auth_type == AuthType::Local {
                assert!(
                    p.env_key.is_none(),
                    "local provider {} should not have env_key",
                    p.name
                );
            }
        }
    }

    #[test]
    fn known_provider_names_unique() {
        let providers = known_providers();
        let mut names: Vec<&str> = providers.iter().map(|p| p.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), providers.len());
    }

    #[test]
    fn normalize_ollama_openai_base_url_appends_v1() {
        assert_eq!(
            normalize_ollama_openai_base_url(Some("http://localhost:11434")),
            "http://localhost:11434/v1"
        );
        assert_eq!(
            normalize_ollama_openai_base_url(Some("http://localhost:11434/v1")),
            "http://localhost:11434/v1"
        );
    }

    #[test]
    fn normalize_ollama_api_base_url_strips_v1() {
        assert_eq!(
            normalize_ollama_api_base_url(Some("http://localhost:11434/v1")),
            "http://localhost:11434"
        );
        assert_eq!(
            normalize_ollama_api_base_url(Some("http://localhost:11434")),
            "http://localhost:11434"
        );
    }

    #[test]
    fn ollama_model_matches_accepts_tag_suffix() {
        assert!(ollama_model_matches("llama3.2:latest", "llama3.2"));
        assert!(ollama_model_matches("qwen2.5:7b", "qwen2.5:7b"));
        assert!(!ollama_model_matches("llama3.2:latest", "qwen2.5"));
    }

    #[test]
    fn verification_uri_complete_prefers_provider_payload() {
        let complete = build_verification_uri_complete(
            "kimi-code",
            "https://auth.kimi.com/device",
            "ABCD-1234",
            Some("https://auth.kimi.com/device?user_code=ABCD-1234".into()),
        );
        assert_eq!(
            complete.as_deref(),
            Some("https://auth.kimi.com/device?user_code=ABCD-1234")
        );
    }

    #[test]
    fn verification_uri_complete_synthesizes_for_kimi() {
        let complete = build_verification_uri_complete(
            "kimi-code",
            "https://auth.kimi.com/device",
            "ABCD-1234",
            None,
        );
        assert_eq!(
            complete.as_deref(),
            Some("https://auth.kimi.com/device?user_code=ABCD-1234")
        );
    }

    #[test]
    fn verification_uri_complete_synthesizes_with_existing_query() {
        let complete = build_verification_uri_complete(
            "kimi-code",
            "https://auth.kimi.com/device?lang=en",
            "ABCD-1234",
            None,
        );
        assert_eq!(
            complete.as_deref(),
            Some("https://auth.kimi.com/device?lang=en&user_code=ABCD-1234")
        );
    }

    #[test]
    fn provider_headers_include_kimi_device_headers() {
        let headers = build_provider_headers("kimi-code").expect("expected kimi-code headers");
        assert!(headers.get("X-Msh-Platform").is_some());
        assert!(headers.get("X-Msh-Device-Id").is_some());
    }

    #[test]
    fn provider_headers_are_none_for_non_kimi() {
        assert!(build_provider_headers("github-copilot").is_none());
    }

    #[test]
    fn key_store_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::with_path(dir.path().join("keys.json"));
        assert!(store.load("anthropic").is_none());
        store.save("anthropic", "sk-test-123").unwrap();
        assert_eq!(store.load("anthropic").unwrap(), "sk-test-123");
        // Overwrite
        store.save("anthropic", "sk-new").unwrap();
        assert_eq!(store.load("anthropic").unwrap(), "sk-new");
        // Multiple providers
        store.save("openai", "sk-openai").unwrap();
        assert_eq!(store.load("openai").unwrap(), "sk-openai");
        assert_eq!(store.load("anthropic").unwrap(), "sk-new");
        let all = store.load_all();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn key_store_path_reports_backing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.json");
        let store = KeyStore::with_path(path.clone());
        assert_eq!(store.path(), path);
    }

    #[test]
    fn key_store_invalid_json_returns_empty_map() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.json");
        std::fs::write(&path, "{ invalid json").unwrap();

        let store = KeyStore::with_path(path);
        assert!(store.load_all_configs().is_empty());
    }

    #[test]
    fn key_store_remove() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::with_path(dir.path().join("keys.json"));
        store.save("anthropic", "sk-test").unwrap();
        store.save("openai", "sk-openai").unwrap();
        assert!(store.load("anthropic").is_some());
        store.remove("anthropic").unwrap();
        assert!(store.load("anthropic").is_none());
        // Other keys unaffected
        assert_eq!(store.load("openai").unwrap(), "sk-openai");
        // Removing non-existent key is fine
        store.remove("nonexistent").unwrap();
    }

    #[test]
    fn key_store_save_config_with_all_fields() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::with_path(dir.path().join("keys.json"));

        // Save full config
        store
            .save_config(
                "openai",
                Some("sk-openai".into()),
                Some("https://custom.api.com/v1".into()),
                Some(vec!["gpt-4o".into()]),
            )
            .unwrap();

        let config = store.load_config("openai").unwrap();
        assert_eq!(config.api_key.as_deref(), Some("sk-openai"));
        assert_eq!(
            config.base_url.as_deref(),
            Some("https://custom.api.com/v1")
        );
        assert_eq!(config.models, vec!["gpt-4o"]);
    }

    #[test]
    fn key_store_save_config_preserves_existing_fields() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::with_path(dir.path().join("keys.json"));

        // Save initial config with all fields
        store
            .save_config(
                "openai",
                Some("sk-openai".into()),
                Some("https://custom.api.com/v1".into()),
                Some(vec!["gpt-4o".into()]),
            )
            .unwrap();

        // Update only models, preserve others
        store
            .save_config("openai", None, None, Some(vec!["gpt-4o-mini".into()]))
            .unwrap();

        let config = store.load_config("openai").unwrap();
        assert_eq!(config.api_key.as_deref(), Some("sk-openai")); // preserved
        assert_eq!(
            config.base_url.as_deref(),
            Some("https://custom.api.com/v1")
        ); // preserved
        assert_eq!(config.models, vec!["gpt-4o-mini"]); // updated
    }

    #[test]
    fn key_store_save_config_preserves_other_providers() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::with_path(dir.path().join("keys.json"));

        store
            .save_config(
                "anthropic",
                Some("sk-anthropic".into()),
                Some("https://api.anthropic.com".into()),
                Some(vec!["claude-sonnet-4".into()]),
            )
            .unwrap();

        store
            .save_config(
                "openai",
                Some("sk-openai".into()),
                Some("https://api.openai.com/v1".into()),
                Some(vec!["gpt-4o".into()]),
            )
            .unwrap();

        // Update only OpenAI models, Anthropic should remain unchanged.
        store
            .save_config("openai", None, None, Some(vec!["gpt-5".into()]))
            .unwrap();

        let anthropic = store.load_config("anthropic").unwrap();
        assert_eq!(anthropic.api_key.as_deref(), Some("sk-anthropic"));
        assert_eq!(
            anthropic.base_url.as_deref(),
            Some("https://api.anthropic.com")
        );
        assert_eq!(anthropic.models, vec!["claude-sonnet-4"]);

        let openai = store.load_config("openai").unwrap();
        assert_eq!(openai.api_key.as_deref(), Some("sk-openai"));
        assert_eq!(
            openai.base_url.as_deref(),
            Some("https://api.openai.com/v1")
        );
        assert_eq!(openai.models, vec!["gpt-5"]);
    }

    #[test]
    fn key_store_concurrent_writes_do_not_drop_provider_entries() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::with_path(dir.path().join("keys.json"));

        let mut handles = Vec::new();
        for (provider, key, models) in [
            ("openai", "sk-openai", vec!["gpt-5".to_string()]),
            ("anthropic", "sk-anthropic", vec![
                "claude-sonnet-4".to_string(),
            ]),
        ] {
            let store = store.clone();
            handles.push(std::thread::spawn(move || {
                for _ in 0..100 {
                    store
                        .save_config(provider, Some(key.to_string()), None, Some(models.clone()))
                        .unwrap();
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let all = store.load_all_configs();
        assert!(all.contains_key("openai"));
        assert!(all.contains_key("anthropic"));
    }

    #[test]
    fn key_store_save_config_clears_empty_values() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::with_path(dir.path().join("keys.json"));

        // Save initial config
        store
            .save_config(
                "openai",
                Some("sk-openai".into()),
                Some("https://custom.api.com/v1".into()),
                Some(vec!["gpt-4o".into()]),
            )
            .unwrap();

        // Clear base_url by setting empty string
        store
            .save_config("openai", None, Some(String::new()), None)
            .unwrap();

        let config = store.load_config("openai").unwrap();
        assert_eq!(config.api_key.as_deref(), Some("sk-openai")); // preserved
        assert!(config.base_url.is_none()); // cleared
        assert_eq!(config.models, vec!["gpt-4o"]); // preserved
    }

    #[test]
    fn key_store_migrates_old_format() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.json");

        // Write old format: simple string values
        let old_data = serde_json::json!({
            "anthropic": "sk-old-key",
            "openai": "sk-openai-old"
        });
        std::fs::write(&path, serde_json::to_string(&old_data).unwrap()).unwrap();

        let store = KeyStore::with_path(path);

        // Should migrate and read correctly
        let config = store.load_config("anthropic").unwrap();
        assert_eq!(config.api_key.as_deref(), Some("sk-old-key"));
        assert!(config.base_url.is_none());
        assert!(config.models.is_empty());

        // load() should still work
        assert_eq!(store.load("openai").unwrap(), "sk-openai-old");
    }

    #[test]
    fn config_with_saved_keys_merges_base_url_and_models() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::with_path(dir.path().join("keys.json"));
        store
            .save_config(
                "openai",
                Some("sk-saved".into()),
                Some("https://custom.api.com/v1".into()),
                Some(vec!["gpt-4o".into()]),
            )
            .unwrap();

        let base = ProvidersConfig::default();
        let merged = config_with_saved_keys(&base, &store, &[]);
        let entry = merged.get("openai").unwrap();
        assert_eq!(
            entry.api_key.as_ref().map(|s| s.expose_secret().as_str()),
            Some("sk-saved")
        );
        assert_eq!(entry.base_url.as_deref(), Some("https://custom.api.com/v1"));
        assert_eq!(entry.models, vec!["gpt-4o"]);
    }

    #[tokio::test]
    async fn remove_key_rejects_unknown_provider() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
        let result = svc
            .remove_key(serde_json::json!({"provider": "nonexistent"}))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn remove_key_rejects_missing_params() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
        assert!(svc.remove_key(serde_json::json!({})).await.is_err());
    }

    #[tokio::test]
    async fn disabled_provider_is_not_reported_configured() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
        let provider = known_providers()
            .into_iter()
            .find(|p| p.name == "openai-codex")
            .expect("openai-codex should exist");

        let mut config = ProvidersConfig::default();
        config
            .providers
            .insert("openai-codex".into(), ProviderEntry {
                enabled: false,
                ..Default::default()
            });

        assert!(!svc.is_provider_configured(&provider, &config));
    }

    #[test]
    fn config_with_saved_keys_merges() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::with_path(dir.path().join("keys.json"));
        store.save("anthropic", "sk-saved").unwrap();

        let base = ProvidersConfig::default();
        let merged = config_with_saved_keys(&base, &store, &[]);
        let entry = merged.get("anthropic").unwrap();
        assert_eq!(
            entry.api_key.as_ref().map(|s| s.expose_secret().as_str()),
            Some("sk-saved")
        );
    }

    #[test]
    fn config_with_saved_keys_does_not_override_existing() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::with_path(dir.path().join("keys.json"));
        store.save("anthropic", "sk-saved").unwrap();

        let mut base = ProvidersConfig::default();
        base.providers.insert("anthropic".into(), ProviderEntry {
            api_key: Some(Secret::new("sk-config".into())),
            ..Default::default()
        });
        let merged = config_with_saved_keys(&base, &store, &[]);
        let entry = merged.get("anthropic").unwrap();
        // Config key takes precedence over saved key.
        assert_eq!(
            entry.api_key.as_ref().map(|s| s.expose_secret().as_str()),
            Some("sk-config")
        );
    }

    #[tokio::test]
    async fn noop_service_returns_empty() {
        use moltis_service_traits::NoopProviderSetupService;
        let svc = NoopProviderSetupService;
        let result = svc.available().await.unwrap();
        assert_eq!(result, serde_json::json!([]));
    }

    #[tokio::test]
    async fn live_service_lists_providers() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
        let result = svc.available().await.unwrap();
        let arr = result.as_array().unwrap();
        assert!(!arr.is_empty());
        // Check that we have expected fields
        let first = &arr[0];
        assert!(first.get("name").is_some());
        assert!(first.get("displayName").is_some());
        assert!(first.get("authType").is_some());
        assert!(first.get("configured").is_some());
        // New fields for endpoint and model configuration
        assert!(first.get("defaultBaseUrl").is_some());
        assert!(first.get("requiresModel").is_some());
        assert!(first.get("uiOrder").is_some());
    }

    #[tokio::test]
    async fn available_hides_unconfigured_providers_not_in_offered_list() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let config = ProvidersConfig {
            offered: vec!["openai".into()],
            ..ProvidersConfig::default()
        };
        let svc = LiveProviderSetupService::new(registry, config, None);

        let result = svc.available().await.unwrap();
        let arr = result.as_array().unwrap();
        for provider in arr {
            let configured = provider
                .get("configured")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let name = provider.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if !configured {
                assert_eq!(
                    name, "openai",
                    "only offered providers should be shown when unconfigured"
                );
            }
        }
    }

    #[tokio::test]
    async fn available_respects_offered_order() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let config = ProvidersConfig {
            offered: vec!["github-copilot".into(), "openai".into(), "anthropic".into()],
            ..ProvidersConfig::default()
        };
        let svc = LiveProviderSetupService::new(registry, config, None);
        let result = svc.available().await.unwrap();
        let arr = result
            .as_array()
            .expect("providers.available should return array");
        let names: Vec<&str> = arr
            .iter()
            .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
            .collect();

        let github_copilot_idx = names
            .iter()
            .position(|name| *name == "github-copilot")
            .expect("github-copilot should be present");
        let openai_idx = names
            .iter()
            .position(|name| *name == "openai")
            .expect("openai should be present");
        let anthropic_idx = names
            .iter()
            .position(|name| *name == "anthropic")
            .expect("anthropic should be present");

        assert!(
            github_copilot_idx < openai_idx && openai_idx < anthropic_idx,
            "offered provider order should be preserved, got: {names:?}"
        );
    }

    #[tokio::test]
    async fn available_hides_configured_provider_outside_offered() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let mut config = ProvidersConfig {
            offered: vec!["openai".into()],
            ..ProvidersConfig::default()
        };
        config.providers.insert("anthropic".into(), ProviderEntry {
            api_key: Some(Secret::new("sk-test".into())),
            ..Default::default()
        });
        let svc = LiveProviderSetupService::new(registry, config, None);
        let result = svc.available().await.unwrap();
        let arr = result
            .as_array()
            .expect("providers.available should return array");
        let names: Vec<&str> = arr
            .iter()
            .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
            .collect();

        let openai_idx = names
            .iter()
            .position(|name| *name == "openai")
            .expect("openai should be present");

        assert!(
            !names.contains(&"anthropic"),
            "providers outside offered should be hidden even when configured, got: {names:?}"
        );
        assert_eq!(openai_idx, 0);
    }

    #[tokio::test]
    async fn available_includes_subscription_provider_with_oauth_token_outside_offered() {
        let dir = tempfile::tempdir().expect("temp dir");
        let token_store = TokenStore::with_path(dir.path().join("oauth_tokens.json"));
        token_store
            .save("openai-codex", &OAuthTokens {
                access_token: Secret::new("token".to_string()),
                refresh_token: None,
                id_token: None,
                account_id: None,
                expires_at: None,
            })
            .expect("save oauth token");

        let key_store = KeyStore::with_path(dir.path().join("provider_keys.json"));
        let config = ProvidersConfig {
            offered: vec!["openai".into()],
            ..ProvidersConfig::default()
        };

        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService {
            registry,
            config: Arc::new(Mutex::new(config)),
            broadcaster: Arc::new(OnceCell::new()),
            token_store,
            key_store,
            pending_oauth: Arc::new(RwLock::new(HashMap::new())),
            deploy_platform: None,
            priority_models: None,
            registry_rebuild_seq: Arc::new(AtomicU64::new(0)),
            env_overrides: HashMap::new(),
            error_parser: default_error_parser,
            callback_bind_addr: "127.0.0.1".to_string(),
        };

        let result = svc.available().await.unwrap();
        let arr = result
            .as_array()
            .expect("providers.available should return array");
        let codex = arr
            .iter()
            .find(|v| v.get("name").and_then(|n| n.as_str()) == Some("openai-codex"))
            .expect("openai-codex should be present when oauth token exists");
        assert_eq!(
            codex.get("configured").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[tokio::test]
    async fn available_includes_configured_custom_provider_outside_offered() {
        let dir = tempfile::tempdir().expect("temp dir");
        let key_store = KeyStore::with_path(dir.path().join("provider_keys.json"));
        key_store
            .save_config_with_display_name(
                "custom-openrouter-ai",
                Some("sk-test".into()),
                Some("https://openrouter.ai/api/v1".into()),
                Some(vec!["openai::gpt-5.2".into()]),
                Some("openrouter.ai".into()),
            )
            .expect("save custom provider");

        let mut config = ProvidersConfig {
            offered: vec!["openai".into()],
            ..ProvidersConfig::default()
        };
        config
            .providers
            .insert("custom-openrouter-ai".into(), ProviderEntry {
                enabled: true,
                ..Default::default()
            });

        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService {
            registry,
            config: Arc::new(Mutex::new(config)),
            broadcaster: Arc::new(OnceCell::new()),
            token_store: TokenStore::new(),
            key_store,
            pending_oauth: Arc::new(RwLock::new(HashMap::new())),
            deploy_platform: None,
            priority_models: None,
            registry_rebuild_seq: Arc::new(AtomicU64::new(0)),
            env_overrides: HashMap::new(),
            error_parser: default_error_parser,
            callback_bind_addr: "127.0.0.1".to_string(),
        };

        let result = svc.available().await.expect("providers.available");
        let arr = result
            .as_array()
            .expect("providers.available should return array");
        let custom = arr
            .iter()
            .find(|v| v.get("name").and_then(|n| n.as_str()) == Some("custom-openrouter-ai"))
            .expect("custom provider should be visible");

        assert_eq!(
            custom.get("configured").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(custom.get("isCustom").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(
            custom.get("displayName").and_then(|v| v.as_str()),
            Some("openrouter.ai")
        );
    }

    #[tokio::test]
    async fn available_includes_default_base_urls() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
        let result = svc.available().await.unwrap();
        let arr = result.as_array().unwrap();

        // Check specific providers have correct default base URLs
        let openai = arr
            .iter()
            .find(|p| p.get("name").and_then(|n| n.as_str()) == Some("openai"))
            .expect("openai not found");
        assert_eq!(
            openai.get("defaultBaseUrl").and_then(|u| u.as_str()),
            Some("https://api.openai.com/v1")
        );

        let ollama = arr
            .iter()
            .find(|p| p.get("name").and_then(|n| n.as_str()) == Some("ollama"))
            .expect("ollama not found");
        assert_eq!(
            ollama.get("defaultBaseUrl").and_then(|u| u.as_str()),
            Some("http://localhost:11434")
        );
        assert_eq!(
            ollama.get("requiresModel").and_then(|r| r.as_bool()),
            Some(false)
        );

        let kimi_code = arr
            .iter()
            .find(|p| p.get("name").and_then(|n| n.as_str()) == Some("kimi-code"))
            .expect("kimi-code not found");
        assert_eq!(
            kimi_code.get("defaultBaseUrl").and_then(|u| u.as_str()),
            Some("https://api.kimi.com/coding/v1")
        );
    }

    #[tokio::test]
    async fn save_key_rejects_unknown_provider() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
        let result = svc
            .save_key(serde_json::json!({"provider": "nonexistent", "apiKey": "test"}))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn save_key_rejects_missing_params() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
        assert!(svc.save_key(serde_json::json!({})).await.is_err());
        assert!(
            svc.save_key(serde_json::json!({"provider": "anthropic"}))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn oauth_start_rejects_unknown_provider() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
        let result = svc
            .oauth_start(serde_json::json!({"provider": "nonexistent"}))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn oauth_start_ignores_redirect_uri_override_for_registered_provider() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);

        let result = svc
            .oauth_start(serde_json::json!({
                "provider": "openai-codex",
                "redirectUri": "https://example.com/auth/callback",
            }))
            .await
            .expect("oauth start should succeed");

        if result
            .get("alreadyAuthenticated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return;
        }
        let auth_url = result
            .get("authUrl")
            .and_then(|v| v.as_str())
            .expect("missing authUrl");
        let parsed = reqwest::Url::parse(auth_url).expect("authUrl should be a valid URL");
        let redirect = parsed
            .query_pairs()
            .find(|(k, _)| k == "redirect_uri")
            .map(|(_, v)| v.into_owned());

        // openai-codex has a pre-registered redirect_uri; client override is ignored.
        assert_eq!(
            redirect.as_deref(),
            Some("http://localhost:1455/auth/callback")
        );
    }

    #[tokio::test]
    async fn oauth_status_returns_not_authenticated() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
        let result = svc
            .oauth_status(serde_json::json!({"provider": "openai-codex"}))
            .await
            .unwrap();
        // Might or might not have tokens depending on environment
        assert!(result.get("authenticated").is_some());
    }

    #[test]
    fn oauth_token_presence_checks_primary_and_home_store() {
        let temp = tempfile::tempdir().expect("temp dir");
        let primary = TokenStore::with_path(temp.path().join("primary-oauth.json"));
        let home = TokenStore::with_path(temp.path().join("home-oauth.json"));

        assert!(!has_oauth_tokens_for_provider(
            "github-copilot",
            &primary,
            Some(&home)
        ));

        home.save("github-copilot", &OAuthTokens {
            access_token: Secret::new("home-token".to_string()),
            refresh_token: None,
            id_token: None,
            account_id: None,
            expires_at: None,
        })
        .expect("save home token");

        assert!(has_oauth_tokens_for_provider(
            "github-copilot",
            &primary,
            Some(&home)
        ));
    }

    #[test]
    fn known_providers_include_new_providers() {
        let providers = known_providers();
        let names: Vec<&str> = providers.iter().map(|p| p.name).collect();
        // All new OpenAI-compatible providers
        assert!(names.contains(&"mistral"), "missing mistral");
        assert!(names.contains(&"openrouter"), "missing openrouter");
        assert!(names.contains(&"cerebras"), "missing cerebras");
        assert!(names.contains(&"minimax"), "missing minimax");
        assert!(names.contains(&"moonshot"), "missing moonshot");
        assert!(names.contains(&"zai"), "missing zai");
        assert!(names.contains(&"kimi-code"), "missing kimi-code");
        assert!(names.contains(&"venice"), "missing venice");
        assert!(names.contains(&"ollama"), "missing ollama");
        // OAuth providers
        assert!(names.contains(&"github-copilot"), "missing github-copilot");
    }

    #[test]
    fn github_copilot_is_oauth_provider() {
        let providers = known_providers();
        let copilot = providers
            .iter()
            .find(|p| p.name == "github-copilot")
            .expect("github-copilot not in known_providers");
        assert_eq!(copilot.auth_type, AuthType::Oauth);
        assert!(copilot.env_key.is_none());
    }

    #[test]
    fn new_api_key_providers_have_correct_env_keys() {
        let expected = [
            ("mistral", "MISTRAL_API_KEY"),
            ("openrouter", "OPENROUTER_API_KEY"),
            ("cerebras", "CEREBRAS_API_KEY"),
            ("minimax", "MINIMAX_API_KEY"),
            ("moonshot", "MOONSHOT_API_KEY"),
            ("zai", "Z_API_KEY"),
            ("kimi-code", "KIMI_API_KEY"),
            ("venice", "VENICE_API_KEY"),
            ("ollama", "OLLAMA_API_KEY"),
        ];
        let providers = known_providers();
        for (name, env_key) in expected {
            let provider = providers
                .iter()
                .find(|p| p.name == name)
                .unwrap_or_else(|| panic!("missing provider: {name}"));
            assert_eq!(provider.env_key, Some(env_key), "wrong env_key for {name}");
            assert_eq!(provider.auth_type, AuthType::ApiKey);
        }
    }

    #[tokio::test]
    async fn save_key_accepts_new_providers() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let _svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);

        // All new API-key providers should be accepted by save_key
        let providers = known_providers();
        for name in [
            "mistral",
            "openrouter",
            "cerebras",
            "minimax",
            "moonshot",
            "zai",
            "kimi-code",
            "venice",
            "ollama",
        ] {
            // We can't actually persist in tests (would write to real disk),
            // but we can verify the provider name is recognized.
            let known = providers
                .iter()
                .find(|p| p.name == name && p.auth_type == AuthType::ApiKey);
            assert!(
                known.is_some(),
                "{name} should be a recognized api-key provider"
            );
        }
    }

    #[tokio::test]
    async fn available_includes_new_providers() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
        let result = svc.available().await.unwrap();
        let arr = result.as_array().unwrap();

        let names: Vec<&str> = arr
            .iter()
            .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
            .collect();

        for expected in [
            "mistral",
            "openrouter",
            "cerebras",
            "minimax",
            "moonshot",
            "zai",
            "kimi-code",
            "venice",
            "ollama",
            "github-copilot",
        ] {
            assert!(
                names.contains(&expected),
                "{expected} not found in available providers: {names:?}"
            );
        }
    }

    #[tokio::test]
    async fn available_hides_local_providers_on_cloud() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(
            registry,
            ProvidersConfig::default(),
            Some("flyio".to_string()),
        );
        let result = svc.available().await.unwrap();
        let arr = result.as_array().unwrap();

        let names: Vec<&str> = arr
            .iter()
            .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
            .collect();

        // local-llm and ollama should be hidden on cloud deployments
        assert!(
            !names.contains(&"local-llm"),
            "local-llm should be hidden on cloud: {names:?}"
        );
        assert!(
            !names.contains(&"ollama"),
            "ollama should be hidden on cloud: {names:?}"
        );

        // Cloud-compatible providers should still be present
        assert!(
            names.contains(&"openai"),
            "openai should be present on cloud: {names:?}"
        );
        assert!(
            names.contains(&"anthropic"),
            "anthropic should be present on cloud: {names:?}"
        );
    }

    #[tokio::test]
    async fn available_shows_all_providers_locally() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
        let result = svc.available().await.unwrap();
        let arr = result.as_array().unwrap();

        let names: Vec<&str> = arr
            .iter()
            .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
            .collect();

        // All providers should be present when running locally
        assert!(
            names.contains(&"ollama"),
            "ollama should be present locally: {names:?}"
        );
        assert!(
            names.contains(&"openai"),
            "openai should be present locally: {names:?}"
        );
    }

    #[test]
    fn has_explicit_provider_settings_detects_populated_provider_entries() {
        let mut empty = ProvidersConfig::default();
        assert!(!has_explicit_provider_settings(&empty));

        empty.providers.insert("openai".into(), ProviderEntry {
            api_key: Some(Secret::new("sk-test".into())),
            ..Default::default()
        });
        assert!(has_explicit_provider_settings(&empty));

        let mut model_only = ProvidersConfig::default();
        model_only.providers.insert("ollama".into(), ProviderEntry {
            models: vec!["llama3".into()],
            ..Default::default()
        });
        assert!(has_explicit_provider_settings(&model_only));
    }

    #[tokio::test]
    async fn validate_key_rejects_unknown_provider() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
        let result = svc
            .validate_key(serde_json::json!({"provider": "nonexistent", "apiKey": "sk-test"}))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown provider"));
    }

    #[tokio::test]
    async fn validate_key_rejects_missing_provider_param() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
        let result = svc.validate_key(serde_json::json!({})).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("missing 'provider'")
        );
    }

    #[tokio::test]
    async fn validate_key_rejects_missing_api_key_for_api_key_provider() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
        let result = svc
            .validate_key(serde_json::json!({"provider": "anthropic"}))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing 'apiKey'"));
    }

    #[tokio::test]
    async fn validate_key_allows_missing_api_key_for_ollama() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
        // Ollama doesn't require an API key, so this should not error on missing apiKey.
        // It will likely return valid=false due to connection issues, but it should not
        // reject with a "missing apiKey" error.
        let result = svc
            .validate_key(serde_json::json!({"provider": "ollama"}))
            .await;
        // Should succeed (return Ok) even without apiKey — the probe may fail,
        // but param validation should pass.
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn validate_key_ollama_without_model_returns_discovered_models() {
        use axum::{Json, Router, routing::get};

        let app = Router::new().route(
            "/api/tags",
            get(|| async {
                Json(serde_json::json!({
                    "models": [
                        {"name": "llama3.2:latest"},
                        {"name": "qwen2.5:7b"}
                    ]
                }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
        let result = svc
            .validate_key(serde_json::json!({
                "provider": "ollama",
                "baseUrl": format!("http://{addr}")
            }))
            .await
            .expect("validate_key should return payload");
        server.abort();

        assert_eq!(result.get("valid").and_then(|v| v.as_bool()), Some(true));
        let models = result
            .get("models")
            .and_then(|v| v.as_array())
            .expect("models array should be present");
        assert!(
            models
                .iter()
                .any(|m| m.get("id").and_then(|v| v.as_str()) == Some("ollama::llama3.2:latest"))
        );
        assert!(
            models
                .iter()
                .any(|m| m.get("id").and_then(|v| v.as_str()) == Some("ollama::qwen2.5:7b"))
        );
    }

    #[tokio::test]
    async fn validate_key_ollama_reports_uninstalled_model() {
        use axum::{Json, Router, routing::get};

        let app = Router::new().route(
            "/api/tags",
            get(|| async {
                Json(serde_json::json!({
                    "models": [
                        {"name": "llama3.2:latest"}
                    ]
                }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
        let result = svc
            .validate_key(serde_json::json!({
                "provider": "ollama",
                "baseUrl": format!("http://{addr}"),
                "model": "qwen2.5:7b"
            }))
            .await
            .expect("validate_key should return payload");
        server.abort();

        assert_eq!(result.get("valid").and_then(|v| v.as_bool()), Some(false));
        let error = result.get("error").and_then(|v| v.as_str()).unwrap_or("");
        assert!(
            error.contains("not installed in Ollama"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn validate_key_ollama_model_probe_uses_v1_endpoint() {
        use axum::{
            Json, Router,
            routing::{get, post},
        };

        let app = Router::new()
            .route(
                "/api/tags",
                get(|| async {
                    Json(serde_json::json!({
                        "models": [
                            {"name": "llama3.2:latest"}
                        ]
                    }))
                }),
            )
            .route(
                "/v1/chat/completions",
                post(|| async {
                    Json(serde_json::json!({
                        "choices": [{"message": {"content": "pong"}}],
                        "usage": {
                            "prompt_tokens": 1,
                            "completion_tokens": 1,
                            "prompt_tokens_details": {"cached_tokens": 0}
                        }
                    }))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
        let result = svc
            .validate_key(serde_json::json!({
                "provider": "ollama",
                "baseUrl": format!("http://{addr}"),
                "model": "llama3.2"
            }))
            .await
            .expect("validate_key should return payload");
        server.abort();

        assert_eq!(result.get("valid").and_then(|v| v.as_bool()), Some(true));
        let models = result
            .get("models")
            .and_then(|v| v.as_array())
            .expect("models array should be present");
        assert!(
            models
                .iter()
                .any(|m| m.get("id").and_then(|v| v.as_str()) == Some("ollama::llama3.2"))
        );
    }

    #[test]
    fn codex_cli_auth_has_access_token_requires_tokens_access_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");

        std::fs::write(&path, r#"{"tokens":{"access_token":"abc123"}}"#).unwrap();
        assert!(codex_cli_auth_has_access_token(&path));

        std::fs::write(&path, r#"{"tokens":{"access_token":""}}"#).unwrap();
        assert!(!codex_cli_auth_has_access_token(&path));

        std::fs::write(&path, r#"{"not_tokens":true}"#).unwrap();
        assert!(!codex_cli_auth_has_access_token(&path));
    }

    #[test]
    fn is_custom_provider_detects_prefix() {
        assert!(is_custom_provider("custom-together-ai"));
        assert!(is_custom_provider("custom-openrouter-ai"));
        assert!(!is_custom_provider("openai"));
        assert!(!is_custom_provider("anthropic"));
    }

    #[test]
    fn derive_provider_name_from_url_extracts_host() {
        assert_eq!(
            derive_provider_name_from_url("https://api.together.ai/v1"),
            Some("custom-together-ai".into())
        );
        assert_eq!(
            derive_provider_name_from_url("https://openrouter.ai/api/v1"),
            Some("custom-openrouter-ai".into())
        );
        assert_eq!(
            derive_provider_name_from_url("https://api.example.com"),
            Some("custom-example-com".into())
        );
        assert_eq!(derive_provider_name_from_url("not-a-url"), None);
    }

    #[test]
    fn make_unique_provider_name_appends_suffix() {
        let mut existing = HashMap::new();
        assert_eq!(
            make_unique_provider_name("custom-foo", &existing),
            "custom-foo"
        );

        existing.insert("custom-foo".into(), ProviderConfig::default());
        assert_eq!(
            make_unique_provider_name("custom-foo", &existing),
            "custom-foo-2"
        );

        existing.insert("custom-foo-2".into(), ProviderConfig::default());
        assert_eq!(
            make_unique_provider_name("custom-foo", &existing),
            "custom-foo-3"
        );
    }

    #[test]
    fn base_url_to_display_name_strips_api_prefix() {
        assert_eq!(
            base_url_to_display_name("https://api.together.ai/v1"),
            "together.ai"
        );
        assert_eq!(
            base_url_to_display_name("https://openrouter.ai/api/v1"),
            "openrouter.ai"
        );
    }

    #[test]
    fn validation_provider_name_for_endpoint_keeps_openai_for_default_url() {
        assert_eq!(
            validation_provider_name_for_endpoint(
                "openai",
                Some("https://api.openai.com/v1"),
                Some("https://api.openai.com/v1/"),
            ),
            "openai"
        );
    }

    #[test]
    fn validation_provider_name_for_endpoint_maps_openai_override_to_custom() {
        assert_eq!(
            validation_provider_name_for_endpoint(
                "openai",
                Some("https://api.openai.com/v1"),
                Some("https://openrouter.ai/api/v1"),
            ),
            "custom-openrouter-ai"
        );
    }

    #[test]
    fn validation_provider_name_for_endpoint_preserves_explicit_custom_provider() {
        assert_eq!(
            validation_provider_name_for_endpoint(
                "custom-openrouter-ai",
                Some("https://api.openai.com/v1"),
                Some("https://openrouter.ai/api/v1"),
            ),
            "custom-openrouter-ai"
        );
    }

    #[test]
    fn normalize_base_url_for_compare_is_stable() {
        assert_eq!(
            normalize_base_url_for_compare("https://OPENROUTER.ai/api/v1/"),
            "https://openrouter.ai/api/v1"
        );
        assert_eq!(
            normalize_base_url_for_compare(" https://openrouter.ai/api/v1 "),
            "https://openrouter.ai/api/v1"
        );
        assert_eq!(
            normalize_base_url_for_compare("http://localhost:11434/v1/"),
            "http://localhost:11434/v1"
        );
        assert_eq!(
            normalize_base_url_for_compare("HTTP://LOCALHOST:11434/v1"),
            "http://localhost:11434/v1"
        );
    }

    #[test]
    fn existing_custom_provider_for_base_url_prefers_canonical_name() {
        let mut existing = HashMap::new();
        existing.insert("custom-openrouter-ai".into(), ProviderConfig {
            base_url: Some("https://openrouter.ai/api/v1".into()),
            ..Default::default()
        });
        existing.insert("custom-openrouter-ai-2".into(), ProviderConfig {
            base_url: Some("https://OPENROUTER.ai/api/v1/".into()),
            ..Default::default()
        });
        existing.insert("custom-together-ai".into(), ProviderConfig {
            base_url: Some("https://api.together.ai/v1".into()),
            ..Default::default()
        });

        assert_eq!(
            existing_custom_provider_for_base_url("https://openrouter.ai/api/v1", &existing),
            Some("custom-openrouter-ai".into())
        );
        assert_eq!(
            existing_custom_provider_for_base_url("https://api.together.ai/v1", &existing),
            Some("custom-together-ai".into())
        );
        assert_eq!(
            existing_custom_provider_for_base_url("https://example.com/v1", &existing),
            None
        );
    }

    #[test]
    fn key_store_save_config_with_display_name() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::with_path(dir.path().join("keys.json"));

        store
            .save_config_with_display_name(
                "custom-together-ai",
                Some("sk-test".into()),
                Some("https://api.together.ai/v1".into()),
                Some(vec!["meta-llama/Llama-3-70b".into()]),
                Some("together.ai".into()),
            )
            .unwrap();

        let config = store.load_config("custom-together-ai").unwrap();
        assert_eq!(config.api_key.as_deref(), Some("sk-test"));
        assert_eq!(
            config.base_url.as_deref(),
            Some("https://api.together.ai/v1")
        );
        assert_eq!(config.display_name.as_deref(), Some("together.ai"));
    }

    #[test]
    fn normalize_model_list_strips_provider_namespace() {
        let models = normalize_model_list(vec![
            "openai::gpt-5.2".into(),
            "custom-openrouter-ai::gpt-5.2".into(),
            "gpt-5.2".into(),
            "  anthropic/claude-sonnet-4-5  ".into(),
        ]);
        assert_eq!(models, vec!["gpt-5.2", "anthropic/claude-sonnet-4-5"]);
    }
}
