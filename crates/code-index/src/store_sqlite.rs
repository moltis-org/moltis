//! SQLite + FTS5 storage backend for code-index.
//!
//! Stores chunks with quantized i8 embeddings and uses FTS5 for keyword search.
//! Vector similarity is done in-memory (brute-force) following the memory system pattern.

use std::path::Path;

use {
    async_trait::async_trait,
    sqlx::{Acquire, Row, SqlitePool, sqlite::SqliteConnectOptions},
};

use crate::{
    error::{Error, Result},
    store::{CodeChunk, CodeIndexStore, dequantize, quantize},
    types::SearchResult,
};

/// Run database migrations for the code-index SQLite backend.
pub async fn run_migrations(pool: &SqlitePool) -> Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(|e| Error::IndexStore(format!("code-index migration failed: {e}")))
}

/// Escape a user query for safe use as an FTS5 MATCH expression.
///
/// FTS5 has its own query language (supports AND, OR, NOT, NEAR, phrase literals).
/// Passing raw input causes parse errors on unbalanced quotes or boolean operators.
/// We quote the entire query as a phrase literal, escaping internal double-quotes.
fn escape_fts5_query(query: &str) -> String {
    let escaped = query.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod fts5_tests {
    use super::escape_fts5_query;

    #[test]
    fn normal_query() {
        assert_eq!(escape_fts5_query("hello world"), "\"hello world\"");
    }

    #[test]
    fn query_with_quotes() {
        assert_eq!(
            escape_fts5_query("find the \"auth\" middleware"),
            "\"find the \"\"auth\"\" middleware\""
        );
    }

    #[test]
    fn query_with_boolean_ops() {
        // Quoted as phrase literal — operators become literal text
        assert_eq!(
            escape_fts5_query("hello AND world OR NOT"),
            "\"hello AND world OR NOT\""
        );
    }

    #[test]
    fn empty_query() {
        assert_eq!(escape_fts5_query(""), "\"\"");
    }

    #[test]
    fn query_with_backslash() {
        // Backslashes pass through — FTS5 phrase literals do not escape them.
        let input = r"path	oile";
        let expected = r#""path	oile""#;
        assert_eq!(escape_fts5_query(input), expected);
    }
}

/// SQLite-backed code index store.
pub struct SqliteCodeIndexStore {
    pool: SqlitePool,
}

impl SqliteCodeIndexStore {
    /// Create a new SQLite store at the given path.
    pub async fn new(db_path: &Path) -> Result<Self> {
        // Ensure parent directory exists — SQLite only creates the file, not directories.
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                Error::IndexStore(format!(
                    "failed to create code-index directory {}: {e}",
                    parent.display()
                ))
            })?;
        }

        let options = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true);

        let pool = SqlitePool::connect_with(options)
            .await
            .map_err(|e| Error::IndexStore(format!("failed to connect to SQLite: {e}")))?;

        let store = Self { pool };
        store.initialize().await?;
        Ok(store)
    }

    /// Create a store from an existing pool (for testing).
    #[cfg(test)]
    pub(crate) async fn from_pool(pool: SqlitePool) -> Result<Self> {
        let store = Self { pool };
        store.initialize().await?;
        Ok(store)
    }

    /// Get a connection from the pool.
    async fn conn(&self) -> Result<sqlx::pool::PoolConnection<sqlx::Sqlite>> {
        self.pool
            .acquire()
            .await
            .map_err(|e| Error::IndexStore(format!("failed to acquire connection: {e}")))
    }
}

#[async_trait]
impl CodeIndexStore for SqliteCodeIndexStore {
    async fn initialize(&self) -> Result<()> {
        run_migrations(&self.pool).await
    }

    async fn upsert_chunks(
        &self,
        project_id: &str,
        file_path: &str,
        chunks: &[CodeChunk],
    ) -> Result<()> {
        let mut conn = self.conn().await?;

        // Use a transaction for batch insert
        let mut tx = conn
            .begin()
            .await
            .map_err(|e| Error::IndexStore(format!("failed to begin transaction: {e}")))?;

        // Delete existing chunks for this file first (full reindex of file)
        sqlx::query("DELETE FROM code_chunks WHERE project_id = ? AND file_path = ?")
            .bind(project_id)
            .bind(file_path)
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::IndexStore(format!("failed to delete existing chunks: {e}")))?;

        // Insert new chunks
        for chunk in chunks {
            let embedding_bytes = chunk
                .embedding
                .as_ref()
                .map(|e| quantize(e))
                .map(|q| q.into_iter().map(|v| v as u8).collect::<Vec<u8>>());

            sqlx::query(
                r#"
                INSERT INTO code_chunks
                    (project_id, file_path, chunk_index, content, embedding, start_line, end_line)
                VALUES
                    (?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(project_id)
            .bind(&chunk.file_path)
            .bind(chunk.chunk_index as i64)
            .bind(&chunk.content)
            .bind(embedding_bytes)
            .bind(chunk.start_line as i64)
            .bind(chunk.end_line as i64)
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::IndexStore(format!("failed to insert chunk: {e}")))?;
        }

        tx.commit()
            .await
            .map_err(|e| Error::IndexStore(format!("failed to commit transaction: {e}")))?;

        Ok(())
    }

    async fn delete_file_chunks(&self, project_id: &str, file_path: &str) -> Result<()> {
        let mut conn = self.conn().await?;

        sqlx::query("DELETE FROM code_chunks WHERE project_id = ? AND file_path = ?")
            .bind(project_id)
            .bind(file_path)
            .execute(&mut *conn)
            .await
            .map_err(|e| Error::IndexStore(format!("failed to delete file chunks: {e}")))?;

        Ok(())
    }

    async fn clear_project(&self, project_id: &str) -> Result<()> {
        let mut conn = self.conn().await?;

        sqlx::query("DELETE FROM code_chunks WHERE project_id = ?")
            .bind(project_id)
            .execute(&mut *conn)
            .await
            .map_err(|e| Error::IndexStore(format!("failed to clear project: {e}")))?;

        Ok(())
    }

    async fn get_project_chunks(&self, project_id: &str) -> Result<Vec<CodeChunk>> {
        let mut conn = self.conn().await?;

        let rows = sqlx::query(
            r#"
            SELECT
                file_path,
                chunk_index,
                content,
                embedding,
                start_line,
                end_line
            FROM code_chunks
            WHERE project_id = ?
            ORDER BY file_path, chunk_index
            "#,
        )
        .bind(project_id)
        .fetch_all(&mut *conn)
        .await
        .map_err(|e| Error::IndexStore(format!("failed to fetch chunks: {e}")))?;

        let mut chunks = Vec::with_capacity(rows.len());
        for row in rows {
            let embedding_bytes: Option<Vec<u8>> = row.get("embedding");
            let embedding = embedding_bytes.map(|bytes| {
                let i8_vec: Vec<i8> = bytes.into_iter().map(|b| b as i8).collect();
                dequantize(&i8_vec)
            });

            chunks.push(CodeChunk {
                file_path: row.get("file_path"),
                chunk_index: row.get::<i64, _>("chunk_index") as usize,
                content: row.get("content"),
                embedding,
                start_line: row.get::<i64, _>("start_line") as usize,
                end_line: row.get::<i64, _>("end_line") as usize,
            });
        }

        Ok(chunks)
    }

    async fn search_keyword(
        &self,
        project_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let mut conn = self.conn().await?;

        // FTS5 query with project filter via JOIN.
        // Query is escaped as a phrase literal to prevent FTS5 parse errors
        // on unbalanced quotes or boolean operators from LLM-generated input.
        let safe_query = escape_fts5_query(query);
        let rows = sqlx::query(
            r#"
            SELECT
                c.file_path,
                c.content,
                c.start_line,
                c.end_line
            FROM code_chunks c
            JOIN code_chunks_fts fts ON c.id = fts.rowid
            WHERE c.project_id = ?
                AND code_chunks_fts MATCH ?
            ORDER BY fts.rank
            LIMIT ?
            "#,
        )
        .bind(project_id)
        .bind(&safe_query)
        .bind(limit as i64)
        .fetch_all(&mut *conn)
        .await
        .map_err(|e| Error::IndexStore(format!("keyword search failed: {e}")))?;

        let results = rows
            .into_iter()
            .enumerate()
            .map(|(rank, row)| {
                let file_path: String = row.get("file_path");
                let start_line = row.get::<i64, _>("start_line") as usize;
                SearchResult {
                    chunk_id: format!("{}:{}:{}", project_id, file_path, start_line),
                    path: file_path,
                    text: row.get("content"),
                    start_line,
                    end_line: row.get::<i64, _>("end_line") as usize,
                    score: 1.0 / (1.0 + rank as f32), // Map rank to (0, 1] range, higher is better
                    source: "builtin".to_string(),
                }
            })
            .collect();

        Ok(results)
    }

    async fn file_count(&self, project_id: &str) -> Result<usize> {
        let mut conn = self.conn().await?;

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(DISTINCT file_path) FROM code_chunks WHERE project_id = ?",
        )
        .bind(project_id)
        .fetch_one(&mut *conn)
        .await
        .map_err(|e| Error::IndexStore(format!("failed to count files: {e}")))?;

        Ok(count as usize)
    }

    async fn chunk_count(&self, project_id: &str) -> Result<usize> {
        let mut conn = self.conn().await?;

        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM code_chunks WHERE project_id = ?")
                .bind(project_id)
                .fetch_one(&mut *conn)
                .await
                .map_err(|e| Error::IndexStore(format!("failed to count chunks: {e}")))?;

        Ok(count as usize)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn make_chunk(
        file_path: &str,
        index: usize,
        content: &str,
        start: usize,
        end: usize,
    ) -> CodeChunk {
        CodeChunk {
            file_path: file_path.to_string(),
            chunk_index: index,
            content: content.to_string(),
            embedding: None,
            start_line: start,
            end_line: end,
        }
    }

    async fn setup() -> SqliteCodeIndexStore {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        SqliteCodeIndexStore::from_pool(pool).await.unwrap()
    }

    #[tokio::test]
    async fn test_initialize_idempotent() {
        let store = setup().await;
        // initialize() was already called in setup(), call again
        store.initialize().await.unwrap();
        store.initialize().await.unwrap();
    }

    #[tokio::test]
    async fn test_upsert_and_get_chunks() {
        let store = setup().await;
        let chunks = vec![
            make_chunk("foo.rs", 0, "fn main() {}", 1, 1),
            make_chunk("foo.rs", 1, "fn helper() {}", 2, 2),
        ];
        store
            .upsert_chunks("proj", "foo.rs", &chunks)
            .await
            .unwrap();

        let got = store.get_project_chunks("proj").await.unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].file_path, "foo.rs");
        assert_eq!(got[0].chunk_index, 0);
        assert_eq!(got[0].content, "fn main() {}");
        assert_eq!(got[0].start_line, 1);
        assert_eq!(got[1].content, "fn helper() {}");
    }

    #[tokio::test]
    async fn test_upsert_replaces_file() {
        let store = setup().await;

        let v1 = vec![make_chunk("a.rs", 0, "version 1", 1, 1)];
        store.upsert_chunks("proj", "a.rs", &v1).await.unwrap();

        let v2 = vec![
            make_chunk("a.rs", 0, "version 2 line 1", 1, 1),
            make_chunk("a.rs", 1, "version 2 line 2", 2, 2),
        ];
        store.upsert_chunks("proj", "a.rs", &v2).await.unwrap();

        let got = store.get_project_chunks("proj").await.unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].content, "version 2 line 1");
        assert_eq!(got[1].content, "version 2 line 2");
    }

    #[tokio::test]
    async fn test_multi_file_isolation() {
        let store = setup().await;
        let chunks_a = vec![make_chunk("a.rs", 0, "file a", 1, 1)];
        let chunks_b = vec![make_chunk("b.rs", 0, "file b", 1, 1)];
        store
            .upsert_chunks("proj", "a.rs", &chunks_a)
            .await
            .unwrap();
        store
            .upsert_chunks("proj", "b.rs", &chunks_b)
            .await
            .unwrap();

        let got = store.get_project_chunks("proj").await.unwrap();
        assert_eq!(got.len(), 2);

        store.delete_file_chunks("proj", "a.rs").await.unwrap();
        let got = store.get_project_chunks("proj").await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].file_path, "b.rs");
    }

    #[tokio::test]
    async fn test_multi_project_isolation() {
        let store = setup().await;
        let chunks = vec![make_chunk("shared.rs", 0, "content", 1, 1)];
        store
            .upsert_chunks("X", "shared.rs", &chunks)
            .await
            .unwrap();
        store
            .upsert_chunks("Y", "shared.rs", &chunks)
            .await
            .unwrap();

        let got_x = store.get_project_chunks("X").await.unwrap();
        assert_eq!(got_x.len(), 1);

        let got_y = store.get_project_chunks("Y").await.unwrap();
        assert_eq!(got_y.len(), 1);
    }

    #[tokio::test]
    async fn test_delete_file_chunks() {
        let store = setup().await;
        store
            .upsert_chunks("proj", "a.rs", &[make_chunk("a.rs", 0, "a", 1, 1)])
            .await
            .unwrap();
        store
            .upsert_chunks("proj", "b.rs", &[make_chunk("b.rs", 0, "b", 1, 1)])
            .await
            .unwrap();

        store.delete_file_chunks("proj", "a.rs").await.unwrap();

        let got = store.get_project_chunks("proj").await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].file_path, "b.rs");
    }

    #[tokio::test]
    async fn test_clear_project() {
        let store = setup().await;
        let chunks = vec![make_chunk("f.rs", 0, "data", 1, 1)];
        store.upsert_chunks("X", "f.rs", &chunks).await.unwrap();
        store.upsert_chunks("Y", "f.rs", &chunks).await.unwrap();

        store.clear_project("X").await.unwrap();

        assert!(store.get_project_chunks("X").await.unwrap().is_empty());
        assert_eq!(store.get_project_chunks("Y").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_embedding_roundtrip() {
        let store = setup().await;
        let original = vec![0.5, -0.3, 0.8, 0.1];
        let mut chunk = make_chunk("emb.rs", 0, "embedded code", 1, 5);
        chunk.embedding = Some(original.clone());

        store
            .upsert_chunks("proj", "emb.rs", &[chunk])
            .await
            .unwrap();

        let got = store.get_project_chunks("proj").await.unwrap();
        assert_eq!(got.len(), 1);
        let emb = got[0].embedding.as_ref().unwrap();
        // i8 quantization loses precision — tolerance of ~0.02
        for (orig, stored) in original.iter().zip(emb.iter()) {
            assert!(
                (orig - stored).abs() < 0.02,
                "original {orig} vs stored {stored}"
            );
        }
    }

    #[tokio::test]
    async fn test_keyword_search_basic() {
        let store = setup().await;
        store
            .upsert_chunks("proj", "main.rs", &[make_chunk(
                "main.rs",
                0,
                "fn main() { println!(\"hello\"); }",
                1,
                1,
            )])
            .await
            .unwrap();
        store
            .upsert_chunks("proj", "greet.py", &[make_chunk(
                "greet.py",
                0,
                "def greet(): pass",
                1,
                1,
            )])
            .await
            .unwrap();

        let results = store.search_keyword("proj", "main", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].path.contains("main.rs"));
    }

    #[tokio::test]
    async fn test_keyword_search_multi_result() {
        let store = setup().await;
        store
            .upsert_chunks("proj", "a.rs", &[make_chunk(
                "a.rs",
                0,
                "database connection pool",
                1,
                1,
            )])
            .await
            .unwrap();
        store
            .upsert_chunks("proj", "b.rs", &[make_chunk(
                "b.rs",
                0,
                "database query builder",
                1,
                1,
            )])
            .await
            .unwrap();
        store
            .upsert_chunks("proj", "c.rs", &[make_chunk(
                "c.rs",
                0,
                "http request handler",
                1,
                1,
            )])
            .await
            .unwrap();

        let results = store.search_keyword("proj", "database", 10).await.unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_keyword_search_no_results() {
        let store = setup().await;
        store
            .upsert_chunks("proj", "a.rs", &[make_chunk(
                "a.rs",
                0,
                "hello world",
                1,
                1,
            )])
            .await
            .unwrap();

        let results = store
            .search_keyword("proj", "nonexistent_xyz", 10)
            .await
            .unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_keyword_search_scores_valid() {
        let store = setup().await;
        store
            .upsert_chunks("proj", "a.rs", &[make_chunk(
                "a.rs",
                0,
                "rust programming language",
                1,
                1,
            )])
            .await
            .unwrap();
        store
            .upsert_chunks("proj", "b.rs", &[make_chunk(
                "b.rs",
                0,
                "rust memory safety",
                1,
                1,
            )])
            .await
            .unwrap();

        let results = store.search_keyword("proj", "rust", 10).await.unwrap();
        for r in &results {
            assert!(
                r.score > 0.0 && r.score <= 1.0,
                "score {} not in (0, 1]",
                r.score
            );
        }
    }

    #[tokio::test]
    async fn test_file_and_chunk_counts() {
        let store = setup().await;
        for file in ["a.rs", "b.rs", "c.rs"] {
            store
                .upsert_chunks("proj", file, &[
                    make_chunk(file, 0, "chunk0", 1, 5),
                    make_chunk(file, 1, "chunk1", 6, 10),
                ])
                .await
                .unwrap();
        }

        assert_eq!(store.file_count("proj").await.unwrap(), 3);
        assert_eq!(store.chunk_count("proj").await.unwrap(), 6);
    }

    #[tokio::test]
    async fn test_empty_project() {
        let store = setup().await;
        assert!(
            store
                .get_project_chunks("nonexistent")
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(store.file_count("nonexistent").await.unwrap(), 0);
        assert_eq!(store.chunk_count("nonexistent").await.unwrap(), 0);
    }
}
