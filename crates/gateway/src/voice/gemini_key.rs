use secrecy::Secret;

const ENV_VARS: [&str; 2] = ["GEMINI_API_KEY", "GOOGLE_API_KEY"];

/// Resolve the Gemini STT key: voice-specific config (or credential store) ->
/// `GEMINI_API_KEY` / `GOOGLE_API_KEY` -> the Gemini LLM provider key, from
/// `providers.gemini.api_key` or the key saved on the LLM settings page.
pub(crate) fn resolve_gemini_key(cfg: &moltis_config::MoltisConfig) -> Option<Secret<String>> {
    resolve_with(
        cfg,
        |name| std::env::var(name).ok(),
        || crate::provider_setup::KeyStore::new().load("gemini"),
    )
}

/// The Gemini key configured for the LLM provider, if any.
pub(crate) fn gemini_llm_key(cfg: &moltis_config::MoltisConfig) -> Option<Secret<String>> {
    llm_key_with(cfg, || {
        crate::provider_setup::KeyStore::new().load("gemini")
    })
}

fn resolve_with(
    cfg: &moltis_config::MoltisConfig,
    env: impl Fn(&str) -> Option<String>,
    stored_llm_key: impl FnOnce() -> Option<String>,
) -> Option<Secret<String>> {
    cfg.voice
        .stt
        .gemini
        .api_key
        .clone()
        .or_else(|| {
            ENV_VARS
                .into_iter()
                .filter_map(&env)
                .find(|value| !value.trim().is_empty())
                .map(Secret::new)
        })
        .or_else(|| llm_key_with(cfg, stored_llm_key))
}

fn llm_key_with(
    cfg: &moltis_config::MoltisConfig,
    stored_llm_key: impl FnOnce() -> Option<String>,
) -> Option<Secret<String>> {
    cfg.providers
        .get("gemini")
        .and_then(|provider| provider.api_key.clone())
        .or_else(|| {
            stored_llm_key()
                .filter(|value| !value.trim().is_empty())
                .map(Secret::new)
        })
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, secrecy::ExposeSecret};

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn no_store() -> Option<String> {
        None
    }

    fn with_llm_provider_key(cfg: &mut moltis_config::MoltisConfig, key: &str) {
        cfg.providers.providers.insert(
            "gemini".to_string(),
            moltis_config::schema::ProviderEntry {
                api_key: Some(Secret::new(key.to_string())),
                ..moltis_config::schema::ProviderEntry::default()
            },
        );
    }

    fn resolved(
        cfg: &moltis_config::MoltisConfig,
        env: impl Fn(&str) -> Option<String>,
        store: impl FnOnce() -> Option<String>,
    ) -> Option<String> {
        resolve_with(cfg, env, store).map(|key| key.expose_secret().clone())
    }

    #[test]
    fn no_key_anywhere_resolves_to_none() {
        let cfg = moltis_config::MoltisConfig::default();
        assert!(resolved(&cfg, no_env, no_store).is_none());
    }

    #[test]
    fn saved_llm_key_is_the_last_fallback() {
        let mut cfg = moltis_config::MoltisConfig::default();
        let store = || Some("saved-llm-key".to_string());
        assert_eq!(
            resolved(&cfg, no_env, store).as_deref(),
            Some("saved-llm-key")
        );
        assert!(resolved(&cfg, no_env, || Some("  ".to_string())).is_none());

        with_llm_provider_key(&mut cfg, "toml-llm-key");
        assert_eq!(
            resolved(&cfg, no_env, store).as_deref(),
            Some("toml-llm-key")
        );
    }

    #[test]
    fn env_vars_win_over_llm_keys_and_skip_blank_values() {
        let mut cfg = moltis_config::MoltisConfig::default();
        with_llm_provider_key(&mut cfg, "llm-key");
        let store = || Some("saved-llm-key".to_string());

        let google_env = |name: &str| (name == "GOOGLE_API_KEY").then(|| "google-env".to_string());
        assert_eq!(
            resolved(&cfg, google_env, store).as_deref(),
            Some("google-env")
        );

        let blank_gemini_env = |name: &str| match name {
            "GEMINI_API_KEY" => Some(" ".to_string()),
            "GOOGLE_API_KEY" => Some("google-env".to_string()),
            _ => None,
        };
        assert_eq!(
            resolved(&cfg, blank_gemini_env, store).as_deref(),
            Some("google-env")
        );

        let both_env = |name: &str| match name {
            "GEMINI_API_KEY" => Some("gemini-env".to_string()),
            "GOOGLE_API_KEY" => Some("google-env".to_string()),
            _ => None,
        };
        assert_eq!(
            resolved(&cfg, both_env, store).as_deref(),
            Some("gemini-env")
        );
    }

    #[test]
    fn voice_key_wins_over_everything() {
        let mut cfg = moltis_config::MoltisConfig::default();
        with_llm_provider_key(&mut cfg, "llm-key");
        cfg.voice.stt.gemini.api_key = Some(Secret::new("voice-key".to_string()));
        let env = |_: &str| Some("env-key".to_string());
        assert_eq!(
            resolved(&cfg, env, || Some("saved-llm-key".to_string())).as_deref(),
            Some("voice-key")
        );
    }

    #[test]
    fn resolve_reads_the_llm_key_saved_in_the_key_store() {
        let _lock = crate::config_override_test_lock();
        let config_dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        moltis_config::set_config_dir(config_dir.path().to_path_buf());
        moltis_config::set_data_dir(data_dir.path().to_path_buf());

        crate::provider_setup::KeyStore::new()
            .save_config("gemini", Some("saved-llm-key".into()), None, None)
            .unwrap();
        let cfg = moltis_config::MoltisConfig::default();
        let from_store = gemini_llm_key(&cfg).map(|key| key.expose_secret().clone());

        moltis_config::clear_config_dir();
        moltis_config::clear_data_dir();
        assert_eq!(from_store.as_deref(), Some("saved-llm-key"));
    }
}
