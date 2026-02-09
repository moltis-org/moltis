use std::pin::Pin;

use {async_trait::async_trait, futures::StreamExt, tokio_stream::Stream};

use crate::model::{ChatMessage, CompletionResponse, LlmProvider, StreamEvent, Usage, UserContent};

/// Provider backed by the `genai` crate (supports Anthropic, OpenAI, Gemini,
/// Groq, Ollama, xAI, DeepSeek, Cohere, and more via a single client).
pub struct GenaiProvider {
    model: String,
    provider_name: String,
    client: genai::Client,
}

impl GenaiProvider {
    /// Create a new `GenaiProvider` with an explicit API key passed via
    /// `AuthResolver`, avoiding the need to set environment variables.
    pub fn new(model: String, provider_name: String, api_key: secrecy::Secret<String>) -> Self {
        use secrecy::ExposeSecret;
        // Expose the secret once to hand it to genai's auth resolver.
        let key = api_key.expose_secret().clone();
        let client = genai::Client::builder()
            .with_auth_resolver(genai::resolver::AuthResolver::from_resolver_fn(
                move |_model_iden| Ok(Some(genai::resolver::AuthData::from_single(key.clone()))),
            ))
            .build();
        Self {
            model,
            provider_name,
            client,
        }
    }
}

fn genai_usage_to_usage(u: &genai::chat::Usage) -> Usage {
    let (cache_read, cache_write) = u
        .prompt_tokens_details
        .as_ref()
        .map(|d| {
            (
                d.cached_tokens.unwrap_or(0) as u32,
                d.cache_creation_tokens.unwrap_or(0) as u32,
            )
        })
        .unwrap_or((0, 0));
    Usage {
        input_tokens: u.prompt_tokens.unwrap_or(0) as u32,
        output_tokens: u.completion_tokens.unwrap_or(0) as u32,
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
    }
}

fn build_genai_messages(messages: &[ChatMessage]) -> Vec<genai::chat::ChatMessage> {
    messages
        .iter()
        .filter_map(|msg| match msg {
            ChatMessage::System { content } => Some(genai::chat::ChatMessage::system(content)),
            ChatMessage::Assistant { content, .. } => Some(genai::chat::ChatMessage::assistant(
                content.as_deref().unwrap_or(""),
            )),
            ChatMessage::User {
                content: UserContent::Text(text),
            } => Some(genai::chat::ChatMessage::user(text)),
            ChatMessage::User {
                content: UserContent::Multimodal(_),
            } => {
                // genai doesn't support multimodal content; send empty string.
                Some(genai::chat::ChatMessage::user(""))
            },
            ChatMessage::Tool { .. } => {
                // genai doesn't have a tool message type; skip.
                None
            },
        })
        .collect()
}

#[async_trait]
impl LlmProvider for GenaiProvider {
    fn name(&self) -> &str {
        &self.provider_name
    }

    fn id(&self) -> &str {
        &self.model
    }

    async fn complete(
        &self,
        messages: &[ChatMessage],
        _tools: &[serde_json::Value],
    ) -> anyhow::Result<CompletionResponse> {
        let chat_req = genai::chat::ChatRequest::new(build_genai_messages(messages));
        let chat_res = self
            .client
            .exec_chat(&self.model, chat_req, None)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let text = chat_res.first_text().map(|s| s.to_string());
        let usage = genai_usage_to_usage(&chat_res.usage);

        Ok(CompletionResponse {
            text,
            tool_calls: vec![],
            usage,
        })
    }

    fn stream(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        Box::pin(async_stream::stream! {
            use genai::chat::ChatStreamEvent;

            let chat_req = genai::chat::ChatRequest::new(build_genai_messages(&messages));
            let mut chat_stream = match self
                .client
                .exec_chat_stream(&self.model, chat_req, None)
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    yield StreamEvent::Error(format!("{e}"));
                    return;
                }
            };

            while let Some(result) = chat_stream.stream.next().await {
                match result {
                    Ok(ChatStreamEvent::Chunk(chunk)) => {
                        if !chunk.content.is_empty() {
                            yield StreamEvent::Delta(chunk.content);
                        }
                    }
                    Ok(ChatStreamEvent::ReasoningChunk(chunk)) => {
                        if !chunk.content.is_empty() {
                            yield StreamEvent::Delta(chunk.content);
                        }
                    }
                    Ok(ChatStreamEvent::End(end)) => {
                        let usage = end.captured_usage
                            .as_ref()
                            .map(genai_usage_to_usage)
                            .unwrap_or_default();
                        yield StreamEvent::Done(usage);
                        return;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        yield StreamEvent::Error(format!("{e}"));
                        return;
                    }
                }
            }
        })
    }
}
