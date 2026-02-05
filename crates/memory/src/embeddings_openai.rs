/// OpenAI embeddings provider using the `/v1/embeddings` endpoint.
use async_trait::async_trait;
use {
    secrecy::ExposeSecret,
    serde::{Deserialize, Serialize},
    sha2::{Digest, Sha256},
};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, histogram, labels, memory as mem_metrics};

use crate::embeddings::EmbeddingProvider;

pub struct OpenAiEmbeddingProvider {
    client: reqwest::Client,
    api_key: secrecy::Secret<String>,
    base_url: String,
    model: String,
    dims: usize,
    provider_key: String,
}

fn compute_provider_key(base_url: &str, model: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"openai:");
    hasher.update(base_url.as_bytes());
    hasher.update(b":");
    hasher.update(model.as_bytes());
    format!("{:x}", hasher.finalize())[..16].to_string()
}

impl OpenAiEmbeddingProvider {
    pub fn new(api_key: String) -> Self {
        let base_url = "https://api.openai.com".to_string();
        let model = "text-embedding-3-small".to_string();
        let provider_key = compute_provider_key(&base_url, &model);
        Self {
            client: reqwest::Client::new(),
            api_key: secrecy::Secret::new(api_key),
            base_url,
            model,
            dims: 1536,
            provider_key,
        }
    }

    pub fn with_model(mut self, model: String, dims: usize) -> Self {
        self.model = model;
        self.dims = dims;
        self.provider_key = compute_provider_key(&self.base_url, &self.model);
        self
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self.provider_key = compute_provider_key(&self.base_url, &self.model);
        self
    }
}

#[derive(Serialize)]
struct EmbeddingRequest {
    model: String,
    input: Vec<String>,
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

#[async_trait]
impl EmbeddingProvider for OpenAiEmbeddingProvider {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        self.embed_batch(&[text.to_string()])
            .await?
            .pop()
            .ok_or_else(|| anyhow::anyhow!("empty embedding response"))
    }

    async fn embed_batch(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        #[cfg(feature = "metrics")]
        let start = std::time::Instant::now();

        #[cfg(feature = "metrics")]
        counter!(mem_metrics::EMBEDDINGS_GENERATED_TOTAL, labels::PROVIDER => "openai")
            .increment(texts.len() as u64);

        let req = EmbeddingRequest {
            model: self.model.clone(),
            input: texts.to_vec(),
        };

        let result = self
            .client
            .post(format!("{}/v1/embeddings", self.base_url))
            .bearer_auth(self.api_key.expose_secret())
            .json(&req)
            .send()
            .await?
            .error_for_status()?
            .json::<EmbeddingResponse>()
            .await;

        #[cfg(feature = "metrics")]
        histogram!(
            "moltis_memory_embedding_duration_seconds",
            labels::PROVIDER => "openai"
        )
        .record(start.elapsed().as_secs_f64());

        let resp = result?;
        Ok(resp.data.into_iter().map(|d| d.embedding).collect())
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    fn provider_key(&self) -> &str {
        &self.provider_key
    }
}
