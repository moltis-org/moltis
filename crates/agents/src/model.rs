use std::pin::Pin;

use {async_trait::async_trait, tokio_stream::Stream};

use crate::multimodal::parse_data_uri;

// ── Typed chat messages ─────────────────────────────────────────────────────

/// Typed chat message for the LLM provider interface.
///
/// Only contains LLM-relevant fields — metadata like `created_at`, `model`,
/// `provider`, `inputTokens`, `outputTokens` cannot exist here, so they
/// can never leak into provider API requests.
#[derive(Debug, Clone)]
pub enum ChatMessage {
    System {
        content: String,
    },
    User {
        content: UserContent,
    },
    Assistant {
        content: Option<String>,
        tool_calls: Vec<ToolCall>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
}

/// User message content: plain text or multimodal (text + images).
#[derive(Debug, Clone)]
pub enum UserContent {
    Text(String),
    Multimodal(Vec<ContentPart>),
}

/// A single part of a multimodal content array.
#[derive(Debug, Clone)]
pub enum ContentPart {
    Text(String),
    Image { media_type: String, data: String },
}

impl ChatMessage {
    /// Create a system message.
    pub fn system(content: impl Into<String>) -> Self {
        Self::System {
            content: content.into(),
        }
    }

    /// Create a user message with plain text.
    pub fn user(content: impl Into<String>) -> Self {
        Self::User {
            content: UserContent::Text(content.into()),
        }
    }

    /// Create a user message with multimodal content.
    pub fn user_multimodal(parts: Vec<ContentPart>) -> Self {
        Self::User {
            content: UserContent::Multimodal(parts),
        }
    }

    /// Create an assistant message with text only (no tool calls).
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::Assistant {
            content: Some(content.into()),
            tool_calls: vec![],
        }
    }

    /// Create an assistant message with tool calls (and optional text).
    pub fn assistant_with_tools(content: Option<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self::Assistant {
            content,
            tool_calls,
        }
    }

    /// Create a tool result message.
    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self::Tool {
            tool_call_id: tool_call_id.into(),
            content: content.into(),
        }
    }

    /// Convert to OpenAI-compatible JSON format.
    ///
    /// Used by providers that speak the OpenAI Chat Completions API:
    /// OpenAI, Mistral, Copilot, Kimi, Cerebras, etc.
    #[must_use]
    pub fn to_openai_value(&self) -> serde_json::Value {
        match self {
            ChatMessage::System { content } => {
                serde_json::json!({ "role": "system", "content": content })
            },
            ChatMessage::User { content } => match content {
                UserContent::Text(text) => {
                    serde_json::json!({ "role": "user", "content": text })
                },
                UserContent::Multimodal(parts) => {
                    let blocks: Vec<serde_json::Value> = parts
                        .iter()
                        .map(|part| match part {
                            ContentPart::Text(text) => {
                                serde_json::json!({ "type": "text", "text": text })
                            },
                            ContentPart::Image { media_type, data } => {
                                let data_uri = format!("data:{media_type};base64,{data}");
                                serde_json::json!({
                                    "type": "image_url",
                                    "image_url": { "url": data_uri }
                                })
                            },
                        })
                        .collect();
                    serde_json::json!({ "role": "user", "content": blocks })
                },
            },
            ChatMessage::Assistant {
                content,
                tool_calls,
            } => {
                if tool_calls.is_empty() {
                    serde_json::json!({
                        "role": "assistant",
                        "content": content.as_deref().unwrap_or(""),
                    })
                } else {
                    let tc_json: Vec<serde_json::Value> = tool_calls
                        .iter()
                        .map(|tc| {
                            serde_json::json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": tc.arguments.to_string(),
                                }
                            })
                        })
                        .collect();
                    let mut msg = serde_json::json!({
                        "role": "assistant",
                        "tool_calls": tc_json,
                    });
                    if let Some(text) = content {
                        msg["content"] = serde_json::Value::String(text.clone());
                    }
                    msg
                }
            },
            ChatMessage::Tool {
                tool_call_id,
                content,
            } => {
                serde_json::json!({
                    "role": "tool",
                    "tool_call_id": tool_call_id,
                    "content": content,
                })
            },
        }
    }
}

/// Convert persisted JSON messages (from session store) to typed `ChatMessage`s.
///
/// Skips messages that don't have a valid `role` field, logging a warning.
/// Metadata fields (`created_at`, `model`, `provider`, `inputTokens`,
/// `outputTokens`, `channel`) are silently dropped — they only exist in
/// the persisted JSON, not in `ChatMessage`.
pub fn values_to_chat_messages(values: &[serde_json::Value]) -> Vec<ChatMessage> {
    let mut messages = Vec::with_capacity(values.len());
    for (i, val) in values.iter().enumerate() {
        let Some(role) = val["role"].as_str() else {
            tracing::warn!(index = i, "skipping message with missing/invalid role");
            continue;
        };
        match role {
            "system" => {
                let content = val["content"].as_str().unwrap_or("").to_string();
                messages.push(ChatMessage::system(content));
            },
            "user" => {
                // Content can be a string or an array (multimodal).
                if let Some(text) = val["content"].as_str() {
                    messages.push(ChatMessage::user(text));
                } else if let Some(arr) = val["content"].as_array() {
                    let parts: Vec<ContentPart> = arr
                        .iter()
                        .filter_map(|block| {
                            let block_type = block["type"].as_str()?;
                            match block_type {
                                "text" => {
                                    let text = block["text"].as_str()?.to_string();
                                    Some(ContentPart::Text(text))
                                },
                                "image_url" => {
                                    let url = block["image_url"]["url"].as_str()?;
                                    let (media_type, data) = parse_data_uri(url)?;
                                    Some(ContentPart::Image {
                                        media_type: media_type.to_string(),
                                        data: data.to_string(),
                                    })
                                },
                                _ => None,
                            }
                        })
                        .collect();
                    messages.push(ChatMessage::user_multimodal(parts));
                } else {
                    messages.push(ChatMessage::user(""));
                }
            },
            "assistant" => {
                let content = val["content"].as_str().map(|s| s.to_string());
                let tool_calls = val["tool_calls"]
                    .as_array()
                    .map(|tcs| {
                        tcs.iter()
                            .filter_map(|tc| {
                                let id = tc["id"].as_str()?.to_string();
                                let name = tc["function"]["name"].as_str()?.to_string();
                                let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
                                let arguments =
                                    serde_json::from_str(args_str).unwrap_or(serde_json::json!({}));
                                Some(ToolCall {
                                    id,
                                    name,
                                    arguments,
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                messages.push(ChatMessage::Assistant {
                    content,
                    tool_calls,
                });
            },
            "tool" => {
                let tool_call_id = val["tool_call_id"].as_str().unwrap_or("").to_string();
                let content = if let Some(s) = val["content"].as_str() {
                    s.to_string()
                } else {
                    val["content"].to_string()
                };
                messages.push(ChatMessage::tool(tool_call_id, content));
            },
            // tool_result entries are UI-only metadata (persisted tool execution
            // output); they are not part of the LLM conversation context.
            "tool_result" => continue,
            other => {
                tracing::warn!(
                    index = i,
                    role = other,
                    "skipping message with unknown role"
                );
            },
        }
    }
    messages
}

// ── Stream events ───────────────────────────────────────────────────────────

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

#[derive(Debug, Clone, Default)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_write_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ChatMessage constructors ─────────────────────────────────────

    #[test]
    fn system_message() {
        let msg = ChatMessage::system("You are helpful.");
        assert!(matches!(msg, ChatMessage::System { content } if content == "You are helpful."));
    }

    #[test]
    fn user_message_text() {
        let msg = ChatMessage::user("Hello");
        assert!(matches!(msg, ChatMessage::User { content: UserContent::Text(t) } if t == "Hello"));
    }

    #[test]
    fn assistant_message_text() {
        let msg = ChatMessage::assistant("Hi there");
        assert!(
            matches!(msg, ChatMessage::Assistant { content: Some(t), tool_calls } if t == "Hi there" && tool_calls.is_empty())
        );
    }

    #[test]
    fn tool_message() {
        let msg = ChatMessage::tool("call_1", "result");
        assert!(
            matches!(msg, ChatMessage::Tool { tool_call_id, content } if tool_call_id == "call_1" && content == "result")
        );
    }

    // ── to_openai_value ──────────────────────────────────────────────

    #[test]
    fn to_openai_system() {
        let val = ChatMessage::system("sys").to_openai_value();
        assert_eq!(val["role"], "system");
        assert_eq!(val["content"], "sys");
    }

    #[test]
    fn to_openai_user_text() {
        let val = ChatMessage::user("hi").to_openai_value();
        assert_eq!(val["role"], "user");
        assert_eq!(val["content"], "hi");
    }

    #[test]
    fn to_openai_user_multimodal() {
        let msg = ChatMessage::user_multimodal(vec![
            ContentPart::Text("describe".into()),
            ContentPart::Image {
                media_type: "image/png".into(),
                data: "abc123".into(),
            },
        ]);
        let val = msg.to_openai_value();
        assert_eq!(val["role"], "user");
        let content = val["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image_url");
        assert!(
            content[1]["image_url"]["url"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
    }

    #[test]
    fn to_openai_assistant_text() {
        let val = ChatMessage::assistant("hello").to_openai_value();
        assert_eq!(val["role"], "assistant");
        assert_eq!(val["content"], "hello");
        assert!(val.get("tool_calls").is_none());
    }

    #[test]
    fn to_openai_assistant_with_tools() {
        let msg = ChatMessage::assistant_with_tools(Some("thinking".into()), vec![ToolCall {
            id: "call_1".into(),
            name: "exec".into(),
            arguments: serde_json::json!({"cmd": "ls"}),
        }]);
        let val = msg.to_openai_value();
        assert_eq!(val["role"], "assistant");
        assert_eq!(val["content"], "thinking");
        let tcs = val["tool_calls"].as_array().unwrap();
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0]["id"], "call_1");
        assert_eq!(tcs[0]["function"]["name"], "exec");
    }

    #[test]
    fn to_openai_tool() {
        let val = ChatMessage::tool("call_1", "output").to_openai_value();
        assert_eq!(val["role"], "tool");
        assert_eq!(val["tool_call_id"], "call_1");
        assert_eq!(val["content"], "output");
    }

    // ── values_to_chat_messages ──────────────────────────────────────

    #[test]
    fn convert_basic_messages() {
        let values = vec![
            serde_json::json!({"role": "system", "content": "sys"}),
            serde_json::json!({"role": "user", "content": "hi"}),
            serde_json::json!({"role": "assistant", "content": "hello"}),
        ];
        let msgs = values_to_chat_messages(&values);
        assert_eq!(msgs.len(), 3);
        assert!(matches!(&msgs[0], ChatMessage::System { content } if content == "sys"));
        assert!(
            matches!(&msgs[1], ChatMessage::User { content: UserContent::Text(t) } if t == "hi")
        );
        assert!(
            matches!(&msgs[2], ChatMessage::Assistant { content: Some(t), .. } if t == "hello")
        );
    }

    #[test]
    fn convert_skips_metadata_fields() {
        let values = vec![serde_json::json!({
            "role": "user",
            "content": "hi",
            "created_at": 12345,
            "model": "gpt-4o",
            "provider": "openai",
            "inputTokens": 10,
            "outputTokens": 5,
            "channel": "web"
        })];
        let msgs = values_to_chat_messages(&values);
        assert_eq!(msgs.len(), 1);
        // The ChatMessage has no metadata fields — they're dropped.
        let val = msgs[0].to_openai_value();
        assert!(val.get("created_at").is_none());
        assert!(val.get("model").is_none());
        assert!(val.get("provider").is_none());
        assert!(val.get("inputTokens").is_none());
        assert!(val.get("outputTokens").is_none());
        assert!(val.get("channel").is_none());
    }

    #[test]
    fn convert_assistant_with_tool_calls() {
        let values = vec![serde_json::json!({
            "role": "assistant",
            "content": "thinking",
            "tool_calls": [{
                "id": "call_1",
                "type": "function",
                "function": {
                    "name": "exec",
                    "arguments": "{\"cmd\":\"ls\"}"
                }
            }]
        })];
        let msgs = values_to_chat_messages(&values);
        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            ChatMessage::Assistant {
                content,
                tool_calls,
            } => {
                assert_eq!(content.as_deref(), Some("thinking"));
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].name, "exec");
                assert_eq!(tool_calls[0].arguments["cmd"], "ls");
            },
            _ => panic!("expected assistant message"),
        }
    }

    #[test]
    fn convert_tool_message() {
        let values = vec![serde_json::json!({
            "role": "tool",
            "tool_call_id": "call_1",
            "content": "result data"
        })];
        let msgs = values_to_chat_messages(&values);
        assert_eq!(msgs.len(), 1);
        assert!(
            matches!(&msgs[0], ChatMessage::Tool { tool_call_id, content } if tool_call_id == "call_1" && content == "result data")
        );
    }

    #[test]
    fn convert_skips_invalid_messages() {
        let values = vec![
            serde_json::json!({"content": "no role"}),
            serde_json::json!({"role": "user", "content": "valid"}),
            serde_json::json!({"role": 42}),
        ];
        let msgs = values_to_chat_messages(&values);
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn roundtrip_to_openai_and_back() {
        let original = [
            ChatMessage::system("sys"),
            ChatMessage::user("hi"),
            ChatMessage::assistant("hello"),
            ChatMessage::tool("call_1", "result"),
        ];
        let values: Vec<serde_json::Value> = original.iter().map(|m| m.to_openai_value()).collect();
        let roundtripped = values_to_chat_messages(&values);
        assert_eq!(roundtripped.len(), 4);
    }

    #[test]
    fn convert_skips_tool_result_entries() {
        let values = vec![
            serde_json::json!({"role": "user", "content": "run ls"}),
            serde_json::json!({
                "role": "tool_result",
                "tool_call_id": "call_1",
                "tool_name": "exec",
                "success": true,
                "result": {"stdout": "file.txt", "exit_code": 0}
            }),
            serde_json::json!({"role": "assistant", "content": "done"}),
        ];
        let msgs = values_to_chat_messages(&values);
        assert_eq!(msgs.len(), 2);
        assert!(matches!(&msgs[0], ChatMessage::User { .. }));
        assert!(matches!(&msgs[1], ChatMessage::Assistant { .. }));
    }
}
