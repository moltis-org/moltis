//! Multi-agent extraction from OpenClaw installations.
//!
//! Reads the `agents.list` array from `openclaw.json` and resolves each
//! agent's workspace, identity metadata (theme), and a
//! sanitized Moltis agent ID.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use {
    serde::{Deserialize, Serialize},
    tracing::{debug, info},
};

use crate::{
    detect::OpenClawDetection,
    identity,
    types::{OpenClawAgentModelConfig, OpenClawConfig},
};

/// Per-agent data extracted from OpenClaw, to be consumed by the gateway.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportedAgent {
    pub openclaw_id: String,
    /// Sanitized Moltis-side agent ID; `"main"` for the default agent.
    pub moltis_id: String,
    pub is_default: bool,
    pub name: Option<String>,
    /// Agent theme (composed from creature/vibe, or explicit theme).
    pub theme: Option<String>,
    /// Resolved source workspace directory for this agent.
    pub source_workspace: Option<PathBuf>,
    /// Model override extracted from the agent entry (e.g. `"anthropic/claude-opus-4-6"`).
    pub model: Option<String>,
}

/// Collection of all agents extracted from an OpenClaw installation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportedAgents {
    pub agents: Vec<ImportedAgent>,
}

/// Sanitize an OpenClaw agent ID into a valid Moltis agent ID.
///
/// - Lowercase, replace non-alphanumeric with `-`, collapse `--`, trim `-`
/// - Truncate to 50 characters
/// - Append `-2`, `-3` etc. if the ID collides with an existing one or `"main"`
pub fn sanitize_agent_id(raw: &str, existing_ids: &HashSet<String>) -> String {
    let lowered: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();

    // Collapse repeated dashes, trim leading/trailing dashes
    let mut collapsed = String::with_capacity(lowered.len());
    let mut prev_dash = true; // treat start as dash to trim leading
    for c in lowered.chars() {
        if c == '-' {
            if !prev_dash {
                collapsed.push('-');
            }
            prev_dash = true;
        } else {
            collapsed.push(c);
            prev_dash = false;
        }
    }
    let trimmed = collapsed.trim_end_matches('-');
    let truncated = if trimmed.len() > 50 {
        &trimmed[..50]
    } else {
        trimmed
    };
    let base = if truncated.is_empty() {
        "agent".to_string()
    } else {
        truncated.to_string()
    };

    // Avoid collision with "main" or existing IDs
    if base != "main" && !existing_ids.contains(&base) {
        return base;
    }

    for suffix in 2..=999 {
        let candidate = format!("{base}-{suffix}");
        if !existing_ids.contains(&candidate) {
            return candidate;
        }
    }

    // Extremely unlikely fallback
    format!("{base}-overflow")
}

/// Extract agent metadata from an OpenClaw installation.
///
/// Reads `openclaw.json` and resolves each agent's workspace, identity,
/// theme. The default agent gets `moltis_id = "main"`.
pub fn import_agents(detection: &OpenClawDetection) -> ImportedAgents {
    let config = identity::load_config(&detection.home_dir);
    let mut agents = Vec::new();
    let mut used_ids: HashSet<String> = HashSet::new();
    used_ids.insert("main".to_string());

    if !config.agents.list.is_empty() {
        agents = extract_from_config_list(&config, detection, &mut used_ids);
    }

    // If config has no agent list but detection found agent dirs, synthesize entries
    if agents.is_empty() && !detection.agent_ids.is_empty() {
        agents = synthesize_from_detection(detection, &mut used_ids);
    }

    let count = agents.len();
    let default_count = agents.iter().filter(|a| a.is_default).count();
    info!(
        agent_count = count,
        default_count, "openclaw agents: extraction complete"
    );

    ImportedAgents { agents }
}

/// Convert non-default imported agents into Moltis [`AgentPreset`] entries.
///
/// Each non-default agent becomes a named preset keyed by `moltis_id`,
/// with identity, model, and a system prompt suffix pointing to the
/// agent's workspace files if they exist.
pub fn agents_to_presets(
    agents: &ImportedAgents,
) -> std::collections::HashMap<String, moltis_config::schema::AgentPreset> {
    let mut presets = std::collections::HashMap::new();

    for agent in &agents.agents {
        if agent.is_default {
            continue;
        }

        let identity = moltis_config::schema::AgentIdentity {
            name: agent.name.clone(),
            emoji: None,
            theme: agent.theme.clone(),
        };

        let preset = moltis_config::schema::AgentPreset {
            identity,
            model: agent.model.clone(),
            ..Default::default()
        };

        debug!(
            moltis_id = %agent.moltis_id,
            name = ?agent.name,
            model = ?agent.model,
            "openclaw agents: created preset from imported agent"
        );

        presets.insert(agent.moltis_id.clone(), preset);
    }

    presets
}

/// Build `ImportedAgent` entries from the `agents.list` array in config.
fn extract_from_config_list(
    config: &OpenClawConfig,
    detection: &OpenClawDetection,
    used_ids: &mut HashSet<String>,
) -> Vec<ImportedAgent> {
    let mut agents = Vec::new();

    for entry in &config.agents.list {
        let is_default = entry.default
            || (config.agents.list.len() == 1)
            || (agents.is_empty() && entry.id == "main");

        let moltis_id = if is_default {
            "main".to_string()
        } else {
            let id = sanitize_agent_id(&entry.id, used_ids);
            used_ids.insert(id.clone());
            id
        };

        let source_workspace =
            resolve_agent_workspace(entry.workspace.as_deref(), &entry.id, detection);

        let theme = extract_agent_identity(&source_workspace, config);

        let model = entry.model.as_ref().map(|m| match m {
            OpenClawAgentModelConfig::Simple(s) => s.clone(),
            OpenClawAgentModelConfig::Full(full) => full.primary.clone().unwrap_or_default(),
        });

        debug!(
            openclaw_id = %entry.id,
            moltis_id = %moltis_id,
            is_default,
            name = ?entry.name,
            theme = ?theme,
            model = ?model,
            workspace = ?source_workspace,
            "openclaw agents: extracted agent"
        );

        agents.push(ImportedAgent {
            openclaw_id: entry.id.clone(),
            moltis_id,
            is_default,
            name: entry.name.clone(),
            theme,
            source_workspace,
            model,
        });
    }

    agents
}

/// Synthesize minimal `ImportedAgent` entries from detected agent directories
/// when no config list is available.
fn synthesize_from_detection(
    detection: &OpenClawDetection,
    used_ids: &mut HashSet<String>,
) -> Vec<ImportedAgent> {
    let mut agents = Vec::new();

    for (i, oc_id) in detection.agent_ids.iter().enumerate() {
        let is_default =
            oc_id == "main" || (i == 0 && !detection.agent_ids.contains(&"main".to_string()));

        let moltis_id = if is_default {
            "main".to_string()
        } else {
            let id = sanitize_agent_id(oc_id, used_ids);
            used_ids.insert(id.clone());
            id
        };

        let agent_dir = detection.home_dir.join("agents").join(oc_id);
        let source_workspace = resolve_agent_dir_workspace(&agent_dir)
            .or_else(|| Some(detection.workspace_dir.clone()));

        debug!(
            openclaw_id = %oc_id,
            moltis_id = %moltis_id,
            is_default,
            "openclaw agents: synthesized agent from filesystem"
        );

        agents.push(ImportedAgent {
            openclaw_id: oc_id.clone(),
            moltis_id,
            is_default,
            name: None,
            theme: None,
            source_workspace,
            model: None,
        });
    }

    agents
}

/// Resolve the workspace directory for a specific agent.
///
/// Priority: configured workspace path → remap basename under `~/` →
/// agent's `agent/` subdir → detection workspace
fn resolve_agent_workspace(
    configured_workspace: Option<&str>,
    agent_id: &str,
    detection: &OpenClawDetection,
) -> Option<PathBuf> {
    // 1. Configured workspace path (exact)
    if let Some(ws) = configured_workspace {
        let path = if PathBuf::from(ws).is_absolute() {
            PathBuf::from(ws)
        } else {
            detection.home_dir.join(ws)
        };
        if path.is_dir() {
            return Some(path);
        }

        // 1b. Remap: basename under ~/ (cross-machine, e.g. /root/clawd → ~/clawd)
        let abs_path = PathBuf::from(ws);
        if let Some(basename) = abs_path.file_name()
            && let Some(user_home) = dirs_next::home_dir()
        {
            let remapped = user_home.join(basename);
            if remapped.is_dir() {
                debug!(
                    configured = ws,
                    remapped = %remapped.display(),
                    agent_id,
                    "openclaw agents: workspace remapped from config basename"
                );
                return Some(remapped);
            }
        }
    }

    // 2. Agent's agent/ subdir (may contain IDENTITY.md, etc.)
    let agent_dir = detection.home_dir.join("agents").join(agent_id);
    if let Some(ws) = resolve_agent_dir_workspace(&agent_dir) {
        return Some(ws);
    }

    // 3. Global workspace
    if detection.workspace_dir.is_dir() {
        return Some(detection.workspace_dir.clone());
    }

    None
}

/// Check for a workspace within an agent's directory structure.
fn resolve_agent_dir_workspace(agent_dir: &Path) -> Option<PathBuf> {
    let nested = agent_dir.join("agent");
    if nested.is_dir() {
        return Some(nested);
    }
    if agent_dir.is_dir() {
        return Some(agent_dir.to_path_buf());
    }
    None
}

/// Extract theme from an agent's workspace IDENTITY.md or config.
fn extract_agent_identity(workspace: &Option<PathBuf>, config: &OpenClawConfig) -> Option<String> {
    // Try IDENTITY.md in the workspace
    if let Some(ws) = workspace {
        let identity_path = ws.join("IDENTITY.md");
        if let Ok(content) = std::fs::read_to_string(&identity_path) {
            let parsed = identity::parse_workspace_identity(&content);
            let theme = parsed
                .theme
                .or_else(|| identity::compose_theme(parsed.creature, parsed.vibe));
            if theme.is_some() {
                return theme;
            }
        }
    }

    // Fall back to ui.assistant from config
    if let Some(assistant) = config.ui.assistant.as_ref() {
        let theme = assistant.theme.clone().or_else(|| {
            identity::compose_theme(assistant.creature.clone(), assistant.vibe.clone())
        });
        return theme;
    }

    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_basic() {
        let ids = HashSet::new();
        assert_eq!(sanitize_agent_id("MyAgent", &ids), "myagent");
    }

    #[test]
    fn sanitize_special_chars() {
        let ids = HashSet::new();
        // Non-alphanumeric chars become `-`, then repeated dashes are collapsed
        assert_eq!(sanitize_agent_id("my_agent!@#test", &ids), "my-agent-test");
    }

    #[test]
    fn sanitize_collapses_dashes() {
        let ids = HashSet::new();
        assert_eq!(sanitize_agent_id("a--b---c", &ids), "a-b-c");
    }

    #[test]
    fn sanitize_trims_dashes() {
        let ids = HashSet::new();
        assert_eq!(sanitize_agent_id("--hello--", &ids), "hello");
    }

    #[test]
    fn sanitize_main_gets_suffix() {
        let ids = HashSet::new();
        assert_eq!(sanitize_agent_id("main", &ids), "main-2");
    }

    #[test]
    fn sanitize_collision_detection() {
        let mut ids = HashSet::new();
        ids.insert("myagent".to_string());
        assert_eq!(sanitize_agent_id("MyAgent", &ids), "myagent-2");
    }

    #[test]
    fn sanitize_multiple_collisions() {
        let mut ids = HashSet::new();
        ids.insert("test".to_string());
        ids.insert("test-2".to_string());
        assert_eq!(sanitize_agent_id("test", &ids), "test-3");
    }

    #[test]
    fn sanitize_empty_string() {
        let ids = HashSet::new();
        assert_eq!(sanitize_agent_id("", &ids), "agent");
    }

    #[test]
    fn sanitize_truncates_long_ids() {
        let ids = HashSet::new();
        let long = "a".repeat(60);
        let result = sanitize_agent_id(&long, &ids);
        assert_eq!(result.len(), 50);
    }

    #[test]
    fn import_agents_single_default() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        std::fs::write(
            home.join("openclaw.json"),
            r#"{"agents":{"list":[{"id":"main","default":true,"name":"Claude"}]}}"#,
        )
        .unwrap();

        let detection = OpenClawDetection {
            home_dir: home.to_path_buf(),
            has_config: true,
            has_credentials: false,
            has_mcp_servers: false,
            workspace_dir: home.join("workspace"),
            has_memory: false,
            has_skills: false,
            agent_ids: vec!["main".to_string()],
            session_count: 0,
            unsupported_channels: Vec::new(),
            has_workspace_files: false,
            workspace_files_found: Vec::new(),
        };

        let result = import_agents(&detection);
        assert_eq!(result.agents.len(), 1);
        assert_eq!(result.agents[0].moltis_id, "main");
        assert!(result.agents[0].is_default);
        assert_eq!(result.agents[0].name.as_deref(), Some("Claude"));
    }

    #[test]
    fn import_agents_multiple() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        std::fs::write(
            home.join("openclaw.json"),
            r#"{"agents":{"list":[
                {"id":"main","default":true,"name":"Claude"},
                {"id":"research","name":"Researcher"}
            ]}}"#,
        )
        .unwrap();

        let detection = OpenClawDetection {
            home_dir: home.to_path_buf(),
            has_config: true,
            has_credentials: false,
            has_mcp_servers: false,
            workspace_dir: home.join("workspace"),
            has_memory: false,
            has_skills: false,
            agent_ids: vec!["main".to_string(), "research".to_string()],
            session_count: 0,
            unsupported_channels: Vec::new(),
            has_workspace_files: false,
            workspace_files_found: Vec::new(),
        };

        let result = import_agents(&detection);
        assert_eq!(result.agents.len(), 2);
        assert_eq!(result.agents[0].moltis_id, "main");
        assert!(result.agents[0].is_default);
        assert_eq!(result.agents[1].moltis_id, "research");
        assert!(!result.agents[1].is_default);
    }

    #[test]
    fn import_agents_with_identity() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        // Create agent workspace with IDENTITY.md
        let agent_dir = home.join("agents").join("research").join("agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("IDENTITY.md"),
            "---\ncreature: fox\nvibe: curious\n---\n",
        )
        .unwrap();

        std::fs::write(
            home.join("openclaw.json"),
            r#"{"agents":{"list":[
                {"id":"main","default":true,"name":"Claude"},
                {"id":"research","name":"Researcher"}
            ]}}"#,
        )
        .unwrap();

        let detection = OpenClawDetection {
            home_dir: home.to_path_buf(),
            has_config: true,
            has_credentials: false,
            has_mcp_servers: false,
            workspace_dir: home.join("workspace"),
            has_memory: false,
            has_skills: false,
            agent_ids: vec!["main".to_string(), "research".to_string()],
            session_count: 0,
            unsupported_channels: Vec::new(),
            has_workspace_files: false,
            workspace_files_found: Vec::new(),
        };

        let result = import_agents(&detection);
        let research = &result.agents[1];
        assert_eq!(research.theme.as_deref(), Some("curious fox"));
    }

    #[test]
    fn import_agents_synthesizes_from_detection() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        // No config, but agent dirs exist
        std::fs::create_dir_all(home.join("agents").join("main")).unwrap();
        std::fs::create_dir_all(home.join("agents").join("helper")).unwrap();

        let detection = OpenClawDetection {
            home_dir: home.to_path_buf(),
            has_config: false,
            has_credentials: false,
            has_mcp_servers: false,
            workspace_dir: home.join("workspace"),
            has_memory: false,
            has_skills: false,
            agent_ids: vec!["main".to_string(), "helper".to_string()],
            session_count: 0,
            unsupported_channels: Vec::new(),
            has_workspace_files: false,
            workspace_files_found: Vec::new(),
        };

        let result = import_agents(&detection);
        assert_eq!(result.agents.len(), 2);
        assert_eq!(result.agents[0].moltis_id, "main");
        assert!(result.agents[0].is_default);
        assert_eq!(result.agents[1].moltis_id, "helper");
        assert!(!result.agents[1].is_default);
    }

    #[test]
    fn import_agents_no_config_no_agents() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        let detection = OpenClawDetection {
            home_dir: home.to_path_buf(),
            has_config: false,
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
        };

        let result = import_agents(&detection);
        assert!(result.agents.is_empty());
    }

    #[test]
    fn import_agents_extracts_model() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        std::fs::write(
            home.join("openclaw.json"),
            r#"{"agents":{"list":[
                {"id":"main","default":true,"name":"Claude"},
                {"id":"fast","name":"Quick","model":"anthropic/claude-haiku-3-5-20241022"}
            ]}}"#,
        )
        .unwrap();

        let detection = OpenClawDetection {
            home_dir: home.to_path_buf(),
            has_config: true,
            has_credentials: false,
            has_mcp_servers: false,
            workspace_dir: home.join("workspace"),
            has_memory: false,
            has_skills: false,
            agent_ids: vec!["main".to_string(), "fast".to_string()],
            session_count: 0,
            unsupported_channels: Vec::new(),
            has_workspace_files: false,
            workspace_files_found: Vec::new(),
        };

        let result = import_agents(&detection);
        assert_eq!(result.agents.len(), 2);
        assert!(result.agents[0].model.is_none()); // main has no model override
        assert_eq!(
            result.agents[1].model.as_deref(),
            Some("anthropic/claude-haiku-3-5-20241022")
        );
    }

    #[test]
    fn agents_to_presets_skips_default() {
        let agents = ImportedAgents {
            agents: vec![
                ImportedAgent {
                    openclaw_id: "main".to_string(),
                    moltis_id: "main".to_string(),
                    is_default: true,
                    name: Some("Claude".to_string()),
                    theme: Some("wise owl".to_string()),
                    ..Default::default()
                },
                ImportedAgent {
                    openclaw_id: "researcher".to_string(),
                    moltis_id: "researcher".to_string(),
                    is_default: false,
                    name: Some("Scout".to_string()),
                    theme: Some("curious fox".to_string()),
                    model: Some("anthropic/claude-haiku-3-5-20241022".to_string()),
                    ..Default::default()
                },
            ],
        };

        let presets = agents_to_presets(&agents);
        assert_eq!(presets.len(), 1);
        assert!(!presets.contains_key("main"));

        let researcher = presets.get("researcher").unwrap();
        assert_eq!(researcher.identity.name.as_deref(), Some("Scout"));
        assert_eq!(researcher.identity.theme.as_deref(), Some("curious fox"));
        assert_eq!(
            researcher.model.as_deref(),
            Some("anthropic/claude-haiku-3-5-20241022")
        );
    }
}
