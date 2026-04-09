use std::{
    collections::{HashMap, HashSet},
    pin::Pin,
    sync::mpsc,
    time::Duration,
};

use {
    async_trait::async_trait,
    futures::{SinkExt, StreamExt},
    moltis_config::schema::{ProviderStreamTransport, WireApi},
    secrecy::ExposeSecret,
    tokio_stream::Stream,
    tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest, http::HeaderValue},
};

use tracing::{debug, trace, warn};

use {
    super::openai_compat::{
        ResponsesStreamState, SseLineResult, StreamingToolState, finalize_responses_stream,
        finalize_stream, parse_openai_compat_usage, parse_openai_compat_usage_from_payload,
        parse_tool_calls, process_openai_sse_line, process_responses_sse_line,
        responses_output_index, split_responses_instructions_and_input, strip_think_tags,
        to_openai_tools, to_responses_api_tools,
    },
    moltis_agents::model::{
        ChatMessage, CompletionResponse, LlmProvider, ModelMetadata, StreamEvent, Usage,
    },
};

pub struct OpenAiProvider {
    api_key: secrecy::Secret<String>,
    model: String,
    base_url: String,
    provider_name: String,
    client: &'static reqwest::Client,
    stream_transport: ProviderStreamTransport,
    wire_api: WireApi,
    metadata_cache: tokio::sync::OnceCell<ModelMetadata>,
    tool_mode_override: Option<moltis_config::ToolMode>,
    /// Optional reasoning effort level for o-series models.
    reasoning_effort: Option<moltis_agents::model::ReasoningEffort>,
    /// Prompt cache retention policy (used for OpenRouter Anthropic passthrough).
    cache_retention: moltis_config::CacheRetention,
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
        .map(|entry| {
            super::DiscoveredModel::new(entry.id, entry.display_name)
                .with_recommended(is_recommended_openai_model(entry.id))
        })
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
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':' | '/'))
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

    let recommended = is_recommended_openai_model(model_id);
    Some(
        super::DiscoveredModel::new(model_id, normalize_display_name(model_id, display_name))
            .with_created_at(created_at)
            .with_recommended(recommended),
    )
}

/// Known OpenAI flagship model IDs (latest generation, no date suffix).
/// These are the models most users care about.
fn is_recommended_openai_model(model_id: &str) -> bool {
    matches!(
        model_id,
        "gpt-5.4" | "gpt-5.4-mini" | "gpt-5.4-pro" | "o4-mini" | "o3"
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

const OPENAI_MAX_TOOL_CALL_ID_LEN: usize = 40;

fn short_stable_hash(value: &str) -> String {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn base_openai_tool_call_id(raw: &str) -> String {
    let mut cleaned: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect();

    if cleaned.is_empty() {
        cleaned = "call".to_string();
    }

    if cleaned.len() <= OPENAI_MAX_TOOL_CALL_ID_LEN {
        return cleaned;
    }

    let hash = short_stable_hash(raw);
    let keep = OPENAI_MAX_TOOL_CALL_ID_LEN.saturating_sub(hash.len() + 1);
    cleaned.truncate(keep);
    if cleaned.is_empty() {
        return format!("call-{hash}");
    }
    format!("{cleaned}-{hash}")
}

fn disambiguate_tool_call_id(base: &str, nonce: usize) -> String {
    let suffix = format!("-{nonce}");
    let keep = OPENAI_MAX_TOOL_CALL_ID_LEN.saturating_sub(suffix.len());

    let mut value = base.to_string();
    if value.len() > keep {
        value.truncate(keep);
    }
    if value.is_empty() {
        value = "call".to_string();
        if value.len() > keep {
            value.truncate(keep);
        }
    }
    format!("{value}{suffix}")
}

fn assign_openai_tool_call_id(
    raw: &str,
    remapped_tool_call_ids: &mut HashMap<String, String>,
    used_tool_call_ids: &mut HashSet<String>,
) -> String {
    if let Some(existing) = remapped_tool_call_ids.get(raw) {
        return existing.clone();
    }

    let base = base_openai_tool_call_id(raw);
    let mut candidate = base.clone();
    let mut nonce = 1usize;
    while used_tool_call_ids.contains(&candidate) {
        candidate = disambiguate_tool_call_id(&base, nonce);
        nonce = nonce.saturating_add(1);
    }

    used_tool_call_ids.insert(candidate.clone());
    remapped_tool_call_ids.insert(raw.to_string(), candidate.clone());
    candidate
}

fn models_endpoint(base_url: &str) -> String {
    format!(
        "{}{OPENAI_MODELS_ENDPOINT_PATH}",
        base_url.trim_end_matches('/')
    )
}

/// Resolve the output index from a Responses API WebSocket streaming event.
///
/// The Responses API includes `output_index` on most events. Falls back to
/// `item_index` / `index` for robustness, then to `fallback`.
pub async fn fetch_models_from_api(
    api_key: secrecy::Secret<String>,
    base_url: String,
) -> anyhow::Result<Vec<super::DiscoveredModel>> {
    let client = crate::shared_http_client();
    let response = client
        .get(models_endpoint(&base_url))
        .timeout(Duration::from_secs(15))
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

/// Spawn model discovery in a background thread and return the receiver
/// immediately, without blocking. Call `.recv()` later to collect the result.
pub fn start_model_discovery(
    api_key: secrecy::Secret<String>,
    base_url: String,
) -> mpsc::Receiver<anyhow::Result<Vec<super::DiscoveredModel>>> {
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

fn fetch_models_blocking(
    api_key: secrecy::Secret<String>,
    base_url: String,
) -> anyhow::Result<Vec<super::DiscoveredModel>> {
    start_model_discovery(api_key, base_url)
        .recv()
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
            client: crate::shared_http_client(),
            stream_transport: ProviderStreamTransport::Sse,
            wire_api: WireApi::ChatCompletions,
            metadata_cache: tokio::sync::OnceCell::new(),
            tool_mode_override: None,
            reasoning_effort: None,
            cache_retention: moltis_config::CacheRetention::Short,
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

    /// Return the reasoning effort string if configured.
    fn reasoning_effort_str(&self) -> Option<&'static str> {
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
    fn apply_reasoning_effort_chat(&self, body: &mut serde_json::Value) {
        if let Some(effort) = self.reasoning_effort_str() {
            body["reasoning_effort"] = serde_json::json!(effort);
        }
    }

    /// Apply `reasoning_effort` for the **Responses** API (used by
    /// `stream_with_tools_websocket()`).
    ///
    /// Format: `"reasoning": { "effort": "high" }` (nested object).
    fn apply_reasoning_effort_responses(&self, body: &mut serde_json::Value) {
        if let Some(effort) = self.reasoning_effort_str() {
            body["reasoning"] = serde_json::json!({ "effort": effort });
        }
    }

    fn apply_probe_output_cap_chat(&self, body: &mut serde_json::Value) {
        let raw = super::raw_model_id(&self.model).to_ascii_lowercase();
        let capability = raw.rsplit('/').next().unwrap_or(raw.as_str());
        let uses_max_completion_tokens = capability.starts_with("gpt-5")
            || capability.starts_with("o1")
            || capability.starts_with("o3")
            || capability.starts_with("o4");
        if uses_max_completion_tokens {
            body["max_completion_tokens"] = serde_json::json!(1);
        } else {
            body["max_tokens"] = serde_json::json!(1);
        }
    }

    async fn probe_chat_completions(&self) -> anyhow::Result<()> {
        let messages = vec![ChatMessage::user("ping")];
        let mut openai_messages = self.serialize_messages_for_request(&messages);
        self.apply_openrouter_cache_control(&mut openai_messages);
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": openai_messages,
        });
        // Probes only answer "can this model respond at all?".
        // Keep them cheap instead of mirroring full reasoning budgets.
        self.apply_probe_output_cap_chat(&mut body);

        debug!(model = %self.model, "openai probe request");
        trace!(body = %serde_json::to_string(&body).unwrap_or_default(), "openai probe request body");

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
            let retry_after_ms = super::retry_after_ms_from_headers(http_resp.headers());
            let body_text = http_resp.text().await.unwrap_or_default();
            if should_warn_on_api_error(status, &body_text) {
                warn!(
                    status = %status,
                    model = %self.model,
                    provider = %self.provider_name,
                    body = %body_text,
                    "openai probe API error"
                );
            } else {
                debug!(
                    status = %status,
                    model = %self.model,
                    provider = %self.provider_name,
                    "openai probe model unsupported for chat/completions endpoint"
                );
            }
            anyhow::bail!(
                "{}",
                super::with_retry_after_marker(
                    format!("OpenAI API error HTTP {status}: {body_text}"),
                    retry_after_ms,
                )
            );
        }

        Ok(())
    }

    async fn probe_responses(&self) -> anyhow::Result<()> {
        let messages = vec![ChatMessage::user("ping")];
        let (instructions, input) = split_responses_instructions_and_input(messages);
        let mut body = serde_json::json!({
            "model": self.model,
            "input": input,
            "max_output_tokens": 1,
        });

        if let Some(instructions) = instructions {
            body["instructions"] = serde_json::Value::String(instructions);
        }

        self.apply_reasoning_effort_responses(&mut body);

        debug!(model = %self.model, "openai responses probe request");
        trace!(body = %serde_json::to_string(&body).unwrap_or_default(), "openai responses probe request body");

        let url = self.responses_sse_url();
        let http_resp = self
            .client
            .post(&url)
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
            let retry_after_ms = super::retry_after_ms_from_headers(http_resp.headers());
            let body_text = http_resp.text().await.unwrap_or_default();
            warn!(
                status = %status,
                model = %self.model,
                provider = %self.provider_name,
                body = %body_text,
                "openai responses probe API error"
            );
            anyhow::bail!(
                "{}",
                super::with_retry_after_marker(
                    format!("OpenAI API error HTTP {status}: {body_text}"),
                    retry_after_ms,
                )
            );
        }

        Ok(())
    }

    fn requires_reasoning_content_on_tool_messages(&self) -> bool {
        self.provider_name.eq_ignore_ascii_case("moonshot")
            || self.base_url.contains("moonshot.ai")
            || self.base_url.contains("moonshot.cn")
            || self.model.starts_with("kimi-")
    }

    fn serialize_messages_for_request(&self, messages: &[ChatMessage]) -> Vec<serde_json::Value> {
        let needs_reasoning_content = self.requires_reasoning_content_on_tool_messages();
        let mut remapped_tool_call_ids = HashMap::new();
        let mut used_tool_call_ids = HashSet::new();
        let mut out = Vec::with_capacity(messages.len());

        for message in messages {
            let mut value = message.to_openai_value();

            if let Some(tool_calls) = value
                .get_mut("tool_calls")
                .and_then(serde_json::Value::as_array_mut)
            {
                for tool_call in tool_calls {
                    let Some(tool_call_id) =
                        tool_call.get("id").and_then(serde_json::Value::as_str)
                    else {
                        continue;
                    };
                    let mapped_id = assign_openai_tool_call_id(
                        tool_call_id,
                        &mut remapped_tool_call_ids,
                        &mut used_tool_call_ids,
                    );
                    tool_call["id"] = serde_json::Value::String(mapped_id);
                }
            } else if value.get("role").and_then(serde_json::Value::as_str) == Some("tool")
                && let Some(tool_call_id) = value
                    .get("tool_call_id")
                    .and_then(serde_json::Value::as_str)
            {
                let mapped_id = remapped_tool_call_ids
                    .get(tool_call_id)
                    .cloned()
                    .unwrap_or_else(|| {
                        assign_openai_tool_call_id(
                            tool_call_id,
                            &mut remapped_tool_call_ids,
                            &mut used_tool_call_ids,
                        )
                    });
                value["tool_call_id"] = serde_json::Value::String(mapped_id);
            }

            if needs_reasoning_content {
                let is_assistant =
                    value.get("role").and_then(serde_json::Value::as_str) == Some("assistant");
                let has_tool_calls = value
                    .get("tool_calls")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|calls| !calls.is_empty());

                if is_assistant && has_tool_calls {
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
                }
            }

            out.push(value);
        }

        out
    }

    fn is_openai_platform_base_url(&self) -> bool {
        reqwest::Url::parse(&self.base_url)
            .ok()
            .and_then(|url| url.host_str().map(ToString::to_string))
            .is_some_and(|host| host.eq_ignore_ascii_case("api.openai.com"))
    }

    /// Returns `true` when this provider targets an Anthropic model via
    /// OpenRouter, which supports prompt caching when `cache_control`
    /// breakpoints are present in the message payload.
    fn is_openrouter_anthropic(&self) -> bool {
        self.base_url.contains("openrouter.ai") && self.model.starts_with("anthropic/")
    }

    /// For OpenRouter Anthropic models, inject `cache_control` breakpoints
    /// on the system message and the last user message to enable prompt
    /// caching passthrough to Anthropic.
    fn apply_openrouter_cache_control(&self, messages: &mut [serde_json::Value]) {
        if !self.is_openrouter_anthropic()
            || matches!(self.cache_retention, moltis_config::CacheRetention::None)
        {
            return;
        }

        let cache_control = serde_json::json!({ "type": "ephemeral" });

        // Add cache_control to the system message content.
        for msg in messages.iter_mut() {
            if msg.get("role").and_then(serde_json::Value::as_str) != Some("system") {
                continue;
            }
            match msg.get_mut("content") {
                Some(content) if content.is_string() => {
                    let text = content.as_str().unwrap_or_default().to_string();
                    msg["content"] = serde_json::json!([{
                        "type": "text",
                        "text": text,
                        "cache_control": cache_control
                    }]);
                },
                Some(content) if content.is_array() => {
                    if let Some(last) = content.as_array_mut().and_then(|a| a.last_mut()) {
                        last["cache_control"] = cache_control.clone();
                    }
                },
                _ => {},
            }
            break;
        }

        // Add cache_control to the last user message.
        if let Some(last_user) = messages
            .iter_mut()
            .rev()
            .find(|m| m.get("role").and_then(serde_json::Value::as_str) == Some("user"))
        {
            match last_user.get_mut("content") {
                Some(content) if content.is_string() => {
                    let text = content.as_str().unwrap_or_default().to_string();
                    last_user["content"] = serde_json::json!([{
                        "type": "text",
                        "text": text,
                        "cache_control": cache_control
                    }]);
                },
                Some(content) if content.is_array() => {
                    if let Some(last) = content.as_array_mut().and_then(|a| a.last_mut()) {
                        last["cache_control"] = cache_control;
                    }
                },
                _ => {},
            }
        }
    }

    /// Build the HTTP URL for the Responses API (`/responses`).
    ///
    /// If the base URL already ends with `/responses`, use it as-is.
    /// Otherwise derive it as a sibling of `/chat/completions`, ensuring
    /// `/v1` is present — matching the normalization in
    /// `responses_websocket_url`.
    fn responses_sse_url(&self) -> String {
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

    fn responses_websocket_url(&self) -> crate::error::Result<String> {
        let mut base = self.base_url.trim_end_matches('/').to_string();
        if !base.ends_with("/v1") {
            base.push_str("/v1");
        }
        let url = format!("{base}/responses");
        if let Some(rest) = url.strip_prefix("https://") {
            return Ok(format!("wss://{rest}"));
        }
        if let Some(rest) = url.strip_prefix("http://") {
            return Ok(format!("ws://{rest}"));
        }
        Err(crate::error::Error::message(format!(
            "invalid OpenAI base_url for websocket mode: expected http:// or https://, got {}",
            self.base_url
        )))
    }

    /// Stream using the OpenAI Responses API format (`/responses`) over SSE.
    #[allow(clippy::collapsible_if)]
    fn stream_responses_sse(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<serde_json::Value>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        Box::pin(async_stream::stream! {
            let (instructions, input) = split_responses_instructions_and_input(messages);
            let mut body = serde_json::json!({
                "model": self.model,
                "input": input,
                "stream": true,
            });

            if let Some(instructions) = instructions {
                body["instructions"] = serde_json::Value::String(instructions);
            }

            if !tools.is_empty() {
                body["tools"] = serde_json::Value::Array(to_responses_api_tools(&tools));
                body["tool_choice"] = serde_json::json!("auto");
            }

            self.apply_reasoning_effort_responses(&mut body);

            debug!(
                model = %self.model,
                tools_count = tools.len(),
                "openai stream_responses_sse request"
            );
            trace!(body = %serde_json::to_string(&body).unwrap_or_default(), "openai responses stream request body");

            let url = self.responses_sse_url();
            let resp = match self
                .client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_key.expose_secret()))
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => {
                    if let Err(e) = r.error_for_status_ref() {
                        let status = e.status().map(|s| s.as_u16()).unwrap_or(0);
                        let retry_after_ms = super::retry_after_ms_from_headers(r.headers());
                        let body_text = r.text().await.unwrap_or_default();
                        yield StreamEvent::Error(super::with_retry_after_marker(
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
            let mut state = ResponsesStreamState::default();
            let mut stream_done = false;

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

                    let Some(data) = line
                        .strip_prefix("data: ")
                        .or_else(|| line.strip_prefix("data:"))
                    else {
                        // Handle bare event types (e.g. "event: response.completed")
                        continue;
                    };

                    match process_responses_sse_line(data, &mut state) {
                        SseLineResult::Done => {
                            stream_done = true;
                            break;
                        }
                        SseLineResult::Events(events) => {
                            for event in events {
                                yield event;
                            }
                        }
                        SseLineResult::Skip => {}
                    }
                }
                if stream_done {
                    break;
                }
            }

            // Process any residual buffered line on EOF.
            if !stream_done {
                let line = buf.trim().to_string();
                if !line.is_empty()
                    && let Some(data) = line
                        .strip_prefix("data: ")
                        .or_else(|| line.strip_prefix("data:"))
                {
                    match process_responses_sse_line(data, &mut state) {
                        SseLineResult::Done | SseLineResult::Skip => {}
                        SseLineResult::Events(events) => {
                            for event in events {
                                yield event;
                            }
                        }
                    }
                }
            }

            // Finalize: emit pending ToolCallComplete events + Done with usage.
            for event in finalize_responses_stream(&mut state) {
                yield event;
            }
        })
    }

    #[allow(clippy::collapsible_if)]
    fn stream_with_tools_sse(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<serde_json::Value>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        Box::pin(async_stream::stream! {
            let mut openai_messages = self.serialize_messages_for_request(&messages);
            self.apply_openrouter_cache_control(&mut openai_messages);
            let mut body = serde_json::json!({
                "model": self.model,
                "messages": openai_messages,
                "stream": true,
                "stream_options": { "include_usage": true },
            });

            if !tools.is_empty() {
                body["tools"] = serde_json::Value::Array(to_openai_tools(&tools));
            }

            self.apply_reasoning_effort_chat(&mut body);

            debug!(
                model = %self.model,
                messages_count = openai_messages.len(),
                tools_count = tools.len(),
                reasoning_effort = ?self.reasoning_effort,
                "openai stream_with_tools request (sse)"
            );
            trace!(body = %serde_json::to_string(&body).unwrap_or_default(), "openai stream request body (sse)");

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
                        let retry_after_ms = super::retry_after_ms_from_headers(r.headers());
                        let body_text = r.text().await.unwrap_or_default();
                        yield StreamEvent::Error(super::with_retry_after_marker(
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

                    let Some(data) = line
                        .strip_prefix("data: ")
                        .or_else(|| line.strip_prefix("data:"))
                    else {
                        continue;
                    };

                    match process_openai_sse_line(data, &mut state) {
                        SseLineResult::Done => {
                            for event in finalize_stream(&mut state) {
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

            // Some OpenAI-compatible providers may close the stream without
            // an explicit [DONE] frame or trailing newline. Process any
            // residual buffered line and always finalize on EOF so usage
            // metadata still propagates.
            let line = buf.trim().to_string();
            if !line.is_empty()
                && let Some(data) = line
                    .strip_prefix("data: ")
                    .or_else(|| line.strip_prefix("data:"))
            {
                match process_openai_sse_line(data, &mut state) {
                    SseLineResult::Done => {
                        for event in finalize_stream(&mut state) {
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

            for event in finalize_stream(&mut state) {
                yield event;
            }
        })
    }

    #[allow(clippy::collapsible_if)]
    fn stream_with_tools_websocket(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<serde_json::Value>,
        fallback_to_sse: bool,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        // Synchronous pre-flight: URL, request, auth header, pool key.
        // Fail fast and fall back to SSE before entering the async generator,
        // which avoids cloning messages/tools for the four sync-check paths.
        let (request, pool_key) = match (|| -> crate::error::Result<_> {
            if !self.is_openai_platform_base_url() {
                return Err(crate::error::Error::message(format!(
                    "websocket mode is only supported for api.openai.com (got {})",
                    self.base_url
                )));
            }
            let ws_url = self.responses_websocket_url()?;
            let pk = super::ws_pool::PoolKey::new(&ws_url, &self.api_key);
            let mut req = ws_url.as_str().into_client_request()?;
            let auth = format!("Bearer {}", self.api_key.expose_secret());
            req.headers_mut()
                .insert("Authorization", HeaderValue::from_str(&auth)?);
            req.headers_mut()
                .insert("OpenAI-Beta", HeaderValue::from_static("responses=v1"));
            Ok((req, pk))
        })() {
            Ok(r) => r,
            Err(err) => {
                if fallback_to_sse {
                    debug!(error = %err, "websocket setup failed, falling back to sse");
                    return self.stream_with_tools_sse(messages, tools);
                }
                return Box::pin(async_stream::stream! {
                    yield StreamEvent::Error(err.to_string());
                });
            },
        };

        Box::pin(async_stream::stream! {
            // Try the pool first; fall back to a fresh connection.
            let (mut ws_stream, created_at) = if let Some(pooled) = super::ws_pool::shared_ws_pool().checkout(&pool_key).await {
                pooled
            } else {
                match tokio_tungstenite::connect_async(request).await {
                    Ok((ws, _)) => (ws, std::time::Instant::now()),
                    Err(err) => {
                        if fallback_to_sse {
                            debug!(error = %err, "websocket connect failed, falling back to sse");
                            let mut sse = self.stream_with_tools_sse(messages, tools);
                            while let Some(event) = sse.next().await {
                                yield event;
                            }
                        } else {
                            yield StreamEvent::Error(err.to_string());
                        }
                        return;
                    }
                }
            };

            let (instructions, input) = split_responses_instructions_and_input(messages);
            let mut response_payload = serde_json::json!({
                "model": self.model,
                "stream": true,
                "store": false,
                "input": input,
            });
            if let Some(instructions) = instructions {
                response_payload["instructions"] = serde_json::Value::String(instructions);
            }
            if !tools.is_empty() {
                response_payload["tools"] = serde_json::Value::Array(to_responses_api_tools(&tools));
                response_payload["tool_choice"] = serde_json::json!("auto");
            }

            self.apply_reasoning_effort_responses(&mut response_payload);

            let create_event = serde_json::json!({
                "type": "response.create",
                "response": response_payload,
            });

            debug!(
                model = %self.model,
                tools_count = tools.len(),
                reasoning_effort = ?self.reasoning_effort,
                "openai stream_with_tools request (websocket)"
            );
            trace!(event = %create_event, "openai websocket create event");

            if let Err(err) = ws_stream
                .send(Message::Text(create_event.to_string().into()))
                .await
            {
                yield StreamEvent::Error(format!("websocket send failed: {err}"));
                return;
            }

            let mut input_tokens: u32 = 0;
            let mut output_tokens: u32 = 0;
            let mut cache_read_tokens: u32 = 0;
            let mut cache_write_tokens: u32 = 0;
            let mut current_tool_index: usize = 0;
            let mut tool_calls: HashMap<usize, (String, String)> = HashMap::new();
            let mut completed_tool_calls: HashSet<usize> = HashSet::new();
            let mut clean_completion = false;

            while let Some(frame) = ws_stream.next().await {
                let text = match frame {
                    Ok(Message::Text(t)) => t.to_string(),
                    Ok(Message::Binary(b)) => String::from_utf8_lossy(&b).into_owned(),
                    Ok(Message::Ping(p)) => {
                        if let Err(err) = ws_stream.send(Message::Pong(p)).await {
                            yield StreamEvent::Error(err.to_string());
                            return;
                        }
                        continue;
                    }
                    Ok(Message::Close(_)) => break,
                    Ok(_) => continue,
                    Err(err) => {
                        yield StreamEvent::Error(err.to_string());
                        return;
                    }
                };

                let Ok(evt) = serde_json::from_str::<serde_json::Value>(&text) else {
                    continue;
                };
                trace!(event = %evt, "openai websocket event");

                match evt["type"].as_str().unwrap_or("") {
                    "response.output_text.delta" => {
                        if let Some(delta) = evt["delta"].as_str()
                            && !delta.is_empty()
                        {
                            yield StreamEvent::Delta(delta.to_string());
                        }
                    }
                    "response.output_item.added" => {
                        if evt["item"]["type"].as_str() == Some("function_call") {
                            let id = evt["item"]["call_id"].as_str().unwrap_or("").to_string();
                            let name = evt["item"]["name"].as_str().unwrap_or("").to_string();
                            let index = responses_output_index(&evt, current_tool_index);
                            current_tool_index = current_tool_index.max(index + 1);
                            tool_calls.insert(index, (id.clone(), name.clone()));
                            yield StreamEvent::ToolCallStart { id, name, index };
                        }
                    }
                    "response.function_call_arguments.delta" => {
                        if let Some(delta) = evt["delta"].as_str()
                            && !delta.is_empty()
                        {
                            let index = responses_output_index(&evt, current_tool_index.saturating_sub(1));
                            yield StreamEvent::ToolCallArgumentsDelta {
                                index,
                                delta: delta.to_string(),
                            };
                        }
                    }
                    "response.function_call_arguments.done" => {
                        let index = responses_output_index(&evt, current_tool_index.saturating_sub(1));
                        if completed_tool_calls.insert(index) {
                            yield StreamEvent::ToolCallComplete { index };
                        }
                    }
                    "response.completed" => {
                        if let Some(usage) = evt.get("response").and_then(|response| response.get("usage")) {
                            let parsed = parse_openai_compat_usage(usage);
                            input_tokens = parsed.input_tokens;
                            output_tokens = parsed.output_tokens;
                            cache_read_tokens = parsed.cache_read_tokens;
                            cache_write_tokens = parsed.cache_write_tokens;
                        }
                        let mut pending: Vec<usize> = tool_calls.keys().copied().collect();
                        pending.sort_unstable();
                        for index in pending {
                            if completed_tool_calls.insert(index) {
                                yield StreamEvent::ToolCallComplete { index };
                            }
                        }
                        clean_completion = true;
                        break;
                    }
                    "error" | "response.failed" => {
                        let msg = evt["error"]["message"]
                            .as_str()
                            .or_else(|| evt["response"]["error"]["message"].as_str())
                            .or_else(|| evt["message"].as_str())
                            .unwrap_or("unknown error");
                        yield StreamEvent::Error(msg.to_string());
                        return;
                    }
                    _ => {}
                }
            }

            // Emit any remaining tool-call completions (fallback for broken streams).
            if !clean_completion {
                let mut pending: Vec<usize> = tool_calls.keys().copied().collect();
                pending.sort_unstable();
                for index in pending {
                    if completed_tool_calls.insert(index) {
                        yield StreamEvent::ToolCallComplete { index };
                    }
                }
            }

            // Return healthy connections to the pool; drop on error / close.
            if clean_completion {
                super::ws_pool::shared_ws_pool()
                    .return_conn(pool_key, ws_stream, created_at)
                    .await;
            }

            yield StreamEvent::Done(Usage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
            });
        })
    }

    /// Non-streaming completion using the Responses API.
    ///
    /// Sends `stream: true` and collects events into a single response, since
    /// many Responses API endpoints only support streaming.
    async fn complete_responses(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> anyhow::Result<CompletionResponse> {
        let (instructions, input) = split_responses_instructions_and_input(messages.to_vec());
        let mut body = serde_json::json!({
            "model": self.model,
            "input": input,
            "stream": true,
        });
        if let Some(instructions) = instructions {
            body["instructions"] = serde_json::Value::String(instructions);
        }
        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(to_responses_api_tools(tools));
            body["tool_choice"] = serde_json::json!("auto");
        }

        debug!(
            model = %self.model,
            messages_count = messages.len(),
            tools_count = tools.len(),
            "openai complete_responses request"
        );
        trace!(body = %serde_json::to_string(&body).unwrap_or_default(), "openai responses request body");

        let url = self.responses_sse_url();
        let http_resp = self
            .client
            .post(&url)
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
            let retry_after_ms = super::retry_after_ms_from_headers(http_resp.headers());
            let body_text = http_resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "{}",
                super::with_retry_after_marker(
                    format!("Responses API error HTTP {status}: {body_text}"),
                    retry_after_ms,
                )
            );
        }

        // Collect SSE events into text + tool calls.
        let mut text_buf = String::new();
        let mut fn_call_ids: Vec<String> = Vec::new();
        let mut fn_call_names: Vec<String> = Vec::new();
        let mut fn_call_args: Vec<String> = Vec::new();
        let mut input_tokens: u32 = 0;
        let mut output_tokens: u32 = 0;
        let mut cache_read_tokens: u32 = 0;
        let cache_write_tokens: u32 = 0;

        let full_body = http_resp.text().await.unwrap_or_default();
        for line in full_body.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some(data) = line
                .strip_prefix("data: ")
                .or_else(|| line.strip_prefix("data:"))
            else {
                continue;
            };
            if data == "[DONE]" {
                break;
            }

            let Ok(evt) = serde_json::from_str::<serde_json::Value>(data) else {
                continue;
            };

            match evt["type"].as_str().unwrap_or("") {
                "response.output_text.delta" => {
                    if let Some(delta) = evt["delta"].as_str() {
                        text_buf.push_str(delta);
                    }
                },
                "response.output_item.added" => {
                    if evt["item"]["type"].as_str() == Some("function_call") {
                        fn_call_ids.push(evt["item"]["call_id"].as_str().unwrap_or("").to_string());
                        fn_call_names.push(evt["item"]["name"].as_str().unwrap_or("").to_string());
                        fn_call_args.push(String::new());
                    }
                },
                "response.function_call_arguments.delta" => {
                    if let Some(delta) = evt["delta"].as_str()
                        && let Some(last) = fn_call_args.last_mut()
                    {
                        last.push_str(delta);
                    }
                },
                "response.completed" => {
                    if let Some(u) = evt["response"]["usage"].as_object() {
                        input_tokens =
                            u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        output_tokens =
                            u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        cache_read_tokens = u
                            .get("input_tokens_details")
                            .and_then(|d| d.get("cached_tokens"))
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as u32;
                    }
                },
                "error" | "response.failed" => {
                    let msg = evt["error"]["message"]
                        .as_str()
                        .or_else(|| evt["response"]["error"]["message"].as_str())
                        .or_else(|| evt["message"].as_str())
                        .unwrap_or("unknown error");
                    anyhow::bail!("Responses API error: {msg}");
                },
                _ => {},
            }
        }

        let text = if text_buf.is_empty() {
            None
        } else {
            Some(text_buf)
        };

        let tool_calls: Vec<moltis_agents::model::ToolCall> = fn_call_ids
            .into_iter()
            .zip(fn_call_names)
            .zip(fn_call_args)
            .filter_map(|((id, name), args)| {
                let arguments: serde_json::Value = serde_json::from_str(&args)
                    .unwrap_or(serde_json::Value::Object(Default::default()));
                if name.is_empty() {
                    return None;
                }
                Some(moltis_agents::model::ToolCall {
                    id,
                    name,
                    arguments,
                })
            })
            .collect();

        Ok(CompletionResponse {
            text,
            tool_calls,
            usage: Usage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
            },
        })
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
        }))
    }

    fn id(&self) -> &str {
        &self.model
    }

    fn supports_tools(&self) -> bool {
        match self.tool_mode_override {
            Some(moltis_config::ToolMode::Native) => true,
            Some(moltis_config::ToolMode::Text | moltis_config::ToolMode::Off) => false,
            Some(moltis_config::ToolMode::Auto) | None => {
                super::supports_tools_for_model(&self.model)
            },
        }
    }

    fn tool_mode(&self) -> Option<moltis_config::ToolMode> {
        self.tool_mode_override
    }

    fn context_window(&self) -> u32 {
        super::context_window_for_model(&self.model)
    }

    fn supports_vision(&self) -> bool {
        super::supports_vision_for_model(&self.model)
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

        let mut openai_messages = self.serialize_messages_for_request(messages);
        self.apply_openrouter_cache_control(&mut openai_messages);
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": openai_messages,
        });

        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(to_openai_tools(tools));
        }

        self.apply_reasoning_effort_chat(&mut body);

        debug!(
            model = %self.model,
            messages_count = messages.len(),
            tools_count = tools.len(),
            reasoning_effort = ?self.reasoning_effort,
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
            let retry_after_ms = super::retry_after_ms_from_headers(http_resp.headers());
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
            anyhow::bail!(
                "{}",
                super::with_retry_after_marker(
                    format!("OpenAI API error HTTP {status}: {body_text}"),
                    retry_after_ms,
                )
            );
        }

        let resp = http_resp.json::<serde_json::Value>().await?;
        trace!(response = %resp, "openai raw response");

        let message = &resp["choices"][0]["message"];

        let text = message["content"].as_str().and_then(|s| {
            let (visible, _thinking) = strip_think_tags(s);
            if visible.is_empty() {
                None
            } else {
                Some(visible)
            }
        });
        let tool_calls = parse_tool_calls(message);

        let usage = parse_openai_compat_usage_from_payload(&resp).unwrap_or_default();

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

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use {
        axum::{Router, extract::Request, routing::post},
        secrecy::Secret,
        tokio_stream::StreamExt,
    };

    use moltis_agents::model::{ChatMessage, ToolCall, Usage};

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

    /// Start a mock JSON server at `/chat/completions` that captures the
    /// request body and returns the given JSON payload.
    async fn start_completion_mock(
        json_payload: String,
    ) -> (String, Arc<Mutex<Vec<CapturedRequest>>>) {
        let captured: Arc<Mutex<Vec<CapturedRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_clone = captured.clone();

        let app = Router::new().route(
            "/chat/completions",
            post(move |req: Request| {
                let cap = captured_clone.clone();
                let payload = json_payload.clone();
                async move {
                    let body_bytes = axum::body::to_bytes(req.into_body(), 1024 * 1024)
                        .await
                        .unwrap_or_default();
                    let body: Option<serde_json::Value> = serde_json::from_slice(&body_bytes).ok();
                    cap.lock().unwrap().push(CapturedRequest { body });

                    axum::response::Response::builder()
                        .header("content-type", "application/json")
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

    #[test]
    fn minimax_serialization_preserves_system_messages() {
        let provider = OpenAiProvider::new_with_name(
            Secret::new("test-key".to_string()),
            "MiniMax-M2.1".to_string(),
            "https://api.minimax.io/v1".to_string(),
            "minimax".to_string(),
        );
        let serialized = provider.serialize_messages_for_request(&[
            ChatMessage::system("sys a"),
            ChatMessage::user("hi"),
            ChatMessage::system("sys b"),
        ]);
        assert_eq!(serialized.len(), 3);
        assert_eq!(serialized[0]["role"], "system");
        assert_eq!(serialized[1]["role"], "user");
        assert_eq!(serialized[2]["role"], "system");
    }

    #[test]
    fn openai_serialization_remaps_long_tool_call_ids() {
        let provider = OpenAiProvider::new(
            Secret::new("test-key".to_string()),
            "gpt-4o".to_string(),
            "https://api.openai.com/v1".to_string(),
        );
        let long_id = "forced-123e4567-e89b-12d3-a456-426614174000";
        let messages = vec![
            ChatMessage::assistant_with_tools(Some("running command".to_string()), vec![
                ToolCall {
                    id: long_id.to_string(),
                    name: "exec".to_string(),
                    arguments: serde_json::json!({ "command": "pwd" }),
                },
            ]),
            ChatMessage::tool(long_id, "ok"),
        ];

        let serialized = provider.serialize_messages_for_request(&messages);
        assert_eq!(serialized.len(), 2);

        let remapped_id = serialized[0]["tool_calls"][0]["id"]
            .as_str()
            .unwrap_or_default();
        assert!(!remapped_id.is_empty());
        assert!(remapped_id.len() <= OPENAI_MAX_TOOL_CALL_ID_LEN);
        assert_ne!(remapped_id, long_id);
        assert_eq!(serialized[1]["tool_call_id"].as_str(), Some(remapped_id));
    }

    #[test]
    fn openai_serialization_keeps_short_tool_call_ids_stable() {
        let provider = OpenAiProvider::new(
            Secret::new("test-key".to_string()),
            "gpt-4o".to_string(),
            "https://api.openai.com/v1".to_string(),
        );
        let short_id = "call_abc";
        let messages = vec![
            ChatMessage::assistant_with_tools(Some("running command".to_string()), vec![
                ToolCall {
                    id: short_id.to_string(),
                    name: "exec".to_string(),
                    arguments: serde_json::json!({ "command": "pwd" }),
                },
            ]),
            ChatMessage::tool(short_id, "ok"),
        ];

        let serialized = provider.serialize_messages_for_request(&messages);
        assert_eq!(
            serialized[0]["tool_calls"][0]["id"].as_str(),
            Some(short_id)
        );
        assert_eq!(serialized[1]["tool_call_id"].as_str(), Some(short_id));
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

    #[tokio::test]
    async fn minimax_stream_request_preserves_system_role_messages() {
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\n\
                   data: [DONE]\n\n";
        let (base_url, captured) = start_sse_mock(sse.to_string()).await;
        let provider = OpenAiProvider::new_with_name(
            Secret::new("test-key".to_string()),
            "MiniMax-M2.1".to_string(),
            base_url,
            "minimax".to_string(),
        );
        let messages = vec![
            ChatMessage::system("stay deterministic"),
            ChatMessage::user("ping"),
        ];

        let mut stream = provider.stream_with_tools(messages, vec![]);
        while stream.next().await.is_some() {}

        let reqs = captured.lock().unwrap();
        assert_eq!(reqs.len(), 1);
        let body = reqs[0].body.as_ref().expect("request should have a body");
        assert!(body.get("system").is_none());

        let history = body["messages"]
            .as_array()
            .expect("messages should be an array");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0]["role"], "system");
        assert_eq!(history[0]["content"], "stay deterministic");
        assert_eq!(history[1]["role"], "user");
    }

    #[tokio::test]
    async fn minimax_complete_request_preserves_system_role_messages_regression_578() {
        let payload = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "hi"
                }
            }],
            "usage": {
                "prompt_tokens": 12,
                "completion_tokens": 1
            }
        });
        let (base_url, captured) = start_completion_mock(payload.to_string()).await;
        let provider = OpenAiProvider::new_with_name(
            Secret::new("test-key".to_string()),
            "MiniMax-M2.7".to_string(),
            base_url,
            "minimax".to_string(),
        );

        let response = provider
            .complete(
                &[
                    ChatMessage::system("you are a helpful assistant"),
                    ChatMessage::user("hello"),
                    ChatMessage::system("extra context"),
                ],
                &[],
            )
            .await
            .expect("request should succeed");
        assert_eq!(response.text.as_deref(), Some("hi"));

        let reqs = captured.lock().unwrap();
        let body = reqs[0].body.as_ref().expect("request should have a body");
        assert!(body.get("system").is_none());

        let history = body["messages"]
            .as_array()
            .expect("messages should be an array");
        assert_eq!(history.len(), 3);
        assert_eq!(history[0]["role"], "system");
        assert_eq!(history[0]["content"], "you are a helpful assistant");
        assert_eq!(history[1]["role"], "user");
        assert_eq!(history[2]["role"], "system");
        assert_eq!(history[2]["content"], "extra context");
    }

    #[tokio::test]
    async fn minimax_stream_preserves_multiple_system_messages_regression_578() {
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":null}]}\n\n\
                   data: [DONE]\n\n";
        let (base_url, captured) = start_sse_mock(sse.to_string()).await;
        let provider = OpenAiProvider::new_with_name(
            Secret::new("test-key".to_string()),
            "MiniMax-M2.7".to_string(),
            base_url,
            "minimax".to_string(),
        );

        let messages = vec![
            ChatMessage::system("you are a helpful assistant"),
            ChatMessage::user("hello"),
            ChatMessage::system("extra context"),
        ];

        let mut stream = provider.stream_with_tools(messages, vec![]);
        while stream.next().await.is_some() {}

        let reqs = captured.lock().unwrap();
        let body = reqs[0].body.as_ref().expect("request should have a body");
        assert!(body.get("system").is_none());

        let history = body["messages"]
            .as_array()
            .expect("messages should be an array");
        assert_eq!(history.len(), 3);
        assert_eq!(history[0]["role"], "system");
        assert_eq!(history[0]["content"], "you are a helpful assistant");
        assert_eq!(history[1]["role"], "user");
        assert_eq!(history[2]["role"], "system");
        assert_eq!(history[2]["content"], "extra context");
    }

    #[tokio::test]
    async fn probe_chat_request_caps_minimax_output_to_one_token() {
        let (base_url, captured) = start_sse_mock("{}".to_string()).await;
        let provider = OpenAiProvider::new_with_name(
            Secret::new("test-key".to_string()),
            "MiniMax-M2.7".to_string(),
            base_url,
            "minimax".to_string(),
        );

        provider.probe().await.expect("probe should succeed");

        let reqs = captured.lock().unwrap();
        assert_eq!(reqs.len(), 1);
        let body = reqs[0].body.as_ref().expect("request should have a body");
        assert_eq!(body["max_tokens"], 1);
        assert!(body.get("max_completion_tokens").is_none());
        assert!(body.get("system").is_none());
    }

    #[tokio::test]
    async fn probe_chat_request_uses_max_completion_tokens_for_gpt5() {
        let (base_url, captured) = start_sse_mock("{}".to_string()).await;
        let provider = OpenAiProvider::new(
            Secret::new("test-key".to_string()),
            "gpt-5.2".to_string(),
            base_url,
        );

        provider.probe().await.expect("probe should succeed");

        let reqs = captured.lock().unwrap();
        assert_eq!(reqs.len(), 1);
        let body = reqs[0].body.as_ref().expect("request should have a body");
        assert_eq!(body["max_completion_tokens"], 1);
        assert!(body.get("max_tokens").is_none());
    }

    #[tokio::test]
    async fn stream_without_done_frame_still_emits_done_with_usage() {
        // Some providers close SSE without [DONE] and without a trailing newline.
        // We must still flush trailing usage and emit Done.
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":5040,\"completion_tokens\":61}}"
        );
        let (base_url, _) = start_sse_mock(sse.to_string()).await;
        let provider = OpenAiProvider::new_with_name(
            Secret::new("test-key".to_string()),
            "MiniMax-M2.1".to_string(),
            base_url,
            "minimax".to_string(),
        );

        let mut stream =
            provider.stream_with_tools(vec![ChatMessage::user("tell me a joke")], vec![]);
        let mut last_done: Option<Usage> = None;
        while let Some(ev) = stream.next().await {
            if let StreamEvent::Done(usage) = ev {
                last_done = Some(usage);
            }
        }

        let usage = last_done.expect("stream should emit Done");
        assert_eq!(usage.input_tokens, 5040);
        assert_eq!(usage.output_tokens, 61);
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
    fn parse_models_payload_accepts_provider_prefixed_model_ids() {
        let payload = serde_json::json!({
            "data": [
                { "id": "openai/gpt-5.2", "created": 3000 },
                { "id": "google/gemini-2.0-flash", "created": 2000 },
                { "id": "openai/gpt-image-1", "created": 1000 },
                { "id": "openai/gpt-4o-mini-tts", "created": 900 }
            ]
        });

        let models = parse_models_payload(&payload);
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["openai/gpt-5.2", "google/gemini-2.0-flash"]);
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
        use crate::DiscoveredModel;
        let discovered = vec![
            DiscoveredModel::new("gpt-5.2", "GPT-5.2"),
            DiscoveredModel::new("zeta-model", "Zeta"),
            DiscoveredModel::new("alpha-model", "Alpha"),
        ];
        let fallback = vec![
            DiscoveredModel::new("gpt-5.2", "fallback"),
            DiscoveredModel::new("gpt-4o", "GPT-4o"),
        ];

        let merged = crate::merge_discovered_with_fallback_catalog(discovered, fallback);
        let ids: Vec<String> = merged.into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["gpt-5.2", "zeta-model", "alpha-model"]);
    }

    #[test]
    fn merge_with_fallback_uses_fallback_when_discovery_is_empty() {
        use crate::DiscoveredModel;
        let merged = crate::merge_discovered_with_fallback_catalog(Vec::new(), vec![
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

    #[tokio::test]
    async fn websocket_auto_falls_back_to_sse_for_non_openai_base_url() {
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":null}]}\n\n",
            "data: {\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":7}}\n\n",
            "data: [DONE]\n\n"
        );
        let (base_url, _) = start_sse_mock(sse.to_string()).await;
        let provider = OpenAiProvider::new(
            Secret::new("test-key".to_string()),
            "gpt-5.2".to_string(),
            base_url,
        )
        .with_stream_transport(ProviderStreamTransport::Auto);

        let mut stream =
            provider.stream_with_tools(vec![ChatMessage::user("test")], sample_tools());

        let mut saw_delta = false;
        let mut saw_done = false;

        while let Some(event) = stream.next().await {
            match event {
                StreamEvent::Delta(delta) => {
                    saw_delta = true;
                    assert_eq!(delta, "hi");
                },
                StreamEvent::Done(usage) => {
                    saw_done = true;
                    assert_eq!(usage.input_tokens, 11);
                    assert_eq!(usage.output_tokens, 7);
                },
                StreamEvent::Error(err) => panic!("unexpected stream error: {err}"),
                _ => {},
            }
        }

        assert!(saw_delta, "expected fallback stream to emit Delta");
        assert!(saw_done, "expected fallback stream to emit Done");
    }

    #[tokio::test]
    async fn websocket_mode_errors_for_non_openai_base_url_without_fallback() {
        let sse = "data: [DONE]\n\n";
        let (base_url, _) = start_sse_mock(sse.to_string()).await;
        let provider = OpenAiProvider::new(
            Secret::new("test-key".to_string()),
            "gpt-5.2".to_string(),
            base_url,
        )
        .with_stream_transport(ProviderStreamTransport::Websocket);

        let mut stream = provider.stream_with_tools(vec![ChatMessage::user("test")], vec![]);
        let first = stream
            .next()
            .await
            .expect("stream should emit an immediate error");
        match first {
            StreamEvent::Error(msg) => {
                assert!(msg.contains("api.openai.com"), "unexpected error: {msg}");
            },
            other => panic!("expected stream error, got {other:?}"),
        }
    }

    // ============================================================
    // Tests for WireApi / responses_sse_url
    // ============================================================

    #[test]
    fn responses_sse_url_from_v1_base() {
        let provider = OpenAiProvider::new(
            Secret::new("test".to_string()),
            "gpt-5.2".to_string(),
            "https://example.com/v1".to_string(),
        );
        assert_eq!(
            provider.responses_sse_url(),
            "https://example.com/v1/responses"
        );
    }

    #[test]
    fn responses_sse_url_from_chat_completions_base() {
        let provider = OpenAiProvider::new(
            Secret::new("test".to_string()),
            "gpt-5.2".to_string(),
            "https://example.com/v1/chat/completions".to_string(),
        );
        assert_eq!(
            provider.responses_sse_url(),
            "https://example.com/v1/responses"
        );
    }

    #[test]
    fn responses_sse_url_already_responses() {
        let provider = OpenAiProvider::new(
            Secret::new("test".to_string()),
            "gpt-5.2".to_string(),
            "https://example.com/v1/responses".to_string(),
        );
        assert_eq!(
            provider.responses_sse_url(),
            "https://example.com/v1/responses"
        );
    }

    #[test]
    fn responses_sse_url_trailing_slash() {
        let provider = OpenAiProvider::new(
            Secret::new("test".to_string()),
            "gpt-5.2".to_string(),
            "https://example.com/v1/".to_string(),
        );
        assert_eq!(
            provider.responses_sse_url(),
            "https://example.com/v1/responses"
        );
    }

    #[test]
    fn responses_sse_url_bare_host_appends_v1() {
        let provider = OpenAiProvider::new(
            Secret::new("test".to_string()),
            "gpt-5.2".to_string(),
            "https://api.openai.com".to_string(),
        );
        assert_eq!(
            provider.responses_sse_url(),
            "https://api.openai.com/v1/responses"
        );
    }

    #[test]
    fn with_wire_api_builder() {
        let provider = OpenAiProvider::new(
            Secret::new("test".to_string()),
            "gpt-5.2".to_string(),
            "https://example.com/v1".to_string(),
        )
        .with_wire_api(WireApi::Responses);
        assert_eq!(provider.wire_api, WireApi::Responses);
    }

    /// Start a mock SSE server at `/responses` that returns the given payload.
    async fn start_responses_mock(
        sse_payload: String,
    ) -> (String, Arc<Mutex<Vec<CapturedRequest>>>) {
        let captured: Arc<Mutex<Vec<CapturedRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_clone = captured.clone();

        let app = Router::new().route(
            "/v1/responses",
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

    #[tokio::test]
    async fn responses_sse_stream_text_delta() {
        let sse = "\
            data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello \"}\n\n\
            data: {\"type\":\"response.output_text.delta\",\"delta\":\"world\"}\n\n\
            data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":2}}}\n\n";
        let (base_url, captured) = start_responses_mock(sse.to_string()).await;
        let provider = OpenAiProvider::new(
            Secret::new("test-key".to_string()),
            "test-model".to_string(),
            base_url,
        )
        .with_wire_api(WireApi::Responses);

        let mut stream = provider.stream_with_tools(vec![ChatMessage::user("hi")], vec![]);
        let mut text = String::new();
        let mut done_count = 0u32;
        let mut done_usage = None;
        let mut raw_count = 0u32;
        while let Some(event) = stream.next().await {
            match event {
                StreamEvent::Delta(d) => text.push_str(&d),
                StreamEvent::Done(u) => {
                    done_count += 1;
                    done_usage = Some(u);
                },
                StreamEvent::ProviderRaw(_) => raw_count += 1,
                _ => {},
            }
        }
        assert_eq!(text, "hello world");
        let usage = done_usage.expect("should have received Done event");
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 2);
        // Must emit exactly one Done — no double-Done on EOF.
        assert_eq!(
            done_count, 1,
            "expected exactly 1 Done event, got {done_count}"
        );
        // Must emit ProviderRaw for each non-completed SSE event (2 delta lines).
        assert!(
            raw_count >= 2,
            "expected at least 2 ProviderRaw events, got {raw_count}"
        );

        // Verify the request used Responses API format (input, not messages)
        let reqs = captured.lock().unwrap();
        assert_eq!(reqs.len(), 1);
        let body = reqs[0].body.as_ref().unwrap();
        assert!(
            body.get("input").is_some(),
            "should use 'input' not 'messages'"
        );
        assert!(body.get("messages").is_none(), "should not have 'messages'");
        assert_eq!(body["stream"], true);
    }

    #[tokio::test]
    async fn responses_sse_stream_propagates_cached_tokens() {
        // Responses API nests cached_tokens under input_tokens_details.
        let sse = "\
            data: {\"type\":\"response.output_text.delta\",\"delta\":\"cached\"}\n\n\
            data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":100,\"output_tokens\":10,\"input_tokens_details\":{\"cached_tokens\":80}}}}\n\n";
        let (base_url, _) = start_responses_mock(sse.to_string()).await;
        let provider = OpenAiProvider::new(
            Secret::new("test-key".to_string()),
            "test-model".to_string(),
            base_url,
        )
        .with_wire_api(WireApi::Responses);

        let mut stream = provider.stream_with_tools(vec![ChatMessage::user("hi")], vec![]);
        let mut done_usage = None;
        while let Some(event) = stream.next().await {
            if let StreamEvent::Done(u) = event {
                done_usage = Some(u);
            }
        }
        let usage = done_usage.expect("should have received Done event");
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 10);
        assert_eq!(usage.cache_read_tokens, 80);
        assert_eq!(usage.cache_write_tokens, 0);
    }

    #[tokio::test]
    async fn responses_sse_stream_cached_tokens_default_to_zero() {
        // Provider omits cached_tokens — should default to zero, not panic.
        let sse = "\
            data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\n\n\
            data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":5,\"output_tokens\":1}}}\n\n";
        let (base_url, _) = start_responses_mock(sse.to_string()).await;
        let provider = OpenAiProvider::new(
            Secret::new("test-key".to_string()),
            "test-model".to_string(),
            base_url,
        )
        .with_wire_api(WireApi::Responses);

        let mut stream = provider.stream_with_tools(vec![ChatMessage::user("hi")], vec![]);
        let mut done_usage = None;
        while let Some(event) = stream.next().await {
            if let StreamEvent::Done(u) = event {
                done_usage = Some(u);
            }
        }
        let usage = done_usage.expect("should have received Done event");
        assert_eq!(usage.cache_read_tokens, 0);
        assert_eq!(usage.cache_write_tokens, 0);
    }

    #[tokio::test]
    async fn probe_responses_request_caps_output_tokens() {
        let (base_url, captured) = start_responses_mock("{}".to_string()).await;
        let provider = OpenAiProvider::new(
            Secret::new("test-key".to_string()),
            "gpt-5.2".to_string(),
            base_url,
        )
        .with_wire_api(WireApi::Responses);

        provider.probe().await.expect("probe should succeed");

        let reqs = captured.lock().unwrap();
        assert_eq!(reqs.len(), 1);
        let body = reqs[0].body.as_ref().expect("request should have a body");
        assert_eq!(body["max_output_tokens"], 1);
        assert!(body.get("stream").is_none());
    }

    #[tokio::test]
    async fn responses_sse_stream_with_tool_calls() {
        let sse = "\
            data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"call_id\":\"call_abc\",\"name\":\"exec\"}}\n\n\
            data: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":0,\"delta\":\"{\\\"cmd\\\":\\\"ls\\\"}\"}\n\n\
            data: {\"type\":\"response.function_call_arguments.done\",\"output_index\":0}\n\n\
            data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":5,\"output_tokens\":3}}}\n\n";
        let (base_url, _) = start_responses_mock(sse.to_string()).await;

        let provider = OpenAiProvider::new(
            Secret::new("test-key".to_string()),
            "test-model".to_string(),
            base_url,
        )
        .with_wire_api(WireApi::Responses);

        let tools = sample_tools();
        let mut stream = provider.stream_with_tools(vec![ChatMessage::user("run ls")], tools);
        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event);
        }

        // Should have ToolCallStart, ToolCallArgumentsDelta, ToolCallComplete, Done
        assert!(
            events
                .iter()
                .any(|e| matches!(e, StreamEvent::ToolCallStart { name, .. } if name == "exec"))
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, StreamEvent::ToolCallArgumentsDelta { .. }))
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, StreamEvent::ToolCallComplete { index } if *index == 0))
        );
        assert!(events.iter().any(|e| matches!(e, StreamEvent::Done(_))));
    }

    #[tokio::test]
    async fn responses_sse_stream_error() {
        let sse = "\
            data: {\"type\":\"error\",\"error\":{\"message\":\"rate limited\"}}\n\n";
        let (base_url, _) = start_responses_mock(sse.to_string()).await;

        let provider = OpenAiProvider::new(
            Secret::new("test-key".to_string()),
            "test-model".to_string(),
            base_url,
        )
        .with_wire_api(WireApi::Responses);

        let mut stream = provider.stream_with_tools(vec![ChatMessage::user("hi")], vec![]);
        // First event is ProviderRaw, second is the Error.
        let first = stream.next().await.expect("should emit ProviderRaw");
        assert!(matches!(first, StreamEvent::ProviderRaw(_)));
        let second = stream.next().await.expect("should emit error");
        assert!(matches!(second, StreamEvent::Error(msg) if msg == "rate limited"));
    }

    // ============================================================
    // Tests for reasoning effort
    // ============================================================

    #[test]
    fn apply_reasoning_effort_chat_injects_top_level_field() {
        let mut provider = OpenAiProvider::new(
            Secret::new("test-key".into()),
            "o3".into(),
            "https://api.openai.com/v1".into(),
        );
        provider.reasoning_effort = Some(moltis_agents::model::ReasoningEffort::High);

        let mut body = serde_json::json!({ "model": "o3", "messages": [] });
        provider.apply_reasoning_effort_chat(&mut body);

        assert_eq!(body["reasoning_effort"], "high");
        assert!(body.get("reasoning").is_none());
    }

    #[test]
    fn apply_reasoning_effort_responses_injects_nested_field() {
        let mut provider = OpenAiProvider::new(
            Secret::new("test-key".into()),
            "o3".into(),
            "https://api.openai.com/v1".into(),
        );
        provider.reasoning_effort = Some(moltis_agents::model::ReasoningEffort::Medium);

        let mut body = serde_json::json!({ "model": "o3", "input": [] });
        provider.apply_reasoning_effort_responses(&mut body);

        assert_eq!(body["reasoning"]["effort"], "medium");
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn apply_reasoning_effort_skipped_when_none() {
        let provider = OpenAiProvider::new(
            Secret::new("test-key".into()),
            "o3".into(),
            "https://api.openai.com/v1".into(),
        );
        let mut body = serde_json::json!({ "model": "o3", "messages": [] });
        provider.apply_reasoning_effort_chat(&mut body);
        assert!(body.get("reasoning_effort").is_none());
        assert!(body.get("reasoning").is_none());
    }

    #[test]
    fn with_reasoning_effort_creates_new_provider() {
        use moltis_agents::model::{LlmProvider, ReasoningEffort};
        let provider = Arc::new(OpenAiProvider::new(
            Secret::new("test-key".into()),
            "o3".into(),
            "https://api.openai.com/v1".into(),
        ));
        assert!(provider.reasoning_effort().is_none());

        let with_effort = provider
            .with_reasoning_effort(ReasoningEffort::Medium)
            .expect("openai supports reasoning_effort");
        assert_eq!(
            with_effort.reasoning_effort(),
            Some(ReasoningEffort::Medium)
        );
        assert_eq!(with_effort.id(), "o3");
    }

    #[test]
    fn openrouter_anthropic_injects_cache_control_on_system_and_last_user() {
        let provider = OpenAiProvider::new_with_name(
            Secret::new("key".into()),
            "anthropic/claude-sonnet-4-20250514".into(),
            "https://openrouter.ai/api/v1".into(),
            "openrouter".into(),
        );

        let mut messages = vec![
            serde_json::json!({"role": "system", "content": "You are helpful."}),
            serde_json::json!({"role": "user", "content": "hello"}),
            serde_json::json!({"role": "assistant", "content": "hi there"}),
            serde_json::json!({"role": "user", "content": "bye"}),
        ];

        provider.apply_openrouter_cache_control(&mut messages);

        // System message converted to content-block array with cache_control.
        let sys_content = messages[0]["content"].as_array().expect("should be array");
        assert_eq!(sys_content.len(), 1);
        assert_eq!(sys_content[0]["text"], "You are helpful.");
        assert_eq!(sys_content[0]["cache_control"]["type"], "ephemeral");

        // First user message should NOT have cache_control.
        assert_eq!(messages[1]["content"], "hello");

        // Last user message should have cache_control.
        let last_user_content = messages[3]["content"].as_array().expect("should be array");
        assert_eq!(last_user_content[0]["text"], "bye");
        assert_eq!(last_user_content[0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn non_openrouter_skips_cache_control() {
        let provider = OpenAiProvider::new(
            Secret::new("key".into()),
            "gpt-4o".into(),
            "https://api.openai.com/v1".into(),
        );

        let mut messages = vec![
            serde_json::json!({"role": "system", "content": "sys"}),
            serde_json::json!({"role": "user", "content": "hello"}),
        ];

        provider.apply_openrouter_cache_control(&mut messages);

        // Nothing should change for non-OpenRouter providers.
        assert_eq!(messages[0]["content"], "sys");
        assert_eq!(messages[1]["content"], "hello");
    }

    #[test]
    fn openrouter_non_anthropic_model_skips_cache_control() {
        let provider = OpenAiProvider::new_with_name(
            Secret::new("key".into()),
            "openai/gpt-4o".into(),
            "https://openrouter.ai/api/v1".into(),
            "openrouter".into(),
        );

        let mut messages = vec![serde_json::json!({"role": "user", "content": "hello"})];

        provider.apply_openrouter_cache_control(&mut messages);
        assert_eq!(messages[0]["content"], "hello");
    }
}
