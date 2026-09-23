//! Markdown-based agent definitions with YAML frontmatter.
//!
//! Scans `~/.moltis/agents/` and `.moltis/agents/` for `.md` files,
//! parsing YAML frontmatter into [`AgentPreset`] fields and using the
//! body as `system_prompt_suffix`.
//!
//! # Format
//!
//! ```markdown
//! ---
//! name: code-reviewer
//! tools: Read, Grep, Glob
//! model: sonnet
//! ---
//! System prompt body here...
//! ```

use std::{collections::HashMap, path::Path};

use tracing::{debug, warn};

use crate::schema::{
    AgentIdentity, AgentPreset, McpServerId, PresetMcpPolicy, PresetSandboxMode,
    PresetSandboxPolicy, PresetToolPolicy, SandboxMountConfig, is_default_agent_preset,
};

/// Frontmatter fields parsed from the YAML block.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct AgentFrontmatter {
    name: Option<String>,
    display_name: Option<String>,
    tools: Option<String>,
    deny_tools: Option<String>,
    model: Option<String>,
    emoji: Option<String>,
    theme: Option<String>,
    delegate_only: bool,
    max_iterations: Option<u64>,
    timeout_secs: Option<u64>,
    reasoning_effort: Option<String>,
    mcp_allow_servers: Option<String>,
    mcp_deny_servers: Option<String>,
    sandbox_mode: Option<String>,
    sandbox_force: bool,
    sandbox_mounts: Option<String>,
    run_as: Option<String>,
    skills_allow: Option<String>,
    skills_deny: Option<String>,
}

#[derive(Debug, Default, serde::Serialize)]
struct AgentFrontmatterOut {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deny_tools: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    emoji: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    theme: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    delegate_only: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_iterations: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mcp_allow_servers: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mcp_deny_servers: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sandbox_mode: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    sandbox_force: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    sandbox_mounts: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    run_as: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    skills_allow: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    skills_deny: Option<String>,
}

/// Parse a markdown agent definition file into a preset name and config.
///
/// Returns `(preset_name, AgentPreset)` or an error if parsing fails.
pub fn parse_agent_md(content: &str) -> anyhow::Result<(String, AgentPreset)> {
    let (frontmatter_str, body) = split_frontmatter(content)?;
    let fm: AgentFrontmatter = serde_yaml::from_str(&frontmatter_str)?;

    let name = fm
        .name
        .ok_or_else(|| anyhow::anyhow!("agent definition missing required 'name' field"))?;

    let allow = fm.tools.map(csv_list).unwrap_or_default();

    let deny = fm.deny_tools.map(csv_list).unwrap_or_default();

    let body_trimmed = body.trim();
    let system_prompt_suffix = if body_trimmed.is_empty() {
        None
    } else {
        Some(body_trimmed.to_string())
    };

    let preset = AgentPreset {
        identity: AgentIdentity {
            name: Some(fm.display_name.unwrap_or_else(|| name.clone())),
            emoji: fm.emoji,
            theme: fm.theme,
        },
        model: fm.model,
        tools: PresetToolPolicy { allow, deny },
        system_prompt_suffix,
        delegate_only: fm.delegate_only,
        max_iterations: fm.max_iterations,
        timeout_secs: fm.timeout_secs,
        reasoning_effort: fm
            .reasoning_effort
            .as_deref()
            .map(|value| value.try_into().map_err(anyhow::Error::msg))
            .transpose()?,
        mcp: parse_mcp_policy(fm.mcp_allow_servers, fm.mcp_deny_servers)?,
        sandbox: PresetSandboxPolicy {
            mode: fm
                .sandbox_mode
                .as_deref()
                .map(|value| value.try_into().map_err(anyhow::Error::msg))
                .transpose()?,
            force: fm.sandbox_force,
            mounts: parse_sandbox_mounts(fm.sandbox_mounts)?,
            run_as: parse_run_as_field(fm.run_as)?,
        },
        skills: crate::schema::PresetSkillPolicy {
            allow: fm.skills_allow.map(csv_list),
            deny: fm.skills_deny.map(csv_list),
        },
        ..Default::default()
    };

    Ok((name, preset))
}

/// Render a markdown sidecar file for an agent preset.
pub fn render_agent_md(name: &str, preset: &AgentPreset) -> anyhow::Result<String> {
    let display_name = preset.identity.name.clone().filter(|value| value != name);
    let (mcp_allow_servers, mcp_deny_servers) = match &preset.mcp {
        PresetMcpPolicy::All => (None, None),
        PresetMcpPolicy::Allow(servers) => (Some(join_mcp_servers(servers)), None),
        PresetMcpPolicy::Deny(servers) => (None, Some(join_mcp_servers(servers))),
    };
    let fm = AgentFrontmatterOut {
        name: name.to_string(),
        display_name,
        tools: non_empty_join(&preset.tools.allow),
        deny_tools: non_empty_join(&preset.tools.deny),
        model: preset.model.clone(),
        emoji: preset.identity.emoji.clone(),
        theme: preset.identity.theme.clone(),
        delegate_only: preset.delegate_only,
        max_iterations: preset.max_iterations,
        timeout_secs: preset.timeout_secs,
        reasoning_effort: preset
            .reasoning_effort
            .map(|effort| effort.as_str().to_string()),
        mcp_allow_servers,
        mcp_deny_servers,
        sandbox_mode: preset.sandbox.mode.map(|mode| match mode {
            PresetSandboxMode::Off => "off".to_string(),
            PresetSandboxMode::All => "all".to_string(),
            PresetSandboxMode::NonMain => "non-main".to_string(),
        }),
        sandbox_force: preset.sandbox.force,
        sandbox_mounts: render_sandbox_mounts(&preset.sandbox.mounts),
        run_as: preset.sandbox.run_as.clone(),
        skills_allow: preset.skills.allow.as_ref().map(|values| values.join(", ")),
        skills_deny: preset
            .skills
            .deny
            .as_ref()
            .and_then(|values| non_empty_join(values)),
    };
    let frontmatter = serde_yaml::to_string(&fm)?;
    let body = preset.system_prompt_suffix.as_deref().unwrap_or_default();
    Ok(format!("---\n{frontmatter}---\n{body}\n"))
}

/// Write a user-global markdown agent definition under `data_dir()/agents`.
pub fn write_user_agent_def(
    name: &str,
    preset: &AgentPreset,
) -> anyhow::Result<std::path::PathBuf> {
    let dir = crate::loader::data_dir().join("agents");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{name}.md"));
    std::fs::write(&path, render_agent_md(name, preset)?)?;
    Ok(path)
}

/// Delete a user-global markdown agent definition if it exists.
pub fn delete_user_agent_def(name: &str) -> anyhow::Result<bool> {
    let path = crate::loader::data_dir()
        .join("agents")
        .join(format!("{name}.md"));
    if !path.exists() {
        return Ok(false);
    }
    std::fs::remove_file(path)?;
    Ok(true)
}

fn csv_list(value: String) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

/// Parse the flat `sandbox_mounts` frontmatter string into mount configs.
///
/// The markdown sidecar has to be flat, so the array of `source:target:access`
/// triples every other surface uses is comma separated here. A comma in a path
/// would therefore not survive the round trip, which is why [`check_mount_set`]
/// refuses one.
///
/// A wrong field count is an error rather than a dropped mount: a mount that
/// silently vanishes is the same class of bug as the named-volume trap. The
/// whole set then goes through `check_mount_set`, the same call the RPC write
/// path makes, because a hand-edited sidecar reaches the runtime without ever
/// passing through that path.
///
/// [`check_mount_set`]: crate::schema::check_mount_set
fn parse_sandbox_mounts(value: Option<String>) -> anyhow::Result<Vec<SandboxMountConfig>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let mounts = csv_list(value)
        .iter()
        .map(|triple| SandboxMountConfig::parse_triple(triple).map_err(anyhow::Error::msg))
        .collect::<anyhow::Result<Vec<_>>>()?;
    crate::schema::check_mount_set(&mounts).map_err(|errors| anyhow::anyhow!(errors.join("; ")))?;
    Ok(mounts)
}

/// Render mount configs back into the flat frontmatter string.
///
/// The inverse of [`parse_sandbox_mounts`]; a write path with no matching
/// parse would be silent data loss, so the two are tested together.
fn render_sandbox_mounts(mounts: &[SandboxMountConfig]) -> Option<String> {
    if mounts.is_empty() {
        return None;
    }
    Some(
        mounts
            .iter()
            .map(SandboxMountConfig::to_triple)
            .collect::<Vec<_>>()
            .join(", "),
    )
}

/// Validate the flat `run_as` frontmatter value.
///
/// Already a flat string, so there is no encoding to undo - but it still has to
/// be checked here, because a markdown sidecar reaches the runtime without ever
/// passing through the RPC write path. A malformed value is an error, never a
/// dropped field: dropping it would run the container as root.
fn parse_run_as_field(value: Option<String>) -> anyhow::Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim().to_string();
    if value.is_empty() {
        return Ok(None);
    }
    crate::schema::check_run_as(&value).map_err(anyhow::Error::msg)?;
    Ok(Some(value))
}

fn non_empty_join(values: &[String]) -> Option<String> {
    if values.is_empty() {
        None
    } else {
        Some(values.join(", "))
    }
}

fn join_mcp_servers(values: &[McpServerId]) -> String {
    values
        .iter()
        .map(McpServerId::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

fn parse_mcp_policy(
    allow: Option<String>,
    deny: Option<String>,
) -> anyhow::Result<PresetMcpPolicy> {
    match (allow, deny) {
        (None, None) => Ok(PresetMcpPolicy::All),
        (Some(value), None) => Ok(PresetMcpPolicy::Allow(
            csv_list(value).into_iter().map(McpServerId::from).collect(),
        )),
        (None, Some(value)) => Ok(PresetMcpPolicy::Deny(
            csv_list(value).into_iter().map(McpServerId::from).collect(),
        )),
        (Some(_), Some(_)) => {
            anyhow::bail!("mcp_allow_servers and mcp_deny_servers are mutually exclusive")
        },
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Split frontmatter (between `---` delimiters) from the body.
fn split_frontmatter(content: &str) -> anyhow::Result<(String, String)> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        anyhow::bail!("agent definition must start with '---' frontmatter delimiter");
    }

    // Skip the opening `---` and find the closing one.
    let after_open = &trimmed[3..];
    let close_pos = after_open
        .find("\n---")
        .ok_or_else(|| anyhow::anyhow!("missing closing '---' frontmatter delimiter"))?;

    let frontmatter = after_open[..close_pos].to_string();
    let body = after_open[close_pos + 4..].to_string(); // skip "\n---"
    Ok((frontmatter, body))
}

/// Discover agent definition files from standard directories.
///
/// Scans `~/.moltis/agents/` (user-global) then `.moltis/agents/` (project-local).
/// Project-local files override user-global ones with the same name.
pub fn discover_agent_defs() -> HashMap<String, AgentPreset> {
    let mut defs = HashMap::new();

    // User-global: ~/.moltis/agents/
    let user_dir = crate::loader::data_dir().join("agents");
    load_defs_from_dir(&user_dir, &mut defs);

    // Project-local: .moltis/agents/
    let project_dir = std::path::PathBuf::from(".moltis").join("agents");
    load_defs_from_dir(&project_dir, &mut defs);

    defs
}

/// Merge discovered agent definitions into the config's preset map.
///
/// TOML presets take precedence. Markdown defs replace built-in presets but
/// are only inserted over user presets when the name does not already exist.
pub fn merge_agent_defs(
    presets: &mut HashMap<String, AgentPreset>,
    defs: HashMap<String, AgentPreset>,
) {
    for (name, preset) in defs {
        let should_insert = presets
            .get(&name)
            .is_none_or(|existing| is_default_agent_preset(&name, existing));
        if should_insert {
            presets.insert(name, preset);
        }
    }
}

/// The preset an agent gets when its definition file cannot be used.
///
/// Skipping the file outright left the agent with no preset at all, and the
/// no-preset branch falls back to the global `tools.exec.sandbox.mode` - which
/// can be `off`. So a definition that asked for a sandbox with three mounts
/// degraded, on a single unreadable character, to no sandbox. This fails the
/// other way: the agent still exists, and it is sandboxed.
///
/// Used for a file that does not parse **and** for a file that cannot be read
/// at all. The two are the same situation from here: an `.md` sitting in the
/// agent-definitions directory is an agent someone configured, and a
/// permission error or a mid-write truncation says nothing about what it asked
/// for. Warning and moving on would have been the `off` path again, reached by
/// a different error type.
///
/// `force` is set as well as `mode`, and it is the half that actually holds.
/// `mode` loses to a per-session override by design - that precedence is the
/// long-standing contract - so a session that had already been toggled to
/// `sandboxEnabled: false` went straight back to the host the moment the
/// definition broke, which is the exact path this function exists to close.
/// Only `force` outranks that override. The cost is that the toggle is refused
/// while the file is broken; fixing the file restores the agent's real policy,
/// and an agent whose declared policy is currently unknown is not one to hand
/// an off switch for.
fn fail_closed_preset() -> AgentPreset {
    AgentPreset {
        sandbox: PresetSandboxPolicy {
            mode: Some(PresetSandboxMode::All),
            force: true,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn load_defs_from_dir(dir: &Path, defs: &mut HashMap<String, AgentPreset>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return, // Directory doesn't exist — that's fine.
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "md") {
            match std::fs::read_to_string(&path) {
                Ok(content) => match parse_agent_md(&content) {
                    Ok((name, preset)) => {
                        debug!(name = %name, path = %path.display(), "loaded agent definition");
                        defs.insert(name, preset);
                    },
                    Err(e) => {
                        warn!(path = %path.display(), error = %e, "failed to parse agent definition");
                        if let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) {
                            defs.insert(name.to_string(), fail_closed_preset());
                        }
                    },
                },
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "failed to read agent definition");
                    if let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) {
                        defs.insert(name.to_string(), fail_closed_preset());
                    }
                },
            }
        }
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_agent_def_with_frontmatter() {
        let content = r#"---
name: reviewer
tools: Read, Grep
model: sonnet
---
You are a code reviewer. Focus on correctness.
"#;

        let (name, preset) = parse_agent_md(content).unwrap();
        assert_eq!(name, "reviewer");
        assert_eq!(preset.model, Some("sonnet".into()));
        assert_eq!(preset.tools.allow, vec!["Read", "Grep"]);
        assert_eq!(
            preset.system_prompt_suffix.as_deref(),
            Some("You are a code reviewer. Focus on correctness.")
        );
    }

    #[test]
    fn test_parse_full_frontmatter() {
        let content = r#"---
name: scout
tools: Read, Grep, Glob
deny_tools: exec
model: haiku
emoji: 🦉
theme: focused and efficient
delegate_only: false
max_iterations: 20
timeout_secs: 60
---
Search thoroughly.
"#;

        let (name, preset) = parse_agent_md(content).unwrap();
        assert_eq!(name, "scout");
        assert_eq!(preset.tools.allow, vec!["Read", "Grep", "Glob"]);
        assert_eq!(preset.tools.deny, vec!["exec"]);
        assert_eq!(preset.identity.emoji.as_deref(), Some("🦉"));
        assert_eq!(
            preset.identity.theme.as_deref(),
            Some("focused and efficient")
        );
        assert!(!preset.delegate_only);
        assert_eq!(preset.max_iterations, Some(20));
        assert_eq!(preset.timeout_secs, Some(60));
    }

    #[test]
    fn test_body_becomes_system_prompt_suffix() {
        let content = "---\nname: test\n---\nThis is the system prompt.";
        let (_, preset) = parse_agent_md(content).unwrap();
        assert_eq!(
            preset.system_prompt_suffix.as_deref(),
            Some("This is the system prompt.")
        );
    }

    #[test]
    fn test_empty_body() {
        let content = "---\nname: minimal\n---\n";
        let (_, preset) = parse_agent_md(content).unwrap();
        assert!(preset.system_prompt_suffix.is_none());
    }

    #[test]
    fn test_missing_delimiters_error() {
        let content = "name: test\nno delimiters here";
        let result = parse_agent_md(content);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must start with '---'")
        );
    }

    #[test]
    fn test_missing_closing_delimiter() {
        let content = "---\nname: test\nno closing";
        let result = parse_agent_md(content);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("missing closing '---'")
        );
    }

    #[test]
    fn test_missing_name_error() {
        let content = "---\ntools: Read\n---\nbody";
        let result = parse_agent_md(content);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing required"));
    }

    #[test]
    fn test_discover_from_directory() {
        let dir = tempfile::tempdir().unwrap();
        let agents_dir = dir.path().join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();

        std::fs::write(
            agents_dir.join("reviewer.md"),
            "---\nname: reviewer\n---\nReview code.",
        )
        .unwrap();
        std::fs::write(
            agents_dir.join("scout.md"),
            "---\nname: scout\ntools: Read\n---\nSearch.",
        )
        .unwrap();
        // Non-md file should be ignored.
        std::fs::write(agents_dir.join("notes.txt"), "not an agent").unwrap();

        let mut defs = HashMap::new();
        load_defs_from_dir(&agents_dir, &mut defs);

        assert_eq!(defs.len(), 2);
        assert!(defs.contains_key("reviewer"));
        assert!(defs.contains_key("scout"));
    }

    #[test]
    fn test_merge_does_not_override_toml() {
        let mut presets = HashMap::new();
        presets.insert("reviewer".to_string(), AgentPreset {
            model: Some("opus".into()),
            ..Default::default()
        });

        let mut defs = HashMap::new();
        defs.insert("reviewer".to_string(), AgentPreset {
            model: Some("haiku".into()),
            ..Default::default()
        });
        defs.insert("scout".to_string(), AgentPreset {
            model: Some("sonnet".into()),
            ..Default::default()
        });

        merge_agent_defs(&mut presets, defs);

        // TOML preset should be preserved.
        assert_eq!(presets["reviewer"].model.as_deref(), Some("opus"));
        // New def should be added.
        assert_eq!(presets["scout"].model.as_deref(), Some("sonnet"));
    }

    #[test]
    fn test_merge_overrides_builtin_default() {
        let mut presets = crate::schema::default_agent_presets();

        let mut defs = HashMap::new();
        defs.insert("research".to_string(), AgentPreset {
            model: Some("sonnet".into()),
            identity: AgentIdentity {
                name: Some("Local Researcher".into()),
                ..Default::default()
            },
            ..Default::default()
        });

        merge_agent_defs(&mut presets, defs);

        assert_eq!(presets["research"].model.as_deref(), Some("sonnet"));
        assert_eq!(
            presets["research"].identity.name.as_deref(),
            Some("Local Researcher")
        );
        assert!(presets.contains_key("coder"));
    }

    #[test]
    fn test_project_overrides_user() {
        let user_dir = tempfile::tempdir().unwrap();
        let project_dir = tempfile::tempdir().unwrap();

        let user_agents = user_dir.path().to_path_buf();
        let project_agents = project_dir.path().to_path_buf();

        std::fs::write(
            user_agents.join("reviewer.md"),
            "---\nname: reviewer\nmodel: haiku\n---\nUser version.",
        )
        .unwrap();
        std::fs::write(
            project_agents.join("reviewer.md"),
            "---\nname: reviewer\nmodel: sonnet\n---\nProject version.",
        )
        .unwrap();

        let mut defs = HashMap::new();
        load_defs_from_dir(&user_agents, &mut defs);
        load_defs_from_dir(&project_agents, &mut defs); // project overrides user

        assert_eq!(defs["reviewer"].model.as_deref(), Some("sonnet"));
        assert_eq!(
            defs["reviewer"].system_prompt_suffix.as_deref(),
            Some("Project version.")
        );
    }

    #[test]
    fn test_render_round_trips_web_ui_fields() {
        let preset = AgentPreset {
            identity: AgentIdentity {
                name: Some("Code Reviewer".into()),
                emoji: Some("🔎".into()),
                theme: Some("careful".into()),
            },
            model: Some("haiku".into()),
            tools: PresetToolPolicy {
                allow: vec!["Read".into(), "Grep".into()],
                deny: vec!["exec".into()],
            },
            system_prompt_suffix: Some("Review for correctness.".into()),
            delegate_only: true,
            timeout_secs: Some(45),
            ..Default::default()
        };

        let rendered = render_agent_md("reviewer", &preset).unwrap();
        let (name, parsed) = parse_agent_md(&rendered).unwrap();

        assert_eq!(name, "reviewer");
        assert_eq!(parsed.identity.name.as_deref(), Some("Code Reviewer"));
        assert_eq!(parsed.identity.emoji.as_deref(), Some("🔎"));
        assert_eq!(parsed.model.as_deref(), Some("haiku"));
        assert_eq!(parsed.tools.allow, vec!["Read", "Grep"]);
        assert_eq!(parsed.tools.deny, vec!["exec"]);
        assert!(parsed.delegate_only);
        assert_eq!(parsed.timeout_secs, Some(45));
        assert_eq!(
            parsed.system_prompt_suffix.as_deref(),
            Some("Review for correctness.")
        );
    }

    #[test]
    fn test_render_round_trips_advanced_fields() {
        use crate::schema::{
            McpServerId, PresetMcpPolicy, PresetSandboxMode, PresetSandboxPolicy,
            PresetSkillPolicy, ReasoningEffort,
        };

        let preset = AgentPreset {
            identity: AgentIdentity {
                name: Some("Secure Agent".into()),
                ..Default::default()
            },
            reasoning_effort: Some(ReasoningEffort::High),
            mcp: PresetMcpPolicy::Allow(vec![McpServerId::new("github"), McpServerId::new("jira")]),
            sandbox: PresetSandboxPolicy {
                mode: Some(PresetSandboxMode::All),
                force: true,
                ..Default::default()
            },
            skills: PresetSkillPolicy {
                allow: Some(vec!["code-review".into(), "testing".into()]),
                deny: Some(vec!["deploy".into()]),
            },
            ..Default::default()
        };

        let rendered = render_agent_md("secure", &preset).unwrap();
        let (name, parsed) = parse_agent_md(&rendered).unwrap();

        assert_eq!(name, "secure");
        assert_eq!(parsed.reasoning_effort, Some(ReasoningEffort::High));
        assert!(matches!(parsed.mcp, PresetMcpPolicy::Allow(ref servers) if servers.len() == 2));
        if let PresetMcpPolicy::Allow(servers) = &parsed.mcp {
            assert_eq!(servers[0].as_str(), "github");
            assert_eq!(servers[1].as_str(), "jira");
        }
        assert_eq!(parsed.sandbox.mode, Some(PresetSandboxMode::All));
        assert!(
            parsed.sandbox.force,
            "sandbox_force must survive the sidecar round trip, or a forced \
             agent quietly stops being forced the next time the UI saves it"
        );
        assert!(
            rendered.contains("sandbox_force: true"),
            "the flat spelling is sandbox_force, got: {rendered}"
        );
        assert_eq!(
            parsed.skills.allow.as_deref(),
            Some(["code-review".to_string(), "testing".to_string()].as_slice())
        );
        assert_eq!(
            parsed.skills.deny.as_deref(),
            Some(["deploy".to_string()].as_slice())
        );
    }

    #[test]
    fn test_render_round_trips_mcp_deny() {
        use crate::schema::{McpServerId, PresetMcpPolicy};

        let preset = AgentPreset {
            identity: AgentIdentity {
                name: Some("Limited".into()),
                ..Default::default()
            },
            mcp: PresetMcpPolicy::Deny(vec![McpServerId::new("risky-server")]),
            ..Default::default()
        };

        let rendered = render_agent_md("limited", &preset).unwrap();
        let (_, parsed) = parse_agent_md(&rendered).unwrap();

        assert!(matches!(parsed.mcp, PresetMcpPolicy::Deny(ref s) if s.len() == 1));
        if let PresetMcpPolicy::Deny(servers) = &parsed.mcp {
            assert_eq!(servers[0].as_str(), "risky-server");
        }
    }

    #[test]
    fn test_render_round_trips_sandbox_mounts() {
        use crate::schema::{
            PresetSandboxMode, PresetSandboxPolicy, SandboxMountAccess, SandboxMountConfig,
        };

        let preset = AgentPreset {
            identity: AgentIdentity {
                name: Some("Walter".into()),
                ..Default::default()
            },
            sandbox: PresetSandboxPolicy {
                mode: Some(PresetSandboxMode::All),
                mounts: vec![
                    SandboxMountConfig {
                        source: "/srv/vault".into(),
                        target: "/srv/vault".into(),
                        access: SandboxMountAccess::Rw,
                    },
                    SandboxMountConfig {
                        source: "/srv/notes".into(),
                        target: "/srv/notes".into(),
                        access: SandboxMountAccess::Ro,
                    },
                ],
                ..Default::default()
            },
            ..Default::default()
        };

        let rendered = render_agent_md("walter", &preset).unwrap();
        assert!(
            rendered.contains("sandbox_mounts:"),
            "rendered frontmatter must carry the mounts: {rendered}"
        );

        let (_, parsed) = parse_agent_md(&rendered).unwrap();
        assert_eq!(
            parsed.sandbox.mounts, preset.sandbox.mounts,
            "mounts must round trip losslessly through frontmatter"
        );
    }

    #[test]
    fn test_render_omits_sandbox_mounts_when_empty() {
        let preset = AgentPreset {
            identity: AgentIdentity {
                name: Some("Plain".into()),
                ..Default::default()
            },
            ..Default::default()
        };

        let rendered = render_agent_md("plain", &preset).unwrap();
        assert!(
            !rendered.contains("sandbox_mounts"),
            "an agent with no mounts must not gain the key: {rendered}"
        );

        let (_, parsed) = parse_agent_md(&rendered).unwrap();
        assert!(parsed.sandbox.mounts.is_empty());
    }

    #[test]
    fn test_sandbox_mounts_frontmatter_runs_the_whole_set_check() {
        // The markdown sidecar reaches the runtime without ever passing through
        // the RPC write path, so without this a hand-edited file with a
        // relative source or two mounts on one target loaded clean and failed
        // at container start instead.
        for mounts in [
            "vault:/srv/vault:rw",
            "/srv/a:/srv/shared:rw, /srv/b:/srv/shared:ro",
            "/srv/../etc:/srv/vault:rw",
            "/:/srv/root:ro",
        ] {
            let content = format!("---\nname: walter\nsandbox_mounts: \"{mounts}\"\n---\nbody\n");
            assert!(
                parse_agent_md(&content).is_err(),
                "sandbox_mounts {mounts:?} must not load"
            );
        }
    }

    #[test]
    fn test_sandbox_mount_paths_may_not_contain_a_wire_separator() {
        // The frontmatter carries the whole list as one comma-separated string
        // of `source:target:access` triples, so a comma or a colon in a path
        // validates, renders, and then re-parses as something else - which used
        // to drop the entire agent.
        for source in ["/srv/my,vault", "/srv/my:vault"] {
            let preset = AgentPreset {
                sandbox: PresetSandboxPolicy {
                    mounts: vec![SandboxMountConfig {
                        source: source.into(),
                        target: "/srv/vault".into(),
                        access: crate::schema::SandboxMountAccess::Rw,
                    }],
                    ..Default::default()
                },
                ..Default::default()
            };
            let rendered = render_agent_md("walter", &preset).unwrap();

            assert!(
                crate::schema::check_mount_set(&preset.sandbox.mounts).is_err(),
                "{source:?} must be refused where it is written"
            );
            assert!(
                parse_agent_md(&rendered).is_err(),
                "and the rendered form of {source:?} must not load either"
            );
        }
    }

    #[test]
    fn test_a_definition_that_does_not_parse_still_yields_a_sandboxed_preset() {
        // Skipping the file left the agent with no preset at all, and the
        // no-preset branch in run_with_tools falls back to the global
        // `tools.exec.sandbox.mode` - which can be `off`. One unreadable
        // character in an agent that asked for a sandbox therefore turned the
        // sandbox off.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("walter.md"),
            "---\nname: walter\nsandbox_mounts: \"vault:/srv/vault:rw\"\n---\nbody\n",
        )
        .unwrap();

        let mut defs = HashMap::new();
        load_defs_from_dir(dir.path(), &mut defs);

        let preset = defs
            .get("walter")
            .expect("a definition that does not parse must still leave the agent present");
        assert_eq!(
            preset.sandbox.mode,
            Some(PresetSandboxMode::All),
            "and it must fail closed, not inherit whatever the global mode is"
        );
        assert!(
            preset.sandbox.forces_sandbox(),
            "`mode` alone loses to a per-session override, so a session already \
             toggled off would reach the host anyway - only `force` holds"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_a_definition_that_cannot_be_read_still_yields_a_sandboxed_preset() {
        // The io::Error half of the same failure. A parse error failed closed
        // while an unreadable file only warned, so a chmod, a mid-write
        // truncation or a bad mount reached the very `off` path the parse
        // branch exists to avoid - by a different error type.
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("walter.md");
        std::fs::write(&path, "---\nname: walter\n---\nbody\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read_to_string(&path).is_ok() {
            // Running as root, where the mode is advisory. Nothing to assert.
            return;
        }

        let mut defs = HashMap::new();
        load_defs_from_dir(dir.path(), &mut defs);

        let preset = defs
            .get("walter")
            .expect("a definition that cannot be read must still leave the agent present");
        assert_eq!(
            preset.sandbox.mode,
            Some(PresetSandboxMode::All),
            "and it must fail closed, the same way a parse error does"
        );
        assert!(
            preset.sandbox.forces_sandbox(),
            "and it must pin the sandbox on, the same way a parse error does"
        );
    }

    #[test]
    fn test_sandbox_mounts_wrong_field_count_errors() {
        let content = "---\nname: walter\nsandbox_mounts: \"/srv/vault:/srv/vault\"\n---\nbody\n";
        let error = parse_agent_md(content).expect_err("a two-field mount must not parse");
        assert!(
            error.to_string().contains("source:target:access"),
            "error should name the expected shape, got: {error}"
        );
    }

    #[test]
    fn test_sandbox_mounts_bad_access_errors() {
        let content =
            "---\nname: walter\nsandbox_mounts: \"/srv/vault:/srv/vault:readwrite\"\n---\nbody\n";
        let error = parse_agent_md(content).expect_err("an unknown access value must not parse");
        assert!(
            error.to_string().contains("readwrite"),
            "error should name the bad value, got: {error}"
        );
    }

    #[test]
    fn test_render_round_trips_sandbox_run_as() {
        use crate::schema::PresetSandboxPolicy;

        let preset = AgentPreset {
            identity: AgentIdentity {
                name: Some("Walter".into()),
                ..Default::default()
            },
            sandbox: PresetSandboxPolicy {
                run_as: Some("1000:1000".into()),
                ..Default::default()
            },
            ..Default::default()
        };

        let rendered = render_agent_md("walter", &preset).unwrap();
        assert!(
            rendered.contains("run_as: 1000:1000"),
            "rendered frontmatter must carry run_as: {rendered}"
        );

        let (_, parsed) = parse_agent_md(&rendered).unwrap();
        assert_eq!(parsed.sandbox.run_as.as_deref(), Some("1000:1000"));
    }

    #[test]
    fn test_render_omits_run_as_when_unset() {
        let preset = AgentPreset {
            identity: AgentIdentity {
                name: Some("Plain".into()),
                ..Default::default()
            },
            ..Default::default()
        };

        let rendered = render_agent_md("plain", &preset).unwrap();
        assert!(
            !rendered.contains("run_as"),
            "an agent with no run_as must not gain the key: {rendered}"
        );
        let (_, parsed) = parse_agent_md(&rendered).unwrap();
        assert!(parsed.sandbox.run_as.is_none());
    }

    #[test]
    fn test_sandbox_run_as_malformed_errors() {
        // A markdown sidecar reaches the runtime without passing the RPC write
        // path, so the shape rule has to hold here too. Dropping the field
        // instead would run the container as root.
        let content = "---\nname: walter\nrun_as: \"1000\"\n---\nbody\n";
        let error = parse_agent_md(content).expect_err("a one-part run_as must not parse");
        assert!(
            error.to_string().contains("uid:gid"),
            "error should name the expected shape, got: {error}"
        );
    }

    #[test]
    fn test_sandbox_run_as_root_uid_errors() {
        let content = "---\nname: walter\nrun_as: \"0:0\"\n---\nbody\n";
        let error = parse_agent_md(content).expect_err("a run_as of 0:0 must not parse");
        assert!(
            error.to_string().contains("uid 0"),
            "error should name the rule, got: {error}"
        );
    }

    #[test]
    fn test_render_round_trips_sandbox_non_main() {
        use crate::schema::{PresetSandboxMode, PresetSandboxPolicy};

        let preset = AgentPreset {
            identity: AgentIdentity {
                name: Some("Sandboxed".into()),
                ..Default::default()
            },
            sandbox: PresetSandboxPolicy {
                mode: Some(PresetSandboxMode::NonMain),
                ..Default::default()
            },
            ..Default::default()
        };

        let rendered = render_agent_md("sandboxed", &preset).unwrap();
        let (_, parsed) = parse_agent_md(&rendered).unwrap();

        assert_eq!(parsed.sandbox.mode, Some(PresetSandboxMode::NonMain));
    }

    #[test]
    fn test_render_preserves_empty_skills_allow() {
        use crate::schema::PresetSkillPolicy;

        let preset = AgentPreset {
            identity: AgentIdentity {
                name: Some("No Skills".into()),
                ..Default::default()
            },
            skills: PresetSkillPolicy {
                allow: Some(Vec::new()),
                deny: None,
            },
            ..Default::default()
        };

        let rendered = render_agent_md("no-skills", &preset).unwrap();
        let (_, parsed) = parse_agent_md(&rendered).unwrap();

        assert_eq!(parsed.skills.allow, Some(Vec::new()));
    }
}
