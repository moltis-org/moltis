//! Path validation and text mutation helpers for memory writes.

use std::{
    fs,
    path::{Path, PathBuf},
};

const ROOT_MEMORY_FILES: [&str; 2] = ["MEMORY.md", "memory.md"];
const LEGACY_MEMORY_DIR: &str = "memory";

/// Validate and resolve a memory write path.
///
/// `data_dir` is the workspace root. `writable_roots` lists additional roots
/// (absolute, or relative to `data_dir`) into which writes are allowed; pass
/// an empty slice for legacy behaviour (only the `memory/` subtree under
/// `data_dir` plus the root `MEMORY.md`/`memory.md` shortcuts).
///
/// Segment rules — every path segment must be non-empty, contain no
/// whitespace, not equal `.` or `..`, and not begin with `.`. The final
/// segment must end in `.md` with a non-empty stem. Absolute inputs and
/// backslash separators are always rejected. After syntactic resolution,
/// the existing ancestor of the target is canonicalised and must lie under
/// an allowed root — this catches pre-existing symlinks that would
/// otherwise escape the workspace.
pub fn validate_memory_path(
    data_dir: &Path,
    writable_roots: &[PathBuf],
    file: &str,
) -> crate::error::Result<PathBuf> {
    let path = file.trim();
    if path.is_empty() {
        return Err(invalid("memory path cannot be empty"));
    }
    if Path::new(path).is_absolute() {
        return Err(invalid("memory path must be relative"));
    }
    if path.contains('\\') {
        return Err(invalid("memory path must use '/' separators"));
    }

    let is_root_file = ROOT_MEMORY_FILES.contains(&path);
    let segments: Vec<&str> = path.split('/').collect();
    for (i, seg) in segments.iter().enumerate() {
        validate_segment(seg, i + 1 == segments.len(), path)?;
    }

    // We compare a fully-resolved target against fully-resolved roots so
    // pre-existing symlinks inside the workspace can't be used to escape it.
    // The caller-facing return path keeps the original `data_dir` spelling.
    let canon_data_dir = canonicalize_or_self(data_dir);
    let symlink_safe_target = symlink_safe_resolve(&canon_data_dir.join(path));
    let allowed = allowed_roots(&canon_data_dir, writable_roots, is_root_file);
    if !allowed
        .iter()
        .any(|root| symlink_safe_target.starts_with(root))
    {
        return Err(invalid(format!(
            "memory path '{}' resolves outside any configured memory root",
            data_dir.join(path).display()
        )));
    }
    Ok(data_dir.join(path))
}

fn validate_segment(seg: &str, is_last: bool, full: &str) -> crate::error::Result<()> {
    if seg.is_empty() {
        return Err(invalid(format!(
            "invalid memory path '{full}': empty segment"
        )));
    }
    if seg == "." || seg == ".." {
        return Err(invalid(format!(
            "invalid memory path '{full}': traversal segment '{seg}'"
        )));
    }
    if seg.chars().any(char::is_whitespace) {
        return Err(invalid(format!(
            "invalid memory path '{full}': whitespace in segment '{seg}'"
        )));
    }
    if seg.starts_with('.') {
        return Err(invalid(format!(
            "invalid memory path '{full}': segment '{seg}' starts with '.'"
        )));
    }
    if is_last {
        if !seg.ends_with(".md") {
            return Err(invalid(format!(
                "invalid memory path '{full}': last segment must end with .md"
            )));
        }
        let stem = &seg[..seg.len() - 3];
        if stem.is_empty() {
            return Err(invalid(format!(
                "invalid memory path '{full}': empty filename stem"
            )));
        }
    }
    Ok(())
}

/// Build the canonical list of permitted roots for a write attempt.
///
/// `canon_data_dir` is the already-canonicalised data root. `include_data_dir`
/// is set only for the root-file shortcuts (`MEMORY.md` / `memory.md`), which
/// must resolve directly under `data_dir`. For all other paths, only the
/// legacy `memory/` subtree (when no roots are configured) or the configured
/// `writable_roots` are permitted.
fn allowed_roots(
    canon_data_dir: &Path,
    writable_roots: &[PathBuf],
    include_data_dir: bool,
) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    let push = |list: &mut Vec<PathBuf>, p: PathBuf| {
        if !list.iter().any(|existing| existing == &p) {
            list.push(p);
        }
    };

    if include_data_dir {
        push(&mut roots, canon_data_dir.to_path_buf());
    }

    if writable_roots.is_empty() {
        push(&mut roots, canon_data_dir.join(LEGACY_MEMORY_DIR));
    } else {
        for root in writable_roots {
            let abs = if root.is_absolute() {
                root.clone()
            } else {
                canon_data_dir.join(root)
            };
            push(&mut roots, canonicalize_or_self(&abs));
        }
    }
    roots
}

fn canonicalize_or_self(p: &Path) -> PathBuf {
    fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Resolve `path` to its symlink-safe canonical form, even if the leaf does
/// not yet exist. Walks up until an ancestor that exists is found,
/// canonicalises it, then re-attaches the remaining (already-validated)
/// trailing segments. This catches pre-existing symlinks along the path that
/// would otherwise allow a write to escape the workspace.
fn symlink_safe_resolve(path: &Path) -> PathBuf {
    let mut current = path.to_path_buf();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(c) = fs::canonicalize(&current) {
            let mut result = c;
            for seg in tail.iter().rev() {
                result.push(seg);
            }
            return result;
        }
        let Some(name) = current.file_name().map(|n| n.to_os_string()) else {
            return path.to_path_buf();
        };
        tail.push(name);
        let Some(parent) = current.parent().map(Path::to_path_buf) else {
            return path.to_path_buf();
        };
        current = parent;
    }
}

fn invalid(msg: impl Into<String>) -> crate::error::Error {
    crate::error::Error::Validation(msg.into())
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

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use tempfile::TempDir;

    use super::{remove_exact_text, validate_memory_path};

    /// Canonicalize a path the same way the validator does so tests work on
    /// macOS where TempDir paths get `/private` prefixed by realpath(3).
    fn canon(p: &std::path::Path) -> PathBuf {
        fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
    }

    fn tmp_with_memory_dir() -> TempDir {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("memory")).unwrap();
        tmp
    }

    #[test]
    fn allows_root_memory_files() {
        let tmp = tmp_with_memory_dir();
        let root = tmp.path();

        assert_eq!(
            canon(&validate_memory_path(root, &[], "MEMORY.md").unwrap()),
            canon(&root.join("MEMORY.md"))
        );
        assert_eq!(
            canon(&validate_memory_path(root, &[], "memory.md").unwrap()),
            canon(&root.join("memory.md"))
        );
    }

    #[test]
    fn allows_single_level_memory_files() {
        let tmp = tmp_with_memory_dir();
        let root = tmp.path();

        assert_eq!(
            canon(&validate_memory_path(root, &[], "memory/notes.md").unwrap()),
            canon(&root.join("memory").join("notes.md"))
        );
        assert_eq!(
            canon(&validate_memory_path(root, &[], "memory/2026-02-14.md").unwrap()),
            canon(&root.join("memory").join("2026-02-14.md"))
        );
    }

    #[test]
    fn allows_nested_memory_subfolders() {
        let tmp = tmp_with_memory_dir();
        let root = tmp.path();

        let resolved = validate_memory_path(root, &[], "memory/work/projects/notes.md").unwrap();
        assert_eq!(
            canon(&resolved),
            canon(&root.join("memory/work/projects/notes.md"))
        );

        let resolved = validate_memory_path(root, &[], "memory/a/b/c/d.md").unwrap();
        assert_eq!(canon(&resolved), canon(&root.join("memory/a/b/c/d.md")));
    }

    #[test]
    fn rejects_invalid_paths() {
        let tmp = tmp_with_memory_dir();
        let root = tmp.path();
        let invalid = [
            "",
            " ",
            "/etc/passwd",
            "../etc/passwd",
            "memory/../../secret.md",
            "memory/./notes.md",
            "memory/.md",
            "memory/.hidden.md",
            "memory/notes.txt",
            "memory/a b.md",
            "memory/sub/ /notes.md",
            "memory/sub/.hidden/notes.md",
            "memory/sub//notes.md",
            "random.md",
            "foo/bar.md",
            "memory\\notes.md",
        ];

        for item in invalid {
            assert!(
                validate_memory_path(root, &[], item).is_err(),
                "expected invalid path: {item}"
            );
        }
    }

    #[test]
    fn roots_mode_accepts_paths_under_configured_root() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();
        fs::create_dir_all(data_dir.join("agents")).unwrap();
        let roots = vec![data_dir.join("agents")];

        let resolved = validate_memory_path(data_dir, &roots, "agents/foo/notes.md").unwrap();
        assert_eq!(
            canon(&resolved),
            canon(&data_dir.join("agents/foo/notes.md"))
        );

        // Multi-segment under the configured root is fine.
        let resolved = validate_memory_path(data_dir, &roots, "agents/foo/sub/log.md").unwrap();
        assert_eq!(
            canon(&resolved),
            canon(&data_dir.join("agents/foo/sub/log.md"))
        );

        // Root-file shortcuts still work.
        let resolved = validate_memory_path(data_dir, &roots, "MEMORY.md").unwrap();
        assert_eq!(canon(&resolved), canon(&data_dir.join("MEMORY.md")));
    }

    #[test]
    fn roots_mode_rejects_paths_outside_roots() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();
        fs::create_dir_all(data_dir.join("agents")).unwrap();
        fs::create_dir_all(data_dir.join("memory")).unwrap();
        let roots = vec![data_dir.join("agents")];

        // memory/ is no longer implicitly writable when roots are configured.
        assert!(validate_memory_path(data_dir, &roots, "memory/notes.md").is_err());
        // Random sibling of an allowed root is rejected.
        assert!(validate_memory_path(data_dir, &roots, "outside/notes.md").is_err());
    }

    #[test]
    fn roots_mode_includes_memory_when_listed() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();
        fs::create_dir_all(data_dir.join("memory")).unwrap();
        fs::create_dir_all(data_dir.join("agents")).unwrap();
        let roots = vec![data_dir.join("memory"), data_dir.join("agents")];

        assert!(validate_memory_path(data_dir, &roots, "memory/work/notes.md").is_ok());
        assert!(validate_memory_path(data_dir, &roots, "agents/foo/notes.md").is_ok());
        assert!(validate_memory_path(data_dir, &roots, "elsewhere/notes.md").is_err());
    }

    #[test]
    fn roots_mode_accepts_relative_root_specifier() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();
        fs::create_dir_all(data_dir.join("agents")).unwrap();
        let roots = vec![PathBuf::from("agents")];

        assert!(validate_memory_path(data_dir, &roots, "agents/foo.md").is_ok());
        assert!(validate_memory_path(data_dir, &roots, "memory/foo.md").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape_from_allowed_root() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();
        let outside = TempDir::new().unwrap();
        fs::create_dir_all(data_dir.join("memory")).unwrap();
        std::os::unix::fs::symlink(outside.path(), data_dir.join("memory/escape")).unwrap();

        // The symlinked subtree points outside the data_dir, so writes there
        // must be rejected even though the syntactic path looks clean.
        let result = validate_memory_path(data_dir, &[], "memory/escape/notes.md");
        assert!(result.is_err(), "symlink escape must be rejected");
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
