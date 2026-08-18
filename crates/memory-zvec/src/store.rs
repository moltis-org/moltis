use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use {
    async_trait::async_trait,
    zvec::{Collection, Doc, Fts, SearchQuery},
};

use moltis_memory::{
    error::{Error, Result as MemoryResult},
    schema::{ChunkRow, FileRow},
    search::{self, SearchResult},
    store::{CacheEntry, MemoryStore, MergeStrategy},
};

use crate::{cache::RedbCache, chunks::ChunkDoc};

const DEFAULT_DIMENSION: u32 = 768;

/// Convert an `anyhow::Error` from a zvec/redb call into a [`Error::Backend`],
/// preserving the full error chain in the message.
fn map_err(e: anyhow::Error) -> Error {
    Error::Backend(format!("{e:#}"))
}

/// Run a blocking zvec/redb call on the blocking pool, mapping a join failure
/// (panic/cancellation) to [`Error::Backend`] instead of panicking.
async fn blocking<F, T>(f: F) -> MemoryResult<T>
where
    F: FnOnce() -> MemoryResult<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Error::Backend(format!("spawn_blocking join failed: {e}")))?
}

/// `true` if the document is a real chunk (not a file marker or meta doc).
fn is_real_chunk(doc: &Doc) -> bool {
    let source = doc.get_string("source").ok().flatten().unwrap_or_default();
    source != crate::files::FILE_MARKER && source != crate::collection::META_DOC_SOURCE
}

/// Build a [`SearchResult`] (without text) from a **vector** search result.
///
/// zvec's `get_score()` for vector (cosine) queries returns DISTANCE
/// (0.0 = identical, 1.0 = orthogonal). We convert to similarity
/// (1 − distance) so downstream ranking (which sorts descending) puts the
/// closest match first.
fn doc_to_vector_result(doc: &Doc) -> SearchResult {
    doc_with_score(doc, 1.0 - doc.get_score())
}

/// Build a [`SearchResult`] from a **keyword/FTS** search result.
///
/// zvec's `get_score()` for FTS queries returns BM25 relevance
/// (higher = more relevant, frequently > 1.0). We pass it through directly
/// (floored at 0.0) since downstream ranking sorts descending.
fn doc_to_keyword_result(doc: &Doc) -> SearchResult {
    doc_with_score(doc, doc.get_score().max(0.0))
}

fn doc_with_score(doc: &Doc, score: f32) -> SearchResult {
    SearchResult {
        chunk_id: doc.get_string("id").ok().flatten().unwrap_or_default(),
        path: doc.get_string("path").ok().flatten().unwrap_or_default(),
        source: doc.get_string("source").ok().flatten().unwrap_or_default(),
        start_line: doc.get_i64("start_line").ok().flatten().unwrap_or(0),
        end_line: doc.get_i64("end_line").ok().flatten().unwrap_or(0),
        score,
        text: doc.get_string("text").ok().flatten().unwrap_or_default(),
    }
}

pub struct ZvecMemoryStore {
    collection: Arc<Collection>,
    /// Auxiliary redb store: embedding cache + durable file metadata +
    /// per-path chunk-PK index. Always present: zvec's HNSW index can't
    /// enumerate docs by filter, so exact-key metadata lives here.
    cache: Arc<RedbCache>,
    /// Embedding dimension. Used for cache keys and dummy listing vectors.
    dimension: u32,
    /// Base path the collection was opened from (e.g. `<data_dir>/memory.zvec`).
    /// The real on-disk files carry a dimension suffix (`memory.zvec_768`), so
    /// [`ZvecMemoryStore::disk_size_bytes`] measures all files sharing this
    /// stem. `None` for in-memory/test stores.
    collection_path: Option<PathBuf>,
    /// Shutdown signal for the periodic-optimize task; dropping the store
    /// closes it and the task exits.
    shutdown: Option<tokio::sync::watch::Sender<()>>,
}

// The gateway spawns a periodic flush task that persists documents and the
// HNSW graph. That task is awaited on the Tokio runtime, which may be torn
// down during process exit before the final flush completes. To avoid silent
// data loss, we perform a best-effort final flush here on a dedicated OS
// thread — it runs independently of the Tokio runtime and completes even if
// the runtime has already been cancelled.
impl Drop for ZvecMemoryStore {
    fn drop(&mut self) {
        let collection = Arc::clone(&self.collection);
        // Dropping `shutdown` signals the periodic task, but we also flush
        // directly to not depend on the task completing before runtime exit.
        self.shutdown.take();
        std::thread::spawn(move || {
            if let Err(e) = collection.flush() {
                tracing::warn!(error = %e, "zvec: final flush on drop failed");
            }
        });
    }
}

impl ZvecMemoryStore {
    fn build(collection: Arc<Collection>, cache: RedbCache, dimension: u32) -> Self {
        // Prefer the dimension recorded in the collection's meta doc, falling
        // back to the caller-supplied value when no meta doc exists (e.g.
        // collections created by `initialize()` that never wrote one). This
        // keeps the embedding-cache key dimension in sync with reality even
        // when a caller forgets `.with_cache_dimension()`.
        let dimension = crate::read_dimension_meta(&collection)
            .ok()
            .flatten()
            .unwrap_or(dimension);
        Self {
            collection,
            cache: Arc::new(cache),
            dimension,
            collection_path: None,
            shutdown: None,
        }
    }

    pub fn new(collection: Collection, cache: RedbCache) -> Self {
        Self::build(Arc::new(collection), cache, DEFAULT_DIMENSION)
    }

    /// Alias kept for call-site readability when an owned collection is passed.
    pub fn with_cache(collection: Collection, cache: RedbCache) -> Self {
        Self::build(Arc::new(collection), cache, DEFAULT_DIMENSION)
    }

    pub fn with_cache_arc(collection: Arc<Collection>, cache: RedbCache) -> Self {
        Self::build(collection, cache, DEFAULT_DIMENSION)
    }

    pub fn with_cache_dimension(mut self, dimension: u32) -> Self {
        self.dimension = dimension;
        self
    }

    /// Record the on-disk collection path so [`MemoryStore::disk_size_bytes`]
    /// can report real disk usage. The zvec library appends a dimension suffix
    /// (`<path>_<dim>`) to the actual files, so the size helper measures every
    /// file sharing this stem.
    pub fn with_collection_disk_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.collection_path = Some(path.into());
        self
    }

    /// Attach the shutdown signal for the periodic-optimize task; dropped (and
    /// thus cancelled) when this store is dropped.
    pub fn with_shutdown_signal(mut self, shutdown: tokio::sync::watch::Sender<()>) -> Self {
        self.shutdown = Some(shutdown);
        self
    }

    pub fn collection_arc(&self) -> Arc<Collection> {
        Arc::clone(&self.collection)
    }

    /// Access the underlying redb auxiliary store (embedding cache + file index).
    pub fn cache_arc(&self) -> Arc<RedbCache> {
        Arc::clone(&self.cache)
    }

    /// Rebuild the zvec full-text-search index from durable documents.
    ///
    /// Re-feeds every stored chunk into the collection to ensure the FTS
    /// index is fully populated. Useful after data migrations or when the
    /// FTS index may be stale. Returns the number of chunks re-ingested.
    pub async fn rebuild_keyword_index(&self) -> MemoryResult<usize> {
        let files = self.list_files().await?;
        let mut total = 0usize;
        let mut all_docs: Vec<ChunkDoc> = Vec::new();

        for file in files {
            let chunks = self.get_chunks_for_file(&file.path).await?;
            if chunks.is_empty() {
                continue;
            }
            total += chunks.len();
            for chunk in &chunks {
                all_docs.push(ChunkDoc::from_chunk_row(chunk, self.dimension));
            }
        }

        if all_docs.is_empty() {
            return Ok(0);
        }

        // Re-feed chunks in batches of 1024 (self-imposed limit to keep
        // per-batch memory bounded) without per-batch flushing. A single flush
        // at the end avoids N/1024 synchronous fsyncs.
        for batch in all_docs.chunks(1024) {
            let collection = Arc::clone(&self.collection);
            let batch_vec = batch.to_vec();
            tokio::task::spawn_blocking(move || {
                crate::chunks::upsert_chunks_no_flush(&collection, &batch_vec)
            })
            .await
            .map_err(|e| Error::Backend(format!("spawn_blocking join failed: {e}")))?
            .map_err(|e| Error::Backend(e.to_string()))?;
        }

        // Single flush to persist all re-ingested chunks.
        let collection = self.collection_arc();
        blocking(move || crate::flush_collection(&collection).map_err(map_err)).await?;

        Ok(total)
    }
}

#[async_trait]
impl MemoryStore for ZvecMemoryStore {
    #[tracing::instrument(skip(self, file), fields(path = %file.path))]
    async fn upsert_file(&self, file: &FileRow) -> MemoryResult<()> {
        let cache = Arc::clone(&self.cache);
        let file = file.clone();
        blocking(move || cache.upsert_file_row(&file).map_err(map_err)).await
    }

    #[tracing::instrument(skip(self), fields(path = %path))]
    async fn get_file(&self, path: &str) -> MemoryResult<Option<FileRow>> {
        let cache = Arc::clone(&self.cache);
        let path = path.to_string();
        blocking(move || cache.get_file_row(&path).map_err(map_err)).await
    }

    #[tracing::instrument(skip(self), fields(path = %path))]
    async fn delete_file(&self, path: &str) -> MemoryResult<()> {
        // Remove chunks + their index entry, then the file metadata.
        self.delete_chunks_for_file(path).await?;
        let cache = Arc::clone(&self.cache);
        let path = path.to_string();
        blocking(move || cache.delete_file_row(&path).map_err(map_err)).await
    }

    #[tracing::instrument(skip(self))]
    async fn list_files(&self) -> MemoryResult<Vec<FileRow>> {
        let cache = Arc::clone(&self.cache);
        blocking(move || cache.list_file_rows().map_err(map_err)).await
    }

    #[tracing::instrument(skip(self, chunks), fields(count = chunks.len()))]
    async fn upsert_chunks(&self, chunks: &[ChunkRow]) -> MemoryResult<()> {
        let docs: Vec<ChunkDoc> = chunks
            .iter()
            .map(|c| ChunkDoc::from_chunk_row(c, self.dimension))
            .collect();

        // Write the redb PK index FIRST. If the process crashes before the
        // zvec upsert completes, phantom PKs remain (referencing absent zvec
        // docs) — these are silently filtered by fetch_with_options and removed
        // on the next delete_chunks_for_file. Reversing this order (zvec first)
        // would leave chunks searchable in zvec but invisible to management
        // paths, permanently polluting search results.
        let cache = Arc::clone(&self.cache);
        let mut by_path: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        for chunk in chunks {
            by_path
                .entry(chunk.path.clone())
                .or_default()
                .push(ChunkDoc::safe_pk(&chunk.id));
        }
        let entries: Vec<(String, Vec<String>)> = by_path.into_iter().collect();
        blocking(move || cache.extend_chunk_pks_batch(&entries).map_err(map_err)).await?;

        // Upsert chunk docs into the zvec collection (for vector/keyword search).
        let collection = Arc::clone(&self.collection);
        blocking(move || crate::chunks::upsert_chunks(&collection, &docs).map_err(map_err)).await?;

        Ok(())
    }

    #[tracing::instrument(skip(self), fields(path = %path))]
    async fn get_chunks_for_file(&self, path: &str) -> MemoryResult<Vec<ChunkRow>> {
        let cache = Arc::clone(&self.cache);
        let path_for_pks = path.to_string();
        let pks = blocking(move || cache.get_chunk_pks(&path_for_pks).map_err(map_err)).await?;

        if pks.is_empty() {
            return Ok(Vec::new());
        }

        let collection = Arc::clone(&self.collection);
        blocking(move || -> MemoryResult<Vec<ChunkRow>> {
            let pk_refs: Vec<&str> = pks.iter().map(String::as_str).collect();
            let docs = collection
                .fetch_with_options(&pk_refs, None, false)
                .map_err(|e| Error::Backend(e.to_string()))?;
            let rows = docs
                .iter()
                .filter(|doc| {
                    let source = doc.get_string("source").ok().flatten().unwrap_or_default();
                    source != crate::files::FILE_MARKER
                        && source != crate::collection::META_DOC_SOURCE
                })
                .map(ChunkDoc::from_doc)
                .collect::<anyhow::Result<Vec<_>>>()
                .map_err(map_err)?
                .into_iter()
                .map(ChunkRow::from)
                .collect();
            Ok(rows)
        })
        .await
    }

    #[tracing::instrument(skip(self), fields(path = %path))]
    async fn delete_chunks_for_file(&self, path: &str) -> MemoryResult<()> {
        let existing_chunks = self.get_chunks_for_file(path).await?;

        // Snapshot the PKs of the chunks we're about to delete. Deleting by PK
        // (instead of by path filter) ensures concurrently-upserted chunks for
        // the same path survive — they're not in this snapshot.
        let pks_to_delete: Vec<String> = existing_chunks
            .iter()
            .map(|c| ChunkDoc::safe_pk(&c.id))
            .collect();

        // Invalidate embedding cache entries for the removed chunks in a
        // single table scan (batched by unique model+hash pair).
        let pairs: Vec<(String, String)> = {
            let mut seen = HashSet::new();
            existing_chunks
                .iter()
                .filter(|c| seen.insert((c.model.clone(), c.hash.clone())))
                .map(|c| (c.model.clone(), c.hash.clone()))
                .collect()
        };
        if !pairs.is_empty() {
            let cache = Arc::clone(&self.cache);
            blocking(move || {
                cache
                    .delete_matching_model_hash_batch(&pairs)
                    .map_err(map_err)
            })
            .await?;
        }

        // Remove the chunk documents from the zvec collection by PK.
        let collection = Arc::clone(&self.collection);
        let pks = pks_to_delete.clone();
        blocking(move || crate::chunks::delete_chunks_by_pks(&collection, &pks).map_err(map_err))
            .await?;

        // Remove only the deleted PKs from the cache index, preserving any
        // concurrently-added PKs.
        let cache = Arc::clone(&self.cache);
        let path_for_index = path.to_string();
        let pks_for_cache = pks_to_delete;
        blocking(move || {
            cache
                .remove_specific_chunk_pks(&path_for_index, &pks_for_cache)
                .map_err(map_err)
        })
        .await
    }

    #[tracing::instrument(skip(self), fields(id = %id))]
    async fn get_chunk_by_id(&self, id: &str) -> MemoryResult<Option<ChunkRow>> {
        let collection = Arc::clone(&self.collection);
        let pk = ChunkDoc::safe_pk(id);
        blocking(move || -> MemoryResult<Option<ChunkRow>> {
            let docs = collection
                .fetch_with_options(&[&pk], None, false)
                .map_err(|e| Error::Backend(e.to_string()))?;
            let row = docs
                .iter()
                .find(|doc| {
                    let source = doc.get_string("source").ok().flatten().unwrap_or_default();
                    source != crate::files::FILE_MARKER
                        && source != crate::collection::META_DOC_SOURCE
                })
                .map(ChunkDoc::from_doc)
                .transpose()
                .map_err(map_err)?
                .map(ChunkRow::from);
            Ok(row)
        })
        .await
    }

    #[tracing::instrument(skip(self), fields(provider = %provider, model = %model))]
    async fn get_cached_embedding(
        &self,
        provider: &str,
        model: &str,
        hash: &str,
    ) -> MemoryResult<Option<Vec<f32>>> {
        let cache = Arc::clone(&self.cache);
        let key = RedbCache::cache_key(provider, model, self.dimension, hash);
        blocking(move || cache.get_cached_embedding(&key).map_err(map_err)).await
    }

    #[tracing::instrument(skip(self, embedding), fields(provider = %provider, model = %model))]
    async fn put_cached_embedding(
        &self,
        provider: &str,
        model: &str,
        _provider_key: &str,
        hash: &str,
        embedding: &[f32],
    ) -> MemoryResult<()> {
        let cache = Arc::clone(&self.cache);
        let key = RedbCache::cache_key(provider, model, self.dimension, hash);
        let emb = embedding.to_vec();
        blocking(move || cache.put_cached_embedding(&key, &emb).map_err(map_err)).await
    }

    #[tracing::instrument(skip(self, entries), fields(count = entries.len()))]
    async fn put_cached_embeddings_batch(&self, entries: &[CacheEntry<'_>]) -> MemoryResult<()> {
        let cache = Arc::clone(&self.cache);
        let dim = self.dimension;
        let batch: Vec<(String, Vec<f32>)> = entries
            .iter()
            .map(|e| {
                (
                    RedbCache::cache_key(e.provider, e.model, dim, e.hash),
                    e.embedding.to_vec(),
                )
            })
            .collect();
        blocking(move || cache.put_cached_embeddings_batch(&batch).map_err(map_err)).await
    }

    #[tracing::instrument(skip(self))]
    async fn count_cached_embeddings(&self) -> MemoryResult<usize> {
        let cache = Arc::clone(&self.cache);
        blocking(move || cache.count_cached_embeddings().map_err(map_err)).await
    }

    #[tracing::instrument(skip(self), fields(keep = keep))]
    async fn evict_embedding_cache(&self, keep: usize) -> MemoryResult<usize> {
        let cache = Arc::clone(&self.cache);
        blocking(move || cache.evict_embedding_cache(keep).map_err(map_err)).await
    }

    #[tracing::instrument(skip(self, embedding, query), fields(query_len = query.len(), limit))]
    async fn hybrid_search(
        &self,
        embedding: &[f32],
        query: &str,
        vector_weight: f32,
        keyword_weight: f32,
        merge_strategy: MergeStrategy,
        limit: usize,
    ) -> MemoryResult<Vec<SearchResult>> {
        let fetch_limit = limit.saturating_mul(3);

        // zvec SearchQuery targets a single field — FTS and vector search are
        // mutually exclusive within one route. `embedding` has HNSW (no FTS),
        // `text` has FTS (no HNSW). Per zvec docs: "run separate queries and
        // merge results in your application." We run both concurrently (they
        // share no state) and merge via the shared [`moltis_memory::search`]
        // functions.
        let (vector_results, keyword_results) = tokio::join!(
            async {
                if !embedding.is_empty() {
                    vector_search_zvec(&self.collection, embedding, fetch_limit).await
                } else {
                    Ok(Vec::new())
                }
            },
            async {
                if !query.is_empty() {
                    keyword_search_zvec(&self.collection, query, fetch_limit).await
                } else {
                    Ok(Vec::new())
                }
            },
        );
        let vector_results = vector_results?;
        let keyword_results = keyword_results?;

        let merged = match merge_strategy {
            MergeStrategy::Weighted => search::merge_weighted(
                &vector_results,
                &keyword_results,
                vector_weight,
                keyword_weight,
            ),
            MergeStrategy::Rrf { k } => search::merge_rrf(
                &vector_results,
                &keyword_results,
                vector_weight,
                keyword_weight,
                k,
            ),
        };

        let mut final_results: Vec<SearchResult> = merged.into_iter().take(limit).collect();
        search::fill_missing_text(self, &mut final_results).await?;
        Ok(final_results)
    }

    fn store_type(&self) -> &'static str {
        "zvec"
    }

    fn hnsw_percent(&self) -> Option<f64> {
        None
    }

    fn disk_size_bytes(&self) -> u64 {
        self.collection_path
            .as_deref()
            .map(disk_usage_for_stem)
            .unwrap_or(0)
    }
}

/// Sum the byte sizes of every regular file living next to `stem_path` whose
/// name starts with `stem_path`'s file-name component.
///
/// zvec writes its collection under `<path>_<dimension>` (a directory
/// containing LOCK, segment dirs, RocksDB files, etc.), and the redb embedding
/// cache lives at `<path>.cache` — all of which share the configured collection
/// stem. Matching by prefix therefore captures the full on-disk footprint
/// regardless of the active dimension, without double counting. Best-effort:
/// returns 0 if the directory cannot be read.
fn disk_usage_for_stem(stem_path: &Path) -> u64 {
    let Some(parent) = stem_path.parent() else {
        return 0;
    };
    let Some(stem) = stem_path.file_name().and_then(|s| s.to_str()) else {
        return 0;
    };
    let dir = match std::fs::read_dir(parent) {
        Ok(d) => d,
        Err(_) => return 0,
    };
    let mut total = 0u64;
    for entry in dir.flatten() {
        let matches = entry
            .file_name()
            .to_str()
            .map(|name| name.starts_with(stem))
            .unwrap_or(false);
        if !matches {
            continue;
        };
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        // Skip symlinks to avoid inflating disk usage with linked files.
        if meta.is_symlink() {
            continue;
        }
        if meta.is_file() {
            total += meta.len();
        } else if meta.is_dir() {
            total += sum_dir_recursive(&entry.path());
        }
    }
    total
}

/// Recursively sum the sizes of all non-symlink regular files in a directory.
fn sum_dir_recursive(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in entries.flatten() {
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if meta.is_symlink() {
            continue;
        }
        if meta.is_file() {
            total += meta.len();
        } else if meta.is_dir() {
            total += sum_dir_recursive(&entry.path());
        }
    }
    total
}

async fn vector_search_zvec(
    collection: &Arc<Collection>,
    query_embedding: &[f32],
    limit: usize,
) -> MemoryResult<Vec<SearchResult>> {
    let collection = Arc::clone(collection);
    let emb = query_embedding.to_vec();
    let limit_i32 = i32::try_from(limit).unwrap_or(i32::MAX);
    let backend_err = |e: zvec::Error| Error::Backend(e.to_string());

    blocking(move || -> MemoryResult<Vec<SearchResult>> {
        let mut query = SearchQuery::new("embedding", &emb, limit_i32).map_err(backend_err)?;
        query
            .set_output_fields(&["id", "path", "source", "start_line", "end_line", "text"])
            .map_err(backend_err)?;
        let docs = collection.query(&query).map_err(backend_err)?;

        // Log index completeness + top score for diagnostics.
        if let Ok(stats) = collection.stats() {
            tracing::debug!(
                doc_count = stats.doc_count,
                index_completeness = ?stats.indexes.iter().map(|i| (i.name.as_str(), i.completeness)).collect::<Vec<_>>(),
                result_count = docs.len(),
                top_score = docs.first().map(|d| d.get_score()).unwrap_or(-1.0),
                "vector_search_zvec: query executed"
            );
        }

        Ok(docs
            .into_iter()
            .filter(is_real_chunk)
            .map(|doc| doc_to_vector_result(&doc))
            .collect())
    })
    .await
}

async fn keyword_search_zvec(
    collection: &Arc<Collection>,
    query: &str,
    limit: usize,
) -> MemoryResult<Vec<SearchResult>> {
    let collection = Arc::clone(collection);
    let query_owned = query.to_string();
    let limit_i32 = i32::try_from(limit).unwrap_or(i32::MAX);
    let backend_err = |e: zvec::Error| Error::Backend(e.to_string());

    blocking(move || -> MemoryResult<Vec<SearchResult>> {
        let mut fts = Fts::new().map_err(backend_err)?;
        // Use match_string for natural-language matching (whole-word,
        // stemming) rather than query_string (boolean expression syntax).
        fts.set_match_string(&query_owned).map_err(backend_err)?;

        let mut s_query = SearchQuery::fts("text", &fts, limit_i32).map_err(backend_err)?;
        s_query
            .set_output_fields(&["id", "path", "source", "start_line", "end_line", "text"])
            .map_err(backend_err)?;
        s_query
            .set_filter(&format!(
                "source != '{}' AND source != '{}'",
                crate::files::FILE_MARKER,
                crate::collection::META_DOC_SOURCE,
            ))
            .map_err(backend_err)?;

        let docs = collection.query(&s_query).map_err(backend_err)?;

        Ok(docs
            .into_iter()
            .filter(is_real_chunk)
            .map(|doc| doc_to_keyword_result(&doc))
            .collect())
    })
    .await
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    fn ensure_init() {
        crate::ensure_zvec_initialized().unwrap();
    }

    fn cache_path() -> PathBuf {
        let dir = std::env::temp_dir().join("moltis-zvec-store-test");
        let _ = std::fs::create_dir_all(&dir);
        let file = format!(
            "store_test_{}_{}.redb",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        dir.join(file)
    }

    fn make_collection() -> (Arc<Collection>, RedbCache, tempfile::TempDir) {
        ensure_init();
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("db");
        let collection = crate::open_or_create_collection(&db_path, Some(768)).unwrap();
        let cache = RedbCache::new(&cache_path()).unwrap();
        (Arc::new(collection), cache, dir)
    }

    fn make_owned_collection() -> (Collection, tempfile::TempDir) {
        ensure_init();
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("db");
        let collection = crate::open_or_create_collection(&db_path, Some(768)).unwrap();
        (collection, dir)
    }

    /// Flush the store's collection and drop it cleanly, avoiding RocksDB
    /// cleanup errors from the C destructor.
    fn close_store(store: ZvecMemoryStore) {
        let coll = store.collection_arc();
        let _ = crate::flush_collection(&coll);
        drop(store);
    }

    // ── Cache tests ──

    #[tokio::test]
    async fn test_cache_put_and_get_roundtrip() {
        let cache_path = cache_path();
        let _ = std::fs::remove_file(&cache_path);
        let (collection, _dir) = make_owned_collection();
        let cache = RedbCache::new(&cache_path).unwrap();
        let store = ZvecMemoryStore::with_cache(collection, cache);

        let embedding = vec![0.1f32, 0.2, 0.3, -0.5];
        store
            .put_cached_embedding("openai", "test-model", "sk-key", "hash1", &embedding)
            .await
            .unwrap();

        let result = store
            .get_cached_embedding("openai", "test-model", "hash1")
            .await
            .unwrap();
        assert!(result.is_some(), "cached embedding must be retrievable");
        let got = result.unwrap();
        assert_eq!(got.len(), embedding.len());
        for (a, b) in got.iter().zip(embedding.iter()) {
            assert!((a - b).abs() < 1e-3);
        }

        close_store(store);
        let _ = std::fs::remove_file(&cache_path);
    }

    #[tokio::test]
    async fn test_cache_get_missing() {
        let cache_path = cache_path();
        let _ = std::fs::remove_file(&cache_path);
        let (collection, _dir) = make_owned_collection();
        let cache = RedbCache::new(&cache_path).unwrap();
        let store = ZvecMemoryStore::with_cache(collection, cache);

        let result = store
            .get_cached_embedding("no", "such", "key")
            .await
            .unwrap();
        assert!(result.is_none());

        close_store(store);
        let _ = std::fs::remove_file(&cache_path);
    }

    #[tokio::test]
    async fn test_cache_put_batch_and_count() {
        let cache_path = cache_path();
        let _ = std::fs::remove_file(&cache_path);
        let (collection, _dir) = make_owned_collection();
        let cache = RedbCache::new(&cache_path).unwrap();
        let store = ZvecMemoryStore::with_cache(collection, cache);

        assert_eq!(
            store.count_cached_embeddings().await.unwrap(),
            0,
            "initial count must be zero"
        );

        let entries: Vec<CacheEntry<'_>> = vec![
            CacheEntry {
                provider: "p",
                model: "m",
                provider_key: "k",
                hash: "h1",
                embedding: &[1.0, 2.0],
            },
            CacheEntry {
                provider: "p",
                model: "m",
                provider_key: "k",
                hash: "h2",
                embedding: &[3.0, 4.0],
            },
        ];
        store.put_cached_embeddings_batch(&entries).await.unwrap();

        let count = store.count_cached_embeddings().await.unwrap();
        assert_eq!(count, 2, "count must reflect both batch inserts");

        assert!(
            store
                .get_cached_embedding("p", "m", "h1")
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            store
                .get_cached_embedding("p", "m", "h2")
                .await
                .unwrap()
                .is_some()
        );

        close_store(store);
        let _ = std::fs::remove_file(&cache_path);
    }

    #[tokio::test]
    async fn test_cache_evict_clears_all() {
        let cache_path = cache_path();
        let _ = std::fs::remove_file(&cache_path);
        let (collection, _dir) = make_owned_collection();
        let cache = RedbCache::new(&cache_path).unwrap();
        let store = ZvecMemoryStore::with_cache(collection, cache);

        let entries: Vec<CacheEntry<'_>> = vec![
            CacheEntry {
                provider: "p",
                model: "m",
                provider_key: "k",
                hash: "e1",
                embedding: &[1.0],
            },
            CacheEntry {
                provider: "p",
                model: "m",
                provider_key: "k",
                hash: "e2",
                embedding: &[2.0],
            },
            CacheEntry {
                provider: "p",
                model: "m",
                provider_key: "k",
                hash: "e3",
                embedding: &[3.0],
            },
        ];
        store.put_cached_embeddings_batch(&entries).await.unwrap();

        let evicted = store.evict_embedding_cache(0).await.unwrap();
        assert_eq!(evicted, 3, "evict(0) must remove all entries");
        assert_eq!(store.count_cached_embeddings().await.unwrap(), 0);

        close_store(store);
        let _ = std::fs::remove_file(&cache_path);
    }

    #[tokio::test]
    async fn test_cache_evict_preserves_n() {
        let cache_path = cache_path();
        let _ = std::fs::remove_file(&cache_path);
        let (collection, _dir) = make_owned_collection();
        let cache = RedbCache::new(&cache_path).unwrap();
        let store = ZvecMemoryStore::with_cache(collection, cache);

        let entries: Vec<CacheEntry<'_>> = vec![
            CacheEntry {
                provider: "p",
                model: "m",
                provider_key: "k",
                hash: "keep1",
                embedding: &[1.0],
            },
            CacheEntry {
                provider: "p",
                model: "m",
                provider_key: "k",
                hash: "keep2",
                embedding: &[2.0],
            },
            CacheEntry {
                provider: "p",
                model: "m",
                provider_key: "k",
                hash: "evict1",
                embedding: &[3.0],
            },
        ];
        store.put_cached_embeddings_batch(&entries).await.unwrap();

        let evicted = store.evict_embedding_cache(2).await.unwrap();
        assert_eq!(evicted, 1);
        assert_eq!(store.count_cached_embeddings().await.unwrap(), 2);

        close_store(store);
        let _ = std::fs::remove_file(&cache_path);
    }

    // merge_weighted / merge_rrf unit coverage lives in moltis-memory's
    // `search::tests` module so it runs regardless of which backend is
    // compiled. The zvec backend exercises them end-to-end via
    // `hybrid_search` in the tests below.

    // ── Disk usage tests ──

    #[test]
    fn test_disk_usage_for_stem_sums_matching_files() {
        let dir = tempfile::tempdir().unwrap();
        let stem = dir.path().join("memory.zvec");
        // Simulate the files zvec/redb write: dimension-suffixed collection,
        // an auxiliary file, and the `.cache` embedding cache.
        std::fs::write(dir.path().join("memory.zvec_768"), [0u8; 100]).unwrap();
        std::fs::write(dir.path().join("memory.zvec.lock"), [0u8; 30]).unwrap();
        std::fs::write(dir.path().join("memory.zvec.cache"), [0u8; 70]).unwrap();
        // Unrelated file must be ignored.
        std::fs::write(dir.path().join("memory.db"), [0u8; 999]).unwrap();

        assert_eq!(disk_usage_for_stem(&stem), 200);
    }

    #[test]
    fn test_disk_usage_for_stem_no_match_is_zero() {
        let dir = tempfile::tempdir().unwrap();
        let stem = dir.path().join("absent.zvec");
        assert_eq!(disk_usage_for_stem(&stem), 0);
    }

    /// Covers the `is_dir()` branch of disk_usage_for_stem (~line 558-560) and
    /// the recursive walk in sum_dir_recursive (~578-582): the existing tests
    /// only match regular files, never a matching subdirectory.
    #[test]
    fn test_disk_usage_for_stem_includes_matching_directories() {
        let dir = tempfile::tempdir().unwrap();
        let stem = dir.path().join("memory.zvec");

        // A matching directory containing nested files and a subdirectory.
        let subdir = dir.path().join("memory.zvec_768");
        std::fs::create_dir_all(&subdir).unwrap();
        std::fs::write(subdir.join("CURRENT"), [0u8; 50]).unwrap();
        std::fs::write(subdir.join("MANIFEST"), [0u8; 30]).unwrap();
        let nested = subdir.join("seg");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("data"), [0u8; 120]).unwrap();

        // A matching regular file alongside it (also counted).
        std::fs::write(dir.path().join("memory.zvec.cache"), [0u8; 40]).unwrap();
        // Non-matching file ignored.
        std::fs::write(dir.path().join("other.db"), [0u8; 999]).unwrap();

        // 50 + 30 + 120 (recursive dir) + 40 (file) = 240.
        assert_eq!(disk_usage_for_stem(&stem), 240);
    }

    #[test]
    fn test_disk_usage_for_stem_without_parent_is_zero() {
        // A bare file name with no parent dir component → early return 0.
        assert_eq!(disk_usage_for_stem(Path::new("bare")), 0);
    }

    /// Covers the MemoryStore trait methods store_type() and hnsw_percent()
    /// (~lines 503-509), which were never exercised by any test.
    #[tokio::test]
    async fn test_trait_store_type_and_hnsw_percent() {
        ensure_init();
        let dir = tempfile::tempdir().unwrap();
        let collection =
            crate::open_or_create_collection(&dir.path().join("db"), Some(768)).unwrap();
        let cache = RedbCache::new(&cache_path()).unwrap();
        let store = ZvecMemoryStore::new(collection, cache);

        assert_eq!(store.store_type(), "zvec");
        assert_eq!(store.hnsw_percent(), None);

        close_store(store);
    }

    /// Covers disk_size_bytes() (~lines 511-516): with no collection_path it
    /// returns 0; with the path set it delegates to disk_usage_for_stem and
    /// reports the real on-disk footprint of the flushed collection.
    #[tokio::test]
    async fn test_disk_size_bytes_reports_collection_footprint() {
        ensure_init();
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("ds-db");

        // Without the path, disk_size_bytes must be 0 (no collection_path set).
        {
            let collection = crate::open_or_create_collection(&db_path, Some(768)).unwrap();
            let cache = RedbCache::new(&cache_path()).unwrap();
            let store = ZvecMemoryStore::new(collection, cache);
            assert_eq!(store.disk_size_bytes(), 0, "no path → 0");
            // Flush so on-disk files exist under <parent>/ds-db_768/.
            let coll = store.collection_arc();
            crate::flush_collection(&coll).unwrap();
            close_store(store);
        }

        let store_with_path = ZvecMemoryStore::with_cache(
            crate::open_or_create_collection(&db_path, Some(768)).unwrap(),
            RedbCache::new(&cache_path()).unwrap(),
        )
        .with_collection_disk_path(&db_path);
        // Measure both in the same stable state (no writes between) so the
        // trait method's delegation to disk_usage_for_stem is exact.
        let expected = disk_usage_for_stem(&db_path);
        assert!(
            expected > 0,
            "flushed collection must produce matching files ({expected} bytes)"
        );
        assert_eq!(
            store_with_path.disk_size_bytes(),
            expected,
            "disk_size_bytes must equal disk_usage_for_stem for the same stem"
        );

        close_store(store_with_path);
    }

    /// Covers the all-empty early-return of rebuild_keyword_index (~line
    /// 200-201): with no chunks anywhere, it must return Ok(0) without
    /// attempting any upsert.
    #[tokio::test]
    async fn test_rebuild_keyword_index_empty_returns_zero() {
        ensure_init();
        let dir = tempfile::tempdir().unwrap();
        let collection =
            crate::open_or_create_collection(&dir.path().join("db"), Some(768)).unwrap();
        let cache = RedbCache::new(&cache_path()).unwrap();
        let store = ZvecMemoryStore::with_cache(collection, cache).with_cache_dimension(768);

        let count = store.rebuild_keyword_index().await.unwrap();
        assert_eq!(count, 0, "empty store rebuild must report 0 chunks");

        close_store(store);
    }

    /// Covers the per-file `continue` (~line 191-192) when a listed file has no
    /// chunks: register one file with no chunks and one file with chunks; the
    /// empty one must be skipped and the populated one re-ingested.
    #[tokio::test]
    async fn test_rebuild_keyword_index_skips_files_without_chunks() {
        ensure_init();
        let dir = tempfile::tempdir().unwrap();
        let collection =
            crate::open_or_create_collection(&dir.path().join("db"), Some(768)).unwrap();
        let cache = RedbCache::new(&cache_path()).unwrap();
        let store = ZvecMemoryStore::with_cache(collection, cache).with_cache_dimension(768);

        // File A: registered but no chunks → must hit the `continue`.
        store
            .upsert_file(&FileRow {
                path: "empty.md".into(),
                source: "empty.md".into(),
                hash: "he".into(),
                mtime: 1,
                size: 1,
            })
            .await
            .unwrap();

        // File B: registered with one chunk → must be re-ingested.
        store
            .upsert_file(&FileRow {
                path: "full.md".into(),
                source: "full.md".into(),
                hash: "hf".into(),
                mtime: 1,
                size: 1,
            })
            .await
            .unwrap();
        store
            .upsert_chunks(&[ChunkRow {
                id: "full-1".into(),
                path: "full.md".into(),
                source: "test".into(),
                start_line: 1,
                end_line: 2,
                hash: "hf".into(),
                model: "m".into(),
                text: "rebuild skip test chunk".into(),
                embedding: Some(vec![0u8; 768 * 4]),
                updated_at: "2025-01-01T00:00:00Z".into(),
            }])
            .await
            .unwrap();

        // Only the one chunk from full.md must be counted (empty.md skipped).
        let count = store.rebuild_keyword_index().await.unwrap();
        assert_eq!(count, 1, "only the populated file's chunk must be counted");

        close_store(store);
    }

    // ── Explicit flush ──

    #[tokio::test]
    async fn test_explicit_flush_persists_collection() {
        ensure_init();
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("flush-db");
        {
            let collection = crate::open_or_create_collection(&db_path, Some(768)).unwrap();
            let cache = RedbCache::new(&cache_path()).unwrap();
            let store = ZvecMemoryStore::new(collection, cache);
            store
                .upsert_chunks(&[ChunkRow {
                    id: "flush-1".into(),
                    path: "flush.md".into(),
                    source: "test".into(),
                    start_line: 1,
                    end_line: 2,
                    hash: "fh".into(),
                    model: "m".into(),
                    text: "explicit flush test".into(),
                    embedding: Some(vec![0u8; 768 * 4]),
                    updated_at: "2025-01-01T00:00:00Z".into(),
                }])
                .await
                .unwrap();
            let coll = store.collection_arc();
            crate::flush_collection(&coll).unwrap();
        }
        // Reopen: the chunk must survive because it was explicitly flushed.
        let collection = crate::open_or_create_collection(&db_path, Some(768)).unwrap();
        let got = crate::files::get_chunk_by_id(&collection, "flush-1").unwrap();
        assert!(got.is_some(), "chunk must survive explicit flush + reopen");
    }

    // ── Hybrid search with both vector and keyword ──

    #[tokio::test]
    async fn test_hybrid_search_both_vector_and_keyword() {
        ensure_init();
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("hybrid-db");
        let collection = crate::open_or_create_collection(&db_path, Some(768)).unwrap();
        let collection = Arc::new(collection);
        let cache = RedbCache::new(&cache_path()).unwrap();
        let store = ZvecMemoryStore::with_cache_arc(Arc::clone(&collection), cache);

        let file = FileRow {
            path: "search/hybrid.md".into(),
            source: "test".into(),
            hash: "h".into(),
            mtime: 1,
            size: 10,
        };
        store.upsert_file(&file).await.unwrap();

        let mut emb = vec![0.0f32; 768];
        emb[0] = 1.0;
        let emb_bytes: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();

        let chunks = vec![
            ChunkRow {
                id: "hy-a".into(),
                path: "search/hybrid.md".into(),
                source: "test".into(),
                start_line: 1,
                end_line: 5,
                hash: "ha".into(),
                model: "m".into(),
                text: "the quick brown fox".into(),
                embedding: Some(emb_bytes.clone()),
                updated_at: "2025-01-01T00:00:00Z".into(),
            },
            ChunkRow {
                id: "hy-b".into(),
                path: "search/hybrid.md".into(),
                source: "test".into(),
                start_line: 6,
                end_line: 10,
                hash: "hb".into(),
                model: "m".into(),
                text: "machine learning concepts".into(),
                embedding: Some(emb_bytes),
                updated_at: "2025-01-01T00:00:00Z".into(),
            },
        ];
        store.upsert_chunks(&chunks).await.unwrap();

        let results = store
            .hybrid_search(&emb, "fox", 0.5, 0.5, MergeStrategy::Weighted, 10)
            .await
            .unwrap();
        assert!(
            !results.is_empty(),
            "hybrid search with both vector and keyword must return results"
        );

        let results_rrf = store
            .hybrid_search(&emb, "learning", 0.5, 0.5, MergeStrategy::Rrf { k: 60 }, 10)
            .await
            .unwrap();
        assert!(
            !results_rrf.is_empty(),
            "RRF hybrid search with both vector and keyword must return results"
        );

        close_store(store);
    }

    // ── get_chunks_for_file empty early return ──

    #[tokio::test]
    async fn test_get_chunks_for_file_empty_path() {
        let (collection, cache, _dir) = make_collection();
        let store = ZvecMemoryStore::with_cache_arc(collection, cache);

        let result = store
            .get_chunks_for_file("path-with-no-entries")
            .await
            .unwrap();
        assert!(result.is_empty());

        close_store(store);
    }

    // ── Hybrid search with both empty ──

    #[tokio::test]
    async fn test_hybrid_search_both_empty_inputs() {
        let (collection, cache, _dir) = make_collection();
        let store = ZvecMemoryStore::with_cache_arc(collection, cache);

        let results = store
            .hybrid_search(&[], "", 1.0, 1.0, MergeStrategy::Weighted, 10)
            .await
            .unwrap();
        assert!(results.is_empty());

        close_store(store);
    }

    // ── Store trait: file ops persist to the collection ──

    #[tokio::test]
    async fn test_store_file_roundtrip() {
        let (collection, cache, _dir) = make_collection();
        let store = ZvecMemoryStore::with_cache_arc(collection, cache);

        let file = FileRow {
            path: "f1.md".into(),
            source: "test".into(),
            hash: "h1".into(),
            mtime: 100,
            size: 200,
        };
        store.upsert_file(&file).await.unwrap();

        let fetched = store.get_file("f1.md").await.unwrap();
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().path, "f1.md");

        let all = store.list_files().await.unwrap();
        assert_eq!(all.len(), 1);

        store.delete_file("f1.md").await.unwrap();
        assert!(store.get_file("f1.md").await.unwrap().is_none());

        close_store(store);
    }

    #[tokio::test]
    async fn test_store_upsert_chunks_and_get_by_id() {
        let (collection, cache, _dir) = make_collection();
        let store = ZvecMemoryStore::with_cache_arc(Arc::clone(&collection), cache);

        let emb = vec![1.0f32; 768];
        let emb_bytes: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
        let chunks = vec![ChunkRow {
            id: "sc-1".into(),
            path: "sc.md".into(),
            source: "test".into(),
            start_line: 1,
            end_line: 5,
            hash: "h".into(),
            model: "m".into(),
            text: "store chunk text".into(),
            embedding: Some(emb_bytes),
            updated_at: "2025-01-01T00:00:00Z".into(),
        }];
        store.upsert_chunks(&chunks).await.unwrap();

        let chunk = store.get_chunk_by_id("sc-1").await.unwrap();
        assert!(chunk.is_some(), "chunk must be fetchable by id");
        assert_eq!(chunk.unwrap().text, "store chunk text");

        close_store(store);
    }

    /// Regression test for keyword-only mode (P1): upserting chunks with no
    /// embedding must not fail with a zvec dimension-mismatch error. This
    /// happens when `memory.embedding_dimension` is set but no embedding
    /// provider is configured — the manager runs keyword-only, produces
    /// `ChunkRow`s with `embedding: None`, and the store must substitute a
    /// zero vector matching the collection dimension.
    #[tokio::test]
    async fn test_store_upsert_chunks_keyword_only_no_embedding() {
        let (collection, cache, _dir) = make_collection();
        let store = ZvecMemoryStore::with_cache_arc(Arc::clone(&collection), cache);

        let chunks = vec![ChunkRow {
            id: "kw-store-1".into(),
            path: "kw-store.md".into(),
            source: "test".into(),
            start_line: 1,
            end_line: 5,
            hash: "h".into(),
            model: String::new(),
            text: "keyword-only store chunk".into(),
            embedding: None,
            updated_at: "2025-01-01T00:00:00Z".into(),
        }];
        store
            .upsert_chunks(&chunks)
            .await
            .expect("keyword-only upsert must succeed via zero-vector fallback");

        let chunk = store
            .get_chunk_by_id("kw-store-1")
            .await
            .expect("fetch must succeed")
            .expect("keyword-only chunk must be retrievable");
        assert_eq!(chunk.text, "keyword-only store chunk");

        // Keyword search must still find it — the whole point of keyword-only mode.
        let results = store
            .hybrid_search(&[], "keyword-only", 0.0, 1.0, MergeStrategy::Weighted, 10)
            .await
            .unwrap();
        assert!(
            results.iter().any(|r| r.chunk_id == "kw-store-1"),
            "keyword search must find the keyword-only chunk"
        );

        close_store(store);
    }

    #[tokio::test]
    async fn test_store_get_chunks_for_file_with_data() {
        let (collection, cache, _dir) = make_collection();
        let store = ZvecMemoryStore::with_cache_arc(Arc::clone(&collection), cache);

        let emb = vec![1.0f32; 768];
        let emb_bytes: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
        let chunks = vec![
            ChunkRow {
                id: "gcf-1".into(),
                path: "gcf.md".into(),
                source: "test".into(),
                start_line: 1,
                end_line: 5,
                hash: "ha".into(),
                model: "m".into(),
                text: "first chunk".into(),
                embedding: Some(emb_bytes.clone()),
                updated_at: "2025-01-01T00:00:00Z".into(),
            },
            ChunkRow {
                id: "gcf-2".into(),
                path: "gcf.md".into(),
                source: "test".into(),
                start_line: 6,
                end_line: 10,
                hash: "hb".into(),
                model: "m".into(),
                text: "second chunk".into(),
                embedding: Some(emb_bytes),
                updated_at: "2025-01-01T00:00:00Z".into(),
            },
        ];
        store.upsert_chunks(&chunks).await.unwrap();

        let result = store.get_chunks_for_file("gcf.md").await.unwrap();
        assert_eq!(result.len(), 2, "must return both chunks for file");

        store.delete_chunks_for_file("gcf.md").await.unwrap();
        let after_delete = store.get_chunks_for_file("gcf.md").await.unwrap();
        assert!(after_delete.is_empty());

        close_store(store);
    }

    /// Regression test: concurrent upserts on the same path must not orphan
    /// chunk PKs (the old read-then-write index update lost them).
    #[tokio::test]
    async fn test_concurrent_upserts_keep_all_chunk_pks() {
        let (collection, cache, _dir) = make_collection();
        let store = Arc::new(ZvecMemoryStore::with_cache_arc(collection, cache));

        let emb_bytes: Vec<u8> = vec![1.0f32; 768]
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        let path = "concurrent.md";

        let groups = 6usize;
        let per_group = 4usize;
        let mut handles = Vec::new();
        for g in 0..groups {
            let store = Arc::clone(&store);
            let emb_bytes = emb_bytes.clone();
            handles.push(tokio::spawn(async move {
                let chunks: Vec<ChunkRow> = (0..per_group)
                    .map(|i| ChunkRow {
                        id: format!("cc-{g}-{i}"),
                        path: "concurrent.md".into(),
                        source: "test".into(),
                        start_line: ((g * per_group + i) as i64) * 10,
                        end_line: ((g * per_group + i) as i64) * 10 + 9,
                        hash: "h".into(),
                        model: "m".into(),
                        text: format!("chunk {g}-{i}"),
                        embedding: Some(emb_bytes.clone()),
                        updated_at: "2025-01-01T00:00:00Z".into(),
                    })
                    .collect();
                store.upsert_chunks(&chunks).await.unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        let fetched = store.get_chunks_for_file(path).await.unwrap();
        assert_eq!(
            fetched.len(),
            groups * per_group,
            "concurrent upserts must not orphan chunks; got {} of {}",
            fetched.len(),
            groups * per_group
        );

        // Cleanup must also reach every chunk now that the index is complete.
        store.delete_chunks_for_file(path).await.unwrap();
        let after_delete = store.get_chunks_for_file(path).await.unwrap();
        assert!(
            after_delete.is_empty(),
            "delete after concurrent upserts must clear every chunk"
        );

        let coll = store.collection_arc();
        let _ = crate::flush_collection(&coll);
        drop(store);
    }

    #[tokio::test]
    async fn test_store_hybrid_search_vector_only_over_limit() {
        let (collection, cache, _dir) = make_collection();
        let store = ZvecMemoryStore::with_cache_arc(Arc::clone(&collection), cache);

        let results = store
            .hybrid_search(
                &[],
                "nonexistent_query_xyz",
                0.0,
                1.0,
                MergeStrategy::Weighted,
                5,
            )
            .await
            .unwrap();
        assert!(
            results.is_empty(),
            "keyword search with no matching data must be empty"
        );

        close_store(store);
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
#[path = "store_persistence_tests.rs"]
mod persistence_tests;
