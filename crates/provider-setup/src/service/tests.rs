#![allow(clippy::unwrap_used, clippy::expect_used)]

use {
    super::*,
    crate::{KeyStore, known_providers::AuthType},
    moltis_config::schema::{ProviderEntry, ProvidersConfig},
    moltis_oauth::{OAuthTokens, TokenStore},
    moltis_providers::ProviderRegistry,
    moltis_service_traits::{NoopProviderSetupService, ProviderSetupService},
    std::{collections::HashMap, sync::Arc},
    tokio::sync::RwLock,
};

#[tokio::test]
async fn noop_service_returns_empty() {
    let svc = NoopProviderSetupService;
    let result = svc.available().await.unwrap();
    assert_eq!(result, serde_json::json!([]));
}

#[tokio::test]
async fn remove_key_rejects_unknown_provider() {
    let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
        &ProvidersConfig::default(),
        HashMap::new(),
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
        HashMap::new(),
    )));
    let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
    assert!(svc.remove_key(serde_json::json!({})).await.is_err());
}

#[tokio::test]
async fn disabled_provider_is_not_reported_configured() {
    let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
        &ProvidersConfig::default(),
        HashMap::new(),
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

#[tokio::test]
async fn live_service_lists_providers() {
    let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
        &ProvidersConfig::default(),
        HashMap::new(),
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
async fn available_marks_provider_configured_from_generic_provider_env() {
    let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
        &ProvidersConfig::default(),
        HashMap::new(),
    )));
    let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None)
        .with_env_overrides(HashMap::from([
            ("MOLTIS_PROVIDER".to_string(), "openai".to_string()),
            (
                "MOLTIS_API_KEY".to_string(),
                "sk-test-openai-generic".to_string(),
            ),
        ]));

    let result = svc.available().await.unwrap();
    let arr = result
        .as_array()
        .expect("providers.available should return array");
    let openai = arr
        .iter()
        .find(|provider| provider.get("name").and_then(|v| v.as_str()) == Some("openai"))
        .expect("openai should be present");

    assert_eq!(
        openai.get("configured").and_then(|v| v.as_bool()),
        Some(true)
    );
}

#[tokio::test]
async fn available_hides_unconfigured_providers_not_in_offered_list() {
    let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
        &ProvidersConfig::default(),
        HashMap::new(),
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
        HashMap::new(),
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
async fn available_accepts_offered_provider_aliases() {
    let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
        &ProvidersConfig::default(),
        HashMap::new(),
    )));
    let config = ProvidersConfig {
        offered: vec!["claude".into()],
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

    assert!(
        names.contains(&"anthropic"),
        "anthropic should be visible when offered contains alias 'claude', got: {names:?}"
    );
}

#[tokio::test]
async fn available_hides_configured_provider_outside_offered() {
    let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
        &ProvidersConfig::default(),
        HashMap::new(),
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
        HashMap::new(),
    )));
    let mut svc = LiveProviderSetupService::new(registry, config, None);
    svc.token_store = token_store;
    svc.key_store = key_store;

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
        HashMap::new(),
    )));
    let mut svc = LiveProviderSetupService::new(registry, config, None);
    svc.key_store = key_store;

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
        HashMap::new(),
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
        HashMap::new(),
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
        HashMap::new(),
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
        HashMap::new(),
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
        HashMap::new(),
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
async fn oauth_start_stores_pending_state_for_registered_redirect_provider() {
    let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
        &ProvidersConfig::default(),
        HashMap::new(),
    )));
    let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);

    let result = svc
        .oauth_start(serde_json::json!({
            "provider": "openai-codex",
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
    let state = parsed
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.into_owned())
        .expect("oauth authUrl should include state");

    assert!(
        svc.pending_oauth.read().await.contains_key(&state),
        "pending oauth map should track non-device flow state for manual completion"
    );
}

#[tokio::test]
async fn oauth_complete_accepts_callback_input_parameter() {
    let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
        &ProvidersConfig::default(),
        HashMap::new(),
    )));
    let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);

    let result = svc
        .oauth_complete(serde_json::json!({
            "callback": "http://localhost:1455/auth/callback?code=fake&state=missing",
        }))
        .await;

    let err = result.expect_err("missing state should fail");
    assert!(
        err.to_string().contains("unknown or expired OAuth state"),
        "expected parsed callback to reach pending-state validation, got: {err}"
    );
}

#[tokio::test]
async fn oauth_complete_rejects_provider_mismatch_without_consuming_state() {
    let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
        &ProvidersConfig::default(),
        HashMap::new(),
    )));
    let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);

    let start_result = match svc
        .oauth_start(serde_json::json!({
            "provider": "openai-codex",
        }))
        .await
    {
        Ok(value) => value,
        Err(error) => panic!("oauth start should succeed: {error}"),
    };

    if start_result
        .get("alreadyAuthenticated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return;
    }

    let auth_url = match start_result.get("authUrl").and_then(|v| v.as_str()) {
        Some(value) => value,
        None => panic!("missing authUrl"),
    };
    let parsed = match reqwest::Url::parse(auth_url) {
        Ok(value) => value,
        Err(error) => panic!("authUrl should be valid: {error}"),
    };
    let state = match parsed
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.into_owned())
    {
        Some(value) => value,
        None => panic!("oauth authUrl should include state"),
    };

    let mismatch_result = svc
        .oauth_complete(serde_json::json!({
            "provider": "github-copilot",
            "callback": format!("http://localhost:1455/auth/callback?code=fake&state={state}"),
        }))
        .await;
    let mismatch_error = match mismatch_result {
        Ok(_) => panic!("provider mismatch should fail"),
        Err(error) => error,
    };

    assert!(
        mismatch_error
            .to_string()
            .contains("provider mismatch for OAuth state"),
        "unexpected mismatch error: {mismatch_error}"
    );
    assert!(
        svc.pending_oauth.read().await.contains_key(&state),
        "provider mismatch should not consume pending OAuth state"
    );
}

#[tokio::test]
async fn oauth_status_returns_not_authenticated() {
    let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
        &ProvidersConfig::default(),
        HashMap::new(),
    )));
    let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
    let result = svc
        .oauth_status(serde_json::json!({"provider": "openai-codex"}))
        .await
        .unwrap();
    // Might or might not have tokens depending on environment
    assert!(result.get("authenticated").is_some());
}

#[tokio::test]
async fn save_key_accepts_new_providers() {
    let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
        &ProvidersConfig::default(),
        HashMap::new(),
    )));
    let _svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);

    let providers = known_providers();
    for name in [
        "mistral",
        "openrouter",
        "cerebras",
        "minimax",
        "moonshot",
        "zai",
        "zai-code",
        "kimi-code",
        "venice",
        "ollama",
        "lmstudio",
    ] {
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
        HashMap::new(),
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
        "zai-code",
        "kimi-code",
        "venice",
        "ollama",
        "lmstudio",
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
        HashMap::new(),
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

    assert!(
        !names.contains(&"local-llm"),
        "local-llm should be hidden on cloud: {names:?}"
    );
    assert!(
        !names.contains(&"ollama"),
        "ollama should be hidden on cloud: {names:?}"
    );
    assert!(
        !names.contains(&"lmstudio"),
        "lmstudio should be hidden on cloud: {names:?}"
    );
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
        HashMap::new(),
    )));
    let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
    let result = svc.available().await.unwrap();
    let arr = result.as_array().unwrap();

    let names: Vec<&str> = arr
        .iter()
        .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
        .collect();

    assert!(
        names.contains(&"ollama"),
        "ollama should be present locally: {names:?}"
    );
    assert!(
        names.contains(&"lmstudio"),
        "lmstudio should be present locally: {names:?}"
    );
    assert!(
        names.contains(&"openai"),
        "openai should be present locally: {names:?}"
    );
}

#[tokio::test]
async fn validate_key_rejects_unknown_provider() {
    let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
        &ProvidersConfig::default(),
        HashMap::new(),
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
        HashMap::new(),
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
        HashMap::new(),
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
        HashMap::new(),
    )));
    let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
    let result = svc
        .validate_key(serde_json::json!({"provider": "ollama"}))
        .await;
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
        HashMap::new(),
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
        HashMap::new(),
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
async fn validate_key_ollama_with_model_returns_model_list() {
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
        HashMap::new(),
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

#[tokio::test]
async fn validate_key_custom_provider_without_model_returns_discovered_models() {
    use axum::{Json, Router, routing::get};

    let app = Router::new().route(
        "/models",
        get(|| async {
            Json(serde_json::json!({
                "data": [
                    {"id": "gpt-4o", "object": "model", "created": 1700000000},
                    {"id": "gpt-4o-mini", "object": "model", "created": 1700000001},
                    {"id": "dall-e-3", "object": "model", "created": 1700000002}
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
        HashMap::new(),
    )));
    let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
    let result = svc
        .validate_key(serde_json::json!({
            "provider": "custom-test-server",
            "apiKey": "sk-test",
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
            .any(|m| m.get("id").and_then(|v| v.as_str()) == Some("custom-test-server::gpt-4o"))
    );
    assert!(
        models.iter().any(
            |m| m.get("id").and_then(|v| v.as_str()) == Some("custom-test-server::gpt-4o-mini")
        )
    );
    assert!(
        !models
            .iter()
            .any(|m| m.get("id").and_then(|v| v.as_str()) == Some("custom-test-server::dall-e-3"))
    );
}

#[tokio::test]
async fn validate_key_custom_provider_uses_saved_base_url_when_request_omits_it() {
    use axum::{Json, Router, routing::get};

    let app = Router::new().route(
        "/models",
        get(|| async {
            Json(serde_json::json!({
                "data": [
                    {"id": "gpt-4o-mini", "object": "model", "created": 1700000001}
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
        HashMap::new(),
    )));
    let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
    svc.key_store
        .save_config(
            "custom-test-server",
            Some("sk-saved".into()),
            Some(format!("http://{addr}")),
            None,
        )
        .expect("save custom provider config");

    let result = svc
        .validate_key(serde_json::json!({
            "provider": "custom-test-server",
            "apiKey": "sk-test"
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
        models.iter().any(
            |m| m.get("id").and_then(|v| v.as_str()) == Some("custom-test-server::gpt-4o-mini")
        ),
        "expected discovered model via saved base_url, got: {models:?}"
    );
}

#[tokio::test]
async fn validate_key_custom_provider_discovery_error_returns_invalid() {
    use axum::{Router, http::StatusCode, routing::get};

    let app = Router::new().route(
        "/models",
        get(|| async { StatusCode::INTERNAL_SERVER_ERROR }),
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
        HashMap::new(),
    )));
    let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
    let result = svc
        .validate_key(serde_json::json!({
            "provider": "custom-test-server",
            "apiKey": "sk-test",
            "baseUrl": format!("http://{addr}")
        }))
        .await
        .expect("validate_key should return payload");
    server.abort();

    assert_eq!(result.get("valid").and_then(|v| v.as_bool()), Some(false));
    let error = result.get("error").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        error.contains("Failed to discover models"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn validate_key_custom_provider_returns_discovered_models_without_probing() {
    use {
        axum::{
            Json, Router,
            http::StatusCode,
            routing::{get, post},
        },
        std::sync::atomic::{AtomicBool, Ordering},
    };

    let completions_called = Arc::new(AtomicBool::new(false));
    let cc1 = completions_called.clone();
    let cc2 = completions_called.clone();

    let app = Router::new()
        .route(
            "/models",
            get(|| async {
                Json(serde_json::json!({
                    "data": [
                        {"id": "llama-3.1-70b", "object": "model", "created": 1700000000},
                    ]
                }))
            }),
        )
        .route(
            "/chat/completions",
            post(move || async move {
                cc1.store(true, Ordering::SeqCst);
                StatusCode::INTERNAL_SERVER_ERROR
            }),
        )
        .route(
            "/v1/chat/completions",
            post(move || async move {
                cc2.store(true, Ordering::SeqCst);
                StatusCode::INTERNAL_SERVER_ERROR
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
        HashMap::new(),
    )));
    let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
    let result = svc
        .validate_key(serde_json::json!({
            "provider": "custom-test-server",
            "apiKey": "sk-test",
            "baseUrl": format!("http://{addr}")
        }))
        .await
        .expect("validate_key should return payload");
    server.abort();

    assert_eq!(result.get("valid").and_then(|v| v.as_bool()), Some(true));
    assert!(
        result.get("models").and_then(|v| v.as_array()).is_some(),
        "should return discovered models"
    );
    assert!(
        !completions_called.load(Ordering::SeqCst),
        "chat completions endpoint must NOT be called when model is unset — \
         the discovery path should return models directly (issue #502)"
    );
}

#[tokio::test]
async fn validate_key_custom_provider_connection_refused_returns_error() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("local addr");
    drop(listener);

    let registry = Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
        &ProvidersConfig::default(),
        HashMap::new(),
    )));
    let svc = LiveProviderSetupService::new(registry, ProvidersConfig::default(), None);
    let result = svc
        .validate_key(serde_json::json!({
            "provider": "custom-test-server",
            "apiKey": "sk-test",
            "baseUrl": format!("http://{addr}")
        }))
        .await
        .expect("validate_key should return payload");

    assert_eq!(result.get("valid").and_then(|v| v.as_bool()), Some(false));
    let error = result.get("error").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        error.contains("Failed to discover models"),
        "should report discovery failure, got: {error}"
    );
}
