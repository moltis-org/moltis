mod completion;
mod request;
mod streaming;
mod websocket;

use std::pin::Pin;

use {
    async_trait::async_trait,
    moltis_config::schema::{ProviderStreamTransport, WireApi},
    secrecy::ExposeSecret,
    tokio_stream::Stream,
};

use tracing::debug;

use crate::{context_window_for_model, supports_tools_for_model, supports_vision_for_model};

use moltis_agents::model::{
    ChatMessage, CompletionResponse, LlmProvider, ModelMetadata, StreamEvent,
};

use super::OpenAiProvider;

impl OpenAiProvider {
    pub fn new(api_key: secrecy::Secret<String>, model: String, base_url: String) -> Self {
        Self {
            api_key,
            model,
            base_url,
            provider_name: "openai".into(),
            client: crate::shared_http_client(),
            stream_transport: ProviderStreamTransport::Sse,
            wire_api: WireApi::ChatCompletions,
            metadata_cache: tokio::sync::OnceCell::new(),
            tool_mode_override: None,
            reasoning_effort: None,
            cache_retention: moltis_config::CacheRetention::Short,
            strict_tools_override: None,
        }
    }

    pub fn new_with_name(
        api_key: secrecy::Secret<String>,
        model: String,
        base_url: String,
        provider_name: String,
    ) -> Self {
        Self {
            api_key,
            model,
            base_url,
            provider_name,
            client: crate::shared_http_client(),
            stream_transport: ProviderStreamTransport::Sse,
            wire_api: WireApi::ChatCompletions,
            metadata_cache: tokio::sync::OnceCell::new(),
            tool_mode_override: None,
            reasoning_effort: None,
            cache_retention: moltis_config::CacheRetention::Short,
            strict_tools_override: None,
        }
    }

    #[must_use]
    pub fn with_cache_retention(mut self, cache_retention: moltis_config::CacheRetention) -> Self {
        self.cache_retention = cache_retention;
        self
    }

    #[must_use]
    pub fn with_stream_transport(mut self, stream_transport: ProviderStreamTransport) -> Self {
        self.stream_transport = stream_transport;
        self
    }

    #[must_use]
    pub fn with_tool_mode(mut self, mode: moltis_config::ToolMode) -> Self {
        self.tool_mode_override = Some(mode);
        self
    }

    #[must_use]
    pub fn with_wire_api(mut self, wire_api: WireApi) -> Self {
        self.wire_api = wire_api;
        self
    }

    #[must_use]
    pub fn with_strict_tools(mut self, strict: bool) -> Self {
        self.strict_tools_override = Some(strict);
        self
    }

    /// Return the reasoning effort string if configured.
    pub(crate) fn reasoning_effort_str(&self) -> Option<&'static str> {
        use moltis_agents::model::ReasoningEffort;
        self.reasoning_effort.map(|e| match e {
            ReasoningEffort::Low => "low",
            ReasoningEffort::Medium => "medium",
            ReasoningEffort::High => "high",
        })
    }

    /// Apply `reasoning_effort` for the **Chat Completions** API (used by
    /// `complete()` and `stream_with_tools_sse()`).
    ///
    /// Format: `"reasoning_effort": "high"` (top-level string field).
    pub(crate) fn apply_reasoning_effort_chat(&self, body: &mut serde_json::Value) {
        if let Some(effort) = self.reasoning_effort_str() {
            body["reasoning_effort"] = serde_json::json!(effort);
        }
    }

    /// Apply `reasoning_effort` for the **Responses** API (used by
    /// `stream_with_tools_websocket()`).
    ///
    /// Format: `"reasoning": { "effort": "high" }` (nested object).
    pub(crate) fn apply_reasoning_effort_responses(&self, body: &mut serde_json::Value) {
        if let Some(effort) = self.reasoning_effort_str() {
            body["reasoning"] = serde_json::json!({ "effort": effort });
        }
    }

    /// Build the HTTP URL for the Responses API (`/responses`).
    ///
    /// If the base URL already ends with `/responses`, use it as-is.
    /// Otherwise derive it as a sibling of `/chat/completions`, ensuring
    /// `/v1` is present — matching the normalization in
    /// `responses_websocket_url`.
    pub(crate) fn responses_sse_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/responses") {
            return base.to_string();
        }
        if let Some(prefix) = base.strip_suffix("/chat/completions") {
            return format!("{prefix}/responses");
        }
        // Ensure /v1 is present, consistent with responses_websocket_url.
        if base.ends_with("/v1") {
            format!("{base}/responses")
        } else {
            format!("{base}/v1/responses")
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &str {
        &self.provider_name
    }

    fn reasoning_effort(&self) -> Option<moltis_agents::model::ReasoningEffort> {
        self.reasoning_effort
    }

    fn with_reasoning_effort(
        self: std::sync::Arc<Self>,
        effort: moltis_agents::model::ReasoningEffort,
    ) -> Option<std::sync::Arc<dyn LlmProvider>> {
        Some(std::sync::Arc::new(Self {
            api_key: self.api_key.clone(),
            model: self.model.clone(),
            base_url: self.base_url.clone(),
            provider_name: self.provider_name.clone(),
            client: self.client,
            stream_transport: self.stream_transport,
            metadata_cache: tokio::sync::OnceCell::new(),
            tool_mode_override: self.tool_mode_override,
            reasoning_effort: Some(effort),
            wire_api: self.wire_api,
            cache_retention: self.cache_retention,
            strict_tools_override: self.strict_tools_override,
        }))
    }

    fn id(&self) -> &str {
        &self.model
    }

    fn supports_tools(&self) -> bool {
        match self.tool_mode_override {
            Some(moltis_config::ToolMode::Native) => true,
            Some(moltis_config::ToolMode::Text | moltis_config::ToolMode::Off) => false,
            Some(moltis_config::ToolMode::Auto) | None => supports_tools_for_model(&self.model),
        }
    }

    fn tool_mode(&self) -> Option<moltis_config::ToolMode> {
        self.tool_mode_override
    }

    fn context_window(&self) -> u32 {
        context_window_for_model(&self.model)
    }

    fn supports_vision(&self) -> bool {
        supports_vision_for_model(&self.model)
    }

    async fn model_metadata(&self) -> anyhow::Result<ModelMetadata> {
        let meta = self
            .metadata_cache
            .get_or_try_init(|| async {
                let url = format!("{}/models/{}", self.base_url, self.model);
                debug!(url = %url, model = %self.model, "fetching model metadata");

                let resp = self
                    .client
                    .get(&url)
                    .header(
                        "Authorization",
                        format!("Bearer {}", self.api_key.expose_secret()),
                    )
                    .send()
                    .await?;

                if !resp.status().is_success() {
                    anyhow::bail!(
                        "model metadata API returned HTTP {}",
                        resp.status().as_u16()
                    );
                }

                let body: serde_json::Value = resp.json().await?;

                // OpenAI uses "context_window", some compat providers use "context_length".
                let context_length = body
                    .get("context_window")
                    .or_else(|| body.get("context_length"))
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32)
                    .unwrap_or_else(|| self.context_window());

                Ok(ModelMetadata {
                    id: body
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&self.model)
                        .to_string(),
                    context_length,
                })
            })
            .await?;
        Ok(meta.clone())
    }

    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> anyhow::Result<CompletionResponse> {
        if matches!(self.wire_api, WireApi::Responses) {
            return self.complete_responses(messages, tools).await;
        }
        self.complete_chat(messages, tools).await
    }

    #[allow(clippy::collapsible_if)]
    fn stream(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        self.stream_with_tools(messages, vec![])
    }

    async fn probe(&self) -> anyhow::Result<()> {
        match self.wire_api {
            WireApi::Responses => self.probe_responses().await,
            WireApi::ChatCompletions => self.probe_chat_completions().await,
        }
    }

    #[allow(clippy::collapsible_if)]
    fn stream_with_tools(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<serde_json::Value>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        match (self.wire_api, self.stream_transport) {
            (WireApi::Responses, ProviderStreamTransport::Sse) => {
                self.stream_responses_sse(messages, tools)
            },
            (WireApi::Responses, _) => {
                // WebSocket / Auto both go through the WS path which already
                // uses the responses format.
                self.stream_with_tools_websocket(
                    messages,
                    tools,
                    matches!(self.stream_transport, ProviderStreamTransport::Auto),
                )
            },
            (WireApi::ChatCompletions, ProviderStreamTransport::Sse) => {
                self.stream_with_tools_sse(messages, tools)
            },
            (WireApi::ChatCompletions, ProviderStreamTransport::Websocket) => {
                self.stream_with_tools_websocket(messages, tools, false)
            },
            (WireApi::ChatCompletions, ProviderStreamTransport::Auto) => {
                self.stream_with_tools_websocket(messages, tools, true)
            },
        }
    }
}
