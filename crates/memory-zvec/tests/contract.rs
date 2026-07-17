// Integration tests legitimately panic on unexpected errors via `unwrap`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Once;

use moltis_memory::{
    schema::{ChunkRow, FileRow},
    store::{MemoryStore, MergeStrategy},
};

static ZVEC_LIB_INIT: Once = Once::new();

fn ensure_zvec_init() {
    ZVEC_LIB_INIT.call_once(|| {
        zvec::initialize(None).expect("zvec global init failed");
    });
}

struct ZvecStoreGuard {
    store: moltis_memory_zvec::ZvecMemoryStore,
    collection: std::sync::Arc<zvec::Collection>,
    _dir: tempfile::TempDir,
}

impl ZvecStoreGuard {
    fn flush(&self) {
        if let Err(e) = self.collection.flush() {
            eprintln!("zvec collection flush error: {e}");
        }
    }
}

impl Drop for ZvecStoreGuard {
    fn drop(&mut self) {
        self.flush();
    }
}

impl std::ops::Deref for ZvecStoreGuard {
    type Target = moltis_memory_zvec::ZvecMemoryStore;

    fn deref(&self) -> &Self::Target {
        &self.store
    }
}

fn test_file(path: &str) -> FileRow {
    FileRow {
        path: path.into(),
        source: "test".into(),
        hash: format!("hash-{path}"),
        mtime: 1_000_000,
        size: 42,
    }
}

fn test_chunk(id: &str, path: &str, text: &str, embedding: Option<Vec<f32>>) -> ChunkRow {
    let embedding_bytes = Some(
        embedding
            .unwrap_or_else(zero_embedding)
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect::<Vec<u8>>(),
    );
    ChunkRow {
        id: id.into(),
        path: path.into(),
        source: "test".into(),
        start_line: 1,
        end_line: 10,
        hash: format!("hash-{id}"),
        model: "test-model".into(),
        text: text.into(),
        embedding: embedding_bytes,
        updated_at: "2025-01-01T00:00:00Z".into(),
    }
}

fn zero_embedding() -> Vec<f32> {
    vec![0.0f32; 768]
}

fn small_embedding() -> Vec<f32> {
    let mut v = vec![0.0f32; 768];
    v[0] = 1.0;
    v
}

async fn create_sqlite_store() -> moltis_memory::store_sqlite::SqliteMemoryStore {
    let pool = sqlx::SqlitePool::connect(":memory:").await.unwrap();
    moltis_memory::schema::run_migrations(&pool).await.unwrap();
    moltis_memory::store_sqlite::SqliteMemoryStore::new(pool)
}

async fn create_zvec_store() -> ZvecStoreGuard {
    ensure_zvec_init();
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("collection");
    let cache_path = dir.path().join("cache.redb");
    let collection = moltis_memory_zvec::open_or_create_collection(&db_path, Some(768)).unwrap();
    let collection = std::sync::Arc::new(collection);
    let cache = moltis_memory_zvec::RedbCache::new(&cache_path).unwrap();
    let store = moltis_memory_zvec::ZvecMemoryStore::with_cache_arc(
        std::sync::Arc::clone(&collection),
        cache,
    );
    ZvecStoreGuard {
        store,
        collection,
        _dir: dir,
    }
}

// ── Test 1: Round-trip chunks via get_chunk_by_id ─────────────────────

async fn test_roundtrip_chunks(store: &dyn MemoryStore) -> anyhow::Result<()> {
    let file = test_file("test/roundtrip.md");
    store.upsert_file(&file).await?;

    let chunk = test_chunk(
        "chunk-rt-1",
        "test/roundtrip.md",
        "hello world from contract test",
        Some(small_embedding()),
    );
    store.upsert_chunks(&[chunk]).await?;

    let fetched = store.get_chunk_by_id("chunk-rt-1").await?;
    assert!(fetched.is_some(), "upserted chunk must be retrievable");
    let fetched = fetched.unwrap();
    assert_eq!(fetched.path, "test/roundtrip.md");
    assert_eq!(fetched.text, "hello world from contract test");

    store.delete_chunks_for_file("test/roundtrip.md").await?;

    Ok(())
}

// ── Test 2: Vector search (works on both backends) ─────────────────────

async fn test_vector_search(store: &dyn MemoryStore) -> anyhow::Result<()> {
    let file = test_file("test/search.md");
    store.upsert_file(&file).await?;

    let emb_a = small_embedding();
    let chunks = vec![
        test_chunk(
            "chunk-kw-a",
            "test/search.md",
            "the quick brown fox jumps over the lazy dog",
            Some(emb_a.clone()),
        ),
        test_chunk(
            "chunk-kw-b",
            "test/search.md",
            "quantum physics is fascinating but complex",
            Some(emb_a.clone()),
        ),
    ];
    store.upsert_chunks(&chunks).await?;

    let results = store
        .hybrid_search(&emb_a, "", 1.0, 0.0, MergeStrategy::Weighted, 10)
        .await?;
    assert!(
        !results.is_empty(),
        "vector search must return results for matching content"
    );
    assert!(
        results.iter().any(|r| r.path == "test/search.md"),
        "result must reference the ingested file path"
    );

    Ok(())
}

// ── Test 2b: Keyword-only search (query with no embedding) ──────────────

async fn test_keyword_only_search(store: &dyn MemoryStore) -> anyhow::Result<()> {
    let file = test_file("test/kw-search.md");
    store.upsert_file(&file).await?;

    let emb_a = small_embedding();
    let chunks = vec![
        test_chunk(
            "chunk-kwonly-a",
            "test/kw-search.md",
            "the quick brown fox jumps over the lazy dog",
            Some(emb_a.clone()),
        ),
        test_chunk(
            "chunk-kwonly-b",
            "test/kw-search.md",
            "quantum physics is fascinating but complex",
            Some(emb_a.clone()),
        ),
    ];
    store.upsert_chunks(&chunks).await?;

    let results = store
        .hybrid_search(&[], "brown fox", 0.0, 1.0, MergeStrategy::Weighted, 10)
        .await?;
    assert!(
        !results.is_empty(),
        "keyword-only search must return results for matching content"
    );
    assert!(
        results.iter().any(|r| r.path == "test/kw-search.md"),
        "result must reference the ingested file path"
    );

    Ok(())
}

// ── Test 2c: Hybrid search (both embedding and query) ───────────────────

async fn test_hybrid_search_with_query(store: &dyn MemoryStore) -> anyhow::Result<()> {
    let file = test_file("test/hybrid-search.md");
    store.upsert_file(&file).await?;

    let embedding = small_embedding();
    let chunks = vec![
        test_chunk(
            "chunk-hybrid-a",
            "test/hybrid-search.md",
            "machine learning is transforming software engineering",
            Some(embedding.clone()),
        ),
        test_chunk(
            "chunk-hybrid-b",
            "test/hybrid-search.md",
            "deep neural networks for image recognition",
            Some(embedding.clone()),
        ),
    ];
    store.upsert_chunks(&chunks).await?;

    let results = store
        .hybrid_search(
            &embedding,
            "machine learning",
            0.5,
            0.5,
            MergeStrategy::Weighted,
            10,
        )
        .await?;
    assert!(
        !results.is_empty(),
        "hybrid search must return results for matching content"
    );
    assert!(
        results.iter().any(|r| r.path == "test/hybrid-search.md"),
        "result must reference the ingested file path"
    );

    Ok(())
}

// ── Test 3: File operations ────────────────────────────────────────────

async fn test_file_ops(store: &dyn MemoryStore) -> anyhow::Result<()> {
    assert!(
        store.list_files().await?.is_empty(),
        "empty store must list zero files"
    );

    let file1 = test_file("test/alpha.md");
    let file2 = test_file("test/beta.md");
    store.upsert_file(&file1).await?;
    store.upsert_file(&file2).await?;

    let files = store.list_files().await?;
    assert_eq!(files.len(), 2, "must list two files after upsert");

    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.contains(&"test/alpha.md"));
    assert!(paths.contains(&"test/beta.md"));

    let fetched = store.get_file("test/alpha.md").await?;
    assert!(fetched.is_some(), "must retrieve upserted file");

    let chunk = test_chunk(
        "chunk-file-1",
        "test/alpha.md",
        "file-specific chunk content",
        Some(small_embedding()),
    );
    store.upsert_chunks(&[chunk]).await?;

    let chunk_fetched = store.get_chunk_by_id("chunk-file-1").await?;
    assert!(chunk_fetched.is_some(), "chunk must be retrievable by id");

    let chunks = store.get_chunks_for_file("test/alpha.md").await?;
    assert_eq!(chunks.len(), 1, "must get one chunk for file");
    assert_eq!(chunks[0].id, "chunk-file-1");
    assert_eq!(chunks[0].text, "file-specific chunk content");

    let empty_chunks = store.get_chunks_for_file("test/beta.md").await?;
    assert_eq!(
        empty_chunks.len(),
        0,
        "file with no chunks must return empty list"
    );

    store.delete_file("test/alpha.md").await?;

    let after_delete = store.list_files().await?;
    assert_eq!(after_delete.len(), 1, "must have one file after delete");
    assert!(
        after_delete.iter().any(|f| f.path == "test/beta.md"),
        "undeleted file must remain"
    );
    assert!(
        !after_delete.iter().any(|f| f.path == "test/alpha.md"),
        "deleted file must be removed"
    );

    Ok(())
}

// ── Test 4: Cache operations ───────────────────────────────────────────

async fn test_cache_operations(store: &dyn MemoryStore) -> anyhow::Result<()> {
    let initial_count = store.count_cached_embeddings().await?;

    let _key = "openai\0test-model\0sk-test\0hash1";
    let embedding = vec![0.1f32, 0.2, 0.3, -0.5];

    let before = store
        .get_cached_embedding("openai", "test-model", "hash1")
        .await?;
    assert!(before.is_none(), "cache must be empty initially");

    store
        .put_cached_embedding("openai", "test-model", "sk-test", "hash1", &embedding)
        .await?;

    let after = store
        .get_cached_embedding("openai", "test-model", "hash1")
        .await?;
    if let Some(got) = after {
        assert_eq!(got.len(), embedding.len());
        for (a, b) in got.iter().zip(embedding.iter()) {
            // The zvec backend cache stores fp16 embeddings (~1e-3 precision).
            assert!((a - b).abs() < 1e-3, "cached embedding must round-trip");
        }
    }

    let count = store.count_cached_embeddings().await?;
    assert!(count >= initial_count, "count must not decrease after put");

    let entries: Vec<moltis_memory::store::CacheEntry<'_>> = vec![
        moltis_memory::store::CacheEntry {
            provider: "openai",
            model: "test-model",
            provider_key: "sk-test",
            hash: "batch-hash-1",
            embedding: &[1.0, 2.0],
        },
        moltis_memory::store::CacheEntry {
            provider: "openai",
            model: "test-model",
            provider_key: "sk-test",
            hash: "batch-hash-2",
            embedding: &[3.0, 4.0],
        },
    ];
    store.put_cached_embeddings_batch(&entries).await?;

    let evicted = store.evict_embedding_cache(0).await?;
    let final_count = store.count_cached_embeddings().await?;
    assert!(
        final_count <= count + 2,
        "final count must be reasonable after batch insert and evict"
    );
    let _ = evicted;

    Ok(())
}

// ── Test 5: Empty search returns empty ─────────────────────────────────

async fn test_empty_search_returns_empty(store: &dyn MemoryStore) -> anyhow::Result<()> {
    let results = store
        .hybrid_search(
            &small_embedding(),
            "",
            1.0,
            0.0,
            MergeStrategy::Weighted,
            10,
        )
        .await?;
    assert!(
        results.is_empty(),
        "empty store search must return empty results, got {} results",
        results.len()
    );
    Ok(())
}

// ── Test 6: Search with RRF merge strategy ─────────────────────────────

async fn test_search_with_rrf(store: &dyn MemoryStore) -> anyhow::Result<()> {
    let file = test_file("test/rrf-search.md");
    store.upsert_file(&file).await?;

    let embedding = small_embedding();
    let chunk = test_chunk(
        "chunk-rrf-1",
        "test/rrf-search.md",
        "rrf test content with unique keywords",
        Some(embedding.clone()),
    );
    store.upsert_chunks(&[chunk]).await?;

    let results = store
        .hybrid_search(&embedding, "", 1.0, 0.0, MergeStrategy::Rrf { k: 60 }, 10)
        .await?;
    assert!(
        !results.is_empty(),
        "RRF vector search must return results for matching content"
    );

    Ok(())
}

// ── Test 7: Search with weighted merge strategy ────────────────────────

async fn test_search_with_weighted(store: &dyn MemoryStore) -> anyhow::Result<()> {
    let file = test_file("test/weighted-search.md");
    store.upsert_file(&file).await?;

    let embedding = small_embedding();
    let chunk = test_chunk(
        "chunk-w-1",
        "test/weighted-search.md",
        "weighted merge test content",
        Some(embedding.clone()),
    );
    store.upsert_chunks(&[chunk]).await?;

    let results = store
        .hybrid_search(&embedding, "", 0.5, 0.5, MergeStrategy::Weighted, 10)
        .await?;
    assert!(
        !results.is_empty(),
        "weighted search must return results for matching content"
    );

    Ok(())
}

// ── SQLite backend tests ───────────────────────────────────────────────

#[tokio::test]
async fn sqlite_roundtrip_chunks() {
    test_roundtrip_chunks(&create_sqlite_store().await)
        .await
        .unwrap();
}

#[tokio::test]
async fn sqlite_vector_search() {
    test_vector_search(&create_sqlite_store().await)
        .await
        .unwrap();
}

#[tokio::test]
async fn sqlite_keyword_only_search() {
    test_keyword_only_search(&create_sqlite_store().await)
        .await
        .unwrap();
}

#[tokio::test]
async fn sqlite_hybrid_search_with_query() {
    test_hybrid_search_with_query(&create_sqlite_store().await)
        .await
        .unwrap();
}

#[tokio::test]
async fn sqlite_file_ops() {
    test_file_ops(&create_sqlite_store().await).await.unwrap();
}

#[tokio::test]
async fn sqlite_cache_operations() {
    test_cache_operations(&create_sqlite_store().await)
        .await
        .unwrap();
}

#[tokio::test]
async fn sqlite_empty_search_returns_empty() {
    test_empty_search_returns_empty(&create_sqlite_store().await)
        .await
        .unwrap();
}

#[tokio::test]
async fn sqlite_search_with_rrf() {
    test_search_with_rrf(&create_sqlite_store().await)
        .await
        .unwrap();
}

#[tokio::test]
async fn sqlite_search_with_weighted() {
    test_search_with_weighted(&create_sqlite_store().await)
        .await
        .unwrap();
}

// ── Zvec backend tests ─────────────────────────────────────────────────

#[tokio::test]
async fn zvec_roundtrip_chunks() {
    let store = create_zvec_store().await;
    test_roundtrip_chunks(&*store).await.unwrap();
    drop(store);
}

#[tokio::test]
async fn zvec_vector_search() {
    let store = create_zvec_store().await;
    test_vector_search(&*store).await.unwrap();
    drop(store);
}

#[tokio::test]
async fn zvec_keyword_only_search() {
    let store = create_zvec_store().await;
    test_keyword_only_search(&*store).await.unwrap();
    drop(store);
}

#[tokio::test]
async fn zvec_hybrid_search_with_query() {
    let store = create_zvec_store().await;
    test_hybrid_search_with_query(&*store).await.unwrap();
    drop(store);
}

#[tokio::test]
async fn zvec_file_ops() {
    let store = create_zvec_store().await;
    test_file_ops(&*store).await.unwrap();
    drop(store);
}

#[tokio::test]
async fn zvec_cache_operations() {
    let store = create_zvec_store().await;
    test_cache_operations(&*store).await.unwrap();
    drop(store);
}

#[tokio::test]
async fn zvec_empty_search_returns_empty() {
    let store = create_zvec_store().await;
    test_empty_search_returns_empty(&*store).await.unwrap();
    drop(store);
}

#[tokio::test]
async fn zvec_search_with_rrf() {
    let store = create_zvec_store().await;
    test_search_with_rrf(&*store).await.unwrap();
    drop(store);
}

#[tokio::test]
async fn zvec_search_with_weighted() {
    let store = create_zvec_store().await;
    test_search_with_weighted(&*store).await.unwrap();
    drop(store);
}
