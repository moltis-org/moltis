//! Storage backend trait for code-index.
//!
//! Abstracts over SQLite (builtin) and potential future backends.

use {async_trait::async_trait, std::collections::HashMap};

use crate::{error::Result, types::SearchResult};

/// A chunk of code with optional embedding.
#[derive(Debug, Clone)]
pub struct CodeChunk {
    /// File path relative to project root.
    pub file_path: String,
    /// Chunk sequence index within the file.
    pub chunk_index: usize,
    /// Content of the chunk.
    pub content: String,
    /// Optional embedding vector (f32 for compute, stored as quantized i8).
    pub embedding: Option<Vec<f32>>,
    /// Start line number (1-indexed).
    pub start_line: usize,
    /// End line number (1-indexed, inclusive).
    pub end_line: usize,
}

/// Storage backend for code-index.
#[async_trait]
pub trait CodeIndexStore: Send + Sync {
    /// Initialize the store (create tables, indexes, etc).
    async fn initialize(&self) -> Result<()>;

    /// Insert or replace chunks for a file within a project.
    async fn upsert_chunks(
        &self,
        project_id: &str,
        file_path: &str,
        chunks: &[CodeChunk],
    ) -> Result<()>;

    /// Delete all chunks for a file.
    async fn delete_file_chunks(&self, project_id: &str, file_path: &str) -> Result<()>;

    /// Delete all chunks for a project.
    async fn clear_project(&self, project_id: &str) -> Result<()>;

    /// Get all chunks for a project (for brute-force vector search).
    async fn get_project_chunks(&self, project_id: &str) -> Result<Vec<CodeChunk>>;

    /// Full-text search using FTS5.
    async fn search_keyword(
        &self,
        project_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>>;

    /// Get indexed file count for status.
    async fn file_count(&self, project_id: &str) -> Result<usize>;

    /// Get total chunk count for status.
    async fn chunk_count(&self, project_id: &str) -> Result<usize>;
}

/// Merge vector and keyword search results using reciprocal rank fusion.
pub fn merge_hybrid_results(
    vector_results: Vec<SearchResult>,
    keyword_results: Vec<SearchResult>,
    limit: usize,
) -> Vec<SearchResult> {
    let k = 60.0; // RRF constant
    let mut scores: HashMap<String, (SearchResult, f64)> = HashMap::new();

    // Score vector results
    for (rank, result) in vector_results.iter().enumerate() {
        let key = format!("{}:{}", result.path, result.start_line);
        let score = 1.0 / (k + rank as f64);
        scores.entry(key).or_insert((result.clone(), 0.0)).1 += score;
    }

    // Score keyword results
    for (rank, result) in keyword_results.iter().enumerate() {
        let key = format!("{}:{}", result.path, result.start_line);
        let score = 1.0 / (k + rank as f64);
        scores.entry(key).or_insert((result.clone(), 0.0)).1 += score;
    }

    // Sort by combined score descending
    let mut results: Vec<(SearchResult, f64)> = scores.into_values().collect();
    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    results.into_iter().take(limit).map(|(r, _)| r).collect()
}

/// Cosine similarity between two vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }

    let dot_product: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot_product / (norm_a * norm_b)
}

/// Quantize f32 embeddings to i8 for storage.
///
/// **Tradeoff:** Saves ~4× space (1 byte vs 4 bytes per dimension) but clamps values
/// to [-1.0, 1.0] before scaling. This is safe for normalized embedding models (most
/// produce unit-length vectors where components naturally fall in this range), but
/// silently destroys magnitude information for non-normalized models. The memory system
/// stores full f32 vectors; this backend quantizes for lower disk usage at the cost of
/// reduced recall on non-normalized inputs.
pub fn quantize(embedding: &[f32]) -> Vec<i8> {
    embedding
        .iter()
        .map(|&v| {
            // Scale to [-127, 127] range
            (v.clamp(-1.0, 1.0) * 127.0).round() as i8
        })
        .collect()
}

/// Dequantize i8 embeddings back to f32 for compute.
pub fn dequantize(embedding: &[i8]) -> Vec<f32> {
    embedding.iter().map(|&v| v as f32 / 127.0).collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn make_result(path: &str, start_line: usize, score: f32) -> SearchResult {
        SearchResult {
            chunk_id: format!("{path}:{start_line}"),
            path: path.to_string(),
            text: "test content".into(),
            start_line,
            end_line: start_line + 5,
            score,
            source: "builtin".into(),
        }
    }

    #[test]
    fn test_cosine_similarity_identical() {
        let v = vec![1.0, 0.0, 0.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-6, "expected ~1.0, got {sim}");
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let sim = cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]);
        assert!((sim - 0.0).abs() < 1e-6, "expected ~0.0, got {sim}");
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let sim = cosine_similarity(&[1.0, 0.0], &[-1.0, 0.0]);
        assert!((sim - (-1.0)).abs() < 1e-6, "expected ~-1.0, got {sim}");
    }

    #[test]
    fn test_cosine_similarity_dimension_mismatch() {
        let sim = cosine_similarity(&[1.0], &[1.0, 2.0]);
        assert_eq!(sim, 0.0);
    }

    #[test]
    fn test_cosine_similarity_zero_vector() {
        let sim = cosine_similarity(&[0.0, 0.0], &[1.0, 0.0]);
        assert_eq!(sim, 0.0);
    }

    #[test]
    fn test_quantize_dequantize_roundtrip() {
        let original = vec![0.5, -0.3, 0.8];
        let quantized = quantize(&original);
        let restored = dequantize(&quantized);
        for (orig, rest) in original.iter().zip(restored.iter()) {
            assert!(
                (orig - rest).abs() < 0.02,
                "original {orig} vs restored {rest}"
            );
        }
    }

    #[test]
    fn test_quantize_clamps_large_values() {
        // Values outside [-1, 1] should be clamped, not cause overflow
        let quantized = quantize(&[5.0, -10.0, 0.5]);
        let restored = dequantize(&quantized);
        assert!(
            restored[0] <= 1.01,
            "clamped high value should be ~1.0, got {}",
            restored[0]
        );
        assert!(
            restored[1] >= -1.01,
            "clamped low value should be ~-1.0, got {}",
            restored[1]
        );
        assert!(
            (restored[2] - 0.5).abs() < 0.02,
            "in-range value preserved, got {}",
            restored[2]
        );
    }

    #[test]
    fn test_merge_hybrid_results_pure_keyword() {
        let keyword = vec![make_result("a.rs", 1, 0.8), make_result("b.rs", 1, 0.6)];
        let merged = merge_hybrid_results(vec![], keyword, 10);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn test_merge_hybrid_results_rrf_boosting() {
        // A result that appears in BOTH vector and keyword sets should rank
        // higher than a result that appears in only one set.
        let shared = make_result("shared.rs", 1, 0.9);
        let keyword_only = make_result("kw_only.rs", 1, 0.95);
        let vector_only = make_result("vec_only.rs", 1, 0.85);

        let vector_results = vec![shared.clone(), vector_only];
        let keyword_results = vec![shared, keyword_only];

        let merged = merge_hybrid_results(vector_results, keyword_results, 10);
        assert_eq!(merged.len(), 3);
        // shared.rs should be first (boosted by appearing in both)
        assert_eq!(merged[0].path, "shared.rs");
    }

    #[test]
    fn test_merge_hybrid_results_limit() {
        let results: Vec<SearchResult> = (0..10)
            .map(|i| make_result("f.rs", i + 1, 1.0 / (i as f32 + 1.0)))
            .collect();

        // Put all 10 in keyword, none in vector
        let merged = merge_hybrid_results(vec![], results, 3);
        assert_eq!(merged.len(), 3);
    }
}
