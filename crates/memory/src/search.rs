/// Search result types. The `SearchResult` struct, citation formatting, and the
/// backend-agnostic hybrid merge functions live here. Each store backend calls
/// these shared merge functions rather than reimplementing them.
use crate::{
    config::CitationMode,
    store::{MemoryStore, MergeStrategy},
};

use std::collections::HashMap;

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
                if results.len() <= 1 {
                    return false;
                }
                let first_path = &results[0].path;
                results.iter().any(|r| r.path != *first_path)
            },
        }
    }
}

/// Keyword-only search when no embedding provider is available.
#[tracing::instrument(skip(store), fields(query_len = query.len(), limit))]
pub async fn keyword_only_search(
    store: &dyn MemoryStore,
    query: &str,
    limit: usize,
) -> crate::error::Result<Vec<SearchResult>> {
    #[cfg(feature = "metrics")]
    use moltis_metrics::memory as mem_metrics;

    #[cfg(feature = "metrics")]
    let start = std::time::Instant::now();

    #[cfg(feature = "metrics")]
    moltis_metrics::counter!(mem_metrics::SEARCHES_TOTAL, moltis_metrics::labels::SEARCH_TYPE => "keyword").increment(1);

    let results = store
        .hybrid_search(&[], query, 0.0, 1.0, MergeStrategy::Weighted, limit)
        .await?;

    #[cfg(feature = "metrics")]
    moltis_metrics::histogram!(mem_metrics::SEARCH_DURATION_SECONDS, moltis_metrics::labels::SEARCH_TYPE => "keyword")
        .record(start.elapsed().as_secs_f64());

    Ok(results)
}

/// Merge vector and keyword results with weighted scores. Deduplicates by `chunk_id`.
pub fn merge_weighted(
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

/// Fill in the `text` field for any results whose text is empty by fetching
/// the full chunk from the store. Shared by all backends.
pub async fn fill_missing_text(
    store: &dyn MemoryStore,
    results: &mut [SearchResult],
) -> crate::error::Result<()> {
    for result in results {
        if result.text.is_empty()
            && let Some(chunk) = store.get_chunk_by_id(&result.chunk_id).await?
        {
            result.text = chunk.text;
        }
    }
    Ok(())
}

/// Merge results using Reciprocal Rank Fusion (RRF).
///
/// RRF is rank-based and score-magnitude-agnostic, avoiding sensitivity to
/// differing score scales between vector cosine similarity and FTS ranks.
///
/// Formula per result: `score = Σ weight / (rrf_k + rank + 1)`
pub fn merge_rrf(
    vector: &[SearchResult],
    keyword: &[SearchResult],
    vector_weight: f32,
    keyword_weight: f32,
    rrf_k: u32,
) -> Vec<SearchResult> {
    let rrf_k = rrf_k as f32;
    let mut scores: HashMap<String, (f32, SearchResult)> = HashMap::new();

    for (rank, r) in vector.iter().enumerate() {
        let rrf_score = vector_weight / (rrf_k + rank as f32 + 1.0);
        let entry = scores.entry(r.chunk_id.clone()).or_insert((0.0, r.clone()));
        entry.0 += rrf_score;
    }

    for (rank, r) in keyword.iter().enumerate() {
        let rrf_score = keyword_weight / (rrf_k + rank as f32 + 1.0);
        let entry = scores.entry(r.chunk_id.clone()).or_insert((0.0, r.clone()));
        entry.0 += rrf_score;
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
        let results = vec![SearchResult {
            chunk_id: "c1".into(),
            path: "test.md".into(),
            source: "daily".into(),
            start_line: 1,
            end_line: 5,
            score: 0.9,
            text: String::new(),
        }];
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

    #[test]
    fn test_merge_strategy_from_str() {
        // config::MergeStrategy (Rrf/Linear) — the user-facing enum parsed
        // from moltis.toml. Distinct from store::MergeStrategy, which is the
        // parameterized runtime enum without FromStr.
        use crate::config::MergeStrategy;

        assert_eq!("rrf".parse::<MergeStrategy>().unwrap(), MergeStrategy::Rrf);
        assert_eq!("RRF".parse::<MergeStrategy>().unwrap(), MergeStrategy::Rrf);
        assert_eq!(
            "linear".parse::<MergeStrategy>().unwrap(),
            MergeStrategy::Linear,
        );
        assert_eq!(
            "LINEAR".parse::<MergeStrategy>().unwrap(),
            MergeStrategy::Linear,
        );
        // Unknown strings default to Rrf (FromStr impl is infallible).
        assert_eq!(
            "unknown".parse::<MergeStrategy>().unwrap(),
            MergeStrategy::Rrf,
        );
        assert_eq!("".parse::<MergeStrategy>().unwrap(), MergeStrategy::Rrf,);
    }

    // ── merge_weighted / merge_rrf ──
    //
    // Canonical coverage for the shared merge functions used by every store
    // backend. Previously these were duplicated in `store_sqlite::tests` and
    // `moltis_memory_zvec::store::tests`; both copies now defer to this
    // module so the critical path is exercised by plain `cargo test
    // -p moltis-memory` regardless of backend feature flags.

    fn make_search_result(id: &str, score: f32) -> SearchResult {
        SearchResult {
            chunk_id: id.into(),
            path: "test/path.md".into(),
            source: "test".into(),
            start_line: 1,
            end_line: 5,
            score,
            text: String::new(),
        }
    }

    #[test]
    fn test_merge_weighted_both_empty() {
        assert!(merge_weighted(&[], &[], 0.7, 0.3).is_empty());
    }

    #[test]
    fn test_merge_weighted_vector_only_preserves_order() {
        let vec_results = vec![make_search_result("a", 0.9), make_search_result("b", 0.8)];
        let merged = merge_weighted(&vec_results, &[], 0.7, 0.3);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].chunk_id, "a");
        assert_eq!(merged[1].chunk_id, "b");
        assert!(merged[0].score > merged[1].score);
    }

    #[test]
    fn test_merge_weighted_keyword_only_preserves_order() {
        let kw_results = vec![make_search_result("x", 0.5), make_search_result("y", 0.3)];
        let merged = merge_weighted(&[], &kw_results, 0.7, 0.3);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].chunk_id, "x");
        assert_eq!(merged[1].chunk_id, "y");
        assert!(merged[0].score > merged[1].score);
    }

    #[test]
    fn test_merge_weighted_deduplication_and_combined_scores() {
        let vec_results = vec![make_search_result("c1", 0.9), make_search_result("c2", 0.5)];
        let kw_results = vec![make_search_result("c1", 0.8), make_search_result("c3", 0.7)];

        let merged = merge_weighted(&vec_results, &kw_results, 0.7, 0.3);

        // c1 appears in both sources and must be deduplicated with the
        // combined weighted score: 0.9*0.7 + 0.8*0.3 = 0.87.
        assert_eq!(merged.len(), 3);
        assert_eq!(
            merged.iter().filter(|r| r.chunk_id == "c1").count(),
            1,
            "overlapping chunk must be deduplicated"
        );
        let c1 = merged.iter().find(|r| r.chunk_id == "c1").unwrap();
        assert!((c1.score - 0.87).abs() < 1e-5);

        // c2 only in vector: 0.5*0.7 = 0.35. c3 only in keyword: 0.7*0.3 = 0.21.
        let c2 = merged.iter().find(|r| r.chunk_id == "c2").unwrap();
        assert!((c2.score - 0.35).abs() < 1e-5);
        let c3 = merged.iter().find(|r| r.chunk_id == "c3").unwrap();
        assert!((c3.score - 0.21).abs() < 1e-5);

        // Descending order.
        assert!(merged[0].score >= merged[1].score);
        assert!(merged[1].score >= merged[2].score);
    }

    #[test]
    fn test_merge_rrf_both_empty() {
        assert!(merge_rrf(&[], &[], 1.0, 1.0, 60).is_empty());
    }

    #[test]
    fn test_merge_rrf_vector_only_preserves_order() {
        let vec_results = vec![make_search_result("a", 0.0), make_search_result("b", 0.0)];
        let merged = merge_rrf(&vec_results, &[], 1.0, 0.0, 60);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].chunk_id, "a");
        assert_eq!(merged[1].chunk_id, "b");
        assert!(merged[0].score > merged[1].score);
    }

    #[test]
    fn test_merge_rrf_keyword_only_preserves_order() {
        let kw_results = vec![make_search_result("k1", 0.0), make_search_result("k2", 0.0)];
        let merged = merge_rrf(&[], &kw_results, 0.0, 1.0, 60);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].chunk_id, "k1");
        assert_eq!(merged[1].chunk_id, "k2");
        assert!(merged[0].score > merged[1].score);
    }

    #[test]
    fn test_merge_rrf_deduplication_and_combined_score() {
        let vec_results = vec![make_search_result("c1", 0.9), make_search_result("c2", 0.5)];
        let kw_results = vec![make_search_result("c1", 0.8), make_search_result("c3", 0.7)];

        let merged = merge_rrf(&vec_results, &kw_results, 0.7, 0.3, 5);

        assert_eq!(merged.len(), 3);
        assert_eq!(
            merged.iter().filter(|r| r.chunk_id == "c1").count(),
            1,
            "overlapping chunk must be deduplicated"
        );
        // c1 ranks 0 in both lists, so it accumulates the max RRF score.
        let c1 = merged.iter().find(|r| r.chunk_id == "c1").unwrap();
        let expected_c1 = 0.7 / (5.0 + 0.0 + 1.0) + 0.3 / (5.0 + 0.0 + 1.0);
        assert!((c1.score - expected_c1).abs() < 1e-6);

        // Descending order.
        for i in 0..merged.len() - 1 {
            assert!(
                merged[i].score >= merged[i + 1].score,
                "results should be sorted descending by score"
            );
        }
    }

    #[test]
    fn test_rrf_vs_weighted_produce_different_rankings() {
        // Construct inputs where weighted (raw-score blend) and RRF
        // (rank-based) must disagree. Vector strongly favors c1; keyword's
        // huge raw score for c2 dominates the weighted blend but is neutral
        // under RRF, which only sees ranks.
        let vec_results = vec![
            make_search_result("c1", 0.95),
            make_search_result("c2", 0.1),
        ];
        let kw_results = vec![
            make_search_result("c2", 10.0),
            make_search_result("c1", 0.5),
        ];

        let weighted = merge_weighted(&vec_results, &kw_results, 0.7, 0.3);
        let rrf = merge_rrf(&vec_results, &kw_results, 0.7, 0.3, 5);

        // Weighted: c2 = 0.1*0.7 + 10.0*0.3 = 3.07 > c1 = 0.95*0.7 + 0.5*0.3 = 0.815.
        assert_eq!(weighted[0].chunk_id, "c2");
        // RRF: c1 is rank 0 in the higher-weighted vector list, so it wins.
        assert_eq!(rrf[0].chunk_id, "c1");
    }
}
