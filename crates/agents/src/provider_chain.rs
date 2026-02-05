//! Provider failover chain with per-provider circuit breakers.
//!
//! `ProviderChain` wraps a primary `LlmProvider` with a list of fallbacks.
//! When the primary fails with a retryable error (rate limit, auth, server error),
//! it automatically tries the next provider in the chain, skipping any that have
//! their circuit breaker tripped.

use std::{
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use {async_trait::async_trait, tokio_stream::Stream, tracing::warn};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, histogram, labels, llm as llm_metrics};

use crate::model::{CompletionResponse, LlmProvider, StreamEvent};

/// How a provider error should be handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderErrorKind {
    /// 429 — rotate to next provider.
    RateLimit,
    /// 401/403 — rotate (bad key or permissions).
    AuthError,
    /// 5xx — rotate to next provider.
    ServerError,
    /// Billing/usage limit exhausted — rotate.
    BillingExhausted,
    /// Context window exceeded — don't rotate, caller should compact.
    ContextWindow,
    /// 400, bad format — don't rotate, it'll fail everywhere.
    InvalidRequest,
    /// Unrecognised error — attempt failover.
    Unknown,
}

impl ProviderErrorKind {
    /// Whether this error kind should trigger failover to the next provider.
    #[must_use]
    pub fn should_failover(self) -> bool {
        matches!(
            self,
            Self::RateLimit
                | Self::AuthError
                | Self::ServerError
                | Self::BillingExhausted
                | Self::Unknown
        )
    }
}

/// Error patterns for context window overflow (reused from runner.rs).
const CONTEXT_WINDOW_PATTERNS: &[&str] = &[
    "context_length_exceeded",
    "max_tokens",
    "too many tokens",
    "request too large",
    "maximum context length",
    "context window",
    "token limit",
    "content_too_large",
    "request_too_large",
];

/// Classify an error into a `ProviderErrorKind` based on the error message.
#[must_use]
pub fn classify_error(err: &anyhow::Error) -> ProviderErrorKind {
    let msg = err.to_string().to_lowercase();

    // Context window — must check first since "request too large" overlaps.
    if CONTEXT_WINDOW_PATTERNS.iter().any(|p| msg.contains(p)) {
        return ProviderErrorKind::ContextWindow;
    }

    // Rate limiting.
    if msg.contains("429")
        || msg.contains("rate limit")
        || msg.contains("rate_limit")
        || msg.contains("too many requests")
    {
        return ProviderErrorKind::RateLimit;
    }

    // Auth errors.
    if msg.contains("401")
        || msg.contains("403")
        || msg.contains("unauthorized")
        || msg.contains("forbidden")
        || msg.contains("invalid api key")
        || msg.contains("invalid_api_key")
        || msg.contains("authentication")
    {
        return ProviderErrorKind::AuthError;
    }

    // Billing / quota exhaustion.
    if msg.contains("billing")
        || msg.contains("quota")
        || msg.contains("insufficient_quota")
        || msg.contains("usage limit")
        || msg.contains("credit")
    {
        return ProviderErrorKind::BillingExhausted;
    }

    // Server errors.
    if msg.contains("500")
        || msg.contains("502")
        || msg.contains("503")
        || msg.contains("504")
        || msg.contains("internal server error")
        || msg.contains("bad gateway")
        || msg.contains("service unavailable")
        || msg.contains("overloaded")
    {
        return ProviderErrorKind::ServerError;
    }

    // Invalid request (400-level, non-auth, non-rate-limit).
    if msg.contains("400") || msg.contains("bad request") || msg.contains("invalid_request") {
        return ProviderErrorKind::InvalidRequest;
    }

    ProviderErrorKind::Unknown
}

// ── Circuit breaker (same pattern as embeddings_fallback.rs) ─────────────

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
        *self.last_failure.lock().unwrap() = Some(Instant::now());
    }

    /// Returns `true` when the circuit is open (provider should be skipped).
    /// Trips after 3 consecutive failures; resets after 60s cooldown.
    fn is_tripped(&self) -> bool {
        let failures = self.consecutive_failures.load(Ordering::SeqCst);
        if failures < 3 {
            return false;
        }
        let last = self.last_failure.lock().unwrap();
        match *last {
            Some(t) if t.elapsed() < Duration::from_secs(60) => true,
            _ => {
                drop(last);
                self.consecutive_failures.store(0, Ordering::SeqCst);
                false
            },
        }
    }
}

/// A provider entry in the failover chain.
struct ChainEntry {
    provider: Arc<dyn LlmProvider>,
    state: ProviderState,
}

/// Failover chain that tries providers in order, with circuit breakers.
///
/// Implements `LlmProvider` itself so callers don't need to know about failover.
pub struct ProviderChain {
    chain: Vec<ChainEntry>,
}

impl ProviderChain {
    /// Build a chain from a list of providers (primary first, then fallbacks).
    pub fn new(providers: Vec<Arc<dyn LlmProvider>>) -> Self {
        let chain = providers
            .into_iter()
            .map(|provider| ChainEntry {
                provider,
                state: ProviderState::new(),
            })
            .collect();
        Self { chain }
    }

    /// Build a chain with one provider (no failover). Useful as a passthrough.
    pub fn single(provider: Arc<dyn LlmProvider>) -> Self {
        Self::new(vec![provider])
    }

    fn primary(&self) -> &ChainEntry {
        &self.chain[0]
    }
}

#[async_trait]
impl LlmProvider for ProviderChain {
    fn name(&self) -> &str {
        self.primary().provider.name()
    }

    fn id(&self) -> &str {
        self.primary().provider.id()
    }

    fn supports_tools(&self) -> bool {
        self.primary().provider.supports_tools()
    }

    fn context_window(&self) -> u32 {
        self.primary().provider.context_window()
    }

    async fn complete(
        &self,
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
    ) -> anyhow::Result<CompletionResponse> {
        let mut errors = Vec::new();
        #[cfg(feature = "metrics")]
        let start = Instant::now();

        for entry in &self.chain {
            if entry.state.is_tripped() {
                continue;
            }

            let provider_name = entry.provider.name().to_string();
            let model_id = entry.provider.id().to_string();

            match entry.provider.complete(messages, tools).await {
                Ok(resp) => {
                    entry.state.record_success();

                    // Record metrics on successful completion
                    #[cfg(feature = "metrics")]
                    {
                        let duration = start.elapsed().as_secs_f64();

                        counter!(
                            llm_metrics::COMPLETIONS_TOTAL,
                            labels::PROVIDER => provider_name.clone(),
                            labels::MODEL => model_id.clone()
                        )
                        .increment(1);

                        counter!(
                            llm_metrics::INPUT_TOKENS_TOTAL,
                            labels::PROVIDER => provider_name.clone(),
                            labels::MODEL => model_id.clone()
                        )
                        .increment(u64::from(resp.usage.input_tokens));

                        counter!(
                            llm_metrics::OUTPUT_TOKENS_TOTAL,
                            labels::PROVIDER => provider_name.clone(),
                            labels::MODEL => model_id.clone()
                        )
                        .increment(u64::from(resp.usage.output_tokens));

                        histogram!(
                            llm_metrics::COMPLETION_DURATION_SECONDS,
                            labels::PROVIDER => provider_name,
                            labels::MODEL => model_id
                        )
                        .record(duration);
                    }

                    return Ok(resp);
                },
                Err(e) => {
                    let kind = classify_error(&e);
                    entry.state.record_failure();

                    // Record error metrics
                    #[cfg(feature = "metrics")]
                    {
                        counter!(
                            llm_metrics::COMPLETION_ERRORS_TOTAL,
                            labels::PROVIDER => provider_name.clone(),
                            labels::MODEL => model_id.clone(),
                            labels::ERROR_TYPE => format!("{kind:?}")
                        )
                        .increment(1);
                    }

                    if !kind.should_failover() {
                        // Non-retryable error — propagate immediately.
                        return Err(e);
                    }

                    warn!(
                        provider = entry.provider.id(),
                        error = %e,
                        kind = ?kind,
                        "provider failed, trying next in chain"
                    );
                    errors.push(format!("{}: {e}", entry.provider.id()));
                },
            }
        }

        anyhow::bail!(
            "all providers in failover chain failed: {}",
            errors.join("; ")
        )
    }

    fn stream(
        &self,
        messages: Vec<serde_json::Value>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        // For streaming, we try the first non-tripped provider.
        // If the stream yields an Error event, we can't transparently retry mid-stream,
        // so we pick the best available provider upfront.
        for entry in &self.chain {
            if !entry.state.is_tripped() {
                return entry.provider.stream(messages);
            }
        }
        // All tripped — try primary anyway (it may have cooled down by now).
        self.primary().provider.stream(messages)
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::model::{StreamEvent, Usage},
        async_trait::async_trait,
        tokio_stream::StreamExt,
    };

    /// A mock provider that always succeeds.
    struct SuccessProvider {
        id: &'static str,
    }

    #[async_trait]
    impl LlmProvider for SuccessProvider {
        fn name(&self) -> &str {
            "success"
        }

        fn id(&self) -> &str {
            self.id
        }

        async fn complete(
            &self,
            _messages: &[serde_json::Value],
            _tools: &[serde_json::Value],
        ) -> anyhow::Result<CompletionResponse> {
            Ok(CompletionResponse {
                text: Some("ok".into()),
                tool_calls: vec![],
                usage: Usage {
                    input_tokens: 1,
                    output_tokens: 1,
                },
            })
        }

        fn stream(
            &self,
            _messages: Vec<serde_json::Value>,
        ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
            Box::pin(tokio_stream::once(StreamEvent::Done(Usage {
                input_tokens: 1,
                output_tokens: 1,
            })))
        }
    }

    /// A mock provider that always fails with a configurable error message.
    struct FailingProvider {
        id: &'static str,
        error_msg: &'static str,
    }

    #[async_trait]
    impl LlmProvider for FailingProvider {
        fn name(&self) -> &str {
            "failing"
        }

        fn id(&self) -> &str {
            self.id
        }

        async fn complete(
            &self,
            _messages: &[serde_json::Value],
            _tools: &[serde_json::Value],
        ) -> anyhow::Result<CompletionResponse> {
            anyhow::bail!("{}", self.error_msg)
        }

        fn stream(
            &self,
            _messages: Vec<serde_json::Value>,
        ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
            Box::pin(tokio_stream::once(StreamEvent::Error(
                self.error_msg.into(),
            )))
        }
    }

    #[tokio::test]
    async fn primary_succeeds_no_failover() {
        let chain = ProviderChain::new(vec![
            Arc::new(SuccessProvider { id: "primary" }),
            Arc::new(SuccessProvider { id: "fallback" }),
        ]);

        let resp = chain.complete(&[], &[]).await.unwrap();
        assert_eq!(resp.text.as_deref(), Some("ok"));
        assert_eq!(chain.id(), "primary");
    }

    #[tokio::test]
    async fn failover_on_rate_limit() {
        let chain = ProviderChain::new(vec![
            Arc::new(FailingProvider {
                id: "primary",
                error_msg: "429 rate limit exceeded",
            }),
            Arc::new(SuccessProvider { id: "fallback" }),
        ]);

        let resp = chain.complete(&[], &[]).await.unwrap();
        assert_eq!(resp.text.as_deref(), Some("ok"));
    }

    #[tokio::test]
    async fn failover_on_server_error() {
        let chain = ProviderChain::new(vec![
            Arc::new(FailingProvider {
                id: "primary",
                error_msg: "500 internal server error",
            }),
            Arc::new(SuccessProvider { id: "fallback" }),
        ]);

        let resp = chain.complete(&[], &[]).await.unwrap();
        assert_eq!(resp.text.as_deref(), Some("ok"));
    }

    #[tokio::test]
    async fn failover_on_auth_error() {
        let chain = ProviderChain::new(vec![
            Arc::new(FailingProvider {
                id: "primary",
                error_msg: "401 unauthorized: invalid api key",
            }),
            Arc::new(SuccessProvider { id: "fallback" }),
        ]);

        let resp = chain.complete(&[], &[]).await.unwrap();
        assert_eq!(resp.text.as_deref(), Some("ok"));
    }

    #[tokio::test]
    async fn no_failover_on_context_window() {
        let chain = ProviderChain::new(vec![
            Arc::new(FailingProvider {
                id: "primary",
                error_msg: "context_length_exceeded: too many tokens",
            }),
            Arc::new(SuccessProvider { id: "fallback" }),
        ]);

        let err = chain.complete(&[], &[]).await.unwrap_err();
        assert!(err.to_string().contains("context_length_exceeded"));
    }

    #[tokio::test]
    async fn no_failover_on_invalid_request() {
        let chain = ProviderChain::new(vec![
            Arc::new(FailingProvider {
                id: "primary",
                error_msg: "400 bad request: invalid_request",
            }),
            Arc::new(SuccessProvider { id: "fallback" }),
        ]);

        let err = chain.complete(&[], &[]).await.unwrap_err();
        assert!(err.to_string().contains("bad request"));
    }

    #[tokio::test]
    async fn all_providers_fail() {
        let chain = ProviderChain::new(vec![
            Arc::new(FailingProvider {
                id: "a",
                error_msg: "429 rate limit",
            }),
            Arc::new(FailingProvider {
                id: "b",
                error_msg: "503 service unavailable",
            }),
        ]);

        let err = chain.complete(&[], &[]).await.unwrap_err();
        assert!(
            err.to_string()
                .contains("all providers in failover chain failed")
        );
    }

    #[tokio::test]
    async fn circuit_breaker_trips_after_three_failures() {
        let chain = ProviderChain::new(vec![
            Arc::new(FailingProvider {
                id: "flaky",
                error_msg: "500 internal server error",
            }),
            Arc::new(SuccessProvider { id: "backup" }),
        ]);

        // Fail 3 times to trip the circuit breaker on the first provider.
        for _ in 0..3 {
            let _ = chain.complete(&[], &[]).await;
        }

        // After tripping, the flaky provider should be skipped.
        assert!(chain.chain[0].state.is_tripped());
    }

    #[tokio::test]
    async fn stream_uses_first_non_tripped() {
        let chain = ProviderChain::new(vec![
            Arc::new(FailingProvider {
                id: "tripped",
                error_msg: "500 error",
            }),
            Arc::new(SuccessProvider { id: "backup" }),
        ]);

        // Trip the first provider.
        for _ in 0..3 {
            let _ = chain.complete(&[], &[]).await;
        }

        // Stream should use backup.
        let mut stream = chain.stream(vec![]);
        let event = stream.next().await.unwrap();
        assert!(matches!(event, StreamEvent::Done(_)));
    }

    #[test]
    fn classify_rate_limit() {
        let err = anyhow::anyhow!("429 Too Many Requests: rate limit exceeded");
        assert_eq!(classify_error(&err), ProviderErrorKind::RateLimit);
    }

    #[test]
    fn classify_auth() {
        let err = anyhow::anyhow!("401 Unauthorized");
        assert_eq!(classify_error(&err), ProviderErrorKind::AuthError);
    }

    #[test]
    fn classify_server() {
        let err = anyhow::anyhow!("502 Bad Gateway");
        assert_eq!(classify_error(&err), ProviderErrorKind::ServerError);
    }

    #[test]
    fn classify_context_window() {
        let err = anyhow::anyhow!("context_length_exceeded: maximum context length is 200000");
        assert_eq!(classify_error(&err), ProviderErrorKind::ContextWindow);
    }

    #[test]
    fn classify_billing() {
        let err = anyhow::anyhow!("insufficient_quota: billing limit reached");
        assert_eq!(classify_error(&err), ProviderErrorKind::BillingExhausted);
    }

    #[test]
    fn classify_invalid_request() {
        let err = anyhow::anyhow!("400 Bad Request: invalid JSON");
        assert_eq!(classify_error(&err), ProviderErrorKind::InvalidRequest);
    }

    #[test]
    fn classify_unknown() {
        let err = anyhow::anyhow!("connection reset by peer");
        assert_eq!(classify_error(&err), ProviderErrorKind::Unknown);
    }

    #[test]
    fn should_failover_mapping() {
        assert!(ProviderErrorKind::RateLimit.should_failover());
        assert!(ProviderErrorKind::AuthError.should_failover());
        assert!(ProviderErrorKind::ServerError.should_failover());
        assert!(ProviderErrorKind::BillingExhausted.should_failover());
        assert!(ProviderErrorKind::Unknown.should_failover());
        assert!(!ProviderErrorKind::ContextWindow.should_failover());
        assert!(!ProviderErrorKind::InvalidRequest.should_failover());
    }

    #[test]
    fn single_provider_chain() {
        let chain = ProviderChain::single(Arc::new(SuccessProvider { id: "only" }));
        assert_eq!(chain.id(), "only");
        assert_eq!(chain.chain.len(), 1);
    }
}
