/// Fallback chain embedding provider: tries providers in order with circuit breaker.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use {
    async_trait::async_trait,
    tracing::{info, warn},
};

use crate::embeddings::EmbeddingProvider;

/// Circuit breaker state for a single provider.
struct ProviderState {
    consecutive_failures: AtomicUsize,
    last_failure: Mutex<Option<Instant>>,
}

impl ProviderState {
    fn new() -> Self {
        Self {
            consecutive_failures: AtomicUsize::new(0),
            last_failure: Mutex::new(None),
        }
    }

    fn record_success(&self) {
        self.consecutive_failures.store(0, Ordering::SeqCst);
    }

    fn record_failure(&self) {
        self.consecutive_failures.fetch_add(1, Ordering::SeqCst);
        *self.last_failure.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    }

    fn is_tripped(&self) -> bool {
        let failures = self.consecutive_failures.load(Ordering::SeqCst);
        if failures < 3 {
            return false;
        }
        // Check cooldown (60s)
        let last = self.last_failure.lock().unwrap_or_else(|e| e.into_inner());
        match *last {
            Some(t) if t.elapsed() < Duration::from_secs(60) => true,
            _ => {
                // Cooldown expired, reset
                drop(last);
                self.consecutive_failures.store(0, Ordering::SeqCst);
                false
            },
        }
    }
}

/// A provider entry in the fallback chain.
struct ChainEntry {
    name: String,
    provider: Box<dyn EmbeddingProvider>,
    state: ProviderState,
}

/// Tries embedding providers in order, skipping those with circuit breaker tripped.
pub struct FallbackEmbeddingProvider {
    chain: Vec<ChainEntry>,
    active: AtomicUsize,
}

impl FallbackEmbeddingProvider {
    pub fn new(providers: Vec<(String, Box<dyn EmbeddingProvider>)>) -> Self {
        let chain = providers
            .into_iter()
            .map(|(name, provider)| ChainEntry {
                name,
                provider,
                state: ProviderState::new(),
            })
            .collect();
        Self {
            chain,
            active: AtomicUsize::new(0),
        }
    }

    pub fn provider_names(&self) -> Vec<&str> {
        self.chain.iter().map(|e| e.name.as_str()).collect()
    }

    pub fn active_provider_name(&self) -> &str {
        let idx = self.active.load(Ordering::SeqCst);
        self.chain
            .get(idx)
            .map(|e| e.name.as_str())
            .unwrap_or("none")
    }

    fn active_entry(&self) -> Option<&ChainEntry> {
        let idx = self.active.load(Ordering::SeqCst);
        self.chain.get(idx)
    }
}

#[async_trait]
impl EmbeddingProvider for FallbackEmbeddingProvider {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let mut errors = Vec::new();
        let start_idx = self.active.load(Ordering::SeqCst);

        for offset in 0..self.chain.len() {
            let idx = (start_idx + offset) % self.chain.len();
            let entry = &self.chain[idx];

            if entry.state.is_tripped() {
                continue;
            }

            match entry.provider.embed(text).await {
                Ok(result) => {
                    entry.state.record_success();
                    if idx != start_idx {
                        info!(
                            from = self.chain[start_idx].name,
                            to = entry.name,
                            "embedding fallback: switched active provider"
                        );
                        self.active.store(idx, Ordering::SeqCst);
                    }
                    return Ok(result);
                },
                Err(e) => {
                    warn!(provider = entry.name, error = %e, "embedding provider failed");
                    entry.state.record_failure();
                    errors.push(format!("{}: {e}", entry.name));
                },
            }
        }

        anyhow::bail!("all embedding providers failed: {}", errors.join("; "))
    }

    async fn embed_batch(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        let mut errors = Vec::new();
        let start_idx = self.active.load(Ordering::SeqCst);

        for offset in 0..self.chain.len() {
            let idx = (start_idx + offset) % self.chain.len();
            let entry = &self.chain[idx];

            if entry.state.is_tripped() {
                continue;
            }

            match entry.provider.embed_batch(texts).await {
                Ok(result) => {
                    entry.state.record_success();
                    if idx != start_idx {
                        info!(
                            from = self.chain[start_idx].name,
                            to = entry.name,
                            "embedding fallback: switched active provider (batch)"
                        );
                        self.active.store(idx, Ordering::SeqCst);
                    }
                    return Ok(result);
                },
                Err(e) => {
                    warn!(provider = entry.name, error = %e, "embedding provider failed (batch)");
                    entry.state.record_failure();
                    errors.push(format!("{}: {e}", entry.name));
                },
            }
        }

        anyhow::bail!(
            "all embedding providers failed (batch): {}",
            errors.join("; ")
        )
    }

    fn model_name(&self) -> &str {
        self.active_entry()
            .map(|e| e.provider.model_name())
            .unwrap_or("fallback")
    }

    fn dimensions(&self) -> usize {
        self.active_entry()
            .map(|e| e.provider.dimensions())
            .unwrap_or(0)
    }

    fn provider_key(&self) -> &str {
        self.active_entry()
            .map(|e| e.provider.provider_key())
            .unwrap_or("fallback")
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    struct FailingProvider;

    #[async_trait]
    impl EmbeddingProvider for FailingProvider {
        async fn embed(&self, _text: &str) -> anyhow::Result<Vec<f32>> {
            anyhow::bail!("always fails")
        }

        fn model_name(&self) -> &str {
            "failing"
        }

        fn dimensions(&self) -> usize {
            8
        }

        fn provider_key(&self) -> &str {
            "failing"
        }
    }

    struct SuccessProvider {
        name: &'static str,
    }

    #[async_trait]
    impl EmbeddingProvider for SuccessProvider {
        async fn embed(&self, _text: &str) -> anyhow::Result<Vec<f32>> {
            Ok(vec![1.0; 8])
        }

        fn model_name(&self) -> &str {
            self.name
        }

        fn dimensions(&self) -> usize {
            8
        }

        fn provider_key(&self) -> &str {
            self.name
        }
    }

    #[tokio::test]
    async fn test_fallback_first_succeeds() {
        let fb = FallbackEmbeddingProvider::new(vec![
            ("a".into(), Box::new(SuccessProvider { name: "a" })),
            ("b".into(), Box::new(SuccessProvider { name: "b" })),
        ]);

        let result = fb.embed("test").await.unwrap();
        assert_eq!(result.len(), 8);
        assert_eq!(fb.active_provider_name(), "a");
    }

    #[tokio::test]
    async fn test_fallback_to_second() {
        let fb = FallbackEmbeddingProvider::new(vec![
            ("fail".into(), Box::new(FailingProvider)),
            ("ok".into(), Box::new(SuccessProvider { name: "ok" })),
        ]);

        let result = fb.embed("test").await.unwrap();
        assert_eq!(result.len(), 8);
        assert_eq!(fb.active_provider_name(), "ok");
    }

    #[tokio::test]
    async fn test_all_fail() {
        let fb = FallbackEmbeddingProvider::new(vec![
            ("a".into(), Box::new(FailingProvider)),
            ("b".into(), Box::new(FailingProvider)),
        ]);

        let err = fb.embed("test").await.unwrap_err();
        assert!(err.to_string().contains("all embedding providers failed"));
    }
}
