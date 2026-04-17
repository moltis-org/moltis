use std::pin::Pin;

use {
    async_openai::{
        config::OpenAIConfig,
        types::chat::{
            ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
            ChatCompletionRequestMessageContentPartImage,
            ChatCompletionRequestMessageContentPartText, ChatCompletionRequestSystemMessageArgs,
            ChatCompletionRequestUserMessageArgs, ChatCompletionRequestUserMessageContent,
            ChatCompletionRequestUserMessageContentPart, CreateChatCompletionRequestArgs, ImageUrl,
        },
    },
    async_trait::async_trait,
    futures::StreamExt,
    tokio_stream::Stream,
};

use moltis_agents::model::{
    ChatMessage, CompletionResponse, LlmProvider, StreamEvent, Usage, UserContent,
};

/// Provider backed by the `async-openai` crate.
/// Works with OpenAI and any OpenAI-compatible API (Ollama, vLLM, etc.)
/// via custom base URL.
pub struct AsyncOpenAiProvider {
    model: String,
    client: async_openai::Client<OpenAIConfig>,
    /// Optional alias for metrics differentiation.
    alias: Option<String>,
}

impl AsyncOpenAiProvider {
    pub fn new(api_key: secrecy::Secret<String>, model: String, base_url: String) -> Self {
        use secrecy::ExposeSecret;
        let config = OpenAIConfig::new()
            .with_api_key(api_key.expose_secret())
            .with_api_base(&base_url);
        Self {
            model,
            client: async_openai::Client::with_config(config),
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
        use secrecy::ExposeSecret;
        let config = OpenAIConfig::new()
            .with_api_key(api_key.expose_secret())
            .with_api_base(&base_url);
        Self {
            model,
            client: async_openai::Client::with_config(config),
            alias,
        }
    }
}

fn build_messages(messages: &[ChatMessage]) -> anyhow::Result<Vec<ChatCompletionRequestMessage>> {
    let mut out = Vec::new();
    for msg in messages {
        match msg {
            ChatMessage::System { content } => {
                out.push(
                    ChatCompletionRequestSystemMessageArgs::default()
                        .content(content.as_str())
                        .build()?
                        .into(),
                );
            },
            ChatMessage::Assistant { content, .. } => {
                out.push(
                    ChatCompletionRequestAssistantMessageArgs::default()
                        .content(content.as_deref().unwrap_or(""))
                        .build()?
                        .into(),
                );
            },
            ChatMessage::User {
                content: UserContent::Text(text),
                name,
            } => {
                let mut builder = ChatCompletionRequestUserMessageArgs::default();
                builder.content(text.as_str());
                if let Some(name) = name {
                    builder.name(name.clone());
                }
                out.push(builder.build()?.into());
            },
            ChatMessage::User {
                content: UserContent::Multimodal(parts),
                name,
            } => {
                let content_parts: Vec<ChatCompletionRequestUserMessageContentPart> = parts
                    .iter()
                    .map(|p| match p {
                        moltis_agents::model::ContentPart::Text(t) => {
                            ChatCompletionRequestUserMessageContentPart::Text(
                                ChatCompletionRequestMessageContentPartText { text: t.clone() },
                            )
                        },
                        moltis_agents::model::ContentPart::Image { media_type, data } => {
                            let data_uri = format!("data:{media_type};base64,{data}");
                            ChatCompletionRequestUserMessageContentPart::ImageUrl(
                                ChatCompletionRequestMessageContentPartImage {
                                    image_url: ImageUrl {
                                        url: data_uri,
                                        detail: None,
                                    },
                                },
                            )
                        },
                    })
                    .collect();
                out.push(ChatCompletionRequestMessage::User(
                    async_openai::types::chat::ChatCompletionRequestUserMessage {
                        content: ChatCompletionRequestUserMessageContent::Array(content_parts),
                        name: name.clone(),
                    },
                ));
            },
            ChatMessage::Tool { content, .. } => {
                // async-openai doesn't have a dedicated tool result builder;
                // send as a user message with the tool output.
                out.push(
                    ChatCompletionRequestUserMessageArgs::default()
                        .content(content.as_str())
                        .build()?
                        .into(),
                );
            },
        }
    }
    Ok(out)
}

#[async_trait]
impl LlmProvider for AsyncOpenAiProvider {
    fn name(&self) -> &str {
        self.alias.as_deref().unwrap_or("async-openai")
    }

    fn id(&self) -> &str {
        &self.model
    }

    async fn complete(
        &self,
        messages: &[ChatMessage],
        _tools: &[serde_json::Value],
    ) -> anyhow::Result<CompletionResponse> {
        let oai_messages = build_messages(messages)?;

        let request = CreateChatCompletionRequestArgs::default()
            .model(&self.model)
            .messages(oai_messages)
            .build()?;

        let response = self.client.chat().create(request).await?;

        let text = response
            .choices
            .first()
            .and_then(|c| c.message.content.clone());

        let usage = response
            .usage
            .as_ref()
            .map(|u| Usage {
                input_tokens: u.prompt_tokens,
                output_tokens: u.completion_tokens,
                ..Default::default()
            })
            .unwrap_or_default();

        Ok(CompletionResponse {
            text,
            tool_calls: vec![],
            usage,
        })
    }

    #[allow(clippy::collapsible_if)]
    fn stream(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        Box::pin(async_stream::stream! {
            let oai_messages = match build_messages(&messages) {
                Ok(m) => m,
                Err(e) => {
                    yield StreamEvent::Error(format!("{e}"));
                    return;
                }
            };

            let request = match CreateChatCompletionRequestArgs::default()
                .model(&self.model)
                .messages(oai_messages)
                .build()
            {
                Ok(r) => r,
                Err(e) => {
                    yield StreamEvent::Error(format!("{e}"));
                    return;
                }
            };

            let mut stream = match self.client.chat().create_stream(request).await {
                Ok(s) => s,
                Err(e) => {
                    yield StreamEvent::Error(format!("{e}"));
                    return;
                }
            };

            while let Some(result) = stream.next().await {
                match result {
                    Ok(response) => {
                        for choice in &response.choices {
                            if let Some(ref content) = choice.delta.content {
                                if !content.is_empty() {
                                    yield StreamEvent::Delta(content.clone());
                                }
                            }
                        }
                        if let Some(ref u) = response.usage {
                            yield StreamEvent::Done(Usage {
                                input_tokens: u.prompt_tokens,
                                output_tokens: u.completion_tokens,
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

#[cfg(test)]
mod tests {
    use {
        crate::async_openai_provider::build_messages,
        async_openai::types::chat::{
            ChatCompletionRequestMessage, ChatCompletionRequestUserMessageContent,
            ChatCompletionRequestUserMessageContentPart,
        },
        moltis_agents::model::{ChatMessage, ContentPart, UserContent},
    };

    #[test]
    fn build_messages_preserves_name_for_text_user_messages() {
        let messages = vec![ChatMessage::User {
            content: UserContent::Text("hello".into()),
            name: Some("Alice".into()),
        }];

        let built = match build_messages(&messages) {
            Ok(built) => built,
            Err(error) => panic!("messages should build: {error}"),
        };
        assert_eq!(built.len(), 1);
        match &built[0] {
            ChatCompletionRequestMessage::User(msg) => {
                assert_eq!(msg.name.as_deref(), Some("Alice"));
                assert_eq!(
                    msg.content,
                    ChatCompletionRequestUserMessageContent::Text("hello".into())
                );
            },
            other => panic!("expected user message, got {other:?}"),
        }
    }

    #[test]
    fn build_messages_preserves_name_for_multimodal_user_messages() {
        let messages = vec![ChatMessage::User {
            content: UserContent::Multimodal(vec![
                ContentPart::Text("describe this".into()),
                ContentPart::Image {
                    media_type: "image/png".into(),
                    data: "base64data".into(),
                },
            ]),
            name: Some("Alice".into()),
        }];

        let built = match build_messages(&messages) {
            Ok(built) => built,
            Err(error) => panic!("messages should build: {error}"),
        };
        assert_eq!(built.len(), 1);
        match &built[0] {
            ChatCompletionRequestMessage::User(msg) => {
                assert_eq!(msg.name.as_deref(), Some("Alice"));
                let ChatCompletionRequestUserMessageContent::Array(parts) = &msg.content else {
                    panic!("expected array content");
                };
                assert!(matches!(
                    parts.first(),
                    Some(ChatCompletionRequestUserMessageContentPart::Text(_))
                ));
                assert!(matches!(
                    parts.get(1),
                    Some(ChatCompletionRequestUserMessageContentPart::ImageUrl(_))
                ));
            },
            other => panic!("expected user message, got {other:?}"),
        }
    }
}
