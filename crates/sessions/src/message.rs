//! Typed message structures for session storage.
//!
//! These types represent the JSON format stored in session JSONL files.
//! They include both LLM-relevant fields (role, content) and metadata
//! fields (created_at, model, provider, tokens, channel).

use serde::{Deserialize, Serialize};

/// A message stored in a session JSONL file.
///
/// Includes both the LLM-relevant content and metadata for UI display
/// and analytics. The `role` field determines which variant this is.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum PersistedMessage {
    System {
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        created_at: Option<u64>,
    },
    User {
        /// Content can be a string (plain text) or array (multimodal).
        content: MessageContent,
        #[serde(skip_serializing_if = "Option::is_none")]
        created_at: Option<u64>,
        /// Channel metadata for UI display (e.g., Telegram sender info).
        #[serde(skip_serializing_if = "Option::is_none")]
        channel: Option<serde_json::Value>,
    },
    Assistant {
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        created_at: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
        #[serde(rename = "inputTokens", skip_serializing_if = "Option::is_none")]
        input_tokens: Option<u32>,
        #[serde(rename = "outputTokens", skip_serializing_if = "Option::is_none")]
        output_tokens: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_calls: Option<Vec<PersistedToolCall>>,
    },
    Tool {
        tool_call_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        created_at: Option<u64>,
    },
}

/// User message content: plain text or multimodal array.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Multimodal(Vec<ContentBlock>),
}

/// A single block in multimodal content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

/// Image URL data (for multimodal content).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrl {
    pub url: String,
}

/// A tool call stored in an assistant message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: PersistedFunction,
}

/// Function details in a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedFunction {
    pub name: String,
    pub arguments: String,
}

impl PersistedMessage {
    /// Create a user message with plain text content.
    pub fn user(text: impl Into<String>) -> Self {
        Self::User {
            content: MessageContent::Text(text.into()),
            created_at: Some(now_ms()),
            channel: None,
        }
    }

    /// Create a user message with plain text and channel metadata.
    pub fn user_with_channel(text: impl Into<String>, channel: serde_json::Value) -> Self {
        Self::User {
            content: MessageContent::Text(text.into()),
            created_at: Some(now_ms()),
            channel: Some(channel),
        }
    }

    /// Create a user message with multimodal content.
    pub fn user_multimodal(blocks: Vec<ContentBlock>) -> Self {
        Self::User {
            content: MessageContent::Multimodal(blocks),
            created_at: Some(now_ms()),
            channel: None,
        }
    }

    /// Create a user message with multimodal content and channel metadata.
    pub fn user_multimodal_with_channel(
        blocks: Vec<ContentBlock>,
        channel: serde_json::Value,
    ) -> Self {
        Self::User {
            content: MessageContent::Multimodal(blocks),
            created_at: Some(now_ms()),
            channel: Some(channel),
        }
    }

    /// Create an assistant message with token usage and model info.
    pub fn assistant(
        text: impl Into<String>,
        model: impl Into<String>,
        provider: impl Into<String>,
        input_tokens: u32,
        output_tokens: u32,
    ) -> Self {
        Self::Assistant {
            content: text.into(),
            created_at: Some(now_ms()),
            model: Some(model.into()),
            provider: Some(provider.into()),
            input_tokens: Some(input_tokens),
            output_tokens: Some(output_tokens),
            tool_calls: None,
        }
    }

    /// Create a system message (e.g., for error display).
    pub fn system(text: impl Into<String>) -> Self {
        Self::System {
            content: text.into(),
            created_at: Some(now_ms()),
        }
    }

    /// Create a tool result message.
    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self::Tool {
            tool_call_id: tool_call_id.into(),
            content: content.into(),
            created_at: Some(now_ms()),
        }
    }

    /// Convert to JSON value for storage.
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("PersistedMessage serialization cannot fail")
    }
}

impl ContentBlock {
    /// Create a text content block.
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    /// Create an image URL content block from base64 data.
    pub fn image_base64(media_type: &str, data: &str) -> Self {
        Self::ImageUrl {
            image_url: ImageUrl {
                url: format!("data:{media_type};base64,{data}"),
            },
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_text_serializes_correctly() {
        let msg = PersistedMessage::User {
            content: MessageContent::Text("hello".to_string()),
            created_at: Some(12345),
            channel: None,
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["role"], "user");
        assert_eq!(json["content"], "hello");
        assert_eq!(json["created_at"], 12345);
        assert!(json.get("channel").is_none());
    }

    #[test]
    fn user_multimodal_serializes_correctly() {
        let msg = PersistedMessage::User {
            content: MessageContent::Multimodal(vec![
                ContentBlock::text("describe this"),
                ContentBlock::image_base64("image/jpeg", "abc123"),
            ]),
            created_at: Some(12345),
            channel: None,
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["role"], "user");
        let content = json["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "describe this");
        assert_eq!(content[1]["type"], "image_url");
        assert!(
            content[1]["image_url"]["url"]
                .as_str()
                .unwrap()
                .starts_with("data:image/jpeg;base64,")
        );
    }

    #[test]
    fn assistant_serializes_correctly() {
        let msg = PersistedMessage::Assistant {
            content: "response".to_string(),
            created_at: Some(12345),
            model: Some("gpt-4o".to_string()),
            provider: Some("openai".to_string()),
            input_tokens: Some(100),
            output_tokens: Some(50),
            tool_calls: None,
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["role"], "assistant");
        assert_eq!(json["content"], "response");
        assert_eq!(json["model"], "gpt-4o");
        assert_eq!(json["provider"], "openai");
        assert_eq!(json["inputTokens"], 100);
        assert_eq!(json["outputTokens"], 50);
    }

    #[test]
    fn user_text_deserializes_correctly() {
        let json = serde_json::json!({
            "role": "user",
            "content": "hello",
            "created_at": 12345
        });
        let msg: PersistedMessage = serde_json::from_value(json).unwrap();
        match msg {
            PersistedMessage::User { content, .. } => {
                assert!(matches!(content, MessageContent::Text(t) if t == "hello"));
            },
            _ => panic!("expected User message"),
        }
    }

    #[test]
    fn user_multimodal_deserializes_correctly() {
        let json = serde_json::json!({
            "role": "user",
            "content": [
                { "type": "text", "text": "describe" },
                { "type": "image_url", "image_url": { "url": "data:image/png;base64,xyz" } }
            ]
        });
        let msg: PersistedMessage = serde_json::from_value(json).unwrap();
        match msg {
            PersistedMessage::User { content, .. } => match content {
                MessageContent::Multimodal(blocks) => {
                    assert_eq!(blocks.len(), 2);
                },
                _ => panic!("expected multimodal content"),
            },
            _ => panic!("expected User message"),
        }
    }

    #[test]
    fn roundtrip_user_text() {
        let original = PersistedMessage::user("test message");
        let json = original.to_value();
        let parsed: PersistedMessage = serde_json::from_value(json).unwrap();
        match parsed {
            PersistedMessage::User { content, .. } => {
                assert!(matches!(content, MessageContent::Text(t) if t == "test message"));
            },
            _ => panic!("expected User message"),
        }
    }

    #[test]
    fn roundtrip_assistant() {
        let original = PersistedMessage::assistant("response", "gpt-4o", "openai", 100, 50);
        let json = original.to_value();
        let parsed: PersistedMessage = serde_json::from_value(json).unwrap();
        match parsed {
            PersistedMessage::Assistant {
                content,
                model,
                provider,
                input_tokens,
                output_tokens,
                ..
            } => {
                assert_eq!(content, "response");
                assert_eq!(model.as_deref(), Some("gpt-4o"));
                assert_eq!(provider.as_deref(), Some("openai"));
                assert_eq!(input_tokens, Some(100));
                assert_eq!(output_tokens, Some(50));
            },
            _ => panic!("expected Assistant message"),
        }
    }
}
