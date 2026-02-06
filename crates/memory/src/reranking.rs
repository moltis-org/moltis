//! LLM-based reranking for search results.
//!
//! This module provides functionality to rerank search results using an LLM,
//! improving relevance by considering semantic understanding beyond keyword/vector matching.

use std::sync::Arc;

use {
    async_trait::async_trait,
    tracing::{debug, warn},
};

use crate::search::SearchResult;

/// Trait for LLM reranking providers.
#[async_trait]
pub trait RerankerProvider: Send + Sync {
    /// Rerank the given results based on their relevance to the query.
    /// Returns the results in a new order with updated scores.
    async fn rerank(
        &self,
        query: &str,
        results: Vec<SearchResult>,
        top_k: usize,
    ) -> anyhow::Result<Vec<SearchResult>>;
}

/// A reranker that uses an LLM to score and reorder results.
pub struct LlmReranker {
    /// The LLM client for making completion requests.
    client: Arc<dyn LlmClient>,
    /// Maximum number of results to consider for reranking.
    max_candidates: usize,
    /// Model to use for reranking (if configurable).
    model: Option<String>,
}

/// Trait for LLM clients that can be used for reranking.
#[async_trait]
pub trait LlmClient: Send + Sync {
    /// Send a completion request and get the response text.
    async fn complete(&self, prompt: &str, model: Option<&str>) -> anyhow::Result<String>;
}

impl LlmReranker {
    /// Create a new LLM reranker with the given client.
    pub fn new(client: Arc<dyn LlmClient>) -> Self {
        Self {
            client,
            max_candidates: 20,
            model: None,
        }
    }

    /// Set the maximum number of candidates to consider.
    pub fn with_max_candidates(mut self, max: usize) -> Self {
        self.max_candidates = max;
        self
    }

    /// Set a specific model to use for reranking.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Build the reranking prompt.
    fn build_prompt(&self, query: &str, results: &[SearchResult]) -> String {
        let mut prompt = String::new();
        prompt.push_str(
            "You are a relevance scoring assistant. Given a query and a list of text passages, ",
        );
        prompt.push_str(
            "score each passage from 0.0 to 1.0 based on how relevant it is to the query.\n\n",
        );
        prompt.push_str(
            "Respond with ONLY a JSON array of scores in the same order as the passages.\n",
        );
        prompt.push_str("Example response: [0.95, 0.72, 0.45, 0.88]\n\n");
        prompt.push_str(&format!("Query: {}\n\n", query));
        prompt.push_str("Passages:\n");

        for (i, result) in results.iter().enumerate() {
            // Truncate long texts to avoid context overflow
            let text = if result.text.len() > 500 {
                format!("{}...", &result.text[..500])
            } else {
                result.text.clone()
            };
            prompt.push_str(&format!("{}. {}\n\n", i + 1, text.replace('\n', " ")));
        }

        prompt.push_str("\nScores (JSON array only):");
        prompt
    }

    /// Parse the LLM response into scores.
    fn parse_scores(&self, response: &str, expected_count: usize) -> anyhow::Result<Vec<f32>> {
        // Try to extract JSON array from response
        let response = response.trim();

        // Find the JSON array in the response
        let start = response.find('[').unwrap_or(0);
        let end = response.rfind(']').map(|i| i + 1).unwrap_or(response.len());
        let json_str = &response[start..end];

        let scores: Vec<f32> = serde_json::from_str(json_str)?;

        if scores.len() != expected_count {
            anyhow::bail!("expected {} scores, got {}", expected_count, scores.len());
        }

        // Clamp scores to 0.0-1.0 range
        Ok(scores.into_iter().map(|s| s.clamp(0.0, 1.0)).collect())
    }
}

#[async_trait]
impl RerankerProvider for LlmReranker {
    async fn rerank(
        &self,
        query: &str,
        mut results: Vec<SearchResult>,
        top_k: usize,
    ) -> anyhow::Result<Vec<SearchResult>> {
        if results.is_empty() {
            return Ok(results);
        }

        // Limit candidates to avoid context overflow
        let candidates: Vec<SearchResult> = results
            .drain(..results.len().min(self.max_candidates))
            .collect();
        let remaining = results; // Any results beyond max_candidates

        let prompt = self.build_prompt(query, &candidates);

        match self.client.complete(&prompt, self.model.as_deref()).await {
            Ok(response) => {
                match self.parse_scores(&response, candidates.len()) {
                    Ok(scores) => {
                        // Combine results with new scores
                        let mut scored: Vec<(f32, SearchResult)> = candidates
                            .into_iter()
                            .zip(scores)
                            .map(|(mut r, score)| {
                                // Blend original score with LLM score (70% LLM, 30% original)
                                r.score = score * 0.7 + r.score * 0.3;
                                (r.score, r)
                            })
                            .collect();

                        // Sort by new score descending
                        scored.sort_by(|a, b| {
                            b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal)
                        });

                        // Take top_k and append any remaining
                        let mut final_results: Vec<SearchResult> =
                            scored.into_iter().map(|(_, r)| r).take(top_k).collect();

                        // Append remaining results if we need more
                        if final_results.len() < top_k {
                            final_results
                                .extend(remaining.into_iter().take(top_k - final_results.len()));
                        }

                        debug!(
                            query = %query,
                            results = final_results.len(),
                            "reranked search results"
                        );

                        Ok(final_results)
                    },
                    Err(e) => {
                        warn!(error = %e, "failed to parse reranking scores, using original order");
                        let mut all = candidates;
                        all.extend(remaining);
                        Ok(all.into_iter().take(top_k).collect())
                    },
                }
            },
            Err(e) => {
                warn!(error = %e, "LLM reranking failed, using original order");
                let mut all = candidates;
                all.extend(remaining);
                Ok(all.into_iter().take(top_k).collect())
            },
        }
    }
}

/// A no-op reranker that returns results unchanged (used when reranking is disabled).
pub struct NoOpReranker;

#[async_trait]
impl RerankerProvider for NoOpReranker {
    async fn rerank(
        &self,
        _query: &str,
        results: Vec<SearchResult>,
        top_k: usize,
    ) -> anyhow::Result<Vec<SearchResult>> {
        Ok(results.into_iter().take(top_k).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockLlmClient {
        response: String,
    }

    #[async_trait]
    impl LlmClient for MockLlmClient {
        async fn complete(&self, _prompt: &str, _model: Option<&str>) -> anyhow::Result<String> {
            Ok(self.response.clone())
        }
    }

    fn make_result(id: &str, text: &str, score: f32) -> SearchResult {
        SearchResult {
            chunk_id: id.into(),
            path: "test.md".into(),
            source: "daily".into(),
            start_line: 1,
            end_line: 5,
            score,
            text: text.into(),
        }
    }

    #[tokio::test]
    async fn test_reranker_reorders_results() {
        let client = Arc::new(MockLlmClient {
            // Reverse the original order
            response: "[0.3, 0.9, 0.6]".into(),
        });
        let reranker = LlmReranker::new(client);

        let results = vec![
            make_result("c1", "First result", 0.9),
            make_result("c2", "Second result", 0.7),
            make_result("c3", "Third result", 0.5),
        ];

        let reranked = reranker.rerank("test query", results, 3).await.unwrap();

        // c2 should now be first (highest LLM score)
        assert_eq!(reranked[0].chunk_id, "c2");
    }

    #[tokio::test]
    async fn test_reranker_handles_empty_results() {
        let client = Arc::new(MockLlmClient {
            response: "[]".into(),
        });
        let reranker = LlmReranker::new(client);

        let results: Vec<SearchResult> = vec![];
        let reranked = reranker.rerank("test query", results, 5).await.unwrap();

        assert!(reranked.is_empty());
    }

    #[tokio::test]
    async fn test_reranker_respects_top_k() {
        let client = Arc::new(MockLlmClient {
            response: "[0.9, 0.8, 0.7, 0.6, 0.5]".into(),
        });
        let reranker = LlmReranker::new(client);

        let results = vec![
            make_result("c1", "Result 1", 0.9),
            make_result("c2", "Result 2", 0.8),
            make_result("c3", "Result 3", 0.7),
            make_result("c4", "Result 4", 0.6),
            make_result("c5", "Result 5", 0.5),
        ];

        let reranked = reranker.rerank("test query", results, 2).await.unwrap();

        assert_eq!(reranked.len(), 2);
    }

    #[tokio::test]
    async fn test_noop_reranker() {
        let reranker = NoOpReranker;

        let results = vec![
            make_result("c1", "First", 0.9),
            make_result("c2", "Second", 0.7),
        ];

        let reranked = reranker.rerank("query", results.clone(), 5).await.unwrap();

        assert_eq!(reranked.len(), 2);
        assert_eq!(reranked[0].chunk_id, "c1");
        assert_eq!(reranked[1].chunk_id, "c2");
    }

    #[test]
    fn test_parse_scores_valid() {
        let reranker = LlmReranker::new(Arc::new(MockLlmClient {
            response: String::new(),
        }));

        let scores = reranker.parse_scores("[0.9, 0.7, 0.5]", 3).unwrap();
        assert_eq!(scores, vec![0.9, 0.7, 0.5]);
    }

    #[test]
    fn test_parse_scores_with_surrounding_text() {
        let reranker = LlmReranker::new(Arc::new(MockLlmClient {
            response: String::new(),
        }));

        let scores = reranker
            .parse_scores("Here are the scores: [0.8, 0.6] based on relevance.", 2)
            .unwrap();
        assert_eq!(scores, vec![0.8, 0.6]);
    }

    #[test]
    fn test_parse_scores_clamps_values() {
        let reranker = LlmReranker::new(Arc::new(MockLlmClient {
            response: String::new(),
        }));

        let scores = reranker.parse_scores("[1.5, -0.2, 0.5]", 3).unwrap();
        assert_eq!(scores, vec![1.0, 0.0, 0.5]);
    }
}
