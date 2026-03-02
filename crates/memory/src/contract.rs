//! Contract tests for the [`MemoryStore`] trait.
//!
//! These functions validate that any `MemoryStore` implementation satisfies
//! the CRUD and search invariants required by the memory system.

#![allow(clippy::unwrap_used)]

use crate::{
    schema::{ChunkRow, FileRow},
    store::MemoryStore,
};

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
    let embedding_bytes =
        embedding.map(|e| e.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>());
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

/// Ingest a document and search for its content — must return at least one result.
pub async fn ingest_then_search_returns_result(store: &dyn MemoryStore) -> anyhow::Result<()> {
    let file = test_file("test/hello.md");
    store.upsert_file(&file).await?;

    // Use a simple embedding vector for vector search.
    let embedding = vec![1.0_f32, 0.0, 0.0, 0.0];
    let chunk = test_chunk(
        "chunk-1",
        "test/hello.md",
        "the quick brown fox",
        Some(embedding),
    );
    store.upsert_chunks(&[chunk]).await?;

    let results = store.keyword_search("quick brown fox", 10).await?;
    assert!(
        !results.is_empty(),
        "keyword search must find ingested content"
    );
    assert!(
        results.iter().any(|r| r.path == "test/hello.md"),
        "result must reference the ingested file path"
    );
    Ok(())
}

/// Delete a file's chunks, then search must return nothing for that content.
pub async fn delete_removes_from_search(store: &dyn MemoryStore) -> anyhow::Result<()> {
    let file = test_file("test/delete-me.md");
    store.upsert_file(&file).await?;

    let embedding = vec![0.0_f32, 1.0, 0.0, 0.0];
    let chunk = test_chunk(
        "chunk-del-1",
        "test/delete-me.md",
        "unique_deletable_content_xyz",
        Some(embedding),
    );
    store.upsert_chunks(&[chunk]).await?;

    // Verify it's searchable first.
    let before = store
        .keyword_search("unique_deletable_content_xyz", 10)
        .await?;
    assert!(
        !before.is_empty(),
        "content must be searchable before delete"
    );

    // Delete and verify removal.
    store.delete_chunks_for_file("test/delete-me.md").await?;
    let after = store
        .keyword_search("unique_deletable_content_xyz", 10)
        .await?;
    assert!(
        after.is_empty(),
        "deleted content must not appear in search results"
    );
    Ok(())
}

/// FTS keyword search finds an exact keyword match.
pub async fn keyword_search_finds_exact_match(store: &dyn MemoryStore) -> anyhow::Result<()> {
    let file = test_file("test/keyword.md");
    store.upsert_file(&file).await?;

    let chunk = test_chunk(
        "chunk-kw-1",
        "test/keyword.md",
        "supercalifragilistic documentation",
        None,
    );
    store.upsert_chunks(&[chunk]).await?;

    let results = store.keyword_search("supercalifragilistic", 10).await?;
    assert!(
        !results.is_empty(),
        "keyword search must find exact keyword match"
    );
    Ok(())
}

/// Searching an empty store must return empty results (not error).
pub async fn empty_search_returns_empty(store: &dyn MemoryStore) -> anyhow::Result<()> {
    let results = store
        .keyword_search("nonexistent_content_that_cannot_match", 10)
        .await?;
    assert!(
        results.is_empty(),
        "empty store search must return empty results, got {} results",
        results.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{schema::run_migrations, store_sqlite::SqliteMemoryStore},
    };

    async fn test_store() -> SqliteMemoryStore {
        let pool = sqlx::SqlitePool::connect(":memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        SqliteMemoryStore::new(pool)
    }

    #[tokio::test]
    async fn contract_ingest_then_search() {
        let store = test_store().await;
        ingest_then_search_returns_result(&store).await.unwrap();
    }

    #[tokio::test]
    async fn contract_delete_removes_from_search() {
        let store = test_store().await;
        delete_removes_from_search(&store).await.unwrap();
    }

    #[tokio::test]
    async fn contract_keyword_search_finds_exact_match() {
        let store = test_store().await;
        keyword_search_finds_exact_match(&store).await.unwrap();
    }

    #[tokio::test]
    async fn contract_empty_search_returns_empty() {
        let store = test_store().await;
        empty_search_returns_empty(&store).await.unwrap();
    }
}
