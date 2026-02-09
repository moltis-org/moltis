use std::collections::HashMap;

use crate::{config_dir::moltis_config_dir, types::OAuthConfig};

/// Default OAuth configurations for known providers.
fn builtin_defaults() -> HashMap<String, OAuthConfig> {
    let mut m = HashMap::new();
    // GitHub Copilot uses device flow (handled by the provider itself),
    // but we store a config entry so `load_oauth_config` returns Some
    // and the gateway recognises it as an OAuth provider.
    m.insert("github-copilot".into(), OAuthConfig {
        client_id: "Iv1.b507a08c87ecfe98".into(),
        auth_url: "https://github.com/login/device/code".into(),
        token_url: "https://github.com/login/oauth/access_token".into(),
        redirect_uri: String::new(),
        scopes: vec![],
        extra_auth_params: vec![],
        device_flow: true,
    });
    m.insert("kimi-code".into(), OAuthConfig {
        client_id: "17e5f671-d194-4dfb-9706-5516cb48c098".into(),
        auth_url: "https://auth.kimi.com/api/oauth/device_authorization".into(),
        token_url: "https://auth.kimi.com/api/oauth/token".into(),
        redirect_uri: String::new(),
        scopes: vec![],
        extra_auth_params: vec![],
        device_flow: true,
    });
    m.insert("openai-codex".into(), OAuthConfig {
        client_id: "app_EMoamEEZ73f0CkXaXp7hrann".into(),
        auth_url: "https://auth.openai.com/oauth/authorize".into(),
        token_url: "https://auth.openai.com/oauth/token".into(),
        redirect_uri: "http://localhost:1455/auth/callback".into(),
        scopes: vec![
            "openid".into(),
            "profile".into(),
            "email".into(),
            "offline_access".into(),
        ],
        extra_auth_params: vec![
            ("id_token_add_organizations".into(), "true".into()),
            ("codex_cli_simplified_flow".into(), "true".into()),
        ],
        device_flow: false,
    });
    m
}

/// Path to the OAuth providers config file.
fn config_path() -> std::path::PathBuf {
    moltis_config_dir().join("oauth_providers.json")
}

/// Load the OAuth config for a provider.
///
/// Priority:
/// 1. User config file (`~/.config/moltis/oauth_providers.json`)
/// 2. Environment variables (`MOLTIS_OAUTH_{PROVIDER}_CLIENT_ID`, etc.)
/// 3. Built-in defaults
pub fn load_oauth_config(provider: &str) -> Option<OAuthConfig> {
    // Start from builtin defaults
    let mut config = builtin_defaults().remove(provider)?;

    // Override from config file
    if let Ok(data) = std::fs::read_to_string(config_path())
        && let Ok(file_configs) = serde_json::from_str::<HashMap<String, OAuthConfig>>(&data)
        && let Some(file_config) = file_configs.get(provider)
    {
        config = file_config.clone();
    }

    // Override individual fields from env vars
    let env_prefix = format!(
        "MOLTIS_OAUTH_{}_",
        provider.to_uppercase().replace('-', "_")
    );
    if let Ok(v) = std::env::var(format!("{env_prefix}CLIENT_ID")) {
        config.client_id = v;
    }
    if let Ok(v) = std::env::var(format!("{env_prefix}AUTH_URL")) {
        config.auth_url = v;
    }
    if let Ok(v) = std::env::var(format!("{env_prefix}TOKEN_URL")) {
        config.token_url = v;
    }
    if let Ok(v) = std::env::var(format!("{env_prefix}REDIRECT_URI")) {
        config.redirect_uri = v;
    }

    Some(config)
}

/// The callback port for a provider config (parsed from redirect_uri).
pub fn callback_port(config: &OAuthConfig) -> u16 {
    url::Url::parse(&config.redirect_uri)
        .ok()
        .and_then(|u| u.port())
        .unwrap_or(1455)
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_github_copilot_config() {
        let config = load_oauth_config("github-copilot").expect("should have github-copilot");
        assert_eq!(config.client_id, "Iv1.b507a08c87ecfe98");
        assert!(config.device_flow);
        assert!(config.redirect_uri.is_empty());
        assert_eq!(config.auth_url, "https://github.com/login/device/code");
        assert_eq!(
            config.token_url,
            "https://github.com/login/oauth/access_token"
        );
    }

    #[test]
    fn load_openai_codex_config() {
        let config = load_oauth_config("openai-codex").expect("should have openai-codex");
        assert!(!config.device_flow);
        assert!(!config.redirect_uri.is_empty());
    }

    #[test]
    fn load_kimi_code_config() {
        let config = load_oauth_config("kimi-code").expect("should have kimi-code");
        assert_eq!(config.client_id, "17e5f671-d194-4dfb-9706-5516cb48c098");
        assert!(config.device_flow);
        assert!(config.redirect_uri.is_empty());
        assert_eq!(
            config.auth_url,
            "https://auth.kimi.com/api/oauth/device_authorization"
        );
        assert_eq!(config.token_url, "https://auth.kimi.com/api/oauth/token");
    }

    #[test]
    fn load_unknown_provider_returns_none() {
        assert!(load_oauth_config("nonexistent-provider").is_none());
    }

    #[test]
    fn callback_port_empty_redirect_uri() {
        let config = load_oauth_config("github-copilot").unwrap();
        // Empty redirect_uri should return default port
        assert_eq!(callback_port(&config), 1455);
    }

    #[test]
    fn callback_port_with_redirect_uri() {
        let config = load_oauth_config("openai-codex").unwrap();
        assert_eq!(callback_port(&config), 1455);
    }
}
