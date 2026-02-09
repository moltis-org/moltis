#![allow(clippy::unwrap_used, clippy::expect_used)]
use {
    moltis_oauth::{OAuthFlow, TokenStore, callback_port, load_oauth_config, pkce::generate_pkce},
    secrecy::{ExposeSecret, Secret},
};

#[test]
fn pkce_generates_valid_challenge() {
    let pkce = generate_pkce();
    // Verifier should be base64url-encoded 32 bytes (43 chars)
    assert_eq!(pkce.verifier.len(), 43);
    // Challenge should be base64url-encoded SHA-256 (43 chars)
    assert_eq!(pkce.challenge.len(), 43);
    // They must be different
    assert_ne!(pkce.verifier, pkce.challenge);
}

#[test]
fn pkce_is_deterministic_sha256() {
    use {
        base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD},
        sha2::{Digest, Sha256},
    };

    let pkce = generate_pkce();
    // Recompute challenge from verifier
    let mut hasher = Sha256::new();
    hasher.update(pkce.verifier.as_bytes());
    let expected = URL_SAFE_NO_PAD.encode(hasher.finalize());
    assert_eq!(pkce.challenge, expected);
}

#[test]
fn load_oauth_config_returns_openai_codex_defaults() {
    let config = load_oauth_config("openai-codex").expect("should have builtin defaults");
    assert_eq!(config.client_id, "app_EMoamEEZ73f0CkXaXp7hrann");
    assert_eq!(config.auth_url, "https://auth.openai.com/oauth/authorize");
    assert_eq!(config.token_url, "https://auth.openai.com/oauth/token");
    assert_eq!(config.redirect_uri, "http://localhost:1455/auth/callback");
    assert!(config.scopes.contains(&"openid".to_string()));
    assert!(config.scopes.contains(&"offline_access".to_string()));
}

#[test]
fn load_oauth_config_returns_none_for_unknown() {
    assert!(load_oauth_config("nonexistent-provider").is_none());
}

#[test]
fn callback_port_parses_from_redirect_uri() {
    let config = load_oauth_config("openai-codex").unwrap();
    assert_eq!(callback_port(&config), 1455);
}

#[test]
fn oauth_flow_start_builds_valid_url() {
    let config = load_oauth_config("openai-codex").unwrap();
    let flow = OAuthFlow::new(config);
    let req = flow.start().unwrap();

    let url = url::Url::parse(&req.url).expect("should be valid URL");
    assert_eq!(url.scheme(), "https");
    assert_eq!(url.host_str(), Some("auth.openai.com"));
    assert_eq!(url.path(), "/oauth/authorize");

    let params: std::collections::HashMap<_, _> = url.query_pairs().collect();
    assert_eq!(
        params.get("response_type").map(|v| v.as_ref()),
        Some("code")
    );
    assert_eq!(
        params.get("client_id").map(|v| v.as_ref()),
        Some("app_EMoamEEZ73f0CkXaXp7hrann")
    );
    assert_eq!(
        params.get("code_challenge_method").map(|v| v.as_ref()),
        Some("S256")
    );
    assert_eq!(
        params.get("id_token_add_organizations").map(|v| v.as_ref()),
        Some("true")
    );
    assert_eq!(
        params.get("codex_cli_simplified_flow").map(|v| v.as_ref()),
        Some("true")
    );
    assert_eq!(params.get("originator").map(|v| v.as_ref()), Some("pi"));
    assert!(params.contains_key("state"));
    assert!(params.contains_key("code_challenge"));
    assert!(params.get("scope").unwrap().contains("openid"));
    assert!(params.get("scope").unwrap().contains("offline_access"));
}

#[test]
fn oauth_flow_start_generates_unique_state() {
    let config = load_oauth_config("openai-codex").unwrap();
    let flow = OAuthFlow::new(config);
    let req1 = flow.start().unwrap();
    let req2 = flow.start().unwrap();
    assert_ne!(req1.state, req2.state);
}

#[test]
fn token_store_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tokens.json");

    let store = TokenStore::with_path(path);
    let tokens = moltis_oauth::OAuthTokens {
        access_token: Secret::new("test-access".into()),
        refresh_token: Some(Secret::new("test-refresh".into())),
        expires_at: Some(9999999999),
    };

    store.save("test-provider", &tokens).unwrap();

    let loaded = store
        .load("test-provider")
        .expect("should load saved tokens");
    assert_eq!(loaded.access_token.expose_secret(), "test-access");
    assert_eq!(
        loaded
            .refresh_token
            .as_ref()
            .map(|s| s.expose_secret().as_str()),
        Some("test-refresh")
    );
    assert_eq!(loaded.expires_at, Some(9999999999));

    let providers = store.list();
    assert_eq!(providers, vec!["test-provider".to_string()]);

    store.delete("test-provider").unwrap();
    assert!(store.load("test-provider").is_none());
}

// NOTE: env var override test is omitted because env vars are process-global
// and would interfere with parallel tests. The override logic is straightforward
// (std::env::var check in defaults.rs) and covered by code review.
