use std::collections::HashMap;

use secrecy::Secret;

const PROVIDER_ENV_CANDIDATES: &[&str] = &["MOLTIS_PROVIDER", "PROVIDER"];
const API_KEY_ENV_CANDIDATES: &[&str] = &["MOLTIS_API_KEY", "API_KEY"];

#[derive(Clone)]
pub struct GenericProviderEnv {
    pub provider: String,
    pub provider_var: &'static str,
    pub api_key: Secret<String>,
    pub api_key_var: &'static str,
}

pub fn env_value_with_overrides(
    env_overrides: &HashMap<String, String>,
    key: &str,
) -> Option<String> {
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

pub fn generic_provider_env(env_overrides: &HashMap<String, String>) -> Option<GenericProviderEnv> {
    let (provider_var, provider_raw) = PROVIDER_ENV_CANDIDATES
        .iter()
        .find_map(|key| env_value_with_overrides(env_overrides, key).map(|value| (*key, value)))?;
    let (api_key_var, api_key) = API_KEY_ENV_CANDIDATES
        .iter()
        .find_map(|key| env_value_with_overrides(env_overrides, key).map(|value| (*key, value)))?;

    Some(GenericProviderEnv {
        provider: normalize_provider_name(&provider_raw)?,
        provider_var,
        api_key: Secret::new(api_key),
        api_key_var,
    })
}

pub fn generic_provider_api_key_from_env(
    provider: &str,
    env_overrides: &HashMap<String, String>,
) -> Option<Secret<String>> {
    let normalized_provider = normalize_provider_name(provider)?;
    let generic = generic_provider_env(env_overrides)?;
    (generic.provider == normalized_provider).then_some(generic.api_key)
}

pub fn generic_provider_env_source_for_provider(
    provider: &str,
    env_overrides: &HashMap<String, String>,
) -> Option<String> {
    let normalized_provider = normalize_provider_name(provider)?;
    let generic = generic_provider_env(env_overrides)?;
    (generic.provider == normalized_provider)
        .then(|| format!("env:{}+{}", generic.provider_var, generic.api_key_var))
}

fn normalize_provider_name(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase().replace('_', "-");
    if normalized.is_empty() {
        return None;
    }

    let canonical = match normalized.as_str() {
        "claude" => "anthropic",
        "google" | "google-gemini" => "gemini",
        "grok" => "xai",
        "local" => "local-llm",
        "z-ai" | "z.ai" | "zhipu" | "zhipu-ai" => "zai",
        other => other,
    };

    Some(canonical.to_string())
}

#[cfg(test)]
mod tests {
    use {super::*, secrecy::ExposeSecret};

    #[test]
    fn generic_provider_env_prefers_namespaced_keys() {
        let env_overrides = HashMap::from([
            ("PROVIDER".to_string(), "anthropic".to_string()),
            ("API_KEY".to_string(), "fallback-key".to_string()),
            ("MOLTIS_PROVIDER".to_string(), "openai".to_string()),
            ("MOLTIS_API_KEY".to_string(), "primary-key".to_string()),
        ]);

        let Some(resolved) = generic_provider_env(&env_overrides) else {
            panic!("generic provider env should resolve");
        };
        assert_eq!(resolved.provider, "openai");
        assert_eq!(resolved.provider_var, "MOLTIS_PROVIDER");
        assert_eq!(resolved.api_key.expose_secret(), "primary-key");
        assert_eq!(resolved.api_key_var, "MOLTIS_API_KEY");
    }

    #[test]
    fn generic_provider_env_normalizes_common_aliases() {
        let env_overrides = HashMap::from([
            ("PROVIDER".to_string(), "google".to_string()),
            ("API_KEY".to_string(), "test-key".to_string()),
        ]);

        let Some(resolved) = generic_provider_env(&env_overrides) else {
            panic!("generic provider env should resolve");
        };
        assert_eq!(resolved.provider, "gemini");
    }

    #[test]
    fn generic_provider_api_key_matches_only_selected_provider() {
        let env_overrides = HashMap::from([
            ("MOLTIS_PROVIDER".to_string(), "openai".to_string()),
            ("MOLTIS_API_KEY".to_string(), "sk-test".to_string()),
        ]);

        assert_eq!(
            generic_provider_api_key_from_env("openai", &env_overrides)
                .as_ref()
                .map(ExposeSecret::expose_secret)
                .map(|value| value.as_str()),
            Some("sk-test")
        );
        assert!(generic_provider_api_key_from_env("anthropic", &env_overrides).is_none());
    }

    #[test]
    fn generic_provider_source_reports_actual_env_keys() {
        let env_overrides = HashMap::from([
            ("PROVIDER".to_string(), "anthropic".to_string()),
            ("API_KEY".to_string(), "sk-test".to_string()),
        ]);

        assert_eq!(
            generic_provider_env_source_for_provider("anthropic", &env_overrides).as_deref(),
            Some("env:PROVIDER+API_KEY")
        );
    }
}
