use std::pin::Pin;

use {async_trait::async_trait, futures::StreamExt, tokio_stream::Stream};

use moltis_agents::model::{
    ChatMessage, CompletionResponse, InputTokenAccounting, LlmProvider, StreamEvent, Usage,
    UserContent,
};

/// Provider backed by the `genai` crate (supports Anthropic, OpenAI, Gemini,
/// Groq, Ollama, xAI, DeepSeek, Cohere, and more via a single client).
pub struct GenaiProvider {
    model: String,
    provider_name: String,
    input_token_accounting: InputTokenAccounting,
    client: genai::Client,
}

impl GenaiProvider {
    /// Create a new `GenaiProvider` with an explicit API key passed via
    /// `AuthResolver`, avoiding the need to set environment variables.
    pub fn new(
        model: String,
        provider_name: String,
        input_token_accounting: InputTokenAccounting,
        api_key: secrecy::Secret<String>,
    ) -> Self {
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
            input_token_accounting,
            client,
        }
    }
}

fn genai_usage_to_usage(
    input_token_accounting: InputTokenAccounting,
    u: &genai::chat::Usage,
) -> Usage {
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
    Usage::from_input_tokens(
        input_token_accounting,
        u.prompt_tokens.unwrap_or(0) as u32,
        u.completion_tokens.unwrap_or(0) as u32,
        cache_read,
        cache_write,
    )
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
                ..
            } => Some(genai::chat::ChatMessage::user(text)),
            ChatMessage::User {
                content: UserContent::Multimodal(_),
                ..
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
        let usage = genai_usage_to_usage(self.input_token_accounting, &chat_res.usage);

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
                            yield StreamEvent::ReasoningDelta(chunk.content);
                        }
                    }
                    Ok(ChatStreamEvent::End(end)) => {
                        let usage = end.captured_usage
                            .as_ref()
                            .map(|usage| genai_usage_to_usage(self.input_token_accounting, usage))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_prompt_usage_removes_cache_buckets_from_fresh_input() {
        let usage = genai::chat::Usage {
            prompt_tokens: Some(100),
            prompt_tokens_details: Some(genai::chat::PromptTokensDetails {
                cached_tokens: Some(30),
                cache_creation_tokens: Some(20),
                audio_tokens: None,
            }),
            completion_tokens: Some(10),
            total_tokens: Some(110),
            ..Default::default()
        };

        let converted = genai_usage_to_usage(InputTokenAccounting::Inclusive, &usage);

        assert_eq!(converted.input_tokens, 50);
        assert_eq!(converted.cache_read_tokens, 30);
        assert_eq!(converted.cache_write_tokens, 20);
        assert_eq!(converted.output_tokens, 10);
    }
}
