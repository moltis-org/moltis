//! Git-tracked file discovery.
//!
//! Enumerates files tracked by git for a given project directory,
//! respecting `.gitignore` and submodule boundaries.

use std::path::{Path, PathBuf};

use gix::bstr::ByteSlice;

#[cfg(feature = "tracing")]
use crate::log::{debug, info};

use crate::error::{Error, Result};

/// Discover all git-tracked files in a repository rooted at `repo_dir`.
///
/// Uses `gix::discover` to open the repository and reads the index
/// (staging area) to enumerate tracked blobs. Submodule paths are
/// excluded. The returned paths are relative to the repository work
/// tree root.
pub fn discover_tracked_files(repo_dir: &Path) -> Result<Vec<PathBuf>> {
    // Force full trust so gix skips ownership checks.  In CI containers
    // the workspace owner differs from the runner uid.
    let open_opts = gix::open::Options::default()
        .permissions(gix::open::Permissions::all())
        .with(gix::sec::Trust::Full);
    let repo = gix::open_opts(repo_dir, open_opts).map_err(|e| Error::GitRepoNotFound {
        path: repo_dir.to_path_buf(),
        message: e.to_string(),
    })?;

    let work_dir = repo
        .workdir()
        .ok_or_else(|| {
            Error::Config(
                "bare repository has no work tree; code index requires a working tree".into(),
            )
        })?
        .to_path_buf();

    #[cfg(feature = "tracing")]
    info!(
        work_dir = %work_dir.display(),
        "discovered git repository for code indexing"
    );

    // Read the git index (staging area) to get tracked files.
    // This includes committed files and staged files, but not untracked
    // or .gitignored files.
    let index = repo.index().map_err(|e| Error::IndexFailed {
        project_id: String::new(),
        message: format!("failed to read git index: {e}"),
    })?;

    // Collect submodule paths to exclude.
    let module_paths = collect_submodule_paths(repo_dir);

    let mut tracked = Vec::new();
    for entry in index.entries() {
        let entry_path = entry.path(&index);
        let rel_path = PathBuf::from(entry_path.to_str_lossy().as_ref());

        // Skip submodule entries.
        if module_paths.iter().any(|mp| rel_path.starts_with(mp)) {
            #[cfg(feature = "tracing")]
            debug!(path = %rel_path.display(), "skipping submodule path");
            continue;
        }

        let abs_path = work_dir.join(&rel_path);

        // Only include files that actually exist on disk.
        // Index entries may reference files that have been removed but
        // not yet staged for deletion.
        if abs_path.is_file() {
            tracked.push(rel_path);
        }
    }

    #[cfg(feature = "tracing")]
    debug!(count = tracked.len(), "enumerated git-tracked files");

    Ok(tracked)
}

/// Collect submodule paths from `.gitmodules`, if present.
///
/// Parses the `.gitmodules` file directly. Simple line-by-line parsing
/// handles the common case without needing full INI parsing.
fn collect_submodule_paths(repo_dir: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let gitmodules_path = repo_dir.join(".gitmodules");

    let content = match std::fs::read_to_string(&gitmodules_path) {
        Ok(c) => c,
        Err(_) => return paths, // No .gitmodules file — no submodules.
    };

    let mut current_is_submodule = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("[submodule") {
            current_is_submodule = true;
        } else if trimmed.starts_with('[') {
            current_is_submodule = false;
        } else if current_is_submodule && let Some(path_val) = trimmed.strip_prefix("path =") {
            let path_val = path_val.trim();
            if !path_val.is_empty() {
                paths.push(PathBuf::from(path_val));
            }
        }
    }

    paths
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_discover_on_nonexistent_dir() {
        let result = discover_tracked_files(Path::new("/nonexistent/path/that/does/not/exist"));
        assert!(result.is_err(), "should fail for nonexistent directory");
    }

    /// Return the workspace repo root if `.git` exists (i.e. a real clone,
    /// not an archive checkout). Returns `None` in CI containers that use
    /// tarball downloads where there is no `.git` directory.
    fn repo_root_if_git() -> Option<&'static Path> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        root.join(".git").exists().then_some(root)
    }

    #[test]
    fn test_discover_on_moltis_repo() {
        let Some(repo_dir) = repo_root_if_git() else {
            eprintln!("skipping: no .git directory (archive checkout)");
            return;
        };
        let files = discover_tracked_files(repo_dir).unwrap();
        assert!(!files.is_empty(), "moltis repo should have tracked files");

        // Key files that must be present.
        assert!(
            files
                .iter()
                .any(|f| f.to_string_lossy().ends_with("Cargo.toml")),
            "root Cargo.toml should be tracked"
        );

        // .gitignored files should not appear.
        assert!(
            !files
                .iter()
                .any(|f| f.to_string_lossy().starts_with("target/")),
            "target/ files should not be tracked"
        );
    }
}
