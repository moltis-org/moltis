use super::types::ToolCall;

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
        /// Optional sender name for channel messages (Telegram, Discord, etc.).
        name: Option<String>,
    },
    Assistant {
        content: Option<String>,
        tool_calls: Vec<ToolCall>,
        reasoning: Option<String>,
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

impl UserContent {
    /// Create a text-only user content.
    pub fn text(s: impl Into<String>) -> Self {
        Self::Text(s.into())
    }
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
            name: None,
        }
    }

    /// Create a user message with plain text and a sender name.
    pub fn user_named(content: impl Into<String>, name: impl Into<String>) -> Self {
        Self::User {
            content: UserContent::Text(content.into()),
            name: Some(name.into()),
        }
    }

    /// Create a user message with multimodal content.
    pub fn user_multimodal(parts: Vec<ContentPart>) -> Self {
        Self::User {
            content: UserContent::Multimodal(parts),
            name: None,
        }
    }

    /// Create a user message with multimodal content and a sender name.
    pub fn user_multimodal_named(parts: Vec<ContentPart>, name: impl Into<String>) -> Self {
        Self::User {
            content: UserContent::Multimodal(parts),
            name: Some(name.into()),
        }
    }

    /// Create an assistant message with text only (no tool calls).
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::Assistant {
            content: Some(content.into()),
            tool_calls: vec![],
            reasoning: None,
        }
    }

    /// Create an assistant message with tool calls (and optional text).
    pub fn assistant_with_tools(content: Option<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self::Assistant {
            content,
            tool_calls,
            reasoning: None,
        }
    }

    /// Create a tool result message.
    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self::Tool {
            tool_call_id: tool_call_id.into(),
            content: content.into(),
        }
    }

    /// Sanitize a user name for the OpenAI `name` field.
    ///
    /// OpenAI requires `name` to match `^[a-zA-Z0-9_-]+$` with a max length of
    /// 64 characters. Spaces are replaced with `_`, non-matching characters are
    /// stripped, and the result is truncated to 64 chars.  Returns `None` if the
    /// sanitized result is empty.
    #[must_use]
    pub fn sanitize_message_name(name: &str) -> Option<String> {
        let sanitized: String = name
            .chars()
            .map(|c| {
                if c == ' ' {
                    '_'
                } else {
                    c
                }
            })
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .take(64)
            .collect();
        if sanitized.is_empty() {
            None
        } else {
            Some(sanitized)
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
            ChatMessage::User { content, name } => {
                let mut msg = match content {
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
                };
                if let Some(sanitized) = name.as_ref().and_then(|n| Self::sanitize_message_name(n))
                {
                    msg["name"] = serde_json::Value::String(sanitized);
                }
                msg
            },
            ChatMessage::Assistant {
                content,
                tool_calls,
                ..
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
                            let mut tc_val = serde_json::json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": tc.arguments.to_string(),
                                }
                            });
                            if let Some(ref meta) = tc.metadata
                                && let Some(obj) = tc_val.as_object_mut()
                            {
                                for (k, v) in meta {
                                    obj.insert(k.clone(), v.clone());
                                }
                            }
                            tc_val
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
