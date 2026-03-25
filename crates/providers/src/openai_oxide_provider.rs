use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use {
    async_trait::async_trait,
    futures::StreamExt,
    openai_oxide::{
        config::ClientConfig,
        types::{
            chat::{
                ChatCompletionMessageParam, ChatCompletionRequest, ContentPart, ImageUrl,
                StreamOptions,
            },
            responses::{
                OutputItem, ReasoningSummary, Response, ResponseCreateRequest, ResponseInput,
                ResponseInputItem, ResponseStreamEvent, ResponseTool,
            },
        },
        OpenAI,
    },
    tokio_stream::Stream,
};

use moltis_agents::model::{
    ChatMessage, CompletionResponse, LlmProvider, ModelMetadata, StreamEvent, ToolCall, Usage,
    UserContent,
};
use moltis_config::{ReasoningEffort, schema::WireApi};

/// Provider backed by the `openai-oxide` crate.
///
/// Supports both Chat Completions and Responses API via `WireApi` config.
/// Full tool calling, vision, reasoning, streaming — replaces 5000+ lines
/// of manual HTTP/SSE code with openai-oxide's typed client.
///
/// Note: `provider-openai-oxide` and `provider-async-openai` both register
/// against the `"openai"` config key. If both features are enabled,
/// whichever registers first wins. Disable one to use the other.
pub struct OpenAiOxideProvider {
    model: String,
    client: OpenAI,
    alias: Option<String>,
    reasoning_effort: Option<ReasoningEffort>,
    wire_api: WireApi,
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
            wire_api: WireApi::default(),
        }
    }

    pub fn with_wire_api(mut self, wire_api: WireApi) -> Self {
        self.wire_api = wire_api;
        self
    }
}

// ── Chat Completions helpers ──

fn build_chat_messages(messages: &[ChatMessage]) -> Vec<ChatCompletionMessageParam> {
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

fn extract_chat_tool_calls(
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

// ── Responses API helpers ──

/// Split ChatMessages into (instructions, input) for Responses API.
/// System messages become instructions, everything else becomes input items.
fn split_for_responses(messages: &[ChatMessage]) -> (Option<String>, Vec<ResponseInputItem>) {
    let mut instructions: Vec<String> = Vec::new();
    let mut input: Vec<ResponseInputItem> = Vec::new();

    for msg in messages {
        match msg {
            ChatMessage::System { content } => {
                if !content.trim().is_empty() {
                    instructions.push(content.clone());
                }
            }
            ChatMessage::User {
                content: UserContent::Text(text),
            } => {
                input.push(ResponseInputItem {
                    role: openai_oxide::types::responses::Role::User,
                    content: serde_json::json!(text),
                });
            }
            ChatMessage::Assistant { content, .. } => {
                if let Some(text) = content {
                    input.push(ResponseInputItem {
                        role: openai_oxide::types::responses::Role::Assistant,
                        content: serde_json::json!(text),
                    });
                }
            }
            _ => {}
        }
    }

    let instr = if instructions.is_empty() {
        None
    } else {
        Some(instructions.join("\n\n"))
    };

    (instr, input)
}

fn tools_to_response_tools(tools: &[serde_json::Value]) -> Vec<ResponseTool> {
    tools
        .iter()
        .filter_map(|t| {
            let func = t.get("function")?;
            Some(ResponseTool::Function {
                name: func.get("name")?.as_str()?.to_string(),
                description: func.get("description").and_then(|d| d.as_str()).map(String::from),
                parameters: func.get("parameters").cloned(),
                strict: func.get("strict").and_then(|s| s.as_bool()),
            })
        })
        .collect()
}

fn extract_responses_tool_calls(response: &Response) -> Vec<ToolCall> {
    response.function_calls().iter().map(|fc| ToolCall {
        id: fc.call_id.clone(),
        name: fc.name.clone(),
        arguments: fc.arguments.clone(),
    }).collect()
}

// ── Chat Completions streaming ──

fn stream_chat<'a>(
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
                        if let Some(ref content) = choice.delta.content {
                            if !content.is_empty() {
                                yield StreamEvent::Delta(content.clone());
                            }
                        }

                        if let Some(ref tcs) = choice.delta.tool_calls {
                            for tc in tcs {
                                let idx = tc.index as usize;
                                if !seen_starts.contains_key(&tc.index) {
                                    seen_starts.insert(tc.index, true);
                                    yield StreamEvent::ToolCallStart {
                                        id: tc.id.clone().unwrap_or_default(),
                                        name: tc.function.as_ref().and_then(|f| f.name.clone()).unwrap_or_default(),
                                        index: idx,
                                    };
                                }
                                if let Some(ref f) = tc.function {
                                    if let Some(ref args) = f.arguments {
                                        if !args.is_empty() {
                                            yield StreamEvent::ToolCallArgumentsDelta { index: idx, delta: args.clone() };
                                        }
                                    }
                                }
                            }
                        }

                        if choice.finish_reason.as_deref() == Some("tool_calls") {
                            for &idx in seen_starts.keys() {
                                yield StreamEvent::ToolCallComplete { index: idx as usize };
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

// ── Responses API streaming ──

fn stream_responses<'a>(
    client: &'a OpenAI,
    request: ResponseCreateRequest,
) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + 'a>> {
    Box::pin(async_stream::stream! {
        let mut stream = match client.responses().create_stream(request).await {
            Ok(s) => s,
            Err(e) => {
                yield StreamEvent::Error(format!("{e}"));
                return;
            }
        };

        let mut tool_index: usize = 0;

        while let Some(result) = stream.next().await {
            match result {
                Ok(event) => match event {
                    ResponseStreamEvent::ResponseOutputTextDelta(evt) => {
                        if !evt.delta.is_empty() {
                            yield StreamEvent::Delta(evt.delta);
                        }
                    }
                    ResponseStreamEvent::ResponseOutputItemAdded(evt) => {
                        if let OutputItem::FunctionCall(fc) = &evt.item {
                            yield StreamEvent::ToolCallStart {
                                id: fc.call_id.clone(),
                                name: fc.name.clone(),
                                index: tool_index,
                            };
                        }
                    }
                    ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(evt) => {
                        if !evt.delta.is_empty() {
                            yield StreamEvent::ToolCallArgumentsDelta {
                                index: tool_index,
                                delta: evt.delta,
                            };
                        }
                    }
                    ResponseStreamEvent::ResponseFunctionCallArgumentsDone(_) => {
                        yield StreamEvent::ToolCallComplete { index: tool_index };
                        tool_index += 1;
                    }
                    ResponseStreamEvent::ResponseCompleted(evt) => {
                        let usage = evt.response.usage.as_ref().map(|u| Usage {
                            input_tokens: u.input_tokens.unwrap_or(0) as u32,
                            output_tokens: u.output_tokens.unwrap_or(0) as u32,
                            ..Default::default()
                        }).unwrap_or_default();
                        yield StreamEvent::Done(usage);
                        return;
                    }
                    ResponseStreamEvent::ResponseFailed(evt) => {
                        let msg = evt.response.error
                            .as_ref()
                            .map(|e| e.message.clone())
                            .unwrap_or_else(|| "response.failed".into());
                        yield StreamEvent::Error(msg);
                        return;
                    }
                    _ => {}
                },
                Err(e) => {
                    yield StreamEvent::Error(format!("{e}"));
                    return;
                }
            }
        }
        yield StreamEvent::Done(Usage::default());
    })
}

// ── LlmProvider implementation ──

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
            wire_api: self.wire_api,
        }))
    }

    fn context_window(&self) -> u32 {
        128_000
    }

    #[tracing::instrument(skip(self, messages, tools), fields(model = %self.model, api = ?self.wire_api))]
    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> anyhow::Result<CompletionResponse> {
        match self.wire_api {
            WireApi::ChatCompletions => self.complete_chat(messages, tools).await,
            WireApi::Responses => self.complete_responses(messages, tools).await,
        }
    }

    fn stream(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        self.stream_with_tools(messages, vec![])
    }

    fn stream_with_tools(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<serde_json::Value>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        match self.wire_api {
            WireApi::ChatCompletions => {
                let oai_messages = build_chat_messages(&messages);
                let mut request = ChatCompletionRequest::new(&self.model, oai_messages);
                if !tools.is_empty() {
                    request.tools = Some(
                        tools.iter().filter_map(|t| serde_json::from_value(t.clone()).ok()).collect(),
                    );
                }
                if let Some(effort) = self.reasoning_effort {
                    request.reasoning_effort = Some(to_oxide_effort(effort));
                }
                request.stream_options = Some(StreamOptions { include_usage: Some(true) });
                stream_chat(&self.client, request)
            }
            WireApi::Responses => {
                let (instructions, input) = split_for_responses(&messages);
                let mut request = ResponseCreateRequest::new(&self.model)
                    .input(ResponseInput::Messages(input));
                if let Some(instr) = instructions {
                    request = request.instructions(instr);
                }
                if !tools.is_empty() {
                    request.tools = Some(tools_to_response_tools(&tools));
                }
                if let Some(effort) = self.reasoning_effort {
                    request.reasoning = Some(openai_oxide::types::responses::Reasoning {
                        effort: Some(to_oxide_effort(effort)),
                        summary: Some(ReasoningSummary::Auto),
                    });
                }
                stream_responses(&self.client, request)
            }
        }
    }
}

impl OpenAiOxideProvider {
    async fn complete_chat(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> anyhow::Result<CompletionResponse> {
        let oai_messages = build_chat_messages(messages);
        let mut request = ChatCompletionRequest::new(&self.model, oai_messages);
        if !tools.is_empty() {
            request.tools = Some(
                tools.iter().filter_map(|t| serde_json::from_value(t.clone()).ok()).collect(),
            );
        }
        if let Some(effort) = self.reasoning_effort {
            request.reasoning_effort = Some(to_oxide_effort(effort));
        }

        let response = self.client.chat().completions().create(request).await?;
        let choice = response.choices.first();

        Ok(CompletionResponse {
            text: choice.and_then(|c| c.message.content.clone()),
            tool_calls: choice.map(|c| extract_chat_tool_calls(&c.message.tool_calls)).unwrap_or_default(),
            usage: response.usage.as_ref().map(|u| Usage {
                input_tokens: u.prompt_tokens.unwrap_or(0) as u32,
                output_tokens: u.completion_tokens.unwrap_or(0) as u32,
                ..Default::default()
            }).unwrap_or_default(),
        })
    }

    async fn complete_responses(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> anyhow::Result<CompletionResponse> {
        let (instructions, input) = split_for_responses(messages);
        let mut request = ResponseCreateRequest::new(&self.model)
            .input(ResponseInput::Messages(input));
        if let Some(instr) = instructions {
            request = request.instructions(instr);
        }
        if !tools.is_empty() {
            request.tools = Some(tools_to_response_tools(tools));
        }
        if let Some(effort) = self.reasoning_effort {
            request.reasoning = Some(openai_oxide::types::responses::Reasoning {
                effort: Some(to_oxide_effort(effort)),
                summary: Some(ReasoningSummary::Auto),
            });
        }

        let response = self.client.responses().create(request).await?;
        let text = response.output_text();
        let tool_calls = extract_responses_tool_calls(&response);

        Ok(CompletionResponse {
            text: if text.is_empty() { None } else { Some(text) },
            tool_calls,
            usage: response.usage.as_ref().map(|u| Usage {
                input_tokens: u.input_tokens.unwrap_or(0) as u32,
                output_tokens: u.output_tokens.unwrap_or(0) as u32,
                ..Default::default()
            }).unwrap_or_default(),
        })
    }
}

fn to_oxide_effort(effort: ReasoningEffort) -> openai_oxide::types::common::ReasoningEffort {
    match effort {
        ReasoningEffort::Low => openai_oxide::types::common::ReasoningEffort::Low,
        ReasoningEffort::Medium => openai_oxide::types::common::ReasoningEffort::Medium,
        ReasoningEffort::High => openai_oxide::types::common::ReasoningEffort::High,
    }
}
