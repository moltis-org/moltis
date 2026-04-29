//! Phone provider detection and configuration helpers.

use {moltis_config::schema::MoltisConfig, secrecy::ExposeSecret};

/// Detect all available phone providers with their status.
pub(super) fn detect_phone_providers(config: &MoltisConfig) -> serde_json::Value {
    let mut providers = Vec::new();

    // Twilio
    let twilio_configured = config
        .phone
        .twilio
        .account_sid
        .as_ref()
        .map(|s| !s.expose_secret().is_empty())
        .unwrap_or(false);

    let twilio_enabled = config.phone.enabled
        && (config.phone.provider.is_empty() || config.phone.provider == "twilio");

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
            "from_number": config.phone.twilio.from_number.clone().unwrap_or_default(),
            "webhook_url": config.phone.twilio.webhook_url.clone().unwrap_or_default(),
        },
    }));

    // Telnyx
    let telnyx_configured = config
        .phone
        .telnyx
        .api_key
        .as_ref()
        .map(|s| !s.expose_secret().is_empty())
        .unwrap_or(false);

    let telnyx_enabled = config.phone.enabled && config.phone.provider == "telnyx";

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
            "from_number": config.phone.telnyx.from_number.clone().unwrap_or_default(),
            "webhook_url": config.phone.telnyx.webhook_url.clone().unwrap_or_default(),
            "connection_id": config.phone.telnyx.connection_id.clone().unwrap_or_default(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_phone_providers_returns_both() {
        let config = MoltisConfig::default();
        let result = detect_phone_providers(&config);
        let providers = result["providers"]
            .as_array()
            .unwrap_or_else(|| panic!("array"));
        assert_eq!(providers.len(), 2);
        assert_eq!(providers[0]["id"], "twilio");
        assert_eq!(providers[1]["id"], "telnyx");
    }

    #[test]
    fn detect_phone_providers_marks_twilio_enabled() {
        let mut config = MoltisConfig::default();
        config.phone.enabled = true;
        config.phone.provider = "twilio".to_string();
        config.phone.twilio.account_sid = Some(secrecy::Secret::new("AC_test_sid".to_string()));
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
        config.phone.telnyx.api_key = Some(secrecy::Secret::new("KEY_test".to_string()));
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
}
