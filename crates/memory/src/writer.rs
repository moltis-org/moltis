//! Path validation and text mutation helpers for memory writes.

use std::path::{Path, PathBuf};

const ROOT_MEMORY_FILES: [&str; 2] = ["MEMORY.md", "memory.md"];
const MEMORY_DIR_PREFIX: &str = "memory/";

/// Validate and resolve a memory write path relative to `data_dir`.
///
/// Allowed targets:
/// - `MEMORY.md`
/// - `memory.md`
/// - `memory/<name>.md` (single segment only)
pub fn validate_memory_path(data_dir: &Path, file: &str) -> crate::error::Result<PathBuf> {
    let path = file.trim();
    if path.is_empty() {
        return Err(crate::error::Error::Validation(
            "memory path cannot be empty".into(),
        ));
    }

    if Path::new(path).is_absolute() {
        return Err(crate::error::Error::Validation(
            "memory path must be relative".into(),
        ));
    }

    if path.contains('\\') {
        return Err(crate::error::Error::Validation(
            "memory path must use '/' separators".into(),
        ));
    }

    if ROOT_MEMORY_FILES.contains(&path) {
        return Ok(data_dir.join(path));
    }

    let Some(name) = path.strip_prefix(MEMORY_DIR_PREFIX) else {
        return Err(crate::error::Error::Validation(format!(
            "invalid memory path '{path}': allowed targets are MEMORY.md, memory.md, or memory/<name>.md"
        )));
    };

    if !is_valid_memory_file_name(name) {
        return Err(crate::error::Error::Validation(format!(
            "invalid memory path '{path}': allowed targets are MEMORY.md, memory.md, or memory/<name>.md"
        )));
    }

    Ok(data_dir.join(MEMORY_DIR_PREFIX).join(name))
}

#[derive(Debug, PartialEq, Eq)]
pub struct TextRemovalResult {
    pub content: String,
    pub matches_removed: usize,
}

/// Remove an exact snippet from memory content.
///
/// The snippet must be non-empty. If the exact text is not found, the helper
/// also tries a line-ending-normalized variant (`\n` <-> `\r\n`) so agents can
/// remove content they previously read from indexed chunks without caring about
/// platform-specific newlines.
pub fn remove_exact_text(
    content: &str,
    snippet: &str,
    remove_all: bool,
) -> crate::error::Result<TextRemovalResult> {
    if snippet.trim().is_empty() {
        return Err(crate::error::Error::Validation(
            "text to remove cannot be empty".into(),
        ));
    }

    let variants = text_variants(snippet);
    for candidate in variants {
        let matches_removed = content.match_indices(candidate.as_str()).count();
        if matches_removed == 0 {
            continue;
        }

        let updated = if remove_all {
            content.replace(candidate.as_str(), "")
        } else {
            content.replacen(candidate.as_str(), "", 1)
        };

        return Ok(TextRemovalResult {
            content: updated,
            matches_removed: if remove_all {
                matches_removed
            } else {
                1
            },
        });
    }

    Err(crate::error::Error::Validation(
        "text to remove was not found in the target memory file".into(),
    ))
}

fn text_variants(snippet: &str) -> Vec<String> {
    let mut variants = vec![snippet.to_string()];
    if snippet.contains("\r\n") {
        let lf = snippet.replace("\r\n", "\n");
        if lf != snippet {
            variants.push(lf);
        }
    } else if snippet.contains('\n') {
        variants.push(snippet.replace('\n', "\r\n"));
    }
    variants
}

fn is_valid_memory_file_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }

    // Exactly one level under memory/.
    if name.contains('/') {
        return false;
    }

    if !name.ends_with(".md") {
        return false;
    }

    if name.chars().any(char::is_whitespace) {
        return false;
    }

    // Reject empty stem (`.md`) and hidden-ish names (`.foo.md`).
    let stem = &name[..name.len() - 3];
    if stem.is_empty() || stem.starts_with('.') {
        return false;
    }

    true
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{remove_exact_text, validate_memory_path};

    #[test]
    fn allows_root_memory_files() {
        let root = Path::new("/tmp/moltis");

        assert_eq!(
            validate_memory_path(root, "MEMORY.md").unwrap(),
            root.join("MEMORY.md")
        );
        assert_eq!(
            validate_memory_path(root, "memory.md").unwrap(),
            root.join("memory.md")
        );
    }

    #[test]
    fn allows_single_level_memory_files() {
        let root = Path::new("/tmp/moltis");

        assert_eq!(
            validate_memory_path(root, "memory/notes.md").unwrap(),
            root.join("memory").join("notes.md")
        );
        assert_eq!(
            validate_memory_path(root, "memory/2026-02-14.md").unwrap(),
            root.join("memory").join("2026-02-14.md")
        );
    }

    #[test]
    fn rejects_invalid_paths() {
        let root = Path::new("/tmp/moltis");
        let invalid = [
            "",
            " ",
            "/etc/passwd",
            "../etc/passwd",
            "memory/../../secret.md",
            "memory/a/b.md",
            "memory/.md",
            "memory/.hidden.md",
            "memory/notes.txt",
            "memory/a b.md",
            "random.md",
            "foo/bar.md",
            "memory\\notes.md",
        ];

        for item in invalid {
            assert!(
                validate_memory_path(root, item).is_err(),
                "expected invalid path: {item}"
            );
        }
    }

    #[test]
    fn remove_exact_text_removes_first_match_by_default() {
        let result = remove_exact_text("alpha\nbeta\nalpha\n", "alpha\n", false).unwrap();
        assert_eq!(result.matches_removed, 1);
        assert_eq!(result.content, "beta\nalpha\n");
    }

    #[test]
    fn remove_exact_text_removes_all_matches_when_requested() {
        let result = remove_exact_text("alpha\nbeta\nalpha\n", "alpha\n", true).unwrap();
        assert_eq!(result.matches_removed, 2);
        assert_eq!(result.content, "beta\n");
    }

    #[test]
    fn remove_exact_text_accepts_line_ending_variant() {
        let result = remove_exact_text("alpha\r\nbeta\r\n", "alpha\n", false).unwrap();
        assert_eq!(result.matches_removed, 1);
        assert_eq!(result.content, "beta\r\n");
    }

    #[test]
    fn remove_exact_text_rejects_missing_or_empty_text() {
        assert!(remove_exact_text("alpha", "", false).is_err());
        assert!(remove_exact_text("alpha", " ", false).is_err());
        assert!(remove_exact_text("alpha", "beta", false).is_err());
    }
}
