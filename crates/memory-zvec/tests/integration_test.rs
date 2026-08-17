// Integration tests legitimately panic on unexpected errors via `unwrap`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Once;

use {
    moltis_memory::{
        schema::{ChunkRow, FileRow},
        store::{CacheEntry, MemoryStore, MergeStrategy},
    },
    moltis_memory_zvec::ZvecMemoryStore,
};

static ZVEC_INIT: Once = Once::new();

fn ensure_zvec_init() {
    ZVEC_INIT.call_once(|| {
        zvec::initialize(None).expect("zvec global init failed");
    });
}

fn distinct_embedding(seed: u8) -> Vec<f32> {
    let mut v = vec![0.0f32; 768];
    v[0] = seed as f32;
    v[1] = (seed.wrapping_mul(7)) as f32;
    v
}

fn chunk_row(id: &str, path: &str, text: &str, emb: &[f32]) -> ChunkRow {
    let emb_bytes = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
    ChunkRow {
        id: id.into(),
        path: path.into(),
        source: "test".into(),
        start_line: 1,
        end_line: 10,
        hash: format!("hash-{id}"),
        model: "test-model".into(),
        text: text.into(),
        embedding: Some(emb_bytes),
        updated_at: "2025-01-01T00:00:00Z".into(),
    }
}

#[tokio::test]
async fn test_full_pipeline() {
    ensure_zvec_init();

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("collection");
    let cache_path = dir.path().join("cache.redb");

    let collection = moltis_memory_zvec::open_or_create_collection(&db_path, Some(768)).unwrap();
    let cache = moltis_memory_zvec::RedbCache::new(&cache_path).unwrap();
    let store = ZvecMemoryStore::with_cache(collection, cache);

    // ── 1. Upsert file ──

    let file = FileRow {
        path: "test/doc.md".into(),
        source: "test".into(),
        hash: "filehash-123".into(),
        mtime: 1_700_000_000,
        size: 4096,
    };
    store.upsert_file(&file).await.unwrap();

    let fetched = store.get_file("test/doc.md").await.unwrap();
    assert!(fetched.is_some(), "upserted file must be retrievable");
    let f = fetched.unwrap();
    assert_eq!(f.path, "test/doc.md");
    assert_eq!(f.source, "test");
    assert_eq!(f.hash, "filehash-123");
    assert_eq!(f.mtime, 1_700_000_000);
    assert_eq!(f.size, 4096);

    // ── 2. Upsert chunks ──

    let emb_a = distinct_embedding(1);
    let emb_b = distinct_embedding(2);
    let emb_c = distinct_embedding(3);

    let chunks = vec![
        chunk_row(
            "int-chunk-1",
            "test/doc.md",
            "the quick brown fox jumps over the lazy dog",
            &emb_a,
        ),
        chunk_row(
            "int-chunk-2",
            "test/doc.md",
            "machine learning is transforming software engineering",
            &emb_b,
        ),
        chunk_row(
            "int-chunk-3",
            "test/doc.md",
            "deep neural networks for image recognition tasks",
            &emb_c,
        ),
    ];
    store.upsert_chunks(&chunks).await.unwrap();

    let c1 = store.get_chunk_by_id("int-chunk-1").await.unwrap();
    assert!(c1.is_some(), "chunk-1 must be retrievable by id");
    assert_eq!(
        c1.unwrap().text,
        "the quick brown fox jumps over the lazy dog"
    );

    let c2 = store.get_chunk_by_id("int-chunk-2").await.unwrap();
    assert!(c2.is_some(), "chunk-2 must be retrievable by id");
    assert_eq!(
        c2.unwrap().text,
        "machine learning is transforming software engineering"
    );

    let c3 = store.get_chunk_by_id("int-chunk-3").await.unwrap();
    assert!(c3.is_some(), "chunk-3 must be retrievable by id");
    assert_eq!(
        c3.unwrap().text,
        "deep neural networks for image recognition tasks"
    );

    // ── 3. get_chunks_for_file ──

    let file_chunks = store.get_chunks_for_file("test/doc.md").await.unwrap();
    assert_eq!(
        file_chunks.len(),
        3,
        "must return all 3 chunks for the file"
    );
    let ids: Vec<&str> = file_chunks.iter().map(|c| c.id.as_str()).collect();
    assert!(ids.contains(&"int-chunk-1"));
    assert!(ids.contains(&"int-chunk-2"));
    assert!(ids.contains(&"int-chunk-3"));

    // ── 4. Hybrid search (Weighted) ──

    let results_weighted = store
        .hybrid_search(&emb_a, "fox", 0.5, 0.5, MergeStrategy::Weighted, 10)
        .await
        .unwrap();
    assert!(
        !results_weighted.is_empty(),
        "weighted hybrid search must return results"
    );
    assert!(
        results_weighted.iter().any(|r| r.path == "test/doc.md"),
        "weighted results must reference the ingested file"
    );

    // ── 5. Hybrid search (RRF) ──

    let results_rrf = store
        .hybrid_search(
            &emb_a,
            "learning",
            0.5,
            0.5,
            MergeStrategy::Rrf { k: 60 },
            10,
        )
        .await
        .unwrap();
    assert!(
        !results_rrf.is_empty(),
        "RRF hybrid search must return results"
    );
    assert!(
        results_rrf.iter().any(|r| r.path == "test/doc.md"),
        "RRF results must reference the ingested file"
    );

    // ── 6. Cache put / get ──

    let cache_embedding = vec![0.1f32, 0.2, 0.3, -0.5, 0.8];
    store
        .put_cached_embedding(
            "openai",
            "text-model",
            "sk-key",
            "hash-c1",
            &cache_embedding,
        )
        .await
        .unwrap();

    let cached = store
        .get_cached_embedding("openai", "text-model", "hash-c1")
        .await
        .unwrap();
    assert!(cached.is_some(), "cached embedding must be retrievable");
    let cached_vec = cached.unwrap();
    assert_eq!(cached_vec.len(), cache_embedding.len());
    for (a, b) in cached_vec.iter().zip(cache_embedding.iter()) {
        assert!(
            (a - b).abs() < 1e-3,
            "cached embedding values must round-trip"
        );
    }

    let cached_missing = store
        .get_cached_embedding("no", "such", "key")
        .await
        .unwrap();
    assert!(
        cached_missing.is_none(),
        "missing cache key must return None"
    );

    // Also batch-insert a few entries via CacheEntry
    let batch_entries: Vec<CacheEntry<'_>> = vec![
        CacheEntry {
            provider: "p",
            model: "m",
            provider_key: "k",
            hash: "bh1",
            embedding: &[1.0, 2.0],
        },
        CacheEntry {
            provider: "p",
            model: "m",
            provider_key: "k",
            hash: "bh2",
            embedding: &[3.0, 4.0],
        },
    ];
    store
        .put_cached_embeddings_batch(&batch_entries)
        .await
        .unwrap();

    // ── 7. Cache evict ──

    let count_before_evict = store.count_cached_embeddings().await.unwrap();
    assert!(
        count_before_evict >= 3,
        "expected at least 3 cache entries (1 single + 2 batch), got {count_before_evict}"
    );

    let evicted = store.evict_embedding_cache(0).await.unwrap();
    assert_eq!(
        evicted, count_before_evict,
        "evict(0) must remove all entries"
    );

    let count_after_evict = store.count_cached_embeddings().await.unwrap();
    assert_eq!(count_after_evict, 0, "cache must be empty after evict(0)");

    // ── 8. Delete chunks for file ──

    store.delete_chunks_for_file("test/doc.md").await.unwrap();

    let after_delete = store.get_chunks_for_file("test/doc.md").await.unwrap();
    assert!(
        after_delete.is_empty(),
        "chunks must be empty after delete_chunks_for_file"
    );

    // ── 9. Teardown ──

    drop(store);
}

/// Regression test for durability (issue: file metadata and chunk-by-path
/// lookups previously lived in process-local HashMaps and were lost on restart).
/// Writes through one store, drops it, reopens at the same paths, and verifies
/// the data survived.
#[tokio::test]
async fn test_store_survives_restart() {
    use moltis_memory::store::MemoryStore;

    ensure_zvec_init();

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("collection");
    let cache_path = dir.path().join("cache.redb");

    let file = FileRow {
        path: "restart/doc.md".into(),
        source: "longterm".into(),
        hash: "h-restart".into(),
        mtime: 42,
        size: 7,
    };
    let emb = distinct_embedding(3);
    let chunk = chunk_row(
        "restart/doc.md:0",
        "restart/doc.md",
        "durable content",
        &emb,
    );

    // Phase 1: write, then fully drop the store (closes collection + redb).
    {
        let collection =
            moltis_memory_zvec::open_or_create_collection(&db_path, Some(768)).unwrap();
        let cache = moltis_memory_zvec::RedbCache::new(&cache_path).unwrap();
        let store = ZvecMemoryStore::with_cache(collection, cache);
        store.upsert_file(&file).await.unwrap();
        store
            .upsert_chunks(std::slice::from_ref(&chunk))
            .await
            .unwrap();
        drop(store);
    }

    // Phase 2: reopen at the same paths — data must persist.
    {
        let collection =
            moltis_memory_zvec::open_or_create_collection(&db_path, Some(768)).unwrap();
        let cache = moltis_memory_zvec::RedbCache::new(&cache_path).unwrap();
        let store = ZvecMemoryStore::with_cache(collection, cache);

        let files = store.list_files().await.unwrap();
        assert!(
            files.iter().any(|f| f.path == "restart/doc.md"),
            "file must persist across restart, got {files:?}"
        );

        let fetched = store.get_file("restart/doc.md").await.unwrap().unwrap();
        assert_eq!(fetched.source, "longterm", "file source must round-trip");
        assert_eq!(fetched.hash, "h-restart");

        let chunks = store.get_chunks_for_file("restart/doc.md").await.unwrap();
        assert_eq!(chunks.len(), 1, "chunk must persist across restart");
        assert_eq!(chunks[0].text, "durable content");

        let by_id = store.get_chunk_by_id("restart/doc.md:0").await.unwrap();
        assert!(
            by_id.is_some(),
            "chunk must be fetchable by id after restart"
        );
    }
}
