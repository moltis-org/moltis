//! Import credentials from Hermes `.env` file.
//!
//! Parses well-known API key environment variables and maps them to
//! Moltis provider names.

use std::path::Path;

use {
    moltis_import_core::report::{CategoryReport, ImportCategory, ImportStatus},
    tracing::debug,
};

use crate::detect::HermesDetection;

/// A discovered credential from Hermes.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoveredCredential {
    pub env_var: String,
    pub provider: String,
}

/// Well-known environment variable to provider mappings.
const CREDENTIAL_MAPPINGS: &[(&str, &str)] = &[
    ("OPENAI_API_KEY", "openai"),
    ("ANTHROPIC_API_KEY", "anthropic"),
    ("OPENROUTER_API_KEY", "openrouter"),
    ("GOOGLE_API_KEY", "google"),
    ("GEMINI_API_KEY", "gemini"),
    ("GROQ_API_KEY", "groq"),
    ("XAI_API_KEY", "xai"),
    ("MISTRAL_API_KEY", "mistral"),
    ("DEEPSEEK_API_KEY", "deepseek"),
];

/// Parse a `.env` file into key-value pairs.
fn parse_env(content: &str) -> Vec<(String, String)> {
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            let key = key.trim().to_string();
            let value = value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string();
            if key.is_empty() || value.is_empty() {
                return None;
            }
            Some((key, value))
        })
        .collect()
}

/// Discover credentials from the Hermes `.env` file.
pub fn discover_credentials(detection: &HermesDetection) -> Vec<DiscoveredCredential> {
    let Some(ref env_path) = detection.env_path else {
        return Vec::new();
    };

    let Ok(content) = std::fs::read_to_string(env_path) else {
        return Vec::new();
    };

    let env_vars = parse_env(&content);
    let mut found = Vec::new();

    for (env_var, provider) in CREDENTIAL_MAPPINGS {
        if env_vars.iter().any(|(k, _)| k == env_var) {
            found.push(DiscoveredCredential {
                env_var: (*env_var).to_string(),
                provider: (*provider).to_string(),
            });
        }
    }

    found
}

/// Import credentials from Hermes `.env` into Moltis provider keys.
///
/// Writes discovered API keys to `provider_keys.json` in the config directory.
/// Skips providers that already have keys configured.
pub fn import_credentials(detection: &HermesDetection, config_dir: &Path) -> CategoryReport {
    let Some(ref env_path) = detection.env_path else {
        return CategoryReport::skipped(ImportCategory::Providers);
    };

    let Ok(content) = std::fs::read_to_string(env_path) else {
        return CategoryReport::failed(
            ImportCategory::Providers,
            "failed to read .env file".to_string(),
        );
    };

    let env_vars = parse_env(&content);
    if env_vars.is_empty() {
        return CategoryReport::skipped(ImportCategory::Providers);
    }

    // Load existing provider keys
    let keys_path = config_dir.join("provider_keys.json");
    let mut existing: serde_json::Map<String, serde_json::Value> = if keys_path.is_file() {
        match std::fs::read_to_string(&keys_path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(map) => map,
                Err(e) => {
                    return CategoryReport::failed(
                        ImportCategory::Providers,
                        format!("existing provider_keys.json is malformed: {e}"),
                    );
                },
            },
            Err(_) => serde_json::Map::new(),
        }
    } else {
        serde_json::Map::new()
    };

    let mut imported = 0;
    let mut skipped = 0;

    for (env_var, provider) in CREDENTIAL_MAPPINGS {
        let Some(value) = env_vars.iter().find(|(k, _)| k == env_var).map(|(_, v)| v) else {
            continue;
        };

        if existing.contains_key(*provider) {
            debug!(provider, env_var, "provider already has key, skipping");
            skipped += 1;
            continue;
        }

        debug!(provider, env_var, "importing credential from .env");
        existing.insert(
            (*provider).to_string(),
            serde_json::json!({ "api_key": value }),
        );
        imported += 1;
    }

    if imported > 0 {
        if let Err(e) = std::fs::create_dir_all(config_dir) {
            return CategoryReport::failed(
                ImportCategory::Providers,
                format!("failed to create config directory: {e}"),
            );
        }
        let json = match serde_json::to_string_pretty(&existing) {
            Ok(j) => j,
            Err(e) => {
                return CategoryReport::failed(
                    ImportCategory::Providers,
                    format!("failed to serialize provider keys: {e}"),
                );
            },
        };
        if let Err(e) = std::fs::write(&keys_path, json) {
            return CategoryReport::failed(
                ImportCategory::Providers,
                format!("failed to write provider_keys.json: {e}"),
            );
        }
    }

    let status = if imported == 0 {
        ImportStatus::Skipped
    } else {
        ImportStatus::Success
    };

    CategoryReport {
        category: ImportCategory::Providers,
        status,
        items_imported: imported,
        items_updated: 0,
        items_skipped: skipped,
        warnings: Vec::new(),
        errors: Vec::new(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use {super::*, std::path::PathBuf};

    fn make_detection(home: &Path) -> HermesDetection {
        HermesDetection {
            home_dir: home.to_path_buf(),
            config_path: None,
            env_path: None,
            skills_dir: None,
            soul_path: None,
            agents_path: None,
            memory_path: None,
            user_path: None,
            has_data: true,
        }
    }

    #[test]
    fn parse_env_basic() {
        let content = r#"
OPENAI_API_KEY=sk-123
ANTHROPIC_API_KEY="sk-ant-456"
# Comment
EMPTY_VAR=

GROQ_API_KEY='gsk-789'
"#;
        let pairs = parse_env(content);
        assert_eq!(pairs.len(), 3);
        assert_eq!(
            pairs[0],
            ("OPENAI_API_KEY".to_string(), "sk-123".to_string())
        );
        assert_eq!(
            pairs[1],
            ("ANTHROPIC_API_KEY".to_string(), "sk-ant-456".to_string())
        );
        assert_eq!(
            pairs[2],
            ("GROQ_API_KEY".to_string(), "gsk-789".to_string())
        );
    }

    #[test]
    fn discover_credentials_finds_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".hermes");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            home.join(".env"),
            "OPENAI_API_KEY=sk-test\nANTHROPIC_API_KEY=sk-ant\nUNKNOWN_KEY=abc\n",
        )
        .unwrap();

        let mut detection = make_detection(&home);
        detection.env_path = Some(home.join(".env"));

        let creds = discover_credentials(&detection);
        assert_eq!(creds.len(), 2);
        assert_eq!(creds[0].provider, "openai");
        assert_eq!(creds[1].provider, "anthropic");
    }

    #[test]
    fn import_credentials_writes_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".hermes");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(".env"), "OPENAI_API_KEY=sk-test-123\n").unwrap();

        let config_dir = tmp.path().join("config");

        let mut detection = make_detection(&home);
        detection.env_path = Some(home.join(".env"));

        let report = import_credentials(&detection, &config_dir);
        assert_eq!(report.status, ImportStatus::Success);
        assert_eq!(report.items_imported, 1);

        let content = std::fs::read_to_string(config_dir.join("provider_keys.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["openai"]["api_key"].as_str(), Some("sk-test-123"));
    }

    #[test]
    fn import_credentials_skips_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".hermes");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(".env"), "OPENAI_API_KEY=sk-new\n").unwrap();

        let config_dir = tmp.path().join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(
            config_dir.join("provider_keys.json"),
            r#"{"openai":{"api_key":"sk-existing"}}"#,
        )
        .unwrap();

        let mut detection = make_detection(&home);
        detection.env_path = Some(home.join(".env"));

        let report = import_credentials(&detection, &config_dir);
        assert_eq!(report.items_skipped, 1);
        assert_eq!(report.items_imported, 0);

        // Existing key preserved
        let content = std::fs::read_to_string(config_dir.join("provider_keys.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["openai"]["api_key"].as_str(), Some("sk-existing"));
    }

    #[test]
    fn import_credentials_no_env_file() {
        let tmp = tempfile::tempdir().unwrap();
        let detection = make_detection(tmp.path());
        let report = import_credentials(&detection, tmp.path());
        assert_eq!(report.status, ImportStatus::Skipped);
    }
}
