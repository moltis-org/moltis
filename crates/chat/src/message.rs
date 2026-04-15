//! Message content conversion and user document handling.

use {
    serde_json::Value,
    tracing::{debug, warn},
};

use {
    moltis_agents::{
        ContentPart, UserContent, multimodal::parse_data_uri, prompt::VOICE_REPLY_SUFFIX,
    },
    moltis_sessions::{ContentBlock, MessageContent, UserDocument, store::SessionStore},
};

use crate::types::{
    InputChannelDocumentFile, InputChannelMeta, InputMediumParam, InputMessageKind, ReplyMedium,
    is_safe_user_audio_filename, sanitize_user_document_display_name, truncate_at_char_boundary,
};

/// Convert session-crate `MessageContent` to agents-crate `UserContent`.
///
/// The two types have different image representations:
/// - `ContentBlock::ImageUrl` stores a data URI string
/// - `ContentPart::Image` stores separated `media_type` + `data` fields
pub(crate) fn format_user_documents_context(documents: &[UserDocument]) -> Option<String> {
    if documents.is_empty() {
        return None;
    }

    let mut sections = Vec::with_capacity(documents.len() + 1);
    sections.push("[Inbound documents available]".to_string());
    for document in documents {
        sections.push(format!(
            "filename: {}\nmime_type: {}\nlocal_path: {}\nmedia_ref: {}",
            document.display_name,
            document.mime_type,
            document
                .absolute_path
                .as_deref()
                .unwrap_or(&document.media_ref),
            document.media_ref
        ));
    }

    Some(sections.join("\n\n"))
}

pub(crate) fn append_user_documents_to_text(text: &str, documents: &[UserDocument]) -> String {
    if let Some(context) = format_user_documents_context(documents) {
        if text.trim().is_empty() {
            context
        } else {
            format!("{text}\n\n{context}")
        }
    } else {
        text.to_string()
    }
}

pub(crate) fn to_user_content(mc: &MessageContent, documents: &[UserDocument]) -> UserContent {
    match mc {
        MessageContent::Text(text) => {
            UserContent::Text(append_user_documents_to_text(text, documents))
        },
        MessageContent::Multimodal(blocks) => {
            let mut parts: Vec<ContentPart> = blocks
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
            if let Some(context) = format_user_documents_context(documents) {
                if let Some(ContentPart::Text(text)) = parts
                    .iter_mut()
                    .find(|part| matches!(part, ContentPart::Text(_)))
                {
                    if !text.trim().is_empty() {
                        text.push_str("\n\n");
                    }
                    text.push_str(&context);
                } else {
                    parts.insert(0, ContentPart::Text(context));
                }
            }
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

pub(crate) fn rewrite_multimodal_text_blocks(
    blocks: &[ContentBlock],
    new_text: &str,
) -> Vec<ContentBlock> {
    let mut rewritten = Vec::with_capacity(blocks.len().max(1));
    let mut inserted_text = false;

    for block in blocks {
        match block {
            ContentBlock::Text { .. } if !inserted_text => {
                rewritten.push(ContentBlock::Text {
                    text: new_text.to_string(),
                });
                inserted_text = true;
            },
            ContentBlock::Text { .. } => {},
            _ => rewritten.push(block.clone()),
        }
    }

    if !inserted_text {
        rewritten.insert(0, ContentBlock::Text {
            text: new_text.to_string(),
        });
    }

    rewritten
}

pub(crate) fn apply_message_received_rewrite(
    message_content: &mut MessageContent,
    params: &mut Value,
    new_text: &str,
) {
    match message_content {
        MessageContent::Text(text) => {
            *text = new_text.to_string();
            if let Some(params_obj) = params.as_object_mut() {
                params_obj.insert("text".to_string(), serde_json::json!(new_text));
                params_obj.remove("content");
            }
        },
        MessageContent::Multimodal(blocks) => {
            let rewritten_blocks = rewrite_multimodal_text_blocks(blocks, new_text);
            match serde_json::to_value(&rewritten_blocks) {
                Ok(content_value) => {
                    *blocks = rewritten_blocks;
                    if let Some(params_obj) = params.as_object_mut() {
                        params_obj.insert("content".to_string(), content_value);
                        params_obj.remove("text");
                        params_obj.remove("message");
                    }
                },
                Err(e) => {
                    warn!(error = %e, "failed to serialize rewritten multimodal content");
                },
            }
        },
    }
}

pub(crate) fn parse_input_medium(params: &Value) -> Option<ReplyMedium> {
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

pub(crate) fn explicit_reply_medium_override(text: &str) -> Option<ReplyMedium> {
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

pub(crate) fn infer_reply_medium(params: &Value, text: &str) -> ReplyMedium {
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

pub(crate) fn apply_voice_reply_suffix(
    system_prompt: String,
    desired_reply_medium: ReplyMedium,
) -> String {
    if desired_reply_medium != ReplyMedium::Voice {
        return system_prompt;
    }

    format!("{system_prompt}{VOICE_REPLY_SUFFIX}")
}

pub(crate) fn user_audio_path_from_params(params: &Value, session_key: &str) -> Option<String> {
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

pub(crate) fn user_documents_from_params(
    params: &Value,
    session_key: &str,
    session_store: &SessionStore,
) -> Option<Vec<UserDocument>> {
    let documents = params.get("_document_files")?.as_array()?;
    let media_dir_key = SessionStore::key_to_filename(session_key);
    let mut parsed = Vec::new();

    for document in documents {
        let Ok(document) = serde_json::from_value::<InputChannelDocumentFile>(document.clone())
        else {
            continue;
        };
        let stored_filename = document.stored_filename.trim();
        let mime_type = document.mime_type.trim();
        if !is_safe_user_audio_filename(stored_filename) || mime_type.is_empty() {
            continue;
        }

        let display_name = sanitize_user_document_display_name(&document.display_name)
            .unwrap_or_else(|| stored_filename.to_string());
        parsed.push(UserDocument {
            display_name,
            stored_filename: stored_filename.to_string(),
            mime_type: mime_type.to_string(),
            media_ref: format!("media/{media_dir_key}/{stored_filename}"),
            absolute_path: Some(
                session_store
                    .media_path_for(session_key, stored_filename)
                    .to_string_lossy()
                    .to_string(),
            ),
        });
    }

    if parsed.is_empty() {
        None
    } else {
        Some(parsed)
    }
}

pub(crate) fn user_documents_for_persistence(
    documents: &[UserDocument],
) -> Option<Vec<UserDocument>> {
    if documents.is_empty() {
        return None;
    }

    Some(
        documents
            .iter()
            .cloned()
            .map(|mut document| {
                document.absolute_path = None;
                document
            })
            .collect(),
    )
}
