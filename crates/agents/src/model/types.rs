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
    pub argument_diagnostic: Option<ToolCallArgumentDiagnostic>,
    /// Provider-specific opaque metadata to round-trip (e.g. Gemini `thought_signature`).
    /// Only allowlisted keys are extracted; see [`TOOL_CALL_METADATA_KEYS`].
    pub metadata: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCallArgumentSource {
    RepairedString,
    EmptyString,
    NullOrMissing,
    MalformedString,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallArgumentDiagnostic {
    pub source: ToolCallArgumentSource,
    pub raw_len: Option<usize>,
    pub raw_preview: Option<String>,
    pub parse_error: Option<String>,
}

impl ToolCallArgumentDiagnostic {
    #[must_use]
    pub fn short_summary(&self) -> String {
        let source = match self.source {
            ToolCallArgumentSource::RepairedString => "repaired-string",
            ToolCallArgumentSource::EmptyString => "empty-string",
            ToolCallArgumentSource::NullOrMissing => "null-or-missing",
            ToolCallArgumentSource::MalformedString => "malformed-string",
        };
        match (self.raw_len, self.parse_error.as_deref()) {
            (Some(raw_len), Some(parse_error)) => {
                format!("arg_decode={source} raw_len={raw_len} parse_error={parse_error}")
            },
            (Some(raw_len), None) => format!("arg_decode={source} raw_len={raw_len}"),
            (None, Some(parse_error)) => format!("arg_decode={source} parse_error={parse_error}"),
            (None, None) => format!("arg_decode={source}"),
        }
    }

    #[must_use]
    pub fn llm_detail(&self) -> String {
        let mut detail = format!("Argument decode status: {}.", self.short_summary());
        if let Some(preview) = &self.raw_preview
            && !preview.is_empty()
        {
            detail.push_str(&format!(" Raw argument preview: {preview}"));
        }
        detail
    }
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
