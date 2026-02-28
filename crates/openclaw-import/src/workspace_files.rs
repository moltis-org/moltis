//! Import workspace personality files from OpenClaw.
//!
//! Copies `SOUL.md`, `IDENTITY.md`, `USER.md`, `TOOLS.md`, `AGENTS.md`,
//! `HEARTBEAT.md`, and `BOOT.md` from the OpenClaw workspace directory to the
//! Moltis data directory, preserving any user-customized files at the destination.

use std::path::Path;

use tracing::{debug, warn};

use crate::{
    detect::OpenClawDetection,
    report::{CategoryReport, ImportCategory, ImportStatus},
};

/// The set of workspace personality files to import.
pub const WORKSPACE_FILE_NAMES: &[&str] = &[
    "SOUL.md",
    "IDENTITY.md",
    "USER.md",
    "TOOLS.md",
    "AGENTS.md",
    "HEARTBEAT.md",
    "BOOT.md",
];

/// Import workspace personality files from the default agent's workspace.
pub fn import_workspace_files(
    detection: &OpenClawDetection,
    dest_data_dir: &Path,
) -> CategoryReport {
    import_agent_workspace_files(&detection.workspace_dir, dest_data_dir)
}

/// Import workspace files for a specific agent from a source workspace
/// to a destination directory.
///
/// This is the core logic, usable for both the default agent
/// (via [`import_workspace_files`]) and non-default agents with per-agent
/// workspaces.
pub fn import_agent_workspace_files(source_workspace: &Path, dest_dir: &Path) -> CategoryReport {
    let mut imported = 0;
    let mut skipped = 0;
    let mut errors = Vec::new();

    for file_name in WORKSPACE_FILE_NAMES {
        let src = source_workspace.join(file_name);
        if !src.is_file() {
            continue;
        }

        let dest = dest_dir.join(file_name);

        match import_single_file(&src, &dest, file_name) {
            Ok(FileImportResult::Created) => {
                debug!(file = *file_name, "imported workspace file (new)");
                imported += 1;
            },
            Ok(FileImportResult::Replaced) => {
                debug!(
                    file = *file_name,
                    "replaced default workspace file with imported content"
                );
                imported += 1;
            },
            Ok(FileImportResult::Skipped) => {
                debug!(file = *file_name, "workspace file already exists, skipping");
                skipped += 1;
            },
            Err(e) => {
                warn!(file = *file_name, error = %e, "failed to import workspace file");
                errors.push(format!("failed to import {file_name}: {e}"));
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
        category: ImportCategory::WorkspaceFiles,
        status,
        items_imported: imported,
        items_updated: 0,
        items_skipped: skipped,
        warnings: Vec::new(),
        errors,
    }
}

enum FileImportResult {
    Created,
    Replaced,
    Skipped,
}

fn import_single_file(
    src: &Path,
    dest: &Path,
    file_name: &str,
) -> crate::error::Result<FileImportResult> {
    let src_content = std::fs::read_to_string(src)?;
    if src_content.trim().is_empty() {
        return Ok(FileImportResult::Skipped);
    }

    // Destination doesn't exist — create it.
    if !dest.exists() {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(dest, &src_content)?;
        return Ok(FileImportResult::Created);
    }

    let dest_content = std::fs::read_to_string(dest)?;
    let dest_trimmed = dest_content.trim();

    // Non-empty file that user may have customized — skip.
    if !dest_trimmed.is_empty() && !is_default_content(file_name, dest_trimmed) {
        return Ok(FileImportResult::Skipped);
    }

    // Empty file or contains only the auto-seeded default — replace.
    std::fs::write(dest, &src_content)?;
    Ok(FileImportResult::Replaced)
}

/// Check whether the destination file contains only the auto-seeded default.
///
/// Currently only `SOUL.md` has auto-seeding via `DEFAULT_SOUL` in
/// `moltis_config::loader`. For other files the function returns `false`
/// (they are only overwritten if empty).
fn is_default_content(file_name: &str, content: &str) -> bool {
    if file_name == "SOUL.md" {
        return content == moltis_config::DEFAULT_SOUL.trim();
    }
    false
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn make_detection(home: &Path) -> OpenClawDetection {
        OpenClawDetection {
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
            has_workspace_files: true,
            workspace_files_found: Vec::new(),
        }
    }

    #[test]
    fn import_new_soul_file() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("moltis");

        std::fs::create_dir_all(home.join("workspace")).unwrap();
        std::fs::write(
            home.join("workspace").join("SOUL.md"),
            "# My Custom Soul\n\nI am unique.",
        )
        .unwrap();

        let detection = make_detection(home);
        let report = import_workspace_files(&detection, &dest);

        assert_eq!(report.status, ImportStatus::Success);
        assert_eq!(report.items_imported, 1);
        let content = std::fs::read_to_string(dest.join("SOUL.md")).unwrap();
        assert!(content.contains("I am unique."));
    }

    #[test]
    fn import_multiple_files() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("moltis");
        let ws = home.join("workspace");
        std::fs::create_dir_all(&ws).unwrap();

        std::fs::write(ws.join("SOUL.md"), "soul content").unwrap();
        std::fs::write(ws.join("IDENTITY.md"), "identity content").unwrap();
        std::fs::write(ws.join("TOOLS.md"), "tools content").unwrap();

        let detection = make_detection(home);
        let report = import_workspace_files(&detection, &dest);

        assert_eq!(report.items_imported, 3);
        assert!(dest.join("SOUL.md").is_file());
        assert!(dest.join("IDENTITY.md").is_file());
        assert!(dest.join("TOOLS.md").is_file());
    }

    #[test]
    fn skip_existing_customized_files() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("moltis");
        let ws = home.join("workspace");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::create_dir_all(&dest).unwrap();

        std::fs::write(ws.join("SOUL.md"), "openclaw soul").unwrap();
        // Pre-existing user-customized file
        std::fs::write(dest.join("SOUL.md"), "my moltis soul").unwrap();

        let detection = make_detection(home);
        let report = import_workspace_files(&detection, &dest);

        assert_eq!(report.items_imported, 0);
        assert_eq!(report.items_skipped, 1);
        // Content should be preserved
        let content = std::fs::read_to_string(dest.join("SOUL.md")).unwrap();
        assert_eq!(content, "my moltis soul");
    }

    #[test]
    fn replace_default_soul() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("moltis");
        let ws = home.join("workspace");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::create_dir_all(&dest).unwrap();

        std::fs::write(ws.join("SOUL.md"), "imported soul").unwrap();
        // Destination has the auto-seeded default
        std::fs::write(dest.join("SOUL.md"), moltis_config::DEFAULT_SOUL).unwrap();

        let detection = make_detection(home);
        let report = import_workspace_files(&detection, &dest);

        assert_eq!(report.items_imported, 1);
        let content = std::fs::read_to_string(dest.join("SOUL.md")).unwrap();
        assert_eq!(content, "imported soul");
    }

    #[test]
    fn skip_empty_source_files() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("moltis");
        let ws = home.join("workspace");
        std::fs::create_dir_all(&ws).unwrap();

        std::fs::write(ws.join("SOUL.md"), "   \n  ").unwrap();

        let detection = make_detection(home);
        let report = import_workspace_files(&detection, &dest);

        assert_eq!(report.status, ImportStatus::Skipped);
        assert_eq!(report.items_imported, 0);
    }

    #[test]
    fn no_workspace_files_returns_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        std::fs::create_dir_all(home.join("workspace")).unwrap();

        let detection = make_detection(home);
        let report = import_workspace_files(&detection, &tmp.path().join("dest"));

        assert_eq!(report.status, ImportStatus::Skipped);
    }

    #[test]
    fn import_per_agent_workspace_files() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_ws = tmp.path().join("agent-ws");
        let dest = tmp.path().join("agent-dest");
        std::fs::create_dir_all(&agent_ws).unwrap();

        std::fs::write(agent_ws.join("SOUL.md"), "agent soul").unwrap();
        std::fs::write(agent_ws.join("IDENTITY.md"), "agent identity").unwrap();

        let report = import_agent_workspace_files(&agent_ws, &dest);

        assert_eq!(report.items_imported, 2);
        assert!(dest.join("SOUL.md").is_file());
        assert!(dest.join("IDENTITY.md").is_file());
    }

    #[test]
    fn replace_empty_destination() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("moltis");
        let ws = home.join("workspace");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::create_dir_all(&dest).unwrap();

        std::fs::write(ws.join("IDENTITY.md"), "imported identity").unwrap();
        // Destination exists but is empty
        std::fs::write(dest.join("IDENTITY.md"), "").unwrap();

        let detection = make_detection(home);
        let report = import_workspace_files(&detection, &dest);

        assert_eq!(report.items_imported, 1);
        let content = std::fs::read_to_string(dest.join("IDENTITY.md")).unwrap();
        assert_eq!(content, "imported identity");
    }

    #[test]
    fn idempotent_import() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("moltis");
        let ws = home.join("workspace");
        std::fs::create_dir_all(&ws).unwrap();

        std::fs::write(ws.join("SOUL.md"), "soul").unwrap();

        let detection = make_detection(home);

        // First import
        let report1 = import_workspace_files(&detection, &dest);
        assert_eq!(report1.items_imported, 1);

        // Second import — should skip (file exists and is non-empty, non-default)
        let report2 = import_workspace_files(&detection, &dest);
        assert_eq!(report2.items_imported, 0);
        assert_eq!(report2.items_skipped, 1);
    }
}
