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

use crate::schema::{AgentIdentity, AgentPreset, PresetToolPolicy};

/// Frontmatter fields parsed from the YAML block.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct AgentFrontmatter {
    name: Option<String>,
    tools: Option<String>,
    deny_tools: Option<String>,
    model: Option<String>,
    emoji: Option<String>,
    theme: Option<String>,
    delegate_only: bool,
    max_iterations: Option<u64>,
    timeout_secs: Option<u64>,
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

    let allow = fm
        .tools
        .map(|t| t.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    let deny = fm
        .deny_tools
        .map(|t| t.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    let body_trimmed = body.trim();
    let system_prompt_suffix = if body_trimmed.is_empty() {
        None
    } else {
        Some(body_trimmed.to_string())
    };

    let preset = AgentPreset {
        identity: AgentIdentity {
            name: Some(name.clone()),
            emoji: fm.emoji,
            theme: fm.theme,
        },
        model: fm.model,
        tools: PresetToolPolicy { allow, deny },
        system_prompt_suffix,
        delegate_only: fm.delegate_only,
        max_iterations: fm.max_iterations,
        timeout_secs: fm.timeout_secs,
        ..Default::default()
    };

    Ok((name, preset))
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
/// TOML presets take precedence — markdown defs are only inserted
/// for names that don't already exist.
pub fn merge_agent_defs(
    presets: &mut HashMap<String, AgentPreset>,
    defs: HashMap<String, AgentPreset>,
) {
    for (name, preset) in defs {
        presets.entry(name).or_insert(preset);
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
                    },
                },
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "failed to read agent definition");
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
}
