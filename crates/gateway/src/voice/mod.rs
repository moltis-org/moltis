//! Voice service implementations for TTS and STT.
//!
//! This module provides concrete implementations of the `TtsService` and
//! `SttService` traits using the moltis-voice crate's providers.

#[cfg(feature = "voice")]
mod stt_service;
#[cfg(feature = "voice")]
mod tts_service;

#[cfg(feature = "voice")]
pub use stt_service::{LiveSttService, SttServiceConfig};
#[cfg(feature = "voice")]
pub use tts_service::LiveTtsService;

// `SttService` trait and `NoopSttService` are defined in `moltis-service-traits`
// and re-exported via `crate::services::*`.
pub use crate::services::{NoopSttService, SttService};

#[cfg(feature = "voice")]
use secrecy::Secret;

// TTS/STT provider IDs are defined once in moltis-config (VoiceTtsProvider,
// VoiceSttProvider) and re-exported through moltis-voice as TtsProviderId /
// SttProviderId. Same type everywhere — no conversion needed.

/// Load config with voice API keys merged from the credential store.
///
/// Voice API keys are stored in the [`KeyStore`] (not `moltis.toml`) so they
/// benefit from vault encryption when enabled.  This function loads the TOML
/// config and overlays any voice-specific keys found in the store, giving the
/// store priority over legacy TOML values.
#[cfg(feature = "voice")]
pub(crate) fn load_voice_config() -> moltis_config::MoltisConfig {
    let mut cfg = moltis_config::discover_and_load();
    merge_voice_keys(&mut cfg);
    cfg
}

/// Overlay voice API keys from [`KeyStore`] onto the given config.
///
/// Keys in the store take precedence over those in the TOML config.
/// Shared keys (ElevenLabs, Google) are applied to both TTS and STT sections.
#[cfg(feature = "voice")]
pub(crate) fn merge_voice_keys(cfg: &mut moltis_config::MoltisConfig) {
    let store = crate::provider_setup::KeyStore::new();

    // ElevenLabs (shared TTS + STT)
    if let Some(key) = store.load("voice-elevenlabs") {
        let secret = Secret::new(key);
        cfg.voice.tts.elevenlabs.api_key = Some(secret.clone());
        cfg.voice.stt.elevenlabs.api_key = Some(secret);
    }

    // Google (shared TTS + STT)
    if let Some(key) = store.load("voice-google") {
        let secret = Secret::new(key);
        cfg.voice.tts.google.api_key = Some(secret.clone());
        cfg.voice.stt.google.api_key = Some(secret);
    }

    // OpenAI TTS (voice-specific, separate from LLM provider key)
    if let Some(key) = store.load("voice-openai") {
        cfg.voice.tts.openai.api_key = Some(Secret::new(key.clone()));
        // Also set STT whisper key since they share the same OpenAI API
        if cfg.voice.stt.whisper.api_key.is_none() {
            cfg.voice.stt.whisper.api_key = Some(Secret::new(key));
        }
    }

    // Whisper STT (voice-specific OpenAI key for STT only)
    if let Some(key) = store.load("voice-whisper") {
        cfg.voice.stt.whisper.api_key = Some(Secret::new(key));
    }

    // Groq STT
    if let Some(key) = store.load("voice-groq") {
        cfg.voice.stt.groq.api_key = Some(Secret::new(key));
    }

    // Deepgram STT
    if let Some(key) = store.load("voice-deepgram") {
        cfg.voice.stt.deepgram.api_key = Some(Secret::new(key));
    }

    // Mistral STT
    if let Some(key) = store.load("voice-mistral") {
        cfg.voice.stt.mistral.api_key = Some(Secret::new(key));
    }
}

/// Map a UI provider name to its credential-store key name.
///
/// Shared providers (e.g. ElevenLabs TTS + STT) map to a single key so the
/// secret is stored once.
#[cfg(feature = "voice")]
pub(crate) fn voice_key_store_name(provider: &str) -> String {
    match provider {
        "elevenlabs" | "elevenlabs-stt" => "voice-elevenlabs".to_string(),
        "openai" | "openai-tts" => "voice-openai".to_string(),
        "google" | "google-tts" => "voice-google".to_string(),
        "whisper" => "voice-whisper".to_string(),
        "groq" => "voice-groq".to_string(),
        "deepgram" => "voice-deepgram".to_string(),
        "mistral" => "voice-mistral".to_string(),
        other => format!("voice-{other}"),
    }
}

/// One-time migration: move voice API keys from `moltis.toml` into the
/// [`KeyStore`] and clear them from the config file.
///
/// Called once at gateway startup.  If a voice key already exists in the
/// store the TOML value is ignored (store wins).  After migration the
/// TOML file no longer contains voice secrets.
#[cfg(feature = "voice")]
pub(crate) fn migrate_voice_keys_to_key_store(config: &moltis_config::MoltisConfig) {
    use secrecy::ExposeSecret;

    let store = crate::provider_setup::KeyStore::new();

    // (store_key, tts_key, stt_key) — for shared providers both may be Some.
    let candidates: Vec<(&str, Option<&Secret<String>>, Option<&Secret<String>>)> = vec![
        (
            "voice-elevenlabs",
            config.voice.tts.elevenlabs.api_key.as_ref(),
            config.voice.stt.elevenlabs.api_key.as_ref(),
        ),
        (
            "voice-openai",
            config.voice.tts.openai.api_key.as_ref(),
            None,
        ),
        (
            "voice-google",
            config.voice.tts.google.api_key.as_ref(),
            config.voice.stt.google.api_key.as_ref(),
        ),
        (
            "voice-whisper",
            None,
            config.voice.stt.whisper.api_key.as_ref(),
        ),
        ("voice-groq", None, config.voice.stt.groq.api_key.as_ref()),
        (
            "voice-deepgram",
            None,
            config.voice.stt.deepgram.api_key.as_ref(),
        ),
        (
            "voice-mistral",
            None,
            config.voice.stt.mistral.api_key.as_ref(),
        ),
    ];

    let mut migrated = Vec::new();
    for (store_key, tts_key, stt_key) in &candidates {
        // Skip if the store already has this key.
        if store.load(store_key).is_some() {
            continue;
        }
        // Pick whichever TOML key is present (TTS first).
        let value = tts_key.or(*stt_key);
        if let Some(secret) = value {
            let plaintext = secret.expose_secret();
            if !plaintext.is_empty() && !plaintext.starts_with('$') {
                if let Err(e) =
                    store.save_config(store_key, Some(plaintext.to_string()), None, None)
                {
                    tracing::warn!(key = store_key, error = %e, "failed to migrate voice key");
                    continue;
                }
                migrated.push(*store_key);
            }
        }
    }

    if migrated.is_empty() {
        return;
    }

    // Clear the TOML entries so secrets don't linger in the config file.
    if let Err(e) = moltis_config::update_config(|cfg| {
        for key in &migrated {
            match *key {
                "voice-elevenlabs" => {
                    cfg.voice.tts.elevenlabs.api_key = None;
                    cfg.voice.stt.elevenlabs.api_key = None;
                },
                "voice-openai" => {
                    cfg.voice.tts.openai.api_key = None;
                },
                "voice-google" => {
                    cfg.voice.tts.google.api_key = None;
                    cfg.voice.stt.google.api_key = None;
                },
                "voice-whisper" => {
                    cfg.voice.stt.whisper.api_key = None;
                },
                "voice-groq" => {
                    cfg.voice.stt.groq.api_key = None;
                },
                "voice-deepgram" => {
                    cfg.voice.stt.deepgram.api_key = None;
                },
                "voice-mistral" => {
                    cfg.voice.stt.mistral.api_key = None;
                },
                _ => {},
            }
        }
    }) {
        tracing::warn!(error = %e, "failed to clear migrated voice keys from config");
    } else {
        tracing::info!(
            count = migrated.len(),
            keys = ?migrated,
            "migrated voice API keys from moltis.toml to credential store"
        );
    }
}

/// Resolve an OpenAI API key with fallback: voice-specific config -> `OPENAI_API_KEY`
/// env var -> LLM provider config (`providers.openai.api_key`).
#[cfg(feature = "voice")]
pub(crate) fn resolve_openai_key(
    voice_key: Option<&Secret<String>>,
    cfg: &moltis_config::MoltisConfig,
) -> Option<Secret<String>> {
    voice_key
        .cloned()
        .or_else(|| std::env::var("OPENAI_API_KEY").ok().map(Secret::new))
        .or_else(|| cfg.providers.get("openai").and_then(|p| p.api_key.clone()))
}

#[cfg(feature = "voice")]
pub(crate) fn resolve_openai_provider_base_url(
    cfg: &moltis_config::MoltisConfig,
) -> Option<String> {
    cfg.providers.get("openai").and_then(|p| p.base_url.clone())
}

#[cfg(feature = "voice")]
pub(crate) fn resolve_openai_tts_base_url(cfg: &moltis_config::MoltisConfig) -> Option<String> {
    cfg.voice
        .tts
        .openai
        .base_url
        .clone()
        .or_else(|| resolve_openai_provider_base_url(cfg))
}

#[cfg(feature = "voice")]
pub(crate) fn resolve_openai_whisper_base_url(cfg: &moltis_config::MoltisConfig) -> Option<String> {
    cfg.voice
        .stt
        .whisper
        .base_url
        .clone()
        .or_else(|| resolve_openai_provider_base_url(cfg))
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(all(test, feature = "voice"))]
mod tests {
    use {super::*, secrecy::ExposeSecret, tempfile::TempDir};

    struct VoiceConfigTestGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        _config_dir: TempDir,
        _data_dir: TempDir,
    }

    impl VoiceConfigTestGuard {
        fn with_config(config_toml: &str) -> Self {
            let lock = crate::config_override_test_lock();
            let config_dir = tempfile::tempdir()
                .unwrap_or_else(|error| panic!("config tempdir should be created: {error}"));
            let data_dir = tempfile::tempdir()
                .unwrap_or_else(|error| panic!("data tempdir should be created: {error}"));
            std::fs::write(config_dir.path().join("moltis.toml"), config_toml)
                .unwrap_or_else(|error| panic!("config should be written: {error}"));
            moltis_config::set_config_dir(config_dir.path().to_path_buf());
            moltis_config::set_data_dir(data_dir.path().to_path_buf());
            Self {
                _lock: lock,
                _config_dir: config_dir,
                _data_dir: data_dir,
            }
        }
    }

    impl Drop for VoiceConfigTestGuard {
        fn drop(&mut self) {
            moltis_config::clear_config_dir();
            moltis_config::clear_data_dir();
        }
    }

    #[test]
    fn test_resolve_openai_key_prefers_voice_key_over_llm_provider_key() {
        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.providers.providers.insert(
            "openai".to_string(),
            moltis_config::schema::ProviderEntry {
                api_key: Some(Secret::new("llm-openai-key".to_string())),
                ..moltis_config::schema::ProviderEntry::default()
            },
        );

        let resolved = resolve_openai_key(Some(&Secret::new("voice-openai-key".to_string())), &cfg)
            .map(|value| value.expose_secret().to_string());
        assert_eq!(resolved.as_deref(), Some("voice-openai-key"));
    }

    #[test]
    fn test_resolve_openai_key_uses_llm_provider_key_when_voice_key_missing() {
        if std::env::var("OPENAI_API_KEY").is_ok() {
            return;
        }

        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.providers.providers.insert(
            "openai".to_string(),
            moltis_config::schema::ProviderEntry {
                api_key: Some(Secret::new("llm-openai-key".to_string())),
                ..moltis_config::schema::ProviderEntry::default()
            },
        );

        let resolved =
            resolve_openai_key(None, &cfg).map(|value| value.expose_secret().to_string());
        assert_eq!(resolved.as_deref(), Some("llm-openai-key"));
    }

    #[test]
    fn test_resolve_openai_tts_base_url_prefers_voice_specific_value() {
        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.voice.tts.openai.base_url = Some("http://127.0.0.1:8003".to_string());
        cfg.providers.providers.insert(
            "openai".to_string(),
            moltis_config::schema::ProviderEntry {
                base_url: Some("http://127.0.0.1:8001".to_string()),
                ..moltis_config::schema::ProviderEntry::default()
            },
        );

        assert_eq!(
            resolve_openai_tts_base_url(&cfg).as_deref(),
            Some("http://127.0.0.1:8003")
        );
    }

    #[test]
    fn test_resolve_openai_tts_base_url_falls_back_to_provider_value() {
        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.providers.providers.insert(
            "openai".to_string(),
            moltis_config::schema::ProviderEntry {
                base_url: Some("http://127.0.0.1:8001".to_string()),
                ..moltis_config::schema::ProviderEntry::default()
            },
        );

        assert_eq!(
            resolve_openai_tts_base_url(&cfg).as_deref(),
            Some("http://127.0.0.1:8001")
        );
    }

    #[test]
    fn test_resolve_openai_whisper_base_url_prefers_voice_specific_value() {
        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.voice.stt.whisper.base_url = Some("http://127.0.0.1:8002".to_string());
        cfg.providers.providers.insert(
            "openai".to_string(),
            moltis_config::schema::ProviderEntry {
                base_url: Some("http://127.0.0.1:8001".to_string()),
                ..moltis_config::schema::ProviderEntry::default()
            },
        );

        assert_eq!(
            resolve_openai_whisper_base_url(&cfg).as_deref(),
            Some("http://127.0.0.1:8002")
        );
    }

    #[test]
    fn test_resolve_openai_whisper_base_url_falls_back_to_provider_value() {
        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.providers.providers.insert(
            "openai".to_string(),
            moltis_config::schema::ProviderEntry {
                base_url: Some("http://127.0.0.1:8001".to_string()),
                ..moltis_config::schema::ProviderEntry::default()
            },
        );

        assert_eq!(
            resolve_openai_whisper_base_url(&cfg).as_deref(),
            Some("http://127.0.0.1:8001")
        );
    }

    #[test]
    fn voice_key_store_name_maps_shared_providers() {
        assert_eq!(voice_key_store_name("elevenlabs"), "voice-elevenlabs");
        assert_eq!(voice_key_store_name("elevenlabs-stt"), "voice-elevenlabs");
        assert_eq!(voice_key_store_name("openai"), "voice-openai");
        assert_eq!(voice_key_store_name("openai-tts"), "voice-openai");
        assert_eq!(voice_key_store_name("google"), "voice-google");
        assert_eq!(voice_key_store_name("google-tts"), "voice-google");
        assert_eq!(voice_key_store_name("whisper"), "voice-whisper");
        assert_eq!(voice_key_store_name("groq"), "voice-groq");
        assert_eq!(voice_key_store_name("deepgram"), "voice-deepgram");
        assert_eq!(voice_key_store_name("mistral"), "voice-mistral");
        assert_eq!(voice_key_store_name("custom"), "voice-custom");
    }

    #[test]
    fn merge_voice_keys_populates_config_from_key_store() {
        let guard = VoiceConfigTestGuard::with_config("");

        // Save a key to the store via the public save_config method.
        let store = crate::provider_setup::KeyStore::new();
        store
            .save_config("voice-elevenlabs", Some("el-test-key".into()), None, None)
            .unwrap();

        let mut cfg = moltis_config::MoltisConfig::default();
        assert!(cfg.voice.tts.elevenlabs.api_key.is_none());

        merge_voice_keys(&mut cfg);

        assert!(cfg.voice.tts.elevenlabs.api_key.is_some());
        assert_eq!(
            cfg.voice.tts.elevenlabs.api_key.unwrap().expose_secret(),
            "el-test-key"
        );

        drop(guard);
    }

    #[test]
    fn migrate_voice_keys_moves_config_keys_to_key_store() {
        let guard = VoiceConfigTestGuard::with_config("");

        // Build a config with voice keys as if they came from TOML.
        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.voice.tts.elevenlabs.api_key = Some(Secret::new("el-legacy-key".to_string()));
        cfg.voice.stt.groq.api_key = Some(Secret::new("groq-legacy-key".to_string()));

        migrate_voice_keys_to_key_store(&cfg);

        // Keys should now be in the store.
        let store = crate::provider_setup::KeyStore::new();
        assert_eq!(
            store.load("voice-elevenlabs").as_deref(),
            Some("el-legacy-key")
        );
        assert_eq!(store.load("voice-groq").as_deref(), Some("groq-legacy-key"));

        // Running again with empty config is a no-op (keys already in store).
        let cfg2 = moltis_config::MoltisConfig::default();
        migrate_voice_keys_to_key_store(&cfg2);
        assert_eq!(
            store.load("voice-elevenlabs").as_deref(),
            Some("el-legacy-key")
        );

        drop(guard);
    }

    #[test]
    fn migrate_voice_keys_skips_env_var_references() {
        let guard = VoiceConfigTestGuard::with_config("");

        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.voice.tts.elevenlabs.api_key = Some(Secret::new("${ELEVENLABS_API_KEY}".to_string()));

        migrate_voice_keys_to_key_store(&cfg);

        // Env var reference should NOT be migrated.
        let store = crate::provider_setup::KeyStore::new();
        assert!(store.load("voice-elevenlabs").is_none());

        drop(guard);
    }
}
