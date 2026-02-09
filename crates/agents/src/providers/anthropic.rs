use std::pin::Pin;

use {async_trait::async_trait, futures::StreamExt, secrecy::ExposeSecret, tokio_stream::Stream};

use tracing::{debug, trace, warn};

use crate::model::{
    ChatMessage, CompletionResponse, ContentPart, LlmProvider, StreamEvent, ToolCall, Usage,
    UserContent,
};

pub struct AnthropicProvider {
    api_key: secrecy::Secret<String>,
    model: String,
    base_url: String,
    client: reqwest::Client,
    /// Optional alias for metrics differentiation (e.g., "anthropic-work", "anthropic-2").
    alias: Option<String>,
}

impl AnthropicProvider {
    pub fn new(api_key: secrecy::Secret<String>, model: String, base_url: String) -> Self {
        Self {
            api_key,
            model,
            base_url,
            client: reqwest::Client::new(),
            alias: None,
        }
    }

    /// Create a new provider with a custom alias for metrics.
    pub fn with_alias(
        api_key: secrecy::Secret<String>,
        model: String,
        base_url: String,
        alias: Option<String>,
    ) -> Self {
        Self {
            api_key,
            model,
            base_url,
            client: reqwest::Client::new(),
            alias,
        }
    }
}

/// Convert tool schemas from the generic format to Anthropic's tool format.
fn to_anthropic_tools(tools: &[serde_json::Value]) -> Vec<serde_json::Value> {
    tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t["name"],
                "description": t["description"],
                "input_schema": t["parameters"],
            })
        })
        .collect()
}

/// Parse tool_use blocks from an Anthropic response.
fn parse_tool_calls(content: &[serde_json::Value]) -> Vec<ToolCall> {
    content
        .iter()
        .filter_map(|block| {
            if block["type"].as_str() == Some("tool_use") {
                Some(ToolCall {
                    id: block["id"].as_str().unwrap_or("").to_string(),
                    name: block["name"].as_str().unwrap_or("").to_string(),
                    arguments: block["input"].clone(),
                })
            } else {
                None
            }
        })
        .collect()
}

/// Convert `ChatMessage` list to Anthropic format.
///
/// Returns `(system_text, anthropic_messages)`. System messages are extracted
/// (Anthropic takes them as a top-level `system` field). Tool messages become
/// user messages with `tool_result` content blocks. Assistant messages with
/// tool calls become `content` arrays with `tool_use` blocks.
fn to_anthropic_messages(messages: &[ChatMessage]) -> (Option<String>, Vec<serde_json::Value>) {
    let mut system_text: Option<String> = None;
    let mut out = Vec::new();

    for msg in messages {
        match msg {
            ChatMessage::System { content } => {
                system_text = Some(match system_text {
                    Some(existing) => format!("{existing}\n\n{content}"),
                    None => content.clone(),
                });
            },
            ChatMessage::User { content } => match content {
                UserContent::Text(text) => {
                    out.push(serde_json::json!({"role": "user", "content": text}));
                },
                UserContent::Multimodal(parts) => {
                    let blocks: Vec<serde_json::Value> = parts
                        .iter()
                        .map(|part| match part {
                            ContentPart::Text(text) => {
                                serde_json::json!({"type": "text", "text": text})
                            },
                            ContentPart::Image { media_type, data } => {
                                serde_json::json!({
                                    "type": "image",
                                    "source": {
                                        "type": "base64",
                                        "media_type": media_type,
                                        "data": data,
                                    }
                                })
                            },
                        })
                        .collect();
                    out.push(serde_json::json!({"role": "user", "content": blocks}));
                },
            },
            ChatMessage::Assistant {
                content,
                tool_calls,
            } => {
                if tool_calls.is_empty() {
                    out.push(serde_json::json!({
                        "role": "assistant",
                        "content": content.as_deref().unwrap_or(""),
                    }));
                } else {
                    let mut blocks = Vec::new();
                    if let Some(text) = content
                        && !text.is_empty()
                    {
                        blocks.push(serde_json::json!({"type": "text", "text": text}));
                    }
                    for tc in tool_calls {
                        blocks.push(serde_json::json!({
                            "type": "tool_use",
                            "id": tc.id,
                            "name": tc.name,
                            "input": tc.arguments,
                        }));
                    }
                    out.push(serde_json::json!({"role": "assistant", "content": blocks}));
                }
            },
            ChatMessage::Tool {
                tool_call_id,
                content,
            } => {
                out.push(serde_json::json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": tool_call_id,
                        "content": content,
                    }]
                }));
            },
        }
    }

    (system_text, out)
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        self.alias.as_deref().unwrap_or("anthropic")
    }

    fn id(&self) -> &str {
        &self.model
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn context_window(&self) -> u32 {
        super::context_window_for_model(&self.model)
    }

    fn supports_vision(&self) -> bool {
        super::supports_vision_for_model(&self.model)
    }

    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> anyhow::Result<CompletionResponse> {
        let (system_text, anthropic_messages) = to_anthropic_messages(messages);

        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": 4096,
            "messages": anthropic_messages,
        });

        if let Some(ref sys) = system_text {
            body["system"] = serde_json::Value::String(sys.clone());
        }

        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(to_anthropic_tools(tools));
        }

        debug!(
            model = %self.model,
            messages_count = anthropic_messages.len(),
            tools_count = tools.len(),
            has_system = system_text.is_some(),
            "anthropic complete request"
        );
        trace!(body = %serde_json::to_string(&body).unwrap_or_default(), "anthropic request body");

        let http_resp = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", self.api_key.expose_secret())
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = http_resp.status();
        if !status.is_success() {
            let body_text = http_resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %body_text, "anthropic API error");
            anyhow::bail!("Anthropic API error HTTP {status}: {body_text}");
        }

        let resp = http_resp.json::<serde_json::Value>().await?;
        trace!(response = %resp, "anthropic raw response");

        let content = resp["content"].as_array().cloned().unwrap_or_default();

        let text = content
            .iter()
            .filter_map(|b| {
                if b["type"].as_str() == Some("text") {
                    b["text"].as_str().map(|s| s.to_string())
                } else {
                    None
                }
            })
            .reduce(|a, b| a + &b);

        let tool_calls = parse_tool_calls(&content);

        let usage = Usage {
            input_tokens: resp["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32,
            output_tokens: resp["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32,
            cache_read_tokens: resp["usage"]["cache_read_input_tokens"]
                .as_u64()
                .unwrap_or(0) as u32,
            cache_write_tokens: resp["usage"]["cache_creation_input_tokens"]
                .as_u64()
                .unwrap_or(0) as u32,
        };

        Ok(CompletionResponse {
            text,
            tool_calls,
            usage,
        })
    }

    #[allow(clippy::collapsible_if)]
    fn stream(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        self.stream_with_tools(messages, vec![])
    }

    #[allow(clippy::collapsible_if)]
    fn stream_with_tools(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<serde_json::Value>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        Box::pin(async_stream::stream! {
            let (system_text, anthropic_messages) = to_anthropic_messages(&messages);

            let mut body = serde_json::json!({
                "model": self.model,
                "max_tokens": 4096,
                "messages": anthropic_messages,
                "stream": true,
            });

            if let Some(ref sys) = system_text {
                body["system"] = serde_json::Value::String(sys.clone());
            }

            if !tools.is_empty() {
                body["tools"] = serde_json::Value::Array(to_anthropic_tools(&tools));
            }

            debug!(
                model = %self.model,
                messages_count = anthropic_messages.len(),
                tools_count = tools.len(),
                has_system = system_text.is_some(),
                "anthropic stream_with_tools request"
            );
            trace!(body = %serde_json::to_string(&body).unwrap_or_default(), "anthropic stream request body");

            let resp = match self
                .client
                .post(format!("{}/v1/messages", self.base_url))
                .header("x-api-key", self.api_key.expose_secret())
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => {
                    if let Err(e) = r.error_for_status_ref() {
                        let status = e.status().map(|s| s.as_u16()).unwrap_or(0);
                        let body_text = r.text().await.unwrap_or_default();
                        yield StreamEvent::Error(format!("HTTP {status}: {body_text}"));
                        return;
                    }
                    r
                }
                Err(e) => {
                    yield StreamEvent::Error(e.to_string());
                    return;
                }
            };

            let mut byte_stream = resp.bytes_stream();
            let mut buf = String::new();
            let mut input_tokens: u32 = 0;
            let mut output_tokens: u32 = 0;
            let mut cache_read_tokens: u32 = 0;
            let mut cache_write_tokens: u32 = 0;

            // Track current content block index for tool calls.
            let mut current_block_index: Option<usize> = None;

            while let Some(chunk) = byte_stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        yield StreamEvent::Error(e.to_string());
                        return;
                    }
                };
                buf.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(pos) = buf.find("\n\n") {
                    let block = buf[..pos].to_string();
                    buf = buf[pos + 2..].to_string();

                    for line in block.lines() {
                        if let Some(data) = line.strip_prefix("data: ") {
                            if let Ok(evt) = serde_json::from_str::<serde_json::Value>(data) {
                                let evt_type = evt["type"].as_str().unwrap_or("");
                                match evt_type {
                                    "content_block_start" => {
                                        let index = evt["index"].as_u64().unwrap_or(0) as usize;
                                        let content_block = &evt["content_block"];
                                        let block_type = content_block["type"].as_str().unwrap_or("");

                                        if block_type == "tool_use" {
                                            let id = content_block["id"].as_str().unwrap_or("").to_string();
                                            let name = content_block["name"].as_str().unwrap_or("").to_string();
                                            current_block_index = Some(index);
                                            yield StreamEvent::ToolCallStart { id, name, index };
                                        }
                                    }
                                    "content_block_delta" => {
                                        let delta = &evt["delta"];
                                        let delta_type = delta["type"].as_str().unwrap_or("");

                                        if delta_type == "text_delta" {
                                            if let Some(text) = delta["text"].as_str() {
                                                if !text.is_empty() {
                                                    yield StreamEvent::Delta(text.to_string());
                                                }
                                            }
                                        } else if delta_type == "input_json_delta" {
                                            if let Some(partial_json) = delta["partial_json"].as_str() {
                                                let index = evt["index"].as_u64().unwrap_or(0) as usize;
                                                yield StreamEvent::ToolCallArgumentsDelta {
                                                    index,
                                                    delta: partial_json.to_string(),
                                                };
                                            }
                                        }
                                    }
                                    "content_block_stop" => {
                                        let index = evt["index"].as_u64().unwrap_or(0) as usize;
                                        // Only emit ToolCallComplete if this was a tool_use block.
                                        if current_block_index == Some(index) {
                                            yield StreamEvent::ToolCallComplete { index };
                                            current_block_index = None;
                                        }
                                    }
                                    "message_delta" => {
                                        let u = &evt["usage"];
                                        if let Some(v) = u["output_tokens"].as_u64() {
                                            output_tokens = v as u32;
                                        }
                                        // Anthropic may report cache tokens in delta
                                        if let Some(v) = u["cache_read_input_tokens"].as_u64() {
                                            cache_read_tokens = v as u32;
                                        }
                                        if let Some(v) = u["cache_creation_input_tokens"].as_u64() {
                                            cache_write_tokens = v as u32;
                                        }
                                    }
                                    "message_start" => {
                                        let u = &evt["message"]["usage"];
                                        if let Some(v) = u["input_tokens"].as_u64() {
                                            input_tokens = v as u32;
                                        }
                                        if let Some(v) = u["cache_read_input_tokens"].as_u64() {
                                            cache_read_tokens = v as u32;
                                        }
                                        if let Some(v) = u["cache_creation_input_tokens"].as_u64() {
                                            cache_write_tokens = v as u32;
                                        }
                                    }
                                    "message_stop" => {
                                        yield StreamEvent::Done(Usage {
                                            input_tokens,
                                            output_tokens,
                                            cache_read_tokens,
                                            cache_write_tokens,
                                        });
                                        return;
                                    }
                                    "error" => {
                                        let msg = evt["error"]["message"]
                                            .as_str()
                                            .unwrap_or("unknown error");
                                        yield StreamEvent::Error(msg.to_string());
                                        return;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        })
    }
}
