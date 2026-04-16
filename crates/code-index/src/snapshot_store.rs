//! File-backed persistence for index snapshots.
//!
//! Stores [`HashSnapshot`] as JSON files under `<data_dir>/code-index/<project_id>.json`.
//! Uses atomic writes (write to temp, rename over target) to prevent partial-read corruption.
//! Project IDs are sanitized against path traversal before use as filenames.

use std::collections::HashMap;
use std::path::PathBuf;

use tracing::debug;

use crate::delta::HashSnapshot;
use crate::error::{Error, Result};

/// Directory name under the data dir for snapshot files.
const SNAPSHOT_DIR: &str = "code-index";

/// Persistent file-backed store for index snapshots.
///
/// Each project gets its own JSON file: `<base_dir>/<sanitized_project_id>.json`.
/// The store is decoupled from SQLite — no migration coupling, no shared pool.
#[derive(Debug, Clone)]
pub struct SnapshotStore {
    base_dir: PathBuf,
}

impl SnapshotStore {
    /// Create a store rooted at the given directory.
    ///
    /// The directory is created on first write if it does not exist.
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// Create a store using the default data directory layout.
    ///
    /// Resolves to `<moltis_data_dir>/code-index/`.
    pub fn default_path() -> Self {
        let base_dir = moltis_config::data_dir().join(SNAPSHOT_DIR);
        Self::new(base_dir)
    }

    /// Load the most recent snapshot for a project.
    ///
    /// Returns `Ok(None)` if no snapshot file exists (first run or deleted).
    /// Returns `Err` only for I/O or parse failures on an existing file.
    pub fn load(&self, project_id: &str) -> Result<Option<HashSnapshot>> {
        let path = self.project_path(project_id)?;
        if !path.exists() {
            debug!(project_id, "no snapshot file found, starting fresh");
            return Ok(None);
        }
        let data = std::fs::read_to_string(&path).map_err(|e| {
            Error::Store(format!(
                "failed to read snapshot for project {project_id}: {e}"
            ))
        })?;
        let snapshot: HashMap<String, String> = serde_json::from_str(&data).map_err(|e| {
            Error::Store(format!(
                "failed to parse snapshot for project {project_id}: {e}"
            ))
        })?;
        Ok(Some(snapshot))
    }

    /// Save a snapshot for a project.
    ///
    /// Uses atomic write (temp file + rename) to prevent partial-write corruption.
    pub fn save(&self, project_id: &str, snapshot: &HashSnapshot) -> Result<()> {
        let path = self.project_path(project_id)?;

        // Ensure the base directory exists.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                Error::Store(format!(
                    "failed to create snapshot directory for project {project_id}: {e}"
                ))
            })?;
        }

        let json = serde_json::to_string_pretty(snapshot).map_err(|e| {
            Error::Store(format!(
                "failed to serialize snapshot for project {project_id}: {e}"
            ))
        })?;

        // Write to temp file, then rename atomically.
        let temp_path = path.with_extension("json.tmp");
        std::fs::write(&temp_path, &json).map_err(|e| {
            Error::Store(format!(
                "failed to write temp snapshot for project {project_id}: {e}"
            ))
        })?;

        std::fs::rename(&temp_path, &path).map_err(|e| {
            // Best-effort cleanup of temp file on rename failure.
            let _ = std::fs::remove_file(&temp_path);
            Error::Store(format!(
                "failed to commit snapshot for project {project_id}: {e}"
            ))
        })?;

        debug!(
            project_id,
            entries = snapshot.len(),
            "snapshot saved"
        );
        Ok(())
    }

    /// Delete the snapshot file for a project.
    ///
    /// Returns `Ok(())` even if no file existed.
    pub fn delete(&self, project_id: &str) -> Result<()> {
        let path = self.project_path(project_id)?;
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| {
                Error::Store(format!(
                    "failed to delete snapshot for project {project_id}: {e}"
                ))
            })?;
            debug!(project_id, "snapshot deleted");
        }
        Ok(())
    }

    /// Resolve a project ID to its snapshot file path.
    ///
    /// Sanitizes the project ID against path traversal before using it as a filename.
    fn project_path(&self, project_id: &str) -> Result<PathBuf> {
        let safe_id = sanitize_project_id(project_id)?;
        Ok(self.base_dir.join(format!("{safe_id}.json")))
    }
}

/// Sanitize a project ID for use as a filename.
///
/// Rejects IDs that contain path separators, traversal sequences, or null bytes.
/// Returns the sanitized ID on success.
fn sanitize_project_id(id: &str) -> Result<&str> {
    if id.is_empty() {
        return Err(Error::Store("project ID must not be empty".into()));
    }
    if id.contains('/')
        || id.contains('\\')
        || id.contains("..")
        || id.contains('\0')
    {
        return Err(Error::Store(format!(
            "project ID contains forbidden characters: {id:?}"
        )));
    }
    // Also reject if the ID, when used as a filename, would resolve outside the base dir.
    // The checks above already prevent traversal, but be defensive.
    if id == "." || id == ".." {
        return Err(Error::Store(format!(
            "project ID must not be a directory traversal: {id:?}"
        )));
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_rejects_traversal() {
        assert!(sanitize_project_id("../etc/passwd").is_err());
        assert!(sanitize_project_id("foo/../../bar").is_err());
        assert!(sanitize_project_id("foo\\..\\bar").is_err());
        assert!(sanitize_project_id("foo\0bar").is_err());
        assert!(sanitize_project_id("..").is_err());
        assert!(sanitize_project_id(".").is_err());
        assert!(sanitize_project_id("").is_err());
    }

    #[test]
    fn test_sanitize_accepts_valid_ids() {
        assert_eq!(sanitize_project_id("my-project").unwrap(), "my-project");
        assert_eq!(sanitize_project_id("project_123").unwrap(), "project_123");
        assert_eq!(sanitize_project_id("org.repo#main").unwrap(), "org.repo#main");
    }

    #[test]
    fn test_sanitize_rejects_slashes() {
        assert!(sanitize_project_id("org/repo").is_err());
        assert!(sanitize_project_id("a/b").is_err());
    }

    #[test]
    fn test_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SnapshotStore::new(tmp.path().to_path_buf());

        let mut snapshot = HashMap::new();
        snapshot.insert("src/main.rs".into(), "abc123".into());
        snapshot.insert("src/lib.rs".into(), "def456".into());

        store.save("test-project", &snapshot).unwrap();

        let loaded = store.load("test-project").unwrap().expect("should have snapshot");
        assert_eq!(loaded, snapshot);
    }

    #[test]
    fn test_load_missing_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SnapshotStore::new(tmp.path().to_path_buf());

        let result = store.load("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SnapshotStore::new(tmp.path().to_path_buf());

        let mut s1 = HashMap::new();
        s1.insert("a.rs".into(), "111".into());
        store.save("proj", &s1).unwrap();

        let mut s2 = HashMap::new();
        s2.insert("b.rs".into(), "222".into());
        store.save("proj", &s2).unwrap();

        let loaded = store.load("proj").unwrap().unwrap();
        assert_eq!(loaded, s2);
        assert!(!loaded.contains_key("a.rs"));
    }

    #[test]
    fn test_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SnapshotStore::new(tmp.path().to_path_buf());

        let snapshot = HashMap::from([("a.rs".into(), "111".into())]);
        store.save("proj", &snapshot).unwrap();
        assert!(store.load("proj").unwrap().is_some());

        store.delete("proj").unwrap();
        assert!(store.load("proj").unwrap().is_none());
    }

    #[test]
    fn test_delete_nonexistent_is_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SnapshotStore::new(tmp.path().to_path_buf());
        store.delete("nothing-here").unwrap();
    }

    #[test]
    fn test_project_path_uses_sanitized_id() {
        let store = SnapshotStore::new(PathBuf::from("/tmp/snapshots"));
        let path = store.project_path("my-project").unwrap();
        assert_eq!(path, PathBuf::from("/tmp/snapshots/my-project.json"));
    }

    #[test]
    fn test_traversal_project_id_rejected() {
        let store = SnapshotStore::new(PathBuf::from("/tmp/snapshots"));
        let result = store.project_path("../../etc/passwd");
        assert!(result.is_err());
    }
}