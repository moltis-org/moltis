/// Batch embedding support using OpenAI's `/v1/batches` API for 50% cost reduction.
///
/// When enabled and the number of texts exceeds `batch_threshold`, uploads a JSONL file
/// to the batch API, polls for completion, and downloads results. Falls back to sequential
/// embedding on failure or timeout.
use anyhow::Result;
use {
    async_trait::async_trait,
    secrecy::{ExposeSecret, Secret},
    serde::Deserialize,
    tracing::{debug, info, warn},
};

use crate::embeddings::EmbeddingProvider;

/// Wraps an existing `EmbeddingProvider` and optionally uses the batch API for large batches.
pub struct BatchEmbeddingProvider {
    inner: Box<dyn EmbeddingProvider>,
    client: reqwest::Client,
    api_key: Secret<String>,
    base_url: String,
    batch_threshold: usize,
}

impl BatchEmbeddingProvider {
    pub fn new(
        inner: Box<dyn EmbeddingProvider>,
        api_key: Secret<String>,
        base_url: String,
        batch_threshold: usize,
    ) -> Self {
        Self {
            inner,
            client: reqwest::Client::new(),
            api_key,
            base_url,
            batch_threshold,
        }
    }

    /// Upload JSONL, create batch, poll until done, download results.
    async fn batch_embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        // 1. Build JSONL content
        let model = self.inner.model_name().to_string();
        let mut jsonl = String::new();
        for (i, text) in texts.iter().enumerate() {
            let line = serde_json::json!({
                "custom_id": format!("emb-{i}"),
                "method": "POST",
                "url": "/v1/embeddings",
                "body": {
                    "model": model,
                    "input": text,
                }
            });
            jsonl.push_str(&serde_json::to_string(&line)?);
            jsonl.push('\n');
        }

        // 2. Upload file
        let form = reqwest::multipart::Form::new()
            .text("purpose", "batch")
            .part(
                "file",
                reqwest::multipart::Part::bytes(jsonl.into_bytes())
                    .file_name("embeddings.jsonl")
                    .mime_str("application/jsonl")?,
            );

        let file_resp: FileUploadResponse = self
            .client
            .post(format!("{}/v1/files", self.base_url))
            .bearer_auth(self.api_key.expose_secret())
            .multipart(form)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        debug!(file_id = %file_resp.id, "batch: uploaded JSONL file");

        // 3. Create batch
        let batch_req = serde_json::json!({
            "input_file_id": file_resp.id,
            "endpoint": "/v1/embeddings",
            "completion_window": "24h",
        });

        let batch_resp: BatchResponse = self
            .client
            .post(format!("{}/v1/batches", self.base_url))
            .bearer_auth(self.api_key.expose_secret())
            .json(&batch_req)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let batch_id = batch_resp.id;
        info!(batch_id = %batch_id, "batch: created embedding batch");

        // 4. Poll for completion (60-minute timeout)
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60 * 60);
        let mut poll_interval = std::time::Duration::from_secs(5);

        loop {
            if tokio::time::Instant::now() > deadline {
                anyhow::bail!("batch embedding timed out after 60 minutes");
            }

            tokio::time::sleep(poll_interval).await;
            poll_interval = std::cmp::min(poll_interval * 2, std::time::Duration::from_secs(60));

            let status: BatchResponse = self
                .client
                .get(format!("{}/v1/batches/{}", self.base_url, batch_id))
                .bearer_auth(self.api_key.expose_secret())
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;

            debug!(batch_id = %batch_id, status = %status.status, "batch: polling");

            match status.status.as_str() {
                "completed" => {
                    let output_file_id = status
                        .output_file_id
                        .ok_or_else(|| anyhow::anyhow!("batch completed but no output file"))?;

                    // 5. Download results
                    let content = self
                        .client
                        .get(format!(
                            "{}/v1/files/{}/content",
                            self.base_url, output_file_id
                        ))
                        .bearer_auth(self.api_key.expose_secret())
                        .send()
                        .await?
                        .error_for_status()?
                        .text()
                        .await?;

                    // Parse JSONL results
                    let mut results: Vec<(usize, Vec<f32>)> = Vec::with_capacity(texts.len());
                    for line in content.lines() {
                        if line.trim().is_empty() {
                            continue;
                        }
                        let entry: BatchResultEntry = serde_json::from_str(line)?;
                        let idx: usize = entry
                            .custom_id
                            .strip_prefix("emb-")
                            .and_then(|s| s.parse().ok())
                            .ok_or_else(|| {
                                anyhow::anyhow!("invalid custom_id: {}", entry.custom_id)
                            })?;
                        if let Some(body) = entry.response.body
                            && let Some(first) = body.data.into_iter().next()
                        {
                            results.push((idx, first.embedding));
                        }
                    }

                    // Sort by index and extract embeddings
                    results.sort_by_key(|(i, _)| *i);
                    let embeddings: Vec<Vec<f32>> =
                        results.into_iter().map(|(_, emb)| emb).collect();

                    if embeddings.len() != texts.len() {
                        anyhow::bail!(
                            "batch returned {} embeddings for {} texts",
                            embeddings.len(),
                            texts.len()
                        );
                    }

                    info!(
                        batch_id = %batch_id,
                        count = embeddings.len(),
                        "batch: embedding complete"
                    );
                    return Ok(embeddings);
                },
                "failed" | "expired" | "cancelled" => {
                    anyhow::bail!("batch {} ended with status: {}", batch_id, status.status);
                },
                _ => continue, // in_progress, validating, etc.
            }
        }
    }
}

#[async_trait]
impl EmbeddingProvider for BatchEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        self.inner.embed(text).await
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.len() >= self.batch_threshold {
            match self.batch_embed(texts).await {
                Ok(results) => return Ok(results),
                Err(e) => {
                    warn!(error = %e, "batch API failed, falling back to sequential");
                },
            }
        }
        self.inner.embed_batch(texts).await
    }

    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    fn dimensions(&self) -> usize {
        self.inner.dimensions()
    }

    fn provider_key(&self) -> &str {
        self.inner.provider_key()
    }
}

// ── API response types ──────────────────────────────────────────────────

#[derive(Deserialize)]
struct FileUploadResponse {
    id: String,
}

#[derive(Deserialize)]
struct BatchResponse {
    id: String,
    status: String,
    output_file_id: Option<String>,
}

#[derive(Deserialize)]
struct BatchResultEntry {
    custom_id: String,
    response: BatchResultResponse,
}

#[derive(Deserialize)]
struct BatchResultResponse {
    body: Option<BatchEmbeddingBody>,
}

#[derive(Deserialize)]
struct BatchEmbeddingBody {
    data: Vec<BatchEmbeddingData>,
}

#[derive(Deserialize)]
struct BatchEmbeddingData {
    embedding: Vec<f32>,
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    struct MockInner;

    #[async_trait]
    impl EmbeddingProvider for MockInner {
        async fn embed(&self, _text: &str) -> Result<Vec<f32>> {
            Ok(vec![1.0; 8])
        }

        async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            Ok(texts.iter().map(|_| vec![1.0; 8]).collect())
        }

        fn model_name(&self) -> &str {
            "mock"
        }

        fn dimensions(&self) -> usize {
            8
        }

        fn provider_key(&self) -> &str {
            "mock"
        }
    }

    #[tokio::test]
    async fn test_below_threshold_uses_inner() {
        let provider = BatchEmbeddingProvider::new(
            Box::new(MockInner),
            Secret::new("test-key".into()),
            "https://api.openai.com".into(),
            50, // threshold
        );

        // Below threshold → uses inner directly
        let texts: Vec<String> = (0..10).map(|i| format!("text {i}")).collect();
        let results = provider.embed_batch(&texts).await.unwrap();
        assert_eq!(results.len(), 10);
        assert_eq!(results[0], vec![1.0; 8]);
    }

    #[tokio::test]
    async fn test_single_embed_uses_inner() {
        let provider = BatchEmbeddingProvider::new(
            Box::new(MockInner),
            Secret::new("test-key".into()),
            "https://api.openai.com".into(),
            50,
        );

        let result = provider.embed("hello").await.unwrap();
        assert_eq!(result, vec![1.0; 8]);
    }
}
