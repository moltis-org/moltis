use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
};

use {
    crate::{Error, Result, storage_layout},
    fd_lock::RwLock,
    serde::{Deserialize, Serialize},
};

/// A single search hit within a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub session_key: String,
    pub snippet: String,
    pub role: String,
    pub message_index: usize,
}

/// Append-only JSONL session storage with file locking.
pub struct SessionStore {
    pub base_dir: PathBuf,
}

#[must_use]
fn slice_on_char_boundaries(content: &str, start: usize, end: usize) -> &str {
    let bounded_start = content.floor_char_boundary(start.min(content.len()));
    let bounded_end = content.floor_char_boundary(end.min(content.len()));
    if bounded_start >= bounded_end {
        return "";
    }
    &content[bounded_start..bounded_end]
}

impl SessionStore {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// Encode a session key as an injective, filesystem-safe component.
    pub fn key_to_filename(key: &str) -> String {
        storage_layout::encode_key(key)
    }

    /// Path where new history for a session key is stored.
    pub fn history_path_for(&self, key: &str) -> PathBuf {
        storage_layout::history_path(&self.base_dir, key)
    }

    fn path_for(&self, key: &str) -> PathBuf {
        self.history_path_for(key)
    }

    /// Directory for session media files (screenshots, audio, etc.).
    fn media_dir_for(&self, key: &str) -> PathBuf {
        storage_layout::media_dir(&self.base_dir, key)
    }

    /// Absolute path for a session media file.
    pub fn media_path_for(&self, key: &str, filename: &str) -> Result<PathBuf> {
        storage_layout::validate_media_filename(filename)?;
        Ok(storage_layout::resolved_media_path(
            &self.base_dir,
            key,
            filename,
        ))
    }

    /// Relative reference for a session media file.
    pub fn media_reference(key: &str, filename: &str) -> Result<String> {
        storage_layout::media_reference(key, filename)
    }

    async fn ensure_migrated(&self, key: &str) -> Result<()> {
        let base_dir = self.base_dir.clone();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || storage_layout::ensure_migrated(&base_dir, &key))
            .await?
    }

    /// Save a media file for a session. Returns the relative path from base_dir.
    pub async fn save_media(&self, key: &str, filename: &str, data: &[u8]) -> Result<String> {
        storage_layout::validate_media_filename(filename)?;
        self.ensure_migrated(key).await?;
        let dir = self.media_dir_for(key);
        let file_path = dir.join(filename);
        let data = data.to_vec();

        tokio::task::spawn_blocking(move || -> Result<()> {
            fs::create_dir_all(&dir)?;
            write_media_file(&file_path, &data)?;
            Ok(())
        })
        .await??;

        Self::media_reference(key, filename)
    }

    /// Read a media file. Returns raw bytes.
    pub async fn read_media(&self, key: &str, filename: &str) -> Result<Vec<u8>> {
        storage_layout::validate_media_filename(filename)?;
        self.ensure_migrated(key).await?;
        let file_path = self.media_dir_for(key).join(filename);

        tokio::task::spawn_blocking(move || -> Result<Vec<u8>> { read_media_file(&file_path) })
            .await?
    }

    /// Append a message (JSON value) as a single line to the session file.
    pub async fn append(&self, key: &str, message: &serde_json::Value) -> Result<()> {
        self.ensure_migrated(key).await?;
        let path = self.path_for(key);
        let line = serde_json::to_string(message)?;

        tokio::task::spawn_blocking(move || -> Result<()> {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let file = OpenOptions::new().create(true).append(true).open(&path)?;
            let mut lock = RwLock::new(file);
            let mut guard = lock
                .write()
                .map_err(|e| Error::lock_failed(e.to_string()))?;
            writeln!(*guard, "{line}")?;
            Ok(())
        })
        .await??;

        Ok(())
    }

    /// Read all messages from a session file.
    pub async fn read(&self, key: &str) -> Result<Vec<serde_json::Value>> {
        self.ensure_migrated(key).await?;
        let path = self.path_for(key);

        tokio::task::spawn_blocking(move || -> Result<Vec<serde_json::Value>> {
            if !path.exists() {
                return Ok(vec![]);
            }
            let file = File::open(&path)?;
            let reader = BufReader::new(file);
            let mut messages = Vec::new();
            for line in reader.lines() {
                let line = line?;
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str(trimmed) {
                    Ok(val) => messages.push(val),
                    Err(e) => {
                        tracing::warn!("skipping malformed JSONL line: {e}");
                    },
                }
            }
            Ok(messages)
        })
        .await?
    }

    /// Read a session without allocating beyond the caller's input bounds.
    /// Limits apply to JSONL file bytes and parsed messages, including entries
    /// that a higher-level consumer may later filter out.
    pub async fn read_bounded(
        &self,
        key: &str,
        max_messages: usize,
        max_bytes: usize,
    ) -> Result<Vec<serde_json::Value>> {
        self.ensure_migrated(key).await?;
        let path = self.path_for(key);

        tokio::task::spawn_blocking(move || -> Result<Vec<serde_json::Value>> {
            if !path.exists() {
                return Ok(vec![]);
            }
            let file = File::open(&path)?;
            let mut reader = BufReader::new(file);
            let mut messages = Vec::new();
            let mut line = String::new();
            let mut bytes_read = 0usize;
            loop {
                line.clear();
                let remaining = max_bytes.saturating_sub(bytes_read);
                let read = reader
                    .by_ref()
                    .take(remaining.saturating_add(1) as u64)
                    .read_line(&mut line)?;
                if read == 0 {
                    break;
                }
                bytes_read = bytes_read.saturating_add(read);
                if bytes_read > max_bytes {
                    return Err(Error::message(format!(
                        "session history exceeds {max_bytes} bytes"
                    )));
                }
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str(trimmed) {
                    Ok(value) => {
                        if messages.len() >= max_messages {
                            return Err(Error::message(format!(
                                "session history exceeds {max_messages} messages"
                            )));
                        }
                        messages.push(value);
                    },
                    Err(error) => {
                        tracing::warn!(%error, "skipping malformed JSONL line");
                    },
                }
            }
            Ok(messages)
        })
        .await?
    }

    /// Read all messages from a session that match a given `run_id`.
    pub async fn read_by_run_id(&self, key: &str, run_id: &str) -> Result<Vec<serde_json::Value>> {
        let all = self.read(key).await?;
        let run_id = run_id.to_string();
        Ok(all
            .into_iter()
            .filter(|msg| msg.get("run_id").and_then(|v| v.as_str()) == Some(&run_id))
            .collect())
    }

    /// Read the last N messages from a session file.
    pub async fn read_last_n(&self, key: &str, n: usize) -> Result<Vec<serde_json::Value>> {
        self.ensure_migrated(key).await?;
        let path = self.path_for(key);

        tokio::task::spawn_blocking(move || -> Result<Vec<serde_json::Value>> {
            if !path.exists() {
                return Ok(vec![]);
            }
            let file = File::open(&path)?;
            let reader = BufReader::new(file);
            let mut all: Vec<serde_json::Value> = Vec::new();
            for line in reader.lines() {
                let line = line?;
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(val) = serde_json::from_str(trimmed) {
                    all.push(val);
                }
            }
            let start = all.len().saturating_sub(n);
            Ok(all[start..].to_vec())
        })
        .await?
    }

    /// Delete the session file and its media directory.
    pub async fn clear(&self, key: &str) -> Result<()> {
        self.ensure_migrated(key).await?;
        let path = self.path_for(key);
        let media_dir = self.media_dir_for(key);
        let base_dir = self.base_dir.clone();
        let key = key.to_string();

        tokio::task::spawn_blocking(move || -> Result<()> {
            storage_layout::suppress_legacy_fallback(&base_dir, &key)?;
            if path.exists() {
                fs::remove_file(&path)?;
            }
            if media_dir.exists() {
                fs::remove_dir_all(&media_dir)?;
            }
            Ok(())
        })
        .await??;

        Ok(())
    }

    /// List all session keys by scanning JSONL files in the base directory.
    pub fn list_keys(&self) -> Vec<String> {
        stored_sessions(&self.base_dir)
            .into_iter()
            .map(|(key, _)| key)
            .collect()
    }

    /// Search all sessions for messages containing `query` (case-insensitive).
    /// Returns up to `max_results` hits, at most one per session.
    pub async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>> {
        self.search_known_keys(query, max_results, &[]).await
    }

    /// Search sessions after migrating the supplied authoritative keys.
    pub async fn search_known_keys(
        &self,
        query: &str,
        max_results: usize,
        known_keys: &[String],
    ) -> Result<Vec<SearchResult>> {
        let base = self.base_dir.clone();
        let query = query.to_lowercase();
        let known_keys = known_keys.to_vec();

        tokio::task::spawn_blocking(move || {
            for key in known_keys {
                if let Err(error) = storage_layout::ensure_migrated(&base, &key) {
                    tracing::warn!(session_key = %key, %error, "failed to migrate session for search");
                }
            }
            let mut results = Vec::new();
            for (session_key, path) in stored_sessions(&base) {
                if results.len() >= max_results {
                    break;
                }

                let Ok(file) = File::open(&path) else {
                    continue;
                };
                let reader = BufReader::new(file);
                for (idx, line) in reader.lines().enumerate() {
                    let Ok(line) = line else {
                        continue;
                    };
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) else {
                        continue;
                    };
                    let content = val.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    if content.to_lowercase().contains(&query) {
                        let role = val
                            .get("role")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();

                        // Build a snippet: find the match position and extract context.
                        let lower = content.to_lowercase();
                        let pos = lower.find(&query).unwrap_or(0);
                        let start = pos.saturating_sub(40);
                        let end = pos.saturating_add(query.len()).saturating_add(60);
                        let snippet = slice_on_char_boundaries(content, start, end).to_string();

                        results.push(SearchResult {
                            session_key: session_key.clone(),
                            snippet,
                            role,
                            message_index: idx,
                        });
                        // One hit per session is enough for autocomplete.
                        break;
                    }
                }
            }

            Ok(results)
        })
        .await?
    }

    /// Replace the entire session history with the given messages.
    pub async fn replace_history(&self, key: &str, messages: Vec<serde_json::Value>) -> Result<()> {
        self.ensure_migrated(key).await?;
        let path = self.path_for(key);

        tokio::task::spawn_blocking(move || -> Result<()> {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&path)?;
            let mut lock = RwLock::new(file);
            let mut guard = lock
                .write()
                .map_err(|e| Error::lock_failed(e.to_string()))?;
            for msg in &messages {
                let line = serde_json::to_string(msg)?;
                writeln!(*guard, "{line}")?;
            }
            Ok(())
        })
        .await??;

        Ok(())
    }

    /// Read all messages as typed [`PersistedMessage`] values.
    ///
    /// Lines that fail to deserialize into `PersistedMessage` are skipped
    /// (with a warning), matching the behavior of [`read`].
    pub async fn read_typed(&self, key: &str) -> Result<Vec<crate::message::PersistedMessage>> {
        self.ensure_migrated(key).await?;
        let path = self.path_for(key);

        tokio::task::spawn_blocking(move || -> Result<Vec<crate::message::PersistedMessage>> {
            if !path.exists() {
                return Ok(vec![]);
            }
            let file = File::open(&path)?;
            let reader = BufReader::new(file);
            let mut messages = Vec::new();
            for line in reader.lines() {
                let line = line?;
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str(trimmed) {
                    Ok(msg) => messages.push(msg),
                    Err(e) => {
                        tracing::warn!("skipping malformed JSONL line (typed): {e}");
                    },
                }
            }
            Ok(messages)
        })
        .await?
    }

    /// Read the last N messages as typed [`PersistedMessage`] values.
    pub async fn read_last_n_typed(
        &self,
        key: &str,
        n: usize,
    ) -> Result<Vec<crate::message::PersistedMessage>> {
        self.ensure_migrated(key).await?;
        let path = self.path_for(key);

        tokio::task::spawn_blocking(move || -> Result<Vec<crate::message::PersistedMessage>> {
            if !path.exists() {
                return Ok(vec![]);
            }
            let file = File::open(&path)?;
            let reader = BufReader::new(file);
            let mut all: Vec<crate::message::PersistedMessage> = Vec::new();
            for line in reader.lines() {
                let line = line?;
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(msg) = serde_json::from_str(trimmed) {
                    all.push(msg);
                }
            }
            let start = all.len().saturating_sub(n);
            Ok(all[start..].to_vec())
        })
        .await?
    }

    /// Replace the entire session history with typed messages.
    pub async fn replace_history_typed(
        &self,
        key: &str,
        messages: &[crate::message::PersistedMessage],
    ) -> Result<()> {
        self.ensure_migrated(key).await?;
        let path = self.path_for(key);
        let values: Vec<serde_json::Value> = messages.iter().map(|m| m.to_value()).collect();

        tokio::task::spawn_blocking(move || -> Result<()> {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&path)?;
            let mut lock = RwLock::new(file);
            let mut guard = lock
                .write()
                .map_err(|e| Error::lock_failed(e.to_string()))?;
            for msg in &values {
                let line = serde_json::to_string(msg)?;
                writeln!(*guard, "{line}")?;
            }
            Ok(())
        })
        .await??;

        Ok(())
    }

    /// Append a typed message to the session file.
    pub async fn append_typed(
        &self,
        key: &str,
        message: &crate::message::PersistedMessage,
    ) -> Result<()> {
        self.append(key, &message.to_value()).await
    }

    /// Count messages in a session file without parsing them.
    pub async fn count(&self, key: &str) -> Result<u32> {
        self.ensure_migrated(key).await?;
        let path = self.path_for(key);

        tokio::task::spawn_blocking(move || -> Result<u32> {
            if !path.exists() {
                return Ok(0);
            }
            let file = File::open(&path)?;
            let reader = BufReader::new(file);
            let count = reader
                .lines()
                .map_while(std::result::Result::ok)
                .filter(|l| !l.trim().is_empty())
                .count();
            Ok(count as u32)
        })
        .await?
    }
}

fn stored_sessions(base_dir: &std::path::Path) -> Vec<(String, PathBuf)> {
    let mut sessions = Vec::new();
    let version_dir = base_dir.join(storage_layout::STORAGE_VERSION);
    collect_stored_sessions(&version_dir, &version_dir, &mut sessions);

    sessions.sort_by(|left, right| left.0.cmp(&right.0));
    sessions
}

fn collect_stored_sessions(
    version_dir: &std::path::Path,
    directory: &std::path::Path,
    sessions: &mut Vec<(String, PathBuf)>,
) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            if entry.file_name() != ".migrated" {
                collect_stored_sessions(version_dir, &path, sessions);
            }
            continue;
        }
        if !file_type.is_file() || entry.file_name() != "history.jsonl" {
            continue;
        }
        let Some(key_dir) = path.parent() else {
            continue;
        };
        let Ok(encoded_path) = key_dir.strip_prefix(version_dir) else {
            continue;
        };
        let Some(key) = storage_layout::decode_key_path(encoded_path) else {
            continue;
        };
        sessions.push((key, path));
    }
}

fn write_media_file(path: &std::path::Path, data: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    file.write_all(data)?;
    Ok(())
}

fn read_media_file(path: &std::path::Path) -> Result<Vec<u8>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;
    Ok(data)
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, serde_json::json};

    fn temp_store() -> (SessionStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf());
        (store, dir)
    }

    #[test]
    fn slice_on_char_boundaries_handles_multibyte_boundary() {
        let content = format!("{}л{}", "a".repeat(39), "z".repeat(20));
        let snippet = slice_on_char_boundaries(&content, 0, 40);
        assert_eq!(snippet.len(), 39);
        assert!(snippet.chars().all(|c| c == 'a'));
    }

    #[tokio::test]
    async fn test_append_and_read() {
        let (store, _dir) = temp_store();

        store
            .append("main", &json!({"role": "user", "content": "hello"}))
            .await
            .unwrap();
        store
            .append("main", &json!({"role": "assistant", "content": "hi"}))
            .await
            .unwrap();

        let msgs = store.read("main").await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[1]["role"], "assistant");
    }

    #[tokio::test]
    async fn test_read_empty() {
        let (store, _dir) = temp_store();
        let msgs = store.read("nonexistent").await.unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn test_read_last_n() {
        let (store, _dir) = temp_store();

        for i in 0..10 {
            store.append("test", &json!({"i": i})).await.unwrap();
        }

        let last3 = store.read_last_n("test", 3).await.unwrap();
        assert_eq!(last3.len(), 3);
        assert_eq!(last3[0]["i"], 7);
        assert_eq!(last3[2]["i"], 9);
    }

    #[tokio::test]
    async fn test_clear() {
        let (store, _dir) = temp_store();

        store
            .append("main", &json!({"role": "user", "content": "hello"}))
            .await
            .unwrap();
        assert_eq!(store.read("main").await.unwrap().len(), 1);

        store.clear("main").await.unwrap();
        assert!(store.read("main").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_count() {
        let (store, _dir) = temp_store();

        assert_eq!(store.count("main").await.unwrap(), 0);
        store
            .append("main", &json!({"role": "user"}))
            .await
            .unwrap();
        store
            .append("main", &json!({"role": "assistant"}))
            .await
            .unwrap();
        assert_eq!(store.count("main").await.unwrap(), 2);
    }

    #[tokio::test]
    async fn test_search_matching() {
        let (store, _dir) = temp_store();

        store
            .append("s1", &json!({"role": "user", "content": "hello world"}))
            .await
            .unwrap();
        store
            .append("s1", &json!({"role": "assistant", "content": "hi there"}))
            .await
            .unwrap();
        store
            .append("s2", &json!({"role": "user", "content": "goodbye world"}))
            .await
            .unwrap();

        let results = store.search("hello", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session_key, "s1");
        assert_eq!(results[0].role, "user");
        assert!(results[0].snippet.contains("hello"));
    }

    #[tokio::test]
    async fn test_search_case_insensitive() {
        let (store, _dir) = temp_store();

        store
            .append("s1", &json!({"role": "user", "content": "Hello World"}))
            .await
            .unwrap();

        let results = store.search("hello world", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session_key, "s1");
    }

    #[tokio::test]
    async fn test_search_no_match() {
        let (store, _dir) = temp_store();

        store
            .append("s1", &json!({"role": "user", "content": "hello"}))
            .await
            .unwrap();

        let results = store.search("xyz", 10).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_search_empty_query() {
        let (store, _dir) = temp_store();

        store
            .append("s1", &json!({"role": "user", "content": "hello"}))
            .await
            .unwrap();

        // Empty query should match nothing (caller should guard against this)
        let results = store.search("", 10).await.unwrap();
        // Empty string is contained in every string, so it would match.
        // The frontend guards against empty queries, but the store doesn't — that's fine.
        assert!(!results.is_empty());
    }

    #[tokio::test]
    async fn test_search_across_sessions() {
        let (store, _dir) = temp_store();

        store
            .append("s1", &json!({"role": "user", "content": "rust is great"}))
            .await
            .unwrap();
        store
            .append(
                "s2",
                &json!({"role": "assistant", "content": "rust is awesome"}),
            )
            .await
            .unwrap();
        store
            .append("s3", &json!({"role": "user", "content": "python is nice"}))
            .await
            .unwrap();

        let results = store.search("rust", 10).await.unwrap();
        assert_eq!(results.len(), 2);
        let keys: Vec<&str> = results.iter().map(|r| r.session_key.as_str()).collect();
        assert!(keys.contains(&"s1"));
        assert!(keys.contains(&"s2"));
    }

    #[tokio::test]
    async fn test_search_max_results() {
        let (store, _dir) = temp_store();

        for i in 0..10 {
            let key = format!("s{i}");
            store
                .append(&key, &json!({"role": "user", "content": "common term"}))
                .await
                .unwrap();
        }

        let results = store.search("common", 3).await.unwrap();
        assert!(results.len() <= 3);
    }

    #[tokio::test]
    async fn test_replace_history() {
        let (store, _dir) = temp_store();

        store
            .append("main", &json!({"role": "user", "content": "hello"}))
            .await
            .unwrap();
        store
            .append("main", &json!({"role": "assistant", "content": "hi"}))
            .await
            .unwrap();
        assert_eq!(store.read("main").await.unwrap().len(), 2);

        let new_history = vec![json!({"role": "assistant", "content": "summary"})];
        store.replace_history("main", new_history).await.unwrap();

        let msgs = store.read("main").await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["content"], "summary");
    }

    #[tokio::test]
    async fn test_replace_history_empty() {
        let (store, _dir) = temp_store();

        store
            .append("main", &json!({"role": "user", "content": "hello"}))
            .await
            .unwrap();

        store.replace_history("main", vec![]).await.unwrap();
        assert!(store.read("main").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_key_sanitization() {
        let (store, dir) = temp_store();

        store
            .append("session:abc-123", &json!({"role": "user"}))
            .await
            .unwrap();
        let msgs = store.read("session:abc-123").await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(
            store.history_path_for("session:abc-123"),
            dir.path()
                .join("v1")
                .join(SessionStore::key_to_filename("session:abc-123"))
                .join("history.jsonl")
        );
    }

    #[tokio::test]
    async fn test_save_and_read_media() {
        let (store, _dir) = temp_store();
        let data = b"fake png data";

        let path = store.save_media("main", "call_1.png", data).await.unwrap();
        assert_eq!(
            path,
            SessionStore::media_reference("main", "call_1.png").unwrap()
        );

        let read_back = store.read_media("main", "call_1.png").await.unwrap();
        assert_eq!(read_back, data);
    }

    #[tokio::test]
    async fn test_save_media_with_colon_key() {
        let (store, _dir) = temp_store();
        let data = b"screenshot bytes";

        let path = store
            .save_media("session:abc", "shot.png", data)
            .await
            .unwrap();
        assert_eq!(
            path,
            SessionStore::media_reference("session:abc", "shot.png").unwrap()
        );

        let read_back = store.read_media("session:abc", "shot.png").await.unwrap();
        assert_eq!(read_back, data);
    }

    #[test]
    fn test_media_path_for_uses_session_media_dir() {
        let (store, dir) = temp_store();
        let path = store.media_path_for("session:abc", "report.pdf").unwrap();
        assert_eq!(
            path,
            dir.path()
                .join("media")
                .join("v1")
                .join(SessionStore::key_to_filename("session:abc"))
                .join("files")
                .join("report.pdf")
        );
    }

    #[tokio::test]
    async fn test_read_media_missing_file() {
        let (store, _dir) = temp_store();
        let result = store.read_media("main", "nonexistent.png").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn media_filenames_cannot_escape_the_session_directory() {
        let (store, _dir) = temp_store();
        for filename in ["", ".", "..", "../escape", "nested/file", "nested\\file"] {
            assert!(store.save_media("main", filename, b"data").await.is_err());
            assert!(store.read_media("main", filename).await.is_err());
            assert!(store.media_path_for("main", filename).is_err());
            assert!(SessionStore::media_reference("main", filename).is_err());
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn media_io_does_not_follow_symlinks() {
        use std::os::unix::fs::symlink;

        let (store, dir) = temp_store();
        let outside = dir.path().join("outside.bin");
        fs::write(&outside, b"outside").unwrap();
        let media_path = store.media_path_for("main", "linked.bin").unwrap();
        fs::create_dir_all(media_path.parent().unwrap()).unwrap();
        symlink(&outside, &media_path).unwrap();

        assert!(
            store
                .save_media("main", "linked.bin", b"changed")
                .await
                .is_err()
        );
        assert!(store.read_media("main", "linked.bin").await.is_err());
        assert_eq!(fs::read(outside).unwrap(), b"outside");
    }

    #[tokio::test]
    async fn test_clear_removes_media_dir() {
        let (store, dir) = temp_store();

        // Create a session and media.
        store
            .append("main", &json!({"role": "user", "content": "hello"}))
            .await
            .unwrap();
        store
            .save_media("main", "shot.png", b"img data")
            .await
            .unwrap();

        let media_dir = dir
            .path()
            .join("media")
            .join("v1")
            .join(SessionStore::key_to_filename("main"))
            .join("files");
        assert!(media_dir.exists());

        store.clear("main").await.unwrap();

        assert!(!media_dir.exists());
        assert!(store.read("main").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn formerly_colliding_keys_have_isolated_history_media_and_search() {
        let (store, _dir) = temp_store();
        store
            .append("a:b", &json!({"role": "user", "content": "colon value"}))
            .await
            .unwrap();
        store
            .append(
                "a_b",
                &json!({"role": "user", "content": "underscore value"}),
            )
            .await
            .unwrap();
        store.save_media("a:b", "same.bin", b"colon").await.unwrap();
        store
            .save_media("a_b", "same.bin", b"underscore")
            .await
            .unwrap();

        assert_eq!(
            store.read("a:b").await.unwrap()[0]["content"],
            "colon value"
        );
        assert_eq!(
            store.read("a_b").await.unwrap()[0]["content"],
            "underscore value"
        );
        assert_eq!(store.read_media("a:b", "same.bin").await.unwrap(), b"colon");
        assert_eq!(
            store.read_media("a_b", "same.bin").await.unwrap(),
            b"underscore"
        );

        let colon_results = store.search("colon value", 10).await.unwrap();
        let underscore_results = store.search("underscore value", 10).await.unwrap();
        assert_eq!(colon_results[0].session_key, "a:b");
        assert_eq!(underscore_results[0].session_key, "a_b");
        assert_eq!(store.list_keys(), vec!["a:b", "a_b"]);
    }

    #[tokio::test]
    async fn chunk_prefix_keys_have_isolated_media_and_clear() {
        let (store, _dir) = temp_store();
        let shorter = "a".repeat(60);
        let longer = "a".repeat(61);
        store
            .save_media(&shorter, "same.bin", b"short")
            .await
            .unwrap();
        store
            .save_media(&longer, "same.bin", b"long")
            .await
            .unwrap();
        store
            .append(&shorter, &json!({"content": "short"}))
            .await
            .unwrap();
        store
            .append(&longer, &json!({"content": "long"}))
            .await
            .unwrap();

        store.clear(&shorter).await.unwrap();

        assert_eq!(store.read(&longer).await.unwrap()[0]["content"], "long");
        assert_eq!(
            store.read_media(&longer, "same.bin").await.unwrap(),
            b"long"
        );
        assert!(store.read(&shorter).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn legacy_collision_is_copied_per_key_and_clear_does_not_resurrect_it() {
        let (store, dir) = temp_store();
        fs::write(
            dir.path().join("a_b.jsonl"),
            "{\"role\":\"user\",\"content\":\"legacy\"}\n",
        )
        .unwrap();
        let legacy_media = dir.path().join("media").join("a_b");
        fs::create_dir_all(&legacy_media).unwrap();
        fs::write(legacy_media.join("old.bin"), b"legacy media").unwrap();

        assert_eq!(store.read("a:b").await.unwrap()[0]["content"], "legacy");
        assert_eq!(store.read("a_b").await.unwrap()[0]["content"], "legacy");
        assert_eq!(
            store.read_media("a:b", "old.bin").await.unwrap(),
            b"legacy media"
        );
        assert_eq!(
            store.read_media("a_b", "old.bin").await.unwrap(),
            b"legacy media"
        );

        store
            .append("a:b", &json!({"role": "assistant", "content": "colon"}))
            .await
            .unwrap();
        store
            .append(
                "a_b",
                &json!({"role": "assistant", "content": "underscore"}),
            )
            .await
            .unwrap();
        assert_eq!(store.read("a:b").await.unwrap()[1]["content"], "colon");
        assert_eq!(store.read("a_b").await.unwrap()[1]["content"], "underscore");

        store.clear("a:b").await.unwrap();
        assert!(store.read("a:b").await.unwrap().is_empty());
        assert_eq!(store.read("a_b").await.unwrap().len(), 2);
        assert_eq!(
            store.read_media("a_b", "old.bin").await.unwrap(),
            b"legacy media"
        );
        assert!(dir.path().join("a_b.jsonl").exists());
    }

    #[tokio::test]
    async fn authoritative_search_migrates_legacy_underscore_keys_without_guessing() {
        let (store, dir) = temp_store();
        fs::write(
            dir.path().join("agent_private.jsonl"),
            "{\"role\":\"user\",\"content\":\"private history\"}\n",
        )
        .unwrap();

        assert!(
            store
                .search("private history", 10)
                .await
                .unwrap()
                .is_empty()
        );
        let results = store
            .search_known_keys("private history", 10, &["agent_private".to_string()])
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session_key, "agent_private");
    }

    #[tokio::test]
    async fn migrated_sessions_reconcile_later_legacy_appends_and_media() {
        let (store, dir) = temp_store();
        let legacy_history = dir.path().join("old_key.jsonl");
        fs::write(
            &legacy_history,
            "{\"role\":\"user\",\"content\":\"first\"}\n",
        )
        .unwrap();
        let legacy_media = dir.path().join("media").join("old_key");
        fs::create_dir_all(&legacy_media).unwrap();

        assert_eq!(store.read("old:key").await.unwrap().len(), 1);
        OpenOptions::new()
            .append(true)
            .open(&legacy_history)
            .unwrap()
            .write_all(b"{\"role\":\"user\",\"content\":\"second\"}\n")
            .unwrap();
        fs::write(legacy_media.join("late.bin"), b"late media").unwrap();

        let messages = store.read("old:key").await.unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1]["content"], "second");
        assert_eq!(
            store.read_media("old:key", "late.bin").await.unwrap(),
            b"late media"
        );
    }

    #[tokio::test]
    async fn late_created_conflicting_legacy_history_is_not_marked_as_copied() {
        let (store, dir) = temp_store();
        store
            .append("late:key", &json!({"content": "current"}))
            .await
            .unwrap();
        fs::write(
            dir.path().join("late_key.jsonl"),
            "{\"content\":\"legacy\"}\n",
        )
        .unwrap();

        let error = store.read("late:key").await.unwrap_err();
        assert!(error.to_string().contains("histories conflict"));
        assert!(
            storage_layout::migration_marker(dir.path(), "late:key")
                .read_dir()
                .is_err()
        );
    }

    #[tokio::test]
    async fn traversal_like_key_stays_inside_versioned_storage() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().join("sessions");
        let store = SessionStore::new(base.clone());
        store
            .append("../escape", &json!({"role": "user"}))
            .await
            .unwrap();

        assert!(
            store
                .history_path_for("../escape")
                .starts_with(base.join("v1"))
        );
        assert!(!root.path().join("escape.jsonl").exists());
        assert_eq!(store.read("../escape").await.unwrap().len(), 1);
    }

    // --- Typed API tests ---

    #[tokio::test]
    async fn test_append_typed_and_read_typed() {
        use crate::message::PersistedMessage;

        let (store, _dir) = temp_store();

        store
            .append_typed("main", &PersistedMessage::user("hello"))
            .await
            .unwrap();
        store
            .append_typed(
                "main",
                &PersistedMessage::assistant("hi", "gpt-4o", "openai", 10, 5, None),
            )
            .await
            .unwrap();

        let msgs = store.read_typed("main").await.unwrap();
        assert_eq!(msgs.len(), 2);
        match &msgs[0] {
            PersistedMessage::User { content, .. } => {
                assert!(matches!(content, crate::message::MessageContent::Text(t) if t == "hello"));
            },
            _ => panic!("expected User message"),
        }
        match &msgs[1] {
            PersistedMessage::Assistant { content, model, .. } => {
                assert_eq!(content, "hi");
                assert_eq!(model.as_deref(), Some("gpt-4o"));
            },
            _ => panic!("expected Assistant message"),
        }
    }

    #[tokio::test]
    async fn read_bounded_rejects_message_and_byte_overflow() {
        let (store, _dir) = temp_store();
        store
            .append("main", &json!({"content": "one"}))
            .await
            .unwrap();
        store
            .append("main", &json!({"content": "two"}))
            .await
            .unwrap();

        let message_error = store.read_bounded("main", 1, 1024).await.unwrap_err();
        assert!(message_error.to_string().contains("exceeds 1 messages"));

        let byte_error = store.read_bounded("main", 10, 8).await.unwrap_err();
        assert!(byte_error.to_string().contains("exceeds 8 bytes"));
    }

    #[tokio::test]
    async fn test_read_typed_empty() {
        let (store, _dir) = temp_store();
        let msgs = store.read_typed("nonexistent").await.unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn test_read_last_n_typed() {
        use crate::message::PersistedMessage;

        let (store, _dir) = temp_store();

        for i in 0..5 {
            store
                .append_typed("test", &PersistedMessage::user(format!("msg-{i}")))
                .await
                .unwrap();
        }

        let last2 = store.read_last_n_typed("test", 2).await.unwrap();
        assert_eq!(last2.len(), 2);
        match &last2[0] {
            PersistedMessage::User { content, .. } => {
                assert!(matches!(content, crate::message::MessageContent::Text(t) if t == "msg-3"));
            },
            _ => panic!("expected User message"),
        }
        match &last2[1] {
            PersistedMessage::User { content, .. } => {
                assert!(matches!(content, crate::message::MessageContent::Text(t) if t == "msg-4"));
            },
            _ => panic!("expected User message"),
        }
    }

    #[tokio::test]
    async fn test_replace_history_typed() {
        use crate::message::PersistedMessage;

        let (store, _dir) = temp_store();

        store
            .append_typed("main", &PersistedMessage::user("old"))
            .await
            .unwrap();
        assert_eq!(store.count("main").await.unwrap(), 1);

        let new_history = vec![
            PersistedMessage::user("new1"),
            PersistedMessage::assistant("new2", "gpt-4o", "openai", 10, 5, None),
        ];
        store
            .replace_history_typed("main", &new_history)
            .await
            .unwrap();

        let msgs = store.read_typed("main").await.unwrap();
        assert_eq!(msgs.len(), 2);
        match &msgs[0] {
            PersistedMessage::User { content, .. } => {
                assert!(matches!(content, crate::message::MessageContent::Text(t) if t == "new1"));
            },
            _ => panic!("expected User message"),
        }
    }

    #[tokio::test]
    async fn test_typed_roundtrip_with_value_api() {
        use crate::message::PersistedMessage;

        let (store, _dir) = temp_store();

        // Write with typed API, read with Value API.
        store
            .append_typed("main", &PersistedMessage::user("typed write"))
            .await
            .unwrap();
        let values = store.read("main").await.unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["role"], "user");
        assert_eq!(values[0]["content"], "typed write");

        // Write with Value API, read with typed API.
        store
            .append(
                "main",
                &json!({"role": "assistant", "content": "value write"}),
            )
            .await
            .unwrap();
        let typed = store.read_typed("main").await.unwrap();
        assert_eq!(typed.len(), 2);
        match &typed[1] {
            PersistedMessage::Assistant { content, .. } => {
                assert_eq!(content, "value write");
            },
            _ => panic!("expected Assistant message"),
        }
    }
}
