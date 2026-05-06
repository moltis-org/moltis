//! OAuth helpers: redirect URI normalization, Codex CLI token parsing,
//! token import, provider-specific headers, and verification URI building.

use std::path::{Path, PathBuf};

use {
    secrecy::Secret,
    serde_json::Value,
    tracing::{debug, info},
};

use moltis_oauth::{TokenStore, normalize_loopback_redirect};

use crate::config_helpers::home_token_store;

// ── Redirect URI normalization ─────────────────────────────────────────────

/// Normalize the `redirect_uri` on a provider `OAuthConfig` loaded via
/// `load_oauth_config` so that loopback values always use the `http`
/// scheme, per RFC 8252 S7.3/S8.3.
///
/// Built-in defaults (e.g. `openai-codex`) already use
/// `http://localhost:1455/auth/callback`, but `load_oauth_config` also
/// reads from `~/.config/moltis/oauth_providers.json` and
/// `MOLTIS_OAUTH_{PROVIDER}_REDIRECT_URI`, either of which could
/// accidentally specify `https://localhost`. Without normalization:
///
/// * `callback_port` would parse the port from the HTTPS form and the
///   spawned `CallbackServer` would try to bind on Moltis's main TLS
///   port, which is already in use by the gateway.
/// * Strict authorization servers would reject the authorization
///   request with `invalid_redirect_uri`.
///
/// No-op for empty or non-loopback URIs.
pub(crate) fn normalize_loaded_redirect_uri(config: &mut moltis_oauth::OAuthConfig) {
    if config.redirect_uri.is_empty() {
        return;
    }
    config.redirect_uri = normalize_loopback_redirect(&config.redirect_uri);
}

// ── Codex CLI auth helpers ─────────────────────────────────────────────────

pub(crate) fn codex_cli_auth_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".codex").join("auth.json"))
}

pub(crate) fn codex_cli_auth_has_access_token(path: &Path) -> bool {
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

// ── Token import ───────────────────────────────────────────────────────────

/// Import auto-detected external OAuth tokens into the token store so all
/// providers read from a single location. Currently handles Codex CLI
/// `~/.codex/auth.json` -> `openai-codex` in the token store.
pub fn import_detected_oauth_tokens(
    detected: &[crate::AutoDetectedProviderSource],
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

// ── Provider-specific OAuth helpers ────────────────────────────────────────

/// Build provider-specific extra headers for device-flow OAuth calls.
pub(crate) fn build_provider_headers(provider: &str) -> Option<reqwest::header::HeaderMap> {
    match provider {
        "kimi-code" => Some(moltis_oauth::kimi_headers()),
        _ => None,
    }
}

/// Some providers require visiting a URL that already embeds the user_code.
/// Prefer provider-returned `verification_uri_complete`; otherwise synthesize
/// one for known providers.
pub(crate) fn build_verification_uri_complete(
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

// ── Token presence check ───────────────────────────────────────────────────

pub(crate) fn has_oauth_tokens_for_provider(
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

/// Convenience wrapper used by `LiveProviderSetupService`.
pub(crate) fn has_oauth_tokens(provider_name: &str, token_store: &TokenStore) -> bool {
    has_oauth_tokens_for_provider(
        provider_name,
        token_store,
        home_token_store().as_ref().map(|(store, _)| store),
    )
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, moltis_oauth::OAuthTokens};

    fn make_oauth_config(redirect_uri: &str) -> moltis_oauth::OAuthConfig {
        moltis_oauth::OAuthConfig {
            client_id: "client".into(),
            auth_url: "https://example.com/authorize".into(),
            token_url: "https://example.com/token".into(),
            redirect_uri: redirect_uri.into(),
            resource: None,
            scopes: Vec::new(),
            extra_auth_params: Vec::new(),
            device_flow: false,
        }
    }

    #[test]
    fn normalize_loaded_redirect_uri_rewrites_https_localhost() {
        let mut config = make_oauth_config("https://localhost:1455/auth/callback");
        normalize_loaded_redirect_uri(&mut config);
        assert_eq!(config.redirect_uri, "http://localhost:1455/auth/callback");
    }

    #[test]
    fn normalize_loaded_redirect_uri_rewrites_https_ipv4_loopback() {
        let mut config = make_oauth_config("https://127.0.0.1:1455/auth/callback");
        normalize_loaded_redirect_uri(&mut config);
        assert_eq!(config.redirect_uri, "http://127.0.0.1:1455/auth/callback");
    }

    #[test]
    fn normalize_loaded_redirect_uri_rewrites_https_ipv6_loopback() {
        let mut config = make_oauth_config("https://[::1]:1455/auth/callback");
        normalize_loaded_redirect_uri(&mut config);
        assert_eq!(config.redirect_uri, "http://[::1]:1455/auth/callback");
    }

    #[test]
    fn normalize_loaded_redirect_uri_preserves_http_scheme() {
        let mut config = make_oauth_config("http://localhost:1455/auth/callback");
        normalize_loaded_redirect_uri(&mut config);
        assert_eq!(config.redirect_uri, "http://localhost:1455/auth/callback");
    }

    #[test]
    fn normalize_loaded_redirect_uri_preserves_real_hostname() {
        let mut config = make_oauth_config("https://moltis.lan/auth/callback");
        normalize_loaded_redirect_uri(&mut config);
        assert_eq!(config.redirect_uri, "https://moltis.lan/auth/callback");
    }

    #[test]
    fn normalize_loaded_redirect_uri_no_op_on_empty_string() {
        let mut config = make_oauth_config("");
        normalize_loaded_redirect_uri(&mut config);
        assert_eq!(config.redirect_uri, "");
    }

    /// Regression guard for the full integration path.
    #[test]
    fn loaded_openai_codex_redirect_parses_to_http_loopback() {
        use moltis_oauth::load_oauth_config;
        let mut config = load_oauth_config("openai-codex").expect("openai-codex should exist");
        normalize_loaded_redirect_uri(&mut config);
        let parsed = url::Url::parse(&config.redirect_uri).expect("parsable URL");
        assert_eq!(parsed.scheme(), "http");
        assert_eq!(parsed.host_str(), Some("localhost"));
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
}
