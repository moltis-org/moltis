//! Import memory/workspace files from Hermes.
//!
//! Copies personality files (SOUL.md, AGENTS.md, MEMORY.md, USER.md)
//! into the Moltis data directory.

use std::path::Path;

use {
    moltis_import_core::report::{CategoryReport, ImportCategory, ImportStatus},
    tracing::debug,
};

use crate::detect::HermesDetection;

/// Well-known Hermes workspace/memory files.
const WORKSPACE_FILES: &[(&str, fn(&HermesDetection) -> Option<&Path>)] = &[
    ("SOUL.md", |d| d.soul_path.as_deref()),
    ("AGENTS.md", |d| d.agents_path.as_deref()),
    ("MEMORY.md", |d| d.memory_path.as_deref()),
    ("USER.md", |d| d.user_path.as_deref()),
];

/// Import memory and workspace files from Hermes into Moltis.
pub fn import_memory(detection: &HermesDetection, dest_dir: &Path) -> CategoryReport {
    let mut imported = 0;
    let mut skipped = 0;
    let mut warnings = Vec::new();

    for &(filename, accessor) in WORKSPACE_FILES {
        let Some(source) = accessor(detection) else {
            continue;
        };

        let dest = dest_dir.join(filename);
        if dest.is_file() {
            debug!(filename, "file already exists, skipping");
            skipped += 1;
            continue;
        }

        if let Err(e) = std::fs::create_dir_all(dest_dir) {
            warnings.push(format!("failed to create directory: {e}"));
            continue;
        }

        match std::fs::copy(source, &dest) {
            Ok(_) => {
                debug!(filename, "imported memory file");
                imported += 1;
            },
            Err(e) => {
                warnings.push(format!("failed to copy {filename}: {e}"));
            },
        }
    }

    if imported == 0 && skipped == 0 && warnings.is_empty() {
        return CategoryReport::skipped(ImportCategory::Memory);
    }

    let status = if !warnings.is_empty() {
        ImportStatus::Partial
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
        errors: Vec::new(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn make_detection(home: &Path) -> HermesDetection {
        HermesDetection {
            home_dir: home.to_path_buf(),
            config_path: None,
            env_path: None,
            skills_dir: None,
            soul_path: None,
            agents_path: None,
            memory_path: None,
            user_path: None,
            has_data: true,
        }
    }

    #[test]
    fn import_memory_copies_files() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".hermes");
        std::fs::create_dir_all(home.join("memories")).unwrap();
        std::fs::write(home.join("SOUL.md"), "# Soul\nI am helpful.").unwrap();
        std::fs::write(home.join("memories").join("MEMORY.md"), "# Memory\nFacts.").unwrap();

        let dest = tmp.path().join("dest");

        let mut detection = make_detection(&home);
        detection.soul_path = Some(home.join("SOUL.md"));
        detection.memory_path = Some(home.join("memories").join("MEMORY.md"));

        let report = import_memory(&detection, &dest);
        assert_eq!(report.status, ImportStatus::Success);
        assert_eq!(report.items_imported, 2);

        assert!(dest.join("SOUL.md").is_file());
        assert!(dest.join("MEMORY.md").is_file());
    }

    #[test]
    fn import_memory_skips_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".hermes");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("SOUL.md"), "new content").unwrap();

        let dest = tmp.path().join("dest");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("SOUL.md"), "existing").unwrap();

        let mut detection = make_detection(&home);
        detection.soul_path = Some(home.join("SOUL.md"));

        let report = import_memory(&detection, &dest);
        assert_eq!(report.items_skipped, 1);
        assert_eq!(report.items_imported, 0);
    }

    #[test]
    fn import_memory_no_files() {
        let tmp = tempfile::tempdir().unwrap();
        let detection = make_detection(tmp.path());
        let report = import_memory(&detection, &tmp.path().join("dest"));
        assert_eq!(report.status, ImportStatus::Skipped);
    }
}
