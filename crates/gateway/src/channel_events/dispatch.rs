use super::*;

pub(in crate::channel_events) async fn dispatch_to_chat(
    state: &Arc<tokio::sync::OnceCell<Arc<GatewayState>>>,
    text: &str,
    reply_to: ChannelReplyTarget,
    meta: ChannelMessageMeta,
) {
    if let Some(state) = state.get() {
        // Start typing immediately so pre-run setup (session/model resolution)
        // does not delay channel feedback.
        let typing_done = start_channel_typing_loop(state, &reply_to);

        let session_key = if let Some(ref sm) = state.services.session_metadata {
            resolve_channel_session(&reply_to, sm).await
        } else {
            default_channel_session_key(&reply_to)
        };
        let effective_text = if state.is_channel_command_mode_enabled(&session_key).await {
            rewrite_for_shell_mode(text).unwrap_or_else(|| text.to_string())
        } else {
            text.to_string()
        };

        // Broadcast a "chat" event so the web UI shows the user message
        // in real-time (like typing from the UI).
        //
        // We intentionally omit `messageIndex` here: the broadcast fires
        // *before* chat.send() persists the message, so store.count()
        // would be stale.  Concurrent channel messages would get the same
        // index, causing the client-side dedup to drop the second one.
        // Without a messageIndex the client skips its dedup check and
        // always renders the message.
        let payload = serde_json::json!({
            "state": "channel_user",
            "text": text,
            "channel": &meta,
            "sessionKey": &session_key,
        });
        broadcast(state, "chat", payload, BroadcastOpts {
            drop_if_slow: false,
            ..Default::default()
        })
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

        // If the message is a thread reply, fetch prior thread messages
        // for context injection so the LLM sees the conversation history.
        let thread_context = if let Some(ref thread_id) = reply_to.message_id
            && let Some(ref reg) = state.services.channel_registry
        {
            match reg
                .fetch_thread_messages(&reply_to.account_id, &reply_to.chat_id, thread_id, 20)
                .await
            {
                Ok(msgs) if !msgs.is_empty() => {
                    let history: Vec<serde_json::Value> = msgs
                        .iter()
                        .map(|m| {
                            serde_json::json!({
                                "role": if m.is_bot { "assistant" } else { "user" },
                                "text": m.text,
                                "sender_id": m.sender_id,
                                "timestamp": m.timestamp,
                            })
                        })
                        .collect();
                    Some(history)
                },
                Ok(_) => None,
                Err(e) => {
                    debug!("failed to fetch thread context: {e}");
                    None
                },
            }
        } else {
            None
        };

        let chat = state.chat().await;
        let mut params = serde_json::json!({
            "text": effective_text,
            "channel": &meta,
            "_session_key": &session_key,
            // Defer reply-target registration until chat.send() actually
            // starts executing this message (after semaphore acquire).
            "_channel_reply_target": &reply_to,
        });

        // Attach thread context if available.
        if let Some(thread_history) = thread_context {
            params["_thread_context"] = serde_json::json!(thread_history);
        }
        // Thread saved voice audio filename so chat.rs persists the audio path.
        if let Some(ref audio_filename) = meta.audio_filename {
            params["_audio_filename"] = serde_json::json!(audio_filename);
        }
        if let Some(ref documents) = meta.documents {
            params["_document_files"] = serde_json::json!(documents);
        }

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

        let send_result = chat.send(params).await;
        if let Some(done_tx) = typing_done {
            let _ = done_tx.send(());
        }

        if let Err(e) = send_result {
            error!("channel dispatch_to_chat failed: {e}");
            // Send the error back to the originating channel so the user
            // knows something went wrong.
            if let Some(outbound) = state.services.channel_outbound_arc() {
                let error_msg = format!("⚠️ {e}");
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
    } else {
        warn!("channel dispatch_to_chat: gateway not ready");
    }
}
