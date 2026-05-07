//! Phone provider detection and configuration helpers.

use {
    moltis_config::schema::MoltisConfig,
    secrecy::{ExposeSecret, Secret},
};

#[cfg(feature = "telephony")]
use moltis_channels::ChannelPlugin as _;

const PHONE_ACCOUNT_ID: &str = "default";
const PHONE_CHANNEL_TYPE: &str = "telephony";

/// Overlay phone provider credentials from the credential store onto config.
///
/// Phone credentials follow the same storage model as voice credentials: the
/// TOML file stores non-secret settings, and `KeyStore` stores secrets.
pub(crate) fn merge_phone_keys(cfg: &mut MoltisConfig) {
    let store = crate::provider_setup::KeyStore::new();

    if let Some(stored) = store.load_config(&phone_key_store_name("twilio")) {
        if let Some(account_sid) = stored.api_key {
            cfg.phone.twilio.account_sid = Some(Secret::new(account_sid));
        }
        if let Some(auth_token) = stored.base_url {
            cfg.phone.twilio.auth_token = Some(Secret::new(auth_token));
        }
    }

    if let Some(stored) = store.load_config(&phone_key_store_name("telnyx")) {
        if let Some(api_key) = stored.api_key {
            cfg.phone.telnyx.api_key = Some(Secret::new(api_key));
        }
        if let Some(connection_id) = stored.base_url {
            cfg.phone.telnyx.connection_id = Some(connection_id);
        }
    }

    if let Some(stored) = store.load_config(&phone_key_store_name("plivo")) {
        if let Some(auth_id) = stored.api_key {
            cfg.phone.plivo.auth_id = Some(auth_id);
        }
        if let Some(auth_token) = stored.base_url {
            cfg.phone.plivo.auth_token = Some(Secret::new(auth_token));
        }
    }
}

/// Build the internal telephony channel account from `[phone]`.
///
/// The telephony plugin is still a channel plugin internally because that is
/// how inbound/outbound agent routing works, but user-facing configuration
/// lives in the dedicated Phone settings section.
pub(crate) fn phone_channel_account(config: &MoltisConfig) -> Option<(String, serde_json::Value)> {
    if !config.phone.enabled {
        return None;
    }

    let provider = if config.phone.provider.trim().is_empty() {
        "twilio"
    } else {
        config.phone.provider.trim()
    };

    let mut account = serde_json::Map::from_iter([
        ("provider".to_string(), serde_json::json!(provider)),
        (
            "inbound_policy".to_string(),
            serde_json::json!(config.phone.inbound_policy),
        ),
        (
            "allowlist".to_string(),
            serde_json::json!(config.phone.allowlist),
        ),
        (
            "max_duration_secs".to_string(),
            serde_json::json!(config.phone.max_duration_secs),
        ),
    ]);

    match provider {
        "twilio" => {
            let account_sid = config.phone.twilio.account_sid.as_ref()?.expose_secret();
            let auth_token = config.phone.twilio.auth_token.as_ref()?.expose_secret();
            account.insert("account_sid".to_string(), serde_json::json!(account_sid));
            account.insert("auth_token".to_string(), serde_json::json!(auth_token));
            account.insert(
                "from_number".to_string(),
                serde_json::json!(config.phone.twilio.from_number.clone().unwrap_or_default()),
            );
            if let Some(url) = config.phone.twilio.webhook_url.as_ref() {
                account.insert("webhook_url".to_string(), serde_json::json!(url));
            }
        },
        "telnyx" => {
            let api_key = config.phone.telnyx.api_key.as_ref()?.expose_secret();
            let connection_id = config.phone.telnyx.connection_id.as_ref()?;
            account.insert("auth_token".to_string(), serde_json::json!(api_key));
            account.insert("account_sid".to_string(), serde_json::json!(connection_id));
            account.insert(
                "from_number".to_string(),
                serde_json::json!(config.phone.telnyx.from_number.clone().unwrap_or_default()),
            );
            if let Some(url) = config.phone.telnyx.webhook_url.as_ref() {
                account.insert("webhook_url".to_string(), serde_json::json!(url));
            }
        },
        "plivo" => {
            let auth_id = config.phone.plivo.auth_id.as_ref()?;
            let auth_token = config.phone.plivo.auth_token.as_ref()?.expose_secret();
            account.insert("account_sid".to_string(), serde_json::json!(auth_id));
            account.insert("auth_token".to_string(), serde_json::json!(auth_token));
            account.insert(
                "from_number".to_string(),
                serde_json::json!(config.phone.plivo.from_number.clone().unwrap_or_default()),
            );
            if let Some(url) = config.phone.plivo.webhook_url.as_ref() {
                account.insert("webhook_url".to_string(), serde_json::json!(url));
            }
        },
        _ => return None,
    }

    Some((
        PHONE_ACCOUNT_ID.to_string(),
        serde_json::Value::Object(account),
    ))
}

/// Detect all available phone providers with their status.
pub(super) fn detect_phone_providers(config: &MoltisConfig) -> serde_json::Value {
    let mut effective_config = config.clone();
    merge_phone_keys(&mut effective_config);
    let mut providers = Vec::new();

    // Twilio
    let twilio_configured = effective_config
        .phone
        .twilio
        .account_sid
        .as_ref()
        .map(|s| !s.expose_secret().is_empty())
        .unwrap_or(false);

    let twilio_enabled = effective_config.phone.enabled
        && (effective_config.phone.provider.is_empty()
            || effective_config.phone.provider == "twilio");

    providers.push(serde_json::json!({
        "id": "twilio",
        "name": "Twilio",
        "type": "telephony",
        "category": "Cloud",
        "description": "Make and receive phone calls via the Twilio API. Largest telephony platform with global reach.",
        "available": twilio_configured,
        "enabled": twilio_enabled,
        "keySource": if twilio_configured { "config" } else { "none" },
        "keyPlaceholder": "AC...",
        "keyUrl": "https://www.twilio.com/console",
        "keyUrlLabel": "Twilio Console",
        "hint": "Requires Account SID, Auth Token, and a phone number",
        "settings": {
            "from_number": effective_config.phone.twilio.from_number.clone().unwrap_or_default(),
            "webhook_url": effective_config.phone.twilio.webhook_url.clone().unwrap_or_default(),
        },
    }));

    // Telnyx
    let telnyx_configured = effective_config
        .phone
        .telnyx
        .api_key
        .as_ref()
        .map(|s| !s.expose_secret().is_empty())
        .unwrap_or(false);

    let telnyx_enabled =
        effective_config.phone.enabled && effective_config.phone.provider == "telnyx";

    providers.push(serde_json::json!({
        "id": "telnyx",
        "name": "Telnyx",
        "type": "telephony",
        "category": "Cloud",
        "description": "Developer-friendly telephony with competitive pricing. Uses Call Control API v2.",
        "available": telnyx_configured,
        "enabled": telnyx_enabled,
        "keySource": if telnyx_configured { "config" } else { "none" },
        "keyPlaceholder": "KEY_...",
        "keyUrl": "https://portal.telnyx.com",
        "keyUrlLabel": "Telnyx Portal",
        "hint": "Requires API Key, Connection ID, and a phone number",
        "settings": {
            "from_number": effective_config.phone.telnyx.from_number.clone().unwrap_or_default(),
            "webhook_url": effective_config.phone.telnyx.webhook_url.clone().unwrap_or_default(),
            "connection_id": effective_config.phone.telnyx.connection_id.clone().unwrap_or_default(),
        },
    }));

    // Plivo
    let plivo_configured = effective_config
        .phone
        .plivo
        .auth_id
        .as_ref()
        .map(|s| !s.is_empty())
        .unwrap_or(false);

    let plivo_enabled =
        effective_config.phone.enabled && effective_config.phone.provider == "plivo";

    providers.push(serde_json::json!({
        "id": "plivo",
        "name": "Plivo",
        "type": "telephony",
        "category": "Cloud",
        "description": "Budget-friendly telephony with strong coverage in Asia. Uses XML-based call control.",
        "available": plivo_configured,
        "enabled": plivo_enabled,
        "keySource": if plivo_configured { "config" } else { "none" },
        "keyPlaceholder": "MA...",
        "keyUrl": "https://console.plivo.com/dashboard/",
        "keyUrlLabel": "Plivo Console",
        "hint": "Requires Auth ID, Auth Token, and a phone number",
        "settings": {
            "from_number": effective_config.phone.plivo.from_number.clone().unwrap_or_default(),
            "webhook_url": effective_config.phone.plivo.webhook_url.clone().unwrap_or_default(),
        },
    }));

    serde_json::json!({ "providers": providers })
}

/// Apply phone provider settings to the config.
pub(super) fn apply_phone_provider_settings(
    cfg: &mut MoltisConfig,
    provider: &str,
    params: &serde_json::Value,
) {
    match provider {
        "twilio" => {
            if let Some(from) = params["from_number"].as_str().filter(|s| !s.is_empty()) {
                cfg.phone.twilio.from_number = Some(from.to_string());
            }
            if let Some(url) = params["webhook_url"].as_str().filter(|s| !s.is_empty()) {
                cfg.phone.twilio.webhook_url = Some(url.to_string());
            }
        },
        "telnyx" => {
            if let Some(from) = params["from_number"].as_str().filter(|s| !s.is_empty()) {
                cfg.phone.telnyx.from_number = Some(from.to_string());
            }
            if let Some(url) = params["webhook_url"].as_str().filter(|s| !s.is_empty()) {
                cfg.phone.telnyx.webhook_url = Some(url.to_string());
            }
            if let Some(conn) = params["connection_id"].as_str().filter(|s| !s.is_empty()) {
                cfg.phone.telnyx.connection_id = Some(conn.to_string());
            }
        },
        "plivo" => {
            if let Some(from) = params["from_number"].as_str().filter(|s| !s.is_empty()) {
                cfg.phone.plivo.from_number = Some(from.to_string());
            }
            if let Some(url) = params["webhook_url"].as_str().filter(|s| !s.is_empty()) {
                cfg.phone.plivo.webhook_url = Some(url.to_string());
            }
        },
        _ => {},
    }
}

/// Remove inline credentials for a provider after they are moved into KeyStore.
pub(super) fn clear_inline_phone_credentials(cfg: &mut MoltisConfig, provider: &str) {
    match provider {
        "twilio" => {
            cfg.phone.twilio.account_sid = None;
            cfg.phone.twilio.auth_token = None;
        },
        "telnyx" => {
            cfg.phone.telnyx.api_key = None;
            cfg.phone.telnyx.connection_id = None;
        },
        "plivo" => {
            cfg.phone.plivo.auth_id = None;
            cfg.phone.plivo.auth_token = None;
        },
        _ => {},
    }
}

/// Toggle a phone provider on/off.
pub(super) fn toggle_phone_provider(provider: &str, enabled: bool) -> anyhow::Result<()> {
    moltis_config::update_config(|cfg| {
        if enabled {
            cfg.phone.enabled = true;
            cfg.phone.provider = provider.to_string();
        } else if cfg.phone.provider == provider {
            cfg.phone.enabled = false;
            cfg.phone.provider = String::new();
        }
    })?;
    Ok(())
}

/// Key store name for a phone provider.
pub(super) fn phone_key_store_name(provider: &str) -> String {
    format!("phone_{provider}")
}

/// Reconcile the in-memory telephony account with the persisted `[phone]`
/// config after an RPC mutation.
///
/// `GatewayState::config` is a startup snapshot, so this deliberately reloads
/// the config file and overlays `KeyStore` credentials before restarting the
/// internal telephony account.
#[cfg(feature = "telephony")]
pub(super) async fn reload_running_phone_account(
    state: &crate::state::GatewayState,
) -> anyhow::Result<()> {
    let mut config = moltis_config::discover_and_load();
    merge_phone_keys(&mut config);

    match phone_channel_account(&config) {
        Some((account_id, account_config)) => {
            if let Some(registry) = state.services.channel_registry.as_ref() {
                registry
                    .start_account(PHONE_CHANNEL_TYPE, &account_id, account_config)
                    .await?;
            } else if let Some(plugin) = state.services.telephony_plugin.as_ref() {
                plugin
                    .write()
                    .await
                    .start_account(&account_id, account_config)
                    .await?;
            }
        },
        None => {
            if let Some(registry) = state.services.channel_registry.as_ref() {
                if registry.resolve_channel_type(PHONE_ACCOUNT_ID).as_deref()
                    == Some(PHONE_CHANNEL_TYPE)
                {
                    registry
                        .stop_account(PHONE_CHANNEL_TYPE, PHONE_ACCOUNT_ID)
                        .await?;
                }
            } else if let Some(plugin) = state.services.telephony_plugin.as_ref() {
                let has_account = plugin.read().await.has_account(PHONE_ACCOUNT_ID);
                if has_account {
                    plugin.write().await.stop_account(PHONE_ACCOUNT_ID).await?;
                }
            }
        },
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use {
        crate::methods::phone::{
            apply_phone_provider_settings, clear_inline_phone_credentials, detect_phone_providers,
            phone_channel_account, phone_key_store_name,
        },
        moltis_config::schema::MoltisConfig,
        secrecy::{ExposeSecret, Secret},
    };

    struct PhoneConfigTestGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        _config_dir: tempfile::TempDir,
        _data_dir: tempfile::TempDir,
    }

    impl PhoneConfigTestGuard {
        fn new() -> Self {
            let lock = crate::config_override_test_lock();
            let config_dir = tempfile::tempdir()
                .unwrap_or_else(|error| panic!("config tempdir should be created: {error}"));
            let data_dir = tempfile::tempdir()
                .unwrap_or_else(|error| panic!("data tempdir should be created: {error}"));
            moltis_config::set_config_dir(config_dir.path().to_path_buf());
            moltis_config::set_data_dir(data_dir.path().to_path_buf());
            Self {
                _lock: lock,
                _config_dir: config_dir,
                _data_dir: data_dir,
            }
        }
    }

    impl Drop for PhoneConfigTestGuard {
        fn drop(&mut self) {
            moltis_config::clear_config_dir();
            moltis_config::clear_data_dir();
        }
    }

    #[test]
    fn detect_phone_providers_returns_all() {
        let config = MoltisConfig::default();
        let result = detect_phone_providers(&config);
        let providers = result["providers"]
            .as_array()
            .unwrap_or_else(|| panic!("array"));
        assert_eq!(providers.len(), 3);
        assert_eq!(providers[0]["id"], "twilio");
        assert_eq!(providers[1]["id"], "telnyx");
        assert_eq!(providers[2]["id"], "plivo");
    }

    #[test]
    fn detect_phone_providers_marks_twilio_enabled() {
        let mut config = MoltisConfig::default();
        config.phone.enabled = true;
        config.phone.provider = "twilio".to_string();
        config.phone.twilio.account_sid = Some(Secret::new("AC_test_sid".to_string()));
        let result = detect_phone_providers(&config);
        let providers = result["providers"]
            .as_array()
            .unwrap_or_else(|| panic!("array"));
        assert_eq!(providers[0]["available"], true);
        assert_eq!(providers[0]["enabled"], true);
        assert_eq!(providers[1]["enabled"], false);
    }

    #[test]
    fn detect_phone_providers_marks_telnyx_enabled() {
        let mut config = MoltisConfig::default();
        config.phone.enabled = true;
        config.phone.provider = "telnyx".to_string();
        config.phone.telnyx.api_key = Some(Secret::new("KEY_test".to_string()));
        let result = detect_phone_providers(&config);
        let providers = result["providers"]
            .as_array()
            .unwrap_or_else(|| panic!("array"));
        assert_eq!(providers[0]["enabled"], false);
        assert_eq!(providers[1]["available"], true);
        assert_eq!(providers[1]["enabled"], true);
    }

    #[test]
    fn apply_phone_provider_settings_updates_twilio() {
        let mut config = MoltisConfig::default();
        let params = serde_json::json!({
            "from_number": "+15551234567",
            "webhook_url": "https://example.com/webhook",
        });
        apply_phone_provider_settings(&mut config, "twilio", &params);
        assert_eq!(
            config.phone.twilio.from_number.as_deref(),
            Some("+15551234567")
        );
    }

    #[test]
    fn apply_phone_provider_settings_updates_telnyx() {
        let mut config = MoltisConfig::default();
        let params = serde_json::json!({
            "from_number": "+15559876543",
            "connection_id": "conn_abc123",
            "webhook_url": "https://example.com/telnyx",
        });
        apply_phone_provider_settings(&mut config, "telnyx", &params);
        assert_eq!(
            config.phone.telnyx.from_number.as_deref(),
            Some("+15559876543")
        );
        assert_eq!(
            config.phone.telnyx.connection_id.as_deref(),
            Some("conn_abc123")
        );
    }

    #[test]
    fn phone_key_store_name_formats_correctly() {
        assert_eq!(phone_key_store_name("twilio"), "phone_twilio");
        assert_eq!(phone_key_store_name("telnyx"), "phone_telnyx");
    }

    #[test]
    fn phone_channel_account_maps_twilio_phone_config_to_internal_channel() {
        let mut config = MoltisConfig::default();
        config.phone.enabled = true;
        config.phone.provider = "twilio".to_string();
        config.phone.inbound_policy = "allowlist".to_string();
        config.phone.allowlist = vec!["+15557654321".to_string()];
        config.phone.twilio.account_sid = Some(Secret::new("AC_test_sid".to_string()));
        config.phone.twilio.auth_token = Some(Secret::new("test_token".to_string()));
        config.phone.twilio.from_number = Some("+15551234567".to_string());
        config.phone.twilio.webhook_url = Some("https://phone.example.com".to_string());

        let (account_id, account) = phone_channel_account(&config)
            .unwrap_or_else(|| panic!("phone account should be available"));

        assert_eq!(account_id, "default");
        assert_eq!(account["provider"], "twilio");
        assert_eq!(account["account_sid"], "AC_test_sid");
        assert_eq!(account["auth_token"], "test_token");
        assert_eq!(account["from_number"], "+15551234567");
        assert_eq!(account["webhook_url"], "https://phone.example.com");
        assert_eq!(account["inbound_policy"], "allowlist");
        assert_eq!(account["allowlist"][0], "+15557654321");
    }

    #[test]
    fn clear_inline_phone_credentials_removes_selected_provider_only() {
        let mut config = MoltisConfig::default();
        config.phone.twilio.account_sid = Some(Secret::new("AC_test_sid".to_string()));
        config.phone.twilio.auth_token = Some(Secret::new("test_token".to_string()));
        config.phone.telnyx.api_key = Some(Secret::new("KEY_test".to_string()));
        config.phone.telnyx.connection_id = Some("conn_test".to_string());

        clear_inline_phone_credentials(&mut config, "telnyx");

        assert!(config.phone.telnyx.api_key.is_none());
        assert!(config.phone.telnyx.connection_id.is_none());
        assert_eq!(
            config
                .phone
                .twilio
                .account_sid
                .as_ref()
                .map(|secret| secret.expose_secret().as_str()),
            Some("AC_test_sid")
        );
    }

    #[cfg(feature = "telephony")]
    #[tokio::test]
    async fn reload_running_phone_account_restarts_account_from_key_store() {
        use {
            crate::{
                auth::{AuthMode, ResolvedAuth},
                methods::phone::reload_running_phone_account,
                services::GatewayServices,
                state::GatewayState,
            },
            moltis_channels::ChannelRegistry,
            std::sync::Arc,
            tokio::sync::RwLock,
        };

        let _guard = PhoneConfigTestGuard::new();
        let telephony_plugin = Arc::new(RwLock::new(moltis_telephony::TelephonyPlugin::new()));
        let mut registry = ChannelRegistry::new();
        registry
            .register(
                Arc::clone(&telephony_plugin) as Arc<RwLock<dyn moltis_channels::ChannelPlugin>>
            )
            .await;
        let registry = Arc::new(registry);
        let services = GatewayServices::noop()
            .with_channel_registry(Arc::clone(&registry))
            .with_telephony_plugin(Arc::clone(&telephony_plugin));
        let state = GatewayState::new(
            ResolvedAuth {
                mode: AuthMode::Token,
                token: None,
                password: None,
            },
            services,
        );

        crate::provider_setup::KeyStore::new()
            .save_config(
                &phone_key_store_name("twilio"),
                Some("AC_old".to_string()),
                Some("old_token".to_string()),
                None,
            )
            .unwrap_or_else(|error| panic!("phone credentials should be stored: {error}"));
        moltis_config::update_config(|cfg| {
            cfg.phone.enabled = true;
            cfg.phone.provider = "twilio".to_string();
            cfg.phone.twilio.from_number = Some("+15550000001".to_string());
            cfg.phone.twilio.account_sid = None;
            cfg.phone.twilio.auth_token = None;
        })
        .unwrap_or_else(|error| panic!("phone config should be stored: {error}"));

        reload_running_phone_account(&state)
            .await
            .unwrap_or_else(|error| panic!("phone account should reload: {error}"));
        let first_manager = telephony_plugin
            .read()
            .await
            .call_manager("default")
            .unwrap_or_else(|| panic!("phone account should be running"));
        assert_eq!(
            registry.resolve_channel_type("default").as_deref(),
            Some("telephony")
        );

        crate::provider_setup::KeyStore::new()
            .save_config(
                &phone_key_store_name("twilio"),
                Some("AC_new".to_string()),
                Some("new_token".to_string()),
                None,
            )
            .unwrap_or_else(|error| panic!("phone credentials should be updated: {error}"));
        moltis_config::update_config(|cfg| {
            cfg.phone.twilio.from_number = Some("+15550000002".to_string());
        })
        .unwrap_or_else(|error| panic!("phone config should be updated: {error}"));

        reload_running_phone_account(&state)
            .await
            .unwrap_or_else(|error| panic!("phone account should reload again: {error}"));
        let plugin = telephony_plugin.read().await;
        let second_manager = plugin
            .call_manager("default")
            .unwrap_or_else(|| panic!("phone account should still be running"));

        assert!(!Arc::ptr_eq(&first_manager, &second_manager));
        assert_eq!(
            plugin.caller_number("default").as_deref(),
            Some("+15550000002")
        );
    }
}
