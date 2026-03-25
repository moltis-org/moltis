use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

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
    ChatMessage, CompletionResponse, LlmProvider, ModelMetadata, StreamEvent, ToolCall, Usage,
    UserContent,
};
use moltis_config::ReasoningEffort;

/// Provider backed by the `openai-oxide` crate.
/// Works with OpenAI and any OpenAI-compatible API (Ollama, vLLM, etc.)
/// via custom base URL.
///
/// Advantages over `async_openai_provider`:
/// - Full streaming tool call support (Start/ArgumentsDelta/Complete events)
/// - Tool call extraction from non-streaming responses
/// - Streaming usage tokens (include_usage = true)
/// - Proper tool message mapping with tool_call_id
/// - Assistant tool_calls replay in conversation history
/// - Vision support (multimodal content)
/// - Reasoning effort configuration
/// - reqwest 0.12 + 0.13 compatibility
///
/// Note: `provider-openai-oxide` and `provider-async-openai` both register
/// against the `"openai"` config key. If both features are enabled,
/// whichever registers first wins. Disable one to use the other.
pub struct OpenAiOxideProvider {
    model: String,
    client: OpenAI,
    alias: Option<String>,
    reasoning_effort: Option<ReasoningEffort>,
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
            reasoning_effort: None,
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

/// Build a request with optional tools and reasoning effort.
fn build_request(
    model: &str,
    messages: Vec<ChatCompletionMessageParam>,
    tools: &[serde_json::Value],
    reasoning_effort: Option<ReasoningEffort>,
) -> ChatCompletionRequest {
    let mut request = ChatCompletionRequest::new(model, messages);

    if !tools.is_empty() {
        request.tools = Some(
            tools
                .iter()
                .filter_map(|t| serde_json::from_value(t.clone()).ok())
                .collect(),
        );
    }

    if let Some(effort) = reasoning_effort {
        let oxide_effort = match effort {
            ReasoningEffort::Low => openai_oxide::types::common::ReasoningEffort::Low,
            ReasoningEffort::Medium => openai_oxide::types::common::ReasoningEffort::Medium,
            ReasoningEffort::High => openai_oxide::types::common::ReasoningEffort::High,
        };
        request.reasoning_effort = Some(oxide_effort);
    }

    request
}

/// Create a streaming request with tools and stream_options.
fn build_stream_request(
    model: &str,
    messages: Vec<ChatCompletionMessageParam>,
    tools: &[serde_json::Value],
    reasoning_effort: Option<ReasoningEffort>,
) -> ChatCompletionRequest {
    let mut request = build_request(model, messages, tools, reasoning_effort);
    request.stream_options = Some(StreamOptions {
        include_usage: Some(true),
    });
    request
}

/// Process streaming chunks — shared between stream() and stream_with_tools().
fn make_stream<'a>(
    client: &'a OpenAI,
    request: ChatCompletionRequest,
) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + 'a>> {
    Box::pin(async_stream::stream! {
        let mut stream = match client.chat().completions().create_stream(request).await {
            Ok(s) => s,
            Err(e) => {
                yield StreamEvent::Error(format!("{e}"));
                return;
            }
        };

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

                        // Tool call streaming
                        if let Some(ref tcs) = choice.delta.tool_calls {
                            for tc in tcs {
                                let idx = tc.index as usize;

                                if !seen_starts.contains_key(&tc.index) {
                                    seen_starts.insert(tc.index, true);
                                    let name = tc
                                        .function
                                        .as_ref()
                                        .and_then(|f| f.name.clone())
                                        .unwrap_or_default();
                                    let id = tc.id.clone().unwrap_or_default();
                                    yield StreamEvent::ToolCallStart { id, name, index: idx };
                                }

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

                        // Tool calls complete
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

#[async_trait]
impl LlmProvider for OpenAiOxideProvider {
    fn name(&self) -> &str {
        self.alias.as_deref().unwrap_or("openai-oxide")
    }

    fn id(&self) -> &str {
        &self.model
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn supports_vision(&self) -> bool {
        true
    }

    fn reasoning_effort(&self) -> Option<ReasoningEffort> {
        self.reasoning_effort
    }

    fn with_reasoning_effort(
        self: Arc<Self>,
        effort: ReasoningEffort,
    ) -> Option<Arc<dyn LlmProvider>> {
        Some(Arc::new(Self {
            model: self.model.clone(),
            client: self.client.clone(),
            alias: self.alias.clone(),
            reasoning_effort: Some(effort),
        }))
    }

    fn context_window(&self) -> u32 {
        // GPT-4o default; overridden by model_metadata() at runtime.
        128_000
    }

    #[tracing::instrument(skip(self, messages, tools), fields(model = %self.model))]
    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> anyhow::Result<CompletionResponse> {
        let oai_messages = build_messages(messages);
        let request = build_request(&self.model, oai_messages, tools, self.reasoning_effort);
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
        let oai_messages = build_messages(&messages);
        let request =
            build_stream_request(&self.model, oai_messages, &[], self.reasoning_effort);
        make_stream(&self.client, request)
    }

    fn stream_with_tools(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<serde_json::Value>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        let oai_messages = build_messages(&messages);
        let request =
            build_stream_request(&self.model, oai_messages, &tools, self.reasoning_effort);
        make_stream(&self.client, request)
    }
}
