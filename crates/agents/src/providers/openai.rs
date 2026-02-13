use std::{collections::HashSet, pin::Pin, sync::mpsc, time::Duration};

use {async_trait::async_trait, futures::StreamExt, secrecy::ExposeSecret, tokio_stream::Stream};

use tracing::{debug, trace, warn};

use {
    super::openai_compat::{
        SseLineResult, StreamingToolState, finalize_stream, parse_tool_calls,
        process_openai_sse_line, to_openai_tools,
    },
    crate::model::{ChatMessage, CompletionResponse, LlmProvider, StreamEvent, Usage},
};

pub struct OpenAiProvider {
    api_key: secrecy::Secret<String>,
    model: String,
    base_url: String,
    provider_name: String,
    client: reqwest::Client,
}

const OPENAI_MODELS_ENDPOINT_PATH: &str = "/models";

#[derive(Clone, Copy)]
struct ModelCatalogEntry {
    id: &'static str,
    display_name: &'static str,
}

impl ModelCatalogEntry {
    const fn new(id: &'static str, display_name: &'static str) -> Self {
        Self { id, display_name }
    }
}

const DEFAULT_OPENAI_MODELS: &[ModelCatalogEntry] = &[
    ModelCatalogEntry::new("gpt-5.2", "GPT-5.2"),
    ModelCatalogEntry::new("gpt-5.2-chat-latest", "GPT-5.2 Chat Latest"),
    ModelCatalogEntry::new("gpt-5-mini", "GPT-5 Mini"),
];

#[must_use]
pub fn default_model_catalog() -> Vec<super::DiscoveredModel> {
    DEFAULT_OPENAI_MODELS
        .iter()
        .map(|entry| super::DiscoveredModel::new(entry.id, entry.display_name))
        .collect()
}

fn title_case_chunk(chunk: &str) -> String {
    if chunk.is_empty() {
        return String::new();
    }
    let mut chars = chunk.chars();
    match chars.next() {
        Some(first) => {
            let mut out = String::new();
            out.push(first.to_ascii_uppercase());
            out.push_str(chars.as_str());
            out
        },
        None => String::new(),
    }
}

fn format_gpt_display_name(model_id: &str) -> String {
    let Some(rest) = model_id.strip_prefix("gpt-") else {
        return model_id.to_string();
    };
    let mut parts = rest.split('-');
    let Some(base) = parts.next() else {
        return "GPT".to_string();
    };
    let mut out = format!("GPT-{base}");
    for part in parts {
        out.push(' ');
        out.push_str(&title_case_chunk(part));
    }
    out
}

fn format_chatgpt_display_name(model_id: &str) -> String {
    let Some(rest) = model_id.strip_prefix("chatgpt-") else {
        return model_id.to_string();
    };
    let mut parts = rest.split('-');
    let Some(base) = parts.next() else {
        return "ChatGPT".to_string();
    };
    let mut out = format!("ChatGPT-{base}");
    for part in parts {
        out.push(' ');
        out.push_str(&title_case_chunk(part));
    }
    out
}

fn formatted_model_name(model_id: &str) -> String {
    if model_id.starts_with("gpt-") {
        return format_gpt_display_name(model_id);
    }
    if model_id.starts_with("chatgpt-") {
        return format_chatgpt_display_name(model_id);
    }
    model_id.to_string()
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

fn is_likely_model_id(model_id: &str) -> bool {
    if model_id.is_empty() || model_id.len() > 160 {
        return false;
    }
    if model_id.chars().any(char::is_whitespace) {
        return false;
    }
    model_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':'))
}

/// Delegates to the shared [`super::is_chat_capable_model`] for filtering
/// non-chat models during discovery.
fn is_chat_capable_model(model_id: &str) -> bool {
    super::is_chat_capable_model(model_id)
}

fn parse_model_entry(entry: &serde_json::Value) -> Option<super::DiscoveredModel> {
    let obj = entry.as_object()?;
    let model_id = obj
        .get("id")
        .or_else(|| obj.get("slug"))
        .or_else(|| obj.get("model"))
        .and_then(serde_json::Value::as_str)?;

    if !is_likely_model_id(model_id) {
        return None;
    }

    let display_name = obj
        .get("display_name")
        .or_else(|| obj.get("displayName"))
        .or_else(|| obj.get("name"))
        .or_else(|| obj.get("title"))
        .and_then(serde_json::Value::as_str);

    let created_at = obj.get("created").and_then(serde_json::Value::as_i64);

    Some(
        super::DiscoveredModel::new(model_id, normalize_display_name(model_id, display_name))
            .with_created_at(created_at),
    )
}

fn collect_candidate_arrays<'a>(
    value: &'a serde_json::Value,
    out: &mut Vec<&'a serde_json::Value>,
) {
    match value {
        serde_json::Value::Array(items) => out.extend(items),
        serde_json::Value::Object(map) => {
            for key in ["models", "data", "items", "results", "available"] {
                if let Some(nested) = map.get(key) {
                    collect_candidate_arrays(nested, out);
                }
            }
        },
        _ => {},
    }
}

fn parse_models_payload(value: &serde_json::Value) -> Vec<super::DiscoveredModel> {
    let mut candidates = Vec::new();
    collect_candidate_arrays(value, &mut candidates);

    let mut models = Vec::new();
    let mut seen = HashSet::new();
    for entry in candidates {
        if let Some(model) = parse_model_entry(entry)
            && is_chat_capable_model(&model.id)
            && seen.insert(model.id.clone())
        {
            models.push(model);
        }
    }

    // Sort by created_at descending (newest first). Models without a
    // timestamp are placed after those with one, preserving relative order.
    models.sort_by(|a, b| match (a.created_at, b.created_at) {
        (Some(a_ts), Some(b_ts)) => b_ts.cmp(&a_ts), // newest first
        (Some(_), None) => std::cmp::Ordering::Less, // timestamp before no-timestamp
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });

    models
}

fn is_chat_endpoint_unsupported_model_error(body_text: &str) -> bool {
    let lower = body_text.to_ascii_lowercase();
    lower.contains("not a chat model")
        || lower.contains("does not support chat")
        || lower.contains("only supported in v1/responses")
        || lower.contains("not supported in the v1/chat/completions endpoint")
        || lower.contains("input content or output modality contain audio")
        || lower.contains("requires audio")
}

fn should_warn_on_api_error(status: reqwest::StatusCode, body_text: &str) -> bool {
    if is_chat_endpoint_unsupported_model_error(body_text) {
        return false;
    }
    !matches!(status.as_u16(), 404)
}

fn models_endpoint(base_url: &str) -> String {
    format!(
        "{}{OPENAI_MODELS_ENDPOINT_PATH}",
        base_url.trim_end_matches('/')
    )
}

async fn fetch_models_from_api(
    api_key: secrecy::Secret<String>,
    base_url: String,
) -> anyhow::Result<Vec<super::DiscoveredModel>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()?;
    let response = client
        .get(models_endpoint(&base_url))
        .header(
            "Authorization",
            format!("Bearer {}", api_key.expose_secret()),
        )
        .header("Accept", "application/json")
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        anyhow::bail!("openai models API error HTTP {status}");
    }
    let payload: serde_json::Value = serde_json::from_str(&body)?;
    let models = parse_models_payload(&payload);
    if models.is_empty() {
        anyhow::bail!("openai models API returned no models");
    }
    Ok(models)
}

fn fetch_models_blocking(
    api_key: secrecy::Secret<String>,
    base_url: String,
) -> anyhow::Result<Vec<super::DiscoveredModel>> {
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(anyhow::Error::from)
            .and_then(|rt| rt.block_on(fetch_models_from_api(api_key, base_url)));
        let _ = tx.send(result);
    });
    rx.recv()
        .map_err(|err| anyhow::anyhow!("openai model discovery worker failed: {err}"))?
}

pub fn live_models(
    api_key: &secrecy::Secret<String>,
    base_url: &str,
) -> anyhow::Result<Vec<super::DiscoveredModel>> {
    let models = fetch_models_blocking(api_key.clone(), base_url.to_string())?;
    debug!(model_count = models.len(), "loaded live models");
    Ok(models)
}

#[must_use]
pub fn available_models(
    api_key: &secrecy::Secret<String>,
    base_url: &str,
) -> Vec<super::DiscoveredModel> {
    let fallback = default_model_catalog();
    if cfg!(test) {
        return fallback;
    }

    let discovered = match live_models(api_key, base_url) {
        Ok(models) => models,
        Err(err) => {
            warn!(error = %err, base_url = %base_url, "failed to fetch openai models, using fallback catalog");
            return fallback;
        },
    };

    let merged = super::merge_discovered_with_fallback_catalog(discovered, fallback);
    debug!(model_count = merged.len(), "loaded openai models catalog");
    merged
}

impl OpenAiProvider {
    pub fn new(api_key: secrecy::Secret<String>, model: String, base_url: String) -> Self {
        Self {
            api_key,
            model,
            base_url,
            provider_name: "openai".into(),
            client: reqwest::Client::new(),
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
            client: reqwest::Client::new(),
        }
    }

    fn requires_reasoning_content_on_tool_messages(&self) -> bool {
        self.provider_name.eq_ignore_ascii_case("moonshot")
            || self.base_url.contains("moonshot.ai")
            || self.base_url.contains("moonshot.cn")
    }

    fn serialize_messages_for_request(&self, messages: &[ChatMessage]) -> Vec<serde_json::Value> {
        let needs_reasoning_content = self.requires_reasoning_content_on_tool_messages();
        messages
            .iter()
            .map(|message| {
                let mut value = message.to_openai_value();

                if !needs_reasoning_content {
                    return value;
                }

                let is_assistant =
                    value.get("role").and_then(serde_json::Value::as_str) == Some("assistant");
                let has_tool_calls = value
                    .get("tool_calls")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|calls| !calls.is_empty());

                if !is_assistant || !has_tool_calls {
                    return value;
                }

                let reasoning_content = value
                    .get("content")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();

                if value.get("content").is_none() {
                    value["content"] = serde_json::Value::String(String::new());
                }

                if value.get("reasoning_content").is_none() {
                    value["reasoning_content"] = serde_json::Value::String(reasoning_content);
                }

                value
            })
            .collect()
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &str {
        &self.provider_name
    }

    fn id(&self) -> &str {
        &self.model
    }

    fn supports_tools(&self) -> bool {
        super::supports_tools_for_model(&self.model)
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
        let openai_messages = self.serialize_messages_for_request(messages);
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": openai_messages,
        });

        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(to_openai_tools(tools));
        }

        debug!(
            model = %self.model,
            messages_count = messages.len(),
            tools_count = tools.len(),
            "openai complete request"
        );
        trace!(body = %serde_json::to_string(&body).unwrap_or_default(), "openai request body");

        let http_resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header(
                "Authorization",
                format!("Bearer {}", self.api_key.expose_secret()),
            )
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = http_resp.status();
        if !status.is_success() {
            let body_text = http_resp.text().await.unwrap_or_default();
            if should_warn_on_api_error(status, &body_text) {
                warn!(
                    status = %status,
                    model = %self.model,
                    provider = %self.provider_name,
                    body = %body_text,
                    "openai API error"
                );
            } else {
                debug!(
                    status = %status,
                    model = %self.model,
                    provider = %self.provider_name,
                    "openai model unsupported for chat/completions endpoint"
                );
            }
            anyhow::bail!("OpenAI API error HTTP {status}: {body_text}");
        }

        let resp = http_resp.json::<serde_json::Value>().await?;
        trace!(response = %resp, "openai raw response");

        let message = &resp["choices"][0]["message"];

        let text = message["content"].as_str().map(|s| s.to_string());
        let tool_calls = parse_tool_calls(message);

        let usage = Usage {
            input_tokens: resp["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            output_tokens: resp["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32,
            cache_read_tokens: resp["usage"]["prompt_tokens_details"]["cached_tokens"]
                .as_u64()
                .unwrap_or(0) as u32,
            ..Default::default()
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
            let openai_messages = self.serialize_messages_for_request(&messages);
            let mut body = serde_json::json!({
                "model": self.model,
                "messages": openai_messages,
                "stream": true,
                "stream_options": { "include_usage": true },
            });

            if !tools.is_empty() {
                body["tools"] = serde_json::Value::Array(to_openai_tools(&tools));
            }

            debug!(
                model = %self.model,
                messages_count = openai_messages.len(),
                tools_count = tools.len(),
                "openai stream_with_tools request"
            );
            trace!(body = %serde_json::to_string(&body).unwrap_or_default(), "openai stream request body");

            let resp = match self
                .client
                .post(format!("{}/chat/completions", self.base_url))
                .header("Authorization", format!("Bearer {}", self.api_key.expose_secret()))
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
            let mut state = StreamingToolState::default();

            while let Some(chunk) = byte_stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        yield StreamEvent::Error(e.to_string());
                        return;
                    }
                };
                buf.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(pos) = buf.find('\n') {
                    let line = buf[..pos].trim().to_string();
                    buf = buf[pos + 1..].to_string();

                    if line.is_empty() {
                        continue;
                    }

                    let Some(data) = line.strip_prefix("data: ") else {
                        continue;
                    };

                    match process_openai_sse_line(data, &mut state) {
                        SseLineResult::Done => {
                            for event in finalize_stream(&state) {
                                yield event;
                            }
                            return;
                        }
                        SseLineResult::Events(events) => {
                            for event in events {
                                yield event;
                            }
                        }
                        SseLineResult::Skip => {}
                    }
                }
            }
        })
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use {
        axum::{Router, extract::Request, routing::post},
        secrecy::Secret,
        tokio_stream::StreamExt,
    };

    use crate::model::{ChatMessage, ToolCall};

    use super::*;

    #[derive(Default, Clone)]
    struct CapturedRequest {
        body: Option<serde_json::Value>,
    }

    /// Start a mock SSE server that captures the request body and returns
    /// the given SSE payload verbatim.
    async fn start_sse_mock(sse_payload: String) -> (String, Arc<Mutex<Vec<CapturedRequest>>>) {
        let captured: Arc<Mutex<Vec<CapturedRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_clone = captured.clone();

        let app = Router::new().route(
            "/chat/completions",
            post(move |req: Request| {
                let cap = captured_clone.clone();
                let payload = sse_payload.clone();
                async move {
                    let body_bytes = axum::body::to_bytes(req.into_body(), 1024 * 1024)
                        .await
                        .unwrap_or_default();
                    let body: Option<serde_json::Value> = serde_json::from_slice(&body_bytes).ok();
                    cap.lock().unwrap().push(CapturedRequest { body });

                    axum::response::Response::builder()
                        .header("content-type", "text/event-stream")
                        .body(axum::body::Body::from(payload))
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

    fn test_provider(base_url: &str) -> OpenAiProvider {
        OpenAiProvider::new(
            Secret::new("test-key".to_string()),
            "gpt-4o".to_string(),
            base_url.to_string(),
        )
    }

    fn sample_tools() -> Vec<serde_json::Value> {
        vec![serde_json::json!({
            "name": "create_skill",
            "description": "Create a new skill",
            "parameters": {
                "type": "object",
                "required": ["name", "content"],
                "properties": {
                    "name": {"type": "string"},
                    "content": {"type": "string"}
                }
            }
        })]
    }

    #[test]
    fn moonshot_serialization_includes_reasoning_content_for_tool_messages() {
        let provider = OpenAiProvider::new_with_name(
            Secret::new("test-key".to_string()),
            "kimi-k2.5".to_string(),
            "https://api.moonshot.ai/v1".to_string(),
            "moonshot".to_string(),
        );
        let messages = vec![ChatMessage::assistant_with_tools(None, vec![ToolCall {
            id: "call_1".into(),
            name: "exec".into(),
            arguments: serde_json::json!({ "command": "uname -a" }),
        }])];

        let serialized = provider.serialize_messages_for_request(&messages);
        assert_eq!(serialized.len(), 1);
        assert_eq!(serialized[0]["role"], "assistant");
        assert_eq!(serialized[0]["content"], "");
        assert_eq!(serialized[0]["reasoning_content"], "");
    }

    #[test]
    fn non_moonshot_serialization_does_not_add_reasoning_content() {
        let provider = OpenAiProvider::new(
            Secret::new("test-key".to_string()),
            "gpt-4o".to_string(),
            "https://api.openai.com/v1".to_string(),
        );
        let messages = vec![ChatMessage::assistant_with_tools(None, vec![ToolCall {
            id: "call_1".into(),
            name: "exec".into(),
            arguments: serde_json::json!({ "command": "uname -a" }),
        }])];

        let serialized = provider.serialize_messages_for_request(&messages);
        assert_eq!(serialized.len(), 1);
        assert!(serialized[0].get("reasoning_content").is_none());
    }

    #[tokio::test]
    async fn moonshot_stream_request_includes_reasoning_content_on_tool_history() {
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\n\
                   data: [DONE]\n\n";
        let (base_url, captured) = start_sse_mock(sse.to_string()).await;
        let provider = OpenAiProvider::new_with_name(
            Secret::new("test-key".to_string()),
            "kimi-k2.5".to_string(),
            base_url,
            "moonshot".to_string(),
        );
        let messages = vec![
            ChatMessage::user("run uname"),
            ChatMessage::assistant_with_tools(None, vec![ToolCall {
                id: "exec:0".into(),
                name: "exec".into(),
                arguments: serde_json::json!({ "command": "uname -a" }),
            }]),
            ChatMessage::tool("exec:0", "Linux host 6.0"),
        ];

        let mut stream = provider.stream_with_tools(messages, sample_tools());
        while stream.next().await.is_some() {}

        let reqs = captured.lock().unwrap();
        assert_eq!(reqs.len(), 1);
        let body = reqs[0].body.as_ref().expect("request should have a body");
        let history = body["messages"]
            .as_array()
            .expect("messages should be an array");
        assert_eq!(history[1]["role"], "assistant");
        assert_eq!(history[1]["content"], "");
        assert_eq!(history[1]["reasoning_content"], "");
        assert!(history[1]["tool_calls"].is_array());
    }

    // ── Regression: stream_with_tools must send tools in the API body ────

    #[tokio::test]
    async fn stream_with_tools_sends_tools_in_request_body() {
        // This is the core regression test: before the fix,
        // stream_with_tools() fell back to stream() which never
        // included tools in the request body.
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":null}]}\n\n\
                   data: [DONE]\n\n";
        let (base_url, captured) = start_sse_mock(sse.to_string()).await;
        let provider = test_provider(&base_url);
        let tools = sample_tools();

        let mut stream = provider.stream_with_tools(vec![ChatMessage::user("test")], tools);
        while stream.next().await.is_some() {}

        let reqs = captured.lock().unwrap();
        assert_eq!(reqs.len(), 1);
        let body = reqs[0].body.as_ref().expect("request should have a body");

        // The body MUST contain the "tools" key with our tool in it.
        let tools_arr = body["tools"]
            .as_array()
            .expect("body must contain 'tools' array");
        assert_eq!(tools_arr.len(), 1);
        assert_eq!(tools_arr[0]["type"], "function");
        assert_eq!(tools_arr[0]["function"]["name"], "create_skill");
    }

    #[tokio::test]
    async fn stream_with_empty_tools_omits_tools_key() {
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":null}]}\n\n\
                   data: [DONE]\n\n";
        let (base_url, captured) = start_sse_mock(sse.to_string()).await;
        let provider = test_provider(&base_url);

        let mut stream = provider.stream_with_tools(vec![ChatMessage::user("test")], vec![]);
        while stream.next().await.is_some() {}

        let reqs = captured.lock().unwrap();
        let body = reqs[0].body.as_ref().unwrap();
        assert!(
            body.get("tools").is_none(),
            "tools key should be absent when no tools provided"
        );
    }

    // ── Regression: stream_with_tools must parse tool_call streaming events ──

    #[tokio::test]
    async fn stream_with_tools_parses_single_tool_call() {
        // Simulates OpenAI streaming a single tool call across multiple SSE chunks.
        let sse = concat!(
            // First chunk: tool call start (id + function name)
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_abc\",\"function\":{\"name\":\"create_skill\",\"arguments\":\"\"}}]},\"finish_reason\":null}]}\n\n",
            // Second chunk: argument delta
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"name\\\"\"}}]},\"finish_reason\":null}]}\n\n",
            // Third chunk: more argument delta
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\": \\\"weather\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
            // Fourth chunk: finish_reason = tool_calls
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            // Usage
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":50,\"completion_tokens\":20}}\n\n",
            "data: [DONE]\n\n",
        );

        let (base_url, _) = start_sse_mock(sse.to_string()).await;
        let provider = test_provider(&base_url);

        let mut stream =
            provider.stream_with_tools(vec![ChatMessage::user("test")], sample_tools());

        let mut events = Vec::new();
        while let Some(ev) = stream.next().await {
            events.push(ev);
        }

        // Must contain ToolCallStart
        let starts: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, StreamEvent::ToolCallStart { .. }))
            .collect();
        assert_eq!(starts.len(), 1, "expected exactly one ToolCallStart");
        match &starts[0] {
            StreamEvent::ToolCallStart { id, name, index } => {
                assert_eq!(id, "call_abc");
                assert_eq!(name, "create_skill");
                assert_eq!(*index, 0);
            },
            _ => unreachable!(),
        }

        // Must contain ToolCallArgumentsDelta events
        let arg_deltas: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, StreamEvent::ToolCallArgumentsDelta { .. }))
            .collect();
        assert!(
            arg_deltas.len() >= 2,
            "expected at least 2 argument deltas, got {}",
            arg_deltas.len()
        );

        // Must contain ToolCallComplete
        let completes: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, StreamEvent::ToolCallComplete { .. }))
            .collect();
        assert_eq!(completes.len(), 1, "expected exactly one ToolCallComplete");

        // Must end with Done including usage
        match events.last().unwrap() {
            StreamEvent::Done(usage) => {
                assert_eq!(usage.input_tokens, 50);
                assert_eq!(usage.output_tokens, 20);
            },
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn stream_with_tools_parses_multiple_tool_calls() {
        // Two parallel tool calls in one response.
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"tool_a\",\"arguments\":\"\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"id\":\"call_2\",\"function\":{\"name\":\"tool_b\",\"arguments\":\"\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"x\\\":1}\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"function\":{\"arguments\":\"{\\\"y\\\":2}\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n",
        );

        let (base_url, _) = start_sse_mock(sse.to_string()).await;
        let provider = test_provider(&base_url);

        let mut stream =
            provider.stream_with_tools(vec![ChatMessage::user("test")], sample_tools());

        let mut events = Vec::new();
        while let Some(ev) = stream.next().await {
            events.push(ev);
        }

        let starts: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::ToolCallStart { id, name, index } => {
                    Some((id.clone(), name.clone(), *index))
                },
                _ => None,
            })
            .collect();
        assert_eq!(starts.len(), 2);
        assert_eq!(starts[0], ("call_1".into(), "tool_a".into(), 0));
        assert_eq!(starts[1], ("call_2".into(), "tool_b".into(), 1));

        let completes: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, StreamEvent::ToolCallComplete { .. }))
            .collect();
        assert_eq!(completes.len(), 2, "expected 2 ToolCallComplete events");
    }

    #[tokio::test]
    async fn stream_with_tools_text_and_tool_call_mixed() {
        // Some providers emit text content before switching to tool calls.
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Let me \"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"help.\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_x\",\"function\":{\"name\":\"my_tool\",\"arguments\":\"{}\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n",
        );

        let (base_url, _) = start_sse_mock(sse.to_string()).await;
        let provider = test_provider(&base_url);

        let mut stream =
            provider.stream_with_tools(vec![ChatMessage::user("test")], sample_tools());

        let mut text_deltas = Vec::new();
        let mut tool_starts = Vec::new();
        while let Some(ev) = stream.next().await {
            match ev {
                StreamEvent::Delta(t) => text_deltas.push(t),
                StreamEvent::ToolCallStart { name, .. } => tool_starts.push(name),
                _ => {},
            }
        }

        assert_eq!(text_deltas.join(""), "Let me help.");
        assert_eq!(tool_starts, vec!["my_tool"]);
    }

    #[test]
    fn parse_models_payload_keeps_chat_capable_models() {
        let payload = serde_json::json!({
            "data": [
                { "id": "gpt-5.2" },
                { "id": "gpt-5.2-2025-12-11" },
                { "id": "gpt-image-1" },
                { "id": "gpt-image-1-mini" },
                { "id": "chatgpt-image-latest" },
                { "id": "gpt-audio" },
                { "id": "o4-mini-deep-research" },
                { "id": "kimi-k2.5" },
                { "id": "moonshot-v1-8k" },
                { "id": "dall-e-3" },
                { "id": "tts-1-hd" },
                { "id": "gpt-4o-mini-tts" },
                { "id": "whisper-1" },
                { "id": "text-embedding-3-large" },
                { "id": "omni-moderation-latest" },
                { "id": "gpt-4o-audio-preview" },
                { "id": "gpt-4o-realtime-preview" },
                { "id": "gpt-4o-mini-transcribe" },
                { "id": "has spaces" },
                { "id": "" }
            ]
        });

        let models = parse_models_payload(&payload);
        let ids: Vec<String> = models.into_iter().map(|m| m.id).collect();
        // Only chat-capable models pass; non-chat (image, TTS, whisper,
        // embedding, moderation, audio, realtime, transcribe) are excluded.
        assert_eq!(ids, vec![
            "gpt-5.2",
            "gpt-5.2-2025-12-11",
            "o4-mini-deep-research",
            "kimi-k2.5",
            "moonshot-v1-8k",
        ]);
    }

    #[test]
    fn parse_models_payload_sorts_by_created_at_descending() {
        let payload = serde_json::json!({
            "data": [
                { "id": "gpt-4o-mini", "created": 1000 },
                { "id": "gpt-5.2", "created": 3000 },
                { "id": "o3", "created": 2000 },
                { "id": "o1" }
            ]
        });

        let models = parse_models_payload(&payload);
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        // Newest first (3000, 2000, 1000), then no-timestamp last
        assert_eq!(ids, vec!["gpt-5.2", "o3", "gpt-4o-mini", "o1"]);
        assert_eq!(models[0].created_at, Some(3000));
        assert_eq!(models[3].created_at, None);
    }

    #[test]
    fn parse_model_entry_extracts_created_at() {
        let entry = serde_json::json!({ "id": "gpt-5.2", "created": 1700000000 });
        let model = parse_model_entry(&entry).unwrap();
        assert_eq!(model.id, "gpt-5.2");
        assert_eq!(model.created_at, Some(1700000000));
    }

    #[test]
    fn parse_model_entry_without_created_at() {
        let entry = serde_json::json!({ "id": "gpt-5.2" });
        let model = parse_model_entry(&entry).unwrap();
        assert_eq!(model.created_at, None);
    }

    #[test]
    fn merge_with_fallback_uses_discovered_models_when_live_fetch_succeeds() {
        use crate::providers::DiscoveredModel;
        let discovered = vec![
            DiscoveredModel::new("gpt-5.2", "GPT-5.2"),
            DiscoveredModel::new("zeta-model", "Zeta"),
            DiscoveredModel::new("alpha-model", "Alpha"),
        ];
        let fallback = vec![
            DiscoveredModel::new("gpt-5.2", "fallback"),
            DiscoveredModel::new("gpt-4o", "GPT-4o"),
        ];

        let merged = crate::providers::merge_discovered_with_fallback_catalog(discovered, fallback);
        let ids: Vec<String> = merged.into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["gpt-5.2", "zeta-model", "alpha-model"]);
    }

    #[test]
    fn merge_with_fallback_uses_fallback_when_discovery_is_empty() {
        use crate::providers::DiscoveredModel;
        let merged = crate::providers::merge_discovered_with_fallback_catalog(Vec::new(), vec![
            DiscoveredModel::new("gpt-5.2", "GPT-5.2"),
            DiscoveredModel::new("gpt-5-mini", "GPT-5 Mini"),
        ]);
        let ids: Vec<String> = merged.into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["gpt-5.2", "gpt-5-mini"]);
    }

    #[test]
    fn default_catalog_includes_gpt_5_2() {
        let defaults = default_model_catalog();
        assert!(defaults.iter().any(|m| m.id == "gpt-5.2"));
    }

    #[test]
    fn default_catalog_excludes_stale_gpt_5_3() {
        let defaults = default_model_catalog();
        assert!(!defaults.iter().any(|m| m.id == "gpt-5.3"));
    }

    #[test]
    fn default_catalog_excludes_legacy_openai_fallback_entries() {
        let defaults = default_model_catalog();
        assert!(!defaults.iter().any(|m| m.id == "chatgpt-4o-latest"));
        assert!(!defaults.iter().any(|m| m.id == "gpt-4-turbo"));
    }

    #[test]
    fn should_warn_on_api_error_suppresses_expected_chat_endpoint_mismatches() {
        let body = r#"{"error":{"message":"This model is only supported in v1/responses and not in v1/chat/completions."}}"#;
        assert!(!should_warn_on_api_error(
            reqwest::StatusCode::NOT_FOUND,
            body
        ));

        let body = r#"{"error":{"message":"This is not a chat model and thus not supported in the v1/chat/completions endpoint."}}"#;
        assert!(!should_warn_on_api_error(
            reqwest::StatusCode::NOT_FOUND,
            body
        ));

        let body = r#"{"error":{"message":"does not support chat"}}"#;
        assert!(!should_warn_on_api_error(
            reqwest::StatusCode::BAD_REQUEST,
            body
        ));
    }

    #[test]
    fn should_warn_on_api_error_keeps_real_failures_as_warnings() {
        let body = r#"{"error":{"message":"invalid api key"}}"#;
        assert!(should_warn_on_api_error(
            reqwest::StatusCode::UNAUTHORIZED,
            body
        ));
        assert!(should_warn_on_api_error(
            reqwest::StatusCode::BAD_REQUEST,
            body
        ));
    }

    #[test]
    fn should_warn_on_api_error_suppresses_audio_model_errors() {
        // Audio models return 400 with this message when probed via
        // /v1/chat/completions. This should not produce a WARN.
        let body = r#"{"error":{"message":"This model requires that either input content or output modality contain audio.","type":"invalid_request_error","param":"model","code":"invalid_value"}}"#;
        assert!(!should_warn_on_api_error(
            reqwest::StatusCode::BAD_REQUEST,
            body
        ));
    }

    #[test]
    fn is_chat_capable_model_filters_non_chat_families() {
        // Chat-capable models pass
        assert!(is_chat_capable_model("gpt-5.2"));
        assert!(is_chat_capable_model("gpt-4o-mini"));
        assert!(is_chat_capable_model("o3"));
        assert!(is_chat_capable_model("o4-mini"));
        assert!(is_chat_capable_model("chatgpt-4o-latest"));
        assert!(is_chat_capable_model("babbage-002"));
        assert!(is_chat_capable_model("davinci-002"));

        // Non-chat models are rejected
        assert!(!is_chat_capable_model("dall-e-3"));
        assert!(!is_chat_capable_model("dall-e-2"));
        assert!(!is_chat_capable_model("gpt-image-1"));
        assert!(!is_chat_capable_model("gpt-image-1-mini"));
        assert!(!is_chat_capable_model("chatgpt-image-latest"));
        assert!(!is_chat_capable_model("gpt-audio"));
        assert!(!is_chat_capable_model("tts-1"));
        assert!(!is_chat_capable_model("tts-1-hd"));
        assert!(!is_chat_capable_model("gpt-4o-mini-tts"));
        assert!(!is_chat_capable_model("gpt-4o-mini-tts-2025-12-15"));
        assert!(!is_chat_capable_model("whisper-1"));
        assert!(!is_chat_capable_model("text-embedding-3-large"));
        assert!(!is_chat_capable_model("text-embedding-ada-002"));
        assert!(!is_chat_capable_model("omni-moderation-latest"));
        assert!(!is_chat_capable_model("omni-moderation-2024-09-26"));
        assert!(!is_chat_capable_model("moderation-latest"));
        assert!(!is_chat_capable_model("sora"));

        // Audio/realtime/transcribe variants
        assert!(!is_chat_capable_model("gpt-4o-audio-preview"));
        assert!(!is_chat_capable_model("gpt-4o-mini-audio-preview"));
        assert!(!is_chat_capable_model("gpt-4o-realtime-preview"));
        assert!(!is_chat_capable_model("gpt-4o-mini-realtime"));
        assert!(!is_chat_capable_model("gpt-4o-mini-transcribe"));
    }
}
