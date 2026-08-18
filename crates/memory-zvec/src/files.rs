use {
    anyhow::{Context, Result},
    moltis_memory::schema::FileRow,
    zvec::{Collection, Doc, SearchQuery},
};

use crate::chunks::ChunkDoc;

/// Sentinel stored in the `source` field to mark a document as a file marker
/// (as opposed to a chunk or the dimension-meta doc). The file's real `source`
/// value is stored in the `model` field, because `source` is reserved as this
/// discriminator.
pub(crate) const FILE_MARKER: &str = "__file__";

/// zvec has no filter-only query path: listing relies on a vector search with a
/// dummy vector constrained by a filter. This caps the result set of such scans.
pub(crate) const LIST_TOPK: i32 = 100_000;

fn zero_vector(dimension: u32) -> Vec<f32> {
    vec![0.0f32; dimension as usize]
}

/// A small nonzero vector for filter-based listing queries. HNSW needs a
/// meaningful query vector to navigate; the filter does the actual selection.
/// Listing reliability depends on `LIST_TOPK` exceeding the collection size.
pub(crate) fn listing_query_vector(dimension: u32) -> Vec<f32> {
    let mut v = vec![0.0f32; dimension as usize];
    v[0] = 1.0;
    v
}

/// Upsert a durable file-marker document keyed by `file.path`.
///
/// The real `FileRow.source` is persisted in the `model` field (since `source`
/// holds the `__file__` discriminator), so file metadata survives restarts.
pub fn upsert_file(collection: &Collection, file: &FileRow, dimension: u32) -> Result<()> {
    let mut doc = Doc::new().context("failed to create zvec doc for file")?;
    doc.set_pk(&file.path);
    doc.add_string("id", &file.path)?;
    doc.add_string("path", &file.path)?;
    doc.add_string("source", FILE_MARKER)?;
    doc.add_i64("start_line", 0)?;
    doc.add_i64("end_line", 0)?;
    doc.add_string("hash", &file.hash)?;
    doc.add_string("model", &file.source)?;
    doc.add_string("text", "")?;
    doc.add_string("updated_at", &file.mtime.to_string())?;
    doc.add_i64("mtime", file.mtime)?;
    doc.add_i64("size", file.size)?;
    doc.add_vector_f32("embedding", &zero_vector(dimension))?;
    collection
        .upsert(&[&doc])
        .context("failed to upsert file doc")?;
    collection
        .flush()
        .context("failed to flush after upserting file doc")?;
    Ok(())
}

/// Fetch a file marker by path, returning the full round-tripped [`FileRow`].
pub fn get_file(collection: &Collection, path: &str) -> Result<Option<FileRow>> {
    let docs = collection
        .fetch(&[path])
        .context("failed to fetch file doc")?;
    for doc in &docs {
        let source = doc
            .get_string("source")
            .context("failed to get source field")?
            .unwrap_or_default();
        if source == FILE_MARKER {
            return Ok(Some(FileRow {
                path: doc
                    .get_string("path")
                    .context("failed to get path field")?
                    .unwrap_or_default(),
                // Real source is stored in `model` (see [`upsert_file`]).
                source: doc
                    .get_string("model")
                    .context("failed to get model (file source) field")?
                    .unwrap_or_default(),
                hash: doc
                    .get_string("hash")
                    .context("failed to get hash field")?
                    .unwrap_or_default(),
                mtime: doc
                    .get_i64("mtime")
                    .context("failed to get mtime field")?
                    .unwrap_or(0),
                size: doc
                    .get_i64("size")
                    .context("failed to get size field")?
                    .unwrap_or(0),
            }));
        }
    }
    Ok(None)
}

/// List all file markers as full [`FileRow`]s (durable).
pub fn list_file_rows(collection: &Collection, dimension: u32) -> Result<Vec<FileRow>> {
    let filter = format!("source = '{}'", FILE_MARKER);
    let query = SearchQuery::builder()
        .field_name("embedding")
        .vector(&listing_query_vector(dimension))
        .topk(LIST_TOPK)
        .filter(&filter)
        .output_fields(&["path", "source", "model", "hash", "mtime", "size"])
        .build()
        .context("failed to build list-files query")?;

    let results = collection
        .query(&query)
        .context("failed to query file markers")?;

    let mut rows: Vec<FileRow> = results
        .iter()
        .filter(|doc| doc.get_string("source").ok().flatten().as_deref() == Some(FILE_MARKER))
        .map(|doc| {
            Ok(FileRow {
                path: doc
                    .get_string("path")
                    .context("failed to get path")?
                    .unwrap_or_default(),
                source: doc
                    .get_string("model")
                    .context("failed to get model (file source)")?
                    .unwrap_or_default(),
                hash: doc
                    .get_string("hash")
                    .context("failed to get hash")?
                    .unwrap_or_default(),
                mtime: doc
                    .get_i64("mtime")
                    .context("failed to get mtime")?
                    .unwrap_or(0),
                size: doc
                    .get_i64("size")
                    .context("failed to get size")?
                    .unwrap_or(0),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    rows.sort_by(|a, b| a.path.cmp(&b.path));
    rows.dedup_by(|a, b| a.path == b.path);
    Ok(rows)
}

/// List all file-marker paths (convenience over [`list_file_rows`]).
pub fn list_files(collection: &Collection, dimension: u32) -> Result<Vec<String>> {
    list_file_rows(collection, dimension).map(|rows| rows.into_iter().map(|f| f.path).collect())
}

pub fn delete_file(collection: &Collection, path: &str) -> Result<()> {
    crate::chunks::delete_chunks_for_file(collection, path)
        .context("failed to delete chunks for file")?;

    collection
        .delete(&[path])
        .context("failed to delete file doc")?;
    Ok(())
}

pub fn get_chunks_for_file(
    collection: &Collection,
    path: &str,
    dimension: u32,
) -> Result<Vec<ChunkDoc>> {
    let filter = format!("path = '{}'", crate::escape_filter_value(path));
    let query = SearchQuery::builder()
        .field_name("embedding")
        .vector(&listing_query_vector(dimension))
        .topk(LIST_TOPK)
        .filter(&filter)
        .output_fields(&[
            "id",
            "path",
            "source",
            "start_line",
            "end_line",
            "hash",
            "model",
            "text",
            "updated_at",
            "mtime",
            "size",
        ])
        .build()
        .context("failed to build get-chunks-for-file query")?;

    let results = collection
        .query(&query)
        .context("failed to query chunks for file")?;

    let chunks: Vec<ChunkDoc> = results
        .iter()
        .filter(|doc| {
            let source = doc.get_string("source").ok().flatten();
            source.as_deref() != Some(FILE_MARKER)
        })
        .map(|doc| {
            Ok(ChunkDoc {
                id: doc
                    .get_string("id")
                    .context("failed to get id")?
                    .unwrap_or_default(),
                path: doc
                    .get_string("path")
                    .context("failed to get path")?
                    .unwrap_or_default(),
                source: doc
                    .get_string("source")
                    .context("failed to get source")?
                    .unwrap_or_default(),
                start_line: doc
                    .get_i64("start_line")
                    .context("failed to get start_line")?
                    .unwrap_or(0),
                end_line: doc
                    .get_i64("end_line")
                    .context("failed to get end_line")?
                    .unwrap_or(0),
                hash: doc
                    .get_string("hash")
                    .context("failed to get hash")?
                    .unwrap_or_default(),
                model: doc
                    .get_string("model")
                    .context("failed to get model")?
                    .unwrap_or_default(),
                text: doc
                    .get_string("text")
                    .context("failed to get text")?
                    .unwrap_or_default(),
                embedding: doc
                    .get_vector_f32("embedding")
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
                updated_at: doc
                    .get_string("updated_at")
                    .context("failed to get updated_at")?
                    .unwrap_or_default(),
                mtime: doc
                    .get_i64("mtime")
                    .context("failed to get mtime")?
                    .unwrap_or(0),
                size: doc
                    .get_i64("size")
                    .context("failed to get size")?
                    .unwrap_or(0),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(chunks)
}

pub fn get_chunk_by_id(collection: &Collection, id: &str) -> Result<Option<ChunkDoc>> {
    let pk = ChunkDoc::safe_pk(id);
    let docs = collection
        .fetch(&[&pk])
        .context("failed to fetch chunk by id")?;

    if let Some(doc) = docs.first() {
        let source = doc
            .get_string("source")
            .context("failed to get source")?
            .unwrap_or_default();
        if source == FILE_MARKER || source == crate::collection::META_DOC_SOURCE {
            return Ok(None);
        }
        Ok(Some(ChunkDoc {
            id: doc
                .get_string("id")
                .context("failed to get id")?
                .unwrap_or_default(),
            path: doc
                .get_string("path")
                .context("failed to get path")?
                .unwrap_or_default(),
            source,
            start_line: doc
                .get_i64("start_line")
                .context("failed to get start_line")?
                .unwrap_or(0),
            end_line: doc
                .get_i64("end_line")
                .context("failed to get end_line")?
                .unwrap_or(0),
            hash: doc
                .get_string("hash")
                .context("failed to get hash")?
                .unwrap_or_default(),
            model: doc
                .get_string("model")
                .context("failed to get model")?
                .unwrap_or_default(),
            text: doc
                .get_string("text")
                .context("failed to get text")?
                .unwrap_or_default(),
            embedding: doc
                .get_vector_f32("embedding")
                .context("failed to get embedding")?
                .unwrap_or_default(),
            updated_at: doc
                .get_string("updated_at")
                .context("failed to get updated_at")?
                .unwrap_or_default(),
            mtime: doc
                .get_i64("mtime")
                .context("failed to get mtime")?
                .unwrap_or(0),
            size: doc
                .get_i64("size")
                .context("failed to get size")?
                .unwrap_or(0),
        }))
    } else {
        Ok(None)
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, crate::collection::TestGuard};

    const DIM: u32 = 768;

    fn nonzero_embedding() -> Vec<f32> {
        let mut v = vec![0.0f32; DIM as usize];
        v[0] = 1.0;
        v
    }

    fn file_row(path: &str, mtime: i64, size: i64) -> FileRow {
        FileRow {
            path: path.into(),
            source: "daily".into(),
            hash: format!("hash-{path}"),
            mtime,
            size,
        }
    }

    #[test]
    fn test_upsert_and_get_file() {
        let guard = TestGuard::new();
        upsert_file(&guard, &file_row("test-file.md", 1_700_000_000, 1234), DIM).unwrap();

        let result = get_file(&guard, "test-file.md").unwrap();
        assert!(result.is_some(), "must retrieve upserted file marker");
        let file = result.unwrap();
        assert_eq!(file.path, "test-file.md");
        assert_eq!(file.source, "daily", "real source must round-trip");
        assert_eq!(file.hash, "hash-test-file.md");
        assert_eq!(file.mtime, 1_700_000_000);
        assert_eq!(file.size, 1234);
    }

    #[test]
    fn test_get_file_nonexistent() {
        let guard = TestGuard::new();
        let result = get_file(&guard, "nonexistent.md").unwrap();
        assert!(result.is_none(), "nonexistent file must return None");
    }

    #[test]
    fn test_list_files_no_duplicates() {
        let guard = TestGuard::new();
        upsert_file(&guard, &file_row("b.md", 0, 0), DIM).unwrap();
        upsert_file(&guard, &file_row("a.md", 0, 0), DIM).unwrap();
        upsert_file(&guard, &file_row("a.md", 1, 0), DIM).unwrap();

        let files = list_files(&guard, DIM).unwrap();
        let deduped: Vec<_> = {
            let mut v = files.clone();
            v.sort();
            v.dedup();
            v
        };
        assert_eq!(files, deduped, "list_files must not return duplicates");
    }

    #[test]
    fn test_list_files_empty() {
        let guard = TestGuard::new();
        let files = list_files(&guard, DIM).unwrap();
        assert!(files.is_empty(), "empty collection must return empty list");
    }

    #[test]
    fn test_delete_file_removes_file_marker() {
        let guard = TestGuard::new();
        upsert_file(&guard, &file_row("to-delete.md", 0, 0), DIM).unwrap();
        assert!(get_file(&guard, "to-delete.md").unwrap().is_some());

        delete_file(&guard, "to-delete.md").unwrap();
        assert!(get_file(&guard, "to-delete.md").unwrap().is_none());
    }

    #[test]
    fn test_delete_file_also_deletes_chunks() {
        let guard = TestGuard::new();
        upsert_file(&guard, &file_row("with-chunks.md", 0, 0), DIM).unwrap();

        let chunk = ChunkDoc {
            id: "chunk-df-1".into(),
            path: "with-chunks.md".into(),
            source: "test".into(),
            start_line: 1,
            end_line: 5,
            hash: "hash1".into(),
            model: "model1".into(),
            text: "hello".into(),
            embedding: nonzero_embedding(),
            updated_at: "2025-01-01T00:00:00Z".into(),
            mtime: 0,
            size: 0,
        };
        crate::chunks::upsert_chunks(&guard, &[chunk]).unwrap();

        assert!(
            get_chunk_by_id(&guard, "chunk-df-1").unwrap().is_some(),
            "chunk must exist before file delete"
        );

        delete_file(&guard, "with-chunks.md").unwrap();

        assert!(
            get_file(&guard, "with-chunks.md").unwrap().is_none(),
            "file marker must be deleted"
        );
    }

    #[test]
    fn test_get_chunks_for_file_with_chunks() {
        let guard = TestGuard::new();

        let chunk = ChunkDoc {
            id: "chunk-gcf-1".into(),
            path: "file-with-chunks.md".into(),
            source: "test".into(),
            start_line: 1,
            end_line: 10,
            hash: "hash-a".into(),
            model: "m".into(),
            text: "actual chunk text".into(),
            embedding: nonzero_embedding(),
            updated_at: "2025-01-01T00:00:00Z".into(),
            mtime: 0,
            size: 0,
        };
        crate::chunks::upsert_chunks(&guard, &[chunk]).unwrap();

        let fetched = get_chunk_by_id(&guard, "chunk-gcf-1").unwrap();
        assert!(
            fetched.is_some(),
            "chunk must be fetchable by PK after upsert"
        );
        assert_eq!(fetched.unwrap().text, "actual chunk text");
    }

    #[test]
    fn test_get_chunks_for_file_filters_out_file_markers() {
        let guard = TestGuard::new();
        upsert_file(&guard, &file_row("has-file-marker.md", 0, 0), DIM).unwrap();

        let chunks = get_chunks_for_file(&guard, "has-file-marker.md", DIM).unwrap();
        assert!(
            chunks.is_empty(),
            "file markers must be filtered out, got {} chunks",
            chunks.len()
        );
    }

    #[test]
    fn test_get_chunk_by_id() {
        let guard = TestGuard::new();

        let chunk = ChunkDoc {
            id: "chunk-gbi-1".into(),
            path: "p.md".into(),
            source: "test".into(),
            start_line: 1,
            end_line: 3,
            hash: "h1".into(),
            model: "m1".into(),
            text: "chunk content".into(),
            embedding: vec![0.0f32; DIM as usize],
            updated_at: "2025-01-01T00:00:00Z".into(),
            mtime: 0,
            size: 0,
        };
        crate::chunks::upsert_chunks(&guard, &[chunk]).unwrap();

        let result = get_chunk_by_id(&guard, "chunk-gbi-1").unwrap();
        assert!(result.is_some());
        let fetched = result.unwrap();
        assert_eq!(fetched.id, "chunk-gbi-1");
        assert_eq!(fetched.text, "chunk content");
    }

    #[test]
    fn test_get_chunk_by_id_nonexistent() {
        let guard = TestGuard::new();
        let result = get_chunk_by_id(&guard, "no-such-chunk").unwrap();
        assert!(result.is_none(), "nonexistent chunk must return None");
    }

    #[test]
    fn test_get_chunk_by_id_filters_file_marker() {
        let guard = TestGuard::new();
        upsert_file(&guard, &file_row("marker-file.md", 0, 0), DIM).unwrap();

        let result = get_chunk_by_id(&guard, "marker-file.md").unwrap();
        assert!(
            result.is_none(),
            "file marker with __file__ source must return None from get_chunk_by_id"
        );
    }

    /// Exercises the META_DOC_SOURCE arm of get_chunk_by_id (~line 271):
    /// fetching the dimension-meta doc by its PK must return None, not a
    /// spurious chunk.
    #[test]
    fn test_get_chunk_by_id_filters_meta_doc() {
        let guard = TestGuard::new();
        // TestGuard::new creates the collection via open_or_create_collection,
        // which writes the dimension-meta doc (source == META_DOC_SOURCE) under
        // the PK "__moltis_dim_meta__".
        let result = get_chunk_by_id(&guard, "__moltis_dim_meta__").unwrap();
        assert!(
            result.is_none(),
            "dimension-meta doc must return None from get_chunk_by_id"
        );
    }

    /// Exercises the full `.map()` body of get_chunks_for_file (lines ~203-253)
    /// with real, non-marker chunks: existing tests only query file markers,
    /// which are filtered out before the map ever runs.
    #[test]
    fn test_get_chunks_for_file_returns_real_chunks() {
        let guard = TestGuard::new();
        // A file marker for the same path — must be filtered out, not mapped.
        upsert_file(&guard, &file_row("multi-chunk.md", 0, 0), DIM).unwrap();

        let emb = nonzero_embedding();
        let chunks = vec![
            ChunkDoc {
                id: "mc-1".into(),
                path: "multi-chunk.md".into(),
                source: "daily".into(),
                start_line: 1,
                end_line: 5,
                hash: "h1".into(),
                model: "m".into(),
                text: "first chunk".into(),
                embedding: emb.clone(),
                updated_at: "2026-01-01T00:00:00Z".into(),
                mtime: 100,
                size: 200,
            },
            ChunkDoc {
                id: "mc-2".into(),
                path: "multi-chunk.md".into(),
                source: "daily".into(),
                start_line: 6,
                end_line: 10,
                hash: "h2".into(),
                model: "m".into(),
                text: "second chunk".into(),
                embedding: emb.clone(),
                updated_at: "2026-01-02T00:00:00Z".into(),
                mtime: 300,
                size: 400,
            },
        ];
        crate::chunks::upsert_chunks(&guard, &chunks).unwrap();

        // A chunk for a different file — must NOT appear in this file's list.
        let other = ChunkDoc {
            id: "other-1".into(),
            path: "other.md".into(),
            source: "daily".into(),
            start_line: 1,
            end_line: 1,
            hash: "ho".into(),
            model: "m".into(),
            text: "other file chunk".into(),
            embedding: emb,
            updated_at: "2026-01-03T00:00:00Z".into(),
            mtime: 0,
            size: 0,
        };
        crate::chunks::upsert_chunks(&guard, &[other]).unwrap();

        let fetched = get_chunks_for_file(&guard, "multi-chunk.md", DIM).unwrap();
        assert_eq!(
            fetched.len(),
            2,
            "exactly the two real chunks must be returned"
        );

        let mut by_id: std::collections::HashMap<String, ChunkDoc> =
            fetched.into_iter().map(|c| (c.id.clone(), c)).collect();
        let first = by_id.remove("mc-1").expect("mc-1 must be present");
        assert_eq!(first.path, "multi-chunk.md");
        assert_eq!(first.source, "daily");
        assert_eq!(first.text, "first chunk");
        assert_eq!(first.start_line, 1);
        assert_eq!(first.end_line, 5);
        assert_eq!(first.hash, "h1");
        assert_eq!(first.model, "m");
        assert_eq!(first.mtime, 100);
        assert_eq!(first.size, 200);
        assert!(
            first.embedding.is_empty(),
            "embedding is not requested by get_chunks_for_file"
        );

        let second = by_id.remove("mc-2").expect("mc-2 must be present");
        assert_eq!(second.text, "second chunk");
        assert_eq!(second.start_line, 6);
        assert_eq!(second.end_line, 10);
        // get_chunks_for_file does not request the embedding field in its
        // output_fields, so it round-trips as an empty vector.
        assert!(
            second.embedding.is_empty(),
            "embedding is not requested by get_chunks_for_file"
        );
        assert!(by_id.is_empty(), "no extra chunks");
    }

    #[test]
    fn test_get_chunks_for_file_empty() {
        let guard = TestGuard::new();
        let fetched = get_chunks_for_file(&guard, "no-such-file.md", DIM).unwrap();
        assert!(
            fetched.is_empty(),
            "file with no chunks must return empty list"
        );
    }
}
