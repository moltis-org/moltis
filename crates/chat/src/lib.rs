use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};

use {
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
    serde_json::Value,
    tokio::{
        sync::{Mutex, OnceCell, OwnedSemaphorePermit, RwLock, Semaphore, mpsc},
        task::AbortHandle,
    },
    tokio_stream::StreamExt,
    tracing::{debug, info, warn},
};

use moltis_config::{MessageQueueMode, ToolMode};

use {
    moltis_agents::{
        AgentRunError, ChatMessage, ContentPart, UserContent,
        model::{StreamEvent, values_to_chat_messages},
        multimodal::parse_data_uri,
        prompt::{
            PromptHostRuntimeContext, PromptRuntimeContext, PromptSandboxRuntimeContext,
            VOICE_REPLY_SUFFIX, build_system_prompt_minimal_runtime,
            build_system_prompt_with_session_runtime,
        },
        runner::{RunnerEvent, run_agent_loop_streaming},
        tool_registry::{AgentTool, ToolRegistry},
    },
    moltis_providers::{ProviderRegistry, raw_model_id},
    moltis_sessions::{
        ContentBlock, MessageContent, PersistedMessage,
        metadata::{SessionEntry, SqliteSessionMetadata},
        store::SessionStore,
    },
    moltis_skills::discover::SkillDiscoverer,
    moltis_tools::policy::{ToolPolicy, profile_tools},
};

pub mod chat_error;
pub mod error;
pub mod runtime;

pub use runtime::{ChatRuntime, TtsOverride};
use {
    chat_error::parse_chat_error,
    moltis_service_traits::{ChatService, ModelService, ServiceError, ServiceResult},
};

/// Extract preview text from a single message JSON value.
fn extract_preview_from_value(msg: &Value) -> Option<String> {
    fn message_text(msg: &Value) -> Option<String> {
        let text = if let Some(s) = msg.get("content").and_then(|v| v.as_str()) {
            s.to_string()
        } else if let Some(blocks) = msg.get("content").and_then(|v| v.as_array()) {
            blocks
                .iter()
                .filter_map(|b| {
                    if b.get("type").and_then(|v| v.as_str()) == Some("text") {
                        b.get("text").and_then(|v| v.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            return None;
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }
    fn truncate_preview(s: &str, max: usize) -> String {
        if s.len() <= max {
            s.to_string()
        } else {
            format!("{}…", &s[..s.floor_char_boundary(max)])
        }
    }
    message_text(msg).map(|t| truncate_preview(&t, 200))
}

/// Placeholder to match the old `BroadcastOpts` pattern. All fields are ignored;
/// the trait's `broadcast` always uses default behaviour.
#[derive(Default)]
pub struct BroadcastOpts {
    pub drop_if_slow: bool,
    pub state_version: Option<()>,
}

/// Compatibility shim: delegates to [`ChatRuntime::broadcast`].
///
/// Matches the old `broadcast(state, topic, payload, opts)` signature so that
/// the hundreds of call sites inside this crate need no change.
async fn broadcast(
    state: &Arc<dyn ChatRuntime>,
    event: &str,
    payload: Value,
    _opts: BroadcastOpts,
) {
    state.broadcast(event, payload).await;
}

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, histogram, labels, llm as llm_metrics};

/// Convert session-crate `MessageContent` to agents-crate `UserContent`.
///
/// The two types have different image representations:
/// - `ContentBlock::ImageUrl` stores a data URI string
/// - `ContentPart::Image` stores separated `media_type` + `data` fields
fn to_user_content(mc: &MessageContent) -> UserContent {
    match mc {
        MessageContent::Text(text) => UserContent::Text(text.clone()),
        MessageContent::Multimodal(blocks) => {
            let parts: Vec<ContentPart> = blocks
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text } => Some(ContentPart::Text(text.clone())),
                    ContentBlock::ImageUrl { image_url } => match parse_data_uri(&image_url.url) {
                        Some((media_type, data)) => {
                            debug!(
                                media_type,
                                data_len = data.len(),
                                "to_user_content: parsed image from data URI"
                            );
                            Some(ContentPart::Image {
                                media_type: media_type.to_string(),
                                data: data.to_string(),
                            })
                        },
                        None => {
                            warn!(
                                url_prefix = truncate_at_char_boundary(&image_url.url, 80),
                                "to_user_content: failed to parse data URI, dropping image"
                            );
                            None
                        },
                    },
                })
                .collect();
            let text_count = parts
                .iter()
                .filter(|p| matches!(p, ContentPart::Text(_)))
                .count();
            let image_count = parts
                .iter()
                .filter(|p| matches!(p, ContentPart::Image { .. }))
                .count();
            debug!(
                text_count,
                image_count,
                total_blocks = blocks.len(),
                "to_user_content: converted multimodal content"
            );
            UserContent::Multimodal(parts)
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum ReplyMedium {
    Text,
    Voice,
}

#[must_use]
fn truncate_at_char_boundary(text: &str, max_bytes: usize) -> &str {
    &text[..text.floor_char_boundary(max_bytes)]
}

#[derive(Debug, Deserialize)]
struct InputChannelMeta {
    #[serde(default)]
    message_kind: Option<InputMessageKind>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum InputMessageKind {
    Text,
    Voice,
    Audio,
    Photo,
    Document,
    Video,
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum InputMediumParam {
    Text,
    Voice,
}

/// Typed broadcast payload for the "final" chat event.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatFinalBroadcast {
    run_id: String,
    session_key: String,
    state: &'static str,
    text: String,
    model: String,
    provider: String,
    input_tokens: u32,
    output_tokens: u32,
    duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_input_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_output_tokens: Option<u32>,
    message_index: usize,
    reply_medium: ReplyMedium,
    #[serde(skip_serializing_if = "Option::is_none")]
    iterations: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls_made: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    audio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    audio_warning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seq: Option<u64>,
}

/// Typed broadcast payload for the "error" chat event.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatErrorBroadcast {
    run_id: String,
    session_key: String,
    state: &'static str,
    error: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    seq: Option<u64>,
}

struct AssistantTurnOutput {
    text: String,
    input_tokens: u32,
    output_tokens: u32,
    duration_ms: u64,
    request_input_tokens: u32,
    request_output_tokens: u32,
    audio_path: Option<String>,
    reasoning: Option<String>,
    llm_api_response: Option<Value>,
}

#[derive(Debug, Clone, Copy, Default)]
struct SessionTokenUsage {
    session_input_tokens: u64,
    session_output_tokens: u64,
    current_request_input_tokens: u64,
    current_request_output_tokens: u64,
}

#[must_use]
fn session_token_usage_from_messages(messages: &[Value]) -> SessionTokenUsage {
    let session_input_tokens = messages
        .iter()
        .filter_map(|m| m.get("inputTokens").and_then(|v| v.as_u64()))
        .sum();
    let session_output_tokens = messages
        .iter()
        .filter_map(|m| m.get("outputTokens").and_then(|v| v.as_u64()))
        .sum();

    let (current_request_input_tokens, current_request_output_tokens) = messages
        .iter()
        .rev()
        .find(|m| m.get("role").and_then(|v| v.as_str()) == Some("assistant"))
        .map_or((0, 0), |m| {
            let input = m
                .get("requestInputTokens")
                .and_then(|v| v.as_u64())
                .or_else(|| m.get("inputTokens").and_then(|v| v.as_u64()))
                .unwrap_or(0);
            let output = m
                .get("requestOutputTokens")
                .and_then(|v| v.as_u64())
                .or_else(|| m.get("outputTokens").and_then(|v| v.as_u64()))
                .unwrap_or(0);
            (input, output)
        });

    SessionTokenUsage {
        session_input_tokens,
        session_output_tokens,
        current_request_input_tokens,
        current_request_output_tokens,
    }
}

#[must_use]
fn estimate_text_tokens(text: &str) -> u64 {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return 0;
    }
    let bytes = trimmed.len() as u64;
    bytes.div_ceil(4).max(1)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn normalize_model_key(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut last_was_separator = true;

    for ch in value.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch.to_ascii_lowercase());
            last_was_separator = false;
            continue;
        }

        if !last_was_separator {
            normalized.push(' ');
            last_was_separator = true;
        }
    }

    normalized.trim().to_string()
}

fn normalize_provider_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn subscription_provider_rank(provider_name: &str) -> usize {
    match normalize_provider_key(provider_name).as_str() {
        "openai-codex" | "github-copilot" => 0,
        _ => 1,
    }
}

#[allow(dead_code)]
fn is_allowlist_exempt_provider(provider_name: &str) -> bool {
    matches!(
        normalize_provider_key(provider_name).as_str(),
        "local-llm" | "ollama"
    )
}

/// Returns `true` if the model matches the allowlist patterns.
/// An empty pattern list means all models are allowed.
/// Matching is case-insensitive against the full model ID, raw model ID, and
/// display name:
/// - patterns with digits use exact-or-suffix matching (boundary aware)
/// - patterns without digits use substring matching
///
/// This keeps precise model pins like "gpt 5.2" from matching variants such as
/// "gpt-5.2-chat-latest", while still allowing broad buckets like "mini".
#[allow(dead_code)]
fn allowlist_pattern_matches_key(pattern: &str, key: &str) -> bool {
    if pattern.chars().any(|ch| ch.is_ascii_digit()) {
        if key == pattern {
            return true;
        }
        return key
            .strip_suffix(pattern)
            .is_some_and(|prefix| prefix.ends_with(' '));
    }
    key.contains(pattern)
}

#[allow(dead_code)]
pub fn model_matches_allowlist(model: &moltis_providers::ModelInfo, patterns: &[String]) -> bool {
    if patterns.is_empty() {
        return true;
    }
    if is_allowlist_exempt_provider(&model.provider) {
        return true;
    }
    let full = normalize_model_key(&model.id);
    let raw = normalize_model_key(raw_model_id(&model.id));
    let display = normalize_model_key(&model.display_name);
    patterns.iter().any(|p| {
        allowlist_pattern_matches_key(p, &full)
            || allowlist_pattern_matches_key(p, &raw)
            || allowlist_pattern_matches_key(p, &display)
    })
}

#[allow(dead_code)]
pub fn model_matches_allowlist_with_provider(
    model: &moltis_providers::ModelInfo,
    provider_name: Option<&str>,
    patterns: &[String],
) -> bool {
    if provider_name.is_some_and(is_allowlist_exempt_provider) {
        return true;
    }
    model_matches_allowlist(model, patterns)
}

fn provider_filter_from_params(params: &Value) -> Option<String> {
    params
        .get("provider")
        .and_then(|v| v.as_str())
        .map(normalize_provider_key)
        .filter(|v| !v.is_empty())
}

fn provider_matches_filter(model_provider: &str, provider_filter: Option<&str>) -> bool {
    provider_filter.is_none_or(|expected| normalize_provider_key(model_provider) == expected)
}

fn probe_max_parallel_per_provider(params: &Value) -> usize {
    params
        .get("maxParallelPerProvider")
        .and_then(|v| v.as_u64())
        .map(|v| v.clamp(1, 8) as usize)
        .unwrap_or(1)
}

fn provider_model_entry(model_id: &str, display_name: &str) -> Value {
    serde_json::json!({
        "modelId": model_id,
        "displayName": display_name,
    })
}

fn push_provider_model(
    grouped: &mut BTreeMap<String, Vec<Value>>,
    provider_name: &str,
    model_id: &str,
    display_name: &str,
) {
    if provider_name.trim().is_empty() || model_id.trim().is_empty() {
        return;
    }
    grouped
        .entry(provider_name.to_string())
        .or_default()
        .push(provider_model_entry(model_id, display_name));
}

const PROBE_RATE_LIMIT_INITIAL_BACKOFF_MS: u64 = 1_000;
const PROBE_RATE_LIMIT_MAX_BACKOFF_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy)]
struct ProbeRateLimitState {
    backoff_ms: u64,
    until: Instant,
}

#[derive(Debug, Default)]
struct ProbeRateLimiter {
    by_provider: Mutex<HashMap<String, ProbeRateLimitState>>,
}

impl ProbeRateLimiter {
    async fn remaining_backoff(&self, provider: &str) -> Option<Duration> {
        let map = self.by_provider.lock().await;
        map.get(provider).and_then(|state| {
            let now = Instant::now();
            (state.until > now).then_some(state.until - now)
        })
    }

    async fn mark_rate_limited(&self, provider: &str) -> Duration {
        let mut map = self.by_provider.lock().await;
        let next_backoff_ms =
            next_probe_rate_limit_backoff_ms(map.get(provider).map(|s| s.backoff_ms));
        let delay = Duration::from_millis(next_backoff_ms);
        let state = ProbeRateLimitState {
            backoff_ms: next_backoff_ms,
            until: Instant::now() + delay,
        };
        let _ = map.insert(provider.to_string(), state);
        delay
    }

    async fn clear(&self, provider: &str) {
        let mut map = self.by_provider.lock().await;
        let _ = map.remove(provider);
    }
}

fn next_probe_rate_limit_backoff_ms(previous_ms: Option<u64>) -> u64 {
    previous_ms
        .map(|ms| ms.saturating_mul(2))
        .unwrap_or(PROBE_RATE_LIMIT_INITIAL_BACKOFF_MS)
        .clamp(
            PROBE_RATE_LIMIT_INITIAL_BACKOFF_MS,
            PROBE_RATE_LIMIT_MAX_BACKOFF_MS,
        )
}

fn is_probe_rate_limited_error(error_obj: &Value, error_text: &str) -> bool {
    if error_obj.get("type").and_then(|v| v.as_str()) == Some("rate_limit_exceeded") {
        return true;
    }

    let lower = error_text.to_ascii_lowercase();
    lower.contains("status=429")
        || lower.contains("http 429")
        || lower.contains("too many requests")
        || lower.contains("rate limit")
        || lower.contains("quota exceeded")
}

#[derive(Debug)]
struct ProbeProviderLimiter {
    permits_per_provider: usize,
    by_provider: Mutex<HashMap<String, Arc<Semaphore>>>,
}

impl ProbeProviderLimiter {
    fn new(permits_per_provider: usize) -> Self {
        Self {
            permits_per_provider,
            by_provider: Mutex::new(HashMap::new()),
        }
    }

    async fn acquire(
        &self,
        provider: &str,
    ) -> Result<OwnedSemaphorePermit, tokio::sync::AcquireError> {
        let provider_sem = {
            let mut map = self.by_provider.lock().await;
            Arc::clone(
                map.entry(provider.to_string())
                    .or_insert_with(|| Arc::new(Semaphore::new(self.permits_per_provider))),
            )
        };

        provider_sem.acquire_owned().await
    }
}

#[derive(Debug)]
enum ProbeStatus {
    Supported,
    Unsupported { detail: String, provider: String },
    Error { message: String },
}

#[derive(Debug)]
struct ProbeOutcome {
    model_id: String,
    display_name: String,
    provider_name: String,
    status: ProbeStatus,
}

/// Run a single model probe: acquire concurrency permits, respect rate-limit
/// backoff, send a "ping" completion, and classify the result.
async fn run_single_probe(
    model_id: String,
    display_name: String,
    provider_name: String,
    provider: Arc<dyn moltis_agents::model::LlmProvider>,
    limiter: Arc<Semaphore>,
    provider_limiter: Arc<ProbeProviderLimiter>,
    rate_limiter: Arc<ProbeRateLimiter>,
) -> ProbeOutcome {
    let _permit = match limiter.acquire_owned().await {
        Ok(permit) => permit,
        Err(_) => {
            return ProbeOutcome {
                model_id,
                display_name,
                provider_name,
                status: ProbeStatus::Error {
                    message: "probe limiter closed".to_string(),
                },
            };
        },
    };
    let _provider_permit = match provider_limiter.acquire(&provider_name).await {
        Ok(permit) => permit,
        Err(_) => {
            return ProbeOutcome {
                model_id,
                display_name,
                provider_name,
                status: ProbeStatus::Error {
                    message: "provider probe limiter closed".to_string(),
                },
            };
        },
    };

    if let Some(wait_for) = rate_limiter.remaining_backoff(&provider_name).await {
        debug!(
            provider = %provider_name,
            model = %model_id,
            wait_ms = wait_for.as_millis() as u64,
            "skipping model probe while provider is in rate-limit backoff"
        );
        return ProbeOutcome {
            model_id,
            display_name,
            provider_name,
            status: ProbeStatus::Error {
                message: format!(
                    "probe skipped due provider backoff ({}ms remaining)",
                    wait_for.as_millis()
                ),
            },
        };
    }

    let probe = [ChatMessage::user("ping")];
    let completion =
        tokio::time::timeout(Duration::from_secs(20), provider.complete(&probe, &[])).await;

    match completion {
        Ok(Ok(_)) => {
            rate_limiter.clear(&provider_name).await;
            ProbeOutcome {
                model_id,
                display_name,
                provider_name,
                status: ProbeStatus::Supported,
            }
        },
        Ok(Err(err)) => {
            let error_text = err.to_string();
            let error_obj = parse_chat_error(&error_text, Some(provider_name.as_str()));
            if is_probe_rate_limited_error(&error_obj, &error_text) {
                let backoff = rate_limiter.mark_rate_limited(&provider_name).await;
                let detail = error_obj
                    .get("detail")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Too many requests while probing model support");
                warn!(
                    provider = %provider_name,
                    model = %model_id,
                    backoff_ms = backoff.as_millis() as u64,
                    "model probe rate limited, applying provider backoff"
                );
                return ProbeOutcome {
                    model_id,
                    display_name,
                    provider_name,
                    status: ProbeStatus::Error {
                        message: format!("{detail} (probe backoff {}ms)", backoff.as_millis()),
                    },
                };
            }

            rate_limiter.clear(&provider_name).await;
            let is_unsupported =
                error_obj.get("type").and_then(|v| v.as_str()) == Some("unsupported_model");

            if is_unsupported {
                let detail = error_obj
                    .get("detail")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Model is not supported for this account/provider")
                    .to_string();
                let parsed_provider = error_obj
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .unwrap_or(provider_name.as_str())
                    .to_string();
                ProbeOutcome {
                    model_id,
                    display_name,
                    provider_name,
                    status: ProbeStatus::Unsupported {
                        detail,
                        provider: parsed_provider,
                    },
                }
            } else {
                ProbeOutcome {
                    model_id,
                    display_name,
                    provider_name,
                    status: ProbeStatus::Error {
                        message: error_text,
                    },
                }
            }
        },
        Err(_) => ProbeOutcome {
            model_id,
            display_name,
            provider_name,
            status: ProbeStatus::Error {
                message: "probe timeout after 20s".to_string(),
            },
        },
    }
}

fn parse_input_medium(params: &Value) -> Option<ReplyMedium> {
    match params
        .get("_input_medium")
        .cloned()
        .and_then(|v| serde_json::from_value::<InputMediumParam>(v).ok())
    {
        Some(InputMediumParam::Voice) => Some(ReplyMedium::Voice),
        Some(InputMediumParam::Text) => Some(ReplyMedium::Text),
        _ => None,
    }
}

fn explicit_reply_medium_override(text: &str) -> Option<ReplyMedium> {
    let lower = text.to_lowercase();
    let voice_markers = [
        "talk to me",
        "say it",
        "say this",
        "speak",
        "voice message",
        "respond with voice",
        "reply with voice",
        "audio reply",
    ];
    if voice_markers.iter().any(|m| lower.contains(m)) {
        return Some(ReplyMedium::Voice);
    }

    let text_markers = [
        "text only",
        "reply in text",
        "respond in text",
        "don't use voice",
        "do not use voice",
        "no audio",
    ];
    if text_markers.iter().any(|m| lower.contains(m)) {
        return Some(ReplyMedium::Text);
    }

    None
}

fn infer_reply_medium(params: &Value, text: &str) -> ReplyMedium {
    if let Some(explicit) = explicit_reply_medium_override(text) {
        return explicit;
    }

    if let Some(input_medium) = parse_input_medium(params) {
        return input_medium;
    }

    if let Some(channel) = params
        .get("channel")
        .cloned()
        .and_then(|v| serde_json::from_value::<InputChannelMeta>(v).ok())
        && channel.message_kind == Some(InputMessageKind::Voice)
    {
        return ReplyMedium::Voice;
    }

    ReplyMedium::Text
}

fn runtime_datetime_prompt_tail(runtime_context: Option<&PromptRuntimeContext>) -> Option<String> {
    let runtime = runtime_context?;
    if let Some(time) = runtime
        .host
        .time
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        return Some(format!("\nThe current user datetime is {time}.\n"));
    }
    runtime
        .host
        .today
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(|today| format!("\nThe current user date is {today}.\n"))
}

fn apply_voice_reply_suffix(
    system_prompt: String,
    desired_reply_medium: ReplyMedium,
    runtime_context: Option<&PromptRuntimeContext>,
) -> String {
    if desired_reply_medium != ReplyMedium::Voice {
        return system_prompt;
    }

    if let Some(tail) = runtime_datetime_prompt_tail(runtime_context)
        && let Some(prefix) = system_prompt.strip_suffix(&tail)
    {
        return format!("{prefix}{VOICE_REPLY_SUFFIX}{tail}");
    }

    format!("{system_prompt}{VOICE_REPLY_SUFFIX}")
}

fn parse_explicit_shell_command(text: &str) -> Option<&str> {
    let trimmed = text.trim_start();
    let rest = trimmed.strip_prefix("/sh")?;
    let first = rest.chars().next()?;
    if !first.is_whitespace() {
        return None;
    }
    let command = &rest[first.len_utf8()..];
    if command.trim().is_empty() {
        None
    } else {
        Some(command)
    }
}

fn capped_tool_result_payload(result: &Value, max_len: usize) -> Value {
    let mut capped = result.clone();
    for field in &["stdout", "stderr"] {
        if let Some(text) = capped.get(*field).and_then(Value::as_str)
            && text.len() > max_len
        {
            let truncated = format!(
                "{}\n\n... [truncated — {} bytes total]",
                truncate_at_char_boundary(text, max_len),
                text.len()
            );
            capped[*field] = Value::String(truncated);
        }
    }
    capped
}

fn shell_reply_text_from_exec_result(result: &Value) -> String {
    let stdout = result
        .get("stdout")
        .and_then(Value::as_str)
        .map(str::trim_end)
        .unwrap_or("");
    if !stdout.is_empty() {
        return stdout.to_string();
    }

    let stderr = result
        .get("stderr")
        .and_then(Value::as_str)
        .map(str::trim_end)
        .unwrap_or("");
    if !stderr.is_empty() {
        return stderr.to_string();
    }

    let exit_code = result.get("exit_code").and_then(Value::as_i64).or_else(|| {
        result
            .get("exit_code")
            .and_then(Value::as_u64)
            .and_then(|code| i64::try_from(code).ok())
    });
    match exit_code {
        Some(code) if code != 0 => format!("Command failed (exit {code})."),
        _ => String::new(),
    }
}

fn is_safe_user_audio_filename(filename: &str) -> bool {
    !filename.is_empty()
        && filename.len() <= 255
        && filename
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
}

fn user_audio_path_from_params(params: &Value, session_key: &str) -> Option<String> {
    let filename = params
        .get("_audio_filename")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?;

    if !is_safe_user_audio_filename(filename) {
        warn!(
            session = %session_key,
            filename = filename,
            "ignoring invalid user audio filename"
        );
        return None;
    }

    let key = SessionStore::key_to_filename(session_key);
    Some(format!("media/{key}/{filename}"))
}

fn detect_runtime_shell() -> Option<String> {
    let candidate = std::env::var("SHELL")
        .ok()
        .or_else(|| std::env::var("COMSPEC").ok())?;
    let trimmed = candidate.trim();
    if trimmed.is_empty() {
        return None;
    }
    let name = Path::new(trimmed)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or(trimmed)
        .trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

async fn detect_host_sudo_access() -> (Option<bool>, Option<String>) {
    #[cfg(not(unix))]
    {
        return (None, Some("unsupported".to_string()));
    }

    #[cfg(unix)]
    {
        let output = tokio::process::Command::new("sudo")
            .arg("-n")
            .arg("true")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await;

        match output {
            Ok(out) if out.status.success() => (Some(true), Some("passwordless".to_string())),
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).to_lowercase();
                if stderr.contains("a password is required") {
                    (Some(false), Some("requires_password".to_string()))
                } else if stderr.contains("not in the sudoers")
                    || stderr.contains("is not in the sudoers")
                    || stderr.contains("is not allowed to run sudo")
                    || stderr.contains("may not run sudo")
                {
                    (Some(false), Some("denied".to_string()))
                } else {
                    (None, Some("unknown".to_string()))
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                (None, Some("not_installed".to_string()))
            },
            Err(_) => (None, Some("unknown".to_string())),
        }
    }
}

async fn detect_host_root_user() -> Option<bool> {
    #[cfg(not(unix))]
    {
        return None;
    }

    #[cfg(unix)]
    {
        if let Some(uid) = std::env::var("EUID")
            .ok()
            .or_else(|| std::env::var("UID").ok())
            .and_then(|raw| raw.trim().parse::<u32>().ok())
        {
            return Some(uid == 0);
        }
        if let Ok(user) = std::env::var("USER") {
            let trimmed = user.trim();
            if !trimmed.is_empty() {
                return Some(trimmed == "root");
            }
        }
        let output = tokio::process::Command::new("id")
            .arg("-u")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .await
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let uid_text = String::from_utf8_lossy(&output.stdout);
        uid_text.trim().parse::<u32>().ok().map(|uid| uid == 0)
    }
}

/// Pre-loaded persona data used to build the system prompt.
struct PromptPersona {
    config: moltis_config::MoltisConfig,
    identity: moltis_config::AgentIdentity,
    user: moltis_config::UserProfile,
    soul_text: Option<String>,
    agents_text: Option<String>,
    tools_text: Option<String>,
    memory_text: Option<String>,
}

fn resolve_prompt_agent_id(session_entry: Option<&SessionEntry>) -> String {
    let Some(entry) = session_entry else {
        return "main".to_string();
    };
    let Some(agent_id) = entry
        .agent_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return "main".to_string();
    };
    if agent_id == "main" {
        return "main".to_string();
    }
    if moltis_config::agent_workspace_dir(agent_id).exists() {
        return agent_id.to_string();
    }
    warn!(
        session = %entry.key,
        agent_id,
        "session references unknown agent workspace, falling back to main prompt persona"
    );
    "main".to_string()
}

/// Load identity, user profile, soul, and workspace text for one agent.
///
/// Both `run_with_tools` and `run_streaming` need the same persona data;
/// this function avoids duplicating the merge logic.
fn load_prompt_persona_for_agent(agent_id: &str) -> PromptPersona {
    let config = moltis_config::discover_and_load();
    let mut identity = config.identity.clone();
    if let Some(file_identity) = moltis_config::load_identity_for_agent(agent_id) {
        if file_identity.name.is_some() {
            identity.name = file_identity.name;
        }
        if file_identity.emoji.is_some() {
            identity.emoji = file_identity.emoji;
        }
        if file_identity.theme.is_some() {
            identity.theme = file_identity.theme;
        }
    }
    let mut user = config.user.clone();
    if let Some(file_user) = moltis_config::load_user() {
        if file_user.name.is_some() {
            user.name = file_user.name;
        }
        if file_user.timezone.is_some() {
            user.timezone = file_user.timezone;
        }
    }
    PromptPersona {
        config,
        identity,
        user,
        soul_text: moltis_config::load_soul_for_agent(agent_id),
        agents_text: moltis_config::load_agents_md_for_agent(agent_id),
        tools_text: moltis_config::load_tools_md_for_agent(agent_id),
        memory_text: moltis_config::load_memory_md_for_agent(agent_id),
    }
}

fn load_prompt_persona_for_session(session_entry: Option<&SessionEntry>) -> PromptPersona {
    let agent_id = resolve_prompt_agent_id(session_entry);
    load_prompt_persona_for_agent(&agent_id)
}

#[derive(Default)]
struct ChannelRuntimeContext {
    surface: Option<String>,
    session_kind: Option<String>,
    channel_type: Option<String>,
    channel_account_id: Option<String>,
    channel_chat_id: Option<String>,
    channel_chat_type: Option<String>,
}

fn infer_channel_chat_type(channel_type: &str, chat_id: &str) -> Option<String> {
    if channel_type.eq_ignore_ascii_case("telegram") {
        if chat_id.starts_with("-100") {
            return Some("channel_or_supergroup".to_string());
        }
        if chat_id.starts_with('-') {
            return Some("group".to_string());
        }
        return Some("private".to_string());
    }
    None
}

fn resolve_channel_runtime_context(
    session_key: &str,
    session_entry: Option<&SessionEntry>,
) -> ChannelRuntimeContext {
    if session_key == "cron:heartbeat" {
        return ChannelRuntimeContext {
            surface: Some("heartbeat".to_string()),
            session_kind: Some("cron".to_string()),
            ..Default::default()
        };
    }

    if session_key.starts_with("cron:") {
        return ChannelRuntimeContext {
            surface: Some("cron".to_string()),
            session_kind: Some("cron".to_string()),
            ..Default::default()
        };
    }

    if let Some(binding_json) = session_entry.and_then(|entry| entry.channel_binding.as_deref())
        && let Ok(binding) =
            serde_json::from_str::<moltis_channels::ChannelReplyTarget>(binding_json)
    {
        let channel_type = binding.channel_type.as_str().to_string();
        let chat_id = binding.chat_id;
        return ChannelRuntimeContext {
            surface: Some(channel_type.clone()),
            session_kind: Some("channel".to_string()),
            channel_chat_type: infer_channel_chat_type(&channel_type, &chat_id),
            channel_type: Some(channel_type),
            channel_account_id: Some(binding.account_id),
            channel_chat_id: Some(chat_id),
        };
    }

    ChannelRuntimeContext {
        surface: Some("web".to_string()),
        session_kind: Some("web".to_string()),
        ..Default::default()
    }
}

async fn build_prompt_runtime_context(
    state: &Arc<dyn ChatRuntime>,
    provider: &Arc<dyn moltis_agents::model::LlmProvider>,
    session_key: &str,
    session_entry: Option<&SessionEntry>,
) -> PromptRuntimeContext {
    let data_dir = moltis_config::data_dir();
    let data_dir_display = data_dir.display().to_string();

    let sudo_fut = detect_host_sudo_access();
    let sandbox_fut = async {
        if let Some(router) = state.sandbox_router() {
            let is_sandboxed = router.is_sandboxed(session_key).await;
            let config = router.config();
            let backend_name = router.backend_name();
            let workspace_mount = config.workspace_mount.to_string();
            let workspace_path = (workspace_mount != "none").then(|| data_dir_display.clone());
            Some(PromptSandboxRuntimeContext {
                exec_sandboxed: is_sandboxed,
                mode: Some(config.mode.to_string()),
                backend: Some(backend_name.to_string()),
                scope: Some(config.scope.to_string()),
                image: Some(router.resolve_image(session_key, None).await),
                home: Some("/home/sandbox".to_string()),
                workspace_mount: Some(workspace_mount),
                workspace_path,
                no_network: prompt_sandbox_no_network_state(backend_name, config.no_network),
                session_override: session_entry.and_then(|entry| entry.sandbox_enabled),
            })
        } else {
            Some(PromptSandboxRuntimeContext {
                exec_sandboxed: false,
                mode: Some("off".to_string()),
                backend: Some("none".to_string()),
                scope: None,
                image: None,
                home: None,
                workspace_mount: None,
                workspace_path: None,
                no_network: None,
                session_override: None,
            })
        }
    };

    let ((sudo_non_interactive, sudo_status), sandbox_ctx) = tokio::join!(sudo_fut, sandbox_fut);

    let configured_timezone = state
        .sandbox_router()
        .and_then(|r| r.config().timezone.clone());
    let timezone = Some(server_prompt_timezone(configured_timezone.as_deref()));

    let location = state
        .cached_location()
        .await
        .as_ref()
        .map(|loc| loc.to_string());
    let channel_context = resolve_channel_runtime_context(session_key, session_entry);

    let mut host_ctx = PromptHostRuntimeContext {
        host: Some(state.hostname().to_string()),
        os: Some(std::env::consts::OS.to_string()),
        arch: Some(std::env::consts::ARCH.to_string()),
        shell: detect_runtime_shell(),
        time: None,
        provider: Some(provider.name().to_string()),
        model: Some(provider.id().to_string()),
        session_key: Some(session_key.to_string()),
        surface: channel_context.surface,
        session_kind: channel_context.session_kind,
        channel_type: channel_context.channel_type,
        channel_account_id: channel_context.channel_account_id,
        channel_chat_id: channel_context.channel_chat_id,
        channel_chat_type: channel_context.channel_chat_type,
        data_dir: Some(data_dir_display),
        sudo_non_interactive,
        sudo_status,
        timezone,
        location,
        ..Default::default()
    };
    refresh_runtime_prompt_time(&mut host_ctx);

    PromptRuntimeContext {
        host: host_ctx,
        sandbox: sandbox_ctx,
    }
}

fn refresh_runtime_prompt_time(host: &mut PromptHostRuntimeContext) {
    host.time = Some(prompt_now_for_timezone(host.timezone.as_deref()));
    host.today = Some(prompt_today_for_timezone(host.timezone.as_deref()));
}

fn server_prompt_timezone(configured_timezone: Option<&str>) -> String {
    if let Some(timezone) = configured_timezone
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return timezone.to_string();
    }
    "server-local".to_string()
}

fn prompt_now_for_timezone(timezone: Option<&str>) -> String {
    #[cfg(any(feature = "web-ui", feature = "push-notifications"))]
    {
        use chrono::{Local, Utc};

        let trimmed_timezone = timezone.map(str::trim).filter(|value| !value.is_empty());

        if let Some(tz) = trimmed_timezone.and_then(|name| name.parse::<chrono_tz::Tz>().ok()) {
            return Utc::now()
                .with_timezone(&tz)
                .format("%Y-%m-%d %H:%M:%S %Z")
                .to_string();
        }

        // Fallback to server local clock when timezone is unknown/non-IANA.
        Local::now().format("%Y-%m-%d %H:%M:%S %Z").to_string()
    }

    #[cfg(not(any(feature = "web-ui", feature = "push-notifications")))]
    {
        let unix_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let tz = timezone.unwrap_or("server-local");
        format!("unix={unix_secs} {tz}")
    }
}

fn prompt_today_for_timezone(timezone: Option<&str>) -> String {
    #[cfg(any(feature = "web-ui", feature = "push-notifications"))]
    {
        use chrono::{Local, Utc};

        let trimmed_timezone = timezone.map(str::trim).filter(|value| !value.is_empty());

        if let Some(tz) = trimmed_timezone.and_then(|name| name.parse::<chrono_tz::Tz>().ok()) {
            return Utc::now().with_timezone(&tz).format("%Y-%m-%d").to_string();
        }

        Local::now().format("%Y-%m-%d").to_string()
    }

    #[cfg(not(any(feature = "web-ui", feature = "push-notifications")))]
    {
        let _ = timezone;
        let unix_days = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            / 86_400;
        format!("unix-day={unix_days}")
    }
}

fn normalized_iana_timezone(timezone: Option<&str>) -> Option<String> {
    timezone
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<chrono_tz::Tz>().ok())
        .map(|tz| tz.to_string())
}

fn default_user_prompt_timezone() -> Option<String> {
    let user = moltis_config::load_user()?;
    user.timezone
        .as_ref()
        .map(|timezone| timezone.name().to_string())
        .and_then(|timezone| normalized_iana_timezone(Some(&timezone)))
}

fn apply_request_runtime_context(host: &mut PromptHostRuntimeContext, params: &Value) {
    host.accept_language = params
        .get("_accept_language")
        .and_then(|v| v.as_str())
        .map(String::from);
    host.remote_ip = params
        .get("_remote_ip")
        .and_then(|v| v.as_str())
        .map(String::from);

    if let Some(timezone) =
        normalized_iana_timezone(params.get("_timezone").and_then(|v| v.as_str()))
            .or_else(default_user_prompt_timezone)
    {
        host.timezone = Some(timezone);
    }

    refresh_runtime_prompt_time(host);
}

fn prompt_sandbox_no_network_state(backend: &str, configured_no_network: bool) -> Option<bool> {
    match backend {
        // Docker supports `--network=none`, so this value is reliable.
        "docker" => Some(configured_no_network),
        // Apple Container currently has no equivalent runtime toggle, and
        // failover wrappers may switch backends dynamically.
        _ => None,
    }
}

fn effective_tool_policy(config: &moltis_config::MoltisConfig) -> ToolPolicy {
    let mut effective = ToolPolicy::default();
    if let Some(profile) = config.tools.policy.profile.as_deref()
        && !profile.is_empty()
    {
        effective = effective.merge_with(&profile_tools(profile));
    }
    let configured = ToolPolicy {
        allow: config.tools.policy.allow.clone(),
        deny: config.tools.policy.deny.clone(),
    };
    effective.merge_with(&configured)
}

fn apply_runtime_tool_filters(
    base: &ToolRegistry,
    config: &moltis_config::MoltisConfig,
    _skills: &[moltis_skills::types::SkillMetadata],
    mcp_disabled: bool,
) -> ToolRegistry {
    let base_registry = if mcp_disabled {
        base.clone_without_mcp()
    } else {
        base.clone_without(&[])
    };

    let policy = effective_tool_policy(config);
    // NOTE: Do not globally restrict tools by discovered skill `allowed_tools`.
    // Skills are always discovered for prompt injection; applying those lists at
    // runtime can unintentionally remove unrelated tools (for example, leaving
    // only `web_fetch` and preventing `create_skill` from being called).
    // Tool availability here is controlled by configured runtime policy.
    base_registry.clone_allowed_by(|name| policy.is_allowed(name))
}

// ── Disabled Models Store ────────────────────────────────────────────────────

/// Persistent store for disabled model IDs.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DisabledModelsStore {
    #[serde(default)]
    pub disabled: HashSet<String>,
    #[serde(default)]
    pub unsupported: HashMap<String, UnsupportedModelInfo>,
}

/// Metadata for a model that failed at runtime due to provider support/account limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsupportedModelInfo {
    pub detail: String,
    pub provider: Option<String>,
    pub updated_at_ms: u64,
}

impl DisabledModelsStore {
    fn config_path() -> Option<PathBuf> {
        moltis_config::config_dir().map(|d| d.join("disabled-models.json"))
    }

    /// Load disabled models from config file.
    pub fn load() -> Self {
        Self::config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_default()
    }

    /// Save disabled models to config file.
    pub fn save(&self) -> error::Result<()> {
        let path = Self::config_path().ok_or(error::Error::NoConfigDirectory)?;
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Disable a model by ID.
    pub fn disable(&mut self, model_id: &str) -> bool {
        self.disabled.insert(model_id.to_string())
    }

    /// Enable a model by ID (remove from disabled set).
    pub fn enable(&mut self, model_id: &str) -> bool {
        self.disabled.remove(model_id)
    }

    /// Check if a model is disabled.
    pub fn is_disabled(&self, model_id: &str) -> bool {
        self.disabled.contains(model_id)
    }

    /// Mark a model as unsupported with a human-readable reason.
    pub fn mark_unsupported(
        &mut self,
        model_id: &str,
        detail: &str,
        provider: Option<&str>,
    ) -> bool {
        let next = UnsupportedModelInfo {
            detail: detail.to_string(),
            provider: provider.map(ToString::to_string),
            updated_at_ms: now_ms(),
        };
        let should_update = self
            .unsupported
            .get(model_id)
            .map(|existing| existing.detail != next.detail || existing.provider != next.provider)
            .unwrap_or(true);

        if should_update {
            self.unsupported.insert(model_id.to_string(), next);
            true
        } else {
            false
        }
    }

    /// Clear unsupported status when a model succeeds again.
    pub fn clear_unsupported(&mut self, model_id: &str) -> bool {
        self.unsupported.remove(model_id).is_some()
    }

    /// Get unsupported metadata for a model.
    pub fn unsupported_info(&self, model_id: &str) -> Option<&UnsupportedModelInfo> {
        self.unsupported.get(model_id)
    }
}

// ── LiveModelService ────────────────────────────────────────────────────────

pub struct LiveModelService {
    providers: Arc<RwLock<ProviderRegistry>>,
    disabled: Arc<RwLock<DisabledModelsStore>>,
    state: Arc<OnceCell<Arc<dyn ChatRuntime>>>,
    detect_gate: Arc<Semaphore>,
    priority_models: Arc<RwLock<Vec<String>>>,
}

impl LiveModelService {
    pub fn new(
        providers: Arc<RwLock<ProviderRegistry>>,
        disabled: Arc<RwLock<DisabledModelsStore>>,
        priority_models: Vec<String>,
    ) -> Self {
        Self {
            providers,
            disabled,
            state: Arc::new(OnceCell::new()),
            detect_gate: Arc::new(Semaphore::new(1)),
            priority_models: Arc::new(RwLock::new(priority_models)),
        }
    }

    /// Shared handle to the priority models list. Pass this to services
    /// that need to update model ordering at runtime (e.g. `save_model`).
    pub fn priority_models_handle(&self) -> Arc<RwLock<Vec<String>>> {
        Arc::clone(&self.priority_models)
    }

    fn build_priority_order(models: &[String]) -> HashMap<String, usize> {
        let mut order = HashMap::new();
        for (idx, model) in models.iter().enumerate() {
            let key = normalize_model_key(model);
            if !key.is_empty() {
                let _ = order.entry(key).or_insert(idx);
            }
        }
        order
    }

    fn priority_rank(order: &HashMap<String, usize>, model: &moltis_providers::ModelInfo) -> usize {
        let full = normalize_model_key(&model.id);
        if let Some(rank) = order.get(&full) {
            return *rank;
        }
        let raw = normalize_model_key(raw_model_id(&model.id));
        if let Some(rank) = order.get(&raw) {
            return *rank;
        }
        let display = normalize_model_key(&model.display_name);
        if let Some(rank) = order.get(&display) {
            return *rank;
        }
        usize::MAX
    }

    fn prioritize_models<'a>(
        order: &HashMap<String, usize>,
        models: impl Iterator<Item = &'a moltis_providers::ModelInfo>,
    ) -> Vec<&'a moltis_providers::ModelInfo> {
        let mut ordered: Vec<(usize, &'a moltis_providers::ModelInfo)> =
            models.enumerate().collect();
        ordered.sort_by_key(|(idx, model)| {
            (
                Self::priority_rank(order, model),
                subscription_provider_rank(&model.provider),
                *idx,
            )
        });
        ordered.into_iter().map(|(_, model)| model).collect()
    }

    async fn priority_order(&self) -> HashMap<String, usize> {
        let list = self.priority_models.read().await;
        Self::build_priority_order(&list)
    }

    /// Set the gateway state reference for broadcasting model updates.
    pub fn set_state(&self, state: Arc<dyn ChatRuntime>) {
        let _ = self.state.set(state);
    }

    async fn broadcast_model_visibility_update(&self, model_id: &str, disabled: bool) {
        if let Some(state) = self.state.get() {
            broadcast(
                state,
                "models.updated",
                serde_json::json!({
                    "modelId": model_id,
                    "disabled": disabled,
                }),
                BroadcastOpts::default(),
            )
            .await;
        }
    }
}

fn normalize_model_lookup_key(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}

fn model_id_provider(model_id: &str) -> Option<&str> {
    model_id.split_once("::").map(|(provider, _)| provider)
}

fn levenshtein_distance(a: &str, b: &str) -> usize {
    if a.is_empty() {
        return b.chars().count();
    }
    if b.is_empty() {
        return a.chars().count();
    }

    let b_chars: Vec<char> = b.chars().collect();
    let a_chars: Vec<char> = a.chars().collect();
    let mut prev: Vec<usize> = (0..=b_chars.len()).collect();
    let mut curr = vec![0; b_chars.len() + 1];

    for (i, a_ch) in a_chars.iter().enumerate() {
        curr[0] = i + 1;
        for (j, b_ch) in b_chars.iter().enumerate() {
            let cost = usize::from(a_ch != b_ch);
            let deletion = prev[j + 1] + 1;
            let insertion = curr[j] + 1;
            let substitution = prev[j] + cost;
            curr[j + 1] = deletion.min(insertion).min(substitution);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[b_chars.len()]
}

fn suggest_model_ids(
    requested_model_id: &str,
    available_model_ids: &[String],
    limit: usize,
) -> Vec<String> {
    if requested_model_id.trim().is_empty() || available_model_ids.is_empty() || limit == 0 {
        return Vec::new();
    }

    let requested_provider = model_id_provider(requested_model_id).map(str::to_ascii_lowercase);
    let requested_raw = raw_model_id(requested_model_id);
    let requested_norm = normalize_model_lookup_key(requested_model_id);
    let requested_raw_norm = normalize_model_lookup_key(requested_raw);

    let mut ranked: Vec<(String, bool, usize, usize, usize)> = available_model_ids
        .iter()
        .filter_map(|candidate| {
            let candidate_provider = model_id_provider(candidate).map(str::to_ascii_lowercase);
            let provider_match = requested_provider
                .as_deref()
                .zip(candidate_provider.as_deref())
                .is_some_and(|(left, right)| left == right);

            let candidate_raw = raw_model_id(candidate);
            let candidate_norm = normalize_model_lookup_key(candidate);
            let candidate_raw_norm = normalize_model_lookup_key(candidate_raw);

            let raw_distance = levenshtein_distance(&requested_raw_norm, &candidate_raw_norm);
            let full_distance = levenshtein_distance(&requested_norm, &candidate_norm);
            let contains = requested_raw_norm.contains(&candidate_raw_norm)
                || candidate_raw_norm.contains(&requested_raw_norm)
                || requested_norm.contains(&candidate_norm)
                || candidate_norm.contains(&requested_norm);

            // Keep nearest neighbors and strong substring matches. This trims
            // unrelated model IDs from suggestion logs/responses.
            let distance_cap = requested_raw_norm
                .len()
                .max(candidate_raw_norm.len())
                .max(3)
                / 2
                + 2;
            if !contains && raw_distance > distance_cap {
                return None;
            }

            Some((
                candidate.clone(),
                provider_match,
                raw_distance,
                full_distance,
                requested_raw_norm.len().abs_diff(candidate_raw_norm.len()),
            ))
        })
        .collect();

    ranked.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1) // provider match first
            .then(left.2.cmp(&right.2)) // nearest raw model id
            .then(left.3.cmp(&right.3)) // nearest full model id
            .then(left.4.cmp(&right.4)) // similar length
            .then(left.0.cmp(&right.0))
    });

    ranked.into_iter().map(|(id, ..)| id).take(limit).collect()
}

#[async_trait]
impl ModelService for LiveModelService {
    async fn list(&self) -> ServiceResult {
        let reg = self.providers.read().await;
        let disabled = self.disabled.read().await;
        let order = self.priority_order().await;
        let prioritized = Self::prioritize_models(
            &order,
            reg.list_models()
                .iter()
                .filter(|m| moltis_providers::is_chat_capable_model(&m.id))
                .filter(|m| !disabled.is_disabled(&m.id))
                .filter(|m| disabled.unsupported_info(&m.id).is_none()),
        );
        info!(model_count = prioritized.len(), "models.list response");
        let models: Vec<_> = prioritized
            .iter()
            .copied()
            .map(|m| {
                let supports_tools = reg.get(&m.id).is_some_and(|p| p.supports_tools());
                let preferred = Self::priority_rank(&order, m) != usize::MAX;
                serde_json::json!({
                    "id": m.id,
                    "provider": m.provider,
                    "displayName": m.display_name,
                    "supportsTools": supports_tools,
                    "preferred": preferred,
                    "createdAt": m.created_at,
                    "unsupported": false,
                    "unsupportedReason": Value::Null,
                    "unsupportedProvider": Value::Null,
                    "unsupportedUpdatedAt": Value::Null,
                })
            })
            .collect();
        Ok(serde_json::json!(models))
    }

    async fn list_all(&self) -> ServiceResult {
        let reg = self.providers.read().await;
        let disabled = self.disabled.read().await;
        let order = self.priority_order().await;
        let prioritized = Self::prioritize_models(
            &order,
            reg.list_models()
                .iter()
                .filter(|m| moltis_providers::is_chat_capable_model(&m.id)),
        );
        info!(model_count = prioritized.len(), "models.list_all response");
        let models: Vec<_> = prioritized
            .iter()
            .copied()
            .map(|m| {
                let supports_tools = reg.get(&m.id).is_some_and(|p| p.supports_tools());
                let unsupported = disabled.unsupported_info(&m.id);
                serde_json::json!({
                    "id": m.id,
                    "provider": m.provider,
                    "displayName": m.display_name,
                    "supportsTools": supports_tools,
                    "createdAt": m.created_at,
                    "disabled": disabled.is_disabled(&m.id),
                    "unsupported": unsupported.is_some(),
                    "unsupportedReason": unsupported.map(|u| u.detail.clone()),
                    "unsupportedProvider": unsupported.and_then(|u| u.provider.clone()),
                    "unsupportedUpdatedAt": unsupported.map(|u| u.updated_at_ms),
                })
            })
            .collect();
        Ok(serde_json::json!(models))
    }

    async fn disable(&self, params: Value) -> ServiceResult {
        let model_id = params
            .get("modelId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'modelId' parameter".to_string())?;

        info!(model = %model_id, "disabling model");

        let mut disabled = self.disabled.write().await;
        disabled.disable(model_id);
        disabled
            .save()
            .map_err(|e| format!("failed to save: {e}"))?;
        drop(disabled);

        self.broadcast_model_visibility_update(model_id, true).await;

        Ok(serde_json::json!({
            "ok": true,
            "modelId": model_id,
        }))
    }

    async fn enable(&self, params: Value) -> ServiceResult {
        let model_id = params
            .get("modelId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'modelId' parameter".to_string())?;

        info!(model = %model_id, "enabling model");

        let mut disabled = self.disabled.write().await;
        disabled.enable(model_id);
        disabled
            .save()
            .map_err(|e| format!("failed to save: {e}"))?;
        drop(disabled);

        self.broadcast_model_visibility_update(model_id, false)
            .await;

        Ok(serde_json::json!({
            "ok": true,
            "modelId": model_id,
        }))
    }

    async fn detect_supported(&self, params: Value) -> ServiceResult {
        let background = params
            .get("background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let reason = params
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("manual")
            .to_string();
        let max_parallel = params
            .get("maxParallel")
            .and_then(|v| v.as_u64())
            .map(|v| v.clamp(1, 32) as usize)
            .unwrap_or(8);
        let max_parallel_per_provider = probe_max_parallel_per_provider(&params);
        let provider_filter = provider_filter_from_params(&params);

        let _run_permit: OwnedSemaphorePermit = if background {
            match Arc::clone(&self.detect_gate).try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => {
                    return Ok(serde_json::json!({
                        "ok": true,
                        "background": true,
                        "reason": reason,
                        "skipped": true,
                        "message": "model probe already running",
                    }));
                },
            }
        } else {
            Arc::clone(&self.detect_gate)
                .acquire_owned()
                .await
                .map_err(|_| ServiceError::message("model probe gate closed"))?
        };

        let state = self.state.get().cloned();

        // Phase 1: notify clients to refresh and show the full current model list first.
        if let Some(state) = state.as_ref() {
            broadcast(
                state,
                "models.updated",
                serde_json::json!({
                    "phase": "catalog",
                    "background": background,
                    "reason": reason,
                    "provider": provider_filter.as_deref(),
                }),
                BroadcastOpts::default(),
            )
            .await;
        }

        let checks = {
            let reg = self.providers.read().await;
            let disabled = self.disabled.read().await;
            reg.list_models()
                .iter()
                .filter(|m| !disabled.is_disabled(&m.id))
                .filter(|m| provider_matches_filter(&m.provider, provider_filter.as_deref()))
                .filter_map(|m| {
                    reg.get(&m.id).map(|provider| {
                        (
                            m.id.clone(),
                            m.display_name.clone(),
                            provider.name().to_string(),
                            provider,
                        )
                    })
                })
                .collect::<Vec<_>>()
        };

        let total = checks.len();
        if let Some(state) = state.as_ref() {
            broadcast(
                state,
                "models.updated",
                serde_json::json!({
                    "phase": "start",
                    "background": background,
                    "reason": reason,
                    "provider": provider_filter.as_deref(),
                    "maxParallelPerProvider": max_parallel_per_provider,
                    "total": total,
                    "checked": 0,
                    "supported": 0,
                    "unsupported": 0,
                    "errors": 0,
                }),
                BroadcastOpts::default(),
            )
            .await;
        }

        let limiter = Arc::new(Semaphore::new(max_parallel));
        let provider_limiter = Arc::new(ProbeProviderLimiter::new(max_parallel_per_provider));
        let rate_limiter = Arc::new(ProbeRateLimiter::default());
        let mut tasks = futures::stream::FuturesUnordered::new();
        for (model_id, display_name, provider_name, provider) in checks {
            let limiter = Arc::clone(&limiter);
            let provider_limiter = Arc::clone(&provider_limiter);
            let rate_limiter = Arc::clone(&rate_limiter);
            tasks.push(tokio::spawn(run_single_probe(
                model_id,
                display_name,
                provider_name,
                provider,
                limiter,
                provider_limiter,
                rate_limiter,
            )));
        }

        let mut results = Vec::with_capacity(total);
        let mut checked = 0usize;
        let mut supported = 0usize;
        let mut unsupported = 0usize;
        let mut flagged = 0usize;
        let mut cleared = 0usize;
        let mut errors = 0usize;
        let mut supported_by_provider: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        let mut unsupported_by_provider: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        let mut errors_by_provider: BTreeMap<String, Vec<Value>> = BTreeMap::new();

        while let Some(joined) = tasks.next().await {
            checked += 1;
            let outcome = match joined {
                Ok(outcome) => outcome,
                Err(err) => {
                    errors += 1;
                    results.push(serde_json::json!({
                        "modelId": "",
                        "displayName": "",
                        "provider": "",
                        "status": "error",
                        "error": format!("probe task failed: {err}"),
                    }));
                    if let Some(state) = state.as_ref() {
                        broadcast(
                            state,
                            "models.updated",
                            serde_json::json!({
                                "phase": "progress",
                                "background": background,
                                "reason": reason,
                                "provider": provider_filter.as_deref(),
                                "total": total,
                                "checked": checked,
                                "supported": supported,
                                "unsupported": unsupported,
                                "errors": errors,
                            }),
                            BroadcastOpts::default(),
                        )
                        .await;
                    }
                    continue;
                },
            };

            match outcome.status {
                ProbeStatus::Supported => {
                    supported += 1;
                    push_provider_model(
                        &mut supported_by_provider,
                        &outcome.provider_name,
                        &outcome.model_id,
                        &outcome.display_name,
                    );
                    let mut changed = false;
                    {
                        let mut store = self.disabled.write().await;
                        if store.clear_unsupported(&outcome.model_id) {
                            changed = true;
                            if let Err(err) = store.save() {
                                warn!(
                                    model = %outcome.model_id,
                                    error = %err,
                                    "failed to persist unsupported model clear"
                                );
                            }
                        }
                    }
                    if changed {
                        cleared += 1;
                        if let Some(state) = state.as_ref() {
                            broadcast(
                                state,
                                "models.updated",
                                serde_json::json!({
                                    "modelId": outcome.model_id,
                                    "unsupported": false,
                                }),
                                BroadcastOpts::default(),
                            )
                            .await;
                        }
                    }

                    results.push(serde_json::json!({
                        "modelId": outcome.model_id,
                        "displayName": outcome.display_name,
                        "provider": outcome.provider_name,
                        "status": "supported",
                    }));
                },
                ProbeStatus::Unsupported { detail, provider } => {
                    unsupported += 1;
                    push_provider_model(
                        &mut unsupported_by_provider,
                        &outcome.provider_name,
                        &outcome.model_id,
                        &outcome.display_name,
                    );
                    let mut changed = false;
                    let mut updated_at_ms = now_ms();
                    {
                        let mut store = self.disabled.write().await;
                        if store.mark_unsupported(&outcome.model_id, &detail, Some(&provider)) {
                            changed = true;
                            if let Some(info) = store.unsupported_info(&outcome.model_id) {
                                updated_at_ms = info.updated_at_ms;
                            }
                            if let Err(save_err) = store.save() {
                                warn!(
                                    model = %outcome.model_id,
                                    provider = provider,
                                    error = %save_err,
                                    "failed to persist unsupported model flag"
                                );
                            }
                        }
                    }
                    if changed {
                        flagged += 1;
                        if let Some(state) = state.as_ref() {
                            broadcast(
                                state,
                                "models.updated",
                                serde_json::json!({
                                    "modelId": outcome.model_id,
                                    "unsupported": true,
                                    "unsupportedReason": detail,
                                    "unsupportedProvider": provider,
                                    "unsupportedUpdatedAt": updated_at_ms,
                                }),
                                BroadcastOpts::default(),
                            )
                            .await;
                        }
                    }

                    results.push(serde_json::json!({
                        "modelId": outcome.model_id,
                        "displayName": outcome.display_name,
                        "provider": outcome.provider_name,
                        "status": "unsupported",
                        "error": detail,
                    }));
                },
                ProbeStatus::Error { message } => {
                    errors += 1;
                    push_provider_model(
                        &mut errors_by_provider,
                        &outcome.provider_name,
                        &outcome.model_id,
                        &outcome.display_name,
                    );
                    results.push(serde_json::json!({
                        "modelId": outcome.model_id,
                        "displayName": outcome.display_name,
                        "provider": outcome.provider_name,
                        "status": "error",
                        "error": message,
                    }));
                },
            }

            if let Some(state) = state.as_ref() {
                broadcast(
                    state,
                    "models.updated",
                    serde_json::json!({
                        "phase": "progress",
                        "background": background,
                        "reason": reason,
                        "provider": provider_filter.as_deref(),
                        "total": total,
                        "checked": checked,
                        "supported": supported,
                        "unsupported": unsupported,
                        "errors": errors,
                    }),
                    BroadcastOpts::default(),
                )
                .await;
            }
        }

        let summary = serde_json::json!({
            "ok": true,
            "probeWord": "ping",
            "background": background,
            "reason": reason,
            "provider": provider_filter.as_deref(),
            "maxParallel": max_parallel,
            "maxParallelPerProvider": max_parallel_per_provider,
            "total": total,
            "checked": checked,
            "supported": supported,
            "unsupported": unsupported,
            "flagged": flagged,
            "cleared": cleared,
            "errors": errors,
            "supportedByProvider": supported_by_provider,
            "unsupportedByProvider": unsupported_by_provider,
            "errorsByProvider": errors_by_provider,
            "results": results,
        });

        // Final refresh event to ensure clients are in sync after the full pass.
        if let Some(state) = state.as_ref() {
            broadcast(
                state,
                "models.updated",
                serde_json::json!({
                    "phase": "complete",
                    "background": background,
                    "reason": reason,
                    "provider": provider_filter.as_deref(),
                    "summary": summary,
                }),
                BroadcastOpts::default(),
            )
            .await;
        }

        Ok(summary)
    }

    async fn test(&self, params: Value) -> ServiceResult {
        let model_id = params
            .get("modelId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'modelId' parameter".to_string())?;

        let provider = {
            let reg = self.providers.read().await;
            if let Some(provider) = reg.get(model_id) {
                provider
            } else {
                let available_model_ids: Vec<String> =
                    reg.list_models().iter().map(|m| m.id.clone()).collect();
                let suggestions = suggest_model_ids(model_id, &available_model_ids, 5);
                warn!(
                    model_id,
                    available_model_count = available_model_ids.len(),
                    available_model_ids = ?available_model_ids,
                    suggested_model_ids = ?suggestions,
                    "models.test received unknown model id"
                );
                let suggestion_hint = if suggestions.is_empty() {
                    String::new()
                } else {
                    format!(". did you mean: {}", suggestions.join(", "))
                };
                return Err(format!("unknown model: {model_id}{suggestion_hint}").into());
            }
        };
        let started = Instant::now();
        info!(model_id, provider = provider.name(), "model probe started");

        // Use streaming and return as soon as the first token arrives.
        // Dropping the stream closes the HTTP connection, which tells the
        // provider to stop generating — effectively max_tokens: 1.
        let probe = vec![ChatMessage::user("ping")];
        let mut stream = provider.stream(probe);

        let result = tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(event) = stream.next().await {
                match event {
                    StreamEvent::Delta(_) | StreamEvent::Done(_) => return Ok(()),
                    StreamEvent::Error(err) => return Err(err),
                    // Skip other events (tool calls, etc.) and keep waiting.
                    _ => continue,
                }
            }
            Err("stream ended without producing any output".to_string())
        })
        .await;

        // Drop the stream early to cancel the request on the provider side.
        drop(stream);

        match result {
            Ok(Ok(())) => {
                info!(
                    model_id,
                    provider = provider.name(),
                    elapsed_ms = started.elapsed().as_millis(),
                    "model probe succeeded"
                );
                Ok(serde_json::json!({
                    "ok": true,
                    "modelId": model_id,
                }))
            },
            Ok(Err(err)) => {
                let error_obj = parse_chat_error(&err, Some(provider.name()));
                let detail = error_obj
                    .get("detail")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&err)
                    .to_string();

                warn!(
                    model_id,
                    provider = provider.name(),
                    elapsed_ms = started.elapsed().as_millis(),
                    error = %detail,
                    "model probe failed"
                );
                Err(detail.into())
            },
            Err(_) => {
                warn!(
                    model_id,
                    provider = provider.name(),
                    elapsed_ms = started.elapsed().as_millis(),
                    "model probe timed out after 10s"
                );
                Err("Connection timed out after 10 seconds".into())
            },
        }
    }
}

// ── LiveChatService ─────────────────────────────────────────────────────────

/// A message that arrived while an agent run was already active on the session.
#[derive(Debug, Clone)]
struct QueuedMessage {
    params: Value,
}

/// A tool call currently executing within an active agent run.
#[derive(Debug, Clone, Serialize)]
pub struct ActiveToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
    #[serde(rename = "startedAt")]
    pub started_at: u64,
}

pub struct LiveChatService {
    providers: Arc<RwLock<ProviderRegistry>>,
    model_store: Arc<RwLock<DisabledModelsStore>>,
    state: Arc<dyn ChatRuntime>,
    active_runs: Arc<RwLock<HashMap<String, AbortHandle>>>,
    active_runs_by_session: Arc<RwLock<HashMap<String, String>>>,
    tool_registry: Arc<RwLock<ToolRegistry>>,
    session_store: Arc<SessionStore>,
    session_metadata: Arc<SqliteSessionMetadata>,
    hook_registry: Option<Arc<moltis_common::hooks::HookRegistry>>,
    /// Per-session semaphore ensuring only one agent run executes per session at a time.
    session_locks: Arc<RwLock<HashMap<String, Arc<Semaphore>>>>,
    /// Per-session message queue for messages arriving during an active run.
    message_queue: Arc<RwLock<HashMap<String, Vec<QueuedMessage>>>>,
    /// Per-session last-seen client sequence number for ordering diagnostics.
    last_client_seq: Arc<RwLock<HashMap<String, u64>>>,
    /// Per-session accumulated thinking text for active runs, so it can be
    /// returned in `sessions.switch` after a page reload.
    active_thinking_text: Arc<RwLock<HashMap<String, String>>>,
    /// Per-session active tool calls for `chat.peek` snapshot.
    active_tool_calls: Arc<RwLock<HashMap<String, Vec<ActiveToolCall>>>>,
    /// Per-session reply medium for active runs, so the frontend can restore
    /// `voicePending` state after a page reload.
    active_reply_medium: Arc<RwLock<HashMap<String, ReplyMedium>>>,
    /// Failover configuration for automatic model/provider failover.
    failover_config: moltis_config::schema::FailoverConfig,
}

impl LiveChatService {
    pub fn new(
        providers: Arc<RwLock<ProviderRegistry>>,
        model_store: Arc<RwLock<DisabledModelsStore>>,
        state: Arc<dyn ChatRuntime>,
        session_store: Arc<SessionStore>,
        session_metadata: Arc<SqliteSessionMetadata>,
    ) -> Self {
        Self {
            providers,
            model_store,
            state,
            active_runs: Arc::new(RwLock::new(HashMap::new())),
            active_runs_by_session: Arc::new(RwLock::new(HashMap::new())),
            tool_registry: Arc::new(RwLock::new(ToolRegistry::new())),
            session_store,
            session_metadata,
            hook_registry: None,
            session_locks: Arc::new(RwLock::new(HashMap::new())),
            message_queue: Arc::new(RwLock::new(HashMap::new())),
            last_client_seq: Arc::new(RwLock::new(HashMap::new())),
            active_thinking_text: Arc::new(RwLock::new(HashMap::new())),
            active_tool_calls: Arc::new(RwLock::new(HashMap::new())),
            active_reply_medium: Arc::new(RwLock::new(HashMap::new())),
            failover_config: moltis_config::schema::FailoverConfig::default(),
        }
    }

    pub fn with_failover(mut self, config: moltis_config::schema::FailoverConfig) -> Self {
        self.failover_config = config;
        self
    }

    pub fn with_tools(mut self, registry: Arc<RwLock<ToolRegistry>>) -> Self {
        self.tool_registry = registry;
        self
    }

    pub fn with_hooks(mut self, registry: moltis_common::hooks::HookRegistry) -> Self {
        self.hook_registry = Some(Arc::new(registry));
        self
    }

    pub fn with_hooks_arc(mut self, registry: Arc<moltis_common::hooks::HookRegistry>) -> Self {
        self.hook_registry = Some(registry);
        self
    }

    fn has_tools_sync(&self) -> bool {
        // Best-effort check: try_read avoids blocking. If the lock is held,
        // assume tools are present (conservative — enables tool mode).
        self.tool_registry
            .try_read()
            .map(|r| {
                let schemas = r.list_schemas();
                let has = !schemas.is_empty();
                tracing::debug!(
                    tool_count = schemas.len(),
                    has_tools = has,
                    "has_tools_sync check"
                );
                has
            })
            .unwrap_or(true)
    }

    /// Return the per-session semaphore, creating one if absent.
    async fn session_semaphore(&self, key: &str) -> Arc<Semaphore> {
        // Fast path: read lock.
        {
            let locks = self.session_locks.read().await;
            if let Some(sem) = locks.get(key) {
                return Arc::clone(sem);
            }
        }
        // Slow path: write lock, insert.
        let mut locks = self.session_locks.write().await;
        Arc::clone(
            locks
                .entry(key.to_string())
                .or_insert_with(|| Arc::new(Semaphore::new(1))),
        )
    }

    async fn abort_run_handle(
        active_runs: &Arc<RwLock<HashMap<String, AbortHandle>>>,
        active_runs_by_session: &Arc<RwLock<HashMap<String, String>>>,
        run_id: Option<&str>,
        session_key: Option<&str>,
    ) -> (Option<String>, bool) {
        let resolved_run_id = if let Some(id) = run_id {
            Some(id.to_string())
        } else if let Some(key) = session_key {
            active_runs_by_session.read().await.get(key).cloned()
        } else {
            None
        };

        let Some(target_run_id) = resolved_run_id.clone() else {
            return (None, false);
        };

        let aborted = if let Some(handle) = active_runs.write().await.remove(&target_run_id) {
            handle.abort();
            true
        } else {
            false
        };

        let mut by_session = active_runs_by_session.write().await;
        if let Some(key) = session_key
            && by_session.get(key).is_some_and(|id| id == &target_run_id)
        {
            by_session.remove(key);
        }
        by_session.retain(|_, id| id != &target_run_id);

        (resolved_run_id, aborted)
    }

    /// Resolve a provider from session metadata, history, or first registered.
    async fn resolve_provider(
        &self,
        session_key: &str,
        history: &[Value],
    ) -> error::Result<Arc<dyn moltis_agents::model::LlmProvider>> {
        let reg = self.providers.read().await;
        let session_model = self
            .session_metadata
            .get(session_key)
            .await
            .and_then(|e| e.model.clone());
        let history_model = history
            .iter()
            .rev()
            .find_map(|m| m.get("model").and_then(|v| v.as_str()).map(String::from));
        let model_id = session_model.or(history_model);

        model_id
            .and_then(|id| reg.get(&id))
            .or_else(|| reg.first())
            .ok_or_else(|| error::Error::message("no LLM providers configured"))
    }

    /// Resolve the active session key for a connection.
    async fn session_key_for(&self, conn_id: Option<&str>) -> String {
        if let Some(cid) = conn_id
            && let Some(key) = self.state.active_session_key(cid).await
        {
            return key;
        }
        "main".to_string()
    }

    /// Resolve the project context prompt section for a session.
    async fn resolve_project_context(
        &self,
        session_key: &str,
        conn_id: Option<&str>,
    ) -> Option<String> {
        let project_id = if let Some(cid) = conn_id {
            self.state.active_project_id(cid).await
        } else {
            None
        };
        // Also check session metadata for project binding (async path).
        let project_id = match project_id {
            Some(pid) => Some(pid),
            None => self
                .session_metadata
                .get(session_key)
                .await
                .and_then(|e| e.project_id),
        };

        let pid = project_id?;
        let val = self
            .state
            .project_service()
            .get(serde_json::json!({"id": pid}))
            .await
            .ok()?;
        let dir = val.get("directory").and_then(|v| v.as_str())?;
        let files = match moltis_projects::context::load_context_files(Path::new(dir)) {
            Ok(f) => f,
            Err(e) => {
                warn!("failed to load project context: {e}");
                return None;
            },
        };
        let project: moltis_projects::Project = serde_json::from_value(val.clone()).ok()?;
        let worktree_dir = self
            .session_metadata
            .get(session_key)
            .await
            .and_then(|e| e.worktree_branch)
            .and_then(|_| {
                let wt_path = Path::new(dir).join(".moltis-worktrees").join(session_key);
                if wt_path.exists() {
                    Some(wt_path)
                } else {
                    None
                }
            });
        let ctx = moltis_projects::ProjectContext {
            project,
            context_files: files,
            worktree_dir,
        };
        Some(ctx.to_prompt_section())
    }
}

#[async_trait]
impl ChatService for LiveChatService {
    async fn send(&self, mut params: Value) -> ServiceResult {
        // Support both text-only and multimodal content.
        // - "text": string → plain text message
        // - "content": array → multimodal content (text + images)
        let (text, message_content) = if let Some(content) = params.get("content") {
            // Multimodal content - extract text for logging/hooks, parse into typed blocks
            let text_part = content
                .as_array()
                .and_then(|arr| {
                    arr.iter()
                        .find(|block| block.get("type").and_then(|t| t.as_str()) == Some("text"))
                        .and_then(|block| block.get("text").and_then(|t| t.as_str()))
                })
                .unwrap_or("[Image]")
                .to_string();

            // Parse JSON blocks into typed ContentBlock structs
            let blocks: Vec<ContentBlock> = content
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|block| {
                            let block_type = block.get("type")?.as_str()?;
                            match block_type {
                                "text" => {
                                    let text = block.get("text")?.as_str()?.to_string();
                                    Some(ContentBlock::text(text))
                                },
                                "image_url" => {
                                    let url = block.get("image_url")?.get("url")?.as_str()?;
                                    Some(ContentBlock::ImageUrl {
                                        image_url: moltis_sessions::message::ImageUrl {
                                            url: url.to_string(),
                                        },
                                    })
                                },
                                _ => None,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            (text_part, MessageContent::Multimodal(blocks))
        } else {
            let text = params
                .get("text")
                .or_else(|| params.get("message"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| "missing 'text', 'message', or 'content' parameter".to_string())?
                .to_string();
            (text.clone(), MessageContent::Text(text))
        };
        let desired_reply_medium = infer_reply_medium(&params, &text);

        let conn_id = params
            .get("_conn_id")
            .and_then(|v| v.as_str())
            .map(String::from);
        let explicit_model = params.get("model").and_then(|v| v.as_str());
        // Use streaming-only mode if explicitly requested or if no tools are registered.
        let explicit_stream_only = params
            .get("stream_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let has_tools = self.has_tools_sync();
        let stream_only = explicit_stream_only || !has_tools;
        tracing::debug!(
            explicit_stream_only,
            has_tools,
            stream_only,
            "send() mode decision"
        );

        // Resolve session key: explicit override (used by cron callbacks) or connection-scoped lookup.
        let session_key = match params.get("_session_key").and_then(|v| v.as_str()) {
            Some(sk) => sk.to_string(),
            None => self.session_key_for(conn_id.as_deref()).await,
        };
        let queued_replay = params
            .get("_queued_replay")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Track client-side sequence number for ordering diagnostics.
        // Note: seq resets to 1 on page reload, so a drop from a high value
        // back to 1 is normal (new browser session) — only flag issues within
        // a continuous ascending sequence.
        let client_seq = params.get("_seq").and_then(|v| v.as_u64());
        if let Some(seq) = client_seq {
            if queued_replay {
                debug!(
                    session = %session_key,
                    seq,
                    "client seq replayed from queue; skipping ordering diagnostics"
                );
            } else {
                let mut seq_map = self.last_client_seq.write().await;
                let last = seq_map.entry(session_key.clone()).or_insert(0);
                if *last == 0 {
                    // First observed sequence for this session in this process.
                    // We cannot infer a gap yet because earlier messages may have
                    // come from another tab/process before we started tracking.
                    debug!(session = %session_key, seq, "client seq initialized");
                } else if seq == 1 && *last > 1 {
                    // Page reload — reset tracking.
                    debug!(
                        session = %session_key,
                        prev_seq = *last,
                        "client seq reset (page reload)"
                    );
                } else if seq <= *last {
                    warn!(
                        session = %session_key,
                        seq,
                        last_seq = *last,
                        "client seq out of order (duplicate or reorder)"
                    );
                } else if seq > *last + 1 {
                    warn!(
                        session = %session_key,
                        seq,
                        last_seq = *last,
                        gap = seq - *last - 1,
                        "client seq gap detected (missing messages)"
                    );
                }
                *last = seq;
            }
        }

        let explicit_shell_command = match &message_content {
            MessageContent::Text(raw) => parse_explicit_shell_command(raw).map(str::to_string),
            MessageContent::Multimodal(_) => None,
        };

        if let Some(shell_command) = explicit_shell_command {
            // Generate run_id early so we can link the user message to this run.
            let run_id = uuid::Uuid::new_v4().to_string();
            let run_id_clone = run_id.clone();
            let channel_meta = params.get("channel").cloned();
            let user_audio = user_audio_path_from_params(&params, &session_key);
            let user_msg = PersistedMessage::User {
                content: message_content,
                created_at: Some(now_ms()),
                audio: user_audio,
                channel: channel_meta,
                seq: client_seq,
                run_id: Some(run_id.clone()),
            };

            let history = self
                .session_store
                .read(&session_key)
                .await
                .unwrap_or_default();
            let user_message_index = history.len();

            // Ensure the session exists in metadata and counts are up to date.
            let _ = self.session_metadata.upsert(&session_key, None).await;
            self.session_metadata
                .touch(&session_key, history.len() as u32)
                .await;

            // If this is a web UI message on a channel-bound session, attach the
            // channel reply target so /sh output can be delivered back to the channel.
            let is_web_message = conn_id.is_some()
                && params.get("_session_key").is_none()
                && params.get("channel").is_none();

            if is_web_message
                && let Some(entry) = self.session_metadata.get(&session_key).await
                && let Some(ref binding_json) = entry.channel_binding
                && let Ok(target) =
                    serde_json::from_str::<moltis_channels::ChannelReplyTarget>(binding_json)
            {
                let is_active = self
                    .session_metadata
                    .get_active_session(
                        target.channel_type.as_str(),
                        &target.account_id,
                        &target.chat_id,
                    )
                    .await
                    .map(|k| k == session_key)
                    .unwrap_or(true);

                if is_active {
                    match serde_json::to_value(&target) {
                        Ok(target_val) => {
                            params["_channel_reply_target"] = target_val;
                        },
                        Err(e) => {
                            warn!(
                                session = %session_key,
                                error = %e,
                                "failed to serialize channel reply target for /sh"
                            );
                        },
                    }
                }
            }

            let deferred_channel_target =
                params
                    .get("_channel_reply_target")
                    .cloned()
                    .and_then(|value| {
                        match serde_json::from_value::<moltis_channels::ChannelReplyTarget>(value) {
                            Ok(target) => Some(target),
                            Err(e) => {
                                warn!(
                                    session = %session_key,
                                    error = %e,
                                    "ignoring invalid _channel_reply_target for /sh"
                                );
                                None
                            },
                        }
                    });

            info!(
                run_id = %run_id,
                user_message = %text,
                session = %session_key,
                command = %shell_command,
                client_seq = ?client_seq,
                mode = "explicit_shell",
                "chat.send"
            );

            // Try to acquire the per-session semaphore. If a run is already active,
            // queue the message according to MessageQueueMode.
            let session_sem = self.session_semaphore(&session_key).await;
            let permit: OwnedSemaphorePermit = match session_sem.clone().try_acquire_owned() {
                Ok(p) => p,
                Err(_) => {
                    let queue_mode = moltis_config::discover_and_load().chat.message_queue_mode;
                    info!(
                        session = %session_key,
                        mode = ?queue_mode,
                        client_seq = ?client_seq,
                        "queueing message (run active)"
                    );
                    let position = {
                        let mut q = self.message_queue.write().await;
                        let entry = q.entry(session_key.clone()).or_default();
                        entry.push(QueuedMessage {
                            params: params.clone(),
                        });
                        entry.len()
                    };
                    broadcast(
                        &self.state,
                        "chat",
                        serde_json::json!({
                            "sessionKey": session_key,
                            "state": "queued",
                            "mode": format!("{queue_mode:?}").to_lowercase(),
                            "position": position,
                        }),
                        BroadcastOpts::default(),
                    )
                    .await;
                    return Ok(serde_json::json!({
                        "ok": true,
                        "queued": true,
                        "mode": format!("{queue_mode:?}").to_lowercase(),
                    }));
                },
            };

            // Persist user message now that it will execute immediately.
            if let Err(e) = self
                .session_store
                .append(&session_key, &user_msg.to_value())
                .await
            {
                warn!("failed to persist /sh user message: {e}");
            }

            // Set preview from first user message if not already set.
            if let Some(entry) = self.session_metadata.get(&session_key).await
                && entry.preview.is_none()
            {
                let preview_text = extract_preview_from_value(&user_msg.to_value());
                if let Some(preview) = preview_text {
                    self.session_metadata
                        .set_preview(&session_key, Some(&preview))
                        .await;
                }
            }

            let state = Arc::clone(&self.state);
            let active_runs = Arc::clone(&self.active_runs);
            let active_runs_by_session = Arc::clone(&self.active_runs_by_session);
            let active_thinking_text = Arc::clone(&self.active_thinking_text);
            let active_tool_calls = Arc::clone(&self.active_tool_calls);
            let active_reply_medium = Arc::clone(&self.active_reply_medium);
            let session_store = Arc::clone(&self.session_store);
            let session_metadata = Arc::clone(&self.session_metadata);
            let tool_registry = Arc::clone(&self.tool_registry);
            let session_key_clone = session_key.clone();
            let message_queue = Arc::clone(&self.message_queue);
            let state_for_drain = Arc::clone(&self.state);
            let accept_language = params
                .get("_accept_language")
                .and_then(|v| v.as_str())
                .map(String::from);
            let conn_id_for_tool = conn_id.clone();

            let handle = tokio::spawn(async move {
                let permit = permit; // hold permit until command run completes
                if let Some(target) = deferred_channel_target {
                    state.push_channel_reply(&session_key_clone, target).await;
                }
                active_reply_medium
                    .write()
                    .await
                    .insert(session_key_clone.clone(), ReplyMedium::Text);

                let assistant_output = run_explicit_shell_command(
                    &state,
                    &run_id_clone,
                    &tool_registry,
                    &session_store,
                    &session_key_clone,
                    &shell_command,
                    user_message_index,
                    accept_language,
                    conn_id_for_tool,
                    client_seq,
                )
                .await;

                let assistant_msg = PersistedMessage::Assistant {
                    content: assistant_output.text,
                    created_at: Some(now_ms()),
                    model: None,
                    provider: None,
                    input_tokens: Some(assistant_output.input_tokens),
                    output_tokens: Some(assistant_output.output_tokens),
                    duration_ms: Some(assistant_output.duration_ms),
                    request_input_tokens: Some(assistant_output.request_input_tokens),
                    request_output_tokens: Some(assistant_output.request_output_tokens),
                    tool_calls: None,
                    reasoning: assistant_output.reasoning,
                    llm_api_response: assistant_output.llm_api_response,
                    audio: assistant_output.audio_path,
                    seq: client_seq,
                    run_id: Some(run_id_clone.clone()),
                };
                if let Err(e) = session_store
                    .append(&session_key_clone, &assistant_msg.to_value())
                    .await
                {
                    warn!("failed to persist /sh assistant message: {e}");
                }
                if let Ok(count) = session_store.count(&session_key_clone).await {
                    session_metadata.touch(&session_key_clone, count).await;
                }

                active_runs.write().await.remove(&run_id_clone);
                let mut runs_by_session = active_runs_by_session.write().await;
                if runs_by_session.get(&session_key_clone) == Some(&run_id_clone) {
                    runs_by_session.remove(&session_key_clone);
                }
                drop(runs_by_session);
                active_thinking_text
                    .write()
                    .await
                    .remove(&session_key_clone);
                active_tool_calls.write().await.remove(&session_key_clone);
                active_reply_medium.write().await.remove(&session_key_clone);

                drop(permit);

                // Drain queued messages for this session.
                let queued = message_queue
                    .write()
                    .await
                    .remove(&session_key_clone)
                    .unwrap_or_default();
                if !queued.is_empty() {
                    let queue_mode = moltis_config::discover_and_load().chat.message_queue_mode;
                    let chat = state_for_drain.chat_service().await;
                    match queue_mode {
                        MessageQueueMode::Followup => {
                            let mut iter = queued.into_iter();
                            let Some(first) = iter.next() else {
                                return;
                            };
                            let rest: Vec<QueuedMessage> = iter.collect();
                            if !rest.is_empty() {
                                message_queue
                                    .write()
                                    .await
                                    .entry(session_key_clone.clone())
                                    .or_default()
                                    .extend(rest);
                            }
                            info!(session = %session_key_clone, "replaying queued message (followup)");
                            let mut replay_params = first.params;
                            replay_params["_queued_replay"] = serde_json::json!(true);
                            if let Err(e) = chat.send(replay_params).await {
                                warn!(session = %session_key_clone, error = %e, "failed to replay queued message");
                            }
                        },
                        MessageQueueMode::Collect => {
                            let combined: Vec<&str> = queued
                                .iter()
                                .filter_map(|m| m.params.get("text").and_then(|v| v.as_str()))
                                .collect();
                            if !combined.is_empty() {
                                info!(
                                    session = %session_key_clone,
                                    count = combined.len(),
                                    "replaying collected messages"
                                );
                                let Some(last) = queued.last() else {
                                    return;
                                };
                                let mut merged = last.params.clone();
                                merged["text"] = serde_json::json!(combined.join("\n\n"));
                                merged["_queued_replay"] = serde_json::json!(true);
                                if let Err(e) = chat.send(merged).await {
                                    warn!(session = %session_key_clone, error = %e, "failed to replay collected messages");
                                }
                            }
                        },
                    }
                }
            });

            self.active_runs
                .write()
                .await
                .insert(run_id.clone(), handle.abort_handle());
            self.active_runs_by_session
                .write()
                .await
                .insert(session_key.clone(), run_id.clone());

            return Ok(serde_json::json!({ "ok": true, "runId": run_id }));
        }

        // Resolve model: explicit param → session metadata → first registered.
        let session_model = if explicit_model.is_none() {
            self.session_metadata
                .get(&session_key)
                .await
                .and_then(|e| e.model)
        } else {
            None
        };
        let model_id = explicit_model.or(session_model.as_deref());

        let provider: Arc<dyn moltis_agents::model::LlmProvider> = {
            let reg = self.providers.read().await;
            let primary = if let Some(id) = model_id {
                reg.get(id).ok_or_else(|| {
                    let available: Vec<_> =
                        reg.list_models().iter().map(|m| m.id.clone()).collect();
                    format!("model '{}' not found. available: {:?}", id, available)
                })?
            } else if !stream_only {
                reg.first_with_tools()
                    .ok_or_else(|| "no LLM providers configured".to_string())?
            } else {
                reg.first()
                    .ok_or_else(|| "no LLM providers configured".to_string())?
            };

            if self.failover_config.enabled {
                let fallbacks = if self.failover_config.fallback_models.is_empty() {
                    // Auto-build: same model on other providers first, then same
                    // provider's other models, then everything else.
                    reg.fallback_providers_for(primary.id(), primary.name())
                } else {
                    reg.providers_for_models(&self.failover_config.fallback_models)
                };
                if fallbacks.is_empty() {
                    primary
                } else {
                    let mut chain = vec![primary];
                    chain.extend(fallbacks);
                    Arc::new(moltis_agents::provider_chain::ProviderChain::new(chain))
                }
            } else {
                primary
            }
        };

        // Check if this is a local model that needs downloading.
        // Only do this check for local-llm providers.
        #[cfg(feature = "local-llm")]
        if provider.name() == "local-llm" {
            let model_to_check = model_id
                .map(raw_model_id)
                .unwrap_or_else(|| raw_model_id(provider.id()))
                .to_string();
            tracing::info!(
                provider_name = provider.name(),
                model_to_check,
                "checking local model cache"
            );
            if let Err(e) = self.state.ensure_local_model_cached(&model_to_check).await {
                return Err(format!("Failed to prepare local model: {}", e).into());
            }
        }

        // Resolve project context for this connection's active project.
        let project_context = self
            .resolve_project_context(&session_key, conn_id.as_deref())
            .await;

        // Dispatch MessageReceived hook (read-only).
        if let Some(ref hooks) = self.hook_registry {
            let channel = params
                .get("channel")
                .and_then(|v| v.as_str())
                .map(String::from);
            let payload = moltis_common::hooks::HookPayload::MessageReceived {
                session_key: session_key.clone(),
                content: text.clone(),
                channel,
            };
            if let Err(e) = hooks.dispatch(&payload).await {
                warn!(session = %session_key, error = %e, "MessageReceived hook failed");
            }
        }

        // Generate run_id early so we can link the user message to its agent run.
        let run_id = uuid::Uuid::new_v4().to_string();

        // Convert session-crate content to agents-crate content for the LLM.
        // Must happen before `message_content` is moved into `user_msg`.
        let user_content = to_user_content(&message_content);

        // Build the user message for later persistence (deferred until we
        // know the message won't be queued — avoids double-persist when a
        // queued message is replayed via send()).
        let channel_meta = params.get("channel").cloned();
        let user_audio = user_audio_path_from_params(&params, &session_key);
        let user_msg = PersistedMessage::User {
            content: message_content,
            created_at: Some(now_ms()),
            audio: user_audio,
            channel: channel_meta,
            seq: client_seq,
            run_id: Some(run_id.clone()),
        };

        // Load conversation history (the current user message is NOT yet
        // persisted — run_streaming / run_agent_loop add it themselves).
        let mut history = self
            .session_store
            .read(&session_key)
            .await
            .unwrap_or_default();

        // Update metadata.
        let _ = self.session_metadata.upsert(&session_key, None).await;
        self.session_metadata
            .touch(&session_key, history.len() as u32)
            .await;

        // If this is a web UI message on a channel-bound session, attach the
        // channel reply target so the run-start path can route the final
        // response back to the channel.
        let is_web_message = conn_id.is_some()
            && params.get("_session_key").is_none()
            && params.get("channel").is_none();

        if is_web_message
            && let Some(entry) = self.session_metadata.get(&session_key).await
            && let Some(ref binding_json) = entry.channel_binding
            && let Ok(target) =
                serde_json::from_str::<moltis_channels::ChannelReplyTarget>(binding_json)
        {
            // Only echo to channel if this is the active session for this chat.
            let is_active = self
                .session_metadata
                .get_active_session(
                    target.channel_type.as_str(),
                    &target.account_id,
                    &target.chat_id,
                )
                .await
                .map(|k| k == session_key)
                .unwrap_or(true);

            if is_active {
                match serde_json::to_value(&target) {
                    Ok(target_val) => {
                        params["_channel_reply_target"] = target_val;
                    },
                    Err(e) => {
                        warn!(
                            session = %session_key,
                            error = %e,
                            "failed to serialize channel reply target"
                        );
                    },
                }
            }
        }

        let deferred_channel_target =
            params
                .get("_channel_reply_target")
                .cloned()
                .and_then(|value| {
                    match serde_json::from_value::<moltis_channels::ChannelReplyTarget>(value) {
                        Ok(target) => Some(target),
                        Err(e) => {
                            warn!(
                                session = %session_key,
                                error = %e,
                                "ignoring invalid _channel_reply_target"
                            );
                            None
                        },
                    }
                });

        // Discover enabled skills/plugins for prompt injection.
        let search_paths = moltis_skills::discover::FsSkillDiscoverer::default_paths();
        let discoverer = moltis_skills::discover::FsSkillDiscoverer::new(search_paths);
        let discovered_skills = match discoverer.discover().await {
            Ok(s) => s,
            Err(e) => {
                warn!("failed to discover skills: {e}");
                Vec::new()
            },
        };

        // Check if MCP tools are disabled for this session and capture
        // per-session sandbox override details for prompt runtime context.
        let session_entry = self.session_metadata.get(&session_key).await;
        let session_agent_id = resolve_prompt_agent_id(session_entry.as_ref());
        let mcp_disabled = session_entry
            .as_ref()
            .and_then(|entry| entry.mcp_disabled)
            .unwrap_or(false);
        let mut runtime_context = build_prompt_runtime_context(
            &self.state,
            &provider,
            &session_key,
            session_entry.as_ref(),
        )
        .await;
        apply_request_runtime_context(&mut runtime_context.host, &params);

        let state = Arc::clone(&self.state);
        let active_runs = Arc::clone(&self.active_runs);
        let active_runs_by_session = Arc::clone(&self.active_runs_by_session);
        let active_thinking_text = Arc::clone(&self.active_thinking_text);
        let active_tool_calls = Arc::clone(&self.active_tool_calls);
        let active_reply_medium = Arc::clone(&self.active_reply_medium);
        let run_id_clone = run_id.clone();
        let tool_registry = Arc::clone(&self.tool_registry);
        let hook_registry = self.hook_registry.clone();

        // Log if tool mode is active but the provider doesn't support tools.
        // Note: We don't broadcast to the user here - they chose the model knowing
        // its limitations. The UI should show capabilities when selecting a model.
        if !stream_only && !provider.supports_tools() {
            debug!(
                provider = provider.name(),
                model = provider.id(),
                "selected provider does not support tool calling"
            );
        }

        info!(
            run_id = %run_id,
            user_message = %text,
            model = provider.id(),
            stream_only,
            session = %session_key,
            reply_medium = ?desired_reply_medium,
            client_seq = ?client_seq,
            "chat.send"
        );

        // Capture user message index (0-based) so we can include assistant
        // message index in the "final" broadcast for client-side deduplication.
        let user_message_index = history.len(); // user msg is at this index in the JSONL

        let provider_name = provider.name().to_string();
        let model_id = provider.id().to_string();
        let model_store = Arc::clone(&self.model_store);
        let session_store = Arc::clone(&self.session_store);
        let session_metadata = Arc::clone(&self.session_metadata);
        let session_agent_id_clone = session_agent_id.clone();
        let session_key_clone = session_key.clone();
        let accept_language = params
            .get("_accept_language")
            .and_then(|v| v.as_str())
            .map(String::from);
        // Auto-compact when the next request is likely to exceed 95% of the
        // model context window.
        let context_window = provider.context_window() as u64;
        let token_usage = session_token_usage_from_messages(&history);
        let estimated_next_input = token_usage
            .current_request_input_tokens
            .saturating_add(estimate_text_tokens(&text));
        let compact_threshold = (context_window * 95) / 100;

        if estimated_next_input >= compact_threshold {
            let pre_compact_msg_count = history.len();
            let pre_compact_total = token_usage
                .current_request_input_tokens
                .saturating_add(token_usage.current_request_output_tokens);

            info!(
                session = %session_key,
                estimated_next_input,
                context_window,
                "auto-compact triggered (estimated next request over 95% threshold)"
            );
            broadcast(
                &self.state,
                "chat",
                serde_json::json!({
                    "sessionKey": session_key,
                    "state": "auto_compact",
                    "phase": "start",
                    "messageCount": pre_compact_msg_count,
                    "totalTokens": pre_compact_total,
                    "inputTokens": token_usage.current_request_input_tokens,
                    "outputTokens": token_usage.current_request_output_tokens,
                    "estimatedNextInputTokens": estimated_next_input,
                    "sessionInputTokens": token_usage.session_input_tokens,
                    "sessionOutputTokens": token_usage.session_output_tokens,
                    "contextWindow": context_window,
                }),
                BroadcastOpts::default(),
            )
            .await;

            let compact_params = serde_json::json!({ "_conn_id": conn_id });
            match self.compact(compact_params).await {
                Ok(_) => {
                    // Reload history after compaction.
                    history = self
                        .session_store
                        .read(&session_key)
                        .await
                        .unwrap_or_default();
                    broadcast(
                        &self.state,
                        "chat",
                        serde_json::json!({
                            "sessionKey": session_key,
                            "state": "auto_compact",
                            "phase": "done",
                            "messageCount": pre_compact_msg_count,
                            "totalTokens": pre_compact_total,
                            "contextWindow": context_window,
                        }),
                        BroadcastOpts::default(),
                    )
                    .await;
                },
                Err(e) => {
                    warn!(session = %session_key, error = %e, "auto-compact failed, proceeding with full history");
                    broadcast(
                        &self.state,
                        "chat",
                        serde_json::json!({
                            "sessionKey": session_key,
                            "state": "auto_compact",
                            "phase": "error",
                            "error": e.to_string(),
                        }),
                        BroadcastOpts::default(),
                    )
                    .await;
                },
            }
        }

        // Try to acquire the per-session semaphore.  If a run is already active,
        // queue the message according to the configured MessageQueueMode instead
        // of blocking the caller.
        let session_sem = self.session_semaphore(&session_key).await;
        let permit: OwnedSemaphorePermit = match session_sem.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                // Active run — enqueue and return immediately.
                let queue_mode = moltis_config::discover_and_load().chat.message_queue_mode;
                info!(
                    session = %session_key,
                    mode = ?queue_mode,
                    client_seq = ?client_seq,
                    "queueing message (run active)"
                );
                let position = {
                    let mut q = self.message_queue.write().await;
                    let entry = q.entry(session_key.clone()).or_default();
                    entry.push(QueuedMessage {
                        params: params.clone(),
                    });
                    entry.len()
                };
                broadcast(
                    &self.state,
                    "chat",
                    serde_json::json!({
                        "sessionKey": session_key,
                        "state": "queued",
                        "mode": format!("{queue_mode:?}").to_lowercase(),
                        "position": position,
                    }),
                    BroadcastOpts::default(),
                )
                .await;
                return Ok(serde_json::json!({
                    "ok": true,
                    "queued": true,
                    "mode": format!("{queue_mode:?}").to_lowercase(),
                }));
            },
        };

        // Persist the user message now that we know it won't be queued.
        // (Queued messages skip this; they are persisted when replayed.)
        if let Err(e) = self
            .session_store
            .append(&session_key, &user_msg.to_value())
            .await
        {
            warn!("failed to persist user message: {e}");
        }

        // Set preview from the first user message if not already set.
        if let Some(entry) = self.session_metadata.get(&session_key).await
            && entry.preview.is_none()
        {
            let preview_text = extract_preview_from_value(&user_msg.to_value());
            if let Some(preview) = preview_text {
                self.session_metadata
                    .set_preview(&session_key, Some(&preview))
                    .await;
            }
        }

        let agent_timeout_secs = moltis_config::discover_and_load().tools.agent_timeout_secs;

        let message_queue = Arc::clone(&self.message_queue);
        let state_for_drain = Arc::clone(&self.state);
        let deferred_channel_target = deferred_channel_target.clone();

        let handle = tokio::spawn(async move {
            let permit = permit; // hold permit until agent run completes
            let ctx_ref = project_context.as_deref();
            if let Some(target) = deferred_channel_target {
                // Register the channel reply target only after we own the
                // session permit, so queued messages keep per-message routing.
                state.push_channel_reply(&session_key_clone, target).await;
            }
            active_reply_medium
                .write()
                .await
                .insert(session_key_clone.clone(), desired_reply_medium);
            if desired_reply_medium == ReplyMedium::Voice {
                broadcast(
                    &state,
                    "chat",
                    serde_json::json!({
                        "runId": run_id_clone,
                        "sessionKey": session_key_clone,
                        "state": "voice_pending",
                    }),
                    BroadcastOpts::default(),
                )
                .await;
            }
            let agent_fut = async {
                if stream_only {
                    run_streaming(
                        &state,
                        &model_store,
                        &run_id_clone,
                        provider,
                        &model_id,
                        &user_content,
                        &provider_name,
                        &history,
                        &session_key_clone,
                        &session_agent_id_clone,
                        desired_reply_medium,
                        ctx_ref,
                        user_message_index,
                        &discovered_skills,
                        Some(&runtime_context),
                        Some(&session_store),
                        client_seq,
                    )
                    .await
                } else {
                    run_with_tools(
                        &state,
                        &model_store,
                        &run_id_clone,
                        provider,
                        &model_id,
                        &tool_registry,
                        &user_content,
                        &provider_name,
                        &history,
                        &session_key_clone,
                        &session_agent_id_clone,
                        desired_reply_medium,
                        ctx_ref,
                        Some(&runtime_context),
                        user_message_index,
                        &discovered_skills,
                        hook_registry,
                        accept_language.clone(),
                        conn_id.clone(),
                        Some(&session_store),
                        mcp_disabled,
                        client_seq,
                        Some(Arc::clone(&active_thinking_text)),
                        Some(Arc::clone(&active_tool_calls)),
                    )
                    .await
                }
            };

            let assistant_text = if agent_timeout_secs > 0 {
                match tokio::time::timeout(Duration::from_secs(agent_timeout_secs), agent_fut).await
                {
                    Ok(result) => result,
                    Err(_) => {
                        warn!(
                            run_id = %run_id_clone,
                            session = %session_key_clone,
                            timeout_secs = agent_timeout_secs,
                            "agent run timed out"
                        );
                        let detail = format!("Agent run timed out after {agent_timeout_secs}s");
                        let error_obj = serde_json::json!({
                            "type": "timeout",
                            "title": "Timed out",
                            "detail": detail,
                        });
                        deliver_channel_error(&state, &session_key_clone, &error_obj).await;
                        broadcast(
                            &state,
                            "chat",
                            serde_json::json!({
                                "runId": run_id_clone,
                                "sessionKey": session_key_clone,
                                "state": "error",
                                "error": error_obj,
                            }),
                            BroadcastOpts::default(),
                        )
                        .await;
                        None
                    },
                }
            } else {
                agent_fut.await
            };

            // Persist assistant response (even empty ones — needed for LLM history coherence).
            if let Some(assistant_output) = assistant_text {
                let assistant_msg = PersistedMessage::Assistant {
                    content: assistant_output.text,
                    created_at: Some(now_ms()),
                    model: Some(model_id.clone()),
                    provider: Some(provider_name.clone()),
                    input_tokens: Some(assistant_output.input_tokens),
                    output_tokens: Some(assistant_output.output_tokens),
                    duration_ms: Some(assistant_output.duration_ms),
                    request_input_tokens: Some(assistant_output.request_input_tokens),
                    request_output_tokens: Some(assistant_output.request_output_tokens),
                    tool_calls: None,
                    reasoning: assistant_output.reasoning,
                    llm_api_response: assistant_output.llm_api_response,
                    audio: assistant_output.audio_path,
                    seq: client_seq,
                    run_id: Some(run_id_clone.clone()),
                };
                if let Err(e) = session_store
                    .append(&session_key_clone, &assistant_msg.to_value())
                    .await
                {
                    warn!("failed to persist assistant message: {e}");
                }
                // Update metadata counts.
                if let Ok(count) = session_store.count(&session_key_clone).await {
                    session_metadata.touch(&session_key_clone, count).await;
                }
            }

            active_runs.write().await.remove(&run_id_clone);
            let mut runs_by_session = active_runs_by_session.write().await;
            if runs_by_session.get(&session_key_clone) == Some(&run_id_clone) {
                runs_by_session.remove(&session_key_clone);
            }
            drop(runs_by_session);
            active_thinking_text
                .write()
                .await
                .remove(&session_key_clone);
            active_tool_calls.write().await.remove(&session_key_clone);
            active_reply_medium.write().await.remove(&session_key_clone);

            // Release the semaphore *before* draining so replayed sends can
            // acquire it. Without this, every replayed `chat.send()` would
            // fail `try_acquire_owned()` and re-queue the message forever.
            drop(permit);

            // Drain queued messages for this session.
            let queued = message_queue
                .write()
                .await
                .remove(&session_key_clone)
                .unwrap_or_default();
            if !queued.is_empty() {
                let queue_mode = moltis_config::discover_and_load().chat.message_queue_mode;
                let chat = state_for_drain.chat_service().await;
                match queue_mode {
                    MessageQueueMode::Followup => {
                        let mut iter = queued.into_iter();
                        let Some(first) = iter.next() else {
                            return;
                        };
                        // Put remaining messages back so the replayed run's
                        // own drain loop picks them up after it completes.
                        let rest: Vec<QueuedMessage> = iter.collect();
                        if !rest.is_empty() {
                            message_queue
                                .write()
                                .await
                                .entry(session_key_clone.clone())
                                .or_default()
                                .extend(rest);
                        }
                        info!(session = %session_key_clone, "replaying queued message (followup)");
                        let mut replay_params = first.params;
                        replay_params["_queued_replay"] = serde_json::json!(true);
                        if let Err(e) = chat.send(replay_params).await {
                            warn!(session = %session_key_clone, error = %e, "failed to replay queued message");
                        }
                    },
                    MessageQueueMode::Collect => {
                        let combined: Vec<&str> = queued
                            .iter()
                            .filter_map(|m| m.params.get("text").and_then(|v| v.as_str()))
                            .collect();
                        if !combined.is_empty() {
                            info!(
                                session = %session_key_clone,
                                count = combined.len(),
                                "replaying collected messages"
                            );
                            // Use the last queued message as the base params, override text.
                            let Some(last) = queued.last() else {
                                return;
                            };
                            let mut merged = last.params.clone();
                            merged["text"] = serde_json::json!(combined.join("\n\n"));
                            merged["_queued_replay"] = serde_json::json!(true);
                            if let Err(e) = chat.send(merged).await {
                                warn!(session = %session_key_clone, error = %e, "failed to replay collected messages");
                            }
                        }
                    },
                }
            }
        });

        self.active_runs
            .write()
            .await
            .insert(run_id.clone(), handle.abort_handle());
        self.active_runs_by_session
            .write()
            .await
            .insert(session_key.clone(), run_id.clone());

        Ok(serde_json::json!({ "ok": true, "runId": run_id }))
    }

    async fn send_sync(&self, params: Value) -> ServiceResult {
        let text = params
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'text' parameter".to_string())?
            .to_string();
        let desired_reply_medium = infer_reply_medium(&params, &text);

        let explicit_model = params.get("model").and_then(|v| v.as_str());
        let stream_only = !self.has_tools_sync();

        // Resolve session key from explicit override.
        let session_key = match params.get("_session_key").and_then(|v| v.as_str()) {
            Some(sk) => sk.to_string(),
            None => "main".to_string(),
        };

        // Resolve provider.
        let provider: Arc<dyn moltis_agents::model::LlmProvider> = {
            let reg = self.providers.read().await;
            if let Some(id) = explicit_model {
                reg.get(id)
                    .ok_or_else(|| format!("model '{id}' not found"))?
            } else if !stream_only {
                reg.first_with_tools()
                    .ok_or_else(|| "no LLM providers configured".to_string())?
            } else {
                reg.first()
                    .ok_or_else(|| "no LLM providers configured".to_string())?
            }
        };

        let user_audio = user_audio_path_from_params(&params, &session_key);
        // Persist the user message.
        let user_msg = PersistedMessage::User {
            content: MessageContent::Text(text.clone()),
            created_at: Some(now_ms()),
            audio: user_audio,
            channel: None,
            seq: None,
            run_id: None,
        };
        if let Err(e) = self
            .session_store
            .append(&session_key, &user_msg.to_value())
            .await
        {
            warn!("send_sync: failed to persist user message: {e}");
        }

        // Ensure this session appears in the sessions list.
        let _ = self.session_metadata.upsert(&session_key, None).await;
        self.session_metadata.touch(&session_key, 1).await;

        let session_entry = self.session_metadata.get(&session_key).await;
        let session_agent_id = resolve_prompt_agent_id(session_entry.as_ref());
        let mut runtime_context = build_prompt_runtime_context(
            &self.state,
            &provider,
            &session_key,
            session_entry.as_ref(),
        )
        .await;
        apply_request_runtime_context(&mut runtime_context.host, &params);

        // Load conversation history (excluding the message we just appended).
        let mut history = self
            .session_store
            .read(&session_key)
            .await
            .unwrap_or_default();
        if !history.is_empty() {
            history.pop();
        }

        let run_id = uuid::Uuid::new_v4().to_string();
        let state = Arc::clone(&self.state);
        let tool_registry = Arc::clone(&self.tool_registry);
        let hook_registry = self.hook_registry.clone();
        let provider_name = provider.name().to_string();
        let model_id = provider.id().to_string();
        let model_store = Arc::clone(&self.model_store);
        let user_message_index = history.len();

        info!(
            run_id = %run_id,
            user_message = %text,
            model = %model_id,
            stream_only,
            session = %session_key,
            reply_medium = ?desired_reply_medium,
            "chat.send_sync"
        );

        if desired_reply_medium == ReplyMedium::Voice {
            broadcast(
                &state,
                "chat",
                serde_json::json!({
                    "runId": run_id,
                    "sessionKey": session_key,
                    "state": "voice_pending",
                }),
                BroadcastOpts::default(),
            )
            .await;
        }

        // send_sync is text-only (used by API calls and channels).
        let user_content = UserContent::text(&text);
        let result = if stream_only {
            run_streaming(
                &state,
                &model_store,
                &run_id,
                provider,
                &model_id,
                &user_content,
                &provider_name,
                &history,
                &session_key,
                &session_agent_id,
                desired_reply_medium,
                None,
                user_message_index,
                &[],
                Some(&runtime_context),
                Some(&self.session_store),
                None, // send_sync: no client seq
            )
            .await
        } else {
            run_with_tools(
                &state,
                &model_store,
                &run_id,
                provider,
                &model_id,
                &tool_registry,
                &user_content,
                &provider_name,
                &history,
                &session_key,
                &session_agent_id,
                desired_reply_medium,
                None,
                Some(&runtime_context),
                user_message_index,
                &[],
                hook_registry,
                None,
                None, // send_sync: no conn_id
                Some(&self.session_store),
                false, // send_sync: MCP tools always enabled for API calls
                None,  // send_sync: no client seq
                None,  // send_sync: no thinking text tracking
                None,  // send_sync: no tool call tracking
            )
            .await
        };

        // Persist assistant response (even empty ones — needed for LLM history coherence).
        if let Some(ref assistant_output) = result {
            let assistant_msg = PersistedMessage::Assistant {
                content: assistant_output.text.clone(),
                created_at: Some(now_ms()),
                model: Some(model_id.clone()),
                provider: Some(provider_name.clone()),
                input_tokens: Some(assistant_output.input_tokens),
                output_tokens: Some(assistant_output.output_tokens),
                duration_ms: Some(assistant_output.duration_ms),
                request_input_tokens: Some(assistant_output.request_input_tokens),
                request_output_tokens: Some(assistant_output.request_output_tokens),
                tool_calls: None,
                reasoning: assistant_output.reasoning.clone(),
                llm_api_response: assistant_output.llm_api_response.clone(),
                audio: assistant_output.audio_path.clone(),
                seq: None,
                run_id: Some(run_id.clone()),
            };
            if let Err(e) = self
                .session_store
                .append(&session_key, &assistant_msg.to_value())
                .await
            {
                warn!("send_sync: failed to persist assistant message: {e}");
            }
            // Update metadata message count.
            if let Ok(count) = self.session_store.count(&session_key).await {
                self.session_metadata.touch(&session_key, count).await;
            }
        }

        match result {
            Some(assistant_output) => Ok(serde_json::json!({
                "text": assistant_output.text,
                "inputTokens": assistant_output.input_tokens,
                "outputTokens": assistant_output.output_tokens,
                "durationMs": assistant_output.duration_ms,
                "requestInputTokens": assistant_output.request_input_tokens,
                "requestOutputTokens": assistant_output.request_output_tokens,
            })),
            None => {
                // Check the last broadcast for this run to get the actual error message.
                let error_msg = state
                    .last_run_error(&run_id)
                    .await
                    .unwrap_or_else(|| "agent run failed (check server logs)".to_string());

                // Persist the error in the session so it's visible in session history.
                let error_entry = PersistedMessage::system(format!("[error] {error_msg}"));
                let _ = self
                    .session_store
                    .append(&session_key, &error_entry.to_value())
                    .await;
                // Update metadata so the session shows in the UI.
                if let Ok(count) = self.session_store.count(&session_key).await {
                    self.session_metadata.touch(&session_key, count).await;
                }

                Err(error_msg.into())
            },
        }
    }

    async fn abort(&self, params: Value) -> ServiceResult {
        let run_id = params.get("runId").and_then(|v| v.as_str());
        let session_key = params.get("sessionKey").and_then(|v| v.as_str());
        if run_id.is_none() && session_key.is_none() {
            return Err("missing 'runId' or 'sessionKey'".into());
        }

        let (resolved_run_id, aborted) = Self::abort_run_handle(
            &self.active_runs,
            &self.active_runs_by_session,
            run_id,
            session_key,
        )
        .await;
        info!(
            requested_run_id = ?run_id,
            session_key = ?session_key,
            resolved_run_id = ?resolved_run_id,
            aborted,
            "chat.abort"
        );

        if aborted && let Some(key) = session_key {
            self.active_thinking_text.write().await.remove(key);
            self.active_tool_calls.write().await.remove(key);
            self.active_reply_medium.write().await.remove(key);
            broadcast(
                &self.state,
                "chat",
                serde_json::json!({
                    "state": "aborted",
                    "runId": resolved_run_id,
                    "sessionKey": key,
                }),
                BroadcastOpts::default(),
            )
            .await;
        }

        Ok(serde_json::json!({ "aborted": aborted, "runId": resolved_run_id }))
    }

    async fn cancel_queued(&self, params: Value) -> ServiceResult {
        let session_key = params
            .get("sessionKey")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'sessionKey'".to_string())?;

        let removed = self
            .message_queue
            .write()
            .await
            .remove(session_key)
            .unwrap_or_default();
        let count = removed.len();
        info!(session = %session_key, count, "cancel_queued: cleared message queue");

        broadcast(
            &self.state,
            "chat",
            serde_json::json!({
                "sessionKey": session_key,
                "state": "queue_cleared",
                "count": count,
            }),
            BroadcastOpts::default(),
        )
        .await;

        Ok(serde_json::json!({ "cleared": count }))
    }

    async fn history(&self, params: Value) -> ServiceResult {
        let conn_id = params
            .get("_conn_id")
            .and_then(|v| v.as_str())
            .map(String::from);
        let session_key = self.session_key_for(conn_id.as_deref()).await;
        let messages = self
            .session_store
            .read(&session_key)
            .await
            .map_err(ServiceError::message)?;
        // Filter out empty assistant messages — they are kept in storage for LLM
        // history coherence but should not be shown in the UI.
        let visible: Vec<Value> = messages
            .into_iter()
            .filter(|msg| {
                if msg.get("role").and_then(|v| v.as_str()) != Some("assistant") {
                    return true;
                }
                msg.get("content")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| !s.trim().is_empty())
            })
            .collect();
        Ok(serde_json::json!(visible))
    }

    async fn inject(&self, _params: Value) -> ServiceResult {
        Err("inject not yet implemented".into())
    }

    async fn clear(&self, params: Value) -> ServiceResult {
        let session_key = if let Some(sk) = params.get("_session_key").and_then(|v| v.as_str()) {
            sk.to_string()
        } else {
            let conn_id = params
                .get("_conn_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            self.session_key_for(conn_id.as_deref()).await
        };

        self.session_store
            .clear(&session_key)
            .await
            .map_err(ServiceError::message)?;

        // Reset client sequence tracking for this session. A cleared chat starts
        // a fresh sequence from the web UI.
        {
            let mut seq_map = self.last_client_seq.write().await;
            seq_map.remove(&session_key);
        }

        // Reset metadata message count and preview.
        self.session_metadata.touch(&session_key, 0).await;
        self.session_metadata.set_preview(&session_key, None).await;

        // Notify all WebSocket clients so the web UI clears the session
        // even when /clear is issued from a channel (e.g. Telegram).
        broadcast(
            &self.state,
            "chat",
            serde_json::json!({
                "sessionKey": session_key,
                "state": "session_cleared",
            }),
            BroadcastOpts::default(),
        )
        .await;

        info!(session = %session_key, "chat.clear");
        Ok(serde_json::json!({ "ok": true }))
    }

    async fn compact(&self, params: Value) -> ServiceResult {
        let session_key = if let Some(sk) = params.get("_session_key").and_then(|v| v.as_str()) {
            sk.to_string()
        } else {
            let conn_id = params
                .get("_conn_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            self.session_key_for(conn_id.as_deref()).await
        };
        let session_entry = self.session_metadata.get(&session_key).await;
        let session_agent_id = resolve_prompt_agent_id(session_entry.as_ref());

        let history = self
            .session_store
            .read(&session_key)
            .await
            .map_err(ServiceError::message)?;

        if history.is_empty() {
            return Err("nothing to compact".into());
        }

        // Dispatch BeforeCompaction hook.
        if let Some(ref hooks) = self.hook_registry {
            let payload = moltis_common::hooks::HookPayload::BeforeCompaction {
                session_key: session_key.clone(),
                message_count: history.len(),
            };
            if let Err(e) = hooks.dispatch(&payload).await {
                warn!(session = %session_key, error = %e, "BeforeCompaction hook failed");
            }
        }

        // Run silent memory turn before summarization — saves important memories to disk.
        // The manager implements MemoryWriter directly (with path validation, size limits,
        // and automatic re-indexing), so no manual sync_path is needed after the turn.
        if let Some(mm) = self.state.memory_manager()
            && let Ok(provider) = self.resolve_provider(&session_key, &history).await
        {
            let chat_history_for_memory = values_to_chat_messages(&history);
            let writer: Arc<dyn moltis_agents::memory_writer::MemoryWriter> = Arc::new(
                AgentScopedMemoryWriter::new(Arc::clone(mm), session_agent_id.clone()),
            );
            match moltis_agents::silent_turn::run_silent_memory_turn(
                provider,
                &chat_history_for_memory,
                writer,
            )
            .await
            {
                Ok(paths) => {
                    if !paths.is_empty() {
                        info!(
                            files = paths.len(),
                            "compact: silent memory turn wrote files"
                        );
                    }
                },
                Err(e) => warn!(error = %e, "compact: silent memory turn failed"),
            }
        }

        // Build a summary prompt from the conversation using structured messages.
        // We pass the typed ChatMessage objects directly so role boundaries are
        // maintained via the API's message structure, preventing prompt injection
        // where user content could mimic role prefixes in concatenated text.
        let mut summary_messages = vec![ChatMessage::system(
            "You are a conversation summarizer. The messages that follow are a conversation you must summarize. Preserve all key facts, decisions, and context. After the conversation, you will receive a final instruction.",
        )];
        summary_messages.extend(values_to_chat_messages(&history));
        summary_messages.push(ChatMessage::user(
            "Summarize the conversation above into a concise form. Output only the summary, no preamble.",
        ));

        // Use the session's model if available, otherwise fall back to the model
        // from the last assistant message, then to the first registered provider.
        let provider = self
            .resolve_provider(&session_key, &history)
            .await
            .map_err(ServiceError::message)?;

        info!(session = %session_key, messages = history.len(), "chat.compact: summarizing");

        let mut stream = provider.stream(summary_messages);
        let mut summary = String::new();
        while let Some(event) = stream.next().await {
            match event {
                StreamEvent::Delta(delta) => summary.push_str(&delta),
                StreamEvent::Done(_) => break,
                StreamEvent::Error(e) => {
                    return Err(format!("compact summarization failed: {e}").into());
                },
                // Tool events not expected in summarization stream.
                StreamEvent::ToolCallStart { .. }
                | StreamEvent::ToolCallArgumentsDelta { .. }
                | StreamEvent::ToolCallComplete { .. }
                // Provider raw payloads are debug metadata, not summary text.
                | StreamEvent::ProviderRaw(_)
                // Ignore provider reasoning blocks; summary body should only
                // include final answer text.
                | StreamEvent::ReasoningDelta(_) => {},
            }
        }

        if summary.is_empty() {
            return Err("compact produced empty summary".into());
        }

        // Replace history with a single assistant message containing the summary.
        let compacted_msg = PersistedMessage::Assistant {
            content: format!("[Conversation Summary]\n\n{summary}"),
            created_at: Some(now_ms()),
            model: None,
            provider: None,
            input_tokens: None,
            output_tokens: None,
            duration_ms: None,
            request_input_tokens: None,
            request_output_tokens: None,
            tool_calls: None,
            reasoning: None,
            llm_api_response: None,
            audio: None,
            seq: None,
            run_id: None,
        };
        let compacted = vec![compacted_msg.to_value()];

        self.session_store
            .replace_history(&session_key, compacted.clone())
            .await
            .map_err(ServiceError::message)?;

        self.session_metadata.touch(&session_key, 1).await;

        // Save compaction summary to memory file and trigger sync.
        if let Some(mm) = self.state.memory_manager() {
            let memory_dir = moltis_config::agent_workspace_dir(&session_agent_id).join("memory");
            if let Err(e) = tokio::fs::create_dir_all(&memory_dir).await {
                warn!(error = %e, "compact: failed to create memory dir");
            } else {
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let filename = format!("compaction-{}-{ts}.md", session_key);
                let path = memory_dir.join(&filename);
                let content = format!(
                    "# Compaction Summary\n\n- **Session**: {session_key}\n- **Timestamp**: {ts}\n\n{summary}"
                );
                if let Err(e) = tokio::fs::write(&path, &content).await {
                    warn!(error = %e, "compact: failed to write memory file");
                } else {
                    let mm = Arc::clone(mm);
                    tokio::spawn(async move {
                        if let Err(e) = mm.sync().await {
                            tracing::warn!("compact: memory sync failed: {e}");
                        }
                    });
                }
            }
        }

        // Dispatch AfterCompaction hook.
        if let Some(ref hooks) = self.hook_registry {
            let payload = moltis_common::hooks::HookPayload::AfterCompaction {
                session_key: session_key.clone(),
                summary_len: summary.len(),
            };
            if let Err(e) = hooks.dispatch(&payload).await {
                warn!(session = %session_key, error = %e, "AfterCompaction hook failed");
            }
        }

        info!(session = %session_key, "chat.compact: done");
        Ok(serde_json::json!(compacted))
    }

    async fn context(&self, params: Value) -> ServiceResult {
        let session_key = if let Some(sk) = params.get("_session_key").and_then(|v| v.as_str()) {
            sk.to_string()
        } else {
            let conn_id = params
                .get("_conn_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            self.session_key_for(conn_id.as_deref()).await
        };

        // Session info
        let message_count = self.session_store.count(&session_key).await.unwrap_or(0);
        let session_entry = self.session_metadata.get(&session_key).await;
        let (provider_name, supports_tools) = {
            let reg = self.providers.read().await;
            let session_model = session_entry.as_ref().and_then(|e| e.model.as_deref());
            if let Some(id) = session_model {
                let p = reg.get(id);
                (
                    p.as_ref().map(|p| p.name().to_string()),
                    p.as_ref().map(|p| p.supports_tools()).unwrap_or(true),
                )
            } else {
                let p = reg.first();
                (
                    p.as_ref().map(|p| p.name().to_string()),
                    p.as_ref().map(|p| p.supports_tools()).unwrap_or(true),
                )
            }
        };
        let session_info = serde_json::json!({
            "key": session_key,
            "messageCount": message_count,
            "model": session_entry.as_ref().and_then(|e| e.model.as_deref()),
            "provider": provider_name,
            "label": session_entry.as_ref().and_then(|e| e.label.as_deref()),
            "projectId": session_entry.as_ref().and_then(|e| e.project_id.as_deref()),
        });

        // Project info & context files
        let conn_id = params
            .get("_conn_id")
            .and_then(|v| v.as_str())
            .map(String::from);
        let project_id = if let Some(cid) = conn_id.as_deref() {
            self.state.active_project_id(cid).await
        } else {
            None
        };
        let project_id =
            project_id.or_else(|| session_entry.as_ref().and_then(|e| e.project_id.clone()));

        let project_info = if let Some(pid) = project_id {
            match self
                .state
                .project_service()
                .get(serde_json::json!({"id": pid}))
                .await
            {
                Ok(val) => {
                    let dir = val.get("directory").and_then(|v| v.as_str());
                    let context_files = if let Some(d) = dir {
                        match moltis_projects::context::load_context_files(Path::new(d)) {
                            Ok(files) => files
                                .iter()
                                .map(|f| {
                                    serde_json::json!({
                                        "path": f.path.display().to_string(),
                                        "size": f.content.len(),
                                    })
                                })
                                .collect::<Vec<_>>(),
                            Err(_) => vec![],
                        }
                    } else {
                        vec![]
                    };
                    serde_json::json!({
                        "id": val.get("id"),
                        "label": val.get("label"),
                        "directory": dir,
                        "systemPrompt": val.get("system_prompt").or(val.get("systemPrompt")),
                        "contextFiles": context_files,
                    })
                },
                Err(_) => serde_json::json!(null),
            }
        } else {
            serde_json::json!(null)
        };

        // Tools (only include if the provider supports tool calling)
        let mcp_disabled = session_entry
            .as_ref()
            .and_then(|e| e.mcp_disabled)
            .unwrap_or(false);
        let config = moltis_config::discover_and_load();
        let tools: Vec<Value> = if supports_tools {
            let registry_guard = self.tool_registry.read().await;
            let effective_registry =
                apply_runtime_tool_filters(&registry_guard, &config, &[], mcp_disabled);
            effective_registry
                .list_schemas()
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "name": s.get("name").and_then(|v| v.as_str()).unwrap_or("unknown"),
                        "description": s.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                    })
                })
                .collect()
        } else {
            vec![]
        };

        // Token usage from API-reported counts stored in messages.
        let messages = self
            .session_store
            .read(&session_key)
            .await
            .unwrap_or_default();
        let usage = session_token_usage_from_messages(&messages);
        let total_tokens = usage.session_input_tokens + usage.session_output_tokens;
        let current_total_tokens =
            usage.current_request_input_tokens + usage.current_request_output_tokens;

        // Context window from the session's provider
        let context_window = {
            let reg = self.providers.read().await;
            let session_model = session_entry.as_ref().and_then(|e| e.model.as_deref());
            if let Some(id) = session_model {
                reg.get(id).map(|p| p.context_window()).unwrap_or(200_000)
            } else {
                reg.first().map(|p| p.context_window()).unwrap_or(200_000)
            }
        };

        // Sandbox info
        let sandbox_info = if let Some(router) = self.state.sandbox_router() {
            let is_sandboxed = router.is_sandboxed(&session_key).await;
            let config = router.config();
            let session_image = session_entry.as_ref().and_then(|e| e.sandbox_image.clone());
            let effective_image = match session_image {
                Some(img) if !img.is_empty() => img,
                _ => router.default_image().await,
            };
            let container_name = {
                let id = router.sandbox_id_for(&session_key);
                format!(
                    "{}-{}",
                    config
                        .container_prefix
                        .as_deref()
                        .unwrap_or("moltis-sandbox"),
                    id.key
                )
            };
            serde_json::json!({
                "enabled": is_sandboxed,
                "backend": router.backend_name(),
                "mode": config.mode,
                "scope": config.scope,
                "workspaceMount": config.workspace_mount,
                "image": effective_image,
                "containerName": container_name,
            })
        } else {
            serde_json::json!({
                "enabled": false,
                "backend": null,
            })
        };
        let sandbox_enabled = sandbox_info
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let host_is_root = detect_host_root_user().await;
        // Sandbox containers currently run as root by default.
        let exec_is_root = if sandbox_enabled {
            Some(true)
        } else {
            host_is_root
        };
        let exec_prompt_symbol = exec_is_root.map(|is_root| {
            if is_root {
                "#"
            } else {
                "$"
            }
        });
        let execution_info = serde_json::json!({
            "mode": if sandbox_enabled { "sandbox" } else { "host" },
            "hostIsRoot": host_is_root,
            "isRoot": exec_is_root,
            "promptSymbol": exec_prompt_symbol,
        });

        // Discover enabled skills/plugins (only if provider supports tools)
        let skills_list: Vec<Value> = if supports_tools {
            let search_paths = moltis_skills::discover::FsSkillDiscoverer::default_paths();
            let discoverer = moltis_skills::discover::FsSkillDiscoverer::new(search_paths);
            match discoverer.discover().await {
                Ok(s) => s
                    .iter()
                    .map(|s| {
                        serde_json::json!({
                            "name": s.name,
                            "description": s.description,
                            "source": s.source,
                        })
                    })
                    .collect(),
                Err(_) => vec![],
            }
        } else {
            vec![]
        };

        // MCP servers (only if provider supports tools)
        let mcp_servers = if supports_tools {
            self.state
                .mcp_service()
                .list()
                .await
                .unwrap_or(serde_json::json!([]))
        } else {
            serde_json::json!([])
        };

        Ok(serde_json::json!({
            "session": session_info,
            "project": project_info,
            "tools": tools,
            "skills": skills_list,
            "mcpServers": mcp_servers,
            "mcpDisabled": mcp_disabled,
            "sandbox": sandbox_info,
            "execution": execution_info,
            "supportsTools": supports_tools,
            "tokenUsage": {
                "inputTokens": usage.session_input_tokens,
                "outputTokens": usage.session_output_tokens,
                "total": total_tokens,
                "currentInputTokens": usage.current_request_input_tokens,
                "currentOutputTokens": usage.current_request_output_tokens,
                "currentTotal": current_total_tokens,
                "estimatedNextInputTokens": usage.current_request_input_tokens,
                "contextWindow": context_window,
            },
        }))
    }

    async fn raw_prompt(&self, params: Value) -> ServiceResult {
        let session_key = if let Some(sk) = params.get("_session_key").and_then(|v| v.as_str()) {
            sk.to_string()
        } else {
            let conn_id = params
                .get("_conn_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            self.session_key_for(conn_id.as_deref()).await
        };

        let conn_id = params
            .get("_conn_id")
            .and_then(|v| v.as_str())
            .map(String::from);

        // Resolve provider.
        let history = self
            .session_store
            .read(&session_key)
            .await
            .unwrap_or_default();
        let provider = self
            .resolve_provider(&session_key, &history)
            .await
            .map_err(ServiceError::message)?;
        let tool_mode = effective_tool_mode(&*provider);
        let native_tools = matches!(tool_mode, ToolMode::Native);
        let tools_enabled = !matches!(tool_mode, ToolMode::Off);

        // Build runtime context.
        let session_entry = self.session_metadata.get(&session_key).await;
        let persona = load_prompt_persona_for_session(session_entry.as_ref());
        let mut runtime_context = build_prompt_runtime_context(
            &self.state,
            &provider,
            &session_key,
            session_entry.as_ref(),
        )
        .await;
        apply_request_runtime_context(&mut runtime_context.host, &params);

        // Resolve project context.
        let project_context = self
            .resolve_project_context(&session_key, conn_id.as_deref())
            .await;

        // Discover skills.
        let search_paths = moltis_skills::discover::FsSkillDiscoverer::default_paths();
        let discoverer = moltis_skills::discover::FsSkillDiscoverer::new(search_paths);
        let discovered_skills = match discoverer.discover().await {
            Ok(s) => s,
            Err(e) => {
                warn!("failed to discover skills: {e}");
                Vec::new()
            },
        };

        // Check MCP disabled.
        let mcp_disabled = session_entry
            .as_ref()
            .and_then(|entry| entry.mcp_disabled)
            .unwrap_or(false);

        // Build filtered tool registry.
        let filtered_registry = {
            let registry_guard = self.tool_registry.read().await;
            if tools_enabled {
                apply_runtime_tool_filters(
                    &registry_guard,
                    &persona.config,
                    &discovered_skills,
                    mcp_disabled,
                )
            } else {
                registry_guard.clone_without(&[])
            }
        };

        let tool_count = filtered_registry.list_schemas().len();

        // Build the system prompt.
        let system_prompt = if tools_enabled {
            build_system_prompt_with_session_runtime(
                &filtered_registry,
                native_tools,
                project_context.as_deref(),
                &discovered_skills,
                Some(&persona.identity),
                Some(&persona.user),
                persona.soul_text.as_deref(),
                persona.agents_text.as_deref(),
                persona.tools_text.as_deref(),
                Some(&runtime_context),
                persona.memory_text.as_deref(),
            )
        } else {
            build_system_prompt_minimal_runtime(
                project_context.as_deref(),
                Some(&persona.identity),
                Some(&persona.user),
                persona.soul_text.as_deref(),
                persona.agents_text.as_deref(),
                persona.tools_text.as_deref(),
                Some(&runtime_context),
                persona.memory_text.as_deref(),
            )
        };

        let char_count = system_prompt.len();

        Ok(serde_json::json!({
            "prompt": system_prompt,
            "charCount": char_count,
            "native_tools": native_tools,
            "tools_enabled": tools_enabled,
            "tool_mode": format!("{:?}", tool_mode),
            "toolCount": tool_count,
        }))
    }

    /// Return the **full messages array** that would be sent to the LLM on the
    /// next call — system prompt + conversation history — in OpenAI format.
    async fn full_context(&self, params: Value) -> ServiceResult {
        let session_key = if let Some(sk) = params.get("_session_key").and_then(|v| v.as_str()) {
            sk.to_string()
        } else {
            let conn_id = params
                .get("_conn_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            self.session_key_for(conn_id.as_deref()).await
        };

        let conn_id = params
            .get("_conn_id")
            .and_then(|v| v.as_str())
            .map(String::from);

        // Resolve provider.
        let history = self
            .session_store
            .read(&session_key)
            .await
            .unwrap_or_default();
        let provider = self
            .resolve_provider(&session_key, &history)
            .await
            .map_err(ServiceError::message)?;
        let tool_mode = effective_tool_mode(&*provider);
        let native_tools = matches!(tool_mode, ToolMode::Native);
        let tools_enabled = !matches!(tool_mode, ToolMode::Off);

        // Build runtime context.
        let session_entry = self.session_metadata.get(&session_key).await;
        let persona = load_prompt_persona_for_session(session_entry.as_ref());
        let mut runtime_context = build_prompt_runtime_context(
            &self.state,
            &provider,
            &session_key,
            session_entry.as_ref(),
        )
        .await;
        apply_request_runtime_context(&mut runtime_context.host, &params);

        // Resolve project context.
        let project_context = self
            .resolve_project_context(&session_key, conn_id.as_deref())
            .await;

        // Discover skills.
        let search_paths = moltis_skills::discover::FsSkillDiscoverer::default_paths();
        let discoverer = moltis_skills::discover::FsSkillDiscoverer::new(search_paths);
        let discovered_skills = match discoverer.discover().await {
            Ok(s) => s,
            Err(e) => {
                warn!("failed to discover skills: {e}");
                Vec::new()
            },
        };

        // Check MCP disabled.
        let mcp_disabled = session_entry
            .as_ref()
            .and_then(|entry| entry.mcp_disabled)
            .unwrap_or(false);

        // Build filtered tool registry.
        let filtered_registry = {
            let registry_guard = self.tool_registry.read().await;
            if tools_enabled {
                apply_runtime_tool_filters(
                    &registry_guard,
                    &persona.config,
                    &discovered_skills,
                    mcp_disabled,
                )
            } else {
                registry_guard.clone_without(&[])
            }
        };

        // Build the system prompt.
        let system_prompt = if tools_enabled {
            build_system_prompt_with_session_runtime(
                &filtered_registry,
                native_tools,
                project_context.as_deref(),
                &discovered_skills,
                Some(&persona.identity),
                Some(&persona.user),
                persona.soul_text.as_deref(),
                persona.agents_text.as_deref(),
                persona.tools_text.as_deref(),
                Some(&runtime_context),
                persona.memory_text.as_deref(),
            )
        } else {
            build_system_prompt_minimal_runtime(
                project_context.as_deref(),
                Some(&persona.identity),
                Some(&persona.user),
                persona.soul_text.as_deref(),
                persona.agents_text.as_deref(),
                persona.tools_text.as_deref(),
                Some(&runtime_context),
                persona.memory_text.as_deref(),
            )
        };

        let system_prompt_chars = system_prompt.len();

        // Keep raw assistant outputs (including provider/model/token metadata)
        // so the UI can show a debug view of what the LLM actually returned.
        let llm_outputs: Vec<Value> = history
            .iter()
            .filter(|entry| entry.get("role").and_then(|r| r.as_str()) == Some("assistant"))
            .cloned()
            .collect();

        // Reconstruct `role: "tool"` messages from persisted `tool_result`
        // entries so the context view shows what the LLM actually saw.
        let history_with_tools: Vec<Value> = history
            .into_iter()
            .map(|val| {
                if val.get("role").and_then(|r| r.as_str()) != Some("tool_result") {
                    return val;
                }
                let tool_call_id = val
                    .get("tool_call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let content = if let Some(err) = val.get("error").and_then(|v| v.as_str()) {
                    format!("Error: {err}")
                } else if let Some(res) = val.get("result") {
                    res.to_string()
                } else {
                    String::new()
                };
                serde_json::json!({
                    "role": "tool",
                    "tool_call_id": tool_call_id,
                    "content": content,
                })
            })
            .collect();

        // Build the full messages array: system prompt + conversation history.
        let mut messages = Vec::with_capacity(1 + history_with_tools.len());
        messages.push(ChatMessage::system(system_prompt));
        messages.extend(values_to_chat_messages(&history_with_tools));

        let openai_messages: Vec<Value> = messages.iter().map(|m| m.to_openai_value()).collect();
        let message_count = openai_messages.len();
        let total_chars: usize = openai_messages
            .iter()
            .map(|v| serde_json::to_string(v).unwrap_or_default().len())
            .sum();

        Ok(serde_json::json!({
            "messages": openai_messages,
            "llmOutputs": llm_outputs,
            "messageCount": message_count,
            "systemPromptChars": system_prompt_chars,
            "totalChars": total_chars,
        }))
    }

    async fn active(&self, params: Value) -> ServiceResult {
        let session_key = params
            .get("sessionKey")
            .or_else(|| params.get("session_key"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'sessionKey' parameter".to_string())?;
        let active = self
            .active_runs_by_session
            .read()
            .await
            .contains_key(session_key);
        Ok(serde_json::json!({ "active": active }))
    }

    async fn active_session_keys(&self) -> Vec<String> {
        self.active_runs_by_session
            .read()
            .await
            .keys()
            .cloned()
            .collect()
    }

    async fn active_thinking_text(&self, session_key: &str) -> Option<String> {
        self.active_thinking_text
            .read()
            .await
            .get(session_key)
            .cloned()
    }

    async fn active_voice_pending(&self, session_key: &str) -> bool {
        self.active_reply_medium
            .read()
            .await
            .get(session_key)
            .is_some_and(|m| *m == ReplyMedium::Voice)
    }

    async fn peek(&self, params: Value) -> ServiceResult {
        let session_key = params
            .get("sessionKey")
            .and_then(|v| v.as_str())
            .unwrap_or("main");

        let active = self
            .active_runs_by_session
            .read()
            .await
            .contains_key(session_key);

        if !active {
            return Ok(serde_json::json!({ "active": false }));
        }

        let thinking_text = self
            .active_thinking_text
            .read()
            .await
            .get(session_key)
            .cloned();

        let tool_calls: Vec<ActiveToolCall> = self
            .active_tool_calls
            .read()
            .await
            .get(session_key)
            .cloned()
            .unwrap_or_default();

        Ok(serde_json::json!({
            "active": true,
            "sessionKey": session_key,
            "thinkingText": thinking_text,
            "toolCalls": tool_calls,
        }))
    }
}

// ── Agent loop mode ─────────────────────────────────────────────────────────

async fn mark_unsupported_model(
    state: &Arc<dyn ChatRuntime>,
    model_store: &Arc<RwLock<DisabledModelsStore>>,
    model_id: &str,
    provider_name: &str,
    error_obj: &Value,
) {
    if error_obj.get("type").and_then(|v| v.as_str()) != Some("unsupported_model") {
        return;
    }

    let detail = error_obj
        .get("detail")
        .and_then(|v| v.as_str())
        .unwrap_or("Model is not supported for this account/provider");
    let provider = error_obj
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or(provider_name);

    let mut store = model_store.write().await;
    if store.mark_unsupported(model_id, detail, Some(provider)) {
        let unsupported = store.unsupported_info(model_id).cloned();
        if let Err(err) = store.save() {
            warn!(
                model = model_id,
                provider = provider,
                error = %err,
                "failed to persist unsupported model flag"
            );
        } else {
            info!(
                model = model_id,
                provider = provider,
                "flagged model as unsupported"
            );
        }
        drop(store);
        broadcast(
            state,
            "models.updated",
            serde_json::json!({
                "modelId": model_id,
                "unsupported": true,
                "unsupportedReason": unsupported.as_ref().map(|u| u.detail.as_str()).unwrap_or(detail),
                "unsupportedProvider": unsupported
                    .as_ref()
                    .and_then(|u| u.provider.as_deref())
                    .unwrap_or(provider),
                "unsupportedUpdatedAt": unsupported.map(|u| u.updated_at_ms).unwrap_or_else(now_ms),
            }),
            BroadcastOpts::default(),
        )
        .await;
    }
}

async fn clear_unsupported_model(
    state: &Arc<dyn ChatRuntime>,
    model_store: &Arc<RwLock<DisabledModelsStore>>,
    model_id: &str,
) {
    let mut store = model_store.write().await;
    if store.clear_unsupported(model_id) {
        if let Err(err) = store.save() {
            warn!(
                model = model_id,
                error = %err,
                "failed to persist unsupported model clear"
            );
        } else {
            info!(model = model_id, "cleared unsupported model flag");
        }
        drop(store);
        broadcast(
            state,
            "models.updated",
            serde_json::json!({
                "modelId": model_id,
                "unsupported": false,
            }),
            BroadcastOpts::default(),
        )
        .await;
    }
}

fn ordered_runner_event_callback() -> (
    Box<dyn Fn(RunnerEvent) + Send + Sync>,
    mpsc::UnboundedReceiver<RunnerEvent>,
) {
    let (tx, rx) = mpsc::unbounded_channel::<RunnerEvent>();
    let callback: Box<dyn Fn(RunnerEvent) + Send + Sync> = Box::new(move |event| {
        if tx.send(event).is_err() {
            debug!("runner event dropped because event processor is closed");
        }
    });
    (callback, rx)
}

const CHANNEL_STREAM_BUFFER_SIZE: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ChannelReplyTargetKey {
    channel_type: moltis_channels::ChannelType,
    account_id: String,
    chat_id: String,
    message_id: Option<String>,
}

impl From<&moltis_channels::ChannelReplyTarget> for ChannelReplyTargetKey {
    fn from(target: &moltis_channels::ChannelReplyTarget) -> Self {
        Self {
            channel_type: target.channel_type,
            account_id: target.account_id.clone(),
            chat_id: target.chat_id.clone(),
            message_id: target.message_id.clone(),
        }
    }
}

struct ChannelStreamWorker {
    sender: moltis_channels::StreamSender,
}

/// Fan out model deltas to channel stream workers (Telegram/Discord edit-in-place).
///
/// Workers are started eagerly so channel typing indicators remain active
/// during long-running tool execution before the first text delta arrives.
/// Stream-dedup only applies after at least one delta has been sent.
struct ChannelStreamDispatcher {
    outbound: Arc<dyn moltis_channels::plugin::ChannelStreamOutbound>,
    targets: Vec<moltis_channels::ChannelReplyTarget>,
    workers: Vec<ChannelStreamWorker>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    completed: Arc<Mutex<HashSet<ChannelReplyTargetKey>>>,
    started: bool,
    sent_delta: bool,
}

impl ChannelStreamDispatcher {
    async fn for_session(state: &Arc<dyn ChatRuntime>, session_key: &str) -> Option<Self> {
        let outbound = state.channel_stream_outbound()?;
        let targets: Vec<moltis_channels::ChannelReplyTarget> = state
            .peek_channel_replies(session_key)
            .await
            .into_iter()
            .collect();
        if targets.is_empty() {
            return None;
        }
        let mut dispatcher = Self {
            outbound,
            targets,
            workers: Vec::new(),
            tasks: Vec::new(),
            completed: Arc::new(Mutex::new(HashSet::new())),
            started: false,
            sent_delta: false,
        };
        dispatcher.ensure_started().await;
        Some(dispatcher)
    }

    async fn ensure_started(&mut self) {
        if self.started {
            return;
        }
        self.started = true;

        for target in self.targets.iter().cloned() {
            if !self.outbound.is_stream_enabled(&target.account_id).await {
                debug!(
                    account_id = target.account_id.as_str(),
                    chat_id = target.chat_id.as_str(),
                    "channel streaming disabled for target account"
                );
                continue;
            }

            let key = ChannelReplyTargetKey::from(&target);
            let (tx, rx) = mpsc::channel(CHANNEL_STREAM_BUFFER_SIZE);
            let outbound = Arc::clone(&self.outbound);
            let completed = Arc::clone(&self.completed);
            let account_id = target.account_id.clone();
            let chat_id = target.chat_id.clone();
            let reply_to = target.message_id.clone();
            let key_for_insert = key.clone();
            let account_for_log = account_id.clone();
            let chat_for_log = chat_id.clone();

            self.workers.push(ChannelStreamWorker { sender: tx });
            self.tasks.push(tokio::spawn(async move {
                match outbound
                    .send_stream(&account_id, &chat_id, reply_to.as_deref(), rx)
                    .await
                {
                    Ok(()) => {
                        completed.lock().await.insert(key_for_insert);
                    },
                    Err(e) => {
                        warn!(
                            account_id = account_for_log,
                            chat_id = chat_for_log,
                            "channel stream outbound failed: {e}"
                        );
                    },
                }
            }));
        }
    }

    async fn send_delta(&mut self, delta: &str) {
        if delta.is_empty() {
            return;
        }
        self.sent_delta = true;
        self.ensure_started().await;
        let event = moltis_channels::StreamEvent::Delta(delta.to_string());
        for worker in &self.workers {
            if worker.sender.send(event.clone()).await.is_err() {
                debug!("channel stream delta dropped: worker closed");
            }
        }
    }

    async fn finish(&mut self) {
        self.send_terminal(moltis_channels::StreamEvent::Done).await;
        self.join_workers().await;
    }

    async fn send_terminal(&mut self, event: moltis_channels::StreamEvent) {
        if self.workers.is_empty() {
            return;
        }
        let workers = std::mem::take(&mut self.workers);
        for worker in &workers {
            if worker.sender.send(event.clone()).await.is_err() {
                debug!("channel stream terminal event dropped: worker closed");
            }
        }
    }

    async fn join_workers(&mut self) {
        let tasks = std::mem::take(&mut self.tasks);
        for task in tasks {
            if let Err(e) = task.await {
                warn!(error = %e, "channel stream worker task join failed");
            }
        }
    }

    async fn completed_target_keys(&self) -> HashSet<ChannelReplyTargetKey> {
        if !self.sent_delta {
            return HashSet::new();
        }
        self.completed.lock().await.clone()
    }
}

async fn run_explicit_shell_command(
    state: &Arc<dyn ChatRuntime>,
    run_id: &str,
    tool_registry: &Arc<RwLock<ToolRegistry>>,
    session_store: &Arc<SessionStore>,
    session_key: &str,
    command: &str,
    user_message_index: usize,
    accept_language: Option<String>,
    conn_id: Option<String>,
    client_seq: Option<u64>,
) -> AssistantTurnOutput {
    let started = Instant::now();
    let tool_call_id = format!("sh_{}", uuid::Uuid::new_v4().simple());
    let tool_args = serde_json::json!({ "command": command });

    send_tool_status_to_channels(state, session_key, "exec", &tool_args).await;

    broadcast(
        state,
        "chat",
        serde_json::json!({
            "runId": run_id,
            "sessionKey": session_key,
            "state": "tool_call_start",
            "toolCallId": tool_call_id,
            "toolName": "exec",
            "arguments": tool_args,
            "seq": client_seq,
        }),
        BroadcastOpts::default(),
    )
    .await;

    let mut exec_params = serde_json::json!({
        "command": command,
        "_session_key": session_key,
    });
    if let Some(lang) = accept_language.as_deref() {
        exec_params["_accept_language"] = serde_json::json!(lang);
    }
    if let Some(cid) = conn_id.as_deref() {
        exec_params["_conn_id"] = serde_json::json!(cid);
    }

    let exec_tool = {
        let registry = tool_registry.read().await;
        registry.get_arc("exec")
    };

    let exec_result = match exec_tool {
        Some(tool) => tool.execute(exec_params).await,
        None => Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "exec tool is not registered",
        )
        .into()),
    };

    let has_channel_targets = !state.peek_channel_replies(session_key).await.is_empty();
    let mut final_text = String::new();

    match exec_result {
        Ok(result) => {
            let capped = capped_tool_result_payload(&result, 10_000);
            let tool_result_msg = PersistedMessage::tool_result(
                tool_call_id.clone(),
                "exec",
                Some(serde_json::json!({ "command": command })),
                true,
                Some(capped.clone()),
                None,
            );
            if let Err(e) = session_store
                .append(session_key, &tool_result_msg.to_value())
                .await
            {
                warn!("failed to persist direct /sh tool result: {e}");
            }

            broadcast(
                state,
                "chat",
                serde_json::json!({
                    "runId": run_id,
                    "sessionKey": session_key,
                    "state": "tool_call_end",
                    "toolCallId": tool_call_id,
                    "toolName": "exec",
                    "success": true,
                    "result": capped,
                    "seq": client_seq,
                }),
                BroadcastOpts::default(),
            )
            .await;

            if has_channel_targets {
                final_text = shell_reply_text_from_exec_result(&result);
                if final_text.is_empty() {
                    final_text = "Command completed.".to_string();
                }
            }
        },
        Err(err) => {
            let error_text = err.to_string();
            let parsed_error = parse_chat_error(&error_text, None);
            let tool_result_msg = PersistedMessage::tool_result(
                tool_call_id.clone(),
                "exec",
                Some(serde_json::json!({ "command": command })),
                false,
                None,
                Some(error_text.clone()),
            );
            if let Err(e) = session_store
                .append(session_key, &tool_result_msg.to_value())
                .await
            {
                warn!("failed to persist direct /sh tool error: {e}");
            }

            broadcast(
                state,
                "chat",
                serde_json::json!({
                    "runId": run_id,
                    "sessionKey": session_key,
                    "state": "tool_call_end",
                    "toolCallId": tool_call_id,
                    "toolName": "exec",
                    "success": false,
                    "error": parsed_error,
                    "seq": client_seq,
                }),
                BroadcastOpts::default(),
            )
            .await;

            if has_channel_targets {
                final_text = error_text;
            }
        },
    }

    if !final_text.trim().is_empty() {
        let streamed_target_keys = HashSet::new();
        deliver_channel_replies(
            state,
            session_key,
            &final_text,
            ReplyMedium::Text,
            &streamed_target_keys,
        )
        .await;
    }

    let final_payload = ChatFinalBroadcast {
        run_id: run_id.to_string(),
        session_key: session_key.to_string(),
        state: "final",
        text: final_text.clone(),
        model: String::new(),
        provider: String::new(),
        input_tokens: 0,
        output_tokens: 0,
        duration_ms: started.elapsed().as_millis() as u64,
        request_input_tokens: Some(0),
        request_output_tokens: Some(0),
        message_index: user_message_index + 2, // +1 for user msg, +1 for tool result
        reply_medium: ReplyMedium::Text,
        iterations: Some(1),
        tool_calls_made: Some(1),
        audio: None,
        audio_warning: None,
        reasoning: None,
        seq: client_seq,
    };
    #[allow(clippy::unwrap_used)] // serializing known-valid struct
    let payload = serde_json::to_value(&final_payload).unwrap();
    broadcast(state, "chat", payload, BroadcastOpts::default()).await;

    AssistantTurnOutput {
        text: final_text,
        input_tokens: 0,
        output_tokens: 0,
        duration_ms: started.elapsed().as_millis() as u64,
        request_input_tokens: 0,
        request_output_tokens: 0,
        audio_path: None,
        reasoning: None,
        llm_api_response: None,
    }
}

const MAX_AGENT_MEMORY_WRITE_BYTES: usize = 50 * 1024;
const MEMORY_SEARCH_FETCH_MULTIPLIER: usize = 8;
const MEMORY_SEARCH_MIN_FETCH: usize = 25;

fn is_valid_agent_memory_leaf_name(name: &str) -> bool {
    if name.is_empty() || name.contains('/') || !name.ends_with(".md") {
        return false;
    }
    if name.chars().any(char::is_whitespace) {
        return false;
    }
    let stem = &name[..name.len() - 3];
    !(stem.is_empty() || stem.starts_with('.'))
}

fn resolve_agent_memory_target_path(agent_id: &str, file: &str) -> anyhow::Result<PathBuf> {
    let trimmed = file.trim();
    if trimmed.is_empty() {
        anyhow::bail!("memory path cannot be empty");
    }

    let workspace = moltis_config::agent_workspace_dir(agent_id);
    if trimmed == "MEMORY.md" || trimmed == "memory.md" {
        return Ok(workspace.join("MEMORY.md"));
    }

    let Some(name) = trimmed.strip_prefix("memory/") else {
        anyhow::bail!(
            "invalid memory path '{trimmed}': allowed targets are MEMORY.md, memory.md, or memory/<name>.md"
        );
    };
    if !is_valid_agent_memory_leaf_name(name) {
        anyhow::bail!(
            "invalid memory path '{trimmed}': allowed targets are MEMORY.md, memory.md, or memory/<name>.md"
        );
    }
    Ok(workspace.join("memory").join(name))
}

fn is_path_in_agent_memory_scope(path: &Path, agent_id: &str) -> bool {
    let workspace = moltis_config::agent_workspace_dir(agent_id);
    let workspace_memory_dir = workspace.join("memory");
    if path == workspace.join("MEMORY.md")
        || path == workspace.join("memory.md")
        || path.starts_with(&workspace_memory_dir)
    {
        return true;
    }

    if agent_id != "main" {
        return false;
    }

    let data_dir = moltis_config::data_dir();
    let root_memory_dir = data_dir.join("memory");
    path == data_dir.join("MEMORY.md")
        || path == data_dir.join("memory.md")
        || path.starts_with(&root_memory_dir)
}

struct AgentScopedMemoryWriter {
    manager: Arc<moltis_memory::manager::MemoryManager>,
    agent_id: String,
}

impl AgentScopedMemoryWriter {
    fn new(manager: Arc<moltis_memory::manager::MemoryManager>, agent_id: String) -> Self {
        Self { manager, agent_id }
    }
}

#[async_trait]
impl moltis_agents::memory_writer::MemoryWriter for AgentScopedMemoryWriter {
    async fn write_memory(
        &self,
        file: &str,
        content: &str,
        append: bool,
    ) -> anyhow::Result<moltis_agents::memory_writer::MemoryWriteResult> {
        if content.len() > MAX_AGENT_MEMORY_WRITE_BYTES {
            anyhow::bail!(
                "content exceeds maximum size of {} bytes ({} bytes provided)",
                MAX_AGENT_MEMORY_WRITE_BYTES,
                content.len()
            );
        }

        let path = resolve_agent_memory_target_path(&self.agent_id, file)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let final_content = if append && tokio::fs::try_exists(&path).await? {
            let existing = tokio::fs::read_to_string(&path).await?;
            format!("{existing}\n\n{content}")
        } else {
            content.to_string()
        };
        let bytes_written = final_content.len();

        tokio::fs::write(&path, &final_content).await?;
        if let Err(error) = self.manager.sync_path(&path).await {
            warn!(path = %path.display(), %error, "agent memory write re-index failed");
        }

        Ok(moltis_agents::memory_writer::MemoryWriteResult {
            location: path.to_string_lossy().into_owned(),
            bytes_written,
        })
    }
}

struct AgentScopedMemorySearchTool {
    manager: Arc<moltis_memory::manager::MemoryManager>,
    agent_id: String,
}

impl AgentScopedMemorySearchTool {
    fn new(manager: Arc<moltis_memory::manager::MemoryManager>, agent_id: String) -> Self {
        Self { manager, agent_id }
    }
}

#[async_trait]
impl AgentTool for AgentScopedMemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> &str {
        "Search agent memory using hybrid vector + keyword search. Returns relevant chunks from daily logs and long-term memory files."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return",
                    "default": 5
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let query = params
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing 'query' parameter"))?;
        let requested_limit = params.get("limit").and_then(Value::as_u64).unwrap_or(5) as usize;
        let limit = requested_limit.clamp(1, 50);
        let search_limit = limit
            .saturating_mul(MEMORY_SEARCH_FETCH_MULTIPLIER)
            .max(MEMORY_SEARCH_MIN_FETCH)
            .max(limit);

        let mut results: Vec<moltis_memory::search::SearchResult> = self
            .manager
            .search(query, search_limit)
            .await?
            .into_iter()
            .filter(|result| is_path_in_agent_memory_scope(Path::new(&result.path), &self.agent_id))
            .collect();
        results.truncate(limit);

        let include_citations = moltis_memory::search::SearchResult::should_include_citations(
            &results,
            self.manager.citation_mode(),
        );
        let items: Vec<Value> = results
            .iter()
            .map(|result| {
                let text = if include_citations {
                    result.text_with_citation()
                } else {
                    result.text.clone()
                };
                serde_json::json!({
                    "chunk_id": result.chunk_id,
                    "path": result.path,
                    "source": result.source,
                    "start_line": result.start_line,
                    "end_line": result.end_line,
                    "score": result.score,
                    "text": text,
                    "citation": format!("{}#{}", result.path, result.start_line),
                })
            })
            .collect();

        Ok(serde_json::json!({
            "results": items,
            "citations_enabled": include_citations
        }))
    }
}

struct AgentScopedMemoryGetTool {
    manager: Arc<moltis_memory::manager::MemoryManager>,
    agent_id: String,
}

impl AgentScopedMemoryGetTool {
    fn new(manager: Arc<moltis_memory::manager::MemoryManager>, agent_id: String) -> Self {
        Self { manager, agent_id }
    }
}

#[async_trait]
impl AgentTool for AgentScopedMemoryGetTool {
    fn name(&self) -> &str {
        "memory_get"
    }

    fn description(&self) -> &str {
        "Retrieve a specific memory chunk by its ID. Use this to get the full text of a chunk found via memory_search."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "chunk_id": {
                    "type": "string",
                    "description": "The chunk ID to retrieve"
                }
            },
            "required": ["chunk_id"]
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let chunk_id = params
            .get("chunk_id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing 'chunk_id' parameter"))?;

        match self.manager.get_chunk(chunk_id).await? {
            Some(chunk)
                if is_path_in_agent_memory_scope(Path::new(&chunk.path), &self.agent_id) =>
            {
                Ok(serde_json::json!({
                    "chunk_id": chunk.id,
                    "path": chunk.path,
                    "source": chunk.source,
                    "start_line": chunk.start_line,
                    "end_line": chunk.end_line,
                    "text": chunk.text,
                }))
            },
            _ => Ok(serde_json::json!({
                "error": "chunk not found",
                "chunk_id": chunk_id,
            })),
        }
    }
}

struct AgentScopedMemorySaveTool {
    writer: AgentScopedMemoryWriter,
}

impl AgentScopedMemorySaveTool {
    fn new(manager: Arc<moltis_memory::manager::MemoryManager>, agent_id: String) -> Self {
        Self {
            writer: AgentScopedMemoryWriter::new(manager, agent_id),
        }
    }
}

#[async_trait]
impl AgentTool for AgentScopedMemorySaveTool {
    fn name(&self) -> &str {
        "memory_save"
    }

    fn description(&self) -> &str {
        "Save content to long-term memory. Writes to MEMORY.md or memory/<name>.md. Content persists across sessions and is searchable via memory_search."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The content to save to memory"
                },
                "file": {
                    "type": "string",
                    "description": "Target file: MEMORY.md, memory.md, or memory/<name>.md",
                    "default": "MEMORY.md"
                },
                "append": {
                    "type": "boolean",
                    "description": "Append to existing file (true) or overwrite (false)",
                    "default": true
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let content = params
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing 'content' parameter"))?;
        let file = params
            .get("file")
            .and_then(Value::as_str)
            .unwrap_or("MEMORY.md");
        let append = params
            .get("append")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        use moltis_agents::memory_writer::MemoryWriter;
        let result = self.writer.write_memory(file, content, append).await?;

        Ok(serde_json::json!({
            "saved": true,
            "path": file,
            "bytes_written": result.bytes_written,
        }))
    }
}

fn install_agent_scoped_memory_tools(
    registry: &mut ToolRegistry,
    manager: &Arc<moltis_memory::manager::MemoryManager>,
    agent_id: &str,
) {
    let had_search = registry.unregister("memory_search");
    let had_get = registry.unregister("memory_get");
    let had_save = registry.unregister("memory_save");

    let agent_id_owned = agent_id.to_string();
    if had_search {
        registry.register(Box::new(AgentScopedMemorySearchTool::new(
            Arc::clone(manager),
            agent_id_owned.clone(),
        )));
    }
    if had_get {
        registry.register(Box::new(AgentScopedMemoryGetTool::new(
            Arc::clone(manager),
            agent_id_owned.clone(),
        )));
    }
    if had_save {
        registry.register(Box::new(AgentScopedMemorySaveTool::new(
            Arc::clone(manager),
            agent_id_owned,
        )));
    }
}

/// Resolve the effective tool mode for a provider.
///
/// Combines the provider's `tool_mode()` override with its `supports_tools()`
/// capability to determine how tools should be dispatched:
/// - `Native` — provider handles tool schemas via API (OpenAI function calling, etc.)
/// - `Text` — tools are described in the prompt; the runner parses tool calls from text
/// - `Off` — no tools at all
fn effective_tool_mode(provider: &dyn moltis_agents::model::LlmProvider) -> ToolMode {
    match provider.tool_mode() {
        Some(ToolMode::Native) => ToolMode::Native,
        Some(ToolMode::Text) => ToolMode::Text,
        Some(ToolMode::Off) => ToolMode::Off,
        Some(ToolMode::Auto) | None => {
            if provider.supports_tools() {
                ToolMode::Native
            } else {
                ToolMode::Text
            }
        },
    }
}

async fn run_with_tools(
    state: &Arc<dyn ChatRuntime>,
    model_store: &Arc<RwLock<DisabledModelsStore>>,
    run_id: &str,
    provider: Arc<dyn moltis_agents::model::LlmProvider>,
    model_id: &str,
    tool_registry: &Arc<RwLock<ToolRegistry>>,
    user_content: &UserContent,
    provider_name: &str,
    history_raw: &[Value],
    session_key: &str,
    agent_id: &str,
    desired_reply_medium: ReplyMedium,
    project_context: Option<&str>,
    runtime_context: Option<&PromptRuntimeContext>,
    user_message_index: usize,
    skills: &[moltis_skills::types::SkillMetadata],
    hook_registry: Option<Arc<moltis_common::hooks::HookRegistry>>,
    accept_language: Option<String>,
    conn_id: Option<String>,
    session_store: Option<&Arc<SessionStore>>,
    mcp_disabled: bool,
    client_seq: Option<u64>,
    active_thinking_text: Option<Arc<RwLock<HashMap<String, String>>>>,
    active_tool_calls: Option<Arc<RwLock<HashMap<String, Vec<ActiveToolCall>>>>>,
) -> Option<AssistantTurnOutput> {
    let run_started = Instant::now();
    let persona = load_prompt_persona_for_agent(agent_id);

    let tool_mode = effective_tool_mode(&*provider);
    let native_tools = matches!(tool_mode, ToolMode::Native);
    let tools_enabled = !matches!(tool_mode, ToolMode::Off);

    let mut filtered_registry = {
        let registry_guard = tool_registry.read().await;
        if tools_enabled {
            apply_runtime_tool_filters(&registry_guard, &persona.config, skills, mcp_disabled)
        } else {
            registry_guard.clone_without(&[])
        }
    };
    if tools_enabled && let Some(manager) = state.memory_manager() {
        install_agent_scoped_memory_tools(&mut filtered_registry, manager, agent_id);
    }

    // Build system prompt:
    // - Native tools: full prompt with tool schemas sent via API
    // - Text tools: full prompt with tool schemas embedded + call guidance
    // - Off: minimal prompt without tools
    let system_prompt = if tools_enabled {
        build_system_prompt_with_session_runtime(
            &filtered_registry,
            native_tools,
            project_context,
            skills,
            Some(&persona.identity),
            Some(&persona.user),
            persona.soul_text.as_deref(),
            persona.agents_text.as_deref(),
            persona.tools_text.as_deref(),
            runtime_context,
            persona.memory_text.as_deref(),
        )
    } else {
        build_system_prompt_minimal_runtime(
            project_context,
            Some(&persona.identity),
            Some(&persona.user),
            persona.soul_text.as_deref(),
            persona.agents_text.as_deref(),
            persona.tools_text.as_deref(),
            runtime_context,
            persona.memory_text.as_deref(),
        )
    };

    // Layer 1: instruct the LLM to write speech-friendly output when voice is active.
    // Keep the runtime datetime/date sentence as the final prompt line for better cache locality.
    let system_prompt =
        apply_voice_reply_suffix(system_prompt, desired_reply_medium, runtime_context);

    // Determine sandbox mode for this session.
    let session_is_sandboxed = if let Some(router) = state.sandbox_router() {
        router.is_sandboxed(session_key).await
    } else {
        false
    };

    // Broadcast tool events to the UI in the order emitted by the runner.
    let state_for_events = Arc::clone(state);
    let run_id_for_events = run_id.to_string();
    let session_key_for_events = session_key.to_string();
    let session_store_for_events = session_store.map(Arc::clone);
    let provider_name_for_events = provider_name.to_string();
    let (on_event, mut event_rx) = ordered_runner_event_callback();
    let channel_stream_dispatcher = ChannelStreamDispatcher::for_session(state, session_key)
        .await
        .map(|dispatcher| Arc::new(Mutex::new(dispatcher)));
    let channel_stream_for_events = channel_stream_dispatcher.as_ref().map(Arc::clone);
    let event_forwarder = tokio::spawn(async move {
        // Track tool call arguments from ToolCallStart so they can be persisted in ToolCallEnd.
        let mut tool_args_map: HashMap<String, Value> = HashMap::new();
        // Track reasoning text that should be persisted with the first tool call after thinking.
        let mut tool_reasoning_map: HashMap<String, String> = HashMap::new();
        let mut latest_reasoning = String::new();
        while let Some(event) = event_rx.recv().await {
            let state = Arc::clone(&state_for_events);
            let run_id = run_id_for_events.clone();
            let sk = session_key_for_events.clone();
            let store = session_store_for_events.clone();
            let seq = client_seq;
            let payload = match event {
                RunnerEvent::Thinking => serde_json::json!({
                    "runId": run_id,
                    "sessionKey": sk,
                    "state": "thinking",
                    "seq": seq,
                }),
                RunnerEvent::ThinkingDone => serde_json::json!({
                    "runId": run_id,
                    "sessionKey": sk,
                    "state": "thinking_done",
                    "seq": seq,
                }),
                RunnerEvent::ToolCallStart {
                    id,
                    name,
                    arguments,
                } => {
                    tool_args_map.insert(id.clone(), arguments.clone());

                    // Track active tool call for chat.peek.
                    if let Some(ref map) = active_tool_calls {
                        map.write()
                            .await
                            .entry(sk.clone())
                            .or_default()
                            .push(ActiveToolCall {
                                id: id.clone(),
                                name: name.clone(),
                                arguments: arguments.clone(),
                                started_at: now_ms(),
                            });
                    }

                    // Attach reasoning to the first tool call after thinking.
                    if !latest_reasoning.is_empty() {
                        tool_reasoning_map
                            .insert(id.clone(), std::mem::take(&mut latest_reasoning));
                    }

                    // Send tool status to channels (Telegram, etc.)
                    let state_clone = Arc::clone(&state);
                    let sk_clone = sk.clone();
                    let name_clone = name.clone();
                    let args_clone = arguments.clone();
                    tokio::spawn(async move {
                        send_tool_status_to_channels(
                            &state_clone,
                            &sk_clone,
                            &name_clone,
                            &args_clone,
                        )
                        .await;
                    });

                    let is_browser = name == "browser";
                    let mut payload = serde_json::json!({
                        "runId": run_id,
                        "sessionKey": sk,
                        "state": "tool_call_start",
                        "toolCallId": id,
                        "toolName": name,
                        "arguments": arguments,
                        "seq": seq,
                    });
                    if is_browser {
                        payload["executionMode"] = serde_json::json!(if session_is_sandboxed {
                            "sandbox"
                        } else {
                            "host"
                        });
                    }
                    payload
                },
                RunnerEvent::ToolCallEnd {
                    id,
                    name,
                    success,
                    error,
                    result,
                } => {
                    // Remove from active tool calls tracking.
                    if let Some(ref map) = active_tool_calls {
                        let mut guard = map.write().await;
                        if let Some(calls) = guard.get_mut(&sk) {
                            calls.retain(|tc| tc.id != id);
                            if calls.is_empty() {
                                guard.remove(&sk);
                            }
                        }
                    }

                    let mut payload = serde_json::json!({
                        "runId": run_id,
                        "sessionKey": sk,
                        "state": "tool_call_end",
                        "toolCallId": id,
                        "toolName": name,
                        "success": success,
                        "seq": seq,
                    });
                    if let Some(ref err) = error {
                        payload["error"] = serde_json::json!(parse_chat_error(err, None));
                    }
                    // Check for screenshot/image to send to channel (Telegram, etc.)
                    let screenshot_to_send = result
                        .as_ref()
                        .and_then(|r| r.get("screenshot"))
                        .and_then(|s| s.as_str())
                        .filter(|s| s.starts_with("data:image/"))
                        .map(String::from);

                    let image_caption = result
                        .as_ref()
                        .and_then(|r| r.get("caption"))
                        .and_then(|c| c.as_str())
                        .map(String::from);

                    // Extract location from show_map results for native pin
                    let location_to_send = if name == "show_map" {
                        result.as_ref().and_then(|r| {
                            let lat = r.get("latitude")?.as_f64()?;
                            let lon = r.get("longitude")?.as_f64()?;
                            let label = r.get("label").and_then(|l| l.as_str()).map(String::from);
                            Some((lat, lon, label))
                        })
                    } else {
                        None
                    };

                    if let Some(ref res) = result {
                        // Cap output sent to the UI to avoid huge WS frames.
                        let mut capped = res.clone();
                        for field in &["stdout", "stderr"] {
                            if let Some(s) = capped.get(*field).and_then(|v| v.as_str())
                                && s.len() > 10_000
                            {
                                let truncated = format!(
                                    "{}\n\n... [truncated — {} bytes total]",
                                    truncate_at_char_boundary(s, 10_000),
                                    s.len()
                                );
                                capped[*field] = Value::String(truncated);
                            }
                        }
                        payload["result"] = capped;
                    }

                    // Send native location pin to channels before the screenshot.
                    if let Some((lat, lon, label)) = location_to_send {
                        let state_clone = Arc::clone(&state);
                        let sk_clone = sk.clone();
                        tokio::spawn(async move {
                            send_location_to_channels(
                                &state_clone,
                                &sk_clone,
                                lat,
                                lon,
                                label.as_deref(),
                            )
                            .await;
                        });
                    }

                    // Send screenshot/image to channel targets (Telegram) if present.
                    if let Some(screenshot_data) = screenshot_to_send {
                        let state_clone = Arc::clone(&state);
                        let sk_clone = sk.clone();
                        tokio::spawn(async move {
                            send_screenshot_to_channels(
                                &state_clone,
                                &sk_clone,
                                &screenshot_data,
                                image_caption.as_deref(),
                            )
                            .await;
                        });
                    }

                    // Buffer tool error result for the channel logbook.
                    if !success {
                        send_tool_result_to_channels(&state, &sk, &name, success, &error, &result)
                            .await;
                    }

                    // Persist tool result to the session JSONL file.
                    if let Some(ref store) = store {
                        let tracked_args = tool_args_map.remove(&id);
                        // Save screenshot to media dir (if present) and replace
                        // with a lightweight path reference. Strip screenshot_scale
                        // (only needed for live rendering). Cap stdout/stderr at
                        // 10 KB, matching the WS broadcast cap.
                        let store_media = Arc::clone(store);
                        let sk_media = sk.clone();
                        let tool_call_id = id.clone();
                        let persisted_result = result.as_ref().map(|res| {
                            let mut r = res.clone();
                            // Try to decode and persist the screenshot to the media
                            // directory. Extract base64 into an owned Vec first to
                            // release the borrow on `r`.
                            let decoded_screenshot = r
                                .get("screenshot")
                                .and_then(|v| v.as_str())
                                .filter(|s| s.starts_with("data:image/"))
                                .and_then(|uri| uri.split(',').nth(1))
                                .and_then(|b64| {
                                    use base64::Engine;
                                    base64::engine::general_purpose::STANDARD.decode(b64).ok()
                                });
                            if let Some(bytes) = decoded_screenshot {
                                let filename = format!("{tool_call_id}.png");
                                let store_ref = Arc::clone(&store_media);
                                let sk_ref = sk_media.clone();
                                tokio::spawn(async move {
                                    if let Err(e) =
                                        store_ref.save_media(&sk_ref, &filename, &bytes).await
                                    {
                                        warn!("failed to save screenshot media: {e}");
                                    }
                                });
                                let sanitized = SessionStore::key_to_filename(&sk_media);
                                r["screenshot"] =
                                    Value::String(format!("media/{sanitized}/{tool_call_id}.png"));
                            }
                            // If screenshot is still a data URI (decode failed), strip it.
                            let strip_screenshot = r
                                .get("screenshot")
                                .and_then(|v| v.as_str())
                                .is_some_and(|s| s.starts_with("data:"));
                            if let Some(obj) = r.as_object_mut() {
                                if strip_screenshot {
                                    obj.remove("screenshot");
                                }
                                obj.remove("screenshot_scale");
                            }
                            for field in &["stdout", "stderr"] {
                                if let Some(s) = r.get(*field).and_then(|v| v.as_str())
                                    && s.len() > 10_000
                                {
                                    let truncated = format!(
                                        "{}\n\n... [truncated — {} bytes total]",
                                        truncate_at_char_boundary(s, 10_000),
                                        s.len()
                                    );
                                    r[*field] = Value::String(truncated);
                                }
                            }
                            r
                        });
                        let tracked_reasoning = tool_reasoning_map.remove(&id);
                        let tool_result_msg = PersistedMessage::tool_result_with_reasoning(
                            id,
                            name,
                            tracked_args,
                            success,
                            persisted_result,
                            error,
                            tracked_reasoning,
                        );
                        let store_clone = Arc::clone(store);
                        let sk_persist = sk.clone();
                        tokio::spawn(async move {
                            if let Err(e) = store_clone
                                .append(&sk_persist, &tool_result_msg.to_value())
                                .await
                            {
                                warn!("failed to persist tool result: {e}");
                            }
                        });
                    }

                    payload
                },
                RunnerEvent::ThinkingText(text) => {
                    latest_reasoning = text.clone();
                    if let Some(ref map) = active_thinking_text {
                        map.write().await.insert(sk.clone(), text.clone());
                    }
                    serde_json::json!({
                        "runId": run_id,
                        "sessionKey": sk,
                        "state": "thinking_text",
                        "text": text,
                        "seq": seq,
                    })
                },
                RunnerEvent::TextDelta(text) => {
                    if let Some(ref dispatcher) = channel_stream_for_events {
                        dispatcher.lock().await.send_delta(&text).await;
                    }
                    serde_json::json!({
                        "runId": run_id,
                        "sessionKey": sk,
                        "state": "delta",
                        "text": text,
                        "seq": seq,
                    })
                },
                RunnerEvent::Iteration(n) => serde_json::json!({
                    "runId": run_id,
                    "sessionKey": sk,
                    "state": "iteration",
                    "iteration": n,
                    "seq": seq,
                }),
                RunnerEvent::SubAgentStart { task, model, depth } => serde_json::json!({
                    "runId": run_id,
                    "sessionKey": sk,
                    "state": "sub_agent_start",
                    "task": task,
                    "model": model,
                    "depth": depth,
                    "seq": seq,
                }),
                RunnerEvent::SubAgentEnd {
                    task,
                    model,
                    depth,
                    iterations,
                    tool_calls_made,
                } => serde_json::json!({
                    "runId": run_id,
                    "sessionKey": sk,
                    "state": "sub_agent_end",
                    "task": task,
                    "model": model,
                    "depth": depth,
                    "iterations": iterations,
                    "toolCallsMade": tool_calls_made,
                    "seq": seq,
                }),
                RunnerEvent::RetryingAfterError { error, delay_ms } => {
                    let error_obj =
                        parse_chat_error(&error, Some(provider_name_for_events.as_str()));
                    if error_obj.get("type").and_then(|v| v.as_str()) == Some("rate_limit_exceeded")
                    {
                        let state_clone = Arc::clone(&state);
                        let sk_clone = sk.clone();
                        let error_clone = error_obj.clone();
                        tokio::spawn(async move {
                            send_retry_status_to_channels(
                                &state_clone,
                                &sk_clone,
                                &error_clone,
                                Duration::from_millis(delay_ms),
                            )
                            .await;
                        });
                    }
                    serde_json::json!({
                        "runId": run_id,
                        "sessionKey": sk,
                        "state": "retrying",
                        "error": error_obj,
                        "retryAfterMs": delay_ms,
                        "seq": seq,
                    })
                },
            };
            broadcast(&state, "chat", payload, BroadcastOpts::default()).await;
        }
        latest_reasoning
    });

    // Convert persisted JSON history to typed ChatMessages for the LLM provider.
    let chat_history = values_to_chat_messages(history_raw);
    let hist = if chat_history.is_empty() {
        None
    } else {
        Some(chat_history)
    };

    // Inject session key and accept-language into tool call params so tools can
    // resolve per-session state and forward the user's locale to web requests.
    let mut tool_context = serde_json::json!({
        "_session_key": session_key,
    });
    if let Some(lang) = accept_language.as_deref() {
        tool_context["_accept_language"] = serde_json::json!(lang);
    }
    if let Some(cid) = conn_id.as_deref() {
        tool_context["_conn_id"] = serde_json::json!(cid);
    }

    let provider_ref = provider.clone();
    let first_result = run_agent_loop_streaming(
        provider,
        &filtered_registry,
        &system_prompt,
        user_content,
        Some(&on_event),
        hist,
        Some(tool_context.clone()),
        hook_registry.clone(),
    )
    .await;

    // On context-window overflow, compact the session and retry once.
    let result = match first_result {
        Err(AgentRunError::ContextWindowExceeded(ref msg)) if session_store.is_some() => {
            let store = session_store?;
            info!(
                run_id,
                session = session_key,
                error = %msg,
                "context window exceeded — compacting and retrying"
            );

            broadcast(
                state,
                "chat",
                serde_json::json!({
                    "runId": run_id,
                    "sessionKey": session_key,
                    "state": "auto_compact",
                    "phase": "start",
                    "reason": "context_window_exceeded",
                }),
                BroadcastOpts::default(),
            )
            .await;

            // Inline compaction: summarize history, replace in store.
            match compact_session(store, session_key, &provider_ref).await {
                Ok(()) => {
                    broadcast(
                        state,
                        "chat",
                        serde_json::json!({
                            "runId": run_id,
                            "sessionKey": session_key,
                            "state": "auto_compact",
                            "phase": "done",
                            "reason": "context_window_exceeded",
                        }),
                        BroadcastOpts::default(),
                    )
                    .await;

                    // Reload compacted history and retry.
                    let compacted_history_raw = store.read(session_key).await.unwrap_or_default();
                    let compacted_chat = values_to_chat_messages(&compacted_history_raw);
                    let retry_hist = if compacted_chat.is_empty() {
                        None
                    } else {
                        Some(compacted_chat)
                    };

                    run_agent_loop_streaming(
                        provider_ref.clone(),
                        &filtered_registry,
                        &system_prompt,
                        user_content,
                        Some(&on_event),
                        retry_hist,
                        Some(tool_context),
                        hook_registry,
                    )
                    .await
                },
                Err(e) => {
                    warn!(run_id, error = %e, "retry compaction failed");
                    broadcast(
                        state,
                        "chat",
                        serde_json::json!({
                            "runId": run_id,
                            "sessionKey": session_key,
                            "state": "auto_compact",
                            "phase": "error",
                            "error": e.to_string(),
                        }),
                        BroadcastOpts::default(),
                    )
                    .await;
                    // Return the original error.
                    first_result
                },
            }
        },
        other => other,
    };

    // Ensure all runner events (including deltas) are broadcast in order before
    // emitting terminal final/error frames.
    drop(on_event);
    let reasoning_text = match event_forwarder.await {
        Ok(reasoning) => reasoning,
        Err(e) => {
            warn!(run_id, error = %e, "runner event forwarder task failed");
            String::new()
        },
    };
    let reasoning = {
        let trimmed = reasoning_text.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    };
    let streamed_target_keys = if let Some(ref dispatcher) = channel_stream_dispatcher {
        let mut dispatcher = dispatcher.lock().await;
        dispatcher.finish().await;
        dispatcher.completed_target_keys().await
    } else {
        HashSet::new()
    };

    match result {
        Ok(result) => {
            clear_unsupported_model(state, model_store, model_id).await;

            let iterations = result.iterations;
            let tool_calls_made = result.tool_calls_made;
            let usage = result.usage;
            let request_usage = result.request_usage;
            let llm_api_response = (!result.raw_llm_responses.is_empty())
                .then_some(Value::Array(result.raw_llm_responses));
            let display_text = result.text;
            let is_silent = display_text.trim().is_empty();

            info!(
                run_id,
                iterations,
                tool_calls = tool_calls_made,
                response = %display_text,
                silent = is_silent,
                "agent run complete"
            );
            // Tool results are persisted between the user message and the
            // assistant message, so the assistant index must account for them.
            let assistant_message_index = user_message_index + 1 + tool_calls_made;

            // Generate & persist TTS audio for voice-medium web UI replies.
            let mut audio_warning: Option<String> = None;
            let audio_path = if !is_silent && desired_reply_medium == ReplyMedium::Voice {
                match generate_tts_audio(state, session_key, &display_text).await {
                    Ok(bytes) => {
                        let filename = format!("{run_id}.ogg");
                        if let Some(store) = session_store {
                            match store.save_media(session_key, &filename, &bytes).await {
                                Ok(path) => Some(path),
                                Err(e) => {
                                    let warning =
                                        format!("TTS audio generated but failed to save: {e}");
                                    warn!(run_id, error = %warning, "failed to save TTS audio to media dir");
                                    audio_warning = Some(warning);
                                    None
                                },
                            }
                        } else {
                            audio_warning = Some(
                                "TTS audio generated but session media storage is unavailable"
                                    .to_string(),
                            );
                            None
                        }
                    },
                    Err(error) => {
                        let error = error.to_string();
                        warn!(run_id, error = %error, "voice reply generation skipped");
                        audio_warning = Some(error);
                        None
                    },
                }
            } else {
                None
            };

            let final_payload = ChatFinalBroadcast {
                run_id: run_id.to_string(),
                session_key: session_key.to_string(),
                state: "final",
                text: display_text.clone(),
                model: provider_ref.id().to_string(),
                provider: provider_name.to_string(),
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                duration_ms: run_started.elapsed().as_millis() as u64,
                request_input_tokens: Some(request_usage.input_tokens),
                request_output_tokens: Some(request_usage.output_tokens),
                message_index: assistant_message_index,
                reply_medium: desired_reply_medium,
                iterations: Some(iterations),
                tool_calls_made: Some(tool_calls_made),
                audio: audio_path.clone(),
                audio_warning,
                reasoning: reasoning.clone(),
                seq: client_seq,
            };
            #[allow(clippy::unwrap_used)] // serializing known-valid struct
            let payload_val = serde_json::to_value(&final_payload).unwrap();
            broadcast(state, "chat", payload_val, BroadcastOpts::default()).await;

            if !is_silent {
                // Send push notification when chat response completes
                #[cfg(feature = "push-notifications")]
                {
                    tracing::info!("push: checking push notification (agent mode)");
                    send_chat_push_notification(state, session_key, &display_text).await;
                }
                deliver_channel_replies(
                    state,
                    session_key,
                    &display_text,
                    desired_reply_medium,
                    &streamed_target_keys,
                )
                .await;
            }
            Some(AssistantTurnOutput {
                text: display_text,
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                duration_ms: run_started.elapsed().as_millis() as u64,
                request_input_tokens: request_usage.input_tokens,
                request_output_tokens: request_usage.output_tokens,
                audio_path,
                reasoning,
                llm_api_response,
            })
        },
        Err(e) => {
            let error_str = e.to_string();
            warn!(run_id, error = %error_str, "agent run error");
            state.set_run_error(run_id, error_str.clone()).await;
            let error_obj = parse_chat_error(&error_str, Some(provider_name));
            mark_unsupported_model(state, model_store, model_id, provider_name, &error_obj).await;
            deliver_channel_error(state, session_key, &error_obj).await;
            let error_payload = ChatErrorBroadcast {
                run_id: run_id.to_string(),
                session_key: session_key.to_string(),
                state: "error",
                error: error_obj,
                seq: client_seq,
            };
            #[allow(clippy::unwrap_used)] // serializing known-valid struct
            let payload_val = serde_json::to_value(&error_payload).unwrap();
            broadcast(state, "chat", payload_val, BroadcastOpts::default()).await;
            None
        },
    }
}

/// Compact a session's history by summarizing it with the given provider.
///
/// This is a standalone helper so `run_with_tools` can call it without
/// requiring `&self` on `LiveChatService`.
async fn compact_session(
    store: &Arc<SessionStore>,
    session_key: &str,
    provider: &Arc<dyn moltis_agents::model::LlmProvider>,
) -> error::Result<()> {
    let history = store
        .read(session_key)
        .await
        .map_err(|source| error::Error::external("failed to read session history", source))?;
    if history.is_empty() {
        return Err(error::Error::message("nothing to compact"));
    }

    // Use structured ChatMessage objects so role boundaries are maintained via
    // the API's message structure, preventing prompt injection where user content
    // could mimic role prefixes in concatenated text.
    let mut summary_messages = vec![ChatMessage::system(
        "You are a conversation summarizer. The messages that follow are a conversation you must summarize. Preserve all key facts, decisions, and context. After the conversation, you will receive a final instruction.",
    )];
    summary_messages.extend(values_to_chat_messages(&history));
    summary_messages.push(ChatMessage::user(
        "Summarize the conversation above into a concise form. Output only the summary, no preamble.",
    ));

    let mut stream = provider.stream(summary_messages);
    let mut summary = String::new();
    while let Some(event) = stream.next().await {
        match event {
            StreamEvent::Delta(delta) => summary.push_str(&delta),
            StreamEvent::Done(_) => break,
            StreamEvent::Error(e) => {
                return Err(error::Error::message(format!(
                    "compact summarization failed: {e}"
                )));
            },
            // Tool events not expected in summarization stream.
            StreamEvent::ToolCallStart { .. }
            | StreamEvent::ToolCallArgumentsDelta { .. }
            | StreamEvent::ToolCallComplete { .. }
            // Provider raw payloads are debug metadata, not summary text.
            | StreamEvent::ProviderRaw(_)
            // Ignore provider reasoning blocks; summary body should only
            // include final answer text.
            | StreamEvent::ReasoningDelta(_) => {},
        }
    }

    if summary.is_empty() {
        return Err(error::Error::message("compact produced empty summary"));
    }

    let compacted_msg = PersistedMessage::Assistant {
        content: format!("[Conversation Summary]\n\n{summary}"),
        created_at: Some(now_ms()),
        model: None,
        provider: None,
        input_tokens: None,
        output_tokens: None,
        duration_ms: None,
        request_input_tokens: None,
        request_output_tokens: None,
        tool_calls: None,
        reasoning: None,
        llm_api_response: None,
        audio: None,
        seq: None,
        run_id: None,
    };
    let compacted = vec![compacted_msg.to_value()];

    store
        .replace_history(session_key, compacted)
        .await
        .map_err(|source| error::Error::external("failed to replace compacted history", source))?;

    Ok(())
}

// ── Streaming mode (no tools) ───────────────────────────────────────────────

const STREAM_RETRYABLE_SERVER_PATTERNS: &[&str] = &[
    "http 500",
    "http 502",
    "http 503",
    "http 504",
    "internal server error",
    "service unavailable",
    "gateway timeout",
    "temporarily unavailable",
    "overloaded",
    "timeout",
    "connection reset",
];
const STREAM_SERVER_RETRY_DELAY_MS: u64 = 2_000;
const STREAM_SERVER_MAX_RETRIES: u8 = 1;
const STREAM_RATE_LIMIT_INITIAL_RETRY_MS: u64 = 2_000;
const STREAM_RATE_LIMIT_MAX_RETRY_MS: u64 = 60_000;
const STREAM_RATE_LIMIT_MAX_RETRIES: u8 = 10;

fn is_retryable_stream_server_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    STREAM_RETRYABLE_SERVER_PATTERNS
        .iter()
        .any(|pattern| lower.contains(pattern))
}

fn next_stream_rate_limit_retry_ms(previous_ms: Option<u64>) -> u64 {
    previous_ms
        .map(|ms| ms.saturating_mul(2))
        .unwrap_or(STREAM_RATE_LIMIT_INITIAL_RETRY_MS)
        .clamp(
            STREAM_RATE_LIMIT_INITIAL_RETRY_MS,
            STREAM_RATE_LIMIT_MAX_RETRY_MS,
        )
}

fn next_stream_retry_delay_ms(
    raw_error: &str,
    error_obj: &Value,
    server_retries_remaining: &mut u8,
    rate_limit_retries_remaining: &mut u8,
    rate_limit_backoff_ms: &mut Option<u64>,
) -> Option<u64> {
    if error_obj.get("type").and_then(Value::as_str) == Some("rate_limit_exceeded") {
        if *rate_limit_retries_remaining == 0 {
            return None;
        }
        *rate_limit_retries_remaining -= 1;

        let current_backoff = *rate_limit_backoff_ms;
        *rate_limit_backoff_ms = Some(next_stream_rate_limit_retry_ms(current_backoff));

        let hinted_ms = error_obj.get("retryAfterMs").and_then(Value::as_u64);
        let delay_ms = hinted_ms
            .or(*rate_limit_backoff_ms)
            .unwrap_or(STREAM_RATE_LIMIT_INITIAL_RETRY_MS);
        return Some(delay_ms.clamp(1, STREAM_RATE_LIMIT_MAX_RETRY_MS));
    }

    if is_retryable_stream_server_error(raw_error) {
        if *server_retries_remaining == 0 {
            return None;
        }
        *server_retries_remaining -= 1;
        return Some(STREAM_SERVER_RETRY_DELAY_MS);
    }

    None
}

async fn run_streaming(
    state: &Arc<dyn ChatRuntime>,
    model_store: &Arc<RwLock<DisabledModelsStore>>,
    run_id: &str,
    provider: Arc<dyn moltis_agents::model::LlmProvider>,
    model_id: &str,
    user_content: &UserContent,
    provider_name: &str,
    history_raw: &[Value],
    session_key: &str,
    agent_id: &str,
    desired_reply_medium: ReplyMedium,
    project_context: Option<&str>,
    user_message_index: usize,
    _skills: &[moltis_skills::types::SkillMetadata],
    runtime_context: Option<&PromptRuntimeContext>,
    session_store: Option<&Arc<SessionStore>>,
    client_seq: Option<u64>,
) -> Option<AssistantTurnOutput> {
    let run_started = Instant::now();
    let persona = load_prompt_persona_for_agent(agent_id);

    let system_prompt = build_system_prompt_minimal_runtime(
        project_context,
        Some(&persona.identity),
        Some(&persona.user),
        persona.soul_text.as_deref(),
        persona.agents_text.as_deref(),
        persona.tools_text.as_deref(),
        runtime_context,
        persona.memory_text.as_deref(),
    );

    // Layer 1: instruct the LLM to write speech-friendly output when voice is active.
    // Keep the runtime datetime/date sentence as the final prompt line for better cache locality.
    let system_prompt =
        apply_voice_reply_suffix(system_prompt, desired_reply_medium, runtime_context);

    let mut messages: Vec<ChatMessage> = Vec::new();
    messages.push(ChatMessage::system(system_prompt));
    // Convert persisted JSON history to typed ChatMessages for the LLM provider.
    messages.extend(values_to_chat_messages(history_raw));
    messages.push(ChatMessage::User {
        content: user_content.clone(),
    });

    let mut server_retries_remaining: u8 = STREAM_SERVER_MAX_RETRIES;
    let mut rate_limit_retries_remaining: u8 = STREAM_RATE_LIMIT_MAX_RETRIES;
    let mut rate_limit_backoff_ms: Option<u64> = None;
    let mut channel_stream_dispatcher =
        ChannelStreamDispatcher::for_session(state, session_key).await;

    'attempts: loop {
        #[cfg(feature = "metrics")]
        let stream_start = Instant::now();

        let mut stream = provider.stream(messages.clone());
        let mut accumulated = String::new();
        let mut accumulated_reasoning = String::new();
        let mut raw_llm_responses: Vec<Value> = Vec::new();

        while let Some(event) = stream.next().await {
            match event {
                StreamEvent::Delta(delta) => {
                    accumulated.push_str(&delta);
                    if let Some(dispatcher) = channel_stream_dispatcher.as_mut() {
                        dispatcher.send_delta(&delta).await;
                    }
                    broadcast(
                        state,
                        "chat",
                        serde_json::json!({
                            "runId": run_id,
                            "sessionKey": session_key,
                            "state": "delta",
                            "text": delta,
                        }),
                        BroadcastOpts::default(),
                    )
                    .await;
                },
                StreamEvent::ReasoningDelta(delta) => {
                    accumulated_reasoning.push_str(&delta);
                    broadcast(
                        state,
                        "chat",
                        serde_json::json!({
                            "runId": run_id,
                            "sessionKey": session_key,
                            "state": "thinking_text",
                            "text": accumulated_reasoning.clone(),
                        }),
                        BroadcastOpts::default(),
                    )
                    .await;
                },
                StreamEvent::ProviderRaw(raw) => {
                    if raw_llm_responses.len() < 256 {
                        raw_llm_responses.push(raw);
                    }
                },
                StreamEvent::Done(usage) => {
                    clear_unsupported_model(state, model_store, model_id).await;

                    // Record streaming completion metrics (mirroring provider_chain.rs)
                    #[cfg(feature = "metrics")]
                    {
                        let duration = stream_start.elapsed().as_secs_f64();
                        counter!(
                            llm_metrics::COMPLETIONS_TOTAL,
                            labels::PROVIDER => provider_name.to_string(),
                            labels::MODEL => model_id.to_string()
                        )
                        .increment(1);
                        counter!(
                            llm_metrics::INPUT_TOKENS_TOTAL,
                            labels::PROVIDER => provider_name.to_string(),
                            labels::MODEL => model_id.to_string()
                        )
                        .increment(u64::from(usage.input_tokens));
                        counter!(
                            llm_metrics::OUTPUT_TOKENS_TOTAL,
                            labels::PROVIDER => provider_name.to_string(),
                            labels::MODEL => model_id.to_string()
                        )
                        .increment(u64::from(usage.output_tokens));
                        counter!(
                            llm_metrics::CACHE_READ_TOKENS_TOTAL,
                            labels::PROVIDER => provider_name.to_string(),
                            labels::MODEL => model_id.to_string()
                        )
                        .increment(u64::from(usage.cache_read_tokens));
                        counter!(
                            llm_metrics::CACHE_WRITE_TOKENS_TOTAL,
                            labels::PROVIDER => provider_name.to_string(),
                            labels::MODEL => model_id.to_string()
                        )
                        .increment(u64::from(usage.cache_write_tokens));
                        histogram!(
                            llm_metrics::COMPLETION_DURATION_SECONDS,
                            labels::PROVIDER => provider_name.to_string(),
                            labels::MODEL => model_id.to_string()
                        )
                        .record(duration);
                    }

                    let is_silent = accumulated.trim().is_empty();
                    let reasoning = {
                        let trimmed = accumulated_reasoning.trim();
                        (!trimmed.is_empty()).then(|| trimmed.to_string())
                    };
                    let streamed_target_keys =
                        if let Some(dispatcher) = channel_stream_dispatcher.as_mut() {
                            dispatcher.finish().await;
                            dispatcher.completed_target_keys().await
                        } else {
                            HashSet::new()
                        };

                    info!(
                        run_id,
                        input_tokens = usage.input_tokens,
                        output_tokens = usage.output_tokens,
                        response = %accumulated,
                        silent = is_silent,
                        "chat stream done"
                    );
                    let assistant_message_index = user_message_index + 1;

                    // Generate & persist TTS audio for voice-medium web UI replies.
                    let mut audio_warning: Option<String> = None;
                    let audio_path = if !is_silent && desired_reply_medium == ReplyMedium::Voice {
                        match generate_tts_audio(state, session_key, &accumulated).await {
                            Ok(bytes) => {
                                let filename = format!("{run_id}.ogg");
                                if let Some(store) = session_store {
                                    match store.save_media(session_key, &filename, &bytes).await {
                                        Ok(path) => Some(path),
                                        Err(e) => {
                                            let warning = format!(
                                                "TTS audio generated but failed to save: {e}"
                                            );
                                            warn!(run_id, error = %warning, "failed to save TTS audio to media dir");
                                            audio_warning = Some(warning);
                                            None
                                        },
                                    }
                                } else {
                                    audio_warning = Some(
                                        "TTS audio generated but session media storage is unavailable"
                                            .to_string(),
                                    );
                                    None
                                }
                            },
                            Err(error) => {
                                let error = error.to_string();
                                warn!(run_id, error = %error, "voice reply generation skipped");
                                audio_warning = Some(error);
                                None
                            },
                        }
                    } else {
                        None
                    };

                    let final_payload = ChatFinalBroadcast {
                        run_id: run_id.to_string(),
                        session_key: session_key.to_string(),
                        state: "final",
                        text: accumulated.clone(),
                        model: provider.id().to_string(),
                        provider: provider_name.to_string(),
                        input_tokens: usage.input_tokens,
                        output_tokens: usage.output_tokens,
                        duration_ms: run_started.elapsed().as_millis() as u64,
                        request_input_tokens: Some(usage.input_tokens),
                        request_output_tokens: Some(usage.output_tokens),
                        message_index: assistant_message_index,
                        reply_medium: desired_reply_medium,
                        iterations: None,
                        tool_calls_made: None,
                        audio: audio_path.clone(),
                        audio_warning,
                        reasoning: reasoning.clone(),
                        seq: client_seq,
                    };
                    #[allow(clippy::unwrap_used)] // serializing known-valid struct
                    let payload_val = serde_json::to_value(&final_payload).unwrap();
                    broadcast(state, "chat", payload_val, BroadcastOpts::default()).await;

                    if !is_silent {
                        // Send push notification when chat response completes
                        #[cfg(feature = "push-notifications")]
                        {
                            tracing::info!("push: checking push notification");
                            send_chat_push_notification(state, session_key, &accumulated).await;
                        }
                        deliver_channel_replies(
                            state,
                            session_key,
                            &accumulated,
                            desired_reply_medium,
                            &streamed_target_keys,
                        )
                        .await;
                    }
                    let llm_api_response =
                        (!raw_llm_responses.is_empty()).then_some(Value::Array(raw_llm_responses));
                    return Some(AssistantTurnOutput {
                        text: accumulated,
                        input_tokens: usage.input_tokens,
                        output_tokens: usage.output_tokens,
                        duration_ms: run_started.elapsed().as_millis() as u64,
                        request_input_tokens: usage.input_tokens,
                        request_output_tokens: usage.output_tokens,
                        audio_path,
                        reasoning,
                        llm_api_response,
                    });
                },
                StreamEvent::Error(msg) => {
                    let error_obj = parse_chat_error(&msg, Some(provider_name));
                    let has_no_streamed_content = accumulated.trim().is_empty()
                        && accumulated_reasoning.trim().is_empty()
                        && raw_llm_responses.is_empty();
                    if has_no_streamed_content
                        && let Some(delay_ms) = next_stream_retry_delay_ms(
                            &msg,
                            &error_obj,
                            &mut server_retries_remaining,
                            &mut rate_limit_retries_remaining,
                            &mut rate_limit_backoff_ms,
                        )
                    {
                        warn!(
                            run_id,
                            error = %msg,
                            delay_ms,
                            server_retries_remaining,
                            rate_limit_retries_remaining,
                            "chat stream transient error, retrying after delay"
                        );
                        if error_obj.get("type").and_then(Value::as_str)
                            == Some("rate_limit_exceeded")
                        {
                            send_retry_status_to_channels(
                                state,
                                session_key,
                                &error_obj,
                                Duration::from_millis(delay_ms),
                            )
                            .await;
                        }
                        broadcast(
                            state,
                            "chat",
                            serde_json::json!({
                                "runId": run_id,
                                "sessionKey": session_key,
                                "state": "retrying",
                                "error": error_obj,
                                "retryAfterMs": delay_ms,
                                "seq": client_seq,
                            }),
                            BroadcastOpts::default(),
                        )
                        .await;
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                        continue 'attempts;
                    }

                    warn!(run_id, error = %msg, "chat stream error");
                    if let Some(dispatcher) = channel_stream_dispatcher.as_mut() {
                        dispatcher.finish().await;
                    }
                    state.set_run_error(run_id, msg.clone()).await;
                    mark_unsupported_model(state, model_store, model_id, provider_name, &error_obj)
                        .await;
                    deliver_channel_error(state, session_key, &error_obj).await;
                    let error_payload = ChatErrorBroadcast {
                        run_id: run_id.to_string(),
                        session_key: session_key.to_string(),
                        state: "error",
                        error: error_obj,
                        seq: client_seq,
                    };
                    #[allow(clippy::unwrap_used)] // serializing known-valid struct
                    let payload_val = serde_json::to_value(&error_payload).unwrap();
                    broadcast(state, "chat", payload_val, BroadcastOpts::default()).await;
                    return None;
                },
                // Tool events not expected in stream-only mode.
                StreamEvent::ToolCallStart { .. }
                | StreamEvent::ToolCallArgumentsDelta { .. }
                | StreamEvent::ToolCallComplete { .. } => {},
            }
        }

        // Stream ended unexpectedly without Done/Error.
        return None;
    }
}

/// Send a push notification when a chat response completes.
/// Only sends if push notifications are configured and there are subscribers.
#[cfg(feature = "push-notifications")]
async fn send_chat_push_notification(state: &Arc<dyn ChatRuntime>, session_key: &str, text: &str) {
    // Create a short summary of the response (first 100 chars)
    let summary = if text.len() > 100 {
        format!("{}…", truncate_at_char_boundary(text, 100))
    } else {
        text.to_string()
    };

    let title = "Message received";
    let url = format!("/chat/{session_key}");

    match state
        .send_push_notification(title, &summary, Some(&url), Some(session_key))
        .await
    {
        Ok(sent) => {
            tracing::info!(sent, "push notification sent");
        },
        Err(e) => {
            tracing::warn!("failed to send push notification: {e}");
        },
    }
}

/// Drain any pending channel reply targets for a session and send the
/// response text back to each originating channel via outbound.
/// Each delivery runs in its own spawned task so slow network calls
/// don't block each other or the chat pipeline.
async fn deliver_channel_replies(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    text: &str,
    desired_reply_medium: ReplyMedium,
    streamed_target_keys: &HashSet<ChannelReplyTargetKey>,
) {
    let drained_targets = state.drain_channel_replies(session_key).await;
    let mut targets = Vec::with_capacity(drained_targets.len());
    let mut streamed_targets = Vec::new();
    // When the reply medium is voice we must still deliver TTS audio even if
    // the text was already streamed — skip the stream dedupe entirely.
    if desired_reply_medium != ReplyMedium::Voice && !streamed_target_keys.is_empty() {
        for target in drained_targets {
            let key = ChannelReplyTargetKey::from(&target);
            if streamed_target_keys.contains(&key) {
                streamed_targets.push(target);
            } else {
                targets.push(target);
            }
        }
    } else {
        targets = drained_targets;
    }
    let is_channel_session = session_key.starts_with("telegram:")
        || session_key.starts_with("msteams:")
        || session_key.starts_with("discord:");
    if targets.is_empty() && streamed_targets.is_empty() {
        let _ = state.drain_channel_status_log(session_key).await;
        if is_channel_session {
            info!(
                session_key,
                text_len = text.len(),
                streamed_count = streamed_target_keys.len(),
                "channel reply delivery skipped: no pending targets after stream dedupe"
            );
        }
        return;
    }
    if text.is_empty() {
        let _ = state.drain_channel_status_log(session_key).await;
        if is_channel_session {
            info!(
                session_key,
                target_count = targets.len() + streamed_targets.len(),
                "channel reply delivery skipped: empty response text"
            );
        }
        return;
    }
    if is_channel_session {
        info!(
            session_key,
            target_count = targets.len(),
            text_len = text.len(),
            reply_medium = ?desired_reply_medium,
            "channel reply delivery starting"
        );
    }
    let outbound = match state.channel_outbound() {
        Some(o) => o,
        None => {
            if is_channel_session {
                info!(
                    session_key,
                    target_count = targets.len(),
                    "channel reply delivery skipped: outbound unavailable"
                );
            }
            return;
        },
    };
    // Drain buffered status log entries to build a logbook suffix.
    let status_log = state.drain_channel_status_log(session_key).await;
    let logbook_html = format_logbook_html(&status_log);
    if !streamed_targets.is_empty() && !logbook_html.is_empty() {
        send_channel_logbook_follow_up_to_targets(
            Arc::clone(&outbound),
            streamed_targets,
            &logbook_html,
        )
        .await;
    }
    if targets.is_empty() {
        if is_channel_session {
            info!(
                session_key,
                text_len = text.len(),
                streamed_count = streamed_target_keys.len(),
                "channel reply delivery completed via stream-only targets"
            );
        }
        return;
    }
    deliver_channel_replies_to_targets(
        outbound,
        targets,
        session_key,
        text,
        Arc::clone(state),
        desired_reply_medium,
        status_log,
        streamed_target_keys,
    )
    .await;
}

/// Format buffered status log entries into a Telegram expandable blockquote HTML.
/// Returns an empty string if there are no entries.
fn format_logbook_html(entries: &[String]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let mut html = String::from("<blockquote expandable>\n\u{1f4cb} <b>Activity log</b>\n");
    for entry in entries {
        // Escape HTML entities in the entry text.
        let escaped = entry
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        html.push_str(&format!("\u{2022} {escaped}\n"));
    }
    html.push_str("</blockquote>");
    html
}

async fn send_channel_logbook_follow_up_to_targets(
    outbound: Arc<dyn moltis_channels::plugin::ChannelOutbound>,
    targets: Vec<moltis_channels::ChannelReplyTarget>,
    logbook_html: &str,
) {
    if targets.is_empty() || logbook_html.is_empty() {
        return;
    }

    let html = logbook_html.to_string();
    let mut tasks = Vec::with_capacity(targets.len());
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let html = html.clone();
        tasks.push(tokio::spawn(async move {
            if let Err(e) = outbound
                .send_html(&target.account_id, &target.chat_id, &html, None)
                .await
            {
                warn!(
                    account_id = target.account_id,
                    chat_id = target.chat_id,
                    "failed to send logbook follow-up: {e}"
                );
            }
        }));
    }

    for task in tasks {
        if let Err(e) = task.await {
            warn!(error = %e, "channel logbook follow-up task join failed");
        }
    }
}

fn format_channel_retry_message(error_obj: &Value, retry_after: Duration) -> String {
    let retry_secs = ((retry_after.as_millis() as u64).saturating_add(999) / 1_000).max(1);
    if error_obj.get("type").and_then(|v| v.as_str()) == Some("rate_limit_exceeded") {
        format!("⏳ Provider rate limited. Retrying in {retry_secs}s.")
    } else {
        format!("⏳ Temporary provider issue. Retrying in {retry_secs}s.")
    }
}

fn format_channel_error_message(error_obj: &Value) -> String {
    let title = error_obj
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Request failed");
    let detail = error_obj
        .get("detail")
        .and_then(|v| v.as_str())
        .unwrap_or("Please try again.");
    format!("⚠️ {title}: {detail}")
}

/// Send a short retry status update to pending channel targets without draining
/// them. The final reply (or terminal error) will still use the same targets.
async fn send_retry_status_to_channels(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    error_obj: &Value,
    retry_after: Duration,
) {
    let targets = state.peek_channel_replies(session_key).await;
    if targets.is_empty() {
        return;
    }

    let outbound = match state.channel_outbound() {
        Some(o) => o,
        None => return,
    };

    let message = format_channel_retry_message(error_obj, retry_after);
    let mut tasks = Vec::with_capacity(targets.len());
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let message = message.clone();
        tasks.push(tokio::spawn(async move {
            let reply_to = target.message_id.as_deref();
            if let Err(e) = outbound
                .send_text_silent(&target.account_id, &target.chat_id, &message, reply_to)
                .await
            {
                warn!(
                    account_id = target.account_id,
                    chat_id = target.chat_id,
                    "failed to send retry status to channel: {e}"
                );
            }
        }));
    }

    for task in tasks {
        if let Err(e) = task.await {
            warn!(error = %e, "channel retry status task join failed");
        }
    }
}

/// Drain pending channel targets for a session and send a terminal error message.
async fn deliver_channel_error(state: &Arc<dyn ChatRuntime>, session_key: &str, error_obj: &Value) {
    let targets = state.drain_channel_replies(session_key).await;
    let status_log = state.drain_channel_status_log(session_key).await;
    if targets.is_empty() {
        return;
    }

    let outbound = match state.channel_outbound() {
        Some(o) => o,
        None => return,
    };

    let error_text = format_channel_error_message(error_obj);
    let logbook_html = format_logbook_html(&status_log);
    let mut tasks = Vec::with_capacity(targets.len());
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let error_text = error_text.clone();
        let logbook_html = logbook_html.clone();
        tasks.push(tokio::spawn(async move {
            let reply_to = target.message_id.as_deref();
            let send_result = if logbook_html.is_empty() {
                outbound
                    .send_text(&target.account_id, &target.chat_id, &error_text, reply_to)
                    .await
            } else {
                outbound
                    .send_text_with_suffix(
                        &target.account_id,
                        &target.chat_id,
                        &error_text,
                        &logbook_html,
                        reply_to,
                    )
                    .await
            };
            if let Err(e) = send_result {
                warn!(
                    account_id = target.account_id,
                    chat_id = target.chat_id,
                    "failed to send channel error reply: {e}"
                );
            }
        }));
    }

    for task in tasks {
        if let Err(e) = task.await {
            warn!(error = %e, "channel error task join failed");
        }
    }
}

async fn deliver_channel_replies_to_targets(
    outbound: Arc<dyn moltis_channels::plugin::ChannelOutbound>,
    targets: Vec<moltis_channels::ChannelReplyTarget>,
    session_key: &str,
    text: &str,
    state: Arc<dyn ChatRuntime>,
    desired_reply_medium: ReplyMedium,
    status_log: Vec<String>,
    streamed_target_keys: &HashSet<ChannelReplyTargetKey>,
) {
    let session_key = session_key.to_string();
    let text = text.to_string();
    let logbook_html = format_logbook_html(&status_log);
    let mut tasks = Vec::with_capacity(targets.len());
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let state = Arc::clone(&state);
        let session_key = session_key.clone();
        let text = text.clone();
        let logbook_html = logbook_html.clone();
        // Text was already delivered via edit-in-place streaming — skip text
        // caption/follow-up and only send the TTS voice audio.
        let text_already_streamed =
            streamed_target_keys.contains(&ChannelReplyTargetKey::from(&target));
        tasks.push(tokio::spawn(async move {
            let tts_payload = match desired_reply_medium {
                ReplyMedium::Voice => build_tts_payload(&state, &session_key, &target, &text).await,
                ReplyMedium::Text => None,
            };
            let reply_to = target.message_id.as_deref();
            match target.channel_type {
                moltis_channels::ChannelType::Telegram => match tts_payload {
                    Some(mut payload) => {
                        let transcript = std::mem::take(&mut payload.text);

                        if text_already_streamed {
                            // Text was already streamed — send voice audio only.
                            if let Err(e) = outbound
                                .send_media(&target.account_id, &target.chat_id, &payload, reply_to)
                                .await
                            {
                                warn!(
                                    account_id = target.account_id,
                                    chat_id = target.chat_id,
                                    "failed to send channel voice reply: {e}"
                                );
                            }
                            // Send logbook as a follow-up if present.
                            if !logbook_html.is_empty()
                                && let Err(e) = outbound
                                    .send_html(
                                        &target.account_id,
                                        &target.chat_id,
                                        &logbook_html,
                                        None,
                                    )
                                    .await
                            {
                                warn!(
                                    account_id = target.account_id,
                                    chat_id = target.chat_id,
                                    "failed to send logbook follow-up: {e}"
                                );
                            }
                        } else if transcript.len()
                            <= moltis_telegram::markdown::TELEGRAM_CAPTION_LIMIT
                        {
                            // Short transcript fits as a caption on the voice message.
                            payload.text = transcript;
                            if let Err(e) = outbound
                                .send_media(&target.account_id, &target.chat_id, &payload, reply_to)
                                .await
                            {
                                warn!(
                                    account_id = target.account_id,
                                    chat_id = target.chat_id,
                                    "failed to send channel voice reply: {e}"
                                );
                            }
                            // Send logbook as a follow-up if present.
                            if !logbook_html.is_empty()
                                && let Err(e) = outbound
                                    .send_html(
                                        &target.account_id,
                                        &target.chat_id,
                                        &logbook_html,
                                        None,
                                    )
                                    .await
                            {
                                warn!(
                                    account_id = target.account_id,
                                    chat_id = target.chat_id,
                                    "failed to send logbook follow-up: {e}"
                                );
                            }
                        } else {
                            // Transcript too long for a caption — send voice
                            // without caption, then the full text as a follow-up.
                            if let Err(e) = outbound
                                .send_media(&target.account_id, &target.chat_id, &payload, reply_to)
                                .await
                            {
                                warn!(
                                    account_id = target.account_id,
                                    chat_id = target.chat_id,
                                    "failed to send channel voice reply: {e}"
                                );
                            }
                            let text_result = if logbook_html.is_empty() {
                                outbound
                                    .send_text(
                                        &target.account_id,
                                        &target.chat_id,
                                        &transcript,
                                        None,
                                    )
                                    .await
                            } else {
                                outbound
                                    .send_text_with_suffix(
                                        &target.account_id,
                                        &target.chat_id,
                                        &transcript,
                                        &logbook_html,
                                        None,
                                    )
                                    .await
                            };
                            if let Err(e) = text_result {
                                warn!(
                                    account_id = target.account_id,
                                    chat_id = target.chat_id,
                                    "failed to send transcript follow-up: {e}"
                                );
                            }
                        }
                    },
                    None => {
                        let result = if logbook_html.is_empty() {
                            outbound
                                .send_text(&target.account_id, &target.chat_id, &text, reply_to)
                                .await
                        } else {
                            outbound
                                .send_text_with_suffix(
                                    &target.account_id,
                                    &target.chat_id,
                                    &text,
                                    &logbook_html,
                                    reply_to,
                                )
                                .await
                        };
                        if let Err(e) = result {
                            warn!(
                                account_id = target.account_id,
                                chat_id = target.chat_id,
                                "failed to send channel reply: {e}"
                            );
                        }
                    },
                },
                moltis_channels::ChannelType::MsTeams
                | moltis_channels::ChannelType::Discord
                | moltis_channels::ChannelType::Whatsapp => match tts_payload {
                    Some(payload) => {
                        if let Err(e) = outbound
                            .send_media(&target.account_id, &target.chat_id, &payload, reply_to)
                            .await
                        {
                            warn!(
                                account_id = target.account_id,
                                chat_id = target.chat_id,
                                "failed to send channel voice reply: {e}"
                            );
                        }
                    },
                    None => {
                        let result = if logbook_html.is_empty() {
                            outbound
                                .send_text(&target.account_id, &target.chat_id, &text, reply_to)
                                .await
                        } else {
                            outbound
                                .send_text_with_suffix(
                                    &target.account_id,
                                    &target.chat_id,
                                    &text,
                                    &logbook_html,
                                    reply_to,
                                )
                                .await
                        };
                        if let Err(e) = result {
                            warn!(
                                account_id = target.account_id,
                                chat_id = target.chat_id,
                                "failed to send channel reply: {e}"
                            );
                        }
                    },
                },
            }
        }));
    }

    for task in tasks {
        if let Err(e) = task.await {
            warn!(error = %e, "channel reply task join failed");
        }
    }
}

#[derive(Debug, Deserialize)]
struct TtsStatusResponse {
    enabled: bool,
}

#[derive(Debug, Serialize)]
struct TtsConvertRequest<'a> {
    text: &'a str,
    format: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "voiceId")]
    voice_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TtsConvertResponse {
    audio: String,
    #[serde(default)]
    mime_type: Option<String>,
}

/// Generate TTS audio bytes for a web UI response.
///
/// Uses the session-level TTS override if configured, otherwise the global TTS
/// config. Returns raw audio bytes (OGG format) on success, `None` if TTS is
/// disabled or generation fails.
async fn generate_tts_audio(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    text: &str,
) -> error::Result<Vec<u8>> {
    use base64::Engine;

    let tts_status = state
        .tts_service()
        .status()
        .await
        .map_err(error::Error::message)?;
    let status: TtsStatusResponse = serde_json::from_value(tts_status)
        .map_err(|_| error::Error::message("invalid tts.status response"))?;
    if !status.enabled {
        return Err(error::Error::message("TTS is disabled or not configured"));
    }

    // Layer 2: strip markdown/URLs the LLM may have included despite the prompt.
    let text = moltis_voice::tts::sanitize_text_for_tts(text);
    let text = text.trim();
    if text.is_empty() {
        return Err(error::Error::message("response has no speakable text"));
    }

    let (_, session_override) = state.tts_overrides(session_key, "").await;

    let request = TtsConvertRequest {
        text,
        format: "ogg",
        provider: session_override.as_ref().and_then(|o| o.provider.clone()),
        voice_id: session_override.as_ref().and_then(|o| o.voice_id.clone()),
        model: session_override.as_ref().and_then(|o| o.model.clone()),
    };

    let request_value = serde_json::to_value(request)
        .map_err(|_| error::Error::message("failed to build tts.convert request"))?;
    let tts_result = state
        .tts_service()
        .convert(request_value)
        .await
        .map_err(error::Error::message)?;

    let response: TtsConvertResponse = serde_json::from_value(tts_result)
        .map_err(|_| error::Error::message("invalid tts.convert response"))?;
    base64::engine::general_purpose::STANDARD
        .decode(&response.audio)
        .map_err(|_| error::Error::message("invalid base64 audio returned by TTS provider"))
}

async fn build_tts_payload(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    target: &moltis_channels::ChannelReplyTarget,
    text: &str,
) -> Option<moltis_common::types::ReplyPayload> {
    use moltis_common::types::{MediaAttachment, ReplyPayload};

    let tts_status = state.tts_service().status().await.ok()?;
    let status: TtsStatusResponse = serde_json::from_value(tts_status).ok()?;
    if !status.enabled {
        return None;
    }

    // Strip markdown/URLs the LLM may have included — use sanitized text
    // only for TTS conversion, but keep the original for the caption.
    let sanitized = moltis_voice::tts::sanitize_text_for_tts(text);

    let channel_key = format!("{}:{}", target.channel_type.as_str(), target.account_id);
    let (channel_override, session_override) = state.tts_overrides(session_key, &channel_key).await;
    let resolved = channel_override.or(session_override);

    let request = TtsConvertRequest {
        text: &sanitized,
        format: "ogg",
        provider: resolved.as_ref().and_then(|o| o.provider.clone()),
        voice_id: resolved.as_ref().and_then(|o| o.voice_id.clone()),
        model: resolved.as_ref().and_then(|o| o.model.clone()),
    };

    let tts_result = state
        .tts_service()
        .convert(serde_json::to_value(request).ok()?)
        .await
        .ok()?;

    let response: TtsConvertResponse = serde_json::from_value(tts_result).ok()?;

    let mime_type = response
        .mime_type
        .unwrap_or_else(|| "audio/ogg".to_string());

    Some(ReplyPayload {
        text: text.to_string(),
        media: Some(MediaAttachment {
            url: format!("data:{mime_type};base64,{}", response.audio),
            mime_type,
        }),
        reply_to_id: None,
        silent: false,
    })
}

/// Buffer a tool execution status into the channel status log for a session.
/// The buffered entries are appended as a collapsible logbook when the final
/// response is delivered, instead of being sent as separate messages.
async fn send_tool_status_to_channels(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    tool_name: &str,
    arguments: &Value,
) {
    let targets = state.peek_channel_replies(session_key).await;
    if targets.is_empty() {
        return;
    }

    // Buffer the status message for the logbook
    let message = format_tool_status_message(tool_name, arguments);
    state.push_channel_status_log(session_key, message).await;
}

/// Buffer a tool error result into the channel status log for a session.
/// Called from `ToolCallEnd` for failed tool calls only — success is implicit
/// and does not need a separate log entry.
async fn send_tool_result_to_channels(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    tool_name: &str,
    success: bool,
    error: &Option<String>,
    result: &Option<Value>,
) {
    if success {
        return;
    }
    let targets = state.peek_channel_replies(session_key).await;
    if targets.is_empty() {
        return;
    }

    let message = format_tool_result_message(tool_name, error, result);
    state.push_channel_status_log(session_key, message).await;
}

/// Format a human-readable error summary for a failed tool call.
fn format_tool_result_message(
    tool_name: &str,
    error: &Option<String>,
    result: &Option<Value>,
) -> String {
    let detail = match tool_name {
        "exec" => {
            let exit_code = result
                .as_ref()
                .and_then(|r| r.get("exitCode"))
                .and_then(|v| v.as_i64());
            let stderr = result
                .as_ref()
                .and_then(|r| r.get("stderr"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let first_line = stderr.lines().next().unwrap_or_default();
            let truncated = truncate_at_char_boundary(first_line, 120);
            match exit_code {
                Some(code) => {
                    if truncated.is_empty() {
                        format!("exit {code}")
                    } else {
                        format!("exit {code} — {truncated}")
                    }
                },
                None => {
                    if truncated.is_empty() {
                        error
                            .as_deref()
                            .map(|e| truncate_at_char_boundary(e, 120).to_string())
                            .unwrap_or_else(|| "failed".to_string())
                    } else {
                        truncated.to_string()
                    }
                },
            }
        },
        _ => {
            // Browser, web_fetch, web_search, and other tools: use error string.
            error
                .as_deref()
                .map(|e| {
                    let first_line = e.lines().next().unwrap_or_default();
                    truncate_at_char_boundary(first_line, 120).to_string()
                })
                .unwrap_or_else(|| "failed".to_string())
        },
    };
    format!("  ❌ {detail}")
}

/// Format a human-readable tool execution message.
fn format_tool_status_message(tool_name: &str, arguments: &Value) -> String {
    match tool_name {
        "browser" => {
            let action = arguments
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let url = arguments.get("url").and_then(|v| v.as_str());
            let ref_ = arguments.get("ref_").and_then(|v| v.as_u64());

            match action {
                "navigate" => {
                    if let Some(u) = url {
                        format!("🌐 Navigating to {}", truncate_url(u))
                    } else {
                        "🌐 Navigating...".to_string()
                    }
                },
                "screenshot" => "📸 Taking screenshot...".to_string(),
                "snapshot" => "📋 Getting page snapshot...".to_string(),
                "click" => {
                    if let Some(r) = ref_ {
                        format!("👆 Clicking element #{}", r)
                    } else {
                        "👆 Clicking...".to_string()
                    }
                },
                "type" => "⌨️ Typing...".to_string(),
                "scroll" => "📜 Scrolling...".to_string(),
                "evaluate" => "⚡ Running JavaScript...".to_string(),
                "wait" => "⏳ Waiting for element...".to_string(),
                "close" => "🚪 Closing browser...".to_string(),
                _ => format!("🌐 Browser: {}", action),
            }
        },
        "exec" => {
            let command = arguments.get("command").and_then(|v| v.as_str());
            if let Some(cmd) = command {
                // Show first ~50 chars of command
                let display_cmd = if cmd.len() > 50 {
                    format!("{}...", truncate_at_char_boundary(cmd, 50))
                } else {
                    cmd.to_string()
                };
                format!("💻 Running: `{}`", display_cmd)
            } else {
                "💻 Executing command...".to_string()
            }
        },
        "web_fetch" => {
            let url = arguments.get("url").and_then(|v| v.as_str());
            if let Some(u) = url {
                format!("🔗 Fetching {}", truncate_url(u))
            } else {
                "🔗 Fetching URL...".to_string()
            }
        },
        "web_search" => {
            let query = arguments.get("query").and_then(|v| v.as_str());
            if let Some(q) = query {
                let display_q = if q.len() > 40 {
                    format!("{}...", truncate_at_char_boundary(q, 40))
                } else {
                    q.to_string()
                };
                format!("🔍 Searching: {}", display_q)
            } else {
                "🔍 Searching...".to_string()
            }
        },
        "calc" => {
            let expr = arguments
                .get("expression")
                .or_else(|| arguments.get("expr"))
                .and_then(|v| v.as_str());
            if let Some(expression) = expr {
                let display = if expression.len() > 50 {
                    format!("{}...", truncate_at_char_boundary(expression, 50))
                } else {
                    expression.to_string()
                };
                format!("🧮 Calculating: {}", display)
            } else {
                "🧮 Calculating...".to_string()
            }
        },
        "memory_search" => "🧠 Searching memory...".to_string(),
        "memory_store" => "🧠 Storing to memory...".to_string(),
        _ => format!("🔧 {}", tool_name),
    }
}

/// Truncate a URL for display (show domain + short path).
fn truncate_url(url: &str) -> String {
    // Try to extract domain from URL
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);

    // Take first 50 chars max
    if without_scheme.len() > 50 {
        format!("{}...", truncate_at_char_boundary(without_scheme, 50))
    } else {
        without_scheme.to_string()
    }
}

/// Send a screenshot to all pending channel targets for a session.
/// Uses `peek_channel_replies` so targets remain for the final text response.
async fn send_screenshot_to_channels(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    screenshot_data: &str,
    caption: Option<&str>,
) {
    use moltis_common::types::{MediaAttachment, ReplyPayload};

    let targets = state.peek_channel_replies(session_key).await;
    if targets.is_empty() {
        return;
    }

    let outbound = match state.channel_outbound() {
        Some(o) => o,
        None => return,
    };

    // Extract actual MIME from "data:image/jpeg;base64,..." instead of
    // hardcoding PNG — supports JPEG, GIF, WebP from send_image tool.
    let mime_type = screenshot_data
        .strip_prefix("data:")
        .and_then(|s| s.split(';').next())
        .unwrap_or("image/png")
        .to_string();

    let payload = ReplyPayload {
        text: caption.unwrap_or_default().to_string(),
        media: Some(MediaAttachment {
            url: screenshot_data.to_string(),
            mime_type,
        }),
        reply_to_id: None,
        silent: false,
    };

    let mut tasks = Vec::with_capacity(targets.len());
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let payload = payload.clone();
        tasks.push(tokio::spawn(async move {
            match target.channel_type {
                moltis_channels::ChannelType::Telegram
                | moltis_channels::ChannelType::MsTeams
                | moltis_channels::ChannelType::Discord
                | moltis_channels::ChannelType::Whatsapp => {
                    let reply_to = target.message_id.as_deref();
                    if let Err(e) = outbound
                        .send_media(&target.account_id, &target.chat_id, &payload, reply_to)
                        .await
                    {
                        warn!(
                            account_id = target.account_id,
                            chat_id = target.chat_id,
                            "failed to send screenshot to channel: {e}"
                        );
                        // Notify the user of the error
                        let error_msg = format!("⚠️ Failed to send screenshot: {e}");
                        let _ = outbound
                            .send_text(&target.account_id, &target.chat_id, &error_msg, reply_to)
                            .await;
                    } else {
                        debug!(
                            account_id = target.account_id,
                            chat_id = target.chat_id,
                            "sent screenshot to channel"
                        );
                    }
                },
            }
        }));
    }

    for task in tasks {
        if let Err(e) = task.await {
            warn!(error = %e, "channel reply task join failed");
        }
    }
}

/// Send a native location pin to all pending channel targets for a session.
/// Uses `peek_channel_replies` so targets remain for the final text response.
async fn send_location_to_channels(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    latitude: f64,
    longitude: f64,
    title: Option<&str>,
) {
    let targets = state.peek_channel_replies(session_key).await;
    if targets.is_empty() {
        return;
    }

    let outbound = match state.channel_outbound() {
        Some(o) => o,
        None => return,
    };

    let title_owned = title.map(String::from);

    let mut tasks = Vec::with_capacity(targets.len());
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let title_ref = title_owned.clone();
        tasks.push(tokio::spawn(async move {
            let reply_to = target.message_id.as_deref();
            if let Err(e) = outbound
                .send_location(
                    &target.account_id,
                    &target.chat_id,
                    latitude,
                    longitude,
                    title_ref.as_deref(),
                    reply_to,
                )
                .await
            {
                warn!(
                    account_id = target.account_id,
                    chat_id = target.chat_id,
                    "failed to send location to channel: {e}"
                );
            } else {
                debug!(
                    account_id = target.account_id,
                    chat_id = target.chat_id,
                    "sent location pin to channel"
                );
            }
        }));
    }

    for task in tasks {
        if let Err(e) = task.await {
            warn!(error = %e, "channel location task join failed");
        }
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {
        super::*,
        anyhow::Result,
        moltis_agents::{model::LlmProvider, tool_registry::AgentTool},
        moltis_common::types::ReplyPayload,
        std::{
            pin::Pin,
            sync::{
                Arc,
                atomic::{AtomicUsize, Ordering},
            },
            time::{Duration, Instant},
        },
        tokio_stream::Stream,
    };

    struct DummyTool {
        name: String,
    }

    struct StaticProvider {
        name: String,
        id: String,
    }

    #[async_trait]
    impl LlmProvider for StaticProvider {
        fn name(&self) -> &str {
            &self.name
        }

        fn id(&self) -> &str {
            &self.id
        }

        async fn complete(
            &self,
            _messages: &[ChatMessage],
            _tools: &[Value],
        ) -> Result<moltis_agents::model::CompletionResponse> {
            anyhow::bail!("not implemented for test")
        }

        fn stream(
            &self,
            _messages: Vec<ChatMessage>,
        ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
            Box::pin(tokio_stream::empty())
        }
    }

    #[async_trait]
    impl AgentTool for DummyTool {
        fn name(&self) -> &str {
            &self.name
        }

        fn description(&self) -> &str {
            "test"
        }

        fn parameters_schema(&self) -> Value {
            serde_json::json!({})
        }

        async fn execute(&self, _params: Value) -> Result<Value> {
            Ok(serde_json::json!({}))
        }
    }

    #[async_trait]
    impl AgentTool for MockExecTool {
        fn name(&self) -> &str {
            "exec"
        }

        fn description(&self) -> &str {
            "mock exec"
        }

        fn parameters_schema(&self) -> Value {
            serde_json::json!({
                "type": "object",
                "required": ["command"],
                "properties": {
                    "command": { "type": "string" }
                }
            })
        }

        async fn execute(&self, params: Value) -> Result<Value> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(command) = params.get("command").and_then(Value::as_str) {
                self.commands
                    .lock()
                    .expect("mock exec commands mutex poisoned")
                    .push(command.to_string());
            }
            Ok(serde_json::json!({
                "stdout": "ok\n",
                "stderr": "",
                "exit_code": 0
            }))
        }
    }

    struct MockChannelOutbound {
        calls: Arc<AtomicUsize>,
        delay: Duration,
    }

    struct RecordingChannelOutbound {
        text_calls: Arc<AtomicUsize>,
        suffix_calls: Arc<AtomicUsize>,
        html_payloads: Arc<Mutex<Vec<String>>>,
    }

    struct MockChannelStreamOutbound {
        deltas: Arc<Mutex<Vec<String>>>,
        reply_tos: Arc<Mutex<Vec<Option<String>>>>,
        completions: Arc<AtomicUsize>,
        fail: bool,
        stream_enabled: bool,
    }

    struct MockChatRuntime {
        channel_replies: Mutex<HashMap<String, Vec<moltis_channels::ChannelReplyTarget>>>,
        channel_status_log: Mutex<HashMap<String, Vec<String>>>,
        channel_outbound: Option<Arc<dyn moltis_channels::ChannelOutbound>>,
        channel_stream_outbound: Option<Arc<dyn moltis_channels::ChannelStreamOutbound>>,
        tts: moltis_service_traits::NoopTtsService,
        project: moltis_service_traits::NoopProjectService,
        mcp: moltis_service_traits::NoopMcpService,
    }

    impl MockChatRuntime {
        fn new() -> Self {
            Self {
                channel_replies: Mutex::new(HashMap::new()),
                channel_status_log: Mutex::new(HashMap::new()),
                channel_outbound: None,
                channel_stream_outbound: None,
                tts: moltis_service_traits::NoopTtsService,
                project: moltis_service_traits::NoopProjectService,
                mcp: moltis_service_traits::NoopMcpService,
            }
        }

        fn with_channel_outbound(
            mut self,
            outbound: Arc<dyn moltis_channels::ChannelOutbound>,
        ) -> Self {
            self.channel_outbound = Some(outbound);
            self
        }

        fn with_channel_stream_outbound(
            mut self,
            outbound: Arc<dyn moltis_channels::ChannelStreamOutbound>,
        ) -> Self {
            self.channel_stream_outbound = Some(outbound);
            self
        }
    }

    #[async_trait]
    impl ChatRuntime for MockChatRuntime {
        async fn broadcast(&self, _topic: &str, _payload: Value) {}

        async fn push_channel_reply(
            &self,
            session_key: &str,
            target: moltis_channels::ChannelReplyTarget,
        ) {
            self.channel_replies
                .lock()
                .await
                .entry(session_key.to_string())
                .or_default()
                .push(target);
        }

        async fn drain_channel_replies(
            &self,
            session_key: &str,
        ) -> Vec<moltis_channels::ChannelReplyTarget> {
            self.channel_replies
                .lock()
                .await
                .remove(session_key)
                .unwrap_or_default()
        }

        async fn peek_channel_replies(
            &self,
            session_key: &str,
        ) -> Vec<moltis_channels::ChannelReplyTarget> {
            self.channel_replies
                .lock()
                .await
                .get(session_key)
                .cloned()
                .unwrap_or_default()
        }

        async fn push_channel_status_log(&self, session_key: &str, message: String) {
            self.channel_status_log
                .lock()
                .await
                .entry(session_key.to_string())
                .or_default()
                .push(message);
        }

        async fn drain_channel_status_log(&self, session_key: &str) -> Vec<String> {
            self.channel_status_log
                .lock()
                .await
                .remove(session_key)
                .unwrap_or_default()
        }

        async fn set_run_error(&self, _run_id: &str, _error: String) {}

        async fn active_session_key(&self, _conn_id: &str) -> Option<String> {
            None
        }

        async fn active_project_id(&self, _conn_id: &str) -> Option<String> {
            None
        }

        fn hostname(&self) -> &str {
            "test"
        }

        fn sandbox_router(&self) -> Option<&Arc<moltis_tools::sandbox::SandboxRouter>> {
            None
        }

        fn memory_manager(&self) -> Option<&Arc<moltis_memory::manager::MemoryManager>> {
            None
        }

        async fn cached_location(&self) -> Option<moltis_config::GeoLocation> {
            None
        }

        async fn tts_overrides(
            &self,
            _session_key: &str,
            _channel_key: &str,
        ) -> (Option<TtsOverride>, Option<TtsOverride>) {
            (None, None)
        }

        fn channel_outbound(&self) -> Option<Arc<dyn moltis_channels::ChannelOutbound>> {
            self.channel_outbound.clone()
        }

        fn channel_stream_outbound(
            &self,
        ) -> Option<Arc<dyn moltis_channels::ChannelStreamOutbound>> {
            self.channel_stream_outbound.clone()
        }

        fn tts_service(&self) -> &dyn moltis_service_traits::TtsService {
            &self.tts
        }

        fn project_service(&self) -> &dyn moltis_service_traits::ProjectService {
            &self.project
        }

        fn mcp_service(&self) -> &dyn moltis_service_traits::McpService {
            &self.mcp
        }

        async fn chat_service(&self) -> Arc<dyn ChatService> {
            Arc::new(moltis_service_traits::NoopChatService)
        }

        async fn last_run_error(&self, _run_id: &str) -> Option<String> {
            None
        }

        async fn send_push_notification(
            &self,
            _title: &str,
            _body: &str,
            _url: Option<&str>,
            _session_key: Option<&str>,
        ) -> error::Result<usize> {
            Ok(0)
        }

        async fn ensure_local_model_cached(&self, _model_id: &str) -> error::Result<bool> {
            Ok(false)
        }
    }

    fn mock_runtime() -> Arc<dyn ChatRuntime> {
        Arc::new(MockChatRuntime::new())
    }

    struct MockExecTool {
        calls: Arc<AtomicUsize>,
        commands: Arc<std::sync::Mutex<Vec<String>>>,
    }

    #[test]
    fn truncate_at_char_boundary_handles_multibyte_boundary() {
        let text = format!("{}л{}", "a".repeat(99), "z");
        let truncated = truncate_at_char_boundary(&text, 100);
        assert_eq!(truncated.len(), 99);
        assert!(truncated.chars().all(|c| c == 'a'));
    }

    #[test]
    fn session_token_usage_prefers_request_fields_for_current_context() {
        let messages = vec![
            serde_json::json!({
                "role": "assistant",
                "inputTokens": 50,
                "outputTokens": 10,
            }),
            serde_json::json!({
                "role": "assistant",
                "inputTokens": 120,
                "outputTokens": 40,
                "requestInputTokens": 75,
                "requestOutputTokens": 20,
            }),
        ];

        let usage = session_token_usage_from_messages(&messages);
        assert_eq!(usage.session_input_tokens, 170);
        assert_eq!(usage.session_output_tokens, 50);
        assert_eq!(usage.current_request_input_tokens, 75);
        assert_eq!(usage.current_request_output_tokens, 20);
    }

    #[test]
    fn session_token_usage_falls_back_to_legacy_turn_fields() {
        let messages = vec![serde_json::json!({
            "role": "assistant",
            "inputTokens": 33,
            "outputTokens": 11,
        })];

        let usage = session_token_usage_from_messages(&messages);
        assert_eq!(usage.current_request_input_tokens, 33);
        assert_eq!(usage.current_request_output_tokens, 11);
    }

    #[test]
    fn estimate_text_tokens_uses_non_empty_floor_and_byte_ratio() {
        assert_eq!(estimate_text_tokens(""), 0);
        assert_eq!(estimate_text_tokens("a"), 1);
        assert_eq!(estimate_text_tokens("abcd"), 1);
        assert_eq!(estimate_text_tokens("abcde"), 2);
    }

    #[test]
    fn parse_explicit_shell_command_extracts_command_text() {
        assert_eq!(
            parse_explicit_shell_command("/sh uname -a"),
            Some("uname -a")
        );
        assert_eq!(parse_explicit_shell_command("/sh\tls"), Some("ls"));
    }

    #[test]
    fn parse_explicit_shell_command_rejects_non_command_inputs() {
        assert!(parse_explicit_shell_command("/sh").is_none());
        assert!(parse_explicit_shell_command("/shell ls").is_none());
        assert!(parse_explicit_shell_command("uname -a").is_none());
    }

    #[test]
    fn prompt_now_for_timezone_returns_non_empty_string() {
        let value = prompt_now_for_timezone(Some("UTC"));
        assert!(!value.is_empty());
    }

    #[test]
    fn prompt_today_for_timezone_returns_non_empty_string() {
        let value = prompt_today_for_timezone(Some("UTC"));
        assert!(!value.is_empty());
    }

    #[test]
    fn server_prompt_timezone_prefers_configured_value() {
        assert_eq!(
            server_prompt_timezone(Some("Europe/Paris")),
            "Europe/Paris".to_string()
        );
    }

    #[test]
    fn server_prompt_timezone_defaults_to_server_local() {
        assert_eq!(server_prompt_timezone(None), "server-local".to_string());
        assert_eq!(server_prompt_timezone(Some("")), "server-local".to_string());
    }

    fn make_session_entry_with_binding(binding: Option<String>) -> SessionEntry {
        SessionEntry {
            id: "sid-1".to_string(),
            key: "session:key".to_string(),
            label: None,
            model: None,
            created_at: 0,
            updated_at: 0,
            message_count: 0,
            last_seen_message_count: 0,
            project_id: None,
            archived: false,
            worktree_branch: None,
            sandbox_enabled: None,
            sandbox_image: None,
            channel_binding: binding,
            parent_session_key: None,
            fork_point: None,
            mcp_disabled: None,
            preview: None,
            agent_id: None,
            version: 0,
        }
    }

    #[test]
    fn resolve_channel_runtime_context_sets_heartbeat_surface() {
        let context = resolve_channel_runtime_context("cron:heartbeat", None);
        assert_eq!(context.surface.as_deref(), Some("heartbeat"));
        assert_eq!(context.session_kind.as_deref(), Some("cron"));
        assert_eq!(context.channel_type, None);
    }

    #[test]
    fn resolve_channel_runtime_context_extracts_channel_binding() {
        let binding = moltis_channels::ChannelReplyTarget {
            channel_type: moltis_channels::ChannelType::Telegram,
            account_id: "bot-main".to_string(),
            chat_id: "123456".to_string(),
            message_id: Some("99".to_string()),
        };
        let binding_json = serde_json::to_string(&binding).expect("serialize binding");
        let entry = make_session_entry_with_binding(Some(binding_json));

        let context = resolve_channel_runtime_context("telegram:bot-main:123456", Some(&entry));
        assert_eq!(context.surface.as_deref(), Some("telegram"));
        assert_eq!(context.session_kind.as_deref(), Some("channel"));
        assert_eq!(context.channel_type.as_deref(), Some("telegram"));
        assert_eq!(context.channel_account_id.as_deref(), Some("bot-main"));
        assert_eq!(context.channel_chat_id.as_deref(), Some("123456"));
        assert_eq!(context.channel_chat_type.as_deref(), Some("private"));
    }

    #[test]
    fn resolve_channel_runtime_context_falls_back_to_web_when_unbound() {
        let context = resolve_channel_runtime_context("main", None);
        assert_eq!(context.surface.as_deref(), Some("web"));
        assert_eq!(context.session_kind.as_deref(), Some("web"));
        assert_eq!(context.channel_type, None);
        assert_eq!(context.channel_account_id, None);
        assert_eq!(context.channel_chat_id, None);
    }

    #[test]
    fn refresh_runtime_prompt_time_sets_host_time() {
        let mut host = PromptHostRuntimeContext {
            timezone: Some("UTC".to_string()),
            ..Default::default()
        };
        refresh_runtime_prompt_time(&mut host);
        assert!(host.time.as_deref().is_some_and(|value| !value.is_empty()));
        assert!(host.today.as_deref().is_some_and(|value| !value.is_empty()));
    }

    #[test]
    fn apply_request_runtime_context_uses_request_timezone() {
        let params = serde_json::json!({
            "_accept_language": "en-US,en;q=0.9",
            "_remote_ip": "203.0.113.10",
            "_timezone": "America/New_York",
        });

        let mut host = PromptHostRuntimeContext {
            timezone: Some("server-local".to_string()),
            ..Default::default()
        };
        apply_request_runtime_context(&mut host, &params);

        assert_eq!(host.accept_language.as_deref(), Some("en-US,en;q=0.9"));
        assert_eq!(host.remote_ip.as_deref(), Some("203.0.113.10"));
        assert_eq!(host.timezone.as_deref(), Some("America/New_York"));
        assert!(host.time.as_deref().is_some_and(|value| !value.is_empty()));
        assert!(host.today.as_deref().is_some_and(|value| value.len() >= 10));
    }

    #[test]
    fn apply_voice_reply_suffix_keeps_datetime_tail_at_end() {
        let runtime_context = PromptRuntimeContext {
            host: PromptHostRuntimeContext {
                time: Some("2026-02-17 16:18:00 CET".to_string()),
                ..Default::default()
            },
            sandbox: None,
        };
        let base_prompt =
            "You are a helpful assistant.\nThe current user datetime is 2026-02-17 16:18:00 CET.\n"
                .to_string();

        let prompt =
            apply_voice_reply_suffix(base_prompt, ReplyMedium::Voice, Some(&runtime_context));

        assert!(prompt.contains("## Voice Reply Mode"));
        assert!(
            prompt
                .trim_end()
                .ends_with("The current user datetime is 2026-02-17 16:18:00 CET.")
        );
        let voice_ix = prompt.find("## Voice Reply Mode");
        let datetime_ix = prompt.rfind("The current user datetime is 2026-02-17 16:18:00 CET.");
        assert!(voice_ix.is_some_and(|idx| datetime_ix.is_some_and(|tail| idx < tail)));
    }

    #[test]
    fn apply_voice_reply_suffix_noop_for_text_reply_mode() {
        let base_prompt = "You are a helpful assistant.".to_string();
        let prompt = apply_voice_reply_suffix(base_prompt.clone(), ReplyMedium::Text, None);
        assert_eq!(prompt, base_prompt);
    }

    #[test]
    fn format_tool_status_message_calc_uses_expression() {
        let message =
            format_tool_status_message("calc", &serde_json::json!({ "expression": "2 + 2 * 3" }));
        assert!(message.contains("🧮 Calculating:"));
        assert!(message.contains("2 + 2 * 3"));
    }

    #[test]
    fn format_tool_status_message_calc_handles_missing_expression() {
        let message = format_tool_status_message("calc", &serde_json::json!({}));
        assert_eq!(message, "🧮 Calculating...");
    }

    #[test]
    fn prompt_sandbox_no_network_state_uses_config_for_docker() {
        assert_eq!(prompt_sandbox_no_network_state("docker", true), Some(true));
        assert_eq!(
            prompt_sandbox_no_network_state("docker", false),
            Some(false)
        );
    }

    #[test]
    fn prompt_sandbox_no_network_state_omits_unsupported_backends() {
        assert_eq!(
            prompt_sandbox_no_network_state("apple-container", true),
            None
        );
        assert_eq!(prompt_sandbox_no_network_state("none", true), None);
        assert_eq!(prompt_sandbox_no_network_state("unknown", false), None);
    }

    #[test]
    fn is_safe_user_audio_filename_allows_sanitized_names() {
        assert!(is_safe_user_audio_filename("voice-123.webm"));
        assert!(is_safe_user_audio_filename("recording_1.ogg"));
    }

    #[test]
    fn user_audio_path_from_params_builds_session_scoped_media_path() {
        let params = serde_json::json!({
            "_audio_filename": "voice-123.webm",
        });
        assert_eq!(
            user_audio_path_from_params(&params, "session:abc"),
            Some("media/session_abc/voice-123.webm".to_string())
        );
    }

    #[test]
    fn user_audio_path_from_params_rejects_invalid_filename() {
        let params = serde_json::json!({
            "_audio_filename": "../secret.webm",
        });
        assert!(user_audio_path_from_params(&params, "main").is_none());
    }

    #[async_trait]
    impl moltis_channels::plugin::ChannelOutbound for MockChannelOutbound {
        async fn send_text(
            &self,
            _account_id: &str,
            _to: &str,
            _text: &str,
            _reply_to: Option<&str>,
        ) -> moltis_channels::Result<()> {
            tokio::time::sleep(self.delay).await;
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn send_media(
            &self,
            _account_id: &str,
            _to: &str,
            _payload: &ReplyPayload,
            _reply_to: Option<&str>,
        ) -> moltis_channels::Result<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl moltis_channels::plugin::ChannelOutbound for RecordingChannelOutbound {
        async fn send_text(
            &self,
            _account_id: &str,
            _to: &str,
            _text: &str,
            _reply_to: Option<&str>,
        ) -> moltis_channels::Result<()> {
            self.text_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn send_media(
            &self,
            _account_id: &str,
            _to: &str,
            _payload: &ReplyPayload,
            _reply_to: Option<&str>,
        ) -> moltis_channels::Result<()> {
            Ok(())
        }

        async fn send_text_with_suffix(
            &self,
            _account_id: &str,
            _to: &str,
            _text: &str,
            _suffix_html: &str,
            _reply_to: Option<&str>,
        ) -> moltis_channels::Result<()> {
            self.suffix_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn send_html(
            &self,
            _account_id: &str,
            _to: &str,
            html: &str,
            _reply_to: Option<&str>,
        ) -> moltis_channels::Result<()> {
            self.html_payloads.lock().await.push(html.to_string());
            Ok(())
        }
    }

    #[async_trait]
    impl moltis_channels::plugin::ChannelStreamOutbound for MockChannelStreamOutbound {
        async fn send_stream(
            &self,
            _account_id: &str,
            _to: &str,
            reply_to: Option<&str>,
            mut stream: moltis_channels::StreamReceiver,
        ) -> moltis_channels::Result<()> {
            if self.fail {
                return Err(moltis_channels::Error::unavailable("stream failed"));
            }
            self.reply_tos
                .lock()
                .await
                .push(reply_to.map(ToString::to_string));
            while let Some(event) = stream.recv().await {
                match event {
                    moltis_channels::StreamEvent::Delta(delta) => {
                        self.deltas.lock().await.push(delta);
                    },
                    moltis_channels::StreamEvent::Done | moltis_channels::StreamEvent::Error(_) => {
                        break;
                    },
                }
            }
            self.completions.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn is_stream_enabled(&self, _account_id: &str) -> bool {
            self.stream_enabled
        }
    }

    async fn sqlite_pool() -> sqlx::SqlitePool {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("sqlite memory pool");
        moltis_projects::run_migrations(&pool)
            .await
            .expect("projects migrations");
        SqliteSessionMetadata::init(&pool)
            .await
            .expect("session metadata migrations");
        pool
    }

    #[tokio::test]
    async fn deliver_channel_replies_waits_for_outbound_sends() {
        let calls = Arc::new(AtomicUsize::new(0));
        let outbound: Arc<dyn moltis_channels::plugin::ChannelOutbound> =
            Arc::new(MockChannelOutbound {
                calls: Arc::clone(&calls),
                delay: Duration::from_millis(50),
            });
        let targets = vec![moltis_channels::ChannelReplyTarget {
            channel_type: moltis_channels::ChannelType::Telegram,
            account_id: "acct".to_string(),
            chat_id: "123".to_string(),
            message_id: None,
        }];
        let state = mock_runtime();

        let start = Instant::now();
        deliver_channel_replies_to_targets(
            outbound,
            targets,
            "session:test",
            "hello",
            state,
            ReplyMedium::Text,
            Vec::new(),
            &HashSet::new(),
        )
        .await;

        assert!(
            start.elapsed() >= Duration::from_millis(45),
            "delivery should wait for outbound send completion"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn deliver_channel_replies_skips_targets_already_streamed() {
        let calls = Arc::new(AtomicUsize::new(0));
        let outbound: Arc<dyn moltis_channels::plugin::ChannelOutbound> =
            Arc::new(MockChannelOutbound {
                calls: Arc::clone(&calls),
                delay: Duration::from_millis(0),
            });
        let state: Arc<dyn ChatRuntime> =
            Arc::new(MockChatRuntime::new().with_channel_outbound(outbound));
        let target = moltis_channels::ChannelReplyTarget {
            channel_type: moltis_channels::ChannelType::Telegram,
            account_id: "acct".to_string(),
            chat_id: "123".to_string(),
            message_id: Some("42".to_string()),
        };

        state
            .push_channel_reply("telegram:acct:123", target.clone())
            .await;

        let mut streamed = HashSet::new();
        streamed.insert(ChannelReplyTargetKey::from(&target));
        deliver_channel_replies(
            &state,
            "telegram:acct:123",
            "hello",
            ReplyMedium::Text,
            &streamed,
        )
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(
            state
                .peek_channel_replies("telegram:acct:123")
                .await
                .is_empty(),
            "channel targets should be drained even when skipped by stream dedupe"
        );
    }

    #[tokio::test]
    async fn deliver_channel_replies_streamed_targets_get_logbook_follow_up() {
        let text_calls = Arc::new(AtomicUsize::new(0));
        let suffix_calls = Arc::new(AtomicUsize::new(0));
        let html_payloads = Arc::new(Mutex::new(Vec::new()));
        let outbound_impl = Arc::new(RecordingChannelOutbound {
            text_calls: Arc::clone(&text_calls),
            suffix_calls: Arc::clone(&suffix_calls),
            html_payloads: Arc::clone(&html_payloads),
        });
        let outbound: Arc<dyn moltis_channels::plugin::ChannelOutbound> = outbound_impl;
        let state: Arc<dyn ChatRuntime> =
            Arc::new(MockChatRuntime::new().with_channel_outbound(outbound));

        let target = moltis_channels::ChannelReplyTarget {
            channel_type: moltis_channels::ChannelType::Discord,
            account_id: "acct".to_string(),
            chat_id: "123".to_string(),
            message_id: Some("42".to_string()),
        };
        let session_key = "discord:acct:123";
        state.push_channel_reply(session_key, target.clone()).await;
        state
            .push_channel_status_log(session_key, "🌐 Browsing: https://example.com".to_string())
            .await;

        let mut streamed = HashSet::new();
        streamed.insert(ChannelReplyTargetKey::from(&target));
        deliver_channel_replies(&state, session_key, "hello", ReplyMedium::Text, &streamed).await;

        assert_eq!(
            text_calls.load(Ordering::SeqCst),
            0,
            "streamed targets should not receive duplicate text sends"
        );
        assert_eq!(
            suffix_calls.load(Ordering::SeqCst),
            0,
            "streamed targets should receive logbook via follow-up html, not text+suffix"
        );
        let payloads = html_payloads.lock().await.clone();
        assert_eq!(payloads.len(), 1, "expected one logbook follow-up");
        assert!(payloads[0].contains("Activity log"));
        assert!(payloads[0].contains("Browsing: https://example.com"));
    }

    #[tokio::test]
    async fn channel_stream_dispatcher_records_completed_targets() {
        let deltas = Arc::new(Mutex::new(Vec::new()));
        let reply_tos = Arc::new(Mutex::new(Vec::new()));
        let completions = Arc::new(AtomicUsize::new(0));
        let stream_outbound: Arc<dyn moltis_channels::plugin::ChannelStreamOutbound> =
            Arc::new(MockChannelStreamOutbound {
                deltas: Arc::clone(&deltas),
                reply_tos: Arc::clone(&reply_tos),
                completions: Arc::clone(&completions),
                fail: false,
                stream_enabled: true,
            });

        let state: Arc<dyn ChatRuntime> =
            Arc::new(MockChatRuntime::new().with_channel_stream_outbound(stream_outbound));
        let target = moltis_channels::ChannelReplyTarget {
            channel_type: moltis_channels::ChannelType::Telegram,
            account_id: "acct".to_string(),
            chat_id: "123".to_string(),
            message_id: Some("55".to_string()),
        };
        let session_key = "telegram:acct:123";
        state.push_channel_reply(session_key, target.clone()).await;

        let mut dispatcher = ChannelStreamDispatcher::for_session(&state, session_key)
            .await
            .expect("stream dispatcher should be created");
        dispatcher.send_delta("Hel").await;
        dispatcher.send_delta("lo").await;
        dispatcher.finish().await;

        let completed = dispatcher.completed_target_keys().await;
        assert!(completed.contains(&ChannelReplyTargetKey::from(&target)));
        assert_eq!(completions.load(Ordering::SeqCst), 1);
        assert_eq!(deltas.lock().await.join(""), "Hello");
        assert_eq!(reply_tos.lock().await.as_slice(), &[Some("55".to_string())]);
    }

    #[tokio::test]
    async fn channel_stream_dispatcher_skips_failed_workers_from_dedupe() {
        let stream_outbound: Arc<dyn moltis_channels::plugin::ChannelStreamOutbound> =
            Arc::new(MockChannelStreamOutbound {
                deltas: Arc::new(Mutex::new(Vec::new())),
                reply_tos: Arc::new(Mutex::new(Vec::new())),
                completions: Arc::new(AtomicUsize::new(0)),
                fail: true,
                stream_enabled: true,
            });
        let state: Arc<dyn ChatRuntime> =
            Arc::new(MockChatRuntime::new().with_channel_stream_outbound(stream_outbound));
        let target = moltis_channels::ChannelReplyTarget {
            channel_type: moltis_channels::ChannelType::Telegram,
            account_id: "acct".to_string(),
            chat_id: "123".to_string(),
            message_id: Some("55".to_string()),
        };
        let session_key = "telegram:acct:123";
        state.push_channel_reply(session_key, target.clone()).await;

        let mut dispatcher = ChannelStreamDispatcher::for_session(&state, session_key)
            .await
            .expect("stream dispatcher should be created");
        dispatcher.send_delta("Hello").await;
        dispatcher.finish().await;

        let completed = dispatcher.completed_target_keys().await;
        assert!(
            !completed.contains(&ChannelReplyTargetKey::from(&target)),
            "failed stream workers must not be excluded from final fallback delivery"
        );
    }

    #[tokio::test]
    async fn channel_stream_dispatcher_skips_stream_disabled_targets() {
        let deltas = Arc::new(Mutex::new(Vec::new()));
        let completions = Arc::new(AtomicUsize::new(0));
        let stream_outbound: Arc<dyn moltis_channels::plugin::ChannelStreamOutbound> =
            Arc::new(MockChannelStreamOutbound {
                deltas: Arc::clone(&deltas),
                reply_tos: Arc::new(Mutex::new(Vec::new())),
                completions: Arc::clone(&completions),
                fail: false,
                stream_enabled: false,
            });
        let state: Arc<dyn ChatRuntime> =
            Arc::new(MockChatRuntime::new().with_channel_stream_outbound(stream_outbound));
        let target = moltis_channels::ChannelReplyTarget {
            channel_type: moltis_channels::ChannelType::Telegram,
            account_id: "acct".to_string(),
            chat_id: "123".to_string(),
            message_id: Some("55".to_string()),
        };
        let session_key = "telegram:acct:123";
        state.push_channel_reply(session_key, target.clone()).await;

        let mut dispatcher = ChannelStreamDispatcher::for_session(&state, session_key)
            .await
            .expect("stream dispatcher should be created");
        dispatcher.send_delta("Hello").await;
        dispatcher.finish().await;

        assert!(dispatcher.completed_target_keys().await.is_empty());
        assert_eq!(completions.load(Ordering::SeqCst), 0);
        assert!(deltas.lock().await.is_empty());
    }

    #[tokio::test]
    async fn channel_stream_dispatcher_no_deltas_do_not_dedup_targets() {
        let stream_outbound: Arc<dyn moltis_channels::plugin::ChannelStreamOutbound> =
            Arc::new(MockChannelStreamOutbound {
                deltas: Arc::new(Mutex::new(Vec::new())),
                reply_tos: Arc::new(Mutex::new(Vec::new())),
                completions: Arc::new(AtomicUsize::new(0)),
                fail: false,
                stream_enabled: true,
            });
        let state: Arc<dyn ChatRuntime> =
            Arc::new(MockChatRuntime::new().with_channel_stream_outbound(stream_outbound));
        let target = moltis_channels::ChannelReplyTarget {
            channel_type: moltis_channels::ChannelType::Telegram,
            account_id: "acct".to_string(),
            chat_id: "123".to_string(),
            message_id: Some("55".to_string()),
        };
        let session_key = "telegram:acct:123";
        state.push_channel_reply(session_key, target.clone()).await;

        let mut dispatcher = ChannelStreamDispatcher::for_session(&state, session_key)
            .await
            .expect("stream dispatcher should be created");
        dispatcher.finish().await;

        let completed = dispatcher.completed_target_keys().await;
        assert!(
            !completed.contains(&ChannelReplyTargetKey::from(&target)),
            "targets must not be stream-deduped when no deltas were sent"
        );
    }

    /// Regression test for #173: voice reply medium must not suppress stream
    /// dedup. When a dispatcher successfully streams to a target, the target
    /// key must appear in the completed set regardless of `ReplyMedium`.
    #[tokio::test]
    async fn channel_stream_voice_dedup_excludes_streamed_targets() {
        let deltas = Arc::new(Mutex::new(Vec::new()));
        let reply_tos = Arc::new(Mutex::new(Vec::new()));
        let completions = Arc::new(AtomicUsize::new(0));
        let stream_outbound: Arc<dyn moltis_channels::plugin::ChannelStreamOutbound> =
            Arc::new(MockChannelStreamOutbound {
                deltas: Arc::clone(&deltas),
                reply_tos: Arc::clone(&reply_tos),
                completions: Arc::clone(&completions),
                fail: false,
                stream_enabled: true,
            });

        let state: Arc<dyn ChatRuntime> =
            Arc::new(MockChatRuntime::new().with_channel_stream_outbound(stream_outbound));
        let target = moltis_channels::ChannelReplyTarget {
            channel_type: moltis_channels::ChannelType::Telegram,
            account_id: "acct".to_string(),
            chat_id: "456".to_string(),
            message_id: Some("77".to_string()),
        };
        let session_key = "telegram:acct:456";
        state.push_channel_reply(session_key, target.clone()).await;

        let mut dispatcher = ChannelStreamDispatcher::for_session(&state, session_key)
            .await
            .expect("stream dispatcher should be created");
        dispatcher.send_delta("voice reply").await;
        dispatcher.finish().await;

        // The completed keys must be returned even when the caller intends a
        // Voice reply — previously this returned an empty set for non-Text
        // mediums, causing double delivery.
        let completed = dispatcher.completed_target_keys().await;
        assert!(
            completed.contains(&ChannelReplyTargetKey::from(&target)),
            "completed targets must be reported regardless of reply medium"
        );
        assert_eq!(completions.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn explicit_sh_bypasses_provider_and_executes_directly() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(SqliteSessionMetadata::new(pool));

        let runtime = Arc::new(MockChatRuntime::new());
        let state: Arc<dyn ChatRuntime> = runtime.clone();

        let providers = Arc::new(RwLock::new(ProviderRegistry::empty()));
        let disabled = Arc::new(RwLock::new(DisabledModelsStore::default()));
        let calls = Arc::new(AtomicUsize::new(0));
        let commands = Arc::new(std::sync::Mutex::new(Vec::new()));

        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MockExecTool {
            calls: Arc::clone(&calls),
            commands: Arc::clone(&commands),
        }));

        let chat = LiveChatService::new(providers, disabled, state, Arc::clone(&store), metadata)
            .with_tools(Arc::new(RwLock::new(registry)));

        let send_result = chat
            .send(serde_json::json!({ "text": "/sh df -h" }))
            .await
            .expect("chat.send should succeed for explicit /sh");
        assert!(
            send_result
                .get("runId")
                .and_then(Value::as_str)
                .is_some_and(|id| !id.is_empty())
        );

        let history = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let messages = store.read("main").await.unwrap_or_default();
                let has_tool_result = messages
                    .iter()
                    .any(|msg| msg.get("role").and_then(Value::as_str) == Some("tool_result"));
                let has_assistant = messages
                    .iter()
                    .any(|msg| msg.get("role").and_then(Value::as_str) == Some("assistant"));
                if has_tool_result && has_assistant {
                    return messages;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("tool result and assistant turn should be persisted");

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let seen_commands = commands
            .lock()
            .expect("mock exec commands mutex poisoned")
            .clone();
        assert_eq!(seen_commands, vec!["df -h".to_string()]);
        assert!(
            history
                .iter()
                .any(|msg| msg.get("role").and_then(Value::as_str) == Some("assistant")),
            "explicit /sh should persist an assistant turn for history coherence"
        );
    }

    #[test]
    fn format_channel_retry_message_rounds_up_seconds() {
        let error_obj = serde_json::json!({ "type": "rate_limit_exceeded" });
        let msg = format_channel_retry_message(&error_obj, Duration::from_millis(1_200));
        assert!(msg.contains("Retrying in 2s"));
    }

    #[test]
    fn format_channel_error_message_prefers_structured_fields() {
        let error_obj = serde_json::json!({
            "title": "Rate limited",
            "detail": "Please wait and try again.",
        });
        let msg = format_channel_error_message(&error_obj);
        assert_eq!(msg, "⚠️ Rate limited: Please wait and try again.");
    }

    #[test]
    fn next_stream_retry_delay_uses_retry_after_for_rate_limits() {
        let mut server_retries_remaining = STREAM_SERVER_MAX_RETRIES;
        let mut rate_limit_retries_remaining = STREAM_RATE_LIMIT_MAX_RETRIES;
        let mut rate_limit_backoff_ms = None;
        let error_obj = serde_json::json!({
            "type": "rate_limit_exceeded",
            "retryAfterMs": 3500
        });

        let delay = next_stream_retry_delay_ms(
            "HTTP 429 Too Many Requests",
            &error_obj,
            &mut server_retries_remaining,
            &mut rate_limit_retries_remaining,
            &mut rate_limit_backoff_ms,
        );

        assert_eq!(delay, Some(3500));
        assert_eq!(
            rate_limit_retries_remaining,
            STREAM_RATE_LIMIT_MAX_RETRIES - 1
        );
        assert_eq!(
            rate_limit_backoff_ms,
            Some(STREAM_RATE_LIMIT_INITIAL_RETRY_MS)
        );
    }

    #[test]
    fn next_stream_retry_delay_retries_transient_server_errors_once() {
        let mut server_retries_remaining = STREAM_SERVER_MAX_RETRIES;
        let mut rate_limit_retries_remaining = STREAM_RATE_LIMIT_MAX_RETRIES;
        let mut rate_limit_backoff_ms = None;

        let first = next_stream_retry_delay_ms(
            "HTTP 503 Service Unavailable",
            &serde_json::json!({"type":"api_error"}),
            &mut server_retries_remaining,
            &mut rate_limit_retries_remaining,
            &mut rate_limit_backoff_ms,
        );
        let second = next_stream_retry_delay_ms(
            "HTTP 503 Service Unavailable",
            &serde_json::json!({"type":"api_error"}),
            &mut server_retries_remaining,
            &mut rate_limit_retries_remaining,
            &mut rate_limit_backoff_ms,
        );

        assert_eq!(first, Some(STREAM_SERVER_RETRY_DELAY_MS));
        assert_eq!(second, None);
    }

    #[tokio::test]
    async fn ordered_runner_event_callback_stays_in_order_with_variable_processing_latency() {
        let (on_event, mut rx) = ordered_runner_event_callback();
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen_for_worker = Arc::clone(&seen);

        let worker = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let RunnerEvent::TextDelta(text) = event {
                    if text == "slow" {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    seen_for_worker.lock().await.push(text);
                }
            }
        });

        on_event(RunnerEvent::TextDelta("slow".to_string()));
        on_event(RunnerEvent::TextDelta("fast".to_string()));
        drop(on_event);

        worker.await.unwrap();
        let observed = seen.lock().await.clone();
        assert_eq!(observed, vec!["slow".to_string(), "fast".to_string()]);
    }

    /// Build a bare session_locks map for testing the semaphore logic
    /// without constructing a full LiveChatService.
    fn make_session_locks() -> Arc<RwLock<HashMap<String, Arc<Semaphore>>>> {
        Arc::new(RwLock::new(HashMap::new()))
    }

    async fn get_or_create_semaphore(
        locks: &Arc<RwLock<HashMap<String, Arc<Semaphore>>>>,
        key: &str,
    ) -> Arc<Semaphore> {
        {
            let map = locks.read().await;
            if let Some(sem) = map.get(key) {
                return Arc::clone(sem);
            }
        }
        let mut map = locks.write().await;
        Arc::clone(
            map.entry(key.to_string())
                .or_insert_with(|| Arc::new(Semaphore::new(1))),
        )
    }

    fn make_active_run_maps() -> (
        Arc<RwLock<HashMap<String, AbortHandle>>>,
        Arc<RwLock<HashMap<String, String>>>,
    ) {
        (
            Arc::new(RwLock::new(HashMap::new())),
            Arc::new(RwLock::new(HashMap::new())),
        )
    }

    #[tokio::test]
    async fn same_session_runs_are_serialized() {
        let locks = make_session_locks();
        let sem = get_or_create_semaphore(&locks, "s1").await;

        // Acquire the permit — simulates a running task.
        let permit = sem.clone().acquire_owned().await.unwrap();

        // A second acquire should not resolve while the first is held.
        let sem2 = sem.clone();
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _p = sem2.acquire_owned().await.unwrap();
            let _ = tx.send(());
        });

        // Give the second task a chance to run — it should be blocked.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            rx.try_recv().is_err(),
            "second run should be blocked while first holds permit"
        );

        // Release first permit.
        drop(permit);

        // Now the second task should complete.
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn different_sessions_run_in_parallel() {
        let locks = make_session_locks();
        let sem_a = get_or_create_semaphore(&locks, "a").await;
        let sem_b = get_or_create_semaphore(&locks, "b").await;

        let _pa = sem_a.clone().acquire_owned().await.unwrap();
        // Session "b" should still be acquirable.
        let _pb = sem_b.clone().acquire_owned().await.unwrap();
    }

    #[tokio::test]
    async fn abort_releases_permit() {
        let locks = make_session_locks();
        let sem = get_or_create_semaphore(&locks, "s").await;

        let sem2 = sem.clone();
        let task = tokio::spawn(async move {
            let _p = sem2.acquire_owned().await.unwrap();
            // Simulate long-running work.
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        });

        // Give the task time to acquire the permit.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Abort the task — this drops the permit.
        task.abort();
        let _ = task.await;

        // The semaphore should now be acquirable.
        let _p = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            sem.clone().acquire_owned(),
        )
        .await
        .expect("permit should be available after abort")
        .unwrap();
    }

    #[tokio::test]
    async fn abort_run_handle_resolves_run_from_session_key() {
        let (active_runs, active_runs_by_session) = make_active_run_maps();

        let task = tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        });

        active_runs
            .write()
            .await
            .insert("run-a".to_string(), task.abort_handle());
        active_runs_by_session
            .write()
            .await
            .insert("main".to_string(), "run-a".to_string());

        let (resolved_run_id, aborted) = LiveChatService::abort_run_handle(
            &active_runs,
            &active_runs_by_session,
            None,
            Some("main"),
        )
        .await;
        assert_eq!(resolved_run_id.as_deref(), Some("run-a"));
        assert!(aborted);
        assert!(active_runs.read().await.is_empty());
        assert!(active_runs_by_session.read().await.is_empty());

        let err = task.await.expect_err("task should be cancelled");
        assert!(err.is_cancelled());
    }

    #[tokio::test]
    async fn abort_run_handle_by_run_id_clears_session_lookup() {
        let (active_runs, active_runs_by_session) = make_active_run_maps();

        let task = tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        });

        active_runs
            .write()
            .await
            .insert("run-b".to_string(), task.abort_handle());
        active_runs_by_session
            .write()
            .await
            .insert("main".to_string(), "run-b".to_string());

        let (resolved_run_id, aborted) = LiveChatService::abort_run_handle(
            &active_runs,
            &active_runs_by_session,
            Some("run-b"),
            None,
        )
        .await;
        assert_eq!(resolved_run_id.as_deref(), Some("run-b"));
        assert!(aborted);
        assert!(active_runs.read().await.is_empty());
        assert!(active_runs_by_session.read().await.is_empty());

        let err = task.await.expect_err("task should be cancelled");
        assert!(err.is_cancelled());
    }

    #[tokio::test]
    async fn agent_timeout_cancels_slow_future() {
        use std::time::Duration;

        let timeout_secs: u64 = 1;
        let slow_fut = async {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Some(("done".to_string(), 0u32, 0u32))
        };

        let result: Option<(String, u32, u32)> =
            tokio::time::timeout(Duration::from_secs(timeout_secs), slow_fut)
                .await
                .unwrap_or_default();

        assert!(
            result.is_none(),
            "slow future should have been cancelled by timeout"
        );
    }

    #[tokio::test]
    async fn agent_timeout_zero_means_no_timeout() {
        use std::time::Duration;

        let timeout_secs: u64 = 0;
        let fast_fut = async { Some(("ok".to_string(), 10u32, 5u32)) };

        let result = if timeout_secs > 0 {
            tokio::time::timeout(Duration::from_secs(timeout_secs), fast_fut)
                .await
                .unwrap_or_default()
        } else {
            fast_fut.await
        };

        assert_eq!(result, Some(("ok".to_string(), 10, 5)));
    }

    // ── Message queue tests ──────────────────────────────────────────────

    fn make_message_queue() -> Arc<RwLock<HashMap<String, Vec<QueuedMessage>>>> {
        Arc::new(RwLock::new(HashMap::new()))
    }

    fn make_target(message_id: &str) -> Value {
        serde_json::json!({
            "channel_type": "telegram",
            "account_id": "bot1",
            "chat_id": "123",
            "message_id": message_id,
        })
    }

    #[tokio::test]
    async fn queue_enqueue_and_drain() {
        let queue = make_message_queue();
        let key = "sess1";

        // Enqueue two messages.
        {
            let mut q = queue.write().await;
            q.entry(key.to_string()).or_default().push(QueuedMessage {
                params: serde_json::json!({"text": "hello"}),
            });
            q.entry(key.to_string()).or_default().push(QueuedMessage {
                params: serde_json::json!({"text": "world"}),
            });
        }

        // Drain.
        let drained = queue.write().await.remove(key).unwrap_or_default();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].params["text"], "hello");
        assert_eq!(drained[1].params["text"], "world");

        // Queue should be empty after drain.
        assert!(queue.read().await.get(key).is_none());
    }

    #[tokio::test]
    async fn queue_collect_concatenates_texts() {
        let msgs = [
            QueuedMessage {
                params: serde_json::json!({"text": "first", "model": "gpt-4"}),
            },
            QueuedMessage {
                params: serde_json::json!({"text": "second"}),
            },
            QueuedMessage {
                params: serde_json::json!({"text": "third", "_conn_id": "c1"}),
            },
        ];

        let combined: Vec<&str> = msgs
            .iter()
            .filter_map(|m| m.params.get("text").and_then(|v| v.as_str()))
            .collect();
        let joined = combined.join("\n\n");
        assert_eq!(joined, "first\n\nsecond\n\nthird");
    }

    #[tokio::test]
    async fn try_acquire_returns_err_when_held() {
        let sem = Arc::new(Semaphore::new(1));
        let _permit = sem.clone().try_acquire_owned().unwrap();

        // Second try_acquire should fail.
        assert!(sem.clone().try_acquire_owned().is_err());
    }

    #[tokio::test]
    async fn try_acquire_succeeds_when_free() {
        let sem = Arc::new(Semaphore::new(1));
        assert!(sem.clone().try_acquire_owned().is_ok());
    }

    #[tokio::test]
    async fn queue_drain_empty_is_noop() {
        let queue = make_message_queue();
        let drained = queue
            .write()
            .await
            .remove("nonexistent")
            .unwrap_or_default();
        assert!(drained.is_empty());
    }

    #[tokio::test]
    async fn queue_drain_drops_permit_before_send() {
        // Simulate the fixed drain flow: after `drop(permit)`, the semaphore
        // should be available for the replayed `chat.send()` to acquire.
        let sem = Arc::new(Semaphore::new(1));
        let permit = sem.clone().try_acquire_owned().unwrap();

        // While held, a second acquire must fail (simulates the bug).
        assert!(sem.clone().try_acquire_owned().is_err());

        // Drop — mirrors the new `drop(permit)` before the drain loop.
        drop(permit);

        // Now the replayed send can acquire the permit.
        assert!(
            sem.clone().try_acquire_owned().is_ok(),
            "permit should be available after explicit drop"
        );
    }

    #[tokio::test]
    async fn followup_drain_sends_only_first_and_requeues_rest() {
        let queue = make_message_queue();
        let key = "sess_drain";

        // Simulate three queued messages.
        {
            let mut q = queue.write().await;
            let entry = q.entry(key.to_string()).or_default();
            entry.push(QueuedMessage {
                params: serde_json::json!({"text": "a"}),
            });
            entry.push(QueuedMessage {
                params: serde_json::json!({"text": "b"}),
            });
            entry.push(QueuedMessage {
                params: serde_json::json!({"text": "c"}),
            });
        }

        // Drain and apply the send-first/requeue-rest logic.
        let queued = queue.write().await.remove(key).unwrap_or_default();

        let mut iter = queued.into_iter();
        let first = iter.next().expect("queued is non-empty");
        let rest: Vec<QueuedMessage> = iter.collect();

        // The first message is the one to send.
        assert_eq!(first.params["text"], "a");

        // Remaining messages are re-queued.
        if !rest.is_empty() {
            queue
                .write()
                .await
                .entry(key.to_string())
                .or_default()
                .extend(rest);
        }

        // Verify the queue now holds exactly the two remaining messages.
        let remaining = queue.read().await;
        let entries = remaining.get(key).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].params["text"], "b");
        assert_eq!(entries[1].params["text"], "c");
    }

    #[tokio::test]
    async fn followup_drain_preserves_per_message_channel_targets() {
        let queue = make_message_queue();
        let key = "sess_channel_targets";

        {
            let mut q = queue.write().await;
            let entry = q.entry(key.to_string()).or_default();
            entry.push(QueuedMessage {
                params: serde_json::json!({
                    "text": "a",
                    "_channel_reply_target": make_target("m1"),
                }),
            });
            entry.push(QueuedMessage {
                params: serde_json::json!({
                    "text": "b",
                    "_channel_reply_target": make_target("m2"),
                }),
            });
            entry.push(QueuedMessage {
                params: serde_json::json!({
                    "text": "c",
                    "_channel_reply_target": make_target("m3"),
                }),
            });
        }

        let queued = queue.write().await.remove(key).unwrap_or_default();
        let mut iter = queued.into_iter();
        let first = iter.next().expect("queued is non-empty");
        let rest: Vec<QueuedMessage> = iter.collect();

        assert_eq!(first.params["_channel_reply_target"]["message_id"], "m1");

        if !rest.is_empty() {
            queue
                .write()
                .await
                .entry(key.to_string())
                .or_default()
                .extend(rest);
        }

        let remaining = queue.read().await;
        let entries = remaining.get(key).expect("requeued messages");
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0].params["_channel_reply_target"]["message_id"],
            "m2"
        );
        assert_eq!(
            entries[1].params["_channel_reply_target"]["message_id"],
            "m3"
        );
    }

    #[tokio::test]
    async fn collect_drain_uses_last_message_channel_target() {
        let queued = [
            QueuedMessage {
                params: serde_json::json!({
                    "text": "first",
                    "_channel_reply_target": make_target("m1"),
                }),
            },
            QueuedMessage {
                params: serde_json::json!({
                    "text": "second",
                    "_channel_reply_target": make_target("m2"),
                }),
            },
            QueuedMessage {
                params: serde_json::json!({
                    "text": "third",
                    "_channel_reply_target": make_target("m3"),
                }),
            },
        ];

        let combined: Vec<&str> = queued
            .iter()
            .filter_map(|m| m.params.get("text").and_then(|v| v.as_str()))
            .collect();
        let mut merged = queued.last().expect("non-empty queue").params.clone();
        merged["text"] = serde_json::json!(combined.join("\n\n"));

        assert_eq!(merged["text"], "first\n\nsecond\n\nthird");
        assert_eq!(merged["_channel_reply_target"]["message_id"], "m3");
    }

    #[test]
    fn message_queue_mode_default_is_followup() {
        let mode = MessageQueueMode::default();
        assert_eq!(mode, MessageQueueMode::Followup);
    }

    #[test]
    fn message_queue_mode_deserializes_from_toml() {
        use serde::Deserialize;

        #[derive(Deserialize)]
        struct Wrapper {
            mode: MessageQueueMode,
        }

        let followup: Wrapper = toml::from_str(r#"mode = "followup""#).unwrap();
        assert_eq!(followup.mode, MessageQueueMode::Followup);

        let collect: Wrapper = toml::from_str(r#"mode = "collect""#).unwrap();
        assert_eq!(collect.mode, MessageQueueMode::Collect);
    }

    #[tokio::test]
    async fn cancel_queued_clears_session_queue() {
        let queue = make_message_queue();
        let key = "sess_cancel";

        // Enqueue two messages.
        {
            let mut q = queue.write().await;
            let entry = q.entry(key.to_string()).or_default();
            entry.push(QueuedMessage {
                params: serde_json::json!({"text": "a"}),
            });
            entry.push(QueuedMessage {
                params: serde_json::json!({"text": "b"}),
            });
        }

        // Cancel (same logic as cancel_queued: remove + unwrap_or_default).
        let removed = queue.write().await.remove(key).unwrap_or_default();
        assert_eq!(removed.len(), 2);

        // Queue should be empty.
        assert!(queue.read().await.get(key).is_none());
    }

    #[tokio::test]
    async fn cancel_queued_returns_count() {
        let queue = make_message_queue();
        let key = "sess_count";

        {
            let mut q = queue.write().await;
            let entry = q.entry(key.to_string()).or_default();
            entry.push(QueuedMessage {
                params: serde_json::json!({"text": "x"}),
            });
            entry.push(QueuedMessage {
                params: serde_json::json!({"text": "y"}),
            });
            entry.push(QueuedMessage {
                params: serde_json::json!({"text": "z"}),
            });
        }

        let removed = queue.write().await.remove(key).unwrap_or_default();
        let count = removed.len();
        assert_eq!(count, 3);
        let result = serde_json::json!({ "cleared": count });
        assert_eq!(result["cleared"], 3);
    }

    #[tokio::test]
    async fn cancel_queued_noop_for_empty_queue() {
        let queue = make_message_queue();
        let key = "sess_empty";

        // Cancel on a session with no queued messages.
        let removed = queue.write().await.remove(key).unwrap_or_default();
        assert_eq!(removed.len(), 0);

        let result = serde_json::json!({ "cleared": removed.len() });
        assert_eq!(result["cleared"], 0);
    }

    #[test]
    fn effective_tool_policy_profile_and_config_merge() {
        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.tools.policy.profile = Some("full".into());
        cfg.tools.policy.deny = vec!["exec".into()];

        let policy = effective_tool_policy(&cfg);
        assert!(!policy.is_allowed("exec"));
        assert!(policy.is_allowed("web_fetch"));
    }

    #[test]
    fn runtime_filters_apply_policy_without_skill_tool_restrictions() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool {
            name: "exec".to_string(),
        }));
        registry.register(Box::new(DummyTool {
            name: "web_fetch".to_string(),
        }));
        registry.register(Box::new(DummyTool {
            name: "create_skill".to_string(),
        }));
        registry.register(Box::new(DummyTool {
            name: "session_state".to_string(),
        }));

        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.tools.policy.allow = vec!["exec".into(), "web_fetch".into(), "create_skill".into()];

        let skills = vec![moltis_skills::types::SkillMetadata {
            name: "my-skill".into(),
            description: "test".into(),
            license: None,
            compatibility: None,
            allowed_tools: vec!["Bash(git:*)".into()],
            homepage: None,
            dockerfile: None,
            requires: Default::default(),
            path: PathBuf::new(),
            source: None,
        }];

        let filtered = apply_runtime_tool_filters(&registry, &cfg, &skills, false);
        assert!(filtered.get("exec").is_some());
        assert!(filtered.get("web_fetch").is_some());
        assert!(filtered.get("create_skill").is_some());
        assert!(filtered.get("session_state").is_none());
    }

    #[test]
    fn runtime_filters_do_not_hide_create_skill_when_skill_allows_only_web_fetch() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool {
            name: "create_skill".to_string(),
        }));
        registry.register(Box::new(DummyTool {
            name: "web_fetch".to_string(),
        }));

        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.tools.policy.allow = vec!["create_skill".into(), "web_fetch".into()];

        let skills = vec![moltis_skills::types::SkillMetadata {
            name: "weather".into(),
            description: "weather checker".into(),
            license: None,
            compatibility: None,
            allowed_tools: vec!["WebFetch".into()],
            homepage: None,
            dockerfile: None,
            requires: Default::default(),
            path: PathBuf::new(),
            source: None,
        }];

        let filtered = apply_runtime_tool_filters(&registry, &cfg, &skills, false);
        assert!(filtered.get("create_skill").is_some());
        assert!(filtered.get("web_fetch").is_some());
    }

    #[test]
    fn priority_models_pin_raw_model_ids_first() {
        let m1 = moltis_providers::ModelInfo {
            id: "openai-codex::gpt-5.2".into(),
            provider: "openai-codex".into(),
            display_name: "GPT 5.2".into(),
            created_at: None,
        };
        let m2 = moltis_providers::ModelInfo {
            id: "anthropic::claude-opus-4-5".into(),
            provider: "anthropic".into(),
            display_name: "Claude Opus 4.5".into(),
            created_at: None,
        };
        let m3 = moltis_providers::ModelInfo {
            id: "google::gemini-3-flash".into(),
            provider: "gemini".into(),
            display_name: "Gemini 3 Flash".into(),
            created_at: None,
        };

        let order =
            LiveModelService::build_priority_order(&["gpt-5.2".into(), "claude-opus-4-5".into()]);
        let ordered = LiveModelService::prioritize_models(&order, vec![&m3, &m2, &m1].into_iter());
        assert_eq!(ordered[0].id, m1.id);
        assert_eq!(ordered[1].id, m2.id);
        assert_eq!(ordered[2].id, m3.id);
    }

    #[test]
    fn priority_models_match_separator_variants() {
        let m1 = moltis_providers::ModelInfo {
            id: "openai-codex::gpt-5.2".into(),
            provider: "openai-codex".into(),
            display_name: "GPT-5.2".into(),
            created_at: None,
        };
        let m2 = moltis_providers::ModelInfo {
            id: "anthropic::claude-sonnet-4-5-20250929".into(),
            provider: "anthropic".into(),
            display_name: "Claude Sonnet 4.5".into(),
            created_at: None,
        };
        let m3 = moltis_providers::ModelInfo {
            id: "google::gemini-3-flash".into(),
            provider: "gemini".into(),
            display_name: "Gemini 3 Flash".into(),
            created_at: None,
        };

        let order =
            LiveModelService::build_priority_order(&["gpt 5.2".into(), "claude-sonnet-4.5".into()]);
        let ordered = LiveModelService::prioritize_models(&order, vec![&m3, &m2, &m1].into_iter());
        assert_eq!(ordered[0].id, m1.id);
        assert_eq!(ordered[1].id, m2.id);
        assert_eq!(ordered[2].id, m3.id);
    }

    #[test]
    fn models_without_priority_prefer_subscription_providers() {
        let m1 = moltis_providers::ModelInfo {
            id: "openai::gpt-5.2".into(),
            provider: "openai".into(),
            display_name: "GPT-5.2".into(),
            created_at: None,
        };
        let m2 = moltis_providers::ModelInfo {
            id: "openai-codex::gpt-5.2".into(),
            provider: "openai-codex".into(),
            display_name: "GPT-5.2".into(),
            created_at: None,
        };
        let m3 = moltis_providers::ModelInfo {
            id: "anthropic::claude-sonnet-4-5-20250929".into(),
            provider: "anthropic".into(),
            display_name: "Claude Sonnet 4.5".into(),
            created_at: None,
        };

        let order = LiveModelService::build_priority_order(&[]);
        let ordered = LiveModelService::prioritize_models(&order, vec![&m1, &m2, &m3].into_iter());
        assert_eq!(ordered[0].id, m2.id);
        assert_eq!(ordered[1].id, m1.id);
        assert_eq!(ordered[2].id, m3.id);
    }

    #[test]
    fn explicit_priority_still_overrides_subscription_preference() {
        let m1 = moltis_providers::ModelInfo {
            id: "openai::gpt-5.2".into(),
            provider: "openai".into(),
            display_name: "GPT-5.2".into(),
            created_at: None,
        };
        let m2 = moltis_providers::ModelInfo {
            id: "openai-codex::gpt-5.2".into(),
            provider: "openai-codex".into(),
            display_name: "GPT-5.2".into(),
            created_at: None,
        };

        let order = LiveModelService::build_priority_order(&["openai::gpt-5.2".into()]);
        let ordered = LiveModelService::prioritize_models(&order, vec![&m2, &m1].into_iter());
        assert_eq!(ordered[0].id, m1.id);
        assert_eq!(ordered[1].id, m2.id);
    }

    #[test]
    fn allowed_models_filters_by_substring_match() {
        let m1 = moltis_providers::ModelInfo {
            id: "anthropic::claude-opus-4-5".into(),
            provider: "anthropic".into(),
            display_name: "Claude Opus 4.5".into(),
            created_at: None,
        };
        let m2 = moltis_providers::ModelInfo {
            id: "openai-codex::gpt-5.2".into(),
            provider: "openai-codex".into(),
            display_name: "GPT 5.2".into(),
            created_at: None,
        };
        let m3 = moltis_providers::ModelInfo {
            id: "google::gemini-3-flash".into(),
            provider: "google".into(),
            display_name: "Gemini 3 Flash".into(),
            created_at: None,
        };

        let patterns: Vec<String> = vec!["opus".into()];
        assert!(model_matches_allowlist(&m1, &patterns));
        assert!(!model_matches_allowlist(&m2, &patterns));
        assert!(!model_matches_allowlist(&m3, &patterns));
    }

    #[test]
    fn allowed_models_empty_shows_all() {
        let m = moltis_providers::ModelInfo {
            id: "anthropic::claude-opus-4-5".into(),
            provider: "anthropic".into(),
            display_name: "Claude Opus 4.5".into(),
            created_at: None,
        };
        assert!(model_matches_allowlist(&m, &[]));
    }

    #[test]
    fn allowed_models_case_insensitive() {
        let m = moltis_providers::ModelInfo {
            id: "anthropic::claude-opus-4-5".into(),
            provider: "anthropic".into(),
            display_name: "Claude Opus 4.5".into(),
            created_at: None,
        };

        // Uppercase pattern matches lowercase model key.
        let patterns = vec![normalize_model_key("OPUS")];
        assert!(model_matches_allowlist(&m, &patterns));

        // Mixed case.
        let patterns = vec![normalize_model_key("OpUs")];
        assert!(model_matches_allowlist(&m, &patterns));
    }

    #[test]
    fn allowed_models_match_separator_variants() {
        let m = moltis_providers::ModelInfo {
            id: "openai-codex::gpt-5.2".into(),
            provider: "openai-codex".into(),
            display_name: "GPT-5.2".into(),
            created_at: None,
        };

        let patterns = vec![normalize_model_key("gpt 5.2")];
        assert!(model_matches_allowlist(&m, &patterns));

        let patterns = vec![normalize_model_key("gpt-5-2")];
        assert!(model_matches_allowlist(&m, &patterns));
    }

    #[test]
    fn allowed_models_numeric_pattern_does_not_match_extended_variants() {
        let exact = moltis_providers::ModelInfo {
            id: "openai::gpt-5.2".into(),
            provider: "openai".into(),
            display_name: "GPT-5.2".into(),
            created_at: None,
        };
        let extended = moltis_providers::ModelInfo {
            id: "openai::gpt-5.2-chat-latest".into(),
            provider: "openai".into(),
            display_name: "GPT-5.2 Chat Latest".into(),
            created_at: None,
        };
        let patterns = vec![normalize_model_key("gpt 5.2")];

        assert!(model_matches_allowlist(&exact, &patterns));
        assert!(!model_matches_allowlist(&extended, &patterns));
    }

    #[test]
    fn allowed_models_numeric_pattern_matches_provider_prefixed_models() {
        let m = moltis_providers::ModelInfo {
            id: "anthropic::claude-sonnet-4-5".into(),
            provider: "anthropic".into(),
            display_name: "Claude Sonnet 4.5".into(),
            created_at: None,
        };
        let patterns = vec![normalize_model_key("sonnet 4.5")];

        assert!(model_matches_allowlist(&m, &patterns));
    }

    #[test]
    fn allowed_models_does_not_filter_local_llm_or_ollama() {
        let local = moltis_providers::ModelInfo {
            id: "local-llm::qwen2.5-coder-7b-q4_k_m".into(),
            provider: "local-llm".into(),
            display_name: "Qwen2.5 Coder 7B".into(),
            created_at: None,
        };
        let ollama = moltis_providers::ModelInfo {
            id: "ollama::llama3.1:8b".into(),
            provider: "ollama".into(),
            display_name: "Llama 3.1 8B".into(),
            created_at: None,
        };
        let patterns = vec![normalize_model_key("opus")];

        assert!(model_matches_allowlist(&local, &patterns));
        assert!(model_matches_allowlist(&ollama, &patterns));
    }

    #[test]
    fn allowed_models_does_not_filter_ollama_when_provider_is_aliased() {
        let aliased = moltis_providers::ModelInfo {
            id: "local-ai::llama3.1:8b".into(),
            provider: "local-ai".into(),
            display_name: "Llama 3.1 8B".into(),
            created_at: None,
        };
        let patterns = vec![normalize_model_key("opus")];

        assert!(model_matches_allowlist_with_provider(
            &aliased,
            Some("ollama"),
            &patterns
        ));
    }

    #[tokio::test]
    async fn list_and_list_all_return_all_registered_models() {
        let mut registry = ProviderRegistry::empty();
        registry.register(
            moltis_providers::ModelInfo {
                id: "anthropic::claude-opus-4-5".to_string(),
                provider: "anthropic".to_string(),
                display_name: "Claude Opus 4.5".to_string(),
                created_at: None,
            },
            Arc::new(StaticProvider {
                name: "anthropic".to_string(),
                id: "anthropic::claude-opus-4-5".to_string(),
            }),
        );
        registry.register(
            moltis_providers::ModelInfo {
                id: "openai-codex::gpt-5.2".to_string(),
                provider: "openai-codex".to_string(),
                display_name: "GPT 5.2".to_string(),
                created_at: None,
            },
            Arc::new(StaticProvider {
                name: "openai-codex".to_string(),
                id: "openai-codex::gpt-5.2".to_string(),
            }),
        );
        registry.register(
            moltis_providers::ModelInfo {
                id: "google::gemini-3-flash".to_string(),
                provider: "google".to_string(),
                display_name: "Gemini 3 Flash".to_string(),
                created_at: None,
            },
            Arc::new(StaticProvider {
                name: "google".to_string(),
                id: "google::gemini-3-flash".to_string(),
            }),
        );

        let disabled = Arc::new(RwLock::new(DisabledModelsStore::default()));
        let service = LiveModelService::new(Arc::new(RwLock::new(registry)), disabled, vec![]);

        let result = service.list().await.unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 3);

        let result = service.list_all().await.unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 3);
    }

    #[tokio::test]
    async fn list_includes_created_at_in_response() {
        let mut registry = ProviderRegistry::empty();
        registry.register(
            moltis_providers::ModelInfo {
                id: "openai::gpt-5.3".to_string(),
                provider: "openai".to_string(),
                display_name: "GPT-5.3".to_string(),
                created_at: Some(1700000000),
            },
            Arc::new(StaticProvider {
                name: "openai".to_string(),
                id: "openai::gpt-5.3".to_string(),
            }),
        );
        registry.register(
            moltis_providers::ModelInfo {
                id: "openai::babbage-002".to_string(),
                provider: "openai".to_string(),
                display_name: "babbage-002".to_string(),
                created_at: Some(1600000000),
            },
            Arc::new(StaticProvider {
                name: "openai".to_string(),
                id: "openai::babbage-002".to_string(),
            }),
        );
        registry.register(
            moltis_providers::ModelInfo {
                id: "anthropic::claude-opus".to_string(),
                provider: "anthropic".to_string(),
                display_name: "Claude Opus".to_string(),
                created_at: None,
            },
            Arc::new(StaticProvider {
                name: "anthropic".to_string(),
                id: "anthropic::claude-opus".to_string(),
            }),
        );

        let disabled = Arc::new(RwLock::new(DisabledModelsStore::default()));
        let service = LiveModelService::new(Arc::new(RwLock::new(registry)), disabled, vec![]);

        let result = service.list().await.unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 3);

        // Verify createdAt is present and correct.
        let gpt = arr.iter().find(|m| m["id"] == "openai::gpt-5.3").unwrap();
        assert_eq!(gpt["createdAt"], 1700000000);

        let babbage = arr
            .iter()
            .find(|m| m["id"] == "openai::babbage-002")
            .unwrap();
        assert_eq!(babbage["createdAt"], 1600000000);

        let claude = arr
            .iter()
            .find(|m| m["id"] == "anthropic::claude-opus")
            .unwrap();
        assert!(claude["createdAt"].is_null());

        // Also verify list_all includes createdAt.
        let result_all = service.list_all().await.unwrap();
        let arr_all = result_all.as_array().unwrap();
        let gpt_all = arr_all
            .iter()
            .find(|m| m["id"] == "openai::gpt-5.3")
            .unwrap();
        assert_eq!(gpt_all["createdAt"], 1700000000);
    }

    #[tokio::test]
    async fn list_includes_ollama_when_provider_is_aliased() {
        let mut registry = ProviderRegistry::empty();
        registry.register(
            moltis_providers::ModelInfo {
                id: "openai-codex::gpt-5.2".to_string(),
                provider: "openai-codex".to_string(),
                display_name: "GPT 5.2".to_string(),
                created_at: None,
            },
            Arc::new(StaticProvider {
                name: "openai-codex".to_string(),
                id: "openai-codex::gpt-5.2".to_string(),
            }),
        );
        registry.register(
            moltis_providers::ModelInfo {
                id: "local-ai::llama3.1:8b".to_string(),
                provider: "local-ai".to_string(),
                display_name: "Llama 3.1 8B".to_string(),
                created_at: None,
            },
            Arc::new(StaticProvider {
                name: "ollama".to_string(),
                id: "local-ai::llama3.1:8b".to_string(),
            }),
        );

        let disabled = Arc::new(RwLock::new(DisabledModelsStore::default()));
        let service = LiveModelService::new(Arc::new(RwLock::new(registry)), disabled, vec![]);

        let result = service.list().await.unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert!(
            arr.iter()
                .any(|m| m.get("id").and_then(|v| v.as_str()) == Some("local-ai::llama3.1:8b"))
        );

        let result = service.list_all().await.unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert!(
            arr.iter()
                .any(|m| m.get("id").and_then(|v| v.as_str()) == Some("local-ai::llama3.1:8b"))
        );
    }

    #[test]
    fn provider_filter_is_normalized_and_ignores_empty() {
        let params = serde_json::json!({"provider": "  OpenAI-CODEX "});
        assert_eq!(
            provider_filter_from_params(&params).as_deref(),
            Some("openai-codex")
        );
        assert!(provider_filter_from_params(&serde_json::json!({"provider": "   "})).is_none());
    }

    #[test]
    fn provider_matches_filter_is_case_insensitive() {
        assert!(provider_matches_filter(
            "openai-codex",
            Some("openai-codex")
        ));
        assert!(provider_matches_filter(
            "OpenAI-Codex",
            Some("openai-codex")
        ));
        assert!(!provider_matches_filter(
            "github-copilot",
            Some("openai-codex")
        ));
        assert!(provider_matches_filter("github-copilot", None));
    }

    #[test]
    fn push_provider_model_groups_models_by_provider() {
        let mut grouped: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        push_provider_model(
            &mut grouped,
            "openai-codex",
            "openai-codex::gpt-5.2",
            "GPT-5.2",
        );
        push_provider_model(
            &mut grouped,
            "openai-codex",
            "openai-codex::gpt-5.1-codex-mini",
            "GPT-5.1 Codex Mini",
        );
        push_provider_model(
            &mut grouped,
            "anthropic",
            "anthropic::claude-sonnet-4-5-20250929",
            "Claude Sonnet 4.5",
        );

        let openai = grouped.get("openai-codex").expect("openai group exists");
        assert_eq!(openai.len(), 2);
        assert_eq!(openai[0]["modelId"], "openai-codex::gpt-5.2");
        assert_eq!(openai[1]["modelId"], "openai-codex::gpt-5.1-codex-mini");

        let anthropic = grouped.get("anthropic").expect("anthropic group exists");
        assert_eq!(anthropic.len(), 1);
        assert_eq!(
            anthropic[0]["modelId"],
            "anthropic::claude-sonnet-4-5-20250929"
        );
    }

    #[tokio::test]
    async fn list_all_includes_disabled_models_and_list_hides_them() {
        let mut registry = ProviderRegistry::empty();
        registry.register(
            moltis_providers::ModelInfo {
                id: "unit-test-model".to_string(),
                provider: "unit-test-provider".to_string(),
                display_name: "Unit Test Model".to_string(),
                created_at: None,
            },
            Arc::new(StaticProvider {
                name: "unit-test-provider".to_string(),
                id: "unit-test-model".to_string(),
            }),
        );

        let disabled = Arc::new(RwLock::new(DisabledModelsStore::default()));
        {
            let mut store = disabled.write().await;
            store.disable("unit-test-provider::unit-test-model");
        }

        let service = LiveModelService::new(Arc::new(RwLock::new(registry)), disabled, vec![]);

        let all = service
            .list_all()
            .await
            .expect("models.list_all should succeed");
        let all_models = all
            .as_array()
            .expect("models.list_all should return an array");
        let all_entry = all_models
            .iter()
            .find(|m| {
                m.get("id").and_then(|v| v.as_str()) == Some("unit-test-provider::unit-test-model")
            })
            .expect("disabled model should still appear in models.list_all");
        assert_eq!(
            all_entry.get("disabled").and_then(|v| v.as_bool()),
            Some(true)
        );

        let visible = service.list().await.expect("models.list should succeed");
        let visible_models = visible
            .as_array()
            .expect("models.list should return an array");
        assert!(
            visible_models
                .iter()
                .all(|m| m.get("id").and_then(|v| v.as_str())
                    != Some("unit-test-provider::unit-test-model")),
            "disabled model should be hidden from models.list",
        );
    }

    #[test]
    fn probe_rate_limit_detection_matches_copilot_429_pattern() {
        let raw = "github-copilot API error status=429 Too Many Requests body=quota exceeded";
        let error_obj = parse_chat_error(raw, Some("github-copilot"));
        assert!(is_probe_rate_limited_error(&error_obj, raw));
        assert_ne!(error_obj["type"], "unsupported_model");
    }

    #[test]
    fn probe_rate_limit_backoff_doubles_and_caps() {
        assert_eq!(next_probe_rate_limit_backoff_ms(None), 1_000);
        assert_eq!(next_probe_rate_limit_backoff_ms(Some(1_000)), 2_000);
        assert_eq!(next_probe_rate_limit_backoff_ms(Some(20_000)), 30_000);
        assert_eq!(next_probe_rate_limit_backoff_ms(Some(30_000)), 30_000);
    }

    #[tokio::test]
    async fn model_test_rejects_missing_model_id() {
        let service = LiveModelService::new(
            Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
                &moltis_config::schema::ProvidersConfig::default(),
            ))),
            Arc::new(RwLock::new(DisabledModelsStore::default())),
            vec![],
        );
        let result = service.test(serde_json::json!({})).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("missing 'modelId'")
        );
    }

    #[tokio::test]
    async fn model_test_rejects_unknown_model() {
        let service = LiveModelService::new(
            Arc::new(RwLock::new(ProviderRegistry::from_env_with_config(
                &moltis_config::schema::ProvidersConfig::default(),
            ))),
            Arc::new(RwLock::new(DisabledModelsStore::default())),
            vec![],
        );
        let result = service
            .test(serde_json::json!({"modelId": "nonexistent::model-xyz"}))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown model"));
    }

    #[tokio::test]
    async fn model_test_unknown_model_includes_suggestion() {
        let mut registry = ProviderRegistry::empty();
        registry.register(
            moltis_providers::ModelInfo {
                id: "openai::gpt-5.2-codex".to_string(),
                provider: "openai".to_string(),
                display_name: "GPT 5.2 Codex".to_string(),
                created_at: None,
            },
            Arc::new(StaticProvider {
                name: "openai".to_string(),
                id: "openai::gpt-5.2-codex".to_string(),
            }),
        );
        registry.register(
            moltis_providers::ModelInfo {
                id: "openai::gpt-5".to_string(),
                provider: "openai".to_string(),
                display_name: "GPT 5".to_string(),
                created_at: None,
            },
            Arc::new(StaticProvider {
                name: "openai".to_string(),
                id: "openai::gpt-5".to_string(),
            }),
        );

        let service = LiveModelService::new(
            Arc::new(RwLock::new(registry)),
            Arc::new(RwLock::new(DisabledModelsStore::default())),
            vec![],
        );
        let result = service
            .test(serde_json::json!({"modelId": "openai::gpt-5.2"}))
            .await;

        assert!(result.is_err());
        let error = result.unwrap_err().to_string();
        assert!(error.contains("unknown model: openai::gpt-5.2"));
        assert!(error.contains("did you mean"));
        assert!(error.contains("openai::gpt-5.2-codex"));
    }

    #[test]
    fn suggest_model_ids_prefers_provider_and_similarity() {
        let available = vec![
            "openai::gpt-5.2-codex".to_string(),
            "openai::gpt-5".to_string(),
            "openai-codex::gpt-5.2-codex".to_string(),
            "anthropic::claude-sonnet-4".to_string(),
        ];

        let suggestions = suggest_model_ids("openai::gpt-5.2", &available, 3);

        assert!(!suggestions.is_empty());
        assert!(
            suggestions.iter().any(|id| id == "openai::gpt-5.2-codex"),
            "close provider-prefixed match should be included"
        );
        assert!(
            suggestions
                .iter()
                .all(|id| id != "anthropic::claude-sonnet-4"),
            "unrelated models should not be suggested"
        );
    }

    #[tokio::test]
    async fn model_test_returns_error_when_provider_fails() {
        let mut registry = ProviderRegistry::from_env_with_config(
            &moltis_config::schema::ProvidersConfig::default(),
        );
        // StaticProvider's complete() returns an error ("not implemented for test")
        registry.register(
            moltis_providers::ModelInfo {
                id: "test-provider::test-model".to_string(),
                provider: "test-provider".to_string(),
                display_name: "Test Model".to_string(),
                created_at: None,
            },
            Arc::new(StaticProvider {
                name: "test-provider".to_string(),
                id: "test-provider::test-model".to_string(),
            }),
        );

        let service = LiveModelService::new(
            Arc::new(RwLock::new(registry)),
            Arc::new(RwLock::new(DisabledModelsStore::default())),
            vec![],
        );
        let result = service
            .test(serde_json::json!({"modelId": "test-provider::test-model"}))
            .await;
        // StaticProvider.complete() returns Err, so test should return an error.
        assert!(result.is_err());
    }

    #[test]
    fn probe_parallel_per_provider_defaults_and_clamps() {
        assert_eq!(probe_max_parallel_per_provider(&serde_json::json!({})), 1);
        assert_eq!(
            probe_max_parallel_per_provider(&serde_json::json!({"maxParallelPerProvider": 1})),
            1
        );
        assert_eq!(
            probe_max_parallel_per_provider(&serde_json::json!({"maxParallelPerProvider": 99})),
            8
        );
    }

    // ── to_user_content tests ─────────────────────────────────────────

    #[test]
    fn to_user_content_text_only() {
        let mc = MessageContent::Text("hello".to_string());
        let uc = to_user_content(&mc);
        match uc {
            UserContent::Text(t) => assert_eq!(t, "hello"),
            _ => panic!("expected Text variant"),
        }
    }

    #[test]
    fn to_user_content_multimodal_with_image() {
        use moltis_sessions::message::{ContentBlock, ImageUrl as SessionImageUrl};

        let mc = MessageContent::Multimodal(vec![
            ContentBlock::Text {
                text: "describe this".to_string(),
            },
            ContentBlock::ImageUrl {
                image_url: SessionImageUrl {
                    url: "data:image/png;base64,AAAA".to_string(),
                },
            },
        ]);
        let uc = to_user_content(&mc);
        match uc {
            UserContent::Multimodal(parts) => {
                assert_eq!(parts.len(), 2);
                match &parts[0] {
                    ContentPart::Text(t) => assert_eq!(t, "describe this"),
                    _ => panic!("expected Text part"),
                }
                match &parts[1] {
                    ContentPart::Image { media_type, data } => {
                        assert_eq!(media_type, "image/png");
                        assert_eq!(data, "AAAA");
                    },
                    _ => panic!("expected Image part"),
                }
            },
            _ => panic!("expected Multimodal variant"),
        }
    }

    #[test]
    fn to_user_content_drops_invalid_data_uri() {
        use moltis_sessions::message::{ContentBlock, ImageUrl as SessionImageUrl};

        let mc = MessageContent::Multimodal(vec![
            ContentBlock::Text {
                text: "just text".to_string(),
            },
            ContentBlock::ImageUrl {
                image_url: SessionImageUrl {
                    url: "https://example.com/image.png".to_string(),
                },
            },
        ]);
        let uc = to_user_content(&mc);
        match uc {
            UserContent::Multimodal(parts) => {
                // The https URL is not a data URI, so it should be dropped
                assert_eq!(parts.len(), 1);
                match &parts[0] {
                    ContentPart::Text(t) => assert_eq!(t, "just text"),
                    _ => panic!("expected Text part"),
                }
            },
            _ => panic!("expected Multimodal variant"),
        }
    }

    // ── Logbook formatting tests ─────────────────────────────────────────

    #[test]
    fn format_logbook_html_empty_entries() {
        assert_eq!(format_logbook_html(&[]), "");
    }

    #[test]
    fn format_logbook_html_single_entry() {
        let entries = vec!["Using Claude Sonnet 4.5. Use /model to change.".to_string()];
        let html = format_logbook_html(&entries);
        assert!(html.starts_with("<blockquote expandable>"));
        assert!(html.ends_with("</blockquote>"));
        assert!(html.contains("\u{1f4cb} <b>Activity log</b>"));
        assert!(html.contains("\u{2022} Using Claude Sonnet 4.5. Use /model to change."));
    }

    #[test]
    fn format_logbook_html_multiple_entries() {
        let entries = vec![
            "Using Claude Sonnet 4.5. Use /model to change.".to_string(),
            "\u{1f50d} Searching: rust async patterns".to_string(),
            "\u{1f4bb} Running: `ls -la`".to_string(),
        ];
        let html = format_logbook_html(&entries);
        // Verify all entries are present as bullet points.
        for entry in &entries {
            let escaped = entry
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            assert!(
                html.contains(&format!("\u{2022} {escaped}")),
                "missing entry: {entry}"
            );
        }
    }

    #[test]
    fn format_logbook_html_escapes_html_entities() {
        let entries = vec!["Running: `echo <script>alert(1)</script>`".to_string()];
        let html = format_logbook_html(&entries);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    // ── Tool result formatting tests ────────────────────────────────────

    #[test]
    fn format_tool_result_exec_with_exit_code_and_stderr() {
        let result = Some(serde_json::json!({
            "exitCode": 1,
            "stderr": "error: file not found\nsecond line"
        }));
        let msg = format_tool_result_message("exec", &None, &result);
        assert_eq!(msg, "  ❌ exit 1 — error: file not found");
    }

    #[test]
    fn format_tool_result_exec_exit_code_no_stderr() {
        let result = Some(serde_json::json!({ "exitCode": 127 }));
        let msg = format_tool_result_message("exec", &None, &result);
        assert_eq!(msg, "  ❌ exit 127");
    }

    #[test]
    fn format_tool_result_exec_no_exit_code_uses_error() {
        let error = Some("command timed out".to_string());
        let msg = format_tool_result_message("exec", &error, &None);
        assert_eq!(msg, "  ❌ command timed out");
    }

    #[test]
    fn format_tool_result_browser_error() {
        let error = Some("Navigation failed: net::ERR_NAME_NOT_RESOLVED".to_string());
        let msg = format_tool_result_message("browser", &error, &None);
        assert_eq!(msg, "  ❌ Navigation failed: net::ERR_NAME_NOT_RESOLVED");
    }

    #[test]
    fn format_tool_result_no_error_fallback() {
        let msg = format_tool_result_message("web_fetch", &None, &None);
        assert_eq!(msg, "  ❌ failed");
    }

    #[test]
    fn extract_location_from_show_map_result() {
        let result = serde_json::json!({
            "latitude": 37.76,
            "longitude": -122.42,
            "label": "La Taqueria",
            "screenshot": "data:image/png;base64,abc",
            "map_links": {}
        });

        // Extraction logic mirrors the ToolCallEnd handler
        let extracted = result
            .get("latitude")
            .and_then(|v| v.as_f64())
            .and_then(|lat| {
                let lon = result.get("longitude")?.as_f64()?;
                let label = result
                    .get("label")
                    .and_then(|l| l.as_str())
                    .map(String::from);
                Some((lat, lon, label))
            });

        let (lat, lon, label) = extracted.unwrap();
        assert!((lat - 37.76).abs() < f64::EPSILON);
        assert!((lon - (-122.42)).abs() < f64::EPSILON);
        assert_eq!(label.as_deref(), Some("La Taqueria"));
    }

    #[test]
    fn extract_location_without_label() {
        let result = serde_json::json!({
            "latitude": 48.8566,
            "longitude": 2.3522,
            "screenshot": "data:image/png;base64,abc"
        });

        let extracted = result
            .get("latitude")
            .and_then(|v| v.as_f64())
            .and_then(|lat| {
                let lon = result.get("longitude")?.as_f64()?;
                let label = result
                    .get("label")
                    .and_then(|l| l.as_str())
                    .map(String::from);
                Some((lat, lon, label))
            });

        let (lat, lon, label) = extracted.unwrap();
        assert!((lat - 48.8566).abs() < f64::EPSILON);
        assert!((lon - 2.3522).abs() < f64::EPSILON);
        assert!(label.is_none());
    }

    #[test]
    fn extract_location_missing_coords_returns_none() {
        let result = serde_json::json!({
            "screenshot": "data:image/png;base64,abc"
        });

        let extracted = result
            .get("latitude")
            .and_then(|v| v.as_f64())
            .and_then(|_lat| {
                let _lon = result.get("longitude")?.as_f64()?;
                Some(())
            });

        assert!(extracted.is_none());
    }

    #[test]
    fn resolve_agent_memory_target_path_maps_to_agent_workspace() {
        let workspace = moltis_config::agent_workspace_dir("ops");
        assert_eq!(
            resolve_agent_memory_target_path("ops", "MEMORY.md").unwrap(),
            workspace.join("MEMORY.md")
        );
        assert_eq!(
            resolve_agent_memory_target_path("ops", "memory/daily.md").unwrap(),
            workspace.join("memory").join("daily.md")
        );
    }

    #[test]
    fn resolve_agent_memory_target_path_rejects_invalid_paths() {
        assert!(resolve_agent_memory_target_path("ops", "").is_err());
        assert!(resolve_agent_memory_target_path("ops", "foo.md").is_err());
        assert!(resolve_agent_memory_target_path("ops", "memory/a/b.md").is_err());
        assert!(resolve_agent_memory_target_path("ops", "memory/.hidden.md").is_err());
    }

    #[test]
    fn path_in_agent_memory_scope_is_isolated_per_agent() {
        let ops_workspace = moltis_config::agent_workspace_dir("ops");
        let ops_memory = ops_workspace.join("memory").join("daily.md");
        assert!(is_path_in_agent_memory_scope(&ops_memory, "ops"));
        assert!(!is_path_in_agent_memory_scope(&ops_memory, "research"));

        let root_memory = moltis_config::data_dir().join("memory").join("root.md");
        assert!(is_path_in_agent_memory_scope(&root_memory, "main"));
        assert!(!is_path_in_agent_memory_scope(&root_memory, "ops"));
    }

    // ── active_session_keys tests ───────────────────────────────────────

    #[tokio::test]
    async fn active_session_keys_empty_when_no_runs() {
        let (_active_runs, active_runs_by_session) = make_active_run_maps();
        let keys: Vec<String> = active_runs_by_session
            .read()
            .await
            .keys()
            .cloned()
            .collect();
        assert!(keys.is_empty());
    }

    #[tokio::test]
    async fn active_session_keys_returns_running_sessions() {
        let (_active_runs, active_runs_by_session) = make_active_run_maps();
        active_runs_by_session
            .write()
            .await
            .insert("session-a".to_string(), "run-1".to_string());
        active_runs_by_session
            .write()
            .await
            .insert("session-b".to_string(), "run-2".to_string());

        let mut keys: Vec<String> = active_runs_by_session
            .read()
            .await
            .keys()
            .cloned()
            .collect();
        keys.sort();
        assert_eq!(keys, vec!["session-a", "session-b"]);
    }

    #[test]
    fn active_tool_call_serializes_with_camel_case() {
        let tc = ActiveToolCall {
            id: "tc_1".into(),
            name: "bash".into(),
            arguments: serde_json::json!({"command": "ls"}),
            started_at: 1700000000000,
        };
        let json = serde_json::to_value(&tc).unwrap();
        assert_eq!(json["id"], "tc_1");
        assert_eq!(json["name"], "bash");
        assert_eq!(json["startedAt"], 1700000000000_u64);
        assert!(json.get("started_at").is_none());
    }

    #[tokio::test]
    async fn peek_returns_inactive_when_no_run() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = SessionStore::new(dir.path().to_path_buf());
        let pool = sqlite_pool().await;
        let metadata = SqliteSessionMetadata::new(pool);

        let service = LiveChatService::new(
            Arc::new(RwLock::new(ProviderRegistry::empty())),
            Arc::new(RwLock::new(DisabledModelsStore::default())),
            Arc::new(MockChatRuntime::new()),
            Arc::new(store),
            Arc::new(metadata),
        );

        let result = service
            .peek(serde_json::json!({ "sessionKey": "main" }))
            .await
            .unwrap();
        assert_eq!(result["active"], false);
    }

    #[tokio::test]
    async fn abort_clears_active_tool_calls() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = SessionStore::new(dir.path().to_path_buf());
        let pool = sqlite_pool().await;
        let metadata = SqliteSessionMetadata::new(pool);

        let service = LiveChatService::new(
            Arc::new(RwLock::new(ProviderRegistry::empty())),
            Arc::new(RwLock::new(DisabledModelsStore::default())),
            Arc::new(MockChatRuntime::new()),
            Arc::new(store),
            Arc::new(metadata),
        );

        // Pre-populate active tool calls for a session.
        service
            .active_tool_calls
            .write()
            .await
            .insert("test-session".into(), vec![ActiveToolCall {
                id: "tc_1".into(),
                name: "bash".into(),
                arguments: serde_json::json!({}),
                started_at: 0,
            }]);
        // Pre-populate active_runs_by_session so abort can find the session.
        let run_id = "test-run".to_string();
        service
            .active_runs_by_session
            .write()
            .await
            .insert("test-session".into(), run_id.clone());
        // Pre-populate active_runs with a dummy task handle.
        let handle = tokio::spawn(async { tokio::time::sleep(Duration::from_secs(60)).await });
        service
            .active_runs
            .write()
            .await
            .insert(run_id.clone(), handle.abort_handle());

        let result = service
            .abort(serde_json::json!({ "sessionKey": "test-session" }))
            .await
            .unwrap();
        assert_eq!(result["aborted"], true);

        // Tool calls should be cleaned up.
        assert!(
            service
                .active_tool_calls
                .read()
                .await
                .get("test-session")
                .is_none()
        );
    }

    // ── effective_tool_mode tests ───────────────────────────────────────

    /// Provider stub for testing `effective_tool_mode()` with configurable
    /// `supports_tools` and `tool_mode` values.
    struct ToolModeTestProvider {
        native: bool,
        mode: Option<ToolMode>,
    }

    #[async_trait]
    impl LlmProvider for ToolModeTestProvider {
        fn name(&self) -> &str {
            "test"
        }

        fn id(&self) -> &str {
            "test-model"
        }

        fn supports_tools(&self) -> bool {
            self.native
        }

        fn tool_mode(&self) -> Option<ToolMode> {
            self.mode
        }

        async fn complete(
            &self,
            _messages: &[ChatMessage],
            _tools: &[Value],
        ) -> Result<moltis_agents::model::CompletionResponse> {
            anyhow::bail!("not implemented")
        }

        fn stream(
            &self,
            _messages: Vec<ChatMessage>,
        ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
            Box::pin(tokio_stream::empty())
        }
    }

    #[test]
    fn effective_tool_mode_native_when_supported_and_auto() {
        let p = ToolModeTestProvider {
            native: true,
            mode: None,
        };
        assert_eq!(effective_tool_mode(&p), ToolMode::Native);
    }

    #[test]
    fn effective_tool_mode_text_when_not_supported_and_auto() {
        let p = ToolModeTestProvider {
            native: false,
            mode: None,
        };
        assert_eq!(effective_tool_mode(&p), ToolMode::Text);
    }

    #[test]
    fn effective_tool_mode_respects_explicit_native() {
        let p = ToolModeTestProvider {
            native: false,
            mode: Some(ToolMode::Native),
        };
        assert_eq!(effective_tool_mode(&p), ToolMode::Native);
    }

    #[test]
    fn effective_tool_mode_respects_explicit_text() {
        let p = ToolModeTestProvider {
            native: true,
            mode: Some(ToolMode::Text),
        };
        assert_eq!(effective_tool_mode(&p), ToolMode::Text);
    }

    #[test]
    fn effective_tool_mode_respects_explicit_off() {
        let p = ToolModeTestProvider {
            native: true,
            mode: Some(ToolMode::Off),
        };
        assert_eq!(effective_tool_mode(&p), ToolMode::Off);
    }

    #[test]
    fn effective_tool_mode_auto_explicit_delegates_to_supports_tools() {
        // Explicit Auto should behave same as None.
        let native = ToolModeTestProvider {
            native: true,
            mode: Some(ToolMode::Auto),
        };
        assert_eq!(effective_tool_mode(&native), ToolMode::Native);

        let text = ToolModeTestProvider {
            native: false,
            mode: Some(ToolMode::Auto),
        };
        assert_eq!(effective_tool_mode(&text), ToolMode::Text);
    }
}
