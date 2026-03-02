//! Path validation helpers for memory writes.

use std::path::{Path, PathBuf};

const ROOT_MEMORY_FILES: [&str; 2] = ["MEMORY.md", "memory.md"];
const MEMORY_DIR_PREFIX: &str = "memory/";

/// Validate and resolve a memory write path relative to `data_dir`.
///
/// Allowed targets:
/// - `MEMORY.md`
/// - `memory.md`
/// - `memory/<name>.md` (single segment only)
pub fn validate_memory_path(data_dir: &Path, file: &str) -> anyhow::Result<PathBuf> {
    let path = file.trim();
    if path.is_empty() {
        anyhow::bail!("memory path cannot be empty");
    }

    if Path::new(path).is_absolute() {
        anyhow::bail!("memory path must be relative");
    }

    if path.contains('\\') {
        anyhow::bail!("memory path must use '/' separators");
    }

    if ROOT_MEMORY_FILES.contains(&path) {
        return Ok(data_dir.join(path));
    }

    let Some(name) = path.strip_prefix(MEMORY_DIR_PREFIX) else {
        anyhow::bail!(
            "invalid memory path '{path}': allowed targets are MEMORY.md, memory.md, or memory/<name>.md"
        );
    };

    if !is_valid_memory_file_name(name) {
        anyhow::bail!(
            "invalid memory path '{path}': allowed targets are MEMORY.md, memory.md, or memory/<name>.md"
        );
    }

    Ok(data_dir.join(MEMORY_DIR_PREFIX).join(name))
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

    use super::validate_memory_path;

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
}
