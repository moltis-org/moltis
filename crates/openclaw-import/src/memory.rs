//! Import memory files (MEMORY.md and all markdown files in `memory/`) from OpenClaw.

use std::path::Path;

use tracing::{debug, warn};

use crate::{
    detect::OpenClawDetection,
    report::{CategoryReport, ImportCategory, ImportStatus},
};

/// Import memory files from OpenClaw workspace to Moltis data directory.
///
/// - `MEMORY.md`: Merged if both exist (imported content appended with separator).
/// - `memory/*.md`: All markdown files copied, skipping files that already exist.
///   This includes daily logs (`YYYY-MM-DD.md`), project notes, and any other
///   custom memory files the user created.
///
/// Does NOT import the SQLite vector database (not portable across embedding models).
pub fn import_memory(detection: &OpenClawDetection, dest_data_dir: &Path) -> CategoryReport {
    import_agent_memory(&detection.workspace_dir, dest_data_dir)
}

/// Import memory for a specific agent from a source workspace to a destination directory.
///
/// This is the core memory import logic, usable for both the default agent
/// (via `import_memory`) and non-default agents with per-agent workspaces.
pub fn import_agent_memory(source_workspace: &Path, dest_data_dir: &Path) -> CategoryReport {
    let mut imported = 0;
    let mut skipped = 0;
    let warnings = Vec::new();
    let mut errors = Vec::new();

    // 1. Import MEMORY.md
    let src_memory = source_workspace.join("MEMORY.md");
    let dest_memory = dest_data_dir.join("MEMORY.md");

    if src_memory.is_file() {
        match import_memory_file(&src_memory, &dest_memory) {
            Ok(MemoryFileResult::Created) => {
                debug!("imported MEMORY.md (new file)");
                imported += 1;
            },
            Ok(MemoryFileResult::Merged) => {
                debug!("merged MEMORY.md with existing");
                imported += 1;
            },
            Ok(MemoryFileResult::Skipped) => {
                debug!("MEMORY.md already contains imported content, skipping");
                skipped += 1;
            },
            Err(e) => {
                warn!(error = %e, "failed to import MEMORY.md");
                errors.push(format!("failed to import MEMORY.md: {e}"));
            },
        }
    }

    // 2. Import all markdown files from memory/ directory
    let src_memory_dir = source_workspace.join("memory");
    let dest_memory_dir = dest_data_dir.join("memory");

    if src_memory_dir.is_dir() {
        if let Err(e) = std::fs::create_dir_all(&dest_memory_dir) {
            errors.push(format!("failed to create memory directory: {e}"));
        } else if let Ok(entries) = std::fs::read_dir(&src_memory_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let Some(name) = path.file_name() else {
                    continue;
                };
                let name_str = name.to_string_lossy();
                if !name_str.ends_with(".md") {
                    continue;
                }

                let dest_file = dest_memory_dir.join(name);
                if dest_file.exists() {
                    debug!(file = %name_str, "memory file already exists, skipping");
                    skipped += 1;
                    continue;
                }

                match std::fs::copy(&path, &dest_file) {
                    Ok(_) => {
                        let kind = if looks_like_daily_log(&name_str) {
                            "daily log"
                        } else {
                            "memory file"
                        };
                        debug!(file = %name_str, kind, "imported memory file");
                        imported += 1;
                    },
                    Err(e) => {
                        warn!(file = %name_str, error = %e, "failed to copy memory file");
                        errors.push(format!("failed to copy {name_str}: {e}"));
                    },
                }
            }
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
        category: ImportCategory::Memory,
        status,
        items_imported: imported,
        items_updated: 0,
        items_skipped: skipped,
        warnings,
        errors,
    }
}

enum MemoryFileResult {
    Created,
    Merged,
    Skipped,
}

const IMPORT_SEPARATOR: &str = "\n\n<!-- Imported from OpenClaw -->\n\n";

fn import_memory_file(src: &Path, dest: &Path) -> crate::error::Result<MemoryFileResult> {
    let src_content = std::fs::read_to_string(src)?;
    if src_content.trim().is_empty() {
        return Ok(MemoryFileResult::Skipped);
    }

    if !dest.exists() {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(dest, &src_content)?;
        return Ok(MemoryFileResult::Created);
    }

    let dest_content = std::fs::read_to_string(dest)?;

    // Idempotency: if dest already contains the import separator, skip
    if dest_content.contains("<!-- Imported from OpenClaw -->") {
        return Ok(MemoryFileResult::Skipped);
    }

    // Merge: append imported content with separator
    let merged = format!("{dest_content}{IMPORT_SEPARATOR}{src_content}");
    std::fs::write(dest, merged)?;
    Ok(MemoryFileResult::Merged)
}

/// Check if a filename looks like a daily log (YYYY-MM-DD.md).
fn looks_like_daily_log(name: &str) -> bool {
    let stem = name.strip_suffix(".md").unwrap_or(name);
    let parts: Vec<&str> = stem.split('-').collect();
    if parts.len() != 3 {
        return false;
    }
    parts[0].len() == 4
        && parts[1].len() == 2
        && parts[2].len() == 2
        && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit()))
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
            has_memory: true,
            has_skills: false,
            agent_ids: Vec::new(),
            session_count: 0,
            unsupported_channels: Vec::new(),
            has_workspace_files: false,
            workspace_files_found: Vec::new(),
        }
    }

    #[test]
    fn import_new_memory_file() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("moltis");

        std::fs::create_dir_all(home.join("workspace")).unwrap();
        std::fs::write(
            home.join("workspace").join("MEMORY.md"),
            "# OpenClaw Memory\n\nI learned things.",
        )
        .unwrap();

        let detection = make_detection(home);
        let report = import_memory(&detection, &dest);

        assert_eq!(report.status, ImportStatus::Success);
        assert_eq!(report.items_imported, 1);
        let content = std::fs::read_to_string(dest.join("MEMORY.md")).unwrap();
        assert!(content.contains("I learned things."));
    }

    #[test]
    fn import_merges_existing_memory() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("moltis");

        std::fs::create_dir_all(home.join("workspace")).unwrap();
        std::fs::write(home.join("workspace").join("MEMORY.md"), "# From OpenClaw").unwrap();

        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("MEMORY.md"), "# Existing Moltis Memory").unwrap();

        let detection = make_detection(home);
        let report = import_memory(&detection, &dest);

        assert_eq!(report.items_imported, 1);
        let content = std::fs::read_to_string(dest.join("MEMORY.md")).unwrap();
        assert!(content.contains("# Existing Moltis Memory"));
        assert!(content.contains("<!-- Imported from OpenClaw -->"));
        assert!(content.contains("# From OpenClaw"));
    }

    #[test]
    fn import_idempotent_memory() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("moltis");

        std::fs::create_dir_all(home.join("workspace")).unwrap();
        std::fs::write(home.join("workspace").join("MEMORY.md"), "stuff").unwrap();

        // Already imported
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(
            dest.join("MEMORY.md"),
            "existing\n\n<!-- Imported from OpenClaw -->\n\nstuff",
        )
        .unwrap();

        let detection = make_detection(home);
        let report = import_memory(&detection, &dest);

        assert_eq!(report.items_skipped, 1);
        assert_eq!(report.items_imported, 0);
    }

    #[test]
    fn import_daily_logs() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("moltis");

        let daily = home.join("workspace").join("memory");
        std::fs::create_dir_all(&daily).unwrap();
        std::fs::write(daily.join("2024-01-15.md"), "day 1").unwrap();
        std::fs::write(daily.join("2024-01-16.md"), "day 2").unwrap();
        std::fs::write(daily.join("notes.txt"), "not a markdown file").unwrap();

        let detection = make_detection(home);
        let report = import_memory(&detection, &dest);

        assert_eq!(report.items_imported, 2);
        assert!(dest.join("memory").join("2024-01-15.md").is_file());
        assert!(dest.join("memory").join("2024-01-16.md").is_file());
        // .txt files are NOT imported (only .md)
        assert!(!dest.join("memory").join("notes.txt").exists());
    }

    #[test]
    fn import_all_markdown_memory_files() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("moltis");

        let mem_dir = home.join("workspace").join("memory");
        std::fs::create_dir_all(&mem_dir).unwrap();
        std::fs::write(mem_dir.join("2024-01-15.md"), "daily log").unwrap();
        std::fs::write(
            mem_dir.join("project-notes.md"),
            "# Project Notes\nSome notes.",
        )
        .unwrap();
        std::fs::write(
            mem_dir.join("api-reference.md"),
            "# API Reference\nEndpoints.",
        )
        .unwrap();
        std::fs::write(mem_dir.join("data.json"), "not imported").unwrap();

        let detection = make_detection(home);
        let report = import_memory(&detection, &dest);

        // All 3 .md files imported (daily log + 2 custom memory files)
        assert_eq!(report.items_imported, 3);
        assert!(dest.join("memory").join("2024-01-15.md").is_file());
        assert!(dest.join("memory").join("project-notes.md").is_file());
        assert!(dest.join("memory").join("api-reference.md").is_file());
        // Non-.md files skipped
        assert!(!dest.join("memory").join("data.json").exists());
    }

    #[test]
    fn daily_log_pattern_detection() {
        assert!(looks_like_daily_log("2024-01-15.md"));
        assert!(looks_like_daily_log("2025-12-31.md"));
        assert!(!looks_like_daily_log("notes.md"));
        assert!(!looks_like_daily_log("24-01-15.md"));
        assert!(!looks_like_daily_log("2024-1-15.md"));
    }

    #[test]
    fn no_memory_returns_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("workspace")).unwrap();

        let detection = make_detection(tmp.path());
        let report = import_memory(&detection, &tmp.path().join("dest"));

        assert_eq!(report.status, ImportStatus::Skipped);
    }
}
