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
    let fts_params =
        IndexParams::fts(None, None, None).context("failed to create FTS index params")?;
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
                debug!(path = %path_str, "removing stale zvec lock file and retrying open");
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
            Collection::create_and_open(&path_str, &schema, None).with_context(|| {
                format!(
                    "failed to create zvec collection at {path_str} (open also failed: {open_err})"
                )
            })
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

fn stale_lock_path(collection_path: &str) -> Option<std::path::PathBuf> {
    let lock = Path::new(collection_path).join("LOCK");
    lock.exists().then_some(lock)
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
                debug!(path = %path_str, "removing stale zvec lock file and retrying open");
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
            let collection = Collection::create_and_open(&path_str, &schema, None)
                .with_context(|| {
                    format!(
                        "failed to create zvec collection at {path_str} (open also failed: {open_err})"
                    )
                })?;
            if let Err(e) = write_dimension_meta(&collection, dim) {
                warn!(error = %e, "failed to write dimension meta doc to new collection");
            }
            Ok(collection)
        },
    }
}

const META_DOC_PK: &str = "__moltis_dim_meta__";

pub fn write_dimension_meta(collection: &Collection, dimension: u32) -> Result<()> {
    let mut doc = Doc::new().context("failed to create zvec meta doc")?;
    doc.set_pk(META_DOC_PK);
    doc.add_string("id", META_DOC_PK)?;
    doc.add_string("path", META_DOC_PK)?;
    doc.add_string("source", "__meta__")?;
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
        if source == "__meta__" {
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
}
