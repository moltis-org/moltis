//! Import skills from OpenClaw to Moltis.
//!
//! OpenClaw skills use the SKILL.md format that Moltis already parses.
//! This module copies skill directories, skipping duplicates.

use std::path::{Path, PathBuf};

use tracing::{debug, warn};

use crate::{
    detect::OpenClawDetection,
    report::{CategoryReport, ImportCategory, ImportStatus},
};

/// Discovered skill ready for import.
#[derive(Debug, Clone)]
pub struct DiscoveredSkill {
    /// Skill directory name (used as the skill name/ID).
    pub name: String,
    /// Source path of the skill directory.
    pub source: PathBuf,
    /// Whether this is a workspace skill (vs managed).
    pub is_workspace: bool,
}

/// Scan OpenClaw for importable skills.
pub fn discover_skills(detection: &OpenClawDetection) -> Vec<DiscoveredSkill> {
    let mut skills = Vec::new();

    // Workspace skills (higher priority)
    let ws_skills = detection.workspace_dir.join("skills");
    if ws_skills.is_dir() {
        scan_skill_dir(&ws_skills, true, &mut skills);
    }

    // Managed skills
    let managed_skills = detection.home_dir.join("skills");
    if managed_skills.is_dir() {
        scan_skill_dir(&managed_skills, false, &mut skills);
    }

    skills
}

fn scan_skill_dir(dir: &Path, is_workspace: bool, skills: &mut Vec<DiscoveredSkill>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // A valid skill directory must contain SKILL.md
        if !path.join("SKILL.md").is_file() {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            skills.push(DiscoveredSkill {
                name: name.to_string(),
                source: path,
                is_workspace,
            });
        }
    }
}

/// Import discovered skills into the Moltis skills directory.
///
/// `dest_skills_dir` is typically `~/.moltis/skills/`.
pub fn import_skills(detection: &OpenClawDetection, dest_skills_dir: &Path) -> CategoryReport {
    let skills = discover_skills(detection);

    if skills.is_empty() {
        return CategoryReport::skipped(ImportCategory::Skills);
    }

    let mut imported = 0;
    let mut skipped = 0;
    let warnings = Vec::new();
    let mut errors = Vec::new();

    for skill in &skills {
        let dest = dest_skills_dir.join(&skill.name);

        // Skip if already exists (idempotency)
        if dest.exists() {
            debug!(name = %skill.name, "skill already exists, skipping");
            skipped += 1;
            continue;
        }

        match copy_dir_recursive(&skill.source, &dest) {
            Ok(()) => {
                debug!(name = %skill.name, "imported skill");
                imported += 1;
            },
            Err(e) => {
                warn!(name = %skill.name, error = %e, "failed to import skill");
                errors.push(format!("failed to import skill '{}': {e}", skill.name));
            },
        }
    }

    let status = if !errors.is_empty() && imported > 0 {
        ImportStatus::Partial
    } else if !errors.is_empty() {
        ImportStatus::Failed
    } else if imported == 0 {
        ImportStatus::Skipped
    } else {
        ImportStatus::Success
    };

    CategoryReport {
        category: ImportCategory::Skills,
        status,
        items_imported: imported,
        items_updated: 0,
        items_skipped: skipped,
        warnings,
        errors,
    }
}

/// Recursively copy a directory.
fn copy_dir_recursive(src: &Path, dest: &Path) -> crate::error::Result<()> {
    std::fs::create_dir_all(dest)?;

    for entry in walkdir::WalkDir::new(src).min_depth(1) {
        let entry = entry?;
        let relative = entry.path().strip_prefix(src)?;
        let target = dest.join(relative);

        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target)?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(entry.path(), &target)?;
        }
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn setup_skills(home: &Path) {
        // Workspace skill
        let ws_skill = home.join("workspace").join("skills").join("my-skill");
        std::fs::create_dir_all(&ws_skill).unwrap();
        std::fs::write(
            ws_skill.join("SKILL.md"),
            "---\nname: my-skill\n---\nDo stuff.",
        )
        .unwrap();
        std::fs::write(ws_skill.join("helper.py"), "# helper").unwrap();

        // Managed skill
        let managed_skill = home.join("skills").join("managed-skill");
        std::fs::create_dir_all(&managed_skill).unwrap();
        std::fs::write(
            managed_skill.join("SKILL.md"),
            "---\nname: managed-skill\n---\nManaged stuff.",
        )
        .unwrap();

        // Invalid dir (no SKILL.md)
        let invalid = home.join("skills").join("not-a-skill");
        std::fs::create_dir_all(&invalid).unwrap();
        std::fs::write(invalid.join("README.md"), "not a skill").unwrap();
    }

    fn make_detection(home: &Path) -> OpenClawDetection {
        OpenClawDetection {
            home_dir: home.to_path_buf(),
            has_config: false,
            has_credentials: false,
            has_mcp_servers: false,
            workspace_dir: home.join("workspace"),
            has_memory: false,
            has_skills: true,
            agent_ids: Vec::new(),
            session_count: 0,
            unsupported_channels: Vec::new(),
            has_workspace_files: false,
            workspace_files_found: Vec::new(),
        }
    }

    #[test]
    fn discover_finds_valid_skills() {
        let tmp = tempfile::tempdir().unwrap();
        setup_skills(tmp.path());

        let detection = make_detection(tmp.path());
        let skills = discover_skills(&detection);

        assert_eq!(skills.len(), 2);
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"my-skill"));
        assert!(names.contains(&"managed-skill"));
    }

    #[test]
    fn import_copies_skill_directories() {
        let tmp = tempfile::tempdir().unwrap();
        setup_skills(tmp.path());

        let dest = tmp.path().join("moltis-skills");
        let detection = make_detection(tmp.path());
        let report = import_skills(&detection, &dest);

        assert_eq!(report.status, ImportStatus::Success);
        assert_eq!(report.items_imported, 2);
        assert!(dest.join("my-skill").join("SKILL.md").is_file());
        assert!(dest.join("my-skill").join("helper.py").is_file());
        assert!(dest.join("managed-skill").join("SKILL.md").is_file());
    }

    #[test]
    fn import_skips_existing() {
        let tmp = tempfile::tempdir().unwrap();
        setup_skills(tmp.path());

        let dest = tmp.path().join("moltis-skills");
        // Pre-create one skill
        std::fs::create_dir_all(dest.join("my-skill")).unwrap();

        let detection = make_detection(tmp.path());
        let report = import_skills(&detection, &dest);

        assert_eq!(report.items_imported, 1); // Only managed-skill
        assert_eq!(report.items_skipped, 1); // my-skill skipped
    }

    #[test]
    fn import_no_skills_returns_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("workspace")).unwrap();

        let detection = make_detection(tmp.path());
        let report = import_skills(&detection, &tmp.path().join("dest"));

        assert_eq!(report.status, ImportStatus::Skipped);
    }
}
