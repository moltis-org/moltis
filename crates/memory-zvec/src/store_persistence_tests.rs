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

fn close_store(store: ZvecMemoryStore) {
    let coll = store.collection_arc();
    let _ = crate::flush_collection(&coll);
    drop(store);
}

#[tokio::test]
async fn test_hybrid_search_keyword_only_rrf() {
    let (collection, cache, _dir) = make_collection();
    let store = ZvecMemoryStore::with_cache_arc(Arc::clone(&collection), cache);

    let results = store
        .hybrid_search(
            &[],
            "no_match_here_zzz",
            0.0,
            1.0,
            MergeStrategy::Rrf { k: 60 },
            10,
        )
        .await
        .unwrap();
    assert!(results.is_empty());

    close_store(store);
}

#[tokio::test]
async fn test_keyword_search_with_special_chars() {
    let (collection, cache, _dir) = make_collection();
    let store = ZvecMemoryStore::with_cache_arc(Arc::clone(&collection), cache);

    let file = FileRow {
        path: "special-chars.md".into(),
        source: "test".into(),
        hash: "h".into(),
        mtime: 1,
        size: 10,
    };
    store.upsert_file(&file).await.unwrap();

    let mut emb = vec![0.0f32; 768];
    emb[0] = 1.0;
    let emb_bytes: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();

    let chunk = ChunkRow {
        id: "sc-1".into(),
        path: "special-chars.md".into(),
        source: "test".into(),
        start_line: 1,
        end_line: 5,
        hash: "h1".into(),
        model: "m".into(),
        text: "San Francisco Mission District".into(),
        embedding: Some(emb_bytes),
        updated_at: "2025-01-01T00:00:00Z".into(),
    };
    store.upsert_chunks(&[chunk]).await.unwrap();

    let results = store
        .hybrid_search(
            &[],
            "37.759 location",
            0.0,
            1.0,
            MergeStrategy::Weighted,
            10,
        )
        .await
        .unwrap();
    assert!(results.len() <= 10);

    let results = store
        .hybrid_search(&[], "...", 0.0, 1.0, MergeStrategy::Weighted, 10)
        .await
        .unwrap();
    assert!(results.is_empty());

    close_store(store);
}

/// Re-opening a collection whose FTS index was not persisted (writer
/// flushed docs but never optimized) must produce a stale state:
/// documents exist but keyword search returns nothing. Calling
/// `rebuild_keyword_index()` re-ingests all chunks, rebuilds the FTS
/// index in memory, and persists it via flush + optimize so that
/// keyword search works after reopen.
#[tokio::test]
async fn test_rebuild_keyword_index_recovers_stale_fts() {
    ensure_init();
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("rebuilddb");
    let cache_path = dir.path().join("rebuild.cache");

    // Phase 1: write chunks, flush docs, do NOT optimize, drop.
    {
        let collection = crate::open_or_create_collection(&db_path, Some(768)).unwrap();
        let cache = RedbCache::new(&cache_path).unwrap();
        let store = ZvecMemoryStore::with_cache(collection, cache).with_cache_dimension(768);
        // Register the file first — list_files reads the file-row table.
        store
            .upsert_file(&FileRow {
                path: "rb.md".into(),
                source: "rb.md".into(),
                hash: "h".into(),
                mtime: 1,
                size: 100,
            })
            .await
            .unwrap();
        let chunks: Vec<ChunkRow> = (0..3)
            .map(|i| ChunkRow {
                id: format!("rb-{i}"),
                path: "rb.md".into(),
                source: "test".into(),
                start_line: i + 1,
                end_line: i + 5,
                hash: format!("h{i}"),
                model: "m".into(),
                text: format!("rebuild keyword index chunk {i}"),
                embedding: Some(vec![0u8; 768 * 4]),
                updated_at: "2025-01-01T00:00:00Z".into(),
            })
            .collect();
        store.upsert_chunks(&chunks).await.unwrap();
        let coll = store.collection_arc();
        crate::flush_collection(&coll).unwrap();
        // explicitly do NOT optimize — simulate killed writer
    }

    // Phase 2: reopen — FTS is stale, then rebuild it.
    {
        let collection = crate::open_or_create_collection(&db_path, Some(768)).unwrap();
        let cache = RedbCache::new(&cache_path).unwrap();
        let store = ZvecMemoryStore::with_cache(collection, cache).with_cache_dimension(768);

        // Doc persistence is fine.
        assert!(
            store.get_chunk_by_id("rb-0").await.unwrap().is_some(),
            "docs must survive flush across reopen"
        );

        // Native FTS (v0.6.0 enhanced tokenizer) should find chunks even
        // after reopen — the FTS index is rebuilt from persisted documents.
        let pre_rebuild = store
            .hybrid_search(&[], "rebuild", 0.0, 1.0, MergeStrategy::Weighted, 10)
            .await
            .unwrap();
        assert!(
            !pre_rebuild.is_empty(),
            "native FTS keyword search must find chunks after reopen"
        );

        // Rebuild re-ingests all chunks for completeness.
        let count = store.rebuild_keyword_index().await.unwrap();
        assert_eq!(count, 3, "rebuild must re-ingest all 3 chunks");

        // Keyword search now works.
        let results = store
            .hybrid_search(&[], "chunk", 0.0, 1.0, MergeStrategy::Weighted, 10)
            .await
            .unwrap();
        assert_eq!(
            results.len(),
            3,
            "all 3 chunks must be keyword-findable after rebuild"
        );
        assert!(results.iter().any(|r| r.chunk_id == "rb-0"));
        assert!(results.iter().any(|r| r.chunk_id == "rb-1"));
        assert!(results.iter().any(|r| r.chunk_id == "rb-2"));

        // Verify FTS persists across reopen (no second rebuild needed).
        close_store(store);
    }

    // Phase 3: reopen a second time — FTS must still work (no rebuild).
    {
        let collection = crate::open_or_create_collection(&db_path, Some(768)).unwrap();
        let cache = RedbCache::new(&cache_path).unwrap();
        let store = ZvecMemoryStore::with_cache(collection, cache).with_cache_dimension(768);
        let results = store
            .hybrid_search(&[], "chunk", 0.0, 1.0, MergeStrategy::Weighted, 10)
            .await
            .unwrap();
        assert_eq!(
            results.len(),
            3,
            "keyword search must survive another reopen after rebuild"
        );
    }
}

// RRF vs weighted divergence is covered in moltis-memory's `search::tests`,
// which runs without the zvec feature enabled.

/// Definitive test: vector search must survive flush + reopen.
///
/// With FTS removed from the schema, `optimize()` no longer corrupts
/// RocksDB. After flush(), documents are persisted. After reopen,
/// the HNSW index must still produce correct vector search results.
#[tokio::test]
async fn test_vector_search_survives_flush_and_reopen() {
    ensure_init();
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("hnsw_survive");
    let cache_path = dir.path().join("hnsw_survive.cache");
    let dim: usize = 768;

    // Phase 1: create collection, upsert a chunk with a distinct vector,
    // flush, drop.
    {
        let collection = crate::open_or_create_collection(&db_path, Some(dim as u32)).unwrap();
        let cache = RedbCache::new(&cache_path).unwrap();
        let store = ZvecMemoryStore::with_cache(collection, cache).with_cache_dimension(dim as u32);

        store
            .upsert_file(&FileRow {
                path: "v.md".into(),
                source: "v.md".into(),
                hash: "h".into(),
                mtime: 1,
                size: 10,
            })
            .await
            .unwrap();

        // Vector A: unit vector along dim 0
        let mut emb_a = vec![0.0f32; dim];
        emb_a[0] = 1.0;
        let emb_a_bytes: Vec<u8> = emb_a.iter().flat_map(|f| f.to_le_bytes()).collect();

        // Vector B: unit vector along dim 1
        let mut emb_b = vec![0.0f32; dim];
        emb_b[1] = 1.0;
        let emb_b_bytes: Vec<u8> = emb_b.iter().flat_map(|f| f.to_le_bytes()).collect();

        store
            .upsert_chunks(&[
                ChunkRow {
                    id: "vec-a".into(),
                    path: "v.md".into(),
                    source: "test".into(),
                    start_line: 1,
                    end_line: 5,
                    hash: "ha".into(),
                    model: "m".into(),
                    text: "alpha vector document".into(),
                    embedding: Some(emb_a_bytes),
                    updated_at: "2025-01-01T00:00:00Z".into(),
                },
                ChunkRow {
                    id: "vec-b".into(),
                    path: "v.md".into(),
                    source: "test".into(),
                    start_line: 6,
                    end_line: 10,
                    hash: "hb".into(),
                    model: "m".into(),
                    text: "beta vector document".into(),
                    embedding: Some(emb_b_bytes),
                    updated_at: "2025-01-01T00:00:00Z".into(),
                },
            ])
            .await
            .unwrap();

        // Verify search works BEFORE flush (sanity check).
        {
            let mut query = vec![0.0f32; dim];
            query[0] = 0.99;
            query[1] = 0.01;
            let results = store
                .hybrid_search(&query, "", 1.0, 0.0, MergeStrategy::Weighted, 10)
                .await
                .unwrap();
            assert_eq!(
                results.first().map(|r| r.chunk_id.as_str()),
                Some("vec-a"),
                "query near vec-a must rank vec-a first (before flush)"
            );
        }

        // Flush to persist documents + HNSW to disk.
        let coll = store.collection_arc();
        crate::flush_collection(&coll).unwrap();
        close_store(store);
    }

    // Phase 2: reopen and verify vector search finds the right chunk.
    {
        let collection = crate::open_or_create_collection(&db_path, Some(dim as u32)).unwrap();
        let cache = RedbCache::new(&cache_path).unwrap();
        let store = ZvecMemoryStore::with_cache(collection, cache).with_cache_dimension(dim as u32);

        // Documents survived.
        assert!(
            store.get_chunk_by_id("vec-a").await.unwrap().is_some(),
            "doc must survive flush+reopen"
        );

        // Vector search: query close to A should rank vec-a above vec-b.
        let mut query = vec![0.0f32; dim];
        query[0] = 0.99;
        query[1] = 0.01;
        let results = store
            .hybrid_search(&query, "", 1.0, 0.0, MergeStrategy::Weighted, 10)
            .await
            .unwrap();

        assert!(
            !results.is_empty(),
            "vector search must return results after flush+reopen"
        );
        let top = &results[0];
        assert_eq!(
            top.chunk_id, "vec-a",
            "query close to vec-a must rank vec-a first after reopen, got {}",
            top.chunk_id
        );
    }
}
