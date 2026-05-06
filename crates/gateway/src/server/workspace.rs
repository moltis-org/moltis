use std::path::PathBuf;

use super::seed_content::{
    DEFAULT_BOOT_MD, DEFAULT_HEARTBEAT_MD, DEFAULT_TOOLS_MD, DEFAULT_WORKSPACE_AGENTS_MD,
};

pub fn sync_persona_into_preset(
    agents: &mut moltis_config::AgentsConfig,
    persona: &crate::agent_persona::AgentPersona,
) {
    let soul = moltis_config::load_soul_for_agent(&persona.id);

    let entry = agents.presets.entry(persona.id.clone()).or_default();

    entry.identity.name = Some(persona.name.clone());
    entry.identity.emoji = persona.emoji.clone();
    entry.identity.theme = persona.theme.clone();

    if let Some(ref soul) = soul
        && !soul.trim().is_empty()
    {
        entry.system_prompt_suffix = Some(soul.clone());
    }
}

pub(crate) fn seed_default_workspace_markdown_files() {
    let data_dir = moltis_config::data_dir();
    seed_file_if_missing(data_dir.join("BOOT.md"), DEFAULT_BOOT_MD);
    seed_file_if_missing(data_dir.join("AGENTS.md"), DEFAULT_WORKSPACE_AGENTS_MD);
    seed_file_if_missing(data_dir.join("TOOLS.md"), DEFAULT_TOOLS_MD);
    seed_file_if_missing(data_dir.join("HEARTBEAT.md"), DEFAULT_HEARTBEAT_MD);
}

pub(crate) fn warn_on_workspace_prompt_file_truncation() {
    let limit_chars = moltis_config::discover_and_load()
        .chat
        .workspace_file_max_chars;
    let data_dir = moltis_config::data_dir();
    let mut paths = vec![data_dir.join("AGENTS.md"), data_dir.join("TOOLS.md")];
    let agents_dir = data_dir.join("agents");
    if let Ok(entries) = std::fs::read_dir(&agents_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            paths.push(path.join("AGENTS.md"));
            paths.push(path.join("TOOLS.md"));
        }
    }

    for path in paths {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(normalized) = moltis_config::normalize_workspace_markdown_content(&content) else {
            continue;
        };
        let char_count = normalized.chars().count();
        if char_count <= limit_chars {
            continue;
        }
        tracing::warn!(
            path = %path.display(),
            char_count,
            limit_chars,
            truncated_chars = char_count.saturating_sub(limit_chars),
            "workspace prompt file exceeds configured prompt cap and will be truncated"
        );
    }
}

fn seed_file_if_missing(path: PathBuf, content: &str) {
    if path.exists() {
        return;
    }
    if let Err(error) = std::fs::write(&path, content) {
        tracing::debug!(
            path = %path.display(),
            "could not write default markdown file: {error}"
        );
    }
}
