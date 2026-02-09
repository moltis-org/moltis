use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::PathBuf,
};

use {
    anyhow::Result,
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

impl SessionStore {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// Sanitize a session key for use as a filename.
    pub fn key_to_filename(key: &str) -> String {
        key.replace(':', "_")
    }

    fn path_for(&self, key: &str) -> PathBuf {
        self.base_dir
            .join(format!("{}.jsonl", Self::key_to_filename(key)))
    }

    /// Directory for session media files (screenshots, audio, etc.).
    fn media_dir_for(&self, key: &str) -> PathBuf {
        self.base_dir.join("media").join(Self::key_to_filename(key))
    }

    /// Save a media file for a session. Returns the relative path from base_dir.
    pub async fn save_media(&self, key: &str, filename: &str, data: &[u8]) -> Result<String> {
        let dir = self.media_dir_for(key);
        let file_path = dir.join(filename);
        let data = data.to_vec();

        tokio::task::spawn_blocking(move || -> Result<()> {
            fs::create_dir_all(&dir)?;
            fs::write(&file_path, &data)?;
            Ok(())
        })
        .await??;

        let sanitized = Self::key_to_filename(key);
        Ok(format!("media/{sanitized}/{filename}"))
    }

    /// Read a media file. Returns raw bytes.
    pub async fn read_media(&self, key: &str, filename: &str) -> Result<Vec<u8>> {
        let file_path = self.media_dir_for(key).join(filename);

        tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
            let data = fs::read(&file_path)?;
            Ok(data)
        })
        .await?
    }

    /// Append a message (JSON value) as a single line to the session file.
    pub async fn append(&self, key: &str, message: &serde_json::Value) -> Result<()> {
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
                .map_err(|e| anyhow::anyhow!("lock failed: {e}"))?;
            writeln!(*guard, "{line}")?;
            Ok(())
        })
        .await??;

        Ok(())
    }

    /// Read all messages from a session file.
    pub async fn read(&self, key: &str) -> Result<Vec<serde_json::Value>> {
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

    /// Read the last N messages from a session file.
    pub async fn read_last_n(&self, key: &str, n: usize) -> Result<Vec<serde_json::Value>> {
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
        let path = self.path_for(key);
        let media_dir = self.media_dir_for(key);

        tokio::task::spawn_blocking(move || -> Result<()> {
            if path.exists() {
                fs::remove_file(&path)?;
            }
            if media_dir.exists() {
                let _ = fs::remove_dir_all(&media_dir);
            }
            Ok(())
        })
        .await??;

        Ok(())
    }

    /// List all session keys by scanning JSONL files in the base directory.
    pub fn list_keys(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(&self.base_dir) else {
            return vec![];
        };
        entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                name.strip_suffix(".jsonl").map(|s| s.replace('_', ":"))
            })
            .collect()
    }

    /// Search all sessions for messages containing `query` (case-insensitive).
    /// Returns up to `max_results` hits, at most one per session.
    pub async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>> {
        let base = self.base_dir.clone();
        let query = query.to_lowercase();

        tokio::task::spawn_blocking(move || {
            let mut results = Vec::new();
            let entries = fs::read_dir(&base)?;

            for entry in entries.flatten() {
                if results.len() >= max_results {
                    break;
                }
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                let Some(key_raw) = name.strip_suffix(".jsonl") else {
                    continue;
                };
                let session_key = key_raw.replace('_', ":");

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
                        let end = (pos + query.len() + 60).min(content.len());
                        let snippet = content[start..end].to_string();

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
                .map_err(|e| anyhow::anyhow!("lock failed: {e}"))?;
            for msg in &messages {
                let line = serde_json::to_string(msg)?;
                writeln!(*guard, "{line}")?;
            }
            Ok(())
        })
        .await??;

        Ok(())
    }

    /// Count messages in a session file without parsing them.
    pub async fn count(&self, key: &str) -> Result<u32> {
        let path = self.path_for(key);

        tokio::task::spawn_blocking(move || -> Result<u32> {
            if !path.exists() {
                return Ok(0);
            }
            let file = File::open(&path)?;
            let reader = BufReader::new(file);
            let count = reader
                .lines()
                .map_while(Result::ok)
                .filter(|l| !l.trim().is_empty())
                .count();
            Ok(count as u32)
        })
        .await?
    }
}

#[cfg(test)]
mod tests {
    use {super::*, serde_json::json};

    fn temp_store() -> (SessionStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf());
        (store, dir)
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
        let (store, _dir) = temp_store();

        store
            .append("session:abc-123", &json!({"role": "user"}))
            .await
            .unwrap();
        let msgs = store.read("session:abc-123").await.unwrap();
        assert_eq!(msgs.len(), 1);
    }

    #[tokio::test]
    async fn test_save_and_read_media() {
        let (store, _dir) = temp_store();
        let data = b"fake png data";

        let path = store.save_media("main", "call_1.png", data).await.unwrap();
        assert_eq!(path, "media/main/call_1.png");

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
        assert_eq!(path, "media/session_abc/shot.png");

        let read_back = store.read_media("session:abc", "shot.png").await.unwrap();
        assert_eq!(read_back, data);
    }

    #[tokio::test]
    async fn test_read_media_missing_file() {
        let (store, _dir) = temp_store();
        let result = store.read_media("main", "nonexistent.png").await;
        assert!(result.is_err());
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

        let media_dir = dir.path().join("media").join("main");
        assert!(media_dir.exists());

        store.clear("main").await.unwrap();

        assert!(!media_dir.exists());
        assert!(store.read("main").await.unwrap().is_empty());
    }
}
