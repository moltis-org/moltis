//! Row types for the memory database and migration runner.

/// A tracked file row.
#[derive(Debug, Clone)]
pub struct FileRow {
    pub path: String,
    pub source: String,
    pub hash: String,
    pub mtime: i64,
    pub size: i64,
}

/// A chunk row.
#[derive(Debug, Clone)]
pub struct ChunkRow {
    pub id: String,
    pub path: String,
    pub source: String,
    pub start_line: i64,
    pub end_line: i64,
    pub hash: String,
    pub model: String,
    pub text: String,
    pub embedding: Option<Vec<u8>>,
    pub updated_at: String,
}

/// Run database migrations for the memory system.
///
/// This creates the `files`, `chunks`, `embedding_cache` tables and the
/// `chunks_fts` full-text search virtual table. Memory uses its own database
/// (`memory.db`) separate from the main `moltis.db`.
pub async fn run_migrations(pool: &sqlx::SqlitePool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations")
        .set_ignore_missing(true)
        .run(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_run_migrations() {
        let pool = sqlx::SqlitePool::connect(":memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        // Verify tables exist
        let row: (i64,) = sqlx::query_as("SELECT count(*) FROM files")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.0, 0);
    }
}
