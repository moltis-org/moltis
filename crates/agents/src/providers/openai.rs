use std::{
    collections::{HashMap, HashSet},
    pin::Pin,
    sync::mpsc,
    time::Duration,
};

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

const OPENAI_MODEL_EXCLUDED_FRAGMENTS: &[&str] = &[
    "audio",
    "realtime",
    "image",
    "search",
    "transcribe",
    "tts",
    "moderation",
    "embedding",
    "whisper",
    "deep-research",
];

const DEFAULT_OPENAI_MODELS: &[(&str, &str)] = &[
    ("gpt-5.2", "GPT-5.2"),
    ("gpt-5.1", "GPT-5.1"),
    ("gpt-5", "GPT-5"),
    ("gpt-5-mini", "GPT-5 Mini"),
    ("gpt-5-nano", "GPT-5 Nano"),
    ("gpt-4.1", "GPT-4.1"),
    ("gpt-4.1-mini", "GPT-4.1 Mini"),
    ("gpt-4.1-nano", "GPT-4.1 Nano"),
    ("gpt-4o", "GPT-4o"),
    ("gpt-4o-mini", "GPT-4o Mini"),
    ("gpt-4-turbo", "GPT-4 Turbo"),
    ("chatgpt-4o-latest", "ChatGPT-4o Latest"),
    ("o4-mini", "o4-mini"),
    ("o3", "o3"),
    ("o3-mini", "o3-mini"),
];

#[must_use]
pub fn default_model_catalog() -> Vec<(String, String)> {
    DEFAULT_OPENAI_MODELS
        .iter()
        .map(|(id, name)| (id.to_string(), name.to_string()))
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

fn is_reasoning_family_model_id(model_id: &str) -> bool {
    let mut chars = model_id.chars();
    matches!(chars.next(), Some('o'))
        && matches!(chars.next(), Some(second) if second.is_ascii_digit())
}

fn has_date_suffix(model_id: &str) -> bool {
    let parts: Vec<&str> = model_id.split('-').collect();
    if parts.len() < 3 {
        return false;
    }
    let year = parts[parts.len() - 3];
    let month = parts[parts.len() - 2];
    let day = parts[parts.len() - 1];
    year.len() == 4
        && month.len() == 2
        && day.len() == 2
        && year.chars().all(|ch| ch.is_ascii_digit())
        && month.chars().all(|ch| ch.is_ascii_digit())
        && day.chars().all(|ch| ch.is_ascii_digit())
}

fn is_supported_chat_model_id(model_id: &str) -> bool {
    let lower = model_id.to_ascii_lowercase();
    let starts_with_supported_family = lower.starts_with("gpt-")
        || lower.starts_with("chatgpt-")
        || is_reasoning_family_model_id(&lower);
    if !starts_with_supported_family {
        return false;
    }
    if has_date_suffix(&lower) {
        return false;
    }
    !OPENAI_MODEL_EXCLUDED_FRAGMENTS
        .iter()
        .any(|fragment| lower.contains(fragment))
}

fn parse_model_entry(entry: &serde_json::Value) -> Option<(String, String)> {
    let obj = entry.as_object()?;
    let model_id = obj
        .get("id")
        .or_else(|| obj.get("slug"))
        .or_else(|| obj.get("model"))
        .and_then(serde_json::Value::as_str)?;

    if !is_likely_model_id(model_id) || !is_supported_chat_model_id(model_id) {
        return None;
    }

    let display_name = obj
        .get("display_name")
        .or_else(|| obj.get("displayName"))
        .or_else(|| obj.get("name"))
        .or_else(|| obj.get("title"))
        .and_then(serde_json::Value::as_str);

    Some((
        model_id.to_string(),
        normalize_display_name(model_id, display_name),
    ))
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

fn parse_models_payload(value: &serde_json::Value) -> Vec<(String, String)> {
    let mut candidates = Vec::new();
    collect_candidate_arrays(value, &mut candidates);

    let mut models = Vec::new();
    let mut seen = HashSet::new();
    for entry in candidates {
        if let Some((id, display_name)) = parse_model_entry(entry)
            && seen.insert(id.clone())
        {
            models.push((id, display_name));
        }
    }
    models
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
) -> anyhow::Result<Vec<(String, String)>> {
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
        anyhow::bail!("openai models API returned no chat models");
    }
    Ok(models)
}

fn fetch_models_blocking(
    api_key: secrecy::Secret<String>,
    base_url: String,
) -> anyhow::Result<Vec<(String, String)>> {
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
) -> anyhow::Result<Vec<(String, String)>> {
    let models = fetch_models_blocking(api_key.clone(), base_url.to_string())?;
    debug!(model_count = models.len(), "loaded openai live models");
    Ok(models)
}

fn merge_with_fallback(
    discovered: Vec<(String, String)>,
    fallback: Vec<(String, String)>,
) -> Vec<(String, String)> {
    let mut discovered_by_id: HashMap<String, String> = discovered.into_iter().collect();
    let mut merged = Vec::new();

    for (id, fallback_display) in fallback {
        let display_name = discovered_by_id.remove(&id).unwrap_or(fallback_display);
        merged.push((id, display_name));
    }

    let mut remaining: Vec<(String, String)> = discovered_by_id.into_iter().collect();
    remaining.sort_by(|left, right| left.0.cmp(&right.0));
    merged.extend(remaining);
    merged
}

#[must_use]
pub fn available_models(
    api_key: &secrecy::Secret<String>,
    base_url: &str,
) -> Vec<(String, String)> {
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

    let merged = merge_with_fallback(discovered, fallback);
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
            warn!(status = %status, body = %body_text, "openai API error");
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
    fn parse_models_payload_filters_non_chat_and_snapshot_models() {
        let payload = serde_json::json!({
            "data": [
                { "id": "gpt-5.2" },
                { "id": "gpt-5.2-2025-12-11" },
                { "id": "gpt-image-1" },
                { "id": "gpt-4o-mini" },
                { "id": "o4-mini-deep-research" },
                { "id": "o3" }
            ]
        });

        let models = parse_models_payload(&payload);
        let ids: Vec<String> = models.into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, vec!["gpt-5.2", "gpt-4o-mini", "o3"]);
    }

    #[test]
    fn merge_with_fallback_preserves_fallback_order_then_appends_new_models() {
        let discovered = vec![
            ("gpt-5.2".to_string(), "GPT-5.2".to_string()),
            ("zeta-model".to_string(), "Zeta".to_string()),
            ("alpha-model".to_string(), "Alpha".to_string()),
        ];
        let fallback = vec![
            ("gpt-5.2".to_string(), "fallback".to_string()),
            ("gpt-4o".to_string(), "GPT-4o".to_string()),
        ];

        let merged = merge_with_fallback(discovered, fallback);
        let ids: Vec<String> = merged.into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, vec!["gpt-5.2", "gpt-4o", "alpha-model", "zeta-model"]);
    }

    #[test]
    fn default_catalog_includes_gpt_5_2() {
        let defaults = default_model_catalog();
        assert!(defaults.iter().any(|(id, _)| id == "gpt-5.2"));
    }
}
