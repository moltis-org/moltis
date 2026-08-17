use {
    anyhow::{Context, Result},
    serde::{Deserialize, Serialize},
    zvec::{Collection, Doc},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkDoc {
    pub id: String,
    pub path: String,
    pub source: String,
    pub start_line: i64,
    pub end_line: i64,
    pub hash: String,
    pub model: String,
    pub text: String,
    pub embedding: Vec<f32>,
    pub updated_at: String,
    pub mtime: i64,
    pub size: i64,
}

impl ChunkDoc {
    pub fn safe_pk(original_id: &str) -> String {
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(original_id.as_bytes());
        hex::encode(&hash[..16])
    }

    /// Build a [`ChunkDoc`] from a stored [`ChunkRow`]. `mtime`/`size` are not
    /// tracked on chunks, so they default to `0`.
    ///
    /// `dimension` is the zvec collection's vector dimension. When `c.embedding`
    /// is `None` (keyword-only mode, no embedding provider configured), a zero
    /// vector of the configured dimension is substituted so the resulting zvec
    /// doc satisfies the collection schema's fixed-dimension vector field.
    /// Without this, zvec rejects the upsert with a dimension-mismatch error
    /// and every chunk sync silently fails, leaving the collection empty.
    pub fn from_chunk_row(c: &moltis_memory::schema::ChunkRow, dimension: u32) -> Self {
        let embedding = c
            .embedding
            .as_ref()
            .map(|blob| {
                blob.chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect()
            })
            .unwrap_or_else(|| vec![0.0f32; dimension as usize]);
        Self {
            id: c.id.clone(),
            path: c.path.clone(),
            source: c.source.clone(),
            start_line: c.start_line,
            end_line: c.end_line,
            hash: c.hash.clone(),
            model: c.model.clone(),
            text: c.text.clone(),
            embedding,
            updated_at: c.updated_at.clone(),
            mtime: 0,
            size: 0,
        }
    }

    /// Extract a [`ChunkDoc`] from a fetched zvec [`Doc`].
    pub fn from_doc(doc: &Doc) -> Result<Self> {
        let str_field = |name: &str| {
            doc.get_string(name)
                .with_context(|| format!("failed to get {name}"))
                .map(|opt| opt.unwrap_or_default())
        };
        let i64_field = |name: &str| {
            doc.get_i64(name)
                .with_context(|| format!("failed to get {name}"))
                .map(|opt| opt.unwrap_or(0))
        };
        Ok(Self {
            id: str_field("id")?,
            path: str_field("path")?,
            source: str_field("source")?,
            start_line: i64_field("start_line")?,
            end_line: i64_field("end_line")?,
            hash: str_field("hash")?,
            model: str_field("model")?,
            text: str_field("text")?,
            embedding: doc
                .get_vector_f32("embedding")
                .ok()
                .flatten()
                .unwrap_or_default(),
            updated_at: str_field("updated_at")?,
            mtime: i64_field("mtime")?,
            size: i64_field("size")?,
        })
    }

    fn to_zvec_doc(&self) -> Result<Doc> {
        let mut doc = Doc::new().context("failed to create zvec doc")?;
        let pk = Self::safe_pk(&self.id);
        doc.set_pk(&pk);
        doc.add_string("id", &self.id)?;
        doc.add_string("path", &self.path)?;
        doc.add_string("source", &self.source)?;
        doc.add_i64("start_line", self.start_line)?;
        doc.add_i64("end_line", self.end_line)?;
        doc.add_string("hash", &self.hash)?;
        doc.add_string("model", &self.model)?;
        doc.add_string("text", &self.text)?;
        doc.add_vector_f32("embedding", &self.embedding)?;
        doc.add_string("updated_at", &self.updated_at)?;
        doc.add_i64("mtime", self.mtime)?;
        doc.add_i64("size", self.size)?;
        Ok(doc)
    }
}

impl From<ChunkDoc> for moltis_memory::schema::ChunkRow {
    fn from(c: ChunkDoc) -> Self {
        use moltis_memory::schema::ChunkRow;
        let embedding = if c.embedding.is_empty() {
            None
        } else {
            Some(c.embedding.iter().flat_map(|f| f.to_le_bytes()).collect())
        };
        ChunkRow {
            id: c.id,
            path: c.path,
            source: c.source,
            start_line: c.start_line,
            end_line: c.end_line,
            hash: c.hash,
            model: c.model,
            text: c.text,
            embedding,
            updated_at: c.updated_at,
        }
    }
}

pub fn upsert_chunks(collection: &Collection, docs: &[ChunkDoc]) -> Result<()> {
    upsert_chunks_no_flush(collection, docs)?;
    collection
        .flush()
        .map_err(|e| anyhow::anyhow!("failed to flush after upserting chunks: {e:#}"))?;
    Ok(())
}

/// Upsert chunks without flushing. Callers must flush the collection
/// themselves when done (e.g. after a batch loop).
pub fn upsert_chunks_no_flush(collection: &Collection, docs: &[ChunkDoc]) -> Result<()> {
    let zvec_docs: Vec<Doc> = docs
        .iter()
        .map(|d| d.to_zvec_doc())
        .collect::<Result<Vec<_>>>()
        .context("failed to convert chunk docs to zvec docs")?;

    let doc_refs: Vec<&Doc> = zvec_docs.iter().collect();

    collection
        .upsert(&doc_refs)
        .map_err(|e| anyhow::anyhow!("failed to upsert chunks: {e:#}"))?;

    Ok(())
}

pub fn delete_chunks_for_file(collection: &Collection, path: &str) -> Result<()> {
    let filter = format!("path = '{}'", crate::escape_filter_value(path));
    collection
        .delete_by_filter(&filter)
        .context("failed to delete chunks by path filter")?;
    Ok(())
}

/// Delete chunks by their primary keys (safe against concurrent upserts).
///
/// Unlike [`delete_chunks_for_file`] which uses a path filter (and thus catches
/// concurrently-upserted chunks for the same path), this only deletes the
/// specific PKs passed in, leaving any newer chunks intact.
pub fn delete_chunks_by_pks(collection: &Collection, pks: &[String]) -> Result<()> {
    if pks.is_empty() {
        return Ok(());
    }
    let pk_refs: Vec<&str> = pks.iter().map(String::as_str).collect();
    collection
        .delete(&pk_refs)
        .map_err(|e| anyhow::anyhow!("failed to delete chunks by pks: {e:#}"))?;
    Ok(())
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, crate::collection::TestGuard};

    #[test]
    fn test_upsert_chunks_dedup_reupsert() {
        let guard = TestGuard::new();

        let chunk = ChunkDoc {
            id: "dedup-1".into(),
            path: "dedup-test.md".into(),
            source: "test".into(),
            start_line: 1,
            end_line: 5,
            hash: "original-hash".into(),
            model: "m".into(),
            text: "original text".into(),
            embedding: vec![0.0f32; 768],
            updated_at: "2025-01-01T00:00:00Z".into(),
            mtime: 0,
            size: 0,
        };
        upsert_chunks(&guard, &[chunk]).unwrap();

        let fetched = crate::files::get_chunk_by_id(&guard, "dedup-1")
            .unwrap()
            .unwrap();
        assert_eq!(fetched.text, "original text");
        assert_eq!(fetched.hash, "original-hash");

        let updated = ChunkDoc {
            id: "dedup-1".into(),
            path: "dedup-test.md".into(),
            source: "test".into(),
            start_line: 2,
            end_line: 10,
            hash: "updated-hash".into(),
            model: "m2".into(),
            text: "updated text".into(),
            embedding: vec![1.0f32; 768],
            updated_at: "2025-06-01T00:00:00Z".into(),
            mtime: 0,
            size: 0,
        };
        upsert_chunks(&guard, &[updated]).unwrap();

        let refetched = crate::files::get_chunk_by_id(&guard, "dedup-1")
            .unwrap()
            .unwrap();
        assert_eq!(refetched.text, "updated text");
        assert_eq!(refetched.hash, "updated-hash");
        assert_eq!(refetched.start_line, 2);
        assert_eq!(refetched.end_line, 10);
        assert_eq!(refetched.model, "m2");
    }

    #[test]
    fn test_safe_pk_is_hex() {
        assert!(
            ChunkDoc::safe_pk("simple-id")
                .chars()
                .all(|c| c.is_ascii_hexdigit())
        );
        assert!(
            ChunkDoc::safe_pk("/path/with/slashes.md:0")
                .chars()
                .all(|c| c.is_ascii_hexdigit())
        );
        assert!(
            ChunkDoc::safe_pk("/tmp/moltis_data/memory/session-main-2026-06-20.md:3")
                .chars()
                .all(|c| c.is_ascii_hexdigit())
        );
    }

    #[test]
    fn test_safe_pk_is_stable() {
        let a = ChunkDoc::safe_pk("/tmp/data/memory/session.md:0");
        let b = ChunkDoc::safe_pk("/tmp/data/memory/session.md:0");
        assert_eq!(a, b, "same input must produce same pk");
    }

    #[test]
    fn test_safe_pk_different_for_different_inputs() {
        let a = ChunkDoc::safe_pk("/tmp/data/file.md:0");
        let b = ChunkDoc::safe_pk("/tmp/data/file.md:1");
        assert_ne!(a, b, "different index must produce different pk");
    }

    #[test]
    fn test_to_zvec_doc_with_special_chars_in_id() {
        let _guard = TestGuard::new();
        let chunk = ChunkDoc {
            id: "/tmp/moltis_data/memory/session-main-2026-06-20.md:3".into(),
            path: "/tmp/moltis_data/memory/session-main-2026-06-20.md".into(),
            source: "daily".into(),
            start_line: 10,
            end_line: 20,
            hash: "abc123".into(),
            model: "embedding".into(),
            text: "chunk with special path".into(),
            embedding: vec![0.1f32; 1024],
            updated_at: "2026-01-01T00:00:00Z".into(),
            mtime: 1000,
            size: 200,
        };
        let doc = chunk
            .to_zvec_doc()
            .expect("to_zvec_doc should succeed with special-path id");
        // The PK must be the safe hash, not the raw path
        let pk = ChunkDoc::safe_pk(&chunk.id);
        assert_ne!(
            doc.get_pk().unwrap(),
            chunk.id.as_str(),
            "PK must be safe hash, not raw path"
        );
        assert_eq!(
            doc.get_pk().unwrap(),
            pk.as_str(),
            "PK must match safe_pk output"
        );
        // The stored id field must still be the original value
        let stored_id = doc.get_string("id").unwrap().unwrap();
        assert_eq!(stored_id, chunk.id, "id field must preserve original path");
    }

    /// Regression test for keyword-only mode: a `ChunkRow` with no embedding
    /// must produce a `ChunkDoc` carrying a zero vector of the collection's
    /// dimension, not an empty vector. An empty vector is rejected by zvec
    /// (dimension mismatch), silently breaking every chunk upsert when no
    /// embedding provider is configured.
    #[test]
    fn test_from_chunk_row_none_embedding_fills_zero_vector() {
        let row = moltis_memory::schema::ChunkRow {
            id: "kw-1".into(),
            path: "kw.md".into(),
            source: "test".into(),
            start_line: 1,
            end_line: 5,
            hash: "h".into(),
            model: String::new(),
            text: "keyword-only chunk".into(),
            embedding: None,
            updated_at: "2025-01-01T00:00:00Z".into(),
        };
        let doc = ChunkDoc::from_chunk_row(&row, 768);
        assert_eq!(
            doc.embedding.len(),
            768,
            "missing embedding must be backfilled"
        );
        assert!(
            doc.embedding.iter().all(|f| *f == 0.0),
            "backfilled embedding must be all zeros"
        );
        assert_eq!(doc.id, "kw-1");
        assert_eq!(doc.text, "keyword-only chunk");
    }

    /// End-to-end regression: upserting a chunk with no embedding (keyword-only
    /// mode) must not return a dimension-mismatch error from zvec, and the
    /// chunk must remain retrievable.
    #[test]
    fn test_upsert_chunks_keyword_only_no_embedding() {
        let guard = TestGuard::new();

        let row = moltis_memory::schema::ChunkRow {
            id: "kw-upsert-1".into(),
            path: "kw-upsert.md".into(),
            source: "test".into(),
            start_line: 1,
            end_line: 5,
            hash: "h".into(),
            model: String::new(),
            text: "keyword-only upsert".into(),
            embedding: None,
            updated_at: "2025-01-01T00:00:00Z".into(),
        };
        let doc = ChunkDoc::from_chunk_row(&row, 768);
        upsert_chunks(&guard, &[doc]).expect(
            "keyword-only chunk upsert must succeed (zero-vector fallback prevents \
             zvec dimension-mismatch)",
        );

        let fetched = crate::files::get_chunk_by_id(&guard, "kw-upsert-1")
            .unwrap()
            .expect("keyword-only chunk must be retrievable after upsert");
        assert_eq!(fetched.text, "keyword-only upsert");
    }

    #[test]
    fn test_from_chunk_doc_to_row_with_embedding() {
        // Exercises the Some(..) arm of From<ChunkDoc> for ChunkRow, which
        // flattens the f32 embedding into little-endian bytes. Only the
        // empty-embedding (None) arm was covered previously.
        let chunk = ChunkDoc {
            id: "emb-1".into(),
            path: "emb.md".into(),
            source: "test".into(),
            start_line: 1,
            end_line: 2,
            hash: "h".into(),
            model: "m".into(),
            text: "with embedding".into(),
            embedding: vec![0.5f32, -0.25, 1.0],
            updated_at: "2026-01-01T00:00:00Z".into(),
            mtime: 0,
            size: 0,
        };

        let row = moltis_memory::schema::ChunkRow::from(chunk);
        let emb = row
            .embedding
            .as_ref()
            .expect("non-empty embedding must be serialized to Some(bytes)");
        // 3 f32 values = 12 bytes (little-endian).
        assert_eq!(emb.len(), 12);
        // First value 0.5f32 little-endian bytes.
        assert_eq!(&emb[0..4], &0.5f32.to_le_bytes());
        assert_eq!(&emb[4..8], &(-0.25f32).to_le_bytes());
        assert_eq!(&emb[8..12], &1.0f32.to_le_bytes());
        assert_eq!(row.id, "emb-1");
        assert_eq!(row.path, "emb.md");
    }

    #[test]
    fn test_from_chunk_doc_to_row_empty_embedding_is_none() {
        // Explicitly assert the empty-embedding arm yields None (mirrors the
        // non-empty test for completeness of the From impl).
        let chunk = ChunkDoc {
            id: "no-emb".into(),
            path: "n.md".into(),
            source: "test".into(),
            start_line: 0,
            end_line: 0,
            hash: "h".into(),
            model: "m".into(),
            text: "no emb".into(),
            embedding: vec![],
            updated_at: "2026-01-01T00:00:00Z".into(),
            mtime: 0,
            size: 0,
        };

        let row = moltis_memory::schema::ChunkRow::from(chunk);
        assert!(row.embedding.is_none(), "empty embedding must map to None");
    }
}
