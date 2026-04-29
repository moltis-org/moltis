use crate::multimodal::parse_data_uri;

use super::{
    chat::{ChatMessage, ContentPart, UserContent},
    decode_tool_call_arguments,
    types::{TOOL_CALL_METADATA_KEYS, ToolCall},
};

/// Extract allowlisted metadata keys from a tool-call JSON object.
///
/// Returns `None` when no metadata keys are present, keeping the common path
/// allocation-free.
#[must_use]
pub fn extract_tool_call_metadata(
    tc: &serde_json::Value,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    let obj = tc.as_object()?;
    let nested = obj.get("metadata").and_then(serde_json::Value::as_object);
    let meta: serde_json::Map<_, _> = TOOL_CALL_METADATA_KEYS
        .iter()
        .filter_map(|&k| {
            obj.get(k)
                .or_else(|| nested.and_then(|metadata| metadata.get(k)))
                .map(|v| (k.to_string(), v.clone()))
        })
        .collect();
    if meta.is_empty() {
        None
    } else {
        Some(meta)
    }
}

fn document_absolute_path_from_media_ref(media_ref: &str) -> String {
    use std::path::Path;
    if Path::new(media_ref).is_absolute() {
        return media_ref.to_string();
    }

    moltis_config::data_dir()
        .join("sessions")
        .join(media_ref)
        .to_string_lossy()
        .to_string()
}

/// Convert persisted JSON messages (from session store) to typed `ChatMessage`s.
///
/// Skips messages that don't have a valid `role` field, logging a warning.
/// Metadata fields (`created_at`, `model`, `provider`, `inputTokens`,
/// `outputTokens`, `channel`) are silently dropped — they only exist in
/// the persisted JSON, not in `ChatMessage`.
pub fn values_to_chat_messages(values: &[serde_json::Value]) -> Vec<ChatMessage> {
    let mut messages = Vec::with_capacity(values.len());
    // Track tool_call IDs emitted by assistant messages so we only include
    // tool/tool_result messages that have a matching assistant tool_call.
    // Orphan tool results (e.g. from explicit /sh commands) would cause
    // provider API errors.
    let mut pending_tool_call_ids: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for (i, val) in values.iter().enumerate() {
        let Some(role) = val["role"].as_str() else {
            tracing::warn!(index = i, "skipping message with missing/invalid role");
            continue;
        };
        match role {
            "system" => {
                let content = val["content"].as_str().unwrap_or("").to_string();
                messages.push(ChatMessage::system(content));
            },
            "user" => {
                // Extract sender name from persisted channel metadata.
                let sender_name = val
                    .get("channel")
                    .and_then(|ch| {
                        ch["sender_name"]
                            .as_str()
                            .or_else(|| ch["username"].as_str())
                    })
                    .map(|s| s.to_string());

                let document_context = val["documents"].as_array().and_then(|documents| {
                    let mut sections = Vec::new();
                    for document in documents {
                        let Some(display_name) = document["display_name"].as_str() else {
                            continue;
                        };
                        let Some(mime_type) = document["mime_type"].as_str() else {
                            continue;
                        };
                        let Some(media_ref) = document["media_ref"].as_str() else {
                            continue;
                        };
                        let absolute_path = document_absolute_path_from_media_ref(media_ref);
                        sections.push(format!(
                            "filename: {display_name}\nmime_type: {mime_type}\nlocal_path: {absolute_path}\nmedia_ref: {media_ref}"
                        ));
                    }
                    if sections.is_empty() {
                        None
                    } else {
                        let mut rendered = vec!["[Inbound documents available]".to_string()];
                        rendered.extend(sections);
                        Some(rendered.join("\n\n"))
                    }
                });

                // Content can be a string or an array (multimodal).
                if let Some(text) = val["content"].as_str() {
                    let content = if let Some(ref document_context) = document_context {
                        if text.trim().is_empty() {
                            document_context.clone()
                        } else {
                            format!("{text}\n\n{document_context}")
                        }
                    } else {
                        text.to_string()
                    };
                    messages.push(ChatMessage::User {
                        content: UserContent::Text(content),
                        name: sender_name,
                    });
                } else if let Some(arr) = val["content"].as_array() {
                    let mut parts: Vec<ContentPart> = arr
                        .iter()
                        .filter_map(|block| {
                            let block_type = block["type"].as_str()?;
                            match block_type {
                                "text" => {
                                    let text = block["text"].as_str()?.to_string();
                                    Some(ContentPart::Text(text))
                                },
                                "image_url" => {
                                    let url = block["image_url"]["url"].as_str()?;
                                    let (media_type, data) = parse_data_uri(url)?;
                                    Some(ContentPart::Image {
                                        media_type: media_type.to_string(),
                                        data: data.to_string(),
                                    })
                                },
                                _ => None,
                            }
                        })
                        .collect();
                    if let Some(document_context) = document_context {
                        if let Some(ContentPart::Text(text)) = parts
                            .iter_mut()
                            .find(|part| matches!(part, ContentPart::Text(_)))
                        {
                            if !text.trim().is_empty() {
                                text.push_str("\n\n");
                            }
                            text.push_str(&document_context);
                        } else {
                            parts.insert(0, ContentPart::Text(document_context));
                        }
                    }
                    messages.push(ChatMessage::User {
                        content: UserContent::Multimodal(parts),
                        name: sender_name,
                    });
                } else {
                    messages.push(ChatMessage::User {
                        content: UserContent::Text(document_context.unwrap_or_default()),
                        name: sender_name,
                    });
                }
            },
            "assistant" => {
                let content = val["content"].as_str().map(|s| s.to_string());
                let tool_calls: Vec<ToolCall> = val["tool_calls"]
                    .as_array()
                    .map(|tcs| {
                        tcs.iter()
                            .filter_map(|tc| {
                                let id = tc["id"].as_str()?.to_string();
                                let name = tc["function"]["name"].as_str()?.to_string();
                                let arguments =
                                    decode_tool_call_arguments(tc["function"].get("arguments"));
                                let metadata = extract_tool_call_metadata(tc);
                                Some(ToolCall {
                                    id,
                                    name,
                                    arguments,
                                    metadata,
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                for tc in &tool_calls {
                    pending_tool_call_ids.insert(tc.id.clone());
                }
                messages.push(ChatMessage::Assistant {
                    content,
                    tool_calls,
                });
            },
            "tool" => {
                let tool_call_id = val["tool_call_id"].as_str().unwrap_or("").to_string();
                if !pending_tool_call_ids.remove(&tool_call_id) {
                    tracing::debug!(tool_call_id, "skipping orphan tool message");
                    continue;
                }
                let content = if let Some(s) = val["content"].as_str() {
                    s.to_string()
                } else {
                    val["content"].to_string()
                };
                messages.push(ChatMessage::tool(tool_call_id, content));
            },
            // tool_result entries are persisted tool execution output; convert
            // them to standard tool messages so the LLM sees its own results.
            "tool_result" => {
                let tool_call_id = val["tool_call_id"].as_str().unwrap_or("").to_string();
                if !pending_tool_call_ids.remove(&tool_call_id) {
                    tracing::debug!(tool_call_id, "skipping orphan tool_result message");
                    continue;
                }
                let content = if let Some(err) = val["error"].as_str() {
                    format!("Error: {err}")
                } else if let Some(result) = val.get("result") {
                    if let Some(s) = result.as_str() {
                        s.to_string()
                    } else {
                        result.to_string()
                    }
                } else {
                    String::new()
                };
                messages.push(ChatMessage::tool(tool_call_id, content));
            },
            // notice entries are UI-only informational messages.
            "notice" => continue,
            other => {
                tracing::warn!(
                    index = i,
                    role = other,
                    "skipping message with unknown role"
                );
            },
        }
    }
    messages
}
