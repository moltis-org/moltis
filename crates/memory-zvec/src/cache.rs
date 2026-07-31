use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use {
    anyhow::{Context, Result},
    half::f16,
    moltis_memory::schema::FileRow,
    redb::{Database, ReadableTable, ReadableTableMetadata, TableDefinition},
    tracing::{debug, warn},
};

const EMBEDDING_CACHE: TableDefinition<&str, &[u8]> = TableDefinition::new("embeddings");
/// Durable file-metadata table: `path` → JSON-encoded [`FileRow`]. zvec's HNSW
/// index cannot reliably enumerate docs by filter, so file metadata is kept in
/// this exact-key store instead of the vector collection.
const FILES: TableDefinition<&str, &[u8]> = TableDefinition::new("files");
/// Path → JSON-encoded `Vec<String>` of chunk primary keys. Lets us fetch all
/// chunks of a file by exact key lookup instead of an unreliable HNSW filter.
const CHUNK_INDEX: TableDefinition<&str, &[u8]> = TableDefinition::new("chunk_index");

#[derive(Debug, Clone)]
pub struct ZvecCacheConfig {
    pub dimension: u32,
    pub cache_max_entries: usize,
}

impl Default for ZvecCacheConfig {
    fn default() -> Self {
        Self {
            dimension: 768,
            cache_max_entries: 200_000,
        }
    }
}

pub struct RedbCache {
    db: Database,
    dimension: u32,
    max_entries: usize,
}

impl RedbCache {
    pub fn new(db_path: &Path) -> Result<Self> {
        Self::with_config(db_path, ZvecCacheConfig::default())
    }

    pub fn with_config(db_path: &Path, config: ZvecCacheConfig) -> Result<Self> {
        let db = if db_path.exists() {
            Database::open(db_path).context("failed to open redb embedding cache")?
        } else {
            Database::create(db_path).context("failed to create redb embedding cache")?
        };
        debug!(
            path = %db_path.display(),
            dimension = config.dimension,
            max_entries = config.cache_max_entries,
            "redb embedding cache opened"
        );
        let cache = Self {
            db,
            dimension: config.dimension,
            max_entries: config.cache_max_entries,
        };
        match cache.sweep_dimension_orphans() {
            Ok(removed) => {
                if removed > 0 {
                    debug!(removed, "swept dimension-orphaned cache entries on startup");
                }
            },
            Err(e) => {
                warn!(error = %e, "failed to sweep dimension orphans on startup");
            },
        }
        Ok(cache)
    }

    #[must_use]
    pub fn cache_key(provider: &str, model: &str, dimension: u32, hash: &str) -> String {
        format!("{provider}\0{model}\0{dimension}\0{hash}")
    }

    pub fn get_cached_embedding(&self, key: &str) -> Result<Option<Vec<f32>>> {
        let read_tx = self
            .db
            .begin_read()
            .context("failed to begin read transaction")?;
        let table = match read_tx.open_table(EMBEDDING_CACHE) {
            Ok(t) => t,
            Err(_) => return Ok(None),
        };

        let embedding = match table.get(key) {
            Ok(Some(guard)) => {
                let value_bytes: &[u8] = guard.value();
                if value_bytes.len() < 8 {
                    return Ok(None);
                }
                decode_embedding_payload(&value_bytes[8..])
            },
            Ok(None) => None,
            Err(e) => return Err(anyhow::anyhow!("redb get error: {e}")),
        };

        // No write-on-read: skip access-timestamp update to avoid serializing
        // concurrent reads. Eviction uses insertion order instead of LRU,
        // which is fine for embeddings (created during sync, rarely re-read).

        Ok(embedding)
    }

    pub fn put_cached_embedding(&self, key: &str, embedding: &[f32]) -> Result<()> {
        if self.max_entries > 0 {
            let count = self.count_cached_embeddings()?;
            if count >= self.max_entries {
                self.evict_embedding_cache(self.max_entries.saturating_sub(1))?;
            }
        }

        let value = build_value(embedding);
        let tx = self
            .db
            .begin_write()
            .context("failed to begin write transaction")?;
        {
            let mut table = tx
                .open_table(EMBEDDING_CACHE)
                .context("failed to open embedding cache table")?;
            table
                .insert(key, value.as_slice())
                .context("failed to insert embedding")?;
        }
        tx.commit().context("failed to commit write transaction")?;
        Ok(())
    }

    pub fn put_cached_embeddings_batch(&self, entries: &[(String, Vec<f32>)]) -> Result<()> {
        if self.max_entries > 0 && !entries.is_empty() {
            let count = self.count_cached_embeddings()?;
            let needed = count + entries.len();
            if needed > self.max_entries {
                let keep = self.max_entries.saturating_sub(entries.len());
                self.evict_embedding_cache(keep)?;
            }
        }

        let tx = self
            .db
            .begin_write()
            .context("failed to begin write transaction")?;
        {
            let mut table = tx
                .open_table(EMBEDDING_CACHE)
                .context("failed to open embedding cache table")?;
            for (key, embedding) in entries {
                let value = build_value(embedding);
                table
                    .insert(key.as_str(), value.as_slice())
                    .context("failed to batch-insert embedding")?;
            }
        }
        tx.commit()
            .context("failed to commit batch write transaction")?;
        Ok(())
    }

    pub fn delete_matching_model_hash(&self, model: &str, hash: &str) -> Result<usize> {
        self.delete_matching_model_hash_batch(&[(model.to_string(), hash.to_string())])
    }

    /// Delete all embedding-cache entries matching ANY of the given
    /// `(model, hash)` pairs in a **single table scan + single write txn**.
    /// Avoids O(pairs × cache_size) when deleting many chunks at once.
    pub fn delete_matching_model_hash_batch(&self, pairs: &[(String, String)]) -> Result<usize> {
        if pairs.is_empty() {
            return Ok(0);
        }
        let keys_to_delete: Vec<String> = {
            let tx = self
                .db
                .begin_read()
                .context("failed to begin read transaction")?;
            let table = match tx.open_table(EMBEDDING_CACHE) {
                Ok(t) => t,
                Err(_) => return Ok(0),
            };

            let mut keys = Vec::new();
            for item in table.iter().context("failed to iterate embedding cache")? {
                let (key_guard, _value_guard) =
                    item.context("failed to read embedding cache entry")?;
                let key: &str = key_guard.value();
                if pairs.iter().any(|(m, h)| key_matches_model_hash(key, m, h)) {
                    keys.push(key.to_string());
                }
            }
            keys
        };

        if keys_to_delete.is_empty() {
            return Ok(0);
        }

        let removed = keys_to_delete.len();
        let tx = self
            .db
            .begin_write()
            .context("failed to begin write transaction for cache invalidation")?;
        {
            let mut table = tx
                .open_table(EMBEDDING_CACHE)
                .context("failed to open embedding cache table for invalidation")?;
            for key in &keys_to_delete {
                table
                    .remove(key.as_str())
                    .context("failed to remove invalidated cache entry")?;
            }
        }
        tx.commit()
            .context("failed to commit cache invalidation transaction")?;
        debug!(removed, "invalidated stale embedding cache entries (batch)");
        Ok(removed)
    }

    pub fn sweep_dimension_orphans(&self) -> Result<usize> {
        let keys_to_delete: Vec<String> = {
            let tx = self
                .db
                .begin_read()
                .context("failed to begin read transaction")?;
            let table = match tx.open_table(EMBEDDING_CACHE) {
                Ok(t) => t,
                Err(_) => return Ok(0),
            };

            let mut keys = Vec::new();
            for item in table.iter().context("failed to iterate embedding cache")? {
                let (key_guard, _value_guard) =
                    item.context("failed to read embedding cache entry")?;
                let key: &str = key_guard.value();
                if parse_dimension_from_key(key) != Some(self.dimension) {
                    keys.push(key.to_string());
                }
            }
            keys
        };

        if keys_to_delete.is_empty() {
            return Ok(0);
        }

        let removed = keys_to_delete.len();
        let tx = self
            .db
            .begin_write()
            .context("failed to begin write transaction for orphan sweep")?;
        {
            let mut table = tx
                .open_table(EMBEDDING_CACHE)
                .context("failed to open embedding cache table for orphan sweep")?;
            for key in &keys_to_delete {
                table
                    .remove(key.as_str())
                    .context("failed to remove orphaned cache entry")?;
            }
        }
        tx.commit()
            .context("failed to commit orphan sweep transaction")?;
        debug!(
            removed,
            current_dim = self.dimension,
            "swept orphaned embedding cache entries"
        );
        Ok(removed)
    }

    // ── File metadata + chunk-path index ──
    // zvec's HNSW index can't reliably enumerate docs by filter, so file
    // metadata and the per-path chunk-PK index live in this redb database as
    // exact-key tables. The embedding cache table above is unaffected.

    pub fn upsert_file_row(&self, file: &FileRow) -> Result<()> {
        let encoded = serde_json::to_vec(file).context("failed to serialize file row")?;
        let tx = self
            .db
            .begin_write()
            .context("failed to begin write transaction for file row")?;
        {
            let mut table = tx.open_table(FILES).context("failed to open files table")?;
            table
                .insert(file.path.as_str(), encoded.as_slice())
                .context("failed to insert file row")?;
        }
        tx.commit().context("failed to commit file row")?;
        Ok(())
    }

    pub fn get_file_row(&self, path: &str) -> Result<Option<FileRow>> {
        let tx = self
            .db
            .begin_read()
            .context("failed to begin read transaction for file row")?;
        let table = match tx.open_table(FILES) {
            Ok(t) => t,
            Err(_) => return Ok(None),
        };
        match table.get(path).context("failed to read file row")? {
            Some(guard) => {
                let file: FileRow = serde_json::from_slice(guard.value())
                    .context("failed to deserialize file row")?;
                Ok(Some(file))
            },
            None => Ok(None),
        }
    }

    pub fn delete_file_row(&self, path: &str) -> Result<()> {
        let tx = self
            .db
            .begin_write()
            .context("failed to begin write transaction for file delete")?;
        {
            let mut table = tx
                .open_table(FILES)
                .context("failed to open files table for delete")?;
            table.remove(path).context("failed to remove file row")?;
        }
        tx.commit().context("failed to commit file delete")?;
        Ok(())
    }

    pub fn list_file_rows(&self) -> Result<Vec<FileRow>> {
        let tx = self
            .db
            .begin_read()
            .context("failed to begin read transaction for file list")?;
        let table = match tx.open_table(FILES) {
            Ok(t) => t,
            Err(_) => return Ok(Vec::new()),
        };
        let mut rows = Vec::new();
        for item in table.iter().context("failed to iterate files table")? {
            let (_key, value) = item.context("failed to read files table entry")?;
            let file: FileRow = serde_json::from_slice(value.value())
                .context("failed to deserialize file row during list")?;
            rows.push(file);
        }
        rows.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(rows)
    }

    /// Replace the chunk-PK list for `path`.
    pub fn set_chunk_pks(&self, path: &str, pks: &[String]) -> Result<()> {
        let encoded = serde_json::to_vec(pks).context("failed to serialize chunk pks")?;
        let tx = self
            .db
            .begin_write()
            .context("failed to begin write transaction for chunk index")?;
        {
            let mut table = tx
                .open_table(CHUNK_INDEX)
                .context("failed to open chunk_index table")?;
            table
                .insert(path, encoded.as_slice())
                .context("failed to insert chunk pks")?;
        }
        tx.commit().context("failed to commit chunk index")?;
        Ok(())
    }

    pub fn get_chunk_pks(&self, path: &str) -> Result<Vec<String>> {
        let tx = self
            .db
            .begin_read()
            .context("failed to begin read transaction for chunk index")?;
        let table = match tx.open_table(CHUNK_INDEX) {
            Ok(t) => t,
            Err(_) => return Ok(Vec::new()),
        };
        match table.get(path).context("failed to read chunk pks")? {
            Some(guard) => {
                let pks: Vec<String> = serde_json::from_slice(guard.value())
                    .context("failed to deserialize chunk pks")?;
                Ok(pks)
            },
            None => Ok(Vec::new()),
        }
    }

    pub fn remove_chunk_pks(&self, path: &str) -> Result<()> {
        let tx = self
            .db
            .begin_write()
            .context("failed to begin write transaction for chunk index delete")?;
        {
            let mut table = tx
                .open_table(CHUNK_INDEX)
                .context("failed to open chunk_index table for delete")?;
            table.remove(path).context("failed to remove chunk pks")?;
        }
        tx.commit().context("failed to commit chunk index delete")?;
        Ok(())
    }

    /// Remove specific PKs from a path's chunk-PK index entry, preserving any
    /// other PKs (e.g. concurrently-added chunks). If the removal empties the
    /// list, the entry is removed entirely.
    pub fn remove_specific_chunk_pks(&self, path: &str, pks: &[String]) -> Result<()> {
        if pks.is_empty() {
            return Ok(());
        }
        let pk_set: std::collections::HashSet<&str> = pks.iter().map(String::as_str).collect();
        let tx = self
            .db
            .begin_write()
            .context("failed to begin write transaction for chunk index update")?;
        {
            let mut table = tx
                .open_table(CHUNK_INDEX)
                .context("failed to open chunk_index table for update")?;

            // Read + deserialize, dropping the immutable borrow before mutating.
            let filtered: Option<Vec<String>> = {
                let guard = table
                    .get(path)
                    .context("failed to read chunk pks for update")?;
                match guard {
                    Some(g) => {
                        let existing: Vec<String> = serde_json::from_slice(g.value())
                            .context("failed to deserialize chunk pks for update")?;
                        Some(
                            existing
                                .into_iter()
                                .filter(|pk| !pk_set.contains(pk.as_str()))
                                .collect(),
                        )
                    },
                    None => None,
                }
            };

            match filtered {
                Some(remaining) if remaining.is_empty() => {
                    table
                        .remove(path)
                        .context("failed to remove empty chunk pks")?;
                },
                Some(remaining) => {
                    let encoded = serde_json::to_vec(&remaining)
                        .context("failed to serialize filtered chunk pks")?;
                    table
                        .insert(path, encoded.as_slice())
                        .context("failed to write filtered chunk pks")?;
                },
                None => {},
            }
        }
        tx.commit().context("failed to commit chunk index update")?;
        Ok(())
    }

    /// Atomically extend the per-path chunk-PK index. Each path's existing PKs
    /// are read, merged, and written back in one write transaction — redb
    /// serializes these, so concurrent callers can't drop each other's PKs.
    pub fn extend_chunk_pks_batch(&self, entries: &[(String, Vec<String>)]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let tx = self
            .db
            .begin_write()
            .context("failed to begin write transaction for chunk index extend")?;
        {
            let mut table = tx
                .open_table(CHUNK_INDEX)
                .context("failed to open chunk_index table for extend")?;
            for (path, new_pks) in entries {
                if new_pks.is_empty() {
                    continue;
                }
                // The txn sees its own prior writes within this loop.
                let merged: Vec<String> = match table
                    .get(path.as_str())
                    .context("failed to read chunk pks for extend")?
                {
                    Some(guard) => {
                        let mut existing: Vec<String> = serde_json::from_slice(guard.value())
                            .context("failed to deserialize chunk pks for extend")?;
                        let existing_set: std::collections::HashSet<String> =
                            existing.iter().cloned().collect();
                        for pk in new_pks {
                            if !existing_set.contains(pk) {
                                existing.push(pk.clone());
                            }
                        }
                        existing
                    },
                    None => new_pks.clone(),
                };
                let encoded =
                    serde_json::to_vec(&merged).context("failed to serialize merged chunk pks")?;
                table
                    .insert(path.as_str(), encoded.as_slice())
                    .context("failed to insert merged chunk pks")?;
            }
        }
        tx.commit().context("failed to commit chunk index extend")?;
        Ok(())
    }

    pub fn count_cached_embeddings(&self) -> Result<usize> {
        let tx = self
            .db
            .begin_read()
            .context("failed to begin read transaction")?;
        let table = match tx.open_table(EMBEDDING_CACHE) {
            Ok(t) => t,
            Err(_) => return Ok(0),
        };
        let count = table.len().context("failed to get table length")?;
        Ok(count as usize)
    }

    pub fn evict_embedding_cache(&self, keep: usize) -> Result<usize> {
        let mut entries: Vec<(String, u64)> = {
            let tx = self
                .db
                .begin_read()
                .context("failed to begin read transaction")?;
            let table = match tx.open_table(EMBEDDING_CACHE) {
                Ok(t) => t,
                Err(_) => return Ok(0),
            };

            let mut vec = Vec::new();
            for item in table.iter().context("failed to iterate embedding cache")? {
                let (key_guard, value_guard) =
                    item.context("failed to read embedding cache entry")?;
                let key: &str = key_guard.value();
                let value: &[u8] = value_guard.value();
                if value.len() < 8 {
                    continue;
                }
                // `value[..8]` is guaranteed 8 bytes (len < 8 is skipped above),
                // but avoid the denied `unwrap_used` lint with a safe fallback.
                let bytes: [u8; 8] = value[..8].try_into().unwrap_or([0; 8]);
                let timestamp = u64::from_le_bytes(bytes);
                vec.push((key.to_string(), timestamp));
            }
            vec
        };

        if entries.len() <= keep {
            return Ok(0);
        }

        entries.sort_by_key(|b| std::cmp::Reverse(b.1));
        let evict_count = entries.len() - keep;

        let tx = self
            .db
            .begin_write()
            .context("failed to begin write transaction for eviction")?;
        {
            let mut table = tx
                .open_table(EMBEDDING_CACHE)
                .context("failed to open embedding cache table for eviction")?;
            for (key, _) in &entries[keep..] {
                table
                    .remove(key.as_str())
                    .context("failed to remove evicted entry")?;
            }
        }
        tx.commit()
            .context("failed to commit eviction transaction")?;
        debug!(evicted = evict_count, "evicted old embedding cache entries");
        Ok(evict_count)
    }
}

fn key_matches_model_hash(key: &str, model: &str, hash: &str) -> bool {
    let parts: Vec<&str> = key.split('\0').collect();
    if parts.len() == 3 {
        parts[1] == model && parts[2] == hash
    } else if parts.len() >= 4 {
        parts[1] == model && parts[3] == hash
    } else {
        false
    }
}

fn parse_dimension_from_key(key: &str) -> Option<u32> {
    let parts: Vec<&str> = key.split('\0').collect();
    parts.get(2).and_then(|s| s.parse().ok())
}

fn build_value(embedding: &[f32]) -> Vec<u8> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let mut value = Vec::with_capacity(8 + embedding.len() * 2);
    value.extend_from_slice(&now.to_le_bytes());
    for f in embedding {
        value.extend_from_slice(&f16::from_f32(*f).to_le_bytes());
    }
    value
}

fn decode_embedding_payload(payload: &[u8]) -> Option<Vec<f32>> {
    f16_bytes_to_f32_vec(payload).ok()
}

fn f16_bytes_to_f32_vec(bytes: &[u8]) -> Result<Vec<f32>> {
    if !bytes.len().is_multiple_of(2) {
        anyhow::bail!("invalid fp16 embedding byte length: not a multiple of 2");
    }
    Ok(bytes
        .chunks_exact(2)
        .map(|c| f16::from_le_bytes([c[0], c[1]]).to_f32())
        .collect())
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn cache_path() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("moltis-redb-cache-test");
        let _ = std::fs::create_dir_all(&dir);
        let count = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let file = format!("test_{}_{}.redb", std::process::id(), count);
        dir.join(file)
    }

    #[test]
    fn test_put_and_get() {
        let path = cache_path();
        let _ = std::fs::remove_file(&path);
        let cache = RedbCache::new(&path).unwrap();

        let key = RedbCache::cache_key("openai", "text-embedding-3-small", 4, "abc123");
        let embedding = vec![0.1f32, 0.2, 0.3, -0.5];
        cache.put_cached_embedding(&key, &embedding).unwrap();

        let result = cache.get_cached_embedding(&key).unwrap();
        assert!(result.is_some());
        let got = result.unwrap();
        assert_eq!(got.len(), embedding.len());
        for (a, b) in got.iter().zip(embedding.iter()) {
            assert!((a - b).abs() < 1e-3);
        }

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_get_missing_key() {
        let path = cache_path();
        let _ = std::fs::remove_file(&path);
        let cache = RedbCache::new(&path).unwrap();

        let result = cache.get_cached_embedding("nonexistent").unwrap();
        assert!(result.is_none());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_count() {
        let path = cache_path();
        let _ = std::fs::remove_file(&path);
        let cache = RedbCache::new(&path).unwrap();

        assert_eq!(cache.count_cached_embeddings().unwrap(), 0);

        cache.put_cached_embedding("key1", &[0.1, 0.2]).unwrap();
        assert_eq!(cache.count_cached_embeddings().unwrap(), 1);

        cache.put_cached_embedding("key2", &[0.3, 0.4]).unwrap();
        assert_eq!(cache.count_cached_embeddings().unwrap(), 2);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_batch_insert() {
        let path = cache_path();
        let _ = std::fs::remove_file(&path);
        let cache = RedbCache::new(&path).unwrap();

        let entries = vec![
            ("a".to_string(), vec![1.0f32, 2.0]),
            ("b".to_string(), vec![3.0, 4.0]),
            ("c".to_string(), vec![5.0, 6.0]),
        ];
        cache.put_cached_embeddings_batch(&entries).unwrap();

        assert_eq!(cache.count_cached_embeddings().unwrap(), 3);
        assert!(cache.get_cached_embedding("a").unwrap().is_some());
        assert!(cache.get_cached_embedding("b").unwrap().is_some());
        assert!(cache.get_cached_embedding("c").unwrap().is_some());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_evict() {
        let path = cache_path();
        let _ = std::fs::remove_file(&path);
        let cache = RedbCache::new(&path).unwrap();

        cache.put_cached_embedding("old1", &[0.0]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        cache.put_cached_embedding("old2", &[0.0]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        cache.put_cached_embedding("newest", &[0.0]).unwrap();

        assert_eq!(cache.count_cached_embeddings().unwrap(), 3);

        let evicted = cache.evict_embedding_cache(2).unwrap();
        assert_eq!(evicted, 1);
        assert_eq!(cache.count_cached_embeddings().unwrap(), 2);

        assert!(cache.get_cached_embedding("newest").unwrap().is_some());
        assert!(cache.get_cached_embedding("old2").unwrap().is_some());
        assert!(cache.get_cached_embedding("old1").unwrap().is_none());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_evict_none_when_under_limit() {
        let path = cache_path();
        let _ = std::fs::remove_file(&path);
        let cache = RedbCache::new(&path).unwrap();

        cache.put_cached_embedding("k1", &[1.0]).unwrap();
        cache.put_cached_embedding("k2", &[2.0]).unwrap();

        let evicted = cache.evict_embedding_cache(5).unwrap();
        assert_eq!(evicted, 0);
        assert_eq!(cache.count_cached_embeddings().unwrap(), 2);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_count_new_database() {
        let path = cache_path();
        let _ = std::fs::remove_file(&path);
        let cache = RedbCache::new(&path).unwrap();
        assert_eq!(cache.count_cached_embeddings().unwrap(), 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_cache_key_includes_dimension() {
        let key = RedbCache::cache_key("openai", "text-embedding-3-small", 1536, "abc123");
        assert_eq!(key, "openai\0text-embedding-3-small\x001536\0abc123");
    }

    #[test]
    fn test_parse_dimension_from_key() {
        assert_eq!(parse_dimension_from_key("p\0m\x003072\0hash"), Some(3072));
        assert_eq!(parse_dimension_from_key("p\0m\x00768\0hash"), Some(768));
        assert_eq!(parse_dimension_from_key("p\0m\0hash"), None);
        assert_eq!(parse_dimension_from_key("justastring"), None);
    }

    #[test]
    fn test_key_matches_model_hash_new_format() {
        assert!(key_matches_model_hash(
            "p\0my-model\x003072\0my-hash",
            "my-model",
            "my-hash"
        ));
        assert!(!key_matches_model_hash(
            "p\0my-model\x003072\0other-hash",
            "my-model",
            "my-hash"
        ));
        assert!(!key_matches_model_hash(
            "p\0other-model\x003072\0my-hash",
            "my-model",
            "my-hash"
        ));
    }

    #[test]
    fn test_key_matches_model_hash_old_format() {
        assert!(key_matches_model_hash(
            "p\0my-model\0my-hash",
            "my-model",
            "my-hash"
        ));
        assert!(!key_matches_model_hash(
            "p\0other\0my-hash",
            "my-model",
            "my-hash"
        ));
    }

    #[test]
    fn test_sweep_dimension_orphans_removes_mismatched() {
        let path = cache_path();
        let _ = std::fs::remove_file(&path);

        let cache = RedbCache::with_config(&path, ZvecCacheConfig {
            dimension: 768,
            cache_max_entries: 200_000,
        })
        .unwrap();

        cache
            .put_cached_embedding("p\0m\x00768\0h1", &[1.0])
            .unwrap();
        cache
            .put_cached_embedding("p\0m\x00384\0h2", &[2.0])
            .unwrap();
        cache
            .put_cached_embedding("p\0m\x001536\0h3", &[3.0])
            .unwrap();
        cache
            .put_cached_embedding("old-format-key", &[4.0])
            .unwrap();

        assert_eq!(cache.count_cached_embeddings().unwrap(), 4);

        let removed = cache.sweep_dimension_orphans().unwrap();
        assert_eq!(removed, 3);

        assert!(
            cache
                .get_cached_embedding("p\0m\x00768\0h1")
                .unwrap()
                .is_some()
        );
        assert!(
            cache
                .get_cached_embedding("p\0m\x00384\0h2")
                .unwrap()
                .is_none()
        );
        assert!(
            cache
                .get_cached_embedding("p\0m\x001536\0h3")
                .unwrap()
                .is_none()
        );
        assert!(
            cache
                .get_cached_embedding("old-format-key")
                .unwrap()
                .is_none()
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_delete_matching_model_hash() {
        let path = cache_path();
        let _ = std::fs::remove_file(&path);
        let cache = RedbCache::new(&path).unwrap();

        cache
            .put_cached_embedding("p\0m1\x00768\0hash-a", &[1.0])
            .unwrap();
        cache
            .put_cached_embedding("p\0m1\x00768\0hash-b", &[2.0])
            .unwrap();
        cache
            .put_cached_embedding("p\0m2\x00768\0hash-a", &[3.0])
            .unwrap();
        cache
            .put_cached_embedding("p\0m2\x00768\0hash-c", &[4.0])
            .unwrap();

        assert_eq!(cache.count_cached_embeddings().unwrap(), 4);

        let removed = cache.delete_matching_model_hash("m1", "hash-a").unwrap();
        assert_eq!(removed, 1);
        assert!(
            cache
                .get_cached_embedding("p\0m1\x00768\0hash-a")
                .unwrap()
                .is_none()
        );
        assert!(
            cache
                .get_cached_embedding("p\0m1\x00768\0hash-b")
                .unwrap()
                .is_some()
        );
        assert!(
            cache
                .get_cached_embedding("p\0m2\x00768\0hash-a")
                .unwrap()
                .is_some()
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_sweep_dimension_orphans_empty_db() {
        let path = cache_path();
        let _ = std::fs::remove_file(&path);
        let cache = RedbCache::with_config(&path, ZvecCacheConfig {
            dimension: 768,
            cache_max_entries: 200_000,
        })
        .unwrap();
        assert_eq!(cache.sweep_dimension_orphans().unwrap(), 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_delete_matching_model_hash_nonexistent() {
        let path = cache_path();
        let _ = std::fs::remove_file(&path);
        let cache = RedbCache::new(&path).unwrap();
        assert_eq!(
            cache
                .delete_matching_model_hash("no-model", "no-hash")
                .unwrap(),
            0
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_fp16_roundtrip_preserves_precision() {
        let path = cache_path();
        let _ = std::fs::remove_file(&path);
        let cache = RedbCache::new(&path).unwrap();

        let key = RedbCache::cache_key("p", "m", 5, "h");
        let embedding = vec![0.1f32, -0.5, 1.0, 2.5, 3.5];
        cache.put_cached_embedding(&key, &embedding).unwrap();

        let result = cache.get_cached_embedding(&key).unwrap();
        assert!(result.is_some());
        let got = result.unwrap();
        assert_eq!(got.len(), embedding.len());
        for (a, b) in got.iter().zip(embedding.iter()) {
            assert!((a - b).abs() < 1e-3, "fp16 roundtrip precision: {a} vs {b}");
        }

        let _ = std::fs::remove_file(&path);
    }

    // ── chunk-PK index atomic extend ──

    #[test]
    fn test_set_get_chunk_pks_roundtrip() {
        let path = cache_path();
        let _ = std::fs::remove_file(&path);
        let cache = RedbCache::new(&path).unwrap();

        assert!(cache.get_chunk_pks("missing").unwrap().is_empty());

        let pks = vec!["a".to_string(), "b".to_string()];
        cache.set_chunk_pks("file.md", &pks).unwrap();
        assert_eq!(cache.get_chunk_pks("file.md").unwrap(), pks);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_extend_chunk_pks_batch_merges_and_dedups() {
        let path = cache_path();
        let _ = std::fs::remove_file(&path);
        let cache = RedbCache::new(&path).unwrap();

        // Seed an existing list for one file.
        cache.set_chunk_pks("a.md", &["pk1".into()]).unwrap();

        let entries = vec![
            ("a.md".to_string(), vec![
                "pk1".into(),
                "pk2".into(),
                "pk3".into(),
            ]),
            ("b.md".to_string(), vec!["bx".into()]),
        ];
        cache.extend_chunk_pks_batch(&entries).unwrap();

        // a.md: pk1 deduped, pk2/pk3 appended.
        assert_eq!(cache.get_chunk_pks("a.md").unwrap(), vec![
            "pk1".to_string(),
            "pk2".to_string(),
            "pk3".to_string()
        ]);
        // b.md: freshly created.
        assert_eq!(cache.get_chunk_pks("b.md").unwrap(), vec!["bx".to_string()]);
        // Untouched file stays empty.
        assert!(cache.get_chunk_pks("c.md").unwrap().is_empty());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_extend_chunk_pks_batch_empty_is_noop() {
        let path = cache_path();
        let _ = std::fs::remove_file(&path);
        let cache = RedbCache::new(&path).unwrap();

        cache.extend_chunk_pks_batch(&[]).unwrap();
        cache
            .extend_chunk_pks_batch(&[("x.md".to_string(), vec![])])
            .unwrap();
        assert!(cache.get_chunk_pks("x.md").unwrap().is_empty());

        let _ = std::fs::remove_file(&path);
    }

    /// Concurrent extends of the same path must never lose a PK.
    #[test]
    fn test_extend_chunk_pks_batch_concurrent_no_loss() {
        let path = cache_path();
        let _ = std::fs::remove_file(&path);
        let cache = std::sync::Arc::new(RedbCache::new(&path).unwrap());

        let threads = 8usize;
        let per_thread = 25usize;
        let mut handles = Vec::with_capacity(threads);
        for t in 0..threads {
            let cache = std::sync::Arc::clone(&cache);
            handles.push(std::thread::spawn(move || {
                let mut pks = Vec::with_capacity(per_thread);
                for i in 0..per_thread {
                    pks.push(format!("pk-{t}-{i}"));
                }
                cache
                    .extend_chunk_pks_batch(&[("shared.md".to_string(), pks)])
                    .unwrap();
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let stored = cache.get_chunk_pks("shared.md").unwrap();
        assert_eq!(
            stored.len(),
            threads * per_thread,
            "concurrent extends must not drop any PKs, got {}",
            stored.len()
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_remove_chunk_pks_deletes_entry() {
        let path = cache_path();
        let _ = std::fs::remove_file(&path);
        let cache = RedbCache::new(&path).unwrap();

        cache
            .set_chunk_pks("file.md", &["pk1".into(), "pk2".into()])
            .unwrap();
        assert_eq!(cache.get_chunk_pks("file.md").unwrap().len(), 2);

        cache.remove_chunk_pks("file.md").unwrap();
        assert!(
            cache.get_chunk_pks("file.md").unwrap().is_empty(),
            "entry must be gone after remove_chunk_pks"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_remove_chunk_pks_missing_is_noop() {
        let path = cache_path();
        let _ = std::fs::remove_file(&path);
        let cache = RedbCache::new(&path).unwrap();

        // Removing a key that was never set must succeed (no-op).
        cache.remove_chunk_pks("never-set.md").unwrap();

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_remove_specific_chunk_pks_empty_input_is_noop() {
        let path = cache_path();
        let _ = std::fs::remove_file(&path);
        let cache = RedbCache::new(&path).unwrap();

        cache
            .set_chunk_pks("file.md", &["pk1".into(), "pk2".into()])
            .unwrap();
        cache.remove_specific_chunk_pks("file.md", &[]).unwrap();
        assert_eq!(
            cache.get_chunk_pks("file.md").unwrap().len(),
            2,
            "empty removal set must leave entry untouched"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_remove_specific_chunk_pks_missing_path_is_noop() {
        let path = cache_path();
        let _ = std::fs::remove_file(&path);
        let cache = RedbCache::new(&path).unwrap();

        // Path never stored: None branch — must succeed without error.
        cache
            .remove_specific_chunk_pks("never-set.md", &["pk1".into()])
            .unwrap();

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_remove_specific_chunk_pks_empties_entry_removes_it() {
        let path = cache_path();
        let _ = std::fs::remove_file(&path);
        let cache = RedbCache::new(&path).unwrap();

        cache
            .set_chunk_pks("file.md", &["pk1".into(), "pk2".into()])
            .unwrap();
        // Remove every PK — list becomes empty, entry must be removed entirely.
        cache
            .remove_specific_chunk_pks("file.md", &["pk1".into(), "pk2".into()])
            .unwrap();
        assert!(
            cache.get_chunk_pks("file.md").unwrap().is_empty(),
            "entry must be removed when all PKs are deleted"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_remove_specific_chunk_pks_partial_preserves_rest() {
        let path = cache_path();
        let _ = std::fs::remove_file(&path);
        let cache = RedbCache::new(&path).unwrap();

        cache
            .set_chunk_pks("file.md", &["pk1".into(), "pk2".into(), "pk3".into()])
            .unwrap();
        // Remove one PK — remaining two must be written back.
        cache
            .remove_specific_chunk_pks("file.md", &["pk2".into()])
            .unwrap();
        let remaining = cache.get_chunk_pks("file.md").unwrap();
        assert_eq!(remaining.len(), 2);
        assert!(remaining.contains(&"pk1".to_string()));
        assert!(remaining.contains(&"pk3".to_string()));
        assert!(!remaining.contains(&"pk2".to_string()));

        let _ = std::fs::remove_file(&path);
    }
}
