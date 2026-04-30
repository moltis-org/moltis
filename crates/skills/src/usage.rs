//! Per-skill usage telemetry.
//!
//! Tracks how often each skill is read (activated) and modified (created,
//! updated, patched). Data is persisted to `<data_dir>/skills-usage.json`
//! with atomic writes.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use {
    serde::{Deserialize, Serialize},
    tokio::sync::RwLock,
};

/// Per-skill usage counters and timestamps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillUsageEntry {
    /// Number of times the skill was activated via `read_skill`.
    pub read_count: u64,
    /// Number of times the skill was created or modified
    /// (create_skill + update_skill + patch_skill).
    pub write_count: u64,
    /// Unix milliseconds of the last `read_skill` call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_read_at: Option<u64>,
    /// Unix milliseconds of the last create/update/patch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_write_at: Option<u64>,
    /// Unix milliseconds when this skill first appeared in telemetry.
    pub created_at: u64,
}

/// Top-level usage file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct UsageFile {
    #[serde(default)]
    skills: HashMap<String, SkillUsageEntry>,
}

/// Thread-safe, file-backed skill usage store.
///
/// Clone-friendly via inner `Arc`.
#[derive(Clone)]
pub struct SkillUsageStore {
    inner: Arc<RwLock<UsageFile>>,
    path: PathBuf,
}

impl SkillUsageStore {
    /// Create a new store, loading existing data from disk if present.
    pub fn new(data_dir: &Path) -> Self {
        let path = data_dir.join("skills-usage.json");
        let file = if path.exists() {
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str::<UsageFile>(&s).ok())
                .unwrap_or_default()
        } else {
            UsageFile::default()
        };
        Self {
            inner: Arc::new(RwLock::new(file)),
            path,
        }
    }

    /// Record a read (activation) event for a skill.
    pub async fn record_read(&self, name: &str) {
        let now = now_millis();
        {
            let mut guard = self.inner.write().await;
            let entry = guard
                .skills
                .entry(name.to_string())
                .or_insert_with(|| SkillUsageEntry {
                    read_count: 0,
                    write_count: 0,
                    last_read_at: None,
                    last_write_at: None,
                    created_at: now,
                });
            entry.read_count += 1;
            entry.last_read_at = Some(now);
        }
        self.flush().await;
    }

    /// Record a write (create/update/patch) event for a skill.
    pub async fn record_write(&self, name: &str) {
        let now = now_millis();
        {
            let mut guard = self.inner.write().await;
            let entry = guard
                .skills
                .entry(name.to_string())
                .or_insert_with(|| SkillUsageEntry {
                    read_count: 0,
                    write_count: 0,
                    last_read_at: None,
                    last_write_at: None,
                    created_at: now,
                });
            entry.write_count += 1;
            entry.last_write_at = Some(now);
        }
        self.flush().await;
    }

    /// Remove a skill's usage entry (called on delete).
    pub async fn remove(&self, name: &str) {
        {
            let mut guard = self.inner.write().await;
            guard.skills.remove(name);
        }
        self.flush().await;
    }

    /// Return a snapshot of all usage entries.
    pub async fn get_all(&self) -> HashMap<String, SkillUsageEntry> {
        self.inner.read().await.skills.clone()
    }

    /// Persist to disk atomically (temp + rename).
    async fn flush(&self) {
        let snapshot = {
            let guard = self.inner.read().await;
            match serde_json::to_string_pretty(&*guard) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, "failed to serialize skill usage");
                    return;
                },
            }
        };
        let tmp = self.path.with_extension("json.tmp");
        if let Some(parent) = self.path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        if let Err(e) = tokio::fs::write(&tmp, &snapshot).await {
            tracing::warn!(error = %e, "failed to write skill usage temp file");
            return;
        }
        if let Err(e) = tokio::fs::rename(&tmp, &self.path).await {
            tracing::warn!(error = %e, "failed to rename skill usage file");
        }
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_record_read_increments() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SkillUsageStore::new(tmp.path());

        store.record_read("demo").await;
        store.record_read("demo").await;

        let all = store.get_all().await;
        let entry = all.get("demo").unwrap();
        assert_eq!(entry.read_count, 2);
        assert_eq!(entry.write_count, 0);
        assert!(entry.last_read_at.is_some());
        assert!(entry.last_write_at.is_none());
    }

    #[tokio::test]
    async fn test_record_write_increments() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SkillUsageStore::new(tmp.path());

        store.record_write("demo").await;

        let all = store.get_all().await;
        let entry = all.get("demo").unwrap();
        assert_eq!(entry.read_count, 0);
        assert_eq!(entry.write_count, 1);
        assert!(entry.last_write_at.is_some());
    }

    #[tokio::test]
    async fn test_remove_deletes_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SkillUsageStore::new(tmp.path());

        store.record_read("demo").await;
        assert!(store.get_all().await.contains_key("demo"));

        store.remove("demo").await;
        assert!(!store.get_all().await.contains_key("demo"));
    }

    #[tokio::test]
    async fn test_persistence_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();

        {
            let store = SkillUsageStore::new(tmp.path());
            store.record_read("alpha").await;
            store.record_write("alpha").await;
            store.record_read("beta").await;
        }

        // New store instance reads from disk.
        let store2 = SkillUsageStore::new(tmp.path());
        let all = store2.get_all().await;
        assert_eq!(all.get("alpha").unwrap().read_count, 1);
        assert_eq!(all.get("alpha").unwrap().write_count, 1);
        assert_eq!(all.get("beta").unwrap().read_count, 1);
    }

    #[tokio::test]
    async fn test_created_at_set_on_first_event() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SkillUsageStore::new(tmp.path());

        store.record_read("new-skill").await;
        let all = store.get_all().await;
        let entry = all.get("new-skill").unwrap();
        assert!(entry.created_at > 0);

        let original = entry.created_at;
        store.record_read("new-skill").await;
        let all = store.get_all().await;
        assert_eq!(
            all.get("new-skill").unwrap().created_at,
            original,
            "created_at must not change on subsequent events"
        );
    }

    #[tokio::test]
    async fn test_missing_file_creates_empty_store() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SkillUsageStore::new(tmp.path());
        assert!(store.get_all().await.is_empty());
    }
}
