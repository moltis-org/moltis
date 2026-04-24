//! Home Assistant config re-exports and resolution helpers.

use crate::error::{Error, Result};

pub use moltis_config::{HomeAssistantAccountConfig, HomeAssistantConfig};

/// Resolve which HA instance to use, returning an error if unavailable.
///
/// Resolution order: explicit `instance` → `default_instance` →
/// sole instance if only one exists.
pub fn resolve_instance<'a>(
    config: &'a HomeAssistantConfig,
    instance: Option<&'a str>,
) -> Result<(&'a str, &'a HomeAssistantAccountConfig)> {
    config.resolve_account(instance).ok_or_else(|| {
        let names: Vec<&str> = config.instances.keys().map(String::as_str).collect();
        Error::Config(format!(
            "no HA instance specified and {} configured; \
                 pass 'instance' or set home_assistant.default_instance",
            match names.len() {
                0 => "none are".to_owned(),
                _ => format!("available: {}", names.join(", ")),
            }
        ))
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use {super::*, secrecy::Secret};

    fn make_config() -> HomeAssistantConfig {
        let mut config = HomeAssistantConfig {
            enabled: true,
            ..Default::default()
        };
        config
            .instances
            .insert("home".to_owned(), HomeAssistantAccountConfig {
                url: Some("http://localhost:8123".to_owned()),
                token: Some(Secret::new("test-token".to_owned())),
                timeout_seconds: 10,
            });
        config
    }

    #[test]
    fn resolve_single_instance_implicitly() {
        let config = make_config();
        let (name, _account) = resolve_instance(&config, None).unwrap();
        assert_eq!(name, "home");
    }

    #[test]
    fn resolve_uses_default_instance() {
        let mut config = make_config();
        config.default_instance = Some("home".to_owned());
        let (name, _) = resolve_instance(&config, None).unwrap();
        assert_eq!(name, "home");
    }

    #[test]
    fn resolve_errors_on_empty() {
        let config = HomeAssistantConfig::default();
        let result = resolve_instance(&config, None);
        assert!(result.is_err());
    }

    #[test]
    fn resolve_errors_on_unknown_instance() {
        let config = make_config();
        let result = resolve_instance(&config, Some("ghost"));
        assert!(result.is_err());
    }

    #[test]
    fn resolve_explicit_instance() {
        let mut config = make_config();
        config
            .instances
            .insert("office".to_owned(), HomeAssistantAccountConfig {
                url: Some("http://office:8123".to_owned()),
                token: Some(Secret::new("office-token".to_owned())),
                timeout_seconds: 10,
            });
        let (name, _) = resolve_instance(&config, Some("office")).unwrap();
        assert_eq!(name, "office");
    }

    #[test]
    fn resolve_ambiguous_without_default() {
        let mut config = make_config();
        config
            .instances
            .insert("office".to_owned(), HomeAssistantAccountConfig {
                url: Some("http://office:8123".to_owned()),
                token: Some(Secret::new("office-token".to_owned())),
                timeout_seconds: 10,
            });
        // Two instances, no default, no explicit choice → error
        let result = resolve_instance(&config, None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("home") && err.contains("office"));
    }

    #[test]
    fn resolve_default_instance_overrides_ambiguous() {
        let mut config = make_config();
        config.default_instance = Some("office".to_owned());
        config
            .instances
            .insert("office".to_owned(), HomeAssistantAccountConfig {
                url: Some("http://office:8123".to_owned()),
                token: Some(Secret::new("office-token".to_owned())),
                timeout_seconds: 10,
            });
        let (name, _) = resolve_instance(&config, None).unwrap();
        assert_eq!(name, "office");
    }

    #[test]
    fn resolve_explicit_overrides_default() {
        let mut config = make_config();
        config.default_instance = Some("home".to_owned());
        config
            .instances
            .insert("office".to_owned(), HomeAssistantAccountConfig {
                url: Some("http://office:8123".to_owned()),
                token: Some(Secret::new("office-token".to_owned())),
                timeout_seconds: 10,
            });
        let (name, _) = resolve_instance(&config, Some("office")).unwrap();
        assert_eq!(name, "office");
    }

    #[test]
    fn resolve_empty_instance_list() {
        let config = HomeAssistantConfig::default();
        let err = resolve_instance(&config, None).unwrap_err();
        assert!(err.to_string().contains("none are"));
    }
}
