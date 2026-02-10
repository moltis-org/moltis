use clap::Subcommand;

#[derive(Subcommand)]
pub enum MemoryAction {
    /// Search memories using keyword (FTS5) search.
    Search {
        /// The search query.
        query: String,
        /// Maximum number of results to return.
        #[arg(long, default_value_t = 5)]
        limit: usize,
        /// Output results as JSON for scripting.
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Show memory system status (files, chunks, database size).
    Status,
}

pub async fn handle_memory(action: MemoryAction) -> anyhow::Result<()> {
    match action {
        MemoryAction::Search { query, limit, json } => search_memory(&query, limit, json).await,
        MemoryAction::Status => show_status().await,
    }
}

/// Resolve the memory.db path using the data directory.
fn memory_db_path() -> std::path::PathBuf {
    moltis_config::data_dir().join("memory.db")
}

/// Open a read-only SQLite connection pool to memory.db.
async fn open_memory_pool() -> anyhow::Result<sqlx::SqlitePool> {
    let db_path = memory_db_path();
    if !db_path.exists() {
        anyhow::bail!(
            "Memory database not found at {}. Start the gateway first to index memories.",
            db_path.display()
        );
    }
    let db_url = format!("sqlite:{}?mode=ro", db_path.display());
    let pool = sqlx::SqlitePool::connect(&db_url).await?;
    Ok(pool)
}

async fn search_memory(query: &str, limit: usize, json: bool) -> anyhow::Result<()> {
    let pool = open_memory_pool().await?;
    let store = moltis_memory::store_sqlite::SqliteMemoryStore::new(pool);
    let results = moltis_memory::search::keyword_only_search(&store, query, limit).await?;

    if results.is_empty() {
        if json {
            println!("[]");
        } else {
            println!("No results found.");
        }
        return Ok(());
    }

    if json {
        print_json(&results)?;
    } else {
        print_human(&results);
    }

    Ok(())
}

fn print_json(results: &[moltis_memory::search::SearchResult]) -> anyhow::Result<()> {
    let items: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            serde_json::json!({
                "score": r.score,
                "path": r.path,
                "start_line": r.start_line,
                "end_line": r.end_line,
                "text": r.text,
            })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&items)?);
    Ok(())
}

fn print_human(results: &[moltis_memory::search::SearchResult]) {
    for (i, r) in results.iter().enumerate() {
        if i > 0 {
            println!();
        }
        println!(
            "[{:.2}] {} (lines {}-{})",
            r.score, r.path, r.start_line, r.end_line
        );
        // Indent the text snippet
        let snippet = r.text.trim();
        let preview: String = snippet.chars().take(200).collect();
        for line in preview.lines() {
            println!("  {line}");
        }
        if snippet.len() > 200 {
            println!("  ...");
        }
    }
}

async fn show_status() -> anyhow::Result<()> {
    let db_path = memory_db_path();
    if !db_path.exists() {
        println!("Memory database not found at {}.", db_path.display());
        println!("Start the gateway to begin indexing memories.");
        return Ok(());
    }

    let pool = open_memory_pool().await?;
    let store = moltis_memory::store_sqlite::SqliteMemoryStore::new(pool);

    let config = moltis_memory::config::MemoryConfig {
        db_path: db_path.to_string_lossy().to_string(),
        ..Default::default()
    };
    let manager = moltis_memory::manager::MemoryManager::keyword_only(config, Box::new(store));
    let status = manager.status().await?;

    println!("Memory status:");
    println!("  Files:           {}", status.total_files);
    println!("  Chunks:          {}", status.total_chunks);
    println!("  Embedding model: {}", status.embedding_model);
    println!("  Database size:   {}", status.db_size_display());
    println!("  Database path:   {}", db_path.display());

    Ok(())
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_db_path_contains_memory_db() {
        let path = memory_db_path();
        assert!(
            path.to_string_lossy().contains("memory.db"),
            "path should contain memory.db, got: {}",
            path.display()
        );
    }

    #[tokio::test]
    async fn test_search_missing_db() {
        // Point data dir to a temp directory with no memory.db
        let tmp = tempfile::TempDir::new().unwrap();
        moltis_config::set_data_dir(tmp.path().to_path_buf());

        let result = search_memory("test", 5, false).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Memory database not found"),
            "expected 'not found' error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_search_with_results() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("memory.db");
        let db_url = format!("sqlite:{}?mode=rwc", db_path.display());

        // Create and populate the database
        let pool = sqlx::SqlitePool::connect(&db_url).await.unwrap();
        moltis_memory::schema::run_migrations(&pool).await.unwrap();

        // Insert test data
        sqlx::query("INSERT INTO files (path, source, hash, mtime, size) VALUES (?, ?, ?, ?, ?)")
            .bind("test.md")
            .bind("daily")
            .bind("abc123")
            .bind(1000_i64)
            .bind(500_i64)
            .execute(&pool)
            .await
            .unwrap();

        sqlx::query(
            "INSERT INTO chunks (id, path, source, start_line, end_line, hash, model, text, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("c1")
        .bind("test.md")
        .bind("daily")
        .bind(1_i64)
        .bind(10_i64)
        .bind("h1")
        .bind("none")
        .bind("rust programming language features")
        .bind("now")
        .execute(&pool)
        .await
        .unwrap();

        pool.close().await;

        // Point data dir to our temp directory
        moltis_config::set_data_dir(tmp.path().to_path_buf());

        // Search should find results
        let pool = open_memory_pool().await.unwrap();
        let store = moltis_memory::store_sqlite::SqliteMemoryStore::new(pool);
        let results = moltis_memory::search::keyword_only_search(&store, "rust", 5)
            .await
            .unwrap();
        assert!(!results.is_empty(), "should find results for 'rust'");
        assert_eq!(results[0].path, "test.md");
    }

    #[tokio::test]
    async fn test_search_no_results() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("memory.db");
        let db_url = format!("sqlite:{}?mode=rwc", db_path.display());

        let pool = sqlx::SqlitePool::connect(&db_url).await.unwrap();
        moltis_memory::schema::run_migrations(&pool).await.unwrap();
        pool.close().await;

        moltis_config::set_data_dir(tmp.path().to_path_buf());

        let pool = open_memory_pool().await.unwrap();
        let store = moltis_memory::store_sqlite::SqliteMemoryStore::new(pool);
        let results = moltis_memory::search::keyword_only_search(&store, "nonexistent", 5)
            .await
            .unwrap();
        assert!(results.is_empty(), "should find no results");
    }

    #[tokio::test]
    async fn test_status_missing_db() {
        let tmp = tempfile::TempDir::new().unwrap();
        moltis_config::set_data_dir(tmp.path().to_path_buf());

        // Should not error, just print a message
        let result = show_status().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_status_with_db() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("memory.db");
        let db_url = format!("sqlite:{}?mode=rwc", db_path.display());

        let pool = sqlx::SqlitePool::connect(&db_url).await.unwrap();
        moltis_memory::schema::run_migrations(&pool).await.unwrap();

        // Insert a file and chunk
        sqlx::query("INSERT INTO files (path, source, hash, mtime, size) VALUES (?, ?, ?, ?, ?)")
            .bind("notes.md")
            .bind("daily")
            .bind("hash1")
            .bind(2000_i64)
            .bind(100_i64)
            .execute(&pool)
            .await
            .unwrap();

        sqlx::query(
            "INSERT INTO chunks (id, path, source, start_line, end_line, hash, model, text, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("c1")
        .bind("notes.md")
        .bind("daily")
        .bind(1_i64)
        .bind(5_i64)
        .bind("h1")
        .bind("none")
        .bind("some content")
        .bind("now")
        .execute(&pool)
        .await
        .unwrap();

        pool.close().await;

        moltis_config::set_data_dir(tmp.path().to_path_buf());

        // Status should succeed and report 1 file, 1 chunk
        let ro_pool = open_memory_pool().await.unwrap();
        let store = moltis_memory::store_sqlite::SqliteMemoryStore::new(ro_pool);
        let config = moltis_memory::config::MemoryConfig {
            db_path: db_path.to_string_lossy().to_string(),
            ..Default::default()
        };
        let manager = moltis_memory::manager::MemoryManager::keyword_only(config, Box::new(store));
        let status = manager.status().await.unwrap();
        assert_eq!(status.total_files, 1);
        assert_eq!(status.total_chunks, 1);
        assert!(status.db_size_bytes > 0);
    }

    #[test]
    fn test_print_json_output() {
        use moltis_memory::search::SearchResult;

        let results = vec![SearchResult {
            chunk_id: "c1".into(),
            path: "test.md".into(),
            source: "daily".into(),
            start_line: 1,
            end_line: 10,
            score: 0.85,
            text: "some content here".into(),
        }];

        // Should not panic
        let result = print_json(&results);
        assert!(result.is_ok());
    }

    #[test]
    fn test_print_human_output() {
        use moltis_memory::search::SearchResult;

        let results = vec![
            SearchResult {
                chunk_id: "c1".into(),
                path: "memory/notes.md".into(),
                source: "daily".into(),
                start_line: 12,
                end_line: 28,
                score: 0.85,
                text: "Today I implemented OAuth2 authentication".into(),
            },
            SearchResult {
                chunk_id: "c2".into(),
                path: "MEMORY.md".into(),
                source: "longterm".into(),
                start_line: 45,
                end_line: 60,
                score: 0.72,
                text: "Authentication architecture uses argon2".into(),
            },
        ];

        // Should not panic
        print_human(&results);
    }
}
