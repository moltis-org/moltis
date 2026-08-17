use std::{path::Path, sync::OnceLock};

use {
    anyhow::{Context, Result},
    tracing::{debug, error, warn},
    zvec::{Collection, CollectionSchema, DataType, Doc, FieldSchema, IndexParams, MetricType},
};

/// Global guard ensuring `zvec::initialize` runs once. The cached `Result`
/// (success or the error string) is reused by every subsequent caller, so
/// concurrent calls never invoke `initialize` twice.
static ZVEC_INIT: OnceLock<Result<(), String>> = OnceLock::new();

/// Ensure zvec library is initialized exactly once (safe to call concurrently).
///
/// Returns `Err` if initialization failed; once it has succeeded, all
/// subsequent calls are no-ops returning `Ok(())`.
pub fn ensure_zvec_initialized() -> Result<()> {
    let cached = ZVEC_INIT.get_or_init(|| zvec::initialize(None).map_err(|e| e.to_string()));
    cached
        .clone()
        .map_err(|e| anyhow::anyhow!("zvec global init failed: {e}"))
}

static ZVEC_GLOBAL_SHUTDOWN: OnceLock<()> = OnceLock::new();

pub(crate) fn ensure_global_shutdown() {
    if ZVEC_GLOBAL_SHUTDOWN.set(()).is_ok()
        && let Err(e) = zvec::shutdown()
    {
        error!("failed to shut down zvec library: {e}");
    }
}

const DEFAULT_VECTOR_DIM: u32 = 768;

fn build_schema(dimension: u32) -> Result<CollectionSchema> {
    // The "text" field is FTS-indexed so zvec's native full-text search (UAX #29
    // tokenizer + Snowball stemmer in C++ core v0.6.0) can perform keyword
    // search without scanning every document. `SearchQuery::fts` targets this
    // field for keyword-only retrieval.
    let fts_params = IndexParams::fts(None, None, None)?;
    CollectionSchema::builder("moltis_chunks")
        .add_field(FieldSchema::new("id", DataType::String, false, 0)?)
        .add_field(FieldSchema::new("path", DataType::String, false, 0)?)
        .add_field(FieldSchema::new("source", DataType::String, false, 0)?)
        .add_field(FieldSchema::new("start_line", DataType::Int64, false, 0)?)
        .add_field(FieldSchema::new("end_line", DataType::Int64, false, 0)?)
        .add_field(FieldSchema::new("hash", DataType::String, false, 0)?)
        .add_field(FieldSchema::new("model", DataType::String, false, 0)?)
        .add_indexed_field("text", DataType::String, fts_params)
        .add_field(FieldSchema::new("updated_at", DataType::String, false, 0)?)
        .add_field(FieldSchema::new("mtime", DataType::Int64, false, 0)?)
        .add_field(FieldSchema::new("size", DataType::Int64, false, 0)?)
        .add_vector_field(
            "embedding",
            DataType::VectorFp32,
            dimension,
            IndexParams::hnsw(MetricType::Cosine, 16, 200)?,
        )
        .build()
        .context("failed to build zvec collection schema")
}

fn collection_path(db_path: &Path, dimension: Option<u32>) -> String {
    match dimension {
        Some(dim) => format!("{}_{dim}", db_path.to_string_lossy()),
        None => db_path.to_string_lossy().into_owned(),
    }
}

pub fn initialize(db_path: &Path, dimension: Option<u32>) -> Result<Collection> {
    ensure_zvec_initialized()?;

    let path_str = collection_path(db_path, dimension);
    debug!(path = %path_str, "initializing zvec memory backend");

    match Collection::open(&path_str, None) {
        Ok(collection) => {
            debug!("opened existing zvec collection at {}", path_str);
            Ok(collection)
        },
        Err(open_err) => {
            if let Some(lock_path) = stale_lock_path(&path_str) {
                warn!(path = %path_str, "removing stale zvec lock file and retrying open");
                if let Err(e) = std::fs::remove_file(&lock_path) {
                    debug!(path = %lock_path.display(), error = %e, "failed to remove stale lock");
                }
                if let Ok(collection) = Collection::open(&path_str, None) {
                    debug!(
                        "opened zvec collection after stale-lock cleanup at {}",
                        path_str
                    );
                    return Ok(collection);
                }
                debug!(path = %path_str, "open still failed after stale-lock removal; falling through to create");
            }
            debug!(path = %path_str, error = %open_err, "open failed; creating new zvec collection");
            let dim = dimension.unwrap_or(DEFAULT_VECTOR_DIM);
            let schema = build_schema(dim)?;
            match Collection::create_and_open(&path_str, &schema, None) {
                Ok(collection) => Ok(collection),
                Err(create_err) => {
                    let create_msg = create_err.to_string().to_lowercase();
                    if create_msg.contains("exists") && is_empty_collection_dir(&path_str) {
                        debug!(
                            path = %path_str,
                            "create_and_open failed with 'exists'; removing stale empty directory and retrying"
                        );
                        if let Err(e) = std::fs::remove_dir_all(&path_str) {
                            debug!(path = %path_str, error = %e, "failed to remove stale empty directory");
                        }
                        return Collection::create_and_open(&path_str, &schema, None)
                            .with_context(|| format!(
                                "failed to create zvec collection at {path_str} after stale-directory cleanup (open also failed: {open_err}, create failed: {create_err})"
                            ));
                    }
                    Err(anyhow::Error::new(create_err).context(format!(
                        "failed to create zvec collection at {path_str} (open also failed: {open_err})"
                    )))
                },
            }
        },
    }
}

pub fn shutdown(collection: Collection) -> Result<()> {
    collection
        .flush()
        .context("failed to flush zvec collection")?;
    drop(collection);
    ensure_global_shutdown();
    debug!("zvec memory backend shut down");
    Ok(())
}

/// Open an existing zvec collection **without** any stale-lock cleanup or
/// create-and-open fallback.
///
/// zvec requires exclusive access to the collection's LOCK file, even in
/// read-only mode. If the gateway is running, this will fail with a clear
/// error. Stop the gateway first for CLI access.
pub fn open_collection(db_path: &Path, dimension: Option<u32>) -> Result<Collection> {
    ensure_zvec_initialized()?;
    let path_str = collection_path(db_path, dimension);
    Collection::open(&path_str, None).with_context(|| {
        format!(
            "failed to open zvec collection at {path_str} \
             (if the gateway is running, stop it first)"
        )
    })
}

fn stale_lock_path(collection_path: &str) -> Option<std::path::PathBuf> {
    let lock = Path::new(collection_path).join("LOCK");
    lock.exists().then_some(lock)
}

/// Returns `true` when the path is an empty or near-empty directory (contains
/// nothing, or only a `LOCK` file). This signals a collection that was killed
/// mid-`create_and_open` and can be safely removed before retrying.
fn is_empty_collection_dir(path: &str) -> bool {
    let dir = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(_) => return false,
    };
    for entry in dir.flatten() {
        if entry.file_name() != "LOCK" {
            return false;
        }
    }
    true
}

pub fn open_or_create_collection(db_path: &Path, dimension: Option<u32>) -> Result<Collection> {
    let path_str = collection_path(db_path, dimension);
    match Collection::open(&path_str, None) {
        Ok(collection) => {
            debug!("opened existing zvec collection at {}", path_str);
            Ok(collection)
        },
        Err(open_err) => {
            // If the collection directory exists from a previous run, a stale
            // LOCK file left behind by a killed process can block the open.
            // Try removing it and retrying before falling through to create.
            if let Some(lock_path) = stale_lock_path(&path_str) {
                warn!(path = %path_str, "removing stale zvec lock file and retrying open");
                if let Err(e) = std::fs::remove_file(&lock_path) {
                    debug!(path = %lock_path.display(), error = %e, "failed to remove stale lock");
                }
                if let Ok(collection) = Collection::open(&path_str, None) {
                    debug!(
                        "opened zvec collection after stale-lock cleanup at {}",
                        path_str
                    );
                    return Ok(collection);
                }
                debug!(path = %path_str, "open still failed after stale-lock removal; falling through to create");
            }
            debug!(path = %path_str, error = %open_err, "open failed; creating new zvec collection");
            let dim = dimension.unwrap_or(DEFAULT_VECTOR_DIM);
            let schema = build_schema(dim)?;
            let collection = match Collection::create_and_open(&path_str, &schema, None) {
                Ok(c) => c,
                Err(create_err) => {
                    let create_msg = create_err.to_string().to_lowercase();
                    if create_msg.contains("exists") && is_empty_collection_dir(&path_str) {
                        debug!(
                            path = %path_str,
                            "create_and_open failed with 'exists'; removing stale empty directory and retrying"
                        );
                        if let Err(e) = std::fs::remove_dir_all(&path_str) {
                            debug!(path = %path_str, error = %e, "failed to remove stale empty directory");
                        }
                        Collection::create_and_open(&path_str, &schema, None).with_context(
                            || format!(
                                "failed to create zvec collection at {path_str} after stale-directory cleanup (open also failed: {open_err}, create failed: {create_err})"
                            ),
                        )?
                    } else {
                        return Err(anyhow::Error::new(create_err).context(format!(
                            "failed to create zvec collection at {path_str} (open also failed: {open_err})"
                        )));
                    }
                },
            };
            if let Err(e) = write_dimension_meta(&collection, dim) {
                warn!(error = %e, "failed to write dimension meta doc to new collection");
            }
            Ok(collection)
        },
    }
}

pub const META_DOC_SOURCE: &str = "__meta__";
const META_DOC_PK: &str = "__moltis_dim_meta__";

pub fn write_dimension_meta(collection: &Collection, dimension: u32) -> Result<()> {
    let mut doc = Doc::new().context("failed to create zvec meta doc")?;
    doc.set_pk(META_DOC_PK);
    doc.add_string("id", META_DOC_PK)?;
    doc.add_string("path", META_DOC_PK)?;
    doc.add_string("source", META_DOC_SOURCE)?;
    doc.add_i64("start_line", dimension as i64)?;
    doc.add_i64("end_line", 0)?;
    doc.add_string("hash", "")?;
    doc.add_string("model", "")?;
    doc.add_string("text", "")?;
    doc.add_string("updated_at", "")?;
    doc.add_i64("mtime", 0)?;
    doc.add_i64("size", 0)?;
    let zero_emb = vec![0.0f32; dimension as usize];
    doc.add_vector_f32("embedding", &zero_emb)?;
    collection
        .upsert(&[&doc])
        .context("failed to upsert dimension meta doc")?;
    debug!(dimension, "wrote dimension meta doc");
    Ok(())
}

pub fn read_dimension_meta(collection: &Collection) -> Result<Option<u32>> {
    let docs = collection
        .fetch(&[META_DOC_PK])
        .context("failed to fetch dimension meta doc")?;
    for doc in &docs {
        let source = doc
            .get_string("source")
            .context("failed to get source field from meta doc")?
            .unwrap_or_default();
        if source == META_DOC_SOURCE {
            let dim = doc
                .get_i64("start_line")
                .context("failed to get start_line from meta doc")?
                .unwrap_or(0);
            if dim > 0 {
                return Ok(Some(dim as u32));
            }
            return Ok(None);
        }
    }
    Ok(None)
}

pub fn flush_collection(collection: &Collection) -> Result<()> {
    collection
        .flush()
        .context("failed to flush zvec collection")
}

#[cfg(test)]
pub(crate) use tests::TestGuard;

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{chunks, files::get_chunk_by_id},
    };

    pub(crate) struct TestGuard {
        pub collection: Collection,
        _dir: tempfile::TempDir,
    }

    impl TestGuard {
        pub fn new() -> Self {
            ensure_zvec_initialized().unwrap();
            let dir = tempfile::tempdir().unwrap();
            let collection =
                open_or_create_collection(dir.path().join("db").as_path(), Some(768)).unwrap();
            Self {
                collection,
                _dir: dir,
            }
        }
    }

    impl Drop for TestGuard {
        fn drop(&mut self) {
            if let Err(e) = self.collection.flush() {
                eprintln!("zvec collection flush error during TestGuard drop: {e}");
            }
        }
    }

    impl std::ops::Deref for TestGuard {
        type Target = Collection;

        fn deref(&self) -> &Self::Target {
            &self.collection
        }
    }

    fn temp_db_path() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test-collection");
        (dir, path)
    }

    #[test]
    fn test_ensure_zvec_initialized_idempotent() {
        ensure_zvec_initialized().unwrap();
        ensure_zvec_initialized().unwrap();
    }

    #[test]
    fn test_collection_path_with_dimension() {
        let path = Path::new("/tmp/db");
        let result = collection_path(path, Some(768));
        assert_eq!(result, "/tmp/db_768");
    }

    #[test]
    fn test_collection_path_without_dimension() {
        let path = Path::new("/tmp/db");
        let result = collection_path(path, None);
        assert_eq!(result, "/tmp/db");
    }

    #[test]
    fn test_open_or_create_collection_creates_new() {
        ensure_zvec_initialized().unwrap();
        let (_dir, path) = temp_db_path();
        let collection = open_or_create_collection(&path, Some(768)).unwrap();
        let chunk = chunks::ChunkDoc {
            id: "oc-t1".into(),
            path: "p".into(),
            source: "s".into(),
            start_line: 1,
            end_line: 2,
            hash: "h".into(),
            model: "m".into(),
            text: "t".into(),
            embedding: vec![0.0f32; 768],
            updated_at: "2025-01-01T00:00:00Z".into(),
            mtime: 0,
            size: 0,
        };
        chunks::upsert_chunks(&collection, &[chunk]).unwrap();
        collection.flush().unwrap();
        drop(collection);
    }

    #[test]
    fn test_open_or_create_collection_reopens_existing() {
        ensure_zvec_initialized().unwrap();
        let (_dir, path) = temp_db_path();
        {
            let collection = open_or_create_collection(&path, Some(768)).unwrap();
            let chunk = chunks::ChunkDoc {
                id: "reopen-1".into(),
                path: "p".into(),
                source: "s".into(),
                start_line: 1,
                end_line: 2,
                hash: "h".into(),
                model: "m".into(),
                text: "data".into(),
                embedding: vec![0.0f32; 768],
                updated_at: "2025-01-01T00:00:00Z".into(),
                mtime: 0,
                size: 0,
            };
            chunks::upsert_chunks(&collection, &[chunk]).unwrap();
            collection.flush().unwrap();
        }
        {
            let collection = open_or_create_collection(&path, Some(768)).unwrap();
            let fetched = get_chunk_by_id(&collection, "reopen-1").unwrap();
            assert!(fetched.is_some(), "data must persist across reopen");
            assert_eq!(fetched.unwrap().text, "data");
            collection.flush().unwrap();
        }
    }

    #[test]
    fn test_build_schema_default_dim() {
        let schema = build_schema(DEFAULT_VECTOR_DIM).unwrap();
        assert_eq!(schema.name(), "moltis_chunks");
    }

    #[test]
    fn test_initialize_then_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("init-shutdown");
        let collection = initialize(&path, Some(768)).unwrap();
        let chunk = chunks::ChunkDoc {
            id: "init-s1".into(),
            path: "p".into(),
            source: "s".into(),
            start_line: 1,
            end_line: 2,
            hash: "h".into(),
            model: "m".into(),
            text: "init test".into(),
            embedding: vec![0.0f32; 768],
            updated_at: "2025-01-01T00:00:00Z".into(),
            mtime: 0,
            size: 0,
        };
        chunks::upsert_chunks(&collection, &[chunk]).unwrap();
        shutdown(collection).unwrap();
    }

    #[test]
    fn test_write_and_read_dimension_meta() {
        let guard = TestGuard::new();
        write_dimension_meta(&guard, 768).unwrap();
        let dim = read_dimension_meta(&guard).unwrap();
        assert_eq!(dim, Some(768), "meta doc must be readable after write");
    }

    #[test]
    fn test_read_dimension_meta_none_when_missing() {
        ensure_zvec_initialized().unwrap();
        let (_dir, path) = temp_db_path();
        let schema = build_schema(768).unwrap();
        let collection =
            Collection::create_and_open(&collection_path(&path, Some(768)), &schema, None).unwrap();
        let dim = read_dimension_meta(&collection).unwrap();
        assert_eq!(dim, None, "missing meta doc must return None");
        collection.flush().unwrap();
        drop(collection);
    }

    #[test]
    fn test_open_or_create_collection_writes_meta_doc() {
        ensure_zvec_initialized().unwrap();
        let (_dir, path) = temp_db_path();
        let collection = open_or_create_collection(&path, Some(768)).unwrap();
        let dim = read_dimension_meta(&collection).unwrap();
        assert_eq!(dim, Some(768), "new collection must have meta doc");
        collection.flush().unwrap();
        drop(collection);
    }

    #[test]
    fn test_open_or_create_collection_recovers_from_stale_lock() {
        ensure_zvec_initialized().unwrap();
        let (_dir, path) = temp_db_path();

        // Create the collection normally.
        {
            let collection = open_or_create_collection(&path, Some(768)).unwrap();
            let chunk = chunks::ChunkDoc {
                id: "stale-lock-1".into(),
                path: "p".into(),
                source: "s".into(),
                start_line: 1,
                end_line: 2,
                hash: "h".into(),
                model: "m".into(),
                text: "stale lock recovery data".into(),
                embedding: vec![0.0f32; 768],
                updated_at: "2025-01-01T00:00:00Z".into(),
                mtime: 0,
                size: 0,
            };
            chunks::upsert_chunks(&collection, &[chunk]).unwrap();
            collection.flush().unwrap();
        }

        // Simulate a stale LOCK left behind by a killed process.
        let coll_dir = collection_path(&path, Some(768));
        let lock_path = Path::new(&coll_dir).join("LOCK");
        std::fs::write(&lock_path, b"").unwrap();
        assert!(lock_path.exists(), "stale LOCK must exist before reopen");

        // Reopen — should remove the stale lock and successfully open the
        // existing collection (not overwrite it).
        let collection = open_or_create_collection(&path, Some(768)).unwrap();
        let fetched = get_chunk_by_id(&collection, "stale-lock-1").unwrap();
        assert!(
            fetched.is_some(),
            "existing data must survive stale-lock recovery"
        );
        assert_eq!(fetched.unwrap().text, "stale lock recovery data");
        collection.flush().unwrap();
        drop(collection);
    }

    /// When a previous `create_and_open` is killed mid-init, the collection
    /// directory exists but contains no valid data.  `open` fails because the
    /// directory is not a valid collection; `create_and_open` fails because the
    /// directory already exists.  `open_or_create_collection` must detect this,
    /// clean up the stale empty directory, and retry `create_and_open`.
    #[test]
    fn test_open_or_create_collection_recovers_from_stale_empty_dir() {
        ensure_zvec_initialized().unwrap();
        let (_dir, path) = temp_db_path();
        let coll_dir = collection_path(&path, Some(768));

        // Simulate a killed create_and_open: create an empty directory where
        // the collection would live, with nothing inside.
        std::fs::create_dir_all(&coll_dir).unwrap();
        assert!(Path::new(&coll_dir).is_dir());

        // open_or_create_collection must detect the stale empty dir, remove it,
        // and create a fresh collection.
        let collection = open_or_create_collection(&path, Some(768)).unwrap();

        // Verify the collection is functional.
        let chunk = chunks::ChunkDoc {
            id: "stale-dir-1".into(),
            path: "p".into(),
            source: "s".into(),
            start_line: 1,
            end_line: 2,
            hash: "h".into(),
            model: "m".into(),
            text: "recovered from stale empty directory".into(),
            embedding: vec![0.0f32; 768],
            updated_at: "2025-01-01T00:00:00Z".into(),
            mtime: 0,
            size: 0,
        };
        chunks::upsert_chunks(&collection, &[chunk]).unwrap();
        collection.flush().unwrap();
        let fetched = get_chunk_by_id(&collection, "stale-dir-1").unwrap();
        assert!(
            fetched.is_some(),
            "fresh collection after stale-dir recovery must accept writes"
        );
        assert_eq!(
            fetched.unwrap().text,
            "recovered from stale empty directory"
        );
        collection.flush().unwrap();
        drop(collection);
    }

    /// The stale-dir guard must NOT trigger for a non-empty directory — that
    /// might be a valid zvec collection, and removing it would cause data loss.
    #[test]
    fn test_open_or_create_collection_does_not_remove_nonempty_dir() {
        ensure_zvec_initialized().unwrap();
        let (_dir, path) = temp_db_path();

        // Create a valid collection with data.
        {
            let collection = open_or_create_collection(&path, Some(768)).unwrap();
            let chunk = chunks::ChunkDoc {
                id: "valid-1".into(),
                path: "p".into(),
                source: "s".into(),
                start_line: 1,
                end_line: 2,
                hash: "h".into(),
                model: "m".into(),
                text: "valid data".into(),
                embedding: vec![0.0f32; 768],
                updated_at: "2025-01-01T00:00:00Z".into(),
                mtime: 0,
                size: 0,
            };
            chunks::upsert_chunks(&collection, &[chunk]).unwrap();
            collection.flush().unwrap();
        }

        // Reopen — must not attempt to remove the non-empty directory.
        let collection = open_or_create_collection(&path, Some(768)).unwrap();
        let fetched = get_chunk_by_id(&collection, "valid-1").unwrap();
        assert!(
            fetched.is_some(),
            "valid data must survive reopen (non-empty dir must not be removed)"
        );
        assert_eq!(fetched.unwrap().text, "valid data");
        collection.flush().unwrap();
        drop(collection);
    }

    #[test]
    fn test_initialize_reopens_existing_collection() {
        // Covers the Collection::open Ok-branch (line ~79-81) of initialize():
        // a second call on an already-created collection must open it rather
        // than recreate it.
        ensure_zvec_initialized().unwrap();
        let (_dir, path) = temp_db_path();
        {
            let collection = initialize(&path, Some(768)).unwrap();
            let chunk = chunks::ChunkDoc {
                id: "init-reopen-1".into(),
                path: "p".into(),
                source: "s".into(),
                start_line: 1,
                end_line: 2,
                hash: "h".into(),
                model: "m".into(),
                text: "persist across initialize reopen".into(),
                embedding: vec![0.0f32; 768],
                updated_at: "2025-01-01T00:00:00Z".into(),
                mtime: 0,
                size: 0,
            };
            chunks::upsert_chunks(&collection, &[chunk]).unwrap();
            collection.flush().unwrap();
        }
        {
            let collection = initialize(&path, Some(768)).unwrap();
            let fetched = get_chunk_by_id(&collection, "init-reopen-1")
                .unwrap()
                .expect("data must persist across initialize() reopen");
            assert_eq!(fetched.text, "persist across initialize reopen");
            collection.flush().unwrap();
        }
    }

    #[test]
    fn test_initialize_recovers_from_stale_lock() {
        // Covers initialize() stale-lock recovery branch (lines ~85-96).
        ensure_zvec_initialized().unwrap();
        let (_dir, path) = temp_db_path();
        {
            let collection = initialize(&path, Some(768)).unwrap();
            let chunk = chunks::ChunkDoc {
                id: "init-lock-1".into(),
                path: "p".into(),
                source: "s".into(),
                start_line: 1,
                end_line: 2,
                hash: "h".into(),
                model: "m".into(),
                text: "init stale lock data".into(),
                embedding: vec![0.0f32; 768],
                updated_at: "2025-01-01T00:00:00Z".into(),
                mtime: 0,
                size: 0,
            };
            chunks::upsert_chunks(&collection, &[chunk]).unwrap();
            collection.flush().unwrap();
        }

        let coll_dir = collection_path(&path, Some(768));
        let lock_path = Path::new(&coll_dir).join("LOCK");
        std::fs::write(&lock_path, b"").unwrap();
        assert!(lock_path.exists());

        let collection = initialize(&path, Some(768)).unwrap();
        let fetched = get_chunk_by_id(&collection, "init-lock-1")
            .unwrap()
            .expect("data must survive stale-lock recovery in initialize()");
        assert_eq!(fetched.text, "init stale lock data");
        collection.flush().unwrap();
        drop(collection);
    }

    #[test]
    fn test_initialize_recovers_from_stale_empty_dir() {
        // Covers initialize() stale-empty-directory recovery branch
        // (lines ~105-116): create_and_open fails with "exists" on a dir that
        // only contains nothing/a LOCK, so it must be removed and retried.
        ensure_zvec_initialized().unwrap();
        let (_dir, path) = temp_db_path();
        let coll_dir = collection_path(&path, Some(768));

        std::fs::create_dir_all(&coll_dir).unwrap();
        assert!(Path::new(&coll_dir).is_dir());

        let collection = initialize(&path, Some(768)).unwrap();
        let chunk = chunks::ChunkDoc {
            id: "init-stale-dir-1".into(),
            path: "p".into(),
            source: "s".into(),
            start_line: 1,
            end_line: 2,
            hash: "h".into(),
            model: "m".into(),
            text: "init recovered from stale empty dir".into(),
            embedding: vec![0.0f32; 768],
            updated_at: "2025-01-01T00:00:00Z".into(),
            mtime: 0,
            size: 0,
        };
        chunks::upsert_chunks(&collection, &[chunk]).unwrap();
        collection.flush().unwrap();
        drop(collection);
    }

    #[test]
    fn test_open_collection_opens_existing() {
        // Covers open_collection() (lines ~143-152): after a collection is
        // created and dropped, open_collection must reopen it read-only.
        ensure_zvec_initialized().unwrap();
        let (_dir, path) = temp_db_path();
        {
            let collection = open_or_create_collection(&path, Some(768)).unwrap();
            let chunk = chunks::ChunkDoc {
                id: "open-coll-1".into(),
                path: "p".into(),
                source: "s".into(),
                start_line: 1,
                end_line: 2,
                hash: "h".into(),
                model: "m".into(),
                text: "open_collection readable".into(),
                embedding: vec![0.0f32; 768],
                updated_at: "2025-01-01T00:00:00Z".into(),
                mtime: 0,
                size: 0,
            };
            chunks::upsert_chunks(&collection, &[chunk]).unwrap();
            collection.flush().unwrap();
        }

        let collection = open_collection(&path, Some(768)).unwrap();
        let fetched = get_chunk_by_id(&collection, "open-coll-1")
            .unwrap()
            .expect("data must be readable via open_collection");
        assert_eq!(fetched.text, "open_collection readable");
        collection.flush().unwrap();
        drop(collection);
    }

    #[test]
    fn test_open_collection_missing_path_errors() {
        // open_collection must NOT create; a non-existent path must error.
        ensure_zvec_initialized().unwrap();
        let (_dir, path) = temp_db_path();
        let result = open_collection(&path, Some(768));
        assert!(
            result.is_err(),
            "open_collection must fail on a non-existent collection"
        );
    }

    #[test]
    fn test_is_empty_collection_dir_branches() {
        let (_dir, path) = temp_db_path();
        let coll_dir = collection_path(&path, Some(768));

        // Non-existent directory → false.
        assert!(
            !is_empty_collection_dir(&coll_dir),
            "missing dir must not be treated as empty"
        );

        // Empty directory → true.
        std::fs::create_dir_all(&coll_dir).unwrap();
        assert!(
            is_empty_collection_dir(&coll_dir),
            "truly empty dir must be treated as empty"
        );

        // Directory containing only a LOCK file → true.
        std::fs::write(Path::new(&coll_dir).join("LOCK"), b"").unwrap();
        assert!(
            is_empty_collection_dir(&coll_dir),
            "dir with only a LOCK file must be treated as empty"
        );

        // Directory containing any other file → false.
        std::fs::write(Path::new(&coll_dir).join("CURRENT"), b"x").unwrap();
        assert!(
            !is_empty_collection_dir(&coll_dir),
            "dir with a non-LOCK file must NOT be treated as empty"
        );
    }

    #[test]
    fn test_read_dimension_meta_none_when_zero() {
        // Covers the dim==0 → Ok(None) arm of read_dimension_meta (~line 278):
        // a meta doc whose start_line is 0 must yield None.
        ensure_zvec_initialized().unwrap();
        let (_dir, path) = temp_db_path();
        let schema = build_schema(768).unwrap();
        let collection =
            Collection::create_and_open(&collection_path(&path, Some(768)), &schema, None).unwrap();

        // Manually upsert a meta doc with start_line (dimension) == 0.
        let mut doc = Doc::new().unwrap();
        doc.set_pk(META_DOC_PK);
        doc.add_string("id", META_DOC_PK).unwrap();
        doc.add_string("path", META_DOC_PK).unwrap();
        doc.add_string("source", META_DOC_SOURCE).unwrap();
        doc.add_i64("start_line", 0).unwrap();
        doc.add_i64("end_line", 0).unwrap();
        doc.add_string("hash", "").unwrap();
        doc.add_string("model", "").unwrap();
        doc.add_string("text", "").unwrap();
        doc.add_string("updated_at", "").unwrap();
        doc.add_i64("mtime", 0).unwrap();
        doc.add_i64("size", 0).unwrap();
        doc.add_vector_f32("embedding", &vec![0.0f32; 768]).unwrap();
        collection.upsert(&[&doc]).unwrap();

        let dim = read_dimension_meta(&collection).unwrap();
        assert_eq!(dim, None, "meta doc with dimension 0 must resolve to None");

        collection.flush().unwrap();
        drop(collection);
    }
}
