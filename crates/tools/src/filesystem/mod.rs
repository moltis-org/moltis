//! Native filesystem tools.
//!
//! Provides `file_read` and `file_info` as built-in replacements for the
//! MCP filesystem server's read/info tools. Additional tools (write, edit,
//! tree, list, search, create_dir, move) will follow in later phases.

pub mod info;
pub mod read;

use {
    crate::error::Error,
    std::path::{Path, PathBuf},
};

/// Validate that a resolved (canonicalized) path falls within the allowed
/// directory list.
///
/// # Rules
/// - If `allowed_dirs` is empty, all paths are permitted (permissive mode).
/// - Otherwise the canonical path must start with one of the canonicalized
///   allowed-dir prefixes.  The prefix comparison is done with a trailing
///   separator so that `/tmp/foo` does *not* match allowed dir `/tmp/fo`.
///
/// # Errors
/// Returns an `Error` with a human-readable message listing the allowed dirs
/// when the path is outside the boundary.
pub(crate) fn check_allowed_dir(
    resolved_path: &Path,
    allowed_dirs: &[String],
) -> crate::Result<()> {
    if allowed_dirs.is_empty() {
        return Ok(());
    }

    let resolved_str = resolved_path.to_string_lossy();

    for dir in allowed_dirs {
        // Canonicalize the allowed dir once per check.  In production the
        // caller typically caches canonicalized prefixes, but since this
        // function is called per-execute the overhead is negligible.
        let canonical_dir = match std::fs::canonicalize(dir) {
            Ok(p) => p,
            Err(_) => continue, // non-existent allowed dir — skip
        };

        let canonical_str = canonical_dir.to_string_lossy();

        // Exact match (the resolved path *is* the allowed dir).
        if resolved_str == canonical_str {
            return Ok(());
        }

        // Prefix match — ensure trailing separator so `/tmp/fo` doesn't
        // match `/tmp/foo/bar`.
        let prefix = if canonical_str.ends_with('/') {
            canonical_str.into_owned()
        } else {
            format!("{canonical_str}/")
        };

        if resolved_str.starts_with(&prefix) {
            return Ok(());
        }
    }

    // Build a helpful message listing the allowed directories.
    let dir_list = allowed_dirs
        .iter()
        .map(|d| format!("  - {d}"))
        .collect::<Vec<_>>()
        .join("\n");

    Err(Error::message(format!(
        "path '{resolved_str}' is outside the allowed directories:\n{dir_list}"
    )))
}

/// Canonicalize a path, returning the canonical form or the original as a
/// fallback (e.g. when the file doesn't exist yet).
///
/// For allowed-dir enforcement we want to resolve symlinks first, so the
/// canonical form is preferred.  The fallback ensures we can still produce
/// a useful error when the path doesn't exist at all.
pub(crate) fn canonicalize_or_original(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- check_allowed_dir tests ---

    #[test]
    fn empty_allowed_dirs_allows_everything() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("some_file.txt");
        std::fs::write(&file, "data").unwrap();
        let resolved = canonicalize_or_original(&file);
        assert!(check_allowed_dir(&resolved, &[]).is_ok());
    }

    #[test]
    fn path_inside_allowed_dir_is_allowed() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("subdir").join("file.txt");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "data").unwrap();
        let resolved = canonicalize_or_original(&file);
        assert!(check_allowed_dir(&resolved, &[tmp.path().to_str().unwrap().to_string()]).is_ok());
    }

    #[test]
    fn path_outside_allowed_dir_is_rejected() {
        let allowed = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let file = other.path().join("secret.txt");
        std::fs::write(&file, "data").unwrap();
        let resolved = canonicalize_or_original(&file);
        let err = check_allowed_dir(&resolved, &[allowed.path().to_str().unwrap().to_string()])
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("outside the allowed directories"), "{msg}");
        assert!(msg.contains(allowed.path().to_str().unwrap()), "{msg}");
    }

    #[test]
    fn prefix_boundary_prevents_false_match() {
        // Create /tmp/test_prefix_guard and /tmp/test_prefix_guard_extra
        let base = tempfile::tempdir().unwrap();
        let allowed_dir = base.path().join("foo");
        let sneaky_dir = base.path().join("foo_secret");
        std::fs::create_dir_all(&allowed_dir).unwrap();
        std::fs::create_dir_all(&sneaky_dir).unwrap();

        let sneaky_file = sneaky_dir.join("data.txt");
        std::fs::write(&sneaky_file, "sneaky").unwrap();
        let resolved = canonicalize_or_original(&sneaky_file);

        let err =
            check_allowed_dir(&resolved, &[allowed_dir.to_str().unwrap().to_string()]).unwrap_err();
        assert!(
            err.to_string().contains("outside the allowed directories"),
            "{}",
            err.to_string()
        );
    }

    #[test]
    fn canonicalizes_allowed_dir_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let link = tmp.path().join("link");
        let target = tmp.path().join("target");
        std::fs::create_dir_all(&target).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let file = target.join("file.txt");
        std::fs::write(&file, "data").unwrap();
        let resolved = canonicalize_or_original(&file);

        // Allowed dir is specified via symlink — should still work because
        // both sides canonicalize to the same real path.
        assert!(check_allowed_dir(&resolved, &[link.to_str().unwrap().to_string()]).is_ok());
    }

    #[test]
    fn symlink_escape_is_detected() {
        let allowed = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();

        let link = allowed.path().join("escape_link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();

        let resolved = canonicalize_or_original(&link);
        let err = check_allowed_dir(&resolved, &[allowed.path().to_str().unwrap().to_string()])
            .unwrap_err();
        assert!(
            err.to_string().contains("outside the allowed directories"),
            "{}",
            err.to_string()
        );
    }

    #[test]
    fn exact_allowed_dir_match_is_allowed() {
        let tmp = tempfile::tempdir().unwrap();
        let resolved = canonicalize_or_original(tmp.path());
        assert!(check_allowed_dir(&resolved, &[tmp.path().to_str().unwrap().to_string()]).is_ok());
    }

    #[test]
    fn multiple_allowed_dirs() {
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();

        let file_in_dir2 = dir2.path().join("file.txt");
        std::fs::write(&file_in_dir2, "data").unwrap();
        let resolved = canonicalize_or_original(&file_in_dir2);

        assert!(
            check_allowed_dir(&resolved, &[
                dir1.path().to_str().unwrap().to_string(),
                dir2.path().to_str().unwrap().to_string(),
            ])
            .is_ok()
        );

        let file_outside = outside.path().join("other.txt");
        std::fs::write(&file_outside, "data").unwrap();
        let resolved_out = canonicalize_or_original(&file_outside);
        assert!(
            check_allowed_dir(&resolved_out, &[
                dir1.path().to_str().unwrap().to_string(),
                dir2.path().to_str().unwrap().to_string(),
            ])
            .is_err()
        );
    }

    #[test]
    fn non_existent_allowed_dir_is_skipped() {
        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("file.txt");
        std::fs::write(&file, "data").unwrap();
        let resolved = canonicalize_or_original(&file);

        // Only allowed dir is non-existent — file should be rejected.
        assert!(check_allowed_dir(&resolved, &["/nonexistent/path/12345".to_string()]).is_err());
    }
}
