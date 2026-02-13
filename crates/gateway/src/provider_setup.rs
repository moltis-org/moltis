use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use secrecy::{ExposeSecret, Secret};

use {
    async_trait::async_trait,
    serde_json::Value,
    tokio::sync::RwLock,
    tracing::{debug, info},
};

use {
    moltis_agents::providers::ProviderRegistry,
    moltis_config::schema::ProvidersConfig,
    moltis_oauth::{
        CallbackServer, OAuthFlow, TokenStore, callback_port, device_flow, load_oauth_config,
    },
};

use crate::services::{ProviderSetupService, ServiceResult};

// ── Key store ──────────────────────────────────────────────────────────────

/// Per-provider stored configuration (API key, base URL, model).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// File-based provider config storage at `~/.config/moltis/provider_keys.json`.
/// Stores per-provider configuration including API keys, base URLs, and models.
#[derive(Debug, Clone)]
pub(crate) struct KeyStore {
    inner: Arc<Mutex<KeyStoreInner>>,
}

#[derive(Debug)]
struct KeyStoreInner {
    path: PathBuf,
}

impl KeyStore {
    pub(crate) fn new() -> Self {
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

    /// Load all provider configs. Handles migration from old format (string values).
    fn load_all_configs_from_path(path: &PathBuf) -> HashMap<String, ProviderConfig> {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return HashMap::new(),
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
                        model: None,
                    })
                })
                .collect();
        }

        HashMap::new()
    }

    fn load_all_configs(&self) -> HashMap<String, ProviderConfig> {
        let guard = self.lock();
        Self::load_all_configs_from_path(&guard.path)
    }

    /// Save all provider configs to disk.
    fn save_all_configs_to_path(
        path: &PathBuf,
        configs: &HashMap<String, ProviderConfig>,
    ) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let data = serde_json::to_string_pretty(configs).map_err(|e| e.to_string())?;

        // Write atomically via temp file + rename so readers never observe
        // partially-written JSON.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let temp_path = path.with_extension(format!("json.tmp.{nanos}"));
        std::fs::write(&temp_path, &data).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o600));
        }

        std::fs::rename(&temp_path, path).map_err(|e| e.to_string())?;

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
    fn load_config(&self, provider: &str) -> Option<ProviderConfig> {
        self.load_all_configs().get(provider).cloned()
    }

    /// Remove a provider's configuration.
    fn remove(&self, provider: &str) -> Result<(), String> {
        let guard = self.lock();
        let mut configs = Self::load_all_configs_from_path(&guard.path);
        configs.remove(provider);
        Self::save_all_configs_to_path(&guard.path, &configs)
    }

    /// Save a provider's API key (simple interface, used in tests).
    #[cfg_attr(not(test), allow(dead_code))]
    fn save(&self, provider: &str, api_key: &str) -> Result<(), String> {
        self.save_config(
            provider,
            Some(api_key.to_string()),
            None, // preserve existing base_url
            None, // preserve existing model
        )
    }

    /// Save a provider's full configuration.
    fn save_config(
        &self,
        provider: &str,
        api_key: Option<String>,
        base_url: Option<String>,
        model: Option<String>,
    ) -> Result<(), String> {
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
        if let Some(m) = model {
            entry.model = if m.is_empty() {
                None
            } else {
                Some(m)
            };
        }

        Self::save_all_configs_to_path(&guard.path, &configs)
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Merge persisted provider configs into a ProvidersConfig so the registry rebuild
/// picks them up without needing env vars.
pub(crate) fn config_with_saved_keys(
    base: &ProvidersConfig,
    key_store: &KeyStore,
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
            if dst.model.is_none()
                && let Some(model) = entry.model
                && !model.trim().is_empty()
            {
                dst.model = Some(model);
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
        if saved.model.is_some() {
            entry.model = saved.model;
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

        // Only override model if config doesn't already have one.
        if let Some(model) = saved.model
            && entry.model.is_none()
        {
            entry.model = Some(model);
        }
    }

    // Merge local-llm config from UI-saved file (local-llm.json)
    #[cfg(feature = "local-llm")]
    {
        if let Some(local_config) = crate::local_llm_setup::LocalLlmConfig::load() {
            // Collect all configured model IDs for multi-model support
            config.local_models = local_config
                .models
                .iter()
                .map(|m| m.model_id.clone())
                .collect();

            // Also set the first model as the default for backward compatibility
            let entry = config.providers.entry("local".into()).or_default();
            if entry.model.is_none()
                && let Some(first_model) = local_config.models.first()
            {
                entry.model = Some(first_model.model_id.clone());
            }
        }
    }

    config
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

async fn discover_ollama_models(base_url: &str) -> Result<Vec<String>, String> {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let response = reqwest::Client::new().get(&url).send().await.map_err(|e| {
        format!("Failed to connect to Ollama at {base_url}. Ensure Ollama is running. ({e})")
    })?;

    if !response.status().is_success() {
        return Err(format!(
            "Ollama model discovery failed at {url} (HTTP {}).",
            response.status()
        ));
    }

    let payload: OllamaTagsResponse = response
        .json()
        .await
        .map_err(|e| format!("Invalid JSON from Ollama model discovery endpoint: {e}"))?;

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

/// Known provider definitions used to populate the "available providers" list.
struct KnownProvider {
    name: &'static str,
    display_name: &'static str,
    auth_type: &'static str,
    env_key: Option<&'static str>,
    /// Default base URL for this provider (for OpenAI-compatible providers).
    default_base_url: Option<&'static str>,
    /// Whether this provider requires a model to be specified.
    requires_model: bool,
}

/// Build the known providers list at runtime, including local-llm if enabled.
fn known_providers() -> Vec<KnownProvider> {
    let providers = vec![
        KnownProvider {
            name: "anthropic",
            display_name: "Anthropic",
            auth_type: "api-key",
            env_key: Some("ANTHROPIC_API_KEY"),
            default_base_url: Some("https://api.anthropic.com"),
            requires_model: false,
        },
        KnownProvider {
            name: "openai",
            display_name: "OpenAI",
            auth_type: "api-key",
            env_key: Some("OPENAI_API_KEY"),
            default_base_url: Some("https://api.openai.com/v1"),
            requires_model: false,
        },
        KnownProvider {
            name: "gemini",
            display_name: "Google Gemini",
            auth_type: "api-key",
            env_key: Some("GEMINI_API_KEY"),
            default_base_url: Some("https://generativelanguage.googleapis.com/v1beta"),
            requires_model: false,
        },
        KnownProvider {
            name: "groq",
            display_name: "Groq",
            auth_type: "api-key",
            env_key: Some("GROQ_API_KEY"),
            default_base_url: Some("https://api.groq.com/openai/v1"),
            requires_model: false,
        },
        KnownProvider {
            name: "xai",
            display_name: "xAI (Grok)",
            auth_type: "api-key",
            env_key: Some("XAI_API_KEY"),
            default_base_url: Some("https://api.x.ai/v1"),
            requires_model: false,
        },
        KnownProvider {
            name: "deepseek",
            display_name: "DeepSeek",
            auth_type: "api-key",
            env_key: Some("DEEPSEEK_API_KEY"),
            default_base_url: Some("https://api.deepseek.com"),
            requires_model: false,
        },
        KnownProvider {
            name: "mistral",
            display_name: "Mistral",
            auth_type: "api-key",
            env_key: Some("MISTRAL_API_KEY"),
            default_base_url: Some("https://api.mistral.ai/v1"),
            requires_model: false,
        },
        KnownProvider {
            name: "openrouter",
            display_name: "OpenRouter",
            auth_type: "api-key",
            env_key: Some("OPENROUTER_API_KEY"),
            default_base_url: Some("https://openrouter.ai/api/v1"),
            requires_model: true, // User must specify which model to use
        },
        KnownProvider {
            name: "cerebras",
            display_name: "Cerebras",
            auth_type: "api-key",
            env_key: Some("CEREBRAS_API_KEY"),
            default_base_url: Some("https://api.cerebras.ai/v1"),
            requires_model: false,
        },
        KnownProvider {
            name: "minimax",
            display_name: "MiniMax",
            auth_type: "api-key",
            env_key: Some("MINIMAX_API_KEY"),
            default_base_url: Some("https://api.minimax.chat/v1"),
            requires_model: false,
        },
        KnownProvider {
            name: "moonshot",
            display_name: "Moonshot",
            auth_type: "api-key",
            env_key: Some("MOONSHOT_API_KEY"),
            default_base_url: Some("https://api.moonshot.cn/v1"),
            requires_model: false,
        },
        KnownProvider {
            name: "venice",
            display_name: "Venice",
            auth_type: "api-key",
            env_key: Some("VENICE_API_KEY"),
            default_base_url: Some("https://api.venice.ai/api/v1"),
            requires_model: true, // User must specify which model to use
        },
        KnownProvider {
            name: "ollama",
            display_name: "Ollama",
            auth_type: "api-key", // API key is optional, handled specially in UI
            env_key: Some("OLLAMA_API_KEY"),
            default_base_url: Some("http://localhost:11434"),
            requires_model: false, // Models are discovered from the local Ollama instance
        },
        KnownProvider {
            name: "openai-codex",
            display_name: "OpenAI Codex",
            auth_type: "oauth",
            env_key: None,
            default_base_url: None,
            requires_model: false,
        },
        KnownProvider {
            name: "github-copilot",
            display_name: "GitHub Copilot",
            auth_type: "oauth",
            env_key: None,
            default_base_url: None,
            requires_model: false,
        },
        KnownProvider {
            name: "kimi-code",
            display_name: "Kimi Code",
            auth_type: "api-key",
            env_key: Some("KIMI_API_KEY"),
            default_base_url: Some("https://api.kimi.com/coding/v1"),
            requires_model: false,
        },
    ];

    // Add local-llm provider when the local-llm feature is enabled
    #[cfg(feature = "local-llm")]
    let providers = {
        let mut p = providers;
        p.push(KnownProvider {
            name: "local-llm",
            display_name: "Local LLM (Offline)",
            auth_type: "local",
            env_key: None,
            default_base_url: None,
            requires_model: true,
        });
        p
    };

    providers
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AutoDetectedProviderSource {
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
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    json.get("tokens")
        .and_then(|t| t.get("access_token"))
        .and_then(|v| v.as_str())
        .is_some_and(|token| !token.trim().is_empty())
}

/// Parse Codex CLI `auth.json` content into `OAuthTokens`.
fn parse_codex_cli_tokens(data: &str) -> Option<moltis_oauth::OAuthTokens> {
    let json: serde_json::Value = serde_json::from_str(data).ok()?;
    let tokens = json.get("tokens")?;
    let access_token = tokens.get("access_token")?.as_str()?.to_string();
    if access_token.trim().is_empty() {
        return None;
    }
    let refresh_token = tokens
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some(moltis_oauth::OAuthTokens {
        access_token: Secret::new(access_token),
        refresh_token: refresh_token.map(Secret::new),
        expires_at: None,
    })
}

/// Import auto-detected external OAuth tokens into the token store so all
/// providers read from a single location. Currently handles Codex CLI
/// `~/.codex/auth.json` → `openai-codex` in the token store.
pub(crate) fn import_detected_oauth_tokens(
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

fn set_provider_enabled_in_config(provider: &str, enabled: bool) -> Result<(), String> {
    moltis_config::update_config(|cfg| {
        let entry = cfg
            .providers
            .providers
            .entry(provider.to_string())
            .or_default();
        entry.enabled = enabled;
    })
    .map(|_| ())
    .map_err(|e| e.to_string())
}

fn normalize_provider_name(value: &str) -> String {
    value.trim().to_ascii_lowercase()
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

pub(crate) fn has_explicit_provider_settings(config: &ProvidersConfig) -> bool {
    config.providers.values().any(|entry| {
        entry
            .api_key
            .as_ref()
            .is_some_and(|k| !k.expose_secret().trim().is_empty())
            || entry
                .model
                .as_deref()
                .is_some_and(|model| !model.trim().is_empty())
            || entry
                .base_url
                .as_deref()
                .is_some_and(|url| !url.trim().is_empty())
    })
}

pub(crate) fn detect_auto_provider_sources(
    config: &ProvidersConfig,
    deploy_platform: Option<&str>,
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
    let local_llm_config_path = config_dir.join("local-llm.json");
    let codex_path = codex_cli_auth_path();

    let mut seen = BTreeSet::new();
    let mut detected = Vec::new();

    for provider in known_providers().into_iter().filter(|p| {
        if is_cloud {
            return p.auth_type != "local" && p.name != "ollama";
        }
        true
    }) {
        let mut sources = Vec::new();

        if let Some(env_key) = provider.env_key
            && std::env::var(env_key)
                .ok()
                .is_some_and(|v| !v.trim().is_empty())
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

        if (provider.auth_type == "oauth" || provider.name == "kimi-code")
            && token_store.load(provider.name).is_some()
        {
            sources.push(format!("file:{}", oauth_tokens_path.display()));
        }
        if (provider.auth_type == "oauth" || provider.name == "kimi-code")
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

pub struct LiveProviderSetupService {
    registry: Arc<RwLock<ProviderRegistry>>,
    config: Arc<Mutex<ProvidersConfig>>,
    token_store: TokenStore,
    key_store: KeyStore,
    pending_oauth: Arc<RwLock<HashMap<String, PendingOAuthFlow>>>,
    /// When set, local-only providers (local-llm, ollama) are hidden from
    /// the available list because they cannot run on cloud VMs.
    deploy_platform: Option<String>,
    /// Normalized allowlist patterns for filtering models (lowercase, non-empty).
    allowed_models: Vec<String>,
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
        allowed_models: Vec<String>,
    ) -> Self {
        let allowed_models: Vec<String> = allowed_models
            .into_iter()
            .map(|p| crate::chat::normalize_model_key(&p))
            .filter(|p| !p.is_empty())
            .collect();
        Self {
            registry,
            config: Arc::new(Mutex::new(config)),
            token_store: TokenStore::new(),
            key_store: KeyStore::new(),
            pending_oauth: Arc::new(RwLock::new(HashMap::new())),
            deploy_platform,
            allowed_models,
        }
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
        // Explicitly disabled providers should not show as configured even if
        // auto-detected credentials exist in home directories.
        if active_config
            .get(provider.name)
            .is_some_and(|entry| !entry.enabled)
        {
            return false;
        }

        // Check if the provider has an API key set via env
        if let Some(env_key) = provider.env_key
            && std::env::var(env_key).is_ok()
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
        if provider.auth_type == "oauth" || provider.name == "kimi-code" {
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
        if provider.auth_type == "local" && provider.name == "local-llm" {
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
        .map_err(|e| e.to_string())?;

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
                    let new_registry = ProviderRegistry::from_env_with_config(&config);
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
        config_with_saved_keys(&base, &self.key_store)
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
                if is_cloud && (provider.auth_type == "local" || provider.name == "ollama") {
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

                // Get saved config for this provider (baseUrl, model)
                let saved_config = self.key_store.load_config(provider.name);
                let base_url = saved_config.as_ref().and_then(|c| c.base_url.clone());
                let model = saved_config.as_ref().and_then(|c| c.model.clone());

                Some((
                    offered_rank.get(&normalized_name).copied(),
                    known_idx,
                    serde_json::json!({
                        "name": provider.name,
                        "displayName": provider.display_name,
                        "authType": provider.auth_type,
                        "configured": configured,
                        "defaultBaseUrl": provider.default_base_url,
                        "baseUrl": base_url,
                        "model": model,
                        "requiresModel": provider.requires_model,
                    }),
                ))
            })
            .collect();

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
        let provider_name = params
            .get("provider")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'provider' parameter".to_string())?;

        // API key is optional for some providers (e.g., Ollama)
        let api_key = params.get("apiKey").and_then(|v| v.as_str());
        let base_url = params.get("baseUrl").and_then(|v| v.as_str());
        let model = params.get("model").and_then(|v| v.as_str());

        // Validate provider name - allow both api-key and local providers
        let known = known_providers();
        let provider = known
            .iter()
            .find(|p| {
                p.name == provider_name && (p.auth_type == "api-key" || p.auth_type == "local")
            })
            .ok_or_else(|| format!("unknown provider: {provider_name}"))?;

        // API key is required for api-key providers (except Ollama which is optional)
        if provider.auth_type == "api-key" && provider_name != "ollama" && api_key.is_none() {
            return Err("missing 'apiKey' parameter".to_string());
        }

        let normalized_base_url = if provider_name == "ollama" {
            base_url.map(|url| normalize_ollama_openai_base_url(Some(url)))
        } else {
            base_url.map(String::from)
        };

        // Persist full config to disk
        self.key_store.save_config(
            provider_name,
            api_key.map(String::from),
            normalized_base_url,
            model.map(String::from),
        )?;
        set_provider_enabled_in_config(provider_name, true)?;
        self.set_provider_enabled_in_memory(provider_name, true);

        // Rebuild the provider registry with saved keys merged into config.
        let effective = self.effective_config();
        let new_registry = ProviderRegistry::from_env_with_config(&effective);
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
            let new_registry = ProviderRegistry::from_env_with_config(&effective);
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

        let use_server_callback = redirect_uri.is_some();
        if let Some(uri) = redirect_uri {
            oauth_config.redirect_uri = uri;
        }

        let port = callback_port(&oauth_config);
        let oauth_config_for_pending = oauth_config.clone();
        let flow = OAuthFlow::new(oauth_config);
        let auth_req = flow.start().map_err(|e| e.to_string())?;

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
        tokio::spawn(async move {
            match CallbackServer::wait_for_code(port, expected_state).await {
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
                            let new_registry = ProviderRegistry::from_env_with_config(&config);
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
            .map_err(|e| e.to_string())?;

        self.token_store
            .save(&pending.provider_name, &tokens)
            .map_err(|e| e.to_string())?;
        set_provider_enabled_in_config(&pending.provider_name, true)?;
        self.set_provider_enabled_in_memory(&pending.provider_name, true);

        let effective = self.effective_config();
        let new_registry = ProviderRegistry::from_env_with_config(&effective);
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

        let providers = known_providers();
        let known = providers
            .iter()
            .find(|p| p.name == provider_name)
            .ok_or_else(|| format!("unknown provider: {provider_name}"))?;

        // Remove persisted API key
        if known.auth_type == "api-key" {
            self.key_store.remove(provider_name)?;
        }

        // Remove OAuth tokens
        if known.auth_type == "oauth" || provider_name == "kimi-code" {
            let _ = self.token_store.delete(provider_name);
        }

        // Persist explicit disable so auto-detected/global credentials do not
        // immediately re-enable the provider on next rebuild.
        set_provider_enabled_in_config(provider_name, false)?;
        self.set_provider_enabled_in_memory(provider_name, false);

        // Remove local-llm config
        #[cfg(feature = "local-llm")]
        if known.auth_type == "local"
            && provider_name == "local-llm"
            && let Some(config_dir) = moltis_config::config_dir()
        {
            let config_path = config_dir.join("local-llm.json");
            let _ = std::fs::remove_file(config_path);
        }

        // Rebuild the provider registry without the removed provider.
        let effective = self.effective_config();
        let new_registry = ProviderRegistry::from_env_with_config(&effective);
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
        use moltis_agents::model::{ChatMessage, LlmProvider};

        let provider_name = params
            .get("provider")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'provider' parameter".to_string())?;

        let api_key = params.get("apiKey").and_then(|v| v.as_str());
        let base_url = params.get("baseUrl").and_then(|v| v.as_str());
        let model = params.get("model").and_then(|v| v.as_str());

        // Validate provider name exists.
        let known = known_providers();
        let provider_info = known
            .iter()
            .find(|p| p.name == provider_name)
            .ok_or_else(|| format!("unknown provider: {provider_name}"))?;

        // API key is required for api-key providers (except Ollama).
        if provider_info.auth_type == "api-key" && provider_name != "ollama" && api_key.is_none() {
            return Err("missing 'apiKey' parameter".to_string());
        }

        let model_value = model.filter(|s| !s.trim().is_empty());
        let base_url_value = base_url.filter(|s| !s.trim().is_empty());

        // Ollama supports native model discovery through /api/tags.
        // If no model is supplied, return discovered models for UI selection.
        if provider_name == "ollama" {
            let ollama_api_base =
                normalize_ollama_api_base_url(base_url_value.or(provider_info.default_base_url));
            let discovered_models = match discover_ollama_models(&ollama_api_base).await {
                Ok(models) => models,
                Err(error) => {
                    return Ok(serde_json::json!({
                        "valid": false,
                        "error": error,
                    }));
                },
            };

            if discovered_models.is_empty() {
                return Ok(serde_json::json!({
                    "valid": false,
                    "error": "No Ollama models found. Install one first with `ollama pull <model>`.",
                }));
            }

            if let Some(requested_model) = model_value {
                let requested_model = normalize_ollama_model_id(requested_model.trim());
                let installed = discovered_models
                    .iter()
                    .any(|installed_model| ollama_model_matches(installed_model, requested_model));
                if !installed {
                    return Ok(serde_json::json!({
                        "valid": false,
                        "error": format!(
                            "Model '{requested_model}' is not installed in Ollama. Install it with `ollama pull {requested_model}`."
                        ),
                    }));
                }
            } else {
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
            provider_name.to_string(),
            moltis_config::schema::ProviderEntry {
                enabled: true,
                api_key: api_key.map(|k| Secret::new(k.to_string())),
                base_url: normalized_base_url,
                model: model_value.map(String::from),
                ..Default::default()
            },
        );

        // Build a temporary registry from the temp config.
        let temp_registry = ProviderRegistry::from_env_with_config(&temp_config);

        // Filter models for this provider and by allowlist.
        let models: Vec<_> = temp_registry
            .list_models()
            .iter()
            .filter(|m| {
                normalize_provider_name(&m.provider) == normalize_provider_name(provider_name)
            })
            .filter(|m| {
                let runtime_provider_name = temp_registry.get(&m.id).map(|p| p.name().to_string());
                crate::chat::model_matches_allowlist_with_provider(
                    m,
                    runtime_provider_name.as_deref(),
                    &self.allowed_models,
                )
            })
            .cloned()
            .collect();

        if models.is_empty() {
            return Ok(serde_json::json!({
                "valid": false,
                "error": "No models available for this provider. Check your credentials and try again.",
            }));
        }

        // Probe the first available model with a "ping" message.
        let probe_model = &models[0];
        let llm_provider: Arc<dyn LlmProvider> = match temp_registry.get(&probe_model.id) {
            Some(p) => p,
            None => {
                return Ok(serde_json::json!({
                    "valid": false,
                    "error": "Could not instantiate provider for probing.",
                }));
            },
        };

        let probe = [ChatMessage::user("ping")];
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            llm_provider.complete(&probe, &[]),
        )
        .await;

        match result {
            Ok(Ok(_)) => {
                // Build model list for the frontend.
                let model_list: Vec<serde_json::Value> = models
                    .iter()
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

                Ok(serde_json::json!({
                    "valid": true,
                    "models": model_list,
                }))
            },
            Ok(Err(err)) => {
                let error_text = err.to_string();
                let error_obj =
                    crate::chat_error::parse_chat_error(&error_text, Some(provider_name));
                let detail = error_obj
                    .get("detail")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&error_text);

                Ok(serde_json::json!({
                    "valid": false,
                    "error": detail,
                }))
            },
            Err(_) => Ok(serde_json::json!({
                "valid": false,
                "error": "Connection timed out after 20 seconds. Check your endpoint URL and try again.",
            })),
        }
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

        // Validate provider exists.
        let known = known_providers();
        if !known.iter().any(|p| p.name == provider_name) {
            return Err(format!("unknown provider: {provider_name}"));
        }

        self.key_store
            .save_config(provider_name, None, None, Some(model.to_string()))?;

        info!(
            provider = provider_name,
            model, "saved model preference for provider"
        );
        Ok(serde_json::json!({ "ok": true }))
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
                p.auth_type == "api-key" || p.auth_type == "oauth" || p.auth_type == "local",
                "invalid auth type for {}: {}",
                p.name,
                p.auth_type
            );
        }
    }

    #[test]
    fn api_key_providers_have_env_key() {
        for p in known_providers() {
            if p.auth_type == "api-key" {
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
            if p.auth_type == "oauth" {
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
            if p.auth_type == "local" {
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
                Some("gpt-4o".into()),
            )
            .unwrap();

        let config = store.load_config("openai").unwrap();
        assert_eq!(config.api_key.as_deref(), Some("sk-openai"));
        assert_eq!(
            config.base_url.as_deref(),
            Some("https://custom.api.com/v1")
        );
        assert_eq!(config.model.as_deref(), Some("gpt-4o"));
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
                Some("gpt-4o".into()),
            )
            .unwrap();

        // Update only the model, preserve others
        store
            .save_config("openai", None, None, Some("gpt-4o-mini".into()))
            .unwrap();

        let config = store.load_config("openai").unwrap();
        assert_eq!(config.api_key.as_deref(), Some("sk-openai")); // preserved
        assert_eq!(
            config.base_url.as_deref(),
            Some("https://custom.api.com/v1")
        ); // preserved
        assert_eq!(config.model.as_deref(), Some("gpt-4o-mini")); // updated
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
                Some("claude-sonnet-4".into()),
            )
            .unwrap();

        store
            .save_config(
                "openai",
                Some("sk-openai".into()),
                Some("https://api.openai.com/v1".into()),
                Some("gpt-4o".into()),
            )
            .unwrap();

        // Update only OpenAI model, Anthropic should remain unchanged.
        store
            .save_config("openai", None, None, Some("gpt-5".into()))
            .unwrap();

        let anthropic = store.load_config("anthropic").unwrap();
        assert_eq!(anthropic.api_key.as_deref(), Some("sk-anthropic"));
        assert_eq!(
            anthropic.base_url.as_deref(),
            Some("https://api.anthropic.com")
        );
        assert_eq!(anthropic.model.as_deref(), Some("claude-sonnet-4"));

        let openai = store.load_config("openai").unwrap();
        assert_eq!(openai.api_key.as_deref(), Some("sk-openai"));
        assert_eq!(
            openai.base_url.as_deref(),
            Some("https://api.openai.com/v1")
        );
        assert_eq!(openai.model.as_deref(), Some("gpt-5"));
    }

    #[test]
    fn key_store_concurrent_writes_do_not_drop_provider_entries() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::with_path(dir.path().join("keys.json"));

        let mut handles = Vec::new();
        for (provider, key, model) in [
            ("openai", "sk-openai", "gpt-5"),
            ("anthropic", "sk-anthropic", "claude-sonnet-4"),
        ] {
            let store = store.clone();
            handles.push(std::thread::spawn(move || {
                for _ in 0..100 {
                    store
                        .save_config(
                            provider,
                            Some(key.to_string()),
                            None,
                            Some(model.to_string()),
                        )
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
                Some("gpt-4o".into()),
            )
            .unwrap();

        // Clear base_url by setting empty string
        store
            .save_config("openai", None, Some(String::new()), None)
            .unwrap();

        let config = store.load_config("openai").unwrap();
        assert_eq!(config.api_key.as_deref(), Some("sk-openai")); // preserved
        assert!(config.base_url.is_none()); // cleared
        assert_eq!(config.model.as_deref(), Some("gpt-4o")); // preserved
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
        assert!(config.model.is_none());

        // load() should still work
        assert_eq!(store.load("openai").unwrap(), "sk-openai-old");
    }

    #[test]
    fn config_with_saved_keys_merges_base_url_and_model() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::with_path(dir.path().join("keys.json"));
        store
            .save_config(
                "openai",
                Some("sk-saved".into()),
                Some("https://custom.api.com/v1".into()),
                Some("gpt-4o".into()),
            )
            .unwrap();

        let base = ProvidersConfig::default();
        let merged = config_with_saved_keys(&base, &store);
        let entry = merged.get("openai").unwrap();
        assert_eq!(
            entry.api_key.as_ref().map(|s| s.expose_secret().as_str()),
            Some("sk-saved")
        );
        assert_eq!(entry.base_url.as_deref(), Some("https://custom.api.com/v1"));
        assert_eq!(entry.model.as_deref(), Some("gpt-4o"));
    }

    #[tokio::test]
    async fn remove_key_rejects_unknown_provider() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None, vec![]);
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
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None, vec![]);
        assert!(svc.remove_key(serde_json::json!({})).await.is_err());
    }

    #[tokio::test]
    async fn disabled_provider_is_not_reported_configured() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None, vec![]);
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
        let merged = config_with_saved_keys(&base, &store);
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
        let merged = config_with_saved_keys(&base, &store);
        let entry = merged.get("anthropic").unwrap();
        // Config key takes precedence over saved key.
        assert_eq!(
            entry.api_key.as_ref().map(|s| s.expose_secret().as_str()),
            Some("sk-config")
        );
    }

    #[tokio::test]
    async fn noop_service_returns_empty() {
        use crate::services::NoopProviderSetupService;
        let svc = NoopProviderSetupService;
        let result = svc.available().await.unwrap();
        assert_eq!(result, serde_json::json!([]));
    }

    #[tokio::test]
    async fn live_service_lists_providers() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None, vec![]);
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
        let svc = LiveProviderSetupService::new(registry, config, None, vec![]);

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
        let svc = LiveProviderSetupService::new(registry, config, None, vec![]);
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
    async fn available_orders_configured_provider_after_offered() {
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
        let svc = LiveProviderSetupService::new(registry, config, None, vec![]);
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
        let anthropic_idx = names
            .iter()
            .position(|name| *name == "anthropic")
            .expect("anthropic should be present");

        assert!(
            openai_idx < anthropic_idx,
            "configured providers outside offered should appear after offered providers, got: {names:?}"
        );
    }

    #[tokio::test]
    async fn available_includes_default_base_urls() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None, vec![]);
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
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None, vec![]);
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
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None, vec![]);
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
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None, vec![]);
        let result = svc
            .oauth_start(serde_json::json!({"provider": "nonexistent"}))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn oauth_start_uses_redirect_uri_override() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None, vec![]);
        let redirect_uri = "https://example.com/auth/callback";

        let result = svc
            .oauth_start(serde_json::json!({
                "provider": "openai-codex",
                "redirectUri": redirect_uri,
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

        assert_eq!(redirect.as_deref(), Some(redirect_uri));
    }

    #[tokio::test]
    async fn oauth_status_returns_not_authenticated() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None, vec![]);
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
        assert_eq!(copilot.auth_type, "oauth");
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
            assert_eq!(provider.auth_type, "api-key");
        }
    }

    #[tokio::test]
    async fn save_key_accepts_new_providers() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let _svc =
            LiveProviderSetupService::new(registry, ProvidersConfig::default(), None, vec![]);

        // All new API-key providers should be accepted by save_key
        let providers = known_providers();
        for name in [
            "mistral",
            "openrouter",
            "cerebras",
            "minimax",
            "moonshot",
            "kimi-code",
            "venice",
            "ollama",
        ] {
            // We can't actually persist in tests (would write to real disk),
            // but we can verify the provider name is recognized.
            let known = providers
                .iter()
                .find(|p| p.name == name && p.auth_type == "api-key");
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
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None, vec![]);
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
            vec![],
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
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None, vec![]);
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
            model: Some("llama3".into()),
            ..Default::default()
        });
        assert!(has_explicit_provider_settings(&model_only));
    }

    #[tokio::test]
    async fn validate_key_rejects_unknown_provider() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None, vec![]);
        let result = svc
            .validate_key(serde_json::json!({"provider": "nonexistent", "apiKey": "sk-test"}))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown provider"));
    }

    #[tokio::test]
    async fn validate_key_rejects_missing_provider_param() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None, vec![]);
        let result = svc.validate_key(serde_json::json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing 'provider'"));
    }

    #[tokio::test]
    async fn validate_key_rejects_missing_api_key_for_api_key_provider() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None, vec![]);
        let result = svc
            .validate_key(serde_json::json!({"provider": "anthropic"}))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing 'apiKey'"));
    }

    #[tokio::test]
    async fn validate_key_allows_missing_api_key_for_ollama() {
        let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
            &ProvidersConfig::default(),
        )));
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None, vec![]);
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
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None, vec![]);
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
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None, vec![]);
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
        let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None, vec![]);
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
}
