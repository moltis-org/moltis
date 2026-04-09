use std::{collections::HashSet, pin::Pin, sync::mpsc, time::Duration};

use {async_trait::async_trait, futures::StreamExt, secrecy::ExposeSecret, tokio_stream::Stream};

use tracing::{debug, trace, warn};

use moltis_agents::model::{
    ChatMessage, CompletionResponse, ContentPart, LlmProvider, StreamEvent, ToolCall, Usage,
    UserContent,
};

pub struct AnthropicProvider {
    api_key: secrecy::Secret<String>,
    model: String,
    base_url: String,
    client: &'static reqwest::Client,
    /// Optional alias for metrics differentiation (e.g., "anthropic-work", "anthropic-2").
    alias: Option<String>,
    /// Optional reasoning effort level for extended thinking.
    reasoning_effort: Option<moltis_agents::model::ReasoningEffort>,
    /// Prompt cache retention policy. When `None`, caching is disabled.
    cache_retention: moltis_config::CacheRetention,
}

const ANTHROPIC_MODELS_ENDPOINT_PATH: &str = "/v1/models";

impl AnthropicProvider {
    pub fn new(api_key: secrecy::Secret<String>, model: String, base_url: String) -> Self {
        Self {
            api_key,
            model,
            base_url,
            client: crate::shared_http_client(),
            alias: None,
            reasoning_effort: None,
            cache_retention: moltis_config::CacheRetention::Short,
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
            client: crate::shared_http_client(),
            alias,
            reasoning_effort: None,
            cache_retention: moltis_config::CacheRetention::Short,
        }
    }

    #[must_use]
    pub fn with_cache_retention(mut self, cache_retention: moltis_config::CacheRetention) -> Self {
        self.cache_retention = cache_retention;
        self
    }

    /// Returns `true` when prompt caching is enabled (short or long retention).
    fn caching_enabled(&self) -> bool {
        !matches!(self.cache_retention, moltis_config::CacheRetention::None)
    }

    /// Apply `thinking` configuration to an Anthropic request body based on
    /// the configured reasoning effort.
    fn apply_thinking(&self, body: &mut serde_json::Value) {
        use moltis_agents::model::ReasoningEffort;
        let Some(effort) = self.reasoning_effort else {
            return;
        };
        let budget_tokens: u64 = match effort {
            ReasoningEffort::Low => 4096,
            ReasoningEffort::Medium => 10240,
            ReasoningEffort::High => 32768,
        };
        body["thinking"] = serde_json::json!({
            "type": "enabled",
            "budget_tokens": budget_tokens,
        });
        // Extended thinking requires higher max_tokens than budget_tokens.
        if let Some(max_tokens) = body["max_tokens"].as_u64()
            && max_tokens <= budget_tokens
        {
            body["max_tokens"] = serde_json::json!(budget_tokens + 4096);
        }
    }

    async fn probe_request(&self) -> anyhow::Result<()> {
        let (system_value, anthropic_messages) =
            to_anthropic_messages(&[ChatMessage::user("ping")], false);

        let mut body = serde_json::json!({
            "model": self.model,
            // Probe for reachability, not full extended-thinking behavior.
            "max_tokens": 1,
            "messages": anthropic_messages,
        });

        if let Some(ref sys) = system_value {
            body["system"] = sys.clone();
        }

        debug!(model = %self.model, "anthropic probe request");
        trace!(body = %serde_json::to_string(&body).unwrap_or_default(), "anthropic probe request body");

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
            let retry_after_ms = retry_after_ms_from_headers(http_resp.headers());
            let body_text = http_resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %body_text, "anthropic probe API error");
            anyhow::bail!(
                "{}",
                with_retry_after_marker(
                    format!("Anthropic API error HTTP {status}: {body_text}"),
                    retry_after_ms
                )
            );
        }

        Ok(())
    }
}

fn formatted_model_name(model_id: &str) -> String {
    let raw = model_id.strip_prefix("claude-").unwrap_or(model_id);
    let mut pieces = Vec::new();
    let chunks: Vec<&str> = raw.split('-').filter(|chunk| !chunk.is_empty()).collect();
    let mut index = 0usize;
    while index < chunks.len() {
        let chunk = chunks[index];
        let piece = if chunk.chars().all(|ch| ch.is_ascii_digit()) && chunk.len() == 1 {
            if let Some(next) = chunks.get(index + 1)
                && next.chars().all(|ch| ch.is_ascii_digit())
                && next.len() == 1
            {
                index += 1;
                format!("{chunk}.{next}")
            } else {
                chunk.to_string()
            }
        } else if chunk.chars().all(|ch| ch.is_ascii_digit()) && chunk.len() == 8 {
            let year = &chunk[0..4];
            let month = &chunk[4..6];
            let day = &chunk[6..8];
            format!("{year}-{month}-{day}")
        } else {
            let mut chars = chunk.chars();
            match chars.next() {
                Some(first) => {
                    let mut out = String::new();
                    out.push(first.to_ascii_uppercase());
                    out.push_str(chars.as_str());
                    out
                },
                None => continue,
            }
        };
        pieces.push(piece);
        index += 1;
    }

    if pieces.is_empty() {
        return model_id.to_string();
    }

    format!("Claude {}", pieces.join(" "))
}

fn normalize_display_name(model_id: &str, display_name: Option<&str>) -> String {
    let normalized = display_name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(model_id);
    if normalized == model_id {
        return formatted_model_name(model_id);
    }
    normalized.to_string()
}

fn parse_model_entry(entry: &serde_json::Value) -> Option<crate::DiscoveredModel> {
    let obj = entry.as_object()?;
    let model_id = obj.get("id").and_then(serde_json::Value::as_str)?;

    if !super::is_chat_capable_model(model_id) {
        return None;
    }

    let display_name = obj.get("display_name").and_then(serde_json::Value::as_str);

    Some(crate::DiscoveredModel::new(
        model_id,
        normalize_display_name(model_id, display_name),
    ))
}

fn parse_models_payload(value: &serde_json::Value) -> Vec<crate::DiscoveredModel> {
    let entries = value
        .get("data")
        .and_then(serde_json::Value::as_array)
        .or_else(|| value.as_array())
        .cloned()
        .unwrap_or_default();

    let mut models = Vec::new();
    let mut seen = HashSet::new();
    for entry in entries {
        if let Some(model) = parse_model_entry(&entry)
            && seen.insert(model.id.clone())
        {
            models.push(model);
        }
    }

    models
}

fn mark_recommended_models(models: &mut [crate::DiscoveredModel]) {
    for model in models.iter_mut().take(3) {
        model.recommended = true;
    }
}

fn models_endpoint(base_url: &str) -> String {
    format!(
        "{}{ANTHROPIC_MODELS_ENDPOINT_PATH}",
        base_url.trim_end_matches('/')
    )
}

pub async fn fetch_models_from_api(
    api_key: secrecy::Secret<String>,
    base_url: String,
) -> anyhow::Result<Vec<crate::DiscoveredModel>> {
    let client = crate::shared_http_client();
    let mut models = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut seen_after_ids = HashSet::new();
    let mut after_id: Option<String> = None;

    loop {
        let mut request = client
            .get(models_endpoint(&base_url))
            .timeout(Duration::from_secs(15))
            .header("x-api-key", api_key.expose_secret())
            .header("anthropic-version", "2023-06-01")
            .header("accept", "application/json");

        if let Some(ref after) = after_id {
            request = request.query(&[("after_id", after)]);
        }

        let response = request.send().await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            anyhow::bail!("anthropic models API error HTTP {status}: {body}");
        }

        let payload: serde_json::Value = serde_json::from_str(&body)?;
        for model in parse_models_payload(&payload) {
            if seen_ids.insert(model.id.clone()) {
                models.push(model);
            }
        }

        let has_more = payload
            .get("has_more")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let next_after_id = payload
            .get("last_id")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(ToOwned::to_owned);

        if !has_more {
            break;
        }

        let Some(next_after_id) = next_after_id else {
            break;
        };

        if !seen_after_ids.insert(next_after_id.clone()) {
            break;
        }
        after_id = Some(next_after_id);
    }

    mark_recommended_models(&mut models);

    if models.is_empty() {
        anyhow::bail!("anthropic models API returned no models");
    }

    Ok(models)
}

pub fn start_model_discovery(
    api_key: secrecy::Secret<String>,
    base_url: String,
) -> mpsc::Receiver<anyhow::Result<Vec<crate::DiscoveredModel>>> {
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(anyhow::Error::from)
            .and_then(|rt| rt.block_on(fetch_models_from_api(api_key, base_url)));
        let _ = tx.send(result);
    });
    rx
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

fn retry_after_ms_from_headers(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    super::retry_after_ms_from_headers(headers)
}

fn with_retry_after_marker(base: String, retry_after_ms: Option<u64>) -> String {
    super::with_retry_after_marker(base, retry_after_ms)
}

/// Convert `ChatMessage` list to Anthropic format.
///
/// Returns `(system_blocks_or_text, anthropic_messages)`. System messages are
/// extracted (Anthropic takes them as a top-level `system` field).
///
/// When `caching` is true, the system prompt is returned as a content-block
/// array with `cache_control` on the last block, and the last user message
/// gets `cache_control` on its final content block (enabling prompt caching
/// on the conversation prefix). When `caching` is false, system is returned
/// as a plain string and no `cache_control` blocks are injected.
fn to_anthropic_messages(
    messages: &[ChatMessage],
    caching: bool,
) -> (Option<serde_json::Value>, Vec<serde_json::Value>) {
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

    let system_value = system_text.map(|text| {
        if caching {
            // Content-block array with cache_control on the last block.
            serde_json::json!([{
                "type": "text",
                "text": text,
                "cache_control": { "type": "ephemeral" }
            }])
        } else {
            // Plain string — no caching.
            serde_json::Value::String(text)
        }
    });

    if caching {
        inject_cache_control_on_last_user_message(&mut out);
    }

    (system_value, out)
}

/// Find the last user message in the output array and add `cache_control`
/// to its final content block, enabling Anthropic prompt caching on the
/// conversation prefix.
fn inject_cache_control_on_last_user_message(messages: &mut [serde_json::Value]) {
    let cache_control = serde_json::json!({ "type": "ephemeral" });

    let Some(last_user) = messages
        .iter_mut()
        .rev()
        .find(|m| m.get("role").and_then(serde_json::Value::as_str) == Some("user"))
    else {
        return;
    };

    match last_user.get_mut("content") {
        // String content — convert to content-block array with cache_control.
        Some(content) if content.is_string() => {
            let text = content.as_str().unwrap_or_default().to_string();
            last_user["content"] = serde_json::json!([{
                "type": "text",
                "text": text,
                "cache_control": cache_control
            }]);
        },
        // Array content — add cache_control to the last block.
        Some(content) if content.is_array() => {
            if let Some(last_block) = content.as_array_mut().and_then(|arr| arr.last_mut()) {
                last_block["cache_control"] = cache_control;
            }
        },
        _ => {},
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        self.alias.as_deref().unwrap_or("anthropic")
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
            client: self.client,
            alias: self.alias.clone(),
            reasoning_effort: Some(effort),
            cache_retention: self.cache_retention,
        }))
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
        let caching = self.caching_enabled();
        let (system_value, anthropic_messages) = to_anthropic_messages(messages, caching);

        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": 4096,
            "messages": anthropic_messages,
        });

        if let Some(ref sys) = system_value {
            body["system"] = sys.clone();
        }

        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(to_anthropic_tools(tools));
        }

        self.apply_thinking(&mut body);

        debug!(
            model = %self.model,
            messages_count = anthropic_messages.len(),
            tools_count = tools.len(),
            has_system = system_value.is_some(),
            caching = caching,
            reasoning_effort = ?self.reasoning_effort,
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
            let retry_after_ms = retry_after_ms_from_headers(http_resp.headers());
            let body_text = http_resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %body_text, "anthropic API error");
            anyhow::bail!(
                "{}",
                with_retry_after_marker(
                    format!("Anthropic API error HTTP {status}: {body_text}"),
                    retry_after_ms
                )
            );
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

        if usage.cache_read_tokens > 0 || usage.cache_write_tokens > 0 {
            debug!(
                model = %self.model,
                cache_read = usage.cache_read_tokens,
                cache_write = usage.cache_write_tokens,
                input = usage.input_tokens,
                "anthropic prompt cache"
            );
        }

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

    async fn probe(&self) -> anyhow::Result<()> {
        self.probe_request().await
    }

    #[allow(clippy::collapsible_if)]
    fn stream_with_tools(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<serde_json::Value>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        Box::pin(async_stream::stream! {
            let caching = self.caching_enabled();
            let (system_value, anthropic_messages) = to_anthropic_messages(&messages, caching);

            let mut body = serde_json::json!({
                "model": self.model,
                "max_tokens": 4096,
                "messages": anthropic_messages,
                "stream": true,
            });

            if let Some(ref sys) = system_value {
                body["system"] = sys.clone();
            }

            if !tools.is_empty() {
                body["tools"] = serde_json::Value::Array(to_anthropic_tools(&tools));
            }

            self.apply_thinking(&mut body);

            debug!(
                model = %self.model,
                messages_count = anthropic_messages.len(),
                tools_count = tools.len(),
                has_system = system_value.is_some(),
                caching = caching,
                reasoning_effort = ?self.reasoning_effort,
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
                        let retry_after_ms = retry_after_ms_from_headers(r.headers());
                        let body_text = r.text().await.unwrap_or_default();
                        yield StreamEvent::Error(with_retry_after_marker(
                            format!("HTTP {status}: {body_text}"),
                            retry_after_ms,
                        ));
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

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use axum::{
        Json, Router,
        extract::{Query, Request},
        http::StatusCode,
        routing::{get, post},
    };

    use super::*;

    #[derive(Default, Clone)]
    struct CapturedRequest {
        body: Option<serde_json::Value>,
    }

    async fn start_probe_mock() -> (String, Arc<Mutex<Vec<CapturedRequest>>>) {
        let captured: Arc<Mutex<Vec<CapturedRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_clone = captured.clone();

        let app = Router::new().route(
            "/v1/messages",
            post(move |req: Request| {
                let cap = captured_clone.clone();
                async move {
                    let body_bytes = axum::body::to_bytes(req.into_body(), 1024 * 1024)
                        .await
                        .unwrap_or_default();
                    let body: Option<serde_json::Value> = serde_json::from_slice(&body_bytes).ok();
                    cap.lock().unwrap().push(CapturedRequest { body });

                    axum::response::Response::builder()
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from("{}"))
                        .unwrap()
                }
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        (format!("http://{addr}"), captured)
    }

    async fn start_models_mock(
        responses: Arc<Mutex<HashMap<Option<String>, (StatusCode, serde_json::Value)>>>,
    ) -> String {
        let app = Router::new().route(
            ANTHROPIC_MODELS_ENDPOINT_PATH,
            get(move |Query(params): Query<HashMap<String, String>>| {
                let responses = responses.clone();
                async move {
                    let key = params.get("after_id").cloned();
                    let (status, body) = responses
                        .lock()
                        .expect("lock responses")
                        .get(&key)
                        .cloned()
                        .unwrap_or_else(|| {
                            (
                                StatusCode::NOT_FOUND,
                                serde_json::json!({ "error": "missing test response" }),
                            )
                        });
                    (status, Json(body))
                }
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        format!("http://{addr}")
    }

    #[test]
    fn retry_after_ms_from_headers_parses_seconds() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("12"),
        );
        assert_eq!(retry_after_ms_from_headers(&headers), Some(12_000));
    }

    #[test]
    fn retry_after_ms_from_headers_ignores_non_numeric_values() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT"),
        );
        assert_eq!(retry_after_ms_from_headers(&headers), None);
    }

    #[test]
    fn with_retry_after_marker_appends_retry_hint() {
        let base = "HTTP 429: rate limit exceeded".to_string();
        assert_eq!(
            with_retry_after_marker(base.clone(), Some(3_000)),
            "HTTP 429: rate limit exceeded (retry_after_ms=3000)"
        );
        assert_eq!(
            with_retry_after_marker(base.clone(), None),
            "HTTP 429: rate limit exceeded"
        );
    }

    #[test]
    fn apply_thinking_injects_budget_for_high_effort() {
        let provider = AnthropicProvider {
            api_key: secrecy::Secret::new("test".into()),
            model: "claude-opus-4-5-20251101".into(),
            base_url: "https://api.anthropic.com".into(),
            client: crate::shared_http_client(),
            alias: None,
            reasoning_effort: Some(moltis_agents::model::ReasoningEffort::High),
            cache_retention: moltis_config::CacheRetention::Short,
        };
        let mut body =
            serde_json::json!({ "model": "claude-opus-4-5-20251101", "max_tokens": 4096 });
        provider.apply_thinking(&mut body);

        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 32768);
        // max_tokens must exceed budget_tokens
        assert!(body["max_tokens"].as_u64().unwrap() > 32768);
    }

    #[test]
    fn apply_thinking_skipped_when_no_effort() {
        let provider = AnthropicProvider::new(
            secrecy::Secret::new("test".into()),
            "claude-opus-4-5-20251101".into(),
            "https://api.anthropic.com".into(),
        );
        let mut body = serde_json::json!({ "model": "test", "max_tokens": 4096 });
        provider.apply_thinking(&mut body);
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn apply_thinking_low_effort_budget() {
        let provider = AnthropicProvider {
            api_key: secrecy::Secret::new("test".into()),
            model: "claude-sonnet-4-5-20250929".into(),
            base_url: "https://api.anthropic.com".into(),
            client: crate::shared_http_client(),
            alias: None,
            reasoning_effort: Some(moltis_agents::model::ReasoningEffort::Low),
            cache_retention: moltis_config::CacheRetention::Short,
        };
        let mut body = serde_json::json!({ "model": "test", "max_tokens": 4096 });
        provider.apply_thinking(&mut body);

        assert_eq!(body["thinking"]["budget_tokens"], 4096);
        // max_tokens should be bumped since it equals budget_tokens
        assert!(body["max_tokens"].as_u64().unwrap() > 4096);
    }

    #[test]
    fn with_reasoning_effort_creates_new_provider() {
        use std::sync::Arc;
        let provider = Arc::new(AnthropicProvider::new(
            secrecy::Secret::new("test-key".into()),
            "claude-opus-4-5-20251101".into(),
            "https://api.anthropic.com".into(),
        ));
        assert!(provider.reasoning_effort().is_none());

        let with_effort = provider
            .with_reasoning_effort(moltis_agents::model::ReasoningEffort::High)
            .expect("anthropic supports reasoning_effort");
        assert_eq!(
            with_effort.reasoning_effort(),
            Some(moltis_agents::model::ReasoningEffort::High)
        );
        assert_eq!(with_effort.id(), "claude-opus-4-5-20251101");
    }

    #[tokio::test]
    async fn probe_request_caps_anthropic_output_to_one_token() {
        let (base_url, captured) = start_probe_mock().await;
        let provider = AnthropicProvider::new(
            secrecy::Secret::new("test-key".into()),
            "claude-sonnet-4-5-20250929".into(),
            base_url,
        );

        provider.probe().await.expect("probe should succeed");

        let reqs = captured.lock().unwrap();
        assert_eq!(reqs.len(), 1);
        let body = reqs[0].body.as_ref().expect("request should have a body");
        assert_eq!(body["max_tokens"], 1);
    }

    #[tokio::test]
    async fn fetch_models_from_api_paginates_deduplicates_and_marks_first_three_once() {
        let mut responses = HashMap::new();
        responses.insert(
            None,
            (
                StatusCode::OK,
                serde_json::json!({
                    "data": [
                        {"id": "claude-opus-4-6", "display_name": "Claude Opus 4.6", "type": "model"},
                        {"id": "claude-sonnet-4-6", "display_name": "Claude Sonnet 4.6", "type": "model"},
                        {"id": "claude-haiku-4-5", "display_name": "Claude Haiku 4.5", "type": "model"}
                    ],
                    "has_more": true,
                    "last_id": "cursor-1"
                }),
            ),
        );
        responses.insert(
            Some("cursor-1".to_string()),
            (
                StatusCode::OK,
                serde_json::json!({
                    "data": [
                        {"id": "claude-haiku-4-5", "display_name": "Claude Haiku 4.5", "type": "model"},
                        {"id": "claude-3-7-sonnet-20250219", "display_name": "Claude 3.7 Sonnet", "type": "model"}
                    ],
                    "has_more": false,
                    "last_id": "cursor-2"
                }),
            ),
        );
        let base_url = start_models_mock(Arc::new(Mutex::new(responses))).await;

        let models = fetch_models_from_api(secrecy::Secret::new("test-key".into()), base_url)
            .await
            .expect("model discovery should succeed");

        let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(ids, vec![
            "claude-opus-4-6",
            "claude-sonnet-4-6",
            "claude-haiku-4-5",
            "claude-3-7-sonnet-20250219",
        ]);
        assert!(models[0].recommended);
        assert!(models[1].recommended);
        assert!(models[2].recommended);
        assert!(!models[3].recommended);
    }

    #[tokio::test]
    async fn fetch_models_from_api_ignores_non_chat_entries() {
        let mut responses = HashMap::new();
        responses.insert(
            None,
            (
                StatusCode::OK,
                serde_json::json!({
                    "data": [
                        {"id": "claude-sonnet-4-6", "display_name": "Claude Sonnet 4.6", "type": "model"},
                        {"id": "claude-embeddings-v1", "display_name": "Claude Embeddings", "type": "model"},
                        {"id": "claude-opus-4-6", "display_name": "Claude Opus 4.6", "type": "model"}
                    ],
                    "has_more": false
                }),
            ),
        );
        let base_url = start_models_mock(Arc::new(Mutex::new(responses))).await;

        let models = fetch_models_from_api(secrecy::Secret::new("test-key".into()), base_url)
            .await
            .expect("model discovery should succeed");

        let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(ids, vec!["claude-sonnet-4-6", "claude-opus-4-6"]);
    }

    #[tokio::test]
    async fn fetch_models_from_api_errors_on_http_failure() {
        let mut responses = HashMap::new();
        responses.insert(
            None,
            (
                StatusCode::TOO_MANY_REQUESTS,
                serde_json::json!({ "error": { "message": "rate limited" } }),
            ),
        );
        let base_url = start_models_mock(Arc::new(Mutex::new(responses))).await;

        let err = fetch_models_from_api(secrecy::Secret::new("test-key".into()), base_url)
            .await
            .expect_err("HTTP failure should surface as an error");

        assert!(err.to_string().contains("HTTP 429"));
    }

    #[tokio::test]
    async fn fetch_models_from_api_errors_when_no_chat_models_are_returned() {
        let mut responses = HashMap::new();
        responses.insert(
            None,
            (
                StatusCode::OK,
                serde_json::json!({
                    "data": [
                        {"id": "claude-embeddings-v1", "display_name": "Claude Embeddings", "type": "model"}
                    ],
                    "has_more": false
                }),
            ),
        );
        let base_url = start_models_mock(Arc::new(Mutex::new(responses))).await;

        let err = fetch_models_from_api(secrecy::Secret::new("test-key".into()), base_url)
            .await
            .expect_err("empty chat-capable catalog should error");

        assert!(err.to_string().contains("returned no models"));
    }

    #[test]
    fn to_anthropic_messages_merges_all_system_into_top_level() {
        use moltis_agents::model::{ChatMessage, UserContent};

        let messages = vec![
            ChatMessage::system("You are a helpful assistant."),
            ChatMessage::User {
                content: UserContent::Text("hello".into()),
            },
            ChatMessage::system("The current user datetime is 2026-03-24 01:23:45 CET."),
            ChatMessage::User {
                content: UserContent::Text("what time is it?".into()),
            },
        ];

        let (system_value, out) = to_anthropic_messages(&messages, true);

        // System is returned as a content-block array with cache_control.
        let blocks = system_value
            .expect("system should be present")
            .as_array()
            .expect("should be array")
            .clone();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(
            blocks[0]["text"],
            "You are a helpful assistant.\n\nThe current user datetime is 2026-03-24 01:23:45 CET."
        );
        assert_eq!(blocks[0]["cache_control"]["type"], "ephemeral");

        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["role"], "user");
        assert_eq!(out[1]["role"], "user");
    }

    #[test]
    fn system_prompt_serializes_as_content_block_array_with_cache_control() {
        let messages = vec![
            ChatMessage::system("You are a coding assistant."),
            ChatMessage::User {
                content: UserContent::Text("hi".into()),
            },
        ];

        let (system_value, _) = to_anthropic_messages(&messages, true);
        let blocks = system_value
            .expect("system should be present")
            .as_array()
            .expect("should be array")
            .clone();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[0]["text"], "You are a coding assistant.");
        assert_eq!(blocks[0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn last_user_message_gets_cache_control() {
        let messages = vec![
            ChatMessage::system("sys"),
            ChatMessage::User {
                content: UserContent::Text("first".into()),
            },
            ChatMessage::Assistant {
                content: Some("reply".into()),
                tool_calls: vec![],
            },
            ChatMessage::User {
                content: UserContent::Text("second".into()),
            },
        ];

        let (_, out) = to_anthropic_messages(&messages, true);

        // First user message should NOT have cache_control.
        assert_eq!(out[0]["content"], "first");

        // Last user message should be converted to content-block array with cache_control.
        let last_user = &out[2];
        assert_eq!(last_user["role"], "user");
        let content = last_user["content"].as_array().expect("should be array");
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "second");
        assert_eq!(content[0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn multimodal_user_message_gets_cache_control_on_last_block() {
        let messages = vec![ChatMessage::User {
            content: UserContent::Multimodal(vec![
                ContentPart::Text("describe this".into()),
                ContentPart::Image {
                    media_type: "image/png".into(),
                    data: "base64data".into(),
                },
            ]),
        }];

        let (_, out) = to_anthropic_messages(&messages, true);
        let content = out[0]["content"].as_array().expect("should be array");
        assert_eq!(content.len(), 2);

        // First block should NOT have cache_control.
        assert!(content[0].get("cache_control").is_none());

        // Last block (image) should have cache_control.
        assert_eq!(content[1]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn no_system_returns_none() {
        let messages = vec![ChatMessage::User {
            content: UserContent::Text("hello".into()),
        }];
        let (system_value, _) = to_anthropic_messages(&messages, true);
        assert!(system_value.is_none());
    }

    #[test]
    fn caching_disabled_returns_plain_string_system() {
        let messages = vec![ChatMessage::system("You are helpful."), ChatMessage::User {
            content: UserContent::Text("hi".into()),
        }];

        let (system_value, out) = to_anthropic_messages(&messages, false);

        // System should be a plain string, not content-block array.
        let sys = system_value.expect("system should be present");
        assert!(sys.is_string(), "expected string, got: {sys:?}");
        assert_eq!(sys, "You are helpful.");

        // User message should NOT have cache_control.
        assert_eq!(out[0]["content"], "hi");
    }

    #[test]
    fn cache_retention_none_skips_cache_control() {
        let provider = AnthropicProvider {
            api_key: secrecy::Secret::new("test".into()),
            model: "claude-sonnet-4-5-20250929".into(),
            base_url: "https://api.anthropic.com".into(),
            client: crate::shared_http_client(),
            alias: None,
            reasoning_effort: None,
            cache_retention: moltis_config::CacheRetention::None,
        };
        assert!(!provider.caching_enabled());
    }

    #[test]
    fn cache_retention_short_enables_caching() {
        let provider = AnthropicProvider::new(
            secrecy::Secret::new("test".into()),
            "claude-sonnet-4-5-20250929".into(),
            "https://api.anthropic.com".into(),
        );
        assert!(provider.caching_enabled());
    }

    #[test]
    fn normalize_display_name_formats_alias_when_missing() {
        assert_eq!(
            normalize_display_name("claude-sonnet-4-6", None),
            "Claude Sonnet 4.6"
        );
        assert_eq!(
            normalize_display_name("claude-sonnet-4-5-20250929", None),
            "Claude Sonnet 4.5 2025-09-29"
        );
    }

    #[test]
    fn parse_models_payload_does_not_mark_recommendations() {
        let payload = serde_json::json!({
            "data": [
                {"id": "claude-opus-4-6", "display_name": "Claude Opus 4.6", "type": "model"},
                {"id": "claude-sonnet-4-6", "display_name": "Claude Sonnet 4.6", "type": "model"},
                {"id": "claude-haiku-4-5", "display_name": "Claude Haiku 4.5", "type": "model"},
                {"id": "claude-3-7-sonnet-20250219", "display_name": "Claude 3.7 Sonnet", "type": "model"}
            ]
        });

        let models = parse_models_payload(&payload);
        assert_eq!(models.len(), 4);
        assert!(!models[0].recommended);
        assert!(!models[1].recommended);
        assert!(!models[2].recommended);
        assert!(!models[3].recommended);
    }

    #[test]
    fn mark_recommended_models_marks_first_three_globally() {
        let mut models = vec![
            crate::DiscoveredModel::new("claude-opus-4-6", "Claude Opus 4.6"),
            crate::DiscoveredModel::new("claude-sonnet-4-6", "Claude Sonnet 4.6"),
            crate::DiscoveredModel::new("claude-haiku-4-5", "Claude Haiku 4.5"),
            crate::DiscoveredModel::new("claude-3-7-sonnet-20250219", "Claude 3.7 Sonnet"),
        ];

        mark_recommended_models(&mut models);

        assert!(models[0].recommended);
        assert!(models[1].recommended);
        assert!(models[2].recommended);
        assert!(!models[3].recommended);
    }
}
