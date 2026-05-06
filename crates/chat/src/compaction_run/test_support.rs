//! Shared test helpers for the LLM-backed compaction strategies.
//!
//! Only compiled in `#[cfg(test)]` (gated at the `mod test_support;`
//! declaration in the parent). Provides a canned-stream [`StubProvider`]
//! that can also capture whether a unique "needle" string appears in the
//! prompt it receives — used by the `structured` mode tests to assert
//! iterative re-compaction forwards the previous summary body into the
//! prompt.

#![allow(clippy::unwrap_used, clippy::expect_used)]
#![allow(dead_code)] // Some helpers are only exercised in llm-compaction builds.

use {
    anyhow::Result,
    async_trait::async_trait,
    futures::Stream,
    moltis_agents::model::{
        ChatMessage, CompletionResponse, ContentPart, LlmProvider, StreamEvent, Usage, UserContent,
    },
    serde_json::Value,
    std::{
        pin::Pin,
        sync::{Arc, Mutex},
    },
};

/// Stub provider that emits a canned sequence of stream events.
///
/// - `context_window` lets the caller force the tail-budget math into the
///   cutting regime (tests typically pass `200`).
/// - `events` is the full sequence returned by `stream()` on every call.
/// - `needle` is an optional text fragment that, when set, is matched
///   against every forwarded [`ChatMessage`]. If any message contains it,
///   `saw_needle` flips to `true` — useful for asserting that iterative
///   re-compaction forwards prior summary content.
pub(super) struct StubProvider {
    pub events: Vec<StreamEvent>,
    pub context_window: u32,
    pub needle: Option<String>,
    pub saw_needle: Arc<Mutex<bool>>,
}

impl StubProvider {
    /// Build a stub that streams `body` as a single `Delta` then `Done`.
    pub fn new_ok(body: &str) -> Self {
        Self {
            events: vec![
                StreamEvent::Delta(body.to_string()),
                StreamEvent::Done(Usage::default()),
            ],
            context_window: 150,
            needle: None,
            saw_needle: Arc::new(Mutex::new(false)),
        }
    }

    /// Build a stub that immediately yields a `StreamEvent::Error`.
    pub fn new_error(msg: &str) -> Self {
        Self {
            events: vec![StreamEvent::Error(msg.to_string())],
            context_window: 150,
            needle: None,
            saw_needle: Arc::new(Mutex::new(false)),
        }
    }

    /// Build a stub that emits only `Done` — simulates an empty summary.
    pub fn new_empty_summary() -> Self {
        Self {
            events: vec![StreamEvent::Done(Usage::default())],
            context_window: 150,
            needle: None,
            saw_needle: Arc::new(Mutex::new(false)),
        }
    }

    /// Record whether any forwarded message contains `needle`.
    pub fn with_needle(mut self, needle: impl Into<String>) -> Self {
        self.needle = Some(needle.into());
        self
    }

    /// True if `stream()` saw at least one message containing the needle.
    pub fn saw_needle(&self) -> bool {
        *self
            .saw_needle
            .lock()
            .expect("stub provider mutex poisoned")
    }
}

fn message_contains(msg: &ChatMessage, needle: &str) -> bool {
    match msg {
        ChatMessage::System { content } => content.contains(needle),
        ChatMessage::User {
            content: UserContent::Text(t),
            ..
        } => t.contains(needle),
        ChatMessage::User {
            content: UserContent::Multimodal(parts),
            ..
        } => parts
            .iter()
            .any(|p| matches!(p, ContentPart::Text(t) if t.contains(needle))),
        ChatMessage::Assistant {
            content: Some(text),
            ..
        } => text.contains(needle),
        ChatMessage::Tool { content, .. } => content.contains(needle),
        _ => false,
    }
}

#[async_trait]
impl LlmProvider for StubProvider {
    fn name(&self) -> &str {
        "stub"
    }

    fn id(&self) -> &str {
        "stub::compaction"
    }

    fn context_window(&self) -> u32 {
        self.context_window
    }

    async fn complete(
        &self,
        _messages: &[ChatMessage],
        _tools: &[Value],
    ) -> Result<CompletionResponse> {
        anyhow::bail!("stub does not implement complete")
    }

    fn stream(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        if let Some(needle) = &self.needle
            && messages.iter().any(|m| message_contains(m, needle))
        {
            *self
                .saw_needle
                .lock()
                .expect("stub provider mutex poisoned") = true;
        }
        let events = self.events.clone();
        Box::pin(tokio_stream::iter(events))
    }
}
