use std::sync::Arc;

use {
    async_trait::async_trait,
    base64::{Engine, engine::general_purpose},
    serde_json::Value,
    tracing::{info, warn},
};

use {
    moltis_common::hooks::HookRegistry,
    moltis_projects::ProjectStore,
    moltis_sessions::{
        message::PersistedMessage, metadata::SqliteSessionMetadata, state_store::SessionStateStore,
        store::SessionStore,
    },
    moltis_tools::sandbox::SandboxRouter,
};

use crate::{
    services::{ServiceResult, SessionService},
    share_store::{
        ShareSnapshot, ShareStore, ShareVisibility, SharedImageAsset, SharedImageSet,
        SharedMapLinks, SharedMessage, SharedMessageRole,
    },
};

const SHARE_BOUNDARY_NOTICE: &str =
    "This session until here has been shared. Later messages are not included in the shared link.";
const SHARE_PREVIEW_MAX_IMAGE_WIDTH: u32 = 430;
const SHARE_PREVIEW_MAX_IMAGE_HEIGHT: u32 = 430;
const SHARE_REDACTED_VALUE: &str = "[REDACTED]";

/// Filter out empty assistant messages from history before sending to the UI.
///
/// Empty assistant messages are persisted in the session JSONL for LLM history
/// coherence (so the model sees a complete user→assistant turn), but they
/// should not be shown in the web UI or sent to channels.
fn filter_ui_history(messages: Vec<Value>) -> Vec<Value> {
    messages
        .into_iter()
        .filter(|msg| {
            if msg.get("role").and_then(|v| v.as_str()) != Some("assistant") {
                return true;
            }
            // Keep assistant messages that have non-empty content.
            msg.get("content")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.trim().is_empty())
        })
        .collect()
}

/// Extract text content from a single message Value.
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

/// Truncate a string to `max` chars, appending "…" if truncated.
fn truncate_preview(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..s.floor_char_boundary(max)])
    }
}

/// Extract preview from a single message (used for first-message preview in chat).
pub(crate) fn extract_preview_from_value(msg: &Value) -> Option<String> {
    message_text(msg).map(|t| truncate_preview(&t, 200))
}

/// Build a preview by combining user and assistant messages until we
/// have enough text (target ~80 chars). Skips tool_result messages.
fn extract_preview(history: &[Value]) -> Option<String> {
    const TARGET: usize = 80;
    const MAX: usize = 200;

    let mut combined = String::new();
    for msg in history {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if role != "user" && role != "assistant" {
            continue;
        }
        let Some(text) = message_text(msg) else {
            continue;
        };
        if !combined.is_empty() {
            combined.push_str(" — ");
        }
        combined.push_str(&text);
        if combined.len() >= TARGET {
            break;
        }
    }
    if combined.is_empty() {
        return None;
    }
    Some(truncate_preview(&combined, MAX))
}

fn value_u64(msg: &Value, key: &str) -> Option<u64> {
    msg.get(key).and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_i64().and_then(|n| (n >= 0).then_some(n as u64)))
    })
}

fn message_text_for_share(msg: &Value) -> Option<String> {
    if let Some(s) = msg.get("content").and_then(|v| v.as_str()) {
        let trimmed = s.trim();
        return (!trimmed.is_empty()).then(|| trimmed.to_string());
    }

    let blocks = msg.get("content").and_then(|v| v.as_array())?;
    let joined = blocks
        .iter()
        .filter_map(|block| {
            if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                block.get("text").and_then(|v| v.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let trimmed = joined.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn media_filename(path: &str) -> Option<&str> {
    let filename = path.rsplit('/').next()?.trim();
    (!filename.is_empty()).then_some(filename)
}

fn audio_mime_type(filename: &str) -> &'static str {
    match filename.rsplit('.').next().unwrap_or_default() {
        "ogg" | "opus" => "audio/ogg",
        "webm" => "audio/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "aac" => "audio/aac",
        "m4a" => "audio/mp4",
        "flac" => "audio/flac",
        _ => "application/octet-stream",
    }
}

fn image_mime_type(filename: &str) -> &'static str {
    match filename.rsplit('.').next().unwrap_or_default() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" | "svgz" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

fn sniff_image_mime(bytes: &[u8], fallback: &str) -> String {
    if bytes.len() >= 8 && bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return "image/png".to_string();
    }
    if bytes.len() >= 3 && bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return "image/jpeg".to_string();
    }
    if bytes.len() >= 6 && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return "image/gif".to_string();
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return "image/webp".to_string();
    }
    fallback.to_string()
}

fn build_image_data_url(mime: &str, bytes: &[u8]) -> String {
    let encoded = general_purpose::STANDARD.encode(bytes);
    format!("data:{mime};base64,{encoded}")
}

fn parse_base64_image_data_url(data_url: &str) -> Option<(String, Vec<u8>)> {
    let (meta, body) = data_url.split_once(',')?;
    if !meta.starts_with("data:image/") || !meta.contains(";base64") {
        return None;
    }
    let mime = meta
        .trim_start_matches("data:")
        .split(';')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_string();
    let decoded = general_purpose::STANDARD.decode(body.trim()).ok()?;
    Some((mime, decoded))
}

async fn message_audio_data_url_for_share(
    msg: &Value,
    session_key: &str,
    store: &SessionStore,
) -> Option<String> {
    let audio_path = msg.get("audio").and_then(|v| v.as_str())?;
    let filename = media_filename(audio_path)?;
    let bytes = store.read_media(session_key, filename).await.ok()?;
    let encoded = general_purpose::STANDARD.encode(bytes);
    Some(format!(
        "data:{};base64,{}",
        audio_mime_type(filename),
        encoded
    ))
}

async fn tool_result_image_for_share(
    msg: &Value,
    session_key: &str,
    store: &SessionStore,
) -> Option<SharedImageSet> {
    let screenshot = msg
        .get("result")
        .and_then(|v| v.get("screenshot"))
        .and_then(|v| v.as_str())
        .map(str::trim)?;
    let (full_mime, full_bytes) = if screenshot.starts_with("data:image/") {
        parse_base64_image_data_url(screenshot)?
    } else {
        let filename = media_filename(screenshot)?;
        let bytes = store.read_media(session_key, filename).await.ok()?;
        (image_mime_type(filename).to_string(), bytes)
    };

    let full_meta = moltis_media::image_ops::get_image_metadata(&full_bytes).ok()?;
    let full_asset = SharedImageAsset {
        data_url: build_image_data_url(&full_mime, &full_bytes),
        width: full_meta.width,
        height: full_meta.height,
    };

    let needs_preview_resize = full_meta.width > SHARE_PREVIEW_MAX_IMAGE_WIDTH
        || full_meta.height > SHARE_PREVIEW_MAX_IMAGE_HEIGHT;
    let preview_bytes = if needs_preview_resize {
        moltis_media::image_ops::resize_image(
            &full_bytes,
            SHARE_PREVIEW_MAX_IMAGE_WIDTH,
            SHARE_PREVIEW_MAX_IMAGE_HEIGHT,
        )
        .unwrap_or_else(|_| full_bytes.clone())
    } else {
        full_bytes.clone()
    };
    let preview_meta = moltis_media::image_ops::get_image_metadata(&preview_bytes).ok()?;
    let preview_mime = sniff_image_mime(&preview_bytes, &full_mime);
    let preview_asset = SharedImageAsset {
        data_url: build_image_data_url(&preview_mime, &preview_bytes),
        width: preview_meta.width,
        height: preview_meta.height,
    };
    let full = if preview_asset.data_url == full_asset.data_url {
        None
    } else {
        Some(full_asset)
    };

    Some(SharedImageSet {
        preview: preview_asset,
        full,
    })
}

fn sanitize_share_url(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url.trim()).ok()?;
    match parsed.scheme() {
        "http" | "https" => Some(parsed.into()),
        _ => None,
    }
}

fn is_assignment_key_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'"' | b'\'' | b'$')
}

fn is_assignment_value_delimiter(byte: u8) -> bool {
    byte.is_ascii_whitespace()
        || matches!(byte, b'&' | b',' | b';' | b')' | b']' | b'}' | b'"' | b'\'')
}

fn normalize_assignment_key(key: &str) -> String {
    key.trim()
        .trim_matches(|ch| ch == '"' || ch == '\'')
        .trim_start_matches('$')
        .trim_start_matches('-')
        .to_ascii_lowercase()
}

fn is_env_var_key(key: &str) -> bool {
    let trimmed = key
        .trim()
        .trim_matches(|ch| ch == '"' || ch == '\'')
        .trim_start_matches('$');
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_uppercase()) {
        return false;
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_uppercase() || ch.is_ascii_digit())
}

fn is_sensitive_assignment_key(key: &str) -> bool {
    let normalized = normalize_assignment_key(key);
    if normalized.is_empty() {
        return false;
    }

    let compact: String = normalized
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect();
    if compact.is_empty() {
        return false;
    }

    matches!(compact.as_str(), "authorization" | "proxyauthorization")
        || compact.ends_with("apikey")
        || compact.ends_with("token")
        || compact.ends_with("secret")
        || compact.ends_with("password")
        || compact.ends_with("passwd")
}

fn should_redact_assignment_key(key: &str) -> bool {
    is_sensitive_assignment_key(key) || is_env_var_key(key)
}

fn starts_with_ignore_ascii_case(text: &str, start: usize, pattern: &str) -> bool {
    let end = start.saturating_add(pattern.len());
    text.get(start..end)
        .is_some_and(|value| value.eq_ignore_ascii_case(pattern))
}

fn assignment_key_bounds(text: &str, separator_idx: usize) -> Option<(usize, usize)> {
    if separator_idx == 0 || separator_idx >= text.len() {
        return None;
    }
    let bytes = text.as_bytes();
    let mut key_end = separator_idx;
    while key_end > 0 && bytes[key_end - 1].is_ascii_whitespace() {
        key_end -= 1;
    }
    if key_end == 0 {
        return None;
    }

    let mut key_start = key_end;
    while key_start > 0 && is_assignment_key_byte(bytes[key_start - 1]) {
        key_start -= 1;
    }
    (key_start < key_end).then_some((key_start, key_end))
}

fn assignment_value_bounds(text: &str, separator_idx: usize, key: &str) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    if separator_idx >= bytes.len() {
        return None;
    }

    let mut value_start = separator_idx + 1;
    while value_start < bytes.len() && bytes[value_start].is_ascii_whitespace() {
        value_start += 1;
    }
    if value_start >= bytes.len() {
        return None;
    }

    let normalized_key = normalize_assignment_key(key);
    let mut quoted = None;
    let mut redact_start = value_start;
    if matches!(bytes[value_start], b'"' | b'\'') {
        quoted = Some(bytes[value_start]);
        redact_start = value_start + 1;
    }
    if redact_start >= bytes.len() {
        return None;
    }

    if matches!(
        normalized_key.as_str(),
        "authorization" | "proxyauthorization"
    ) && starts_with_ignore_ascii_case(text, redact_start, "bearer ")
    {
        redact_start += "bearer ".len();
    }
    if redact_start >= bytes.len() {
        return None;
    }

    let mut value_end = redact_start;
    if let Some(quote_byte) = quoted {
        while value_end < bytes.len() && bytes[value_end] != quote_byte {
            value_end += 1;
        }
    } else {
        while value_end < bytes.len() && !is_assignment_value_delimiter(bytes[value_end]) {
            value_end += 1;
        }
    }
    (value_end > redact_start).then_some((redact_start, value_end))
}

fn redact_assignment_values(text: &str) -> String {
    let mut redacted = text.to_string();
    let mut idx = 0usize;

    while idx < redacted.len() {
        let next_separator = redacted.as_bytes()[idx..]
            .iter()
            .position(|byte| matches!(byte, b'=' | b':'))
            .map(|offset| idx + offset);
        let Some(separator_idx) = next_separator else {
            break;
        };

        let Some((key_start, key_end)) = assignment_key_bounds(&redacted, separator_idx) else {
            idx = separator_idx + 1;
            continue;
        };
        let key = redacted[key_start..key_end].trim();
        if !should_redact_assignment_key(key) {
            idx = separator_idx + 1;
            continue;
        }

        let Some((value_start, value_end)) = assignment_value_bounds(&redacted, separator_idx, key)
        else {
            idx = separator_idx + 1;
            continue;
        };
        if redacted[value_start..value_end].trim().is_empty()
            || &redacted[value_start..value_end] == SHARE_REDACTED_VALUE
        {
            idx = separator_idx + 1;
            continue;
        }

        redacted.replace_range(value_start..value_end, SHARE_REDACTED_VALUE);
        idx = value_start + SHARE_REDACTED_VALUE.len();
    }

    redacted
}

fn find_case_insensitive(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    if from >= haystack.len() {
        return None;
    }
    let needle_lower = needle.to_ascii_lowercase();
    let haystack_lower = haystack[from..].to_ascii_lowercase();
    haystack_lower
        .find(&needle_lower)
        .map(|offset| from + offset)
}

fn redact_bearer_tokens(text: &str) -> String {
    let mut redacted = text.to_string();
    let mut idx = 0usize;
    let needle = "bearer ";

    while let Some(start) = find_case_insensitive(&redacted, needle, idx) {
        let token_start = start + needle.len();
        if token_start >= redacted.len() {
            break;
        }
        if start > 0 && redacted.as_bytes()[start - 1].is_ascii_alphanumeric() {
            idx = token_start;
            continue;
        }

        let bytes = redacted.as_bytes();
        let mut token_end = token_start;
        while token_end < bytes.len() && !is_assignment_value_delimiter(bytes[token_end]) {
            token_end += 1;
        }
        if token_end <= token_start || &redacted[token_start..token_end] == SHARE_REDACTED_VALUE {
            idx = token_end.saturating_add(1);
            continue;
        }

        redacted.replace_range(token_start..token_end, SHARE_REDACTED_VALUE);
        idx = token_start + SHARE_REDACTED_VALUE.len();
    }

    redacted
}

fn redact_share_secret_values(text: &str) -> String {
    let with_assignments = redact_assignment_values(text);
    redact_bearer_tokens(&with_assignments)
}

fn tool_result_map_links_for_share(msg: &Value) -> Option<SharedMapLinks> {
    let map_links = msg
        .get("result")
        .and_then(|v| v.get("map_links"))
        .and_then(|v| v.as_object())?;

    let links = SharedMapLinks {
        apple_maps: map_links
            .get("apple_maps")
            .and_then(|v| v.as_str())
            .and_then(sanitize_share_url),
        google_maps: map_links
            .get("google_maps")
            .and_then(|v| v.as_str())
            .and_then(sanitize_share_url),
        openstreetmap: map_links
            .get("openstreetmap")
            .and_then(|v| v.as_str())
            .and_then(sanitize_share_url),
    };

    (links.apple_maps.is_some() || links.google_maps.is_some() || links.openstreetmap.is_some())
        .then_some(links)
}

fn tool_result_text_for_share(msg: &Value) -> Option<String> {
    let result = msg.get("result");
    let mut sections = Vec::new();

    if let Some(label) = result
        .and_then(|v| v.get("label"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|label| !label.is_empty())
    {
        sections.push(redact_share_secret_values(label));
    }
    if let Some(stdout) = result
        .and_then(|v| v.get("stdout"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|stdout| !stdout.is_empty())
    {
        sections.push(redact_share_secret_values(stdout));
    }
    if let Some(stderr) = result
        .and_then(|v| v.get("stderr"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|stderr| !stderr.is_empty())
    {
        sections.push(format!("stderr:\n{}", redact_share_secret_values(stderr)));
    }
    if let Some(error) = msg
        .get("error")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|error| !error.is_empty())
    {
        sections.push(format!("error: {}", redact_share_secret_values(error)));
    }
    if let Some(exit_code) = result
        .and_then(|v| v.get("exit_code"))
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_u64().and_then(|n| i64::try_from(n).ok()))
        })
        .filter(|exit_code| *exit_code != 0)
    {
        sections.push(format!("exit {exit_code}"));
    }

    let content = sections.join("\n\n");
    let trimmed = content.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

async fn to_shared_message(
    msg: &Value,
    session_key: &str,
    store: &SessionStore,
) -> Option<SharedMessage> {
    let role = match msg.get("role").and_then(|v| v.as_str()) {
        Some("user") => SharedMessageRole::User,
        Some("assistant") => SharedMessageRole::Assistant,
        Some("tool_result") => SharedMessageRole::ToolResult,
        _ => return None,
    };

    let content = match role {
        SharedMessageRole::ToolResult => tool_result_text_for_share(msg).unwrap_or_default(),
        SharedMessageRole::User | SharedMessageRole::Assistant => {
            message_text_for_share(msg).unwrap_or_default()
        },
        SharedMessageRole::System | SharedMessageRole::Notice => String::new(),
    };
    let audio_data_url = match role {
        SharedMessageRole::User | SharedMessageRole::Assistant => {
            message_audio_data_url_for_share(msg, session_key, store).await
        },
        SharedMessageRole::ToolResult | SharedMessageRole::System | SharedMessageRole::Notice => {
            None
        },
    };
    let image = match role {
        SharedMessageRole::ToolResult => tool_result_image_for_share(msg, session_key, store).await,
        SharedMessageRole::User
        | SharedMessageRole::Assistant
        | SharedMessageRole::System
        | SharedMessageRole::Notice => None,
    };
    let map_links = match role {
        SharedMessageRole::ToolResult => tool_result_map_links_for_share(msg),
        SharedMessageRole::User
        | SharedMessageRole::Assistant
        | SharedMessageRole::System
        | SharedMessageRole::Notice => None,
    };
    let tool_success = match role {
        SharedMessageRole::ToolResult => msg.get("success").and_then(|v| v.as_bool()),
        SharedMessageRole::User
        | SharedMessageRole::Assistant
        | SharedMessageRole::System
        | SharedMessageRole::Notice => None,
    };
    let tool_name = match role {
        SharedMessageRole::ToolResult => msg
            .get("tool_name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToOwned::to_owned),
        SharedMessageRole::User
        | SharedMessageRole::Assistant
        | SharedMessageRole::System
        | SharedMessageRole::Notice => None,
    };
    let tool_command = match role {
        SharedMessageRole::ToolResult => {
            if tool_name.as_deref() == Some("exec") {
                msg.get("arguments")
                    .and_then(|v| v.get("command"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(redact_share_secret_values)
            } else {
                None
            }
        },
        SharedMessageRole::User
        | SharedMessageRole::Assistant
        | SharedMessageRole::System
        | SharedMessageRole::Notice => None,
    };

    if content.is_empty() && audio_data_url.is_none() && image.is_none() && map_links.is_none() {
        return None;
    }
    let created_at = value_u64(msg, "created_at");
    let model = if role == SharedMessageRole::Assistant {
        msg.get("model")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned)
    } else {
        None
    };
    let provider = if role == SharedMessageRole::Assistant {
        msg.get("provider")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned)
    } else {
        None
    };

    Some(SharedMessage {
        role,
        content,
        audio_data_url,
        image,
        image_data_url: None,
        map_links,
        tool_success,
        tool_name,
        tool_command,
        created_at,
        model,
        provider,
    })
}

/// Live session service backed by JSONL store + SQLite metadata.
pub struct LiveSessionService {
    store: Arc<SessionStore>,
    metadata: Arc<SqliteSessionMetadata>,
    share_store: Option<Arc<ShareStore>>,
    sandbox_router: Option<Arc<SandboxRouter>>,
    project_store: Option<Arc<dyn ProjectStore>>,
    hook_registry: Option<Arc<HookRegistry>>,
    state_store: Option<Arc<SessionStateStore>>,
    browser_service: Option<Arc<dyn crate::services::BrowserService>>,
}

impl LiveSessionService {
    pub fn new(store: Arc<SessionStore>, metadata: Arc<SqliteSessionMetadata>) -> Self {
        Self {
            store,
            metadata,
            share_store: None,
            sandbox_router: None,
            project_store: None,
            hook_registry: None,
            state_store: None,
            browser_service: None,
        }
    }

    pub fn with_sandbox_router(mut self, router: Arc<SandboxRouter>) -> Self {
        self.sandbox_router = Some(router);
        self
    }

    pub fn with_share_store(mut self, store: Arc<ShareStore>) -> Self {
        self.share_store = Some(store);
        self
    }

    pub fn with_project_store(mut self, store: Arc<dyn ProjectStore>) -> Self {
        self.project_store = Some(store);
        self
    }

    pub fn with_hooks(mut self, registry: Arc<HookRegistry>) -> Self {
        self.hook_registry = Some(registry);
        self
    }

    pub fn with_state_store(mut self, store: Arc<SessionStateStore>) -> Self {
        self.state_store = Some(store);
        self
    }

    pub fn with_browser_service(
        mut self,
        browser: Arc<dyn crate::services::BrowserService>,
    ) -> Self {
        self.browser_service = Some(browser);
        self
    }
}

#[async_trait]
impl SessionService for LiveSessionService {
    async fn list(&self) -> ServiceResult {
        let all = self.metadata.list().await;

        let mut entries: Vec<Value> = Vec::with_capacity(all.len());
        for e in all {
            // Check if this session is the active one for its channel binding.
            let active_channel = if let Some(ref binding_json) = e.channel_binding {
                if let Ok(target) =
                    serde_json::from_str::<moltis_channels::ChannelReplyTarget>(binding_json)
                {
                    self.metadata
                        .get_active_session(
                            target.channel_type.as_str(),
                            &target.account_id,
                            &target.chat_id,
                        )
                        .await
                        .map(|k| k == e.key)
                        .unwrap_or(false)
                } else {
                    false
                }
            } else {
                false
            };

            entries.push(serde_json::json!({
                "id": e.id,
                "key": e.key,
                "label": e.label,
                "model": e.model,
                "createdAt": e.created_at,
                "updatedAt": e.updated_at,
                "messageCount": e.message_count,
                "lastSeenMessageCount": e.last_seen_message_count,
                "projectId": e.project_id,
                "sandbox_enabled": e.sandbox_enabled,
                "sandbox_image": e.sandbox_image,
                "worktree_branch": e.worktree_branch,
                "channelBinding": e.channel_binding,
                "activeChannel": active_channel,
                "parentSessionKey": e.parent_session_key,
                "forkPoint": e.fork_point,
                "mcpDisabled": e.mcp_disabled,
                "preview": e.preview,
                "version": e.version,
            }));
        }
        Ok(serde_json::json!(entries))
    }

    async fn preview(&self, params: Value) -> ServiceResult {
        let key = params
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'key' parameter".to_string())?;
        let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;

        let messages = self
            .store
            .read_last_n(key, limit)
            .await
            .map_err(|e| e.to_string())?;
        Ok(serde_json::json!({ "messages": filter_ui_history(messages) }))
    }

    async fn resolve(&self, params: Value) -> ServiceResult {
        let key = params
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'key' parameter".to_string())?;

        let entry = self
            .metadata
            .upsert(key, None)
            .await
            .map_err(|e| e.to_string())?;
        let history = self.store.read(key).await.map_err(|e| e.to_string())?;

        // Recompute preview from combined messages every time resolve runs,
        // so sessions get the latest multi-message preview algorithm.
        if !history.is_empty() {
            let new_preview = extract_preview(&history);
            if new_preview.as_deref() != entry.preview.as_deref() {
                self.metadata.set_preview(key, new_preview.as_deref()).await;
            }
        }

        // Dispatch SessionStart hook for newly created sessions (empty history).
        if history.is_empty()
            && let Some(ref hooks) = self.hook_registry
        {
            let payload = moltis_common::hooks::HookPayload::SessionStart {
                session_key: key.to_string(),
            };
            if let Err(e) = hooks.dispatch(&payload).await {
                warn!(session = %key, error = %e, "SessionStart hook failed");
            }
        }

        Ok(serde_json::json!({
            "entry": {
                "id": entry.id,
                "key": entry.key,
                "label": entry.label,
                "model": entry.model,
                "createdAt": entry.created_at,
                "updatedAt": entry.updated_at,
                "messageCount": entry.message_count,
                "projectId": entry.project_id,
                "archived": entry.archived,
                "sandbox_enabled": entry.sandbox_enabled,
                "sandbox_image": entry.sandbox_image,
                "worktree_branch": entry.worktree_branch,
                "mcpDisabled": entry.mcp_disabled,
                "version": entry.version,
            },
            "history": filter_ui_history(history),
        }))
    }

    async fn patch(&self, params: Value) -> ServiceResult {
        let key = params
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'key' parameter".to_string())?;
        let label = params
            .get("label")
            .and_then(|v| v.as_str())
            .map(String::from);
        let model = params
            .get("model")
            .and_then(|v| v.as_str())
            .map(String::from);

        let entry = self
            .metadata
            .get(key)
            .await
            .ok_or_else(|| format!("session '{key}' not found"))?;
        if label.is_some() {
            if entry.channel_binding.is_some() {
                return Err("cannot rename a channel-bound session".to_string());
            }
            let _ = self.metadata.upsert(key, label).await;
        }
        if model.is_some() {
            self.metadata.set_model(key, model).await;
        }
        if params.get("project_id").is_some() {
            let project_id = params
                .get("project_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from);
            self.metadata.set_project_id(key, project_id).await;
        }
        // Update worktree_branch if provided.
        if params.get("worktree_branch").is_some() {
            let worktree_branch = params
                .get("worktree_branch")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from);
            self.metadata
                .set_worktree_branch(key, worktree_branch)
                .await;
        }

        // Update sandbox_image if provided.
        if params.get("sandbox_image").is_some() {
            let sandbox_image = params
                .get("sandbox_image")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from);
            self.metadata
                .set_sandbox_image(key, sandbox_image.clone())
                .await;
            // Push image override to sandbox router.
            if let Some(ref router) = self.sandbox_router {
                if let Some(ref img) = sandbox_image {
                    router.set_image_override(key, img.clone()).await;
                } else {
                    router.remove_image_override(key).await;
                }
            }
        }

        // Update mcp_disabled if provided.
        if params.get("mcp_disabled").is_some() {
            let mcp_disabled = params.get("mcp_disabled").and_then(|v| v.as_bool());
            self.metadata.set_mcp_disabled(key, mcp_disabled).await;
        }

        // Update sandbox_enabled if provided.
        if params.get("sandbox_enabled").is_some() {
            let sandbox_enabled = params.get("sandbox_enabled").and_then(|v| v.as_bool());
            self.metadata
                .set_sandbox_enabled(key, sandbox_enabled)
                .await;
            // Push override to sandbox router.
            if let Some(ref router) = self.sandbox_router {
                if let Some(enabled) = sandbox_enabled {
                    router.set_override(key, enabled).await;
                } else {
                    router.remove_override(key).await;
                }
            }
        }

        let entry = self
            .metadata
            .get(key)
            .await
            .ok_or_else(|| format!("session '{key}' not found after update"))?;
        Ok(serde_json::json!({
            "id": entry.id,
            "key": entry.key,
            "label": entry.label,
            "model": entry.model,
            "sandbox_enabled": entry.sandbox_enabled,
            "sandbox_image": entry.sandbox_image,
            "worktree_branch": entry.worktree_branch,
            "mcpDisabled": entry.mcp_disabled,
            "version": entry.version,
        }))
    }

    async fn share_create(&self, params: Value) -> ServiceResult {
        let key = params
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'key' parameter".to_string())?;

        let visibility = params
            .get("visibility")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<ShareVisibility>().ok())
            .unwrap_or(ShareVisibility::Public);

        let share_store = self
            .share_store
            .as_ref()
            .ok_or_else(|| "session share store not configured".to_string())?;

        let entry = self
            .metadata
            .get(key)
            .await
            .ok_or_else(|| format!("session '{key}' not found"))?;
        let history = self.store.read(key).await.map_err(|e| e.to_string())?;

        let snapshot = ShareSnapshot {
            session_key: key.to_string(),
            session_label: entry.label.clone(),
            cutoff_message_count: history.len() as u32,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            messages: {
                let mut shared_messages = Vec::new();
                for msg in &history {
                    if let Some(shared) = to_shared_message(msg, key, self.store.as_ref()).await {
                        shared_messages.push(shared);
                    }
                }
                shared_messages
            },
        };
        let snapshot_json = serde_json::to_string(&snapshot).map_err(|e| e.to_string())?;

        let created = share_store
            .create_or_replace(
                key,
                visibility,
                snapshot_json,
                snapshot.cutoff_message_count,
            )
            .await
            .map_err(|e| e.to_string())?;

        // Persist a UI-only notice in the source session so users can see
        // the exact cutoff marker without affecting future LLM context.
        let boundary_notice = PersistedMessage::Notice {
            content: SHARE_BOUNDARY_NOTICE.to_string(),
            created_at: Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            ),
        };
        if let Err(e) = self.store.append(key, &boundary_notice.to_value()).await {
            warn!(
                session_key = key,
                share_id = created.share.id,
                error = %e,
                "failed to persist share boundary notice; revoking share"
            );
            let _ = share_store.revoke(&created.share.id).await;
            return Err(format!("failed to persist share boundary notice: {e}"));
        }
        match self.store.count(key).await {
            Ok(message_count) => {
                self.metadata.touch(key, message_count).await;
            },
            Err(e) => {
                warn!(session_key = key, error = %e, "failed to update session message count");
            },
        }

        Ok(serde_json::json!({
            "id": created.share.id,
            "sessionKey": created.share.session_key,
            "visibility": created.share.visibility.as_str(),
            "path": format!("/share/{}", created.share.id),
            "createdAt": created.share.created_at,
            "views": created.share.views,
            "snapshotMessageCount": created.share.snapshot_message_count,
            "accessKey": created.access_key,
            "notice": SHARE_BOUNDARY_NOTICE,
        }))
    }

    async fn share_list(&self, params: Value) -> ServiceResult {
        let key = params
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'key' parameter".to_string())?;

        let share_store = self
            .share_store
            .as_ref()
            .ok_or_else(|| "session share store not configured".to_string())?;

        let shares = share_store
            .list_for_session(key)
            .await
            .map_err(|e| e.to_string())?;

        let items: Vec<Value> = shares
            .into_iter()
            .map(|share| {
                serde_json::json!({
                    "id": share.id,
                    "sessionKey": share.session_key,
                    "visibility": share.visibility.as_str(),
                    "path": format!("/share/{}", share.id),
                    "views": share.views,
                    "createdAt": share.created_at,
                    "revokedAt": share.revoked_at,
                })
            })
            .collect();
        Ok(serde_json::json!(items))
    }

    async fn share_revoke(&self, params: Value) -> ServiceResult {
        let id = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'id' parameter".to_string())?;

        let share_store = self
            .share_store
            .as_ref()
            .ok_or_else(|| "session share store not configured".to_string())?;

        let revoked = share_store.revoke(id).await.map_err(|e| e.to_string())?;
        Ok(serde_json::json!({ "revoked": revoked }))
    }

    async fn reset(&self, params: Value) -> ServiceResult {
        let key = params
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'key' parameter".to_string())?;

        self.store.clear(key).await.map_err(|e| e.to_string())?;
        self.metadata.touch(key, 0).await;
        self.metadata.set_preview(key, None).await;

        Ok(serde_json::json!({}))
    }

    async fn delete(&self, params: Value) -> ServiceResult {
        let key = params
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'key' parameter".to_string())?;

        if key == "main" {
            return Err("cannot delete the main session".to_string());
        }

        let force = params
            .get("force")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Check for worktree cleanup before deleting metadata.
        if let Some(entry) = self.metadata.get(key).await
            && entry.worktree_branch.is_some()
            && let Some(ref project_id) = entry.project_id
            && let Some(ref project_store) = self.project_store
            && let Ok(Some(project)) = project_store.get(project_id).await
        {
            let project_dir = &project.directory;
            let wt_dir = project_dir.join(".moltis-worktrees").join(key);

            // Safety checks unless force is set.
            if !force
                && wt_dir.exists()
                && let Ok(true) =
                    moltis_projects::WorktreeManager::has_uncommitted_changes(&wt_dir).await
            {
                return Err(
                    "worktree has uncommitted changes; use force: true to delete anyway"
                        .to_string(),
                );
            }

            // Run teardown command if configured.
            if let Some(ref cmd) = project.teardown_command
                && wt_dir.exists()
                && let Err(e) =
                    moltis_projects::WorktreeManager::run_teardown(&wt_dir, cmd, project_dir, key)
                        .await
            {
                tracing::warn!("worktree teardown failed: {e}");
            }

            if let Err(e) = moltis_projects::WorktreeManager::cleanup(project_dir, key).await {
                tracing::warn!("worktree cleanup failed: {e}");
            }
        }

        self.store.clear(key).await.map_err(|e| e.to_string())?;

        // Clean up sandbox resources for this session.
        if let Some(ref router) = self.sandbox_router
            && let Err(e) = router.cleanup_session(key).await
        {
            tracing::warn!("sandbox cleanup for session {key}: {e}");
        }

        // Cascade-delete session state.
        if let Some(ref state_store) = self.state_store
            && let Err(e) = state_store.delete_session(key).await
        {
            tracing::warn!("session state cleanup for {key}: {e}");
        }

        self.metadata.remove(key).await;

        // Dispatch SessionEnd hook (read-only).
        if let Some(ref hooks) = self.hook_registry {
            let payload = moltis_common::hooks::HookPayload::SessionEnd {
                session_key: key.to_string(),
            };
            if let Err(e) = hooks.dispatch(&payload).await {
                warn!(session = %key, error = %e, "SessionEnd hook failed");
            }
        }

        Ok(serde_json::json!({}))
    }

    async fn compact(&self, _params: Value) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn fork(&self, params: Value) -> ServiceResult {
        let parent_key = params
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'key' parameter".to_string())?;
        let label = params
            .get("label")
            .and_then(|v| v.as_str())
            .map(String::from);

        let messages = self
            .store
            .read(parent_key)
            .await
            .map_err(|e| e.to_string())?;
        let msg_count = messages.len();

        let fork_point = params
            .get("forkPoint")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(msg_count);

        if fork_point > msg_count {
            return Err(format!(
                "forkPoint {fork_point} exceeds message count {msg_count}"
            ));
        }

        let new_key = format!("session:{}", uuid::Uuid::new_v4());
        let forked_messages: Vec<Value> = messages[..fork_point].to_vec();

        self.store
            .replace_history(&new_key, forked_messages)
            .await
            .map_err(|e| e.to_string())?;

        let _entry = self
            .metadata
            .upsert(&new_key, label)
            .await
            .map_err(|e| e.to_string())?;

        self.metadata.touch(&new_key, fork_point as u32).await;

        // Inherit model, project, and mcp_disabled from parent.
        if let Some(parent) = self.metadata.get(parent_key).await {
            if parent.model.is_some() {
                self.metadata.set_model(&new_key, parent.model).await;
            }
            if parent.project_id.is_some() {
                self.metadata
                    .set_project_id(&new_key, parent.project_id)
                    .await;
            }
            if parent.mcp_disabled.is_some() {
                self.metadata
                    .set_mcp_disabled(&new_key, parent.mcp_disabled)
                    .await;
            }
        }

        // Set parent relationship.
        self.metadata
            .set_parent(
                &new_key,
                Some(parent_key.to_string()),
                Some(fork_point as u32),
            )
            .await;

        // Re-fetch after all mutations to get the final version.
        let final_entry = self
            .metadata
            .get(&new_key)
            .await
            .ok_or_else(|| format!("forked session '{new_key}' not found after creation"))?;
        Ok(serde_json::json!({
            "sessionKey": new_key,
            "id": final_entry.id,
            "label": final_entry.label,
            "forkPoint": fork_point,
            "messageCount": fork_point,
            "version": final_entry.version,
        }))
    }

    async fn branches(&self, params: Value) -> ServiceResult {
        let key = params
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'key' parameter".to_string())?;

        let children = self.metadata.list_children(key).await;
        let items: Vec<Value> = children
            .into_iter()
            .map(|e| {
                serde_json::json!({
                    "key": e.key,
                    "label": e.label,
                    "forkPoint": e.fork_point,
                    "messageCount": e.message_count,
                    "createdAt": e.created_at,
                })
            })
            .collect();
        Ok(serde_json::json!(items))
    }

    async fn search(&self, params: Value) -> ServiceResult {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();

        if query.is_empty() {
            return Ok(serde_json::json!([]));
        }

        let max = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

        let results = self
            .store
            .search(query, max)
            .await
            .map_err(|e| e.to_string())?;

        let enriched: Vec<Value> = {
            let mut out = Vec::with_capacity(results.len());
            for r in results {
                let label = self
                    .metadata
                    .get(&r.session_key)
                    .await
                    .and_then(|e| e.label);
                out.push(serde_json::json!({
                    "sessionKey": r.session_key,
                    "snippet": r.snippet,
                    "role": r.role,
                    "messageIndex": r.message_index,
                    "label": label,
                }));
            }
            out
        };

        Ok(serde_json::json!(enriched))
    }

    async fn mark_seen(&self, key: &str) {
        self.metadata.mark_seen(key).await;
    }

    async fn clear_all(&self) -> ServiceResult {
        let all = self.metadata.list().await;
        let mut deleted = 0u32;

        for entry in &all {
            // Keep main, channel-bound (telegram etc.), and cron sessions.
            if entry.key == "main"
                || entry.channel_binding.is_some()
                || entry.key.starts_with("telegram:")
                || entry.key.starts_with("cron:")
            {
                continue;
            }

            // Reuse delete logic via params.
            let params = serde_json::json!({ "key": entry.key, "force": true });
            if let Err(e) = self.delete(params).await {
                warn!(session = %entry.key, error = %e, "clear_all: failed to delete session");
                continue;
            }
            deleted += 1;
        }

        // Close all browser containers since all user sessions are being cleared.
        if let Some(ref browser) = self.browser_service {
            info!("closing all browser sessions after clear_all");
            browser.close_all().await;
        }

        Ok(serde_json::json!({ "deleted": deleted }))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn filter_ui_history_removes_empty_assistant_messages() {
        let messages = vec![
            serde_json::json!({"role": "user", "content": "hello"}),
            serde_json::json!({"role": "assistant", "content": "hi there"}),
            serde_json::json!({"role": "user", "content": "run ls"}),
            // Empty assistant after tool use — should be filtered
            serde_json::json!({"role": "assistant", "content": ""}),
            serde_json::json!({"role": "user", "content": "run pwd"}),
            serde_json::json!({"role": "assistant", "content": "here is the output"}),
        ];
        let filtered = filter_ui_history(messages);
        assert_eq!(filtered.len(), 5);
        // The empty assistant message at index 3 should be gone.
        assert_eq!(filtered[2]["role"], "user");
        assert_eq!(filtered[2]["content"], "run ls");
        assert_eq!(filtered[3]["role"], "user");
        assert_eq!(filtered[3]["content"], "run pwd");
    }

    #[test]
    fn filter_ui_history_removes_whitespace_only_assistant() {
        let messages = vec![
            serde_json::json!({"role": "user", "content": "hello"}),
            serde_json::json!({"role": "assistant", "content": "   \n  "}),
        ];
        let filtered = filter_ui_history(messages);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0]["role"], "user");
    }

    #[test]
    fn filter_ui_history_keeps_non_empty_assistant() {
        let messages = vec![
            serde_json::json!({"role": "assistant", "content": "real response"}),
            serde_json::json!({"role": "assistant", "content": ".", "model": "gpt-4o"}),
        ];
        let filtered = filter_ui_history(messages);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn filter_ui_history_keeps_non_assistant_roles() {
        let messages = vec![
            serde_json::json!({"role": "system", "content": ""}),
            serde_json::json!({"role": "tool", "tool_call_id": "x", "content": ""}),
            serde_json::json!({"role": "user", "content": ""}),
        ];
        // Non-assistant roles pass through even if content is empty.
        let filtered = filter_ui_history(messages);
        assert_eq!(filtered.len(), 3);
    }

    // --- Preview extraction tests ---

    #[test]
    fn message_text_from_string_content() {
        let msg = serde_json::json!({"role": "user", "content": "hello world"});
        assert_eq!(message_text(&msg), Some("hello world".to_string()));
    }

    #[test]
    fn message_text_from_content_blocks() {
        let msg = serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "hello"},
                {"type": "image_url", "url": "http://example.com/img.png"},
                {"type": "text", "text": "world"}
            ]
        });
        assert_eq!(message_text(&msg), Some("hello world".to_string()));
    }

    #[test]
    fn message_text_empty_content() {
        let msg = serde_json::json!({"role": "user", "content": "  "});
        assert_eq!(message_text(&msg), None);
    }

    #[test]
    fn message_text_no_content_field() {
        let msg = serde_json::json!({"role": "user"});
        assert_eq!(message_text(&msg), None);
    }

    #[test]
    fn truncate_preview_short_string() {
        assert_eq!(truncate_preview("short", 200), "short");
    }

    #[test]
    fn truncate_preview_long_string() {
        let long = "a".repeat(250);
        let result = truncate_preview(&long, 200);
        assert!(result.ends_with('…'));
        // 200 'a' chars + the '…' char
        assert!(result.len() <= 204); // 200 bytes + up to 3 for '…'
    }

    #[test]
    fn extract_preview_from_value_basic() {
        let msg = serde_json::json!({"role": "user", "content": "tell me a joke"});
        let result = extract_preview_from_value(&msg);
        assert_eq!(result, Some("tell me a joke".to_string()));
    }

    #[test]
    fn extract_preview_single_short_message() {
        let history = vec![serde_json::json!({"role": "user", "content": "hi"})];
        let result = extract_preview(&history);
        // Short message is still returned, just won't reach the 80-char target
        assert_eq!(result, Some("hi".to_string()));
    }

    #[test]
    fn extract_preview_combines_messages_until_target() {
        let history = vec![
            serde_json::json!({"role": "user", "content": "hi"}),
            serde_json::json!({"role": "assistant", "content": "Hello! How can I help you today?"}),
            serde_json::json!({"role": "user", "content": "Tell me about Rust programming language"}),
        ];
        let result = extract_preview(&history).expect("should produce preview");
        assert!(result.contains("hi"));
        assert!(result.contains(" — "));
        assert!(result.contains("Hello!"));
        // Should stop once target (80) is reached
        assert!(result.len() >= 30);
    }

    #[test]
    fn extract_preview_skips_system_and_tool_messages() {
        let history = vec![
            serde_json::json!({"role": "system", "content": "You are a helpful assistant."}),
            serde_json::json!({"role": "user", "content": "hello"}),
            serde_json::json!({"role": "tool", "content": "tool output"}),
            serde_json::json!({"role": "assistant", "content": "Hi there!"}),
        ];
        let result = extract_preview(&history).expect("should produce preview");
        // Should not contain system or tool content
        assert!(!result.contains("helpful assistant"));
        assert!(!result.contains("tool output"));
        assert!(result.contains("hello"));
        assert!(result.contains("Hi there!"));
    }

    #[test]
    fn extract_preview_empty_history() {
        let result = extract_preview(&[]);
        assert_eq!(result, None);
    }

    #[test]
    fn extract_preview_only_system_messages() {
        let history =
            vec![serde_json::json!({"role": "system", "content": "You are a helpful assistant."})];
        let result = extract_preview(&history);
        assert_eq!(result, None);
    }

    #[test]
    fn extract_preview_truncates_at_max() {
        // Build a very long message that exceeds MAX (200)
        let long_text = "a".repeat(300);
        let history = vec![serde_json::json!({"role": "user", "content": long_text})];
        let result = extract_preview(&history).expect("should produce preview");
        assert!(result.ends_with('…'));
        assert!(result.len() <= 204);
    }

    #[test]
    fn media_filename_extracts_last_segment() {
        assert_eq!(media_filename("media/main/voice.ogg"), Some("voice.ogg"));
        assert_eq!(media_filename("voice.ogg"), Some("voice.ogg"));
        assert_eq!(media_filename(""), None);
    }

    #[test]
    fn audio_mime_type_maps_known_extensions() {
        assert_eq!(audio_mime_type("voice.ogg"), "audio/ogg");
        assert_eq!(audio_mime_type("voice.webm"), "audio/webm");
        assert_eq!(audio_mime_type("voice.mp3"), "audio/mpeg");
        assert_eq!(audio_mime_type("voice.unknown"), "application/octet-stream");
    }

    #[test]
    fn image_mime_type_maps_known_extensions() {
        assert_eq!(image_mime_type("map.png"), "image/png");
        assert_eq!(image_mime_type("map.jpeg"), "image/jpeg");
        assert_eq!(image_mime_type("map.webp"), "image/webp");
        assert_eq!(image_mime_type("map.unknown"), "application/octet-stream");
    }

    #[test]
    fn sanitize_share_url_rejects_unsafe_schemes() {
        assert_eq!(
            sanitize_share_url("https://maps.apple.com/?q=test"),
            Some("https://maps.apple.com/?q=test".to_string())
        );
        assert_eq!(sanitize_share_url("javascript:alert(1)"), None);
        assert_eq!(sanitize_share_url("data:text/html,test"), None);
    }

    #[tokio::test]
    async fn message_audio_data_url_for_share_reads_media_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = moltis_sessions::store::SessionStore::new(dir.path().to_path_buf());
        let bytes = b"OggSfake".to_vec();
        store
            .save_media("main", "voice.ogg", &bytes)
            .await
            .expect("save media");

        let msg = serde_json::json!({
            "role": "assistant",
            "audio": "media/main/voice.ogg",
        });

        let data_url = message_audio_data_url_for_share(&msg, "main", &store).await;
        assert!(data_url.is_some());
        assert!(
            data_url
                .as_deref()
                .unwrap_or_default()
                .starts_with("data:audio/ogg;base64,")
        );
    }

    #[tokio::test]
    async fn to_shared_message_skips_system_and_notice_roles() {
        let dir = tempfile::tempdir().unwrap();
        let store = moltis_sessions::store::SessionStore::new(dir.path().to_path_buf());

        let system_msg = serde_json::json!({
            "role": "system",
            "content": "system info",
        });
        let notice_msg = serde_json::json!({
            "role": "notice",
            "content": "share boundary",
        });
        let assistant_msg = serde_json::json!({
            "role": "assistant",
            "content": "hello",
        });

        assert!(
            to_shared_message(&system_msg, "main", &store)
                .await
                .is_none()
        );
        assert!(
            to_shared_message(&notice_msg, "main", &store)
                .await
                .is_none()
        );
        assert!(
            to_shared_message(&assistant_msg, "main", &store)
                .await
                .is_some()
        );
    }

    #[tokio::test]
    async fn to_shared_message_includes_user_audio_without_text() {
        let dir = tempfile::tempdir().unwrap();
        let store = moltis_sessions::store::SessionStore::new(dir.path().to_path_buf());
        store
            .save_media("main", "voice-input.webm", b"RIFFfake")
            .await
            .expect("save media");

        let user_audio_msg = serde_json::json!({
            "role": "user",
            "content": "",
            "audio": "media/main/voice-input.webm",
        });

        let shared = to_shared_message(&user_audio_msg, "main", &store)
            .await
            .expect("shared message");

        assert!(matches!(shared.role, SharedMessageRole::User));
        assert!(shared.content.is_empty());
        assert!(
            shared
                .audio_data_url
                .as_deref()
                .unwrap_or_default()
                .starts_with("data:audio/webm;base64,")
        );
    }

    #[tokio::test]
    async fn to_shared_message_includes_tool_result_screenshot_and_map_links() {
        let dir = tempfile::tempdir().unwrap();
        let store = moltis_sessions::store::SessionStore::new(dir.path().to_path_buf());
        let tiny_png = general_purpose::STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+tmXcAAAAASUVORK5CYII=")
            .unwrap();
        store
            .save_media("main", "call-map.png", &tiny_png)
            .await
            .expect("save media");

        let tool_msg = serde_json::json!({
            "role": "tool_result",
            "tool_name": "show_map",
            "success": true,
            "created_at": 1_770_966_725_000_u64,
            "result": {
                "label": "Tartine Bakery",
                "screenshot": "media/main/call-map.png",
                "map_links": {
                    "google_maps": "https://www.google.com/maps/search/?api=1&query=Tartine+Bakery",
                    "apple_maps": "javascript:alert(1)",
                    "openstreetmap": "https://www.openstreetmap.org/search?query=Tartine+Bakery",
                },
            },
        });

        let shared = to_shared_message(&tool_msg, "main", &store)
            .await
            .expect("shared tool_result message");

        assert!(matches!(shared.role, SharedMessageRole::ToolResult));
        assert_eq!(shared.tool_success, Some(true));
        assert_eq!(shared.tool_name.as_deref(), Some("show_map"));
        assert!(shared.tool_command.is_none());
        assert!(shared.audio_data_url.is_none());
        assert!(shared.image_data_url.is_none());
        let image = shared.image.expect("shared image variants");
        assert!(image.preview.data_url.starts_with("data:image/png;base64,"));
        assert_eq!(image.preview.width, 1);
        assert_eq!(image.preview.height, 1);
        assert!(image.full.is_none());
        let map_links = shared.map_links.expect("map links");
        assert!(map_links.google_maps.is_some());
        assert!(map_links.openstreetmap.is_some());
        assert!(map_links.apple_maps.is_none());
        assert!(shared.content.contains("Tartine Bakery"));
    }

    #[test]
    fn tool_result_text_for_share_preserves_full_stdout() {
        let large_stdout = format!("{{\"items\":[\"{}\"]}}", "x".repeat(2_000));
        let msg = serde_json::json!({
            "role": "tool_result",
            "result": {
                "stdout": large_stdout
            }
        });

        let text = tool_result_text_for_share(&msg).expect("tool text should exist");
        assert!(text.contains("\"items\""));
        assert!(!text.contains("(truncated)"));
        assert!(!text.ends_with('…'));
        assert!(text.len() > 1_800);
    }

    #[test]
    fn redact_share_secret_values_masks_env_vars_and_api_tokens() {
        let input = "OPENAI_API_KEY=sk-openai BRAVE_API_KEY=brave-secret Authorization: Bearer bearer-secret https://api.example.com/search?q=test&api_key=url-secret";
        let redacted = redact_share_secret_values(input);

        assert!(!redacted.contains("sk-openai"));
        assert!(!redacted.contains("brave-secret"));
        assert!(!redacted.contains("bearer-secret"));
        assert!(!redacted.contains("url-secret"));
        assert!(redacted.contains("OPENAI_API_KEY=[REDACTED]"));
        assert!(redacted.contains("BRAVE_API_KEY=[REDACTED]"));
        assert!(redacted.contains("Bearer [REDACTED]"));
    }

    #[test]
    fn tool_result_text_for_share_redacts_sensitive_values() {
        let msg = serde_json::json!({
            "role": "tool_result",
            "result": {
                "stdout": "{\"apiKey\":\"llm-secret\",\"voice_api_key\":\"voice-secret\"}\nOPENAI_API_KEY=env-secret",
                "stderr": "Authorization: Bearer bearer-secret\nx-api-key: header-secret",
            }
        });

        let text = tool_result_text_for_share(&msg).unwrap_or_default();
        assert!(!text.contains("llm-secret"));
        assert!(!text.contains("voice-secret"));
        assert!(!text.contains("env-secret"));
        assert!(!text.contains("bearer-secret"));
        assert!(!text.contains("header-secret"));
        assert!(text.contains(SHARE_REDACTED_VALUE));
    }

    #[tokio::test]
    async fn to_shared_message_includes_exec_command_for_tool_result() {
        let dir = tempfile::tempdir().unwrap();
        let store = moltis_sessions::store::SessionStore::new(dir.path().to_path_buf());
        let tool_msg = serde_json::json!({
            "role": "tool_result",
            "tool_name": "exec",
            "arguments": {
                "command": "curl -s https://example.com"
            },
            "success": true,
            "result": {
                "stdout": "{\"ok\":true}",
                "stderr": "",
                "exit_code": 0,
            },
        });

        let shared = to_shared_message(&tool_msg, "main", &store)
            .await
            .expect("shared exec tool result");
        assert_eq!(shared.tool_name.as_deref(), Some("exec"));
        assert_eq!(
            shared.tool_command.as_deref(),
            Some("curl -s https://example.com")
        );
        assert!(shared.content.contains("{\"ok\":true}"));
    }

    #[tokio::test]
    async fn to_shared_message_redacts_exec_command_and_output_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let store = moltis_sessions::store::SessionStore::new(dir.path().to_path_buf());
        let tool_msg = serde_json::json!({
            "role": "tool_result",
            "tool_name": "exec",
            "arguments": {
                "command": "OPENAI_API_KEY=sk-openai curl -s -H 'Authorization: Bearer bearer-secret' 'https://api.example.com?q=test&api_key=url-secret'"
            },
            "success": true,
            "result": {
                "stdout": "{\"api_key\":\"stdout-secret\"}",
                "stderr": "ELEVENLABS_API_KEY=voice-secret",
                "exit_code": 0,
            },
        });

        let shared = to_shared_message(&tool_msg, "main", &store)
            .await
            .expect("shared exec tool result");

        assert_eq!(shared.tool_name.as_deref(), Some("exec"));
        let command = shared.tool_command.unwrap_or_default();
        assert!(!command.contains("sk-openai"));
        assert!(!command.contains("bearer-secret"));
        assert!(!command.contains("url-secret"));
        assert!(command.contains(SHARE_REDACTED_VALUE));

        assert!(!shared.content.contains("stdout-secret"));
        assert!(!shared.content.contains("voice-secret"));
        assert!(shared.content.contains(SHARE_REDACTED_VALUE));
    }

    // --- Browser service integration tests ---

    use std::sync::atomic::{AtomicU32, Ordering};

    /// Mock browser service that tracks lifecycle method calls.
    struct MockBrowserService {
        close_all_calls: AtomicU32,
    }

    impl MockBrowserService {
        fn new() -> Self {
            Self {
                close_all_calls: AtomicU32::new(0),
            }
        }

        fn close_all_count(&self) -> u32 {
            self.close_all_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl crate::services::BrowserService for MockBrowserService {
        async fn request(&self, _p: serde_json::Value) -> crate::services::ServiceResult {
            Err("mock".into())
        }

        async fn close_all(&self) {
            self.close_all_calls.fetch_add(1, Ordering::SeqCst);
        }
    }

    async fn sqlite_pool() -> sqlx::SqlitePool {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        moltis_sessions::metadata::SqliteSessionMetadata::init(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn with_browser_service_builder() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(moltis_sessions::store::SessionStore::new(
            dir.path().to_path_buf(),
        ));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(moltis_sessions::metadata::SqliteSessionMetadata::new(pool));

        let mock = Arc::new(MockBrowserService::new());
        let svc = LiveSessionService::new(store, metadata)
            .with_browser_service(Arc::clone(&mock) as Arc<dyn crate::services::BrowserService>);

        assert!(svc.browser_service.is_some());
    }

    #[tokio::test]
    async fn clear_all_calls_browser_close_all() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(moltis_sessions::store::SessionStore::new(
            dir.path().to_path_buf(),
        ));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(moltis_sessions::metadata::SqliteSessionMetadata::new(pool));

        let mock = Arc::new(MockBrowserService::new());
        let svc = LiveSessionService::new(store, metadata)
            .with_browser_service(Arc::clone(&mock) as Arc<dyn crate::services::BrowserService>);

        let result = svc.clear_all().await;
        assert!(result.is_ok());
        assert_eq!(mock.close_all_count(), 1, "close_all should be called once");
    }

    #[tokio::test]
    async fn clear_all_without_browser_service() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(moltis_sessions::store::SessionStore::new(
            dir.path().to_path_buf(),
        ));
        let pool = sqlite_pool().await;
        let metadata = Arc::new(moltis_sessions::metadata::SqliteSessionMetadata::new(pool));

        // No browser_service wired.
        let svc = LiveSessionService::new(store, metadata);

        let result = svc.clear_all().await;
        assert!(result.is_ok());
    }
}
