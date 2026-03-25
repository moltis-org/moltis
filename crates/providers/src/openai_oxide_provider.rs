use std::collections::HashMap;
use std::pin::Pin;

use {
    async_trait::async_trait,
    futures::StreamExt,
    openai_oxide::{
        config::ClientConfig,
        types::chat::{
            ChatCompletionMessageParam, ChatCompletionRequest, ContentPart, ImageUrl,
            StreamOptions,
        },
        OpenAI,
    },
    tokio_stream::Stream,
};

use moltis_agents::model::{
    ChatMessage, CompletionResponse, LlmProvider, StreamEvent, ToolCall, Usage, UserContent,
};

/// Provider backed by the `openai-oxide` crate.
/// Works with OpenAI and any OpenAI-compatible API (Ollama, vLLM, etc.)
/// via custom base URL.
///
/// Note: `provider-openai-oxide` and `provider-async-openai` both register
/// against the `"openai"` config key. If both features are enabled,
/// whichever registers first wins. Disable one to use the other.
pub struct OpenAiOxideProvider {
    model: String,
    client: OpenAI,
    /// Optional alias for metrics differentiation.
    alias: Option<String>,
}

impl OpenAiOxideProvider {
    pub fn new(api_key: secrecy::Secret<String>, model: String, base_url: String) -> Self {
        Self::with_alias(api_key, model, base_url, None)
    }

    pub fn with_alias(
        api_key: secrecy::Secret<String>,
        model: String,
        base_url: String,
        alias: Option<String>,
    ) -> Self {
        use secrecy::ExposeSecret;
        let config = ClientConfig::new(api_key.expose_secret()).base_url(&base_url);
        let client = OpenAI::with_config(config);
        Self {
            model,
            client,
            alias,
        }
    }
}

fn build_messages(messages: &[ChatMessage]) -> Vec<ChatCompletionMessageParam> {
    let mut out = Vec::new();
    for msg in messages {
        match msg {
            ChatMessage::System { content } => {
                out.push(ChatCompletionMessageParam::System {
                    content: content.clone(),
                    name: None,
                });
            }
            ChatMessage::Assistant {
                content,
                tool_calls,
            } => {
                let tc = if tool_calls.is_empty() {
                    None
                } else {
                    Some(
                        tool_calls
                            .iter()
                            .map(|tc| openai_oxide::types::chat::ToolCall {
                                id: tc.id.clone(),
                                type_: "function".into(),
                                function: openai_oxide::types::chat::FunctionCall {
                                    name: tc.name.clone(),
                                    arguments: tc.arguments.to_string(),
                                },
                            })
                            .collect(),
                    )
                };
                out.push(ChatCompletionMessageParam::Assistant {
                    content: content.clone(),
                    name: None,
                    tool_calls: tc,
                    refusal: None,
                });
            }
            ChatMessage::User {
                content: UserContent::Text(text),
            } => {
                out.push(ChatCompletionMessageParam::User {
                    content: openai_oxide::types::chat::UserContent::Text(text.clone()),
                    name: None,
                });
            }
            ChatMessage::User {
                content: UserContent::Multimodal(parts),
            } => {
                let content_parts: Vec<ContentPart> = parts
                    .iter()
                    .map(|p| match p {
                        moltis_agents::model::ContentPart::Text(t) => ContentPart::Text {
                            text: t.clone(),
                        },
                        moltis_agents::model::ContentPart::Image { media_type, data } => {
                            let data_uri = format!("data:{media_type};base64,{data}");
                            ContentPart::ImageUrl {
                                image_url: ImageUrl {
                                    url: data_uri,
                                    detail: None,
                                },
                            }
                        }
                    })
                    .collect();
                out.push(ChatCompletionMessageParam::User {
                    content: openai_oxide::types::chat::UserContent::Parts(content_parts),
                    name: None,
                });
            }
            ChatMessage::Tool {
                content,
                tool_call_id,
                ..
            } => {
                out.push(ChatCompletionMessageParam::Tool {
                    content: content.clone(),
                    tool_call_id: tool_call_id.clone(),
                });
            }
        }
    }
    out
}

/// Extract ToolCall from openai-oxide response.
fn extract_tool_calls(
    tool_calls: &Option<Vec<openai_oxide::types::chat::ToolCall>>,
) -> Vec<ToolCall> {
    tool_calls
        .as_ref()
        .map(|tcs| {
            tcs.iter()
                .map(|tc| ToolCall {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    arguments: serde_json::from_str(&tc.function.arguments)
                        .unwrap_or(serde_json::Value::Object(Default::default())),
                })
                .collect()
        })
        .unwrap_or_default()
}

#[async_trait]
impl LlmProvider for OpenAiOxideProvider {
    fn name(&self) -> &str {
        self.alias.as_deref().unwrap_or("openai-oxide")
    }

    fn id(&self) -> &str {
        &self.model
    }

    #[tracing::instrument(skip(self, messages, tools), fields(model = %self.model))]
    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> anyhow::Result<CompletionResponse> {
        let oai_messages = build_messages(messages);
        let mut request = ChatCompletionRequest::new(&self.model, oai_messages);

        if !tools.is_empty() {
            request.tools = Some(
                tools
                    .iter()
                    .filter_map(|t| serde_json::from_value(t.clone()).ok())
                    .collect(),
            );
        }

        let response = self.client.chat().completions().create(request).await?;

        let choice = response.choices.first();
        let text = choice.and_then(|c| c.message.content.clone());
        let tool_calls = choice
            .map(|c| extract_tool_calls(&c.message.tool_calls))
            .unwrap_or_default();

        let usage = response
            .usage
            .as_ref()
            .map(|u| Usage {
                input_tokens: u.prompt_tokens.unwrap_or(0) as u32,
                output_tokens: u.completion_tokens.unwrap_or(0) as u32,
                ..Default::default()
            })
            .unwrap_or_default();

        Ok(CompletionResponse {
            text,
            tool_calls,
            usage,
        })
    }

    fn stream(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        Box::pin(async_stream::stream! {
            let oai_messages = build_messages(&messages);
            let mut request = ChatCompletionRequest::new(&self.model, oai_messages);
            request.stream_options = Some(StreamOptions {
                include_usage: Some(true),
            });

            let mut stream = match self.client.chat().completions().create_stream(request).await {
                Ok(s) => s,
                Err(e) => {
                    yield StreamEvent::Error(format!("{e}"));
                    return;
                }
            };

            // Track which tool call indices we've seen start for.
            let mut seen_starts: HashMap<i32, bool> = HashMap::new();

            while let Some(result) = stream.next().await {
                match result {
                    Ok(response) => {
                        for choice in &response.choices {
                            // Text deltas
                            if let Some(ref content) = choice.delta.content {
                                if !content.is_empty() {
                                    yield StreamEvent::Delta(content.clone());
                                }
                            }

                            // Streaming tool calls
                            if let Some(ref tcs) = choice.delta.tool_calls {
                                for tc in tcs {
                                    let idx = tc.index as usize;

                                    // Emit ToolCallStart on first delta for this index
                                    if !seen_starts.contains_key(&tc.index) {
                                        seen_starts.insert(tc.index, true);
                                        let name = tc
                                            .function
                                            .as_ref()
                                            .and_then(|f| f.name.clone())
                                            .unwrap_or_default();
                                        let id = tc.id.clone().unwrap_or_default();
                                        yield StreamEvent::ToolCallStart {
                                            id,
                                            name,
                                            index: idx,
                                        };
                                    }

                                    // Emit argument deltas
                                    if let Some(ref f) = tc.function {
                                        if let Some(ref args) = f.arguments {
                                            if !args.is_empty() {
                                                yield StreamEvent::ToolCallArgumentsDelta {
                                                    index: idx,
                                                    delta: args.clone(),
                                                };
                                            }
                                        }
                                    }
                                }
                            }

                            // Check finish_reason for tool_calls completion
                            if choice.finish_reason.as_deref() == Some("tool_calls") {
                                for &idx in seen_starts.keys() {
                                    yield StreamEvent::ToolCallComplete {
                                        index: idx as usize,
                                    };
                                }
                            }
                        }

                        if let Some(ref u) = response.usage {
                            yield StreamEvent::Done(Usage {
                                input_tokens: u.prompt_tokens.unwrap_or(0) as u32,
                                output_tokens: u.completion_tokens.unwrap_or(0) as u32,
                                ..Default::default()
                            });
                            return;
                        }
                    }
                    Err(e) => {
                        yield StreamEvent::Error(format!("{e}"));
                        return;
                    }
                }
            }

            yield StreamEvent::Done(Usage::default());
        })
    }
}
