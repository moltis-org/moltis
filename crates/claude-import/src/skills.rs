//! Import skills and commands from Claude Code.
//!
//! - Skills in `~/.claude/skills/*/SKILL.md` are copied directly.
//! - Commands in `~/.claude/commands/**/*.md` are converted to SKILL.md format.

use std::path::{Path, PathBuf};

use {
    moltis_import_core::{
        report::{CategoryReport, ImportCategory, ImportStatus},
        skills::{copy_skill_dirs, create_skill_from_command},
    },
    tracing::{debug, warn},
};

use crate::detect::ClaudeDetection;

/// Discovered skill ready for import.
#[derive(Debug, Clone)]
pub struct DiscoveredSkill {
    pub name: String,
    pub source: PathBuf,
}

/// Discovered command that will be converted to a skill.
#[derive(Debug, Clone)]
pub struct DiscoveredCommand {
    pub name: String,
    pub source: PathBuf,
    pub relative_path: String,
}

/// Discover SKILL.md-based skills in the Claude skills directory.
pub fn discover_skills(detection: &ClaudeDetection) -> Vec<DiscoveredSkill> {
    let Some(ref skills_dir) = detection.user_skills_dir else {
        return Vec::new();
    };

    let Ok(entries) = std::fs::read_dir(skills_dir) else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter(|entry| entry.path().is_dir() && entry.path().join("SKILL.md").is_file())
        .filter_map(|entry| {
            entry.file_name().to_str().map(|name| DiscoveredSkill {
                name: name.to_string(),
                source: entry.path(),
            })
        })
        .collect()
}

/// Discover markdown command files in the Claude commands directory.
pub fn discover_commands(detection: &ClaudeDetection) -> Vec<DiscoveredCommand> {
    let Some(ref commands_dir) = detection.user_commands_dir else {
        return Vec::new();
    };

    let mut commands = Vec::new();
    collect_markdown_files(commands_dir, commands_dir, &mut commands);
    commands
}

fn collect_markdown_files(root: &Path, dir: &Path, out: &mut Vec<DiscoveredCommand>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_markdown_files(root, &path, out);
        } else if path.extension().is_some_and(|ext| ext == "md") {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();

            let name = build_command_skill_name(&relative);
            if !name.is_empty() {
                out.push(DiscoveredCommand {
                    name,
                    source: path,
                    relative_path: relative,
                });
            }
        }
    }
}

/// Build a skill name from a command's relative path.
///
/// `commands/namespace/name.md` → `claude-command-namespace-name`
fn build_command_skill_name(relative_path: &str) -> String {
    let path = Path::new(relative_path);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let parent = path
        .parent()
        .and_then(|p| p.to_str())
        .unwrap_or("")
        .replace(['/', '\\'], "-");

    let parts: Vec<&str> = ["claude-command"]
        .into_iter()
        .chain(if parent.is_empty() {
            None
        } else {
            Some(parent.as_str())
        })
        .chain(Some(stem))
        .collect();

    sanitize_name(&parts.join("-"))
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .to_lowercase()
}

/// Import skills and commands from Claude Code into Moltis.
pub fn import_skills(detection: &ClaudeDetection, dest_skills_dir: &Path) -> CategoryReport {
    let skills = discover_skills(detection);
    let commands = discover_commands(detection);

    if skills.is_empty() && commands.is_empty() {
        return CategoryReport::skipped(ImportCategory::Skills);
    }

    // Copy SKILL.md-based skills
    let skill_sources: Vec<(String, &Path)> = skills
        .iter()
        .map(|s| (s.name.clone(), s.source.as_path()))
        .collect();

    let mut report = copy_skill_dirs(&skill_sources, dest_skills_dir);

    // Convert commands to skills
    for cmd in &commands {
        let content = match std::fs::read_to_string(&cmd.source) {
            Ok(c) => c,
            Err(e) => {
                warn!(name = %cmd.name, error = %e, "failed to read command file");
                report
                    .warnings
                    .push(format!("failed to read command '{}': {e}", cmd.name));
                continue;
            },
        };

        let source_label = format!("Claude command {}", cmd.relative_path);
        match create_skill_from_command(&cmd.name, &source_label, &content, dest_skills_dir) {
            Ok(true) => {
                debug!(name = %cmd.name, "converted command to skill");
                report.items_imported += 1;
            },
            Ok(false) => {
                debug!(name = %cmd.name, "command skill already exists, skipping");
                report.items_skipped += 1;
            },
            Err(e) => {
                warn!(name = %cmd.name, error = %e, "failed to create skill from command");
                report.warnings.push(format!(
                    "failed to create skill from command '{}': {e}",
                    cmd.name
                ));
            },
        }
    }

    if report.items_imported > 0 && report.warnings.is_empty() {
        report.status = ImportStatus::Success;
    } else if !report.warnings.is_empty() {
        report.status = ImportStatus::Partial;
    }

    report
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn make_detection() -> ClaudeDetection {
        ClaudeDetection {
            home_dir: None,
            user_settings_path: None,
            user_local_settings_path: None,
            user_claude_json_path: None,
            user_memory_path: None,
            user_skills_dir: None,
            user_commands_dir: None,
            desktop_config_path: None,
            has_data: false,
        }
    }

    #[test]
    fn discover_skills_finds_skill_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(skills_dir.join("my-skill")).unwrap();
        std::fs::write(
            skills_dir.join("my-skill").join("SKILL.md"),
            "---\nname: test\n---\nContent",
        )
        .unwrap();

        // Not a skill (no SKILL.md)
        std::fs::create_dir_all(skills_dir.join("not-a-skill")).unwrap();

        let mut detection = make_detection();
        detection.user_skills_dir = Some(skills_dir);

        let skills = discover_skills(&detection);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "my-skill");
    }

    #[test]
    fn discover_commands_finds_markdown() {
        let tmp = tempfile::tempdir().unwrap();
        let cmds_dir = tmp.path().join("commands");
        std::fs::create_dir_all(cmds_dir.join("utils")).unwrap();
        std::fs::write(cmds_dir.join("hello.md"), "# Hello").unwrap();
        std::fs::write(cmds_dir.join("utils").join("format.md"), "# Format code").unwrap();

        let mut detection = make_detection();
        detection.user_commands_dir = Some(cmds_dir);

        let commands = discover_commands(&detection);
        assert_eq!(commands.len(), 2);

        let names: Vec<&str> = commands.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"claude-command-hello"));
        assert!(names.contains(&"claude-command-utils-format"));
    }

    #[test]
    fn import_skills_copies_skills_and_commands() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(skills_dir.join("existing-skill")).unwrap();
        std::fs::write(
            skills_dir.join("existing-skill").join("SKILL.md"),
            "---\nname: test\n---\nContent",
        )
        .unwrap();

        let cmds_dir = tmp.path().join("commands");
        std::fs::create_dir_all(&cmds_dir).unwrap();
        std::fs::write(cmds_dir.join("deploy.md"), "# Deploy\n\nDeploy the app.").unwrap();

        let dest = tmp.path().join("dest-skills");

        let mut detection = make_detection();
        detection.user_skills_dir = Some(skills_dir);
        detection.user_commands_dir = Some(cmds_dir);

        let report = import_skills(&detection, &dest);
        assert_eq!(report.items_imported, 2);

        assert!(dest.join("existing-skill").join("SKILL.md").is_file());
        assert!(
            dest.join("claude-command-deploy")
                .join("SKILL.md")
                .is_file()
        );

        // Command-derived skill should have frontmatter
        let content =
            std::fs::read_to_string(dest.join("claude-command-deploy").join("SKILL.md")).unwrap();
        assert!(content.contains("name: claude-command-deploy"));
        assert!(content.contains("Imported from: Claude command deploy.md"));
    }

    #[test]
    fn build_command_skill_name_handles_nested() {
        assert_eq!(build_command_skill_name("hello.md"), "claude-command-hello");
        assert_eq!(
            build_command_skill_name("utils/format.md"),
            "claude-command-utils-format"
        );
        assert_eq!(
            build_command_skill_name("a/b/deep.md"),
            "claude-command-a-b-deep"
        );
    }
}
