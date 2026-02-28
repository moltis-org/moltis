//! Import data from an existing OpenClaw installation into Moltis.
//!
//! Provides detection, scanning, and selective import of:
//! - User/agent identity
//! - LLM provider keys and model preferences
//! - Skills (SKILL.md format)
//! - Memory (MEMORY.md and daily logs)
//! - Telegram and Discord channel configuration
//! - Chat sessions (JSONL format)

pub mod agents;
pub mod channels;
pub mod detect;
pub mod error;
pub mod identity;
pub mod memory;
pub mod providers;
pub mod report;
pub mod sessions;
pub mod skills;
pub mod types;
#[cfg(feature = "file-watcher")]
pub mod watcher;

use std::path::Path;

use {
    report::{CategoryReport, ImportCategory, ImportReport, ImportStatus},
    serde::{Deserialize, Serialize},
    tracing::{debug, info, warn},
};

pub use detect::{OpenClawDetection, detect};

/// What the user chose to import.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportSelection {
    pub identity: bool,
    pub providers: bool,
    pub skills: bool,
    pub memory: bool,
    pub channels: bool,
    pub sessions: bool,
}

impl ImportSelection {
    /// Select all categories.
    pub fn all() -> Self {
        Self {
            identity: true,
            providers: true,
            skills: true,
            memory: true,
            channels: true,
            sessions: true,
        }
    }
}

/// Summary of what data is available for import.
#[derive(Debug, Clone, Serialize)]
pub struct ImportScan {
    pub identity_available: bool,
    /// Previewed agent name (non-destructive extraction, not yet imported).
    pub identity_agent_name: Option<String>,
    /// Previewed theme (non-destructive extraction, not yet imported).
    pub identity_theme: Option<String>,
    /// Previewed user name (non-destructive extraction, not yet imported).
    pub identity_user_name: Option<String>,
    pub providers_available: bool,
    pub skills_count: usize,
    pub memory_available: bool,
    pub memory_files_count: usize,
    pub channels_available: bool,
    pub telegram_accounts: usize,
    pub discord_accounts: usize,
    pub sessions_count: usize,
    pub unsupported_channels: Vec<String>,
    pub agent_ids: Vec<String>,
    pub agents: Vec<agents::ImportedAgent>,
}

/// Scan an OpenClaw installation without importing anything.
pub fn scan(detection: &OpenClawDetection) -> ImportScan {
    // Extract identity preview (non-destructive, just reads files)
    let (_, previewed_identity) = identity::import_identity(detection);
    let identity_available = previewed_identity.agent_name.is_some()
        || previewed_identity.theme.is_some()
        || previewed_identity.user_name.is_some()
        || previewed_identity.user_timezone.is_some();

    let skills = skills::discover_skills(detection);
    let memory_files_count = count_memory_files(&detection.workspace_dir);

    let (_, channels_result) = channels::import_channels(detection);
    let telegram_accounts = channels_result.telegram.len();
    let discord_accounts = channels_result.discord.len();

    // Check for provider keys
    let (providers_report, _) = providers::import_providers(detection);
    let providers_available = providers_report.items_imported > 0;

    let imported_agents = agents::import_agents(detection);

    ImportScan {
        identity_available,
        identity_agent_name: previewed_identity.agent_name,
        identity_theme: previewed_identity.theme,
        identity_user_name: previewed_identity.user_name,
        providers_available,
        skills_count: skills.len(),
        memory_available: detection.has_memory,
        memory_files_count,
        channels_available: telegram_accounts > 0 || discord_accounts > 0,
        telegram_accounts,
        discord_accounts,
        sessions_count: detection.session_count,
        unsupported_channels: detection.unsupported_channels.clone(),
        agent_ids: detection.agent_ids.clone(),
        agents: imported_agents.agents,
    }
}

/// Perform a selective import from OpenClaw into Moltis.
///
/// Each category is independent — partial failures don't block others.
/// Returns a detailed report of what was imported, skipped, and failed.
pub fn import(
    detection: &OpenClawDetection,
    selection: &ImportSelection,
    config_dir: &Path,
    data_dir: &Path,
) -> ImportReport {
    let mut report = ImportReport::new();

    // Extract agents (always, needed for session mapping and per-agent memory)
    let imported_agents = agents::import_agents(detection);
    let agent_id_mapping: std::collections::HashMap<String, String> = imported_agents
        .agents
        .iter()
        .map(|a| (a.openclaw_id.clone(), a.moltis_id.clone()))
        .collect();
    report.imported_agents = Some(imported_agents.clone());

    // Identity
    if selection.identity {
        let (cat_report, imported_identity) = identity::import_identity(detection);
        if cat_report.items_imported > 0
            && let Err(e) = persist_identity(&imported_identity, config_dir)
        {
            warn!("failed to persist identity to config: {e}");
        }
        report.imported_identity = Some(imported_identity);
        report.add_category(cat_report);
    }

    // Providers
    if selection.providers {
        let (mut cat_report, imported_providers) = providers::import_providers(detection);
        let mut write_errors = Vec::new();

        if !imported_providers.providers.is_empty() {
            let keys_path = config_dir.join("provider_keys.json");
            if let Err(e) =
                providers::write_provider_keys(&imported_providers.providers, &keys_path)
            {
                write_errors.push(format!("failed to write provider keys: {e}"));
            }
        }

        if !imported_providers.oauth_tokens.is_empty()
            && let Err(e) = providers::write_oauth_tokens_to_path(
                &imported_providers.oauth_tokens,
                &config_dir.join("oauth_tokens.json"),
            )
        {
            write_errors.push(format!("failed to write OAuth tokens: {e}"));
        }

        if write_errors.is_empty() {
            report.add_category(cat_report);
        } else {
            cat_report.errors.extend(write_errors);
            cat_report.status = if cat_report.items_imported > 0 {
                ImportStatus::Partial
            } else {
                ImportStatus::Failed
            };
            report.add_category(cat_report);
        }
    }

    // Skills
    if selection.skills {
        let skills_dir = data_dir.join("skills");
        report.add_category(skills::import_skills(detection, &skills_dir));
    }

    // Memory (default agent)
    if selection.memory {
        report.add_category(memory::import_memory(detection, data_dir));

        // Per-agent memory for non-default agents
        for agent in imported_agents.agents.iter().filter(|a| !a.is_default) {
            if let Some(ref source_ws) = agent.source_workspace
                && (source_ws.join("MEMORY.md").is_file() || source_ws.join("memory").is_dir())
            {
                let agent_data_dir = data_dir.join("agents").join(&agent.moltis_id);
                let agent_report = memory::import_agent_memory(source_ws, &agent_data_dir);
                if agent_report.items_imported > 0 {
                    debug!(
                        agent = %agent.moltis_id,
                        imported = agent_report.items_imported,
                        "imported per-agent memory"
                    );
                }
                report.add_category(agent_report);
            }
        }
    }

    // Channels
    if selection.channels {
        let (cat_report, imported_channels) = channels::import_channels(detection);
        if (!imported_channels.telegram.is_empty() || !imported_channels.discord.is_empty())
            && let Err(e) = persist_channels(&imported_channels, config_dir)
        {
            warn!("failed to persist channels to config: {e}");
        }
        report.imported_channels = Some(imported_channels);
        report.add_category(cat_report);
    }

    // Sessions (all agents)
    if selection.sessions {
        let sessions_dir = data_dir.join("sessions");
        let memory_sessions_dir = data_dir.join("memory").join("sessions");
        report.add_category(sessions::import_sessions(
            detection,
            &sessions_dir,
            &memory_sessions_dir,
            &agent_id_mapping,
        ));
    }

    // Always add TODO items for unsupported features
    add_todos(&mut report, detection);

    // Save import state
    let state_path = data_dir.join("openclaw-import-state.json");
    let _ = save_import_state(&state_path, &report);

    report
}

/// Run an incremental import of sessions only.
///
/// This is used by the file watcher to sync new/changed sessions without
/// re-running the full import (identity, providers, skills, etc.).
pub fn import_sessions_only(detection: &OpenClawDetection, data_dir: &Path) -> CategoryReport {
    let sessions_dir = data_dir.join("sessions");
    let memory_sessions_dir = data_dir.join("memory").join("sessions");
    // Build agent mapping for session key namespacing
    let imported_agents = agents::import_agents(detection);
    let agent_id_mapping: std::collections::HashMap<String, String> = imported_agents
        .agents
        .iter()
        .map(|a| (a.openclaw_id.clone(), a.moltis_id.clone()))
        .collect();
    sessions::import_sessions(
        detection,
        &sessions_dir,
        &memory_sessions_dir,
        &agent_id_mapping,
    )
}

/// Persistent import state for idempotency tracking.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportState {
    pub last_import_at: Option<u64>,
    pub categories_imported: Vec<ImportCategory>,
}

/// Load previously saved import state.
pub fn load_import_state(data_dir: &Path) -> Option<ImportState> {
    let path = data_dir.join("openclaw-import-state.json");
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn save_import_state(path: &Path, report: &ImportReport) -> error::Result<()> {
    let imported: Vec<ImportCategory> = report
        .categories
        .iter()
        .filter(|c| c.status == ImportStatus::Success || c.status == ImportStatus::Partial)
        .map(|c| c.category)
        .collect();

    let state = ImportState {
        last_import_at: Some(now_ms()),
        categories_imported: imported,
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(&state)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Persist imported identity data to `moltis.toml`.
///
/// Loads any existing config, merges identity and timezone, and writes back.
fn persist_identity(imported: &identity::ImportedIdentity, config_dir: &Path) -> error::Result<()> {
    let config_path = config_dir.join("moltis.toml");
    let mut config = load_or_default_config(&config_path);

    info!(
        config_path = %config_path.display(),
        existing_name = ?config.identity.name,
        existing_theme = ?config.identity.theme,
        imported_name = ?imported.agent_name,
        imported_theme = ?imported.theme,
        imported_user_name = ?imported.user_name,
        "openclaw persist_identity: loaded existing config"
    );

    if let Some(ref name) = imported.agent_name {
        info!(name, "openclaw persist_identity: writing agent name");
        config.identity.name = Some(name.clone());
    }

    if config.identity.theme.is_none()
        && let Some(ref theme) = imported.theme
    {
        info!(theme, "openclaw persist_identity: writing theme");
        config.identity.theme = Some(theme.clone());
    }

    if let Some(ref tz_str) = imported.user_timezone {
        if let Ok(tz) = tz_str.parse::<moltis_config::Timezone>() {
            debug!(timezone = tz_str, "persisting user timezone to moltis.toml");
            config.user.timezone = Some(tz);
        } else {
            warn!(timezone = tz_str, "unknown timezone, skipping");
        }
    }

    if let Some(ref user_name) = imported.user_name {
        debug!(user_name, "persisting user name to moltis.toml");
        config.user.name = Some(user_name.clone());
    }

    save_config_to_path(&config_path, &config)
}

/// Persist imported channel configs to `[channels.*]` in `moltis.toml`.
fn persist_channels(imported: &channels::ImportedChannels, config_dir: &Path) -> error::Result<()> {
    let config_path = config_dir.join("moltis.toml");
    let mut config = load_or_default_config(&config_path);

    for ch in &imported.telegram {
        ensure_channel_offered(&mut config.channels.offered, "telegram");
        let allowlist: Vec<String> = ch.allowed_users.iter().map(|id| id.to_string()).collect();

        // Map OpenClaw dm_policy to Moltis format (default to "allowlist")
        let dm_policy = match ch.dm_policy.as_deref() {
            Some("pairing") => "pairing",
            Some("otp") => "otp",
            Some("open") => "open",
            Some("disabled") => "disabled",
            _ => "allowlist",
        };

        let value = serde_json::json!({
            "token": ch.bot_token,
            "dm_policy": dm_policy,
            "allowlist": allowlist,
        });

        debug!(account_id = %ch.account_id, "persisting Telegram channel to moltis.toml");
        config
            .channels
            .telegram
            .insert(ch.account_id.clone(), value);
    }

    for ch in &imported.discord {
        ensure_channel_offered(&mut config.channels.offered, "discord");

        let dm_policy = map_discord_dm_policy(ch.dm_policy.as_deref());
        let group_policy = map_discord_group_policy(ch.group_policy.as_deref());
        let mention_mode = map_discord_mention_mode(ch.mention_mode.as_deref());

        let value = serde_json::json!({
            "token": ch.token,
            "dm_policy": dm_policy,
            "group_policy": group_policy,
            "mention_mode": mention_mode,
            "allowlist": ch.allowlist,
            "guild_allowlist": ch.guild_allowlist,
        });

        debug!(account_id = %ch.account_id, "persisting Discord channel to moltis.toml");
        config.channels.discord.insert(ch.account_id.clone(), value);
    }

    save_config_to_path(&config_path, &config)
}

fn ensure_channel_offered(offered: &mut Vec<String>, channel: &str) {
    if !offered
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(channel))
    {
        offered.push(channel.to_string());
    }
}

fn map_discord_dm_policy(policy: Option<&str>) -> &'static str {
    match policy {
        Some("open") => "open",
        Some("disabled") => "disabled",
        Some("allowlist") | Some("pairing") | Some("otp") => "allowlist",
        _ => "allowlist",
    }
}

fn map_discord_group_policy(policy: Option<&str>) -> &'static str {
    match policy {
        Some("allowlist") => "allowlist",
        Some("disabled") => "disabled",
        _ => "open",
    }
}

fn map_discord_mention_mode(mode: Option<&str>) -> &'static str {
    match mode {
        Some("always") => "always",
        Some("none") => "none",
        _ => "mention",
    }
}

/// Load a `MoltisConfig` from a TOML file, or return defaults if not found.
fn load_or_default_config(path: &Path) -> moltis_config::MoltisConfig {
    if !path.is_file() {
        return moltis_config::MoltisConfig::default();
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return moltis_config::MoltisConfig::default();
    };
    toml::from_str(&content).unwrap_or_default()
}

/// Serialize a `MoltisConfig` to TOML and write it to the given path.
fn save_config_to_path(path: &Path, config: &moltis_config::MoltisConfig) -> error::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let toml_str = toml::to_string_pretty(config)?;
    std::fs::write(path, toml_str)?;
    Ok(())
}

fn add_todos(report: &mut ImportReport, detection: &OpenClawDetection) {
    report.add_todo(
        "Sub-agents",
        "OpenClaw's agent delegation/sub-agent spawning is not yet supported in Moltis.",
    );

    for channel in &detection.unsupported_channels {
        report.add_todo(
            format!("{channel} channel"),
            format!("The {channel} channel is not yet implemented in Moltis."),
        );
    }

    if detection.has_memory {
        report.add_todo(
            "Vector embeddings",
            "OpenClaw's SQLite embedding database is not portable across embedding models. Memory files were imported but re-indexing may be needed.",
        );
    }

    report.add_todo(
        "Tool policies",
        "OpenClaw's tool policy format differs from Moltis's configuration.",
    );
}

fn count_memory_files(workspace_dir: &Path) -> usize {
    let daily_dir = workspace_dir.join("memory");
    if !daily_dir.is_dir() {
        return 0;
    }
    std::fs::read_dir(&daily_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
                .count()
        })
        .unwrap_or(0)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn setup_full_openclaw(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();

        // Config
        std::fs::write(
            dir.join("openclaw.json"),
            r#"{
                "agents": {
                    "defaults": {
                        "model": {"primary": "anthropic/claude-opus-4-6"},
                        "userTimezone": "America/New_York",
                        "userName": "Penso"
                    },
                    "list": [{"id": "main", "default": true, "name": "Claude"}]
                },
                "ui": {"assistant": {"name": "Claude", "creature": "owl", "vibe": "wise"}},
                "channels": {
                    "telegram": {"botToken": "123:ABC", "allowFrom": [111]}
                }
            }"#,
        )
        .unwrap();

        // Auth profiles
        let agent_dir = dir.join("agents").join("main").join("agent");
        std::fs::create_dir_all(agent_dir.join("sessions")).unwrap();
        std::fs::write(
            agent_dir.join("auth-profiles.json"),
            r#"{"version":1,"profiles":{"anth":{"type":"api_key","provider":"anthropic","key":"sk-test"}}}"#,
        )
        .unwrap();

        // Session
        std::fs::write(
            agent_dir.join("sessions").join("main.jsonl"),
            r#"{"type":"message","message":{"role":"user","content":"Hello"}}"#,
        )
        .unwrap();

        // Workspace
        let ws = dir.join("workspace");
        std::fs::create_dir_all(ws.join("memory")).unwrap();
        std::fs::create_dir_all(ws.join("skills").join("test-skill")).unwrap();
        std::fs::write(ws.join("MEMORY.md"), "# Memory").unwrap();
        std::fs::write(ws.join("memory").join("2024-01-15.md"), "log").unwrap();
        std::fs::write(
            ws.join("skills").join("test-skill").join("SKILL.md"),
            "---\nname: test-skill\n---\nDo stuff.",
        )
        .unwrap();
    }

    #[test]
    fn scan_returns_available_data() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".openclaw");
        setup_full_openclaw(&home);

        let detection = detect::detect_at(home).unwrap();
        let scan_result = scan(&detection);

        assert!(scan_result.identity_available);
        assert!(scan_result.providers_available);
        assert_eq!(scan_result.skills_count, 1);
        assert!(scan_result.memory_available);
        assert_eq!(scan_result.memory_files_count, 1);
        assert!(scan_result.channels_available);
        assert_eq!(scan_result.telegram_accounts, 1);
        assert_eq!(scan_result.discord_accounts, 0);
        assert_eq!(scan_result.sessions_count, 1);
    }

    #[test]
    fn scan_counts_discord_accounts() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".openclaw");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            home.join("openclaw.json"),
            r#"{
                "channels": {
                    "discord": {
                        "accounts": {
                            "bot1": {"token": "Bot token-1"},
                            "bot2": {"token": "Bot token-2"}
                        }
                    }
                }
            }"#,
        )
        .unwrap();

        let detection = detect::detect_at(home).unwrap();
        let scan_result = scan(&detection);
        assert!(scan_result.channels_available);
        assert_eq!(scan_result.telegram_accounts, 0);
        assert_eq!(scan_result.discord_accounts, 2);
    }

    #[test]
    fn scan_marks_oauth_only_provider_as_available() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".openclaw");

        let agent_dir = home.join("agents").join("main").join("agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("auth-profiles.json"),
            r#"{
                "version": 1,
                "profiles": {
                    "codex-main": {
                        "type": "oauth",
                        "provider": "openai-codex",
                        "access": "at-123",
                        "refresh": "rt-456"
                    }
                }
            }"#,
        )
        .unwrap();

        let detection = detect::detect_at(home).expect("openclaw install should be detected");
        let scan_result = scan(&detection);
        assert!(scan_result.providers_available);
    }

    #[test]
    fn full_import_all_categories() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".openclaw");
        setup_full_openclaw(&home);

        let config_dir = tmp.path().join("config");
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();

        let detection = detect::detect_at(home).unwrap();
        let report = import(&detection, &ImportSelection::all(), &config_dir, &data_dir);

        // Check that all categories have a report
        assert!(report.categories.len() >= 6);

        // Check specific imports
        assert!(config_dir.join("provider_keys.json").is_file());
        assert!(data_dir.join("MEMORY.md").is_file());
        assert!(
            data_dir
                .join("skills")
                .join("test-skill")
                .join("SKILL.md")
                .is_file()
        );

        // Check import state saved
        assert!(data_dir.join("openclaw-import-state.json").is_file());

        // Check TODOs generated
        assert!(!report.todos.is_empty());
        assert!(report.todos.iter().any(|t| t.feature == "Sub-agents"));
    }

    #[test]
    fn selective_import() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".openclaw");
        setup_full_openclaw(&home);

        let config_dir = tmp.path().join("config");
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();

        let detection = detect::detect_at(home).unwrap();

        // Only import memory
        let selection = ImportSelection {
            memory: true,
            ..Default::default()
        };

        let report = import(&detection, &selection, &config_dir, &data_dir);

        assert_eq!(report.categories.len(), 1);
        assert_eq!(report.categories[0].category, ImportCategory::Memory);

        // Provider keys should NOT be written
        assert!(!config_dir.join("provider_keys.json").exists());
    }

    #[test]
    fn providers_import_writes_oauth_tokens_without_api_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".openclaw");
        let config_dir = tmp.path().join("config");
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();

        let agent_dir = home.join("agents").join("main").join("agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("auth-profiles.json"),
            r#"{
                "version": 1,
                "profiles": {
                    "codex-main": {
                        "type": "oauth",
                        "provider": "openai-codex",
                        "access": "at-123",
                        "refresh": "rt-456"
                    }
                }
            }"#,
        )
        .unwrap();

        let detection = detect::detect_at(home).expect("openclaw install should be detected");
        let selection = ImportSelection {
            providers: true,
            ..Default::default()
        };
        let report = import(&detection, &selection, &config_dir, &data_dir);

        assert_eq!(report.categories.len(), 1);
        assert_eq!(report.categories[0].category, ImportCategory::Providers);
        assert_eq!(report.categories[0].status, ImportStatus::Success);
        assert!(!config_dir.join("provider_keys.json").exists());
        assert!(config_dir.join("oauth_tokens.json").exists());
    }

    #[test]
    fn import_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".openclaw");
        setup_full_openclaw(&home);

        let config_dir = tmp.path().join("config");
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();

        let detection = detect::detect_at(home).unwrap();

        // First import
        let report1 = import(&detection, &ImportSelection::all(), &config_dir, &data_dir);
        let _total1 = report1.total_imported();

        // Second import — should skip most things
        let report2 = import(&detection, &ImportSelection::all(), &config_dir, &data_dir);

        // Skills and sessions should be skipped on second run
        let skills_report = report2
            .categories
            .iter()
            .find(|c| c.category == ImportCategory::Skills);
        if let Some(sr) = skills_report {
            assert!(sr.items_skipped > 0 || sr.items_imported == 0);
        }
    }

    #[test]
    fn load_import_state_works() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path();

        // No state file → None
        assert!(load_import_state(data_dir).is_none());

        // Write state
        let state = ImportState {
            last_import_at: Some(12345),
            categories_imported: vec![ImportCategory::Memory, ImportCategory::Skills],
        };
        let json = serde_json::to_string(&state).unwrap();
        std::fs::write(data_dir.join("openclaw-import-state.json"), json).unwrap();

        let loaded = load_import_state(data_dir).unwrap();
        assert_eq!(loaded.last_import_at, Some(12345));
        assert_eq!(loaded.categories_imported.len(), 2);
    }

    #[test]
    fn import_persists_identity_to_config() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".openclaw");
        setup_full_openclaw(&home);

        let config_dir = tmp.path().join("config");
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();

        let detection = detect::detect_at(home).unwrap();
        let selection = ImportSelection {
            identity: true,
            ..Default::default()
        };
        let report = import(&detection, &selection, &config_dir, &data_dir);

        // Identity should be persisted to moltis.toml
        let config_path = config_dir.join("moltis.toml");
        assert!(config_path.is_file(), "moltis.toml should be created");

        let content = std::fs::read_to_string(&config_path).unwrap();
        let config: moltis_config::MoltisConfig = toml::from_str(&content).unwrap();

        assert_eq!(config.identity.name.as_deref(), Some("Claude"));
        assert_eq!(config.identity.theme.as_deref(), Some("wise owl"));
        assert_eq!(config.user.name.as_deref(), Some("Penso"));
        assert_eq!(
            config.user.timezone.as_ref().map(|t| t.name()),
            Some("America/New_York")
        );

        // Report should include imported identity
        assert!(report.imported_identity.is_some());
        let id = report.imported_identity.unwrap();
        assert_eq!(id.agent_name.as_deref(), Some("Claude"));
        assert_eq!(id.theme.as_deref(), Some("wise owl"));
        assert_eq!(id.user_name.as_deref(), Some("Penso"));
        assert_eq!(id.user_timezone.as_deref(), Some("America/New_York"));
    }

    #[test]
    fn import_persists_channels_to_config() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".openclaw");
        setup_full_openclaw(&home);

        let config_dir = tmp.path().join("config");
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();

        let detection = detect::detect_at(home).unwrap();
        let selection = ImportSelection {
            channels: true,
            ..Default::default()
        };
        let report = import(&detection, &selection, &config_dir, &data_dir);

        // Channels should be persisted to moltis.toml
        let config_path = config_dir.join("moltis.toml");
        assert!(config_path.is_file(), "moltis.toml should be created");

        let content = std::fs::read_to_string(&config_path).unwrap();
        let config: moltis_config::MoltisConfig = toml::from_str(&content).unwrap();

        assert!(
            !config.channels.telegram.is_empty(),
            "telegram channels should be populated"
        );
        assert!(
            config
                .channels
                .offered
                .iter()
                .any(|channel| channel == "telegram")
        );
        let entry = config.channels.telegram.get("default").unwrap();
        assert_eq!(entry["token"].as_str(), Some("123:ABC"));
        assert_eq!(entry["dm_policy"].as_str(), Some("allowlist"));

        // Allowlist should contain the user ID
        let allowlist = entry["allowlist"].as_array().unwrap();
        assert!(allowlist.iter().any(|v| v.as_str() == Some("111")));

        // Report should include imported channels
        assert!(report.imported_channels.is_some());
        let ch = report.imported_channels.unwrap();
        assert_eq!(ch.telegram.len(), 1);
        assert_eq!(ch.telegram[0].bot_token, "123:ABC");
    }

    #[test]
    fn import_persists_discord_channels_to_config() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".openclaw");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            home.join("openclaw.json"),
            r#"{
                "channels": {
                    "discord": {
                        "token": "Bot xyz",
                        "dmPolicy": "pairing",
                        "groupPolicy": "allowlist",
                        "mentionMode": "always",
                        "allowFrom": [111, "user-222"],
                        "guildAllowlist": [333]
                    }
                }
            }"#,
        )
        .unwrap();

        let config_dir = tmp.path().join("config");
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();

        let detection = detect::detect_at(home).unwrap();
        let selection = ImportSelection {
            channels: true,
            ..Default::default()
        };
        let report = import(&detection, &selection, &config_dir, &data_dir);

        let config_path = config_dir.join("moltis.toml");
        assert!(config_path.is_file(), "moltis.toml should be created");

        let content = std::fs::read_to_string(&config_path).unwrap();
        let config: moltis_config::MoltisConfig = toml::from_str(&content).unwrap();

        assert!(
            config
                .channels
                .offered
                .iter()
                .any(|channel| channel == "discord")
        );
        let entry = config.channels.discord.get("default").unwrap();
        assert_eq!(entry["token"].as_str(), Some("Bot xyz"));
        assert_eq!(entry["dm_policy"].as_str(), Some("allowlist"));
        assert_eq!(entry["group_policy"].as_str(), Some("allowlist"));
        assert_eq!(entry["mention_mode"].as_str(), Some("always"));
        assert!(
            entry["allowlist"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value.as_str() == Some("111"))
        );
        assert!(
            entry["allowlist"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value.as_str() == Some("user-222"))
        );
        assert!(
            entry["guild_allowlist"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value.as_str() == Some("333"))
        );

        assert!(report.imported_channels.is_some());
        let channels = report.imported_channels.unwrap();
        assert_eq!(channels.telegram.len(), 0);
        assert_eq!(channels.discord.len(), 1);
        assert_eq!(channels.discord[0].token, "Bot xyz");
    }

    #[test]
    fn import_merges_identity_with_existing_config() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".openclaw");
        setup_full_openclaw(&home);

        let config_dir = tmp.path().join("config");
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();

        // Pre-existing config with theme already set
        let existing = moltis_config::MoltisConfig {
            identity: moltis_config::AgentIdentity {
                theme: Some("chill cat".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&existing).unwrap();
        std::fs::write(config_dir.join("moltis.toml"), &toml_str).unwrap();

        let detection = detect::detect_at(home).unwrap();
        let selection = ImportSelection {
            identity: true,
            ..Default::default()
        };
        import(&detection, &selection, &config_dir, &data_dir);

        let content = std::fs::read_to_string(config_dir.join("moltis.toml")).unwrap();
        let config: moltis_config::MoltisConfig = toml::from_str(&content).unwrap();

        // Imported name should be set
        assert_eq!(config.identity.name.as_deref(), Some("Claude"));
        // Pre-existing theme should be preserved (not overwritten)
        assert_eq!(config.identity.theme.as_deref(), Some("chill cat"));
    }

    #[test]
    fn full_import_persists_identity_and_channels() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".openclaw");
        setup_full_openclaw(&home);

        let config_dir = tmp.path().join("config");
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();

        let detection = detect::detect_at(home).unwrap();
        let report = import(&detection, &ImportSelection::all(), &config_dir, &data_dir);

        // moltis.toml should contain both identity and channels
        let content = std::fs::read_to_string(config_dir.join("moltis.toml")).unwrap();
        let config: moltis_config::MoltisConfig = toml::from_str(&content).unwrap();

        assert_eq!(config.identity.name.as_deref(), Some("Claude"));
        assert_eq!(config.user.name.as_deref(), Some("Penso"));
        assert!(!config.channels.telegram.is_empty());

        // Report should have both
        assert!(report.imported_identity.is_some());
        assert!(report.imported_channels.is_some());
    }
}
