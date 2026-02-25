//! Import channel configuration from OpenClaw (currently Telegram only).

use std::path::Path;

use {
    serde::{Deserialize, Serialize},
    tracing::debug,
};

use crate::{
    detect::OpenClawDetection,
    report::{CategoryReport, ImportCategory, ImportStatus},
    types::{OpenClawConfig, OpenClawTelegramAccount},
};

/// Imported Telegram channel configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedTelegramChannel {
    /// Account identifier (from OpenClaw's accounts map key).
    pub account_id: String,
    /// Bot token.
    pub bot_token: String,
    /// DM policy.
    pub dm_policy: Option<String>,
    /// Allowed user IDs (numeric Telegram user IDs).
    pub allowed_users: Vec<i64>,
}

/// Import result for channels.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportedChannels {
    pub telegram: Vec<ImportedTelegramChannel>,
}

/// Import channel configuration from OpenClaw.
pub fn import_channels(detection: &OpenClawDetection) -> (CategoryReport, ImportedChannels) {
    let config_path = detection.home_dir.join("openclaw.json");
    let config = load_config(&config_path);

    let mut result = ImportedChannels::default();
    let mut imported = 0;
    let mut warnings = Vec::new();

    if let Some(tg) = &config.channels.telegram {
        // Try accounts map first
        if let Some(accounts) = &tg.accounts {
            for (id, account) in accounts {
                if let Some(channel) = extract_telegram_account(id, account) {
                    debug!(account_id = %id, "imported Telegram account");
                    result.telegram.push(channel);
                    imported += 1;
                }
            }
        }

        // Fall back to flat top-level config
        if result.telegram.is_empty() && tg.bot_token.is_some() {
            let token = tg.bot_token.as_ref();
            let allowed_users = parse_allow_from(&tg.allow_from);
            result.telegram.push(ImportedTelegramChannel {
                account_id: "default".to_string(),
                bot_token: token.cloned().unwrap_or_default(),
                dm_policy: tg.dm_policy.clone(),
                allowed_users,
            });
            imported += 1;
        }
    }

    // Record unsupported channels as warnings/skips.
    let mut skipped = 0;
    for ch in &detection.unsupported_channels {
        warnings.push(format!("channel '{ch}' is not yet supported by Moltis"));
        skipped += 1;
    }

    let status = if imported == 0 {
        ImportStatus::Skipped
    } else {
        ImportStatus::Success
    };

    let mut report = CategoryReport {
        category: ImportCategory::Channels,
        status,
        items_imported: imported,
        items_updated: 0,
        items_skipped: skipped,
        warnings,
        errors: Vec::new(),
    };

    if !report.warnings.is_empty() && imported > 0 {
        report.status = ImportStatus::Partial;
    }

    (report, result)
}

fn extract_telegram_account(
    id: &str,
    account: &OpenClawTelegramAccount,
) -> Option<ImportedTelegramChannel> {
    let token = account.bot_token.as_ref()?;
    if token.is_empty() {
        return None;
    }

    // Skip disabled accounts
    if account.enabled == Some(false) {
        return None;
    }

    let allowed_users = parse_allow_from(&account.allow_from);

    Some(ImportedTelegramChannel {
        account_id: id.to_string(),
        bot_token: token.clone(),
        dm_policy: account.dm_policy.clone(),
        allowed_users,
    })
}

/// Parse OpenClaw's `allowFrom` array into Telegram user IDs.
///
/// OpenClaw allows both numbers and strings like `"tg:123456"`.
fn parse_allow_from(values: &[serde_json::Value]) -> Vec<i64> {
    values
        .iter()
        .filter_map(|v| {
            if let Some(n) = v.as_i64() {
                Some(n)
            } else if let Some(s) = v.as_str() {
                // Strip "tg:" prefix
                let stripped = s.strip_prefix("tg:").unwrap_or(s);
                stripped.parse::<i64>().ok()
            } else {
                None
            }
        })
        .collect()
}

fn load_config(path: &Path) -> OpenClawConfig {
    if !path.is_file() {
        return OpenClawConfig::default();
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return OpenClawConfig::default();
    };
    json5::from_str(&content).unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn make_detection(home: &Path) -> OpenClawDetection {
        OpenClawDetection {
            home_dir: home.to_path_buf(),
            has_config: true,
            has_credentials: false,
            has_mcp_servers: false,
            workspace_dir: home.join("workspace"),
            has_memory: false,
            has_skills: false,
            agent_ids: Vec::new(),
            session_count: 0,
            unsupported_channels: Vec::new(),
        }
    }

    #[test]
    fn import_telegram_accounts() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("openclaw.json"),
            r#"{
                "channels": {
                    "telegram": {
                        "accounts": {
                            "mybot": {
                                "botToken": "123:ABC",
                                "dmPolicy": "pairing",
                                "allowFrom": [111, "tg:222"]
                            }
                        }
                    }
                }
            }"#,
        )
        .unwrap();

        let detection = make_detection(tmp.path());
        let (report, result) = import_channels(&detection);

        assert_eq!(report.status, ImportStatus::Success);
        assert_eq!(result.telegram.len(), 1);
        assert_eq!(result.telegram[0].bot_token, "123:ABC");
        assert_eq!(result.telegram[0].dm_policy.as_deref(), Some("pairing"));
        assert_eq!(result.telegram[0].allowed_users, vec![111, 222]);
    }

    #[test]
    fn import_telegram_flat_config() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("openclaw.json"),
            r#"{"channels":{"telegram":{"botToken":"456:DEF","allowFrom":[333]}}}"#,
        )
        .unwrap();

        let detection = make_detection(tmp.path());
        let (_, result) = import_channels(&detection);

        assert_eq!(result.telegram.len(), 1);
        assert_eq!(result.telegram[0].account_id, "default");
        assert_eq!(result.telegram[0].bot_token, "456:DEF");
    }

    #[test]
    fn import_skips_disabled_accounts() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("openclaw.json"),
            r#"{"channels":{"telegram":{"accounts":{"disabled-bot":{"botToken":"789:GHI","enabled":false}}}}}"#,
        )
        .unwrap();

        let detection = make_detection(tmp.path());
        let (report, result) = import_channels(&detection);

        assert_eq!(report.status, ImportStatus::Skipped);
        assert!(result.telegram.is_empty());
    }

    #[test]
    fn parse_allow_from_mixed() {
        let values = vec![
            serde_json::json!(123),
            serde_json::json!("tg:456"),
            serde_json::json!("789"),
            serde_json::json!("not-a-number"),
        ];
        let result = parse_allow_from(&values);
        assert_eq!(result, vec![123, 456, 789]);
    }

    #[test]
    fn no_telegram_returns_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("openclaw.json"), r#"{"channels":{}}"#).unwrap();

        let detection = make_detection(tmp.path());
        let (report, _) = import_channels(&detection);
        assert_eq!(report.status, ImportStatus::Skipped);
    }

    #[test]
    fn unsupported_channels_in_warnings() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("openclaw.json"),
            r#"{"channels":{"telegram":{"botToken":"t"},"whatsapp":{"enabled":true}}}"#,
        )
        .unwrap();

        let mut detection = make_detection(tmp.path());
        detection.unsupported_channels = vec!["whatsapp".to_string()];

        let (report, _) = import_channels(&detection);
        assert_eq!(report.status, ImportStatus::Partial);
        assert_eq!(report.items_imported, 1);
        assert_eq!(report.items_skipped, 1);
        assert!(report.warnings.iter().any(|w| w.contains("whatsapp")));
    }
}
