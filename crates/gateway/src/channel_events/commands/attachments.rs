use std::sync::Arc;

use tracing::{debug, error, warn};

use moltis_channels::{ChannelAttachment, ChannelMessageMeta, ChannelReplyTarget};

use crate::{
    broadcast::{BroadcastOpts, broadcast},
    state::GatewayState,
};

use super::super::{
    default_channel_session_key, resolve_channel_agent_id, resolve_channel_session,
    start_channel_typing_loop,
};

pub(in crate::channel_events) async fn dispatch_to_chat_with_attachments(
    state: &Arc<tokio::sync::OnceCell<Arc<GatewayState>>>,
    text: &str,
    attachments: Vec<ChannelAttachment>,
    reply_to: ChannelReplyTarget,
    meta: ChannelMessageMeta,
) {
    if attachments.is_empty() {
        // No attachments, use the regular dispatch
        super::super::dispatch::dispatch_to_chat(state, text, reply_to, meta).await;
        return;
    }

    let Some(state) = state.get() else {
        warn!("channel dispatch_to_chat_with_attachments: gateway not ready");
        return;
    };

    // Start typing immediately so image preprocessing/session setup doesn't
    // delay channel feedback.
    let typing_done = start_channel_typing_loop(state, &reply_to);

    let session_key = if let Some(ref sm) = state.services.session_metadata {
        resolve_channel_session(&reply_to, sm).await
    } else {
        default_channel_session_key(&reply_to)
    };

    // Build multimodal content array (OpenAI format)
    let mut content_parts: Vec<serde_json::Value> = Vec::new();

    // Add text part if not empty
    if !text.is_empty() {
        content_parts.push(serde_json::json!({
            "type": "text",
            "text": text,
        }));
    }

    // Add image parts
    for attachment in &attachments {
        let base64_data =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &attachment.data);
        let data_uri = format!("data:{};base64,{}", attachment.media_type, base64_data);
        content_parts.push(serde_json::json!({
            "type": "image_url",
            "image_url": {
                "url": data_uri,
            },
        }));
    }

    debug!(
        session_key = %session_key,
        text_len = text.len(),
        attachment_count = attachments.len(),
        "dispatching multimodal message to chat"
    );

    // Broadcast a "chat" event so the web UI shows the user message.
    // See the text-only dispatch above for why messageIndex is omitted.
    let payload = serde_json::json!({
        "state": "channel_user",
        "text": if text.is_empty() { "[Image]" } else { text },
        "channel": &meta,
        "sessionKey": &session_key,
        "hasAttachments": true,
    });
    broadcast(state, "chat", payload, BroadcastOpts {
        drop_if_slow: true,
        ..Default::default()
    })
    .await;

    // Persist channel binding (ensure session row exists first --
    // set_channel_binding is an UPDATE so the row must already be present).
    if let Ok(binding_json) = serde_json::to_string(&reply_to)
        && let Some(ref session_meta) = state.services.session_metadata
    {
        let entry = session_meta.get(&session_key).await;
        if entry.as_ref().is_none_or(|e| e.channel_binding.is_none()) {
            let existing = session_meta
                .list_channel_sessions(
                    reply_to.channel_type.as_str(),
                    &reply_to.account_id,
                    &reply_to.chat_id,
                )
                .await;
            let n = existing.len() + 1;
            let _ = session_meta
                .upsert(
                    &session_key,
                    Some(format!("{} {n}", reply_to.channel_type.display_name())),
                )
                .await;
        }
        session_meta
            .set_channel_binding(&session_key, Some(binding_json))
            .await;
        if let Some(entry) = session_meta.get(&session_key).await
            && entry
                .agent_id
                .as_deref()
                .map(str::trim)
                .is_none_or(|value| value.is_empty())
        {
            let default_agent =
                resolve_channel_agent_id(state, &session_key, meta.agent_id.as_deref()).await;
            let _ = session_meta
                .set_agent_id(&session_key, Some(&default_agent))
                .await;
        }
    }

    // Channel platforms do not expose bot read receipts. Use inbound
    // user activity as a heuristic and mark prior session history seen.
    state.services.session.mark_seen(&session_key).await;

    let chat = state.chat().await;
    let mut params = serde_json::json!({
        "content": content_parts,
        "channel": &meta,
        "_session_key": &session_key,
        // Defer reply-target registration until chat.send() actually
        // starts executing this message (after semaphore acquire).
        "_channel_reply_target": &reply_to,
    });
    if let Some(ref documents) = meta.documents {
        params["_document_files"] = serde_json::json!(documents);
    }

    // Forward the channel's default model if configured
    if let Some(ref model) = meta.model {
        params["model"] = serde_json::json!(model);

        let session_has_model = if let Some(ref sm) = state.services.session_metadata {
            sm.get(&session_key).await.and_then(|e| e.model).is_some()
        } else {
            false
        };
        if !session_has_model {
            let _ = state
                .services
                .session
                .patch(serde_json::json!({
                    "key": &session_key,
                    "model": model,
                }))
                .await;

            let display: String = if let Ok(models_val) = state.services.model.list().await
                && let Some(models) = models_val.as_array()
            {
                models
                    .iter()
                    .find(|m| m.get("id").and_then(|v| v.as_str()) == Some(model))
                    .and_then(|m| m.get("displayName").and_then(|v| v.as_str()))
                    .unwrap_or(model)
                    .to_string()
            } else {
                model.clone()
            };
            let msg = format!("Using {display}. Use /model to change.");
            state.push_channel_status_log(&session_key, msg).await;
        }
    } else {
        let session_has_model = if let Some(ref sm) = state.services.session_metadata {
            sm.get(&session_key).await.and_then(|e| e.model).is_some()
        } else {
            false
        };
        if !session_has_model
            && let Ok(models_val) = state.services.model.list().await
            && let Some(models) = models_val.as_array()
            && let Some(first) = models.first()
            && let Some(id) = first.get("id").and_then(|v| v.as_str())
        {
            params["model"] = serde_json::json!(id);
            let _ = state
                .services
                .session
                .patch(serde_json::json!({
                    "key": &session_key,
                    "model": id,
                }))
                .await;

            let display = first
                .get("displayName")
                .and_then(|v| v.as_str())
                .unwrap_or(id);
            let msg = format!("Using {display}. Use /model to change.");
            state.push_channel_status_log(&session_key, msg).await;
        }
    }

    let send_result = chat.send(params).await;
    if let Some(done_tx) = typing_done {
        let _ = done_tx.send(());
    }

    if let Err(e) = send_result {
        error!("channel dispatch_to_chat_with_attachments failed: {e}");
        if let Some(outbound) = state.services.channel_outbound_arc() {
            let error_msg = format!("\u{26a0}\u{fe0f} {e}");
            if let Err(send_err) = outbound
                .send_text(
                    &reply_to.account_id,
                    &reply_to.outbound_to(),
                    &error_msg,
                    reply_to.message_id.as_deref(),
                )
                .await
            {
                warn!("failed to send error back to channel: {send_err}");
            }
        }
    }
}
