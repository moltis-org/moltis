//! Import channel configuration from OpenClaw.

use std::path::Path;

use {
    serde::{Deserialize, Serialize},
    serde_json::Value,
    tracing::debug,
};

use crate::{
    detect::OpenClawDetection,
    report::{CategoryReport, ImportCategory, ImportStatus},
    types::{OpenClawConfig, OpenClawDiscordAccount, OpenClawTelegramAccount},
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

/// Imported Discord channel configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedDiscordChannel {
    /// Account identifier (from OpenClaw's accounts map key).
    pub account_id: String,
    /// Bot token.
    pub token: String,
    /// DM policy.
    pub dm_policy: Option<String>,
    /// Group policy.
    pub group_policy: Option<String>,
    /// Mention mode.
    pub mention_mode: Option<String>,
    /// Allowed users (Discord IDs or usernames).
    pub allowlist: Vec<String>,
    /// Allowed guild/server IDs.
    pub guild_allowlist: Vec<String>,
}

/// Imported Signal channel configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedSignalChannel {
    /// Account identifier (from OpenClaw's accounts map key or `default`).
    pub account_id: String,
    /// Signal account loaded by signal-cli, usually an E.164 phone number.
    pub account: Option<String>,
    /// Signal account UUID if OpenClaw stored it.
    pub account_uuid: Option<String>,
    /// signal-cli daemon HTTP URL.
    pub http_url: Option<String>,
    /// DM policy.
    pub dm_policy: Option<String>,
    /// Allowed senders.
    pub allowlist: Vec<String>,
    /// Group policy.
    pub group_policy: Option<String>,
    /// Allowed Signal group IDs.
    pub group_allowlist: Vec<String>,
    /// Group mention mode.
    pub mention_mode: Option<String>,
    /// Whether the account is enabled.
    pub enabled: Option<bool>,
}

/// Import result for channels.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportedChannels {
    pub telegram: Vec<ImportedTelegramChannel>,
    pub discord: Vec<ImportedDiscordChannel>,
    pub signal: Vec<ImportedSignalChannel>,
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

    if let Some(dc) = &config.channels.discord {
        // Try accounts map first
        if let Some(accounts) = &dc.accounts {
            for (id, account) in accounts {
                if let Some(channel) = extract_discord_account(id, account) {
                    debug!(account_id = %id, "imported Discord account");
                    result.discord.push(channel);
                    imported += 1;
                }
            }
        }

        // Fall back to flat top-level config
        if result.discord.is_empty() {
            if dc.enabled == Some(false) {
                // disabled flat account
            } else if let Some(token) = dc.token.as_ref().filter(|t| !t.is_empty()) {
                let allowlist = parse_allowlist(&dc.allow_from);
                let guild_allowlist = parse_allowlist(&dc.guild_allowlist);
                result.discord.push(ImportedDiscordChannel {
                    account_id: "default".to_string(),
                    token: token.clone(),
                    dm_policy: dc.dm_policy.clone(),
                    group_policy: dc.group_policy.clone(),
                    mention_mode: dc.mention_mode.clone(),
                    allowlist,
                    guild_allowlist,
                });
                imported += 1;
            }
        }
    }

    if let Some(signal) = &config.channels.signal {
        for channel in extract_signal_channels(signal) {
            debug!(account_id = %channel.account_id, "imported Signal account");
            result.signal.push(channel);
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

fn extract_discord_account(
    id: &str,
    account: &OpenClawDiscordAccount,
) -> Option<ImportedDiscordChannel> {
    let token = account.token.as_ref()?;
    if token.is_empty() {
        return None;
    }

    // Skip disabled accounts
    if account.enabled == Some(false) {
        return None;
    }

    let allowlist = parse_allowlist(&account.allow_from);
    let guild_allowlist = parse_allowlist(&account.guild_allowlist);

    Some(ImportedDiscordChannel {
        account_id: id.to_string(),
        token: token.clone(),
        dm_policy: account.dm_policy.clone(),
        group_policy: account.group_policy.clone(),
        mention_mode: account.mention_mode.clone(),
        allowlist,
        guild_allowlist,
    })
}

fn extract_signal_channels(value: &Value) -> Vec<ImportedSignalChannel> {
    let mut channels = Vec::new();

    if let Some(accounts) = value.get("accounts").and_then(Value::as_object) {
        for (id, account) in accounts {
            if let Some(channel) = extract_signal_account(id, account) {
                channels.push(channel);
            }
        }
    }

    if channels.is_empty()
        && let Some(channel) = extract_signal_account("default", value)
    {
        channels.push(channel);
    }

    channels
}

fn extract_signal_account(id: &str, value: &Value) -> Option<ImportedSignalChannel> {
    if get_bool(value, &["enabled"]) == Some(false) {
        return None;
    }

    let account = get_string(value, &[
        "account",
        "number",
        "phoneNumber",
        "phone_number",
        "username",
    ])
    .or_else(|| (id != "default").then(|| id.to_string()));
    let account_uuid = get_string(value, &["accountUuid", "account_uuid", "uuid"]);
    let http_url = get_string(value, &["httpUrl", "http_url", "daemonUrl", "daemon_url"]);
    let allowlist = get_string_list(value, &[
        "allowFrom",
        "allow_from",
        "allowlist",
        "allowedSenders",
        "allowed_senders",
    ]);
    let group_allowlist = get_string_list(value, &[
        "groupAllowlist",
        "group_allowlist",
        "groupAllowFrom",
        "group_allow_from",
    ]);

    if account.is_none() && account_uuid.is_none() && http_url.is_none() && allowlist.is_empty() {
        return None;
    }

    Some(ImportedSignalChannel {
        account_id: id.to_string(),
        account,
        account_uuid,
        http_url,
        dm_policy: get_string(value, &["dmPolicy", "dm_policy"]),
        allowlist,
        group_policy: get_string(value, &["groupPolicy", "group_policy"]),
        group_allowlist,
        mention_mode: get_string(value, &["mentionMode", "mention_mode"]),
        enabled: get_bool(value, &["enabled"]),
    })
}

/// Parse OpenClaw's `allowFrom` array into Telegram user IDs.
///
/// OpenClaw allows both numbers and strings like `"tg:123456"`.
fn parse_allow_from(values: &[Value]) -> Vec<i64> {
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

/// Parse OpenClaw `allowFrom`/`guildAllowlist` arrays into string IDs.
fn parse_allowlist(values: &[Value]) -> Vec<String> {
    values
        .iter()
        .filter_map(|v| {
            if let Some(s) = v.as_str() {
                Some(s.to_string())
            } else {
                v.as_i64().map(|n| n.to_string())
            }
        })
        .collect()
}

fn get_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

fn get_bool(value: &Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_bool))
}

fn get_string_list(value: &Value, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_array))
        .map(|values| parse_allowlist(values))
        .unwrap_or_default()
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
            has_workspace_files: false,
            workspace_files_found: Vec::new(),
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
    fn import_discord_accounts() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("openclaw.json"),
            r#"{
                "channels": {
                    "discord": {
                        "accounts": {
                            "mybot": {
                                "token": "Bot token-123",
                                "dmPolicy": "pairing",
                                "groupPolicy": "allowlist",
                                "mentionMode": "always",
                                "allowFrom": ["111", 222],
                                "guildAllowlist": ["333", 444]
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
        assert_eq!(result.discord.len(), 1);
        assert_eq!(result.discord[0].token, "Bot token-123");
        assert_eq!(result.discord[0].dm_policy.as_deref(), Some("pairing"));
        assert_eq!(result.discord[0].group_policy.as_deref(), Some("allowlist"));
        assert_eq!(result.discord[0].mention_mode.as_deref(), Some("always"));
        assert_eq!(result.discord[0].allowlist, vec!["111", "222"]);
        assert_eq!(result.discord[0].guild_allowlist, vec!["333", "444"]);
    }

    #[test]
    fn import_discord_flat_config() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("openclaw.json"),
            r#"{"channels":{"discord":{"token":"Bot xyz","allowFrom":["abc"]}}}"#,
        )
        .unwrap();

        let detection = make_detection(tmp.path());
        let (_, result) = import_channels(&detection);

        assert_eq!(result.discord.len(), 1);
        assert_eq!(result.discord[0].account_id, "default");
        assert_eq!(result.discord[0].token, "Bot xyz");
        assert_eq!(result.discord[0].allowlist, vec!["abc"]);
    }

    #[test]
    fn import_signal_accounts() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("openclaw.json"),
            r#"{
                "channels": {
                    "signal": {
                        "accounts": {
                            "personal": {
                                "account": "+15551234567",
                                "accountUuid": "550e8400-e29b-41d4-a716-446655440000",
                                "httpUrl": "http://127.0.0.1:8080",
                                "dmPolicy": "allowlist",
                                "allowFrom": ["+15557654321"],
                                "groupPolicy": "allowlist",
                                "groupAllowlist": ["group-1"],
                                "mentionMode": "always"
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
        assert_eq!(result.signal.len(), 1);
        assert_eq!(result.signal[0].account_id, "personal");
        assert_eq!(result.signal[0].account.as_deref(), Some("+15551234567"));
        assert_eq!(
            result.signal[0].account_uuid.as_deref(),
            Some("550e8400-e29b-41d4-a716-446655440000")
        );
        assert_eq!(
            result.signal[0].http_url.as_deref(),
            Some("http://127.0.0.1:8080")
        );
        assert_eq!(result.signal[0].allowlist, vec!["+15557654321"]);
        assert_eq!(result.signal[0].group_allowlist, vec!["group-1"]);
        assert_eq!(result.signal[0].mention_mode.as_deref(), Some("always"));
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
    fn import_skips_disabled_discord_accounts() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("openclaw.json"),
            r#"{"channels":{"discord":{"accounts":{"disabled-bot":{"token":"Bot abc","enabled":false}}}}}"#,
        )
        .unwrap();

        let detection = make_detection(tmp.path());
        let (report, result) = import_channels(&detection);

        assert_eq!(report.status, ImportStatus::Skipped);
        assert!(result.discord.is_empty());
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
