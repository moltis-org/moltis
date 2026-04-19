//! Delta computation for incremental reindexing.
//!
//! Compares the current state of indexable files against a previously
//! known state to produce a [`SyncDelta`] — the set of files added,
//! removed, or modified since the last index. This enables efficient
//! partial reindexing instead of re-scanning the entire project.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    time::UNIX_EPOCH,
};

use serde::{Deserialize, Serialize};

#[cfg(feature = "tracing")]
use crate::log::debug;

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

/// Metadata for a file stored in the hash snapshot.
///
/// Carries enough information to short-circuit content hashing on
/// incremental runs: if `modified_time` and `size` both match the
/// previous entry, the file is assumed unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileMeta {
    /// SHA-256 content hash (hex-encoded).
    pub content_hash: String,
    /// File modification time as Unix epoch seconds.
    pub modified_time: u64,
    /// File size in bytes.
    pub size: u64,
}

/// A snapshot of file metadata from a previous indexing run.
///
/// Maps `relative_path → FileMeta`. Used for incremental delta
/// computation to avoid re-hashing unchanged files.
pub type HashSnapshot = HashMap<String, FileMeta>;

/// Compute the delta between the current project state and a previous hash snapshot.
///
/// 1. Discover git-tracked files
/// 2. Filter by extension, size, binary
/// 3. For each filtered file, stat for mtime+size; skip hash if unchanged
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

        // Stat the file to get mtime + size for cheap change detection.
        let meta = match std::fs::metadata(&file.path) {
            Ok(m) => m,
            Err(e) => {
                #[cfg(feature = "tracing")]
                debug!(
                    path = %file.relative_path.display(),
                    error = %e,
                    "skipping file: stat failed"
                );
                // Carry forward previous metadata so the file isn't spuriously
                // marked as "removed" on the next delta call.
                if let Some(prev) = previous.get(rel_str.as_str()) {
                    current_snapshot.insert(rel_str.clone(), prev.clone());
                }
                continue;
            },
        };

        let modified_time = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());
        let size = meta.len();

        // Fast path: if file exists in previous snapshot with matching
        // mtime and size, carry forward the hash without re-reading.
        if let Some(prev_meta) = previous.get(rel_str.as_str())
            && prev_meta.modified_time == modified_time
            && prev_meta.size == size
        {
            current_snapshot.insert(rel_str.clone(), prev_meta.clone());
            continue;
        }

        // Slow path: compute content hash.
        let hash = match content_hash(&file.path) {
            Ok(h) => h,
            Err(e) => {
                #[cfg(feature = "tracing")]
                debug!(
                    path = %file.relative_path.display(),
                    error = %e,
                    "skipping file: cannot compute content hash"
                );
                // Carry forward previous metadata so the file isn't spuriously
                // marked as "removed" on the next delta call. If the file is
                // new (not in previous), it's simply omitted from this cycle.
                if let Some(prev) = previous.get(rel_str.as_str()) {
                    current_snapshot.insert(rel_str.clone(), prev.clone());
                }
                continue;
            },
        };

        let file_meta = FileMeta {
            content_hash: hash.clone(),
            modified_time,
            size,
        };
        current_snapshot.insert(rel_str.clone(), file_meta.clone());

        if let Some(prev_meta) = previous.get(rel_str.as_str()) {
            if prev_meta.content_hash != hash {
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

        let meta = match std::fs::metadata(&file.path) {
            Ok(m) => m,
            Err(e) => {
                #[cfg(feature = "tracing")]
                debug!(
                    path = %file.relative_path.display(),
                    error = %e,
                    "skipping file: stat failed"
                );
                continue;
            },
        };

        let modified_time = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());

        let hash = match content_hash(&file.path) {
            Ok(h) => h,
            Err(e) => {
                #[cfg(feature = "tracing")]
                debug!(
                    path = %file.relative_path.display(),
                    error = %e,
                    "skipping file: cannot compute content hash"
                );
                continue;
            },
        };

        snapshot.insert(rel_str, FileMeta {
            content_hash: hash,
            modified_time,
            size: meta.len(),
        });
    }

    Ok(snapshot)
}

/// Build a hash snapshot from already-filtered files (no discovery pass).
///
/// Use this when the caller has already run `discover` + `filter`,
/// to avoid a TOCTOU window from double-scanning the filesystem.
pub fn build_snapshot_from_filtered(filtered: &[FilteredFile]) -> HashSnapshot {
    let mut snapshot = HashMap::new();
    for file in filtered {
        let rel_str = file.relative_path.to_string_lossy().into_owned();

        let meta = match std::fs::metadata(&file.path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let modified_time = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());

        if let Ok(hash) = content_hash(&file.path) {
            snapshot.insert(rel_str, FileMeta {
                content_hash: hash,
                modified_time,
                size: meta.len(),
            });
        }
    }
    snapshot
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
        let previous = HashSnapshot::new();

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
        previous.insert("fake/deleted_file.rs".to_string(), FileMeta {
            content_hash: "abc123".to_string(),
            modified_time: 0,
            size: 100,
        });

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
        for (path, meta) in &snapshot {
            assert_eq!(
                meta.content_hash.len(),
                64,
                "hash for {path} should be 64 hex chars"
            );
            assert!(
                meta.content_hash.chars().all(|c| c.is_ascii_hexdigit()),
                "hash for {path} should be hex, got {}",
                meta.content_hash
            );
        }
    }
}
