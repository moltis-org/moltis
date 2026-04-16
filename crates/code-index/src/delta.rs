//! Delta computation for incremental reindexing.
//!
//! Compares the current state of indexable files against a previously
//! known state to produce a [`SyncDelta`] — the set of files added,
//! removed, or modified since the last index. This enables efficient
//! partial reindexing instead of re-scanning the entire project.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use crate::{
    config::CodeIndexConfig,
    discover::discover_tracked_files,
    error::Result,
    filter::{content_hash, filter_tracked_files},
    types::FilteredFile,
};

/// The set of changes between two index snapshots.
#[derive(Debug, Clone)]
pub struct SyncDelta {
    /// Files that are new since the last index (not in previous snapshot).
    pub added: Vec<FilteredFile>,
    /// Files that were removed since the last index (in previous, not current).
    pub removed: Vec<String>,
    /// Files that exist in both but whose content hash changed.
    pub modified: Vec<FilteredFile>,
}

/// A snapshot of file hashes from a previous indexing run.
///
/// Maps `relative_path → content_hash`.
pub type HashSnapshot = HashMap<String, String>;

/// Compute the delta between the current project state and a previous hash snapshot.
///
/// 1. Discover git-tracked files
/// 2. Filter by extension, size, binary
/// 3. Compute content hashes for all filtered files
/// 4. Compare against the previous snapshot to find added/removed/modified
///
/// Returns the delta and the current hash snapshot (for use in the next delta).
pub fn compute_delta(
    project_dir: &Path,
    config: &CodeIndexConfig,
    previous: &HashSnapshot,
) -> Result<(SyncDelta, HashSnapshot)> {
    let tracked = discover_tracked_files(project_dir)?;
    let filtered = filter_tracked_files(project_dir, &tracked, config)?;

    let previous_paths: HashSet<&str> = previous.keys().map(|s| s.as_str()).collect();

    let mut added = Vec::new();
    let mut modified = Vec::new();
    let mut current_snapshot = HashMap::new();

    for file in &filtered {
        let rel_str = file.relative_path.to_string_lossy().into_owned();

        // Compute current content hash.
        let hash = match content_hash(&file.path) {
            Ok(h) => h,
            Err(e) => {
                tracing::debug!(
                    path = %file.relative_path.display(),
                    error = %e,
                    "skipping file: cannot compute content hash"
                );
                // Carry forward the previous hash so the file isn't spuriously
                // marked as "removed" on the next delta call. If the file is
                // new (not in previous), it's simply omitted from this cycle.
                if let Some(prev_hash) = previous.get(rel_str.as_str()) {
                    current_snapshot.insert(rel_str.clone(), prev_hash.clone());
                }
                continue;
            },
        };

        current_snapshot.insert(rel_str.clone(), hash.clone());

        if let Some(prev_hash) = previous.get(rel_str.as_str()) {
            if prev_hash != &hash {
                // File exists in both but hash changed.
                modified.push(file.clone());
            }
            // else: unchanged, no action needed.
        } else {
            // New file not in previous snapshot.
            added.push(file.clone());
        }
    }

    // Find removed files: in previous but not in current filtered set.
    let current_paths: HashSet<&str> = current_snapshot.keys().map(|s| s.as_str()).collect();

    let removed = previous_paths
        .iter()
        .filter(|p| !current_paths.contains(*p))
        .map(|p| (*p).to_string())
        .collect();

    let delta = SyncDelta {
        added,
        removed,
        modified,
    };

    Ok((delta, current_snapshot))
}

/// Build a hash snapshot from the current filtered file set.
///
/// Convenience function for the initial index (no previous snapshot).
/// Equivalent to calling [`compute_delta`] with an empty previous snapshot.
pub fn build_initial_snapshot(
    project_dir: &Path,
    config: &CodeIndexConfig,
) -> Result<HashSnapshot> {
    let tracked = discover_tracked_files(project_dir)?;
    let filtered = filter_tracked_files(project_dir, &tracked, config)?;

    let mut snapshot = HashMap::new();

    for file in &filtered {
        let rel_str = file.relative_path.to_string_lossy().into_owned();
        let hash = match content_hash(&file.path) {
            Ok(h) => h,
            Err(e) => {
                tracing::debug!(
                    path = %file.relative_path.display(),
                    error = %e,
                    "skipping file: cannot compute content hash"
                );
                continue;
            },
        };
        snapshot.insert(rel_str, hash);
    }

    Ok(snapshot)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn test_config() -> CodeIndexConfig {
        CodeIndexConfig::default()
    }

    #[test]
    fn test_compute_delta_empty_previous() {
        // With an empty previous snapshot, all files should be "added".
        let repo_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();

        let config = test_config();
        let previous = HashMap::new();

        let (delta, _snapshot) = compute_delta(repo_dir, &config, &previous).unwrap();
        assert!(
            !delta.added.is_empty(),
            "all files should be added with empty previous snapshot"
        );
        assert!(
            delta.removed.is_empty(),
            "nothing should be removed with empty previous snapshot"
        );
        assert!(
            delta.modified.is_empty(),
            "nothing should be modified with empty previous snapshot"
        );
    }

    #[test]
    fn test_compute_delta_identical_snapshot() {
        // With the current snapshot as previous, nothing should change.
        let repo_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();

        let config = test_config();

        // Build an initial snapshot.
        let previous = build_initial_snapshot(repo_dir, &config).unwrap();
        assert!(!previous.is_empty(), "snapshot should have entries");

        // Compare against itself — no changes.
        let (delta, _) = compute_delta(repo_dir, &config, &previous).unwrap();
        assert!(
            delta.added.is_empty(),
            "no new files should be added against identical snapshot"
        );
        assert!(
            delta.removed.is_empty(),
            "no files should be removed against identical snapshot"
        );
        assert!(
            delta.modified.is_empty(),
            "no files should be modified against identical snapshot"
        );
    }

    #[test]
    fn test_compute_delta_simulated_removal() {
        // With a previous snapshot containing a fake extra file, it should
        // show up as "removed" since it doesn't exist on disk.
        let repo_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();

        let config = test_config();
        let mut previous = build_initial_snapshot(repo_dir, &config).unwrap();

        // Insert a fake file that doesn't exist on disk.
        previous.insert("fake/deleted_file.rs".to_string(), "abc123".to_string());

        let (delta, _) = compute_delta(repo_dir, &config, &previous).unwrap();
        assert!(
            delta.removed.contains(&"fake/deleted_file.rs".to_string()),
            "fake entry should show as removed"
        );
    }

    #[test]
    fn test_build_initial_snapshot_populates_hashes() {
        let repo_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();

        let config = test_config();
        let snapshot = build_initial_snapshot(repo_dir, &config).unwrap();

        // All hashes should be 64-character hex SHA-256 strings.
        for (path, hash) in &snapshot {
            assert!(
                hash.len() == 64,
                "hash for {path} should be 64 hex chars, got {}",
                hash.len()
            );
            assert!(
                hash.chars().all(|c| c.is_ascii_hexdigit()),
                "hash for {path} should be hex, got {hash}"
            );
        }
    }
}
