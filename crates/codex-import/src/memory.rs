//! Import instructions from Codex CLI.
//!
//! Copies `instructions.md` from `~/.codex/` as a workspace reference file.

use std::path::Path;

use {
    moltis_import_core::report::{CategoryReport, ImportCategory, ImportStatus},
    tracing::debug,
};

use crate::detect::CodexDetection;

/// Import Codex instructions file into Moltis data directory.
pub fn import_memory(detection: &CodexDetection, dest_dir: &Path) -> CategoryReport {
    let Some(ref source) = detection.instructions_path else {
        return CategoryReport::skipped(ImportCategory::Memory);
    };

    let dest = dest_dir.join("CODEX_INSTRUCTIONS.md");
    if dest.is_file() {
        debug!("CODEX_INSTRUCTIONS.md already exists in destination, skipping");
        return CategoryReport {
            category: ImportCategory::Memory,
            status: ImportStatus::Skipped,
            items_imported: 0,
            items_updated: 0,
            items_skipped: 1,
            warnings: Vec::new(),
            errors: Vec::new(),
        };
    }

    if let Err(e) = std::fs::create_dir_all(dest_dir) {
        return CategoryReport::failed(
            ImportCategory::Memory,
            format!("failed to create directory: {e}"),
        );
    }

    match std::fs::copy(source, &dest) {
        Ok(_) => {
            debug!(
                source = %source.display(),
                dest = %dest.display(),
                "imported Codex instructions.md"
            );
            CategoryReport::success(ImportCategory::Memory, 1)
        },
        Err(e) => CategoryReport::failed(
            ImportCategory::Memory,
            format!("failed to copy instructions.md: {e}"),
        ),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn make_detection() -> CodexDetection {
        CodexDetection {
            home_dir: std::path::PathBuf::new(),
            config_path: None,
            instructions_path: None,
            has_data: false,
        }
    }

    #[test]
    fn import_memory_copies_file() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("instructions.md");
        std::fs::write(&source, "# Instructions\nAlways be concise.").unwrap();

        let dest_dir = tmp.path().join("dest");

        let mut detection = make_detection();
        detection.instructions_path = Some(source);

        let report = import_memory(&detection, &dest_dir);
        assert_eq!(report.status, ImportStatus::Success);
        assert_eq!(report.items_imported, 1);

        let content = std::fs::read_to_string(dest_dir.join("CODEX_INSTRUCTIONS.md")).unwrap();
        assert!(content.contains("Always be concise"));
    }

    #[test]
    fn import_memory_skips_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("instructions.md");
        std::fs::write(&source, "new content").unwrap();

        let dest_dir = tmp.path().join("dest");
        std::fs::create_dir_all(&dest_dir).unwrap();
        std::fs::write(dest_dir.join("CODEX_INSTRUCTIONS.md"), "existing content").unwrap();

        let mut detection = make_detection();
        detection.instructions_path = Some(source);

        let report = import_memory(&detection, &dest_dir);
        assert_eq!(report.items_skipped, 1);
        assert_eq!(report.items_imported, 0);

        let content = std::fs::read_to_string(dest_dir.join("CODEX_INSTRUCTIONS.md")).unwrap();
        assert_eq!(content, "existing content");
    }

    #[test]
    fn import_memory_no_source() {
        let tmp = tempfile::tempdir().unwrap();
        let detection = make_detection();
        let report = import_memory(&detection, tmp.path());
        assert_eq!(report.status, ImportStatus::Skipped);
    }
}
