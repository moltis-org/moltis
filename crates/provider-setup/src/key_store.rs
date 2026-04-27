use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use {serde_json::Value, tracing::warn};

use moltis_providers::model_id::raw_model_id;

// ── Model normalization helpers ────────────────────────────────────────────

pub(crate) fn normalize_model_list(models: impl IntoIterator<Item = String>) -> Vec<String> {
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
        if out.iter().any(|existing: &String| existing == &normalized) {
            continue;
        }
        out.push(normalized);
    }
    out
}

pub(crate) fn parse_models_param(params: &Value) -> Vec<String> {
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

// ── ProviderConfig ─────────────────────────────────────────────────────────

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

// ── KeyStore ───────────────────────────────────────────────────────────────

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

    pub(crate) fn with_path(path: PathBuf) -> Self {
        Self {
            inner: Arc::new(Mutex::new(KeyStoreInner { path })),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, KeyStoreInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn path(&self) -> PathBuf {
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
    ) -> crate::error::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                warn!(
                    path = %parent.display(),
                    error = %error,
                    "failed to create provider key store directory"
                );
                crate::error::Error::external(
                    "failed to create provider key store directory",
                    error,
                )
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
            crate::error::Error::external("failed to write provider key store temp file", error)
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
            crate::error::Error::external("failed to atomically replace provider key store", error)
        })?;

        Ok(())
    }

    /// Load all API keys (used in tests).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn load_all(&self) -> HashMap<String, String> {
        self.load_all_configs()
            .into_iter()
            .filter_map(|(k, v)| v.api_key.map(|key| (k, key)))
            .collect()
    }

    /// Load a provider's API key.
    pub fn load(&self, provider: &str) -> Option<String> {
        self.load_all_configs()
            .get(provider)
            .and_then(|c| c.api_key.clone())
    }

    /// Load a provider's full config.
    pub fn load_config(&self, provider: &str) -> Option<ProviderConfig> {
        self.load_all_configs().get(provider).cloned()
    }

    /// Remove a provider's configuration.
    pub fn remove(&self, provider: &str) -> crate::error::Result<()> {
        let guard = self.lock();
        let mut configs = Self::load_all_configs_from_path(&guard.path);
        configs.remove(provider);
        Self::save_all_configs_to_path(&guard.path, &configs)
    }

    /// Save a provider's API key (simple interface, used in tests).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn save(&self, provider: &str, api_key: &str) -> crate::error::Result<()> {
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
    ) -> crate::error::Result<()> {
        self.save_config_with_display_name(provider, api_key, base_url, models, None)
    }

    /// Load all provider configs from vault-encrypted storage, falling back to
    /// plaintext when the vault is unavailable or when the plaintext file is
    /// newer than the encrypted copy (indicating a sync write occurred since
    /// the last vault-unseal encryption).
    #[cfg(feature = "vault")]
    pub async fn load_all_configs_encrypted<C: moltis_vault::Cipher>(
        &self,
        vault: Option<&moltis_vault::Vault<C>>,
    ) -> HashMap<String, ProviderConfig> {
        let path = self.path();

        // If the plaintext is newer than the .enc file, a sync write happened
        // after the last vault-unseal encryption.  Prefer the fresher plaintext
        // so we don't silently return stale data.
        let enc_path = path.with_extension("json.enc");
        if path.exists() && enc_path.exists() {
            let json_mod = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            let enc_mod = std::fs::metadata(&enc_path).and_then(|m| m.modified()).ok();
            if let (Some(j), Some(e)) = (json_mod, enc_mod)
                && j > e
            {
                return Self::load_all_configs_from_path(&path);
            }
        }

        match moltis_vault::migration::load_encrypted_or_plaintext(vault, &path, "provider_keys")
            .await
        {
            Ok(Some(content)) => {
                if let Ok(configs) =
                    serde_json::from_str::<HashMap<String, ProviderConfig>>(&content)
                {
                    return configs;
                }
                // Fall back to old format migration
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
                warn!("encrypted provider key store is invalid JSON");
                HashMap::new()
            },
            Ok(None) => HashMap::new(),
            Err(moltis_vault::VaultError::Sealed) => {
                warn!("vault sealed, falling back to plaintext provider key store");
                Self::load_all_configs_from_path(&path)
            },
            Err(e) => {
                warn!(error = %e, "failed to decrypt provider key store, falling back to plaintext");
                Self::load_all_configs_from_path(&path)
            },
        }
    }

    /// Save all provider configs with vault encryption when available,
    /// falling back to plaintext.
    ///
    /// Always writes the plaintext `.json` too so sync callers continue to
    /// work until the full async migration is complete.
    #[cfg(feature = "vault")]
    pub async fn save_all_configs_encrypted<C: moltis_vault::Cipher>(
        &self,
        vault: Option<&moltis_vault::Vault<C>>,
        configs: &HashMap<String, ProviderConfig>,
    ) -> crate::error::Result<()> {
        let path = self.path();
        // Always write the plaintext file for sync consumers.
        Self::save_all_configs_to_path(&path, configs)?;

        // Write encrypted copy when vault is available.
        if let Some(vault) = vault {
            let data = serde_json::to_string_pretty(configs).map_err(|error| {
                warn!(error = %error, "failed to serialize provider key store");
                error
            })?;
            if let Err(e) = moltis_vault::migration::save_encrypted_or_plaintext(
                Some(vault),
                &path,
                "provider_keys",
                &data,
            )
            .await
            {
                warn!(error = %e, "failed to write encrypted provider key store");
            }
        }
        Ok(())
    }

    /// Save a provider's full configuration, including an optional display name.
    pub(crate) fn save_config_with_display_name(
        &self,
        provider: &str,
        api_key: Option<String>,
        base_url: Option<String>,
        models: Option<Vec<String>>,
        display_name: Option<String>,
    ) -> crate::error::Result<()> {
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

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

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
