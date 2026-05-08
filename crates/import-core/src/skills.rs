//! Shared skill import utilities.
//!
//! Provides directory-copy logic for SKILL.md-based skills, reusable
//! across all import sources.

use std::path::Path;

use tracing::{debug, warn};

use crate::report::{CategoryReport, ImportCategory, ImportStatus};

/// Copy skill directories from `source_dirs` to `dest_skills_dir`.
///
/// Each source directory must contain a `SKILL.md` file. Skills with names
/// that already exist in the destination are skipped (not overwritten).
pub fn copy_skill_dirs(source_dirs: &[(String, &Path)], dest_skills_dir: &Path) -> CategoryReport {
    if source_dirs.is_empty() {
        return CategoryReport::skipped(ImportCategory::Skills);
    }

    if let Err(e) = std::fs::create_dir_all(dest_skills_dir) {
        return CategoryReport::failed(
            ImportCategory::Skills,
            format!("failed to create skills directory: {e}"),
        );
    }

    let mut imported = 0;
    let mut skipped = 0;
    let mut warnings = Vec::new();

    for (name, source) in source_dirs {
        let dest = dest_skills_dir.join(name);
        if dest.exists() {
            debug!(name, "skill already exists, skipping");
            skipped += 1;
            continue;
        }

        if let Err(e) = copy_dir_recursive(source, &dest) {
            warn!(name, error = %e, "failed to copy skill");
            warnings.push(format!("failed to copy skill '{name}': {e}"));
            continue;
        }

        debug!(name, "imported skill");
        imported += 1;
    }

    let status = if imported == 0 && warnings.is_empty() {
        ImportStatus::Skipped
    } else if !warnings.is_empty() {
        ImportStatus::Partial
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
        errors: Vec::new(),
    }
}

/// Create a skill from a markdown command file, wrapping it in SKILL.md frontmatter.
///
/// Returns `Ok(true)` if created, `Ok(false)` if skipped (already exists).
pub fn create_skill_from_command(
    name: &str,
    source_label: &str,
    markdown_content: &str,
    dest_skills_dir: &Path,
) -> std::io::Result<bool> {
    let skill_dir = dest_skills_dir.join(name);
    if skill_dir.exists() {
        return Ok(false);
    }

    let description =
        first_paragraph(markdown_content).unwrap_or_else(|| format!("Imported command {name}"));
    let truncated = truncate_to_char_boundary(&description, 180);

    let content = format!(
        "---\nname: {name}\ndescription: {desc}\n---\n\n<!-- Imported from: {source_label} -->\n\n{body}\n",
        name = name,
        desc = serde_json::to_string(truncated).unwrap_or_else(|_| format!("\"{truncated}\"")),
        source_label = source_label,
        body = markdown_content.trim_end(),
    );

    std::fs::create_dir_all(&skill_dir)?;
    std::fs::write(skill_dir.join("SKILL.md"), content)?;
    Ok(true)
}

/// Extract the first non-empty paragraph from markdown content (stripping frontmatter).
fn first_paragraph(content: &str) -> Option<String> {
    let stripped = if content.starts_with("---") {
        // Strip YAML frontmatter
        content
            .strip_prefix("---")
            .and_then(|rest| rest.split_once("---"))
            .map_or(content, |(_, after)| after)
    } else {
        content
    };

    stripped
        .split("\n\n")
        .map(|part| part.split_whitespace().collect::<Vec<_>>().join(" "))
        .find(|part| !part.is_empty())
}

/// Truncate a string to at most `max_bytes` bytes on a valid char boundary.
fn truncate_to_char_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    // Walk backwards from max_bytes to find a char boundary
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Recursively copy a directory.
fn copy_dir_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest_path = dest.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else {
            std::fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn copy_skill_dirs_works() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src-skill");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("SKILL.md"), "---\nname: test\n---\nDo stuff.").unwrap();

        let dest = tmp.path().join("skills");
        let sources = vec![("test-skill".to_string(), src.as_path())];
        let report = copy_skill_dirs(&sources, &dest);

        assert_eq!(report.status, ImportStatus::Success);
        assert_eq!(report.items_imported, 1);
        assert!(dest.join("test-skill").join("SKILL.md").is_file());
    }

    #[test]
    fn copy_skill_dirs_skips_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src-skill");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("SKILL.md"), "content").unwrap();

        let dest = tmp.path().join("skills");
        let existing = dest.join("test-skill");
        std::fs::create_dir_all(&existing).unwrap();
        std::fs::write(existing.join("SKILL.md"), "existing").unwrap();

        let sources = vec![("test-skill".to_string(), src.as_path())];
        let report = copy_skill_dirs(&sources, &dest);

        assert_eq!(report.items_skipped, 1);
        assert_eq!(report.items_imported, 0);
        // Existing content should be preserved
        let content = std::fs::read_to_string(dest.join("test-skill").join("SKILL.md")).unwrap();
        assert_eq!(content, "existing");
    }

    #[test]
    fn create_skill_from_command_wraps_markdown() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("skills");

        let created =
            create_skill_from_command("my-cmd", "Claude command", "# Hello\n\nDo things.", &dest)
                .unwrap();

        assert!(created);
        let content = std::fs::read_to_string(dest.join("my-cmd").join("SKILL.md")).unwrap();
        assert!(content.contains("name: my-cmd"));
        assert!(content.contains("# Hello"));
        assert!(content.contains("Imported from: Claude command"));
    }

    #[test]
    fn create_skill_from_command_skips_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("skills");
        std::fs::create_dir_all(dest.join("my-cmd")).unwrap();

        let created = create_skill_from_command("my-cmd", "test", "content", &dest).unwrap();
        assert!(!created);
    }

    #[test]
    fn first_paragraph_strips_frontmatter() {
        let md = "---\nname: test\n---\n\nFirst paragraph here.\n\nSecond paragraph.";
        assert_eq!(
            first_paragraph(md),
            Some("First paragraph here.".to_string())
        );
    }
}
