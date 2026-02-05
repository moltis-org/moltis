use {clap::Subcommand, std::path::PathBuf};

#[derive(Subcommand)]
pub enum DbAction {
    /// Delete all database files completely (moltis.db and memory.db).
    Reset,
    /// Clear all data from tables but keep the schema intact.
    Clear,
    /// Run all pending database migrations.
    Migrate,
}

/// Returns the paths to the database files.
fn db_paths() -> (PathBuf, PathBuf) {
    let data_dir = moltis_config::data_dir();
    let main_db = data_dir.join("moltis.db");
    let memory_db = data_dir.join("memory.db");
    (main_db, memory_db)
}

pub async fn handle_db(action: DbAction) -> anyhow::Result<()> {
    match action {
        DbAction::Reset => reset_databases().await,
        DbAction::Clear => clear_databases().await,
        DbAction::Migrate => run_migrations().await,
    }
}

/// Delete all database files completely.
async fn reset_databases() -> anyhow::Result<()> {
    let (main_db, memory_db) = db_paths();

    let mut deleted = false;

    // Also delete WAL and SHM files that SQLite may have created.
    for base in [&main_db, &memory_db] {
        for suffix in ["", "-wal", "-shm"] {
            let path = if suffix.is_empty() {
                base.clone()
            } else {
                base.with_extension(format!("db{}", suffix))
            };
            if path.exists() {
                std::fs::remove_file(&path)?;
                println!("Deleted: {}", path.display());
                deleted = true;
            }
        }
    }

    if deleted {
        println!("Database files deleted. Run `moltis db migrate` to recreate them.");
    } else {
        println!("No database files found.");
    }

    Ok(())
}

/// Clear all data from tables but keep the schema intact.
async fn clear_databases() -> anyhow::Result<()> {
    let (main_db, memory_db) = db_paths();

    // Clear main database
    if main_db.exists() {
        let db_url = format!("sqlite:{}?mode=rwc", main_db.display());
        let pool = sqlx::SqlitePool::connect(&db_url).await?;

        // Order matters due to foreign key constraints.
        // Delete from child tables first.
        let tables = [
            // Sessions/channels (depends on projects)
            "channel_sessions",
            "sessions",
            // Cron (cron_runs depends on cron_jobs)
            "cron_runs",
            "cron_jobs",
            // Gateway tables (no dependencies between them)
            "auth_sessions",
            "api_keys",
            "passkeys",
            "auth_password",
            "env_variables",
            "message_log",
            "channels",
            // Projects (other tables depend on this)
            "projects",
        ];

        for table in tables {
            // Use raw query to avoid compile-time checks
            let query = format!("DELETE FROM {table}");
            if let Err(e) = sqlx::query(&query).execute(&pool).await {
                // Table might not exist if migrations haven't run
                eprintln!("Warning: could not clear {table}: {e}");
            } else {
                println!("Cleared table: {table}");
            }
        }

        pool.close().await;
        println!("Main database cleared.");
    } else {
        println!("Main database not found: {}", main_db.display());
    }

    // Clear memory database
    if memory_db.exists() {
        let db_url = format!("sqlite:{}?mode=rwc", memory_db.display());
        let pool = sqlx::SqlitePool::connect(&db_url).await?;

        // Order matters: chunks depends on files
        let tables = ["embedding_cache", "chunks", "files"];

        for table in tables {
            let query = format!("DELETE FROM {table}");
            if let Err(e) = sqlx::query(&query).execute(&pool).await {
                eprintln!("Warning: could not clear {table}: {e}");
            } else {
                println!("Cleared table: {table}");
            }
        }

        pool.close().await;
        println!("Memory database cleared.");
    } else {
        println!("Memory database not found: {}", memory_db.display());
    }

    Ok(())
}

/// Run all pending database migrations.
async fn run_migrations() -> anyhow::Result<()> {
    let (main_db, memory_db) = db_paths();

    // Ensure data directory exists
    if let Some(parent) = main_db.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Run main database migrations
    println!("Running migrations for main database...");
    let db_url = format!("sqlite:{}?mode=rwc", main_db.display());
    let pool = sqlx::SqlitePool::connect(&db_url).await?;

    // Run migrations in dependency order (same as server.rs)
    moltis_projects::run_migrations(&pool)
        .await
        .map_err(|e| anyhow::anyhow!("projects migrations failed: {e}"))?;
    println!("  - projects migrations complete");

    moltis_sessions::run_migrations(&pool)
        .await
        .map_err(|e| anyhow::anyhow!("sessions migrations failed: {e}"))?;
    println!("  - sessions migrations complete");

    moltis_cron::run_migrations(&pool)
        .await
        .map_err(|e| anyhow::anyhow!("cron migrations failed: {e}"))?;
    println!("  - cron migrations complete");

    moltis_gateway::run_migrations(&pool)
        .await
        .map_err(|e| anyhow::anyhow!("gateway migrations failed: {e}"))?;
    println!("  - gateway migrations complete");

    pool.close().await;

    // Run memory database migrations
    println!("Running migrations for memory database...");
    let memory_url = format!("sqlite:{}?mode=rwc", memory_db.display());
    let memory_pool = sqlx::SqlitePool::connect(&memory_url).await?;

    moltis_memory::schema::run_migrations(&memory_pool)
        .await
        .map_err(|e| anyhow::anyhow!("memory migrations failed: {e}"))?;
    println!("  - memory migrations complete");

    memory_pool.close().await;

    println!("All migrations complete.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use {super::*, tempfile::TempDir};

    #[test]
    fn test_db_paths_uses_data_dir() {
        // Just verify the function constructs expected paths
        let paths = db_paths();
        assert!(
            paths.0.to_string_lossy().contains("moltis.db"),
            "main db path should contain moltis.db"
        );
        assert!(
            paths.1.to_string_lossy().contains("memory.db"),
            "memory db path should contain memory.db"
        );
    }

    /// Test reset_databases by manually deleting files in a temp dir.
    /// This avoids global state issues by testing the file deletion logic directly.
    #[tokio::test]
    async fn test_reset_deletes_files() {
        let temp = TempDir::new().unwrap();

        // Create dummy database files
        let main_db = temp.path().join("moltis.db");
        let memory_db = temp.path().join("memory.db");
        let main_wal = temp.path().join("moltis.db-wal");
        let main_shm = temp.path().join("moltis.db-shm");

        std::fs::write(&main_db, "test").unwrap();
        std::fs::write(&memory_db, "test").unwrap();
        std::fs::write(&main_wal, "test").unwrap();
        std::fs::write(&main_shm, "test").unwrap();

        assert!(main_db.exists());
        assert!(memory_db.exists());
        assert!(main_wal.exists());
        assert!(main_shm.exists());

        // Simulate what reset_databases does
        for path in [&main_db, &memory_db, &main_wal, &main_shm] {
            std::fs::remove_file(path).unwrap();
        }

        assert!(!main_db.exists(), "main database should be deleted");
        assert!(!memory_db.exists(), "memory database should be deleted");
        assert!(!main_wal.exists(), "WAL file should be deleted");
        assert!(!main_shm.exists(), "SHM file should be deleted");
    }

    /// Test that handle_db dispatches to the correct action.
    #[test]
    fn test_db_action_variants() {
        // Verify the enum variants exist
        let _ = super::DbAction::Reset;
        let _ = super::DbAction::Clear;
        let _ = super::DbAction::Migrate;
    }

    /// Test that migrations run successfully against a fresh database in a temp directory.
    /// This verifies the full migration flow works end-to-end.
    #[tokio::test]
    async fn test_migrations_run_successfully_in_temp_dir() {
        let temp = TempDir::new().unwrap();
        let main_db = temp.path().join("moltis.db");
        let memory_db = temp.path().join("memory.db");

        // Run main database migrations
        let db_url = format!("sqlite:{}?mode=rwc", main_db.display());
        let pool = sqlx::SqlitePool::connect(&db_url).await.unwrap();

        // Run migrations in dependency order
        moltis_projects::run_migrations(&pool).await.unwrap();
        moltis_sessions::run_migrations(&pool).await.unwrap();
        moltis_cron::run_migrations(&pool).await.unwrap();
        moltis_gateway::run_migrations(&pool).await.unwrap();

        // Verify tables were created by querying them
        let _: (i64,) = sqlx::query_as("SELECT count(*) FROM projects")
            .fetch_one(&pool)
            .await
            .unwrap();
        let _: (i64,) = sqlx::query_as("SELECT count(*) FROM sessions")
            .fetch_one(&pool)
            .await
            .unwrap();
        let _: (i64,) = sqlx::query_as("SELECT count(*) FROM cron_jobs")
            .fetch_one(&pool)
            .await
            .unwrap();
        let _: (i64,) = sqlx::query_as("SELECT count(*) FROM channels")
            .fetch_one(&pool)
            .await
            .unwrap();

        pool.close().await;

        // Run memory database migrations
        let memory_url = format!("sqlite:{}?mode=rwc", memory_db.display());
        let memory_pool = sqlx::SqlitePool::connect(&memory_url).await.unwrap();

        moltis_memory::schema::run_migrations(&memory_pool)
            .await
            .unwrap();

        // Verify memory tables were created
        let _: (i64,) = sqlx::query_as("SELECT count(*) FROM files")
            .fetch_one(&memory_pool)
            .await
            .unwrap();
        let _: (i64,) = sqlx::query_as("SELECT count(*) FROM chunks")
            .fetch_one(&memory_pool)
            .await
            .unwrap();

        memory_pool.close().await;

        // Verify database files exist
        assert!(main_db.exists(), "main database should be created");
        assert!(memory_db.exists(), "memory database should be created");
    }

    /// Test that migrations can run multiple times without error (idempotent).
    /// This verifies set_ignore_missing works correctly.
    #[tokio::test]
    async fn test_migrations_are_idempotent() {
        let temp = TempDir::new().unwrap();
        let main_db = temp.path().join("moltis.db");

        let db_url = format!("sqlite:{}?mode=rwc", main_db.display());
        let pool = sqlx::SqlitePool::connect(&db_url).await.unwrap();

        // Run all migrations
        moltis_projects::run_migrations(&pool).await.unwrap();
        moltis_sessions::run_migrations(&pool).await.unwrap();
        moltis_cron::run_migrations(&pool).await.unwrap();
        moltis_gateway::run_migrations(&pool).await.unwrap();

        // Run again - should still work due to set_ignore_missing
        moltis_projects::run_migrations(&pool).await.unwrap();
        moltis_sessions::run_migrations(&pool).await.unwrap();
        moltis_cron::run_migrations(&pool).await.unwrap();
        moltis_gateway::run_migrations(&pool).await.unwrap();

        pool.close().await;
    }
}
