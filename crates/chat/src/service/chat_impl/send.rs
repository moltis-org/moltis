//! `ChatService` trait implementation for `LiveChatService`.

use std::{sync::Arc, time::Duration};

use {
    serde_json::Value,
    tokio::sync::{OwnedSemaphorePermit, oneshot},
    tracing::{debug, info, warn},
};

use moltis_service_traits::ServiceResult;

#[cfg(feature = "local-llm")]
use moltis_providers::model_id::raw_model_id;

use crate::{
    agent_loop::run_explicit_shell_command,
    channels::deliver_channel_error,
    message::{
        apply_message_received_rewrite, to_user_content, user_audio_path_from_params,
        user_documents_for_persistence, user_documents_from_params,
    },
    prompt::{
        apply_request_runtime_context, build_prompt_runtime_context, discover_skills_if_enabled,
        filter_skills_for_agent, load_prompt_persona_for_session, resolve_channel_runtime_context,
        resolve_prompt_agent_id, resolve_prompt_mode_context,
    },
    run_with_tools::run_with_tools,
    streaming::run_streaming,
    types::*,
};

use {
    super::*,
    crate::service::{
        build_persisted_assistant_message,
        types::{TurnAdmission, commit_successful_turn, commit_terminal_run},
    },
};

use super::{
    queue_drain,
    send_params::{SendParams, parse, turn_result},
};

use {
    crate::memory_tools::AgentScopedMemoryWriter,
    moltis_agents::model::values_to_chat_messages_with_tool_result_limit,
};

impl LiveChatService {
    #[tracing::instrument(skip(self, params), fields(session_id))]
    pub(super) async fn send_impl(
        &self,
        mut params: Value,
        completion: Option<oneshot::Sender<ServiceResult>>,
        queue_if_busy: bool,
    ) -> ServiceResult {
        let SendParams {
            history_limits,
            mut text,
            mut message_content,
            desired_reply_medium,
            conn_id,
            explicit_model,
            tool_controls,
            request_tool_policy,
            ephemeral,
            stream_only,
        } = parse(&params, self.has_tools_sync())?;

        // Resolve session key from explicit overrides, public request params, or connection context.
        let session_key = self.resolve_session_key_from_params(&params).await;
        // Use exactly the gateway/runner predicate so authorized `/sh` forms
        // cannot fall through into a less restricted agent turn.
        let explicit_shell_command = match &message_content {
            MessageContent::Text(raw) => moltis_agents::runner::explicit_shell_command(raw),
            MessageContent::Multimodal(_) => None,
        };
        let channel_bound_web = self
            .apply_channel_bound_public_context(&mut params, &session_key)
            .await?;
        if channel_bound_web && explicit_shell_command.is_some() {
            return Err(
                "shell commands cannot run in a channel-bound web session; switch sessions first"
                    .into(),
            );
        }

        // Resolve request restrictions after channel binding, which can only
        // narrow caller-supplied policy and private-context access.
        let request_tool_policy = if channel_bound_web {
            tool_policy::parse_request_tool_policy(&params)?
        } else {
            request_tool_policy
        };
        let request_tool_audience = tool_policy::parse_request_tool_audience(&params)?;
        let private_context = tool_policy::allows_private_context(&params);
        if !private_context {
            public_context::mark_public_channel(&mut params);
        }
        let request_tool_registry = tool_policy::resolve_request_tool_registry(
            &self.tool_registry,
            request_tool_policy.as_ref(),
            request_tool_audience,
        )
        .await;

        let queued_replay = params
            .get("_queued_replay")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Carried through queueing/replay so reactions follow the message.
        let ack_keys = crate::channel_acks::ack_keys_from_params(&params);
        // This identity owns all activity and cleanup for the admitted turn.
        let run_id = uuid::Uuid::new_v4().to_string();

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

        info!(
            session = %session_key,
            text_len = text.len(),
            has_content = params.get("content").is_some(),
            model = ?explicit_model,
            client_seq = ?client_seq,
            queued_replay,
            "chat.send: received"
        );

        let message_queue_mode = self.config.chat.message_queue_mode;
        let admission = if queue_if_busy && explicit_shell_command.is_none() {
            let mut queued_params = params.clone();
            // Delayed channel turns cannot retain authorization that may be
            // revoked before replay. The original params remain available if
            // admission succeeds immediately.
            if tool_policy::downgrade_queued_channel_request(&mut queued_params, private_context) {
                public_context::mark_public_channel(&mut queued_params);
            }
            self.admit_turn(&session_key, queued_params, queued_replay)
                .await
        } else {
            let session_sem = self.session_semaphore(&session_key).await;
            let queues = self.message_queue.write().await;
            if queues
                .get(&session_key)
                .is_some_and(|queue| queue.draining || !queue.messages.is_empty())
            {
                return Err(if explicit_shell_command.is_some() {
                    "shell commands cannot be queued; retry when the active run finishes".into()
                } else {
                    "session already has an active turn".into()
                });
            }
            let permit = match session_sem.try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => {
                    return Err(if explicit_shell_command.is_some() {
                        "shell commands cannot be queued; retry when the active run finishes".into()
                    } else {
                        "session already has an active turn".into()
                    });
                },
            };
            drop(queues);
            TurnAdmission::Acquired(permit)
        };
        let permit: OwnedSemaphorePermit = match admission {
            TurnAdmission::Acquired(p) => {
                info!(
                    session = %session_key,
                    client_seq = ?client_seq,
                    queued_replay,
                    "chat.send: acquired session permit"
                );
                self.state
                    .activate_channel_acks(&run_id, &session_key, ack_keys.clone())
                    .await;
                p
            },
            TurnAdmission::Queued(position) => {
                let queue_mode = message_queue_mode;
                info!(
                    session = %session_key,
                    mode = ?queue_mode,
                    position,
                    client_seq = ?client_seq,
                    queued_replay,
                    "chat.send: queued because session is active"
                );
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

        if let Some(shell_command) = explicit_shell_command {
            if request_tool_policy
                .as_ref()
                .is_some_and(|policy| !policy.is_allowed("exec"))
            {
                self.finish_unstarted_turn(&run_id, &session_key, permit, queued_replay)
                    .await;
                return Err("exec tool is denied by the request tool policy".into());
            }
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

            let history = match self.load_turn_history(&session_key, history_limits).await {
                Ok(history) => history,
                Err(error) => {
                    self.finish_unstarted_turn(&run_id, &session_key, permit, queued_replay)
                        .await;
                    return Err(error);
                },
            };
            let user_message_index = history.len();

            // Ensure the session exists in metadata and counts are up to date.
            if !ephemeral {
                let _ = self.session_metadata.upsert(&session_key, None).await;
                self.session_metadata
                    .touch(&session_key, history.len() as u32)
                    .await;
            }

            // `/sh` reaching this point can only be a native channel turn the
            // gateway authorized, so its reply target is already in `params`.
            // Web turns on a channel-bound session were rejected above, and
            // `apply_channel_bound_public_context` is what derives a target from
            // the session's binding for every other request.
            let deferred_channel_target = params
                .get(crate::params::CHANNEL_REPLY_TARGET)
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
                user_message_bytes = text.len(),
                session = %session_key,
                command_bytes = shell_command.len(),
                client_seq = ?client_seq,
                mode = "explicit_shell",
                "chat.send"
            );

            // Persist user message now that it will execute immediately.
            if !ephemeral
                && let Err(e) = self
                    .session_store
                    .append(&session_key, &user_msg.to_value())
                    .await
            {
                warn!("failed to persist /sh user message: {e}");
            }

            // Set preview from first user message if not already set.
            if !ephemeral
                && let Some(entry) = self.session_metadata.get(&session_key).await
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
            let tool_registry = Arc::clone(&request_tool_registry);
            let session_key_clone = session_key.clone();
            let message_queue = Arc::clone(&self.message_queue);
            let state_for_drain = Arc::clone(&self.state);
            let accept_language = params
                .get("_accept_language")
                .and_then(|v| v.as_str())
                .map(String::from);
            let conn_id_for_tool = conn_id.clone();
            let (_, working_dir) = self
                .resolve_turn_context(&session_key, conn_id.as_deref())
                .await;
            let working_dir = working_dir.map(|directory| directory.display().to_string());

            let (start_run, run_registered) = tokio::sync::oneshot::channel();
            let handle = tokio::spawn(async move {
                if run_registered.await.is_err() {
                    return;
                }
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
                    &terminal_runs,
                    &tool_registry,
                    (!ephemeral).then_some(&session_store),
                    &session_key_clone,
                    &shell_command,
                    user_message_index,
                    accept_language,
                    conn_id_for_tool,
                    client_seq,
                    working_dir,
                )
                .await;

                let mut assistant_output = assistant_output;
                let completion_result = Ok(turn_result(&assistant_output));
                let final_payload = assistant_output.final_broadcast.take();
                let assistant_msg = (!ephemeral).then(|| {
                    build_persisted_assistant_message(
                        assistant_output,
                        None,
                        None,
                        client_seq,
                        Some(run_id_clone.clone()),
                    )
                });
                commit_successful_turn(
                    &terminal_runs,
                    &run_id_clone,
                    async {
                        if let Some(assistant_msg) = assistant_msg {
                            if let Err(e) = session_store
                                .append(&session_key_clone, &assistant_msg.to_value())
                                .await
                            {
                                warn!("failed to persist /sh assistant message: {e}");
                            }
                            if let Ok(count) = session_store.count(&session_key_clone).await {
                                session_metadata.touch(&session_key_clone, count).await;
                            }
                        }

                        // Explicit /sh runs never reach the model completion path.
                        crate::channel_acks::note_turn_finished(&state, &run_id_clone, true).await;
                    },
                    async {
                        if let Some(payload) = final_payload {
                            broadcast(&state, "chat", payload, BroadcastOpts::default()).await;
                        }
                    },
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

                drop(permit);
                if let Some(completion) = completion {
                    let _ = completion.send(completion_result);
                }
                queue_drain::drain_and_replay(
                    &message_queue,
                    &session_key_clone,
                    message_queue_mode,
                    &state_for_drain,
                )
                .await;
            });

            Self::register_run_handle(
                &self.active_runs,
                &self.active_runs_by_session,
                &run_id,
                &session_key,
                handle.abort_handle(),
            )
            .await;
            let _ = start_run.send(());

            info!(
                run_id = %run_id,
                session = %session_key,
                client_seq = ?client_seq,
                mode = "explicit_shell",
                "chat.send: returning run id"
            );
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
        let model_id = explicit_model.as_deref().or(session_model.as_deref());

        let provider_result: Result<Arc<dyn moltis_agents::model::LlmProvider>, String> = {
            let reg = self.providers.read().await;
            let primary_result = if let Some(id) = model_id {
                reg.get(id).ok_or_else(|| {
                    let available: Vec<_> =
                        reg.list_models().iter().map(|m| m.id.clone()).collect();
                    format!("model '{}' not found. available: {:?}", id, available)
                })
            } else if !stream_only {
                reg.first_with_tools()
                    .ok_or_else(|| "no LLM providers configured".to_string())
            } else {
                reg.first()
                    .ok_or_else(|| "no LLM providers configured".to_string())
            };

            match primary_result {
                Err(error) => Err(error),
                Ok(primary) => {
                    let user_selected = model_id.is_some();
                    let skip_failover = !self.failover_config.enabled
                        || (self.failover_config.exact_model && user_selected);
                    if skip_failover {
                        Ok(primary)
                    } else {
                        let fallbacks = if self.failover_config.fallback_models.is_empty() {
                            reg.fallback_providers_for(primary.id(), primary.name())
                        } else {
                            reg.providers_for_models(&self.failover_config.fallback_models)
                        };
                        if fallbacks.is_empty() {
                            Ok(primary)
                        } else {
                            let mut chain = vec![primary];
                            chain.extend(fallbacks);
                            Ok(Arc::new(moltis_agents::provider_chain::ProviderChain::new(
                                chain,
                            )))
                        }
                    }
                },
            }
        };
        let provider = match provider_result {
            Ok(provider) => provider,
            Err(error) => {
                self.finish_unstarted_turn(&run_id, &session_key, permit, queued_replay)
                    .await;
                return Err(error.into());
            },
        };
        info!(
            session = %session_key,
            provider = provider.name(),
            model = provider.id(),
            stream_only,
            client_seq = ?client_seq,
            "chat.send: provider resolved"
        );

        // Check if this is a local model that needs downloading/loading.
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
                self.finish_unstarted_turn(&run_id, &session_key, permit, queued_replay)
                    .await;
                return Err(format!("Failed to prepare local model: {}", e).into());
            }
            // Pre-load the model into RAM (broadcasts lifecycle events so the
            // chat UI shows "Loading model X into memory..." before inference).
            if let Err(e) = self.state.ensure_local_model_loaded(&model_to_check).await {
                tracing::warn!(model = model_to_check, error = %e, "lifecycle pre-load failed, inference will still lazy-load");
            }
        }

        // Resolve project context plus optional command-generated context.
        let (project_context, working_dir) = if private_context {
            self.resolve_turn_context(&session_key, conn_id.as_deref())
                .await
        } else {
            (None, None)
        };

        // Load conversation history (the current user message is NOT yet
        // persisted — run_streaming / run_agent_loop add it themselves).
        let mut history = match self.load_turn_history(&session_key, history_limits).await {
            Ok(history) => history,
            Err(error) => {
                self.finish_unstarted_turn(&run_id, &session_key, permit, queued_replay)
                    .await;
                return Err(error);
            },
        };
        let persisted_history_len = history.len();
        if !private_context {
            history = public_context::filter_public_history(history);
        }
        info!(
            session = %session_key,
            history_len = history.len(),
            client_seq = ?client_seq,
            "chat.send: history loaded"
        );

        // Update metadata.
        if !ephemeral {
            let _ = self.session_metadata.upsert(&session_key, None).await;
            self.session_metadata
                .touch(&session_key, persisted_history_len as u32)
                .await;
        }

        let deferred_channel_target = params
            .get(crate::params::CHANNEL_REPLY_TARGET)
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
            info!(
                session = %session_key,
                client_seq = ?client_seq,
                "chat.send: dispatching MessageReceived hook"
            );
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

                    self.finish_unstarted_turn(&run_id, &session_key, permit, queued_replay)
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
            info!(
                session = %session_key,
                client_seq = ?client_seq,
                "chat.send: MessageReceived hook complete"
            );
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
        let discovered_skills = discover_skills_if_enabled(&self.config).await;

        // Check if MCP tools are disabled for this session and capture
        // per-session sandbox override details for prompt runtime context.
        let session_entry = self.session_metadata.get(&session_key).await;
        let session_agent_id = resolve_prompt_agent_id(session_entry.as_ref());

        // Apply per-agent skill policy (allow/deny by name or category).
        let discovered_skills =
            if let Some(preset) = self.config.agents.get_preset(&session_agent_id) {
                filter_skills_for_agent(discovered_skills, &preset.skills)
            } else {
                discovered_skills
            };
        info!(
            session = %session_key,
            skills_len = discovered_skills.len(),
            agent_id = %session_agent_id,
            client_seq = ?client_seq,
            "chat.send: skills discovered"
        );
        info!(
            session = %session_key,
            agent_id = %session_agent_id,
            client_seq = ?client_seq,
            "chat.send: loading persona"
        );
        let persona = load_prompt_persona_for_session(
            &session_key,
            session_entry.as_ref(),
            self.session_state_store.as_deref(),
        )
        .await;
        let runtime_limits = persona.config.agent_runtime_limits(&session_agent_id);
        info!(
            session = %session_key,
            agent_id = %session_agent_id,
            timeout_secs = runtime_limits.timeout_secs,
            timeout_source = runtime_limits.timeout_source.as_str(),
            max_iterations = runtime_limits.max_iterations,
            max_iterations_source = runtime_limits.max_iterations_source.as_str(),
            client_seq = ?client_seq,
            "chat.send: persona loaded"
        );
        let mcp_disabled = session_entry
            .as_ref()
            .and_then(|entry| entry.mcp_disabled)
            .unwrap_or(false);
        info!(
            session = %session_key,
            client_seq = ?client_seq,
            "chat.send: building runtime context"
        );
        let mut runtime_context = build_prompt_runtime_context(
            &self.state,
            &persona.config,
            &provider,
            &session_key,
            session_entry.as_ref(),
        )
        .await;
        runtime_context.mode = resolve_prompt_mode_context(&persona.config, session_entry.as_ref());
        runtime_context.host.working_dir = working_dir
            .as_ref()
            .map(|directory| directory.display().to_string());
        apply_request_runtime_context(&mut runtime_context.host, &params);
        info!(
            session = %session_key,
            agent_id = %session_agent_id,
            mcp_disabled,
            has_project_context = project_context.is_some(),
            client_seq = ?client_seq,
            "chat.send: runtime context built"
        );

        let state = Arc::clone(&self.state);
        let active_runs = Arc::clone(&self.active_runs);
        let active_runs_by_session = Arc::clone(&self.active_runs_by_session);
        let active_thinking_text = Arc::clone(&self.active_thinking_text);
        let active_tool_calls = Arc::clone(&self.active_tool_calls);
        let active_partial_assistant = Arc::clone(&self.active_partial_assistant);
        let active_reply_medium = Arc::clone(&self.active_reply_medium);
        let run_id_clone = run_id.clone();
        let overlay = self
            .session_tool_overlays
            .read()
            .await
            .get(&session_key)
            .cloned();
        let tool_registry = if let Some(overlay) = overlay {
            let mut combined = self.tool_registry.read().await.clone_allowed_by(|_| true);
            let overlay = overlay.read().await;
            combined.extend_from(&overlay);
            let combined = Arc::new(tokio::sync::RwLock::new(combined));
            tool_policy::resolve_request_tool_registry(
                &combined,
                request_tool_policy.as_ref(),
                request_tool_audience,
            )
            .await
        } else {
            Arc::clone(&request_tool_registry)
        };
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
            user_message_bytes = text.len(),
            model = provider.id(),
            stream_only,
            session = %session_key,
            reply_medium = ?desired_reply_medium,
            client_seq = ?client_seq,
            "chat.send"
        );

        // Capture user message index (0-based) so we can include assistant
        // message index in the "final" broadcast for client-side deduplication.
        let user_message_index = persisted_history_len; // user msg is at this index in the JSONL

        let provider_name = provider.name().to_string();
        let model_id = provider.id().to_string();
        if !ephemeral
            && self
                .session_metadata
                .get(&session_key)
                .await
                .and_then(|entry| entry.model)
                .as_deref()
                != Some(model_id.as_str())
        {
            self.session_metadata
                .set_model(&session_key, Some(model_id.clone()))
                .await;
        }
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
        // the session persona and matches the pre-PR-#653 hardcoded trigger.
        let compaction_cfg = &persona.config.chat.compaction;
        let context_window = provider.context_window() as u64;
        let token_usage = session_token_usage_from_messages(&history);
        let estimated_next_input = token_usage
            .current_request_input_tokens
            .saturating_add(estimate_text_tokens(&text));
        let compact_threshold =
            compute_auto_compact_threshold(context_window, compaction_cfg.threshold_percent);

        if private_context && !ephemeral && estimated_next_input >= compact_threshold {
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
                    history = match self.load_turn_history(&session_key, history_limits).await {
                        Ok(history) => history,
                        Err(error) => {
                            self.finish_unstarted_turn(
                                &run_id,
                                &session_key,
                                permit,
                                queued_replay,
                            )
                            .await;
                            return Err(error);
                        },
                    };
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

        // Persist the user message now that we know it won't be queued.
        // (Queued messages skip this; they are persisted when replayed.)
        if !ephemeral
            && let Err(e) = self
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
        if !ephemeral
            && let Some(entry) = self.session_metadata.get(&session_key).await
            && entry.preview.is_none()
        {
            let preview_text = extract_preview_from_value(&user_msg.to_value());
            if let Some(preview) = preview_text {
                self.session_metadata
                    .set_preview(&session_key, Some(&preview))
                    .await;
            }
        }

        let outer_agent_timeout_secs = if stream_only {
            runtime_limits.timeout_secs
        } else {
            0
        };

        let message_queue = Arc::clone(&self.message_queue);
        let state_for_drain = Arc::clone(&self.state);
        let active_event_forwarders = Arc::clone(&self.active_event_forwarders);
        let terminal_runs = Arc::clone(&self.terminal_runs);
        let deferred_channel_target = deferred_channel_target.clone();

        let (start_run, run_registered) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            if run_registered.await.is_err() {
                return;
            }
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
            if !ephemeral {
                active_partial_assistant.write().await.insert(
                    session_key_clone.clone(),
                    ActiveAssistantDraft::new(&run_id_clone, &model_id, &provider_name, client_seq),
                );
            }
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
            let extraction_max_tool_result_bytes = persona.config.tools.max_tool_result_bytes;
            let auto_title_enabled = persona.config.chat.auto_title;
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
                        (!ephemeral).then_some(&session_store),
                        client_seq,
                        (!ephemeral).then(|| Arc::clone(&active_partial_assistant)),
                        &terminal_runs,
                        private_context,
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
                        (!ephemeral).then_some(&session_store),
                        mcp_disabled,
                        client_seq,
                        Some(Arc::clone(&active_thinking_text)),
                        Some(Arc::clone(&active_tool_calls)),
                        (!ephemeral).then(|| Arc::clone(&active_partial_assistant)),
                        &active_event_forwarders,
                        &terminal_runs,
                        sender_name,
                        Some(tool_controls),
                        private_context,
                    )
                    .await
                }
            };

            let assistant_text = if outer_agent_timeout_secs > 0 {
                tokio::pin!(agent_fut);
                let deadline = tokio::time::sleep(Duration::from_secs(outer_agent_timeout_secs));
                tokio::pin!(deadline);
                tokio::select! {
                    result = &mut agent_fut => result,
                    () = &mut deadline => {
                        // A committed run is only completing bounded final I/O;
                        // do not replace an accepted channel final with timeout.
                        if terminal_runs.read().await.contains(&run_id_clone) {
                            agent_fut.await
                        } else {
                        warn!(
                            run_id = %run_id_clone,
                            session = %session_key_clone,
                            timeout_secs = outer_agent_timeout_secs,
                            "agent run timed out"
                        );
                        let detail =
                            format!("Agent run timed out after {outer_agent_timeout_secs}s");
                        let error_obj = serde_json::json!({
                            "type": "timeout",
                            "title": "Timed out",
                            "detail": detail,
                        });
                        state.set_run_error(&run_id_clone, detail.clone()).await;
                        commit_terminal_run(&terminal_runs, &run_id_clone).await;
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
                        }
                    }
                }
            } else {
                agent_fut.await
            };

            let completion_result = match assistant_text.as_ref() {
                Some(output) => Ok(turn_result(output)),
                None => Err(state
                    .last_run_error(&run_id_clone)
                    .await
                    .unwrap_or_else(|| "agent run failed (check server logs)".to_string())
                    .into()),
            };

            // Channel delivery is complete when a successful output returns.
            // Claim terminal ownership before persistence so abort cannot turn
            // a committed assistant message into an aborted run.
            if let Some(mut assistant_output) = assistant_text {
                let final_payload = assistant_output.final_broadcast.take();
                let assistant_msg = (!ephemeral).then(|| {
                    build_persisted_assistant_message(
                        assistant_output,
                        Some(model_id.clone()),
                        Some(provider_name.clone()),
                        client_seq,
                        Some(run_id_clone.clone()),
                    )
                });
                commit_successful_turn(
                    &terminal_runs,
                    &run_id_clone,
                    async {
                        if let Some(assistant_msg) = assistant_msg
                            && let Err(e) = session_store
                                .append(&session_key_clone, &assistant_msg.to_value())
                                .await
                        {
                            warn!("failed to persist assistant message: {e}");
                        }
                        if !ephemeral {
                            crate::channel_feedback::record_web_reply_trace(
                                &state,
                                &session_key_clone,
                                &run_id_clone,
                            )
                            .await;
                        }
                        crate::channel_acks::note_turn_finished(&state, &run_id_clone, true).await;
                    },
                    async {
                        if let Some(payload) = final_payload {
                            broadcast(&state, "chat", payload, BroadcastOpts::default()).await;
                        }
                    },
                )
                .await;

                // Update metadata counts.
                if !ephemeral && let Ok(count) = session_store.count(&session_key_clone).await {
                    session_metadata.touch(&session_key_clone, count).await;

                    // ── Periodic background memory extraction ──────────────
                    // Every `auto_extract_interval` turns, spawn a background
                    // silent turn to save important recent context to memory.
                    // Uses config values captured before persona was moved.
                    let interval = auto_extract_interval;
                    let write_mode = extraction_write_mode;
                    let max_tool_result_bytes = extraction_max_tool_result_bytes;
                    // A "turn" = user + assistant = 2 messages.
                    let turn_number = count / 2;
                    if private_context
                        && interval > 0
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
                            let chat_msgs = values_to_chat_messages_with_tool_result_limit(
                                &recent,
                                max_tool_result_bytes,
                            );
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
            } else {
                crate::channel_acks::note_turn_finished(&state, &run_id_clone, false).await;
            }

            // ── Auto-title generation ──────────────────────────────
            // After the first completed turn, trigger background title
            // generation. We check >= 2 (not == 2) because agentic turns
            // with tool calls produce more than 2 stored messages.
            // `generate_title_if_needed` guards against duplicate titles.
            if private_context
                && !ephemeral
                && auto_title_enabled
                && let Ok(count) = session_store.count(&session_key_clone).await
                && count >= 2
                && !queued_replay
            {
                state.trigger_auto_title(&session_key_clone).await;
            }

            let _ =
                LiveChatService::wait_for_event_forwarder(&active_event_forwarders, &run_id_clone)
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
            if let Some(completion) = completion {
                let _ = completion.send(completion_result);
            }
            queue_drain::drain_and_replay(
                &message_queue,
                &session_key_clone,
                message_queue_mode,
                &state_for_drain,
            )
            .await;
        });

        Self::register_run_handle(
            &self.active_runs,
            &self.active_runs_by_session,
            &run_id,
            &session_key,
            handle.abort_handle(),
        )
        .await;
        let _ = start_run.send(());

        info!(
            run_id = %run_id,
            session = %session_key,
            client_seq = ?client_seq,
            "chat.send: returning run id"
        );
        Ok(serde_json::json!({ "ok": true, "runId": run_id }))
    }
}
