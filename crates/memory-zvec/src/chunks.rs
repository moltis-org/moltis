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
        hex::encode(&hash[..8])
    }

    /// Build a [`ChunkDoc`] from a stored [`ChunkRow`]. `mtime`/`size` are not
    /// tracked on chunks, so they default to `0`.
    pub fn from_chunk_row(c: &moltis_memory::schema::ChunkRow) -> Self {
        let embedding = c
            .embedding
            .as_ref()
            .map(|blob| {
                blob.chunks(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect()
            })
            .unwrap_or_default();
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
    let zvec_docs: Vec<Doc> = docs
        .iter()
        .map(|d| d.to_zvec_doc())
        .collect::<Result<Vec<_>>>()
        .context("failed to convert chunk docs to zvec docs")?;

    let doc_refs: Vec<&Doc> = zvec_docs.iter().collect();

    collection
        .upsert(&doc_refs)
        .map_err(|e| anyhow::anyhow!("failed to upsert chunks: {e:#}"))?;
    collection
        .flush()
        .map_err(|e| anyhow::anyhow!("failed to flush after upserting chunks: {e:#}"))?;

    Ok(())
}

pub fn delete_chunks_for_file(collection: &Collection, path: &str) -> Result<()> {
    let filter = format!("path = '{}'", crate::escape_filter_value(path));
    collection
        .delete_by_filter(&filter)
        .context("failed to delete chunks by path filter")?;
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
}
