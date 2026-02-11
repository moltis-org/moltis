use std::sync::Arc;

use {
    anyhow::{Result, anyhow},
    async_trait::async_trait,
    moltis_tools::image_cache::ImageBuilder,
    tracing::{debug, error, info, warn},
};

use {
    moltis_channels::{
        ChannelAttachment, ChannelEvent, ChannelEventSink, ChannelMessageMeta, ChannelReplyTarget,
    },
    moltis_sessions::metadata::SqliteSessionMetadata,
};

use crate::{
    broadcast::{BroadcastOpts, broadcast},
    state::GatewayState,
};

/// Default (deterministic) session key for a channel chat.
fn default_channel_session_key(target: &ChannelReplyTarget) -> String {
    format!(
        "{}:{}:{}",
        target.channel_type, target.account_id, target.chat_id
    )
}

/// Resolve the active session key for a channel chat.
/// Uses the forward mapping table if an override exists, otherwise falls back
/// to the deterministic key.
async fn resolve_channel_session(
    target: &ChannelReplyTarget,
    metadata: &SqliteSessionMetadata,
) -> String {
    if let Some(key) = metadata
        .get_active_session(
            target.channel_type.as_str(),
            &target.account_id,
            &target.chat_id,
        )
        .await
    {
        return key;
    }
    default_channel_session_key(target)
}

/// Broadcasts channel events over the gateway WebSocket.
///
/// Uses a deferred `OnceCell` reference so the sink can be created before
/// `GatewayState` exists (same pattern as cron callbacks).
pub struct GatewayChannelEventSink {
    state: Arc<tokio::sync::OnceCell<Arc<GatewayState>>>,
}

impl GatewayChannelEventSink {
    pub fn new(state: Arc<tokio::sync::OnceCell<Arc<GatewayState>>>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl ChannelEventSink for GatewayChannelEventSink {
    async fn emit(&self, event: ChannelEvent) {
        if let Some(state) = self.state.get() {
            let payload = match serde_json::to_value(&event) {
                Ok(v) => v,
                Err(e) => {
                    warn!("failed to serialize channel event: {e}");
                    return;
                },
            };
            broadcast(state, "channel", payload, BroadcastOpts {
                drop_if_slow: true,
                ..Default::default()
            })
            .await;
        }
    }

    async fn dispatch_to_chat(
        &self,
        text: &str,
        reply_to: ChannelReplyTarget,
        meta: ChannelMessageMeta,
    ) {
        if let Some(state) = self.state.get() {
            let session_key = if let Some(ref sm) = state.services.session_metadata {
                resolve_channel_session(&reply_to, sm).await
            } else {
                default_channel_session_key(&reply_to)
            };

            // Broadcast a "chat" event so the web UI shows the user message
            // in real-time (like typing from the UI).
            // Include messageIndex so the client can deduplicate against history.
            let msg_index = if let Some(ref store) = state.services.session_store {
                store.count(&session_key).await.unwrap_or(0)
            } else {
                0
            };
            let payload = serde_json::json!({
                "state": "channel_user",
                "text": text,
                "channel": &meta,
                "sessionKey": &session_key,
                "messageIndex": msg_index,
            });
            broadcast(state, "chat", payload, BroadcastOpts {
                drop_if_slow: true,
                ..Default::default()
            })
            .await;

            // Register the reply target so the chat "final" broadcast can
            // route the response back to the originating channel.
            state
                .push_channel_reply(&session_key, reply_to.clone())
                .await;

            // Persist channel binding so web UI messages on this session
            // can be echoed back to the channel.
            if let Ok(binding_json) = serde_json::to_string(&reply_to)
                && let Some(ref session_meta) = state.services.session_metadata
            {
                // Ensure the session row exists and label it on first use.
                // `set_channel_binding` is an UPDATE, so the row must exist
                // before we can set the binding column.
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
                        .upsert(&session_key, Some(format!("Telegram {n}")))
                        .await;
                }
                session_meta
                    .set_channel_binding(&session_key, Some(binding_json))
                    .await;
            }

            let chat = state.chat().await;
            let mut params = serde_json::json!({
                "text": text,
                "channel": &meta,
                "_session_key": &session_key,
            });
            // Forward the channel's default model to chat.send() if configured.
            // If no channel model is set, check if the session already has a model.
            // If neither exists, assign the first registered model so the session
            // behaves the same as the web UI (which always sends an explicit model).
            if let Some(ref model) = meta.model {
                params["model"] = serde_json::json!(model);

                // Notify the user which model was assigned from the channel config
                // on the first message of a new session (no model set yet).
                let session_has_model = if let Some(ref sm) = state.services.session_metadata {
                    sm.get(&session_key).await.and_then(|e| e.model).is_some()
                } else {
                    false
                };
                if !session_has_model {
                    // Persist channel model on the session.
                    let _ = state
                        .services
                        .session
                        .patch(serde_json::json!({
                            "key": &session_key,
                            "model": model,
                        }))
                        .await;

                    // Buffer model notification for the logbook instead of sending separately.
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

                    // Buffer model notification for the logbook.
                    let display = first
                        .get("displayName")
                        .and_then(|v| v.as_str())
                        .unwrap_or(id);
                    let msg = format!("Using {display}. Use /model to change.");
                    state.push_channel_status_log(&session_key, msg).await;
                }
            }

            // Send a repeating "typing" indicator every 4s until chat.send()
            // completes. Telegram's typing status expires after ~5s.
            let send_result = if let Some(outbound) = state.services.channel_outbound_arc() {
                let (done_tx, mut done_rx) = tokio::sync::oneshot::channel::<()>();
                let account_id = reply_to.account_id.clone();
                let chat_id = reply_to.chat_id.clone();
                tokio::spawn(async move {
                    debug!(
                        account_id = account_id,
                        chat_id = chat_id,
                        "starting typing indicator loop"
                    );
                    loop {
                        if let Err(e) = outbound.send_typing(&account_id, &chat_id).await {
                            debug!(
                                account_id = account_id,
                                chat_id = chat_id,
                                "typing indicator failed: {e}"
                            );
                        } else {
                            debug!(
                                account_id = account_id,
                                chat_id = chat_id,
                                "typing indicator sent"
                            );
                        }
                        tokio::select! {
                            _ = tokio::time::sleep(std::time::Duration::from_secs(4)) => {
                                debug!(
                                    account_id = account_id,
                                    chat_id = chat_id,
                                    "typing loop: 4s elapsed, sending again"
                                );
                            },
                            _ = &mut done_rx => {
                                debug!(
                                    account_id = account_id,
                                    chat_id = chat_id,
                                    "typing loop: chat completed, stopping"
                                );
                                break;
                            },
                        }
                    }
                });
                let result = chat.send(params).await;
                let _ = done_tx.send(());
                result
            } else {
                chat.send(params).await
            };

            if let Err(e) = send_result {
                error!("channel dispatch_to_chat failed: {e}");
                // Send the error back to the originating channel so the user
                // knows something went wrong.
                if let Some(outbound) = state.services.channel_outbound_arc() {
                    let error_msg = format!("⚠️ {e}");
                    if let Err(send_err) = outbound
                        .send_text(
                            &reply_to.account_id,
                            &reply_to.chat_id,
                            &error_msg,
                            reply_to.message_id.as_deref(),
                        )
                        .await
                    {
                        warn!("failed to send error back to channel: {send_err}");
                    }
                }
            }
        } else {
            warn!("channel dispatch_to_chat: gateway not ready");
        }
    }

    async fn request_disable_account(&self, channel_type: &str, account_id: &str, reason: &str) {
        warn!(
            channel_type,
            account_id,
            reason,
            "stopping local polling: detected bot already running on another instance"
        );

        if let Some(state) = self.state.get() {
            // Note: We intentionally do NOT remove the channel from the database.
            // The channel config should remain persisted so other moltis instances
            // sharing the same database can still use it. The polling loop will
            // cancel itself after this call returns.

            // Broadcast an event so the UI can update.
            let channel_type: moltis_channels::ChannelType = match channel_type.parse() {
                Ok(ct) => ct,
                Err(e) => {
                    warn!("request_disable_account: {e}");
                    return;
                },
            };
            let event = ChannelEvent::AccountDisabled {
                channel_type,
                account_id: account_id.to_string(),
                reason: reason.to_string(),
            };
            let payload = match serde_json::to_value(&event) {
                Ok(v) => v,
                Err(e) => {
                    warn!("failed to serialize AccountDisabled event: {e}");
                    return;
                },
            };
            broadcast(state, "channel", payload, BroadcastOpts {
                drop_if_slow: true,
                ..Default::default()
            })
            .await;
        } else {
            warn!("request_disable_account: gateway not ready");
        }
    }

    async fn request_sender_approval(
        &self,
        _channel_type: &str,
        account_id: &str,
        identifier: &str,
    ) {
        if let Some(state) = self.state.get() {
            let params = serde_json::json!({
                "account_id": account_id,
                "identifier": identifier,
            });
            match state.services.channel.sender_approve(params).await {
                Ok(_) => {
                    info!(account_id, identifier, "OTP self-approval: sender approved");
                },
                Err(e) => {
                    warn!(
                        account_id,
                        identifier,
                        error = %e,
                        "OTP self-approval: failed to approve sender"
                    );
                },
            }
        } else {
            warn!("request_sender_approval: gateway not ready");
        }
    }

    async fn transcribe_voice(&self, audio_data: &[u8], format: &str) -> Result<String> {
        let state = self
            .state
            .get()
            .ok_or_else(|| anyhow!("gateway not ready"))?;

        let result = state
            .services
            .stt
            .transcribe_bytes(
                bytes::Bytes::copy_from_slice(audio_data),
                format,
                None,
                None,
                None,
            )
            .await
            .map_err(|e| anyhow!("transcription failed: {}", e))?;

        let text = result
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("transcription result missing text"))?;

        Ok(text.to_string())
    }

    async fn voice_stt_available(&self) -> bool {
        let Some(state) = self.state.get() else {
            return false;
        };

        match state.services.stt.status().await {
            Ok(status) => status
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            Err(_) => false,
        }
    }

    async fn update_location(
        &self,
        reply_to: &ChannelReplyTarget,
        latitude: f64,
        longitude: f64,
    ) -> bool {
        let Some(state) = self.state.get() else {
            warn!("update_location: gateway not ready");
            return false;
        };

        let session_key = if let Some(ref sm) = state.services.session_metadata {
            resolve_channel_session(reply_to, sm).await
        } else {
            default_channel_session_key(reply_to)
        };

        // Update in-memory cache.
        let geo = moltis_config::GeoLocation::now(latitude, longitude, None);
        state.inner.write().await.cached_location = Some(geo.clone());

        // Persist to USER.md (best-effort).
        let mut user = moltis_config::load_user().unwrap_or_default();
        user.location = Some(geo);
        if let Err(e) = moltis_config::save_user(&user) {
            warn!(error = %e, "failed to persist location to USER.md");
        }

        // Check for a pending tool-triggered location request.
        let pending_key = format!("channel_location:{session_key}");
        let pending = state
            .inner
            .write()
            .await
            .pending_invokes
            .remove(&pending_key);
        if let Some(invoke) = pending {
            let result = serde_json::json!({
                "location": {
                    "latitude": latitude,
                    "longitude": longitude,
                    "accuracy": 0.0,
                }
            });
            let _ = invoke.sender.send(result);
            info!(session_key, "resolved pending channel location request");
            return true;
        }

        false
    }

    async fn dispatch_to_chat_with_attachments(
        &self,
        text: &str,
        attachments: Vec<ChannelAttachment>,
        reply_to: ChannelReplyTarget,
        meta: ChannelMessageMeta,
    ) {
        if attachments.is_empty() {
            // No attachments, use the regular dispatch
            self.dispatch_to_chat(text, reply_to, meta).await;
            return;
        }

        let Some(state) = self.state.get() else {
            warn!("channel dispatch_to_chat_with_attachments: gateway not ready");
            return;
        };

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
            let base64_data = base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &attachment.data,
            );
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

        // Broadcast a "chat" event so the web UI shows the user message
        let msg_index = if let Some(ref store) = state.services.session_store {
            store.count(&session_key).await.unwrap_or(0)
        } else {
            0
        };

        // For the broadcast, just show the text portion
        let payload = serde_json::json!({
            "state": "channel_user",
            "text": if text.is_empty() { "[Image]" } else { text },
            "channel": &meta,
            "sessionKey": &session_key,
            "messageIndex": msg_index,
            "hasAttachments": true,
        });
        broadcast(state, "chat", payload, BroadcastOpts {
            drop_if_slow: true,
            ..Default::default()
        })
        .await;

        // Register the reply target
        state
            .push_channel_reply(&session_key, reply_to.clone())
            .await;

        // Persist channel binding (ensure session row exists first —
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
                    .upsert(&session_key, Some(format!("Telegram {n}")))
                    .await;
            }
            session_meta
                .set_channel_binding(&session_key, Some(binding_json))
                .await;
        }

        let chat = state.chat().await;
        let mut params = serde_json::json!({
            "content": content_parts,
            "channel": &meta,
            "_session_key": &session_key,
        });

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

        // Send typing indicator and dispatch to chat
        let send_result = if let Some(outbound) = state.services.channel_outbound_arc() {
            let (done_tx, mut done_rx) = tokio::sync::oneshot::channel::<()>();
            let account_id = reply_to.account_id.clone();
            let chat_id = reply_to.chat_id.clone();
            tokio::spawn(async move {
                loop {
                    if let Err(e) = outbound.send_typing(&account_id, &chat_id).await {
                        debug!(account_id, chat_id, "typing indicator failed: {e}");
                    }
                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_secs(4)) => {},
                        _ = &mut done_rx => break,
                    }
                }
            });
            let result = chat.send(params).await;
            let _ = done_tx.send(());
            result
        } else {
            chat.send(params).await
        };

        if let Err(e) = send_result {
            error!("channel dispatch_to_chat_with_attachments failed: {e}");
            if let Some(outbound) = state.services.channel_outbound_arc() {
                let error_msg = format!("⚠️ {e}");
                if let Err(send_err) = outbound
                    .send_text(
                        &reply_to.account_id,
                        &reply_to.chat_id,
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

    async fn dispatch_command(
        &self,
        command: &str,
        reply_to: ChannelReplyTarget,
    ) -> anyhow::Result<String> {
        let state = self
            .state
            .get()
            .ok_or_else(|| anyhow!("gateway not ready"))?;
        let session_metadata = state
            .services
            .session_metadata
            .as_ref()
            .ok_or_else(|| anyhow!("session metadata not available"))?;
        let session_key = resolve_channel_session(&reply_to, session_metadata).await;
        let chat = state.chat().await;

        // Extract the command name (first word) and args (rest).
        let cmd = command.split_whitespace().next().unwrap_or("");
        let args = command[cmd.len()..].trim();

        match cmd {
            "new" => {
                // Create a new session with a fresh UUID key.
                let new_key = format!("session:{}", uuid::Uuid::new_v4());
                let binding_json = serde_json::to_string(&reply_to)
                    .map_err(|e| anyhow!("failed to serialize binding: {e}"))?;

                // Sequential label: count existing sessions for this chat.
                let existing = session_metadata
                    .list_channel_sessions(
                        reply_to.channel_type.as_str(),
                        &reply_to.account_id,
                        &reply_to.chat_id,
                    )
                    .await;
                let n = existing.len() + 1;

                // Create the new session entry with channel binding.
                session_metadata
                    .upsert(&new_key, Some(format!("Telegram {n}")))
                    .await
                    .map_err(|e| anyhow!("failed to create session: {e}"))?;
                session_metadata
                    .set_channel_binding(&new_key, Some(binding_json.clone()))
                    .await;

                // Ensure the old session also has a channel binding (for listing).
                let old_entry = session_metadata.get(&session_key).await;
                if old_entry
                    .as_ref()
                    .and_then(|e| e.channel_binding.as_ref())
                    .is_none()
                {
                    session_metadata
                        .set_channel_binding(&session_key, Some(binding_json))
                        .await;
                }

                // Update forward mapping.
                session_metadata
                    .set_active_session(
                        reply_to.channel_type.as_str(),
                        &reply_to.account_id,
                        &reply_to.chat_id,
                        &new_key,
                    )
                    .await;

                info!(
                    old_session = %session_key,
                    new_session = %new_key,
                    "channel /new: created new session"
                );

                // Assign a model to the new session: prefer the channel's
                // configured model, fall back to the first registered model.
                let channel_model: Option<String> =
                    state.services.channel.status().await.ok().and_then(|v| {
                        let channels = v.get("channels")?.as_array()?;
                        channels
                            .iter()
                            .find(|ch| {
                                ch.get("account_id").and_then(|v| v.as_str())
                                    == Some(&reply_to.account_id)
                            })
                            .and_then(|ch| {
                                ch.get("config")?.get("model")?.as_str().map(String::from)
                            })
                    });

                let models_val = state.services.model.list().await.ok();
                let models = models_val.as_ref().and_then(|v| v.as_array());

                let (model_id, model_display): (Option<String>, String) = if let Some(ref cm) =
                    channel_model
                {
                    let d = models
                        .and_then(|ms| {
                            ms.iter()
                                .find(|m| m.get("id").and_then(|v| v.as_str()) == Some(cm.as_str()))
                                .and_then(|m| m.get("displayName").and_then(|v| v.as_str()))
                        })
                        .unwrap_or(cm.as_str());
                    (Some(cm.clone()), d.to_string())
                } else if let Some(ms) = models
                    && let Some(first) = ms.first()
                    && let Some(id) = first.get("id").and_then(|v| v.as_str())
                {
                    let d = first
                        .get("displayName")
                        .and_then(|v| v.as_str())
                        .unwrap_or(id);
                    (Some(id.to_string()), d.to_string())
                } else {
                    (None, String::new())
                };

                if let Some(ref mid) = model_id {
                    let _ = state
                        .services
                        .session
                        .patch(serde_json::json!({
                            "key": &new_key,
                            "model": mid,
                        }))
                        .await;
                }

                // Notify web UI so the session list refreshes.
                broadcast(
                    state,
                    "session",
                    serde_json::json!({
                        "kind": "created",
                        "sessionKey": &new_key,
                    }),
                    BroadcastOpts {
                        drop_if_slow: true,
                        ..Default::default()
                    },
                )
                .await;

                if model_display.is_empty() {
                    Ok("New session started.".to_string())
                } else {
                    Ok(format!(
                        "New session started. Using *{model_display}*. Use /model to change."
                    ))
                }
            },
            "clear" => {
                let params = serde_json::json!({ "_session_key": &session_key });
                chat.clear(params).await.map_err(|e| anyhow!("{e}"))?;
                Ok("Session cleared.".to_string())
            },
            "compact" => {
                let params = serde_json::json!({ "_session_key": &session_key });
                chat.compact(params).await.map_err(|e| anyhow!("{e}"))?;
                Ok("Session compacted.".to_string())
            },
            "context" => {
                let params = serde_json::json!({ "_session_key": &session_key });
                let res = chat.context(params).await.map_err(|e| anyhow!("{e}"))?;

                let session_info = res.get("session").cloned().unwrap_or_default();
                let msg_count = session_info
                    .get("messageCount")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let provider = session_info
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let model = session_info
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default");

                let tokens = res.get("tokenUsage").cloned().unwrap_or_default();
                let total = tokens.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
                let context_window = tokens
                    .get("contextWindow")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                // Sandbox section
                let sandbox = res.get("sandbox").cloned().unwrap_or_default();
                let sandbox_enabled = sandbox
                    .get("enabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let sandbox_line = if sandbox_enabled {
                    let image = sandbox
                        .get("image")
                        .and_then(|v| v.as_str())
                        .unwrap_or("default");
                    format!("**Sandbox:** on · `{image}`")
                } else {
                    "**Sandbox:** off".to_string()
                };

                // Skills/plugins section
                let skills = res
                    .get("skills")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let skills_line = if skills.is_empty() {
                    "**Plugins:** none".to_string()
                } else {
                    let names: Vec<_> = skills
                        .iter()
                        .filter_map(|s| s.get("name").and_then(|v| v.as_str()))
                        .collect();
                    format!("**Plugins:** {}", names.join(", "))
                };

                Ok(format!(
                    "**Session:** `{session_key}`\n**Messages:** {msg_count}\n**Provider:** {provider}\n**Model:** `{model}`\n{sandbox_line}\n{skills_line}\n**Tokens:** ~{total}/{context_window}"
                ))
            },
            "sessions" => {
                let sessions = session_metadata
                    .list_channel_sessions(
                        reply_to.channel_type.as_str(),
                        &reply_to.account_id,
                        &reply_to.chat_id,
                    )
                    .await;

                if sessions.is_empty() {
                    return Ok("No sessions found. Send a message to start one.".to_string());
                }

                if args.is_empty() {
                    // List mode.
                    let mut lines = Vec::new();
                    for (i, s) in sessions.iter().enumerate() {
                        let label = s.label.as_deref().unwrap_or(&s.key);
                        let marker = if s.key == session_key {
                            " *"
                        } else {
                            ""
                        };
                        lines.push(format!(
                            "{}. {} ({} msgs){}",
                            i + 1,
                            label,
                            s.message_count,
                            marker,
                        ));
                    }
                    lines.push("\nUse /sessions N to switch.".to_string());
                    Ok(lines.join("\n"))
                } else {
                    // Switch mode.
                    let n: usize = args
                        .parse()
                        .map_err(|_| anyhow!("usage: /sessions [number]"))?;
                    if n == 0 || n > sessions.len() {
                        return Err(anyhow!("invalid session number. Use 1–{}.", sessions.len()));
                    }
                    let target_session = &sessions[n - 1];

                    // Update forward mapping.
                    session_metadata
                        .set_active_session(
                            reply_to.channel_type.as_str(),
                            &reply_to.account_id,
                            &reply_to.chat_id,
                            &target_session.key,
                        )
                        .await;

                    let label = target_session
                        .label
                        .as_deref()
                        .unwrap_or(&target_session.key);
                    info!(
                        session = %target_session.key,
                        "channel /sessions: switched session"
                    );

                    broadcast(
                        state,
                        "session",
                        serde_json::json!({
                            "kind": "switched",
                            "sessionKey": &target_session.key,
                        }),
                        BroadcastOpts {
                            drop_if_slow: true,
                            ..Default::default()
                        },
                    )
                    .await;

                    Ok(format!("Switched to: {label}"))
                }
            },
            "model" => {
                let models_val = state
                    .services
                    .model
                    .list()
                    .await
                    .map_err(|e| anyhow!("{e}"))?;
                let models = models_val
                    .as_array()
                    .ok_or_else(|| anyhow!("bad model list"))?;

                let current_model = {
                    let entry = session_metadata.get(&session_key).await;
                    entry.and_then(|e| e.model.clone())
                };

                if args.is_empty() {
                    // List unique providers.
                    let mut providers: Vec<String> = models
                        .iter()
                        .filter_map(|m| {
                            m.get("provider").and_then(|v| v.as_str()).map(String::from)
                        })
                        .collect();
                    providers.dedup();

                    if providers.len() <= 1 {
                        // Single provider — list models directly.
                        return Ok(format_model_list(models, current_model.as_deref(), None));
                    }

                    // Multiple providers — list them for selection.
                    // Prefix with "providers:" so Telegram handler knows.
                    let current_provider = current_model.as_deref().and_then(|cm| {
                        models.iter().find_map(|m| {
                            let id = m.get("id").and_then(|v| v.as_str())?;
                            if id == cm {
                                m.get("provider").and_then(|v| v.as_str()).map(String::from)
                            } else {
                                None
                            }
                        })
                    });
                    let mut lines = vec!["providers:".to_string()];
                    for (i, p) in providers.iter().enumerate() {
                        let count = models
                            .iter()
                            .filter(|m| m.get("provider").and_then(|v| v.as_str()) == Some(p))
                            .count();
                        let marker = if current_provider.as_deref() == Some(p) {
                            " *"
                        } else {
                            ""
                        };
                        lines.push(format!("{}. {} ({} models){}", i + 1, p, count, marker));
                    }
                    Ok(lines.join("\n"))
                } else if let Some(provider) = args.strip_prefix("provider:") {
                    // List models for a specific provider.
                    Ok(format_model_list(
                        models,
                        current_model.as_deref(),
                        Some(provider),
                    ))
                } else {
                    // Switch mode — arg is a 1-based global index.
                    let n: usize = args
                        .parse()
                        .map_err(|_| anyhow!("usage: /model [number]"))?;
                    if n == 0 || n > models.len() {
                        return Err(anyhow!("invalid model number. Use 1–{}.", models.len()));
                    }
                    let chosen = &models[n - 1];
                    let model_id = chosen
                        .get("id")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| anyhow!("model has no id"))?;
                    let display = chosen
                        .get("displayName")
                        .and_then(|v| v.as_str())
                        .unwrap_or(model_id);

                    let patch_res = state
                        .services
                        .session
                        .patch(serde_json::json!({
                            "key": &session_key,
                            "model": model_id,
                        }))
                        .await
                        .map_err(|e| anyhow!("{e}"))?;
                    let version = patch_res
                        .get("version")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);

                    broadcast(
                        state,
                        "session",
                        serde_json::json!({
                            "kind": "patched",
                            "sessionKey": &session_key,
                            "version": version,
                        }),
                        BroadcastOpts {
                            drop_if_slow: true,
                            ..Default::default()
                        },
                    )
                    .await;

                    Ok(format!("Model switched to: {display}"))
                }
            },
            "sandbox" => {
                let is_enabled = if let Some(ref router) = state.sandbox_router {
                    router.is_sandboxed(&session_key).await
                } else {
                    false
                };

                if args.is_empty() {
                    // Show current status and image list.
                    let current_image = {
                        let entry = session_metadata.get(&session_key).await;
                        let session_img = entry.and_then(|e| e.sandbox_image.clone());
                        match session_img {
                            Some(img) if !img.is_empty() => img,
                            _ => {
                                if let Some(ref router) = state.sandbox_router {
                                    router.default_image().await
                                } else {
                                    moltis_tools::sandbox::DEFAULT_SANDBOX_IMAGE.to_string()
                                }
                            },
                        }
                    };

                    let status = if is_enabled {
                        "on"
                    } else {
                        "off"
                    };

                    // List available images.
                    let builder = moltis_tools::image_cache::DockerImageBuilder::new();
                    let cached = builder.list_cached().await.unwrap_or_default();

                    let default_img = moltis_tools::sandbox::DEFAULT_SANDBOX_IMAGE.to_string();
                    let mut images: Vec<(String, Option<String>)> =
                        vec![(default_img.clone(), None)];
                    for img in &cached {
                        images.push((
                            img.tag.clone(),
                            Some(format!("{} ({})", img.skill_name, img.size)),
                        ));
                    }

                    let mut lines = vec![format!("status:{status}")];
                    for (i, (tag, subtitle)) in images.iter().enumerate() {
                        let marker = if *tag == current_image {
                            " *"
                        } else {
                            ""
                        };
                        let label = if let Some(sub) = subtitle {
                            format!("{}. {} — {}{}", i + 1, tag, sub, marker)
                        } else {
                            format!("{}. {}{}", i + 1, tag, marker)
                        };
                        lines.push(label);
                    }
                    Ok(lines.join("\n"))
                } else if args == "on" || args == "off" {
                    let new_val = args == "on";
                    let patch_res = state
                        .services
                        .session
                        .patch(serde_json::json!({
                            "key": &session_key,
                            "sandbox_enabled": new_val,
                        }))
                        .await
                        .map_err(|e| anyhow!("{e}"))?;
                    let version = patch_res
                        .get("version")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    broadcast(
                        state,
                        "session",
                        serde_json::json!({
                            "kind": "patched",
                            "sessionKey": &session_key,
                            "version": version,
                        }),
                        BroadcastOpts {
                            drop_if_slow: true,
                            ..Default::default()
                        },
                    )
                    .await;
                    let label = if new_val {
                        "enabled"
                    } else {
                        "disabled"
                    };
                    Ok(format!("Sandbox {label}."))
                } else if let Some(rest) = args.strip_prefix("image ") {
                    let n: usize = rest
                        .parse()
                        .map_err(|_| anyhow!("usage: /sandbox image [number]"))?;

                    let default_img = moltis_tools::sandbox::DEFAULT_SANDBOX_IMAGE.to_string();
                    let builder = moltis_tools::image_cache::DockerImageBuilder::new();
                    let cached = builder.list_cached().await.unwrap_or_default();
                    let mut images: Vec<String> = vec![default_img];
                    for img in &cached {
                        images.push(img.tag.clone());
                    }

                    if n == 0 || n > images.len() {
                        return Err(anyhow!("invalid image number. Use 1–{}.", images.len()));
                    }
                    let chosen = &images[n - 1];

                    // If choosing the default image, clear the session override.
                    let patch_value = if n == 1 {
                        ""
                    } else {
                        chosen.as_str()
                    };
                    let patch_res = state
                        .services
                        .session
                        .patch(serde_json::json!({
                            "key": &session_key,
                            "sandbox_image": patch_value,
                        }))
                        .await
                        .map_err(|e| anyhow!("{e}"))?;
                    let version = patch_res
                        .get("version")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);

                    broadcast(
                        state,
                        "session",
                        serde_json::json!({
                            "kind": "patched",
                            "sessionKey": &session_key,
                            "version": version,
                        }),
                        BroadcastOpts {
                            drop_if_slow: true,
                            ..Default::default()
                        },
                    )
                    .await;

                    Ok(format!("Image set to: {chosen}"))
                } else {
                    Err(anyhow!("usage: /sandbox [on|off|image N]"))
                }
            },
            _ => Err(anyhow!("unknown command: /{cmd}")),
        }
    }
}

/// Format a numbered model list, optionally filtered by provider.
///
/// Each line is: `N. DisplayName [provider] *` (where `*` marks the current model).
/// Uses the global index (across all models) so the switch command works with
/// the same numbering regardless of filtering.
fn format_model_list(
    models: &[serde_json::Value],
    current_model: Option<&str>,
    provider_filter: Option<&str>,
) -> String {
    let mut lines = Vec::new();
    for (i, m) in models.iter().enumerate() {
        let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        let provider = m.get("provider").and_then(|v| v.as_str()).unwrap_or("");
        let display = m.get("displayName").and_then(|v| v.as_str()).unwrap_or(id);
        if let Some(filter) = provider_filter
            && provider != filter
        {
            continue;
        }
        let marker = if current_model == Some(id) {
            " *"
        } else {
            ""
        };
        lines.push(format!("{}. {} [{}]{}", i + 1, display, provider, marker));
    }
    lines.join("\n")
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, moltis_channels::ChannelType};

    #[test]
    fn channel_event_serialization() {
        let event = ChannelEvent::InboundMessage {
            channel_type: ChannelType::Telegram,
            account_id: "bot1".into(),
            peer_id: "123".into(),
            username: Some("alice".into()),
            sender_name: Some("Alice".into()),
            message_count: Some(5),
            access_granted: true,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["kind"], "inbound_message");
        assert_eq!(json["channel_type"], "telegram");
        assert_eq!(json["account_id"], "bot1");
        assert_eq!(json["peer_id"], "123");
        assert_eq!(json["username"], "alice");
        assert_eq!(json["sender_name"], "Alice");
        assert_eq!(json["message_count"], 5);
        assert_eq!(json["access_granted"], true);
    }

    #[test]
    fn channel_session_key_format() {
        let target = ChannelReplyTarget {
            channel_type: ChannelType::Telegram,
            account_id: "bot1".into(),
            chat_id: "12345".into(),
            message_id: None,
        };
        assert_eq!(default_channel_session_key(&target), "telegram:bot1:12345");
    }

    #[test]
    fn channel_session_key_group() {
        let target = ChannelReplyTarget {
            channel_type: ChannelType::Telegram,
            account_id: "bot1".into(),
            chat_id: "-100999".into(),
            message_id: None,
        };
        assert_eq!(
            default_channel_session_key(&target),
            "telegram:bot1:-100999"
        );
    }

    #[test]
    fn channel_event_serialization_nulls() {
        let event = ChannelEvent::InboundMessage {
            channel_type: ChannelType::Telegram,
            account_id: "bot1".into(),
            peer_id: "123".into(),
            username: None,
            sender_name: None,
            message_count: None,
            access_granted: false,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["kind"], "inbound_message");
        assert!(json["username"].is_null());
        assert_eq!(json["access_granted"], false);
    }
}
