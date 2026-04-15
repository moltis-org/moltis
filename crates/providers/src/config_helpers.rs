//! Configuration helpers: API key resolution, model list normalization.

use std::collections::{HashMap, HashSet};

use {moltis_config::schema::ProvidersConfig, secrecy::ExposeSecret};

use crate::model_id::configured_model_for_provider;

/// Resolve an env value from overrides or process environment.
pub(crate) fn env_value(env_overrides: &HashMap<String, String>, key: &str) -> Option<String> {
    moltis_config::env_value_with_overrides(env_overrides, key)
}

/// Resolve an API key from config (Secret) or environment variable,
/// keeping the value wrapped in `Secret<String>` to avoid leaking it.
pub(crate) fn resolve_api_key(
    config: &ProvidersConfig,
    provider: &str,
    env_key: &str,
    env_overrides: &HashMap<String, String>,
) -> Option<secrecy::Secret<String>> {
    config
        .get(provider)
        .and_then(|e| e.api_key.clone())
        .or_else(|| env_value(env_overrides, env_key).map(secrecy::Secret::new))
        .or_else(|| moltis_config::generic_provider_api_key_from_env(provider, env_overrides))
        .filter(|s| !s.expose_secret().is_empty())
}

pub(crate) fn configured_models_for_provider(
    config: &ProvidersConfig,
    provider: &str,
) -> Vec<String> {
    let configured = config
        .get(provider)
        .map(|entry| entry.models.clone())
        .unwrap_or_default();

    normalize_unique_models(
        configured
            .into_iter()
            .map(|model| configured_model_for_provider(model.trim()).to_string()),
    )
}

pub(crate) fn normalize_unique_models(models: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut normalized_models = Vec::new();
    let mut seen = HashSet::new();
    for model in models {
        let normalized = model.trim().to_string();
        if normalized.is_empty() || !seen.insert(normalized.clone()) {
            continue;
        }
        normalized_models.push(normalized);
    }
    normalized_models
}

pub(crate) fn should_fetch_models(config: &ProvidersConfig, provider: &str) -> bool {
    config.get(provider).is_none_or(|entry| entry.fetch_models)
}

pub(crate) fn subscription_preference_rank(provider_name: &str) -> usize {
    if matches!(provider_name, "openai-codex" | "github-copilot") {
        0
    } else {
        1
    }
}

#[cfg_attr(
    not(any(feature = "provider-openai-codex", feature = "provider-github-copilot")),
    allow(dead_code)
)]
pub(crate) fn oauth_discovery_enabled(config: &ProvidersConfig, provider_name: &str) -> bool {
    config.get(provider_name).is_none_or(|entry| entry.enabled)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn oauth_discovery_enabled_ignores_offered_allowlist() {
        let config = ProvidersConfig {
            offered: vec!["openai".into()],
            ..ProvidersConfig::default()
        };
        assert!(oauth_discovery_enabled(&config, "openai-codex"));
        assert!(oauth_discovery_enabled(&config, "github-copilot"));
    }

    #[test]
    fn oauth_discovery_enabled_respects_explicit_disable() {
        let mut config = ProvidersConfig {
            offered: vec!["openai".into()],
            ..ProvidersConfig::default()
        };
        config.providers.insert(
            "openai-codex".into(),
            moltis_config::schema::ProviderEntry {
                enabled: false,
                ..Default::default()
            },
        );
        config.providers.insert(
            "github-copilot".into(),
            moltis_config::schema::ProviderEntry {
                enabled: false,
                ..Default::default()
            },
        );
        assert!(!oauth_discovery_enabled(&config, "openai-codex"));
        assert!(!oauth_discovery_enabled(&config, "github-copilot"));
    }
}
