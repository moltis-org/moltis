/// Hybrid search: combine vector similarity and keyword/FTS results.
use std::collections::HashMap;

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, histogram, labels, memory as mem_metrics};

use crate::{config::CitationMode, embeddings::EmbeddingProvider, store::MemoryStore};

/// A search result with metadata.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub chunk_id: String,
    pub path: String,
    pub source: String,
    pub start_line: i64,
    pub end_line: i64,
    pub score: f32,
    pub text: String,
}

impl SearchResult {
    /// Format the result text with a citation appended.
    /// Format: `{text}\n\nSource: {path}#{start_line}`
    pub fn text_with_citation(&self) -> String {
        format!(
            "{}\n\nSource: {}#{}",
            self.text.trim(),
            self.path,
            self.start_line
        )
    }

    /// Determine whether to include citations based on mode and result set.
    pub fn should_include_citations(results: &[SearchResult], mode: CitationMode) -> bool {
        match mode {
            CitationMode::On => true,
            CitationMode::Off => false,
            CitationMode::Auto => {
                // Include citations if results come from multiple files
                if results.len() <= 1 {
                    return false;
                }
                let first_path = &results[0].path;
                results.iter().any(|r| r.path != *first_path)
            },
        }
    }
}

/// Perform hybrid search: embed the query, run vector + keyword search, merge with weights.
pub async fn hybrid_search(
    store: &dyn MemoryStore,
    embedder: &dyn EmbeddingProvider,
    query: &str,
    limit: usize,
    vector_weight: f32,
    keyword_weight: f32,
) -> anyhow::Result<Vec<SearchResult>> {
    #[cfg(feature = "metrics")]
    let start = std::time::Instant::now();

    #[cfg(feature = "metrics")]
    counter!(mem_metrics::SEARCHES_TOTAL, labels::SEARCH_TYPE => "hybrid").increment(1);

    let query_embedding = embedder.embed(query).await?;

    let fetch_limit = limit * 3; // over-fetch for merging
    let vector_results = store.vector_search(&query_embedding, fetch_limit).await?;
    let keyword_results = store.keyword_search(query, fetch_limit).await?;

    let merged = merge_results(
        &vector_results,
        &keyword_results,
        vector_weight,
        keyword_weight,
    );

    let mut final_results: Vec<SearchResult> = merged.into_iter().take(limit).collect();

    // Fill in text for results that need it
    for result in &mut final_results {
        if result.text.is_empty()
            && let Some(chunk) = store.get_chunk_by_id(&result.chunk_id).await?
        {
            result.text = chunk.text;
        }
    }

    #[cfg(feature = "metrics")]
    histogram!(mem_metrics::SEARCH_DURATION_SECONDS, labels::SEARCH_TYPE => "hybrid")
        .record(start.elapsed().as_secs_f64());

    Ok(final_results)
}

/// Keyword-only search when no embedding provider is available.
pub async fn keyword_only_search(
    store: &dyn MemoryStore,
    query: &str,
    limit: usize,
) -> anyhow::Result<Vec<SearchResult>> {
    #[cfg(feature = "metrics")]
    let start = std::time::Instant::now();

    #[cfg(feature = "metrics")]
    counter!(mem_metrics::SEARCHES_TOTAL, labels::SEARCH_TYPE => "keyword").increment(1);

    let mut results = store.keyword_search(query, limit).await?;

    for result in &mut results {
        if result.text.is_empty()
            && let Some(chunk) = store.get_chunk_by_id(&result.chunk_id).await?
        {
            result.text = chunk.text;
        }
    }

    #[cfg(feature = "metrics")]
    histogram!(mem_metrics::SEARCH_DURATION_SECONDS, labels::SEARCH_TYPE => "keyword")
        .record(start.elapsed().as_secs_f64());

    Ok(results)
}

/// Merge vector and keyword results with weighted scores. Deduplicates by chunk_id.
fn merge_results(
    vector: &[SearchResult],
    keyword: &[SearchResult],
    vector_weight: f32,
    keyword_weight: f32,
) -> Vec<SearchResult> {
    let mut scores: HashMap<String, (f32, SearchResult)> = HashMap::new();

    for r in vector {
        let entry = scores.entry(r.chunk_id.clone()).or_insert((0.0, r.clone()));
        entry.0 += r.score * vector_weight;
    }

    for r in keyword {
        let entry = scores.entry(r.chunk_id.clone()).or_insert((0.0, r.clone()));
        entry.0 += r.score * keyword_weight;
    }

    let mut results: Vec<SearchResult> = scores
        .into_values()
        .map(|(score, mut r)| {
            r.score = score;
            r
        })
        .collect();

    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    fn make_result(id: &str, score: f32) -> SearchResult {
        SearchResult {
            chunk_id: id.into(),
            path: "test.md".into(),
            source: "daily".into(),
            start_line: 1,
            end_line: 5,
            score,
            text: String::new(),
        }
    }

    fn make_result_with_path(id: &str, path: &str, text: &str) -> SearchResult {
        SearchResult {
            chunk_id: id.into(),
            path: path.into(),
            source: "daily".into(),
            start_line: 10,
            end_line: 20,
            score: 0.9,
            text: text.into(),
        }
    }

    #[test]
    fn test_merge_results_deduplication() {
        let vec_results = vec![make_result("c1", 0.9), make_result("c2", 0.5)];
        let kw_results = vec![make_result("c1", 0.8), make_result("c3", 0.7)];

        let merged = merge_results(&vec_results, &kw_results, 0.7, 0.3);

        // c1 should have combined score: 0.9*0.7 + 0.8*0.3 = 0.63 + 0.24 = 0.87
        let c1 = merged.iter().find(|r| r.chunk_id == "c1").unwrap();
        assert!((c1.score - 0.87).abs() < 1e-5);

        // c2: 0.5*0.7 = 0.35
        let c2 = merged.iter().find(|r| r.chunk_id == "c2").unwrap();
        assert!((c2.score - 0.35).abs() < 1e-5);

        // c3: 0.7*0.3 = 0.21
        let c3 = merged.iter().find(|r| r.chunk_id == "c3").unwrap();
        assert!((c3.score - 0.21).abs() < 1e-5);

        // Sorted descending
        assert!(merged[0].score >= merged[1].score);
        assert!(merged[1].score >= merged[2].score);
    }

    #[test]
    fn test_merge_empty() {
        let merged = merge_results(&[], &[], 0.7, 0.3);
        assert!(merged.is_empty());
    }

    #[test]
    fn test_text_with_citation() {
        let result = make_result_with_path("c1", "memory/notes.md", "Some important content");
        let cited = result.text_with_citation();
        assert_eq!(
            cited,
            "Some important content\n\nSource: memory/notes.md#10"
        );
    }

    #[test]
    fn test_text_with_citation_trims_whitespace() {
        let mut result = make_result_with_path("c1", "test.md", "  content with spaces  \n");
        result.start_line = 42;
        let cited = result.text_with_citation();
        assert_eq!(cited, "content with spaces\n\nSource: test.md#42");
    }

    #[test]
    fn test_should_include_citations_on() {
        let results = vec![make_result("c1", 0.9)];
        assert!(SearchResult::should_include_citations(
            &results,
            CitationMode::On
        ));
    }

    #[test]
    fn test_should_include_citations_off() {
        let results = vec![
            make_result_with_path("c1", "a.md", "text"),
            make_result_with_path("c2", "b.md", "text"),
        ];
        assert!(!SearchResult::should_include_citations(
            &results,
            CitationMode::Off
        ));
    }

    #[test]
    fn test_should_include_citations_auto_single_file() {
        let results = vec![
            make_result_with_path("c1", "same.md", "text1"),
            make_result_with_path("c2", "same.md", "text2"),
        ];
        // Same file, auto mode should NOT include citations
        assert!(!SearchResult::should_include_citations(
            &results,
            CitationMode::Auto
        ));
    }

    #[test]
    fn test_should_include_citations_auto_multiple_files() {
        let results = vec![
            make_result_with_path("c1", "file1.md", "text1"),
            make_result_with_path("c2", "file2.md", "text2"),
        ];
        // Multiple files, auto mode SHOULD include citations
        assert!(SearchResult::should_include_citations(
            &results,
            CitationMode::Auto
        ));
    }

    #[test]
    fn test_should_include_citations_auto_empty() {
        let results: Vec<SearchResult> = vec![];
        assert!(!SearchResult::should_include_citations(
            &results,
            CitationMode::Auto
        ));
    }

    #[test]
    fn test_citation_mode_from_str() {
        assert_eq!("on".parse::<CitationMode>().unwrap(), CitationMode::On);
        assert_eq!("ON".parse::<CitationMode>().unwrap(), CitationMode::On);
        assert_eq!("true".parse::<CitationMode>().unwrap(), CitationMode::On);
        assert_eq!("always".parse::<CitationMode>().unwrap(), CitationMode::On);

        assert_eq!("off".parse::<CitationMode>().unwrap(), CitationMode::Off);
        assert_eq!("OFF".parse::<CitationMode>().unwrap(), CitationMode::Off);
        assert_eq!("false".parse::<CitationMode>().unwrap(), CitationMode::Off);
        assert_eq!("never".parse::<CitationMode>().unwrap(), CitationMode::Off);

        assert_eq!("auto".parse::<CitationMode>().unwrap(), CitationMode::Auto);
        assert_eq!(
            "anything".parse::<CitationMode>().unwrap(),
            CitationMode::Auto
        );
    }
}
