//! Import LLM provider configuration from OpenClaw.
//!
//! Reads auth-profiles.json for API keys and openclaw.json for model selection,
//! then maps to the Moltis `provider_keys.json` format.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use {
    moltis_oauth::{OAuthTokens, TokenStore},
    secrecy::Secret,
    serde::{Deserialize, Serialize},
    tracing::debug,
};

use crate::{
    detect::{OpenClawDetection, resolve_agent_auth_profiles_path},
    report::{CategoryReport, ImportCategory, ImportStatus},
    types::{OpenClawAuthProfile, OpenClawAuthProfileStore, OpenClawConfig},
};

/// Moltis `provider_keys.json` entry (matches gateway `ProviderConfig`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoltisProviderConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// Imported provider data ready to be written.
#[derive(Debug, Clone, Default)]
pub struct ImportedProviders {
    /// Provider name → config (for `provider_keys.json`).
    pub providers: HashMap<String, MoltisProviderConfig>,
    /// Provider name → OAuth token set (for `oauth_tokens.json`).
    pub oauth_tokens: HashMap<String, OAuthTokens>,
    /// Primary model reference (e.g. "claude-opus-4-6").
    pub primary_model: Option<String>,
    /// Primary model provider name.
    pub primary_provider: Option<String>,
    /// Fallback model references.
    pub fallback_models: Vec<(String, String)>,
}

/// Map an OpenClaw provider name to a Moltis provider name.
pub fn map_provider_name(oc_name: &str) -> String {
    match oc_name.to_lowercase().as_str() {
        "anthropic" => "anthropic".to_string(),
        "openai" => "openai".to_string(),
        "google" => "gemini".to_string(),
        "groq" => "groq".to_string(),
        "xai" => "xai".to_string(),
        "deepseek" => "deepseek".to_string(),
        "ollama" => "ollama".to_string(),
        "openrouter" => "openrouter".to_string(),
        _ => oc_name.to_string(),
    }
}

/// Parse an OpenClaw model reference like `"anthropic/claude-opus-4-6"`
/// into `(provider_name, model_id)`.
pub fn parse_model_ref(model_ref: &str) -> Option<(String, String)> {
    let (provider, model) = model_ref.split_once('/')?;
    if provider.is_empty() || model.is_empty() {
        return None;
    }
    Some((map_provider_name(provider), model.to_string()))
}

/// Import provider configuration from an OpenClaw installation.
pub fn import_providers(detection: &OpenClawDetection) -> (CategoryReport, ImportedProviders) {
    let mut result = ImportedProviders::default();
    let warnings = Vec::new();
    let mut imported_providers: HashSet<String> = HashSet::new();

    // 1. Load auth profiles from all agents
    let mut provider_keys: HashMap<String, String> = HashMap::new();
    let mut oauth_tokens: HashMap<String, OAuthTokens> = HashMap::new();
    for agent_id in &detection.agent_ids {
        let agent_dir = detection.home_dir.join("agents").join(agent_id);
        if let Some(profiles_path) = resolve_agent_auth_profiles_path(&agent_dir)
            && let Some(store) = load_auth_profiles(&profiles_path)
        {
            for profile in store.profiles.values() {
                let provider = map_provider_name(profile.provider());
                if let Some(key) = extract_api_key(profile)
                    && !key.is_empty()
                {
                    debug!(provider = %provider, "found API key for provider");
                    provider_keys.entry(provider.clone()).or_insert(key);
                }
                if let Some(tokens) = extract_oauth_tokens(profile) {
                    oauth_tokens.entry(provider).or_insert(tokens);
                }
            }
        }
    }

    // 2. Also check for a credentials directory
    let oauth_path = detection.home_dir.join("credentials").join("oauth.json");
    if let Some(store) = load_auth_profiles(&oauth_path) {
        for profile in store.profiles.values() {
            let provider = map_provider_name(profile.provider());
            if let Some(key) = extract_api_key(profile)
                && !key.is_empty()
            {
                provider_keys.entry(provider.clone()).or_insert(key);
            }
            if let Some(tokens) = extract_oauth_tokens(profile) {
                oauth_tokens.entry(provider).or_insert(tokens);
            }
        }
    }

    // 3. Load config for model preferences
    let config = load_config(&detection.home_dir.join("openclaw.json"));

    // 4. Parse primary model
    if let Some(ref primary) = config.agents.defaults.model.primary
        && let Some((provider, model)) = parse_model_ref(primary)
    {
        debug!(provider = %provider, model = %model, "parsed primary model");
        result.primary_provider = Some(provider.clone());
        result.primary_model = Some(model.clone());

        // Ensure provider entry exists with model preference
        let entry = result.providers.entry(provider.clone()).or_default();
        if !entry.models.contains(&model) {
            entry.models.push(model);
        }
        imported_providers.insert(provider);
    }

    // 5. Parse fallback models
    for fallback in &config.agents.defaults.model.fallbacks {
        if let Some((provider, model)) = parse_model_ref(fallback) {
            result
                .fallback_models
                .push((provider.clone(), model.clone()));
            let entry = result.providers.entry(provider.clone()).or_default();
            if !entry.models.contains(&model) {
                entry.models.push(model);
            }
            imported_providers.insert(provider);
        }
    }

    // 6. Merge API keys into provider configs
    for (provider, key) in provider_keys {
        let entry = result.providers.entry(provider.clone()).or_default();
        entry.api_key = Some(key);
        imported_providers.insert(provider);
    }

    // 7. Merge OAuth tokens.
    for (provider, tokens) in oauth_tokens {
        result.oauth_tokens.insert(provider.clone(), tokens);
        imported_providers.insert(provider);
    }

    // Ensure model-only providers are counted as imported providers.
    for provider in result.providers.keys() {
        imported_providers.insert(provider.clone());
    }

    let items = imported_providers.len();

    let status = if items == 0 {
        ImportStatus::Skipped
    } else {
        ImportStatus::Success
    };

    let has_warnings = !warnings.is_empty();
    let mut report = CategoryReport {
        category: ImportCategory::Providers,
        status,
        items_imported: items,
        items_updated: 0,
        items_skipped: 0,
        warnings,
        errors: Vec::new(),
    };

    if has_warnings {
        report.status = ImportStatus::Partial;
    }

    (report, result)
}

/// Write imported providers to a `provider_keys.json` file.
pub fn write_provider_keys(
    providers: &HashMap<String, MoltisProviderConfig>,
    dest: &Path,
) -> crate::error::Result<()> {
    if providers.is_empty() {
        return Ok(());
    }

    // Load existing file if present
    let mut existing: HashMap<String, MoltisProviderConfig> = if dest.is_file() {
        let content = std::fs::read_to_string(dest)?;
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        HashMap::new()
    };

    // Merge: imported values override existing
    for (name, config) in providers {
        existing.insert(name.clone(), config.clone());
    }

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(&existing)?;
    std::fs::write(dest, json)?;
    Ok(())
}

fn load_config(path: &Path) -> OpenClawConfig {
    if !path.is_file() {
        return OpenClawConfig::default();
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return OpenClawConfig::default();
    };
    json5::from_str(&content).unwrap_or_default()
}

fn load_auth_profiles(path: &Path) -> Option<OpenClawAuthProfileStore> {
    if !path.is_file() {
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn extract_api_key(profile: &OpenClawAuthProfile) -> Option<String> {
    match profile {
        OpenClawAuthProfile::ApiKey { key, .. } => key.clone(),
        _ => None,
    }
}

fn extract_oauth_tokens(profile: &OpenClawAuthProfile) -> Option<OAuthTokens> {
    match profile {
        OpenClawAuthProfile::Token { token, expires, .. } => {
            let access = token.as_ref()?.trim();
            if access.is_empty() {
                return None;
            }
            Some(OAuthTokens {
                access_token: Secret::new(access.to_string()),
                refresh_token: None,
                id_token: None,
                account_id: None,
                expires_at: normalize_expiry(*expires),
            })
        },
        OpenClawAuthProfile::Oauth {
            access_token,
            refresh_token,
            expires,
            account_id,
            ..
        } => {
            let access = access_token.as_ref()?.trim();
            if access.is_empty() {
                return None;
            }
            Some(OAuthTokens {
                access_token: Secret::new(access.to_string()),
                refresh_token: refresh_token
                    .as_ref()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| Secret::new(s.to_string())),
                id_token: None,
                account_id: account_id.clone(),
                expires_at: normalize_expiry(*expires),
            })
        },
        _ => None,
    }
}

fn normalize_expiry(expires: Option<u64>) -> Option<u64> {
    let value = expires?;
    if value > 1_000_000_000_000 {
        Some(value / 1000)
    } else {
        Some(value)
    }
}

/// Write imported OAuth tokens to Moltis `oauth_tokens.json`.
pub fn write_oauth_tokens(tokens: &HashMap<String, OAuthTokens>) -> crate::error::Result<()> {
    let store = TokenStore::new();
    write_oauth_tokens_with_store(tokens, &store)
}

/// Write imported OAuth tokens to a specific `oauth_tokens.json` path.
pub fn write_oauth_tokens_to_path(
    tokens: &HashMap<String, OAuthTokens>,
    path: &Path,
) -> crate::error::Result<()> {
    let store = TokenStore::with_path(path.to_path_buf());
    write_oauth_tokens_with_store(tokens, &store)
}

fn write_oauth_tokens_with_store(
    tokens: &HashMap<String, OAuthTokens>,
    store: &TokenStore,
) -> crate::error::Result<()> {
    for (provider, token_set) in tokens {
        store.save(provider, token_set).map_err(|e| {
            crate::error::Error::message(format!(
                "failed to write oauth token for '{provider}': {e}"
            ))
        })?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use {
        super::*,
        crate::detect::OpenClawDetection,
        secrecy::{ExposeSecret, Secret},
    };

    #[test]
    fn parse_model_ref_valid() {
        let (p, m) = parse_model_ref("anthropic/claude-opus-4-6").unwrap();
        assert_eq!(p, "anthropic");
        assert_eq!(m, "claude-opus-4-6");
    }

    #[test]
    fn parse_model_ref_google_maps_to_gemini() {
        let (p, m) = parse_model_ref("google/gemini-2.5-pro").unwrap();
        assert_eq!(p, "gemini");
        assert_eq!(m, "gemini-2.5-pro");
    }

    #[test]
    fn parse_model_ref_invalid() {
        assert!(parse_model_ref("no-slash").is_none());
        assert!(parse_model_ref("/model").is_none());
        assert!(parse_model_ref("provider/").is_none());
    }

    #[test]
    fn map_provider_names() {
        assert_eq!(map_provider_name("anthropic"), "anthropic");
        assert_eq!(map_provider_name("google"), "gemini");
        assert_eq!(map_provider_name("Google"), "gemini");
        assert_eq!(map_provider_name("openai"), "openai");
        assert_eq!(map_provider_name("unknown-provider"), "unknown-provider");
    }

    #[test]
    fn import_with_auth_profiles() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        // Config
        std::fs::write(
            home.join("openclaw.json"),
            r#"{"agents":{"defaults":{"model":{"primary":"anthropic/claude-opus-4-6","fallbacks":["openai/gpt-4o"]}}}}"#,
        )
        .unwrap();

        // Auth profiles
        let agent_dir = home.join("agents").join("main").join("agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("auth-profiles.json"),
            r#"{"version":1,"profiles":{"anthropic-main":{"type":"api_key","provider":"anthropic","key":"sk-ant-test"}}}"#,
        )
        .unwrap();

        let detection = OpenClawDetection {
            home_dir: home.to_path_buf(),
            has_config: true,
            has_credentials: true,
            has_mcp_servers: false,
            workspace_dir: home.join("workspace"),
            has_memory: false,
            has_skills: false,
            agent_ids: vec!["main".to_string()],
            session_count: 0,
            unsupported_channels: Vec::new(),
            has_workspace_files: false,
            workspace_files_found: Vec::new(),
        };

        let (report, result) = import_providers(&detection);
        assert_eq!(report.status, ImportStatus::Success);
        assert!(result.providers.contains_key("anthropic"));
        assert_eq!(
            result.providers["anthropic"].api_key.as_deref(),
            Some("sk-ant-test")
        );
        assert_eq!(result.primary_provider.as_deref(), Some("anthropic"));
        assert_eq!(result.primary_model.as_deref(), Some("claude-opus-4-6"));
        assert_eq!(result.fallback_models.len(), 1);
    }

    #[test]
    fn write_provider_keys_creates_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("provider_keys.json");

        let mut providers = HashMap::new();
        providers.insert("anthropic".to_string(), MoltisProviderConfig {
            api_key: Some("sk-test".to_string()),
            models: vec!["claude-opus-4-6".to_string()],
            ..Default::default()
        });

        write_provider_keys(&providers, &dest).unwrap();
        assert!(dest.is_file());

        let content = std::fs::read_to_string(&dest).unwrap();
        let loaded: HashMap<String, MoltisProviderConfig> = serde_json::from_str(&content).unwrap();
        assert_eq!(loaded["anthropic"].api_key.as_deref(), Some("sk-test"));
    }

    #[test]
    fn write_provider_keys_merges_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("provider_keys.json");

        // Write initial
        std::fs::write(
            &dest,
            r#"{"openai":{"apiKey":"sk-existing","models":["gpt-4o"]}}"#,
        )
        .unwrap();

        let mut providers = HashMap::new();
        providers.insert("anthropic".to_string(), MoltisProviderConfig {
            api_key: Some("sk-new".to_string()),
            ..Default::default()
        });

        write_provider_keys(&providers, &dest).unwrap();

        let content = std::fs::read_to_string(&dest).unwrap();
        let loaded: HashMap<String, MoltisProviderConfig> = serde_json::from_str(&content).unwrap();
        assert!(loaded.contains_key("openai"));
        assert!(loaded.contains_key("anthropic"));
    }

    #[test]
    fn import_oauth_profile_alias_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        let agent_dir = home.join("agents").join("main").join("agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("auth-profiles.json"),
            r#"{
                "version": 1,
                "profiles": {
                    "codex-main": {
                        "type": "oauth",
                        "provider": "openai-codex",
                        "access": "at-123",
                        "refresh": "rt-456",
                        "expires": 1770231225962,
                        "accountId": "acct-1"
                    }
                }
            }"#,
        )
        .unwrap();

        let detection = OpenClawDetection {
            home_dir: home.to_path_buf(),
            has_config: false,
            has_credentials: true,
            has_mcp_servers: false,
            workspace_dir: home.join("workspace"),
            has_memory: false,
            has_skills: false,
            agent_ids: vec!["main".to_string()],
            session_count: 0,
            unsupported_channels: Vec::new(),
            has_workspace_files: false,
            workspace_files_found: Vec::new(),
        };

        let (report, result) = import_providers(&detection);
        assert_eq!(report.status, ImportStatus::Success);
        assert_eq!(report.items_imported, 1);

        let tokens = result
            .oauth_tokens
            .get("openai-codex")
            .expect("oauth tokens should be imported");
        assert_eq!(tokens.access_token.expose_secret(), "at-123");
        assert_eq!(
            tokens
                .refresh_token
                .as_ref()
                .map(|token| token.expose_secret().as_str()),
            Some("rt-456")
        );
        // OpenClaw stores milliseconds; Moltis token store expects seconds.
        assert_eq!(tokens.expires_at, Some(1_770_231_225));
        assert_eq!(tokens.account_id.as_deref(), Some("acct-1"));
    }

    #[test]
    fn write_oauth_tokens_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TokenStore::with_path(tmp.path().join("oauth_tokens.json"));

        let mut tokens = HashMap::new();
        tokens.insert("openai-codex".to_string(), OAuthTokens {
            access_token: Secret::new("at-123".to_string()),
            refresh_token: Some(Secret::new("rt-456".to_string())),
            id_token: None,
            account_id: Some("acct-1".to_string()),
            expires_at: Some(1_770_231_225),
        });

        write_oauth_tokens_with_store(&tokens, &store).unwrap();
        let loaded = store
            .load("openai-codex")
            .expect("token store should contain openai-codex");
        assert_eq!(loaded.access_token.expose_secret(), "at-123");
        assert_eq!(
            loaded
                .refresh_token
                .as_ref()
                .map(|token| token.expose_secret().as_str()),
            Some("rt-456")
        );
        assert_eq!(loaded.account_id.as_deref(), Some("acct-1"));
        assert_eq!(loaded.expires_at, Some(1_770_231_225));
    }
}
