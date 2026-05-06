/// Response from an LLM completion call.
#[derive(Debug)]
pub struct CompletionResponse {
    pub text: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
}

pub const MAX_CAPTURED_PROVIDER_RAW_EVENTS: usize = 256;

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
    /// Provider-specific opaque metadata to round-trip (e.g. Gemini `thought_signature`).
    /// Only allowlisted keys are extracted; see [`TOOL_CALL_METADATA_KEYS`].
    pub metadata: Option<serde_json::Map<String, serde_json::Value>>,
}

/// Keys extracted from provider tool-call JSON into [`ToolCall::metadata`].
pub const TOOL_CALL_METADATA_KEYS: &[&str] = &["thought_signature"];

#[derive(Debug, Clone, Default)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_write_tokens: u32,
}

impl Usage {
    #[must_use]
    pub fn saturating_add(&self, other: &Self) -> Self {
        Self {
            input_tokens: self.input_tokens.saturating_add(other.input_tokens),
            output_tokens: self.output_tokens.saturating_add(other.output_tokens),
            cache_read_tokens: self
                .cache_read_tokens
                .saturating_add(other.cache_read_tokens),
            cache_write_tokens: self
                .cache_write_tokens
                .saturating_add(other.cache_write_tokens),
        }
    }

    pub fn saturating_add_assign(&mut self, other: &Self) {
        *self = self.saturating_add(other);
    }
}

pub fn push_capped_provider_raw_event(
    raw_events: &mut Vec<serde_json::Value>,
    raw_event: serde_json::Value,
) {
    if raw_events.len() < MAX_CAPTURED_PROVIDER_RAW_EVENTS {
        raw_events.push(raw_event);
    }
}

/// Runtime model metadata fetched from provider APIs.
#[derive(Debug, Clone)]
pub struct ModelMetadata {
    pub id: String,
    pub context_length: u32,
}
