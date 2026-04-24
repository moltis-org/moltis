#[allow(unused_imports)]
use std::sync::Arc;

#[allow(unused_imports)]
use {
    async_trait::async_trait,
    base64::{Engine, engine::general_purpose},
    serde::Deserialize,
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

#[allow(unused_imports)]
#[cfg(feature = "fs-tools")]
use moltis_tools::fs::FsState;

#[allow(unused_imports)]
use crate::{
    agent_persona::AgentPersonaStore,
    services::{ServiceError, ServiceResult, SessionService, TtsService},
    session_types::{PatchParams, VoiceGenerateParams, VoiceTarget, parse_params},
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
const SESSION_PREVIEW_MAX_CHARS: usize = 200;
const UI_HISTORY_MAX_BYTES: usize = 2 * 1024 * 1024;
const UI_HISTORY_MIN_MESSAGES: usize = 120;
const UI_HISTORY_TRIM_STEP: usize = 50;

fn resolve_hook_channel_binding(
    session_key: &str,
    session_entry: Option<&moltis_sessions::metadata::SessionEntry>,
) -> Option<moltis_common::hooks::ChannelBinding> {
    let binding = match moltis_channels::resolve_session_channel_binding(
        session_key,
        session_entry.and_then(|entry| entry.channel_binding.as_deref()),
    ) {
        Ok(binding) => binding,
        Err(error) => {
            warn!(
                error = %error,
                session = %session_key,
                "failed to parse channel_binding JSON; falling back to web"
            );
            moltis_channels::web_session_channel_binding()
        },
    };
    (!binding.is_empty()).then_some(binding)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TtsStatusPayload {
    enabled: bool,
    #[serde(default)]
    max_text_length: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TtsConvertPayload {
    audio: String,
}

/// Filter out empty assistant messages from history before sending to the UI.
///
/// Empty assistant messages are persisted in the session JSONL for LLM history
/// coherence (so the model sees a complete user→assistant turn), but they
/// should not be shown in the web UI or sent to channels.
fn filter_ui_history(messages: Vec<Value>) -> Vec<Value> {
    messages
        .into_iter()
        .enumerate()
        .filter_map(|(idx, mut msg)| {
            if msg.get("role").and_then(|v| v.as_str()) == Some("assistant") {
                let has_content = msg
                    .get("content")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| !s.trim().is_empty());
                let has_reasoning = msg
                    .get("reasoning")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| !s.trim().is_empty());
                let has_audio = msg
                    .get("audio")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| !s.trim().is_empty());
                let keep = has_content || has_reasoning || has_audio;
                if !keep {
                    return None;
                }
            }
            if let Some(obj) = msg.as_object_mut() {
                obj.insert("historyIndex".to_string(), serde_json::json!(idx));
            }
            Some(msg)
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

fn sanitize_tts_text(text: &str) -> String {
    #[cfg(feature = "voice")]
    {
        moltis_voice::tts::sanitize_text_for_tts(text).to_string()
    }

    #[cfg(not(feature = "voice"))]
    {
        text.to_string()
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

/// Build a preview by combining user and assistant messages until we
/// have enough text (target ~80 chars). Skips tool_result messages.
fn extract_preview(history: &[Value]) -> Option<String> {
    const TARGET: usize = 80;

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
    Some(truncate_preview(&combined, SESSION_PREVIEW_MAX_CHARS))
}

fn trim_ui_history(mut history: Vec<Value>) -> (Vec<Value>, usize) {
    if history.is_empty() {
        return (history, 0);
    }

    let mut dropped = 0usize;
    loop {
        let size = serde_json::to_vec(&history).map_or(0, |buf| buf.len());
        if size <= UI_HISTORY_MAX_BYTES || history.len() <= UI_HISTORY_MIN_MESSAGES {
            break;
        }

        let removable = history.len().saturating_sub(UI_HISTORY_MIN_MESSAGES);
        if removable == 0 {
            break;
        }

        let trim_count = removable.min(UI_HISTORY_TRIM_STEP);
        history.drain(0..trim_count);
        dropped += trim_count;
    }

    (history, dropped)
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

fn message_reasoning_for_share(msg: &Value) -> Option<String> {
    let reasoning = msg.get("reasoning").and_then(|v| v.as_str())?;
    let trimmed = reasoning.trim();
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
    let reasoning = match role {
        SharedMessageRole::Assistant => message_reasoning_for_share(msg),
        SharedMessageRole::User
        | SharedMessageRole::ToolResult
        | SharedMessageRole::System
        | SharedMessageRole::Notice => None,
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

    if content.is_empty()
        && reasoning.is_none()
        && audio_data_url.is_none()
        && image.is_none()
        && map_links.is_none()
    {
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
        reasoning,
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

mod maintenance;
mod service;
mod share;
pub(crate) mod summary;
#[cfg(test)]
mod tests;
mod voice;

pub use service::LiveSessionService;
