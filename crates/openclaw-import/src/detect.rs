//! Detection of an existing OpenClaw installation.

use std::path::{Path, PathBuf};

use tracing::{debug, info};

/// Result of scanning for an OpenClaw installation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct OpenClawDetection {
    /// Root directory (`~/.openclaw/` or `OPENCLAW_HOME`).
    pub home_dir: PathBuf,
    /// Whether `openclaw.json` exists.
    pub has_config: bool,
    /// Whether agent auth-profiles exist (credentials).
    pub has_credentials: bool,
    /// Whether `mcp-servers.json` exists at the root.
    pub has_mcp_servers: bool,
    /// Resolved workspace directory (respects `OPENCLAW_PROFILE`).
    pub workspace_dir: PathBuf,
    /// Whether the workspace has `MEMORY.md` or `memory/` directory.
    pub has_memory: bool,
    /// Whether workspace or managed skills directories exist.
    pub has_skills: bool,
    /// Agent IDs discovered under `agents/`.
    pub agent_ids: Vec<String>,
    /// Total session file count across all agents.
    pub session_count: usize,
    /// Names of configured but unsupported channels.
    pub unsupported_channels: Vec<String>,
    /// Whether the workspace has any personality files (SOUL.md, IDENTITY.md, etc.).
    pub has_workspace_files: bool,
    /// Names of workspace personality files found.
    pub workspace_files_found: Vec<String>,
}

/// Detect an OpenClaw installation.
///
/// Checks `OPENCLAW_HOME` env var first, then `~/.openclaw/`.
/// Returns `None` if the directory does not exist.
pub fn detect() -> Option<OpenClawDetection> {
    let home = resolve_home_dir();
    match &home {
        Some(path) => info!(path = %path.display(), "openclaw detect: resolved home directory"),
        None => info!(
            "openclaw detect: could not resolve home directory (dirs_next::home_dir returned None)"
        ),
    }
    detect_at(home?)
}

/// Detect OpenClaw at a specific directory (for testing).
pub fn detect_at(home_dir: PathBuf) -> Option<OpenClawDetection> {
    if !home_dir.is_dir() {
        info!(path = %home_dir.display(), "openclaw detect: home directory does not exist or is not a directory");
        return None;
    }

    info!(path = %home_dir.display(), "openclaw detect: home directory found");

    let has_config = home_dir.join("openclaw.json").is_file();
    let has_mcp_servers = home_dir.join("mcp-servers.json").is_file();

    let workspace_dir = resolve_workspace_dir(&home_dir);
    let workspace_exists = workspace_dir.is_dir();
    let has_memory =
        workspace_dir.join("MEMORY.md").is_file() || workspace_dir.join("memory").is_dir();

    let has_skills = home_dir.join("skills").is_dir() || workspace_dir.join("skills").is_dir();

    let (agent_ids, session_count, has_credentials) = scan_agents(&home_dir);

    let unsupported_channels = if has_config {
        scan_unsupported_channels(&home_dir)
    } else {
        Vec::new()
    };

    let workspace_file_names = [
        "SOUL.md",
        "IDENTITY.md",
        "USER.md",
        "TOOLS.md",
        "AGENTS.md",
        "HEARTBEAT.md",
        "BOOT.md",
    ];
    let workspace_files_found: Vec<String> = workspace_file_names
        .iter()
        .filter(|name| workspace_dir.join(name).is_file())
        .map(|name| (*name).to_string())
        .collect();
    let has_workspace_files = !workspace_files_found.is_empty();

    info!(
        path = %home_dir.display(),
        workspace = %workspace_dir.display(),
        workspace_exists,
        has_config,
        has_credentials,
        has_mcp_servers,
        has_memory,
        has_skills,
        has_workspace_files,
        workspace_files = ?workspace_files_found,
        agent_count = agent_ids.len(),
        session_count,
        "openclaw detect: scan complete"
    );

    Some(OpenClawDetection {
        home_dir,
        has_config,
        has_credentials,
        has_mcp_servers,
        workspace_dir,
        has_memory,
        has_skills,
        agent_ids,
        session_count,
        unsupported_channels,
        has_workspace_files,
        workspace_files_found,
    })
}

/// Resolve the OpenClaw home directory from env or default.
fn resolve_home_dir() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("OPENCLAW_HOME") {
        let path = PathBuf::from(home);
        if path.is_dir() {
            debug!(path = %path.display(), "openclaw detect: using OPENCLAW_HOME env var");
            return Some(path);
        }
        info!(path = %path.display(), "openclaw detect: OPENCLAW_HOME set but directory does not exist");
    }

    let home = dirs_next::home_dir();
    match &home {
        Some(h) => debug!(home = %h.display(), "openclaw detect: resolved user home directory"),
        None => info!(
            "openclaw detect: dirs_next::home_dir() returned None — cannot determine home directory"
        ),
    }
    home.map(|h| h.join(".openclaw"))
}

/// Resolve the workspace directory.
///
/// Resolution order:
/// 1. `OPENCLAW_PROFILE` env → `<home>/workspace-<profile>`
/// 2. `workspace` field from `openclaw.json` config (exact path)
/// 3. Remap: basename of configured workspace under `~/` (handles cross-machine paths like `/root/clawd` → `~/clawd`)
/// 4. Default `<home>/workspace` (if it exists)
/// 5. Well-known `~/clawd` directory (fallback for non-standard setups)
/// 6. Default `<home>/workspace` (even if absent, for logging)
fn resolve_workspace_dir(home: &Path) -> PathBuf {
    // 1. Explicit profile env var
    if let Some(p) = std::env::var("OPENCLAW_PROFILE")
        .ok()
        .filter(|p| !p.is_empty())
    {
        let profiled = home.join(format!("workspace-{p}"));
        debug!(path = %profiled.display(), "openclaw detect: workspace from OPENCLAW_PROFILE");
        return profiled;
    }

    // 2–3. Read workspace from config
    if let Some(configured) = read_config_workspace(home) {
        let configured_path = PathBuf::from(&configured);

        // 2. Exact configured path
        if configured_path.is_dir() {
            debug!(path = %configured_path.display(), "openclaw detect: workspace from config (exact)");
            return configured_path;
        }

        // 3. Remap basename under user home (cross-machine: /root/clawd → ~/clawd)
        if let Some(basename) = configured_path.file_name()
            && let Some(user_home) = dirs_next::home_dir()
        {
            let remapped = user_home.join(basename);
            if remapped.is_dir() {
                info!(
                    configured = %configured_path.display(),
                    remapped = %remapped.display(),
                    "openclaw detect: workspace remapped from config basename"
                );
                return remapped;
            }
        }

        debug!(
            configured = configured,
            "openclaw detect: configured workspace not found, trying fallbacks"
        );
    }

    // 4. Default location (if it exists)
    let default_ws = home.join("workspace");
    if default_ws.is_dir() {
        return default_ws;
    }

    // 5. Well-known ~/clawd directory (only when a config exists,
    //    indicating a real OpenClaw installation)
    let has_config = home.join("openclaw.json").is_file();
    if has_config && let Some(user_home) = dirs_next::home_dir() {
        let clawd = user_home.join("clawd");
        if clawd.is_dir() {
            info!(path = %clawd.display(), "openclaw detect: workspace found at ~/clawd");
            return clawd;
        }
    }

    // 6. Default (even if absent — callers check is_dir())
    default_ws
}

/// Read the `agents.defaults.workspace` field from `openclaw.json`.
fn read_config_workspace(home: &Path) -> Option<String> {
    let config_path = home.join("openclaw.json");
    let content = std::fs::read_to_string(&config_path).ok()?;
    let config: crate::types::OpenClawConfig = json5::from_str(&content).ok()?;
    config.agents.defaults.workspace.filter(|w| !w.is_empty())
}

/// Resolve the sessions directory for one agent, supporting both historical
/// and current OpenClaw layouts.
pub(crate) fn resolve_agent_sessions_dir(agent_dir: &Path) -> Option<PathBuf> {
    let nested = agent_dir.join("agent").join("sessions");
    if nested.is_dir() {
        return Some(nested);
    }

    let flat = agent_dir.join("sessions");
    if flat.is_dir() {
        return Some(flat);
    }

    None
}

/// Resolve auth-profiles path for one agent, supporting both historical and
/// current OpenClaw layouts.
pub(crate) fn resolve_agent_auth_profiles_path(agent_dir: &Path) -> Option<PathBuf> {
    let nested = agent_dir.join("agent").join("auth-profiles.json");
    if nested.is_file() {
        return Some(nested);
    }

    let flat = agent_dir.join("auth-profiles.json");
    if flat.is_file() {
        return Some(flat);
    }

    None
}

/// Scan `agents/` directory for agent IDs, session counts, and credentials.
fn scan_agents(home: &Path) -> (Vec<String>, usize, bool) {
    let agents_dir = home.join("agents");
    let mut agent_ids = Vec::new();
    let mut session_count = 0;
    let mut has_credentials = false;

    let Ok(entries) = std::fs::read_dir(&agents_dir) else {
        return (agent_ids, session_count, has_credentials);
    };

    for entry in entries.flatten() {
        let agent_dir = entry.path();
        if !agent_dir.is_dir() {
            continue;
        }
        if let Some(name) = agent_dir.file_name().and_then(|n| n.to_str()) {
            agent_ids.push(name.to_string());

            // Count sessions
            if let Some(sessions_dir) = resolve_agent_sessions_dir(&agent_dir)
                && let Ok(session_entries) = std::fs::read_dir(&sessions_dir)
            {
                session_count += session_entries
                    .flatten()
                    .filter(|e| e.path().extension().is_some_and(|ext| ext == "jsonl"))
                    .count();
            }

            // Check for auth-profiles.json
            if resolve_agent_auth_profiles_path(&agent_dir).is_some() {
                has_credentials = true;
            }
        }
    }

    agent_ids.sort();
    (agent_ids, session_count, has_credentials)
}

/// Scan the config for unsupported channel names.
fn scan_unsupported_channels(home: &Path) -> Vec<String> {
    let config_path = home.join("openclaw.json");
    let Ok(content) = std::fs::read_to_string(&config_path) else {
        return Vec::new();
    };
    let Ok(config) = json5::from_str::<crate::types::OpenClawConfig>(&content) else {
        return Vec::new();
    };

    let mut unsupported = Vec::new();
    if config.channels.whatsapp.is_some() {
        unsupported.push("whatsapp".to_string());
    }
    if config.channels.slack.is_some() {
        unsupported.push("slack".to_string());
    }
    if config.channels.signal.is_some() {
        unsupported.push("signal".to_string());
    }
    if config.channels.imessage.is_some() {
        unsupported.push("imessage".to_string());
    }
    unsupported
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn setup_openclaw_dir(dir: &Path) {
        // Create minimal OpenClaw directory structure
        std::fs::create_dir_all(dir.join("workspace").join("memory")).unwrap();
        std::fs::create_dir_all(dir.join("workspace").join("skills").join("my-skill")).unwrap();
        std::fs::create_dir_all(dir.join("skills").join("managed-skill")).unwrap();
        std::fs::create_dir_all(
            dir.join("agents")
                .join("main")
                .join("agent")
                .join("sessions"),
        )
        .unwrap();

        // Config
        std::fs::write(
            dir.join("openclaw.json"),
            r#"{"agents":{"defaults":{"model":{"primary":"anthropic/claude-opus-4-6"}}}}"#,
        )
        .unwrap();

        // Memory
        std::fs::write(dir.join("workspace").join("MEMORY.md"), "# Memory\n").unwrap();
        std::fs::write(
            dir.join("workspace").join("memory").join("2024-01-15.md"),
            "daily log",
        )
        .unwrap();

        // Skill
        std::fs::write(
            dir.join("workspace")
                .join("skills")
                .join("my-skill")
                .join("SKILL.md"),
            "---\nname: my-skill\n---\nInstructions here.",
        )
        .unwrap();

        // Session
        std::fs::write(
            dir.join("agents")
                .join("main")
                .join("agent")
                .join("sessions")
                .join("main.jsonl"),
            r#"{"type":"message","message":{"role":"user","content":"Hello"}}"#,
        )
        .unwrap();

        // Auth profiles
        std::fs::write(
            dir.join("agents")
                .join("main")
                .join("agent")
                .join("auth-profiles.json"),
            r#"{"version":1,"profiles":{}}"#,
        )
        .unwrap();
    }

    #[test]
    fn detect_at_valid_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".openclaw");
        setup_openclaw_dir(&home);

        let detection = detect_at(home).expect("should detect");
        assert!(detection.has_config);
        assert!(detection.has_memory);
        assert!(detection.has_skills);
        assert!(detection.has_credentials);
        assert_eq!(detection.agent_ids, vec!["main"]);
        assert_eq!(detection.session_count, 1);
    }

    #[test]
    fn detect_at_flat_agent_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".openclaw");
        std::fs::create_dir_all(home.join("agents").join("main").join("sessions")).unwrap();
        std::fs::write(
            home.join("agents")
                .join("main")
                .join("sessions")
                .join("main.jsonl"),
            r#"{"type":"message","message":{"role":"user","content":"Hello"}}"#,
        )
        .unwrap();
        std::fs::write(
            home.join("agents").join("main").join("auth-profiles.json"),
            r#"{"version":1,"profiles":{}}"#,
        )
        .unwrap();

        let detection = detect_at(home).expect("should detect");
        assert!(detection.has_credentials);
        assert_eq!(detection.agent_ids, vec!["main"]);
        assert_eq!(detection.session_count, 1);
    }

    #[test]
    fn detect_at_missing_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let result = detect_at(tmp.path().join("nonexistent"));
        assert!(result.is_none());
    }

    #[test]
    fn detect_at_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".openclaw");
        std::fs::create_dir_all(&home).unwrap();

        let detection = detect_at(home).expect("should detect even if empty");
        assert!(!detection.has_config);
        assert!(!detection.has_memory);
        assert!(!detection.has_skills);
        assert!(!detection.has_credentials);
        assert!(detection.agent_ids.is_empty());
        assert_eq!(detection.session_count, 0);
    }

    #[test]
    fn unsupported_channels_detected() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".openclaw");
        std::fs::create_dir_all(home.join("workspace")).unwrap();
        std::fs::write(
            home.join("openclaw.json"),
            r#"{"channels":{"whatsapp":{"enabled":true},"discord":{"token":"x"}}}"#,
        )
        .unwrap();

        let detection = detect_at(home).expect("should detect");
        assert!(
            detection
                .unsupported_channels
                .contains(&"whatsapp".to_string())
        );
        assert!(
            !detection
                .unsupported_channels
                .contains(&"discord".to_string())
        );
    }

    #[test]
    fn workspace_profile_resolution() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();

        // Without profile — should use "workspace"
        let ws = resolve_workspace_dir(&home);
        assert_eq!(ws, home.join("workspace"));
    }

    #[test]
    fn workspace_from_config() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".openclaw");
        std::fs::create_dir_all(&home).unwrap();

        // Create a workspace directory and point the config at it
        let ws_dir = tmp.path().join("my-workspace");
        std::fs::create_dir_all(&ws_dir).unwrap();
        std::fs::write(ws_dir.join("MEMORY.md"), "# Memory\n").unwrap();

        std::fs::write(
            home.join("openclaw.json"),
            format!(
                r#"{{"agents":{{"defaults":{{"workspace":"{}"}}}}}}"#,
                ws_dir.display()
            ),
        )
        .unwrap();

        let ws = resolve_workspace_dir(&home);
        assert_eq!(ws, ws_dir);
    }

    #[test]
    fn workspace_from_config_remaps_cross_machine_path() {
        // Simulates: config says /root/clawd but ~/clawd exists locally
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".openclaw");
        std::fs::create_dir_all(&home).unwrap();

        // Config points to a non-existent absolute path
        std::fs::write(
            home.join("openclaw.json"),
            r#"{"agents":{"defaults":{"workspace":"/root/clawd"}}}"#,
        )
        .unwrap();

        // /root/clawd doesn't exist, so should fall back.
        // We can't easily test the ~/clawd remap in unit tests (depends on
        // real home dir), but we verify it doesn't crash and falls back.
        let ws = resolve_workspace_dir(&home);
        // Without ~/clawd existing, it should fall back to default
        assert!(
            ws.to_string_lossy().contains("workspace") || ws.to_string_lossy().contains("clawd")
        );
    }

    #[test]
    fn detect_resolves_workspace_from_config() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".openclaw");
        std::fs::create_dir_all(
            home.join("agents")
                .join("main")
                .join("agent")
                .join("sessions"),
        )
        .unwrap();
        std::fs::write(
            home.join("agents")
                .join("main")
                .join("agent")
                .join("auth-profiles.json"),
            r#"{"version":1,"profiles":{}}"#,
        )
        .unwrap();

        // Create workspace at a custom location
        let ws_dir = tmp.path().join("clawd");
        std::fs::create_dir_all(ws_dir.join("memory")).unwrap();
        std::fs::write(ws_dir.join("MEMORY.md"), "# Memory\n").unwrap();

        // Config points to the custom workspace
        std::fs::write(
            home.join("openclaw.json"),
            format!(
                r#"{{"agents":{{"defaults":{{"workspace":"{}"}}}}}}"#,
                ws_dir.display()
            ),
        )
        .unwrap();

        let detection = detect_at(home).expect("should detect");
        assert_eq!(detection.workspace_dir, ws_dir);
        assert!(detection.has_memory);
    }

    #[test]
    fn detect_workspace_files() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".openclaw");
        std::fs::create_dir_all(home.join("workspace")).unwrap();
        std::fs::write(home.join("workspace").join("SOUL.md"), "# My custom soul").unwrap();
        std::fs::write(home.join("workspace").join("TOOLS.md"), "# Tool guidance").unwrap();

        let detection = detect_at(home).expect("should detect");
        assert!(detection.has_workspace_files);
        assert_eq!(detection.workspace_files_found.len(), 2);
        assert!(
            detection
                .workspace_files_found
                .contains(&"SOUL.md".to_string())
        );
        assert!(
            detection
                .workspace_files_found
                .contains(&"TOOLS.md".to_string())
        );
    }

    #[test]
    fn detect_no_workspace_files() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".openclaw");
        std::fs::create_dir_all(home.join("workspace")).unwrap();

        let detection = detect_at(home).expect("should detect");
        assert!(!detection.has_workspace_files);
        assert!(detection.workspace_files_found.is_empty());
    }
}
