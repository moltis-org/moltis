//! `ChatService` trait implementation for `LiveChatService`.

use std::{sync::Arc, time::Duration};

use {
    serde_json::Value,
    tokio::sync::OwnedSemaphorePermit,
    tracing::{debug, info, warn},
};

use {moltis_config::MessageQueueMode, moltis_service_traits::ServiceResult};

#[cfg(feature = "local-llm")]
use moltis_providers::model_id::raw_model_id;

use crate::{
    agent_loop::run_explicit_shell_command,
    channels::deliver_channel_error,
    message::{
        apply_message_received_rewrite, infer_reply_medium, to_user_content,
        user_audio_path_from_params, user_documents_for_persistence, user_documents_from_params,
    },
    prompt::{
        apply_request_runtime_context, build_prompt_runtime_context, discover_skills_if_enabled,
        load_prompt_persona_for_agent, load_prompt_persona_for_session,
        resolve_channel_runtime_context, resolve_prompt_agent_id,
    },
    run_with_tools::run_with_tools,
    streaming::run_streaming,
    types::*,
};

use {super::*, crate::service::build_persisted_assistant_message};

use {crate::memory_tools::AgentScopedMemoryWriter, moltis_agents::model::values_to_chat_messages};

impl LiveChatService {
    #[tracing::instrument(skip(self, params), fields(session_id))]
    pub(super) async fn send_impl(&self, mut params: Value) -> ServiceResult {
        // Support both text-only and multimodal content.
        // - "text": string → plain text message
        // - "content": array → multimodal content (text + images)
        //
        // Note: `text` and `message_content` are `mut` because a
        // `MessageReceived` hook may return `ModifyPayload` to rewrite the
        // inbound message before the turn begins (see GH #639).
        let (mut text, mut message_content) = if let Some(content) = params.get("content") {
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

        // Resolve session key from explicit overrides, public request params, or connection context.
        let session_key = self.resolve_session_key_from_params(&params).await;
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
            let user_documents =
                user_documents_from_params(&params, &session_key, self.session_store.as_ref());
            let user_msg = PersistedMessage::User {
                content: message_content,
                created_at: Some(now_ms()),
                audio: user_audio,
                documents: user_documents
                    .as_deref()
                    .and_then(user_documents_for_persistence),
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
                        target.thread_id.as_deref(),
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
            let active_partial_assistant = Arc::clone(&self.active_partial_assistant);
            let active_reply_medium = Arc::clone(&self.active_reply_medium);
            let terminal_runs = Arc::clone(&self.terminal_runs);
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
                    &terminal_runs,
                    &session_key_clone,
                    &shell_command,
                    user_message_index,
                    accept_language,
                    conn_id_for_tool,
                    client_seq,
                )
                .await;

                let assistant_msg = build_persisted_assistant_message(
                    assistant_output,
                    None,
                    None,
                    client_seq,
                    Some(run_id_clone.clone()),
                );
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
                terminal_runs.write().await.remove(&run_id_clone);
                active_partial_assistant
                    .write()
                    .await
                    .remove(&session_key_clone);
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

        // Generate run_id early so we can link the user message to its agent run.
        let run_id = uuid::Uuid::new_v4().to_string();

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
                    target.thread_id.as_deref(),
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

        // Dispatch the `MessageReceived` hook before the turn starts. The
        // hook can:
        //   - return `Continue` → proceed normally;
        //   - return `ModifyPayload({"content": "..."})` → rewrite the
        //     inbound text before it is persisted or sent to the model;
        //   - return `Block(reason)` → abort this turn entirely. The user
        //     message is NOT persisted, no run is started, and the reason
        //     is surfaced to the channel/web sender.
        //
        // Hook errors are treated as fail-open: a broken hook must not be
        // able to wedge every inbound message. See GH #639.
        if let Some(ref hooks) = self.hook_registry {
            let session_entry = self.session_metadata.get(&session_key).await;
            let channel = params
                .get("channel")
                .and_then(|v| v.as_str())
                .map(String::from);
            let channel_binding = Some(resolve_channel_runtime_context(
                &session_key,
                session_entry.as_ref(),
            ))
            .filter(|binding| !binding.is_empty());
            let payload = moltis_common::hooks::HookPayload::MessageReceived {
                session_key: session_key.clone(),
                content: text.clone(),
                channel,
                channel_binding,
            };
            match hooks.dispatch(&payload).await {
                Ok(moltis_common::hooks::HookAction::Continue) => {},
                Ok(moltis_common::hooks::HookAction::ModifyPayload(new_payload)) => {
                    match new_payload.get("content").and_then(|v| v.as_str()) {
                        Some(new_text) => {
                            info!(
                                session = %session_key,
                                "MessageReceived hook rewrote inbound content"
                            );
                            text = new_text.to_string();
                            apply_message_received_rewrite(
                                &mut message_content,
                                &mut params,
                                new_text,
                            );
                        },
                        None => {
                            warn!(
                                session = %session_key,
                                "MessageReceived hook ModifyPayload ignored: expected object with `content` string"
                            );
                        },
                    }
                },
                Ok(moltis_common::hooks::HookAction::Block(reason)) => {
                    info!(
                        session = %session_key,
                        reason = %reason,
                        "MessageReceived hook blocked inbound message"
                    );

                    // Surface the rejection to channel senders via the
                    // existing channel-error delivery path. If the caller
                    // attached a reply target (web-UI-on-bound-session or an
                    // inbound channel message), re-register it so
                    // `deliver_channel_error` has a destination to drain.
                    if let Some(target) = deferred_channel_target.clone() {
                        self.state.push_channel_reply(&session_key, target).await;
                        let error_obj = serde_json::json!({
                            "type": "message_rejected",
                            "message": reason,
                        });
                        deliver_channel_error(&self.state, &session_key, &error_obj).await;
                    }

                    // Broadcast a rejection event so web UI clients see it.
                    broadcast(
                        &self.state,
                        "chat",
                        serde_json::json!({
                            "state": "rejected",
                            "sessionKey": session_key,
                            "reason": reason,
                        }),
                        BroadcastOpts::default(),
                    )
                    .await;

                    return Ok(serde_json::json!({
                        "ok": false,
                        "rejected": true,
                        "reason": reason,
                    }));
                },
                Err(e) => {
                    warn!(
                        session = %session_key,
                        error = %e,
                        "MessageReceived hook failed; proceeding fail-open"
                    );
                },
            }
        }

        // Convert session-crate content to agents-crate content for the LLM.
        // Must happen before `message_content` is moved into `user_msg`, and
        // must happen AFTER the MessageReceived hook dispatch so a
        // `ModifyPayload` rewrite is reflected in both `user_content` (what
        // the LLM sees) and `user_msg` (what gets persisted).
        let user_documents =
            user_documents_from_params(&params, &session_key, self.session_store.as_ref())
                .unwrap_or_default();
        let user_content = to_user_content(&message_content, &user_documents);

        // Build the user message for later persistence (deferred until we
        // know the message won't be queued — avoids double-persist when a
        // queued message is replayed via send()).
        let channel_meta = params.get("channel").cloned();
        // Extract sender name from channel metadata for LLM identity.
        let sender_name = channel_meta
            .as_ref()
            .and_then(|ch| {
                ch["sender_name"]
                    .as_str()
                    .or_else(|| ch["username"].as_str())
            })
            .map(|s| s.to_string());
        let user_audio = user_audio_path_from_params(&params, &session_key);
        let user_msg = PersistedMessage::User {
            content: message_content,
            created_at: Some(now_ms()),
            audio: user_audio,
            documents: user_documents_for_persistence(&user_documents),
            channel: channel_meta,
            seq: client_seq,
            run_id: Some(run_id.clone()),
        };

        // Discover enabled skills/plugins for prompt injection (gated on
        // `[skills] enabled` — see #655).
        let discovered_skills =
            discover_skills_if_enabled(&moltis_config::discover_and_load()).await;

        // Check if MCP tools are disabled for this session and capture
        // per-session sandbox override details for prompt runtime context.
        let session_entry = self.session_metadata.get(&session_key).await;
        let session_agent_id = resolve_prompt_agent_id(session_entry.as_ref());
        let persona = load_prompt_persona_for_session(
            &session_key,
            session_entry.as_ref(),
            self.session_state_store.as_deref(),
        )
        .await;
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
        let active_partial_assistant = Arc::clone(&self.active_partial_assistant);
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
        // Auto-compact when the next request is likely to exceed
        // `chat.compaction.threshold_percent` of the model context window.
        // The value is clamped to the 0.1–0.95 range in case config
        // validation missed a typo; the default (0.95) is loaded via
        // load_prompt_persona_for_agent for the session's agent and
        // matches the pre-PR-#653 hardcoded trigger.
        let compaction_cfg = &load_prompt_persona_for_agent(&session_agent_id)
            .config
            .chat
            .compaction;
        let context_window = provider.context_window() as u64;
        let token_usage = session_token_usage_from_messages(&history);
        let estimated_next_input = token_usage
            .current_request_input_tokens
            .saturating_add(estimate_text_tokens(&text));
        let compact_threshold =
            compute_auto_compact_threshold(context_window, compaction_cfg.threshold_percent);

        if estimated_next_input >= compact_threshold {
            let pre_compact_msg_count = history.len();
            let pre_compact_total = token_usage
                .current_request_input_tokens
                .saturating_add(token_usage.current_request_output_tokens);

            info!(
                session = %session_key,
                estimated_next_input,
                context_window,
                threshold_percent = compaction_cfg.threshold_percent,
                compact_threshold,
                "auto-compact triggered (estimated next request over chat.compaction.threshold_percent)"
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

            let compact_params = serde_json::json!({ "_session_key": &session_key });
            match self.compact(compact_params).await {
                Ok(_) => {
                    // Reload history after compaction.
                    history = self
                        .session_store
                        .read(&session_key)
                        .await
                        .unwrap_or_default();
                    // This `auto_compact done` event is a lifecycle
                    // signal for subscribers that pre-emptive
                    // auto-compact finished. The mode/token metadata
                    // lives on the `chat.compact done` event that
                    // `self.compact()` broadcasts from the inside —
                    // the `compactBroadcastPath: "inner"` marker below
                    // lets hook / webhook consumers detect that and
                    // subscribe to that event instead. The parallel
                    // `run_with_tools` context-overflow path emits a
                    // self-contained `auto_compact done` (with
                    // `compactBroadcastPath: "wrapper"`) that carries
                    // the metadata directly.
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
                            "compactBroadcastPath": "inner",
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

        // Broadcast a user_message event so that other connected clients
        // (e.g. the web UI when the message was sent via the GraphQL API)
        // can display the message in real-time without a page reload.
        // We intentionally omit messageIndex (same rationale as
        // channel_user in dispatch.rs) and include `seq` so that the
        // originating web client can suppress the echo it already
        // rendered optimistically.
        broadcast(
            &self.state,
            "chat",
            serde_json::json!({
                "state": "user_message",
                "text": text,
                "sessionKey": session_key,
                "seq": client_seq,
            }),
            BroadcastOpts::default(),
        )
        .await;

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
        let active_event_forwarders = Arc::clone(&self.active_event_forwarders);
        let terminal_runs = Arc::clone(&self.terminal_runs);
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
            active_partial_assistant.write().await.insert(
                session_key_clone.clone(),
                ActiveAssistantDraft::new(&run_id_clone, &model_id, &provider_name, client_seq),
            );
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
            // Clone the provider for potential periodic memory extraction
            // (the original Arc is moved into run_with_tools / run_streaming).
            let provider_for_extraction = Arc::clone(&provider);
            // Capture config values before persona is moved into the agent future.
            let auto_extract_interval = persona.config.memory.auto_extract_interval;
            let extraction_write_mode = persona.config.memory.agent_write_mode;
            let agent_fut = async {
                if stream_only {
                    run_streaming(
                        persona,
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
                        sender_name,
                        Some(&session_store),
                        client_seq,
                        Some(Arc::clone(&active_partial_assistant)),
                        &terminal_runs,
                    )
                    .await
                } else {
                    run_with_tools(
                        persona,
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
                        Some(Arc::clone(&active_partial_assistant)),
                        &active_event_forwarders,
                        &terminal_runs,
                        sender_name,
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
                        state.set_run_error(&run_id_clone, detail.clone()).await;
                        deliver_channel_error(&state, &session_key_clone, &error_obj).await;
                        terminal_runs.write().await.insert(run_id_clone.clone());
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
                let assistant_msg = build_persisted_assistant_message(
                    assistant_output,
                    Some(model_id.clone()),
                    Some(provider_name.clone()),
                    client_seq,
                    Some(run_id_clone.clone()),
                );
                if let Err(e) = session_store
                    .append(&session_key_clone, &assistant_msg.to_value())
                    .await
                {
                    warn!("failed to persist assistant message: {e}");
                }
                // Update metadata counts.
                if let Ok(count) = session_store.count(&session_key_clone).await {
                    session_metadata.touch(&session_key_clone, count).await;

                    // ── Periodic background memory extraction ──────────────
                    // Every `auto_extract_interval` turns, spawn a background
                    // silent turn to save important recent context to memory.
                    // Uses config values captured before persona was moved.
                    let interval = auto_extract_interval;
                    let write_mode = extraction_write_mode;
                    // A "turn" = user + assistant = 2 messages.
                    let turn_number = count / 2;
                    if interval > 0
                        && turn_number > 0
                        && turn_number % interval == 0
                        && !stream_only
                        && memory_write_mode_allows_save(write_mode)
                        && let Some(mm) = state.memory_manager()
                    {
                        let window = (interval as usize) * 2;
                        let recent: Vec<serde_json::Value> =
                            if let Ok(h) = session_store.read(&session_key_clone).await {
                                h.into_iter()
                                    .rev()
                                    .take(window)
                                    .collect::<Vec<_>>()
                                    .into_iter()
                                    .rev()
                                    .collect()
                            } else {
                                Vec::new()
                            };
                        if !recent.is_empty() {
                            let chat_msgs = values_to_chat_messages(&recent);
                            let agent_id = session_agent_id_clone.clone();
                            let mm = Arc::clone(mm);
                            let prov = Arc::clone(&provider_for_extraction);
                            tokio::spawn(async move {
                                let writer: Arc<dyn moltis_agents::memory_writer::MemoryWriter> =
                                    Arc::new(AgentScopedMemoryWriter::new(
                                        mm, agent_id, write_mode,
                                    ));
                                match moltis_agents::silent_turn::run_silent_memory_turn_with_prompt(
                                        prov,
                                        &chat_msgs,
                                        writer,
                                        moltis_agents::silent_turn::SilentTurnPrompt::PeriodicExtract,
                                    )
                                    .await
                                    {
                                        Ok(paths) if !paths.is_empty() => {
                                            tracing::info!(
                                                files = paths.len(),
                                                turn = turn_number,
                                                "periodic memory extraction: wrote files"
                                            );
                                        },
                                        Ok(_) => {},
                                        Err(e) => {
                                            tracing::warn!(
                                                error = %e,
                                                "periodic memory extraction failed"
                                            );
                                        },
                                    }
                            });
                        }
                    }
                }
            }

            let _ = LiveChatService::wait_for_event_forwarder(
                &active_event_forwarders,
                &session_key_clone,
            )
            .await;

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
            terminal_runs.write().await.remove(&run_id_clone);
            active_partial_assistant
                .write()
                .await
                .remove(&session_key_clone);
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
}
