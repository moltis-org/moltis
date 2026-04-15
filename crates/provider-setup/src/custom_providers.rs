//! Helpers for custom OpenAI-compatible providers (name derivation,
//! URL normalization, deduplication).

use std::collections::HashMap;

use crate::key_store::ProviderConfig;

pub(crate) const CUSTOM_PROVIDER_PREFIX: &str = "custom-";

pub(crate) fn is_custom_provider(name: &str) -> bool {
    name.starts_with(CUSTOM_PROVIDER_PREFIX)
}

/// Derive a provider name from a URL, e.g. `https://api.together.ai/v1` -> `custom-together-ai`.
pub(crate) fn derive_provider_name_from_url(raw: &str) -> Option<String> {
    let parsed = url::Url::parse(raw).ok()?;
    let host = parsed.host_str()?;
    let stripped = host.strip_prefix("api.").unwrap_or(host);
    let slug = stripped.replace('.', "-");
    Some(format!("{CUSTOM_PROVIDER_PREFIX}{slug}"))
}

/// Return a unique provider name by appending `-2`, `-3`, etc. if the base
/// name is already taken.
pub(crate) fn make_unique_provider_name(
    base: &str,
    existing: &HashMap<String, ProviderConfig>,
) -> String {
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
/// `https://api.together.ai/v1` -> `together.ai`
pub(crate) fn base_url_to_display_name(raw: &str) -> String {
    url::Url::parse(raw)
        .ok()
        .and_then(|u| u.host_str().map(ToOwned::to_owned))
        .map(|host| host.strip_prefix("api.").unwrap_or(&host).to_string())
        .unwrap_or_else(|| raw.to_string())
}

pub(crate) fn normalize_base_url_for_compare(raw: &str) -> String {
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

pub(crate) fn existing_custom_provider_for_base_url(
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

pub(crate) fn validation_provider_name_for_endpoint(
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

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

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
}
