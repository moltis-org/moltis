//! Per-skill usage telemetry.
//!
//! Tracks how often each skill is read (activated) and modified (created,
//! updated, patched). Data is persisted to `<data_dir>/skills-usage.json`
//! with atomic writes, debounced to avoid excessive I/O.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use {
    serde::{Deserialize, Serialize},
    tokio::sync::{Notify, RwLock},
};

/// Minimum interval between disk flushes (seconds).
const FLUSH_DEBOUNCE_SECS: u64 = 5;

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

/// Shared interior state.
struct Inner {
    data: UsageFile,
    dirty: bool,
    last_flush_at: u64,
}

/// Thread-safe, file-backed skill usage store.
///
/// Flushes are debounced: mutations mark the store dirty but only write to
/// disk when at least [`FLUSH_DEBOUNCE_SECS`] have elapsed since the last
/// flush. A background notify ensures eventual persistence.
///
/// Clone-friendly via inner `Arc`.
#[derive(Clone)]
pub struct SkillUsageStore {
    inner: Arc<RwLock<Inner>>,
    path: PathBuf,
    flush_notify: Arc<Notify>,
}

impl SkillUsageStore {
    /// Create a new store with synchronous file I/O. Requires an active Tokio
    /// runtime (spawns a background flush task). Prefer [`open`](Self::open)
    /// in async contexts.
    pub fn new(data_dir: &Path) -> Self {
        let path = data_dir.join("skills-usage.json");
        let data = if path.exists() {
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str::<UsageFile>(&s).ok())
                .unwrap_or_default()
        } else {
            UsageFile::default()
        };
        Self::from_data(data, path)
    }

    /// Create a new store with async I/O (preferred in async contexts).
    pub async fn open(data_dir: &Path) -> Self {
        let path = data_dir.join("skills-usage.json");
        let data = match tokio::fs::read_to_string(&path).await {
            Ok(s) => serde_json::from_str::<UsageFile>(&s).unwrap_or_default(),
            Err(_) => UsageFile::default(),
        };
        Self::from_data(data, path)
    }

    fn from_data(data: UsageFile, path: PathBuf) -> Self {
        let store = Self {
            inner: Arc::new(RwLock::new(Inner {
                data,
                dirty: false,
                last_flush_at: 0,
            })),
            path,
            flush_notify: Arc::new(Notify::new()),
        };
        store.spawn_flush_task();
        store
    }

    /// Spawn a background task that flushes dirty data on notification.
    fn spawn_flush_task(&self) {
        let inner = Arc::clone(&self.inner);
        let path = self.path.clone();
        let notify = Arc::clone(&self.flush_notify);
        tokio::spawn(async move {
            loop {
                notify.notified().await;
                // Coalesce rapid mutations by sleeping briefly.
                tokio::time::sleep(tokio::time::Duration::from_secs(FLUSH_DEBOUNCE_SECS)).await;
                flush_to_disk(&inner, &path).await;
            }
        });
    }

    /// Record a read (activation) event for a skill.
    pub async fn record_read(&self, name: &str) {
        let now = now_millis();
        {
            let mut guard = self.inner.write().await;
            let entry = guard
                .data
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
            guard.dirty = true;
        }
        self.maybe_flush().await;
    }

    /// Record a write (create/update/patch) event for a skill.
    pub async fn record_write(&self, name: &str) {
        let now = now_millis();
        {
            let mut guard = self.inner.write().await;
            let entry = guard
                .data
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
            guard.dirty = true;
        }
        // Writes are less frequent — flush immediately.
        flush_to_disk(&self.inner, &self.path).await;
    }

    /// Remove a skill's usage entry (called on delete).
    pub async fn remove(&self, name: &str) {
        {
            let mut guard = self.inner.write().await;
            guard.data.skills.remove(name);
            guard.dirty = true;
        }
        flush_to_disk(&self.inner, &self.path).await;
    }

    /// Return a snapshot of all usage entries.
    pub async fn get_all(&self) -> HashMap<String, SkillUsageEntry> {
        self.inner.read().await.data.skills.clone()
    }

    /// Force-flush any pending dirty data to disk. Call during graceful
    /// shutdown to ensure debounced read events are not lost.
    pub async fn shutdown_flush(&self) {
        flush_to_disk(&self.inner, &self.path).await;
    }

    /// Flush immediately if enough time has elapsed, otherwise notify the
    /// background task to handle it after the debounce interval.
    async fn maybe_flush(&self) {
        let now = now_secs();
        let (is_dirty, elapsed) = {
            let guard = self.inner.read().await;
            (guard.dirty, now.saturating_sub(guard.last_flush_at))
        };
        if !is_dirty {
            return;
        }
        if elapsed >= FLUSH_DEBOUNCE_SECS {
            flush_to_disk(&self.inner, &self.path).await;
        } else {
            self.flush_notify.notify_one();
        }
    }
}

/// Persist to disk atomically (temp + rename).
///
/// Serializes data and clears the dirty flag under a single write lock
/// to avoid a TOCTOU race where a concurrent mutation between snapshot
/// and flag-clear would be silently lost. On I/O failure, re-sets the
/// dirty flag so the background task will retry.
async fn flush_to_disk(inner: &RwLock<Inner>, path: &Path) {
    let snapshot = {
        let mut guard = inner.write().await;
        if !guard.dirty {
            return;
        }
        let s = match serde_json::to_string_pretty(&guard.data) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "failed to serialize skill usage");
                return;
            },
        };
        guard.dirty = false;
        guard.last_flush_at = now_secs();
        s
    };
    // I/O proceeds outside the lock. On failure, re-mark dirty for retry.
    let tmp = path.with_extension("json.tmp");
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if let Err(e) = tokio::fs::write(&tmp, &snapshot).await {
        tracing::warn!(error = %e, "failed to write skill usage temp file");
        inner.write().await.dirty = true;
        return;
    }
    if let Err(e) = tokio::fs::rename(&tmp, path).await {
        tracing::warn!(error = %e, "failed to rename skill usage file");
        inner.write().await.dirty = true;
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_record_read_increments() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SkillUsageStore::open(tmp.path()).await;

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
        let store = SkillUsageStore::open(tmp.path()).await;

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
        let store = SkillUsageStore::open(tmp.path()).await;

        store.record_read("demo").await;
        assert!(store.get_all().await.contains_key("demo"));

        store.remove("demo").await;
        assert!(!store.get_all().await.contains_key("demo"));
    }

    #[tokio::test]
    async fn test_persistence_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();

        {
            let store = SkillUsageStore::open(tmp.path()).await;
            // record_write flushes immediately, so data is on disk.
            store.record_write("alpha").await;
            store.record_read("alpha").await;
            // Force flush for the read (which may be debounced).
            flush_to_disk(&store.inner, &store.path).await;
            store.record_write("beta").await;
        }

        // New store instance reads from disk.
        let store2 = SkillUsageStore::open(tmp.path()).await;
        let all = store2.get_all().await;
        assert_eq!(all.get("alpha").unwrap().read_count, 1);
        assert_eq!(all.get("alpha").unwrap().write_count, 1);
        assert_eq!(all.get("beta").unwrap().write_count, 1);
    }

    #[tokio::test]
    async fn test_created_at_set_on_first_event() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SkillUsageStore::open(tmp.path()).await;

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
        let store = SkillUsageStore::open(tmp.path()).await;
        assert!(store.get_all().await.is_empty());
    }

    #[tokio::test]
    async fn test_sync_constructor_works() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SkillUsageStore::new(tmp.path());
        store.record_write("test").await;
        assert_eq!(store.get_all().await.get("test").unwrap().write_count, 1);
    }
}
