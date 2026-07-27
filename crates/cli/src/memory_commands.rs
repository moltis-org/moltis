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
    /// Re-index chunks from one zvec collection to another with re-embedding.
    #[cfg(feature = "zvec")]
    Reindex {
        /// Source zvec collection path (default: active collection from config).
        #[arg(long)]
        from: Option<String>,
        /// Target zvec collection path (required).
        #[arg(long)]
        to: String,
        /// Model for re-embedding (default: current config model).
        #[arg(long)]
        model: Option<String>,
        /// Target embedding dimension (default: same as source).
        #[arg(long)]
        target_dim: Option<u32>,
        /// Print plan without executing.
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        /// Skip confirmation prompt.
        #[arg(long, default_value_t = false)]
        yes: bool,
    },
}

pub async fn handle_memory(action: MemoryAction) -> anyhow::Result<()> {
    match action {
        MemoryAction::Search { query, limit, json } => search_memory(&query, limit, json).await,
        MemoryAction::Status => show_status().await,
        #[cfg(feature = "zvec")]
        MemoryAction::Reindex {
            from,
            to,
            model,
            target_dim,
            dry_run,
            yes,
        } => handle_reindex(from, to, model, target_dim, dry_run, yes).await,
    }
}

/// Resolve the memory.db path using the data directory. (Used by the
/// SQLite-backed tests below.)
#[cfg(test)]
fn memory_db_path() -> std::path::PathBuf {
    moltis_config::data_dir().join("memory.db")
}

/// The active memory store plus its on-disk location, or a pre-formatted
/// "not found" message when no backing data exists yet.
///
/// `search_memory` and `show_status` route through this so they query the
/// backend the gateway actually writes to (SQLite for `Builtin`, zvec for
/// `Zvec`) instead of always opening `memory.db` — which is empty/absent when
/// zvec is active.
enum ActiveStore {
    Open {
        store: Box<dyn moltis_memory::store::MemoryStore>,
        display_path: std::path::PathBuf,
    },
    NotFound {
        message: String,
    },
}

/// Load config and open whichever memory backend the gateway writes to.
async fn open_active_store() -> anyhow::Result<ActiveStore> {
    let config = moltis_config::discover_and_load();
    let data_dir = moltis_config::data_dir();
    open_store_for_backend(&config.memory, &data_dir).await
}

/// Backend-dispatching core of [`open_active_store`], factored out so tests can
/// drive it with an explicit config + data dir (without relying on the global
/// config file).
#[allow(unused_variables)]
async fn open_store_for_backend(
    mem_cfg: &moltis_config::schema::MemoryEmbeddingConfig,
    data_dir: &std::path::Path,
) -> anyhow::Result<ActiveStore> {
    match mem_cfg.backend {
        #[cfg(feature = "zvec")]
        moltis_config::MemoryBackend::Zvec => open_zvec_store(mem_cfg, data_dir).await,
        _ => open_sqlite_store(data_dir).await,
    }
}

/// Open the built-in SQLite backend at `<data_dir>/memory.db`.
async fn open_sqlite_store(data_dir: &std::path::Path) -> anyhow::Result<ActiveStore> {
    let db_path = data_dir.join("memory.db");
    if !db_path.exists() {
        return Ok(ActiveStore::NotFound {
            message: format!(
                "Memory database not found at {}. Start the gateway first to index memories.",
                db_path.display()
            ),
        });
    }
    let db_url = format!("sqlite:{}?mode=ro", db_path.display());
    let pool = sqlx::SqlitePool::connect(&db_url).await?;
    let store: Box<dyn moltis_memory::store::MemoryStore> =
        Box::new(moltis_memory::store_sqlite::SqliteMemoryStore::new(pool));
    Ok(ActiveStore::Open {
        store,
        display_path: db_path,
    })
}

/// Open the zvec backend's collection under `<data_dir>/<db_path>`.
///
/// The collection only exists once the gateway has run; before that we return
/// [`ActiveStore::NotFound`] so callers can tell the user to start the gateway.
#[cfg(feature = "zvec")]
async fn open_zvec_store(
    mem_cfg: &moltis_config::schema::MemoryEmbeddingConfig,
    data_dir: &std::path::Path,
) -> anyhow::Result<ActiveStore> {
    let db_name = mem_cfg.db_path.as_deref().unwrap_or("memory.zvec");
    let collection_stem = data_dir.join(db_name);
    let default_dim = mem_cfg.embedding_dimension.unwrap_or(768);
    let dim = resolve_zvec_dimension(&collection_stem, mem_cfg.embedding_dimension, default_dim);

    // zvec writes the HNSW collection to `<stem>_<dim>`. If that file isn't on
    // disk, the gateway hasn't indexed anything yet — don't silently create an
    // empty collection.
    let collection_file = format!("{}_{}", collection_stem.display(), dim);
    if !std::path::Path::new(&collection_file).exists() {
        return Ok(ActiveStore::NotFound {
            message: format!(
                "Memory collection not found at {collection_file}. \
                 Start the gateway first to index memories."
            ),
        });
    }

    moltis_memory_zvec::ensure_zvec_initialized()?;

    let cache_path = {
        let mut p = collection_stem.clone();
        p.as_mut_os_string().push(".cache");
        p
    };
    let display_path = collection_stem.clone();
    let store = {
        let stem = collection_stem;
        let cp = cache_path;
        tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            let collection = moltis_memory_zvec::open_or_create_collection(&stem, Some(dim))?;
            let cache = moltis_memory_zvec::RedbCache::new(&cp)?;
            Ok(
                moltis_memory_zvec::ZvecMemoryStore::with_cache(collection, cache)
                    .with_cache_dimension(dim)
                    .with_collection_disk_path(stem.as_path()),
            )
        })
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking join failed: {e}"))??
    };

    // If a writer (e.g. a killed gateway) flushed documents but exited before
    // optimizing, the reopened collection has its documents but an empty FTS
    // index — keyword search returns nothing. Probe the FTS index with a token
    // from an existing chunk; if it's stale, rebuild it via re-ingestion.
    #[cfg(feature = "zvec")]
    {
        use moltis_memory::store::{MemoryStore as _, MergeStrategy};
        let probe = &store;
        if let Ok(files) = probe.list_files().await
            && !files.is_empty()
            && let Ok(chunks) = probe.get_chunks_for_file(&files[0].path).await
            && let Some(first) = chunks.first()
            && let Some(token) = first.text.split_whitespace().next()
            && !token.is_empty()
            && let Ok(results) = probe
                .hybrid_search(&[], token, 0.0, 1.0, MergeStrategy::Weighted, 1)
                .await
            && results.is_empty()
        {
            let count = store.rebuild_keyword_index().await?;
            eprintln!(
                "Keyword index was stale (likely a non-optimized writer exit); rebuilt {count} chunks."
            );
        }
    }

    Ok(ActiveStore::Open {
        store: Box::new(store),
        display_path,
    })
}

/// Resolve the embedding dimension for a zvec collection stem.
///
/// The zvec backend writes its HNSW collection to `<stem>_<dim>` on disk
/// (suffix appended by `collection_path()` in moltis-memory-zvec; the cache
/// and lock files share the bare stem and are skipped by the `_<digits>`
/// match below). To survive later changes to `memory.embedding_dimension`,
/// scan the parent directory for an existing suffixed sibling and prefer
/// the one matching `config_dim` when present. Falls back to `config_dim`
/// (so a fresh collection can still be created) and then `default_dim`.
#[cfg(feature = "zvec")]
fn resolve_zvec_dimension(
    stem: &std::path::Path,
    config_dim: Option<u32>,
    default_dim: u32,
) -> u32 {
    use std::path::Path;

    let parent = stem.parent().unwrap_or_else(|| Path::new("."));
    let stem_name = match stem.file_name().and_then(|s| s.to_str()) {
        Some(s) if !s.is_empty() => s,
        _ => return config_dim.unwrap_or(default_dim),
    };

    let mut first_found: Option<u32> = None;
    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().into_string().ok() else {
                continue;
            };
            let Some(suffix) = name.strip_prefix(stem_name) else {
                continue;
            };
            let Some(digits) = suffix.strip_prefix('_') else {
                continue;
            };
            // Reject suffixes with extra characters (e.g. `<stem>_768.lock`)
            // so we only match the bare collection file.
            if !digits.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            let Ok(dim) = digits.parse::<u32>() else {
                continue;
            };
            if Some(dim) == config_dim {
                return dim;
            }
            first_found.get_or_insert(dim);
        }
    }
    first_found.unwrap_or_else(|| config_dim.unwrap_or(default_dim))
}

/// Open a read-only SQLite connection pool to memory.db. (Used by the
/// SQLite-backed tests below.)
#[cfg(test)]
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
    let store = match open_active_store().await? {
        ActiveStore::Open { store, .. } => store,
        ActiveStore::NotFound { message } => anyhow::bail!(message),
    };
    let results = moltis_memory::search::keyword_only_search(store.as_ref(), query, limit).await?;

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
    let (store, display_path) = match open_active_store().await? {
        ActiveStore::Open {
            store,
            display_path,
        } => (store, display_path),
        ActiveStore::NotFound { message } => {
            println!("{message}");
            return Ok(());
        },
    };

    let config = moltis_memory::config::MemoryConfig {
        db_path: display_path.to_string_lossy().to_string(),
        ..Default::default()
    };
    let manager = moltis_memory::manager::MemoryManager::keyword_only(config, store);
    let status = manager.status().await?;

    println!("Memory status:");
    println!("  Files:           {}", status.total_files);
    println!("  Chunks:          {}", status.total_chunks);
    println!("  Embedding model: {}", status.embedding_model);
    println!("  Backend:         {}", status.backend_type);
    println!("  Database size:   {}", status.db_size_display());
    println!("  Database path:   {}", display_path.display());

    Ok(())
}

#[cfg(feature = "zvec")]
async fn handle_reindex(
    from: Option<String>,
    to: String,
    model: Option<String>,
    target_dim: Option<u32>,
    dry_run: bool,
    yes: bool,
) -> anyhow::Result<()> {
    use std::path::Path;

    use {
        moltis_memory::{embeddings::EmbeddingProvider, store::MemoryStore},
        secrecy::ExposeSecret,
    };

    let config = moltis_config::discover_and_load();
    let data_dir = moltis_config::data_dir();

    // The zvec backend stores its HNSW collection on disk as `<stem>_<dim>`
    // (e.g. `memory.zvec_768`), with the suffix appended by
    // `collection_path()` in moltis-memory-zvec. The gateway always opens
    // the active collection with `Some(dim)`, so the bare-stem file the
    // previous default produced (`memory_zvec` with no suffix) never matched
    // a real collection and `open_or_create_collection` would silently
    // create a fresh empty one. Mirror the gateway default stem here and
    // resolve the dimension up front by scanning for an existing suffixed
    // sibling, falling back to the configured dimension and then 768.
    let source_stem = match from {
        Some(ref p) => std::path::PathBuf::from(p),
        None => {
            let db_name = config.memory.db_path.as_deref().unwrap_or("memory.zvec");
            data_dir.join(db_name)
        },
    };
    let default_dim = config.memory.embedding_dimension.unwrap_or(768);
    let source_dim_guess =
        resolve_zvec_dimension(&source_stem, config.memory.embedding_dimension, default_dim);

    moltis_memory_zvec::ensure_zvec_initialized()?;

    // Open the source as a full store so enumeration uses the durable redb
    // file/chunk index (HNSW filter queries can't reliably list documents).
    let source_cache_path = {
        let mut p = source_stem.clone();
        p.as_mut_os_string().push(".cache");
        p
    };
    let source = {
        let sp = source_stem.clone();
        let cp = source_cache_path.clone();
        let dim = source_dim_guess;
        tokio::task::spawn_blocking(move || {
            let collection = moltis_memory_zvec::open_or_create_collection(&sp, Some(dim))?;
            let cache = moltis_memory_zvec::RedbCache::new(&cp)?;
            Ok::<_, anyhow::Error>(moltis_memory_zvec::ZvecMemoryStore::with_cache(
                collection, cache,
            ))
        })
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking join failed: {e}"))??
    };

    let source_dim = {
        let coll = source.collection_arc();
        tokio::task::spawn_blocking(move || moltis_memory_zvec::read_dimension_meta(&coll))
            .await
            .map_err(|e| anyhow::anyhow!("spawn_blocking join failed: {e}"))??
            .unwrap_or(source_dim_guess)
    };

    let source_rows = source.list_files().await?;
    let file_paths: Vec<String> = source_rows.iter().map(|f| f.path.clone()).collect();

    let mut total_chunks: usize = 0;
    for fp in &file_paths {
        total_chunks += source.get_chunks_for_file(fp).await?.len();
    }

    let model_name = model.unwrap_or_else(|| {
        config
            .memory
            .model
            .clone()
            .unwrap_or_else(|| "text-embedding-3-small".to_string())
    });

    let target_dim = target_dim.unwrap_or(source_dim);
    let embed_dims = target_dim as usize;

    let api_key = config
        .memory
        .api_key
        .as_ref()
        .map(|k| k.expose_secret().clone())
        .or_else(|| std::env::var("OPENAI_API_KEY").ok())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No embedding API key configured. Set [memory] api_key in config \
                 or OPENAI_API_KEY environment variable."
            )
        })?;

    let base_url = config
        .memory
        .base_url
        .clone()
        .unwrap_or_else(|| "https://api.openai.com".to_string());

    let embedder = moltis_memory::embeddings_openai::OpenAiEmbeddingProvider::new(api_key)
        .with_base_url(base_url)
        .with_model(model_name.clone(), embed_dims);

    println!("Re-index plan:");
    println!(
        "  Source collection:  {}_{}",
        source_stem.display(),
        source_dim
    );
    println!("  Target collection:  {}", Path::new(&to).display());
    println!("  Source dimension:   {}", source_dim);
    println!("  Target dimension:   {}", target_dim);
    println!("  Model:              {}", model_name);
    println!("  Chunks to process:  {}", total_chunks);

    if dry_run {
        return Ok(());
    }

    if !yes {
        println!();
        print!("Proceed with re-index? [y/N]: ");
        use std::io::Write;
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        let answer = answer.trim().to_lowercase();
        if answer != "y" && answer != "yes" {
            println!("Cancelled.");
            return Ok(());
        }
    }

    // Open the target as a store so writes populate its redb index too.
    let target_path = Path::new(&to).to_path_buf();
    let target_cache_path = {
        let mut p = target_path.clone();
        p.as_mut_os_string().push(".cache");
        p
    };
    let mut target = {
        let tp = target_path.clone();
        let cp = target_cache_path.clone();
        tokio::task::spawn_blocking(move || {
            let collection = moltis_memory_zvec::open_or_create_collection(&tp, Some(target_dim))?;
            let cache = moltis_memory_zvec::RedbCache::new(&cp)?;
            Ok::<_, anyhow::Error>(moltis_memory_zvec::ZvecMemoryStore::with_cache(
                collection, cache,
            ))
        })
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking join failed: {e}"))??
    };
    target = target.with_cache_dimension(target_dim);

    let pb = indicatif::ProgressBar::new(total_chunks as u64);
    let style = indicatif::ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
        .unwrap_or_else(|_| indicatif::ProgressStyle::default_bar())
        .progress_chars("#>-");
    pb.set_style(style);

    let mut processed: usize = 0;

    for file_row in &source_rows {
        let chunks = source.get_chunks_for_file(&file_row.path).await?;
        if chunks.is_empty() {
            continue;
        }

        // Carry the file metadata into the target store.
        target.upsert_file(file_row).await?;

        let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
        let embeddings: Vec<Vec<f32>> = embedder.embed_batch(&texts).await?;

        let new_chunks: Vec<moltis_memory::schema::ChunkRow> = chunks
            .into_iter()
            .zip(embeddings)
            .map(|(mut c, emb)| {
                let emb_bytes: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
                c.model = model_name.clone();
                c.embedding = Some(emb_bytes);
                c
            })
            .collect();

        let count = new_chunks.len();
        target.upsert_chunks(&new_chunks).await?;
        processed += count;
        pb.inc(count as u64);
    }

    pb.finish_and_clear();

    let mut final_total: usize = 0;
    for fp in target.list_files().await? {
        final_total += target.get_chunks_for_file(&fp.path).await?.len();
    }

    println!();
    println!(
        "Re-index complete: {} chunks processed, {} verified in target.",
        processed, final_total
    );

    if final_total != total_chunks {
        anyhow::bail!(
            "Chunk count mismatch: source had {} chunks, target has {}.",
            total_chunks,
            final_total
        );
    }

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

    #[cfg(feature = "zvec")]
    mod zvec_dim {
        use super::*;

        fn write_collection(dir: &std::path::Path, stem: &str, dim: u32) {
            std::fs::write(dir.join(format!("{stem}_{dim}")), b"").unwrap();
        }

        #[test]
        fn prefers_config_dim_when_matching_sibling_exists() {
            let tmp = tempfile::TempDir::new().unwrap();
            let stem = tmp.path().join("memory.zvec");
            write_collection(tmp.path(), "memory.zvec", 1536);
            write_collection(tmp.path(), "memory.zvec", 768);
            // config_dim 768 is one of the siblings → pick it even though 1536 sorts first.
            assert_eq!(resolve_zvec_dimension(&stem, Some(768), 1536), 768);
        }

        #[test]
        fn falls_back_to_first_sibling_when_config_dim_absent() {
            let tmp = tempfile::TempDir::new().unwrap();
            let stem = tmp.path().join("memory.zvec");
            write_collection(tmp.path(), "memory.zvec", 1024);
            // No matching sibling for config_dim=1536, but a real collection exists → use it.
            assert_eq!(resolve_zvec_dimension(&stem, Some(1536), 768), 1024);
        }

        #[test]
        fn ignores_cache_and_lock_files() {
            let tmp = tempfile::TempDir::new().unwrap();
            let stem = tmp.path().join("memory.zvec");
            // Cache/lock files share the stem but must not be parsed as dimensions.
            std::fs::write(tmp.path().join("memory.zvec.cache"), b"").unwrap();
            std::fs::write(tmp.path().join("memory.zvec.lock"), b"").unwrap();
            std::fs::write(tmp.path().join("memory.zvec_768.bak"), b"").unwrap();
            write_collection(tmp.path(), "memory.zvec", 768);
            assert_eq!(resolve_zvec_dimension(&stem, None, 1024), 768);
        }

        #[test]
        fn uses_config_dim_when_no_sibling_exists() {
            let tmp = tempfile::TempDir::new().unwrap();
            let stem = tmp.path().join("memory.zvec");
            // Fresh collection: no sibling on disk, must fall through to config_dim.
            assert_eq!(resolve_zvec_dimension(&stem, Some(1536), 768), 1536);
        }

        #[test]
        fn uses_default_dim_when_nothing_else_available() {
            let tmp = tempfile::TempDir::new().unwrap();
            let stem = tmp.path().join("memory.zvec");
            assert_eq!(resolve_zvec_dimension(&stem, None, 768), 768);
        }

        #[test]
        fn handles_relative_stem_without_parent() {
            // A stem with no parent component should not panic; parent resolves to ".".
            // We can't assert on a specific dim here since it depends on cwd contents,
            // but we can verify the call doesn't panic and returns a positive value.
            let stem = std::path::PathBuf::from("nonexistent_test_stem_12345");
            let dim = resolve_zvec_dimension(&stem, None, 768);
            assert_eq!(dim, 768);
        }
    }

    /// Backend routing: `search_memory`/`show_status` must query the zvec
    /// collection (not the always-empty `memory.db`) when zvec is active.
    #[cfg(feature = "zvec")]
    mod zvec_routing {
        use {
            super::*, moltis_config::schema::MemoryEmbeddingConfig,
            moltis_memory::store::MemoryStore as _,
        };

        fn mem_cfg_zvec(db_name: &str, dim: Option<u32>) -> MemoryEmbeddingConfig {
            MemoryEmbeddingConfig {
                backend: moltis_config::MemoryBackend::Zvec,
                db_path: Some(db_name.to_string()),
                embedding_dimension: dim,
                ..Default::default()
            }
        }

        #[tokio::test]
        async fn not_found_when_collection_missing() {
            let tmp = tempfile::TempDir::new().unwrap();
            let cfg = mem_cfg_zvec("memory.zvec", Some(768));
            match open_store_for_backend(&cfg, tmp.path()).await.unwrap() {
                ActiveStore::NotFound { message } => {
                    assert!(message.contains("not found"), "{message}");
                },
                ActiveStore::Open { .. } => {
                    panic!("expected NotFound when no collection exists on disk")
                },
            }
        }

        #[tokio::test]
        async fn sqlite_not_found_for_builtin_backend() {
            // Builtin backend must keep using the SQLite path.
            let tmp = tempfile::TempDir::new().unwrap();
            let cfg = MemoryEmbeddingConfig::default(); // backend = Builtin
            match open_store_for_backend(&cfg, tmp.path()).await.unwrap() {
                ActiveStore::NotFound { message } => {
                    assert!(message.contains("Memory database not found"), "{message}");
                },
                ActiveStore::Open { .. } => {
                    panic!("expected NotFound when no memory.db exists")
                },
            }
        }

        #[tokio::test]
        async fn opens_existing_collection_and_searches() {
            // Seed a real zvec collection with one chunk, then route through
            // open_store_for_backend and verify keyword search finds it.
            moltis_memory_zvec::ensure_zvec_initialized().unwrap();
            let tmp = tempfile::TempDir::new().unwrap();
            let stem = tmp.path().join("memory.zvec");
            let cache_path = {
                let mut p = stem.clone();
                p.as_mut_os_string().push(".cache");
                p
            };

            {
                let collection =
                    moltis_memory_zvec::open_or_create_collection(&stem, Some(768)).unwrap();
                let cache = moltis_memory_zvec::RedbCache::new(&cache_path).unwrap();
                let store = moltis_memory_zvec::ZvecMemoryStore::with_cache(collection, cache);
                let emb_bytes: Vec<u8> = vec![0.0f32; 768]
                    .iter()
                    .flat_map(|f| f.to_le_bytes())
                    .collect();
                let chunks = vec![moltis_memory::schema::ChunkRow {
                    id: "route-1".into(),
                    path: "route.md".into(),
                    source: "test".into(),
                    start_line: 1,
                    end_line: 5,
                    hash: "h".into(),
                    model: "m".into(),
                    text: "routing cli commands to the zvec backend".into(),
                    embedding: Some(emb_bytes),
                    updated_at: "2025-01-01T00:00:00Z".into(),
                }];
                store.upsert_chunks(&chunks).await.unwrap();
                // zvec buffers writes; flush + optimize so the reopened handle
                // can see them (including the FTS index). The gateway achieves
                // this steady state via its periodic-optimize background task.
                let coll = store.collection_arc();
                moltis_memory_zvec::flush_collection(&coll).unwrap();
                store.optimize().unwrap();
            }

            let cfg = mem_cfg_zvec("memory.zvec", Some(768));
            match open_store_for_backend(&cfg, tmp.path()).await.unwrap() {
                ActiveStore::Open {
                    store,
                    display_path,
                } => {
                    assert_eq!(display_path, stem);
                    // Diagnostic: confirm the chunk persisted across reopen.
                    let persisted = store.get_chunk_by_id("route-1").await.unwrap();
                    assert!(
                        persisted.is_some(),
                        "seeded chunk must persist across collection reopen"
                    );
                    let results =
                        moltis_memory::search::keyword_only_search(store.as_ref(), "routing", 5)
                            .await
                            .unwrap();
                    assert!(
                        results.iter().any(|r| r.chunk_id == "route-1"),
                        "keyword search should find the seeded chunk, got {results:?}"
                    );
                },
                ActiveStore::NotFound { message } => {
                    panic!("expected Open, collection was seeded: {message}")
                },
            }
        }
    }
}
