use std::pin::Pin;

use {async_trait::async_trait, tokio_stream::Stream};

/// Events emitted during streaming LLM completion.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Text content delta.
    Delta(String),
    /// A tool call has started (content_block_start with tool_use).
    ToolCallStart {
        /// Tool call ID from the provider.
        id: String,
        /// Tool name being called.
        name: String,
        /// Index of this tool call in the response (0-based).
        index: usize,
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
        messages: &[serde_json::Value],
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

    /// Stream a completion, yielding delta/done/error events.
    fn stream(
        &self,
        messages: Vec<serde_json::Value>,
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
        messages: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        self.stream(messages)
    }
}

/// Response from an LLM completion call.
#[derive(Debug)]
pub struct CompletionResponse {
    pub text: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}
