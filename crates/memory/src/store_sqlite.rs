/// SQLite implementation of the `MemoryStore` trait.
use async_trait::async_trait;
use sqlx::SqlitePool;

use crate::{
    schema::{ChunkRow, FileRow},
    search::SearchResult,
    store::MemoryStore,
};

pub struct SqliteMemoryStore {
    pool: SqlitePool,
}

impl SqliteMemoryStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

/// Deserialize a BLOB of little-endian f32s.
fn blob_to_vec(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Serialize a slice of f32s to a BLOB of little-endian bytes.
fn vec_to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Cosine similarity between two vectors.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 {
        0.0
    } else {
        dot / denom
    }
}

#[async_trait]
impl MemoryStore for SqliteMemoryStore {
    async fn upsert_file(&self, file: &FileRow) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO files (path, source, hash, mtime, size) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(path) DO UPDATE SET source=excluded.source, hash=excluded.hash, mtime=excluded.mtime, size=excluded.size",
        )
        .bind(&file.path)
        .bind(&file.source)
        .bind(&file.hash)
        .bind(file.mtime)
        .bind(file.size)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_file(&self, path: &str) -> anyhow::Result<Option<FileRow>> {
        let row: Option<(String, String, String, i64, i64)> =
            sqlx::query_as("SELECT path, source, hash, mtime, size FROM files WHERE path = ?")
                .bind(path)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(path, source, hash, mtime, size)| FileRow {
            path,
            source,
            hash,
            mtime,
            size,
        }))
    }

    async fn delete_file(&self, path: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM files WHERE path = ?")
            .bind(path)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_files(&self) -> anyhow::Result<Vec<FileRow>> {
        let rows: Vec<(String, String, String, i64, i64)> =
            sqlx::query_as("SELECT path, source, hash, mtime, size FROM files")
                .fetch_all(&self.pool)
                .await?;
        Ok(rows
            .into_iter()
            .map(|(path, source, hash, mtime, size)| FileRow {
                path,
                source,
                hash,
                mtime,
                size,
            })
            .collect())
    }

    async fn upsert_chunks(&self, chunks: &[ChunkRow]) -> anyhow::Result<()> {
        for chunk in chunks {
            let emb_blob = chunk.embedding.as_deref();
            sqlx::query(
                "INSERT INTO chunks (id, path, source, start_line, end_line, hash, model, text, embedding, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(id) DO UPDATE SET
                   path=excluded.path, source=excluded.source, start_line=excluded.start_line,
                   end_line=excluded.end_line, hash=excluded.hash, model=excluded.model,
                   text=excluded.text, embedding=excluded.embedding, updated_at=excluded.updated_at",
            )
            .bind(&chunk.id)
            .bind(&chunk.path)
            .bind(&chunk.source)
            .bind(chunk.start_line)
            .bind(chunk.end_line)
            .bind(&chunk.hash)
            .bind(&chunk.model)
            .bind(&chunk.text)
            .bind(emb_blob)
            .bind(&chunk.updated_at)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    async fn get_chunks_for_file(&self, path: &str) -> anyhow::Result<Vec<ChunkRow>> {
        let rows: Vec<(String, String, String, i64, i64, String, String, String, Option<Vec<u8>>, String)> =
            sqlx::query_as(
                "SELECT id, path, source, start_line, end_line, hash, model, text, embedding, updated_at FROM chunks WHERE path = ? ORDER BY start_line",
            )
            .bind(path)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    path,
                    source,
                    start_line,
                    end_line,
                    hash,
                    model,
                    text,
                    embedding,
                    updated_at,
                )| {
                    ChunkRow {
                        id,
                        path,
                        source,
                        start_line,
                        end_line,
                        hash,
                        model,
                        text,
                        embedding,
                        updated_at,
                    }
                },
            )
            .collect())
    }

    async fn delete_chunks_for_file(&self, path: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM chunks WHERE path = ?")
            .bind(path)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn get_chunk_by_id(&self, id: &str) -> anyhow::Result<Option<ChunkRow>> {
        let row: Option<(String, String, String, i64, i64, String, String, String, Option<Vec<u8>>, String)> =
            sqlx::query_as(
                "SELECT id, path, source, start_line, end_line, hash, model, text, embedding, updated_at FROM chunks WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(
            |(id, path, source, start_line, end_line, hash, model, text, embedding, updated_at)| {
                ChunkRow {
                    id,
                    path,
                    source,
                    start_line,
                    end_line,
                    hash,
                    model,
                    text,
                    embedding,
                    updated_at,
                }
            },
        ))
    }

    async fn get_cached_embedding(
        &self,
        provider: &str,
        model: &str,
        hash: &str,
    ) -> anyhow::Result<Option<Vec<f32>>> {
        let row: Option<(Vec<u8>,)> = sqlx::query_as(
            "SELECT embedding FROM embedding_cache WHERE provider = ? AND model = ? AND hash = ?",
        )
        .bind(provider)
        .bind(model)
        .bind(hash)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(blob,)| blob_to_vec(&blob)))
    }

    async fn put_cached_embedding(
        &self,
        provider: &str,
        model: &str,
        provider_key: &str,
        hash: &str,
        embedding: &[f32],
    ) -> anyhow::Result<()> {
        let blob = vec_to_blob(embedding);
        let dims = embedding.len() as i64;
        sqlx::query(
            "INSERT INTO embedding_cache (provider, model, provider_key, hash, embedding, dims)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(provider, model, provider_key, hash) DO UPDATE SET
               embedding=excluded.embedding, dims=excluded.dims, updated_at=datetime('now')",
        )
        .bind(provider)
        .bind(model)
        .bind(provider_key)
        .bind(hash)
        .bind(&blob)
        .bind(dims)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn count_cached_embeddings(&self) -> anyhow::Result<usize> {
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM embedding_cache")
            .fetch_one(&self.pool)
            .await?;
        Ok(count as usize)
    }

    async fn evict_embedding_cache(&self, keep: usize) -> anyhow::Result<usize> {
        let result = sqlx::query(
            "DELETE FROM embedding_cache WHERE rowid IN (
                SELECT rowid FROM embedding_cache ORDER BY updated_at ASC LIMIT MAX(0, (SELECT COUNT(*) FROM embedding_cache) - ?)
            )",
        )
        .bind(keep as i64)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() as usize)
    }

    async fn vector_search(
        &self,
        query_embedding: &[f32],
        limit: usize,
    ) -> anyhow::Result<Vec<SearchResult>> {
        // Load all chunks with embeddings and compute cosine similarity in-process
        let rows: Vec<(String, String, String, i64, i64, Option<Vec<u8>>)> = sqlx::query_as(
            "SELECT id, path, source, start_line, end_line, embedding FROM chunks WHERE embedding IS NOT NULL",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut scored: Vec<SearchResult> = rows
            .into_iter()
            .filter_map(|(id, path, source, start_line, end_line, emb)| {
                let emb = emb?;
                let vec = blob_to_vec(&emb);
                let score = cosine_similarity(query_embedding, &vec);
                Some(SearchResult {
                    chunk_id: id,
                    path,
                    source,
                    start_line,
                    end_line,
                    score,
                    text: String::new(), // filled by caller if needed
                })
            })
            .collect();

        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(limit);
        Ok(scored)
    }

    async fn keyword_search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<SearchResult>> {
        let rows: Vec<(String, String, String, i64, i64, f64)> = sqlx::query_as(
            "SELECT c.id, c.path, c.source, c.start_line, c.end_line, rank
             FROM chunks_fts f
             JOIN chunks c ON c.rowid = f.rowid
             WHERE chunks_fts MATCH ?
             ORDER BY rank
             LIMIT ?",
        )
        .bind(query)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        // FTS5 rank is negative (lower = better). Normalize to 0..1 range.
        let min_rank = rows.iter().map(|r| r.5).fold(f64::INFINITY, f64::min);
        let max_rank = rows.iter().map(|r| r.5).fold(f64::NEG_INFINITY, f64::max);
        let range = max_rank - min_rank;

        Ok(rows
            .into_iter()
            .map(|(id, path, source, start_line, end_line, rank)| {
                let score = if range.abs() < 1e-9 {
                    1.0
                } else {
                    // Invert: most negative rank → highest score
                    (1.0 - ((rank - min_rank) / range)) as f32
                };
                SearchResult {
                    chunk_id: id,
                    path,
                    source,
                    start_line,
                    end_line,
                    score,
                    text: String::new(),
                }
            })
            .collect())
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, crate::schema::run_migrations};

    async fn setup() -> SqliteMemoryStore {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        SqliteMemoryStore::new(pool)
    }

    #[tokio::test]
    async fn test_file_crud() {
        let store = setup().await;
        let file = FileRow {
            path: "memory/2024-01-01.md".into(),
            source: "daily".into(),
            hash: "abc123".into(),
            mtime: 1000,
            size: 500,
        };
        store.upsert_file(&file).await.unwrap();

        let got = store
            .get_file("memory/2024-01-01.md")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.hash, "abc123");

        assert!(store.get_file("nonexistent").await.unwrap().is_none());

        let files = store.list_files().await.unwrap();
        assert_eq!(files.len(), 1);

        store.delete_file("memory/2024-01-01.md").await.unwrap();
        assert!(
            store
                .get_file("memory/2024-01-01.md")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_chunk_crud() {
        let store = setup().await;
        let file = FileRow {
            path: "test.md".into(),
            source: "daily".into(),
            hash: "h".into(),
            mtime: 0,
            size: 0,
        };
        store.upsert_file(&file).await.unwrap();

        let emb = vec_to_blob(&[1.0, 0.0, 0.0]);
        let chunk = ChunkRow {
            id: "c1".into(),
            path: "test.md".into(),
            source: "daily".into(),
            start_line: 1,
            end_line: 10,
            hash: "ch".into(),
            model: "test-model".into(),
            text: "hello world".into(),
            embedding: Some(emb),
            updated_at: "2024-01-01".into(),
        };
        store.upsert_chunks(&[chunk]).await.unwrap();

        let chunks = store.get_chunks_for_file("test.md").await.unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "hello world");

        let got = store.get_chunk_by_id("c1").await.unwrap().unwrap();
        assert_eq!(got.start_line, 1);

        store.delete_chunks_for_file("test.md").await.unwrap();
        assert!(
            store
                .get_chunks_for_file("test.md")
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn test_embedding_cache() {
        let store = setup().await;
        let emb = vec![0.1, 0.2, 0.3];
        store
            .put_cached_embedding("openai", "text-embedding-3-small", "key1", "hash1", &emb)
            .await
            .unwrap();

        let cached = store
            .get_cached_embedding("openai", "text-embedding-3-small", "hash1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cached.len(), 3);
        assert!((cached[0] - 0.1).abs() < 1e-6);
    }

    #[tokio::test]
    async fn test_vector_search() {
        let store = setup().await;
        let file = FileRow {
            path: "t.md".into(),
            source: "s".into(),
            hash: "h".into(),
            mtime: 0,
            size: 0,
        };
        store.upsert_file(&file).await.unwrap();

        // Insert two chunks with different embeddings
        let c1 = ChunkRow {
            id: "c1".into(),
            path: "t.md".into(),
            source: "s".into(),
            start_line: 1,
            end_line: 5,
            hash: "h1".into(),
            model: "m".into(),
            text: "first".into(),
            embedding: Some(vec_to_blob(&[1.0, 0.0, 0.0])),
            updated_at: "now".into(),
        };
        let c2 = ChunkRow {
            id: "c2".into(),
            path: "t.md".into(),
            source: "s".into(),
            start_line: 6,
            end_line: 10,
            hash: "h2".into(),
            model: "m".into(),
            text: "second".into(),
            embedding: Some(vec_to_blob(&[0.0, 1.0, 0.0])),
            updated_at: "now".into(),
        };
        store.upsert_chunks(&[c1, c2]).await.unwrap();

        let results = store.vector_search(&[1.0, 0.0, 0.0], 2).await.unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].chunk_id, "c1");
        assert!((results[0].score - 1.0).abs() < 1e-6);
    }

    #[tokio::test]
    async fn test_keyword_search() {
        let store = setup().await;
        let file = FileRow {
            path: "t.md".into(),
            source: "s".into(),
            hash: "h".into(),
            mtime: 0,
            size: 0,
        };
        store.upsert_file(&file).await.unwrap();

        let c1 = ChunkRow {
            id: "c1".into(),
            path: "t.md".into(),
            source: "s".into(),
            start_line: 1,
            end_line: 5,
            hash: "h1".into(),
            model: "m".into(),
            text: "rust programming language".into(),
            embedding: None,
            updated_at: "now".into(),
        };
        let c2 = ChunkRow {
            id: "c2".into(),
            path: "t.md".into(),
            source: "s".into(),
            start_line: 6,
            end_line: 10,
            hash: "h2".into(),
            model: "m".into(),
            text: "python scripting language".into(),
            embedding: None,
            updated_at: "now".into(),
        };
        store.upsert_chunks(&[c1, c2]).await.unwrap();

        let results = store.keyword_search("rust", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].chunk_id, "c1");
    }

    #[test]
    fn test_cosine_similarity() {
        assert!((cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!((cosine_similarity(&[1.0, 0.0], &[0.0, 1.0])).abs() < 1e-6);
        assert!((cosine_similarity(&[1.0, 0.0], &[-1.0, 0.0]) + 1.0).abs() < 1e-6);
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
        assert_eq!(cosine_similarity(&[1.0], &[1.0, 2.0]), 0.0);
    }

    #[test]
    fn test_blob_roundtrip() {
        let v = vec![0.1f32, 0.2, 0.3, -0.5];
        let blob = vec_to_blob(&v);
        let back = blob_to_vec(&blob);
        for (a, b) in v.iter().zip(back.iter()) {
            assert!((a - b).abs() < 1e-7);
        }
    }
}
