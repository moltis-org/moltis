use std::{pin::Pin, sync::Arc, time::Duration};

use {async_trait::async_trait, futures::StreamExt, tokio_stream::Stream};

use super::{
    ReasoningEffort,
    chat::ChatMessage,
    types::{CompletionResponse, ModelMetadata, Usage},
};

// ── Stream events ───────────────────────────────────────────────────────────

/// Events emitted during streaming LLM completion.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Text content delta.
    Delta(String),
    /// Raw provider event payload (for debugging API responses).
    ProviderRaw(serde_json::Value),
    /// Reasoning/planning text delta (not user-visible final answer text).
    ReasoningDelta(String),
    /// A tool call has started (content_block_start with tool_use).
    ToolCallStart {
        /// Tool call ID from the provider.
        id: String,
        /// Tool name being called.
        name: String,
        /// Index of this tool call in the response (0-based).
        index: usize,
        /// Provider-specific metadata (e.g. Gemini `thought_signature`).
        metadata: Option<serde_json::Map<String, serde_json::Value>>,
    },
    /// Streaming delta for tool call arguments (JSON fragment).
    ToolCallArgumentsDelta {
        /// Index of the tool call this delta belongs to.
        index: usize,
        /// JSON fragment to append to the arguments.
        delta: String,
    },
    /// A tool call's arguments are complete.
    ToolCallComplete {
        /// Index of the completed tool call.
        index: usize,
    },
    /// Stream completed successfully.
    Done(Usage),
    /// An error occurred.
    Error(String),
}

/// LLM provider trait (Anthropic, OpenAI, Google, etc.).
#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;

    /// Model identifier (e.g. "claude-sonnet-4-20250514", "gpt-4o").
    fn id(&self) -> &str;

    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> anyhow::Result<CompletionResponse>;

    /// Whether this provider supports tool/function calling.
    /// Defaults to false; providers that handle the `tools` parameter
    /// in `complete()` should override this to return true.
    fn supports_tools(&self) -> bool {
        false
    }

    /// Context window size in tokens for this model.
    /// Used to detect when conversation approaches the limit and trigger auto-compact.
    fn context_window(&self) -> u32 {
        200_000
    }

    /// Whether this provider supports vision (image inputs).
    /// When true, tool results containing images will be sent as multimodal
    /// content blocks instead of stripping the image data.
    fn supports_vision(&self) -> bool {
        false
    }

    /// Configured tool mode for this provider, if any.
    ///
    /// Returns `None` when the provider has no explicit tool mode override
    /// (the caller should fall back to `Auto` behavior based on `supports_tools()`).
    fn tool_mode(&self) -> Option<moltis_config::ToolMode> {
        None
    }

    /// Stream a completion, yielding delta/done/error events.
    fn stream(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>>;

    /// Stream a completion with tool support.
    ///
    /// Like `stream()`, but accepts tool schemas and can emit `ToolCallStart`,
    /// `ToolCallArgumentsDelta`, and `ToolCallComplete` events in addition to
    /// text deltas.
    ///
    /// Default implementation falls back to `stream()` (ignoring tools).
    /// Providers with native streaming tool support should override this.
    fn stream_with_tools(
        &self,
        messages: Vec<ChatMessage>,
        _tools: Vec<serde_json::Value>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        self.stream(messages)
    }

    /// Configured reasoning effort for this provider instance, if any.
    ///
    /// Providers that support extended thinking (Anthropic, OpenAI o-series)
    /// use this value when building API requests.
    fn reasoning_effort(&self) -> Option<ReasoningEffort> {
        None
    }

    /// Return a new provider with reasoning effort set, if supported.
    ///
    /// Returns `None` for providers that don't support reasoning effort.
    /// Used by sub-agent spawning to apply per-agent reasoning settings
    /// without mutating the shared registry provider.
    fn with_reasoning_effort(
        self: Arc<Self>,
        _effort: ReasoningEffort,
    ) -> Option<Arc<dyn LlmProvider>> {
        None
    }

    /// Send the cheapest request available that proves the model can answer.
    ///
    /// The default implementation streams a tiny prompt and returns as soon as
    /// the first text delta or terminal event arrives. Providers can override
    /// this to use provider-specific low-cost probe requests.
    async fn probe(&self) -> anyhow::Result<()> {
        let probe = vec![ChatMessage::user("ping")];
        let mut stream = self.stream(probe);

        let result = tokio::time::timeout(Duration::from_secs(30), async {
            while let Some(event) = stream.next().await {
                match event {
                    StreamEvent::Delta(_) | StreamEvent::Done(_) => return Ok(()),
                    StreamEvent::Error(err) => return Err(anyhow::anyhow!(err)),
                    _ => continue,
                }
            }
            Err(anyhow::anyhow!("stream ended without producing any output"))
        })
        .await;

        drop(stream);

        match result {
            Ok(inner) => inner,
            Err(_) => Err(anyhow::anyhow!("Connection timed out after 30 seconds")),
        }
    }

    /// Fetch runtime model metadata from the provider API.
    ///
    /// The default implementation returns a `ModelMetadata` derived from the
    /// static `context_window()` value. Providers that support a `/models`
    /// endpoint can override this to fetch the actual context length at runtime.
    async fn model_metadata(&self) -> anyhow::Result<ModelMetadata> {
        Ok(ModelMetadata {
            id: self.id().to_string(),
            context_length: self.context_window(),
        })
    }
}
